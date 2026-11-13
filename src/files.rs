//! Listing and previewing files — always through the scope (`ScopeStore::resolve`), never directly.

use crate::scope::{ScopeError, ScopedPath};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Why a folder could not be listed or a file previewed. The path is not repeated.
#[derive(Debug)]
pub enum FilesError {
    /// The scope refused the file, or could not open it.
    Scope(ScopeError),
    /// The system refused or failed to read.
    Io(std::io::Error),
    /// A preview was asked of something that is not a regular file.
    NotAFile,
}

impl FilesError {
    /// The stable code the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            FilesError::Scope(e) => e.code(),
            FilesError::Io(_) => "files_io",
            FilesError::NotAFile => "files_not_a_file",
        }
    }
}

impl fmt::Display for FilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FilesError::Scope(_) => "the scope refused the file",
            FilesError::Io(_) => "the folder or file could not be read",
            FilesError::NotAFile => "not a file",
        })
    }
}

impl std::error::Error for FilesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FilesError::Scope(e) => Some(e),
            FilesError::Io(e) => Some(e),
            FilesError::NotAFile => None,
        }
    }
}

impl From<ScopeError> for FilesError {
    fn from(error: ScopeError) -> Self {
        FilesError::Scope(error)
    }
}

impl From<std::io::Error> for FilesError {
    fn from(error: std::io::Error) -> Self {
        FilesError::Io(error)
    }
}

// @fixed: a mebibyte, the unit of every size limit below
const MIB: u64 = 1024 * 1024;

/// The limits of one call. The app passes what it needs (`None` keeps the kit default); a value above the hard ceiling is
/// lowered to it, and a zero is raised to one, so no call is unbounded or empty.
///
/// | limit | default | ceiling |
/// |---|---|---|
/// | `max_entries` | 2 000 | 20 000 |
/// | `max_text_preview` (bytes) | 64 KiB | 1 MiB |
/// | `max_image_preview` (bytes) | 5 MiB | 25 MiB |
/// | `max_pdf_preview` (bytes) | 12 MiB | 50 MiB |
/// | `max_send` (bytes) | 50 MiB | 200 MiB |
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub max_entries: Option<usize>,
    pub max_text_preview: Option<usize>,
    pub max_image_preview: Option<u64>,
    pub max_pdf_preview: Option<u64>,
    pub max_send: Option<u64>,
}

// @fixed: the kit default and the hard ceiling of each limit. A default is what the apps shipped with before the limits
// became parameters. A ceiling is what the page can hold in memory as one base64 string or one list without freezing: a
// listing of 20 000 rows, a text of 1 MiB, an image of 25 MiB, a PDF of 50 MiB, a sent file of 200 MiB.
pub const DEFAULT_MAX_ENTRIES: usize = 2000;
pub const CEILING_MAX_ENTRIES: usize = 20_000;
pub const DEFAULT_MAX_TEXT_PREVIEW: usize = 64 * 1024;
pub const CEILING_MAX_TEXT_PREVIEW: usize = MIB as usize;
pub const DEFAULT_MAX_IMAGE_PREVIEW: u64 = 5 * MIB;
pub const CEILING_MAX_IMAGE_PREVIEW: u64 = 25 * MIB;
pub const DEFAULT_MAX_PDF_PREVIEW: u64 = 12 * MIB;
pub const CEILING_MAX_PDF_PREVIEW: u64 = 50 * MIB;
pub const DEFAULT_MAX_SEND: u64 = 50 * MIB;
pub const CEILING_MAX_SEND: u64 = 200 * MIB;

impl Limits {
    /// Most entries a listing holds.
    pub fn entries(&self) -> usize {
        self.max_entries
            .unwrap_or(DEFAULT_MAX_ENTRIES)
            .clamp(1, CEILING_MAX_ENTRIES)
    }

    /// Most bytes of a text file a preview reads.
    pub fn text_preview(&self) -> usize {
        self.max_text_preview
            .unwrap_or(DEFAULT_MAX_TEXT_PREVIEW)
            .clamp(1, CEILING_MAX_TEXT_PREVIEW)
    }

    /// Largest image a preview embeds.
    pub fn image_preview(&self) -> u64 {
        self.max_image_preview
            .unwrap_or(DEFAULT_MAX_IMAGE_PREVIEW)
            .clamp(1, CEILING_MAX_IMAGE_PREVIEW)
    }

