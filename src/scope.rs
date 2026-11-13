//! The folder scope — the heart of the app's least-privilege file access.
//!
//! The app can read a path **only** when it lies inside a folder the user granted with the native
//! picker. Grants are persisted (a JSON file in the app's data directory), listed in the UI and
//! revocable. Every path is canonicalised before it is compared, so `..` segments and symlinks that
//! leave a granted folder are refused; broad or sensitive locations can never be granted.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum ScopeError {
    /// The folder is too broad or too sensitive to be granted (home, `/`, `~/.ssh`, `/System`…).
    Forbidden(String),
    /// The path does not exist or is not a directory / file as expected.
    NotFound(String),
    /// The path is outside every granted folder.
    OutsideScope(String),
    Io(String),
}

impl ScopeError {
    /// The stable code the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            ScopeError::Forbidden(_) => "scope_forbidden",
            ScopeError::NotFound(_) => "scope_not_found",
            ScopeError::OutsideScope(_) => "scope_outside",
            ScopeError::Io(_) => "scope_io",
        }
    }
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeError::Forbidden(p) => write!(f, "forbidden: {p}"),
            ScopeError::NotFound(p) => write!(f, "not found: {p}"),
            ScopeError::OutsideScope(p) => write!(f, "outside the granted folders: {p}"),
            ScopeError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ScopeError {}

/// A path the scope let through: canonical, inside a granted folder (or a file dropped this session), not sensitive.
///
/// Only [`ScopeStore::resolve`] and [`crate::access::Access::resolve`] make one, so a function that takes a `ScopedPath`
/// can never be handed a path the scope did not see. A read goes through [`ScopedPath::open`], which confirms that the
/// opened file is still this one.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use krizaka_rust::scope::ScopeStore;
/// let dir = tempfile::tempdir()?;
/// let home = dir.path().canonicalize()?;
/// std::fs::create_dir(home.join("docs"))?;
/// std::fs::write(home.join("docs/a.txt"), "hello")?;
/// let mut scope = ScopeStore::default();
/// scope.grant_with_home(&home.join("docs"), Some(&home))?;
/// let file = scope.resolve_with_home(&home.join("docs/a.txt"), Some(&home))?;
/// assert_eq!(krizaka_rust::files::preview(&file)?.text.as_deref(), Some("hello"));
/// # Ok(())
/// # }
/// ```
///
/// It cannot be made from a bare path:
///
/// ```compile_fail
/// let _ = krizaka_rust::scope::ScopedPath(std::path::PathBuf::from("/etc/hosts"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopedPath(PathBuf);

impl ScopedPath {
    /// `canon` has passed the scope's checks: only the scope's gates call this.
    pub(crate) fn new(canon: PathBuf) -> Self {
        ScopedPath(canon)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    /// Opens the file once, read only, and confirms the descriptor is the file the scope let through.
    ///
    /// The path may have changed since it was resolved: the file, or one of its parent folders, swapped for a link out
    /// of the grant. The last component is opened without following a link, and the system is asked where the opened
    /// file really is; anything but the resolved path is refused. Read the metadata and the content from the returned
    /// file, never from the path again.
    pub fn open(&self) -> Result<fs::File, ScopeError> {
        let file = open_no_follow(&self.0).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ScopeError::NotFound(self.0.display().to_string()),
            _ => ScopeError::Io(e.to_string()),
        })?;
        let opened = opened_path(&file).map_err(|e| ScopeError::Io(e.to_string()))?;
        if opened != self.0 {
            return Err(ScopeError::OutsideScope(self.0.display().to_string()));
        }
        Ok(file)
    }
}

impl AsRef<Path> for ScopedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Read only, the last component never followed if it is a link, never blocking on a pipe swapped in.
#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<fs::File> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    Ok(fs::File::from(rustix::fs::open(
        path,
        flags,
        Mode::empty(),
    )?))
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

/// Where the system says the opened file is (`F_GETPATH`), whatever path was used to open it.
#[cfg(target_os = "macos")]
fn opened_path(file: &fs::File) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let path = rustix::fs::getpath(file)?;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(
        path.into_bytes(),
    )))
}

#[cfg(target_os = "linux")]
fn opened_path(file: &fs::File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Elsewhere the opened file cannot be located, so nothing is read.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn opened_path(_file: &fs::File) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the opened file cannot be located on this system",
    ))
}

/// Directories (relative to the user's home) that can never be granted, nor read through a grant.
const SENSITIVE_HOME_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".config",
    ".npmrc",
    "Library",
    ".claude",
    ".cargo",
    ".krizaka-worktrees",
];

