//! `gd-lc6.5` A2: the startup image probe and the inline thumbnail load
//! state. Expected values come from the assignment's table, not from this
//! file's own logic.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

use crate::app::App;
use crate::app::inline_images::RETRY_AFTER;
use crate::app::media::{LoadError, LoadRequest, LoadResult, Loaded, MediaJobs, View};
use crate::app::tests::pr_info_tests::build_pr_app;
use crate::forge::traits::ForgeRepository;
use crate::media::MediaRef;
use crate::media::graphics::ImageProtocolSetting;
use crate::media::open::{MediaError, Opened};

pub(crate) const BODY: &str = "Intro\n![one](https://x.test/1.png)\n![two](https://x.test/2.png)\n<video src=\"https://x.test/v.mp4\"></video>";

pub(crate) struct LoadCall {
    pub index: usize,
    pub url: String,
    pub area: Rect,
    pub view: View,
    pub source: Option<Arc<image::DynamicImage>>,
}

/// Records every call and hands back a `Receiver` whose `Sender` the test
/// holds, so a test drives polling with results of its choosing.
pub(crate) struct FakeMediaJobs {
    pub probe_calls: Cell<usize>,
    probe_result: Option<Picker>,
    pub load_calls: RefCell<Vec<LoadCall>>,
    load_senders: RefCell<Vec<Sender<LoadResult>>>,
}

impl FakeMediaJobs {
    pub fn new(probe_result: Option<Picker>) -> Self {
        Self {
            probe_calls: Cell::new(0),
            probe_result,
            load_calls: RefCell::new(Vec::new()),
            load_senders: RefCell::new(Vec::new()),
        }
    }

    pub fn send_load(&self, call: usize, result: LoadResult) {
        self.load_senders.borrow()[call]
            .send(result)
            .expect("app still holds this load call's receiver");
    }

    /// Drops the call's `Sender`, as a worker thread that panicked would.
    pub fn disconnect_load(&self, call: usize) {
        drop(std::mem::replace(
            &mut self.load_senders.borrow_mut()[call],
            mpsc::channel().0,
        ));
    }

    pub fn load_count(&self) -> usize {
        self.load_calls.borrow().len()
    }
}

impl MediaJobs for FakeMediaJobs {
    fn probe(&self, _setting: &ImageProtocolSetting) -> Option<Picker> {
        self.probe_calls.set(self.probe_calls.get() + 1);
        self.probe_result.clone()
    }

    fn open_external(
        &self,
        _media: MediaRef,
        _repo: Option<ForgeRepository>,
    ) -> Receiver<Result<Opened, MediaError>> {
        mpsc::channel().1
    }

