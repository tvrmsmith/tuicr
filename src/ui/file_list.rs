use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FileTreeItem, FocusedPanel};
use crate::ui::diff_view::apply_horizontal_scroll;
use crate::ui::styles;

const EXPANDED_GLYPH: &str = "\u{25bc}"; // ▼
const COLLAPSED_GLYPH: &str = "\u{25b6}"; // ▶
const REVIEWED_BOX: &str = "\u{25a3}"; // ▣
const UNREVIEWED_BOX: &str = "\u{25a2}"; // ▢

pub(super) fn render_file_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::FileList;

    let mut title = format!(
        " Files \u{00b7} {}/{} ",
        app.reviewed_count(),
        app.file_count()
    );
    // A filter changes what the counts above mean, so say how much is hidden.
    if app.file_filter_active() {
        title.push_str(&format!(
            "\u{00b7} {} of {} ",
            app.file_count(),
            app.unfiltered_file_count()
        ));
    }
    // After the filter qualifier, which keeps its precedence over every
    // grouping state: it changes what the counts mean (`gd-26r.15`). The room
    // left is the border line less its two corners.
    if let Some(status) = app.grouping_status() {
        let room = (area.width as usize).saturating_sub(2 + title.width());
        title.push_str(&status.chip(room));
    }
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    // Bottom border doubles as the filter status / prompt line. Using a
    // block title instead of reserving an inner row keeps the list geometry
    // (and therefore viewport_height and every scroll calculation) untouched.
    if let Some(bottom) = filter_footer(app) {
        block = block.title_bottom(bottom);
    }

    let inner = block.inner(area);
    app.file_list_inner_area = Some(inner);
    let visible_items = app.build_visible_items();

    let max_content_width = visible_items
        .iter()
        .map(|item| match item {
            FileTreeItem::Directory { label, depth, .. } => depth * 2 + 2 + label.width(),
            FileTreeItem::File { label, depth, .. } => depth * 2 + 4 + label.width(),
            FileTreeItem::Group {
                label,
                reviewed,
                total,
                drifted,
                unbounded,
                ..
            } => {
                2 + group_marker(*unbounded, *drifted).width()
                    + label.width()
                    + 1
                    + group_count(*reviewed, *total).width()
            }
        })
        .max()
        .unwrap_or(0);

    app.file_list_state.viewport_width = inner.width as usize;
    app.file_list_state.viewport_height = inner.height as usize;
    app.file_list_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.file_list_state.scroll_x > max_scroll_x {
        app.file_list_state.scroll_x = max_scroll_x;
    }
    let scroll_x = app.file_list_state.scroll_x;
    let grouped = app.grouping.is_some();

    // When diff panel is focused, sync file list selection to current view
    // But preserve the current offset to not interfere with manual scrolling
    if app.focused_panel == FocusedPanel::Diff {
        let current_file_idx = app.diff_state.current_file_idx;
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == current_file_idx
            {
                if app.file_list_state.selected() != tree_idx {
                    // Save current offset before changing selection
                    let current_offset = app.file_list_state.list_state.offset();
                    app.file_list_state.select(tree_idx);
                    // Restore offset to prevent auto-scrolling
                    *app.file_list_state.list_state.offset_mut() = current_offset;
                }
                break;
            }
        }
    }

    let items: Vec<ListItem> = visible_items
        .iter()
        .map(|item| {
            let line = match item {
                // The group name is rendered **verbatim** — no prettification,
                // no title-casing, no substituting a directory name for a poor
                // one. A generic name beside a large count is the cheapest
                // signal that the grouping is weak there, and papering over it
                // hides the signal (`docs/SIDEBAR_MODEL.md`).
                FileTreeItem::Group {
                    label,
                    reviewed,
                    total,
                    expanded,
                    drifted,
                    unbounded,
                    ..
                } => {
                    let icon = if *expanded {
                        EXPANDED_GLYPH
                    } else {
                        COLLAPSED_GLYPH
                    };
                    let marker = group_marker(*unbounded, *drifted);
                    let count = group_count(*reviewed, *total);
                    // The count is flush right, which is what makes the
                    // collapsed overview scannable as a column of sizes, so a
                    // name too wide for the panel gives way to it rather than
                    // pushing it past the right edge. Panning is left alone
                    // for the same reason it is on file rows.
                    let inner_width = inner.width as usize;
                    let name = if scroll_x == 0 {
                        elide_middle(
                            label,
                            inner_width.saturating_sub(3 + marker.width() + count.width()),
                        )
                    } else {
                        label.clone()
                    };
                    let gap = inner_width
                        .saturating_sub(2 + marker.width() + name.width() + count.width())
                        .max(1);
                    Line::from(vec![
                        Span::styled(format!("{icon} "), styles::dir_icon_style(&app.theme)),
                        Span::raw(marker),
                        Span::raw(name),
                        Span::raw(" ".repeat(gap)),
                        Span::styled(count, styles::dim_style(&app.theme)),
                    ])
                }
                FileTreeItem::Directory {
                    label,
                    depth,
                    expanded,
                    ..
                } => {
                    let indent = "  ".repeat(*depth);
                    let icon = if *expanded {
                        EXPANDED_GLYPH
                    } else {
                        COLLAPSED_GLYPH
                    };
                    Line::from(vec![
                        Span::raw(indent),
                        Span::styled(format!("{icon} "), styles::dir_icon_style(&app.theme)),
                        Span::raw(label.clone()),
                    ])
                }
                FileTreeItem::File {
                    file_idx,
                    label,
                    depth,
                } => {
                    let file = &app.diff_files[*file_idx];
                    let path = file.display_path();
                    let is_reviewed = app.session.is_file_reviewed(path);
                    let checkbox = if is_reviewed {
                        REVIEWED_BOX
                    } else {
                        UNREVIEWED_BOX
                    };
                    let checkbox_style = if is_reviewed {
                        styles::reviewed_style(&app.theme)
                    } else {
                        styles::pending_style(&app.theme)
                    };
                    if file.is_commit_message {
                        Line::from(vec![
                            Span::styled(format!("{checkbox} "), checkbox_style),
                            Span::raw(format!("  {}", path.display())),
                        ])
                    } else {
                        let indent = "  ".repeat(*depth);
                        let mut spans = vec![
                            Span::raw(indent),
                            Span::styled(format!("{checkbox} "), checkbox_style),
                        ];
                        // Pristine mode reviews unchanged code; the M/A/D
                        // badge would lie. Suppress it and leave the row as
                        // checkbox + filename.
                        if !app.is_pristine_mode {
                            let status = file.status.as_char();
                            spans.push(Span::styled(
                                format!("{status} "),
                                styles::file_status_style(&app.theme, status),
                            ));
                        }
                        // A grouped row carries the full relative path, which
                        // out-measures a narrow panel. Elide its middle so the
                        // file name — the part that distinguishes it from its
                        // siblings — survives (`docs/SIDEBAR_MODEL.md`).
                        // Panning is left alone: once the user scrolls right
                        // they asked for the whole path, and `scroll_x` is
                        // measured against the unelided width.
                        let chrome: usize = spans.iter().map(|span| span.width()).sum();
                        let elide = grouped && scroll_x == 0;
                        let text = if elide {
                            elide_middle(label, (inner.width as usize).saturating_sub(chrome))
                        } else {
                            label.clone()
                        };
                        spans.push(Span::raw(text));
                        Line::from(spans)
                    }
                }
            };

            ListItem::new(apply_horizontal_scroll(line, scroll_x))
        })
        .collect();

    // Full-row bg highlight on the selected row (no leading cursor glyph or
    // underline modifier) — mirrors how the diff view highlights its cursor
    // line.
    let list = List::new(items)
        .style(styles::panel_style(&app.theme))
        .highlight_style(styles::selected_style(&app.theme))
        .block(block);

    frame.render_stateful_widget(list, area, &mut app.file_list_state.list_state);

    // Park the terminal cursor at the end of the prompt buffer so typing has
    // a visible insertion point.
    if let Some(draft) = app.file_tree_draft() {
        let prefix = PROMPT_PAD + 2 + draft.buffer.width() as u16;
        frame.set_cursor_position(Position {
            x: (area.x + prefix).min(area.x + area.width.saturating_sub(1)),
            y: area.y + area.height.saturating_sub(1),
        });
    }
}

