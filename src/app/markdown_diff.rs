//! Rendered `.md` diffs: replaces each markdown diff line's syntect spans with
//! its row from `syntax::markdown_render::render_lines`. Reads only what the
//! loaders left on the `DiffFile` (its hunks and `full_text`), never fetches.

use super::*;
use crate::model::FullText;
use crate::syntax::SyntaxHighlighter;
use crate::syntax::is_markdown_path;
use crate::syntax::markdown_render::render_lines;
use ratatui::style::Style;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type Row = Vec<(Style, String)>;

/// Rendered rows per markdown file, keyed by display path.
#[derive(Default)]
pub(crate) struct MarkdownDiffCache {
    files: HashMap<PathBuf, CachedFile>,
}

#[cfg(test)]
impl MarkdownDiffCache {
    /// The display paths holding cached rows.
    pub(crate) fn paths(&self) -> Vec<&PathBuf> {
        self.files.keys().collect()
    }
}

struct CachedFile {
    source: Source,
    /// The highlighter the rows were rendered under. A theme change replaces
    /// it, which re-renders from `source`. Held rather than kept as an
    /// address: a cloned `Theme` builds a fresh highlighter, which can land at
    /// a freed one's address.
    highlighter: Arc<SyntaxHighlighter>,
    rows: RenderedSides,
}

/// The two sides a file's rows were rendered from.
enum Source {
    /// A Modified or Renamed file's `full_text`. Held, so its address cannot
    /// be reused by a later load while this entry compares against it.
    /// `rebuilt_old` stands in for a missing old side.
    Full {
        text: Arc<FullText>,
        rebuilt_old: Option<Vec<String>>,
    },
    /// An Added or Deleted file's hunk lines, which are the whole file.
    Hunks { old: Vec<String>, new: Vec<String> },
}

impl Source {
    /// The old and new side lines.
    fn sides(&self) -> (&[String], &[String]) {
        match self {
            Self::Full { text, rebuilt_old } => (
                rebuilt_old
                    .as_deref()
                    .or(text.old.as_deref())
                    .unwrap_or_default(),
                text.new.as_deref().unwrap_or_default(),
            ),
            Self::Hunks { old, new } => (old, new),
        }
    }

    /// Whether `file` would render from this same source. A held `full_text`
    /// matches by identity: the loader that built it cut the hunks too, and
    /// they passed the checks when this entry was rendered.
    fn matches(&self, file: &DiffFile) -> bool {
        match self {
            Self::Full { text, .. } => file
                .full_text
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, text)),
            Self::Hunks { old, new } => match file.status {
                FileStatus::Added => {
                    old.is_empty() && new.iter().map(Some).eq(hunk_side(file, |l| l.new_lineno))
                }
                FileStatus::Deleted => {
                    new.is_empty() && old.iter().map(Some).eq(hunk_side(file, |l| l.old_lineno))
                }
                _ => false,
            },
        }
    }
}

/// Both sides of a file rendered one row per source line.
struct RenderedSides {
    old_rows: Vec<Row>,
    new_rows: Vec<Row>,
}

impl RenderedSides {
    fn render(theme: &crate::theme::Theme, source: &Source) -> Self {
        let (old, new) = source.sides();
        Self {
            old_rows: render_lines(theme, &old.join("\n")),
            new_rows: render_lines(theme, &new.join("\n")),
        }
    }
}

/// Whether `file` is a text diff of a markdown file worth rendering.
fn is_renderable_markdown(file: &DiffFile) -> bool {
    is_markdown_path(file.display_path())
        && !file.is_binary
        && !file.is_too_large
        && !file.is_commit_message
}

