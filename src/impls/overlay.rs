use std::path::Path;

use libc::{mode_t, stat, F_OK, O_APPEND, O_CREAT, O_RDWR, O_TRUNC, O_WRONLY};

use crate::filesystem::{LowLevelFS, OpenResult, VfsDirEntry};

/// A purely virtual overlay FS
#[allow(unused)]
#[derive(Debug)]
pub struct OverlayFS {
    id: String,
    top: Box<dyn LowLevelFS>,
    middle: Option<Box<dyn LowLevelFS>>,
    base: Box<dyn LowLevelFS>,
}

impl OverlayFS {
    pub fn new(
        id: impl Into<String>,
        top_rw_layer: Box<dyn LowLevelFS>,
        base: Box<dyn LowLevelFS>,
    ) -> Self {
        Self {
            id: id.into(),
            top: top_rw_layer,
            middle: None,
            base,
        }
    }

    pub fn with_extra_base(mut self, layer: Box<dyn LowLevelFS>) -> Self {
        self.middle = Some(layer);
        self
    }

    pub fn top_layer(&self) -> &dyn LowLevelFS {
        self.top.as_ref()
    }

    pub fn base_layer(&self) -> &dyn LowLevelFS {
        self.base.as_ref()
    }

    fn visible_layer(&self, path: &Path) -> &dyn LowLevelFS {
        if self.top.access(path, F_OK) == 0 {
            return self.top.as_ref();
        }
        if let Some(middle) = &self.middle {
            if middle.access(path, F_OK) == 0 {
                return middle.as_ref();
            }
        }
        self.base.as_ref()
    }

    fn path_exists_below_top(&self, path: &Path) -> bool {
        self.middle
            .as_ref()
            .is_some_and(|fs| fs.access(path, F_OK) == 0)
            || self.base.access(path, F_OK) == 0
    }

    fn ensure_top_parent_dirs(&self, path: &Path) {
        let Some(parent_dir) = path.parent() else {
            return;
        };

        if self.top.access(parent_dir, F_OK) == 0 || !self.path_exists_below_top(parent_dir) {
            return;
        }

        for p in parent_dir
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter(|p| *p != Path::new("/"))
        {
            if self.top.access(p, F_OK) != 0 {
                self.top.mkdir(p, 0o755);
            }
        }
    }
}

impl LowLevelFS for OverlayFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, path: &Path, mode: i32) -> i32 {
        let fs = self.visible_layer(path);
        if mode == F_OK {
            fs.access(path, F_OK)
        } else {
            fs.access(path, mode)
        }
    }

    fn open(&self, path: &Path, oflag: i32, mode: mode_t) -> i32 {
        let writes = oflag & (O_WRONLY | O_RDWR | O_APPEND | O_CREAT | O_TRUNC) != 0;
        if writes {
            self.ensure_top_parent_dirs(path);
            return self.top.open(path, oflag, mode);
        }

        self.visible_layer(path).open(path, oflag, mode)
    }

    fn open_with_handle(&self, path: &Path, oflag: i32, mode: mode_t) -> OpenResult {
        let writes = oflag & (O_WRONLY | O_RDWR | O_APPEND | O_CREAT | O_TRUNC) != 0;
        if writes {
            self.ensure_top_parent_dirs(path);
            return self.top.open_with_handle(path, oflag, mode);
        }

        self.visible_layer(path).open_with_handle(path, oflag, mode)
    }

    fn openat(&self, _dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
        self.open(path, flag, mode)
    }

    fn mkdir(&self, path: &Path, mode: mode_t) -> i32 {
        // Write operation targets only the top layer
        // If existed in lower layer it shadows it
        self.ensure_top_parent_dirs(path);
        self.top.mkdir(path, mode)
    }

    fn unlink(&self, path: &Path) -> i32 {
        if self.top.access(path, F_OK) == 0 {
            return self.top.unlink(path);
        }

        self.visible_layer(path).unlink(path)
    }

    fn rmdir(&self, path: &Path) -> i32 {
        if self.top.access(path, F_OK) == 0 {
            return self.top.rmdir(path);
        }

        self.visible_layer(path).rmdir(path)
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> i32 {
        if self.top.access(old_path, F_OK) == 0 {
            self.ensure_top_parent_dirs(new_path);
            return self.top.rename(old_path, new_path);
        }

        self.visible_layer(old_path).rename(old_path, new_path)
    }

    fn chmod(&self, path: &Path, mode: mode_t) -> i32 {
        self.visible_layer(path).chmod(path, mode)
    }

    fn stat(&self, path: &Path, statbuf: &mut stat) -> i32 {
        self.visible_layer(path).stat(path, statbuf)
    }

    fn read_dir(&self, path: &Path) -> Option<Vec<VfsDirEntry>> {
        let mut entries = Vec::new();
        let mut saw_virtual_layer = false;

        for layer in [
            Some(self.top.as_ref()),
            self.middle.as_deref(),
            Some(self.base.as_ref()),
        ]
        .into_iter()
        .flatten()
        {
            let Some(layer_entries) = layer.read_dir(path) else {
                continue;
            };

            saw_virtual_layer = true;
            for entry in layer_entries {
                if !entries
                    .iter()
                    .any(|existing: &VfsDirEntry| existing.name == entry.name)
                {
                    entries.push(entry);
                }
            }
        }

        saw_virtual_layer.then_some(entries)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::MemoryFS;

    #[test]
    fn test_overlay_access() {
        let fs = OverlayFS::new("overlay", MemoryFS::new("top"), MemoryFS::new("base"));
        let test_path = Path::new("/bin");
        assert_ne!(fs.access(test_path, F_OK), 0);

        // Ensure created items go to top layer and are visible in main fs
        fs.mkdir(test_path, 0);
        assert_eq!(fs.top_layer().access(test_path, F_OK), 0);
        assert_eq!(fs.access(test_path, F_OK), 0);
    }

    #[test]
    fn test_overlay_creates_parent_dirs_without_touching_root() {
        let top = MemoryFS::new("top");
        let base = MemoryFS::new("base");
        base.mkdir(Path::new("/home"), 0);
        base.mkdir(Path::new("/home/leite"), 0);

        let fs = OverlayFS::new("overlay", top, base);
        fs.open(
            Path::new("/home/leite/hello.txt"),
            O_CREAT | O_WRONLY,
            0o644,
        );

        assert_eq!(fs.top_layer().access(Path::new("/home"), F_OK), 0);
        assert_eq!(fs.top_layer().access(Path::new("/home/leite"), F_OK), 0);
    }

    #[test]
    fn test_overlay_forwards_opened_file_handle() {
        let fs = OverlayFS::new("overlay", MemoryFS::new("top"), MemoryFS::new("base"));

        let result = fs.open_with_handle(Path::new("/file.txt"), O_CREAT | O_WRONLY, 0o644);

        assert!(result.fd >= 0);
        assert!(result.opened.is_some());
    }
}
