use std::path::Path;

use libc::{mode_t, F_OK, O_APPEND, O_RDWR, O_WRONLY};

use crate::filesystem::LowLevelFS;

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
}

impl LowLevelFS for OverlayFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, path: &Path, mode: i32) -> i32 {
        // Exists is upper layer? Use it
        if self.top.access(path, F_OK) == 0 {
            return match mode == F_OK {
                true => 0,
                false => self.top.access(path, mode),
            };
        }
        if let Some(middle) = &self.middle {
            if middle.access(path, F_OK) == 0 {
                return match mode == F_OK {
                    true => 0,
                    false => middle.access(path, mode),
                };
            }
        }
        // Otherwise, base has the final answer
        self.base.access(path, mode)
    }

    fn open(&self, path: &Path, oflag: i32, mode: mode_t) -> i32 {
        // Is this a write request? They are special, because the dir struct might exist in a sub layer
        if oflag & (O_WRONLY | O_RDWR | O_APPEND) > 0 {
            let parent_dir = path.parent().unwrap();
            // Dir exists? Forward

            if self.top.access(parent_dir, F_OK) == 0 {
                return self.top.open(path, oflag, mode);
            }
            // Path exist in a sub layer? Create Path
            if self
                .middle
                .as_ref()
                .is_some_and(|fs| fs.access(parent_dir, F_OK) == 0)
                || self.base.access(parent_dir, F_OK) == 0
            {
                let ancestors: Vec<_> = parent_dir.ancestors().collect();
                for p in ancestors.into_iter().rev() {
                    if self.top.access(p, F_OK) != 0 {
                        self.top.mkdir(p, 0o755);
                    }
                }
            }
            // Now attempt write normally
            return self.top.open(path, oflag, mode);
        }

        // Read only. Basically use the first fs where it's found
        if self.top.access(path, F_OK) == 0 {
            return self.top.open(path, oflag, mode);
        }
        if let Some(middle) = &self.middle {
            if middle.access(path, F_OK) == 0 {
                return middle.open(path, oflag, mode);
            }
        }
        // Base has the final answer
        self.base.open(path, oflag, mode)
    }

    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
        todo!()
    }

    fn mkdir(&self, path: &Path, mode: mode_t) -> i32 {
        // Write operation targets only the top layer
        // If existed in lower layer it shadows it
        self.top.mkdir(path, mode)
    }

    fn chmod(&self, path: &Path, mode: mode_t) -> i32 {
        // Exists is upper layer? Use it
        if self.top.access(path, F_OK) == 0 {
            return self.top.chmod(path, mode);
        }
        if let Some(middle) = &self.middle {
            if middle.access(path, F_OK) == 0 {
                return middle.chmod(path, mode);
            }
        }
        // Otherwise, base has the final answer
        self.base.chmod(path, mode)
    }

}

#[cfg(test)]
mod test {
    use super::*;

    use crate::MemoryFS;

    #[test]
    fn test_overlay_access() {
        let fs = OverlayFS::new(
            "overlay",
            MemoryFS::new("top"),
            MemoryFS::new("base"),
        );
        let test_path = Path::new("/bin");
        assert_ne!(fs.access(test_path, F_OK), 0);

        // Ensure created items go to top layer and are visible in main fs
        fs.mkdir(test_path, 0);
        assert_eq!(fs.top_layer().access(test_path, F_OK), 0);
        assert_eq!(fs.access(test_path, F_OK), 0);
    }
}
