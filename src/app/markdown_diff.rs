//! Rendered `.md` diffs: replaces each markdown diff line's syntect spans with
//! its row from `syntax::markdown_render::render_lines`. Reads only what the
//! loaders left on the `DiffFile` (its hunks and `full_text`), never fetches.

use super::*;
use crate::model::FullText;
use crate::syntax::SyntaxHighlighter;
use crate::syntax::is_markdown_path;
use crate::syntax::markdown_render::render_lines;
use ratatui::style::Style;
use std::collections::HashMap;
use std::sync::Arc;

type Row = Vec<(Style, String)>;

/// Rendered rows per markdown file, keyed by display path.
#[derive(Default)]
pub(crate) struct MarkdownDiffCache {
    files: HashMap<PathBuf, CachedFile>,
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
    Full(Arc<FullText>),
    /// An Added or Deleted file's hunk lines, which are the whole file.
    Hunks { old: Vec<String>, new: Vec<String> },
}

impl Source {
    /// The old and new side lines.
    fn sides(&self) -> (&[String], &[String]) {
        match self {
            Self::Full(text) => (
                text.old.as_deref().unwrap_or_default(),
                text.new.as_deref().unwrap_or_default(),
            ),
            Self::Hunks { old, new } => (old, new),
        }
    }

    fn same_as(&self, other: &Source) -> bool {
        match (self, other) {
            (Self::Full(a), Self::Full(b)) => Arc::ptr_eq(a, b),
            (
                Self::Hunks {
                    old: a_old,
                    new: a_new,
                },
                Self::Hunks {
                    old: b_old,
                    new: b_new,
                },
            ) => a_old == b_old && a_new == b_new,
            _ => false,
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
/// `full_text` with both sides that every hunk line agrees with; a mismatch
/// means the text is not the revision the diff was cut from.
fn render_source(file: &DiffFile) -> Option<Source> {
    match file.status {
        FileStatus::Added => Some(Source::Hunks {
            old: Vec::new(),
            new: hunk_side(file, |line| line.new_lineno)?,
        }),
        FileStatus::Deleted => Some(Source::Hunks {
            old: hunk_side(file, |line| line.old_lineno)?,
            new: Vec::new(),
        }),
        _ => {
            let text = file.full_text.as_ref()?;
            let (old, new) = (text.old.as_deref()?, text.new.as_deref()?);
            hunks_agree(file, old, new).then(|| Source::Full(Arc::clone(text)))
        }
    }
}

/// The hunk lines of a file whose hunks are the whole of one side, in order.
/// `None` unless `lineno` numbers them 1, 2, 3, and so on.
fn hunk_side(file: &DiffFile, lineno: impl Fn(&DiffLine) -> Option<u32>) -> Option<Vec<String>> {
    file.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .enumerate()
        .map(|(idx, line)| (lineno(line) == Some(idx as u32 + 1)).then(|| line.content.clone()))
        .collect()
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

    /// `file`'s rendered rows per hunk line, from the cache when its source
    /// and highlighter still match, else rendered afresh.
    fn render_markdown_file(
        &self,
        cache: &mut MarkdownDiffCache,
        file: &DiffFile,
    ) -> Option<Vec<Vec<Row>>> {
        let source = render_source(file)?;
        let highlighter = self.theme.syntax_highlighter_arc();
        let path = file.display_path();
        if let Some(cached) = cache.files.get(path)
            && Arc::ptr_eq(&cached.highlighter, &highlighter)
            && cached.source.same_as(&source)
        {
            return spans_for_hunks(file, &cached.rows);
        }

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
