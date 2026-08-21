//! The append-only grouping feedback log (`gd-26r.43`): its payload, its path,
//! and the append.
//!
//! One line of compact JSON per submitted vote, at
//! `<data dir>/grouping-feedback.jsonl` — a sibling of the `reviews/` directory
//! sessions live in, never a file in a repo working tree. `docs/GROUPING_FEEDBACK.md`
//! is the schema's prose owner and says what every field means to the offline
//! reader; this module is where the shape itself is decided.
//!
//! Three rulings from `gd-26r.18` are load-bearing here and are not this
//! module's to revisit. **tuicr never reads this file back**: the log improves
//! the refine step through an offline, human-driven loop, and an accumulating
//! hidden state would invalidate `gd-26r.32`'s byte-for-byte prompt parity bar.
//! **A skip writes nothing at all**, not even a bare entry. And **there is no
//! CLI surface**; an agent doing the offline pass reads the documented path.
//!
//! Append only, with no deduplication on write. Voting, then reopening the
//! review a week later and voting again, produces two entries about one review
//! — genuinely different data if a `:regroup` happened between them. The
//! offline pass decides which wins.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TuicrError};
use crate::forge::traits::PrSessionKey;
use crate::grouping::Grouping;
use crate::model::review::SessionDiffSource;
use crate::model::{DiffFile, FileStatus};
use crate::persistence::storage;

/// Bumped when the shape below changes in a way an offline reader has to know
/// about. It versions the *shape*; what pins the meaning of the values — the
/// pass names, the tag list, the size caps and the refine prompt — is
/// [`GroupingFeedbackEntry::tuicr_version`].
pub const SCHEMA_VERSION: u32 = 1;

const LOG_FILENAME: &str = "grouping-feedback.jsonl";
const LOCK_FILENAME: &str = "grouping-feedback.jsonl.lock";

/// One submitted vote, and everything an offline reader needs to understand
/// what it was a vote about.
///
/// Every domain type is mapped into a payload type here rather than being
/// given a `Serialize` derive of its own. `GroupId`'s inner string is private
/// on purpose (`src/grouping/mod.rs`), the engine's `Assignment` is explicitly
/// unpersisted debug state, and a derive on either would make this log's wire
/// shape a hostage to a refactor in the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupingFeedbackEntry {
    pub schema_version: u32,
    /// The build that wrote the line. The single most useful field for an
    /// offline reader: it pins the refine prompt, the pass names, the tag list
    /// and the size caps, all of which shape the snapshot below and none of
    /// which are versioned in themselves.
    pub tuicr_version: String,
    pub recorded_at: DateTime<Utc>,
    pub review: ReviewIdentity,
    /// `useful`, `mixed` or `useless`.
    pub verdict: String,
    /// A subset of `GROUPING_FEEDBACK_TAGS`, in constant order rather than
    /// click order.
    pub tags: Vec<String>,
    pub note: Option<String>,
    pub marks: Marks,
    pub arm: Arm,
    /// Whether the reader had grouping switched on when they voted. A verdict
    /// cast with `<leader>g` off is a verdict on a partition the reader was not
    /// looking at, and `files` is still in the partition's reading order rather
    /// than the directory order on screen.
    pub grouping_enabled: bool,
    /// Whether the review carried the commit-message pseudo-file. It sits
    /// outside the partition by ruling (`docs/TOTAL_COVERAGE.md`) and so appears
    /// nowhere in `files`, which would otherwise make the file count disagree
    /// with the row count the reader saw.
    pub has_commit_message: bool,
    /// **Authoritative for group existence and reading order.** A group holding
    /// no file in this changeset is legal — a narrowed commit range keeps the
    /// group its files return to (`ReviewSession::grouping_for`) — so
    /// reconstructing the partition by grouping `files` alone would silently
    /// drop it and renumber everything after it.
    pub groups: Vec<LoggedGroup>,
    /// Every file the partition placed, in reading order: the groups in turn,
    /// and inside each group the within-group sort. Membership is this array
    /// grouped by `group`, which cannot contradict `groups` because both come
    /// from the one `Grouping`.
    pub files: Vec<LoggedFile>,
    /// Files in the changeset the partition does not place. Normally empty: the
    /// partition is total by construction. A non-empty array means the grouping
    /// had drifted behind the changeset when the vote was cast.
    pub unpartitioned: Vec<UnpartitionedFile>,
}

