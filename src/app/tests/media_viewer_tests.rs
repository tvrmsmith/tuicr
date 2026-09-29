//! `gd-lc6.1` A5: the loader port, Enter on a media row, and the media
//! viewer's navigation/polling. Expected values come from `A5-viewer.md`'s
//! table, not from this file's own logic.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

use crate::app::media::{
    LoadError, LoadRequest, LoadResult, Loaded, MediaJobs, MediaSlot, View, decode_and_render,
    render_view,
};
use crate::app::tests::pr_info_tests::build_pr_app;
use crate::app::{App, InputMode};
use crate::forge::traits::ForgeRepository;
use crate::input::Action;
use crate::media::graphics::ImageProtocolSetting;
use crate::media::open::{MediaError, Opened};
use crate::media::{MediaKind, MediaRef};

const BODY: &str = "Intro\n![one](https://x.test/1.png)\n![two](https://x.test/2.png)\n<video src=\"https://x.test/v.mp4\"></video>";

fn rect_r() -> Rect {
    Rect::new(0, 1, 80, 22)
}

fn rect_r2() -> Rect {
    Rect::new(0, 1, 100, 30)
}

struct LoadCall {
    generation: u64,
    index: usize,
    url: String,
    area: Rect,
    view: View,
    source: Option<Arc<image::DynamicImage>>,
}

/// Records every call and hands back a `Receiver` whose `Sender` the test
/// holds, so a test drives the App's polling with results of its choosing
/// instead of a real thread's timing.
struct FakeMediaJobs {
    probe_calls: Cell<usize>,
    probe_result: Option<Picker>,
    load_calls: RefCell<Vec<LoadCall>>,
    load_senders: RefCell<Vec<Sender<LoadResult>>>,
    open_calls: RefCell<Vec<String>>,
    open_senders: RefCell<Vec<Sender<Result<Opened, MediaError>>>>,
}

impl FakeMediaJobs {
    fn new(probe_result: Option<Picker>) -> Self {
        Self {
            probe_calls: Cell::new(0),
            probe_result,
            load_calls: RefCell::new(Vec::new()),
            load_senders: RefCell::new(Vec::new()),
            open_calls: RefCell::new(Vec::new()),
            open_senders: RefCell::new(Vec::new()),
        }
    }

    fn send_open(&self, call: usize, result: Result<Opened, MediaError>) {
        self.open_senders.borrow()[call]
            .send(result)
            .expect("app still holds this open call's receiver");
    }

    fn send_load(&self, call: usize, result: LoadResult) {
        self.load_senders.borrow()[call]
            .send(result)
            .expect("app still holds this load call's receiver");
    }

    /// Drops the call's `Sender`, as a worker thread that panicked would.
    fn disconnect_open(&self, call: usize) {
        drop(std::mem::replace(
            &mut self.open_senders.borrow_mut()[call],
            mpsc::channel().0,
        ));
    }

    fn disconnect_load(&self, call: usize) {
        drop(std::mem::replace(
            &mut self.load_senders.borrow_mut()[call],
            mpsc::channel().0,
        ));
    }
}

impl MediaJobs for FakeMediaJobs {
    fn probe(&self, _setting: &ImageProtocolSetting) -> Option<Picker> {
        self.probe_calls.set(self.probe_calls.get() + 1);
        self.probe_result.clone()
    }

    fn open_external(
        &self,
        media: MediaRef,
        _repo: Option<ForgeRepository>,
    ) -> Receiver<Result<Opened, MediaError>> {
        self.open_calls.borrow_mut().push(media.url.clone());
        let (tx, rx) = mpsc::channel();
        self.open_senders.borrow_mut().push(tx);
        rx
    }

    fn load(&self, req: LoadRequest) -> Receiver<LoadResult> {
        self.load_calls.borrow_mut().push(LoadCall {
            generation: req.generation,
            index: req.index,
            url: req.media.url.clone(),
            area: req.area,
            view: req.view,
            source: req.source.clone(),
        });
        let (tx, rx) = mpsc::channel();
        self.load_senders.borrow_mut().push(tx);
        rx
    }
}

/// Lets `Rc<FakeMediaJobs>` fill `App.media_jobs`'s `Box<dyn MediaJobs>` slot
/// while the test keeps its own handle to inspect calls and send results.
impl MediaJobs for Rc<FakeMediaJobs> {
    fn probe(&self, setting: &ImageProtocolSetting) -> Option<Picker> {
        (**self).probe(setting)
    }

