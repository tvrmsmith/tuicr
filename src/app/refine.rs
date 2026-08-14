//! Blocking refine: the wait, the screens it draws, and the two ways out of it.
//!
//! It runs on both entry paths, because `[grouping].refine` is a statement
//! about the review and not about the command line. A target given up front
//! (`-r`, `--working-tree`, `--all-files`, `tuicr <sha>`) is refined before the
//! terminal enters the alternate screen, on the stderr status line below. A
//! target picked in the TUI — bare `tuicr`, or `tuicr pr` before a PR is
//! chosen — has no pre-TUI moment to block in, so the same wait runs when the
//! diff first loads and paints itself into the alternate screen instead
//! ([`Painted`]). The semantics are identical on both: it blocks, the cancel
//! keys abandon it, and every failure opens the heuristic partition.
//!
//! **The TUI does not render until the grouping is final** (`gd-26r.14`, upheld
//! by `gd-26r.27`). What buys that block is partition quality — F1 0.427 and
//! 0.711 against the heuristics' 0.394 and 0.378 — and *not* group order, which
//! came out at +0.042 on fixture 1 after a 30–70 second wait. So the screen
//! below is honest about a wait that can run over a minute, and both escapes
//! from it are first-class outcomes rather than error paths:
//!
//! * **cancel**, which is why blocking startup needs a minimal event read at
//!   all — a wait with no input loop cannot honour it; and
//! * **a timeout**, generous by default because the money is spent at dispatch,
//!   so a premature one throws away a paid-for result and gets the unordered
//!   grouping anyway.
//!
//! Cancelled, timed out, unauthenticated, unparseable twice: all land in the
//! same place, a working heuristic-grouped session with the within-group sort
//! already applied. The heuristic partition is total by construction, so
//! falling back can never leave a file unassigned.
//!
//! There is deliberately **no async startup path** here. `gd-26r.27` looked at
//! building one and did not relax blocking, so nothing lands under a reader at
//! startup: the wait either produces the whole grouping or produces nothing.

use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use super::App;
use crate::grouping::changeset::Changeset;
use crate::grouping::passes::GroupingConfig;
use crate::grouping::refine::{self, Refined};
use crate::grouping::{Grouping, vertex};

/// Redraw cadence for the elapsed clock, and the longest the wait can go
/// without noticing a keypress or an answer.
const TICK: Duration = Duration::from_millis(200);

/// One retry, on parse failure only, with the identical prompt resent and its
/// own full timeout (`docs/GROUPS_CONTRACT.md`). Two independent one-shot calls,
/// so the single-shot ruling holds literally — there is no conversation and no
/// error fed back.
const ATTEMPTS: u32 = 2;

/// How the blocking wait ended.
#[derive(Debug)]
pub enum RefineOutcome {
    /// The refined grouping is parked on the app for `enable_grouping` to
    /// adopt, with the number of contract violations that had to be repaired.
    Refined {
        repairs: usize,
    },
    /// The human did not want to wait. Not an error, and the screen must not
    /// imply the environment failed.
    Cancelled,
    TimedOut,
    Failed(String),
    /// Refine was never dispatched, and why.
    Skipped(Skipped),
}

/// Why a configured refine did not run. The two cases mean opposite things to
/// the human and are kept apart rather than collapsed: one is a wait still to
/// come, the other is a wait that will never come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// There is no changeset yet — bare `tuicr` on the target selector, or
    /// `tuicr pr` before a PR is picked. Nothing has been decided: the wait is
    /// dispatched when the diff loads.
    NoChangeset,
    /// `[grouping].refine` is off, or config never reached the app. Nothing was
    /// asked for and nothing is reported.
    NotConfigured,
    /// The session already holds a grouping of this changeset. Reopening never
    /// refines (`docs/REGROUPING_STATE.md`), so the wait is once per review.
    AlreadyGrouped,
}