/// How an offline reader finds the review this vote was about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIdentity {
    pub session_id: String,
    /// `ReviewSession::version`, the session file's own schema version.
    pub session_version: String,
    /// The same slug sessions are keyed by. `None` when it could not be
    /// resolved — deriving a local slug discovers the git repo and reads its
    /// `origin` remote, and a vote must not fail because that did not work.
    pub slug: Option<String>,
    /// Canonicalized where the filesystem allowed it, matching how sessions are
    /// keyed (`relative_path_for_slug`).
    pub repo_path: PathBuf,
    pub branch: Option<String>,
    pub base_commit: String,
    pub commit_range: Option<Vec<String>>,
    pub diff_source: SessionDiffSource,
    /// The identity of a PR review, for which `repo_path` is only a local
    /// checkout and says nothing.
    pub pr: Option<PrSessionKey>,
    /// Landings *since the review was opened* (`:regroup`, a refine answer, the
    /// `gd-26r.36` backstop). A fresh review whose startup refine landed reads
    /// `0`: that wait is not a landing. Non-zero says the snapshot below was
    /// not the only partition the human saw.
    pub landing_count: u32,
}

/// What the reader marked as wrong while they read (`gd-26r.41`), snapshotted
/// when the prompt opened.
///
/// Kept as its own block rather than folded into `files` and `groups`, because
/// a mark is the human's input and outlives what it points at: file marks are
/// path-keyed and survive a landing, so a marked path need not still be in the
/// changeset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marks {
    pub groups: Vec<MarkedGroup>,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkedGroup {
    /// Joins to `LoggedGroup::id` while the group still exists.
    pub id: String,
    /// `None` when a landing dropped the group between the mark and the vote.
    pub name: Option<String>,
}

/// Which arm produced the grouping being voted on.
///
/// One session mixes `heuristics` and `refined` groups (`gd-26r.32`), so
/// without this "the grouping was bad" cannot say whether the complaint is
/// about the model or about the fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arm {
    /// **The config in force when the vote was cast, not the config that
    /// produced the grouping.** Reopening a refined review with
    /// `[grouping].refine = false` reads `refine_enabled: false` beside a
    /// `groups` array full of `refined`; that is not a contradiction.
    pub config_now: ArmConfig,
    /// The refine call whose result is the partition below, or `None` when no
    /// refine call produced it. `None` beside `refined` groups means a grouping
    /// restored from an earlier sitting: the attempt is in-memory state and is
    /// deliberately never persisted to the session file, so reopening a review
    /// loses it. `None` beside `heuristics` groups means refine never ran.
    pub attempt: Option<RefineAttempt>,
    /// A `:regroup` refine call was still in flight when the vote was cast, so
    /// the partition below is about to be replaced by one this entry does not
    /// describe.
    pub refine_in_flight: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmConfig {
    pub refine_enabled: bool,
    /// The model `[grouping]` and the environment resolve to right now. Not
    /// necessarily the model that ran: see [`RefineAttempt::model`].
    pub model: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// A refine call that reached a terminal state and whose result was adopted.
///
/// Written when the grouping it produced is installed, not when the call
/// returns: a parked answer can be dropped before adoption, and an outcome
/// recorded at return time would claim a partition the reader never saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefineAttempt {
    /// The model the call asked for, resolved through the same
    /// environment-then-config-then-default precedence the call itself uses.
    pub model: String,
    /// Reasoning effort. Pinned in source with no config knob (`gd-26r.24`).
    pub effort: String,
    /// FNV-1a of the prompt's invariant text: the instructions, the rules and
    /// the size guidance, with the changeset's own rendering excluded. **This
    /// is the field that groups entries by which prompt was in force**, which
    /// is what the offline loop revises.
    pub prompt_template_fingerprint: String,
    /// FNV-1a of the exact bytes sent. Unique per call, so it identifies the
    /// call and not the prompt.
    pub prompt_fingerprint: String,
    pub prompt_bytes: usize,
    /// 1, or 2 when the first answer was unparseable and the identical prompt
    /// was resent. A call that needed the retry is a materially worse call.
    pub attempts: u32,
    pub outcome: RefineAttemptOutcome,
}

