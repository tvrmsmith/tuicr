//! VCS abstraction layer for supporting multiple version control systems.
//!
//! Currently supports:
//! - Git
//! - Mercurial
//! - Jujutsu
//!
//! ## Detection Order
//!
//! When auto-detecting the VCS type, Jujutsu is tried first because jj repos
//! are Git-backed and contain a `.git` directory. If jj detection fails, Git
//! is tried next, then Mercurial.

pub(crate) mod diff_parser;
pub mod file;
pub mod git;
mod hg;
mod jj;
pub mod pr_noop;
pub mod pristine;
pub(crate) mod traits;
pub(crate) mod whitespace;

pub use file::FileBackend;
pub use git::{GitBackend, GitBackendPreference};
pub use hg::HgBackend;
pub use jj::JjBackend;
pub use pr_noop::PrNoopVcs;
pub use traits::{
    ChangeKind, CommitInfo, DiffWhitespaceMode, ResolvedRevisionRange, RevisionDiffTarget,
    VcsBackend, VcsChangeStatus, VcsInfo, WhitespaceAutoPolicy,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus, FullText, LineOrigin, LineSide};
use crate::syntax::{
    HighlightedLines, HighlightedSpans, SyntaxHighlighter, is_markdown_path,
    needs_full_file_highlight,
};

/// Boundary marker emitted between files in batched `hg cat` / `jj file show`
/// output. The long random suffix makes accidental collision with real source
/// content effectively impossible.
pub(crate) const BATCH_BOUNDARY: &str = "@@TUICR_BATCH_BOUNDARY_e97f2d44_8b1a@@";

/// Collect the paths, on the given side, of the files the full-file pass
/// reads: container-grammar files (Vue, Svelte, PHP and friends) and, with
/// `markdown_full_text`, markdown files. Used by the CLI backends to know
/// which files to batch-fetch.
pub(crate) fn container_file_paths(
    files: &[DiffFile],
    side: LineSide,
    markdown_full_text: bool,
) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|f| wants_full_file(f, markdown_full_text))
        .filter_map(|f| match side {
            LineSide::Old => f.old_path.clone(),
            LineSide::New => f.new_path.clone(),
        })
        .collect()
}

fn syntax_path(file: &DiffFile) -> Option<&Path> {
    file.new_path.as_deref().or(file.old_path.as_deref())
}

/// Whether the full-file pass re-highlights `file` from its whole content.
fn needs_container_highlight(file: &DiffFile) -> bool {
    syntax_path(file).is_some_and(needs_full_file_highlight)
}

/// Whether the full-file pass attaches `file`'s `full_text`. Added and
/// Deleted files render from their hunks, which are the whole file.
fn needs_full_text(file: &DiffFile, markdown_full_text: bool) -> bool {
    markdown_full_text
        && !matches!(file.status, FileStatus::Added | FileStatus::Deleted)
        && syntax_path(file).is_some_and(is_markdown_path)
}

/// Whether the full-file pass reads `file` at all. Binary, too-large, and
/// hunkless entries never are.
fn wants_full_file(file: &DiffFile, markdown_full_text: bool) -> bool {
    !file.is_binary
        && !file.is_too_large
        && !file.hunks.is_empty()
        && (needs_container_highlight(file) || needs_full_text(file, markdown_full_text))
}

/// Expand tabs to spaces in diff line content so highlighted spans line up
/// with the displayed text in side-by-side and unified rendering.
pub(crate) fn tabify(s: &str) -> String {
    s.replace('\t', "    ")
}