impl RefineOutcome {
    /// What to tell the human afterwards, if anything. `None` is the quiet
    /// path: a clean refine, and a refine that has not run *yet*.
    ///
    /// A configured refine that will never run on this session is not quiet.
    /// `[grouping].refine = true` and no wait, no warning and no refined group
    /// is indistinguishable from the setting being ignored, which is the one
    /// reading of it that is wrong.
    ///
    /// A repaired one is not quiet. The count is the arm's verdict
    /// (`docs/GROUPS_CONTRACT.md`) and a model that invented five paths is one
    /// config edit away at all times, so it joins the other startup warnings
    /// rather than being computed and dropped.
    pub fn warning(&self) -> Option<String> {
        match self {
            RefineOutcome::Refined { repairs: 0 }
            | RefineOutcome::Skipped(Skipped::NoChangeset)
            | RefineOutcome::Skipped(Skipped::NotConfigured) => None,
            RefineOutcome::Skipped(Skipped::AlreadyGrouped) => Some(
                "Refine skipped; this session's saved grouping was kept. Reopening never \
                 regroups."
                    .to_string(),
            ),
            RefineOutcome::Refined { repairs } => Some(format!(
                "Refine applied with {repairs} contract violation{} repaired.",
                if *repairs == 1 { "" } else { "s" }
            )),
            RefineOutcome::Cancelled => {
                Some("Refine cancelled; grouped by heuristics.".to_string())
            }
            RefineOutcome::TimedOut => Some(
                "Refine timed out; grouped by heuristics. Raise \
                 [grouping].refine_timeout_ms to wait longer."
                    .to_string(),
            ),
            RefineOutcome::Failed(reason) => Some(format!(
                "Refine unavailable ({reason}); grouped by heuristics."
            )),
        }
    }
}

/// The call itself, behind a handle the wait can abandon. `'static` because it
/// runs on a thread the wait does not join: cancelling means walking away from
/// a request in flight, not stopping it.
pub type RefineCall = Arc<dyn Fn(&str, Duration) -> Result<String, String> + Send + Sync>;

/// The pre-TUI progress surface. A trait so the wait can be tested without a
/// terminal — and so the two states a human must be able to tell apart, waiting
/// and retrying, are named rather than formatted inline.
pub trait Screen {
    fn waiting(&mut self, elapsed: Duration, attempt: u32);
    /// The wait is over. The stderr line wipes itself here, because the TUI
    /// opens on the next frame and must not find half a status line under it;
    /// the in-TUI overlay has nothing to do, since the frame the caller draws
    /// next is what removes it.
    fn finish(&mut self);
}

/// The minimal event read blocking startup requires.
pub trait CancelKeys {
    /// Wait up to `within` for input. `true` means the human asked to stop.
    fn cancelled(&mut self, within: Duration) -> bool;
    /// Whether a key can be read at all. The screen advertises the cancel key
    /// only when this is `true`, so the wait never offers an escape it would
    /// then ignore.
    fn cancellable(&self) -> bool {
        true
    }
}

/// What `[grouping]` settled about the arm, kept on the app because both entry
/// paths dispatch from it and only one of them is reachable from the binary's
/// startup code.
#[derive(Debug, Clone)]
pub struct RefineConfig {
    pub timeout: Duration,
    pub settings: vertex::Settings,
}

impl App {
    /// Runs the blocking refine over a target given up front, if it is going to
    /// run at all, and parks the result for [`App::enable_grouping`] to pick up.
    ///
    /// Called from the binary before the terminal enters the alternate screen,
    /// because there is no TUI yet to render into and the whole point of the
    /// decision is that there is not one until this returns. Bare `tuicr` has
    /// no changeset here and is refined by [`App::refine_loaded_diff`] instead.
    pub fn refine_grouping_at_startup(&mut self) -> RefineOutcome {
        let Some(config) = self.refine_config.clone() else {
            return RefineOutcome::Skipped(Skipped::NotConfigured);
        };
        // The skip decision comes before anything with a side effect: a session
        // that is not going to refine must not have raw mode toggled under it,
        // nor pay for a second parse of the changeset.
        let changeset = match self.refinable_over_a_new_session() {
            Ok(changeset) => changeset,
            Err(why) => return RefineOutcome::Skipped(why),
        };
        let files = changeset.len();
        // The screen may only advertise the key the keyboard can actually
        // read: raw mode fails on a run with no controlling terminal, and a
        // wait that tells the human to press Esc and then ignores it is worse
        // than one that admits it cannot be interrupted.
        let mut keys = TerminalKeys::enter();
        let mut screen = StatusLine::over(files, keys.cancellable());
        self.refine_changeset(
            changeset,
            config.timeout,
            live_call(config.settings),
            &mut screen,
            &mut keys,
        )
    }

