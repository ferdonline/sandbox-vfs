use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use libc::{F_OK, O_CREAT, O_TRUNC, O_WRONLY};
use sandbox_vfs::{filesystem::LowLevelFS, root_vfs::RootVFS, BindFS, MemoryFS, OverlayFS};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sandbox-vfs-overlay-test-{}-{unique}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn overlay_writes_new_entries_to_memory_layer_not_bind_layer() {
    let lower = TempDir::new();
    fs::create_dir(lower.path().join("existing")).unwrap();
    fs::write(lower.path().join("existing").join("seed.txt"), b"lower").unwrap();

    let overlay = OverlayFS::new(
        "overlay",
        MemoryFS::new("upper"),
        Box::new(BindFS::new("lower", lower.path())),
    );

    assert_eq!(overlay.access(Path::new("/existing/seed.txt"), F_OK), 0);

    assert_eq!(overlay.mkdir(Path::new("/created"), 0o755), 0);
    assert_eq!(overlay.access(Path::new("/created"), F_OK), 0);
    assert_eq!(overlay.top_layer().access(Path::new("/created"), F_OK), 0);
    assert!(!lower.path().join("created").exists());

    assert_eq!(overlay.mkdir(Path::new("/existing/new_dir"), 0o755), 0);
    assert_eq!(overlay.access(Path::new("/existing/new_dir"), F_OK), 0);
    assert_eq!(
        overlay
            .top_layer()
            .access(Path::new("/existing/new_dir"), F_OK),
        0
    );
    assert!(!lower.path().join("existing").join("new_dir").exists());

    let fd = overlay.open(
        Path::new("/existing/new_file.txt"),
        O_CREAT | O_WRONLY,
        0o644,
    );
    assert!(fd > 0);
    assert_eq!(
        overlay
            .top_layer()
            .access(Path::new("/existing/new_file.txt"), F_OK),
        0
    );
    assert!(!lower.path().join("existing").join("new_file.txt").exists());
}

#[test]
fn root_vfs_writes_under_bind_mount_to_real_directory() {
    let mounted = TempDir::new();
    let root = RootVFS::new(MemoryFS::new("root"))
        .with_mount("/mnt", Box::new(BindFS::new("mounted", mounted.path())));

    assert_eq!(root.mkdir(Path::new("/mnt/new_dir"), 0o755), 0);
    assert!(mounted.path().join("new_dir").is_dir());

    let file_path = Path::new("/mnt/new_dir/hello.txt");
    let fd = root.open(file_path, O_CREAT | O_WRONLY | O_TRUNC, 0o644);
    assert!(fd > 0);
    assert_eq!(root.access(file_path, F_OK), 0);

    let real_file = mounted.path().join("new_dir").join("hello.txt");
    assert!(real_file.is_file());

    let contents = b"hello from bind mount\n";
    let written = unsafe { libc::write(fd, contents.as_ptr().cast(), contents.len()) };
    assert_eq!(written, contents.len() as isize);
    unsafe {
        libc::close(fd);
    }

    assert_eq!(fs::read(real_file).unwrap(), contents);
}
