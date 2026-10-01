//! Rendered markdown for the PR description panel: every syntax marker
//! hidden, paragraphs reflowed to the panel width, close to GitHub's look.
//!
//! One pulldown-cmark pass records, per source byte, the style it renders in,
//! plus replacement runs for marker ranges (`**`, `[`, `](url)`, list bullets)
//! and table rows, which are laid out once the width is known.
//! Rows are then built line by line from that record, joining prose lines
//! into one logical line before word-wrapping it. Keeping the record per
//! source byte is what lets each row report the source lines it came from.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
    TagEnd,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

mod details;

use super::SyntaxHighlighter;
use crate::theme::Theme;

type Runs = Vec<(Style, String)>;

/// A chip's padding while rows reflow: `wrap` breaks only at `' '`, so the
/// pads stay glued to the chip text. `to_line` turns it back into a space.
const NBSP: char = '\u{a0}';

/// One reflowed display row of rendered markdown.
pub(crate) struct BlockRow {
    pub line: Line<'static>,
    /// 0-based source lines (the body split on '\n') of the row group this row
    /// belongs to. Every row of one reflowed group carries the group's range.
    pub source: Range<usize>,
    /// The `<details>` section this row is the summary of, numbered by the
    /// opening tag's order in the source from 0 (nested sections included).
    /// `None` on every other row.
    pub details: Option<usize>,
}

/// Render `src` as reflowed markdown rows no wider than `width` columns
/// (a code row wider than `width` wraps with its continuation prefix, and a
/// table wider than `width` wraps text inside its columns).
///
/// A block-level `<details>` section with a matching `</details>` renders as
/// one summary row, `▸ label` collapsed or `▾ label` expanded, in place of its
/// header (the `<details>` line through its `</summary>` line). Its closing
/// line renders nothing, and its body renders only while it is expanded.
/// Sections start collapsed, or expanded when the tag carries `open`;
/// `toggled` names the sections that render opposite to that default, and
/// ids that name no section are ignored.
///
/// `keep_apart` lists merged, non-overlapping source-line ranges. Lines in a
/// range never join lines outside it. Each range yields at least one row
/// whose `source` is exactly that range, except a range wholly inside a
/// collapsed section, which yields none. A range that overlaps a section's
/// header without matching it gets its row right after the summary row.
///
/// Pure: no I/O, no global state; same inputs give the same rows.
pub(crate) fn render_block(
    theme: &Theme,
    src: &str,
    width: usize,
    keep_apart: &[Range<usize>],
    toggled: &BTreeSet<usize>,
) -> Vec<BlockRow> {
    let width = width.max(1);
    let mut doc = Doc::build(theme, src);
    doc.fold_details(toggled);
    let lines = doc.line_count();
    let keep_apart: Vec<Range<usize>> = keep_apart
        .iter()
        .map(|r| r.start.min(lines)..r.end.min(lines))
        .filter(|r| !r.is_empty())
        .collect();
    // The `keep_apart` range each source line belongs to, if any.
    let mut apart = vec![None; lines];
    for range in &keep_apart {
        apart[range.clone()].fill(Some(range.clone()));
    }
    let mut rows = trim_blank_rows(doc.rows(width, &apart));
    // A range whose lines all render nothing (a fence, a dropped blank) still
    // owes the caller a row to anchor on.
    for range in keep_apart {
        if rows.iter().any(|row| row.source == range) {
            continue;
        }
        // The header holding the range shows as its summary row, so the
        // anchor follows that row.
        let summary = rows.iter().rposition(|row| {
            row.details.is_some() && row.source.start < range.end && range.start < row.source.end
        });
        let at = match summary {
            Some(last) => last + 1,
            None if range.clone().all(|line| doc.folded[line]) => continue,
            None => rows.partition_point(|row| row.source.start < range.start),
        };
        rows.insert(
            at,
            BlockRow {
                line: Line::default(),
                source: range,
                details: None,
            },
        );
    }
    rows
}

struct Look {
    base: Style,
    /// The theme's heading style with bold and underline cleared: themes often
    /// bold headings already, so each level states both explicitly.
    heading: Style,
    /// h6, raw HTML, and a code block's language tag.
    dim: Style,
    /// Inline code, drawn as a chip padded one space each side.
    chip: Style,
    link: Style,
    image: Style,
    /// Code block gutter, table bars, and rules.
    border: Style,
    /// Fenced code whose language syntect does not know.
    code_block: Style,
    /// List bullets, numbers, and task boxes.
    bullet: Style,
    quote_bar: Style,
    quote_text: Style,
}

/// glow's inline code chip on a dark theme, in 256-colour.
pub(crate) const GLOW_CHIP_FG: Color = Color::Indexed(203);
pub(crate) const GLOW_CHIP_BG: Color = Color::Indexed(236);

impl Look {
    fn new(theme: &Theme) -> Self {
        let palette = &theme.syntax_highlighter().markdown_palette;
        Self {
            base: palette.base,
            heading: palette
                .heading
                .remove_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            dim: Style::default().fg(theme.fg_dim),
            // glow's chip. Its dark gray would blot a light panel, so light
            // themes derive theirs.
            chip: if theme.is_dark() {
                Style::default().fg(GLOW_CHIP_FG).bg(GLOW_CHIP_BG)
            } else {
                palette.code.bg(theme.bg_highlight)
            },
            link: palette.link.add_modifier(Modifier::UNDERLINED),
            image: palette.link,
            border: Style::default().fg(theme.border_unfocused),
            code_block: Style::default().fg(theme.fg_secondary),
            bullet: palette.list,
            quote_bar: palette.quote,
            quote_text: palette.quote.add_modifier(Modifier::ITALIC),
        }
    }

    fn heading(&self, level: HeadingLevel) -> Style {
        match level {
            HeadingLevel::H1 => self
                .heading
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            HeadingLevel::H2 | HeadingLevel::H4 => self.heading.add_modifier(Modifier::UNDERLINED),
            HeadingLevel::H3 => self.heading.add_modifier(Modifier::BOLD),
            HeadingLevel::H5 => self.heading.add_modifier(Modifier::ITALIC),
            HeadingLevel::H6 => self.dim.add_modifier(Modifier::ITALIC),
        }
    }

    /// `style` underlined in the border colour, as a table header's last row is.
    fn header(&self, style: Style) -> Style {
        let style = style.add_modifier(Modifier::UNDERLINED);
        self.border
            .fg
            .map_or(style, |color| style.underline_color(color))
    }
}

fn is_block(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
    )
}

/// Alerts keep ANSI named colours, which follow the terminal's palette on
/// light themes too.
fn alert_label(kind: BlockQuoteKind) -> (Style, &'static str) {
    let (color, label) = match kind {
        BlockQuoteKind::Note => (Color::Blue, "ℹ Note"),
        BlockQuoteKind::Tip => (Color::Green, "💡 Tip"),
        BlockQuoteKind::Important => (Color::Magenta, "❗ Important"),
        BlockQuoteKind::Warning => (Color::Yellow, "⚠ Warning"),
        BlockQuoteKind::Caution => (Color::Red, "⛔ Caution"),
    };
    (
        Style::default().fg(color).add_modifier(Modifier::BOLD),
        label,
    )
}

/// A link or image being walked: where it starts and where its text ends.
struct OpenLink {
    start: usize,
    text_end: usize,
    autolink: bool,
    image: bool,
}

/// A table being walked: its column alignments, source range, and each row's
/// cell ranges, header first.
struct OpenTable {
    aligns: Vec<Alignment>,
    range: Range<usize>,
    rows: Vec<(bool, Vec<Range<usize>>)>,
}

/// A walked table's column alignments and natural column widths.
struct Table {
    aligns: Vec<Alignment>,
    natural: Vec<usize>,
}

/// One source row of a table: its rendered cells, laid out by `table_lines`.
struct TableRow {
    /// Index into `Doc::tables`.
    table: usize,
    head: bool,
    cells: Vec<Runs>,
}

struct Doc<'a> {
    src: &'a str,
    look: Look,
    /// Render style of each source byte.
    styles: Vec<Style>,
    /// Marker ranges keyed by start byte: the range's end and what it renders
    /// as (often nothing).
    repl: BTreeMap<usize, (usize, Runs)>,
    /// Entity references keyed by start byte: the range's end and the text it
    /// decodes to, drawn in the style of its first byte.
    decoded: HashMap<usize, (usize, String)>,
    line_starts: Vec<usize>,
    /// Lines that render alone, never joined into a paragraph.
    no_reflow: Vec<bool>,
    /// Lines where the parser opened a block (an item, a paragraph) or a hard
    /// break ended the previous line, so they never join the line above.
    block_start: Vec<bool>,
    /// h1 and h2 lines and table headers, whose underline pads their last row
    /// in the given style out to the given width, capped at the row width.
    underline_pad: HashMap<usize, (Style, usize)>,
    /// Lines that render no row at all (code fences, link definitions).
    skip: Vec<bool>,
    /// Runs drawn before a line's own text (the code gutter).
    prefixes: HashMap<usize, Runs>,
    /// Runs right-aligned on a line's first row (a code block's language).
    labels: HashMap<usize, Runs>,
    tables: Vec<Table>,
    /// Lines drawn as a table row instead of from their source.
    table_rows: HashMap<usize, TableRow>,
    /// Horizontal rule lines, drawn across the full width.
    rules: Vec<bool>,
    /// Byte ranges of the HTML blocks, where `<details>` sections open.
    html_blocks: Vec<Range<usize>>,
    /// Summary rows keyed by the first line of the header they replace.
    summaries: HashMap<usize, Summary>,
    /// Body and closing lines of collapsed sections.
    folded: Vec<bool>,
}