/// What `file` renders from, or `None` when it must keep its syntect spans.
/// Added and Deleted files render from their hunks. Any other file needs a
/// `full_text` new side that every hunk line agrees with; a mismatch means
/// the text is not the revision the diff was cut from. A missing old side
/// (PR loaders read only the head) is rebuilt from the new side and hunks.
fn render_source(file: &DiffFile) -> Option<Source> {
    match file.status {
        FileStatus::Added => Some(Source::Hunks {
            old: Vec::new(),
            new: owned_hunk_side(file, |line| line.new_lineno)?,
        }),
        FileStatus::Deleted => Some(Source::Hunks {
            old: owned_hunk_side(file, |line| line.old_lineno)?,
            new: Vec::new(),
        }),
        _ => {
            let text = file.full_text.as_ref()?;
            let new = text.new.as_deref()?;
            let rebuilt_old = match text.old.as_deref() {
                Some(old) => {
                    if !hunks_agree(file, old, new) {
                        return None;
                    }
                    None
                }
                None => Some(rebuild_old_side(file, new)?),
            };
            Some(Source::Full {
                text: Arc::clone(text),
                rebuilt_old,
            })
        }
    }
}

/// The hunk lines of a file whose hunks are the whole of one side, in order.
/// A line yields `None` unless `lineno` numbers it at its place: 1, 2, 3, and
/// so on.
fn hunk_side(
    file: &DiffFile,
    lineno: impl Fn(&DiffLine) -> Option<u32>,
) -> impl Iterator<Item = Option<&String>> {
    file.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .enumerate()
        .map(move |(idx, line)| (lineno(line) == Some(idx as u32 + 1)).then_some(&line.content))
}

/// `hunk_side` collected, or `None` when any line is out of place.
fn owned_hunk_side(
    file: &DiffFile,
    lineno: impl Fn(&DiffLine) -> Option<u32>,
) -> Option<Vec<String>> {
    hunk_side(file, lineno).map(|line| line.cloned()).collect()
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

/// Whether every hunk line equals its side's line at its number: Context and
/// Addition lines in `new`, Context and Deletion lines in `old`.
fn hunks_agree(file: &DiffFile, old: &[String], new: &[String]) -> bool {
    let matches = |side: &[String], lineno: Option<u32>, content: &str| {
        lineno
            .and_then(|n| side.get(n.checked_sub(1)? as usize))
            .is_some_and(|line| line == content)
    };
    file.hunks.iter().flat_map(|hunk| &hunk.lines).all(|line| {
        let in_new = matches!(line.origin, LineOrigin::Context | LineOrigin::Addition);
        let in_old = matches!(line.origin, LineOrigin::Context | LineOrigin::Deletion);
        (!in_new || matches(new, line.new_lineno, &line.content))
            && (!in_old || matches(old, line.old_lineno, &line.content))
    })
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
    /// With `render_markdown_diffs` on, replaces each markdown diff line's
    /// `highlighted_spans` with its rendered row. A file that cannot render
    /// keeps its syntect spans. Drops cached rows of files no longer shown.
    pub(crate) fn apply_markdown_diff_renders(&mut self) {
        self.apply_markdown_diff_renders_to(|_| true);
        let shown: HashSet<&PathBuf> = self.diff_files.iter().map(DiffFile::display_path).collect();
        self.markdown_diff_cache
            .files
            .retain(|path, _| shown.contains(path));
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

    /// `file`'s rendered rows per hunk line, from the cache when its source
    /// and highlighter still match, else rendered afresh.
    fn render_markdown_file(
        &self,
        cache: &mut MarkdownDiffCache,
        file: &DiffFile,
    ) -> Option<Vec<Vec<Row>>> {
        let highlighter = self.theme.syntax_highlighter_arc();
        let path = file.display_path();
        if let Some(cached) = cache.files.get(path)
            && Arc::ptr_eq(&cached.highlighter, &highlighter)
            && cached.source.matches(file)
        {
            return spans_for_hunks(file, &cached.rows);
        }

        let source = render_source(file)?;
        let rows = RenderedSides::render(&self.theme, &source);
        let spans = spans_for_hunks(file, &rows)?;
        cache.files.insert(
            path.clone(),
            CachedFile {
                source,
                highlighter,
                rows,
            },
        );
        Some(spans)
    }
}
