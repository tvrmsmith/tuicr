//! Startup collects warnings from sources that do not know about each other,
//! so any run can produce several at once against one message slot. These cover
//! the queue that shows all of them: `App::set_startup_warnings` takes the slot
//! for the first and hands it to the next each time a TTL runs out.

use std::thread;
use std::time::{Duration, Instant};

use super::grouping_tests::ungrouped_app;
use crate::app::{App, MESSAGE_TTL_WARNING};

fn app() -> App {
    ungrouped_app(Vec::new())
}

/// The content of the message slot, or the empty string when nothing is in it.
pub(super) fn message(app: &App) -> String {
    app.message
        .as_ref()
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// Run the current message's TTL out, the way the main loop's tick does, and
/// let whatever is queued behind it take the slot.
pub(super) fn expire_message(app: &mut App) {
    let expires_at = &mut app
        .message
        .as_mut()
        .expect("a message is in the slot")
        .expires_at;
    *expires_at = Some(Instant::now());
    assert!(app.clear_expired_message(), "the TTL was honoured");
}

#[test]
fn every_startup_warning_reaches_the_slot_in_arrival_order() {
    let mut app = app();
    let warnings: Vec<String> = (1..=4).map(|n| format!("warning {n}")).collect();

    app.set_startup_warnings(warnings.clone());

    for (index, warning) in warnings.iter().enumerate() {
        assert_eq!(
            message(&app),
            format!("{warning} ({}/4)", index + 1),
            "the queue drains in arrival order, counted so the reader knows \
             more are coming"
        );
        if index + 1 < warnings.len() {
            expire_message(&mut app);
        }
    }

    expire_message(&mut app);
    assert_eq!(message(&app), "", "the last one leaves the slot empty");
}

#[test]
fn a_lone_startup_warning_is_not_counted() {
    let mut app = app();

    app.set_startup_warnings(vec!["the only warning".to_string()]);

    assert_eq!(
        message(&app),
        "the only warning",
        "a `(1/1)` would be noise on the common case"
    );
}

#[test]
fn a_quiet_startup_leaves_the_slot_empty() {
    let mut app = app();

    app.set_startup_warnings(Vec::new());

    assert_eq!(message(&app), "");
}

#[test]
fn a_message_the_human_asked_for_replaces_the_queue() {
    let mut app = app();
    app.set_startup_warnings(vec!["first".to_string(), "second".to_string()]);

    app.set_message("reviewed");

    assert_eq!(message(&app), "reviewed");
    expire_message(&mut app);
    assert_eq!(
        message(&app),
        "",
        "an answer to a keypress ends the startup queue rather than being \
         followed by it: the human has moved on"
    );
}

#[test]
fn a_warning_set_before_a_slow_first_frame_keeps_its_full_ttl_on_screen() {
    let mut app = app();
    app.set_warning("docs/guide.md shows as source");

    assert!(
        !app.clear_expired_message(),
        "a message nobody has seen yet does not expire"
    );
    // The first frame of a PR load can block on forge reads for seconds.
    thread::sleep(Duration::from_millis(20));
    let drawn_at = Instant::now();
    app.start_message_clock();

    let expires_at = app
        .message
        .as_ref()
        .and_then(|m| m.expires_at)
        .expect("a drawn warning has a deadline");
    assert!(
        expires_at >= drawn_at + MESSAGE_TTL_WARNING,
        "the TTL counts from the first drawn frame, not from when the warning was set"
    );
}
