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
import struct

mem_fs = os.environ["SANDBOX_VFS_MEMORY_MOUNT"]

fd = os.open(os.path.join(mem_fs, "file.txt"), os.O_CREAT | os.O_WRONLY, 0o644)
os.close(fd)
os.mkdir(os.path.join(mem_fs, "subdir"))

fd = os.open(mem_fs, os.O_RDONLY)
try:
    buf = ctypes.create_string_buffer(1024)
    written = ctypes.CDLL(None).getdents64(fd, buf, len(buf))
finally:
    os.close(fd)

assert written > 0, written
entries = []
offset = 0
while offset < written:
    reclen = struct.unpack_from("H", buf.raw, offset + 16)[0]
    name_start = offset + 19
    name_end = buf.raw.index(b"\0", name_start)
    entries.append(buf.raw[name_start:name_end].decode())
    offset += reclen

entries = sorted(name for name in entries if name not in (".", ".."))
assert entries == ["file.txt", "subdir"], entries
'
