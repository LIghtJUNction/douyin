#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sdk_root=${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/android-sdk}}
android_jar="$sdk_root/platforms/android-36/android.jar"
d8_binary="$sdk_root/build-tools/35.0.0/d8"
if [[ $# -gt 1 || ( $# -eq 1 && $1 != --check ) ]]; then
    echo "Usage: bash tools/build-device-helper.sh [--check]" >&2
    exit 1
fi
if [[ ! -f "$android_jar" || ! -x "$d8_binary" ]]; then
    echo "Set ANDROID_SDK_ROOT to an Android SDK with platform android-36 and build-tools 35.0.0." >&2
    exit 1
fi
build_dir=$(mktemp -d)
trap 'rm -rf -- "$build_dir"' EXIT
mkdir "$build_dir/classes" "$build_dir/dex"
javac --release 8 -classpath "$android_jar" -d "$build_dir/classes" \
    "$repo_dir/assets/device-metadata/DeviceMetadata.java"
"$d8_binary" --min-api 26 --lib "$android_jar" --output "$build_dir/dex" \
    "$build_dir/classes/org/lightjunction/douyin/DeviceMetadata.class"
if [[ ${1:-} == --check ]]; then
    cmp "$build_dir/dex/classes.dex" "$repo_dir/assets/device-metadata/classes.dex"
else
    install -m 644 "$build_dir/dex/classes.dex" "$repo_dir/assets/device-metadata/classes.dex"
fi
