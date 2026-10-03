//! Images and videos referenced from a pull request description.

pub mod detect;
pub mod graphics;
pub mod open;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    /// A bare user-attachments URL: GitHub decides image or video from the
    /// served Content-Type, so the kind is unknown until the file is fetched.
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub url: String,
    /// Alt text for images, file name otherwise. Whitespace is collapsed to
    /// single spaces. Never empty: falls back to the URL's last path segment,
    /// then the whole URL.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaLine {
    /// 0-based source line (the body split on `\n`) where the media starts.
    pub line: usize,
    /// Byte in the body where the media's markup starts.
    pub start: usize,
    /// Source lines the media's markup covers, so a multi-line `<img>` or
    /// `<video>` spans several.
    pub lines: std::ops::Range<usize>,
    /// True when the covered lines hold nothing but media plus whitespace,
    /// HTML tags, blockquote/list markers and empty link wrappers, so the
    /// renderer replaces those lines with placeholders.
    pub media_only: bool,
    pub media: MediaRef,
}
