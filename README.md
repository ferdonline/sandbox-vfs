# sandbox-vfs

`sandbox-vfs` is an experimental Rust library for routing libc file-system
calls through a virtual file-system layer.

The goal is to provide user-space file-system isolation without requiring
privileged mounts. That makes it a useful playground for restricted container
setups, sandboxed tests, and overlay-style file-system experiments.

## Why not FUSE?

FUSE is the right tool when you need a real mounted filesystem with kernel-level
visibility. `sandbox-vfs` aims at a different niche: changing the filesystem
view of one process tree without creating a mount.

That gives it a few useful properties:

- No FUSE device, mount permission, or privileged helper is required.
- The virtual view is scoped to the intercepted process instead of the host.
- Read-only container images can get writable overlays for selected paths.
- Test and build commands can see fake `/etc`, `$HOME`, cache, or config files
  without changing the surrounding environment.

A concrete use case is safer Kubernetes workloads. Many hardened clusters avoid
privileged containers, extra Linux capabilities, host mounts, or `/dev/fuse`.
With libc interposition, a container entrypoint can run under a lightweight
virtual filesystem view while keeping the pod unprivileged and avoiding mount
lifecycle cleanup.

## Status

This project is still early. The pure VFS pieces are usable for tests and
experimentation, while libc interception is currently Linux-specific.

Implemented pieces:

- `LowLevelFS`, the common trait for file-system backends.
- `MemoryFS`, an in-memory backend intended for tests and virtual entries.
- `BindFS`, a path-rewriting backend rooted at a real directory.
- `OverlayFS`, a writable top layer over one or more lower layers.
- `RootVFS`, a simple mount dispatcher.

## Development

Run the test suite with:

```sh
cargo test
```

Build the interceptor library on Linux with:

```sh
cargo build
```

The example writer binary is useful when testing interception manually:

```sh
cargo run --bin file_writer -- "hello" /tmp/sandbox-vfs-demo.txt
```

## Notes

`BindFS` is a convenience path mapper, not a security boundary. Symlinks,
hardlinks, remounts, and other host file-system features may escape the mapped
root unless additional controls are applied.
