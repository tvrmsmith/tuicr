//! `gd-lc6.1` A5: the loader port, Enter on a media row, and the media
//! viewer's navigation/polling. Expected values come from `A5-viewer.md`'s
//! table, not from this file's own logic.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};

use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

use crate::app::media::{LoadError, LoadRequest, LoadResult, MediaJobs, MediaSlot};
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
    press_in_viewer(&mut app, Action::MediaNext);
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
    press_in_viewer(&mut app, Action::MediaNext);
    let repaint_before = app.force_full_repaint;
    press_in_viewer(&mut app, Action::MediaNext);
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
    press_in_viewer(&mut app, Action::MediaPrev);
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
    press_in_viewer(&mut app, Action::MediaNext);
    press_in_viewer(&mut app, Action::MediaPrev);
    let stale_generation = fake.load_calls.borrow()[0].generation;
    assert!(viewer(&app).generation > stale_generation);
    fake.send_load(
        0,
        LoadResult {
            generation: stale_generation,
            index: 1,
            area: rect_r(),
            outcome: Ok(protocol_for(rect_r())),
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
    press_in_viewer(&mut app, Action::MediaNext);
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
            outcome: Ok(protocol),
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
    press_in_viewer(&mut app, Action::MediaNext);
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
            outcome: Ok(protocol),
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
    press_in_viewer(&mut app, Action::MediaNext);
    app.poll_media_viewer_events();
    let generation = fake.load_calls.borrow()[1].generation;
    fake.send_load(
        1,
        LoadResult {
            generation,
            index: 2,
            area: rect_r(),
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

// 19: after #18, a LoadResult arrives, poll.
#[test]
fn a_load_result_after_close_does_not_panic() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    press_enter(&mut app);
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    press_in_viewer(&mut app, Action::MediaClose);
    let protocol = Picker::halfblocks()
        .new_protocol(
            image::DynamicImage::new_rgb8(4, 4),
            rect_r().as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap();
    fake.send_load(
        0,
        LoadResult {
            generation: 0,
            index: 1,
            area: rect_r(),
            outcome: Ok(protocol),
        },
    );
    app.poll_media_viewer_events();
    assert!(app.media_viewer.is_none());
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
    press_in_viewer(&mut app, Action::MediaNext);
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
    let outcome = crate::app::media::render_file(&path, &Picker::halfblocks(), rect_r());
    fake.send_load(
        0,
        LoadResult {
            generation: fake.load_calls.borrow()[0].generation,
            index: 1,
            area: rect_r(),
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
    let outcome = crate::app::media::decode_and_render(&LoadRequest {
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
    let outcome = crate::app::media::render_file(&path, &Picker::halfblocks(), rect_r());
    fake.send_load(
        0,
        LoadResult {
            generation: fake.load_calls.borrow()[0].generation,
            index: 1,
            area: rect_r(),
            outcome,
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
