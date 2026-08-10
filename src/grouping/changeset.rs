//! The changeset under grouping, and the only signals it carries: path shape,
//! filename tokens, change kind, rename pairs.
//!
//! Deliberately no diff bodies. The scored fixtures carry paths and statuses
//! only (`docs/GROUPING_PASSES.md`), so an engine that read hunks would be
//! graded on evidence the fixtures cannot supply.

use crate::model::{DiffFile, FileStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl From<FileStatus> for ChangeKind {
    fn from(status: FileStatus) -> Self {
        match status {
            FileStatus::Added => ChangeKind::Added,
            FileStatus::Modified => ChangeKind::Modified,
            FileStatus::Deleted => ChangeKind::Deleted,
            // A copy is a rename that left its source behind; both reach the
            // engine as one entry keyed by the new path.
            FileStatus::Renamed | FileStatus::Copied => ChangeKind::Renamed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    /// Old path of a rename, when the diff reported one.
    pub rename_from: Option<String>,
}

/// Extensions stripped when reducing a filename to its concern-bearing stem. An
/// extension names the file's language or role, never its concern, so any
/// extension left off this list survives tokenisation and clusters files by
/// layer — the failure GROUPING.md rule 1 forbids. The list is therefore
/// audited per ecosystem rather than grown one entry at a time; it covers the
/// languages, project/build files and markup formats a changeset is likely to
/// carry, not only the web stack the first fixture happened to use.
#[rustfmt::skip]
pub const EXTENSIONS: &[&str] = &[
    // web and scripting
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts",
    "vue", "svelte", "astro", "css", "scss", "sass", "less", "html", "htm",
    // .NET, including project and build files
    "cs", "vb", "fs", "csproj", "vbproj", "fsproj", "sln", "props", "targets",
    "razor", "cshtml", "resx", "config", "nuspec",
    // JVM
    "java", "kt", "kts", "scala", "groovy", "gradle",
    // other languages
    "rs", "go", "py", "rb", "php", "pl", "lua", "swift", "dart", "ex", "exs", "erl",
    "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "m", "mm",
    "sh", "bash", "zsh", "ps1", "psm1", "sql",
    // data, markup and interface definitions
    "md", "mdx", "rst", "txt", "json", "jsonc", "yml", "yaml", "toml", "ini", "cfg",
    "xml", "csv", "proto", "graphql", "gql", "lock", "snap", "svg", "tf", "tfvars",
];

/// Filename segments that mark a file as a test rather than naming a concern.
const TEST_MARKERS: &[&str] = &["test", "spec", "tests", "specs"];

/// Lockfiles, vendored trees and generated output: skimmed, not read.
/// Lives here rather than in `passes` because both the mechanical pass and the
/// order metric's "mechanical sorts last" constraint ask the same question, and
/// a grouping that files a lockfile under `mechanical` while the scorer does not
/// recognise it as mechanical would grade itself against a different rule 8.
const MECHANICAL_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "go.sum",
    "poetry.lock",
];
const MECHANICAL_DIRS: &[&str] = &[
    "vendor/",
    "node_modules/",
    "dist/",
    "build/",
    "generated/",
    "__generated__/",
];
const MECHANICAL_SUFFIXES: &[&str] = &[".generated.ts", ".gen.go", "_pb2.py", ".snap"];

/// Filename or directory segments marking a test as broader than a unit test.
/// Deliberately short: every entry here reorders real files, and a loose match
/// (`int`, `it`) would catch concern words.
const BROAD_TEST_MARKERS: &[&str] = &["integration", "e2e", "endtoend", "acceptance", "smoke"];

impl ChangedFile {
    /// A diff entry as the engine sees it. `display_path` is the same key
    /// `ReviewSession.files` uses, so an assignment and a review record name
    /// the same file with no translation.
    pub fn from_diff_file(file: &DiffFile) -> Self {
        Self {
            path: file.display_path().to_string_lossy().to_string(),
            kind: ChangeKind::from(file.status),
            // Git reports a rename as one entry keyed by the new path
            // (`GROUPING.md` rule 11), so the old path is provenance, never a
            // second file.
            rename_from: match file.status {
                FileStatus::Renamed | FileStatus::Copied => file
                    .old_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
                _ => None,
            },
        }
    }

    pub fn dir(&self) -> &str {
        self.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
    }

    pub fn file_name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map(|(_, f)| f)
            .unwrap_or(&self.path)
    }

    pub fn top_level_dir(&self) -> &str {
        self.path
            .split_once('/')
            .map(|(d, _)| d)
            .unwrap_or(&self.path)
    }

    pub fn is_test(&self) -> bool {
        let name = self.file_name();
        let has_marker = name
            .split('.')
            .any(|segment| TEST_MARKERS.contains(&segment.to_ascii_lowercase().as_str()));
        has_marker || self.path.contains("/__tests__/") || self.path.starts_with("tests/")
    }

    /// Lockfile, vendored tree or generated output (GROUPING.md rule 8).
    pub fn is_mechanical(&self) -> bool {
        let name = self.file_name();
        MECHANICAL_NAMES.contains(&name)
            || MECHANICAL_DIRS.iter().any(|dir| self.path.contains(dir))
            || MECHANICAL_SUFFIXES
                .iter()
                .any(|suffix| name.ends_with(suffix))
    }

    /// A test that exercises more than one unit: integration, end-to-end,
    /// acceptance, smoke. False for anything [`Self::is_test`] rejects, so a
    /// production file named `integration-client.ts` is not caught.
    pub fn is_broad_test(&self) -> bool {
        if !self.is_test() {
            return false;
        }
        let lowered = self.path.to_ascii_lowercase();
        let in_path = BROAD_TEST_MARKERS
            .iter()
            .any(|marker| lowered.contains(&format!("/{marker}/")));
        let in_name = split_tokens(self.file_name())
            .iter()
            .any(|token| BROAD_TEST_MARKERS.contains(&token.as_str()));
        in_path || in_name
    }

    /// The filename with extensions and test markers removed, dot-separated
    /// segments rejoined with `-`, so `a.b.test.ts` and `a-b.ts` reduce alike.
    pub fn stem(&self) -> String {
        let mut segments: Vec<&str> = self.file_name().split('.').collect();
        while segments.len() > 1 {
            let last = segments.last().unwrap().to_ascii_lowercase();
            if EXTENSIONS.contains(&last.as_str()) || TEST_MARKERS.contains(&last.as_str()) {
                segments.pop();
            } else {
                break;
            }
        }
        segments.join("-")
    }

    /// Concern tokens from the filename: camelCase and separator split,
    /// lowercased, crudely singularised so `issues` and `issue` cluster.
    pub fn name_tokens(&self) -> Vec<String> {
        split_tokens(&self.stem())
    }

    /// Tokens of the immediately containing directory, a weaker concern signal
    /// than the filename but the only one some files carry.
    pub fn dir_tokens(&self) -> Vec<String> {
        let leaf = self.dir().rsplit('/').next().unwrap_or("");
        split_tokens(leaf)
    }
}

fn split_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for chunk in text.split(['-', '_', '.', '/', ' ', '[', ']']) {
        for token in split_camel_case(chunk) {
            let token = singularise(&token.to_ascii_lowercase());
            if token.len() > 1 && !TEST_MARKERS.contains(&token.as_str()) {
                tokens.push(token);
            }
        }
    }
    tokens
}