    /// The same wait, over a diff that loaded after the alternate screen was
    /// already up: a target picked in the TUI, or a PR chosen from the picker.
    ///
    /// `paint` draws one frame of the wait — the in-TUI equivalent of the
    /// stderr status line, which cannot be used here because the next
    /// `Terminal::draw` would paint over it.
    ///
    /// The session check the pre-TUI path makes cannot be repeated as written:
    /// [`App::order_files_by_group`] records whatever grouping it installed, so
    /// by the time this runs the session always holds one. It is made against
    /// [`App::refine_over_saved_grouping`] instead, which is that same question
    /// answered before the recording — so a reopened review still refuses, and
    /// still says so.
    pub fn refine_loaded_diff(&mut self, paint: &mut dyn FnMut(&str)) -> RefineOutcome {
        // Cleared before anything can return, because the main loop dispatches
        // on it: an exit that left it set would re-enter the wait every tick.
        self.refine_wanted = false;
        let Some(config) = self.refine_config.clone() else {
            return RefineOutcome::Skipped(Skipped::NotConfigured);
        };
        self.refine_loaded_with(
            config.timeout,
            live_call(config.settings),
            paint,
            &mut TerminalKeys::borrowed(),
        )
    }

    /// The seam the tests drive: same wait, with the call, the screen and the
    /// keyboard supplied. Cancel and timeout are the two paths most likely to
    /// be built and never exercised, so they are exercised here.
    ///
    /// Test-only: the binary reaches the same wait through
    /// [`App::refine_grouping_at_startup`], which supplies the live call and a
    /// real terminal.
    #[cfg(test)]
    pub(crate) fn refine_grouping_with(
        &mut self,
        timeout: Duration,
        call: RefineCall,
        screen: &mut dyn Screen,
        keys: &mut dyn CancelKeys,
    ) -> RefineOutcome {
        match self.refinable_over_a_new_session() {
            Ok(changeset) => self.refine_changeset(changeset, timeout, call, screen, keys),
            Err(why) => RefineOutcome::Skipped(why),
        }
    }

    /// The in-TUI wait with everything supplied, and the one behavioural
    /// difference from the pre-TUI path: a refined answer is adopted here
    /// rather than parked, because the sidebar it has to replace is already on
    /// screen.
    pub(crate) fn refine_loaded_with(
        &mut self,
        timeout: Duration,
        call: RefineCall,
        paint: &mut dyn FnMut(&str),
        keys: &mut dyn CancelKeys,
    ) -> RefineOutcome {
        self.refine_wanted = false;
        // A reopened review reaches here — arming is about the human picking a
        // target, not about what the session turned out to hold — and refuses
        // out loud rather than silently.
        if self.refine_over_saved_grouping {
            return RefineOutcome::Skipped(Skipped::AlreadyGrouped);
        }
        let changeset = match self.refinable_over_fresh_heuristics() {
            Ok(changeset) => changeset,
            Err(why) => return RefineOutcome::Skipped(why),
        };
        let files = changeset.len();
        let cancellable = keys.cancellable();
        let outcome = self.refine_changeset(
            changeset,
            timeout,
            call,
            &mut Painted {
                files,
                cancellable,
                paint,
            },
            keys,
        );
        if matches!(outcome, RefineOutcome::Refined { .. }) {
            // Takes `pending_refined`, which outranks the heuristic grouping
            // the load already recorded into the session.
            //
            // `false`, so the reorder re-finds the file the human is on by
            // path: only the partition changed, not which files are in it, and
            // a repartition that snapped the cursor back to the top would undo
            // the position `reload_diff_files` restores.
            self.sort_files_by_directory(false);
            // Group rows are keyed by `GroupId` and the adopted grouping's ids
            // are new, so the seeded set no longer names a single row on
            // screen. Without this every model-made group renders closed.
            self.expand_all_dirs();
            // `line_annotations` is built by walking `diff_files` in order, so
            // the repartition above left every annotation pointing at the file
            // that used to be at its index.
            self.rebuild_annotations();
        }
        outcome
    }

    /// The changeset a refine would run over, or why it must not run at all.
    ///
    /// The pre-TUI form: nothing has grouped this changeset yet, so a session
    /// that already holds a grouping of it is a reopened review and refuses.
    fn refinable_over_a_new_session(&self) -> Result<Changeset, Skipped> {
        let changeset = self.refinable_over_fresh_heuristics()?;
        if self.session.grouping_for(&changeset).is_some() {
            return Err(Skipped::AlreadyGrouped);
        }
        Ok(changeset)
    }

