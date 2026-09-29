//! Fetch media referenced from a pull request into a local cache, and hand it
//! to the OS viewer or a browser.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::forge::traits::{ForgeKind, ForgeRepository};
use crate::media::MediaRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    File(PathBuf),
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// 401, 403 or 404: GitHub answers 404 for a private asset without access.
    Unauthorized {
        status: u16,
    },
    Http {
        status: u16,
    },
    /// A 2xx response that is neither a known image/video Content-Type nor
    /// sniffs as one, such as an HTML error page.
    NotMedia {
        content_type: String,
    },
    Io(String),
    Open(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Unauthorized { status } => write!(
                f,
                "media request failed with status {status}: run `gh auth login`"
            ),
            MediaError::Http { status } => write!(f, "media request failed with status {status}"),
            MediaError::NotMedia { content_type } => write!(
                f,
                "media URL did not return an image or video (Content-Type: {content_type})"
            ),
            MediaError::Io(message) => write!(f, "media io error: {message}"),
            MediaError::Open(message) => write!(f, "failed to open media: {message}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// Download `media` into the per-user cache dir, reusing a cached file.
/// Blocking; call off the UI thread.
pub fn fetch(media: &MediaRef, repo: Option<&ForgeRepository>) -> Result<PathBuf, MediaError> {
    let token_wanted = wants_token(&media.url, repo);
    let token = if token_wanted {
        repo.and_then(|repo| gh_token(&repo.host))
    } else {
        None
    };
    fetch_to_dir(&media.url, token_wanted, token.as_deref(), &cache_dir())
}

/// `fetch`, then hand the file to the OS viewer. Opens the URL in the browser
/// instead when the download has no known media extension, so the OS never
/// picks a handler for an unknown type, or on an unauthorized fetch of a URL
/// that wanted a token.
pub fn open_external(
    media: &MediaRef,
    repo: Option<&ForgeRepository>,
) -> Result<Opened, MediaError> {
    open_with(
        &media.url,
        wants_token(&media.url, repo),
        fetch(media, repo),
        spawn_viewer,
    )
}

/// `open_external`'s routing of a finished fetch of `url`, with the OS
/// viewer passed in as `spawn` so tests can observe what it is handed.
fn open_with(
    url: &str,
    token_wanted: bool,
    fetched: Result<PathBuf, MediaError>,
    spawn: impl FnOnce(&str) -> Result<(), MediaError>,
) -> Result<Opened, MediaError> {
    match fetched {
        Ok(path) if path.extension().is_none() => {
            spawn(url)?;
            Ok(Opened::Browser)
        }
        Ok(path) => {
            spawn(path.as_os_str().to_string_lossy().as_ref())?;
            Ok(Opened::File(path))
        }
        Err(MediaError::Unauthorized { .. }) if token_wanted => {
            spawn(url)?;
            Ok(Opened::Browser)
        }
        Err(err) => Err(err),
    }
}

/// The directory `fetch` downloads into and caches from: the user's platform
/// cache dir, or the temp dir when no home directory resolves.
fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "tuicr")
        .map(|dirs| dirs.cache_dir().join("media"))
        .unwrap_or_else(|| std::env::temp_dir().join("tuicr-media"))
}

/// Create `dir` and its parents, owner-only on Unix so no other local user
/// can plant or read cached files.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// `gh`'s cached token for `host`, or `None` on any failure (not installed,
/// not logged in, empty output). Not unit tested: it shells out to `gh`.
fn gh_token(host: &str) -> Option<String> {
    let output =
        crate::process::run_command_output("gh", None, ["auth", "token", "--hostname", host])
            .ok()?;
    let token = output.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Hand `target` (a file path or a URL) to the OS viewer: `open` on macOS,
/// `xdg-open` elsewhere. Detached, with stdio silenced.
fn spawn_viewer(target: &str) -> Result<(), MediaError> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| MediaError::Open(err.to_string()))?;
    Ok(())
}

