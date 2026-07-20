# MemoryFS Architecture

`MemoryFS` combines a Rust-managed filesystem namespace with kernel-managed
regular-file contents.

This lets sandbox-vfs implement directories, virtual paths, and stable inode
identity in Rust while still returning real file descriptors that applications
can use with ordinary system calls.

## The Three Identities

A filesystem operation involves three different kinds of identity:

```text
Directory entry            Node                         Open file description
"report.txt" -> NodeId     File contents and metadata   One particular open()
                                                       offset and open flags
```

- A **directory entry** associates a name with a node.
- A **node** is the persistent file or directory object.
- An **open file description** represents one open instance, including its
  current offset and status flags.

Keeping these separate is important. For example, two calls to `open()` for the
same file must see the same contents but have independent offsets.

## Rust-Managed Namespace

Each `MemoryFS` has a root node and a map of node IDs to nodes:

```text
MemoryFS
  root: NodeId
  nodes: NodeId -> Node

Directory node
  entries: filename -> NodeId
```

Paths are not stored as node identity. Resolving `/work/report.txt` walks the
directory tree:

```text
root -> "work" -> directory node -> "report.txt" -> file node
```

Node IDs are process-global and stable for the lifetime of their node. This
allows `stat()` and directory entries returned by `getdents64()` to report the
same inode number.

This representation also provides the foundation for rename, unlink, and hard
links: those operations change directory entries without replacing the node.

## Kernel-Backed Regular Files

Every regular-file node owns one anonymous Linux `memfd`.

The memfd is authoritative for:

- File contents
- File size
- Blocks and kernel-managed timestamps
- Operations such as `read`, `write`, `mmap`, and `ftruncate`

Because applications receive real descriptors referring to the memfd,
sandbox-vfs does not need to intercept normal file I/O.

The canonical memfd descriptor is kept private inside the node. When the
application opens the virtual file, `MemoryFS` opens `/proc/self/fd/N` using a
raw `openat` syscall and returns the new descriptor.

```text
File node
  canonical memfd
       |
       +-- open() -> application fd 10, offset 0
       +-- open() -> application fd 11, offset 0
```

Both application descriptors share the same contents but have independent open
file descriptions and offsets. Closing either descriptor does not destroy the
node or invalidate the other descriptor.

The raw syscall is used to avoid recursively entering sandbox-vfs's interposed
`open` hook.

## Directories

Directory contents live entirely in Rust as name-to-node mappings.

Directories currently own a placeholder memfd so `open()` can return a real
descriptor. `RootVFS` tracks that descriptor and uses the virtual directory
contents when servicing `getdents64()`.

The placeholder memfd does not contain the directory entries.

## Metadata Ownership

Metadata ownership is intentionally split:

| Metadata | Owner |
| --- | --- |
| Regular-file contents and size | Kernel memfd |
| Stable virtual inode number | Rust node |
| Directory entries | Rust directory node |
| Virtual link count | Rust node and namespace |
| Virtual file kind and mode | Rust node |

`MemoryFS::stat()` starts with `fstat()` for regular files, then overlays
virtual metadata such as the node ID, link count, kind, and mode.

## Open Descriptor Tracking

`RootVFS` tracks both the virtual path used to open a descriptor and an optional
backend-owned opened-file handle:

```text
fd -> {
    virtual path,
    opened backend object,
    directory offset
}
```

The virtual path remains useful for resolving relative `openat()` calls for
backends that do not expose virtual handles. Operations that act on the
already-opened object, such as `fstat()` and `getdents64()`, use the
backend-owned handle instead.

For `MemoryFS`, the handle retains an `Arc` to the opened node. It therefore
continues to identify the same node even if its original path becomes stale.
When unlink support is added, this will allow an unlinked node to remain usable
through existing open descriptors until the final reference is closed,
matching normal Unix filesystem behavior.

Backends that rely entirely on real kernel descriptors do not need to provide a
virtual opened-file handle.

For `MemoryFS`, simple relative `openat()` and `mkdirat()` paths are also
resolved through the opened directory handle. This keeps an opened directory
useful even if its tracked virtual path becomes stale. Paths containing `..`
still fall back to the path-based resolver because parent lookup is
namespace-dependent until directory nodes grow parent-entry tracking.
