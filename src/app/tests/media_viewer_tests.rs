//! `gd-lc6.1` A5: the loader port, Enter on a media row, and the media
//! viewer's navigation/polling. Expected values come from `A5-viewer.md`'s
//! table, not from this file's own logic.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};

use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

use crate::app::media::{LoadRequest, LoadResult, MediaJobs, MediaSlot};
use crate::app::tests::pr_info_tests::build_pr_app;
use crate::app::{App, InputMode};
use crate::forge::traits::ForgeRepository;
use crate::media::MediaRef;
use crate::media::graphics::ImageProtocolSetting;
use crate::media::open::{MediaError, Opened};

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
        let _ = self.open_senders.borrow()[call].send(result);
    }

    fn send_load(&self, call: usize, result: LoadResult) {
        let _ = self.load_senders.borrow()[call].send(result);
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

fn viewer(app: &App) -> &crate::app::media::MediaViewer {
    app.media_viewer.as_ref().expect("media viewer open")
}

// 1: picker probes to None; cursor 2, Enter.
#[test]
fn enter_with_no_picker_opens_externally() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(*fake.open_calls.borrow(), vec!["https://x.test/1.png"]);
    assert_eq!(app.message.as_ref().unwrap().content, "Opening one…");
}

// 2: after #1, Ok(Opened::File(p)).
#[test]
fn open_result_file_reports_opened() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    fake.send_open(0, Ok(Opened::File("/tmp/one.png".into())));
    assert!(app.poll_media_open_events());
    assert_eq!(app.message.as_ref().unwrap().content, "Opened one");
}

// 3: after #1, Ok(Opened::Browser).
#[test]
fn open_result_browser_reports_opened_in_browser() {
    let (mut app, fake) = setup(None);
    app.diff_state.cursor_line = 2;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    assert_eq!(fake.probe_calls.get(), 1);
}

// 6: app.output_to_stdout = true.
#[test]
fn output_to_stdout_skips_the_probe() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.output_to_stdout = true;
    app.diff_state.cursor_line = 2;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    assert_eq!(fake.probe_calls.get(), 0);
    assert_eq!(fake.open_calls.borrow().len(), 1);
}

// 7: fake probe returns Some(picker); cursor 3, Enter.
#[test]
fn enter_with_a_picker_opens_the_viewer() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
    let repaint_before = app.force_full_repaint;
    app.media_next();
    assert_eq!(viewer(&app).index, 2);
    assert_eq!(app.force_full_repaint, repaint_before);
}

// 12: viewer at index 0, MediaPrev.
#[test]
fn media_prev_at_the_first_item_is_a_no_op() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 2;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    assert_eq!(viewer(&app).index, 0);
    app.media_prev();
    assert_eq!(viewer(&app).index, 0);
}

// 13: after #10, sends the 1st call's LoadResult (old generation, Ok).
#[test]
fn a_stale_generation_result_is_dropped() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
    app.poll_media_viewer_events();
    let stale_generation = fake.load_calls.borrow()[0].generation;
    fake.send_load(
        0,
        LoadResult {
            generation: stale_generation,
            index: 1,
            area: rect_r(),
            outcome: Ok(Picker::halfblocks()
                .new_protocol(
                    image::DynamicImage::new_rgb8(4, 4),
                    rect_r().as_size(),
                    ratatui_image::Resize::Fit(None),
                )
                .unwrap()),
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
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
        MediaSlot::Failed(message) => panic!("expected Ready, got Failed({message:?})"),
    }
}

// 15: after #14, set_area(R2), poll.
#[test]
fn a_new_area_reissues_the_load_at_the_new_size() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
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
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_next();
    app.poll_media_viewer_events();
    let generation = fake.load_calls.borrow()[1].generation;
    fake.send_load(
        1,
        LoadResult {
            generation,
            index: 2,
            area: rect_r(),
            outcome: Err("decode failed".to_string()),
        },
    );
    app.poll_media_viewer_events();
    match &viewer(&app).current {
        MediaSlot::Failed(message) => assert_eq!(message, "decode failed"),
        MediaSlot::Loading => panic!("expected Failed, got Loading"),
        MediaSlot::Ready { .. } => panic!("expected Failed, got Ready"),
    }
}

// 17: viewer open, MediaOpenExternal.
#[test]
fn media_open_external_fetches_the_current_item_and_keeps_the_viewer_open() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_open_external();
    assert_eq!(app.input_mode, InputMode::MediaViewer);
    assert_eq!(*fake.open_calls.borrow(), vec!["https://x.test/2.png"]);
    assert_eq!(app.message.as_ref().unwrap().content, "Opening two…");
}

// 18: viewer open, MediaClose.
#[test]
fn media_close_resets_mode_and_forces_a_repaint() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_close();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.media_viewer.is_none());
    assert!(app.force_full_repaint);
}

// 19: after #18, a LoadResult arrives, poll.
#[test]
fn a_load_result_after_close_does_not_panic() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.diff_state.cursor_line = 3;
    app.enter_media(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).unwrap());
    app.media_viewer.as_mut().unwrap().set_area(rect_r());
    app.poll_media_viewer_events();
    app.media_close();
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
    assert!(crate::ui::pr_info_panel::pr_info_media_at_cursor(&app).is_none());
    if let Some(index) = crate::ui::pr_info_panel::pr_info_media_at_cursor(&app) {
        app.enter_media(index);
    }
    assert_eq!(fake.probe_calls.get(), 0);
    assert_eq!(fake.open_calls.borrow().len(), 0);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.message.is_none());
}