    fn load(&self, req: LoadRequest) -> Receiver<LoadResult> {
        self.load_calls.borrow_mut().push(LoadCall {
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

/// `build_pr_app()` with `BODY`, viewport 82 and a fake loader installed. The
/// picker is left unresolved.
pub(crate) fn setup(probe_result: Option<Picker>) -> (App, Rc<FakeMediaJobs>) {
    let mut app = build_pr_app();
    let mut info = app.pr_info.take().expect("pr info");
    info.details.body = BODY.to_string();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = 82;
    app.show_inline_images = true;
    app.rebuild_annotations();
    let fake = Rc::new(FakeMediaJobs::new(probe_result));
    app.media_jobs = Box::new(fake.clone());
    (app, fake)
}

/// `setup` with the picker already resolved to halfblocks.
pub(crate) fn setup_enabled() -> (App, Rc<FakeMediaJobs>) {
    let (mut app, fake) = setup(None);
    app.image_picker = Some(Some(Picker::halfblocks()));
    (app, fake)
}

// B1.1
#[test]
fn flag_off_adds_no_rows_even_with_a_resolved_picker() {
    let (mut app, _fake) = setup_enabled();
    app.show_inline_images = false;
    app.rebuild_annotations();
    assert!(!app.inline_images_enabled());
    let rows = crate::ui::pr_info_panel::pr_info_rows(&app);
    assert_eq!(rows[3].line.to_string(), "[image: two]");
    assert!(rows.iter().all(|row| !row.thumbnail));
}

// B1.2
#[test]
fn flag_on_without_a_picker_is_not_enabled() {
    let (mut app, _fake) = setup(None);
    app.show_inline_images = true;
    app.image_picker = Some(None);
    assert!(!app.inline_images_enabled());
}

// B1.3
#[test]
fn cursor_down_snaps_past_each_image_block() {
    let (mut app, _fake) = setup_enabled();
    app.diff_state.cursor_line = 2;
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, 13);
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, 24);
}

// B1.4
#[test]
fn cursor_up_snaps_to_the_caption_above_each_image_block() {
    let (mut app, _fake) = setup_enabled();
    app.diff_state.cursor_line = 24;
    app.cursor_up(1);
    assert_eq!(app.diff_state.cursor_line, 13);
    app.cursor_up(1);
    assert_eq!(app.diff_state.cursor_line, 2);
}

// B1.5
#[test]
fn a_multi_line_move_down_that_lands_in_a_block_snaps_past_it() {
    let (mut app, _fake) = setup_enabled();
    app.diff_state.cursor_line = 1;
    app.cursor_down(5);
    assert_eq!(app.diff_state.cursor_line, 13);
}

// B1.6
#[test]
fn a_multi_line_move_up_that_lands_in_a_block_snaps_to_its_caption() {
    let (mut app, _fake) = setup_enabled();
    app.diff_state.cursor_line = 13;
    app.cursor_up(3);
    assert_eq!(app.diff_state.cursor_line, 2);
}

// B1.7
#[test]
fn flag_off_cursor_down_steps_one_row() {
    let (mut app, _fake) = setup_enabled();
    app.show_inline_images = false;
    app.rebuild_annotations();
    app.diff_state.cursor_line = 2;
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, 3);
}

// B1.8
#[test]
fn the_viewport_follows_a_cursor_that_snapped_past_a_block() {
    let (mut app, _fake) = setup_enabled();
    app.diff_state.viewport_height = 10;
    app.diff_state.scroll_offset = 0;
    app.diff_state.cursor_line = 2;
    app.cursor_down(1);
    let scroll = app.diff_state.scroll_offset;
    assert!(scroll <= 13 && 13 < scroll + app.diff_state.viewport_height);
}

pub(crate) fn box_b() -> Rect {
    Rect::new(0, 0, 75, 9)
}

pub(crate) fn box_b2() -> Rect {
    Rect::new(0, 0, 93, 9)
}

fn render_height(app: &App) -> usize {
    crate::ui::pr_info_panel::pr_info_render_height(app)
}

/// An `Ok` result for `index` drawn at `area`, with a fresh source image.
pub(crate) fn ok_result(index: usize, area: Rect) -> (LoadResult, Arc<image::DynamicImage>) {
    let source = Arc::new(image::DynamicImage::new_rgb8(4, 4));
    let protocol = Picker::halfblocks()
        .new_protocol(
            (*source).clone(),
            area.as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap();
    let result = LoadResult {
        generation: 0,
        index,
        area,
        view: View::Fit,
        outcome: Ok(Loaded {
            protocol,
            source: Arc::clone(&source),
        }),
    };
    (result, source)
}

pub(crate) fn err_result(index: usize, area: Rect) -> LoadResult {
    LoadResult {
        generation: 0,
        index,
        area,
        view: View::Fit,
        outcome: Err(LoadError::Failed("x".into())),
    }
}

// 1
#[test]
fn resolve_grows_the_panel_and_keeps_a_cursor_inside_it() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    let before = render_height(&app);
    app.diff_state.cursor_line = 2;
    app.diff_state.scroll_offset = 0;
    app.resolve_image_picker();
    assert!(matches!(app.image_picker, Some(Some(_))));
    assert_eq!(render_height(&app), before + 20);
    assert_eq!(app.diff_state.cursor_line, 2);
    assert_eq!(app.diff_state.scroll_offset, 0);
}

// 2
#[test]
fn resolve_shifts_a_cursor_and_scroll_below_the_panel() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    let h = render_height(&app);
    let annotations = app.line_annotations.len();
    app.diff_state.cursor_line = h + 5;
    app.diff_state.scroll_offset = h + 1;
    app.resolve_image_picker();
    assert_eq!(app.diff_state.cursor_line, h + 25);
    assert_eq!(app.diff_state.scroll_offset, h + 21);
    assert_eq!(app.line_annotations.len(), annotations + 20);
}

