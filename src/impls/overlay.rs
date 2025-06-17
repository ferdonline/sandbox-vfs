use std::path::Path;

use libc::F_OK;

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

    fn open(&self, path: &Path, mode: i32) -> i32 {
        // Exists is upper layer? Use it
        if self.top.access(path, F_OK) == 0 {
            return self.top.open(path, mode);
        }
        if let Some(middle) = &self.middle {
            if middle.access(path, F_OK) == 0 {
                return middle.open(path, mode);
            }
        }
        // Otherwise, base has the final answer
        self.base.open(path, mode)
    }

    fn mkdir(&self, path: &Path, mode: i32) -> i32 {
        // Write operation targets only the top layer
        // If existed in lower layer it shadows it
        self.top.mkdir(path, mode)
    }

    fn chmod(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
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
            Box::new(MemoryFS::new("top")),
            Box::new(MemoryFS::new("base")),
        );
        let test_path = Path::new("/bin");
        assert_ne!(fs.access(test_path, F_OK), 0);

        // Ensure created items go to top layer and are visible in main fs
        fs.mkdir(test_path, 0);
        assert_eq!(fs.top_layer().access(test_path, F_OK), 0);
        assert_eq!(fs.access(test_path, F_OK), 0);
    }
}
