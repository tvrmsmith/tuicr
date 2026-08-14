use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use super::comment::Comment;
use super::diff_types::{DiffFile, FileStatus};
use crate::forge::remote_comments::PrCommentsVisibility;
use crate::forge::traits::PrSessionKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearScope {
    CommentsOnly,
    CommentsAndReviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReview {
    pub path: PathBuf,
    pub reviewed: bool,
    pub status: FileStatus,
    pub file_comments: Vec<Comment>,
    pub line_comments: HashMap<u32, Vec<Comment>>,
    #[serde(default)]
    pub reviewed_hunks: BTreeSet<String>,
    #[serde(default)]
    pub content_hash: Option<u64>,
    /// Which group this file was placed in, as a stable opaque id into
    /// [`ReviewSession::groups`] (`docs/REGROUPING_STATE.md`). `None` on a
    /// session saved before grouping existed, and on every session reviewed
    /// with grouping off.
    ///
    /// Review state itself is path-keyed and knows nothing about groups, so a
    /// file that changes group keeps everything it had.
    #[serde(default)]
    pub group_id: Option<String>,
}

/// One row of the session's group table (`docs/REGROUPING_STATE.md`).
///
/// The grouping is a persisted artefact of the session, not a derived view:
/// reopening never regroups, so yesterday's review shows yesterday's groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroup {
    /// Stable identity. What [`FileReview::group_id`] and the sidebar's
    /// group-scoped expansion state key on — never the name, which a regroup
    /// may change while leaving the group intact.
    pub id: String,
    /// Display only, rendered verbatim.
    pub name: String,
    /// Position in reading order.
    pub order: usize,
    /// `heuristics` | `refined` | `incremental` — what produced *this* group. A
    /// partial refine response leaves untouched groups on their old source, so
    /// one session can hold a mix.
    pub source: String,
    /// Set when the group was opened by incremental assignment rather than by
    /// a full pass.
    #[serde(default)]
    pub new_since_full_pass: bool,
}

impl FileReview {
    pub fn new(path: PathBuf, status: FileStatus, content_hash: u64) -> Self {
        Self {
            path,
            reviewed: false,
            status,
            file_comments: Vec::new(),
            line_comments: HashMap::new(),
            reviewed_hunks: BTreeSet::new(),
            content_hash: Some(content_hash),
            group_id: None,
        }
    }

    pub fn comment_count(&self) -> usize {
        self.file_comments.len() + self.line_comments.values().map(|v| v.len()).sum::<usize>()
    }

    pub fn add_file_comment(&mut self, comment: Comment) {
        self.file_comments.push(comment);
    }

    pub fn add_line_comment(&mut self, line: u32, comment: Comment) {
        self.line_comments.entry(line).or_default().push(comment);
    }

