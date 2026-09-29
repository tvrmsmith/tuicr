//! Polls the forge for the PR head while a PR is under review, and reloads
//! onto a new head the same way `:e` does once the reviewer is free.

use std::sync::mpsc::{Receiver, TryRecvError};

use super::*;
use crate::slug::short_sha;

/// How often PR mode asks the forge for the PR's head.
pub(crate) const PR_HEAD_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// A PR head move the poll saw that is not on screen yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrHeadMove {
    pub from: String,
    pub to: String,
}

impl PrHeadMove {
    pub fn describe(&self) -> String {
        format!("{} → {}", short_sha(&self.from), short_sha(&self.to))
    }
}

/// An in-flight head poll, tagged with the PR it asked about so a result
/// outliving a switch to another PR is dropped.
pub(crate) struct PrHeadPoll {
    pub(in crate::app) repository: ForgeRepository,
    pub(in crate::app) number: u64,
    pub(in crate::app) rx: Receiver<std::result::Result<String, String>>,
}

impl App {
    /// Per-tick entry point from the event loop. Returns `true` when a redraw
    /// is needed.
    pub fn poll_pr_head_watch(&mut self) -> bool {
        let landed = self.poll_pr_head_poll_events();
        let applied = self.apply_pending_pr_head_move();
        let now = Instant::now();
        if self.pr_head_poll_due(now) {
            self.next_pr_head_poll_at = now + PR_HEAD_POLL_INTERVAL;
            self.spawn_pr_head_poll();
        }
        landed || applied
    }

    pub(crate) fn pr_head_poll_due(&self, now: Instant) -> bool {
        matches!(self.diff_source, DiffSource::PullRequest(_))
            && self.pr_head_poll.is_none()
            && now >= self.next_pr_head_poll_at
    }

    /// Starts the poll clock over for a freshly entered PR head, dropping
    /// anything the previous head's poll left behind.
    pub(in crate::app) fn rearm_pr_head_watch(&mut self) {
        self.next_pr_head_poll_at = Instant::now() + PR_HEAD_POLL_INTERVAL;
        self.pr_head_poll = None;
        self.pr_head_move = None;
        self.last_pr_head_poll_error = None;
    }

    /// Records the head the forge reported. A head that differs from the one
    /// on screen becomes a pending move; one that matches clears it.
    pub(crate) fn note_polled_pr_head(&mut self, head_sha: &str) {
        let DiffSource::PullRequest(current) = &self.diff_source else {
            return;
        };
        let shown = current.key.head_sha.clone();
        self.current_pr_head = Some(head_sha.to_string());
        self.pr_head_move = (head_sha != shown).then(|| PrHeadMove {
            from: shown,
            to: head_sha.to_string(),
        });
    }

    /// Reloads onto a pending head move unless the reviewer is busy. Any mode
    /// but `Normal` defers it, so a comment draft or a submit modal is never
    /// torn down under the reviewer. A `:e` that lands a new head first clears
    /// the move through `rearm_pr_head_watch`. Returns `true` when state
    /// changed.
    pub(crate) fn apply_pending_pr_head_move(&mut self) -> bool {
        // The reviewer left PR mode while the move waited; it is moot now.
        if self.pr_head_move.is_some() && !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            self.pr_head_move = None;
            return true;
        }
        if self.input_mode != InputMode::Normal
            || self.pr_reload_state.is_some()
            || self.pr_submit_state.is_some()
        {
            return false;
        }
        let Some(head_move) = self.pr_head_move.take() else {
            return false;
        };
        match self.spawn_pr_reload() {
            Ok(()) => {
                self.set_message(format!("PR head moved {}, reloading", head_move.describe()))
            }
            Err(e) => self.set_error(format!("Reload failed: {e}")),
        }
        true
    }

    fn spawn_pr_head_poll(&mut self) {
        use crate::forge::traits::PullRequestTarget;

        let DiffSource::PullRequest(current) = &self.diff_source else {
            return;
        };
        let repository = current.key.repository.clone();
        let number = current.key.number;
        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let show_pr_checks = self.show_pr_checks;
        let show_pr_comments = self.show_pr_comments;

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_head_poll = Some(PrHeadPoll {
            repository: repository.clone(),
            number,
            rx,
        });
        std::thread::spawn(move || {
            let backend = create_forge_backend(
                &repository,
                local_checkout,
                show_pr_checks,
                show_pr_comments,
            );
            let target = PullRequestTarget::with_repository(repository, number, number.to_string());
            let head = backend
                .get_pull_request(target)
                .map(|details| details.head_sha)
                .map_err(|e| e.to_string());
            let _ = tx.send(head);
        });
    }

    /// Pumps a landed head poll. A failure warns once per distinct error so a
    /// dropped network connection does not repeat the warning every minute.
    fn poll_pr_head_poll_events(&mut self) -> bool {
        let Some(poll) = self.pr_head_poll.take() else {
            return false;
        };
        let result = match poll.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                self.pr_head_poll = Some(poll);
                return false;
            }
            Err(TryRecvError::Disconnected) => {
                Err("the poll thread exited without an answer".to_string())
            }
        };
        if !self
            .shown_pr_key()
            .is_some_and(|key| key.repository == poll.repository && key.number == poll.number)
        {
            return false;
        }

        match result {
            Ok(head_sha) => {
                self.last_pr_head_poll_error = None;
                self.note_polled_pr_head(&head_sha);
                true
            }
            Err(err) => {
                let text = format!("PR head check failed: {err}");
                if self.last_pr_head_poll_error.as_deref() == Some(text.as_str()) {
                    return false;
                }
                self.last_pr_head_poll_error = Some(text.clone());
                self.set_warning(text);
                true
            }
        }
    }
}