/// Drop the middle of `label` so it fits in `max_width` cells, marking the cut
/// with `…`. Two thirds of the budget go to the tail and the remainder to the
/// head, which is the 1:2 split mockup A is drawn at (`docs/SIDEBAR_MODEL.md`):
/// the tail carries the file name and the head still says which part of the
/// tree the row came from. Widths are display cells, not characters, so a
/// fullwidth segment is measured the way the panel renders it, and the walk is
/// by grapheme cluster so a combining mark cannot lead the tail (macOS hands
/// back NFD paths) nor a joiner dangle at the end of the head.
fn elide_middle(label: &str, max_width: usize) -> String {
    if label.width() <= max_width {
        return label.to_string();
    }
    if max_width <= 1 {
        return "\u{2026}".repeat(max_width);
    }

    let budget = max_width - 1;
    let tail_budget = budget * 2 / 3;
    let head_budget = budget - tail_budget;

    let mut head = String::new();
    let mut used = 0;
    for cluster in label.graphemes(true) {
        let width = cluster.width();
        if used + width > head_budget {
            break;
        }
        head.push_str(cluster);
        used += width;
    }

    let mut tail = String::new();
    let mut used = 0;
    for cluster in label.graphemes(true).rev() {
        let width = cluster.width();
        if used + width > tail_budget {
            break;
        }
        tail.insert_str(0, cluster);
        used += width;
    }

    format!("{head}\u{2026}{tail}")
}

