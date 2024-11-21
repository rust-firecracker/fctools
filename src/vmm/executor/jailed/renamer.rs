use std::path::{Path, PathBuf};

/// An error that can be emitted by a [JailRenamer].
#[derive(Debug)]
pub enum JailRenamerError {
    PathHasNoFilename(PathBuf),
    Other(Box<dyn std::error::Error + Send>),
}

impl std::error::Error for JailRenamerError {}

impl std::fmt::Display for JailRenamerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailRenamerError::PathHasNoFilename(path) => {
                write!(f, "A supposed file has no filename: {}", path.display())
            }
            JailRenamerError::Other(err) => write!(f, "Another error occurred: {err}"),
        }
    }
}

/// A trait defining a method of conversion between an outer path and an inner path. This conversion
/// should always produce the same path (or error) for the same given outside-jail path.
pub trait JailRenamer: Send + Sync + Clone {
    /// Rename the outer path to an inner path.
    fn rename_for_jail(&self, outer_path: &Path) -> Result<PathBuf, JailRenamerError>;
}

/// A resolver that transforms a host path with filename (including extension) "p" into /p
/// inside the jail. Given that files have unique names, this should be enough for most scenarios.
#[derive(Debug, Clone, Default)]
pub struct FlatJailRenamer;

impl JailRenamer for FlatJailRenamer {
    fn rename_for_jail(&self, outside_path: &Path) -> Result<PathBuf, JailRenamerError> {
        Ok(PathBuf::from(
            "/".to_owned()
                + &outside_path
                    .file_name()
                    .ok_or_else(|| JailRenamerError::PathHasNoFilename(outside_path.to_owned()))?
                    .to_string_lossy(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::vmm::executor::jailed::JailJoin;

    use super::{FlatJailRenamer, JailRenamer};

    #[test]
    fn jail_join_performs_correctly() {
        assert_eq!(
            PathBuf::from("/jail").jail_join(&PathBuf::from("/inner")),
            PathBuf::from("/jail/inner")
        );
    }

    #[test]
    fn flat_jail_renamer_moves_correctly() {
        let renamer = FlatJailRenamer::default();
        assert_renamer(&renamer, "/opt/file", "/file");
        assert_renamer(&renamer, "/tmp/some_path.txt", "/some_path.txt");
        assert_renamer(&renamer, "/some/complex/outside/path/filename.ext4", "/filename.ext4");
    }

    fn assert_renamer(renamer: &impl JailRenamer, path: &str, expectation: &str) {
        assert_eq!(
            renamer.rename_for_jail(&PathBuf::from(path)).unwrap().to_str().unwrap(),
            expectation
        );
    }
}