#[test]
fn resolve_moves_a_cursor_on_the_first_row_below_the_panel() {
    let (mut app, _fake) = setup(Some(Picker::halfblocks()));
    let h = render_height(&app);
    app.diff_state.cursor_line = h;
    app.diff_state.scroll_offset = h;
    app.resolve_image_picker();
    assert_eq!(app.diff_state.cursor_line, h + 20);
    assert_eq!(app.diff_state.scroll_offset, h + 20);
}

// 3
#[test]
fn resolve_without_a_picker_changes_nothing() {
    let (mut app, fake) = setup(None);
    let h = render_height(&app);
    app.diff_state.cursor_line = h + 5;
    app.diff_state.scroll_offset = h + 1;
    app.resolve_image_picker();
    assert!(matches!(app.image_picker, Some(None)));
    assert_eq!(render_height(&app), h);
    assert_eq!(app.diff_state.cursor_line, h + 5);
    assert_eq!(app.diff_state.scroll_offset, h + 1);
    assert_eq!(fake.probe_calls.get(), 1);
}

// 4
#[test]
fn resolve_under_output_to_stdout_skips_the_probe() {
    let (mut app, fake) = setup(Some(Picker::halfblocks()));
    app.output_to_stdout = true;
    app.resolve_image_picker();
    assert_eq!(fake.probe_calls.get(), 0);
    assert!(matches!(app.image_picker, Some(None)));
}

fn assert_load(fake: &FakeMediaJobs, call: usize, index: usize, url: &str, area: Rect) {
    let calls = fake.load_calls.borrow();
    assert_eq!(calls[call].index, index);
    assert_eq!(calls[call].url, url);
    assert_eq!(calls[call].area, area);
    assert_eq!(calls[call].view, View::Fit);
}

// 5
#[test]
fn first_poll_requests_every_image_but_not_the_video() {
    let (mut app, fake) = setup_enabled();
    assert!(!app.poll_inline_images());
    assert_eq!(fake.load_count(), 2);
    assert_load(&fake, 0, 0, "https://x.test/1.png", box_b());
    assert_load(&fake, 1, 1, "https://x.test/2.png", box_b());
    assert!(fake.load_calls.borrow().iter().all(|c| c.source.is_none()));
}

// 6
#[test]
fn second_poll_with_nothing_sent_requests_nothing() {
    let (mut app, fake) = setup_enabled();
    app.poll_inline_images();
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 2);
}

// 7
#[test]
fn an_ok_result_becomes_ready_for_its_own_area_only() {
    let (mut app, fake) = setup_enabled();
    app.poll_inline_images();
    fake.send_load(0, ok_result(0, box_b()).0);
    assert!(app.poll_inline_images());
    assert!(app.inline_image(0, box_b()).is_some());
    assert!(app.inline_image(1, box_b()).is_none());
    assert!(app.inline_image(0, box_b2()).is_none());
}

