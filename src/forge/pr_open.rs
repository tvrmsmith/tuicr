//! PR open path.
//!
//! Given a `ForgeBackend` and a `PullRequestTarget`, produce the materials
//! the App needs to enter PR review mode: parsed diff files, a session, and
//! a `PrSessionKey` that scopes persistence and remote context fetches.
//!
//! Key invariants enforced here:
//! - The current local checkout is never treated as the source of truth.
//!   File identity comes from forge metadata; SHAs come from PR metadata.
//! - `.tuicrignore` is applied only when the caller supplies a local
//!   checkout path. Outside a checkout, the unfiltered diff is shown.
//! - No checkout mutation. We never spawn `git checkout/fetch/reset/stash`
//!   or branch-creation commands here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    ForgeBackend, ForgeFileLinesRequest, ForgeFileSide, ForgeRepository, PrSessionKey,
    PullRequestCommit, PullRequestDetails, PullRequestInfo, PullRequestReviewMetadata,
    PullRequestTarget,
};
use crate::model::{DiffFile, FilePatch, FileStatus, ReviewSession, SessionDiffSource};
use crate::syntax::SyntaxHighlighter;
use crate::tuicrignore;
use crate::vcs::diff_parser::parse_file_patches;

/// Everything the App needs to enter PR review mode.
#[derive(Debug)]
pub struct OpenedPullRequest {
    pub details: PullRequestDetails,
    pub diff_files: Vec<DiffFile>,
    pub session: ReviewSession,
    pub key: PrSessionKey,
    /// PR commits in newest-first display order. Empty when the forge
    /// returned no commits (or the backend failed and we degraded
    /// gracefully — the cumulative diff stays usable).
    pub commits: Vec<PullRequestCommit>,
    /// Best-effort metadata for detecting commits since the viewer's last
    /// submitted review. Empty when unsupported or unavailable.
    pub review_metadata: PullRequestReviewMetadata,
    /// Extended PR metadata for the description panel.
    pub pr_info: PullRequestInfo,
}

/// Send-safe data fetched before the main thread materializes diff hunks.
pub type PrFetchData = (
    PullRequestDetails,
    Vec<FilePatch>,
    Vec<PullRequestCommit>,
    PullRequestReviewMetadata,
    PullRequestInfo,
    UnreadableMarkdown,
);

/// The markdown files whose text a PR load could not read, in patch order.
/// Each shows as source; [`Self::warning`] tells the reviewer why.
#[derive(Debug, Default)]
pub struct UnreadableMarkdown {
    failures: Vec<MarkdownReadFailure>,
}

#[derive(Debug)]
struct MarkdownReadFailure {
    path: PathBuf,
    error: String,
}

impl UnreadableMarkdown {
    /// The status-bar warning naming the failure, or `None` when every read
    /// succeeded. Several failures share one line, quoting the first error.
    pub fn warning(&self) -> Option<String> {
        match self.failures.as_slice() {
            [] => None,
            [only] => Some(format!(
                "{} shows as source: could not read its text ({})",
                only.path.display(),
                only.error
            )),
            [first, ..] => Some(format!(
                "{} .md files show as source: could not read their text ({})",
                self.failures.len(),
                first.error
            )),
        }
    }
}

/// Open a PR target through a forge backend and prepare review state.
///
/// `local_checkout` is optional: when provided, `.tuicrignore` rules at the
/// root are applied. When absent (PR opened via URL outside a checkout, or
/// for a different repo), no filtering happens.
pub fn open_pull_request(
    backend: &(dyn ForgeBackend + Sync),
    target: PullRequestTarget,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
    markdown_full_text: bool,
) -> Result<(OpenedPullRequest, UnreadableMarkdown)> {
    let (details, patches, commits, review_metadata, pr_info, unreadable) =
        fetch_pr_data(backend, target, markdown_full_text)?;
    let opened = prepare_open_pr(
        details,
        patches,
        commits,
        review_metadata,
        pr_info,
        local_checkout,
        highlighter,
    )?;
    Ok((opened, unreadable))
}