/// Whether a GitHub token should be sent with the request: the repo is on
/// GitHub and the URL is https on the repo's host.
fn wants_token(url: &str, repo: Option<&ForgeRepository>) -> bool {
    let Some(repo) = repo else {
        return false;
    };
    if repo.kind != ForgeKind::GitHub {
        return false;
    }
    let Ok(uri) = url.parse::<ureq::http::Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("https") && uri.host() == Some(repo.host.as_str())
}

/// A file already in `dir` for `hash`, either bare or with an extension.
fn cached_file(dir: &Path, hash: &str) -> Option<PathBuf> {
    let prefix_with_ext = format!("{hash}.");
    std::fs::read_dir(dir).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_str()?;
        if name == hash || name.starts_with(&prefix_with_ext) {
            Some(entry.path())
        } else {
            None
        }
    })
}

/// File extension for a `Content-Type` value, parameters ignored. `None` for
/// any type not in the known image/video set.
fn extension_from_content_type(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next().unwrap_or("").trim();
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        _ => None,
    }
}

/// The image and video extensions a download may be saved with; anything
/// else is saved bare so the OS opener never sees a type the PR author chose.
const MEDIA_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "svg", "mp4", "mov", "webm",
];

/// File extension from the URL path's last segment (query/fragment
/// stripped), when it is in `MEDIA_EXTENSIONS` ignoring case. Case is
/// preserved.
fn extension_from_url_path(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = last_segment.rsplit_once('.')?;
    MEDIA_EXTENSIONS
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known))
        .then_some(ext)
}

/// Whether a body starting with `head` is a png, jpeg, gif or webp image, or
/// an mp4/mov (ISO `ftyp` box) or webm (EBML) container.
fn sniffs_as_media(head: &[u8]) -> bool {
    head.starts_with(b"\x89PNG\r\n\x1a\n")
        || head.starts_with(b"\xFF\xD8\xFF")
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP".as_slice()))
        || head.get(4..8) == Some(b"ftyp".as_slice())
        || head.starts_with(b"\x1A\x45\xDF\xA3")
}