/// Slice `[start_line, end_line]` (1-indexed, inclusive) into `DiffLine`s.
pub(crate) fn slice_context_lines(content: &str, start_line: u32, end_line: u32) -> Vec<DiffLine> {
    if start_line > end_line || start_line == 0 {
        return Vec::new();
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    for line_num in start_line..=end_line {
        let idx = (line_num - 1) as usize;
        if idx >= lines.len() {
            break;
        }
        result.push(DiffLine {
            origin: LineOrigin::Context,
            content: tabify(lines[idx]),
            old_lineno: Some(line_num),
            new_lineno: Some(line_num),
            highlighted_spans: None,
        });
    }
    result
}

/// Read a file from the working tree, returning `None` on any IO error.
pub(crate) fn read_workdir_file(root: &Path, rel: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// Parse the output of a batched `hg cat` / `jj file show` invocation whose
/// template prefixed each file with `\n{BATCH_BOUNDARY}\n{path}\n` before
/// emitting `{data}`. Returns a `path → data` map.
pub(crate) fn parse_batched_files(output: &str) -> HashMap<PathBuf, String> {
    let sep = format!("\n{BATCH_BOUNDARY}\n");
    output
        .split(&sep)
        .filter(|s| !s.is_empty())
        .filter_map(|block| {
            let mut iter = block.splitn(2, '\n');
            let path = iter.next()?;
            let data = iter.next().unwrap_or("");
            Some((PathBuf::from(path), data.to_string()))
        })
        .collect()
}

/// Run the full-file pass (`enhance_with_full_file_highlight`) using the
/// files' full content at the requested revisions. `new_rev = None` reads the
/// new side from the working tree on disk instead of calling `fetch_batch`.
/// The `fetch_batch` closure is the backend-specific batched-fetch primitive
/// (`hg cat -r REV ...` or `jj file show -r REV ...`).
pub(crate) fn apply_container_full_file_highlight<F>(
    root: &Path,
    old_rev: &str,
    new_rev: Option<&str>,
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
    markdown_full_text: bool,
    fetch_batch: F,
) -> Result<()>
where
    F: Fn(&Path, &str, &[PathBuf]) -> Result<HashMap<PathBuf, String>>,
{
    let old_paths = container_file_paths(files, LineSide::Old, markdown_full_text);
    let new_paths = container_file_paths(files, LineSide::New, markdown_full_text);

    if old_paths.is_empty() && new_paths.is_empty() {
        return Ok(());
    }

    let old_map = fetch_batch(root, old_rev, &old_paths)?;
    let new_map = match new_rev {
        Some(rev) => fetch_batch(root, rev, &new_paths)?,
        None => HashMap::new(),
    };

    let workdir = new_rev.is_none().then(|| root.to_path_buf());

    enhance_with_full_file_highlight(
        files,
        highlighter,
        markdown_full_text,
        |p| old_map.get(p).cloned(),
        |p| match (new_map.get(p), workdir.as_deref()) {
            (Some(content), _) => Some(content.clone()),
            (None, Some(root)) => read_workdir_file(root, p),
            (None, None) => None,
        },
    );

    Ok(())
}

/// Files larger than this skip the full-file pass: no full-file highlight
/// (per-hunk highlighting stays) and no `full_text`. Keeps a runaway-cost
/// ceiling on diffs that include huge generated artefacts (lockfiles, vendored
/// bundles, fixtures).
const MAX_HIGHLIGHT_FILE_BYTES: usize = 1024 * 1024;

/// Whether `content` is small enough, and text enough (no NUL byte), for the
/// full-file pass.
fn fits_full_file_pass(content: &str) -> bool {
    content.len() <= MAX_HIGHLIGHT_FILE_BYTES && !content.as_bytes().contains(&0u8)
}

/// Both sides' lines, split like `str::lines` and tab-expanded like
/// `DiffLine::content`. `None` when neither side was read, or when a side that
/// was read fails `fits_full_file_pass`.
pub(crate) fn full_text(old: Option<&str>, new: Option<&str>) -> Option<FullText> {
    if old.is_none() && new.is_none() {
        return None;
    }
    if !old
        .iter()
        .chain(new.iter())
        .all(|content| fits_full_file_pass(content))
    {
        return None;
    }
    let lines = |content: &str| content.lines().map(tabify).collect();
    Some(FullText {
        old: old.map(lines),
        new: new.map(lines),
    })
}

/// Re-highlight each diff line using full-file context, for files whose
/// grammar needs it (Vue, Svelte, Astro, MDX), and with `markdown_full_text`
/// attach each markdown file's `full_text`. Other files keep their existing
/// per-hunk highlighting unchanged, and markdown files keep theirs.
///
/// `fetch_old`/`fetch_new` return the entire content of the file at the old
/// and new sides respectively (or `None` if unavailable). When a side is
/// available, every diff line on that side is replaced with the span at its
/// 1-based lineno from the full-file highlight. Lines whose side could not be
/// fetched keep whatever the parser already assigned.
///
/// Runs in three phases: fetch (serial, since fetch closures may close over
/// `!Send` state such as `git2::Repository`), highlight (parallel via
/// `std::thread::scope`, since syntect's `SyntaxSet` and `Theme` are `Sync`
/// and each file's syntect work is independent), and apply spans (serial,
/// needs `&mut files`).
pub(crate) fn enhance_with_full_file_highlight<F, G>(
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
    markdown_full_text: bool,
    mut fetch_old: F,
    mut fetch_new: G,
) where
    F: FnMut(&Path) -> Option<String>,
    G: FnMut(&Path) -> Option<String>,
{
    let mut jobs: Vec<HighlightJob> = Vec::new();
    let mut full_texts: Vec<(usize, Option<FullText>)> = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        if !wants_full_file(file, markdown_full_text) {
            continue;
        }
        let Some(syntax_path) = syntax_path(file) else {
            continue;
        };
        let old_content = file.old_path.as_deref().and_then(&mut fetch_old);
        let new_content = file.new_path.as_deref().and_then(&mut fetch_new);
        if needs_full_text(file, markdown_full_text) {
            full_texts.push((
                idx,
                full_text(old_content.as_deref(), new_content.as_deref()),
            ));
            continue;
        }
        if old_content.is_none() && new_content.is_none() {
            continue;
        }
        jobs.push(HighlightJob {
            file_idx: idx,
            syntax_path: syntax_path.to_path_buf(),
            old_content,
            new_content,
        });
    }

    for (idx, text) in full_texts {
        files[idx].full_text = text.map(Arc::new);
    }

    if jobs.is_empty() {
        return;
    }

    let results = highlight_jobs_parallel(&jobs, highlighter);

    for (idx, old, new) in results {
        if old.is_none() && new.is_none() {
            continue;
        }
        apply_full_file_spans(&mut files[idx], highlighter, old.as_deref(), new.as_deref());
    }
}

struct HighlightJob {
    file_idx: usize,
    syntax_path: PathBuf,
    old_content: Option<String>,
    new_content: Option<String>,
}

type HighlightResult = (usize, Option<HighlightedLines>, Option<HighlightedLines>);

fn highlight_jobs_parallel(
    jobs: &[HighlightJob],
    highlighter: &SyntaxHighlighter,
) -> Vec<HighlightResult> {
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(jobs.len());

    if parallelism <= 1 {
        return jobs
            .iter()
            .map(|j| highlight_single_job(j, highlighter))
            .collect();
    }

    let chunk_size = jobs.len().div_ceil(parallelism);
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|j| highlight_single_job(j, highlighter))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("highlight thread panicked"))
            .collect()
    })
}