    fn open_external(
        &self,
        media: MediaRef,
        repo: Option<ForgeRepository>,
    ) -> Receiver<Result<Opened, MediaError>> {
        (**self).open_external(media, repo)
    }

    fn load(&self, req: LoadRequest) -> Receiver<LoadResult> {
        (**self).load(req)
    }
}

/// `build_pr_app()` with the body every expected-value row in `A5-viewer.md`
/// assumes, plus a wide enough viewport that the four body rows each render
/// as one line (row 0 is the panel's own title line, so body rows start at
/// 1: `Intro`, `[image: one]`, `[image: two]`, `[video: v.mp4]`).
fn setup(probe_result: Option<Picker>) -> (App, Rc<FakeMediaJobs>) {
    let mut app = build_pr_app();
    let mut info = app.pr_info.take().expect("pr info");
    info.details.body = BODY.to_string();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = 82;
    app.rebuild_annotations();
    let fake = Rc::new(FakeMediaJobs::new(probe_result));
    app.media_jobs = Box::new(fake.clone());
    (app, fake)
}

/// Enter through the real diff-view handler.
fn press_enter(app: &mut App) {
    crate::handler::handle_diff_action(app, Action::SelectFile);
}

/// A viewer key through the real media-viewer handler.
fn press_in_viewer(app: &mut App, action: Action) {
    crate::handler::handle_media_viewer_action(app, action);
}

fn protocol_for(area: Rect) -> ratatui_image::protocol::Protocol {
    Picker::halfblocks()
        .new_protocol(
            image::DynamicImage::new_rgb8(4, 4),
            area.as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap()
}

fn loaded_with(protocol: ratatui_image::protocol::Protocol) -> Loaded {
    Loaded {
        protocol,
        source: Arc::new(image::DynamicImage::new_rgb8(4, 4)),
    }
}

fn loaded_for(area: Rect) -> Loaded {
    loaded_with(protocol_for(area))
}

fn viewer(app: &App) -> &crate::app::media::MediaViewer {
    app.media_viewer.as_ref().expect("media viewer open")
}

// 1: picker probes to None; cursor 2, Enter.
#[test]
fn enter_with_no_picker_opens_externally() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(*fake.open_calls.borrow(), vec!["https://x.test/1.png"]);
    assert_eq!(app.message.as_ref().unwrap().content, "Opening one…");
}

// 2: after #1, Ok(Opened::File(p)).
#[test]
fn open_result_file_reports_opened() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    fake.send_open(0, Ok(Opened::File("/tmp/one.png".into())));
    assert!(app.poll_media_open_events());
    assert_eq!(app.message.as_ref().unwrap().content, "Opened one");
}

// 3: after #1, Ok(Opened::Browser).
#[test]
fn open_result_browser_reports_opened_in_browser() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    fake.send_open(0, Ok(Opened::Browser));
    app.poll_media_open_events();
    assert_eq!(
        app.message.as_ref().unwrap().content,
        "Opened one in the browser"
    );
}

// 4: after #1, Err(MediaError::Http { status: 500 }).
#[test]
fn open_result_error_sets_error_message() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    fake.send_open(0, Err(MediaError::Http { status: 500 }));
    app.poll_media_open_events();
    assert_eq!(
        app.message.as_ref().unwrap().content,
        "media request failed with status 500"
    );
}

// 5: fake probe returns None; Enter on row 2 twice.
#[test]
fn probe_runs_at_most_once() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    press_enter(&mut app);
    assert_eq!(fake.probe_calls.get(), 1);
}

// 6: app.output_to_stdout = true.
#[test]
fn output_to_stdout_skips_the_probe() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.output_to_stdout = true;
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    assert_eq!(fake.probe_calls.get(), 0);
    assert_eq!(fake.open_calls.borrow().len(), 1);
}

// 7: fake probe returns Some(picker); cursor 3, Enter.
#[test]
fn enter_with_a_picker_opens_the_viewer() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    assert_eq!(app.input_mode, InputMode::MediaViewer);
    let v = viewer(&app);
    assert_eq!(v.index, 1);
    assert_eq!(v.items.len(), 3);
    assert_eq!(fake.load_calls.borrow().len(), 0);
}

// 8: after #7, set_area(R), poll.
#[test]
fn set_area_issues_a_load_request() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    let calls = fake.load_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].index, 1);
    assert_eq!(calls[0].url, "https://x.test/2.png");
    assert_eq!(calls[0].area, rect_r());
}

