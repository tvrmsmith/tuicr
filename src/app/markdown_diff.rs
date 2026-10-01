//! Rendered `.md` diffs: replaces each markdown diff line's syntect spans with
//! its row from `syntax::markdown_render::render_lines`.

use super::*;
use crate::syntax::markdown_render::render_lines;
use ratatui::style::Style;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type Row = Vec<(Style, String)>;

/// Where a file's content comes from. Two files at the same path with
/// different providers are different sources.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProviderId {
    diff_source: std::mem::Discriminant<DiffSource>,
    forge_attached: bool,
    /// The PR head sha in PR mode, the reference commit otherwise.
    revision: Option<String>,
}

/// A fetch or check that failed for exactly these hunks of this file.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FailureKey {
    provider: ProviderId,
    path: PathBuf,
    content_hash: u64,
    hunk_headers: Vec<String>,
}

#[derive(Default)]
pub(crate) struct MarkdownDiffCache {
    /// Failures not to retry on every rebuild (`rebuild_annotations` runs
    /// inside draw on resize). A changed key part retries; so does
    /// `forget_failures`.
    failures: HashSet<FailureKey>,
    /// Rendered files, only ever stored after both sides rendered. A fetched
    /// file's `new` is its cached source.
    files: HashMap<(ProviderId, PathBuf), CachedFile>,
}

struct CachedFile {
    new: Vec<String>,
    old: Vec<String>,
    /// Address of the highlighter the rows were rendered under: a theme change
    /// replaces it, which re-renders from `new` without a fetch.
    highlighter: usize,
    rows: RenderedSides,
}

impl CachedFile {
    /// Whether the rows still stand for `old`, `new`, and `highlighter`. `new`
    /// is `None` when it came from this entry.
    fn matches(&self, highlighter: usize, old: &[String], new: Option<&[String]>) -> bool {
        self.highlighter == highlighter && self.old == old && new.is_none_or(|new| self.new == new)
    }
}

impl MarkdownDiffCache {
    /// Allows every failed file to try again, for an explicit reload.
    pub(crate) fn forget_failures(&mut self) {
        self.failures.clear();
    }
}

/// Both sides of a file rendered one row per source line.
struct RenderedSides {
    old_rows: Vec<Row>,
    new_rows: Vec<Row>,
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// Whether `file` is a text diff of a markdown file worth rendering.
fn is_renderable_markdown(file: &DiffFile) -> bool {
    is_markdown_path(file.display_path())
        && !file.is_binary
        && !file.is_too_large
        && !file.is_commit_message
}

/// Index of the line before a hunk's range on one side. A zero-count range
/// names the line it follows, so its start is already that index.
fn cursor_before(start: u32, count: u32) -> usize {
    if count == 0 {
        start as usize
    } else {
        start.saturating_sub(1) as usize
    }
}

/// The old side, rebuilt from the new side plus `file`'s hunks. `None` when
/// the hunks do not line up with `new`: a hunk starts off the cursor, a count
/// is not used up, or a Context/Addition line differs from `new` at its
/// number. That means `new` is not the revision the diff was cut from.
fn rebuild_old_side(file: &DiffFile, new: &[String]) -> Option<Vec<String>> {
    let mut old: Vec<String> = Vec::with_capacity(new.len());
    let mut new_cursor = 0usize;
    for hunk in &file.hunks {
        let hunk_new_start = cursor_before(hunk.new_start, hunk.new_count);
        if hunk_new_start < new_cursor || hunk_new_start > new.len() {
            return None;
        }
        old.extend_from_slice(&new[new_cursor..hunk_new_start]);
        new_cursor = hunk_new_start;
        if old.len() != cursor_before(hunk.old_start, hunk.old_count) {
            return None;
        }

        let (mut old_used, mut new_used) = (0u32, 0u32);
        for line in &hunk.lines {
            match line.origin {
                LineOrigin::Context | LineOrigin::Addition => {
                    if new.get(new_cursor) != Some(&line.content)
                        || line.new_lineno != Some(new_cursor as u32 + 1)
                    {
                        return None;
                    }
                    new_cursor += 1;
                    new_used += 1;
                    if line.origin == LineOrigin::Context {
                        if line.old_lineno != Some(old.len() as u32 + 1) {
                            return None;
                        }
                        old.push(line.content.clone());
                        old_used += 1;
                    }
                }
                LineOrigin::Deletion => {
                    if line.old_lineno != Some(old.len() as u32 + 1) {
                        return None;
                    }
                    old.push(line.content.clone());
                    old_used += 1;
                }
            }
        }
        if old_used != hunk.old_count || new_used != hunk.new_count {
            return None;
        }
    }
    old.extend_from_slice(&new[new_cursor..]);
    Some(old)
}

/// The old and new sides of an Added file: the hunk is the whole new side and
/// nothing came before it.
fn added_file_sides(file: &DiffFile) -> Option<(Vec<String>, Vec<String>)> {
    let new: Vec<String> = file
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.content.clone())
        .collect();
    let old = rebuild_old_side(file, &new)?;
    old.is_empty().then_some((old, new))
}

/// The old and new sides of a Deleted file: the hunk is the whole old side and
/// nothing remains.
fn deleted_file_sides(file: &DiffFile) -> Option<(Vec<String>, Vec<String>)> {
    let old: Vec<String> = file
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .enumerate()
        .map(|(idx, line)| (line.old_lineno == Some(idx as u32 + 1)).then(|| line.content.clone()))
        .collect::<Option<_>>()?;
    Some((old, Vec::new()))
}

