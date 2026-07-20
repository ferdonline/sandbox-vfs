#!/usr/bin/env bash
set -euo pipefail

cargo build --features hooks --lib --bin file_writer

upper="$(mktemp -d)"
name="sandbox-vfs-preload-smoke-$$"
virtual_path="/${name}.txt"
preload="${PWD}/target/debug/libsandbox_vfs.so"
writer="${PWD}/target/debug/file_writer"

cleanup() {
    rm -rf "${upper}"
}
trap cleanup EXIT

if [ -e "${virtual_path}" ]; then
    echo "Refusing to run: ${virtual_path} already exists on the host" >&2
    exit 1
fi

# Check 1
# Ensure a file created comes out in upper layer
SANDBOX_VFS_LOWER="/" SANDBOX_VFS_UPPER="${upper}" LD_PRELOAD="${preload}" \
"${writer}" "hello from preload" "${virtual_path}"

expected="${upper}/${name}.txt"

if [ ! -f "${expected}" ]; then
    echo "Expected redirected file at ${expected}" >&2
    exit 1
fi

if [ "$(cat "${expected}")" != "hello from preload" ]; then
    echo "Redirected file had unexpected contents" >&2
    exit 1
fi

if [ -e "${virtual_path}" ]; then
    echo "Write leaked to the host path ${virtual_path}" >&2
    exit 1
fi


# Check 2
# Ensure we can list directories, working with memoryFS
# Note: Test on a mount so that python loads normally from the root fs
SANDBOX_VFS_LOWER="/" \
SANDBOX_VFS_UPPER="${upper}" \
SANDBOX_VFS_MEMORY_MOUNT="/memFS" \
LD_PRELOAD="${preload}" \
python3 -c '
import ctypes
import os

mem_fs = os.environ["SANDBOX_VFS_MEMORY_MOUNT"]
libc = ctypes.CDLL(None, use_errno=True)

def check(result):
    if result < 0:
        errno = ctypes.get_errno()
        raise OSError(errno, os.strerror(errno))
    return result

def cpath(path):
    return os.fsencode(path)

fd = os.open(os.path.join(mem_fs, "file.txt"), os.O_CREAT | os.O_WRONLY, 0o644)
os.close(fd)
os.mkdir(os.path.join(mem_fs, "subdir"))

entries = sorted(os.listdir(mem_fs))
assert entries == ["file.txt", "subdir"], entries

dir_fd = check(libc.open(cpath(mem_fs), os.O_RDONLY | os.O_DIRECTORY, 0))
dup_fd = check(libc.dup(dir_fd))
fd = check(libc.openat(dup_fd, b"via_dup.txt", os.O_CREAT | os.O_WRONLY, 0o644))
check(libc.write(fd, b"dup fd", 6))
check(libc.close(fd))
check(libc.close(dir_fd))
check(libc.close(dup_fd))

os.mkdir(os.path.join(mem_fs, "work"))
os.mkdir(os.path.join(mem_fs, "work", "build"))
build_fd = check(libc.open(cpath(os.path.join(mem_fs, "work", "build")), os.O_RDONLY | os.O_DIRECTORY, 0))
check(libc.rename(cpath(os.path.join(mem_fs, "work")), cpath(os.path.join(mem_fs, "renamed"))))
fd = check(libc.openat(build_fd, b"../from_dotdot.txt", os.O_CREAT | os.O_WRONLY, 0o644))
check(libc.write(fd, b"renamed parent", 14))
check(libc.close(fd))
check(libc.close(build_fd))

assert sorted(os.listdir(mem_fs)) == ["file.txt", "renamed", "subdir", "via_dup.txt"]
assert sorted(os.listdir(os.path.join(mem_fs, "renamed"))) == ["build", "from_dotdot.txt"]

os.unlink(os.path.join(mem_fs, "via_dup.txt"))
os.unlink(os.path.join(mem_fs, "renamed", "from_dotdot.txt"))
os.rmdir(os.path.join(mem_fs, "renamed", "build"))
os.rmdir(os.path.join(mem_fs, "renamed"))

assert sorted(os.listdir(mem_fs)) == ["file.txt", "subdir"]
'