/// Network-only half of the PR open path: fetch PR metadata, structured file
/// patches, and the commit list, and with `markdown_full_text` each markdown
/// file's head text (`attach_markdown_full_texts`). Safe to run on a
/// background thread because it does no syntax parsing and holds nothing that
/// isn't `Send`.
///
/// The commit list is best-effort: if the forge fails on that endpoint
/// only, we still return the diff so PR review proceeds without the
/// inline selector. The first two calls remain required.
pub fn fetch_pr_data(
    backend: &(dyn ForgeBackend + Sync),
    target: PullRequestTarget,
    markdown_full_text: bool,
) -> Result<PrFetchData> {
    let pr_info = backend.get_pull_request_info(target)?;
    let details = pr_info.details.clone();
    let mut patches = backend.get_pull_request_diff(&details)?;
    let unreadable = if markdown_full_text {
        attach_markdown_full_texts(
            backend,
            &details.repository,
            &details.base_sha,
            &details.head_sha,
            &mut patches,
        )
    } else {
        UnreadableMarkdown::default()
    };
    let commits = backend
        .list_pull_request_commits(&details)
        .unwrap_or_default();
    let review_metadata = backend
        .list_pull_request_review_metadata(&details)
        .unwrap_or_default();
    Ok((
        details,
        patches,
        commits,
        review_metadata,
        pr_info,
        unreadable,
    ))
}

/// Network-only half of a PR commit-range reload: the patches between
/// `start_sha` and `end_sha`, and with `markdown_full_text` each markdown
/// file's text at `end_sha`, with the files whose text could not be read.
pub fn fetch_pr_range_patches(
    backend: &(dyn ForgeBackend + Sync),
    details: &PullRequestDetails,
    start_sha: &str,
    end_sha: &str,
    markdown_full_text: bool,
) -> Result<(Vec<FilePatch>, UnreadableMarkdown)> {
    let mut patches = backend.get_pull_request_commit_range_diff(details, start_sha, end_sha)?;
    let unreadable = if markdown_full_text {
        attach_markdown_full_texts(
            backend,
            &details.repository,
            start_sha,
            end_sha,
            &mut patches,
        )
    } else {
        UnreadableMarkdown::default()
    };
    Ok((patches, unreadable))
}

/// How many markdown head texts `attach_markdown_full_texts` reads at once.
const MARKDOWN_FETCH_WORKERS: usize = 4;

