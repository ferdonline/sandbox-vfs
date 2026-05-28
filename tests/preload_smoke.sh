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

SANDBOX_VFS_LOWER="/" \
SANDBOX_VFS_UPPER="${upper}" \
LD_PRELOAD="${preload}" \
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
