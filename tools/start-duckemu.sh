#!/usr/bin/env bash
set -euo pipefail

: "${DUCKEMU_ROOT:?Set DUCKEMU_ROOT to the DuckDroid2 source directory}"
: "${DUCKEMU_ROM_SYMBOLS:?Set DUCKEMU_ROM_SYMBOLS to the ROM symbols directory}"

build="${DUCKEMU_BUILD:-$DUCKEMU_ROOT/build}"
socket_dir="${DOUYIN_DUCKEMU_SOCKET_DIR:-/tmp/duckemu-dy7}"
system="$build/duckemu_system"
worker="$build/duckemu_worker"
library="$DUCKEMU_ROOT/code/user_code/dy7_case/assets/libmetasec_ml.so"
properties="$DUCKEMU_ROOT/code/user_code/dy7_case/dy7.properties"
libc="$DUCKEMU_ROM_SYMBOLS/system/lib64/bootstrap/libc.so"
lib64="$DUCKEMU_ROM_SYMBOLS/system/lib64"

for path in "$system" "$worker" "$library" "$properties" "$libc"; do
    if [[ ! -f "$path" ]]; then
        echo "Missing required file: $path" >&2
        exit 1
    fi
done
if [[ ! -x "$system" || ! -x "$worker" ]]; then
    echo "DuckDroid2 binaries are not executable" >&2
    exit 1
fi
if [[ "$(sha256sum "$library" | cut -d' ' -f1)" != "1fe03703b1f3a806b220271ba2ec1dbed049d8b5f0216b70c0c724d5c5bc027c" ]]; then
    echo "Unsupported libmetasec_ml.so" >&2
    exit 1
fi

mkdir -p "$socket_dir"
rm -f "$socket_dir/dy7.sock"
exec "$system" \
    --socket-dir "$socket_dir" \
    --worker-exec "$worker" \
    --app "id=dy7,library=$library,rom-libc=$libc,property-file=$properties,library-path=$lib64:$lib64/bootstrap,instances=1,workers=1,init-timeout-us=120000000,invoke-timeout-us=120000000" \
    "$@"