/// Download `url` into `dir`, reusing a cached file. Only media is cached: a
/// 2xx body with no known media Content-Type must sniff as media, else it is
/// `NotMedia`, or `Unauthorized` when `token_wanted` (a login page served in
/// place of a private asset). Blocking; the internal seam `fetch` builds on.
fn fetch_to_dir(
    url: &str,
    token_wanted: bool,
    token: Option<&str>,
    dir: &Path,
) -> Result<PathBuf, MediaError> {
    let hash = format!("{:016x}", crate::hash::fnv1a_64(url.as_bytes()));
    if let Some(cached) = cached_file(dir, &hash) {
        return Ok(cached);
    }

    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // No global timeout: a screen recording can take longer than any fixed
        // budget to stream, so only the connection and first byte are bounded.
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build();
    let agent: ureq::Agent = config.into();

    let mut request = agent.get(url);
    if let Some(token) = token {
        request = request.header("Authorization", format!("token {token}"));
    }
    let mut response = request
        .call()
        .map_err(|err| MediaError::Io(err.to_string()))?;

    let status = response.status().as_u16();
    match status {
        401 | 403 | 404 => return Err(MediaError::Unauthorized { status }),
        200..=299 => {}
        _ => return Err(MediaError::Http { status }),
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let known_type_ext = extension_from_content_type(&content_type);
    let mut reader = response.body_mut().as_reader();
    let mut head = Vec::new();
    (&mut reader)
        .take(16)
        .read_to_end(&mut head)
        .map_err(|err| MediaError::Io(err.to_string()))?;
    if known_type_ext.is_none() && !sniffs_as_media(&head) {
        return Err(if token_wanted {
            MediaError::Unauthorized { status }
        } else {
            MediaError::NotMedia { content_type }
        });
    }
    let ext = known_type_ext.or_else(|| extension_from_url_path(url));

    let file_name = match ext {
        Some(ext) => format!("{hash}.{ext}"),
        None => hash,
    };

    let final_path = dir.join(&file_name);
    // Each download streams into its own temp file, so concurrent fetches of
    // one URL never share a file; the atomic rename picks the cached copy.
    let persist_result = (|| -> io::Result<()> {
        create_private_dir(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&head)?;
        io::copy(&mut reader, &mut file)?;
        file.persist(&final_path)?;
        Ok(())
    })();
    persist_result.map_err(|err| MediaError::Io(err.to_string()))?;

    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

    /// A canned response the stub server sends for one request.
    struct StubResponse {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    }

    /// The request line's path plus the header names/values the stub server
    /// received, lower-cased header names as HTTP requires.
    type SeenRequest = Vec<(String, String)>;

    fn read_request(stream: &TcpStream) -> (String, SeenRequest) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("read request line");
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_string();

        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
            }
        }
        (path, headers)
    }

    fn write_response(mut stream: &TcpStream, response: &StubResponse) {
        write!(stream, "HTTP/1.1 {} status\r\n", response.status).expect("write status line");
        for (name, value) in &response.headers {
            write!(stream, "{name}: {value}\r\n").expect("write header");
        }
        // The stub serves one request per connection, so tell the client not
        // to reuse it; a pooled reuse after the redirect hits a closed socket.
        write!(stream, "Connection: close\r\n").expect("write connection header");
        let declares_length = response
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
        if !declares_length {
            write!(stream, "Content-Length: {}\r\n", response.body.len())
                .expect("write content length");
        }
        write!(stream, "\r\n").expect("write blank line");
        stream.write_all(&response.body).expect("write body");
        stream.flush().expect("flush response");
    }

    /// Serve up to `max_requests` connections on `127.0.0.1`, each answered by
    /// `handle(request_index, path, base_url)`. `base_url` is this server's
    /// own address, known only once bound, for handlers that redirect back
    /// into the same server. Returns the base URL and the requests seen so
    /// far (grows as the server serves them).
    fn spawn_server(
        max_requests: usize,
        mut handle: impl FnMut(usize, &str, &str) -> StubResponse + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<SeenRequest>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let port = listener.local_addr().expect("local addr").port();
        let base_url = format!("http://127.0.0.1:{port}");
        let base_url_in_thread = base_url.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_in_thread = Arc::clone(&seen);

        thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                if index >= max_requests {
                    break;
                }
                let Ok(stream) = stream else { break };
                let (path, headers) = read_request(&stream);
                seen_in_thread
                    .lock()
                    .expect("lock seen requests")
                    .push(headers);
                let response = handle(index, &path, &base_url_in_thread);
                write_response(&stream, &response);
            }
        });

        (base_url, seen)
    }

    #[test]
    fn unauthorized_display_ends_with_gh_auth_login_hint() {
        let message = MediaError::Unauthorized { status: 404 }.to_string();
        assert!(message.ends_with("run `gh auth login`"));
    }

    #[test]
    fn wants_token_for_github_asset_on_the_repo_host() {
        let repo = ForgeRepository::github("github.com", "owner", "repo");
        assert!(wants_token(
            "https://github.com/user-attachments/assets/x",
            Some(&repo)
        ));
    }

    #[test]
    fn does_not_want_token_over_plain_http() {
        let repo = ForgeRepository::github("github.com", "owner", "repo");
        assert!(!wants_token("http://github.com/a.png", Some(&repo)));
    }

    #[test]
    fn does_not_want_token_for_a_different_host() {
        let repo = ForgeRepository::github("github.com", "owner", "repo");
        assert!(!wants_token(
            "https://private-user-images.githubusercontent.com/1/a.png?jwt=z",
            Some(&repo)
        ));
    }

    #[test]
    fn wants_token_for_a_ghe_host_matching_the_repo_host() {
        let repo = ForgeRepository::github("ghe.example.com", "owner", "repo");
        assert!(wants_token(
            "https://ghe.example.com/user-attachments/assets/x",
            Some(&repo)
        ));
    }

    #[test]
    fn does_not_want_token_when_repo_is_not_github() {
        let repo = ForgeRepository::gitlab("github.com", "owner", "repo");
        assert!(!wants_token(
            "https://github.com/user-attachments/assets/x",
            Some(&repo)
        ));
    }

    #[test]
    fn does_not_want_token_without_a_repo() {
        assert!(!wants_token(
            "https://github.com/user-attachments/assets/x",
            None
        ));
    }

    #[test]
    fn does_not_want_token_for_an_unparsable_url() {
        let repo = ForgeRepository::github("github.com", "owner", "repo");
        assert!(!wants_token("not a url", Some(&repo)));
    }

    #[test]
    fn fetch_to_dir_downloads_and_names_by_content_type() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/png".to_string())],
            body: b"PNGDATA".to_vec(),
        });

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        let expected_name = format!("{:016x}.png", crate::hash::fnv1a_64(url.as_bytes()));
        assert_eq!(path, dir.path().join(expected_name));
        assert_eq!(std::fs::read(&path).expect("read fetched file"), b"PNGDATA");
    }

    #[test]
    fn fetch_to_dir_maps_video_mp4_content_type() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "video/mp4".to_string())],
            body: b"MP4DATA".to_vec(),
        });

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("mp4"));
    }

    #[test]
    fn fetch_to_dir_ignores_content_type_parameters() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/jpeg; charset=binary".to_string())],
            body: b"JPGDATA".to_vec(),
        });

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("jpg"));
    }

    #[test]
    fn fetch_to_dir_falls_back_to_url_extension_case_preserved() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, path, _base_url| {
            assert_eq!(path, "/shot.PNG?x=1");
            StubResponse {
                status: 200,
                headers: vec![("Content-Type", "application/octet-stream".to_string())],
                body: PNG_MAGIC.to_vec(),
            }
        });
        let url = format!("{url}/shot.PNG?x=1");

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("PNG"));
    }

    #[test]
    fn fetch_to_dir_drops_a_url_extension_outside_the_media_set() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "application/octet-stream".to_string())],
            body: PNG_MAGIC.to_vec(),
        });
        let url = format!("{url}/x.jar");

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        let expected_name = format!("{:016x}", crate::hash::fnv1a_64(url.as_bytes()));
        assert_eq!(path, dir.path().join(expected_name));
    }

    #[cfg(unix)]
    #[test]
    fn fetch_to_dir_creates_a_missing_cache_dir_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("temp dir");
        let dir = root.path().join("tuicr").join("media");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/png".to_string())],
            body: b"PNGDATA".to_vec(),
        });

        fetch_to_dir(&url, false, None, &dir).expect("fetch succeeds");

        let mode = std::fs::metadata(&dir)
            .expect("dir metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn fetch_to_dir_has_no_extension_when_none_is_known() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, path, _base_url| {
            assert_eq!(path, "/assets/abc");
            StubResponse {
                status: 200,
                headers: vec![("Content-Type", "application/octet-stream".to_string())],
                body: PNG_MAGIC.to_vec(),
            }
        });
        let url = format!("{url}/assets/abc");

        let path = fetch_to_dir(&url, false, None, dir.path()).expect("fetch succeeds");

        let expected_name = format!("{:016x}", crate::hash::fnv1a_64(url.as_bytes()));
        assert_eq!(path, dir.path().join(expected_name));
    }

    #[test]
    fn fetch_to_dir_sends_token_only_on_the_first_request_of_a_redirect() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, seen) = spawn_server(2, |index, path, base_url| match index {
            0 => {
                assert_eq!(path, "/");
                StubResponse {
                    status: 302,
                    headers: vec![("Location", format!("{base_url}/final"))],
                    body: Vec::new(),
                }
            }
            _ => {
                assert_eq!(path, "/final");
                StubResponse {
                    status: 200,
                    headers: vec![("Content-Type", "image/gif".to_string())],
                    body: b"GIF".to_vec(),
                }
            }
        });

        let path = fetch_to_dir(&url, true, Some("tok"), dir.path()).expect("fetch succeeds");

        let seen = seen.lock().expect("lock seen requests");
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0]
                .iter()
                .any(|(name, value)| name == "authorization" && value == "token tok")
        );
        assert!(!seen[1].iter().any(|(name, _)| name == "authorization"));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("gif"));
        assert_eq!(std::fs::read(&path).expect("read fetched file"), b"GIF");
    }

    #[test]
    fn fetch_to_dir_maps_404_to_unauthorized() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 404,
            headers: vec![],
            body: Vec::new(),
        });

        let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");

        assert_eq!(error, MediaError::Unauthorized { status: 404 });
    }

    #[test]
    fn fetch_to_dir_maps_401_and_403_to_unauthorized() {
        for status in [401, 403] {
            let dir = tempfile::tempdir().expect("temp dir");
            let (url, _seen) = spawn_server(1, move |_index, _path, _base_url| StubResponse {
                status,
                headers: vec![],
                body: Vec::new(),
            });

            let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");

            assert_eq!(error, MediaError::Unauthorized { status });
        }
    }

    #[test]
    fn fetch_to_dir_maps_500_to_http_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 500,
            headers: vec![],
            body: Vec::new(),
        });

        let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");

        assert_eq!(error, MediaError::Http { status: 500 });
    }

    fn dir_entries(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect()
    }

    #[test]
    fn fetch_to_dir_leaves_no_temp_file_after_success_or_http_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (ok_url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/png".to_string())],
            body: b"PNGDATA".to_vec(),
        });
        let fetched = fetch_to_dir(&ok_url, false, None, dir.path()).expect("fetch succeeds");
        assert_eq!(dir_entries(dir.path()), vec![fetched.clone()]);

        let (error_url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 500,
            headers: vec![],
            body: Vec::new(),
        });
        fetch_to_dir(&error_url, false, None, dir.path()).expect_err("fetch fails");
        assert_eq!(dir_entries(dir.path()), vec![fetched]);
    }

    #[test]
    fn fetch_to_dir_persists_nothing_when_the_body_is_cut_short() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, seen) = spawn_server(2, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![
                ("Content-Type", "image/png".to_string()),
                ("Content-Length", "100".to_string()),
            ],
            body: vec![0u8; 40],
        });

        let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");
        assert!(matches!(error, MediaError::Io(_)), "{error:?}");
        assert_eq!(dir_entries(dir.path()), Vec::<PathBuf>::new());

        fetch_to_dir(&url, false, None, dir.path()).expect_err("refetch fails");
        assert_eq!(seen.lock().expect("lock seen requests").len(), 2);
    }

    #[test]
    fn fetch_to_dir_refuses_and_does_not_cache_an_html_page() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, seen) = spawn_server(2, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "text/html; charset=utf-8".to_string())],
            body: b"<html>sign in</html>".to_vec(),
        });
        let url = format!("{url}/shot.png");

        let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");
        assert_eq!(
            error,
            MediaError::NotMedia {
                content_type: "text/html; charset=utf-8".to_string()
            }
        );
        assert_eq!(dir_entries(dir.path()), Vec::<PathBuf>::new());

        fetch_to_dir(&url, false, None, dir.path()).expect_err("refetch fails");
        assert_eq!(seen.lock().expect("lock seen requests").len(), 2);
    }

    #[test]
    fn fetch_to_dir_maps_a_login_page_to_unauthorized_when_a_token_was_wanted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(2, |index, _path, base_url| match index {
            0 => StubResponse {
                status: 302,
                headers: vec![("Location", format!("{base_url}/login"))],
                body: Vec::new(),
            },
            _ => StubResponse {
                status: 200,
                headers: vec![("Content-Type", "text/html".to_string())],
                body: b"<html>sign in</html>".to_vec(),
            },
        });

        let error = fetch_to_dir(&url, true, Some("tok"), dir.path()).expect_err("fetch fails");

        assert_eq!(error, MediaError::Unauthorized { status: 200 });
        assert_eq!(dir_entries(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn fetch_to_dir_refuses_an_unknown_type_whose_bytes_are_not_media() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "application/octet-stream".to_string())],
            body: b"BYTES".to_vec(),
        });

        let error = fetch_to_dir(&url, false, None, dir.path()).expect_err("fetch fails");

        assert_eq!(
            error,
            MediaError::NotMedia {
                content_type: "application/octet-stream".to_string()
            }
        );
        assert_eq!(dir_entries(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn sniffs_each_supported_image_and_video_container() {
        for head in [
            PNG_MAGIC.to_vec(),
            b"\xFF\xD8\xFF\xE0".to_vec(),
            b"GIF87a".to_vec(),
            b"GIF89a".to_vec(),
            b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec(),
            b"\x00\x00\x00\x18ftypmp42".to_vec(),
            b"\x00\x00\x00\x14ftypqt  ".to_vec(),
            b"\x1A\x45\xDF\xA3\x9F".to_vec(),
        ] {
            assert!(sniffs_as_media(&head), "{head:?}");
        }
        for head in [
            b"<!DOCTYPE html>".to_vec(),
            b"RIFF\x00\x00\x00\x00WAVEfmt ".to_vec(),
            Vec::new(),
        ] {
            assert!(!sniffs_as_media(&head), "{head:?}");
        }
    }

    /// Runs `open_with` on `fetched`, returning its result and every target
    /// it handed the OS viewer.
    fn route(
        url: &str,
        token_wanted: bool,
        fetched: Result<PathBuf, MediaError>,
    ) -> (Result<Opened, MediaError>, Vec<String>) {
        let mut spawned = Vec::new();
        let result = open_with(url, token_wanted, fetched, |target| {
            spawned.push(target.to_string());
            Ok(())
        });
        (result, spawned)
    }

    #[test]
    fn open_with_sends_an_extensionless_download_to_the_browser_not_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "application/octet-stream".to_string())],
            body: PNG_MAGIC.to_vec(),
        });
        let url = format!("{url}/assets/abc");

        let fetched = fetch_to_dir(&url, false, None, dir.path());
        let (result, spawned) = route(&url, false, fetched);

        assert_eq!(result, Ok(Opened::Browser));
        assert_eq!(spawned, vec![url]);
    }

    #[test]
    fn open_with_hands_a_known_media_file_to_the_viewer() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/png".to_string())],
            body: PNG_MAGIC.to_vec(),
        });

        let fetched = fetch_to_dir(&url, false, None, dir.path());
        let (result, spawned) = route(&url, false, fetched);

        let expected = dir.path().join(format!(
            "{:016x}.png",
            crate::hash::fnv1a_64(url.as_bytes())
        ));
        assert_eq!(result, Ok(Opened::File(expected.clone())));
        assert_eq!(spawned, vec![expected.to_string_lossy().to_string()]);
    }

    #[test]
    fn open_with_falls_back_to_the_browser_when_a_wanted_token_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 404,
            headers: vec![],
            body: Vec::new(),
        });

        let fetched = fetch_to_dir(&url, true, None, dir.path());
        let (result, spawned) = route(&url, true, fetched);

        assert_eq!(result, Ok(Opened::Browser));
        assert_eq!(spawned, vec![url]);
    }

    #[test]
    fn open_with_reports_unauthorized_without_spawning_when_no_token_was_wanted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, _seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 404,
            headers: vec![],
            body: Vec::new(),
        });

        let fetched = fetch_to_dir(&url, false, None, dir.path());
        let (result, spawned) = route(&url, false, fetched);

        assert_eq!(result, Err(MediaError::Unauthorized { status: 404 }));
        assert_eq!(spawned, Vec::<String>::new());
    }

    #[test]
    fn fetch_to_dir_reuses_the_cache_and_serves_one_request() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (url, seen) = spawn_server(1, |_index, _path, _base_url| StubResponse {
            status: 200,
            headers: vec![("Content-Type", "image/png".to_string())],
            body: b"PNGDATA".to_vec(),
        });

        let first = fetch_to_dir(&url, false, None, dir.path()).expect("first fetch succeeds");
        let second = fetch_to_dir(&url, false, None, dir.path()).expect("second fetch succeeds");

        assert_eq!(first, second);
        assert_eq!(seen.lock().expect("lock seen requests").len(), 1);
    }
}