/// Each hunk line's rendered row: deletions from the old side by old line
/// number, everything else from the new side by new line number.
fn spans_for_hunks(file: &DiffFile, sides: &RenderedSides) -> Option<Vec<Vec<Row>>> {
    file.hunks
        .iter()
        .map(|hunk| {
            hunk.lines
                .iter()
                .map(|line| {
                    let (rows, lineno) = match line.origin {
                        LineOrigin::Deletion => (&sides.old_rows, line.old_lineno?),
                        LineOrigin::Context | LineOrigin::Addition => {
                            (&sides.new_rows, line.new_lineno?)
                        }
                    };
                    rows.get(lineno.checked_sub(1)? as usize).cloned()
                })
                .collect()
        })
        .collect()
}

impl App {
    pub fn set_render_markdown_diffs(&mut self, on: bool) {
        self.render_markdown_diffs = on;
        self.rebuild_annotations();
    }

    /// With `render_markdown_diffs` on, replaces each markdown diff line's
    /// `highlighted_spans` with its rendered row. A file that cannot render
    /// keeps its syntect spans.
    pub(crate) fn apply_markdown_diff_renders(&mut self) {
        self.apply_markdown_diff_renders_to(|_| true);
    }

    /// Like `apply_markdown_diff_renders`, for the files `include` accepts by
    /// index. Theme preview uses it to re-render only the file on screen.
    pub(crate) fn apply_markdown_diff_renders_to(&mut self, include: impl Fn(usize) -> bool) {
        if !self.render_markdown_diffs {
            return;
        }
        let mut cache = std::mem::take(&mut self.markdown_diff_cache);
        let rendered: Vec<(usize, Vec<Vec<Row>>)> = self
            .diff_files
            .iter()
            .enumerate()
            .filter(|(idx, file)| include(*idx) && is_renderable_markdown(file))
            .filter_map(|(idx, file)| Some((idx, self.render_markdown_file(&mut cache, file)?)))
            .collect();
        self.markdown_diff_cache = cache;
        for (idx, spans) in rendered {
            let hunks = &mut self.diff_files[idx].hunks;
            for (hunk, hunk_spans) in hunks.iter_mut().zip(spans) {
                for (line, row) in hunk.lines.iter_mut().zip(hunk_spans) {
                    line.highlighted_spans = Some(row);
                }
            }
        }
    }

    fn provider_id(&self) -> ProviderId {
        let revision = match &self.diff_source {
            DiffSource::PullRequest(pr) => Some(pr.key.head_sha.clone()),
            _ => self.ref_commit().map(str::to_string),
        };
        ProviderId {
            diff_source: std::mem::discriminant(&self.diff_source),
            forge_attached: self.forge_backend.is_some(),
            revision,
        }
    }

    /// `file`'s rendered rows per hunk line, from the cache when its source,
    /// old side, and highlighter still match, else rendered afresh. A fetched
    /// source that no longer fits the hunks is dropped and fetched once more.
    fn render_markdown_file(
        &self,
        cache: &mut MarkdownDiffCache,
        file: &DiffFile,
    ) -> Option<Vec<Vec<Row>>> {
        let key = (self.provider_id(), file.display_path().clone());
        let highlighter = Arc::as_ptr(&self.theme.syntax_highlighter_arc()) as usize;
        let hunk_only = match file.status {
            FileStatus::Added => Some(added_file_sides(file)?),
            FileStatus::Deleted => Some(deleted_file_sides(file)?),
            _ => None,
        };

        let mut sides = hunk_only;
        if let Some(cached) = cache.files.get(&key) {
            match &sides {
                Some((old, new)) => {
                    if cached.matches(highlighter, old, Some(new)) {
                        return spans_for_hunks(file, &cached.rows);
                    }
                }
                None => {
                    if let Some(old) = rebuild_old_side(file, &cached.new) {
                        if cached.matches(highlighter, &old, None) {
                            return spans_for_hunks(file, &cached.rows);
                        }
                        sides = Some((old, cached.new.clone()));
                    }
                }
            }
        }

        let (old, new) = match sides {
            Some(sides) => sides,
            None => {
                let failure = FailureKey {
                    provider: key.0.clone(),
                    path: key.1.clone(),
                    content_hash: file.content_hash,
                    hunk_headers: file.hunks.iter().map(|h| h.header.clone()).collect(),
                };
                if cache.failures.contains(&failure) {
                    return None;
                }
                let fetched = self
                    .fetch_new_side(file)
                    .and_then(|new| Some((rebuild_old_side(file, &new)?, new)));
                match fetched {
                    Some(sides) => sides,
                    None => {
                        cache.files.remove(&key);
                        cache.failures.insert(failure);
                        return None;
                    }
                }
            }
        };

        let rows = RenderedSides {
            old_rows: render_lines(&self.theme, &old.join("\n")),
            new_rows: render_lines(&self.theme, &new.join("\n")),
        };
        let spans = spans_for_hunks(file, &rows)?;
        cache.files.insert(
            key,
            CachedFile {
                new,
                old,
                highlighter,
                rows,
            },
        );
        Some(spans)
    }

    /// The whole new side of `file` through the context provider, in one call:
    /// every adapter stops at the end of the file instead of erroring. A failed
    /// fetch and an empty one both give `None`.
    fn fetch_new_side(&self, file: &DiffFile) -> Option<Vec<String>> {
        let fetched = self
            .context_provider()
            .fetch_context_lines(
                file.old_path.as_ref(),
                file.new_path.as_ref(),
                file.status,
                1,
                u32::MAX,
            )
            .ok()?;
        if fetched.is_empty() {
            return None;
        }
        Some(fetched.into_iter().map(|line| line.content).collect())
    }
}