/// The row a `<details>` section's header lines render as.
struct Summary {
    id: usize,
    /// The header's source lines.
    lines: Range<usize>,
    label: String,
    expanded: bool,
}

impl<'a> Doc<'a> {
    fn build(theme: &Theme, src: &'a str) -> Self {
        let hl = theme.syntax_highlighter();
        let look = Look::new(theme);
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        let lines = line_starts.len();
        let mut doc = Self {
            src,
            styles: vec![look.base; src.len()],
            look,
            repl: BTreeMap::new(),
            decoded: HashMap::new(),
            line_starts,
            no_reflow: vec![false; lines],
            block_start: vec![false; lines],
            underline_pad: HashMap::new(),
            skip: vec![false; lines],
            prefixes: HashMap::new(),
            labels: HashMap::new(),
            tables: Vec::new(),
            table_rows: HashMap::new(),
            rules: vec![false; lines],
            html_blocks: Vec::new(),
            summaries: HashMap::new(),
            folded: vec![false; lines],
        };
        doc.walk(hl);
        doc
    }

    fn walk(&mut self, hl: &SyntaxHighlighter) {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_GFM);

        let mut links: Vec<OpenLink> = Vec::new();
        // syntect state for the fenced block being walked, if its language is known.
        let mut code: Option<syntect::easy::HighlightLines> = None;
        let mut in_code = false;
        // End of the last text event, which owns a backslash right before the
        // next one; an unowned backslash there is an escape.
        let mut text_end = 0;
        let mut table: Option<OpenTable> = None;
        let mut list_depth = 0usize;
        let parser = Parser::new_ext(self.src, opts);
        // Link reference definitions (`[d]: https://…`) emit no events; their
        // links already show the text, so the definition lines render nothing.
        let definitions: Vec<Range<usize>> = parser
            .reference_definitions()
            .iter()
            .map(|(_, def)| def.span.clone())
            .collect();
        for span in definitions {
            for line in self.lines_of(&span) {
                self.skip[line] = true;
            }
        }
        for (event, r) in parser.into_offset_iter() {
            let link_edge = matches!(
                event,
                Event::Start(Tag::Link { .. } | Tag::Image { .. })
                    | Event::End(TagEnd::Link | TagEnd::Image)
            );
            if !link_edge {
                for link in &mut links {
                    link.text_end = link.text_end.max(r.end);
                }
            }
            if let Event::Start(tag) = &event
                && is_block(tag)
            {
                let line = self.line_of(r.start);
                self.block_start[line] = true;
            }
            match event {
                Event::Start(Tag::Heading { level, .. }) => self.heading(level, r),
                Event::HardBreak => {
                    // The `\` or trailing spaces before the newline.
                    let line = self.line_of(r.start);
                    let end = r.end.min(self.line_range(line).end);
                    self.replace(r.start..end, vec![]);
                    if let Some(next) = self.block_start.get_mut(line + 1) {
                        *next = true;
                    }
                }
                Event::Start(Tag::List(_)) => list_depth += 1,
                Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
                Event::Start(Tag::Item) => self.item(r, list_depth),
                Event::Start(Tag::BlockQuote(kind)) => self.quote(r, kind),
                Event::Html(_) | Event::InlineHtml(_) => self.paint(r, self.look.dim),
                Event::Start(Tag::HtmlBlock) => {
                    self.html_blocks.push(r.clone());
                    for line in self.lines_of(&r) {
                        self.no_reflow[line] = true;
                    }
                }
                Event::Rule => {
                    let line = self.line_of(r.start);
                    self.no_reflow[line] = true;
                    self.rules[line] = true;
                }
                Event::TaskListMarker(done) => {
                    let glyph = if done { "☑" } else { "☐" };
                    self.replace(r, vec![(self.look.bullet, glyph.to_string())]);
                }
                Event::Start(Tag::Strong) => {
                    self.paint(r.clone(), Style::new().add_modifier(Modifier::BOLD));
                    self.delimiters(r, 2);
                }
                Event::Start(Tag::Emphasis) => {
                    self.paint(r.clone(), Style::new().add_modifier(Modifier::ITALIC));
                    self.delimiters(r, 1);
                }
                Event::Start(Tag::Strikethrough) => {
                    self.paint(r.clone(), Style::new().add_modifier(Modifier::CROSSED_OUT));
                    let n = self.run_len(r.start, '~');
                    self.delimiters(r, n);
                }
                Event::Code(_) => self.chip(r),
                Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. }) => {
                    let image = matches!(event, Event::Start(Tag::Image { .. }));
                    links.push(OpenLink {
                        start: r.start,
                        text_end: r.start + if image { 2 } else { 1 },
                        autolink: matches!(link_type, LinkType::Autolink | LinkType::Email),
                        image,
                    })
                }
                Event::End(TagEnd::Link | TagEnd::Image) => {
                    if let Some(link) = links.pop() {
                        self.link(link, r.end);
                    }
                    // A closed inner image is part of its enclosing link's
                    // text (`[![CI](badge)](run)`).
                    for link in &mut links {
                        link.text_end = link.text_end.max(r.end);
                    }
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let lang = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or(""),
                        CodeBlockKind::Indented => "",
                    };
                    code = (!lang.is_empty())
                        .then(|| hl.syntax_set.find_syntax_by_token(lang))
                        .flatten()
                        .map(|syntax| syntect::easy::HighlightLines::new(syntax, &hl.theme));
                    in_code = true;
                    self.code_block(r, matches!(kind, CodeBlockKind::Fenced(_)), lang);
                }
                Event::End(TagEnd::CodeBlock) => {
                    code = None;
                    in_code = false;
                }
                Event::Start(Tag::Table(aligns)) => {
                    table = Some(OpenTable {
                        aligns,
                        range: r,
                        rows: Vec::new(),
                    })
                }
                Event::Start(Tag::TableHead | Tag::TableRow) => {
                    if let Some(t) = table.as_mut() {
                        t.rows
                            .push((matches!(event, Event::Start(Tag::TableHead)), Vec::new()));
                    }
                }
                Event::Start(Tag::TableCell) => {
                    if let Some((_, cells)) = table.as_mut().and_then(|t| t.rows.last_mut()) {
                        cells.push(r);
                    }
                }
                Event::End(TagEnd::Table) => {
                    if let Some(t) = table.take() {
                        self.table(t);
                    }
                }
                Event::Text(text) if !in_code => {
                    if r.start > text_end && self.src[..r.start].ends_with('\\') {
                        self.replace(r.start - 1..r.start, vec![]);
                    }
                    if self.src[r.clone()] != *text {
                        self.decoded.insert(r.start, (r.end, text.to_string()));
                    }
                    text_end = r.end;
                }
                Event::Text(text) => {
                    let Some(block) = code.as_mut() else {
                        continue;
                    };
                    let Ok(runs) = block.highlight_line(&text, &hl.syntax_set) else {
                        continue;
                    };
                    let mut at = r.start;
                    for (style, piece) in runs {
                        let style = SyntaxHighlighter::syntect_to_ratatui_style(style);
                        self.paint(at..at + piece.len(), style);
                        at += piece.len();
                    }
                }
                _ => {}
            }
        }
    }

    fn line_of(&self, byte: usize) -> usize {
        match self.line_starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    /// Byte length of the run of `ch` starting at `at`.
    fn run_len(&self, at: usize, ch: char) -> usize {
        self.src[at..].chars().take_while(|c| *c == ch).count() * ch.len_utf8()
    }

    fn replace(&mut self, r: Range<usize>, runs: Runs) {
        if r.start < r.end {
            self.repl.entry(r.start).or_insert((r.end, runs));
        }
    }

    fn paint(&mut self, r: Range<usize>, style: Style) {
        let end = r.end.min(self.src.len());
        if let Some(slots) = self.styles.get_mut(r.start..end) {
            for slot in slots {
                *slot = slot.patch(style);
            }
        }
    }

    /// Hide the `n`-byte delimiters at both ends of an emphasis range.
    fn delimiters(&mut self, r: Range<usize>, n: usize) {
        if n == 0 || r.end < r.start + 2 * n {
            return;
        }
        self.replace(r.start..r.start + n, vec![]);
        self.replace(r.end - n..r.end, vec![]);
    }

    fn chip(&mut self, r: Range<usize>) {
        let ticks = self.run_len(r.start, '`');
        let chip = self.look.chip;
        let pad = vec![(chip, NBSP.to_string())];
        self.paint(r.start + ticks..r.end.saturating_sub(ticks), chip);
        self.replace(r.start..r.start + ticks, pad.clone());
        self.replace(r.end.saturating_sub(ticks)..r.end, pad);
    }

    /// Show a link's text, underlined, or an image's alt text after `▣`, and
    /// hide the brackets and URL.
    fn link(&mut self, link: OpenLink, end: usize) {
        let style = if link.image {
            self.look.image
        } else {
            self.look.link
        };
        if link.autolink {
            // `<https://…>`: the URL is the text, only the angle brackets go.
            self.paint(link.start..end, style);
            self.replace(link.start..link.start + 1, vec![]);
            self.replace(end.saturating_sub(1)..end, vec![]);
            return;
        }
        let (open, glyph) = if link.image {
            (2, vec![(style, "▣ ".to_string())])
        } else {
            (1, vec![])
        };
        let text_start = (link.start + open).min(end);
        let text_end = link.text_end.clamp(text_start, end);
        self.paint(text_start..text_end, style);
        self.replace(link.start..text_start, glyph);
        self.replace(text_end..end, vec![]);
    }

    fn lines_of(&self, r: &Range<usize>) -> std::ops::RangeInclusive<usize> {
        let first = self.line_of(r.start);
        let last = self.line_of(r.end.saturating_sub(1).max(r.start));
        first..=last
    }

    /// A fenced block draws no fence rows: a gutter marks its content lines and
    /// the language sits right-aligned on the first one.
    fn code_block(&mut self, r: Range<usize>, fenced: bool, lang: &str) {
        let lines = self.lines_of(&r);
        let (first, last) = (*lines.start(), *lines.end());
        for line in lines {
            self.no_reflow[line] = true;
        }
        if !fenced {
            return;
        }
        let fence = self.src[r.start..].chars().next();
        // An unclosed fence runs to the end of the body, with no closing row.
        let closed = last > first
            && fence.is_some_and(|f| {
                let close =
                    self.src[self.content_start(last, false)..self.line_range(last).end].trim_end();
                close.len() >= self.run_len(r.start, f) && close.chars().all(|c| c == f)
            });
        let content = first + 1..if closed { last } else { last + 1 };
        // A list marker before the fence (`- ```rust`) keeps its row.
        if self.content_start(first, false) < r.start {
            self.replace(r.start..self.line_range(first).end, vec![]);
        } else {
            self.skip[first] = true;
        }
        if closed {
            self.skip[last] = true;
        }
        for line in content.clone() {
            self.paint(self.line_range(line), self.look.code_block);
            self.prefixes
                .insert(line, vec![(self.look.border, "│ ".to_string())]);
        }
        if !content.is_empty() && !lang.is_empty() {
            self.labels
                .insert(content.start, vec![(self.look.dim, lang.to_string())]);
        }
    }

    /// Inner ` │ ` bars, no outer bars. The header row is underlined across
    /// the whole table in the border colour, which replaces the delimiter row.
    fn table(&mut self, t: OpenTable) {
        for line in self.lines_of(&t.range) {
            self.no_reflow[line] = true;
            // Every line not a cell row (the delimiter) renders nothing.
            self.skip[line] = true;
        }
        let rows: Vec<(usize, bool, Vec<Runs>)> = t
            .rows
            .iter()
            .filter_map(|(is_head, cells)| {
                let line = self.line_of(cells.first()?.start);
                let rendered = cells
                    .iter()
                    .map(|cell| {
                        let s = &self.src[cell.clone()];
                        let lead = s.len() - s.trim_start().len();
                        let trail = s.len() - s.trim_end().len();
                        let mut runs =
                            self.emit(cell.start + lead, (cell.end - trail).max(cell.start + lead));
                        if *is_head {
                            for (style, _) in &mut runs {
                                *style = style.add_modifier(Modifier::BOLD);
                            }
                        }
                        runs
                    })
                    .collect();
                Some((line, *is_head, rendered))
            })
            .collect();
        let columns = t.aligns.len().max(
            rows.iter()
                .map(|(_, _, cells)| cells.len())
                .max()
                .unwrap_or(0),
        );
        let mut natural = vec![1; columns];
        for (_, _, cells) in &rows {
            for (width, cell) in natural.iter_mut().zip(cells) {
                *width = (*width).max(runs_width(cell));
            }
        }

        // The header's underline reaches the table's edge through
        // `underline_pad`, since the last column carries no trailing pad.
        let table_width = natural.iter().sum::<usize>() + 3 * (natural.len() - 1);
        let table = self.tables.len();
        self.tables.push(Table {
            aligns: t.aligns,
            natural,
        });
        for (line, head, cells) in rows {
            if head {
                self.underline_pad
                    .insert(line, (self.look.header(self.look.base), table_width));
            }
            self.skip[line] = false;
            self.table_rows
                .insert(line, TableRow { table, head, cells });
        }
    }

    /// The display rows of one table row at `width`: each cell wrapped inside
    /// its column, the bars on every row, and a header underlined on its last
    /// row only.
    fn table_lines(&self, row: &TableRow, width: usize) -> Vec<Runs> {
        let table = &self.tables[row.table];
        let widths = fit_columns(&table.natural, width);
        let cells: Vec<Vec<Runs>> = widths
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let cell = row.cells.get(i).cloned().unwrap_or_default();
                wrap(&cell, w, &[]).into_iter().map(trim_end).collect()
            })
            .collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(1);
        let (base, border) = (self.look.base, self.look.border);
        let mut lines: Vec<Runs> = (0..height)
            .map(|k| {
                let mut out: Runs = Vec::new();
                for (i, (&width, cell)) in widths.iter().zip(&cells).enumerate() {
                    if i > 0 {
                        push_str(&mut out, base, " ");
                        push_str(&mut out, border, "│");
                        push_str(&mut out, base, " ");
                    }
                    let text = cell.get(k).cloned().unwrap_or_default();
                    let pad = width.saturating_sub(runs_width(&text));
                    let (left, right) = match table.aligns.get(i) {
                        Some(Alignment::Right) => (pad, 0),
                        Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    push_spaces(&mut out, base, left);
                    out.extend(text);
                    // Trailing pad on the last column only makes rows wrap.
                    if i + 1 < widths.len() {
                        push_spaces(&mut out, base, right);
                    }
                }
                out
            })
            .collect();
        if row.head
            && let Some(last) = lines.last_mut()
        {
            for (style, _) in last {
                *style = self.look.header(*style);
            }
        }
        lines
    }

    /// Bullets become `•` `◦` `▪` by depth; ordered numbers stay.
    fn item(&mut self, r: Range<usize>, depth: usize) {
        let start = r.start + self.run_len(r.start, ' ');
        let Some(marker) = self.src[start..].chars().next() else {
            return;
        };
        let bullet = self.look.bullet;
        if matches!(marker, '-' | '*' | '+') {
            let glyph = ["•", "◦", "▪"][depth.saturating_sub(1) % 3];
            self.replace(start..start + 1, vec![(bullet, glyph.to_string())]);
        } else {
            let digits = self.src[start..]
                .chars()
                .take_while(char::is_ascii_digit)
                .count();
            self.paint(start..start + digits + 1, bullet);
        }
    }

    /// Every `>` marker becomes a `▎` bar. A GitHub alert's `[!KIND]` tag
    /// becomes a label, and label and bars take the alert's colour.
    fn quote(&mut self, r: Range<usize>, kind: Option<BlockQuoteKind>) {
        let alert = kind.map(alert_label);
        let bar_style = alert.map_or(self.look.quote_bar, |(style, _)| style);
        let bar = vec![(bar_style, "▎".to_string())];
        self.paint(r.clone(), self.look.quote_text);
        for line in self.lines_of(&r) {
            // The first line's markers start at the quote, past any list marker.
            let from = self.line_range(line).start.max(r.start);
            let end = self.line_range(line).end;
            for (i, ch) in self.src[from..end].char_indices() {
                match ch {
                    '>' => self.replace(from + i..from + i + 1, bar.clone()),
                    ' ' => {}
                    _ => break,
                }
            }
        }
        if let Some((style, label)) = alert
            && let Some(open) = self.src[r.clone()].find("[!")
            && let Some(close) = self.src[r.start + open..r.end].find(']')
        {
            let tag = r.start + open..r.start + open + close + 1;
            let line = self.line_of(tag.start);
            self.no_reflow[line] = true;
            self.replace(tag, vec![(style, label.to_string())]);
        }
    }

    fn heading(&mut self, level: HeadingLevel, r: Range<usize>) {
        let style = self.look.heading(level);
        let lines = self.lines_of(&r);
        let (first, last) = (*lines.start(), *lines.end());
        let setext = !self.src[r.start..].starts_with('#');
        // A setext heading's last line is its `===` or `---` underline.
        let text_last = if setext && last > first {
            last - 1
        } else {
            first
        };
        if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
            self.underline_pad.insert(text_last, (style, usize::MAX));
        }
        if setext {
            for line in first..=text_last {
                self.no_reflow[line] = true;
                self.paint(self.line_range(line), style);
            }
            if text_last < last {
                self.skip[last] = true;
            }
            return;
        }
        let line = first;
        self.no_reflow[line] = true;
        let hashes = self.run_len(r.start, '#');
        let marker_end = r.start + hashes + self.run_len(r.start + hashes, ' ');
        let line_end = self.line_range(line).end.min(r.end);
        let marker_end = marker_end.min(line_end);
        let text = self.src[marker_end..line_end].trim_end();
        // An optional closing `#` run counts only after a space (`# C#` keeps it).
        let open = text.trim_end_matches('#');
        let text = if open.len() < text.len() && (open.is_empty() || open.ends_with(' ')) {
            open.trim_end()
        } else {
            text
        };
        let content_end = marker_end + text.len();
        self.paint(marker_end..content_end, style);
        self.replace(r.start..marker_end, vec![]);
        self.replace(content_end..line_end, vec![]);
    }

    /// Replace each `<details>` section's header with a summary row and its
    /// closing line with nothing, and hide the body of each collapsed one.
    /// `toggled` names the sections that render opposite to their default:
    /// collapsed, or expanded when the tag carries `open`.
    fn fold_details(&mut self, toggled: &BTreeSet<usize>) {
        let sections = details::sections(self.src, &self.html_blocks);
        let by_id: HashMap<usize, (Option<usize>, bool)> = sections
            .iter()
            .map(|s| (s.id, (s.parent, s.open != toggled.contains(&s.id))))
            .collect();
        // A section inside a collapsed one shows no summary row.
        let under_collapsed = |mut parent: Option<usize>| {
            while let Some(&(up, expanded)) = parent.and_then(|id| by_id.get(&id)) {
                if !expanded {
                    return true;
                }
                parent = up;
            }
            false
        };
        for section in &sections {
            let header = self.lines_of(&section.header);
            let (first, last) = (*header.start(), *header.end());
            let close = self.line_of(section.close).max(last);
            let expanded = by_id[&section.id].1;
            for line in (first..=last).chain([close]) {
                self.skip[line] = true;
            }
            if !expanded {
                for line in last + 1..=close {
                    self.skip[line] = true;
                    self.folded[line] = true;
                }
            }
            if under_collapsed(section.parent) {
                continue;
            }
            self.summaries.insert(
                first,
                Summary {
                    id: section.id,
                    lines: first..last + 1,
                    label: section.label.clone(),
                    expanded,
                },
            );
        }
    }

    /// `▸ label` collapsed or `▾ label` expanded, wrapped under the label's start.
    fn summary_rows(&self, summary: &Summary, width: usize) -> Vec<BlockRow> {
        let runs = vec![
            (
                self.look.bullet,
                if summary.expanded { "▾" } else { "▸" }.to_string(),
            ),
            (self.look.base, format!(" {}", summary.label)),
        ];
        let indent = [(self.look.base, "  ".to_string())];
        wrap(&runs, width, &indent)
            .into_iter()
            .map(|row| BlockRow {
                line: to_line(row),
                source: summary.lines.clone(),
                details: Some(summary.id),
            })
            .collect()
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte range of `line`, without its `\n` (or a CRLF body's `\r\n`).
    fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.line_starts[line];
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.src.len(), |next| next - 1);
        let end = if self.src[start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        start..end
    }

    /// The rendered runs of source bytes `a..b`, replacements applied.
    fn emit(&self, a: usize, b: usize) -> Runs {
        let mut out: Runs = Vec::new();
        let mut at = a;
        while at < b {
            if let Some((end, runs)) = self.repl.get(&at) {
                for (style, text) in runs {
                    push_str(&mut out, *style, text);
                }
                at = (*end).max(at + 1);
                continue;
            }
            if let Some((end, text)) = self.decoded.get(&at) {
                push_str(&mut out, self.styles[at], text);
                at = (*end).max(at + 1);
                continue;
            }
            let Some(ch) = self.src[at..].chars().next() else {
                break;
            };
            push_str(&mut out, self.styles[at], ch.encode_utf8(&mut [0; 4]));
            at += ch.len_utf8();
        }
        out
    }

    /// Byte where a line's prose starts: past indentation and quote markers,
    /// and with `past_marker`, past a list marker and task box too.
    fn content_start(&self, line: usize, past_marker: bool) -> usize {
        let r = self.line_range(line);
        let s = &self.src[r.clone()];
        let mut at = s.len() - s.trim_start_matches([' ', '>']).len();
        if past_marker {
            let rest = &s[at..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if ["- ", "* ", "+ "].iter().any(|m| rest.starts_with(m)) {
                at += 2;
            } else if digits > 0 && [". ", ") "].iter().any(|m| rest[digits..].starts_with(m)) {
                at += digits + 2;
            }
            at = s.len() - s[at..].trim_start_matches([' ', '>']).len();
            if ["[ ] ", "[x] ", "[X] "]
                .iter()
                .any(|m| s[at..].starts_with(m))
            {
                at += 4;
            }
        }
        r.start + at
    }

    /// The runs continuation rows of `line` start with: its lead (gutter,
    /// quote bars, list marker) with everything but the bars blanked.
    fn continuation(&self, line: usize) -> Runs {
        let mut lead = self.prefixes.get(&line).cloned().unwrap_or_default();
        lead.extend(self.emit(self.line_range(line).start, self.content_start(line, true)));
        lead.into_iter()
            .map(|(style, text)| {
                let kept = text
                    .chars()
                    .map(|c| if matches!(c, '│' | '▎') { c } else { ' ' })
                    .collect();
                (style, kept)
            })
            .collect()
    }

    fn is_blank(&self, line: usize) -> bool {
        self.src[self.line_range(line)].trim().is_empty()
    }

    /// Whether `next` continues the paragraph that `line` belongs to.
    fn joins(&self, line: usize, next: usize, apart: &[Option<Range<usize>>]) -> bool {
        next < self.line_count()
            && apart[line] == apart[next]
            && !self.no_reflow[line]
            && !self.no_reflow[next]
            && !self.skip[next]
            && !self.block_start[next]
            && !self.is_blank(line)
            && !self.is_blank(next)
    }

    /// Rows for every line. A row's source is its group's lines, widened to
    /// the whole `apart` range the group sits in.
    fn rows(&self, width: usize, apart: &[Option<Range<usize>>]) -> Vec<BlockRow> {
        let mut out = Vec::new();
        let mut line = 0;
        while line < self.line_count() {
            if let Some(summary) = self.summaries.get(&line) {
                out.extend(self.summary_rows(summary, width));
                line = summary.lines.end;
                continue;
            }
            if self.skip[line] {
                line += 1;
                continue;
            }
            let range = self.line_range(line);
            let mut logical = self.prefixes.get(&line).cloned().unwrap_or_default();
            if self.rules[line] {
                logical.push((self.look.border, "─".repeat(width)));
            } else if !self.table_rows.contains_key(&line) {
                logical.extend(self.emit(range.start, range.end));
            }
            let mut last = line;
            while self.joins(last, last + 1, apart) {
                last += 1;
                push_str(&mut logical, self.look.base, " ");
                logical
                    .extend(self.emit(self.content_start(last, false), self.line_range(last).end));
            }
            let cont = self.continuation(line);
            let mut rows: Vec<Runs> = match self.table_rows.get(&line) {
                // Table rows wrap again only when the table cannot give each
                // column one cell of room.
                Some(row) => self
                    .table_lines(row, width)
                    .iter()
                    .flat_map(|l| wrap(l, width, &cont))
                    .collect(),
                None => wrap(&logical, width, &cont),
            };
            if let (Some(label), Some(first)) = (self.labels.get(&line), rows.first_mut()) {
                let gap = width.saturating_sub(runs_width(first) + runs_width(label));
                if gap > 0 {
                    push_str(first, self.look.base, &" ".repeat(gap));
                    first.extend(label.iter().cloned());
                }
            }
            if let (Some(&(style, to)), Some(row)) =
                (self.underline_pad.get(&line), rows.last_mut())
            {
                let gap = to.min(width).saturating_sub(runs_width(row));
                push_spaces(row, style, gap);
            }
            let source = apart[line].clone().unwrap_or(line..last + 1);
            for row in rows {
                out.push(BlockRow {
                    line: to_line(row),
                    source: source.clone(),
                    details: None,
                });
            }
            line = last + 1;
        }
        out
    }
}

/// Drop leading and trailing blank rows and collapse runs of them to one.
fn trim_blank_rows(rows: Vec<BlockRow>) -> Vec<BlockRow> {
    let blank = |row: &BlockRow| row.line.spans.iter().all(|s| s.content.trim().is_empty());
    let mut out: Vec<BlockRow> = Vec::with_capacity(rows.len());
    for row in rows {
        if blank(&row) && out.last().is_none_or(blank) {
            continue;
        }
        out.push(row);
    }
    if out.last().is_some_and(blank) {
        out.pop();
    }
    out
}

fn push_str(out: &mut Runs, style: Style, s: &str) {
    match out.last_mut() {
        Some((prev, text)) if *prev == style => text.push_str(s),
        _ => out.push((style, s.to_string())),
    }
}

fn push_spaces(out: &mut Runs, style: Style, n: usize) {
    if n > 0 {
        push_str(out, style, &" ".repeat(n));
    }
}

/// Column widths for a table drawn in `width` columns: the natural widths
/// when they fit, else the widest columns shrink first, down to one column
/// each, and narrow columns keep their natural width.
fn fit_columns(natural: &[usize], width: usize) -> Vec<usize> {
    let room = width.saturating_sub(3 * natural.len().saturating_sub(1));
    let total = |cap: usize| natural.iter().map(|&w| w.min(cap)).sum::<usize>();
    let widest = natural.iter().copied().max().unwrap_or(1);
    if total(widest) <= room {
        return natural.to_vec();
    }
    let cap = (1..widest)
        .rev()
        .find(|&cap| total(cap) <= room)
        .unwrap_or(1);
    // Less than one column per capped column is left, so each gets at most one.
    let mut spare = room.saturating_sub(total(cap));
    natural
        .iter()
        .map(|&w| {
            if w > cap && spare > 0 {
                spare -= 1;
                cap + 1
            } else {
                w.min(cap)
            }
        })
        .collect()
}

/// `runs` without the trailing spaces a wrapped row can end in.
fn trim_end(mut runs: Runs) -> Runs {
    while let Some((_, text)) = runs.last_mut() {
        let kept = text.trim_end_matches(' ').len();
        if kept > 0 {
            text.truncate(kept);
            break;
        }
        runs.pop();
    }
    runs
}

fn runs_width(runs: &[(Style, String)]) -> usize {
    runs.iter().map(|(_, t)| t.width()).sum()
}

/// Word-wrap styled runs to `width`, starting continuation rows with `cont`.
/// A word wider than a whole row is split by character.
fn wrap(runs: &Runs, width: usize, cont: &[(Style, String)]) -> Vec<Runs> {
    // A hanging indent wider than half the row would leave too little room
    // for text, so narrow rows drop it.
    let cont: &[(Style, String)] = if runs_width(cont) * 2 > width {
        &[]
    } else {
        cont
    };
    let cont_w = runs_width(cont);
    // A word can span style changes (`**gone**.` is one word), so each token is
    // itself a list of runs.
    let mut tokens: Vec<(bool, Runs)> = Vec::new();
    for (style, text) in runs {
        for ch in text.chars() {
            let space = ch == ' ';
            let piece = ch.encode_utf8(&mut [0; 4]).to_owned();
            match tokens.last_mut() {
                Some((was_space, token)) if *was_space == space => push_str(token, *style, &piece),
                _ => tokens.push((space, vec![(*style, piece)])),
            }
        }
    }

    let mut rows: Vec<Runs> = Vec::new();
    let mut row: Runs = Vec::new();
    let mut row_w = 0;
    // Width of the row's hanging indent: a row holding only that is empty.
    let mut row_base = 0;
    for (space, token) in tokens {
        let w = runs_width(&token);
        if row_w + w > width && row_w > row_base {
            rows.push(std::mem::replace(&mut row, cont.to_vec()));
            (row_w, row_base) = (cont_w, cont_w);
            if space {
                continue;
            }
        }
        if row_w + w <= width {
            for (style, text) in &token {
                push_str(&mut row, *style, text);
            }
            row_w += w;
            continue;
        }
        for (style, text) in token {
            for ch in text.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if row_w + cw > width && row_w > row_base {
                    rows.push(std::mem::replace(&mut row, cont.to_vec()));
                    (row_w, row_base) = (cont_w, cont_w);
                }
                push_str(&mut row, style, ch.encode_utf8(&mut [0; 4]));
                row_w += cw;
            }
        }
    }
    rows.push(row);
    rows
}