/// Reads each changed Modified, Renamed, or Copied markdown patch's text at
/// `head_sha` into `FilePatch::full_text`, leaving the old side `None` for
/// the App to rebuild from the hunks. The forge's base sha is the base branch
/// tip, not the merge base the diff was cut from, so reading it would show
/// source for any PR whose base moved. Added and Deleted files need none:
/// their hunk is the whole file. Patches the backend's checkout ignores are
/// skipped, as `prepare_open_pr` drops them, and nothing is read from a
/// backend that cannot read content (Gerrit). The reads run on up to
/// `MARKDOWN_FETCH_WORKERS` threads. A file whose read fails keeps `None`,
/// shows as source, and is returned in patch order whichever worker
/// finishes first.
fn attach_markdown_full_texts(
    backend: &(dyn ForgeBackend + Sync),
    repository: &ForgeRepository,
    base_sha: &str,
    head_sha: &str,
    patches: &mut [FilePatch],
) -> UnreadableMarkdown {
    if !backend.can_read_file_content() {
        return UnreadableMarkdown::default();
    }
    let mut wanted: Vec<&mut FilePatch> = patches
        .iter_mut()
        .filter(|patch| wants_markdown_full_text(patch))
        .collect();
    if let Some(root) = backend.local_checkout_path() {
        wanted = tuicrignore::filter_file_patches(&root, wanted);
    }
    let per_worker = wanted.len().div_ceil(MARKDOWN_FETCH_WORKERS).max(1);
    let failures = std::thread::scope(|scope| {
        let workers: Vec<_> = wanted
            .chunks_mut(per_worker)
            .map(|group| {
                scope.spawn(move || {
                    group
                        .iter_mut()
                        .filter_map(|patch| {
                            attach_markdown_full_text(
                                backend, repository, base_sha, head_sha, patch,
                            )
                            .err()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    UnreadableMarkdown { failures }
}

fn attach_markdown_full_text(
    backend: &(dyn ForgeBackend + Sync),
    repository: &ForgeRepository,
    base_sha: &str,
    head_sha: &str,
    patch: &mut FilePatch,
) -> std::result::Result<(), MarkdownReadFailure> {
    let Some(path) = ForgeFileLinesRequest::path_for_side(
        ForgeFileSide::Head,
        patch.old_path.as_ref(),
        patch.new_path.as_ref(),
    ) else {
        return Ok(());
    };
    let new = backend
        .fetch_file_content(ForgeFileLinesRequest {
            repository: repository.clone(),
            base_sha: base_sha.to_string(),
            head_sha: head_sha.to_string(),
            path: path.clone(),
            status: patch.status,
            side: ForgeFileSide::Head,
            start_line: 1,
            end_line: u32::MAX,
        })
        .map_err(|error| MarkdownReadFailure {
            path,
            error: error.to_string(),
        })?;
    patch.full_text = crate::vcs::full_text(None, Some(&new)).map(Arc::new);
    Ok(())
}

fn wants_markdown_full_text(patch: &FilePatch) -> bool {
    !matches!(patch.status, FileStatus::Added | FileStatus::Deleted)
        && !patch.is_binary
        && !patch.is_too_large
        && patch.patch.lines().any(|line| line.starts_with("@@ "))
        && patch
            .display_path()
            .is_some_and(crate::syntax::is_markdown_path)
}

/// CPU-only half of the PR open path: apply `.tuicrignore` to the raw
/// patches, then parse the hunks and build the session. Filtering before the
/// parse keeps an ignored large file from being highlighted at all. Runs on
/// the main thread because `SyntaxHighlighter` is not trivially
/// `Send`-cloneable.
pub fn prepare_open_pr(
    details: PullRequestDetails,
    patches: Vec<FilePatch>,
    commits: Vec<PullRequestCommit>,
    review_metadata: PullRequestReviewMetadata,
    pr_info: PullRequestInfo,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
) -> Result<OpenedPullRequest> {
    let had_patches = !patches.is_empty();
    let patches = match local_checkout {
        Some(root) => tuicrignore::filter_file_patches(root, patches),
        None => patches,
    };

    let diff_files = if had_patches && patches.is_empty() {
        Vec::new()
    } else {
        match parse_file_patches(patches, highlighter) {
            Ok(files) => files,
            Err(TuicrError::NoChanges) => {
                return Err(TuicrError::Forge(format!(
                    "Pull request #{} has no file changes",
                    details.number
                )));
            }
            Err(e) => return Err(e),
        }
    };

    let key = PrSessionKey::from_details(&details);
    let session = build_session(&details, &key, &diff_files);
    // Forge returns commits oldest-first; the inline selector renders
    // newest-first so reverse here once.
    let mut commits = commits;
    commits.reverse();

    Ok(OpenedPullRequest {
        details,
        diff_files,
        session,
        key,
        commits,
        review_metadata,
        pr_info,
    })
}

fn build_session(
    details: &PullRequestDetails,
    key: &PrSessionKey,
    diff_files: &[DiffFile],
) -> ReviewSession {
    // The session's repo_path is purely a presentation/identity slot for PR
    // sessions. We use a virtual path so PR sessions don't collide with
    // local sessions stored under the same on-disk repo root.
    let repo_path = pr_session_repo_path(key);
    let branch_name = Some(details.head_ref_name.clone());
    let mut session = ReviewSession::new(
        repo_path,
        details.head_sha.clone(),
        branch_name,
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(key.clone());
    for file in diff_files {
        session.add_diff_file(file);
    }
    session
}

/// Synthetic path used as `ReviewSession::repo_path` for PR sessions.
/// Keeps PR session filenames distinct from local sessions and conveys
/// enough identity (`forge:host/owner/repo`) for humans inspecting the
/// reviews directory.
pub fn pr_session_repo_path(key: &PrSessionKey) -> PathBuf {
    PathBuf::from(format!(
        "forge:{}/{}/{}",
        key.repository.host, key.repository.owner, key.repository.name,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{
        ForgeFileLinesRequest, ForgeRepository, PagedPullRequests, PullRequestDetails,
        PullRequestListQuery,
    };
    use crate::model::DiffLine;
    use chrono::Utc;
    use std::sync::Mutex;

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn details() -> PullRequestDetails {
        PullRequestDetails {
            repository: repo(),
            number: 125,
            title: "Review workflow".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: Some("alice".to_string()),
            head_ref_name: "reviews".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "abcdef0123456789".to_string(),
            base_sha: "1234567890abcdef".to_string(),
            body: "body".to_string(),
            updated_at: Some(Utc::now()),
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    struct StaticBackend {
        details: PullRequestDetails,
        patch: String,
        calls: Mutex<Vec<&'static str>>,
    }

    impl ForgeBackend for StaticBackend {
        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            self.calls.lock().unwrap().push("get_pull_request");
            Ok(self.details.clone())
        }
        fn get_pull_request_diff(&self, _pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
            self.calls.lock().unwrap().push("get_pull_request_diff");
            Ok(crate::vcs::diff_parser::git_fixture_file_patches(
                &self.patch,
            ))
        }
        fn fetch_file_lines(&self, _req: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
            unimplemented!()
        }
        /// Records the call and fails as the trait default does, leaving
        /// `can_read_file_content` at its default.
        fn fetch_file_content(&self, _request: ForgeFileLinesRequest) -> Result<String> {
            self.calls.lock().unwrap().push("fetch_file_content");
            Err(TuicrError::Forge(
                "this backend cannot read file contents".to_string(),
            ))
        }
        fn list_review_threads(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(Vec::new())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<Vec<FilePatch>> {
            Ok(crate::vcs::diff_parser::git_fixture_file_patches(
                &self.patch,
            ))
        }
        fn create_review(
            &self,
            _pr: &PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    const SIMPLE_PATCH: &str = r##"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;

    #[test]
    fn should_parse_pr_diff_and_build_session_keyed_by_head_sha() {
        // given
        let backend = StaticBackend {
            details: details(),
            patch: SIMPLE_PATCH.to_string(),
            calls: Mutex::new(Vec::new()),
        };
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let (opened, _) = open_pull_request(&backend, target, None, &highlighter, false).unwrap();
        // then
        assert_eq!(opened.diff_files.len(), 1);
        assert_eq!(opened.key.head_sha, "abcdef0123456789");
        assert_eq!(opened.key.number, 125);
        assert_eq!(opened.session.diff_source, SessionDiffSource::PullRequest);
        assert_eq!(
            opened.session.pr_session_key.as_ref().map(|k| k.number),
            Some(125),
        );
        assert_eq!(
            opened.session.repo_path,
            PathBuf::from("forge:github.com/agavra/tuicr"),
        );
        // and — both forge calls were made, in order
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            &["get_pull_request", "get_pull_request_diff"],
        );
    }

    /// Patch fixture covering add/modify/delete/rename in a single PR diff.
    /// Tests pair its file blocks with explicit metadata before invoking the
    /// shared hunk parser.
    const MULTI_STATUS_PATCH: &str = r##"diff --git a/added.rs b/added.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/added.rs
@@ -0,0 +1,2 @@
+pub fn new_thing() {}
+
diff --git a/modified.rs b/modified.rs
index 1111111..2222222 100644
--- a/modified.rs
+++ b/modified.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
diff --git a/deleted.rs b/deleted.rs
deleted file mode 100644
index 3333333..0000000
--- a/deleted.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-pub fn gone() {}
-
diff --git a/old_name.rs b/new_name.rs
similarity index 100%
rename from old_name.rs
rename to new_name.rs
"##;

    #[test]
    fn should_parse_multi_status_pr_patch_into_correct_diff_files() {
        // given a backend serving a patch with add/modify/delete/rename
        let backend = StaticBackend {
            details: details(),
            patch: MULTI_STATUS_PATCH.to_string(),
            calls: Mutex::new(Vec::new()),
        };
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let (opened, _) = open_pull_request(&backend, target, None, &highlighter, false).unwrap();
        // then — all four files are recognized with correct statuses
        assert_eq!(opened.diff_files.len(), 4);
        let statuses: Vec<(String, crate::model::FileStatus)> = opened
            .diff_files
            .iter()
            .map(|f| (f.display_path().to_string_lossy().into_owned(), f.status))
            .collect();
        // Order is not guaranteed by the parser, so look up by name.
        let by_name: std::collections::HashMap<_, _> = statuses.into_iter().collect();
        assert_eq!(
            by_name.get("added.rs"),
            Some(&crate::model::FileStatus::Added)
        );
        assert_eq!(
            by_name.get("modified.rs"),
            Some(&crate::model::FileStatus::Modified)
        );
        assert_eq!(
            by_name.get("deleted.rs"),
            Some(&crate::model::FileStatus::Deleted)
        );
        assert_eq!(
            by_name.get("new_name.rs"),
            Some(&crate::model::FileStatus::Renamed)
        );
    }

    #[test]
    fn should_not_read_markdown_text_from_a_backend_that_cannot_read_content() {
        let backend = StaticBackend {
            details: details(),
            patch: format!(
                "diff --git a/docs/guide.md b/docs/guide.md\n\
                 index 1111111..2222222 100644\n\
                 --- a/docs/guide.md\n\
                 +++ b/docs/guide.md\n{GUIDE_PATCH}"
            ),
            calls: Mutex::new(Vec::new()),
        };
        let target = PullRequestTarget::with_repository(repo(), 125, "125");

        let (opened, unreadable) =
            open_pull_request(&backend, target, None, &SyntaxHighlighter::default(), true)
                .expect("open PR");

        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            &["get_pull_request", "get_pull_request_diff"],
        );
        assert_eq!(unreadable.warning(), None);
        assert_eq!(file_at(&opened.diff_files, "docs/guide.md").full_text, None);
    }

    #[test]
    fn should_surface_empty_pr_as_forge_error() {
        // given a PR with no file changes (empty patch)
        let backend = StaticBackend {
            details: details(),
            patch: String::new(),
            calls: Mutex::new(Vec::new()),
        };
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let err = open_pull_request(&backend, target, None, &highlighter, false).unwrap_err();
        // then
        let msg = err.to_string();
        assert!(
            msg.contains("Pull request #125 has no file changes"),
            "unexpected error message: {msg}"
        );
    }

    const GUIDE_HEAD: &str = "# Guide\n\n| a | new |\n";
    const GUIDE_PATCH: &str = "@@ -1,3 +1,3 @@\n # Guide\n \n-| a | old |\n+| a | new |\n";
    const LIB_PATCH: &str = "@@ -1,1 +1,1 @@\n-fn a() {}\n+fn b() {}\n";

    /// Serves a modified markdown file and a modified Rust file, and the
    /// markdown file's head content per sha, recording every `fetch_file_content` request.
    /// Reads of `unavailable` paths fail with `content unavailable`; reads of
    /// paths with no content fail naming the path. A read of `last_to_finish`
    /// waits until every patch's read has started.
    struct ServingBackend {
        patches: Vec<FilePatch>,
        content: std::collections::HashMap<(String, PathBuf), String>,
        unavailable: Vec<PathBuf>,
        last_to_finish: Option<PathBuf>,
        local_checkout: Option<PathBuf>,
        content_requests: Mutex<Vec<(String, PathBuf)>>,
    }

    impl ServingBackend {
        fn new() -> Self {
            let guide = PathBuf::from("docs/guide.md");
            let content = [
                ((details().head_sha, guide.clone()), GUIDE_HEAD.to_string()),
                (("end9".to_string(), guide), GUIDE_HEAD.to_string()),
            ]
            .into_iter()
            .collect();
            Self {
                patches: vec![
                    modified("docs/guide.md", GUIDE_PATCH),
                    modified("src/lib.rs", LIB_PATCH),
                ],
                content,
                unavailable: Vec::new(),
                last_to_finish: None,
                local_checkout: None,
                content_requests: Mutex::new(Vec::new()),
            }
        }

        /// Gives up after a few seconds so a serial reader fails its
        /// assertion instead of hanging.
        fn wait_for_every_read_to_start(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.content_requests.lock().unwrap().len() < self.patches.len()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        fn requested_shas(&self) -> Vec<String> {
            self.content_requests
                .lock()
                .unwrap()
                .iter()
                .map(|(sha, _)| sha.clone())
                .collect()
        }
    }

    fn modified(path: &str, patch: &str) -> FilePatch {
        FilePatch::new(
            Some(PathBuf::from(path)),
            Some(PathBuf::from(path)),
            crate::model::FileStatus::Modified,
            patch,
        )
    }

    impl ForgeBackend for ServingBackend {
        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            Ok(details())
        }
        fn get_pull_request_diff(&self, _pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
            Ok(self.patches.clone())
        }
        fn fetch_file_lines(&self, _req: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
            unimplemented!()
        }
        fn local_checkout_path(&self) -> Option<PathBuf> {
            self.local_checkout.clone()
        }
        fn can_read_file_content(&self) -> bool {
            true
        }
        fn fetch_file_content(&self, request: ForgeFileLinesRequest) -> Result<String> {
            let key = (request.sha().to_string(), request.path.clone());
            self.content_requests.lock().unwrap().push(key.clone());
            if self.last_to_finish.as_ref() == Some(&request.path) {
                self.wait_for_every_read_to_start();
            }
            if self.unavailable.contains(&request.path) {
                return Err(TuicrError::Forge("content unavailable".to_string()));
            }
            self.content
                .get(&key)
                .cloned()
                .ok_or_else(|| TuicrError::Forge(format!("no content for {key:?}")))
        }
        fn list_review_threads(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(Vec::new())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<Vec<FilePatch>> {
            self.get_pull_request_diff(&details())
        }
        fn create_review(
            &self,
            _pr: &PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    fn lines(text: &str) -> Option<Vec<String>> {
        Some(text.lines().map(str::to_string).collect())
    }

    fn file_at<'a>(files: &'a [DiffFile], path: &str) -> &'a DiffFile {
        files
            .iter()
            .find(|file| file.display_path() == Path::new(path))
            .unwrap_or_else(|| panic!("no diff file for {path}"))
    }

    fn open_serving(
        backend: &ServingBackend,
        markdown_full_text: bool,
    ) -> (OpenedPullRequest, UnreadableMarkdown) {
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        open_pull_request(
            backend,
            target,
            None,
            &SyntaxHighlighter::default(),
            markdown_full_text,
        )
        .expect("open PR")
    }

    #[test]
    fn should_attach_only_the_head_text_to_a_modified_markdown_file_on_open() {
        let backend = ServingBackend::new();

        let (opened, _) = open_serving(&backend, true);

        assert_eq!(
            file_at(&opened.diff_files, "docs/guide.md")
                .full_text
                .as_deref(),
            Some(&crate::model::FullText {
                old: None,
                new: lines(GUIDE_HEAD),
            })
        );
        assert_eq!(file_at(&opened.diff_files, "src/lib.rs").full_text, None);
        assert_eq!(
            *backend.content_requests.lock().unwrap(),
            [(
                "abcdef0123456789".to_string(),
                std::path::PathBuf::from("docs/guide.md")
            )]
        );
    }

    #[test]
    fn should_attach_each_markdown_file_its_own_head_text() {
        let page_head = |n: usize| format!("# Page {n}\n\n| a | new |\n");
        let paths: Vec<String> = (0..9).map(|n| format!("docs/page{n}.md")).collect();
        let backend = ServingBackend {
            patches: paths
                .iter()
                .map(|path| modified(path, GUIDE_PATCH))
                .collect(),
            content: paths
                .iter()
                .enumerate()
                .map(|(n, path)| ((details().head_sha, PathBuf::from(path)), page_head(n)))
                .collect(),
            ..ServingBackend::new()
        };

        let (opened, _) = open_serving(&backend, true);

        let attached: Vec<(String, Option<crate::model::FullText>)> = opened
            .diff_files
            .iter()
            .map(|file| {
                (
                    file.display_path().display().to_string(),
                    file.full_text.as_deref().cloned(),
                )
            })
            .collect();
        let expected: Vec<(String, Option<crate::model::FullText>)> = paths
            .iter()
            .enumerate()
            .map(|(n, path)| {
                (
                    path.clone(),
                    Some(crate::model::FullText {
                        old: None,
                        new: lines(&page_head(n)),
                    }),
                )
            })
            .collect();
        assert_eq!(attached, expected);
    }

    #[test]
    fn should_neither_read_nor_warn_when_markdown_full_text_is_off() {
        let backend = ServingBackend {
            unavailable: vec![PathBuf::from("docs/guide.md")],
            ..ServingBackend::new()
        };

        let (opened, unreadable) = open_serving(&backend, false);

        assert_eq!(backend.content_requests.lock().unwrap().len(), 0);
        assert_eq!(unreadable.warning(), None);
        assert_eq!(file_at(&opened.diff_files, "docs/guide.md").full_text, None);
    }

    #[test]
    fn should_open_without_full_text_when_the_content_fetch_fails() {
        let backend = ServingBackend {
            unavailable: vec![PathBuf::from("docs/guide.md")],
            ..ServingBackend::new()
        };

        let (opened, _) = open_serving(&backend, true);

        assert_eq!(backend.requested_shas(), [details().head_sha]);
        assert_eq!(file_at(&opened.diff_files, "docs/guide.md").full_text, None);
    }

    #[test]
    fn should_name_the_markdown_file_whose_text_could_not_be_read() {
        let backend = ServingBackend {
            unavailable: vec![PathBuf::from("docs/guide.md")],
            ..ServingBackend::new()
        };

        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let (.., unreadable) = fetch_pr_data(&backend, target, true).expect("PR data");

        assert_eq!(
            unreadable.warning().as_deref(),
            Some("docs/guide.md shows as source: could not read its text (content unavailable)")
        );
    }

    #[test]
    fn should_count_unreadable_markdown_files_and_quote_the_first_in_patch_order() {
        let paths: Vec<PathBuf> = (0..3)
            .map(|n| PathBuf::from(format!("docs/page{n}.md")))
            .collect();
        let backend = ServingBackend {
            patches: paths
                .iter()
                .map(|path| modified(path.to_str().unwrap(), GUIDE_PATCH))
                .collect(),
            content: std::collections::HashMap::new(),
            unavailable: vec![paths[0].clone()],
            last_to_finish: Some(paths[0].clone()),
            ..ServingBackend::new()
        };

        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let (.., unreadable) = fetch_pr_data(&backend, target, true).expect("PR data");

        assert_eq!(
            unreadable.warning().as_deref(),
            Some("3 .md files show as source: could not read their text (content unavailable)")
        );
    }

    #[test]
    fn should_fetch_markdown_text_only_for_kept_patches_with_hunks() {
        let checkout = tempfile::tempdir().expect("temp dir");
        std::fs::write(checkout.path().join(".tuicrignore"), "drafts/\n")
            .expect("write .tuicrignore");
        let renamed = FilePatch::new(
            Some(PathBuf::from("docs/before.md")),
            Some(PathBuf::from("docs/after.md")),
            crate::model::FileStatus::Renamed,
            "",
        );
        let backend = ServingBackend {
            patches: vec![
                modified("docs/guide.md", GUIDE_PATCH),
                modified("drafts/notes.md", GUIDE_PATCH),
                renamed,
                modified("docs/mode-only.md", ""),
            ],
            local_checkout: Some(checkout.path().to_path_buf()),
            ..ServingBackend::new()
        };

        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        fetch_pr_data(&backend, target, true).expect("PR data");

        assert_eq!(
            *backend.content_requests.lock().unwrap(),
            [(details().head_sha, PathBuf::from("docs/guide.md"))]
        );
    }

    #[test]
    fn should_read_markdown_text_at_the_range_end_for_a_range_diff() {
        let backend = ServingBackend::new();

        let (patches, _) = fetch_pr_range_patches(&backend, &details(), "start5", "end9", true)
            .expect("range patches");

        assert_eq!(backend.requested_shas(), ["end9"]);
        let guide = patches
            .iter()
            .find(|patch| patch.display_path() == Some(Path::new("docs/guide.md")))
            .expect("guide patch");
        assert_eq!(
            guide.full_text.as_deref(),
            Some(&crate::model::FullText {
                old: None,
                new: lines(GUIDE_HEAD),
            })
        );
    }

    #[test]
    fn should_drop_ignored_patches_before_parsing_them() {
        // given a checkout whose .tuicrignore excludes dist/, and a PR diff
        // whose ignored patch body is malformed (a hunk header the parser
        // rejects)
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(dir.path().join(".tuicrignore"), "dist/\n")
            .expect("failed to write .tuicrignore");
        let patches = vec![
            FilePatch::new(
                None,
                Some(std::path::PathBuf::from("src/main.rs")),
                crate::model::FileStatus::Modified,
                "@@ -1,3 +1,3 @@\n pub fn answer() -> u32 {\n-    41\n+    42\n }\n",
            ),
            FilePatch::new(
                None,
                Some(std::path::PathBuf::from("dist/bundle.js")),
                crate::model::FileStatus::Modified,
                "@@not-a-hunk\n+minified one-liner\n",
            ),
        ];
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = prepare_open_pr(
            details(),
            patches,
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details()),
            Some(dir.path()),
            &highlighter,
        )
        .expect("ignored patch must never reach the parser");
        // then only the kept file was parsed into the review
        let kept: Vec<String> = opened
            .diff_files
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();
        assert_eq!(kept, vec!["src/main.rs"]);
    }

    #[test]
    fn should_open_an_empty_review_when_every_pr_patch_is_ignored() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(dir.path().join(".tuicrignore"), "dist/\n")
            .expect("failed to write .tuicrignore");
        let highlighter = SyntaxHighlighter::default();

        let opened = prepare_open_pr(
            details(),
            vec![FilePatch::new(
                None,
                Some(std::path::PathBuf::from("dist/bundle.js")),
                crate::model::FileStatus::Modified,
                "@@ -1 +1 @@\n-old\n+new\n",
            )],
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details()),
            Some(dir.path()),
            &highlighter,
        )
        .expect("an ignored-only PR should open as an empty review");

        assert!(opened.diff_files.is_empty());
    }
}