/// How the call ended. Every arm but `landed` means the partition being voted
/// on is the heuristic fallback, which is the distinction the log exists to
/// draw.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RefineAttemptOutcome {
    /// The model's partition, after `repairs` contract violations were repaired
    /// into it (`docs/GROUPS_CONTRACT.md`).
    Landed {
        repairs: usize,
    },
    /// The human did not want to wait.
    Cancelled,
    TimedOut,
    /// Unauthenticated, unreachable, or unparseable on both attempts.
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggedGroup {
    pub id: String,
    pub name: String,
    /// Position in the reading order, which is also this array's order.
    /// Recorded explicitly so a tool that re-sorts the array cannot lose it.
    pub order: usize,
    /// `heuristics`, `refined` or `incremental`.
    pub source: String,
    /// Over the size cap and deliberately left whole (`gd-26r.38`). The cap
    /// itself is not in the payload; `tuicr_version` pins it.
    pub unbounded: bool,
    pub new_since_full_pass: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggedFile {
    /// The diff's display path: the new path of a rename, and the old path of a
    /// deletion, which is therefore a path that no longer exists.
    pub path: PathBuf,
    /// `None` when the grouping placed a path the changeset no longer carries,
    /// which is the same drift `unpartitioned` reports from the other side.
    pub status: Option<FileStatus>,
    /// The source of a rename or a copy. The engine sees a copy as a rename
    /// that left its source behind, so both statuses fill this.
    pub renamed_from: Option<PathBuf>,
    /// Joins to [`LoggedGroup::id`].
    pub group: String,
    /// The heuristic pass that claimed the file, or `restored` for a file the
    /// heuristics never placed — a refined assignment, or one rehydrated from a
    /// session. This is derived debug state the session never keeps
    /// (`Assignment`, `src/grouping/mod.rs`); it is free at vote time because
    /// the grouping is in memory, and it is what says WHY a file landed where
    /// it did.
    pub pass: String,
    /// The group that nearly claimed the file instead, and why
    /// (`docs/GROUPING.md` § Ambiguous assignments).
    pub runner_up: Option<LoggedRunnerUp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggedRunnerUp {
    /// A group *name*, not an id: this is what the engine recorded when it made
    /// the call, and the losing group need not exist in the final partition.
    pub group: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnpartitionedFile {
    pub path: PathBuf,
    pub status: FileStatus,
}

/// Everything the entry is built from. A struct rather than a parameter list
/// so the test surface is a literal and the builder stays pure.
pub struct EntrySources<'a> {
    /// `useful`, `mixed` or `useless`.
    pub verdict: &'a str,
    /// Already in constant order: the prompt resolves them that way.
    pub tags: &'a [&'a str],
    pub note: Option<&'a str>,
    /// `ReviewSession::marked_group_ids`. Names are resolved here against
    /// `grouping`, which is what makes a mark on a group a landing dropped read
    /// back as a bare id.
    pub marked_group_ids: &'a [&'a str],
    /// `ReviewSession::marked_files`.
    pub marked_files: &'a [&'a Path],
    pub arm: Arm,
    pub grouping_enabled: bool,
    /// Built by the caller: the slug and the canonical repo path are I/O.
    pub review: ReviewIdentity,
    pub diff_files: &'a [DiffFile],
    pub grouping: &'a Grouping,
    pub recorded_at: DateTime<Utc>,
}

/// What the diff says about one path, keyed the way the grouping keys it.
struct ChangedFile<'a> {
    status: FileStatus,
    renamed_from: Option<&'a Path>,
}

