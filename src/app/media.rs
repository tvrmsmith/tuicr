//! Loader port for the in-TUI media viewer (`gd-lc6.1`). `MediaJobs` is the
//! one real seam: `ThreadMediaJobs` runs the blocking probe/fetch/decode work
//! on background threads, and tests install a fake that records calls and
//! hands back a `Receiver` whose `Sender` the test holds.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, Resize};

use super::{App, InputMode};
use crate::forge::traits::ForgeRepository;
use crate::media::graphics::{self, ImageProtocolSetting};
use crate::media::open::{self, MediaError, Opened};
use crate::media::{MediaKind, MediaRef};

pub(crate) struct LoadRequest {
    pub generation: u64,
    pub index: usize,
    pub media: MediaRef,
    pub repo: Option<ForgeRepository>,
    pub picker: Picker,
    pub area: Rect,
    pub view: View,
    /// `Some` skips the fetch and decode and renders this image.
    pub source: Option<Arc<DynamicImage>>,
}

pub(crate) struct LoadResult {
    pub generation: u64,
    pub index: usize,
    pub area: Rect,
    /// Echoed from the request: a pan keeps the generation, so the result
    /// must say which view it drew.
    pub view: View,
    pub outcome: Result<Loaded, LoadError>,
}

pub(crate) struct Loaded {
    pub protocol: Protocol,
    /// The decoded image, cached by the viewer.
    pub source: Arc<DynamicImage>,
}

/// Which part of the source image a load draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Fit,
    /// Source-pixel crop, drawn one image pixel per terminal pixel.
    Window {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug)]
pub(crate) enum LoadError {
    /// A declared video, or a download that turned out to be one: the viewer
    /// shows the open-outside card instead of a decoder error.
    NotImage,
    Failed(String),
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
    /// Real adapter: thread running `decode_and_render`, which fetches the
    /// item (unless `req.source` is cached) and draws `req.view` of it via
    /// `render_view`.
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
                view: req.view,
                outcome,
            });
        });
        rx
    }
}

/// Open media viewer state. Holds only the current item's decoded image,
/// its encoded protocol (or its loading/failed status) and the zoom; paging
/// re-requests rather than caching every item.
pub(crate) struct MediaViewer {
    pub items: Vec<MediaRef>,
    pub index: usize,
    /// Bumped whenever the current item, its target area or the zoom level
    /// changes, so a `LoadResult` for a superseded request is dropped instead
    /// of applied. A pan keeps it, so the frame on screen stays until the
    /// panned one lands.
    pub generation: u64,
    pub current: MediaSlot,
    area: Option<Rect>,
    /// The `(generation, index, area, view)` of the last `load` request
    /// issued, so `poll_media_viewer_events` doesn't re-request every tick.
    requested: Option<(u64, usize, Rect, View)>,
    /// Channel for that request's in-flight `media_jobs.load` call. Owned by
    /// the viewer so closing it drops any result still on its way.
    load_rx: Option<Receiver<LoadResult>>,
    pub zoom: Zoom,
    /// The current item's decoded image, so a pan or zoom crops and encodes
    /// without decoding again. Cleared on paging.
    pub source: Option<Arc<DynamicImage>>,
}

/// Fit-to-area or 1:1, in which case `origin` is the window's top-left in
/// source pixels. The origin may lie outside the image after a resize; it is
/// clamped against the current area whenever a window is computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zoom {
    Fit,
    Actual { origin: (u32, u32) },
}

/// A 1:1 window over a source image: its size and the pan rules that follow.
#[derive(Clone, Copy)]
struct Geometry {
    image: (u32, u32),
    window: (u32, u32),
}

impl Geometry {
    fn new(image: &DynamicImage, area: Rect, font: FontSize) -> Self {
        let image = (image.width(), image.height());
        let window = (
            image.0.min(u32::from(area.width) * u32::from(font.width)),
            image.1.min(u32::from(area.height) * u32::from(font.height)),
        );
        Self { image, window }
    }

    /// The whole image fits the area, so 1:1 would equal fit.
    fn is_whole_image(&self) -> bool {
        self.window == self.image
    }

    fn clamp(&self, origin: (u32, u32)) -> (u32, u32) {
        (
            origin.0.min(self.image.0 - self.window.0),
            origin.1.min(self.image.1 - self.window.1),
        )
    }

    fn centred(&self) -> (u32, u32) {
        (
            (self.image.0 - self.window.0) / 2,
            (self.image.1 - self.window.1) / 2,
        )
    }

    /// One pan press: a quarter of the window along each axis, clamped.
    fn panned(&self, origin: (u32, u32), dx: i32, dy: i32) -> (u32, u32) {
        let (x, y) = self.clamp(origin);
        let step = |at: u32, dir: i32, window: u32| {
            if dir < 0 {
                at.saturating_sub(window / 4)
            } else {
                at.saturating_add(window / 4 * u32::from(dir > 0))
            }
        };
        self.clamp((step(x, dx, self.window.0), step(y, dy, self.window.1)))
    }

