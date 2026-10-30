<div align="center">
  <img src="logo.png" alt="Fievel logo" width="160">
  <h1>Fievel</h1>
  <p>
    <a href="https://github.com/MontyTheSoftwareEngineer/fievel/stargazers"><img src="https://img.shields.io/github/stars/MontyTheSoftwareEngineer/fievel?style=for-the-badge&logo=github&label=Stars" alt="GitHub stars"></a>
    <a href="https://github.com/MontyTheSoftwareEngineer/fievel/releases/latest"><img src="https://img.shields.io/github/v/release/MontyTheSoftwareEngineer/fievel?display_name=tag&style=for-the-badge&label=Latest%20release" alt="Latest release"></a>
    <a href="https://github.com/MontyTheSoftwareEngineer/fievel/blob/master/LICENSE"><img src="https://img.shields.io/github/license/MontyTheSoftwareEngineer/fievel?style=for-the-badge" alt="MIT license"></a>
    <a href="https://ko-fi.com/montythesoftwareengineer"><img src="https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white" alt="Support Fievel on Ko-fi"></a>
  </p>
</div>

Never touch the mouse again. Replace mouse movements with your keyboard on Linux, macOS, and Windows.
Fievel is a Linux, macOS, and Windows application written in Rust to replace
mouse movements with keyboard. It includes everything needed to completely
replace a mouse with a keyboard, plus a key remapper that
allows mapping combo's so that you can do home-row style mods on ANY keyboard,
not just fancy programmable keyboards.

On Linux, Fievel reads keyboards through evdev and creates two uinput devices:
`fievel keyboard` for typing and keyboard mappings, and `fievel pointer` for
mouse events. On macOS 14 or later, it uses a Quartz event tap, synthesized
CoreGraphics events, and native AppKit overlays. On Windows, it uses low-level
keyboard and mouse hooks, synthesized `SendInput` events, and a transparent
native overlay.

## Tutorial

