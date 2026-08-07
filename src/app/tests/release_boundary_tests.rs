use super::target_selector_tests::{TestReviewsDir, build_app_from_parts};
use crate::app::*;
use crate::model::FileStatus;
use crate::vcs::traits::VcsType;

const SOME_HASH: u64 = 0xabc;

fn vcs_info() -> VcsInfo {
    VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    }
}

fn session_with_files(files: bool) -> ReviewSession {
    let info = vcs_info();
    let mut session = ReviewSession::new(
        info.root_path.clone(),
        info.head_commit.clone(),
        info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    if files {
        session.add_file(
            PathBuf::from("src/main.rs"),
            FileStatus::Modified,
            SOME_HASH,
        );
    }
    session
}

fn build_app_with_session(session: ReviewSession) -> App {
    build_app_from_parts(vcs_info(), Vec::new(), None, session)
}

fn add_comment(app: &mut App, content: &str) {
    app.session
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .push(Comment::new(
            content.to_string(),
            CommentType::from_id("note"),
            None,
        ));
}

#[test]
fn should_not_register_a_session_before_a_review_target_is_chosen() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(false));

    let saved = app
        .ensure_ephemeral_session_file()
        .expect("registration should not error");

    // A file-less session is the bare target selector. Registering it would
    // publish a `file_count: 0` attach target that never receives a comment.
    assert_eq!(saved, None);
    assert!(!app.has_persisted_session_file());
}

#[test]
fn should_register_a_session_once_it_has_files() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));

    app.ensure_ephemeral_session_file()
        .expect("registration should not error");

    assert!(app.has_persisted_session_file());
}

#[test]
fn should_publish_the_batch_and_persist_it_on_release() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));
    app.ensure_ephemeral_session_file().unwrap();
    add_comment(&mut app, "first");

    let released = app.release_comments().expect("release should not error");

    assert_eq!(released, 1);
    assert_eq!(app.session.release_count, 1);
    assert!(app.session.released_at.is_some());

    let path = app
        .session_path
        .clone()
        .expect("session should have a path");
    let on_disk = crate::persistence::storage::load_session(&path).expect("session should load");
    assert_eq!(on_disk.release_count, 1);
    assert_eq!(on_disk.unreleased_count(), 0);
    assert_eq!(
        on_disk
            .comments()
            .map(|comment| comment.released_in)
            .collect::<Vec<_>>(),
        vec![Some(1)]
    );
}

#[test]
fn should_leave_the_release_counter_alone_on_a_plain_save() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));
    app.ensure_ephemeral_session_file().unwrap();
    add_comment(&mut app, "first");

    // `:w` is not the boundary — it stays idempotent so mid-review saves never
    // publish a batch the human did not mean to send.
    app.save_current_session_merging_external()
        .expect("save should not error");

    assert_eq!(app.session.release_count, 0);
    assert_eq!(app.session.unreleased_count(), 1);
}

#[test]
fn should_republish_a_comment_that_was_edited_after_release() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));
    app.ensure_ephemeral_session_file().unwrap();
    add_comment(&mut app, "first");
    app.release_comments().unwrap();

    app.session
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments[0]
        .apply_edit("reworded".to_string(), CommentType::from_id("note"));
    assert_eq!(app.session.unreleased_count(), 1);

    let released = app.release_comments().unwrap();

    assert_eq!(released, 1);
    assert_eq!(app.session.release_count, 2);
    assert_eq!(
        app.session
            .comments()
            .map(|comment| comment.released_in)
            .collect::<Vec<_>>(),
        vec![Some(2)]
    );
}