/// A group row's `reviewed/total` badge.
fn group_count(reviewed: usize, total: usize) -> String {
    format!("{reviewed}/{total}")
}

/// The marker slot of a group row: `! ` on a group the size cap could not bound
/// (`gd-26r.23`), `~ ` on a group incremental assignment produced
/// (`gd-26r.15`), nothing otherwise (`docs/SIDEBAR_MODEL.md`).
///
/// **One slot, one glyph**, and `!` outranks `~` in the rare row that could
/// carry both. `~` says the row is a little stale; `!` says the row will not
/// help you at all, which is the more urgent of the two and the one the count
/// column cannot say for itself. A cap-forced *split* carries neither: those
/// groups are as fresh as anything else the pass produced.
///
/// Between the expand icon and the label, never between the label and the
/// count: the count is flush right and that column is what makes a collapsed
/// overview scannable as a list of sizes (`gd-26r.31`). Restyling the label
/// instead was rejected — invisible on a theme without italics, and unreadable
/// to anyone comparing two shades.
fn group_marker(unbounded: bool, drifted: bool) -> &'static str {
    if unbounded {
        "! "
    } else if drifted {
        "~ "
    } else {
        ""
    }
}

/// Leading `│` border plus one space before the prompt sigil.
const PROMPT_PAD: u16 = 2;

/// Bottom-border content for the file tree: the active prompt while one is
/// open, otherwise a summary of the applied filters and search.
fn filter_footer(app: &App) -> Option<Line<'static>> {
    let theme = &app.theme;

    if let Some(draft) = app.file_tree_draft() {
        return Some(Line::from(vec![
            Span::styled(
                format!(" {} ", draft.prompt.sigil()),
                styles::mode_style(theme),
            ),
            Span::styled(
                format!("{} ", draft.buffer),
                Style::default().fg(theme.fg_primary),
            ),
        ]));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut push = |label: char, value: String| {
        if !spans.is_empty() {
            spans.push(Span::styled(
                " \u{00b7} ",
                Style::default().fg(theme.fg_dim),
            ));
        }
        spans.push(Span::styled(
            format!("{label}:"),
            Style::default().fg(theme.fg_dim),
        ));
        spans.push(Span::styled(value, Style::default().fg(theme.fg_secondary)));
    };

    // Only the filters earn a slot here: they change what the panes contain
    // and there is no other persistent cue for them. The `/` query is left
    // out on purpose — the panel is narrow (a third pattern truncates to
    // noise), and every `n`/`N` step reports the query and match position in
    // the status bar anyway.
    if let Some(include) = app.file_filter.include.as_ref() {
        push('i', include.source.clone());
    }
    if let Some(exclude) = app.file_filter.exclude.as_ref() {
        push('e', exclude.source.clone());
    }

    if spans.is_empty() {
        return None;
    }
    spans.insert(0, Span::raw(" "));
    spans.push(Span::raw(" "));
    Some(Line::from(spans))
}