    /// Largest PDF a preview embeds.
    pub fn pdf_preview(&self) -> u64 {
        self.max_pdf_preview
            .unwrap_or(DEFAULT_MAX_PDF_PREVIEW)
            .clamp(1, CEILING_MAX_PDF_PREVIEW)
    }

    /// The largest file "Send to workspace" will read into memory.
    pub fn send(&self) -> u64 {
        self.max_send
            .unwrap_or(DEFAULT_MAX_SEND)
            .clamp(1, CEILING_MAX_SEND)
    }
}

/// What a file is, for the page. The wire values are the lower-case names, unchanged.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Folder,
    Pdf,
    Image,
    Video,
    Audio,
    Text,
    Document,
    Sheet,
    File,
}

/// The mime of an extension the table does not know.
const UNKNOWN_MIME: &str = "application/octet-stream";

/// The one table: extension, kind and MIME. Add a type here and nowhere else.
const TYPES: &[(&str, Kind, &str)] = &[
    ("pdf", Kind::Pdf, "application/pdf"),
    ("png", Kind::Image, "image/png"),
    ("jpg", Kind::Image, "image/jpeg"),
    ("jpeg", Kind::Image, "image/jpeg"),
    ("gif", Kind::Image, "image/gif"),
    ("webp", Kind::Image, "image/webp"),
    ("heic", Kind::Image, "image/heic"),
    ("svg", Kind::Image, "image/svg+xml"),
    ("mp4", Kind::Video, "video/mp4"),
    ("m4v", Kind::Video, "video/mp4"),
    ("mov", Kind::Video, "video/quicktime"),
    ("webm", Kind::Video, "video/webm"),
    ("mp3", Kind::Audio, "audio/mpeg"),
    ("wav", Kind::Audio, "audio/wav"),
    ("m4a", Kind::Audio, "audio/mp4"),
    ("flac", Kind::Audio, "audio/flac"),
    ("txt", Kind::Text, "text/plain"),
    ("log", Kind::Text, "text/plain"),
    ("md", Kind::Text, "text/markdown"),
    ("csv", Kind::Text, "text/csv"),
    ("tsv", Kind::Text, "text/tab-separated-values"),
    ("json", Kind::Text, "application/json"),
    ("yaml", Kind::Text, "application/yaml"),
    ("yml", Kind::Text, "application/yaml"),
    ("xml", Kind::Text, "application/xml"),
    ("html", Kind::Text, "text/html"),
    ("doc", Kind::Document, "application/msword"),
    (
        "docx",
        Kind::Document,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        "odt",
        Kind::Document,
        "application/vnd.oasis.opendocument.text",
    ),
    ("rtf", Kind::Document, "application/rtf"),
    ("pages", Kind::Document, "application/vnd.apple.pages"),
    ("xls", Kind::Sheet, "application/vnd.ms-excel"),
    (
        "xlsx",
        Kind::Sheet,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        "ods",
        Kind::Sheet,
        "application/vnd.oasis.opendocument.spreadsheet",
    ),
    ("numbers", Kind::Sheet, "application/vnd.apple.numbers"),
];

fn type_of(name: &str) -> Option<&'static (&'static str, Kind, &'static str)> {
    let ext = extension(name);
    TYPES.iter().find(|(e, _, _)| *e == ext)
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub kind: Kind,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub kind: Kind,
    pub text: Option<String>,
    /// base64 of the file (images, PDFs) within the preview limits.
    pub data: Option<String>,
}

pub fn kind_of(name: &str) -> Kind {
    type_of(name).map_or(Kind::File, |(_, kind, _)| *kind)
}

pub fn mime_of(name: &str) -> &'static str {
    type_of(name).map_or(UNKNOWN_MIME, |(_, _, mime)| mime)
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// A temp, partial or hidden name that is never listed nor reported by the folder watch.
pub fn is_noise(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || name.ends_with(".crdownload")
        || name.ends_with(".part")
        || name.ends_with(".tmp")
        || name.ends_with(".download")
}