// 9: after #8, poll again.
#[test]
fn polling_again_does_not_reissue_the_same_request() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.poll_media_viewer_events();
    assert_eq!(fake.load_calls.borrow().len(), 1);
}

// 10: after #8, MediaNext, poll.
#[test]
fn media_next_advances_and_reissues_a_load() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(viewer(&app).index, 2);
    let calls = fake.load_calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].index, 2);
    assert_eq!(calls[1].area, rect_r());
    assert!(calls[1].generation > calls[0].generation);
}

// 11: after #10, MediaNext (already last item).
#[test]
fn media_next_at_the_last_item_is_a_no_op() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    let repaint_before = app.force_full_repaint;
    press_in_viewer(&mut app, Action::MediaRight);
    assert_eq!(viewer(&app).index, 2);
    assert_eq!(app.force_full_repaint, repaint_before);
}

// 12: viewer at index 0, MediaPrev.
#[test]
fn media_prev_at_the_first_item_is_a_no_op() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    assert_eq!(viewer(&app).index, 0);
    press_in_viewer(&mut app, Action::MediaLeft);
    assert_eq!(viewer(&app).index, 0);
}

// 13: the viewer pages away and back before the 1st call's LoadResult
// (old generation, Ok) arrives on its still-open channel.
#[test]
fn a_stale_generation_result_is_dropped() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    press_in_viewer(&mut app, Action::MediaLeft);
    let stale_generation = fake.load_calls.borrow()[0].generation;
    assert!(viewer(&app).generation > stale_generation);
    fake.send_load(
        0,
        LoadResult {
            generation: stale_generation,
            index: 1,
            area: rect_r(),
            view: View::Fit,
            outcome: Ok(loaded_for(rect_r())),
        },
    );
    app.poll_media_viewer_events();
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
}

// 14: after #10, sends the 2nd call's LoadResult Ok(protocol).
#[test]
fn a_current_generation_ok_result_becomes_ready() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    let generation = fake.load_calls.borrow()[1].generation;
    let protocol = Picker::halfblocks()
        .new_protocol(
            image::DynamicImage::new_rgb8(4, 4),
            rect_r().as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap();
    fake.send_load(
        1,
        LoadResult {
            generation,
            index: 2,
            area: rect_r(),
            view: View::Fit,
            outcome: Ok(loaded_with(protocol)),
        },
    );
    app.poll_media_viewer_events();
    match &viewer(&app).current {
        MediaSlot::Ready { area, .. } => assert_eq!(*area, rect_r()),
        MediaSlot::Loading => panic!("expected Ready, got Loading"),
        MediaSlot::NotImage => panic!("expected Ready, got NotImage"),
        MediaSlot::Failed(message) => panic!("expected Ready, got Failed({message:?})"),
    }
}

// 15: after #14, set_area(R2), poll.
#[test]
fn a_new_area_reissues_the_load_at_the_new_size() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    let generation = fake.load_calls.borrow()[1].generation;
    let protocol = Picker::halfblocks()
        .new_protocol(
            image::DynamicImage::new_rgb8(4, 4),
            rect_r().as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap();
    fake.send_load(
        1,
        LoadResult {
            generation,
            index: 2,
            area: rect_r(),
            view: View::Fit,
            outcome: Ok(loaded_with(protocol)),
        },
    );
    app.poll_media_viewer_events();

    app.media_viewer.as_mut().unwrap().set_area(rect_r2());
    app.poll_media_viewer_events();
    let calls = fake.load_calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].index, 2);
    assert_eq!(calls[2].area, rect_r2());
}

// 16: after #10, sends Err("decode failed") for the current generation.
#[test]
fn a_current_generation_error_result_becomes_failed() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    let generation = fake.load_calls.borrow()[1].generation;
    fake.send_load(
        1,
        LoadResult {
            generation,
            index: 2,
            area: rect_r(),
            view: View::Fit,
            outcome: Err(LoadError::Failed("decode failed".to_string())),
        },
    );
    app.poll_media_viewer_events();
    match &viewer(&app).current {
        MediaSlot::Failed(message) => assert_eq!(message, "decode failed"),
        MediaSlot::Loading => panic!("expected Failed, got Loading"),
        MediaSlot::NotImage => panic!("expected Failed, got NotImage"),
        MediaSlot::Ready { .. } => panic!("expected Failed, got Ready"),
    }
}