#[cfg(test)]
mod tests {
    //! Render checks for the file tree panel, driven through the real
    //! `ui::render`: the prompt/filter line in its bottom border, the middle
    //! elision of grouped file rows, and group rows themselves.
    use super::elide_middle;
    use crate::app::tests::grouping_tests::{
        build_app_over, empty_session, make_file as file, stub_vcs_info,
    };
    use crate::app::{App, FileTreeItem, FileTreeMode, FileTreePrompt, FocusedPanel};
    use crate::grouping::changeset::Changeset;
    use crate::grouping::{GroupId, GroupSource, Grouping, PresentedGroup};
    use crate::model::{DiffFile, FileStatus, ReviewSession};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    /// The sidebar tests build the same app the `crate::app` tests do; only the
    /// two panel flags below are this module's own.
    fn build_app(files: Vec<DiffFile>, session: ReviewSession) -> App {
        let mut app = build_app_over(files, session);
        app.show_file_list = true;
        app.focused_panel = FocusedPanel::FileList;
        app
    }

    fn app_with(paths: &[&str]) -> App {
        let session = empty_session(&stub_vcs_info());
        build_app(paths.iter().map(|p| file(p)).collect(), session)
    }

    /// Everything above, plus a grouping recorded on the session so the group
    /// name is the record's rather than whatever the heuristics derive — the
    /// same route a refined grouping takes back out of a session file.
    fn app_with_group_named(name: &str, paths: &[&str]) -> App {
        let files: Vec<DiffFile> = paths.iter().map(|p| file(p)).collect();
        let mut session = empty_session(&stub_vcs_info());
        for file in &files {
            session.add_file(file.display_path().clone(), file.status, file.content_hash);
        }
        session.record_grouping(&Grouping::restore(
            &Changeset::from_diff_files(&files),
            vec![PresentedGroup {
                id: GroupId::new(),
                name: name.to_string(),
                source: GroupSource::Heuristics,
                new_since_full_pass: false,
                unbounded: false,
                members: paths.iter().map(|p| (*p).to_string()).collect(),
            }],
        ));

        let mut app = build_app(files, session);
        app.enable_grouping();
        app.expand_all_dirs();
        assert_eq!(
            group_row(&app).0,
            name,
            "the session restored the record's grouping, not a heuristic one"
        );
        app
    }

    /// The first group row's name and its `reviewed/total` badge.
    fn group_row(app: &App) -> (String, String) {
        app.build_visible_items()
            .iter()
            .find_map(|item| match item {
                FileTreeItem::Group {
                    label,
                    reviewed,
                    total,
                    ..
                } => Some((label.clone(), super::group_count(*reviewed, *total))),
                _ => None,
            })
            .expect("a group row")
    }

    fn draw(app: &mut App) -> Buffer {
        draw_at(app, 120)
    }