fn highlight_single_job(job: &HighlightJob, highlighter: &SyntaxHighlighter) -> HighlightResult {
    let old = job
        .old_content
        .as_deref()
        .and_then(|c| highlight_content(highlighter, &job.syntax_path, c));
    let new = job
        .new_content
        .as_deref()
        .and_then(|c| highlight_content(highlighter, &job.syntax_path, c));
    (job.file_idx, old, new)
}

fn highlight_content(
    highlighter: &SyntaxHighlighter,
    path: &Path,
    content: &str,
) -> Option<HighlightedLines> {
    if !fits_full_file_pass(content) {
        return None;
    }
    let lines: Vec<String> = content.lines().map(tabify).collect();
    highlighter.highlight_file_lines(path, &lines)
}

fn apply_full_file_spans(
    file: &mut DiffFile,
    highlighter: &SyntaxHighlighter,
    old_highlight: Option<&[Option<HighlightedSpans>]>,
    new_highlight: Option<&[Option<HighlightedSpans>]>,
) {
    for hunk in &mut file.hunks {
        for line in &mut hunk.lines {
            let old_idx = line.old_lineno.map(|n| n.saturating_sub(1) as usize);
            let new_idx = line.new_lineno.map(|n| n.saturating_sub(1) as usize);
            let spans = highlighter.highlighted_line_for_diff_with_background(
                old_highlight,
                new_highlight,
                old_idx,
                new_idx,
                line.origin,
            );
            if spans.is_some() {
                line.highlighted_spans = spans;
            }
        }
    }
}