/// System locations that can never be granted (they are broad, or hold other programs' data).
const FORBIDDEN_ABSOLUTE: &[&str] = &[
    "/",
    "/System",
    "/Library",
    "/Users",
    "/Volumes",
    "/private",
    "/etc",
    "/usr",
    "/bin",
    "/sbin",
    "/var",
    "/opt",
    "/Applications",
    "/dev",
    "/cores",
    "/tmp",
];

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeStore {
    roots: Vec<PathBuf>,
}

impl ScopeStore {
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn load(file: &Path) -> Self {
        Self::load_with_home(file, home_dir().as_deref())
    }

    pub fn load_with_home(file: &Path, home: Option<&Path>) -> Self {
        let Ok(raw) = fs::read_to_string(file) else {
            return Self::default();
        };
        let mut store: ScopeStore = serde_json::from_str(&raw).unwrap_or_default();
        // A folder that has vanished, or that became forbidden, silently drops out of the scope.
        store
            .roots
            .retain(|r| r.is_dir() && check_grantable(r, home).is_ok());
        store
    }

    /// Written atomically: a crash mid-write never loses the grants.
    pub fn save(&self, file: &Path) -> Result<(), ScopeError> {
        crate::settings::save(file, self).map_err(|e| ScopeError::Io(e.to_string()))
    }

    /// Grants a folder (idempotent). Returns the canonical path stored.
    pub fn grant(&mut self, folder: &Path) -> Result<PathBuf, ScopeError> {
        self.grant_with_home(folder, home_dir().as_deref())
    }

    pub fn grant_with_home(
        &mut self,
        folder: &Path,
        home: Option<&Path>,
    ) -> Result<PathBuf, ScopeError> {
        let canon = folder
            .canonicalize()
            .map_err(|_| ScopeError::NotFound(folder.display().to_string()))?;
        if !canon.is_dir() {
            return Err(ScopeError::NotFound(canon.display().to_string()));
        }
        check_grantable(&canon, home)?;
        // A child of an already granted folder adds nothing; a parent replaces its children.
        if self.roots.iter().any(|r| canon.starts_with(r)) {
            return Ok(canon);
        }
        self.roots.retain(|r| !r.starts_with(&canon));
        self.roots.push(canon.clone());
        self.roots.sort();
        Ok(canon)
    }

    pub fn revoke(&mut self, folder: &Path) -> bool {
        let before = self.roots.len();
        self.roots.retain(|r| r != folder);
        self.roots.len() != before
    }

    pub fn revoke_all(&mut self) {
        self.roots.clear();
    }

    /// Resolves `path` to its canonical form **iff** it lies inside a granted folder and is not a
    /// sensitive location. This is the one gate every read goes through.
    pub fn resolve(&self, path: &Path) -> Result<ScopedPath, ScopeError> {
        self.resolve_with_home(path, home_dir().as_deref())
    }

