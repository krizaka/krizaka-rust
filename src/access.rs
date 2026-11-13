//! Who may read what: the folders the person granted, and the files they dropped on the window this session.

use crate::scope::{ScopeError, ScopeStore, ScopedPath};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Access {
    scope: ScopeStore,
    /// Canonical paths the operating system reported as dropped on the window. A drop is one file, for this session only.
    dropped: HashSet<PathBuf>,
}

impl Access {
    pub fn new(scope: ScopeStore) -> Self {
        Access {
            scope,
            dropped: HashSet::new(),
        }
    }

    pub fn scope(&self) -> &ScopeStore {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut ScopeStore {
        &mut self.scope
    }

    /// Remembers a file the OS says was dropped (a folder, a missing path or a link out of nowhere is ignored).
    pub fn allow_dropped(&mut self, path: &Path) -> Option<PathBuf> {
        let canon = path.canonicalize().ok()?;
        if !canon.is_file() {
            return None;
        }
        self.dropped.insert(canon.clone());
        Some(canon)
    }

    /// The one gate: a granted folder's file, or a file dropped this session. Anything else is refused.
    pub fn resolve(&self, path: &Path) -> Result<ScopedPath, ScopeError> {
        match self.scope.resolve(path) {
            Ok(p) => Ok(p),
            Err(error) => match path.canonicalize() {
                Ok(canon) if self.dropped.contains(&canon) => Ok(ScopedPath::new(canon)),
                _ => Err(error),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn a_dropped_file_is_readable_and_its_neighbours_are_not() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("bail.pdf"), "%PDF").unwrap();
        fs::write(d.path().join("secret.pdf"), "%PDF").unwrap();
        let mut access = Access::default();
        assert!(access.resolve(&d.path().join("bail.pdf")).is_err());
        access.allow_dropped(&d.path().join("bail.pdf")).unwrap();
        assert!(access.resolve(&d.path().join("bail.pdf")).is_ok());
        assert!(access.resolve(&d.path().join("secret.pdf")).is_err());
    }

    #[test]
    fn a_dropped_folder_or_a_path_that_does_not_exist_grants_nothing() {
        let d = tempdir().unwrap();
        let mut access = Access::default();
        assert!(access.allow_dropped(d.path()).is_none());
        assert!(access
            .allow_dropped(&d.path().join("missing.pdf"))
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_that_was_not_dropped_is_not_a_way_in() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("secret.pdf"), "%PDF").unwrap();
        fs::write(d.path().join("bail.pdf"), "%PDF").unwrap();
        std::os::unix::fs::symlink(d.path().join("secret.pdf"), d.path().join("innocent.pdf"))
            .unwrap();
        let mut access = Access::default();
        access.allow_dropped(&d.path().join("bail.pdf")).unwrap();
        assert!(access.resolve(&d.path().join("innocent.pdf")).is_err());
    }
}