    fn draw_at(app: &mut App, width: u16) -> Buffer {
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Just the file-list panel's columns, so an assertion about the sidebar
    /// cannot be satisfied by the diff pane's own header. The cut comes from
    /// the inner area the renderer just stored, so it follows the split
    /// instead of restating it.
    fn sidebar_text(app: &App, buffer: &Buffer) -> String {
        let inner = app.file_list_inner_area.expect("the panel was rendered");
        let end = (inner.x + inner.width).min(buffer.area.width);
        (0..buffer.area.height)
            .map(|y| {
                (0..end)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn grouped_app_with(paths: &[&str]) -> App {
        let mut app = app_with(paths);
        for path in paths {
            app.session
                .add_file(PathBuf::from(path), FileStatus::Modified, 0);
        }
        app.enable_grouping();
        app.expand_all_dirs();
        app
    }

    /// The panel is 20% of the terminal, so at 120 columns a grouped file row
    /// has 22 inner cells less 6 of chrome (indent, checkbox, status badge) —
    /// 16 for a path of 33.
    #[test]
    fn should_elide_the_middle_of_a_grouped_path_and_keep_the_file_name() {
        let mut app = grouped_app_with(&["src/main/generated/api/v2/repo.ts"]);

        let buffer = draw(&mut app);
        let text = sidebar_text(&app, &buffer);

        assert!(
            text.contains("src/m\u{2026}v2/repo.ts"),
            "expected a middle-elided path in the tree, got:\n{text}"
        );
    }

    /// Mockup A's own rows (`docs/SIDEBAR_MODEL.md`) drawn through the
    /// renderer at the mockup's 38-cell inner width. The mockups omit the
    /// M/A/D badge, so a shipped grouped row spends 6 cells of chrome rather
    /// than the 4 they draw: 32 cells of path, split as an 11-cell head, the
    /// ellipsis and a 20-cell tail.
    #[test]
    fn should_split_the_mockup_rows_the_way_the_renderer_draws_them() {
        let mut app = app_with_group_named(
            "enterprise-host-routing",
            &[
                "src/main/github/github-enterprise-repository.ts",
                "mobile/src/tasks/github-project-host-routing-source.test.ts",
            ],
        );

        // 20% of 200 columns is a 40-cell panel, so 38 inside its border.
        let buffer = draw_at(&mut app, 200);
        let text = sidebar_text(&app, &buffer);

        assert_eq!(
            app.file_list_inner_area.map(|inner| inner.width),
            Some(38),
            "this test only measures mockup A if the panel is the mockup's width"
        );
        assert!(
            text.contains("src/main/gi\u{2026}rprise-repository.ts"),
            "expected mockup A's first row, got:\n{text}"
        );
        assert!(
            text.contains("mobile/src/\u{2026}uting-source.test.ts"),
            "expected mockup A's second row, got:\n{text}"
        );
    }

    /// A grouped sidebar holding one group a full pass produced and one
    /// incremental assignment opened, restored through the session the way a
    /// reopened review restores it — which is what makes the marker and the
    /// drift share persisted facts rather than facts about this process.
    fn drifted_app() -> App {
        let steady = ["src/auth/login.rs", "src/auth/session.rs", "docs/auth.md"];
        let arrived = ["infra/terraform/network.tf"];
        let files: Vec<DiffFile> = steady
            .iter()
            .chain(arrived.iter())
            .map(|p| file(p))
            .collect();
        let mut session = empty_session(&stub_vcs_info());
        for file in &files {
            session.add_file(file.display_path().clone(), file.status, file.content_hash);
        }
        session.record_grouping(&Grouping::restore(
            &Changeset::from_diff_files(&files),
            vec![
                PresentedGroup {
                    id: GroupId::new(),
                    name: "steady".to_string(),
                    source: GroupSource::Heuristics,
                    new_since_full_pass: false,
                    unbounded: false,
                    members: steady.iter().map(|p| (*p).to_string()).collect(),
                },
                PresentedGroup {
                    id: GroupId::new(),
                    name: "arrivals".to_string(),
                    source: GroupSource::Incremental,
                    new_since_full_pass: true,
                    unbounded: false,
                    members: arrived.iter().map(|p| (*p).to_string()).collect(),
                },
            ],
        ));

        let mut app = build_app(files, session);
        app.enable_grouping();
        app.expand_all_dirs();
        app
    }

    /// One file in four arrived since the last full pass, and the header says
    /// so after the counts, in plain text, with the command that fixes it.
    #[test]
    fn should_report_drift_in_the_sidebar_header_with_the_command_that_clears_it() {
        let mut app = drifted_app();

        // 20% of 180 columns is a 36-cell panel, which is the narrowest header
        // that holds the counts and the whole advisory.
        let buffer = draw_at(&mut app, 180);
        let text = sidebar_text(&app, &buffer);

        assert!(
            text.contains("Files \u{00b7} 0/4 \u{00b7} 25% new \u{00b7} :regroup"),
            "expected the drift chip after the counts, got:\n{text}"
        );
    }

    /// The advisory goes whole rather than truncating to `:regr`, and the
    /// percentage — the part the slot exists for — stays.
    #[test]
    fn should_drop_the_regroup_advice_from_a_header_too_narrow_for_it() {
        let mut app = drifted_app();

        let buffer = draw_at(&mut app, 150);
        let text = sidebar_text(&app, &buffer);

        assert!(
            text.contains("Files \u{00b7} 0/4 \u{00b7} 25% new"),
            "expected the drift share to survive the narrow header, got:\n{text}"
        );
        assert!(
            !text.contains(":regr"),
            "expected no truncated advisory, got:\n{text}"
        );
    }

    /// The marker sits between the expand icon and the label. The count column
    /// stays flush right, which is the whole reason it does not sit there.
    #[test]
    fn should_mark_only_the_group_incremental_assignment_opened() {
        let mut app = drifted_app();

        let buffer = draw_at(&mut app, 180);
        let text = sidebar_text(&app, &buffer);

        let marked = text
            .lines()
            .find(|line| line.contains("arrivals"))
            .expect("the drifted group has a row");
        let unmarked = text
            .lines()
            .find(|line| line.contains("steady"))
            .expect("the full-pass group has a row");
        assert!(
            marked.contains("\u{25bc} ~ arrivals"),
            "expected the marker before the label, got:\n{text}"
        );
        assert!(
            marked.trim_end().ends_with("0/1"),
            "the count stays flush right beside the marker, got: {marked}"
        );
        assert!(
            !unmarked.contains('~'),
            "a group a full pass produced carries no marker, got: {unmarked}"
        );
    }

    /// The same slot, the other glyph (`gd-26r.23`): a group the size cap's one
    /// split pass could not bound says so with `!`, and the split it *could*
    /// make says nothing at all — those pieces are ordinary groups of the pass.
    ///
    /// Restored through the session like the drift marker above, because a
    /// reopened review never regroups and the row has to keep saying it.
    #[test]
    fn should_mark_only_the_group_the_size_cap_could_not_bound() {
        let flat = ["src/gen/a.rs", "src/gen/b.rs"];
        let split = ["src/api/c.rs"];
        let files: Vec<DiffFile> = flat.iter().chain(split.iter()).map(|p| file(p)).collect();
        let mut session = empty_session(&stub_vcs_info());
        for file in &files {
            session.add_file(file.display_path().clone(), file.status, file.content_hash);
        }
        session.record_grouping(&Grouping::restore(
            &Changeset::from_diff_files(&files),
            vec![
                PresentedGroup {
                    id: GroupId::new(),
                    name: "generated".to_string(),
                    source: GroupSource::Heuristics,
                    new_since_full_pass: false,
                    unbounded: true,
                    members: flat.iter().map(|p| (*p).to_string()).collect(),
                },
                PresentedGroup {
                    id: GroupId::new(),
                    name: "client \u{b7} api".to_string(),
                    source: GroupSource::Heuristics,
                    new_since_full_pass: false,
                    unbounded: false,
                    members: split.iter().map(|p| (*p).to_string()).collect(),
                },
            ],
        ));

        let mut app = build_app(files, session);
        app.enable_grouping();
        app.expand_all_dirs();

        let buffer = draw_at(&mut app, 180);
        let text = sidebar_text(&app, &buffer);

        let marked = text
            .lines()
            .find(|line| line.contains("generated"))
            .expect("the unbounded group has a row");
        let piece = text
            .lines()
            .find(|line| line.contains("client"))
            .expect("the split piece has a row");
        assert!(
            marked.contains("\u{25bc} ! generated"),
            "expected the marker before the label, got:\n{text}"
        );
        assert!(
            marked.trim_end().ends_with("0/2"),
            "the count stays flush right beside the marker, got: {marked}"
        );
        assert!(
            !piece.contains('!') && !piece.contains('~'),
            "a piece the split pass did bound carries no marker, got: {piece}"
        );
    }

    /// Toggled off, the sidebar is the plain file tree again: no chip, no
    /// marker, nothing about a grouping that is not on screen.
    #[test]
    fn should_carry_no_grouping_chrome_in_the_ungrouped_sidebar() {
        let mut app = drifted_app();
        app.toggle_grouping();

        let buffer = draw_at(&mut app, 180);
        let text = sidebar_text(&app, &buffer);

        assert!(
            !text.contains("new"),
            "expected no drift chip in the ungrouped header, got:\n{text}"
        );
        assert!(
            !text.contains('~'),
            "expected no marker on a directory tree, got:\n{text}"
        );
    }

    #[test]
    fn should_measure_a_fullwidth_path_in_cells_not_characters() {
        // Fourteen characters, twenty-three cells: measured as characters this
        // fits in a 16-cell row and is handed to ratatui to clip.
        let label = "日本語/設定/読み込み.rs";

        let elided = elide_middle(label, 16);

        assert!(
            elided.width() <= 16,
            "elided to {} cells: {elided}",
            elided.width()
        );
        assert!(
            elided.starts_with("日本") && elided.ends_with("込み.rs"),
            "expected head and tail of the path, got {elided}"
        );
    }

    /// Both cuts have to land on a grapheme boundary at every width, so no
    /// combining mark ever leads the tail and no joiner ever dangles at the end
    /// of the head.
    fn assert_cuts_on_cluster_boundaries(label: &str) {
        let boundaries: Vec<usize> = label
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain(std::iter::once(label.len()))
            .collect();

        for max_width in 2..=label.width() {
            let elided = elide_middle(label, max_width);
            let Some((head, tail)) = elided.split_once('\u{2026}') else {
                continue;
            };
            assert!(
                boundaries.contains(&head.len()),
                "head cut mid-cluster at width {max_width}, got {elided:?}"
            );
            assert!(
                boundaries.contains(&(label.len() - tail.len())),
                "tail cut mid-cluster at width {max_width}, got {elided:?}"
            );
        }
    }

    /// macOS hands paths back in NFD, so an accented component is a base
    /// character followed by a combining mark. The reverse walk that builds the
    /// tail reaches the mark first; taking it without its base stacks it on the
    /// ellipsis.
    #[test]
    fn should_not_orphan_a_combining_mark_onto_the_ellipsis() {
        assert_cuts_on_cluster_boundaries("src/ge\u{301}ne\u{301}re\u{301}/api/v2/fo\u{301}rm.rs");
    }

    /// The head has the mirror problem: a zero-width joiner or variation
    /// selector costs nothing to admit, so the head can keep one whose partner
    /// it then rejects.
    #[test]
    fn should_not_dangle_a_joiner_before_the_ellipsis() {
        assert_cuts_on_cluster_boundaries(
            "src/\u{1f468}\u{200d}\u{1f4bb}/wo\u{fe0f}rk/v2/handler\u{fe0f}.rs",
        );
    }

    #[test]
    fn should_stop_eliding_once_the_user_pans_the_panel() {
        let mut app = grouped_app_with(&["src/main/generated/api/v2/repo.ts"]);
        app.file_list_state.scroll_x = 1;

        let buffer = draw(&mut app);
        let text = sidebar_text(&app, &buffer);

        assert!(
            text.contains("src/main/generate"),
            "panning asks for the whole path, not an elided one, got:\n{text}"
        );
    }

    /// Elision is the grouped sidebar's answer to full paths. The ungrouped
    /// tree under `Flat` draws the same full paths and is out of this slice's
    /// scope: it clips and leans on horizontal panning, not on a directory
    /// tree, which `Flat` does not draw.
    #[test]
    fn should_leave_an_ungrouped_flat_path_unelided() {
        let mut app = app_with(&["src/main/generated/api/v2/repo.ts"]);
        app.file_tree_mode = FileTreeMode::Flat;
        app.expand_all_dirs();

        let buffer = draw(&mut app);
        let text = sidebar_text(&app, &buffer);

        // 22 inner cells less 4 of chrome: the row clips at `generated`.
        assert!(
            text.contains("src/main/generated"),
            "expected the clipped head of the path, got:\n{text}"
        );
        assert!(
            !text.contains('\u{2026}'),
            "the ungrouped tree clips rather than elides, got:\n{text}"
        );
    }

    /// A name too wide for the panel must not push the count off the row: the
    /// flush-right `reviewed/total` is what makes the collapsed overview
    /// scannable as a column of sizes.
    #[test]
    fn should_keep_a_group_count_on_the_row_when_the_name_is_too_wide() {
        let mut app = app_with_group_named(
            "enterprise-host-routing",
            &["src/main/github/host.ts", "src/main/github/routing.ts"],
        );

        // 22 inner cells: a 23-cell name and a 3-cell count cannot both fit.
        let buffer = draw(&mut app);
        let text = sidebar_text(&app, &buffer);

        let (name, count) = group_row(&app);
        assert!(
            name.width() + count.width() > 22,
            "this test only bites if the name would otherwise clip the count"
        );
        assert!(
            text.contains("enter\u{2026}st-routing 0/2"),
            "expected the name to give way to the count, got:\n{text}"
        );
    }

    /// Panning turns the group row's elision off for the same reason it does on
    /// file rows: the pan is how the user reads the rest of a long name.
    #[test]
    fn should_stop_eliding_a_group_name_once_the_user_pans_the_panel() {
        let mut app = app_with_group_named(
            "enterprise-host-routing",
            &["src/main/github/host.ts", "src/main/github/routing.ts"],
        );
        app.file_list_state.scroll_x = 1;

        let buffer = draw(&mut app);
        let text = sidebar_text(&app, &buffer);

        assert!(
            text.contains("nterprise-host-rout"),
            "the panned row draws the name unelided, got:\n{text}"
        );
        assert!(
            !text.contains('\u{2026}'),
            "and drops the ellipsis the unpanned row carries, got:\n{text}"
        );
    }

    #[test]
    fn should_draw_a_group_row_collapsed_and_expanded() {
        let mut app = app_with_group_named("host-routing", &["src/host.ts", "src/routing.ts"]);

        let buffer = draw(&mut app);
        let expanded = sidebar_text(&app, &buffer);

        assert!(
            expanded.contains("\u{25bc} host-routing"),
            "an expanded group is drawn with the open glyph, got:\n{expanded}"
        );
        assert!(
            expanded.contains("host-routing     0/2"),
            "the count is flush right against the panel edge, got:\n{expanded}"
        );
        assert!(
            expanded.contains("src/host.ts"),
            "an expanded group lists its members, got:\n{expanded}"
        );

        app.collapse_all_dirs();
        let buffer = draw(&mut app);
        let collapsed = sidebar_text(&app, &buffer);

        assert!(
            collapsed.contains("\u{25b6} host-routing     0/2"),
            "a collapsed group keeps its count, got:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("src/host.ts"),
            "a collapsed group emits no rows below it, got:\n{collapsed}"
        );
    }

    #[test]
    fn should_render_the_prompt_sigil_and_buffer_while_a_prompt_is_open() {
        let mut app = app_with(&["src/main.rs", "README.md"]);
        app.begin_file_tree_prompt(FileTreePrompt::Include);
        for ch in r"\.rs$".chars() {
            app.file_tree_prompt_insert_char(ch);
        }

        let text = buffer_text(&draw(&mut app));

        assert!(
            text.contains(r"i \.rs$"),
            "expected the include prompt in the tree border, got:\n{text}"
        );
    }

    #[test]
    fn should_render_applied_filters_in_the_border_after_the_prompt_closes() {
        let mut app = app_with(&["src/main.rs", "tests/smoke.rs", "README.md"]);
        app.begin_file_tree_prompt(FileTreePrompt::Include);
        for ch in r"\.rs$".chars() {
            app.file_tree_prompt_insert_char(ch);
        }
        app.commit_file_tree_prompt();
        app.begin_file_tree_prompt(FileTreePrompt::Exclude);
        for ch in "^tests/".chars() {
            app.file_tree_prompt_insert_char(ch);
        }
        app.commit_file_tree_prompt();

        let text = buffer_text(&draw(&mut app));

        assert!(
            text.contains(r"i:\.rs$") && text.contains("e:^tests/"),
            "expected both applied patterns in the tree border, got:\n{text}"
        );
        // Title reports the filtered count against the unfiltered total.
        assert!(
            text.contains("1 of 3"),
            "expected a filtered/total count in the tree title, got:\n{text}"
        );
    }

    #[test]
    fn should_leave_the_border_clean_when_no_filter_is_set() {
        let mut app = app_with(&["src/main.rs"]);

        let text = buffer_text(&draw(&mut app));

        assert!(
            !text.contains("i:") && !text.contains("e:"),
            "unfiltered tree should not advertise filters, got:\n{text}"
        );
    }
}