// 17: viewer open, MediaOpenExternal.
#[test]
fn media_open_external_fetches_the_current_item_and_keeps_the_viewer_open() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    press_in_viewer(&mut app, Action::MediaOpenExternal);
    assert_eq!(app.input_mode, InputMode::MediaViewer);
    assert_eq!(*fake.open_calls.borrow(), vec!["https://x.test/2.png"]);
    assert_eq!(app.message.as_ref().unwrap().content, "Opening two…");
}

// 18: viewer open, MediaClose.
#[test]
fn media_close_resets_mode_and_forces_a_repaint() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    press_in_viewer(&mut app, Action::MediaClose);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.media_viewer.is_none());
    assert!(app.force_full_repaint);
}

// 19: after #18, a LoadResult arrives, poll; then Enter reopens the viewer
// on the same item. The old result must not fill the new viewer, which
// starts at the same generation and index.
#[test]
fn a_load_result_after_close_does_not_reach_a_reopened_viewer() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaClose);
    let stale = LoadResult {
        generation: fake.load_calls.borrow()[0].generation,
        index: 1,
        area: rect_r(),
        view: View::Fit,
        outcome: Ok(loaded_for(rect_r())),
    };
    let _ = fake.load_senders.borrow()[0].send(stale);
    app.poll_media_viewer_events();
    assert!(app.media_viewer.is_none());

    press_enter(&mut app);
    app.poll_media_viewer_events();
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    assert_eq!(fake.load_calls.borrow().len(), 2);
}

// 20: cursor 1 (Intro), Enter.
#[test]
fn enter_on_a_prose_row_falls_through_unchanged() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 1;
    press_enter(&mut app);
    assert_eq!(fake.probe_calls.get(), 0);
    assert_eq!(fake.open_calls.borrow().len(), 0);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.message.is_none());
}

// A load worker that dies without sending (a decoder panic) ends the spinner
// with an error instead of leaving the item Loading forever.
#[test]
fn a_load_worker_that_disconnects_fails_the_current_item() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    fake.disconnect_load(0);
    assert!(app.poll_media_viewer_events());
    assert!(matches!(viewer(&app).current, MediaSlot::Failed(_)));
    assert_eq!(fake.load_calls.borrow().len(), 1);
}

// A disconnect from a load the viewer already paged away from leaves the new
// item's Loading slot alone.
#[test]
fn a_stale_load_worker_disconnect_is_ignored() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaRight);
    fake.disconnect_load(0);
    app.poll_media_viewer_events();
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
    assert_eq!(fake.load_calls.borrow()[1].index, 2);
}

// An open worker that dies without sending replaces "Opening …" with an error.
#[test]
fn an_open_worker_that_disconnects_reports_an_error() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    press_enter(&mut app);
    fake.disconnect_open(0);
    assert!(app.poll_media_open_events());
    assert_eq!(app.message.as_ref().unwrap().content, "Failed to open one");
}

/// Opens the viewer on item 2 (`two`), writes `bytes` to a download named
/// `file_name`, runs the real `render_file` on it as a bare attachment, feeds
/// that result back through polling, and returns the drawn screen's rows.
fn viewer_rows_for_download(file_name: &str, bytes: &[u8]) -> Vec<String> {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(file_name);
    std::fs::write(&path, bytes).expect("write download");
    let outcome = crate::app::media::render_file(&path, &Picker::halfblocks(), rect_r(), View::Fit);
    fake.send_load(
        0,
        LoadResult {
            generation: fake.load_calls.borrow()[0].generation,
            index: 1,
            area: rect_r(),
            view: View::Fit,
            outcome,
        },
    );
    app.poll_media_viewer_events();

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 12)).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, &mut app))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect()
}

// A bare user-attachments URL served as video/mp4 is saved with an .mp4
// extension; the viewer shows the declared-video card, not a decoder error.
#[test]
fn an_attachment_that_downloads_as_video_shows_the_open_outside_card() {
    let mp4 = b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom";
    let rows = viewer_rows_for_download("0123456789abcdef.mp4", mp4);
    let card: Vec<&str> = rows[1..rows.len() - 1]
        .iter()
        .map(String::as_str)
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(card, vec!["two", "press o to open outside tuicr"]);
}

// A file that claims to be an image but does not decode keeps a readable
// error between the label and the hint.
#[test]
fn an_image_that_fails_to_decode_shows_its_error_on_the_card() {
    let rows = viewer_rows_for_download("0123456789abcdef.png", b"not a png");
    let card: Vec<&str> = rows[1..rows.len() - 1]
        .iter()
        .map(String::as_str)
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(card.len(), 3, "{card:?}");
    assert_eq!(card[0], "two");
    assert!(card[1].to_lowercase().contains("png"), "{card:?}");
    assert_eq!(card[2], "press o to open outside tuicr");
}

