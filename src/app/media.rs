//! Loader port for the in-TUI media viewer (`gd-lc6.1`). `MediaJobs` is the
//! one real seam: `ThreadMediaJobs` runs the blocking probe/fetch/decode work
//! on background threads, and tests install a fake that records calls and
//! hands back a `Receiver` whose `Sender` the test holds.

use std::io::IsTerminal;
use std::sync::mpsc::{self, Receiver};

use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, Resize};

use super::{App, InputMode};
use crate::forge::traits::ForgeRepository;
use crate::media::MediaRef;
use crate::media::graphics::{self, ImageProtocolSetting};
use crate::media::open::{self, MediaError, Opened};

pub(crate) struct LoadRequest {
    pub generation: u64,
    pub index: usize,
    pub media: MediaRef,
    pub repo: Option<ForgeRepository>,
    pub picker: Picker,
    pub area: Rect,
}

pub(crate) struct LoadResult {
    pub generation: u64,
    pub index: usize,
    pub area: Rect,
    pub outcome: Result<Protocol, String>,
}

pub(crate) trait MediaJobs {
    /// Real adapter: `graphics::probe`, returning `None` when stdin is not a
    /// terminal.
    fn probe(&self, setting: &ImageProtocolSetting) -> Option<Picker>;
    /// Real adapter: thread running `media::open::open_external`.
    fn open_external(
        &self,
        media: MediaRef,
        repo: Option<ForgeRepository>,
    ) -> Receiver<Result<Opened, MediaError>>;
    /// Real adapter: thread running `media::open::fetch`, `image::open`
    /// (decode), then `req.picker.new_protocol(..)`.
    fn load(&self, req: LoadRequest) -> Receiver<LoadResult>;
}

/// The real adapter: every method runs its blocking work on a background
/// thread and reports back over the returned `Receiver`.
pub(crate) struct ThreadMediaJobs;

impl MediaJobs for ThreadMediaJobs {
    fn probe(&self, setting: &ImageProtocolSetting) -> Option<Picker> {
        // The query writes escapes and reads the replies from stdin; with no
        // terminal there, nothing answers.
        if !std::io::stdin().is_terminal() {
            return None;
        }
        graphics::probe(setting)
    }

    fn open_external(
        &self,
        media: MediaRef,
        repo: Option<ForgeRepository>,
    ) -> Receiver<Result<Opened, MediaError>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = open::open_external(&media, repo.as_ref());
            let _ = tx.send(result);
        });
        rx
    }

    fn load(&self, req: LoadRequest) -> Receiver<LoadResult> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = decode_and_render(&req);
            let _ = tx.send(LoadResult {
                generation: req.generation,
                index: req.index,
                area: req.area,
                outcome,
            });
        });
        rx
    }
}

/// Open media viewer state. Holds only the current item's decoded protocol
/// (or its loading/failed status); paging re-requests rather than caching
/// every item.
pub(crate) struct MediaViewer {
    pub items: Vec<MediaRef>,
    pub index: usize,
    /// Bumped whenever the current item or its target area changes, so a
    /// `LoadResult` for a superseded request is dropped instead of applied.
    pub generation: u64,
    pub current: MediaSlot,
    area: Option<Rect>,
    /// The `(generation, index, area)` of the last `load` request issued, so
    /// `poll_media_viewer_events` doesn't re-request every tick.
    requested: Option<(u64, usize, Rect)>,
}

pub(crate) enum MediaSlot {
    Loading,
    Ready { protocol: Protocol, area: Rect },
    Failed(String),
}

impl MediaViewer {
    pub fn new(items: Vec<MediaRef>, index: usize) -> Self {
        Self {
            items,
            index,
            generation: 0,
            current: MediaSlot::Loading,
            area: None,
            requested: None,
        }
    }

    /// Called by the renderer every frame with the image area.
    pub fn set_area(&mut self, area: Rect) {
        self.area = Some(area);
    }
}

