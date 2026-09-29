//! Full-screen media viewer (`gd-lc6.1`), opened by Enter on a PR-description
//! media placeholder row. Mirrors `help_popup::render_message_details`'s
//! early-return, full-screen shape.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use ratatui_image::Image;

use crate::app::App;
use crate::app::media::MediaSlot;
use crate::ui::{status_bar, styles};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    let (header_area, image_area, status_area) = (rows[0], rows[1], rows[2]);

    let theme = &app.theme;
    frame.render_widget(
        ratatui::widgets::Block::default().style(styles::panel_style(theme)),
        area,
    );

    let Some(viewer) = app.media_viewer.as_mut() else {
        return;
    };
    viewer.set_area(fit_area(image_area));

    let label = viewer
        .items
        .get(viewer.index)
        .map(|item| item.label.as_str())
        .unwrap_or_default();
    let header = format!(
        "media {}/{} \u{b7} {label}",
        viewer.index + 1,
        viewer.items.len()
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            header,
            Style::default().fg(theme.fg_primary),
        ))),
        header_area,
    );

    match &viewer.current {
        MediaSlot::Loading => {
            frame.render_widget(
                Paragraph::new("Loading\u{2026}")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.fg_secondary)),
                center_line(image_area),
            );
        }
        MediaSlot::Ready { protocol, .. } => {
            let size = protocol.size();
            let width = size.width.min(image_area.width);
            let centered = Rect {
                x: image_area.x + (image_area.width - width) / 2,
                y: image_area.y,
                width,
                height: size.height.min(image_area.height),
            };
            frame.render_widget(Image::new(protocol), centered);
        }
        MediaSlot::NotImage | MediaSlot::Failed(_) => {
            let mut text = vec![Line::from(Span::styled(
                label.to_string(),
                Style::default().fg(theme.fg_primary),
            ))];
            if let MediaSlot::Failed(message) = &viewer.current {
                text.push(Line::from(Span::styled(
                    message.clone(),
                    Style::default().fg(theme.message_error_fg),
                )));
            }
            text.push(Line::from(Span::styled(
                "press o to open outside tuicr",
                Style::default().fg(theme.fg_secondary),
            )));
            let height = text.len() as u16;
            frame.render_widget(
                Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false }),
                center_block(image_area, height),
            );
        }
    }

    status_bar::render_status_bar(frame, app, status_area);
}

/// The box an image is fitted into: `area` less 1/16 of each dimension.
///
/// Terminals report their cell size in whole pixels, and some round up: Orca
/// answers 8x18 for a 7.668x17.525 cell. Sized with the rounded cell, the
/// image draws a few percent larger than `area`, and a sixel that reaches the
/// last screen row scrolls the whole screen. The margin absorbs up to one
/// pixel of rounding per cell for cells at least 16 pixels tall.
fn fit_area(area: Rect) -> Rect {
    Rect {
        width: area.width - area.width.div_ceil(16),
        height: area.height - area.height.div_ceil(16),
        ..area
    }
}

/// A single centered row within `area`, for one-line status text.
fn center_line(area: Rect) -> Rect {
    center_block(area, 1)
}

/// `height` rows centered vertically within `area`.
fn center_block(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    let top = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x: area.x,
        y: top,
        width: area.width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use crate::app::App;
    use crate::app::InputMode;
    use crate::app::media::{MediaSlot, MediaViewer};
    use crate::app::tests::pr_info_tests::build_pr_app;

    #[test]
    fn fit_area_leaves_room_for_rounded_cell_sizes() {
        assert_eq!(
            super::fit_area(Rect::new(0, 1, 120, 38)),
            Rect::new(0, 1, 112, 35)
        );
        assert_eq!(
            super::fit_area(Rect::new(0, 1, 10, 4)),
            Rect::new(0, 1, 9, 3)
        );
    }

    const BODY: &str = "Intro\n![one](https://x.test/1.png)\n![two](https://x.test/2.png)\n<video src=\"https://x.test/v.mp4\"></video>";

    /// `build_pr_app()` with the same body and index-1 viewer the app-level
    /// `media_viewer_tests` fixture uses, opened directly (skipping Enter)
    /// since only the rendered shape is under test here.
    fn app_with_viewer(current: MediaSlot) -> App {
        let mut app = build_pr_app();
        let mut info = app.pr_info.take().expect("pr info");
        info.details.body = BODY.to_string();
        app.pr_info = Some(info);
        app.diff_state.viewport_width = 82;
        app.rebuild_annotations();

        let items: Vec<_> = crate::ui::pr_info_panel::pr_body_media(app.pr_info.as_ref().unwrap())
            .into_iter()
            .map(|line| line.media)
            .collect();
        let mut viewer = MediaViewer::new(items, 1);
        viewer.current = current;
        app.media_viewer = Some(viewer);
        app.input_mode = InputMode::MediaViewer;
        app
    }

    fn draw_app(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn loading_shows_the_header_a_loading_message_and_the_media_mode_chip() {
        let mut app = app_with_viewer(MediaSlot::Loading);
        let buffer = draw_app(&mut app, 60, 12);

        assert!(row_text(&buffer, 0).contains("media 2/3 \u{b7} two"));
        assert!((0..buffer.area.height).any(|y| row_text(&buffer, y).contains("Loading\u{2026}")));
        assert!(row_text(&buffer, buffer.area.height - 1).contains(" MEDIA "));
    }

    #[test]
    fn failed_shows_the_open_outside_hint_and_the_label() {
        let mut app = app_with_viewer(MediaSlot::Failed("boom".to_string()));
        let buffer = draw_app(&mut app, 60, 12);

        assert!(
            (0..buffer.area.height)
                .any(|y| row_text(&buffer, y).contains("press o to open outside tuicr"))
        );
        assert!((0..buffer.area.height).any(|y| row_text(&buffer, y).contains("two")));
    }
}