/// Lists a directory already resolved through the scope. Hidden files are skipped; folders first, then by name; at most
/// `limits.entries()` are kept, after the sort.
///
/// A bare path is refused at compile time:
///
/// ```compile_fail
/// let _ = krizaka_rust::files::list_dir(std::path::Path::new("/etc"), &Default::default());
/// ```
pub fn list_dir(dir: &ScopedPath, limits: &Limits) -> Result<Listing, FilesError> {
    let mut entries: Vec<Entry> = Vec::new();
    let rd = fs::read_dir(dir.as_path())?;
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        if is_noise(&name) {
            continue;
        }
        // `symlink_metadata`: a link is reported as a link and never followed here.
        let Ok(meta) = item.path().symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(Entry {
            kind: if meta.is_dir() {
                Kind::Folder
            } else {
                kind_of(&name)
            },
            path: item.path().display().to_string(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
            name,
        });
    }
    // Sorted before it is cut, so the entries kept are the first ones in order, not the first the system listed. The
    // lower-cased name is built once per entry.
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    let max = limits.entries();
    let truncated = entries.len() > max;
    entries.truncate(max);
    Ok(Listing { entries, truncated })
}

/// Previews a file already resolved through the scope.
///
/// A bare path is refused at compile time: only the scope makes the path a preview reads.
///
/// ```compile_fail
/// let _ = krizaka_rust::files::preview(std::path::Path::new("/etc/hosts"), &Default::default());
/// ```
pub fn preview(file: &ScopedPath, limits: &Limits) -> Result<Preview, FilesError> {
    // One descriptor for the metadata and the content: the path is not looked at again once it is open.
    let opened = file.open()?;
    let meta = opened.metadata()?;
    if !meta.is_file() {
        return Err(FilesError::NotAFile);
    }
    let name = file
        .as_path()
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let kind = kind_of(&name);
    let mut p = Preview {
        mime: mime_of(&name).into(),
        kind,
        size: meta.len(),
        name,
        text: None,
        data: None,
    };
    match kind {
        Kind::Text => {
            let buf = read_at_most(opened, limits.text_preview() as u64)?;
            p.text = Some(String::from_utf8_lossy(&buf).to_string());
        }
        Kind::Image if meta.len() <= limits.image_preview() => {
            p.data = Some(STANDARD.encode(read_at_most(opened, limits.image_preview())?));
        }
        Kind::Pdf if meta.len() <= limits.pdf_preview() => {
            p.data = Some(STANDARD.encode(read_at_most(opened, limits.pdf_preview())?));
        }
        _ => {}
    }
    Ok(p)
}

