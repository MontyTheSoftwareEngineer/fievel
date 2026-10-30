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
release_dir="$repo_root/target/release"
app_path="$release_dir/Fievel.app"
bundle_identifier=com.montythesoftwareengineer.fievel
mkdir -p "$release_dir"

staging=$(mktemp -d "${TMPDIR:-/tmp}/fievel-release.XXXXXX")
app_staging=$(mktemp -d "$release_dir/.fievel-app.XXXXXX")
archive_tmp=
cleanup() {
    rm -rf -- "$staging"
    rm -rf -- "$app_staging"
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

app_bundle="$app_staging/Fievel.app"
app_contents="$app_bundle/Contents"
mkdir -p "$app_contents/MacOS" "$app_contents/Resources"
cp -p "$binary" "$app_contents/MacOS/fievel"

icon_plist_entry=
if [ -f "$repo_root/assets/fievel.icns" ]; then
    cp -p "$repo_root/assets/fievel.icns" "$app_contents/Resources/fievel.icns"
    icon_plist_entry='<key>CFBundleIconFile</key><string>fievel.icns</string>'
fi

cat > "$app_contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>fievel</string>
    <key>CFBundleIdentifier</key>
    <string>$bundle_identifier</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Fievel</string>
    <key>CFBundleDisplayName</key>
    <string>Fievel</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>LSUIElement</key>
    <true/>
    $icon_plist_entry
</dict>
</plist>
EOF

codesign --force --deep --sign - \
    --identifier "$bundle_identifier" \
    --requirements "=designated => identifier \"$bundle_identifier\"" \
    "$app_bundle"

ditto "$app_bundle" "$staging/Fievel.app"
tar -czf "$archive_tmp" -C "$staging" \
    fievel Fievel.app README.md LICENSE
mv -f -- "$archive_tmp" "$archive"

rm -rf -- "$app_path"
mv -- "$app_bundle" "$app_path"

echo "Created $archive"
echo "Created $app_path"