// 8
#[test]
fn a_failed_result_is_requested_again_once_the_retry_delay_passes() {
    let (mut app, fake) = setup_enabled();
    let failed_at = Instant::now();
    app.poll_inline_images_at(failed_at);
    fake.send_load(1, err_result(1, box_b()));
    assert!(app.poll_inline_images_at(failed_at));
    assert!(app.inline_image(1, box_b()).is_none());
    app.poll_inline_images_at(failed_at + RETRY_AFTER - Duration::from_secs(1));
    assert_eq!(fake.load_count(), 2);
    app.poll_inline_images_at(failed_at + RETRY_AFTER);
    assert_eq!(fake.load_count(), 3);
    assert_load(&fake, 2, 1, "https://x.test/2.png", box_b());
}

// 8, disconnected
#[test]
fn a_disconnected_loader_is_requested_again_once_the_retry_delay_passes() {
    let (mut app, fake) = setup_enabled();
    let failed_at = Instant::now();
    app.poll_inline_images_at(failed_at);
    fake.disconnect_load(1);
    assert!(app.poll_inline_images_at(failed_at));
    assert!(app.inline_image(1, box_b()).is_none());
    app.poll_inline_images_at(failed_at + RETRY_AFTER - Duration::from_secs(1));
    assert_eq!(fake.load_count(), 2);
    app.poll_inline_images_at(failed_at + RETRY_AFTER);
    assert_eq!(fake.load_count(), 3);
    assert_load(&fake, 2, 1, "https://x.test/2.png", box_b());
}

#[test]
fn a_not_image_result_is_never_requested_again() {
    let (mut app, fake) = setup_enabled();
    let failed_at = Instant::now();
    app.poll_inline_images_at(failed_at);
    let mut result = err_result(1, box_b());
    result.outcome = Err(LoadError::NotImage);
    fake.send_load(1, result);
    assert!(app.poll_inline_images_at(failed_at));
    app.poll_inline_images_at(failed_at + RETRY_AFTER * 10);
    assert_eq!(fake.load_count(), 2);
}

fn loads_for(fake: &FakeMediaJobs, index: usize) -> usize {
    fake.load_calls
        .borrow()
        .iter()
        .filter(|c| c.index == index)
        .count()
}

#[test]
fn a_thumbnail_that_fails_three_times_in_a_row_is_never_requested_again() {
    let (mut app, fake) = setup_enabled();
    let mut now = Instant::now();
    app.poll_inline_images_at(now);
    for _ in 0..3 {
        let call = fake
            .load_calls
            .borrow()
            .iter()
            .rposition(|c| c.index == 0)
            .unwrap();
        fake.send_load(call, err_result(0, box_b()));
        app.poll_inline_images_at(now);
        now += RETRY_AFTER;
        app.poll_inline_images_at(now);
    }
    assert_eq!(loads_for(&fake, 0), 3);
    app.poll_inline_images_at(now + RETRY_AFTER * 10);
    assert_eq!(loads_for(&fake, 0), 3);
}

#[test]
fn a_success_resets_the_failure_count() {
    let (mut app, fake) = setup_enabled();
    let mut now = Instant::now();
    app.poll_inline_images_at(now);
    for _ in 0..2 {
        let call = fake
            .load_calls
            .borrow()
            .iter()
            .rposition(|c| c.index == 0)
            .unwrap();
        fake.send_load(call, err_result(0, box_b()));
        app.poll_inline_images_at(now);
        now += RETRY_AFTER;
        app.poll_inline_images_at(now);
    }
    assert_eq!(loads_for(&fake, 0), 3);
    fake.send_load(3, ok_result(0, box_b()).0);
    assert!(app.poll_inline_images_at(now));
    assert!(app.inline_image(0, box_b()).is_some());

    app.diff_state.viewport_width = 102;
    app.poll_inline_images_at(now);
    assert_eq!(loads_for(&fake, 0), 4);
    fake.send_load(4, err_result(0, box_b2()));
    app.poll_inline_images_at(now);
    app.poll_inline_images_at(now + RETRY_AFTER);
    assert_eq!(loads_for(&fake, 0), 5);
}