    /// The in-TUI form: the load that just finished recorded its heuristic
    /// grouping into the session, so the check above would refuse every refine
    /// it is there to allow. That path asks the question before the recording
    /// instead, against [`App::refine_over_saved_grouping`].
    fn refinable_over_fresh_heuristics(&self) -> Result<Changeset, Skipped> {
        let changeset = Changeset::from_diff_files(&self.diff_files);
        if changeset.is_empty() {
            return Err(Skipped::NoChangeset);
        }
        Ok(changeset)
    }

    fn refine_changeset(
        &mut self,
        changeset: Changeset,
        timeout: Duration,
        call: RefineCall,
        screen: &mut dyn Screen,
        keys: &mut dyn CancelKeys,
    ) -> RefineOutcome {
        let heuristic = crate::grouping::group_changeset(&changeset, GroupingConfig::default());
        let prompt = refine::prompt(&changeset, &heuristic);

        let outcome = await_refined(&changeset, &heuristic, prompt, timeout, call, screen, keys);
        screen.finish();

        match outcome {
            Waited::Refined(refined) => {
                let repairs = refined.repairs.len();
                self.pending_refined = Some(refined.grouping);
                RefineOutcome::Refined { repairs }
            }
            Waited::Cancelled => RefineOutcome::Cancelled,
            Waited::TimedOut => RefineOutcome::TimedOut,
            Waited::Failed(reason) => RefineOutcome::Failed(reason),
        }
    }
}

/// The shipped call, bound to the arm `[grouping]` resolved.
fn live_call(settings: vertex::Settings) -> RefineCall {
    Arc::new(move |prompt: &str, timeout| vertex::call(prompt, timeout, &settings))
}

enum Waited {
    Refined(Refined),
    Cancelled,
    TimedOut,
    Failed(String),
}

/// The wait proper: dispatch, watch the clock and the keyboard, read the
/// answer, retry once if it was not JSON.
///
/// Nothing partial is ever returned. Application is atomic — a full replacement
/// partition — so there is no half-applied state for a cancel to unwind.
#[allow(clippy::too_many_arguments)]
fn await_refined(
    changeset: &Changeset,
    heuristic: &Grouping,
    prompt: String,
    timeout: Duration,
    call: RefineCall,
    screen: &mut dyn Screen,
    keys: &mut dyn CancelKeys,
) -> Waited {
    // Elapsed is reported from the start of the whole wait, not of the current
    // attempt: a retry doubles what the human sits through, and a clock that
    // restarted would hide exactly that.
    let began = Instant::now();

    for attempt in 1..=ATTEMPTS {
        let (answers, answer) = mpsc::channel();
        let worker = Arc::clone(&call);
        let sent = prompt.clone();
        // Detached on purpose. A cancelled call is abandoned, not joined: the
        // human is waiting on the TUI, not on a socket, and the send fails
        // harmlessly once this receiver is dropped.
        std::thread::spawn(move || {
            let _ = answers.send(worker(&sent, timeout));
        });

        // The retry gets its own full timeout. Accepted cost, stated rather
        // than buried: worst-case blocking startup doubles.
        let deadline = Instant::now() + timeout;

        loop {
            match answer.try_recv() {
                Ok(Ok(body)) => match refine::apply(&body, changeset, heuristic) {
                    Ok(refined) => return Waited::Refined(refined),
                    // The only retryable failure, and only once: a body that
                    // yields no JSON or no `groups` array. Everything else is
                    // terminal on the first attempt, and any *parseable* body
                    // is applied however much it had to be repaired.
                    Err(reason) => {
                        if attempt == ATTEMPTS {
                            return Waited::Failed(reason);
                        }
                        break;
                    }
                },
                Ok(Err(reason)) => return Waited::Failed(reason),
                Err(TryRecvError::Disconnected) => {
                    return Waited::Failed("the refine call ended without answering".to_string());
                }
                Err(TryRecvError::Empty) => {}
            }

            let now = Instant::now();
            if now >= deadline {
                return Waited::TimedOut;
            }
            screen.waiting(began.elapsed(), attempt);
            if keys.cancelled(TICK.min(deadline - now)) {
                return Waited::Cancelled;
            }
        }
    }

    // Unreachable: the loop above either returns or breaks, and the last
    // attempt cannot break. Stated as a failure rather than a panic because a
    // fallback is always safe and a startup crash never is.
    Waited::Failed("the refine call ran out of attempts".to_string())
}

