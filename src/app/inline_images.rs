use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::protocol::Protocol;

use super::App;
use super::media::{LoadError, LoadRequest, LoadResult, View};
use crate::media::{MediaKind, MediaLine};
use crate::ui::pr_info_panel;

/// How long a failed fetch waits before its slot asks again. The network
/// drops out and comes back, so a fetch failure is not final.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Consecutive failed loads after which a slot gives up until the PR body
/// changes. A bad URL or an undecodable image never starts working.
pub(crate) const MAX_ATTEMPTS: u32 = 3;

/// Thumbnail load state for the PR description's inline images.
#[derive(Default)]
pub(crate) struct InlineImages {
    /// The PR body `media` and `slots` were built for.
    body: String,
    /// `pr_body_media` of `body`, parsed once per body rather than per tick.
    media: Vec<MediaLine>,
    /// One per `media` entry.
    slots: Vec<Slot>,
    /// The `(media index, area)` pairs the latest frame drew.
    pub(crate) drawn: Vec<(usize, Rect)>,
}

#[derive(Default)]
struct Slot {
    /// The in-flight load, if any. At most one per slot.
    rx: Option<Receiver<LoadResult>>,
    /// The decoded image, so a resize encodes without fetching again.
    source: Option<Arc<DynamicImage>>,
    ready: Option<(Rect, Protocol)>,
    /// Set by `LoadError::NotImage`: the URL is not an image, so it is never
    /// requested again.
    not_image: bool,
    /// Set by a failed or disconnected load: no request before this instant.
    retry_at: Option<Instant>,
    /// Consecutive failed or disconnected loads. Reset by a successful load.
    failures: u32,
    /// Set once `failures` reaches `MAX_ATTEMPTS`: never requested again.
    gave_up: bool,
}

impl Slot {
    fn record_failure(&mut self, now: Instant) {
        self.failures += 1;
        if self.failures >= MAX_ATTEMPTS {
            self.gave_up = true;
        } else {
            self.retry_at = Some(now + RETRY_AFTER);
        }
    }
}

impl App {
    /// True when the `inline_images` flag is on and the terminal image probe
    /// found a picker. The flag gates the rows even if `enter_media` resolved
    /// the picker lazily.
    pub(crate) fn inline_images_enabled(&self) -> bool {
        self.show_inline_images && self.image_picker.as_ref().is_some_and(Option::is_some)
    }

    /// The reserved rows `[start, end)` of the inline thumbnail containing
    /// `line`, if any. The cursor never rests inside one.
    pub(crate) fn inline_image_block(&self, line: usize) -> Option<(usize, usize)> {
        if !self.inline_images_enabled() {
            return None;
        }
        let rows = pr_info_panel::pr_info_rows(self);
        let first = line.saturating_sub(pr_info_panel::THUMBNAIL_ROWS - 1);
        (first..=line.min(rows.len().checked_sub(1)?))
            .find(|&start| rows[start].thumbnail && line < start + pr_info_panel::THUMBNAIL_ROWS)
            .map(|start| (start, start + pr_info_panel::THUMBNAIL_ROWS))
    }

    /// Main-loop tick: applies finished thumbnail loads and requests the
    /// ones a new image or a resized box needs. Returns true iff it applied a
    /// result.
    pub fn poll_inline_images(&mut self) -> bool {
        self.poll_inline_images_at(Instant::now())
    }

    pub(crate) fn poll_inline_images_at(&mut self, now: Instant) -> bool {
        let Some(picker) = self.image_picker.as_ref().and_then(Option::as_ref).cloned() else {
            return false;
        };
        let Some(info) = self.pr_info.as_ref() else {
            return false;
        };
        let state = &mut self.inline_images;
        if state.body != info.details.body {
            state.body.clone_from(&info.details.body);
            state.media = pr_info_panel::pr_body_media(info);
            state.slots.clear();
            state.slots.resize_with(state.media.len(), Slot::default);
        }

        let mut redraw = false;
        for slot in &mut state.slots {
            let Some(rx) = slot.rx.as_ref() else {
                continue;
            };
            match rx.try_recv() {
                Ok(result) => {
                    slot.rx = None;
                    redraw = true;
                    match result.outcome {
                        Ok(loaded) => {
                            slot.source = Some(loaded.source);
                            slot.ready = Some((result.area, loaded.protocol));
                            slot.failures = 0;
                            slot.retry_at = None;
                        }
                        Err(LoadError::NotImage) => slot.not_image = true,
                        Err(LoadError::Failed(_)) => slot.record_failure(now),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    slot.rx = None;
                    slot.record_failure(now);
                    redraw = true;
                }
            }
        }

        let area = pr_info_panel::thumbnail_box(self.diff_state.viewport_width);
        if area.width == 0 || area.height == 0 {
            return redraw;
        }
        for (index, (line, slot)) in state.media.iter().zip(&mut state.slots).enumerate() {
            let current = slot.ready.as_ref().is_some_and(|(ready, _)| *ready == area);
            let waiting = slot.retry_at.is_some_and(|at| now < at);
            if line.media.kind != MediaKind::Image
                || slot.not_image
                || slot.gave_up
                || waiting
                || slot.rx.is_some()
                || current
            {
                continue;
            }
            slot.rx = Some(self.media_jobs.load(LoadRequest {
                generation: 0,
                index,
                media: line.media.clone(),
                repo: Some(info.details.repository.clone()),
                picker: picker.clone(),
                area,
                view: View::Fit,
                source: slot.source.clone(),
            }));
        }
        redraw
    }

    /// The encoded thumbnail for media `index`, only when it is Ready for
    /// exactly `size` and the slots were built for the current PR body.
    pub(crate) fn inline_image(&self, index: usize, size: Rect) -> Option<&Protocol> {
        let body = &self.pr_info.as_ref()?.details.body;
        if *body != self.inline_images.body {
            return None;
        }
        let (area, protocol) = self.inline_images.slots.get(index)?.ready.as_ref()?;
        (*area == size).then_some(protocol)
    }

    /// Startup probe (main.rs, before the main loop). The panel grows by the
    /// thumbnail rows once a picker is found, so a cursor or scroll position
    /// below the panel moves down with the content it pointed at.
    pub fn resolve_image_picker(&mut self) {
        let old_height = pr_info_panel::pr_info_render_height(self);
        let picker = if self.output_to_stdout {
            None
        } else {
            self.media_jobs.probe(&self.image_protocol)
        };
        self.image_picker = Some(picker);
        if self.pr_info.is_none() {
            return;
        }
        let new_height = pr_info_panel::pr_info_render_height(self);
        if new_height == old_height {
            return;
        }
        self.rebuild_annotations();
        for position in [
            &mut self.diff_state.cursor_line,
            &mut self.diff_state.scroll_offset,
        ] {
            if *position >= old_height {
                *position = (*position + new_height).saturating_sub(old_height);
            }
        }
    }
}