// A declared `<video>` is not downloaded: the load resolves to the
// open-outside card even when its URL cannot be fetched.
#[test]
fn a_declared_video_is_not_an_image() {
    let outcome = decode_and_render(&LoadRequest {
        generation: 0,
        index: 0,
        media: MediaRef {
            url: "http://127.0.0.1:1/v.mp4".to_string(),
            label: "v.mp4".to_string(),
            kind: MediaKind::Video,
        },
        repo: None,
        picker: Picker::halfblocks(),
        area: rect_r(),
        view: View::Fit,
        source: None,
    });
    assert!(matches!(outcome, Err(LoadError::NotImage)));
}

// A bare attachment saved without an extension decodes by sniffing its bytes
// and shows as Ready.
#[test]
fn an_extensionless_png_download_becomes_ready() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("0123456789abcdef");
    image::DynamicImage::new_rgb8(4, 4)
        .save_with_format(&path, image::ImageFormat::Png)
        .expect("write png");
    let outcome = crate::app::media::render_file(&path, &Picker::halfblocks(), rect_r(), View::Fit);
    let source = outcome
        .as_ref()
        .map(|loaded| loaded.source.clone())
        .unwrap();
    fake.send_load(
        0,
        LoadResult {
            generation: fake.load_calls.borrow()[0].generation,
            index: 1,
            area: rect_r(),
            view: View::Fit,
            outcome,
        },
    );
    app.poll_media_viewer_events();
    assert_eq!((source.width(), source.height()), (4, 4));
    match &viewer(&app).current {
        MediaSlot::Ready { area, .. } => assert_eq!(*area, rect_r()),
        MediaSlot::Loading => panic!("expected Ready, got Loading"),
        MediaSlot::NotImage => panic!("expected Ready, got NotImage"),
        MediaSlot::Failed(message) => panic!("expected Ready, got Failed({message:?})"),
    }
}

/// 2000x1000, red left half and blue right half.
fn split_image() -> image::DynamicImage {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(2000, 1000, |x, _| {
        if x < 1000 {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    }))
}

fn area_80x22() -> Rect {
    Rect::new(0, 0, 80, 22)
}

fn render_window(view: View) -> (ratatui_image::protocol::Protocol, ratatui::buffer::Buffer) {
    let protocol = render_view(&split_image(), &Picker::halfblocks(), area_80x22(), view).unwrap();
    let mut buffer = ratatui::buffer::Buffer::empty(area_80x22());
    ratatui::widgets::Widget::render(
        ratatui_image::Image::new(&protocol),
        area_80x22(),
        &mut buffer,
    );
    (protocol, buffer)
}

fn assert_cell(buffer: &ratatui::buffer::Buffer, x: u16, y: u16, rgb: (u8, u8, u8)) {
    let cell = &buffer[(x, y)];
    let color = ratatui::style::Color::Rgb(rgb.0, rgb.1, rgb.2);
    assert_eq!((cell.fg, cell.bg), (color, color), "cell ({x}, {y})");
}

// Fit scales the 2:1 image to the width of the area: 80x20 cells.
#[test]
fn fit_scales_the_whole_image_into_the_area() {
    let (protocol, _) = render_window(View::Fit);
    let size = protocol.size();
    assert_eq!((size.width, size.height), (80, 20));
}

// A window over the right half is all blue, drawn unscaled over the whole area.
#[test]
fn a_window_over_the_right_half_draws_only_blue() {
    let (protocol, buffer) = render_window(View::Window {
        x: 1200,
        y: 0,
        width: 800,
        height: 440,
    });
    let size = protocol.size();
    assert_eq!((size.width, size.height), (80, 22));
    for (x, y) in [(0, 0), (79, 0), (0, 21), (79, 21)] {
        assert_cell(&buffer, x, y, (0, 0, 255));
    }
}

// A window over the lower left is all red.
#[test]
fn a_window_over_the_lower_left_draws_only_red() {
    let (protocol, buffer) = render_window(View::Window {
        x: 200,
        y: 560,
        width: 800,
        height: 440,
    });
    let size = protocol.size();
    assert_eq!((size.width, size.height), (80, 22));
    for (x, y) in [(0, 0), (79, 0), (0, 21), (79, 21)] {
        assert_cell(&buffer, x, y, (255, 0, 0));
    }
}