    pub fn resolve_with_home(
        &self,
        path: &Path,
        home: Option<&Path>,
    ) -> Result<ScopedPath, ScopeError> {
        let canon = path
            .canonicalize()
            .map_err(|_| ScopeError::NotFound(path.display().to_string()))?;
        if !self.roots.iter().any(|r| canon.starts_with(r)) {
            return Err(ScopeError::OutsideScope(path.display().to_string()));
        }
        if is_sensitive(&canon, home) {
            return Err(ScopeError::Forbidden(canon.display().to_string()));
        }
        Ok(ScopedPath::new(canon))
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
}

fn is_sensitive(canon: &Path, home: Option<&Path>) -> bool {
    if let Some(home) = home {
        for d in SENSITIVE_HOME_DIRS {
            if canon.starts_with(home.join(d)) {
                return true;
            }
        }
    }
    false
}

fn check_grantable(canon: &Path, home: Option<&Path>) -> Result<(), ScopeError> {
    let shown = canon.display().to_string();
    for f in FORBIDDEN_ABSOLUTE {
        // The folder itself, never what is under it (`/Users/me/Documents` is fine).
        if canon == Path::new(f) {
            return Err(ScopeError::Forbidden(shown));
        }
    }
    // macOS volumes' roots and `/Users/<name>` — a whole home is too broad.
    if let Some(home) = home {
        if canon == home {
            return Err(ScopeError::Forbidden(shown));
        }
        if is_sensitive(canon, Some(home)) {
            return Err(ScopeError::Forbidden(shown));
        }
        // Also refuse a folder that *contains* the home (e.g. `/Users`).
        if home.starts_with(canon) {
            return Err(ScopeError::Forbidden(shown));
        }
    }
    for f in [
        "/System",
        "/Library",
        "/usr",
        "/bin",
        "/sbin",
        "/etc",
        "/private/etc",
    ] {
        if canon.starts_with(f) {
            return Err(ScopeError::Forbidden(shown));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fake_home() -> tempfile::TempDir {
        let h = tempdir().unwrap();
        fs::create_dir_all(h.path().join("Documents/clients")).unwrap();
        fs::create_dir_all(h.path().join(".ssh")).unwrap();
        fs::write(h.path().join(".ssh/id_ed25519"), "secret").unwrap();
        fs::write(h.path().join("Documents/clients/a.pdf"), "%PDF").unwrap();
        h
    }

    #[test]
    fn grants_a_normal_folder_and_resolves_inside_it() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let mut s = ScopeStore::default();
        let root = s
            .grant_with_home(&home.join("Documents"), Some(&home))
            .unwrap();
        assert!(s
            .resolve_with_home(&root.join("clients/a.pdf"), Some(&home))
            .is_ok());
    }

    #[test]
    fn refuses_the_home_itself_root_and_sensitive_folders() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let mut s = ScopeStore::default();
        assert!(matches!(
            s.grant_with_home(&home, Some(&home)),
            Err(ScopeError::Forbidden(_))
        ));
        assert!(matches!(
            s.grant_with_home(Path::new("/"), Some(&home)),
            Err(ScopeError::Forbidden(_))
        ));
        assert!(matches!(
            s.grant_with_home(&home.join(".ssh"), Some(&home)),
            Err(ScopeError::Forbidden(_))
        ));
        assert!(matches!(
            s.grant_with_home(Path::new("/System"), Some(&home)),
            Err(ScopeError::Forbidden(_))
        ));
        assert!(s.roots().is_empty());
    }

    #[test]
    fn refuses_a_parent_of_home() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let parent = home.parent().unwrap().to_path_buf();
        let mut s = ScopeStore::default();
        assert!(matches!(
            s.grant_with_home(&parent, Some(&home)),
            Err(ScopeError::Forbidden(_))
        ));
    }

    #[test]
    fn refuses_paths_outside_the_scope_and_dotdot_escapes() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let mut s = ScopeStore::default();
        let root = s
            .grant_with_home(&home.join("Documents"), Some(&home))
            .unwrap();
        let escape = root.join("clients/../../.ssh/id_ed25519");
        assert!(matches!(
            s.resolve_with_home(&escape, Some(&home)),
            Err(ScopeError::OutsideScope(_))
        ));
        assert!(matches!(
            s.resolve_with_home(Path::new("/etc/hosts"), Some(&home)),
            Err(ScopeError::OutsideScope(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_that_leaves_the_granted_folder() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let mut s = ScopeStore::default();
        let root = s
            .grant_with_home(&home.join("Documents"), Some(&home))
            .unwrap();
        std::os::unix::fs::symlink(home.join(".ssh/id_ed25519"), root.join("innocent.txt"))
            .unwrap();
        assert!(matches!(
            s.resolve_with_home(&root.join("innocent.txt"), Some(&home)),
            Err(ScopeError::OutsideScope(_))
        ));
    }

    #[test]
    fn revoking_removes_access_and_a_parent_grant_absorbs_children() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let mut s = ScopeStore::default();
        let child = s
            .grant_with_home(&home.join("Documents/clients"), Some(&home))
            .unwrap();
        let parent = s
            .grant_with_home(&home.join("Documents"), Some(&home))
            .unwrap();
        assert_eq!(s.roots(), std::slice::from_ref(&parent));
        assert!(s
            .resolve_with_home(&child.join("a.pdf"), Some(&home))
            .is_ok());
        assert!(s.revoke(&parent));
        assert!(matches!(
            s.resolve_with_home(&child.join("a.pdf"), Some(&home)),
            Err(ScopeError::OutsideScope(_))
        ));
        assert!(!s.revoke(&parent));
    }

    #[test]
    fn grants_persist_and_vanished_folders_drop_out() {
        let h = fake_home();
        let home = h.path().canonicalize().unwrap();
        let file = h.path().join("data/scope.json");
        let mut s = ScopeStore::default();
        s.grant_with_home(&home.join("Documents"), Some(&home))
            .unwrap();
        s.save(&file).unwrap();
        let loaded = ScopeStore::load_with_home(&file, Some(&home));
        assert_eq!(loaded.roots().len(), 1);
        fs::remove_dir_all(home.join("Documents")).unwrap();
        assert!(ScopeStore::load_with_home(&file, Some(&home))
            .roots()
            .is_empty());
    }
}