// 9
#[test]
fn a_resized_viewport_rerequests_with_the_cached_source() {
    let (mut app, fake) = setup_enabled();
    app.poll_inline_images();
    let (result, source) = ok_result(0, box_b());
    fake.send_load(0, result);
    app.poll_inline_images();
    app.diff_state.viewport_width = 102;
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 3);
    assert_load(&fake, 2, 0, "https://x.test/1.png", box_b2());
    let sent = fake.load_calls.borrow()[2].source.clone().unwrap();
    assert!(Arc::ptr_eq(&sent, &source));
    assert!(app.inline_image(0, box_b()).is_some());
    assert!(app.inline_image(0, box_b2()).is_none());
    fake.send_load(2, ok_result(0, box_b2()).0);
    assert!(app.poll_inline_images());
    assert!(app.inline_image(0, box_b2()).is_some());
}

// 10
#[test]
fn a_resize_waits_for_the_load_in_flight() {
    let (mut app, fake) = setup_enabled();
    app.poll_inline_images();
    app.diff_state.viewport_width = 102;
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 2);
    fake.send_load(0, ok_result(0, box_b()).0);
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 3);
    assert_load(&fake, 2, 0, "https://x.test/1.png", box_b2());
    assert!(fake.load_calls.borrow()[2].source.is_some());
}

// 11
#[test]
fn a_picker_that_found_nothing_loads_nothing() {
    let (mut app, fake) = setup_enabled();
    app.image_picker = Some(None);
    assert!(!app.poll_inline_images());
    assert_eq!(fake.load_count(), 0);
}

// 11b
#[test]
fn flag_off_loads_nothing_even_with_a_resolved_picker() {
    let (mut app, fake) = setup_enabled();
    app.show_inline_images = false;
    assert!(!app.poll_inline_images());
    assert_eq!(fake.load_count(), 0);
}

// 12
#[test]
fn an_empty_box_loads_nothing() {
    let (mut app, fake) = setup_enabled();
    app.diff_state.viewport_width = 3;
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 0);
}

// 13
#[test]
fn a_changed_body_resets_the_slots() {
    let (mut app, fake) = setup_enabled();
    app.poll_inline_images();
    fake.send_load(0, ok_result(0, box_b()).0);
    app.poll_inline_images();
    app.pr_info.as_mut().unwrap().details.body = "![three](https://x.test/3.png)".into();
    app.poll_inline_images();
    assert_eq!(fake.load_count(), 3);
    assert_load(&fake, 2, 0, "https://x.test/3.png", box_b());
    assert!(fake.load_calls.borrow()[2].source.is_none());
    assert!(app.inline_image(0, box_b()).is_none());
}

// A3: drawing the Ready thumbnails over their reserved rows.

const HALF: &str = "\u{2580}";

fn render(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// An `Ok` result whose image is solid red, so every drawn cell is `\u{2580}`.
fn red_result(index: usize, area: Rect) -> LoadResult {
    let red =
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([255, 0, 0])));
    let protocol = Picker::halfblocks()
        .new_protocol(
            red.clone(),
            area.as_size(),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap();
    LoadResult {
        generation: 0,
        index,
        area,
        view: View::Fit,
        outcome: Ok(Loaded {
            protocol,
            source: Arc::new(red),
        }),
    }
}

/// Both images Ready at the box the first frame settles on.
fn ready_app(picker: Option<Picker>) -> (App, Rc<FakeMediaJobs>) {
    let (mut app, fake) = setup(None);
    app.image_picker = Some(picker.clone());
    app.rebuild_annotations();
    render(&mut app, 120, 60);
    app.rebuild_annotations();
    app.poll_inline_images();
    let area = crate::ui::pr_info_panel::thumbnail_box(app.diff_state.viewport_width);
    for index in 0..fake.load_count() {
        fake.send_load(index, red_result(index, area));
    }
    app.poll_inline_images();
    (app, fake)
}

fn inner(app: &App) -> Rect {
    app.diff_inner_area.expect("diff rendered")
}