fn split_camel_case(chunk: &str) -> Vec<String> {
    let chars: Vec<char> = chunk.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let boundary = (chars[i].is_uppercase() && chars[i - 1].is_lowercase())
            || (chars[i].is_lowercase()
                && chars[i - 1].is_uppercase()
                && i >= 2
                && chars[i - 2].is_uppercase());
        if boundary {
            let end = if chars[i].is_lowercase() { i - 1 } else { i };
            if end > start {
                parts.push(chars[start..end].iter().collect());
            }
            start = end;
        }
    }
    if start < chars.len() {
        parts.push(chars[start..].iter().collect());
    }
    parts
}

fn singularise(token: &str) -> String {
    if token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") && !token.ends_with("us") {
        token[..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Changeset {
    pub files: Vec<ChangedFile>,
}

impl Changeset {
    /// The real files of a loaded diff, in `diff_files` order.
    ///
    /// The commit-message pseudo-file is skipped: it sits **outside** the
    /// partition entirely (`docs/TOTAL_COVERAGE.md` Decision 1), so the engine
    /// is never asked to reason about a pathless entry.
    pub fn from_diff_files(diff_files: &[DiffFile]) -> Self {
        let files = diff_files
            .iter()
            .filter(|file| !file.is_commit_message)
            .map(ChangedFile::from_diff_file)
            .collect();
        Self { files }
    }

    /// Parses `<status>\t<path>` lines, with `R<score>\t<old>\t<new>` for renames.
    pub fn parse(text: &str) -> Self {
        let files = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let mut fields = line.split('\t');
                let status = fields.next().unwrap_or("M");
                let first = fields.next().unwrap_or_default().to_string();
                let second = fields.next().map(str::to_string);
                match status.chars().next().unwrap_or('M') {
                    'A' => ChangedFile {
                        path: first,
                        kind: ChangeKind::Added,
                        rename_from: None,
                    },
                    'D' => ChangedFile {
                        path: first,
                        kind: ChangeKind::Deleted,
                        rename_from: None,
                    },
                    'R' | 'C' => match second {
                        Some(new_path) => ChangedFile {
                            path: new_path,
                            kind: ChangeKind::Renamed,
                            rename_from: Some(first),
                        },
                        None => ChangedFile {
                            path: first,
                            kind: ChangeKind::Renamed,
                            rename_from: None,
                        },
                    },
                    _ => ChangedFile {
                        path: first,
                        kind: ChangeKind::Modified,
                        rename_from: None,
                    },
                }
            })
            .collect();
        Self { files }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when the diff held nothing groupable — every entry filtered out, or
    /// an empty diff. The engine's callers branch on it rather than grouping
    /// nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
