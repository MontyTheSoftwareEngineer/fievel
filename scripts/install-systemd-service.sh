#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
unit_file="$HOME/.config/systemd/user/fievel.service"

if [ -e "$unit_file" ]; then
    echo "fievel.service is already installed; skipping."
    exit 0
fi

if ! command -v systemctl >/dev/null 2>&1; then
    echo "systemctl is required to install the user service." >&2
    exit 1
fi

if [ -f "$repo_root/Cargo.toml" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "cargo is required to build fievel; install Rust and try again." >&2
        exit 1
    fi
    cargo build --release --manifest-path "$repo_root/Cargo.toml"
    fievel_bin="$repo_root/target/release/fievel"
elif [ -x "$repo_root/fievel" ]; then
    fievel_bin="$repo_root/fievel"
else
    echo "Could not find a built fievel binary or Cargo.toml in $repo_root." >&2
    exit 1
fi

install -Dm755 "$fievel_bin" "$HOME/.local/bin/fievel"
install -Dm644 "$repo_root/systemd/fievel.service" "$unit_file"

systemctl --user daemon-reload
systemctl --user enable fievel.service
if systemctl --user is-active --quiet graphical-session.target; then
    systemctl --user start fievel.service
fi

echo "Installed and enabled fievel.service."
echo "For a standalone Hyprland session, see the README for the session start/stop commands."