fn drawn_at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> bool {
    buffer[(x, y)].symbol() == HALF
}

// A3 1
#[test]
fn normal_mode_draws_both_thumbnails_and_keeps_the_caption() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(drawn_at(&buffer, i.x + 2, i.y + 3));
    assert!(drawn_at(&buffer, i.x + 2, i.y + 14));
    assert_eq!(buffer[(i.x + 2, i.y + 2)].symbol(), "[");
}

fn half_in_column(buffer: &ratatui::buffer::Buffer, x: u16, ys: std::ops::Range<u16>) -> bool {
    ys.into_iter().any(|y| drawn_at(buffer, x, y))
}

// A3 2
#[test]
fn a_partly_scrolled_out_thumbnail_is_not_drawn() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.diff_state.scroll_offset = 4;
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(!half_in_column(&buffer, i.x + 2, i.y..i.y + 10));
    assert!(drawn_at(&buffer, i.x + 2, i.y + 10));
}

// A3 3
#[test]
fn a_thumbnail_whose_rows_do_not_all_fit_is_not_drawn() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    let buffer = render(&mut app, 120, 24);
    let i = inner(&app);
    assert_eq!(i.height, 20);
    assert!(drawn_at(&buffer, i.x + 2, i.y + 3));
    assert!(!half_in_column(&buffer, i.x + 2, i.y + 14..i.y + 20));
}

// A3 4
#[test]
fn help_mode_hides_the_thumbnails_and_normal_mode_restores_them() {
    use crate::app::InputMode;
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.input_mode = InputMode::Help;
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(!drawn_at(&buffer, i.x + 2, i.y + 3));
    assert!(!drawn_at(&buffer, i.x + 2, i.y + 14));
    app.input_mode = InputMode::Normal;
    let buffer = render(&mut app, 120, 60);
    assert!(drawn_at(&buffer, i.x + 2, i.y + 3));
    assert!(drawn_at(&buffer, i.x + 2, i.y + 14));
}

// A3 5
#[test]
fn horizontal_scroll_hides_the_thumbnails() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.diff_state.wrap_lines = false;
    // A wide file header, so the renderer keeps scroll_x instead of clamping it to 0.
    app.diff_files[0].new_path = Some(format!("src/{}.rs", "d".repeat(150)).into());
    app.rebuild_annotations();
    app.diff_state.scroll_x = 1;
    let buffer = render(&mut app, 120, 60);
    assert_eq!(app.diff_state.scroll_x, 1);
    let i = inner(&app);
    assert!(!half_in_column(&buffer, i.x + 2, i.y..i.y + i.height));
    assert!(!half_in_column(&buffer, i.x + 1, i.y..i.y + i.height));
}

// A3 6
#[test]
fn placeholders_draw_no_thumbnails() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.image_picker = Some(None);
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    for x in i.x..i.x + i.width {
        assert!(!half_in_column(&buffer, x, i.y..i.y + i.height));
    }
}

// A3 7
#[test]
fn a_vanished_thumbnail_forces_a_full_repaint() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    render(&mut app, 120, 60);
    app.force_full_repaint = false;
    render(&mut app, 120, 60);
    assert!(!app.force_full_repaint);
    app.diff_state.scroll_offset = 4;
    render(&mut app, 120, 60);
    assert!(app.force_full_repaint);
}

#[test]
fn a_moved_thumbnail_forces_a_full_repaint() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    render(&mut app, 120, 60);
    let before = app.inline_images.drawn.clone();
    assert_eq!(before.len(), 2);
    app.force_full_repaint = false;
    app.diff_state.scroll_offset = 1;
    render(&mut app, 120, 60);
    assert!(app.force_full_repaint);
    let moved: Vec<_> = before
        .iter()
        .map(|&(index, r)| (index, Rect::new(r.x, r.y - 1, r.width, r.height)))
        .collect();
    assert_eq!(app.inline_images.drawn, moved);
}