/// The shipped progress surface: one line on stderr, redrawn in place.
///
/// stderr rather than stdout because `--stdout` hands stdout to the export, and
/// a progress line in an exported review is a corrupted export. The sink is a
/// parameter only so the bytes it writes — the retry suffix, the file count, the
/// erase that keeps half a status line from ending up under the TUI — can be
/// read back by a test.
pub struct StatusLine<W: Write = std::io::Stderr> {
    out: W,
    files: usize,
    cancellable: bool,
    drawn: bool,
}

impl StatusLine {
    /// The screen for a wait over `files` files. The count is part of what the
    /// screen owes the human: it is the only thing on it that says why the wait
    /// is as long as it is.
    ///
    /// `cancellable` is whether the keyboard handed to the same wait can read a
    /// key at all, so the line never offers an escape the wait would ignore.
    pub fn over(files: usize, cancellable: bool) -> Self {
        Self::writing_to(files, cancellable, std::io::stderr())
    }
}

impl<W: Write> StatusLine<W> {
    pub(crate) fn writing_to(files: usize, cancellable: bool, out: W) -> Self {
        Self {
            out,
            files,
            cancellable,
            drawn: false,
        }
    }
}

/// One frame of the wait, in words. Shared by both surfaces so the stderr line
/// and the in-TUI overlay cannot drift into telling the human different things.
fn status_text(files: usize, elapsed: Duration, attempt: u32, cancellable: bool) -> String {
    let seconds = elapsed.as_secs();
    // A retry has to be legible or a doubled wait reads as a hang.
    let retry = if attempt > 1 {
        format!(" · retrying ({attempt} of {ATTEMPTS})")
    } else {
        String::new()
    };
    let files = if files > 0 {
        format!("{files} files · ")
    } else {
        String::new()
    };
    // Only offered when the keyboard behind the wait can read it. Raw mode
    // fails on a run with no controlling terminal, and the wait then has its
    // timeout and nothing else.
    let cancel = if cancellable {
        " · esc cancels and opens now"
    } else {
        ""
    };
    format!(
        "Refining groups — {files}{}:{:02} elapsed{retry}{cancel}",
        seconds / 60,
        seconds % 60,
    )
}

impl<W: Write> Screen for StatusLine<W> {
    fn waiting(&mut self, elapsed: Duration, attempt: u32) {
        // `\x1b[2K` clears the line the carriage return is about to overwrite,
        // so a shorter frame cannot leave the tail of a longer one behind.
        let _ = write!(
            self.out,
            "\r\x1b[2K{}",
            status_text(self.files, elapsed, attempt, self.cancellable)
        );
        let _ = self.out.flush();
        self.drawn = true;
    }

    fn finish(&mut self) {
        if !self.drawn {
            return;
        }
        let _ = write!(self.out, "\r\x1b[2K");
        let _ = self.out.flush();
    }
}

/// The in-TUI progress surface: one line handed to a painter, which draws it
/// over the frame the alternate screen is already showing.
///
/// It exists because the stderr line cannot be used once the alternate screen
/// is up: the next `Terminal::draw` paints straight over it, so a wait that ran
/// there would be invisible and a wait a human cannot see is a hang.
///
/// Only frames are painted. **The caller owns the repaint that takes the
/// overlay down**, and the shipped one does it by redrawing the whole frame
/// once the wait returns — which is also the frame that shows the refined
/// grouping, so a wipe here would only cost a flicker.
pub struct Painted<'a> {
    files: usize,
    cancellable: bool,
    paint: &'a mut dyn FnMut(&str),
}

impl Screen for Painted<'_> {
    fn waiting(&mut self, elapsed: Duration, attempt: u32) {
        (self.paint)(&status_text(self.files, elapsed, attempt, self.cancellable));
    }

    fn finish(&mut self) {}
}

/// The real keyboard, in raw mode for the duration of the wait so a keypress
/// arrives without an Enter behind it.
///
/// Raw mode can fail — stdin is not always a terminal, and a run with no
/// controlling terminal (CI, a detached process) is the ordinary case. When it
/// does, the wait keeps its timeout and loses only its cancel key, which is
/// strictly better than refusing to refine.
pub struct TerminalKeys {
    /// Whether keys can be read at all. False when raw mode could not be
    /// entered, which is also the only state the wait degrades in.
    reads: bool,
    /// Whether this handle turned raw mode on and therefore owes a restore.
    /// The in-TUI wait borrows a terminal that is already raw and must leave it
    /// that way — disabling raw mode under a live TUI is a broken session.
    restore: bool,
}

