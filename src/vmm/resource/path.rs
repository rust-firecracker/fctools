use std::path::{Path, PathBuf};

use crate::runtime::Runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedPath(pub(super) PathBuf);

impl DetachedPath {
    pub fn into_inner(self) -> PathBuf {
        self.0
    }

    pub async fn copy<R: Runtime, P: Into<PathBuf>>(&mut self, runtime: &R, new_path: P) -> Result<(), std::io::Error> {
        let new_path = new_path.into();
        runtime.fs_copy(&self.0, &new_path).await?;
        self.0 = new_path;
        Ok(())
    }

    pub async fn rename<R: Runtime, P: Into<PathBuf>>(
        &mut self,
        runtime: &R,
        new_path: P,
    ) -> Result<(), std::io::Error> {
        let new_path = new_path.into();
        runtime.fs_rename(&self.0, &new_path).await?;
        self.0 = new_path;
        Ok(())
    }

    pub async fn remove<R: Runtime>(self, runtime: &R) -> Result<(), (Self, std::io::Error)> {
        if let Err(err) = runtime.fs_remove_file(&self.0).await {
            return Err((self, err));
        }

        Ok(())
    }
}

impl AsRef<Path> for DetachedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}