// A window straddling the red/blue seam shows red on its left edge, blue on its right.
#[test]
fn a_window_across_the_seam_draws_both_colours() {
    let (_, buffer) = render_window(View::Window {
        x: 600,
        y: 0,
        width: 800,
        height: 440,
    });
    assert_eq!(buffer[(0, 0)].fg, ratatui::style::Color::Rgb(255, 0, 0));
    assert_eq!(buffer[(79, 0)].fg, ratatui::style::Color::Rgb(0, 0, 255));
}

fn image_request(
    media: MediaRef,
    source: Option<Arc<image::DynamicImage>>,
    view: View,
) -> LoadRequest {
    LoadRequest {
        generation: 0,
        index: 0,
        media,
        repo: None,
        picker: Picker::halfblocks(),
        area: area_80x22(),
        view,
        source,
    }
}

// A cached source is rendered as given: the host never resolves, so a fetch would fail.
#[test]
fn a_cached_source_renders_without_fetching() {
    let source = Arc::new(split_image());
    let media = MediaRef {
        url: "https://x.test/1.png".to_string(),
        label: "one".to_string(),
        kind: MediaKind::Image,
    };
    let view = View::Window {
        x: 1200,
        y: 0,
        width: 800,
        height: 440,
    };
    let loaded = decode_and_render(&image_request(media, Some(source.clone()), view)).unwrap();
    let size = loaded.protocol.size();
    assert_eq!((size.width, size.height), (80, 22));
    assert!(Arc::ptr_eq(&loaded.source, &source));
}

// A declared video stays NotImage even when a source is supplied.
#[test]
fn a_declared_video_with_a_source_is_not_an_image() {
    let media = MediaRef {
        url: "https://x.test/v.mp4".to_string(),
        label: "v.mp4".to_string(),
        kind: MediaKind::Video,
    };
    let outcome = decode_and_render(&image_request(
        media,
        Some(Arc::new(split_image())),
        View::Fit,
    ));
    assert!(matches!(outcome, Err(LoadError::NotImage)));
}

// Zoom and pan (gd-b78 A2). Fixture: font 10x20 px, `rect_r()` is 800x440 px,
// `rect_r2()` is 1000x600 px, the source image is 2000x1000.

fn source_image() -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::new_rgb8(2000, 1000))
}

fn window(x: u32, y: u32) -> View {
    View::Window {
        x,
        y,
        width: 800,
        height: 440,
    }
}

/// Answers load call `call` with the generation, index, and area it was
/// issued with, the given `view`, and `source`.
fn answer_call(fake: &FakeMediaJobs, call: usize, view: View, source: &Arc<image::DynamicImage>) {
    let (generation, index, area) = {
        let calls = fake.load_calls.borrow();
        (calls[call].generation, calls[call].index, calls[call].area)
    };
    fake.send_load(
        call,
        LoadResult {
            generation,
            index,
            area,
            view,
            outcome: Ok(Loaded {
                protocol: protocol_for(area),
                source: Arc::clone(source),
            }),
        },
    );
}

/// The viewer on item index 1 with `rect_r()` set and polled, call 0 issued
/// and not yet answered.
fn open_viewer_at_r() -> (App, Rc<FakeMediaJobs>) {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    (app, fake)
}

/// State R: `open_viewer_at_r` with call 0 answered at `View::Fit`, slot Ready.
fn state_r(source: &Arc<image::DynamicImage>) -> (App, Rc<FakeMediaJobs>) {
    let (mut app, fake) = open_viewer_at_r();
    answer_call(&fake, 0, View::Fit, source);
    app.poll_media_viewer_events();
    (app, fake)
}

/// State Z: state R, zoomed, the zoom call answered at `window(600, 280)`.
fn state_z(source: &Arc<image::DynamicImage>) -> (App, Rc<FakeMediaJobs>) {
    let (mut app, fake) = state_r(source);
    press_in_viewer(&mut app, Action::MediaZoom);
    app.poll_media_viewer_events();
    answer_call(&fake, 1, window(600, 280), source);
    app.poll_media_viewer_events();
    (app, fake)
}