    pub fn toggle_hunk_reviewed(&mut self, key: String) -> bool {
        if self.reviewed_hunks.contains(&key) {
            self.reviewed_hunks.remove(&key);
            false
        } else {
            self.reviewed_hunks.insert(key);
            true
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionDiffSource {
    #[default]
    WorkingTree,
    Staged,
    Unstaged,
    StagedAndUnstaged,
    CommitRange,
    WorkingTreeAndCommits,
    StagedUnstagedAndCommits,
    /// Remote pull request review. Per-PR identity lives in
    /// `ReviewSession::pr_session_key`; this variant is a discriminator so
    /// the persistence layer can route to PR-specific filename construction.
    PullRequest,
    /// Whole-repo annotation surface. Every tracked file is shown in
    /// context-only rendering, sourced from `git ls-files`. The persisted
    /// `base_commit` for these sessions starts with `"pristine:"` so the
    /// reload path can match by prefix instead of exact HEAD.
    Pristine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSession {
    pub id: String,
    pub version: String,
    pub repo_path: PathBuf,
    #[serde(default)]
    pub branch_name: Option<String>,
    pub base_commit: String,
    #[serde(default)]
    pub diff_source: SessionDiffSource,
    #[serde(default)]
    pub commit_range: Option<Vec<String>>,
    /// Identity for PR-mode sessions. `None` for local sessions. Default is
    /// `None` so existing local session JSON deserializes unchanged.
    #[serde(default)]
    pub pr_session_key: Option<PrSessionKey>,
    /// Per-session visibility setting for existing remote forge comments.
    /// Only meaningful in PR mode. Defaults to `Unresolved` so a fresh PR
    /// session — or a session saved before this field existed — shows
    /// unresolved threads.
    #[serde(default)]
    pub remote_comments_visibility: PrCommentsVisibility,
    /// Persisted inline commit selector range for PR sessions. Indices
    /// reference the per-head-SHA `pr_commits` list captured at open
    /// time. `None` means "all commits" (or no selector). Older sessions
    /// without this field deserialize as `None`.
    #[serde(default)]
    pub commit_selection_range: Option<(usize, usize)>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// How many times the human has released this session with `:send`.
    /// Monotonic, and the only unambiguous "the batch is ready" signal a
    /// polling agent has: `updated_at` moves on every comment keystroke and
    /// on every idempotent `:w`, so it cannot punctuate a review. `0` means
    /// nothing has been released yet. Old session JSON rehydrates as `0`.
    #[serde(default)]
    pub release_count: u32,
    /// When the most recent `:send` happened, or `None` if the session has
    /// never been released.
    #[serde(default)]
    pub released_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub review_comments: Vec<Comment>,
    pub files: HashMap<PathBuf, FileReview>,
    /// The session's groups, in reading order. Empty on a session saved before
    /// grouping existed, and on every session reviewed with grouping off.
    #[serde(default)]
    pub groups: Vec<SessionGroup>,
    pub session_notes: Option<String>,
}

impl ReviewSession {
    pub fn new(
        repo_path: PathBuf,
        base_commit: String,
        branch_name: Option<String>,
        diff_source: SessionDiffSource,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            version: "1.3".to_string(),
            repo_path,
            branch_name,
            base_commit,
            diff_source,
            commit_range: None,
            pr_session_key: None,
            remote_comments_visibility: PrCommentsVisibility::default(),
            commit_selection_range: None,
            created_at: now,
            updated_at: now,
            release_count: 0,
            released_at: None,
            review_comments: Vec::new(),
            files: HashMap::new(),
            groups: Vec::new(),
            session_notes: None,
        }
    }

    pub fn reviewed_count(&self) -> usize {
        self.files.values().filter(|f| f.reviewed).count()
    }

    pub fn has_reviewed_state(&self) -> bool {
        self.files
            .values()
            .any(|file| file.reviewed || !file.reviewed_hunks.is_empty())
    }

    /// Registers a file in the session. Returns true if the file was previously
    /// reviewed but its content changed, causing reviewed status to be reset.
    pub fn add_file(&mut self, path: PathBuf, status: FileStatus, content_hash: u64) -> bool {
        if let Some(review) = self.files.get_mut(&path) {
            let old_hash = review.content_hash;
            review.content_hash = Some(content_hash);
            let rehashed = old_hash.is_some_and(|old| old != content_hash);
            if rehashed {
                // A content-changed file is reassigned by incremental
                // assignment (`docs/REGROUPING_STATE.md`), and dropping its id
                // here is how it becomes unplaced. Nothing is lost by moving
                // it: the `reviewed` flag it carried is cleared below for the
                // same reason, so a reassignment never relocates something the
                // human had approved. A session saved before hashes existed
                // (`old_hash` unset) is not a content change and keeps its
                // grouping.
                review.group_id = None;
            }
            if review.reviewed && old_hash != Some(content_hash) {
                review.reviewed = false;
                return true;
            }
            return false;
        }
        self.files
            .insert(path.clone(), FileReview::new(path, status, content_hash));
        false
    }

    pub fn add_diff_file(&mut self, file: &DiffFile) -> bool {
        let path = file.display_path().clone();
        let invalidated = self.add_file(path.clone(), file.status, file.content_hash);
        if let Some(review) = self.files.get_mut(&path) {
            let valid_hunks: BTreeSet<_> = file.hunk_review_keys().into_iter().collect();
            review
                .reviewed_hunks
                .retain(|key| valid_hunks.contains(key));
        }
        invalidated
    }

    /// Register a transient filtered diff without dropping hunk keys that
    /// belong to the broader persisted review scope.
    pub fn add_diff_file_preserving_hunks(&mut self, file: &DiffFile) -> bool {
        self.add_file(file.display_path().clone(), file.status, file.content_hash)
    }

    pub fn get_file_mut(&mut self, path: &PathBuf) -> Option<&mut FileReview> {
        self.files.get_mut(path)
    }

    /// Every comment in the session, review-level and per-file, in no
    /// particular order.
    pub fn comments_mut(&mut self) -> impl Iterator<Item = &mut Comment> {
        self.review_comments
            .iter_mut()
            .chain(self.files.values_mut().flat_map(|file| {
                file.file_comments
                    .iter_mut()
                    .chain(file.line_comments.values_mut().flatten())
            }))
    }

    /// Every comment in the session, review-level and per-file, in no
    /// particular order.
    pub fn comments(&self) -> impl Iterator<Item = &Comment> {
        self.review_comments
            .iter()
            .chain(self.files.values().flat_map(|file| {
                file.file_comments
                    .iter()
                    .chain(file.line_comments.values().flatten())
            }))
    }

    /// Comments written since the last release.
    pub fn unreleased_count(&self) -> usize {
        self.comments().filter(|c| !c.is_released()).count()
    }

    /// Publish the current batch: bump `release_count`, stamp every
    /// unreleased comment with the new batch number, and record the time.
    /// Returns how many comments the bump admitted.
    ///
    /// The counter moves even when nothing is unreleased. `:send` means "look
    /// now", and "I am done, no further notes" is a message the human needs to
    /// be able to send after answering an agent's replies — that was the
    /// go-idle-waiting-on-a-human failure the loop experiment recorded.
    pub fn release(&mut self) -> usize {
        self.release_count += 1;
        let batch = self.release_count;
        self.released_at = Some(Utc::now());
        let mut released = 0;
        for comment in self.comments_mut() {
            if comment.released_in.is_none() {
                comment.released_in = Some(batch);
                released += 1;
            }
        }
        released
    }

    pub fn has_comments(&self) -> bool {
        !self.review_comments.is_empty() || self.files.values().any(|f| f.comment_count() > 0)
    }

    pub fn clear_comments(&mut self, scope: ClearScope) -> (usize, usize) {
        let mut cleared = self.review_comments.len();
        let mut unreviewed = 0;
        self.review_comments.clear();
        for file in self.files.values_mut() {
            cleared += file.comment_count();
            file.file_comments.clear();
            file.line_comments.clear();
            if scope == ClearScope::CommentsAndReviewed {
                if file.reviewed || !file.reviewed_hunks.is_empty() {
                    unreviewed += 1;
                }
                file.reviewed = false;
                file.reviewed_hunks.clear();
            }
        }
        (cleared, unreviewed)
    }

    /// Writes a grouping into the session: the group table in reading order,
    /// and each file's group id.
    ///
    /// Files the grouping does not mention keep whatever they had, so a
    /// grouping computed over a narrowed changeset does not blank the ids of
    /// files that are merely out of view.
    pub fn record_grouping(&mut self, grouping: &crate::grouping::Grouping) {
        self.groups = grouping
            .groups()
            .iter()
            .enumerate()
            .map(|(order, group)| SessionGroup {
                id: group.id.as_str().to_string(),
                name: group.name.clone(),
                order,
                source: group.source.as_str().to_string(),
                new_since_full_pass: group.new_since_full_pass,
            })
            .collect();

        for assignment in grouping.assignments() {
            if let Some(review) = self.files.get_mut(&assignment.path) {
                review.group_id = Some(assignment.group_id.as_str().to_string());
            }
        }
    }

    /// Whether the session's grouping already describes *this* changeset, which
    /// is the question "has this review been grouped already?" and therefore the
    /// question "would a refine be a second one?".
    ///
    /// A **majority** of the paths, not all of them and not one of them, and it
    /// has to be tolerant in one direction and strict in the other. Reopening a
    /// session whose working tree moved a little under it must not refine again
    /// (`docs/REGROUPING_STATE.md`: the wait is once per review), so a few
    /// unplaced files cannot make the answer `false`. Switching to a target the
    /// session never grouped must refine, so a couple of paths in common cannot
    /// make it `true`.
    ///
    /// Distinct from [`Self::grouping_for`], which now answers for any drift at
    /// all by placing the files that moved. This is the gate; that is the work.
    pub fn covers(&self, changeset: &crate::grouping::changeset::Changeset) -> bool {
        if self.groups.is_empty() || changeset.is_empty() {
            return false;
        }
        let known: BTreeSet<&str> = self.groups.iter().map(|group| group.id.as_str()).collect();
        let placed = changeset
            .files
            .iter()
            .filter(|file| {
                self.files
                    .get(&PathBuf::from(&file.path))
                    .and_then(|review| review.group_id.as_deref())
                    .is_some_and(|id| known.contains(id))
            })
            .count();
        placed * 2 > changeset.len()
    }

    /// The grouping this session was last saved with, carried onto `changeset`.
    ///
    /// `None` only when the session has never been grouped. Reopening a session
    /// never regroups (`docs/REGROUPING_STATE.md`): a grouping that still
    /// describes the working tree comes back verbatim, group ids and all, and
    /// drift below that level is **incremental assignment**, not a reason to
    /// recompute. Every file that kept its group id stays exactly where it was;
    /// the files that appeared, moved or lost their id to a content change are
    /// placed by [`crate::grouping::assign_incrementally`].
    ///
    /// Recomputing the whole partition over any drift at all — which is what
    /// this did before `gd-26r.33` — is deterministic and free, and it throws
    /// away the one thing incremental assignment exists to keep: a reader who
    /// is working through the groups must not have them renamed and renumbered
    /// because one file appeared.
    pub fn grouping_for(
        &self,
        changeset: &crate::grouping::changeset::Changeset,
    ) -> Option<crate::grouping::Grouping> {
        use crate::grouping::passes::GroupingConfig;
        use crate::grouping::{GroupId, GroupSource, PresentedGroup, assign_incrementally};

        if self.groups.is_empty() {
            return None;
        }

        let known: HashMap<&str, &SessionGroup> = self
            .groups
            .iter()
            .map(|group| (group.id.as_str(), group))
            .collect();

        let mut members: HashMap<&str, Vec<String>> = HashMap::new();
        for file in &changeset.files {
            // A file with no review record, no id, or an id the table no longer
            // holds is simply unplaced. It reaches incremental assignment
            // below, which is the one rule that covers every non-regroup path.
            let Some(group_id) = self
                .files
                .get(&PathBuf::from(&file.path))
                .and_then(|review| review.group_id.as_deref())
                .filter(|id| known.contains_key(id))
            else {
                continue;
            };
            members.entry(group_id).or_default().push(file.path.clone());
        }

        let mut table: Vec<&SessionGroup> = self.groups.iter().collect();
        table.sort_by_key(|group| group.order);

        // A group holding nothing in *this* changeset keeps its order slot and
        // its id while any file in the session still names it — a narrowed
        // commit range must not lose the group its files return to. A group
        // nothing names at all is dead and is dropped, so switching between
        // unrelated ranges cannot silt the table up.
        let named: BTreeSet<&str> = self
            .files
            .values()
            .filter_map(|review| review.group_id.as_deref())
            .collect();

        let kept: Vec<PresentedGroup> = table
            .into_iter()
            .filter(|group| named.contains(group.id.as_str()))
            .map(|group| PresentedGroup {
                id: GroupId::from_persisted(group.id.clone()),
                name: group.name.clone(),
                source: GroupSource::from_persisted(&group.source),
                new_since_full_pass: group.new_since_full_pass,
                members: members.remove(group.id.as_str()).unwrap_or_default(),
            })
            .collect();

        Some(assign_incrementally(
            changeset,
            kept,
            GroupingConfig::default(),
        ))
    }

    pub fn is_file_reviewed(&self, path: &PathBuf) -> bool {
        self.files.get(path).map(|r| r.reviewed).unwrap_or(false)
    }

    pub fn is_hunk_reviewed(&self, path: &PathBuf, key: &str) -> bool {
        self.files
            .get(path)
            .is_some_and(|review| review.reviewed_hunks.contains(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::comment::{Comment, CommentType};
    use crate::model::{DiffHunk, DiffLine, LineOrigin};

    // Arbitrary hash value for tests that don't care about the specific hash.
    const SOME_HASH: u64 = 0xdeadbeef;

    fn test_session() -> ReviewSession {
        ReviewSession::new(
            PathBuf::from("/repo"),
            "abc123".to_string(),
            None,
            SessionDiffSource::WorkingTree,
        )
    }

    fn test_hunk(new_start: u32, content: &str) -> DiffHunk {
        DiffHunk {
            header: format!("@@ -{new_start},1 +{new_start},1 @@"),
            lines: vec![DiffLine {
                origin: LineOrigin::Context,
                content: content.to_string(),
                old_lineno: Some(new_start),
                new_lineno: Some(new_start),
                highlighted_spans: None,
            }],
            old_start: new_start,
            old_count: 1,
            new_start,
            new_count: 1,
        }
    }

    fn test_diff_file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    #[test]
    fn should_return_zero_when_clearing_empty_session() {
        let mut session = test_session();
        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 0);
    }

    #[test]
    fn should_clear_review_level_comments() {
        let mut session = test_session();
        session.review_comments.push(Comment::new(
            "note".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        session.review_comments.push(Comment::new(
            "issue".to_string(),
            CommentType::from_id("issue"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 0);
        assert!(session.review_comments.is_empty());
    }

    #[test]
    fn should_clear_file_and_line_comments() {
        let mut session = test_session();
        let path = PathBuf::from("src/main.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        file.add_line_comment(
            10,
            Comment::new("line".to_string(), CommentType::from_id("note"), None),
        );

        let (cleared, _) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);

        let file = session.files.get(&path).unwrap();
        assert!(file.file_comments.is_empty());
        assert!(file.line_comments.is_empty());
    }

    #[test]
    fn should_reset_reviewed_status_on_all_files() {
        let mut session = test_session();
        let path_a = PathBuf::from("a.rs");
        let path_b = PathBuf::from("b.rs");
        session.add_file(path_a.clone(), FileStatus::Modified, SOME_HASH);
        session.add_file(path_b.clone(), FileStatus::Added, SOME_HASH);

        session.get_file_mut(&path_a).unwrap().reviewed = true;
        session.get_file_mut(&path_b).unwrap().reviewed = true;

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 2);
        assert!(!session.is_file_reviewed(&path_a));
        assert!(!session.is_file_reviewed(&path_b));
    }

    #[test]
    fn should_only_count_reviewed_files_as_unreviewed() {
        let mut session = test_session();
        let reviewed = PathBuf::from("reviewed.rs");
        let pending = PathBuf::from("pending.rs");
        session.add_file(reviewed.clone(), FileStatus::Modified, SOME_HASH);
        session.add_file(pending.clone(), FileStatus::Modified, SOME_HASH);

        session.get_file_mut(&reviewed).unwrap().reviewed = true;

        let (_, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(unreviewed, 1);
    }

    #[test]
    fn should_clear_both_comments_and_reviewed_status() {
        let mut session = test_session();
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.reviewed = true;
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        session.review_comments.push(Comment::new(
            "review".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 1);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_clear_hunk_reviewed_status() {
        let mut session = test_session();
        let file = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = file.display_path().clone();
        let key = file.hunk_review_key(0).unwrap();

        session.add_diff_file(&file);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);

        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 1);
        assert!(!session.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_preserve_reviewed_status_when_requested() {
        let mut session = test_session();
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.reviewed = true;
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        session.review_comments.push(Comment::new(
            "review".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsOnly);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 0);
        assert!(session.is_file_reviewed(&path));
    }

    #[test]
    fn should_store_content_hash_on_new_file() {
        let mut session = test_session();
        let path = PathBuf::from("new.rs");
        session.add_file(path.clone(), FileStatus::Added, 42);

        let file = session.files.get(&path).unwrap();
        assert_eq!(file.content_hash, Some(42));
        assert!(!file.reviewed);
    }

    #[test]
    fn should_keep_reviewed_when_hash_unchanged() {
        let mut session = test_session();
        let path = PathBuf::from("stable.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.get_file_mut(&path).unwrap().reviewed = true;

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 100);
        assert!(!invalidated);
        assert!(session.is_file_reviewed(&path));
    }

    #[test]
    fn should_reset_reviewed_when_hash_changes() {
        let mut session = test_session();
        let path = PathBuf::from("changed.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.get_file_mut(&path).unwrap().reviewed = true;

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 200);
        assert!(invalidated);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_not_report_invalidated_for_unreviewed_file_with_changed_hash() {
        let mut session = test_session();
        let path = PathBuf::from("pending.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 200);
        assert!(!invalidated);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_update_hash_even_when_not_reviewed() {
        let mut session = test_session();
        let path = PathBuf::from("evolving.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.add_file(path.clone(), FileStatus::Modified, 200);

        let file = session.files.get(&path).unwrap();
        assert_eq!(file.content_hash, Some(200));
    }

    /// Snapshot of a session JSON produced before PR 3 landed. New fields
    /// must deserialize with defaults; this guards against accidental
    /// breaking changes to the on-disk format.
    const LEGACY_SESSION_JSON: &str = r##"{
        "id": "abc-uuid",
        "version": "1.2",
        "repo_path": "/tmp/test-repo",
        "branch_name": "main",
        "base_commit": "deadbeef",
        "diff_source": "working_tree",
        "created_at": "2026-05-01T12:00:00Z",
        "updated_at": "2026-05-01T12:00:00Z",
        "review_comments": [],
        "files": {},
        "session_notes": null
    }"##;

    mod release_boundary {
        use super::*;

        /// A session with one review-level comment and one line comment, so a
        /// release has to reach both storage locations.
        fn session_with_two_comments() -> ReviewSession {
            let mut session = test_session();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, 1);
            session.review_comments.push(Comment::new(
                "overall".to_string(),
                CommentType::None,
                None,
            ));
            session.get_file_mut(&path).unwrap().add_line_comment(
                42,
                Comment::new("here".to_string(), CommentType::None, None),
            );
            session
        }

        #[test]
        fn should_stamp_every_unreleased_comment_with_the_new_batch_number() {
            // given
            let mut session = session_with_two_comments();
            assert_eq!(session.unreleased_count(), 2);
            // when
            let released = session.release();
            // then
            assert_eq!(released, 2);
            assert_eq!(session.release_count, 1);
            assert!(session.released_at.is_some());
            assert!(session.comments().all(|c| c.released_in == Some(1)));
            assert_eq!(session.unreleased_count(), 0);
        }

        #[test]
        fn should_leave_earlier_batches_alone_on_a_second_release() {
            // given a released session that gains one more comment
            let mut session = session_with_two_comments();
            session.release();
            session
                .review_comments
                .push(Comment::new("late".to_string(), CommentType::None, None));
            // when
            let released = session.release();
            // then only the new comment joins batch 2
            assert_eq!(released, 1);
            assert_eq!(session.release_count, 2);
            assert_eq!(
                session
                    .comments()
                    .filter(|c| c.released_in == Some(1))
                    .count(),
                2
            );
            assert_eq!(
                session
                    .comments()
                    .filter(|c| c.released_in == Some(2))
                    .count(),
                1
            );
        }

        #[test]
        fn should_bump_the_counter_even_when_nothing_is_unreleased() {
            // given a session with nothing new to say
            let mut session = session_with_two_comments();
            session.release();
            // when the human sends anyway — "that is all, no further notes"
            let released = session.release();
            // then the poller still sees the counter move
            assert_eq!(released, 0);
            assert_eq!(session.release_count, 2);
        }

        #[test]
        fn should_default_release_state_for_legacy_session_json() {
            // given a session JSON predating the release boundary
            let session: ReviewSession = serde_json::from_str(LEGACY_SESSION_JSON).unwrap();
            // then it reads as never released
            assert_eq!(session.release_count, 0);
            assert_eq!(session.released_at, None);
        }
    }

    #[test]
    fn should_deserialize_pre_pr3_session_without_breakage() {
        // given a session JSON from before PR 3 landed
        // when
        let session: ReviewSession =
            serde_json::from_str(LEGACY_SESSION_JSON).expect("legacy session should parse");
        // then — new fields default to None / their default and identity is preserved
        assert_eq!(session.id, "abc-uuid");
        assert_eq!(session.base_commit, "deadbeef");
        assert_eq!(session.diff_source, SessionDiffSource::WorkingTree);
        assert!(session.pr_session_key.is_none());
        assert!(session.commit_range.is_none());
        // Per spec: an older session without remote_comments_visibility
        // defaults to `Unresolved` on read so PR-mode behavior stays sane.
        assert_eq!(
            session.remote_comments_visibility,
            PrCommentsVisibility::Unresolved
        );
    }

    #[test]
    fn should_round_trip_remote_comments_visibility_on_session() {
        // given
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.remote_comments_visibility = PrCommentsVisibility::All;
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(
            restored.remote_comments_visibility,
            PrCommentsVisibility::All
        );
    }

    #[test]
    fn should_round_trip_commit_selection_range_on_pr_session() {
        // given a PR session with a strict-subset commit range selection
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.commit_selection_range = Some((1, 3));
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(restored.commit_selection_range, Some((1, 3)));
    }

    #[test]
    fn should_round_trip_comment_commit_id_on_session() {
        // given a session with a file comment and a line comment both scoped
        // to a single commit
        use crate::model::comment::LineSide;
        let mut session = test_session();
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, SOME_HASH);
        let file_comment = Comment::new(
            "file note on commit aaa".to_string(),
            CommentType::from_id("note"),
            None,
        )
        .with_commit_id("aaa111");
        let line_comment = Comment::new(
            "line note on commit bbb".to_string(),
            CommentType::from_id("issue"),
            Some(LineSide::New),
        )
        .with_commit_id("bbb222");
        let review = session.get_file_mut(&PathBuf::from("src/lib.rs")).unwrap();
        review.add_file_comment(file_comment.clone());
        review.add_line_comment(42, line_comment.clone());

        // when serialized and restored
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();

        // then the commit_id survives the round trip
        let r = restored.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        assert_eq!(r.file_comments.len(), 1);
        assert_eq!(
            r.file_comments[0].commit_id,
            Some("aaa111".to_string()),
            "file comment commit_id must round-trip"
        );
        let line = r.line_comments.get(&42).unwrap();
        assert_eq!(line.len(), 1);
        assert_eq!(
            line[0].commit_id,
            Some("bbb222".to_string()),
            "line comment commit_id must round-trip"
        );
    }

    #[test]
    fn should_default_commit_id_to_none_for_legacy_comment_json() {
        // given a comment JSON saved before commit_id existed
        let json = r#"{
            "id": "legacy-id",
            "content": "old note",
            "comment_type": "note",
            "created_at": "2024-01-01T00:00:00Z",
            "line_context": null,
            "side": null,
            "line_range": null,
            "author": "user",
            "lifecycle_state": "local_draft",
            "remote_review_id": null,
            "remote_comment_id": null
        }"#;
        // when
        let comment: Comment = serde_json::from_str(json).unwrap();
        // then
        assert_eq!(
            comment.commit_id, None,
            "comment JSON without commit_id must default to None"
        );
        assert_eq!(
            comment.in_reply_to, None,
            "comment JSON without in_reply_to must default to None"
        );
    }

    #[test]
    fn should_default_commit_selection_range_to_none_for_legacy_session() {
        // given a session JSON saved before commit_selection_range existed
        // when
        let session: ReviewSession =
            serde_json::from_str(LEGACY_SESSION_JSON).expect("legacy session should parse");
        // then
        assert_eq!(session.commit_selection_range, None);
    }

    #[test]
    fn should_round_trip_pr_session_via_serde() {
        // given
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let key = PrSessionKey::new(
            crate::forge::traits::ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        );
        session.pr_session_key = Some(key.clone());
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(restored.pr_session_key, Some(key));
        assert_eq!(restored.diff_source, SessionDiffSource::PullRequest);
    }

    #[test]
    fn should_default_reviewed_hunks_for_legacy_file_review() {
        let json = r#"{
            "path": "src/main.rs",
            "reviewed": false,
            "status": "modified",
            "file_comments": [],
            "line_comments": {},
            "content_hash": 123
        }"#;

        let review: FileReview = serde_json::from_str(json).unwrap();
        assert!(review.reviewed_hunks.is_empty());
    }

    #[test]
    fn should_roundtrip_reviewed_hunks() {
        let mut session = test_session();
        let file = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = file.display_path().clone();
        let key = file.hunk_review_key(0).unwrap();

        session.add_diff_file(&file);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let json = serde_json::to_string(&session).unwrap();
        let loaded: ReviewSession = serde_json::from_str(&json).unwrap();
        assert!(loaded.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_preserve_reviewed_hunk_when_only_line_numbers_shift() {
        let mut session = test_session();
        let original = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = original.display_path().clone();
        let key = original.hunk_review_key(0).unwrap();

        session.add_diff_file(&original);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let shifted = test_diff_file("src/main.rs", vec![test_hunk(30, "same")]);
        let shifted_key = shifted.hunk_review_key(0).unwrap();
        session.add_diff_file(&shifted);

        assert_eq!(key, shifted_key);
        assert!(session.is_hunk_reviewed(&path, &shifted_key));
    }

    #[test]
    fn should_use_line_aware_keys_for_repeated_identical_hunks() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "same"), test_hunk(20, "same")],
        );
        let path = original.display_path().clone();
        let first_key = original.hunk_review_key(0).unwrap();
        let second_key = original.hunk_review_key(1).unwrap();

        session.add_diff_file(&original);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(first_key.clone());

        let shifted = test_diff_file(
            "src/main.rs",
            vec![test_hunk(30, "same"), test_hunk(40, "same")],
        );
        session.add_diff_file(&shifted);

        assert_ne!(first_key, second_key);
        assert_ne!(first_key, shifted.hunk_review_key(0).unwrap());
        assert_ne!(second_key, shifted.hunk_review_key(1).unwrap());
        assert!(!session.is_hunk_reviewed(&path, &first_key));
        assert!(!session.is_hunk_reviewed(&path, &second_key));
    }

    #[test]
    fn should_not_move_reviewed_status_between_identical_hunks() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![
                test_hunk(10, "same"),
                test_hunk(20, "same"),
                test_hunk(30, "same"),
            ],
        );
        let path = original.display_path().clone();
        let first_key = original.hunk_review_key(0).unwrap();
        let second_key = original.hunk_review_key(1).unwrap();
        let third_key = original.hunk_review_key(2).unwrap();

        session.add_diff_file(&original);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(first_key.clone());
        review.toggle_hunk_reviewed(second_key.clone());

        let updated = test_diff_file(
            "src/main.rs",
            vec![
                test_hunk(10, "same"),
                test_hunk(20, "changed"),
                test_hunk(30, "same"),
            ],
        );
        let updated_first_key = updated.hunk_review_key(0).unwrap();
        let updated_third_key = updated.hunk_review_key(2).unwrap();
        session.add_diff_file(&updated);

        assert_eq!(first_key, updated_first_key);
        assert_eq!(third_key, updated_third_key);
        assert!(session.is_hunk_reviewed(&path, &updated_first_key));
        assert!(!session.is_hunk_reviewed(&path, &updated.hunk_review_key(1).unwrap()));
        assert!(!session.is_hunk_reviewed(&path, &updated_third_key));
    }

    #[test]
    fn should_prune_reviewed_hunks_that_no_longer_exist() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "kept"), test_hunk(20, "removed")],
        );
        let path = original.display_path().clone();
        let kept_key = original.hunk_review_key(0).unwrap();
        let removed_key = original.hunk_review_key(1).unwrap();

        session.add_diff_file(&original);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(kept_key.clone());
        review.toggle_hunk_reviewed(removed_key.clone());

        let updated = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "kept"), test_hunk(30, "new")],
        );
        session.add_diff_file(&updated);

        assert!(session.is_hunk_reviewed(&path, &kept_key));
        assert!(!session.is_hunk_reviewed(&path, &removed_key));
    }

    #[test]
    fn should_preserve_reviewed_hunks_for_transient_diff_views() {
        let mut session = test_session();
        let full = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "first"), test_hunk(20, "second")],
        );
        let path = full.display_path().clone();
        let first_key = full.hunk_review_key(0).unwrap();
        let second_key = full.hunk_review_key(1).unwrap();

        session.add_diff_file(&full);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(first_key.clone());
        review.toggle_hunk_reviewed(second_key.clone());

        let subset = test_diff_file("src/main.rs", vec![test_hunk(10, "first")]);
        session.add_diff_file_preserving_hunks(&subset);

        assert!(session.is_hunk_reviewed(&path, &first_key));
        assert!(session.is_hunk_reviewed(&path, &second_key));
    }

    #[test]
    fn should_reset_reviewed_when_legacy_session_has_no_hash() {
        let mut session = test_session();
        let path = PathBuf::from("legacy.rs");

        // Simulate a legacy session entry without content_hash.
        session.files.insert(
            path.clone(),
            FileReview {
                path: path.clone(),
                reviewed: true,
                status: FileStatus::Modified,
                file_comments: Vec::new(),
                line_comments: HashMap::new(),
                reviewed_hunks: BTreeSet::new(),
                content_hash: None,
                group_id: None,
            },
        );

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 999);
        assert!(invalidated);
        assert!(!session.is_file_reviewed(&path));
        assert_eq!(session.files.get(&path).unwrap().content_hash, Some(999));
    }
}