/// The first `limit` bytes of an opened file (a file that grew since its metadata was read is still capped).
fn read_at_most(file: fs::File, limit: u64) -> Result<Vec<u8>, FilesError> {
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeStore;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A fake home with the folder `docs` granted, and a folder outside every grant.
    struct Granted {
        home: TempDir,
        outside: TempDir,
        scope: ScopeStore,
    }

    impl Granted {
        fn new() -> Result<Granted, Box<dyn std::error::Error>> {
            let home = tempdir()?;
            let outside = tempdir()?;
            fs::create_dir(home.path().join("docs"))?;
            let mut scope = ScopeStore::default();
            let canon = home.path().canonicalize()?;
            scope.grant_with_home(&canon.join("docs"), Some(&canon))?;
            Ok(Granted {
                home,
                outside,
                scope,
            })
        }

        fn docs(&self) -> PathBuf {
            self.home.path().join("docs")
        }

        fn resolve(&self, path: &Path) -> Result<ScopedPath, Box<dyn std::error::Error>> {
            let home = self.home.path().canonicalize()?;
            Ok(self.scope.resolve_with_home(path, Some(&home))?)
        }
    }

    #[test]
    fn lists_folders_first_hides_noise_and_skips_symlinks() -> TestResult {
        let granted = Granted::new()?;
        let d = granted.docs();
        fs::create_dir(d.join("b-folder"))?;
        fs::write(d.join("a.pdf"), "%PDF")?;
        fs::write(d.join(".DS_Store"), "x")?;
        fs::write(d.join("~$draft.docx"), "x")?;
        fs::write(d.join("big.crdownload"), "x")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", d.join("link.txt"))?;
        let l = list_dir(&granted.resolve(&d)?, &Limits::default())?;
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["b-folder", "a.pdf"]);
        assert_eq!(l.entries[0].kind, Kind::Folder);
        assert_eq!(l.entries[1].kind, Kind::Pdf);
        Ok(())
    }

    #[test]
    fn previews_text_is_capped_and_pdf_is_base64() -> TestResult {
        let granted = Granted::new()?;
        let d = granted.docs();
        fs::write(d.join("n.md"), "x".repeat(DEFAULT_MAX_TEXT_PREVIEW + 100))?;
        fs::write(d.join("a.pdf"), "%PDF-1.4")?;
        fs::write(d.join("x.bin"), [0u8, 1, 2])?;
        let t = preview(&granted.resolve(&d.join("n.md"))?, &Limits::default())?;
        assert_eq!(t.size, (DEFAULT_MAX_TEXT_PREVIEW + 100) as u64);
        assert_eq!(
            t.text.map(|text| text.len()),
            Some(DEFAULT_MAX_TEXT_PREVIEW)
        );
        let p = preview(&granted.resolve(&d.join("a.pdf"))?, &Limits::default())?;
        assert_eq!(p.mime, "application/pdf");
        assert_eq!(
            p.data.map(|data| STANDARD.decode(data)).transpose()?,
            Some(b"%PDF-1.4".to_vec())
        );
        let b = preview(&granted.resolve(&d.join("x.bin"))?, &Limits::default())?;
        assert!(b.text.is_none() && b.data.is_none());
        assert!(preview(&granted.resolve(&d)?, &Limits::default()).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_file_swapped_for_a_link_out_after_resolve_is_not_read() -> TestResult {
        let granted = Granted::new()?;
        let target = granted.docs().join("a.txt");
        fs::write(&target, "public")?;
        fs::write(granted.outside.path().join("a.txt"), "secret")?;
        let resolved = granted.resolve(&target)?;
        fs::remove_file(&target)?;
        std::os::unix::fs::symlink(granted.outside.path().join("a.txt"), &target)?;
        let read = preview(&resolved, &Limits::default());
        assert!(read.is_err(), "read through a swapped link: {read:?}");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_swapped_for_a_link_out_after_resolve_is_not_read() -> TestResult {
        let granted = Granted::new()?;
        let sub = granted.docs().join("sub");
        fs::create_dir(&sub)?;
        fs::write(sub.join("a.txt"), "public")?;
        fs::write(granted.outside.path().join("a.txt"), "secret")?;
        let resolved = granted.resolve(&sub.join("a.txt"))?;
        fs::rename(&sub, granted.docs().join("old"))?;
        std::os::unix::fs::symlink(granted.outside.path(), &sub)?;
        let read = preview(&resolved, &Limits::default());
        assert!(read.is_err(), "read through a swapped parent: {read:?}");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_file_swapped_for_a_pipe_after_resolve_is_refused_without_blocking() -> TestResult {
        let granted = Granted::new()?;
        let target = granted.docs().join("a.txt");
        fs::write(&target, "public")?;
        let resolved = granted.resolve(&target)?;
        fs::remove_file(&target)?;
        let made = std::process::Command::new("mkfifo").arg(&target).status()?;
        assert!(made.success());
        assert!(preview(&resolved, &Limits::default()).is_err());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_path_typed_in_another_case_is_previewed() -> TestResult {
        let granted = Granted::new()?;
        fs::write(granted.docs().join("Note.txt"), "hello")?;
        let typed = granted.docs().join("NOTE.TXT");
        if !typed.exists() {
            return Ok(()); // a case-sensitive volume: the other case is another file
        }
        let resolved = granted.resolve(&typed)?;
        assert_eq!(
            preview(&resolved, &Limits::default())?.text.as_deref(),
            Some("hello")
        );
        Ok(())
    }

    #[test]
    fn noise_names() {
        for n in [".hidden", "~$a.docx", "a.part", "a.crdownload", "a.tmp"] {
            assert!(is_noise(n), "{n}");
        }
        assert!(!is_noise("contract.pdf"));
    }

    #[test]
    fn the_listing_is_sorted_before_it_is_cut() -> TestResult {
        let granted = Granted::new()?;
        let d = granted.docs();
        // created in reverse order: the system may list them in any order, the first two by name must be kept
        for name in ["e", "d", "c", "b", "a"] {
            fs::write(d.join(format!("{name}.txt")), "x")?;
        }
        fs::create_dir(d.join("zz-folder"))?;
        let limits = Limits {
            max_entries: Some(3),
            ..Limits::default()
        };
        let l = list_dir(&granted.resolve(&d)?, &limits)?;
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zz-folder", "a.txt", "b.txt"]);
        assert!(l.truncated);
        let all = list_dir(&granted.resolve(&d)?, &Limits::default())?;
        assert!(!all.truncated && all.entries.len() == 6);
        Ok(())
    }

    #[test]
    fn a_limit_defaults_and_is_held_by_its_ceiling() {
        let none = Limits::default();
        assert_eq!(none.entries(), DEFAULT_MAX_ENTRIES);
        assert_eq!(none.send(), DEFAULT_MAX_SEND);
        let huge = Limits {
            max_entries: Some(usize::MAX),
            max_text_preview: Some(usize::MAX),
            max_image_preview: Some(u64::MAX),
            max_pdf_preview: Some(u64::MAX),
            max_send: Some(u64::MAX),
        };
        assert_eq!(huge.entries(), CEILING_MAX_ENTRIES);
        assert_eq!(huge.text_preview(), CEILING_MAX_TEXT_PREVIEW);
        assert_eq!(huge.image_preview(), CEILING_MAX_IMAGE_PREVIEW);
        assert_eq!(huge.pdf_preview(), CEILING_MAX_PDF_PREVIEW);
        assert_eq!(huge.send(), CEILING_MAX_SEND);
        let zero = Limits {
            max_entries: Some(0),
            max_send: Some(0),
            ..Limits::default()
        };
        assert_eq!((zero.entries(), zero.send()), (1, 1));
    }

    #[test]
    fn limits_come_from_the_page_and_refuse_unknown_names() -> Result<(), serde_json::Error> {
        let l: Limits = serde_json::from_str(r#"{"maxEntries": 10, "maxSend": 99}"#)?;
        assert_eq!((l.entries(), l.send()), (10, 99));
        assert!(serde_json::from_str::<Limits>(r#"{"maxEntry": 10}"#).is_err());
        Ok(())
    }

    #[test]
    fn a_preview_follows_the_limits_of_its_call() -> TestResult {
        let granted = Granted::new()?;
        let d = granted.docs();
        fs::write(d.join("n.txt"), "abcdefgh")?;
        fs::write(d.join("p.png"), [7u8; 100])?;
        let tight = Limits {
            max_text_preview: Some(3),
            max_image_preview: Some(50),
            ..Limits::default()
        };
        let t = preview(&granted.resolve(&d.join("n.txt"))?, &tight)?;
        assert_eq!(t.text.as_deref(), Some("abc"));
        let image = granted.resolve(&d.join("p.png"))?;
        assert!(preview(&image, &tight)?.data.is_none());
        assert!(preview(&image, &Limits::default())?.data.is_some());
        Ok(())
    }

    #[test]
    fn one_table_gives_kind_and_mime_and_the_wire_values_hold() -> Result<(), serde_json::Error> {
        assert_eq!(kind_of("IMG.HEIC"), Kind::Image);
        assert_eq!(mime_of("IMG.HEIC"), "image/heic");
        assert_eq!(kind_of("a.webm"), Kind::Video);
        assert_eq!(mime_of("a.webm"), "video/webm");
        assert_eq!(kind_of("a.unknown"), Kind::File);
        assert_eq!(mime_of("a.unknown"), "application/octet-stream");
        assert_eq!(kind_of("noext"), Kind::File);
        for (ext, kind, mime) in TYPES {
            let name = format!("f.{ext}");
            assert_eq!((kind_of(&name), mime_of(&name)), (*kind, *mime), "{ext}");
            assert_ne!(*kind, Kind::File);
        }
        let wire: Vec<String> = [
            Kind::Folder,
            Kind::Pdf,
            Kind::Image,
            Kind::Video,
            Kind::Audio,
            Kind::Text,
            Kind::Document,
            Kind::Sheet,
            Kind::File,
        ]
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<_, _>>()?;
        assert_eq!(
            wire,
            [
                "\"folder\"",
                "\"pdf\"",
                "\"image\"",
                "\"video\"",
                "\"audio\"",
                "\"text\"",
                "\"document\"",
                "\"sheet\"",
                "\"file\""
            ]
        );
        Ok(())
    }
}