/// Presses `action`, polls, and answers the call it issued (if any) with that
/// call's own generation and view, then polls again.
fn press_resolved(
    app: &mut App,
    fake: &FakeMediaJobs,
    source: &Arc<image::DynamicImage>,
    action: Action,
) {
    let before = fake.load_calls.borrow().len();
    press_in_viewer(app, action);
    app.poll_media_viewer_events();
    if fake.load_calls.borrow().len() > before {
        let view = fake.load_calls.borrow()[before].view;
        answer_call(fake, before, view, source);
        app.poll_media_viewer_events();
    }
}

fn call_count(fake: &FakeMediaJobs) -> usize {
    fake.load_calls.borrow().len()
}

fn call_view(fake: &FakeMediaJobs, call: usize) -> View {
    fake.load_calls.borrow()[call].view
}

fn ready_view(app: &App) -> View {
    match &viewer(app).current {
        MediaSlot::Ready { view, .. } => *view,
        _ => panic!("expected the slot to be Ready"),
    }
}

// Row 1
#[test]
fn zoom_from_fit_requests_a_centred_window_of_the_cached_source() {
    let img = source_image();
    let (mut app, fake) = state_r(&img);
    app.force_full_repaint = false;
    press_in_viewer(&mut app, Action::MediaZoom);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 2);
    let calls = fake.load_calls.borrow();
    assert_eq!(calls[1].index, 1);
    assert_eq!(calls[1].area, rect_r());
    assert_eq!(calls[1].view, window(600, 280));
    assert!(Arc::ptr_eq(calls[1].source.as_ref().unwrap(), &img));
    assert!(calls[1].generation > calls[0].generation);
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
    assert!(app.force_full_repaint);
}

// Row 2
#[test]
fn up_and_down_at_fit_do_nothing() {
    let img = source_image();
    let (mut app, fake) = state_r(&img);
    press_in_viewer(&mut app, Action::MediaDown);
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaUp);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 1);
    assert_eq!(viewer(&app).index, 1);
    assert!(matches!(viewer(&app).current, MediaSlot::Ready { .. }));
}

// Row 3
#[test]
fn right_at_fit_pages_and_drops_the_cached_source() {
    let img = source_image();
    let (mut app, fake) = state_r(&img);
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(viewer(&app).index, 2);
    let calls = fake.load_calls.borrow();
    assert!(calls[1].source.is_none());
    assert_eq!(calls[1].view, View::Fit);
}

// Row 4
#[test]
fn left_at_fit_pages_back() {
    let img = source_image();
    let (mut app, _fake) = state_r(&img);
    press_in_viewer(&mut app, Action::MediaLeft);
    assert_eq!(viewer(&app).index, 0);
}

// Row 5
#[test]
fn pan_keeps_the_generation_and_the_frame_on_screen() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 3);
    assert_eq!(call_view(&fake, 2), window(800, 280));
    let calls = fake.load_calls.borrow();
    assert_eq!(calls[2].generation, calls[1].generation);
    drop(calls);
    assert_eq!(ready_view(&app), window(600, 280));
}

// Row 6
#[test]
fn panning_right_stops_at_the_image_edge() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    for _ in 0..3 {
        press_resolved(&mut app, &fake, &img, Action::MediaRight);
    }
    assert_eq!(call_view(&fake, 2), window(800, 280));
    assert_eq!(call_view(&fake, 3), window(1000, 280));
    assert_eq!(call_view(&fake, 4), window(1200, 280));
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 5);
}

// Row 7
#[test]
fn panning_up_stops_at_the_image_edge() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    for _ in 0..3 {
        press_resolved(&mut app, &fake, &img, Action::MediaUp);
    }
    assert_eq!(call_view(&fake, 2), window(600, 170));
    assert_eq!(call_view(&fake, 3), window(600, 60));
    assert_eq!(call_view(&fake, 4), window(600, 0));
    press_in_viewer(&mut app, Action::MediaUp);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 5);
}

// Row 8
#[test]
fn panning_down_moves_a_quarter_window() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    press_in_viewer(&mut app, Action::MediaDown);
    app.poll_media_viewer_events();
    assert_eq!(call_view(&fake, 2), window(600, 390));
}

// Row 9
#[test]
fn panning_left_moves_a_quarter_window() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    press_in_viewer(&mut app, Action::MediaLeft);
    app.poll_media_viewer_events();
    assert_eq!(call_view(&fake, 2), window(400, 280));
    assert_eq!(viewer(&app).index, 1);
}

// Row 10
#[test]
fn a_pan_waits_while_a_frame_is_in_flight() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(call_view(&fake, 2), window(800, 280));
    press_in_viewer(&mut app, Action::MediaRight);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 3);
}