[![Watch the Fievel tutorial on YouTube](https://img.youtube.com/vi/uhxbh40br0Q/hqdefault.jpg)](https://youtu.be/uhxbh40br0Q)

## Quickstart

No Rust toolchain or build is needed: download the latest builds from:
[Releases](https://github.com/MontyTheSoftwareEngineer/fievel/releases/latest).

### Linux:
For Linux you probably need to:
Set up [device permissions](#device-permissions) once.
From the download directory:

```sh
tar -xzf fievel-linux-x64.tar.gz
chmod +x fievel
./fievel
```
On first launch, Fievel creates `~/.config/fievel/fievel.config` from the
commented defaults embedded in the binary.

### MacOS:
Start the fievel binary from Terminal, or double click the Fievel.app app bundle to launch. 
On Mac, the first two launches will cause MacOS to prompt you for permissions, which you will need to allow
in order to use Fievel. Without Input Monitoring and Accessibility persmissions, Fievel will not launch (you
will get error messages until you allow these).
The last permission required is Screen Recording in order for Hint Mode to operate, but Fievel will start
without it. Press keybind for Hint Mode (default is cmd + space) for it to trigger the prompt to allow 
screen recording permissions. Hint Mode requires this permission as it needs to catpure a screenshot of the 
screen for hint processing.

On first launch, Fievel creates `~/.config/fievel/fievel.config` from the
commented defaults embedded in the binary.

### Windows:

For Windows, just launch the Fievel.exe. On first launch, Fievel creates
`%APPDATA%\Fievel\fievel.config` from the commented defaults embedded in the
binary.

To enter Free Mouse Mode use either F3 or d+f simultaneously and use **H/J/K/L** to move the mouse. 
Release F3 or d and f to type normally.

To customize, edit the generated `fievel.config` at the platform-specific
location above, then stop and restart Fievel to reload it, or press
**Super+Shift+R** while Fievel is running. The reload shortcut can be changed
with `keys.reload_config`, and a desktop notification confirms whether the
reload succeeded. The source [`fievel.config`](fievel.config) is also available
as an example and enables [home-row mods and navigation layers](#home-row-mods-and-navigation-layers),
including **D+F** for Free Mouse Mode alongside **F3**.

Press **Super+Shift+?** to open the config screen in your browser.
It shows the complete configuration, including defaults; saving validates,
writes, and applies the settings immediately. Change its shortcut with
`keys.config_screen`.

See [Configuration](#configuration) for all settings.

### macOS

Build on macOS 14 or later with Rust and Cargo:

```sh
cargo build --release
./target/release/fievel
```

Fievel shows a cursor icon in the menu bar while running. To embed a custom
PNG or ICNS icon in the build, set `FIEVEL_MENU_BAR_ICON` to the image file:

```sh
FIEVEL_MENU_BAR_ICON=./assets/fievel.png cargo build --release
```

The icon is embedded in the binary, so the release archive does not need the
image file. If the variable is unset, Fievel uses the macOS cursor symbol.

To create a macOS app bundle and release archive after building:

```sh
./scripts/package-release-macos.sh
```

The app is written to `target/release/Fievel.app`; open it from Finder to run
Fievel without a terminal.
The app bundle is ad-hoc signed with a stable identifier so macOS privacy
permissions can be associated with Fievel across rebuilds; it is not Developer
ID signed or notarized.

The menu bar menu shows the config file Fievel loaded and can open the config
folder or reload configuration changes without restarting Fievel. You can also open
the configuration screen from this menu to edit config in browser. If the file
is missing, Fievel recreates it from the embedded defaults on startup.

The app bundle uses the generic macOS icon unless `assets/fievel.icns` exists.
To provide a custom Finder icon, replace that optional file with an `.icns`
icon (a 1024x1024 PNG is a good source image to convert to `.icns`).

On first launch, allow **Input Monitoring** and **Accessibility** for the
terminal or app running Fievel in **System Settings → Privacy & Security**.
Quit and restart Fievel after granting access. Hint mode also needs **Screen
Recording**; macOS requests it the first time **Super+Space** or **Super+I** is
used. Grant it to the same terminal/app and restart Fievel before trying hints
again. Fievel does not need root access.

Some terminal apps enable **Secure Keyboard Entry**, which prevents global
keyboard capture while that terminal is focused. If keys type into the terminal
instead of activating Fievel, turn Secure Keyboard Entry off in the terminal's
menu or focus another app while using Fievel.

### Windows

Windows 10 version 1703 or later is required for per-monitor DPI awareness.

Build with Rust and Cargo from the project directory, then launch from
PowerShell:

```powershell
cargo build --release
.\target\release\fievel.exe
```

To create a Windows release archive after building:

```powershell
.\scripts\package-release-windows.ps1
```

The archive is written to `target\release\fievel-<version>-windows-<arch>.zip`.

Fievel runs as the current user; administrator access is not normally needed.
While running, it installs global low-level keyboard and mouse hooks so it can
remap keys and account for physical pointer movement. 

Fievel adds an icon to the taskbar notification area while running. Click it to
view the odometer totals and config status, open the config screen,
open the config folder, reload the config, or quit Fievel.

The default configuration is
`%APPDATA%\Fievel\fievel.config`; the odometer is stored in
`%LOCALAPPDATA%\Fievel\odometer.toml`. Both paths can be overridden for config
with `--config PATH`. Windows uses the shared Linux-style key names for its
supported common keys.

Windows security boundaries still apply: Fievel cannot capture input on the
secure desktop (for example, the UAC prompt or sign-in screen), and Windows
blocks a non-elevated process from injecting input into an elevated
application. Run Fievel elevated only if you specifically need to control an
elevated app. Protected or secure content may also be unavailable to visual
hint detection.

## Optional: Run at startup

These options start Fievel when you sign in. They are optional; you can also
launch it manually as shown in the [Quickstart](#quickstart).

### Linux

Install the service if you want Fievel to run in the background, start with
your graphical session, restart automatically if it exits, and send its logs
to the systemd journal.

From a cloned repository with Rust and Cargo installed, run:

```sh
./scripts/install-systemd-service.sh
```

The installer builds Fievel, installs it to `~/.local/bin/fievel`, and enables
the service for `graphical-session.target`. If your session manager starts that
target, the service starts with your graphical session. It still needs the
[device permissions](#device-permissions) described below.
The service restarts Fievel whenever it exits; use
`systemctl --user stop fievel.service` when you want it to stay stopped. The
installer skips an existing unit. To update the unit after changing it in the
repository, run:

```sh
install -Dm644 systemd/fievel.service "$HOME/.config/systemd/user/fievel.service"
systemctl --user daemon-reload
systemctl --user restart fievel.service
```

For a standalone Hyprland session that does not manage
`graphical-session.target`, replace a direct `exec-once = fievel` entry with:

```ini
exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
exec-once = systemctl --user start fievel.service
exec-shutdown = systemctl --user stop fievel.service
```

### macOS

Move `Fievel.app` to a permanent location, such as `/Applications`, and launch
it once to grant the required macOS permissions. Then open **System Settings >
General > Login Items** (called **Login Items & Extensions** on some macOS
versions), click **+** under **Open at Login**, and select `Fievel.app`.
Remove it from **Open at Login** to disable automatic startup.

### Windows

Keep the extracted `fievel.exe` in a permanent location. Create a shortcut to
it, then press **Win+R**, enter `shell:startup`, and move the shortcut into the
Startup folder that opens. Remove the shortcut from that folder to disable
automatic startup.

## Controls

By default, hold **F3** (weird I know, but it was meant for me NOT to press it)
to enter **Free Mouse Mode** and release F3 to leave it. You can also use a key
remap bind like d+f to enter free mouse mode.
This can be configured by setting `free_mouse = "f3"` to desired key (or chord).
By default `mode = "hold"` meaning the Free Mouse Mode button needs to be held.
While holding it, press **Z** to lock Free Mouse Mode, then release the
activation key. Press the activation key again to unlock it (it remains active
until released), or press **Z** again to leave Free Mouse Mode.
With `mode = "toggle"`, you can press the Free Mouse Mode button once to enter
Free Mouse Mode, and pressing it again will exit Free Mouse Mode.
By default, a faint **fievel** rectangle remains at the bottom-left while Free
Mouse Mode is active. You can hide this by setting `notify = false`.

| Key while Free Mouse Mode is active | Action |
| --- | --- |
| H / J / K / L | Move left / down / up / right |
| M + , | Home (top of page) |
| N + . | End (bottom of page) |
| Space | Left button down on press, up on release (supports dragging) |
| I | Right button down on press, up on release |
| N / M / , / . | Scroll left / down / up / right |
| S | Hold for fast movement and scrolling; add either Shift key for double speed |
| A | Hold for slow movement and scrolling; release for normal speed |
| Q | Show a brief red pulse around the cursor |


The locator key is configurable with `keys.locate` and defaults to Q.
It is consumed only while Free Mouse Mode is active.
On Linux, the locator compares Wayland screencopy frames with and without the
cursor. It requires `wlr-screencopy` cursor-overlay support, and rapidly
changing screen content can occasionally confuse detection. On macOS, it draws
a brief pulse at the current cursor position using the native overlay. Windows
uses the same native-overlay pulse.

Home/End shortcuts are enabled by default and only work in Free Mouse Mode.
Hold the configured scroll up+down keys together to send Home, or scroll
left+right to send End, in either press order.
Set `home_end_enabled = false` to disable these shortcuts.

## Hint mode

Press **Super+Space** for left-click hints or **Super+I** for right-click hints.
Fievel detects targets on screen and draws green boxes with letter labels.
Compact, high-contrast icons are also detected separately when faint backgrounds
merge them with notification badges or neighboring controls. Colored badge edges
are also separated from neutral icon contours, even when they touch.
Type a label to click its center. As you type, non-matching hints disappear and
matching letters are highlighted. **Backspace** removes the last letter.

Recursive grid hints are disabled by default. Set
`hints.recursive_grid_enabled = true` in the `[hints]` config section to enable
them. Then press **Super+G** for a left-click grid on the display containing
the pointer. Choose a quadrant with **U/I/J/K** (upper-left, upper-right,
lower-left, lower-right) to subdivide recursively. The pointer follows the
selected region's center as you type or backtrack. Press **Space** to left-click
or **R** to right-click at the pointer. Configure quadrant keys using letters
A-Z with `hints.keys.grid_upper_left`, `grid_upper_right`, `grid_lower_left`,
and `grid_lower_right`; set `grid_left_click` and `grid_right_click` to change
the click buttons. The overlay labels show your configured quadrant keys.
**Backspace** steps back, and **Escape** or **Enter** cancels.

In label-hint mode, press the same shortcut again to cancel, or the other
shortcut to switch click buttons without losing your selection. **Escape** or
**Enter** also cancels; Backspace cancels when no letters have been typed.
Hint shortcuts are inactive while Free Mouse Mode is active, so Super+click can
be used for desktop window actions without opening hints.

Boxes have a dark grey fill. Hold **Left Ctrl** to temporarily hide all hints;
release to restore them. Remapped modifiers work too, such as **S+D** mapped to
Ctrl or **A+S** mapped to Super.

Configure shortcuts under `[hints.keys]` and colors under `[hints]`.
`readability_color` controls the fill; `toggle_background` is the hold-to-hide
key despite its name. **F8** (`debug`) cycles normal hints, detected edges, and
color-coded accepted/rejected targets. Clicking is paused in debug views.

To customize recursive-grid quadrants independently, set colors in the
`[hints.grid_colors]` table using `upper_left`, `upper_right`, `lower_left`, and
`lower_right`. Values accept `#RRGGBB` or `#RRGGBBAA`. Each color sets that
quadrant's border and fill tint at every recursion depth; fill opacity continues
to use `[hints].fill_color`. Unset quadrants retain the existing `[hints]`
`border_color` and `fill_color`.

On Linux, hint mode requires a Wayland compositor with wlr layer-shell,
screencopy, and virtual-pointer support, such as Hyprland or Sway. On macOS 14
or later, it uses ScreenCaptureKit and a native transparent overlay and requires
Screen Recording permission. On Windows, it uses GDI desktop capture and a
native transparent overlay; protected content may not be capturable. Detection
is visual, so it can miss controls or label non-clickable text. Exit and
re-enter after scrolling or changing the screen. See
[Hint detection](HINT-DETECTION.md) for tuning and debug details.


## Build and run

With [Rust and Cargo](https://rustup.rs/) installed, build from the project directory:

```sh
cargo build --release
```

Set up [device permissions](#device-permissions) once, then run the binary:

```sh
./target/release/fievel
```

Or copy it to a directory on your `PATH`

### Package a release

After building the release binary, create a versioned Linux x64 archive with:

```sh
./scripts/package-release.sh
```

This writes `target/release/fievel-<version>-linux-x64.tar.gz`, containing the
binary, example config, repository scripts, systemd unit, README, and license.
The binary also includes the commented default config and creates the user
config file on first launch.
The installer in the archive uses its bundled `fievel` binary and does not need
the Rust toolchain. Pass an archive path to write somewhere else:

```sh
./scripts/package-release.sh ./fievel-linux-x64.tar.gz
```

### Movement odometer

While fievel is running, it automatically keeps separate totals since reset for
movement emitted by fievel and movement from physical mice/trackpads, including
when Free Mouse Mode is inactive. In another terminal, run:

```sh
fievel --odometer
```

This prints both totals and exits without opening input devices, grabbing a
keyboard, loading configuration, or disturbing the running instance. It also
works when fievel is stopped.

Reset both totals from any terminal:

```sh
fievel --reset-odometer
```

This permanently clears both counters. 
Distances are displayed in **estimated miles**, using a reference scale of
**96 units per inch** (6,082,560 units per mile). Linux units are raw input
units; macOS units are display points; Windows units are screen pixels. This
is a conventional reference, not detected device DPI or physical distance.
You can choose a different reference scale when reporting:

```sh
fievel --odometer --odometer-units-per-inch 800
```


Totals are stored in `$XDG_STATE_HOME/fievel/odometer.toml` when that variable
is set to an absolute path. Linux otherwise uses
`~/.local/state/fievel/odometer.toml`; macOS uses
`~/Library/Application Support/Fievel/odometer.toml`; Windows uses
`%LOCALAPPDATA%\Fievel\odometer.toml`.

### Device permissions

To run without sudo, fievel needs read access to each keyboard's input device and
write access to `/dev/uinput`. On distributions that grant input-device access
through the `input` group, run this once as your normal user:

```sh
#!/bin/sh
set -e

sudo groupadd --system --force fievel
sudo usermod -aG input,fievel "$USER"

sudo tee /etc/udev/rules.d/99-fievel-input.rules <<EOF
# Output: Virtual device creation
KERNEL=="uinput", GROUP="fievel", MODE:="0660"
EOF

sudo modprobe uinput
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf

sudo udevadm control --reload-rules && sudo udevadm trigger
```

**Log out and back in** for the new group memberships to take effect. The module
will also load automatically on future boots. If your distribution does not use
the `input` group, grant read access to the selected keyboard using its device
permission mechanism instead.

## Configuration

On startup, Linux and macOS read `~/.config/fievel/fievel.config`; Windows
reads `%APPDATA%\Fievel\fievel.config`. If the file is missing, Fievel creates
it from the commented defaults embedded in the binary. Select another file
with `--config PATH`.
The built-in defaults below do not enable remappings. The source
[`fievel.config`](fievel.config) uses these defaults and adds the home-row
mappings described below.

```toml
mode = "hold" # "hold" (default) or "toggle"
notify = true # Persistent, faint Free Mouse Mode indicator
home_end_enabled = true # Scroll up+down sends Home; scroll left+right sends End

[macros]
enabled = false # Set true to enable macro recording and playback
hotkey = "super + e" # Open macro registers; configurable

[speeds]
normal = 300
slow = 100
fast = 900
scroll = 6
scroll_slow = 1.5
scroll_fast = 24

[easing]
movement = 0.2
# movement_stop = 1 # Optional; easing while slowing down, stopping, or reversing
scroll = 0.3

[keys]
free_mouse = "f3" # Also accepts chords such as "leftalt + space"
reload_config = "leftmeta + leftshift + r" # Reload configuration on Linux, macOS, and Windows
config_screen = "leftmeta + leftshift + slash" # Open the browser-based settings screen
toggle_hold = "z" # Locks hold mode until pressed again or Free Mouse Mode is pressed
left = "h"
down = "j"
up = "k"
right = "l"
left_click = "space"
right_click = "i"
scroll_left = "n"
scroll_down = "m"
scroll_up = ","
scroll_right = "."
slow = "a"
fast = "s"
locate = "q"

[keycast]
enabled = false # Opt in to the caster hotkey; casting starts off
hotkey = "esc"
max_keys = 8
timeout = 3

[hints]
recursive_grid_enabled = false # Set true to enable recursive grid hints
label_symbols = "abcdefghijklmnopqrstuvwxyz"
border_color = "#00ff00e0"
fill_color = "#00ff0018"
readability_color = "#404040e6"
label_color = "#ffffffff"
label_highlight_color = "#ffc107ff"
min_width = 8
max_width = 499
min_height = 4
max_height = 96

[hints.keys]
left = "leftmeta + space"
right = "leftmeta + i"
grid = "leftmeta + g"
grid_upper_left = "u"
grid_upper_right = "i"
grid_lower_left = "j"
grid_lower_right = "k"
grid_left_click = "space"
grid_right_click = "r"
cancel = "enter"
toggle_background = "leftctrl"
debug = "f8"
```

On some systems like Windows or Mac, super (win or cmd) may be inconvenient,
so there is an alt-based config included that uses alt instead of super for
most keybinds import [`fievel.alt.config`](fievel.alt.config)
in the settings screen and save, or launch with `--config fievel.alt.config`.
Its keybinds use Alt while its example remappings still emit Super.
The settings screen can also import another TOML config for review before saving
and export the current saved settings as `fievel.config`.

All settings are optional; omitted values keep their defaults. If the default
file is missing, a message is printed and built-in defaults are used.
On Wayland, if the cursor is hidden and its display cannot be identified from
screencopy, grid mode starts at the center of the first available output.
Unknown settings, unknown key names, duplicate control bindings, repeated keys
within an activation chord, and nonpositive,
nonfinite, or greater-than-100000 speeds are rejected.
Easing values must be finite numbers between 0 and 1. `movement_stop` applies
when the pointer slows down, stops, or reverses; leaving it unset keeps the
symmetric `movement` easing. With `movement_stop = 1` (or 0) the pointer stops
instantly and a reversal restarts from rest, so a low `movement` value can give
fine taps without coasting. Hint labels must use 2-26 unique lowercase ASCII
letters, and hint size limits must stay positive with max >= min.

Key names are case-insensitive Linux-style key names: `h`, `space`, `f4`, `leftshift`,
`KEY_LEFTCTRL`, etc. Modifier aliases `ctrl`/`control`, `super`/`meta`, `win`/`windows`,
`cmd`/`command`, `alt`/`option`, and `shift` are accepted on every OS; aliases bind the
corresponding left-side key (`cmd` is Command on macOS, `win` is the Windows key on Windows).
`,`/`comma`, `.`/`dot`, `escape`/`esc`, and `return`/`enter` are also accepted.
Controls must use distinct single keys. Shift passes through to the focused app
in Free Mouse Mode unless a speed key is held. With the fast key (S by default),
either Shift key enables double speed for movement and scrolling. `free_mouse` and
`[hints.keys]` activators accept either a single distinct key or a `+`-separated
chord such as `"leftalt + space"` or `"leftctrl+leftalt+f3"`. All chord keys must
be held together, in either press order; extra held keys do not prevent
activation. For a buffered home-row activation chord such as D+F, use a
`free_mouse` action in `[remap.main]` as shown below.
The top-level `mode`, `notify`, and `home_end_enabled` settings must appear before any table
(`[speeds]`, `[easing]`, `[keys]`, `[keycast]`, or `[hints]`).
`"hold"` activates mouse mode only while every activation key is down; releasing
any chord member leaves the mode. While active in hold mode, `toggle_hold`
(**Z** by default) locks Free Mouse Mode so the activation key can be released.
Pressing the activation key unlocks it and keeps the mode active until release;
pressing `toggle_hold` again exits immediately. `"toggle"` switches it on/off each time the
whole chord becomes held; releases and keyboard autorepeat do not toggle it.
Release and repress at least one member to toggle again. Slow/fast and
mouse-button bindings still use hold behavior in either mode.


### Home-row mods and navigation layers

Fievel provides chord-based home-row mods: press neighboring letter keys
together to hold a modifier, and use those letters normally when no chord
completes. It also supports timed keys and named navigation layers.
These mappings are optional and disabled in the built-in defaults, but enabled
in the embedded default config. There is no separate `homerow.config` to load.
To disable them, remove or comment out all `[remap...]` tables and their entries.
The bundled mappings are:

```toml
mode = "hold"

[remap]
chord_timeout = 25 # milliseconds

[remap.main]
"a+s" = "super"
"s+d" = "ctrl"
"i+o" = "ctrl"
"u+i" = "super"
"a+f" = "escape"
"d+f" = "free_mouse" # Direct mouse action, no F3 event; either press order
"w+e" = "layer(nav)"
capslock = "timeout(layer(control), 175, layer(nav))"

[remap.layers.nav]
h = "left"
j = "down"
k = "up"
l = "right"
```

Binding names are single Linux keys or `+`-separated chords, in either press
order. Actions are key names (`escape`, `f3`, `left`, etc.), held modifiers
(`ctrl`, `super`), `layer(name)`, `free_mouse`, or
`timeout(short_action, milliseconds, held_action)`. `layer(control)` and
`layer(meta)` are aliases for holding left Ctrl and left Super.

Chord members are buffered for up to `chord_timeout` milliseconds (default 25).
A completed chord consumes its members, so letters do not leak into the
application. If no chord completes, pending keys are replayed in press order
through their single-key mappings. Only potential chord members incur this
delay.
Holding a modifier chord holds the modifier, not just a tap.

The Caps Lock binding provides a timed Ctrl/navigation choice: releasing
before 175 ms taps Ctrl; pressing another key before that deadline chooses
Ctrl first, allowing a quick Caps+C to send Ctrl+C. Holding Caps without
another press for 175 ms chooses navigation instead. That choice lasts until
Caps is released, so a quick Ctrl shortcut does not turn into navigation
mid-shortcut. 

`free_mouse` follows the top-level `mode`: hold the chord in hold mode, or
press it again to switch off in toggle mode. Mouse controls take precedence
over keyboard mappings while mouse mode is active, so H/J/K/L move the pointer
even with navigation held and I clicks rather than waiting for I+O. Activation
members remain reserved until release. With this profile, hold D+F to control
the mouse, then hold S for fast movement or A for slow movement.


To try the combined example from the project directory, stop any running Fievel
instance, then run:

```bash
./target/release/fievel --config fievel.config
```


## Caster mode (keycast)

Enable keycast in your configuration, then restart fievel:

```toml
[keycast]
enabled = true
hotkey = "esc"
max_keys = 8
timeout = 3
```

While Free Mouse Mode is active, press **Escape** to toggle casting on or off.
Then press **K** and **L** to move up-right: `K L` appears at the bottom-right.
The display shows your configured movement, click, scroll, and speed-control
bindings in press order, including the scroll keys used for Home/End shortcuts.
It does not show activation keys, the caster hotkey, normal typing, key releases,
or keyboard autorepeat. Releasing and pressing a control again adds another entry.

`max_keys` keeps the newest **8** presses by default (allowed: 1-32), removing
the oldest when full. The whole history clears after **3 seconds** without a
new control press; `timeout` accepts seconds greater than zero and at most 60,
including fractions. 

## Mode indicator

`notify = true` (the default) enables a persistent graphical indicator, **not**
a desktop notification. While Free Mouse Mode is active, a rectangle labeled 
`fievel` appears in the bottom left of the screen. It changes to `HOLD` while
hold mode is locked.
It stays visible for the entire hold or toggled-on interval, with no timer.