    fn view(&self, origin: (u32, u32)) -> View {
        let (x, y) = self.clamp(origin);
        View::Window {
            x,
            y,
            width: self.window.0,
            height: self.window.1,
        }
    }
}

pub(crate) enum MediaSlot {
    Loading,
    Ready {
        protocol: Protocol,
        area: Rect,
        /// The view this protocol draws.
        view: View,
    },
    NotImage,
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
            load_rx: None,
            zoom: Zoom::Fit,
            source: None,
        }
    }

    /// Called by the renderer every frame with the image area. A changed area
    /// supersedes the current draw, so it bumps the generation.
    pub fn set_area(&mut self, area: Rect) {
        if self.area.is_some_and(|prior| prior != area) {
            self.generation += 1;
        }
        self.area = Some(area);
    }

    fn geometry(&self, font: FontSize) -> Option<Geometry> {
        Some(Geometry::new(self.source.as_ref()?, self.area?, font))
    }

    /// The view the current zoom and area call for, or `None` when a zoomed
    /// window cannot be computed yet.
    fn wanted_view(&self, font: FontSize) -> Option<View> {
        match self.zoom {
            Zoom::Fit => Some(View::Fit),
            Zoom::Actual { origin } => Some(self.geometry(font)?.view(origin)),
        }
    }

    /// Show another item: back to fit, the cached image dropped.
    fn page_to(&mut self, index: usize) {
        self.index = index;
        self.generation += 1;
        self.current = MediaSlot::Loading;
        self.requested = None;
        self.zoom = Zoom::Fit;
        self.source = None;
    }
}

/// A declared video is `NotImage` without being downloaded. With a cached
/// `source` nothing is fetched; otherwise the item is fetched and handed to
/// `render_file`.
pub(crate) fn decode_and_render(req: &LoadRequest) -> Result<Loaded, LoadError> {
    if req.media.kind == MediaKind::Video {
        return Err(LoadError::NotImage);
    }
    if let Some(source) = &req.source {
        let protocol = render_view(source, &req.picker, req.area, req.view)?;
        return Ok(Loaded {
            protocol,
            source: Arc::clone(source),
        });
    }
    let path =
        open::fetch(&req.media, req.repo.as_ref()).map_err(|e| LoadError::Failed(e.to_string()))?;
    render_file(&path, &req.picker, req.area, req.view)
}

/// Draw `view` of `image` into a protocol sized for `area`. Only the visible
/// window is cropped out and encoded.
pub(crate) fn render_view(
    image: &DynamicImage,
    picker: &Picker,
    area: Rect,
    view: View,
) -> Result<Protocol, LoadError> {
    let visible = match view {
        View::Fit => image.clone(),
        View::Window {
            x,
            y,
            width,
            height,
        } => image.crop_imm(x, y, width, height),
    };
    picker
        .new_protocol(
            visible,
            area.as_size(),
            Resize::Fit(Some(FilterType::Lanczos3)),
        )
        .map_err(|e| LoadError::Failed(e.to_string()))
}