// Row 11
#[test]
fn the_poll_that_draws_a_frame_requests_the_latest_view() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    for _ in 0..2 {
        press_in_viewer(&mut app, Action::MediaRight);
        app.poll_media_viewer_events();
    }
    answer_call(&fake, 2, window(800, 280), &img);
    app.poll_media_viewer_events();
    assert_eq!(ready_view(&app), window(800, 280));
    assert_eq!(call_count(&fake), 4);
    assert_eq!(call_view(&fake, 3), window(1000, 280));
}

// Row 12
#[test]
fn zoom_from_actual_returns_to_fit_with_the_cached_source() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    app.force_full_repaint = false;
    press_in_viewer(&mut app, Action::MediaZoom);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 3);
    let calls = fake.load_calls.borrow();
    assert_eq!(calls[2].view, View::Fit);
    assert!(Arc::ptr_eq(calls[2].source.as_ref().unwrap(), &img));
    assert!(calls[2].generation > calls[1].generation);
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
    assert!(app.force_full_repaint);
}

// Row 13
#[test]
fn after_zooming_back_to_fit_right_pages_again() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    press_resolved(&mut app, &fake, &img, Action::MediaZoom);
    press_in_viewer(&mut app, Action::MediaRight);
    assert_eq!(viewer(&app).index, 2);
}

// Row 14
#[test]
fn zoom_and_pan_do_nothing_before_the_image_is_decoded() {
    let (mut app, fake) = open_viewer_at_r();
    press_in_viewer(&mut app, Action::MediaZoom);
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaDown);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 1);
    press_in_viewer(&mut app, Action::MediaRight);
    assert_eq!(viewer(&app).index, 2);
}

// Row 15
#[test]
fn zoom_does_nothing_when_the_whole_image_fits_the_area() {
    let small = Arc::new(image::DynamicImage::new_rgb8(400, 200));
    let (mut app, fake) = state_r(&small);
    press_in_viewer(&mut app, Action::MediaZoom);
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), 1);
    assert!(matches!(viewer(&app).current, MediaSlot::Ready { .. }));
}

// Row 16
#[test]
fn a_resize_reclamps_the_window_against_the_new_area() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    for _ in 0..3 {
        press_resolved(&mut app, &fake, &img, Action::MediaRight);
    }
    let before = call_count(&fake);
    let generation_before = fake.load_calls.borrow()[before - 1].generation;
    app.media_viewer.as_mut().unwrap().set_area(rect_r2());
    app.poll_media_viewer_events();
    assert_eq!(call_count(&fake), before + 1);
    let calls = fake.load_calls.borrow();
    let resized = calls.last().unwrap();
    assert_eq!(
        resized.view,
        View::Window {
            x: 1000,
            y: 280,
            width: 1000,
            height: 600
        }
    );
    assert!(Arc::ptr_eq(resized.source.as_ref().unwrap(), &img));
    assert!(resized.generation > generation_before);
}

// Row 17
#[test]
fn panning_after_a_resize_starts_from_the_clamped_origin() {
    let img = source_image();
    let (mut app, fake) = state_z(&img);
    for _ in 0..3 {
        press_resolved(&mut app, &fake, &img, Action::MediaRight);
    }
    app.media_viewer.as_mut().unwrap().set_area(rect_r2());
    app.poll_media_viewer_events();
    let resized = call_count(&fake) - 1;
    let view = call_view(&fake, resized);
    answer_call(&fake, resized, view, &img);
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaLeft);
    app.poll_media_viewer_events();
    assert_eq!(
        fake.load_calls.borrow().last().unwrap().view,
        View::Window {
            x: 750,
            y: 280,
            width: 1000,
            height: 600
        }
    );
}

// Row 18
#[test]
fn a_stale_result_caches_the_source_and_the_same_poll_reissues_from_it() {
    let img = source_image();
    let (mut app, fake) = open_viewer_at_r();
    app.media_viewer.as_mut().unwrap().set_area(rect_r2());
    answer_call(&fake, 0, View::Fit, &img);
    app.poll_media_viewer_events();
    assert!(matches!(viewer(&app).current, MediaSlot::Loading));
    assert_eq!(call_count(&fake), 2);
    let calls = fake.load_calls.borrow();
    assert_eq!(calls[1].area, rect_r2());
    assert_eq!(calls[1].view, View::Fit);
    assert!(Arc::ptr_eq(calls[1].source.as_ref().unwrap(), &img));
}
