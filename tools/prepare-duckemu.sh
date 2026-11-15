#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${DUCKEMU_ROOT:-$HOME/.local/share/douyin-cli/duckemu}"
symbols="${DUCKEMU_ROM_SYMBOLS:-$root/rom-symbols}"
build="${DUCKEMU_BUILD:-$root/build}"
rom_url="https://b4.timschumi.net/lineage-archive/lineage-21.0-20241230-nightly-lemonade-signed.zip"
rom_sha256="996d4fc28d495ac0fb831d65e9de98f7fd2749d78c72c9664808623b605f7ee6"
library_sha256="1fe03703b1f3a806b220271ba2ec1dbed049d8b5f0216b70c0c724d5c5bc027c"

for command in git cmake ninja clang llvm-objcopy debugfs unzip sha256sum; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Missing required command: $command" >&2
        exit 1
    fi
done

if [[ ! -d "$root/.git" ]]; then
    if [[ -e "$root" ]]; then
        echo "$root exists but is not a DuckDroid2 checkout" >&2
        exit 1
    fi
    git clone --depth 1 --recurse-submodules --shallow-submodules \
        https://github.com/djskncxm/DuckDroid2.git "$root"
fi

library="$root/code/user_code/dy7_case/assets/libmetasec_ml.so"
if [[ "$(sha256sum "$library" | cut -d' ' -f1)" != "$library_sha256" ]]; then
    echo "Unsupported libmetasec_ml.so in $root" >&2
    exit 1
fi

mkdir -p "$root/downloads" "$root/rom-images" "$symbols/system/lib64/bootstrap"
archive="$root/downloads/lineage-21.0-20241230-nightly-lemonade-signed.zip"
payload="$root/downloads/lineage-payload.bin"
image="$root/rom-images/system.img"
if [[ ! -f "$symbols/system/lib64/bootstrap/libc.so" ]]; then
    if [[ ! -f "$archive" ]]; then
        if command -v aria2c >/dev/null 2>&1; then
            aria2c -x 16 -s 16 -k 32M -c --max-tries=10 --retry-wait=2 \
                --dir="$root/downloads" --out="$(basename "$archive")" "$rom_url"
        else
            curl -L --fail --retry 3 --output "$archive" "$rom_url"
        fi
    fi
    if [[ "$(sha256sum "$archive" | cut -d' ' -f1)" != "$rom_sha256" ]]; then
        echo "ROM checksum mismatch; remove $archive and retry" >&2
        exit 1
    fi

    if [[ ! -x "$root/downloads/payload-dumper-go" ]]; then
        curl -L --fail --retry 3 \
            --output "$root/downloads/payload-dumper-go.tar.gz" \
            "https://github.com/ssut/payload-dumper-go/releases/download/2.1.0/payload-dumper-go_2.1.0_linux_amd64.tar.gz"
        tar -xzf "$root/downloads/payload-dumper-go.tar.gz" -C "$root/downloads"
        chmod +x "$root/downloads/payload-dumper-go"
    fi
    if [[ ! -f "$payload" ]]; then
        unzip -p "$archive" payload.bin > "$payload"
    fi
    if [[ ! -f "$image" ]]; then
        "$root/downloads/payload-dumper-go" -p system -o "$root/rom-images" "$payload"
    fi
    for file in libc.so libm.so libdl.so libdl_android.so; do
        debugfs -R "dump /system/lib64/bootstrap/$file $symbols/system/lib64/bootstrap/$file" "$image"
    done
    for file in libz.so liblog.so libc++.so; do
        debugfs -R "dump /system/lib64/$file $symbols/system/lib64/$file" "$image"
    done
fi

cmake -S "$root/code" -B "$build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DDUCKEMU_ANDROID_ENV_ROOT="$root/android-env" \
    -DDUCKEMU_ROM_LIB64="$symbols/system/lib64" \
    -DDUCKEMU_ROM_BOOTSTRAP="$symbols/system/lib64/bootstrap" \
    -DDUCKEMU_ROM_SYMBOLS="$symbols"
cmake --build "$build" --target duckemu_dy7_case duckemu_system duckemu_worker -j "${DUCKEMU_BUILD_JOBS:-4}"

cat <<EOF
DuckDroid2 is ready.
Start it with:
  DUCKEMU_ROOT="$root" DUCKEMU_BUILD="$build" DUCKEMU_ROM_SYMBOLS="$symbols" "$script_dir/start-duckemu.sh"
EOF