/// The changeset behind the vote, by display path — the new path of a rename,
/// the old path of a deletion — which is the key the grouping's assignments
/// carry. The commit-message pseudo-file is left out: it sits outside the
/// partition (`docs/TOTAL_COVERAGE.md`), so it belongs in neither `files` nor
/// `unpartitioned`.
fn changed_files(diff_files: &[DiffFile]) -> BTreeMap<&Path, ChangedFile<'_>> {
    diff_files
        .iter()
        .filter(|file| !file.is_commit_message)
        .map(|file| {
            let changed = ChangedFile {
                status: file.status,
                // Keyed on the status, not on `old_path` alone: a modified file
                // can carry one, and that is not provenance.
                renamed_from: match file.status {
                    FileStatus::Renamed | FileStatus::Copied => file.old_path.as_deref(),
                    _ => None,
                },
            };
            (file.display_path().as_path(), changed)
        })
        .collect()
}

impl GroupingFeedbackEntry {
    /// Pure. No I/O: the caller resolves the slug and canonicalizes the repo
    /// path, both of which touch the filesystem.
    pub fn build(sources: EntrySources<'_>) -> Self {
        let changed = changed_files(sources.diff_files);
        let placed: BTreeSet<&Path> = sources
            .grouping
            .assignments()
            .iter()
            .map(|assignment| assignment.path.as_path())
            .collect();
        Self {
            schema_version: SCHEMA_VERSION,
            tuicr_version: env!("CARGO_PKG_VERSION").to_string(),
            recorded_at: sources.recorded_at,
            review: sources.review,
            verdict: sources.verdict.to_string(),
            tags: sources.tags.iter().map(|tag| (*tag).to_string()).collect(),
            note: sources.note.map(str::to_string),
            marks: Marks {
                groups: sources
                    .marked_group_ids
                    .iter()
                    .map(|id| MarkedGroup {
                        id: (*id).to_string(),
                        name: sources
                            .grouping
                            .groups()
                            .iter()
                            .find(|group| group.id.as_str() == *id)
                            .map(|group| group.name.clone()),
                    })
                    .collect(),
                files: sources
                    .marked_files
                    .iter()
                    .map(|path| path.to_path_buf())
                    .collect(),
            },
            arm: sources.arm,
            grouping_enabled: sources.grouping_enabled,
            has_commit_message: sources.diff_files.iter().any(|file| file.is_commit_message),
            groups: sources
                .grouping
                .groups()
                .iter()
                .enumerate()
                .map(|(order, group)| LoggedGroup {
                    id: group.id.as_str().to_string(),
                    name: group.name.clone(),
                    order,
                    source: group.source.as_str().to_string(),
                    unbounded: group.unbounded,
                    new_since_full_pass: group.new_since_full_pass,
                })
                .collect(),
            files: sources
                .grouping
                .assignments()
                .iter()
                .map(|assignment| LoggedFile {
                    path: assignment.path.clone(),
                    status: changed
                        .get(assignment.path.as_path())
                        .map(|file| file.status),
                    renamed_from: changed
                        .get(assignment.path.as_path())
                        .and_then(|file| file.renamed_from)
                        .map(Path::to_path_buf),
                    group: assignment.group_id.as_str().to_string(),
                    pass: assignment.pass.to_string(),
                    runner_up: assignment
                        .runner_up
                        .as_ref()
                        .map(|runner_up| LoggedRunnerUp {
                            group: runner_up.group.clone(),
                            reason: runner_up.reason.clone(),
                        }),
                })
                .collect(),
            unpartitioned: changed
                .iter()
                .filter(|(path, _)| !placed.contains(*path))
                .map(|(path, file)| UnpartitionedFile {
                    path: path.to_path_buf(),
                    status: file.status,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_FEEDBACK_LOG: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn set_test_feedback_log(path: Option<PathBuf>) {
    TEST_FEEDBACK_LOG.with(|cell| *cell.borrow_mut() = path);
}

/// Points the log at a temp path of its own and takes the whole directory with
/// it when it drops, following `storage`'s `TestReviewsDirGuard`. Lives out
/// here rather than in this module's tests because the app-level tests of the
/// submit path need the same isolation.
#[cfg(test)]
pub(crate) struct TestFeedbackLogGuard {
    dir: PathBuf,
    pub(crate) path: PathBuf,
}

#[cfg(test)]
impl Drop for TestFeedbackLogGuard {
    fn drop(&mut self) {
        set_test_feedback_log(None);
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The directory is deliberately not created: the append owns that, and a test
/// that asserts a skip wrote nothing needs to see its absence.
#[cfg(test)]
pub(crate) fn with_test_feedback_log() -> TestFeedbackLogGuard {
    let dir = std::env::temp_dir().join(format!("tuicr-feedback-test-{}", uuid::Uuid::new_v4()));
    let path = dir.join(LOG_FILENAME);
    set_test_feedback_log(Some(path.clone()));
    TestFeedbackLogGuard { dir, path }
}

/// The one seam on where the log lives. [`append`] takes no path parameter, so
/// no caller can put a vote anywhere but here.
fn feedback_log_path() -> Result<PathBuf> {
    #[cfg(test)]
    {
        // Mirrors `storage::get_reviews_dir`: a thread-local so two parallel
        // tests never share a log, set by tests that care about isolation and
        // falling back to a per-thread temp path for a test that reaches the
        // append incidentally. The real data directory is never used in test
        // mode.
        let configured = TEST_FEEDBACK_LOG.with(|cell| cell.borrow().clone());
        if let Some(path) = configured {
            return Ok(path);
        }
        let thread_id = std::thread::current().id();
        Ok(std::env::temp_dir()
            .join(format!(
                "tuicr-test-thread-{:?}-{}",
                thread_id,
                std::process::id()
            ))
            .join(LOG_FILENAME))
    }

    #[cfg(not(test))]
    {
        Ok(storage::data_dir()?.join(LOG_FILENAME))
    }
}

/// Appends one line. Creates the parent directory. Never truncates.
///
/// Locked on a sibling of the log, because two tuicr processes voting at once
/// must not interleave a torn line: an entry runs to tens of kilobytes, well
/// past the size any single `write` is atomic at.
pub(crate) fn append(entry: &GroupingFeedbackEntry) -> Result<()> {
    let path = feedback_log_path()?;
    let parent = path.parent().ok_or_else(|| {
        TuicrError::Io(std::io::Error::other(format!(
            "feedback log path has no parent: {}",
            path.display()
        )))
    })?;
    fs::create_dir_all(parent)?;
    let _lock = storage::acquire_file_lock(&parent.join(LOCK_FILENAME))?;

    // Compact, and written in one call: one entry is one line.
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grouping::changeset::Changeset;
    use crate::grouping::passes::PassClaim;
    use crate::grouping::{GroupId, GroupSource, Grouping, PresentedGroup, RunnerUp};
    use crate::model::ReviewSession;

    fn diff_file(path: &str, status: FileStatus) -> DiffFile {
        let path = PathBuf::from(path);
        let (old_path, new_path) = match status {
            FileStatus::Deleted => (Some(path), None),
            _ => (None, Some(path)),
        };
        DiffFile {
            old_path,
            new_path,
            status,
            hunks: Vec::new(),
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    /// A grouping restored from the given group table, so the reading order is
    /// exactly the order written here rather than one the passes chose.
    fn restored(diff_files: &[DiffFile], groups: &[(&str, &[&str])]) -> Grouping {
        let changeset = Changeset::from_diff_files(diff_files);
        let presented = groups
            .iter()
            .map(|(name, members)| PresentedGroup {
                id: GroupId::from_persisted(format!("id-{name}")),
                name: (*name).to_string(),
                source: GroupSource::Heuristics,
                new_since_full_pass: false,
                unbounded: false,
                members: members.iter().map(|path| (*path).to_string()).collect(),
            })
            .collect();
        Grouping::restore(&changeset, presented)
    }

    fn sources<'a>(
        session: &'a ReviewSession,
        diff_files: &'a [DiffFile],
        grouping: &'a Grouping,
    ) -> EntrySources<'a> {
        EntrySources {
            verdict: "useful",
            tags: &[],
            note: None,
            marked_group_ids: &[],
            marked_files: &[],
            arm: Arm {
                config_now: ArmConfig {
                    refine_enabled: false,
                    model: None,
                    timeout_ms: None,
                },
                attempt: None,
                refine_in_flight: false,
            },
            grouping_enabled: true,
            review: identity(session),
            diff_files,
            grouping,
            recorded_at: Utc::now(),
        }
    }

    fn identity(session: &ReviewSession) -> ReviewIdentity {
        ReviewIdentity {
            session_id: session.id.clone(),
            session_version: session.version.clone(),
            slug: None,
            repo_path: session.repo_path.clone(),
            branch: session.branch_name.clone(),
            base_commit: session.base_commit.clone(),
            commit_range: session.commit_range.clone(),
            diff_source: session.diff_source,
            pr: session.pr_session_key.clone(),
            landing_count: session.landing_count,
        }
    }

    fn session() -> ReviewSession {
        ReviewSession::new(
            PathBuf::from("/repo"),
            "base".to_string(),
            Some("branch".to_string()),
            SessionDiffSource::WorkingTree,
        )
    }

    fn paths(entry: &GroupingFeedbackEntry) -> Vec<&str> {
        entry
            .files
            .iter()
            .map(|file| file.path.to_str().unwrap())
            .collect()
    }

    #[test]
    fn should_log_files_in_grouping_reading_order_not_diff_order() {
        let diff_files = vec![
            diff_file("src/a.rs", FileStatus::Modified),
            diff_file("src/b.rs", FileStatus::Modified),
        ];
        let grouping = restored(
            &diff_files,
            &[("second", &["src/b.rs"]), ("first", &["src/a.rs"])],
        );
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert_eq!(paths(&entry), vec!["src/b.rs", "src/a.rs"]);
    }

    #[test]
    fn should_flag_the_commit_message_and_keep_it_out_of_the_partition() {
        let mut commit_message = diff_file("COMMIT_MSG", FileStatus::Modified);
        commit_message.is_commit_message = true;
        let diff_files = vec![commit_message, diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert!(entry.has_commit_message);
        assert_eq!(paths(&entry), vec!["src/a.rs"]);
        assert!(entry.unpartitioned.is_empty());
    }

    #[test]
    fn should_log_an_empty_group_with_its_order() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("empty", &[]), ("populated", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let orders: Vec<(&str, usize)> = entry
            .groups
            .iter()
            .map(|group| (group.name.as_str(), group.order))
            .collect();
        assert_eq!(orders, vec![("empty", 0), ("populated", 1)]);
        assert_eq!(paths(&entry), vec!["src/a.rs"]);
    }

    #[test]
    fn should_log_each_group_with_its_id_source_and_markers() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let changeset = Changeset::from_diff_files(&diff_files);
        let grouping = Grouping::restore(
            &changeset,
            vec![PresentedGroup {
                id: GroupId::from_persisted("id-only".to_string()),
                name: "only".to_string(),
                source: GroupSource::Refined,
                new_since_full_pass: true,
                unbounded: true,
                members: vec!["src/a.rs".to_string()],
            }],
        );
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let group = &entry.groups[0];
        assert_eq!(group.id, "id-only");
        assert_eq!(group.source, "refined");
        assert!(group.unbounded);
        assert!(group.new_since_full_pass);
        assert_eq!(entry.files[0].group, "id-only");
    }

    /// A diff entry that reports an old path alongside `status`.
    fn moved_file(old: &str, new: &str, status: FileStatus) -> DiffFile {
        DiffFile {
            old_path: Some(PathBuf::from(old)),
            ..diff_file(new, status)
        }
    }

    fn logged<'a>(entry: &'a GroupingFeedbackEntry, path: &str) -> &'a LoggedFile {
        entry
            .files
            .iter()
            .find(|file| file.path.as_path() == Path::new(path))
            .unwrap_or_else(|| panic!("{path} is not in the logged files"))
    }

    #[test]
    fn should_log_a_rename_under_its_new_path_with_its_source() {
        let diff_files = vec![moved_file("src/old.rs", "src/new.rs", FileStatus::Renamed)];
        let grouping = restored(&diff_files, &[("only", &["src/new.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let file = logged(&entry, "src/new.rs");
        assert_eq!(file.status, Some(FileStatus::Renamed));
        assert_eq!(file.renamed_from, Some(PathBuf::from("src/old.rs")));
    }

    #[test]
    fn should_log_a_copy_with_its_source() {
        let diff_files = vec![moved_file("src/from.rs", "src/to.rs", FileStatus::Copied)];
        let grouping = restored(&diff_files, &[("only", &["src/to.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let file = logged(&entry, "src/to.rs");
        assert_eq!(file.status, Some(FileStatus::Copied));
        assert_eq!(file.renamed_from, Some(PathBuf::from("src/from.rs")));
    }

    #[test]
    fn should_ignore_an_old_path_on_a_file_that_was_not_renamed() {
        let diff_files = vec![moved_file("src/old.rs", "src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert_eq!(logged(&entry, "src/a.rs").renamed_from, None);
    }

    #[test]
    fn should_log_a_deletion_under_the_path_that_no_longer_exists() {
        let diff_files = vec![diff_file("src/gone.rs", FileStatus::Deleted)];
        let grouping = restored(&diff_files, &[("only", &["src/gone.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let file = logged(&entry, "src/gone.rs");
        assert_eq!(file.status, Some(FileStatus::Deleted));
    }

    #[test]
    fn should_log_a_placed_path_the_changeset_lost_with_no_status() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs", "src/stale.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert_eq!(logged(&entry, "src/stale.rs").status, None);
    }

    #[test]
    fn should_report_a_changeset_path_the_partition_never_placed() {
        let diff_files = vec![
            diff_file("src/a.rs", FileStatus::Modified),
            diff_file("src/loose.rs", FileStatus::Added),
        ];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert_eq!(paths(&entry), vec!["src/a.rs"]);
        assert_eq!(entry.unpartitioned.len(), 1);
        assert_eq!(entry.unpartitioned[0].path, PathBuf::from("src/loose.rs"));
        assert_eq!(entry.unpartitioned[0].status, FileStatus::Added);
    }

    #[test]
    fn should_carry_the_claiming_pass_and_the_runner_up_per_file() {
        let diff_files = vec![
            diff_file("src/a.rs", FileStatus::Modified),
            diff_file("src/b.rs", FileStatus::Modified),
        ];
        let changeset = Changeset::from_diff_files(&diff_files);
        let grouping = Grouping::present(
            &changeset,
            vec![
                PassClaim {
                    path: "src/a.rs".to_string(),
                    group: "alpha".to_string(),
                    pass: "cluster",
                    runner_up: None,
                },
                PassClaim {
                    path: "src/b.rs".to_string(),
                    group: "beta".to_string(),
                    pass: "directory",
                    runner_up: Some(RunnerUp {
                        group: "alpha".to_string(),
                        reason: "shares a filename token".to_string(),
                    }),
                },
            ],
            GroupSource::Heuristics,
        );
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        let unambiguous = logged(&entry, "src/a.rs");
        assert_eq!(unambiguous.pass, "cluster");
        assert!(unambiguous.runner_up.is_none());
        let ambiguous = logged(&entry, "src/b.rs");
        assert_eq!(ambiguous.pass, "directory");
        let runner_up = ambiguous.runner_up.as_ref().unwrap();
        assert_eq!(runner_up.group, "alpha");
        assert_eq!(runner_up.reason, "shares a filename token");
    }

    #[test]
    fn should_record_tags_in_the_order_given_and_the_note_verbatim() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(EntrySources {
            tags: &["too coarse", "bad group names"],
            note: Some("  the split down the middle was wrong  "),
            ..sources(&session, &diff_files, &grouping)
        });

        assert_eq!(entry.tags, vec!["too coarse", "bad group names"]);
        assert_eq!(
            entry.note.as_deref(),
            Some("  the split down the middle was wrong  ")
        );
    }

    #[test]
    fn should_stamp_the_schema_and_build_versions() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert_eq!(entry.schema_version, SCHEMA_VERSION);
        assert_eq!(entry.tuicr_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn should_name_a_marked_group_that_survived_and_leave_a_dropped_one_bare() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(EntrySources {
            marked_group_ids: &["id-only", "id-gone"],
            marked_files: &[Path::new("src/a.rs")],
            ..sources(&session, &diff_files, &grouping)
        });

        assert_eq!(entry.marks.groups[0].id, "id-only");
        assert_eq!(entry.marks.groups[0].name.as_deref(), Some("only"));
        assert_eq!(entry.marks.groups[1].id, "id-gone");
        assert_eq!(entry.marks.groups[1].name, None);
        assert_eq!(entry.marks.files, vec![PathBuf::from("src/a.rs")]);
    }

    #[test]
    fn should_report_no_commit_message_when_the_diff_carries_none() {
        let diff_files = vec![diff_file("src/a.rs", FileStatus::Modified)];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs"])]);
        let session = session();

        let entry = GroupingFeedbackEntry::build(sources(&session, &diff_files, &grouping));

        assert!(!entry.has_commit_message);
    }

    // ---------- The writer ----------

    fn lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn an_entry(note: Option<&str>) -> GroupingFeedbackEntry {
        let diff_files = vec![
            diff_file("src/a.rs", FileStatus::Modified),
            diff_file("src/b.rs", FileStatus::Added),
        ];
        let grouping = restored(&diff_files, &[("only", &["src/a.rs", "src/b.rs"])]);
        let session = session();
        GroupingFeedbackEntry::build(EntrySources {
            verdict: "mixed",
            tags: &["too coarse"],
            note,
            marked_group_ids: &["id-only"],
            marked_files: &[Path::new("src/b.rs")],
            ..sources(&session, &diff_files, &grouping)
        })
    }

    #[test]
    fn should_append_rather_than_truncate() {
        let guard = with_test_feedback_log();

        append(&an_entry(Some("first"))).unwrap();
        let after_one = lines(&guard.path);
        append(&an_entry(Some("second"))).unwrap();
        let after_two = lines(&guard.path);

        assert_eq!(after_one.len(), 1);
        assert_eq!(after_two.len(), 2);
        assert_eq!(after_two[0], after_one[0]);
        for line in &after_two {
            serde_json::from_str::<GroupingFeedbackEntry>(line).unwrap();
        }
    }

    #[test]
    fn should_keep_a_note_with_a_newline_on_one_line() {
        let guard = with_test_feedback_log();

        append(&an_entry(Some("line one\nline two"))).unwrap();

        let written = lines(&guard.path);
        assert_eq!(written.len(), 1);
        let read_back: GroupingFeedbackEntry = serde_json::from_str(&written[0]).unwrap();
        assert_eq!(read_back.note.as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn should_create_the_parent_directory() {
        let guard = with_test_feedback_log();
        assert!(!guard.dir.exists());

        append(&an_entry(None)).unwrap();

        assert!(guard.path.is_file());
    }

    #[test]
    fn should_round_trip_an_entry_through_the_log() {
        let guard = with_test_feedback_log();
        let written = an_entry(Some("the tests were split off"));

        append(&written).unwrap();

        let read_back: GroupingFeedbackEntry =
            serde_json::from_str(&lines(&guard.path)[0]).unwrap();
        assert_eq!(read_back.verdict, written.verdict);
        assert_eq!(read_back.tags, written.tags);
        assert_eq!(read_back.note, written.note);
        assert_eq!(read_back.marks.files, written.marks.files);
        assert_eq!(
            read_back
                .marks
                .groups
                .iter()
                .map(|group| (group.id.as_str(), group.name.as_deref()))
                .collect::<Vec<_>>(),
            written
                .marks
                .groups
                .iter()
                .map(|group| (group.id.as_str(), group.name.as_deref()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read_back.arm.config_now.refine_enabled,
            written.arm.config_now.refine_enabled
        );
        assert!(read_back.arm.attempt.is_none());
        assert!(!read_back.arm.refine_in_flight);
        assert_eq!(
            read_back
                .groups
                .iter()
                .map(|group| (group.id.as_str(), group.name.as_str(), group.order))
                .collect::<Vec<_>>(),
            written
                .groups
                .iter()
                .map(|group| (group.id.as_str(), group.name.as_str(), group.order))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read_back
                .files
                .iter()
                .map(|file| (
                    file.path.as_path(),
                    file.status,
                    file.group.as_str(),
                    file.pass.as_str()
                ))
                .collect::<Vec<_>>(),
            written
                .files
                .iter()
                .map(|file| (
                    file.path.as_path(),
                    file.status,
                    file.group.as_str(),
                    file.pass.as_str()
                ))
                .collect::<Vec<_>>()
        );
    }
}
