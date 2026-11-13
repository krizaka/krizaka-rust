//! Listing and previewing files — always through the scope (`ScopeStore::resolve`), never directly.

use crate::scope::{ScopeError, ScopedPath};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
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

pub const MAX_ENTRIES: usize = 2000;
pub const MAX_TEXT_PREVIEW: usize = 64 * 1024;
pub const MAX_IMAGE_PREVIEW: u64 = 5 * 1024 * 1024;
pub const MAX_PDF_PREVIEW: u64 = 12 * 1024 * 1024;
/// The largest file "Send to workspace" will read into memory.
pub const MAX_SEND: u64 = 50 * 1024 * 1024;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub kind: String,
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
    pub kind: String,
    pub text: Option<String>,
    /// base64 of the file (images, PDFs) within the preview limits.
    pub data: Option<String>,
}

pub fn kind_of(name: &str) -> &'static str {
    match extension(name).as_str() {
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" => "image",
        "mp4" | "mov" | "webm" | "m4v" => "video",
        "mp3" | "wav" | "m4a" | "flac" => "audio",
        "txt" | "md" | "csv" | "json" | "yaml" | "yml" | "xml" | "html" | "log" | "tsv" => "text",
        "doc" | "docx" | "odt" | "rtf" | "pages" => "document",
        "xls" | "xlsx" | "ods" | "numbers" => "sheet",
        _ => "file",
    }
}

pub fn mime_of(name: &str) -> &'static str {
    match extension(name).as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
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

/// Lists a directory already resolved through the scope. Hidden files are skipped; folders first.
///
/// A bare path is refused at compile time:
///
/// ```compile_fail
/// let _ = krizaka_rust::files::list_dir(std::path::Path::new("/etc"));
/// ```
pub fn list_dir(dir: &ScopedPath) -> Result<Listing, FilesError> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut truncated = false;
    let rd = fs::read_dir(dir.as_path())?;
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        if is_noise(&name) {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
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
                "folder".into()
            } else {
                kind_of(&name).into()
            },
            path: item.path().display().to_string(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
            name,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing { entries, truncated })
}

/// Previews a file already resolved through the scope.
///
/// A bare path is refused at compile time: only the scope makes the path a preview reads.
///
/// ```compile_fail
/// let _ = krizaka_rust::files::preview(std::path::Path::new("/etc/hosts"));
/// ```
pub fn preview(file: &ScopedPath) -> Result<Preview, FilesError> {
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
        kind: kind.into(),
        size: meta.len(),
        name,
        text: None,
        data: None,
    };
    match kind {
        "text" => {
            let buf = read_at_most(opened, MAX_TEXT_PREVIEW as u64)?;
            p.text = Some(String::from_utf8_lossy(&buf).to_string());
        }
        "image" if meta.len() <= MAX_IMAGE_PREVIEW => {
            p.data = Some(STANDARD.encode(read_at_most(opened, MAX_IMAGE_PREVIEW)?));
        }
        "pdf" if meta.len() <= MAX_PDF_PREVIEW => {
            p.data = Some(STANDARD.encode(read_at_most(opened, MAX_PDF_PREVIEW)?));
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
        let l = list_dir(&granted.resolve(&d)?)?;
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["b-folder", "a.pdf"]);
        assert_eq!(l.entries[1].kind, "pdf");
        Ok(())
    }

    #[test]
    fn previews_text_is_capped_and_pdf_is_base64() -> TestResult {
        let granted = Granted::new()?;
        let d = granted.docs();
        fs::write(d.join("n.md"), "x".repeat(MAX_TEXT_PREVIEW + 100))?;
        fs::write(d.join("a.pdf"), "%PDF-1.4")?;
        fs::write(d.join("x.bin"), [0u8, 1, 2])?;
        let t = preview(&granted.resolve(&d.join("n.md"))?)?;
        assert_eq!(t.size, (MAX_TEXT_PREVIEW + 100) as u64);
        assert_eq!(t.text.map(|text| text.len()), Some(MAX_TEXT_PREVIEW));
        let p = preview(&granted.resolve(&d.join("a.pdf"))?)?;
        assert_eq!(p.mime, "application/pdf");
        assert_eq!(
            p.data.map(|data| STANDARD.decode(data)).transpose()?,
            Some(b"%PDF-1.4".to_vec())
        );
        let b = preview(&granted.resolve(&d.join("x.bin"))?)?;
        assert!(b.text.is_none() && b.data.is_none());
        assert!(preview(&granted.resolve(&d)?).is_err());
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
        let read = preview(&resolved);
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
        let read = preview(&resolved);
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
        assert!(preview(&resolved).is_err());
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
        assert_eq!(preview(&resolved)?.text.as_deref(), Some("hello"));
        Ok(())
    }

    #[test]
    fn noise_names() {
        for n in [".hidden", "~$a.docx", "a.part", "a.crdownload", "a.tmp"] {
            assert!(is_noise(n), "{n}");
        }
        assert!(!is_noise("contract.pdf"));
    }
}
