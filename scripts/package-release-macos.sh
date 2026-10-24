#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
binary="$repo_root/target/release/fievel"

if [ ! -x "$binary" ]; then
    echo "Built fievel binary not found at $binary; run cargo build --release first." >&2
    exit 1
fi

version=$(awk '
    $0 == "[package]" { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "version" {
        gsub(/"/, "", $3)
        print $3
        exit
    }
' "$repo_root/Cargo.toml")
if [ -z "$version" ]; then
    echo "Could not read package version from $repo_root/Cargo.toml." >&2
    exit 1
fi

case $(uname -m) in
    arm64|aarch64) arch=arm64 ;;
    x86_64) arch=x64 ;;
    *)
        echo "Unsupported macOS architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

archive_path=${1:-"$repo_root/target/release/fievel-$version-macos-$arch.tar.gz"}
archive_dir=$(dirname -- "$archive_path")
archive_name=$(basename -- "$archive_path")
mkdir -p "$archive_dir"
archive_dir=$(CDPATH= cd -- "$archive_dir" && pwd)
archive="$archive_dir/$archive_name"

staging=$(mktemp -d "${TMPDIR:-/tmp}/fievel-release.XXXXXX")
archive_tmp=
cleanup() {
    rm -rf -- "$staging"
    if [ -n "$archive_tmp" ]; then
        rm -f -- "$archive_tmp"
    fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
archive_tmp=$(mktemp "$archive.tmp.XXXXXX")

cp -p "$binary" "$staging/fievel"
cp -p "$repo_root/README.md" "$staging/README.md"
cp -p "$repo_root/LICENSE" "$staging/LICENSE"
cp -p "$repo_root/fievel.config" "$staging/fievel.config"

tar -czf "$archive_tmp" -C "$staging" \
    fievel README.md LICENSE fievel.config
mv -f -- "$archive_tmp" "$archive"

echo "Created $archive"
