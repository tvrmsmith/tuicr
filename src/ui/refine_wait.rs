//! The in-TUI face of the blocking refine wait.
//!
//! A target picked inside the TUI loads its diff with the alternate screen
//! already up, so the pre-TUI status line on stderr would be painted over by
//! the next frame. This draws the same words as a centred overlay instead
//! (`crate::app::refine`), over an otherwise empty frame: the sidebar behind it
//! is the heuristic grouping the wait is about to replace, and showing it would
//! advertise an answer that is not final.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// How the overlay is painted. Resolved styles rather than the theme itself:
/// the caller holds the app mutably for the duration of the wait, so the theme
/// cannot be borrowed across it.
#[derive(Debug, Clone, Copy)]
pub struct WaitStyle {
    pub body: Style,
    pub border: Style,
}

/// One frame of the wait: `text` is what `crate::app::refine` would have
/// written to stderr.
pub fn render_refine_wait(frame: &mut Frame, style: WaitStyle, text: &str) {
    let area = centered(frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Grouping ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(style.body)
        .border_style(style.border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(style.body);
    frame.render_widget(paragraph, inner);
}

/// Wide enough for the status line at a normal width, three rows tall: the
/// text, and a border either side of it.
fn centered(area: Rect) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(3)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Percentage(80)])
        .flex(Flex::Center)
        .areas(row);
    cell
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::text::Text;

    const WIDTH: u16 = 80;
    const HEIGHT: u16 = 24;

    /// The overlay drawn over a frame filled with `X`s. The filler is what
    /// makes "wrote this cell" checkable at all: the overlay's own padding is
    /// blank, so against a blank frame every assertion below would hold whether
    /// the box was painted or not. A cell still holding an `X` is exactly a
    /// cell the overlay did not write.
    fn painted(text: &str) -> Buffer {
        let backend = TestBackend::new(WIDTH, HEIGHT);
        let mut terminal = Terminal::new(backend).expect("a test terminal");
        terminal
            .draw(|frame| {
                let filler = Text::from(vec!["X".repeat(WIDTH as usize).into(); HEIGHT as usize]);
                frame.render_widget(Paragraph::new(filler), frame.area());
                render_refine_wait(frame, style(), text);
            })
            .expect("draw the wait");
        terminal.backend().buffer().clone()
    }

    fn style() -> WaitStyle {
        WaitStyle {
            body: Style::default(),
            border: Style::default(),
        }
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect()
    }

    /// 80% of 80 columns centred is columns 8..72, and three rows centred in 24
    /// is rows 11..14.
    #[test]
    fn the_wait_is_a_three_row_box_centred_in_the_frame() {
        let buffer = painted("Grouping 6 files, esc cancels");

        assert_eq!(
            row(&buffer, 10),
            "X".repeat(80),
            "the row above is untouched"
        );
        assert_eq!(row(&buffer, 14), "X".repeat(80), "and so is the row below");

        let top = row(&buffer, 11);
        assert_eq!(&top[..8], "XXXXXXXX", "nothing is written beside it");
        assert_eq!(top.chars().nth(8), Some('┌'));
        assert_eq!(top.chars().nth(71), Some('┐'));
        assert!(top.contains("Grouping"), "the box is titled: {top}");
    }

    #[test]
    fn the_box_writes_every_cell_it_covers() {
        let buffer = painted("Grouping 6 files, esc cancels");
        for y in 11..14 {
            let line = row(&buffer, y);
            let inside: String = line.chars().skip(8).take(64).collect();
            assert!(
                !inside.contains('X'),
                "row {y} left a cell unwritten: {inside}"
            );
        }
    }

    #[test]
    fn the_status_line_is_centred_inside_the_box() {
        let text = "Grouping 6 files, esc cancels";
        let buffer = painted(text);
        let line = row(&buffer, 12);
        let inner: String = line.chars().skip(9).take(62).collect();
        assert_eq!(inner.trim(), text);
        let leading = inner.len() - inner.trim_start().len();
        let trailing = inner.len() - inner.trim_end().len();
        assert!(
            leading.abs_diff(trailing) <= 1,
            "the text sits in the middle: {leading} before, {trailing} after"
        );
    }

    /// A status line longer than the box wraps rather than running off the
    /// edge, and the box stays three rows: the overflow is clipped, not written
    /// outside the box.
    #[test]
    fn a_status_line_wider_than_the_box_neither_overflows_nor_grows_it() {
        let text = "Grouping ".repeat(20);
        let buffer = painted(&text);

        assert_eq!(row(&buffer, 10), "X".repeat(80));
        assert_eq!(row(&buffer, 14), "X".repeat(80));
        let line = row(&buffer, 12);
        assert_eq!(&line[..8], "XXXXXXXX", "nothing is written left of the box");
        assert!(
            line.chars()
                .skip(9)
                .take(62)
                .collect::<String>()
                .contains("Grouping"),
            "the beginning of the line is shown: {line}"
        );
    }
}
