//! `tuicr-session: <slug>` on stderr is the discovery contract for agents and
//! wrapper scripts: a slug they see is a session file they can attach to and
//! post comments into. These cover startup registration deciding whether there
//! is anything to announce.

use std::fs;
use std::path::{Path, PathBuf};

use super::release_boundary_tests::{build_app_with_session, session_with_files};
use super::target_selector_tests::TestReviewsDir;

#[test]
fn should_announce_the_slug_once_the_session_file_exists() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));

    let registration = app.register_session_at_startup();

    assert!(app.has_persisted_session_file());
    assert_eq!(registration.slug, app.session_slug());
    assert_eq!(registration.warning, None);
}

#[test]
fn should_not_announce_a_slug_before_a_review_target_is_chosen() {
    let _reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(false));

    let registration = app.register_session_at_startup();

    assert_eq!(
        registration.slug, None,
        "the pre-selection slug changes when a target is chosen, so it would \
         never receive a comment"
    );
    assert_eq!(
        registration.warning, None,
        "waiting for a target is the normal path, not a failure"
    );
}

/// A read-only reviews dir is the shape a real init failure takes: the slug
/// still derives, but the write it names never lands.
#[cfg(unix)]
#[test]
fn should_not_announce_a_slug_when_registration_fails() {
    let reviews_dir = TestReviewsDir::new();
    let mut app = build_app_with_session(session_with_files(true));
    let _read_only = ReadOnlyDir::new(reviews_dir.path());

    let registration = app.register_session_at_startup();

    assert!(!app.has_persisted_session_file(), "no file landed");
    assert!(
        app.session_slug().is_some(),
        "the slug derives on the failure path too — announcing it is what \
         would hand agents a slug pointing at nothing"
    );
    assert_eq!(
        registration.slug, None,
        "a failed registration announces nothing"
    );
    let warning = registration.warning.expect("the failure is reported");
    assert!(
        warning.starts_with("Failed to initialize review session file: "),
        "unexpected warning: {warning}"
    );
}

/// Makes a directory read-only, restoring write permission on drop so the temp
/// dir behind it can still be cleaned up.
#[cfg(unix)]
struct ReadOnlyDir(PathBuf);

#[cfg(unix)]
impl ReadOnlyDir {
    fn new(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o555)).unwrap();
        Self(path.to_path_buf())
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyDir {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}