/// Decode the downloaded `path` and draw `view` of it for `area`. A file
/// `fetch` named with a video extension (from the served Content-Type or the
/// URL) is `NotImage` without a decode attempt.
pub(crate) fn render_file(
    path: &Path,
    picker: &Picker,
    area: Rect,
    view: View,
) -> Result<Loaded, LoadError> {
    let is_video_file = path.extension().is_some_and(|ext| {
        ["mp4", "mov", "webm"]
            .iter()
            .any(|video| ext.eq_ignore_ascii_case(video))
    });
    if is_video_file {
        return Err(LoadError::NotImage);
    }
    let failed = |e: &dyn std::fmt::Display| LoadError::Failed(e.to_string());
    // Sniff the format from the bytes: a bare user-attachments download is
    // saved without an extension when the server sends no known Content-Type.
    let image = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|e| failed(&e))?
        .decode()
        .map_err(|e| failed(&e))?;
    let source = Arc::new(image);
    let protocol = render_view(&source, picker, area, view)?;
    Ok(Loaded { protocol, source })
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

    /// The terminal cell size in pixels, once a picker exists.
    fn media_font_size(&self) -> Option<FontSize> {
        Some(self.image_picker.as_ref()?.as_ref()?.font_size())
    }

    /// `→`/`l`: pan right while zoomed, otherwise the next item, clamped (no
    /// wrap).
    pub(crate) fn media_right(&mut self) {
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        if viewer.zoom != Zoom::Fit {
            self.media_pan(1, 0);
        } else if viewer.index + 1 < viewer.items.len() {
            viewer.page_to(viewer.index + 1);
            self.force_full_repaint = true;
        }
    }

    /// `←`/`h`: pan left while zoomed, otherwise the previous item, clamped
    /// (no wrap).
    pub(crate) fn media_left(&mut self) {
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        if viewer.zoom != Zoom::Fit {
            self.media_pan(-1, 0);
        } else if viewer.index > 0 {
            viewer.page_to(viewer.index - 1);
            self.force_full_repaint = true;
        }
    }

    /// `↑`/`k`: pan up while zoomed.
    pub(crate) fn media_up(&mut self) {
        self.media_pan(0, -1);
    }

    /// `↓`/`j`: pan down while zoomed.
    pub(crate) fn media_down(&mut self) {
        self.media_pan(0, 1);
    }

    /// Moves the zoomed window a quarter of its size. The generation stays,
    /// so the frame on screen remains until the new window lands.
    fn media_pan(&mut self, dx: i32, dy: i32) {
        let Some(font) = self.media_font_size() else {
            return;
        };
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        let Zoom::Actual { origin } = viewer.zoom else {
            return;
        };
        let Some(geometry) = viewer.geometry(font) else {
            return;
        };
        viewer.zoom = Zoom::Actual {
            origin: geometry.panned(origin, dx, dy),
        };
    }

    /// `z`: toggle between fit and 1:1. Needs the decoded image, and does
    /// nothing when the whole image already fits the area.
    pub(crate) fn media_zoom(&mut self) {
        let Some(font) = self.media_font_size() else {
            return;
        };
        let Some(viewer) = self.media_viewer.as_mut() else {
            return;
        };
        let Some(geometry) = viewer.geometry(font) else {
            return;
        };
        viewer.zoom = match viewer.zoom {
            Zoom::Actual { .. } => Zoom::Fit,
            Zoom::Fit if geometry.is_whole_image() => return,
            Zoom::Fit => Zoom::Actual {
                origin: geometry.centred(),
            },
        };
        viewer.generation += 1;
        viewer.current = MediaSlot::Loading;
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
    /// it if it's stale) and, once the image
    /// area is known, issues the next `load` request the current item and
    /// area need. Returns whether either step changed anything worth
    /// redrawing for.
    pub fn poll_media_viewer_events(&mut self) -> bool {
        let mut redraw = self.drain_media_load_result();
        redraw |= self.request_media_load_if_needed();
        redraw
    }

    fn drain_media_load_result(&mut self) -> bool {
        let Some(viewer) = self.media_viewer.as_mut() else {
            return false;
        };
        let Some(rx) = viewer.load_rx.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                viewer.load_rx = None;
                if result.index != viewer.index {
                    return false;
                }
                let current = result.generation == viewer.generation;
                viewer.current = match result.outcome {
                    Ok(loaded) => {
                        // The index matched, so even a stale result decoded this item.
                        viewer.source = Some(loaded.source);
                        if !current {
                            return false;
                        }
                        MediaSlot::Ready {
                            protocol: loaded.protocol,
                            area: result.area,
                            view: result.view,
                        }
                    }
                    Err(_) if !current => return false,
                    Err(LoadError::NotImage) => MediaSlot::NotImage,
                    Err(LoadError::Failed(message)) => MediaSlot::Failed(message),
                };
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                viewer.load_rx = None;
                let is_current = viewer.requested.is_some_and(|(generation, index, ..)| {
                    generation == viewer.generation && index == viewer.index
                });
                if !is_current {
                    return false;
                }
                viewer.current = MediaSlot::Failed("media loader stopped unexpectedly".to_string());
                true
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
        let Some(view) = viewer.wanted_view(picker.font_size()) else {
            return false;
        };
        let is_ready = matches!(
            &viewer.current,
            MediaSlot::Ready { area: a, view: v, .. } if *a == area && *v == view
        );
        let is_requested = viewer.requested == Some((viewer.generation, viewer.index, area, view));
        if is_ready || is_requested {
            return false;
        }
        // One load in flight per generation: a pan waits for the frame on its
        // way, then the poll that draws it asks for the latest view.
        let in_flight = viewer.load_rx.is_some()
            && viewer
                .requested
                .is_some_and(|(generation, ..)| generation == viewer.generation);
        if in_flight {
            return false;
        }
        let stale_area = viewer
            .requested
            .is_some_and(|(_, _, prior, _)| prior != area)
            || matches!(&viewer.current, MediaSlot::Ready { area: a, .. } if *a != area);
        if stale_area {
            viewer.current = MediaSlot::Loading;
        }
        let generation = viewer.generation;
        let index = viewer.index;
        let Some(media) = viewer.items.get(index).cloned() else {
            return false;
        };
        viewer.requested = Some((generation, index, area, view));
        let repo = self
            .pr_info
            .as_ref()
            .map(|info| info.details.repository.clone());
        viewer.load_rx = Some(self.media_jobs.load(LoadRequest {
            generation,
            index,
            media,
            repo,
            picker,
            area,
            view,
            source: viewer.source.clone(),
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
                let label = self.media_open_label.take().unwrap_or_default();
                self.set_error(format!("Failed to open {label}"));
                true
            }
        }
    }
}