fn decode_and_render(req: &LoadRequest) -> Result<Protocol, String> {
    let path = open::fetch(&req.media, req.repo.as_ref()).map_err(|e| e.to_string())?;
    // Sniff the format from the bytes: a bare user-attachments download is
    // saved without an extension when the server sends no known Content-Type.
    let image = image::ImageReader::open(&path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    req.picker
        .new_protocol(
            image,
            req.area.as_size(),
            Resize::Fit(Some(FilterType::Lanczos3)),
        )
        .map_err(|e| e.to_string())
}

impl App {
    /// Enter on a PR-description media placeholder row (`index` is
    /// `pr_info_panel::pr_info_media_at_cursor`'s result): opens the in-TUI
    /// viewer when the terminal can draw images, otherwise fetches the item
    /// and hands it to the OS opener.
    pub(crate) fn enter_media(&mut self, index: usize) {
        let Some(info) = self.pr_info.as_ref() else {
            return;
        };
        let items: Vec<MediaRef> = crate::ui::pr_info_panel::pr_body_media(info)
            .into_iter()
            .map(|line| line.media)
            .collect();
        let Some(item) = items.get(index).cloned() else {
            return;
        };
        let repo = Some(info.details.repository.clone());

        if self.image_picker.is_none() {
            let picker = if self.output_to_stdout {
                None
            } else {
                self.media_jobs.probe(&self.image_protocol)
            };
            self.image_picker = Some(picker);
        }

        if self.image_picker.as_ref().is_some_and(Option::is_some) {
            self.input_mode = InputMode::MediaViewer;
            self.media_viewer = Some(MediaViewer::new(items, index));
        } else {
            self.spawn_media_open(item, repo);
        }
    }

    /// `→`/`l`: advance to the next item, clamped (no wrap).
    pub(crate) fn media_next(&mut self) {
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        if viewer.index + 1 >= viewer.items.len() {
            return;
        }
        viewer.index += 1;
        viewer.generation += 1;
        viewer.current = MediaSlot::Loading;
        viewer.requested = None;
        self.force_full_repaint = true;
    }

    /// `←`/`h`: go back to the previous item, clamped (no wrap).
    pub(crate) fn media_prev(&mut self) {
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        if viewer.index == 0 {
            return;
        }
        viewer.index -= 1;
        viewer.generation += 1;
        viewer.current = MediaSlot::Loading;
        viewer.requested = None;
        self.force_full_repaint = true;
    }

    /// `o`: fetch the current item and hand it to the OS opener. Leaves the
    /// viewer open.
    pub(crate) fn media_open_external(&mut self) {
        let Some(viewer) = self.media_viewer.as_ref() else {
            return;
        };
        let Some(item) = viewer.items.get(viewer.index).cloned() else {
            return;
        };
        let repo = self
            .pr_info
            .as_ref()
            .map(|info| info.details.repository.clone());
        self.spawn_media_open(item, repo);
    }

    /// `Esc`/`q`: close the viewer. The sixel/kitty escape it drew only
    /// clears on a full repaint.
    pub(crate) fn media_close(&mut self) {
        self.input_mode = InputMode::Normal;
        self.media_viewer = None;
        self.force_full_repaint = true;
    }

    fn spawn_media_open(&mut self, item: MediaRef, repo: Option<ForgeRepository>) {
        let label = item.label.clone();
        self.media_open_rx = Some(self.media_jobs.open_external(item, repo));
        self.media_open_label = Some(label.clone());
        self.set_message(format!("Opening {label}\u{2026}"));
    }

    /// Called every main-loop tick. Drains a finished `load` result (dropping
    /// it if it's stale or the viewer already closed) and, once the image
    /// area is known, issues the next `load` request the current item and
    /// area need. Returns whether either step changed anything worth
    /// redrawing for.
    pub fn poll_media_viewer_events(&mut self) -> bool {
        let mut redraw = self.drain_media_load_result();
        redraw |= self.request_media_load_if_needed();
        redraw
    }

    fn drain_media_load_result(&mut self) -> bool {
        let Some(rx) = self.media_load_rx.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.media_load_rx = None;
                let Some(viewer) = self.media_viewer.as_mut() else {
                    return false;
                };
                if result.generation != viewer.generation || result.index != viewer.index {
                    return false;
                }
                viewer.current = match result.outcome {
                    Ok(protocol) => MediaSlot::Ready {
                        protocol,
                        area: result.area,
                    },
                    Err(message) => MediaSlot::Failed(message),
                };
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.media_load_rx = None;
                false
            }
        }
    }

    fn request_media_load_if_needed(&mut self) -> bool {
        let Some(picker) = self.image_picker.as_ref().and_then(Option::as_ref).cloned() else {
            return false;
        };
        let Some(viewer) = self.media_viewer.as_mut() else {
            return false;
        };
        let Some(area) = viewer.area else {
            return false;
        };
        let is_requested = viewer.requested == Some((viewer.generation, viewer.index, area));
        let is_ready = matches!(&viewer.current, MediaSlot::Ready { area: a, .. } if *a == area);
        if is_requested || is_ready {
            return false;
        }
        let stale_area = viewer.requested.is_some_and(|(_, _, prior)| prior != area)
            || matches!(&viewer.current, MediaSlot::Ready { area: a, .. } if *a != area);
        if stale_area {
            viewer.generation += 1;
            viewer.current = MediaSlot::Loading;
        }
        let generation = viewer.generation;
        let index = viewer.index;
        let Some(media) = viewer.items.get(index).cloned() else {
            return false;
        };
        viewer.requested = Some((generation, index, area));
        let repo = self
            .pr_info
            .as_ref()
            .map(|info| info.details.repository.clone());
        self.media_load_rx = Some(self.media_jobs.load(LoadRequest {
            generation,
            index,
            media,
            repo,
            picker,
            area,
        }));
        true
    }

    /// Called every main-loop tick. Reports the outcome of an in-flight
    /// `open_external` call (from Enter or `MediaOpenExternal`).
    pub fn poll_media_open_events(&mut self) -> bool {
        let Some(rx) = self.media_open_rx.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.media_open_rx = None;
                let label = self.media_open_label.take().unwrap_or_default();
                match outcome {
                    Ok(Opened::File(_)) => self.set_message(format!("Opened {label}")),
                    Ok(Opened::Browser) => {
                        self.set_message(format!("Opened {label} in the browser"))
                    }
                    Err(error) => self.set_error(error.to_string()),
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.media_open_rx = None;
                self.media_open_label = None;
                false
            }
        }
    }
}