/// Orca's terminal (xterm.js) draws an underline white after SGR 59, which
/// ratatui sends at the end of every frame. An explicit underline colour
/// equal to the text colour sidesteps it.
fn pin_underline(style: Style) -> Style {
    match style.fg {
        Some(fg)
            if style.add_modifier.contains(Modifier::UNDERLINED)
                && style.underline_color.is_none() =>
        {
            style.underline_color(fg)
        }
        _ => style,
    }
}

fn to_line(runs: Runs) -> Line<'static> {
    Line::from(
        runs.into_iter()
            .map(|(style, text)| Span::styled(text.replace(NBSP, " "), pin_underline(style)))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn render(src: &str, width: usize) -> Vec<BlockRow> {
        render_block(&Theme::dark(), src, width, &[], &BTreeSet::new())
    }

    fn text(row: &BlockRow) -> String {
        row.line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn should_render_no_rows_for_empty_or_blank_source() {
        assert!(render("", 40).is_empty());
        assert!(render("\n \n", 40).is_empty());
    }

    #[test]
    fn should_keep_a_line_that_exactly_fits_the_width_on_one_row() {
        let rows = render("aaaa bbbb", 9);

        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "aaaa bbbb");
    }

    #[test]
    fn should_wrap_only_the_word_that_overflows_the_width() {
        let rows = render("aaaa bbbb cc", 9);

        let texts: Vec<String> = rows
            .iter()
            .map(|r| text(r).trim_end().to_string())
            .collect();
        assert_eq!(texts, ["aaaa bbbb", "cc"]);
    }

    #[test]
    fn should_reflow_paragraph_lines_into_one_row() {
        let rows = render("one\ntwo\nthree", 40);

        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "one two three");
        assert_eq!(rows[0].source, 0..3);
    }

    fn texts_trimmed(rows: &[BlockRow]) -> Vec<String> {
        rows.iter()
            .map(|r| text(r).trim_end().to_string())
            .collect()
    }

    #[test]
    fn should_keep_one_blank_row_between_blocks_and_drop_edge_blanks() {
        let rows = render("# Title\n\nPara one.\n\n\n\nPara two.", 40);
        assert_eq!(
            texts_trimmed(&rows),
            ["Title", "", "Para one.", "", "Para two."]
        );

        let rows = render("\n\nText\n\n", 40);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "Text");
    }

    fn spans_all(row: &BlockRow, pred: impl Fn(&Span) -> bool) -> bool {
        row.line.spans.iter().all(pred)
    }

    fn has(span: &Span, modifier: Modifier) -> bool {
        span.style.add_modifier.contains(modifier)
    }

    #[test]
    fn should_underline_h1_and_h2_across_the_width_without_markers() {
        let rows = render("# Summary\n## Details", 20);

        assert_eq!(rows.len(), 2);
        assert_eq!(text(&rows[0]), format!("Summary{}", " ".repeat(13)));
        assert!(spans_all(&rows[0], |s| has(s, Modifier::BOLD)
            && has(s, Modifier::UNDERLINED)));
        assert_eq!(text(&rows[1]), format!("Details{}", " ".repeat(13)));
        assert!(spans_all(&rows[1], |s| has(s, Modifier::UNDERLINED)
            && !has(s, Modifier::BOLD)));
    }

    fn single_row(src: &str, width: usize) -> BlockRow {
        let mut rows = render(src, width);
        assert_eq!(rows.len(), 1, "rows for {src:?}");
        rows.remove(0)
    }

    #[test]
    fn should_style_h3_to_h6_down_the_ladder() {
        let three = single_row("### Three", 40);
        assert_eq!(text(&three), "Three");
        assert!(spans_all(&three, |s| has(s, Modifier::BOLD)
            && !has(s, Modifier::UNDERLINED)));

        let four = single_row("#### Four", 40);
        assert_eq!(text(&four), "Four");
        assert!(spans_all(&four, |s| has(s, Modifier::UNDERLINED)));

        let five = single_row("##### Five", 40);
        assert_eq!(text(&five), "Five");
        assert!(spans_all(&five, |s| has(s, Modifier::ITALIC)
            && !has(s, Modifier::BOLD)
            && !has(s, Modifier::UNDERLINED)));

        let six = single_row("###### Six", 40);
        assert_eq!(text(&six), "Six");
        assert!(spans_all(&six, |s| has(s, Modifier::ITALIC)
            && s.style.fg == Some(Theme::dark().fg_dim)));
    }

    /// The spans of `row` whose text contains `needle`.
    fn spans_with<'a>(row: &'a BlockRow, needle: &str) -> Vec<&'a Span<'static>> {
        row.line
            .spans
            .iter()
            .filter(|s| s.content.contains(needle))
            .collect()
    }

    #[test]
    fn should_wrap_trailing_punctuation_with_its_word() {
        let rows = render("aaaa bbbb **gone**.", 13);
        assert_eq!(texts_trimmed(&rows), ["aaaa bbbb", "gone."]);
        let gone = spans_with(&rows[1], "gone");
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].content, "gone");
        assert!(has(gone[0], Modifier::BOLD));
        assert!(
            spans_with(&rows[1], ".")
                .iter()
                .all(|s| !has(s, Modifier::BOLD))
        );

        let rows = render("aaaa bbbb **gone**.", 40);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "aaaa bbbb gone.");
    }

    /// The style of each character of the first occurrence of `needle` in `row`.
    fn styles_of(row: &BlockRow, needle: &str) -> Vec<Style> {
        let chars: Vec<(char, Style)> = row
            .line
            .spans
            .iter()
            .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
            .collect();
        let want: Vec<char> = needle.chars().collect();
        let at = chars
            .windows(want.len())
            .position(|w| w.iter().map(|(c, _)| *c).eq(want.iter().copied()))
            .unwrap_or_else(|| panic!("{needle:?} not in {:?}", text(row)));
        chars[at..at + want.len()].iter().map(|(_, s)| *s).collect()
    }

    fn code_fg(theme: &Theme) -> Option<Color> {
        theme.syntax_highlighter().markdown_palette.code.fg
    }

    #[test]
    fn should_hide_inline_markers_and_style_their_text() {
        let theme = Theme::dark();
        let row = single_row("**bold** _it_ ~~gone~~ [link](https://x.test/p) `code`", 80);

        let text = text(&row);
        assert_eq!(text, "bold it gone link  code ");
        for hidden in ["*", "_", "~", "[", "]", "(", ")", "https"] {
            assert!(!text.contains(hidden), "{hidden:?} shows in {text:?}");
        }
        assert!(
            styles_of(&row, "bold")
                .iter()
                .all(|s| s.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            styles_of(&row, "it")
                .iter()
                .all(|s| s.add_modifier.contains(Modifier::ITALIC))
        );
        assert!(
            styles_of(&row, "gone")
                .iter()
                .all(|s| s.add_modifier.contains(Modifier::CROSSED_OUT))
        );
        assert!(styles_of(&row, "link").iter().all(|s| {
            s.add_modifier.contains(Modifier::UNDERLINED) && s.underline_color == s.fg
        }));
        assert!(
            styles_of(&row, " code ")
                .iter()
                .all(|s| s.bg == Some(GLOW_CHIP_BG))
        );
        assert!(
            styles_of(&row, "code")
                .iter()
                .all(|s| s.fg == Some(GLOW_CHIP_FG))
        );
        assert_ne!(code_fg(&theme), Some(GLOW_CHIP_FG));
    }

    fn row_width(row: &BlockRow) -> usize {
        row.line.width()
    }

    #[test]
    fn should_draw_code_block_with_gutter_and_language_tag_and_no_fences() {
        let theme = Theme::dark();
        let rows = render("```rust\nfn main() {\n    let x = 1;\n}\n```", 40);

        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert!(!text(row).contains("```"));
            assert!(text(row).starts_with("│ "), "{:?}", text(row));
            assert_eq!(styles_of(row, "│")[0].fg, Some(theme.border_unfocused));
        }
        assert_eq!(row_width(&rows[0]), 40);
        assert!(text(&rows[0]).starts_with("│ fn main() {"));
        assert!(text(&rows[0]).ends_with("rust"));
        assert!(
            styles_of(&rows[0], "rust")
                .iter()
                .all(|s| s.fg == Some(theme.fg_dim))
        );
        assert_eq!(text(&rows[1]).trim_end(), "│     let x = 1;");
        assert_eq!(text(&rows[2]).trim_end(), "│ }");
        assert_ne!(
            styles_of(&rows[0], "fn")[0].fg,
            styles_of(&rows[0], "main")[0].fg,
            "syntect should colour rust"
        );
    }

    fn texts(rows: &[BlockRow]) -> Vec<String> {
        rows.iter().map(text).collect()
    }

    #[test]
    fn should_draw_table_with_inner_bars_and_underlined_header_in_place_of_delimiter() {
        let border = Some(Theme::dark().border_unfocused);
        let rows = render(
            "| Name | Qty |\n| --- | ---: |\n| apple | 3 |\n| kiwi | 12 |",
            40,
        );

        assert_eq!(texts(&rows), ["Name  │ Qty", "apple │   3", "kiwi  │  12"]);
        for row in &rows {
            let text = text(row);
            assert!(!text.contains('-') && !text.contains('|'), "{text:?}");
            assert!(!text.starts_with('│') && !text.ends_with('│'), "{text:?}");
            for span in row.line.spans.iter().filter(|s| s.content.contains('│')) {
                assert_eq!(span.style.fg, border);
            }
        }
        assert!(spans_all(&rows[0], |s| has(s, Modifier::UNDERLINED)
            && s.style.underline_color == border));
        for cell in ["Name", "Qty"] {
            assert!(
                styles_of(&rows[0], cell)
                    .iter()
                    .all(|s| s.add_modifier.contains(Modifier::BOLD))
            );
        }
    }

    #[test]
    fn should_underline_header_to_panel_edge_without_blank_row_when_table_is_wider() {
        let border = Some(Theme::dark().border_unfocused);
        let rows = render(
            "| Long header cell | B |\n| --- | --- |\n| x | a much longer body cell that overflows |",
            40,
        );

        assert_eq!(text(&rows[0]).trim_end(), "Long header cell │ B");
        assert_eq!(row_width(&rows[0]), 40);
        assert!(spans_all(&rows[0], |s| has(s, Modifier::UNDERLINED)
            && s.style.underline_color == border));
        assert!(text(&rows[1]).starts_with("x "), "{:?}", texts(&rows));
        assert!(rows.iter().all(|row| row_width(row) <= 40));
    }

    /// Display columns of each `│` in `row`.
    fn bar_columns(row: &BlockRow) -> Vec<usize> {
        let mut at = 0;
        let mut bars = Vec::new();
        for ch in text(row).chars() {
            if ch == '│' {
                bars.push(at);
            }
            at += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        bars
    }

    #[test]
    fn should_wrap_cells_within_their_columns_when_table_is_wider_than_width() {
        let rows = render(
            "| Step | Owner | State | Notes |\n| --- | --- | --- | --- |\n\
             | Move the nightly export job onto the shared scheduler and retire the old cron entry | ops | done | ok |",
            40,
        );

        assert_eq!(
            texts_trimmed(&rows),
            [
                "Step             │ Owner │ State │ Notes",
                "Move the nightly │ ops   │ done  │ ok",
                "export job onto  │       │       │",
                "the shared       │       │       │",
                "scheduler and    │       │       │",
                "retire the old   │       │       │",
                "cron entry       │       │       │",
            ]
        );
        assert!(rows.iter().all(|row| bar_columns(row) == [17, 25, 33]));
        assert!(rows.iter().all(|row| row_width(row) <= 40));
    }

    #[test]
    fn should_underline_only_the_last_row_of_a_wrapped_header_across_its_bars() {
        let border = Some(Theme::dark().border_unfocused);
        let rows = render("| Alpha beta gamma | B |\n| --- | --- |\n| x | y |", 16);

        assert_eq!(
            texts(&rows),
            ["Alpha beta   │ B", "gamma        │  ", "x            │ y"]
        );
        for cell in ["Alpha beta", "gamma", "B"] {
            let row = rows.iter().find(|r| text(r).contains(cell)).unwrap();
            assert!(
                styles_of(row, cell)
                    .iter()
                    .all(|s| s.add_modifier.contains(Modifier::BOLD))
            );
        }
        assert!(spans_all(&rows[0], |s| !has(s, Modifier::UNDERLINED)));
        assert!(spans_all(&rows[1], |s| has(s, Modifier::UNDERLINED)
            && s.style.underline_color == border));
        assert!(spans_all(&rows[2], |s| !has(s, Modifier::UNDERLINED)));
    }

    #[test]
    fn should_keep_narrow_columns_at_natural_width_and_shrink_the_widest_first() {
        assert_eq!(fit_columns(&[4, 5, 5, 5], 40), [4, 5, 5, 5]);
        assert_eq!(fit_columns(&[83, 5, 5, 5], 40), [16, 5, 5, 5]);
        assert_eq!(fit_columns(&[30, 20, 2], 32), [12, 12, 2]);
        assert_eq!(fit_columns(&[30, 21, 2], 33), [13, 12, 2]);
        assert_eq!(fit_columns(&[9, 9, 9], 4), [1, 1, 1]);
    }

    #[test]
    fn should_wrap_table_rows_again_when_one_cell_per_column_overflows() {
        let rows = render("| aaaa | bbbb | cccc |\n|---|---|---|\n| x | y | z |", 4);

        let texts: Vec<String> = rows.iter().map(text).collect();
        assert!(rows.iter().all(|row| row_width(row) <= 4), "{texts:?}");
        let cells: String = texts
            .concat()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        assert_eq!(cells, "abcabcabcabcxyz");
        assert_eq!(texts.concat().matches('│').count(), 2 * 5, "{texts:?}");
    }

    #[test]
    fn should_mark_list_items_by_depth_and_task_state() {
        let rows = render(
            "- one\n  - two\n    - three\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo",
            40,
        );

        assert_eq!(
            texts(&rows),
            [
                "• one",
                "  ◦ two",
                "    ▪ three",
                "",
                "1. first",
                "2. second",
                "",
                "• ☑ done",
                "• ☐ todo",
            ]
        );
    }

    #[test]
    fn should_hang_wrapped_list_item_under_its_text() {
        let rows = render("- a long item that wraps", 12);

        assert_eq!(texts_trimmed(&rows), ["• a long", "  item that", "  wraps"]);
    }

    #[test]
    fn should_repeat_quote_bar_on_wrapped_rows() {
        let rows = render("> quoted text that wraps around", 14);

        assert_eq!(
            texts_trimmed(&rows),
            ["▎ quoted text", "▎ that wraps", "▎ around"]
        );
    }

    #[test]
    fn should_replace_alert_tag_with_coloured_label_and_bar() {
        let rows = render("> [!WARNING]\n> Mind the gap.", 40);

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !text(row).contains("[!")));
        assert!(
            styles_of(&rows[0], "Warning")
                .iter()
                .all(|s| s.fg == Some(Color::Yellow))
        );
        for row in &rows {
            assert!(text(row).starts_with('▎'));
            assert_eq!(styles_of(row, "▎")[0].fg, Some(Color::Yellow));
        }
        assert_eq!(text(&rows[1]).trim_end(), "▎ Mind the gap.");
    }

    #[test]
    fn should_draw_rule_across_the_width_in_border_colour() {
        let rows = render("a\n\n---\n\nb", 10);

        assert_eq!(texts(&rows), ["a", "", "──────────", "", "b"]);
        assert!(
            styles_of(&rows[2], "─")
                .iter()
                .all(|s| s.fg == Some(Theme::dark().border_unfocused))
        );
    }

    #[test]
    fn should_keep_raw_html_as_dimmed_text_line_for_line() {
        let src = "<div>\n<p>More</p>\n</div>";
        let rows = render(src, 40);

        assert_eq!(texts(&rows), src.split('\n').collect::<Vec<_>>());
        for row in &rows {
            assert!(spans_all(row, |s| s.style.fg == Some(Theme::dark().fg_dim)));
        }
    }

    #[test]
    fn should_show_image_as_glyph_and_alt_text() {
        let row = single_row("see ![a](https://x.test/1.png) end", 40);

        assert_eq!(text(&row), "see ▣ a end");
    }

    fn text_and_source(rows: &[BlockRow]) -> Vec<(String, Range<usize>)> {
        rows.iter().map(|r| (text(r), r.source.clone())).collect()
    }

    #[test]
    fn should_not_join_kept_apart_lines_to_their_neighbours() {
        let src = "Before:\n![img](https://x.test/i.png)\nAfter:";
        let theme = Theme::dark();

        let rows = render_block(
            &theme,
            src,
            40,
            std::slice::from_ref(&(1..2)),
            &BTreeSet::new(),
        );
        assert_eq!(
            text_and_source(&rows),
            [
                ("Before:".to_string(), 0..1),
                ("▣ img".to_string(), 1..2),
                ("After:".to_string(), 2..3),
            ]
        );

        let rows = render_block(&theme, src, 40, &[], &BTreeSet::new());
        assert_eq!(
            text_and_source(&rows),
            [("Before: ▣ img After:".to_string(), 0..3)]
        );
    }

    #[test]
    fn should_give_a_kept_apart_range_that_renders_nothing_an_anchor_row() {
        let rows = render_block(
            &Theme::dark(),
            "```\ncode\n```",
            40,
            std::slice::from_ref(&(0..1)),
            &BTreeSet::new(),
        );

        let sources: Vec<Range<usize>> = rows.iter().map(|r| r.source.clone()).collect();
        assert_eq!(sources, [0..1, 1..2]);
    }

    #[test]
    fn should_derive_chip_and_border_colours_from_a_light_theme() {
        let light = Theme::light();

        let chip = render_block(&light, "`x`", 40, &[], &BTreeSet::new());
        for s in styles_of(&chip[0], " x ") {
            assert_eq!(s.bg, Some(light.bg_highlight));
            assert_ne!(s.bg, Some(light.panel_bg));
            assert!(s.fg.is_some() && s.fg != s.bg, "{s:?}");
        }

        let rule = render_block(&light, "---", 40, &[], &BTreeSet::new());
        assert!(
            styles_of(&rule[0], "─")
                .iter()
                .all(|s| s.fg == Some(light.border_unfocused))
        );
    }

    /// Sources whose rows are all prose, so none may exceed the width.
    const PROSE: &[&str] = &[
        "# Summary\n## Details",
        "### Three",
        "#### Four",
        "##### Five",
        "###### Six",
        "# Title\n\nPara one.\n\n\n\nPara two.",
        "\n\nText\n\n",
        "",
        "\n \n",
        "one\ntwo\nthree",
        "aaaa bbbb **gone**.",
        "**bold** _it_ ~~gone~~ [link](https://x.test/p) `code`",
        "- one\n  - two\n    - three\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo",
        "- a long item that wraps",
        "> quoted text that wraps around",
        "> [!WARNING]\n> Mind the gap.",
        "a\n\n---\n\nb",
        "<details>\n<summary>More</summary>\n</details>",
        "see ![a](https://x.test/1.png) end",
        "Before:\n![img](https://x.test/i.png)\nAfter:",
    ];

    /// Prose of double-width characters. A 1-column row cannot hold one, so it
    /// is checked only from width 8 up.
    const CJK: &str = "日本語 `コード` です";

    /// Sources with table or code rows, which wrap past the width with their
    /// continuation prefix.
    const WIDE: &[&str] = &[
        "```rust\nfn main() {\n    let x = 1;\n}\n```",
        "| Name | Qty |\n| --- | ---: |\n| apple | 3 |\n| kiwi | 12 |",
        "`x`",
        "---",
    ];

    #[test]
    fn should_hold_width_and_underline_invariants_at_any_width() {
        for theme in [Theme::dark(), Theme::light()] {
            for width in [1, 8, 40] {
                let cjk = (width > 1).then_some((&CJK, true));
                for (src, prose) in PROSE
                    .iter()
                    .map(|s| (s, true))
                    .chain(WIDE.iter().map(|s| (s, false)))
                    .chain(cjk)
                {
                    for keep_apart in [&[][..], std::slice::from_ref(&(1..2))] {
                        for row in render_block(&theme, src, width, keep_apart, &BTreeSet::new()) {
                            if prose {
                                assert!(
                                    row_width(&row) <= width,
                                    "{src:?} at width {width}: {:?}",
                                    text(&row)
                                );
                            }
                            for span in &row.line.spans {
                                if has(span, Modifier::UNDERLINED) {
                                    assert!(
                                        span.style.underline_color.is_some(),
                                        "{src:?}: {span:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn should_hide_setext_heading_underline() {
        let rows = render("Summary\n=======\n\nBody", 20);

        assert_eq!(texts_trimmed(&rows), ["Summary", "", "Body"]);
        assert_eq!(text(&rows[0]), format!("Summary{}", " ".repeat(13)));
    }

    #[test]
    fn should_hide_the_link_around_a_linked_image() {
        let row = single_row("[![CI](https://x.test/b.svg)](https://x.test/run)", 40);

        assert_eq!(text(&row), "▣ CI");
    }

    #[test]
    fn should_pad_heading_in_heading_style_after_trailing_chip() {
        let row = single_row("# Use `foo`", 20);

        let pad = row.line.spans.last().expect("padding span");
        assert!(pad.content.trim().is_empty());
        assert_ne!(pad.style.bg, Some(GLOW_CHIP_BG));
        assert!(has(pad, Modifier::BOLD) && has(pad, Modifier::UNDERLINED));
    }

    #[test]
    fn should_move_chip_that_lands_on_wrap_boundary_to_next_row_with_its_pads() {
        let rows = render("see and `inline_code()` here", 16);

        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(texts, ["see and ", " inline_code()  ", "here"]);
        assert!(
            rows[0]
                .line
                .spans
                .iter()
                .all(|s| s.style.bg != Some(GLOW_CHIP_BG))
        );
        assert!(
            styles_of(&rows[1], " inline_code() ")
                .iter()
                .all(|s| s.bg == Some(GLOW_CHIP_BG))
        );
    }

    #[test]
    fn should_hide_closing_hashes_of_atx_heading() {
        let row = single_row("# Title #", 20);

        assert_eq!(text(&row), format!("Title{}", " ".repeat(15)));
    }

    #[test]
    fn should_render_reference_link_and_drop_its_definition() {
        let row = single_row("See [the docs][d].\n\n[d]: https://x.test/docs", 40);

        assert_eq!(text(&row), "See the docs.");
        assert!(
            styles_of(&row, "the docs")
                .iter()
                .all(|s| s.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn should_show_escaped_characters_and_entities_decoded() {
        let row = single_row("snake\\_case \\*not bold\\* a &amp; b \\\\ c", 60);

        assert_eq!(text(&row), "snake_case *not bold* a & b \\ c");
    }

    #[test]
    fn should_style_an_entity_inside_link_text_as_the_link() {
        let row = single_row("[Tom &amp; Jerry](https://x.test)", 40);

        assert_eq!(text(&row), "Tom & Jerry");
        assert!(
            styles_of(&row, "&")
                .iter()
                .all(|s| s.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn should_keep_an_escaped_pipe_in_a_table_cell() {
        let rows = render("| a |\n|---|\n| x \\| |", 40);

        assert_eq!(texts_trimmed(&rows), ["a", "x |"]);
    }

    #[test]
    fn should_bar_a_quote_that_opens_on_a_list_item_line() {
        let rows = render("- > quoted words\n  > more", 10);

        let texts = texts_trimmed(&rows);
        assert_eq!(texts, ["• ▎ quoted", "  ▎ words", "  ▎ more"]);
    }

    #[test]
    fn should_draw_a_fence_that_opens_on_a_list_item_line() {
        let rows = render("- ```rust\n  let x = 1;\n  ```", 30);

        let texts = texts_trimmed(&rows);
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert_eq!(texts[0], "•");
        assert!(texts[1].starts_with("│   let x = 1;"), "{texts:?}");
        assert!(texts[1].ends_with("rust"), "{texts:?}");
    }

    #[test]
    fn should_hide_a_closing_fence_inside_a_quote() {
        let rows = render("> ```\n> a\n> ```", 30);

        assert_eq!(rows.len(), 1, "{:?}", texts(&rows));
    }

    #[test]
    fn should_keep_the_last_line_of_an_unclosed_fence_that_starts_with_the_fence_char() {
        assert_eq!(
            texts_trimmed(&render("```\n`foo` bar", 30)),
            ["│ `foo` bar"]
        );
        assert_eq!(
            texts_trimmed(&render("~~~\nfoo\n~ bar", 30)),
            ["│ foo", "│ ~ bar"]
        );
        assert_eq!(texts_trimmed(&render("````\nx\n```", 30)), ["│ x", "│ ```"]);
    }

    /// A table with a header and 40 rows, `| f00.rs | 0 |` to `| f39.rs | 39 |`.
    fn table() -> String {
        let mut lines = vec![
            "| File | Lines |".to_string(),
            "|------|-------|".to_string(),
        ];
        lines.extend((0..40).map(|i| format!("| f{i:02}.rs | {i} |")));
        lines.join("\n")
    }

    /// The table inside a `<details>` section whose opening tag is `open_tag`,
    /// with a summary of "Coverage report", between two paragraphs.
    fn coverage_with(open_tag: &str) -> String {
        [
            "Intro",
            "",
            open_tag,
            "<summary>Coverage report</summary>",
            "",
            &table(),
            "",
            "</details>",
            "",
            "Outro",
        ]
        .join("\n")
    }

    fn coverage() -> String {
        coverage_with("<details>")
    }

    fn render_toggled(src: &str, width: usize, toggled: &[usize]) -> Vec<BlockRow> {
        let toggled: BTreeSet<usize> = toggled.iter().copied().collect();
        render_block(&Theme::dark(), src, width, &[], &toggled)
    }

    fn details_of(rows: &[BlockRow]) -> Vec<Option<usize>> {
        rows.iter().map(|r| r.details).collect()
    }

    #[test]
    fn should_collapse_a_details_section_to_its_summary_row_by_default() {
        let rows = render_toggled(&coverage(), 40, &[]);

        assert_eq!(
            texts_trimmed(&rows),
            ["Intro", "", "▸ Coverage report", "", "Outro"]
        );
        assert_eq!(details_of(&rows), [None, None, Some(0), None, None]);
        assert_eq!(rows[2].source, 2..4);
    }

    /// Case 2's rows: `src` with the coverage section expanded.
    fn assert_coverage_expanded(rows: &[BlockRow]) {
        let table_rows = texts_trimmed(&render_toggled(&table(), 40, &[]));
        assert!(table_rows.len() >= 41, "table rows: {table_rows:?}");
        let mut expected: Vec<String> = ["Intro", "", "▾ Coverage report", ""]
            .map(String::from)
            .to_vec();
        expected.extend(table_rows);
        expected.extend(["", "Outro"].map(String::from));

        assert_eq!(texts_trimmed(rows), expected);
    }

    #[test]
    fn should_expand_a_toggled_section_to_render_its_body() {
        let rows = render_toggled(&coverage(), 40, &[0]);

        assert_coverage_expanded(&rows);
    }

    #[test]
    fn should_start_an_open_section_expanded_and_collapse_it_when_toggled() {
        let open = coverage_with("<details open>");

        let rows = render_toggled(&open, 40, &[]);
        assert_eq!(text(&rows[2]).trim_end(), "▾ Coverage report");
        assert_eq!(
            texts_trimmed(&rows),
            texts_trimmed(&render_toggled(&coverage(), 40, &[0]))
        );

        assert_eq!(
            texts_trimmed(&render_toggled(&open, 40, &[0])),
            texts_trimmed(&render_toggled(&coverage(), 40, &[]))
        );
    }

    #[test]
    fn should_read_open_as_an_attribute_not_a_substring() {
        let rows = render_toggled(&coverage_with(r#"<details class="opener">"#), 40, &[]);

        assert_eq!(text(&rows[2]).trim_end(), "▸ Coverage report");
    }

    #[test]
    fn should_toggle_nested_sections_independently_and_hide_the_inner_under_a_collapsed_outer() {
        let src = [
            "<details>",
            "<summary>Outer</summary>",
            "",
            "Outer body",
            "",
            "<details>",
            "<summary>Inner</summary>",
            "",
            "Inner body",
            "",
            "</details>",
            "",
            "</details>",
        ]
        .join("\n");

        assert_eq!(texts_trimmed(&render_toggled(&src, 40, &[])), ["▸ Outer"]);

        let rows = render_toggled(&src, 40, &[0]);
        assert_eq!(
            texts_trimmed(&rows),
            ["▾ Outer", "", "Outer body", "", "▸ Inner"]
        );
        assert_eq!(rows[4].details, Some(1));

        assert_eq!(
            texts_trimmed(&render_toggled(&src, 40, &[0, 1])),
            ["▾ Outer", "", "Outer body", "", "▾ Inner", "", "Inner body"]
        );
        assert_eq!(texts_trimmed(&render_toggled(&src, 40, &[1])), ["▸ Outer"]);
    }

    #[test]
    fn should_label_a_section_without_a_summary_details() {
        let rows = render_toggled("<details>\n\nBody\n\n</details>", 40, &[]);

        assert_eq!(texts_trimmed(&rows), ["▸ Details"]);
    }

    #[test]
    fn should_strip_tags_and_collapse_whitespace_in_the_summary_label() {
        let src = "<details><summary><b>Logs</b>  for   run</summary>\n\nBody\n\n</details>";

        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[])),
            ["▸ Logs for run"]
        );
    }

    #[test]
    fn should_label_a_section_with_an_empty_summary_details() {
        let src = "<details><summary> </summary>\n\nBody\n\n</details>";

        assert_eq!(texts_trimmed(&render_toggled(src, 40, &[])), ["▸ Details"]);
    }

    #[test]
    fn should_close_a_section_on_its_header_line_with_an_empty_body() {
        let src = "<details><summary>X</summary>hidden</details>\n\nAfter";

        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[])),
            ["▸ X", "", "After"]
        );
        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[0])),
            ["▾ X", "", "After"]
        );
    }

    #[test]
    fn should_show_the_next_sections_summary_on_a_line_it_shares_with_a_close() {
        let src = "<details><summary>A</summary>\n\na body\n\n</details><details><summary>B</summary>\n\nb body\n\n</details>";

        let rows = render_toggled(src, 40, &[]);
        assert_eq!(texts_trimmed(&rows), ["▸ A", "▸ B"]);
        assert_eq!(details_of(&rows), [Some(0), Some(1)]);

        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[1])),
            ["▸ A", "▾ B", "", "b body"]
        );
    }

    #[test]
    fn should_take_a_summary_block_separated_by_blank_lines_as_the_header() {
        let src = "<details>\n\n<summary>X</summary>\n\nBody\n\n</details>";

        assert_eq!(texts_trimmed(&render_toggled(src, 40, &[])), ["▸ X"]);
        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[0])),
            ["▾ X", "", "Body"]
        );
    }

    #[test]
    fn should_render_an_unclosed_section_as_source() {
        let rows = render_toggled("<details>\n<summary>X</summary>\n\nBody", 40, &[]);

        assert_eq!(
            texts_trimmed(&rows),
            ["<details>", "<summary>X</summary>", "", "Body"]
        );
        assert!(rows.iter().all(|r| r.details.is_none()));
    }

    #[test]
    fn should_never_open_a_section_from_an_html_comment() {
        let rows = render_toggled("<!-- wrap logs in <details> -->\n\nBody", 40, &[]);

        assert!(rows.iter().all(|r| r.details.is_none()));
        assert_eq!(rows.last().map(text).as_deref(), Some("Body"));
    }

    #[test]
    fn should_never_open_a_section_inside_a_comment_or_pre_block() {
        for src in [
            "<!-- </details><details><summary>X</summary></details> -->\n\nBody",
            "<pre>\n<details><summary>X</summary></details>\n</pre>\n\nBody",
        ] {
            let rows = render_toggled(src, 40, &[]);

            assert!(rows.iter().all(|r| r.details.is_none()), "{src:?}");
        }
    }

    #[test]
    fn should_never_open_a_section_from_code() {
        for src in [
            "```\n<details>\n<summary>X</summary>\n```\n\nAfter",
            "`<details>` inline\n\nAfter",
        ] {
            let rows = render_toggled(src, 40, &[]);

            assert!(rows.iter().all(|r| r.details.is_none()), "{src:?}");
            assert_eq!(rows.last().map(text).as_deref(), Some("After"), "{src:?}");
        }
    }

    #[test]
    fn should_wrap_a_long_summary_under_its_label_with_every_row_on_the_section() {
        let src = "<details><summary>alpha beta gamma</summary>\n\nBody\n\n</details>";

        let rows = render_toggled(src, 12, &[]);
        assert_eq!(texts_trimmed(&rows), ["▸ alpha beta", "  gamma"]);
        assert_eq!(details_of(&rows), [Some(0), Some(0)]);
        assert_eq!(rows[0].source, rows[1].source);
    }

    #[test]
    fn should_draw_the_marker_in_bullet_style_and_the_label_in_base_style() {
        let theme = Theme::dark();
        let look = Look::new(&theme);
        let rows = render_block(&theme, &coverage(), 40, &[], &BTreeSet::new());

        assert_eq!(styles_of(&rows[2], "▸"), [look.bullet]);
        assert!(
            styles_of(&rows[2], "Coverage report")
                .iter()
                .all(|style| *style == look.base)
        );
    }

    #[test]
    fn should_give_a_kept_apart_range_inside_a_collapsed_section_no_anchor_row() {
        let theme = Theme::dark();
        let src = "<details>\n<summary>S</summary>\n\n![a](https://x.test/a.png)\n\n</details>";
        let keep_apart = std::slice::from_ref(&(3..4));

        let rows = render_block(&theme, src, 40, keep_apart, &BTreeSet::new());
        assert_eq!(texts_trimmed(&rows), ["▸ S"]);
        assert!(rows.iter().all(|r| r.source != (3..4)));

        let rows = render_block(&theme, src, 40, keep_apart, &BTreeSet::from([0]));
        assert!(rows.iter().any(|r| r.source == (3..4)));
    }

    #[test]
    fn should_anchor_a_kept_apart_range_overlapping_a_header_after_the_summary_row() {
        let rows = render_block(
            &Theme::dark(),
            "<details>\n<summary>Shot</summary>\n\nBody\n\n</details>",
            40,
            std::slice::from_ref(&(0..1)),
            &BTreeSet::new(),
        );

        assert_eq!(
            text_and_source(&rows),
            [("▸ Shot".to_string(), 0..2), (String::new(), 0..1)]
        );
    }

    #[test]
    fn should_fold_a_section_in_a_crlf_body() {
        let src = "<details>\r\n<summary>X</summary>\r\n\r\nBody\r\n\r\n</details>";

        assert_eq!(texts_trimmed(&render_toggled(src, 40, &[])), ["▸ X"]);
        assert_eq!(
            texts_trimmed(&render_toggled(src, 40, &[0])),
            ["▾ X", "", "Body"]
        );
    }

    #[test]
    fn should_ignore_a_toggled_id_that_names_no_section() {
        let rows = render_toggled(&coverage(), 40, &[7]);

        assert_eq!(
            texts_trimmed(&rows),
            ["Intro", "", "▸ Coverage report", "", "Outro"]
        );
        assert_eq!(details_of(&rows), [None, None, Some(0), None, None]);
    }
}