/// Detect the VCS type and return the appropriate backend. With
/// `markdown_full_text`, its diff loads read each markdown file's full text
/// into `DiffFile::full_text`.
///
/// Detection order: Jujutsu → Git → Mercurial.
/// Jujutsu is tried first because jj repos are Git-backed.
pub fn detect_vcs(
    git_backend_preference: GitBackendPreference,
    whitespace_mode: DiffWhitespaceMode,
    markdown_full_text: bool,
) -> Result<Box<dyn VcsBackend>> {
    // Try jj first since jj repos are Git-backed
    if let Ok(backend) = JjBackend::discover(whitespace_mode.clone()) {
        return Ok(Box::new(
            backend.with_markdown_full_text(markdown_full_text),
        ));
    }

    // Try git
    if let Ok(backend) = GitBackend::discover(git_backend_preference, whitespace_mode.clone()) {
        return Ok(Box::new(
            backend.with_markdown_full_text(markdown_full_text),
        ));
    }

    // Try hg
    if let Ok(backend) = HgBackend::discover(whitespace_mode) {
        return Ok(Box::new(
            backend.with_markdown_full_text(markdown_full_text),
        ));
    }

    Err(TuicrError::NotARepository)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::traits::VcsType;
    use std::path::PathBuf;

    #[test]
    fn exports_are_accessible() {
        // Verify that public types are properly exported
        let _: fn(GitBackendPreference, DiffWhitespaceMode, bool) -> Result<Box<dyn VcsBackend>> =
            detect_vcs;

        // VcsInfo can be constructed
        let info = VcsInfo {
            root_path: PathBuf::from("/test"),
            head_commit: "abc".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };
        assert_eq!(info.head_commit, "abc");

        // CommitInfo can be constructed
        let commit = CommitInfo {
            id: "abc".to_string(),
            short_id: "abc".to_string(),
            branch_name: Some("main".to_string()),
            summary: "test".to_string(),
            body: None,
            author: "author".to_string(),
            time: chrono::Utc::now(),
        };
        assert_eq!(commit.id, "abc");
    }

    #[test]
    fn detect_vcs_outside_repo_returns_error() {
        // When run outside any VCS repo, should return NotARepository
        // Note: This test may pass or fail depending on where tests are run
        // In CI or outside a repo, it should fail with NotARepository
        // Inside the tuicr repo (which is git), it will succeed
        let result = detect_vcs(
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
            false,
        );

        // We just verify the function runs without panic
        // The actual result depends on the environment
        match result {
            Ok(backend) => {
                // If we're in a repo, we should get valid info
                let info = backend.info();
                assert!(!info.head_commit.is_empty());
            }
            Err(TuicrError::NotARepository) => {
                // Expected when outside a repo
            }
            Err(e) => {
                panic!("Unexpected error: {e:?}");
            }
        }
    }

    #[test]
    fn slice_context_lines_expands_tabs() {
        let content = "fn main() {\n\tprintln!(\"hi\");\n}";

        let lines = slice_context_lines(content, 2, 2);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "    println!(\"hi\");");
    }

    fn vue_diff_file(
        idx: usize,
        deleted_line: &str,
        added_line: &str,
        target_line: u32,
    ) -> DiffFile {
        one_line_change(
            &format!("Comp{idx}.vue"),
            deleted_line,
            added_line,
            target_line,
        )
    }

    fn one_line_change(
        path: &str,
        deleted_line: &str,
        added_line: &str,
        target_line: u32,
    ) -> DiffFile {
        use crate::model::diff_types::{DiffHunk, DiffLine, FileStatus, LineOrigin};
        let path = PathBuf::from(path);
        let hunk = DiffHunk {
            header: format!("@@ -{target_line} +{target_line} @@"),
            lines: vec![
                DiffLine {
                    origin: LineOrigin::Deletion,
                    content: deleted_line.to_string(),
                    old_lineno: Some(target_line),
                    new_lineno: None,
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: added_line.to_string(),
                    old_lineno: None,
                    new_lineno: Some(target_line),
                    highlighted_spans: None,
                },
            ],
            old_start: target_line,
            old_count: 1,
            new_start: target_line,
            new_count: 1,
        };
        DiffFile {
            old_path: Some(path.clone()),
            new_path: Some(path),
            status: FileStatus::Modified,
            hunks: vec![hunk],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
            full_text: None,
        }
    }

    const GUIDE_OLD: &str = "# Guide\n\n| a | old |\n";
    const GUIDE_NEW: &str = "# Guide\n\n| a | new |\n";

    fn lines(text: &str) -> Option<Vec<String>> {
        Some(text.lines().map(str::to_string).collect())
    }

    /// Runs the hg/jj full-file pass over a modified markdown file and a
    /// modified Rust file, with the new side read from a working tree that
    /// holds `GUIDE_NEW`. Returns the files and every `(rev, paths)` batch
    /// fetch.
    fn container_pass(markdown_full_text: bool) -> (Vec<DiffFile>, Vec<(String, Vec<PathBuf>)>) {
        let workdir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(workdir.path().join("docs")).expect("create docs");
        std::fs::write(workdir.path().join("docs/guide.md"), GUIDE_NEW).expect("write guide");
        let mut files = vec![
            one_line_change("docs/guide.md", "| a | old |", "| a | new |", 3),
            one_line_change("src/lib.rs", "fn a() {}", "fn b() {}", 1),
        ];
        let fetches = std::sync::Mutex::new(Vec::new());

        apply_container_full_file_highlight(
            workdir.path(),
            "base",
            None,
            &mut files,
            &SyntaxHighlighter::default(),
            markdown_full_text,
            |_, rev, paths| {
                fetches
                    .lock()
                    .unwrap()
                    .push((rev.to_string(), paths.to_vec()));
                Ok(paths
                    .iter()
                    .filter(|path| path.as_path() == Path::new("docs/guide.md"))
                    .map(|path| (path.clone(), GUIDE_OLD.to_string()))
                    .collect())
            },
        )
        .expect("full-file pass");

        (files, fetches.into_inner().unwrap())
    }

    #[test]
    fn container_pass_attaches_markdown_full_text_from_the_old_rev_and_workdir() {
        let (files, fetches) = container_pass(true);

        assert_eq!(
            files[0].full_text.as_deref(),
            Some(&FullText {
                old: lines(GUIDE_OLD),
                new: lines(GUIDE_NEW),
            })
        );
        assert_eq!(files[1].full_text, None);
        assert_eq!(
            fetches,
            [("base".to_string(), vec![PathBuf::from("docs/guide.md")])]
        );
    }

    #[test]
    fn container_pass_reads_nothing_for_markdown_when_full_text_is_off() {
        let (files, fetches) = container_pass(false);

        assert!(fetches.is_empty());
        assert_eq!(files[0].full_text, None);
    }

    fn make_vue_file(idx: usize) -> (DiffFile, String, String) {
        let old = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\n\
                   import { ref } from 'vue'\nconst msg = ref('hi')\nconst other = 1\n</script>\n";
        let new = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\n\
                   import { ref } from 'vue'\nconst msg = ref('hello')\nconst other = 1\n</script>\n";
        let file = vue_diff_file(idx, "const msg = ref('hi')", "const msg = ref('hello')", 7);
        (file, old.to_string(), new.to_string())
    }

    fn highlight_n_vue_files(n: usize) -> Vec<DiffFile> {
        use crate::syntax::SyntaxHighlighter;

        let mut files = Vec::with_capacity(n);
        let mut content_map: HashMap<PathBuf, (String, String)> = HashMap::new();
        for i in 0..n {
            let (file, old, new) = make_vue_file(i);
            let path = file.new_path.clone().unwrap();
            content_map.insert(path, (old, new));
            files.push(file);
        }

        let highlighter = SyntaxHighlighter::default();
        enhance_with_full_file_highlight(
            &mut files,
            &highlighter,
            false,
            |p| content_map.get(p).map(|(o, _)| o.clone()),
            |p| content_map.get(p).map(|(_, n)| n.clone()),
        );
        files
    }

    fn assert_all_lines_highlighted(files: &[DiffFile]) {
        for (i, file) in files.iter().enumerate() {
            for line in &file.hunks[0].lines {
                let spans = line.highlighted_spans.as_ref().unwrap_or_else(|| {
                    panic!(
                        "file {i} line {:?} should have highlighted spans",
                        line.content
                    )
                });
                let unique_fgs: std::collections::HashSet<_> =
                    spans.iter().filter_map(|(s, _)| s.fg).collect();
                assert!(
                    unique_fgs.len() > 1,
                    "file {i} line {:?} should have multiple distinct fg colors, got {unique_fgs:?}",
                    line.content
                );
            }
        }
    }

    #[test]
    fn enhance_full_file_highlight_serial_path_one_file() {
        // Single file takes the serial branch in highlight_jobs_parallel.
        let files = highlight_n_vue_files(1);
        assert_all_lines_highlighted(&files);
    }

    #[test]
    fn enhance_full_file_highlight_parallel_path_many_files() {
        // 12 files exceeds typical parallelism, forcing chunked thread::scope.
        let files = highlight_n_vue_files(12);
        assert_eq!(files.len(), 12);
        assert_all_lines_highlighted(&files);
    }

    fn synth_vue_file(idx: usize) -> (DiffFile, String, String) {
        let mut html = String::from("<template>\n  <div class=\"app\">\n");
        for i in 0..80 {
            html.push_str(&format!("    <span class=\"item-{i}\">item {i}</span>\n"));
        }
        html.push_str("  </div>\n</template>\n\n");

        let mut script =
            String::from("<script setup lang=\"ts\">\nimport { ref, computed } from 'vue'\n\n");
        for i in 0..90 {
            script.push_str(&format!("const value{i} = ref({i})\n"));
        }
        script.push_str("</script>\n\n");

        let mut style = String::from("<style scoped>\n");
        for i in 0..30 {
            style.push_str(&format!(".item-{i} {{ color: rgb({i}, 0, 0); }}\n"));
        }
        style.push_str("</style>\n");

        let old = format!("{html}{script}{style}");
        let new = old.replace("const value0 = ref(0)", "const value0 = ref(42)");

        let target_line = new
            .lines()
            .position(|l| l.starts_with("const value0 = ref(42)"))
            .expect("synth content must contain target line") as u32
            + 1;
        let file = vue_diff_file(
            idx,
            "const value0 = ref(0)",
            "const value0 = ref(42)",
            target_line,
        );
        (file, old, new)
    }

    /// Manual bench: parallel vs serial highlight at realistic scales.
    /// Run with: `cargo test --release vcs::tests::bench_highlight_parallel_vs_serial -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_highlight_parallel_vs_serial() {
        use crate::syntax::SyntaxHighlighter;
        use std::time::Instant;

        let highlighter = SyntaxHighlighter::default();
        let scales = [1usize, 5, 12, 25, 50];
        let runs = 5;

        for &n in &scales {
            let mut files_template = Vec::with_capacity(n);
            let mut content_map: HashMap<PathBuf, (String, String)> = HashMap::new();
            for i in 0..n {
                let (file, old, new) = synth_vue_file(i);
                content_map.insert(file.new_path.clone().unwrap(), (old, new));
                files_template.push(file);
            }

            let jobs: Vec<HighlightJob> = files_template
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let path = f.new_path.clone().unwrap();
                    let (old, new) = content_map.get(&path).unwrap();
                    HighlightJob {
                        file_idx: idx,
                        syntax_path: path,
                        old_content: Some(old.clone()),
                        new_content: Some(new.clone()),
                    }
                })
                .collect();

            // Warmup
            let _ = highlight_jobs_parallel(&jobs, &highlighter);

            let mut par_total = std::time::Duration::ZERO;
            for _ in 0..runs {
                let t = Instant::now();
                let _ = highlight_jobs_parallel(&jobs, &highlighter);
                par_total += t.elapsed();
            }
            let par_mean = par_total / runs as u32;

            let mut ser_total = std::time::Duration::ZERO;
            for _ in 0..runs {
                let t = Instant::now();
                let _: Vec<HighlightResult> = jobs
                    .iter()
                    .map(|j| highlight_single_job(j, &highlighter))
                    .collect();
                ser_total += t.elapsed();
            }
            let ser_mean = ser_total / runs as u32;

            let speedup = ser_mean.as_secs_f64() / par_mean.as_secs_f64().max(1e-9);
            println!(
                "N={n:>3}: serial={ser_mean:>10.2?}  parallel={par_mean:>10.2?}  speedup={speedup:.2}x"
            );
        }
    }

    #[test]
    fn enhance_full_file_highlight_results_match_input_order() {
        // Each file's highlighted spans must land on that file's hunk lines,
        // not a neighbour's. Distinguishable by line content.
        let files = highlight_n_vue_files(6);
        for (i, file) in files.iter().enumerate() {
            let path = file.new_path.as_ref().unwrap();
            assert_eq!(path.to_str().unwrap(), format!("Comp{i}.vue"));
            let added = file
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .find(|l| l.origin == crate::model::diff_types::LineOrigin::Addition)
                .expect("addition line");
            assert!(
                added.highlighted_spans.is_some(),
                "file {i} addition unhighlighted"
            );
        }
    }
}