impl TerminalKeys {
    /// The pre-TUI wait, which owns the terminal state it changes.
    fn enter() -> Self {
        let raw = crossterm::terminal::enable_raw_mode().is_ok();
        Self {
            reads: raw,
            restore: raw,
        }
    }

    /// The in-TUI wait, which borrows the alternate screen's raw mode.
    fn borrowed() -> Self {
        Self {
            reads: true,
            restore: false,
        }
    }
}

impl Drop for TerminalKeys {
    fn drop(&mut self) {
        if self.restore {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

impl CancelKeys for TerminalKeys {
    fn cancellable(&self) -> bool {
        self.reads
    }

    fn cancelled(&mut self, within: Duration) -> bool {
        if !self.reads {
            std::thread::sleep(within);
            return false;
        }
        cancelled_via(&mut Crossterm, within)
    }
}

/// The two terminal reads the cancel loop makes, behind a trait so the loop
/// itself can be driven without a terminal.
trait Events {
    fn poll(&mut self, within: Duration) -> std::io::Result<bool>;
    fn read(&mut self) -> std::io::Result<Event>;
}

struct Crossterm;

impl Events for Crossterm {
    fn poll(&mut self, within: Duration) -> std::io::Result<bool> {
        event::poll(within)
    }

    fn read(&mut self) -> std::io::Result<Event> {
        event::read()
    }
}

/// Waits up to `within` for a cancel key, stepping over everything else the
/// terminal had to say.
///
/// Every arm that gives up early consumes the rest of the slice. A poll that
/// keeps reporting an event `read` then fails on — stdin at EOF — would
/// otherwise spin a core for the whole timeout.
fn cancelled_via(events: &mut dyn Events, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let read = match events.poll(left) {
            Ok(true) => events.read(),
            Ok(false) => return false,
            Err(error) => Err(error),
        };
        match read {
            Ok(event) if is_cancel(&event) => return true,
            // Anything else the terminal had to say — a resize, a mouse
            // move — is not a cancel, and the slice is not over.
            Ok(_) => continue,
            Err(_) => {
                std::thread::sleep(left);
                return false;
            }
        }
    }
}

/// Esc, `q` and Ctrl-C. Ctrl-C is here rather than left to the signal because
/// raw mode swallows it, and a wait a human cannot interrupt with Ctrl-C is the
/// hang this screen exists to avoid looking like.
pub(crate) fn is_cancel(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind == KeyEventKind::Release {
        return false;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => true,
        KeyCode::Char('c') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    /// A scripted terminal: what `poll` says, then what `read` hands back.
    struct Scripted {
        events: Vec<std::io::Result<Event>>,
        polls: usize,
    }

    impl Events for Scripted {
        fn poll(&mut self, _within: Duration) -> std::io::Result<bool> {
            self.polls += 1;
            Ok(!self.events.is_empty())
        }

        fn read(&mut self) -> std::io::Result<Event> {
            self.events.remove(0)
        }
    }

    fn press(code: KeyCode) -> std::io::Result<Event> {
        Ok(Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
    }

    #[test]
    fn a_cancel_key_ends_the_wait_and_anything_else_waits_out_its_slice() {
        let mut cancelling = Scripted {
            events: vec![Ok(Event::Resize(80, 24)), press(KeyCode::Esc)],
            polls: 0,
        };
        assert!(cancelled_via(&mut cancelling, Duration::from_millis(50)));
        assert_eq!(cancelling.polls, 2, "the resize did not end the slice");

        let mut quiet = Scripted {
            events: Vec::new(),
            polls: 0,
        };
        assert!(!cancelled_via(&mut quiet, Duration::from_millis(5)));
    }

    /// stdin at EOF polls ready forever and fails every read. The loop has to
    /// sit out the rest of the slice rather than spinning a core through the
    /// whole timeout.
    #[test]
    fn a_terminal_that_polls_ready_and_never_reads_does_not_spin() {
        let mut broken = Scripted {
            events: vec![Err(std::io::Error::other("stdin is gone"))],
            polls: 0,
        };
        let slice = Duration::from_millis(30);
        let began = Instant::now();
        assert!(!cancelled_via(&mut broken, slice));
        assert_eq!(broken.polls, 1);
        assert!(
            began.elapsed() >= slice,
            "the failed read consumed the rest of the slice"
        );
    }
}