#[test]
fn a_new_body_hides_the_old_thumbnails_before_the_next_poll() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    render(&mut app, 120, 60);
    app.force_full_repaint = false;
    let area = crate::ui::pr_info_panel::thumbnail_box(app.diff_state.viewport_width);
    assert!(app.inline_image(0, area).is_some());
    app.pr_info.as_mut().unwrap().details.body =
        "Intro\n![other](https://x.test/other.png)\n![two](https://x.test/2.png)".into();
    app.rebuild_annotations();
    assert!(app.inline_image(0, area).is_none());
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(!half_in_column(&buffer, i.x + 2, i.y..i.y + i.height));
    assert!(app.force_full_repaint);
    assert!(app.inline_images.drawn.is_empty());
}

// A3 8
#[test]
fn side_by_side_draws_the_thumbnails() {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.diff_view_mode = crate::app::DiffViewMode::SideBySide;
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(drawn_at(&buffer, i.x + 2, i.y + 3));
}

fn thumbnails_drawn(app: &mut App) -> bool {
    let buffer = render(app, 120, 60);
    let i = inner(app);
    drawn_at(&buffer, i.x + 2, i.y + 3) && drawn_at(&buffer, i.x + 2, i.y + 14)
}

#[test]
fn search_outside_help_draws_the_thumbnails() {
    use crate::app::InputMode;
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.input_mode = InputMode::Search;
    app.search_return_mode = InputMode::Normal;
    assert!(thumbnails_drawn(&mut app));
}

#[test]
fn search_inside_help_hides_the_thumbnails() {
    use crate::app::InputMode;
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.input_mode = InputMode::Search;
    app.search_return_mode = InputMode::Help;
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(!half_in_column(&buffer, i.x + 2, i.y..i.y + i.height));
}

#[test]
fn command_without_completion_matches_draws_the_thumbnails() {
    use crate::app::{CommandCompletionState, InputMode};
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.input_mode = InputMode::Command;
    app.command_completion = Some(CommandCompletionState {
        prefix: "zz".into(),
        matches: Vec::new(),
        selected: 0,
    });
    assert!(thumbnails_drawn(&mut app));
}

#[test]
fn command_with_completion_matches_hides_the_thumbnails() {
    use crate::app::{CommandCompletionState, InputMode};
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    app.input_mode = InputMode::Command;
    app.command_completion = Some(CommandCompletionState {
        prefix: "q".into(),
        matches: vec!["q", "quit"],
        selected: 0,
    });
    let buffer = render(&mut app, 120, 60);
    let i = inner(&app);
    assert!(!half_in_column(&buffer, i.x + 2, i.y..i.y + i.height));
}

/// Draws the thumbnails in Normal mode, then renders one frame in `mode`,
/// which does not draw the diff view.
fn repaints_on_leaving_for(mode: crate::app::InputMode) -> bool {
    let (mut app, _fake) = ready_app(Some(Picker::halfblocks()));
    assert!(thumbnails_drawn(&mut app));
    app.force_full_repaint = false;
    app.input_mode = mode;
    render(&mut app, 120, 60);
    app.force_full_repaint && app.inline_images.drawn.is_empty()
}

#[test]
fn opening_the_media_viewer_over_a_thumbnail_forces_a_full_repaint() {
    assert!(repaints_on_leaving_for(crate::app::InputMode::MediaViewer));
}

#[test]
fn opening_message_details_over_a_thumbnail_forces_a_full_repaint() {
    assert!(repaints_on_leaving_for(
        crate::app::InputMode::MessageDetails
    ));
}

#[test]
fn opening_the_summary_over_a_thumbnail_forces_a_full_repaint() {
    assert!(repaints_on_leaving_for(crate::app::InputMode::Summary));
}

#[test]
fn opening_the_commit_selector_over_a_thumbnail_forces_a_full_repaint() {
    assert!(repaints_on_leaving_for(crate::app::InputMode::CommitSelect));
}
