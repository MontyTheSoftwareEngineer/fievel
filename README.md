# fievel

Never touch the mouse again! A Linux application written in rust to replace 
mouse movements with keyboard. Includes everythign that I needed to completely
replace my mouse with keyboard. There is also an included key remapper that
allows mapping combo's so that you can do home-row style mods on ANY keyboard,
not just fancy programmable keyboards.

Fievel reads all suitable keyboards through evdev by default and creates two uinput devices:
`fievel keyboard` for typing and keyboard mappings, and `fievel pointer` for
mouse events. 

## Quickstart

No Rust toolchain or build is needed: download the Linux x64 binary archive
(`fievel-linux-x64.tar.gz`) from
[Releases](https://github.com/MontyTheSoftwareEngineer/fievel/releases/latest).
Set up [device permissions](#device-permissions) once.
From the download directory:

```sh
tar -xzf fievel-linux-x64.tar.gz
chmod +x fievel
./fievel
```

Hold **F3** and use **H/J/K/L** to move the mouse. Release F3 to type normally.
Press **Ctrl+C** to stop. No config file is required to use the defaults.

To customize, copy the source repository's [`fievel.config`](fievel.config) to
`~/.config/fievel/fievel.config` (create the directory if needed), then stop
and restart Fievel to reload it. This example also enables
[home-row mods and navigation layers](#home-row-mods-and-navigation-layers),
including **D+F** for Free Mouse Mode alongside **F3**.

See [Configuration](#configuration) for all settings.

## Controls

By default, hold **F3** (weird I know, but it was meant for me NOT to press it)
to enter **Free Mouse Mode** and release F3 to leave it.
This can be configured by setting `free_mouse = "f3"` to desired key (or chord).
By default `mode = "hold"` meaning the Free Mouse Mode button needs to be held.
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
| S | Hold for fast movement and scrolling; release for normal speed |
| A | Hold for slow movement and scrolling; release for normal speed |


Home/End shortcuts are enabled by default and only work in Free Mouse Mode.
Hold the configured scroll up+down keys together to send Home, or scroll
left+right to send End, in either press order.
Set `home_end_enabled = false` to disable these shortcuts.

## Hint mode

Press **Super+Space** for left-click hints or **Super+I** for right-click hints.
Fievel detects targets on screen and draws green boxes with letter labels.
Type a label to click its center. As you type, non-matching hints disappear and
matching letters are highlighted. **Backspace** removes the last letter.

Press the same shortcut again to cancel, or the other shortcut to switch click
buttons without losing your selection. **Escape** or **Enter** also cancels;
Backspace cancels when no letters have been typed.

Boxes have a dark grey fill. Hold **Left Ctrl** to temporarily hide all hints;
release to restore them. Remapped modifiers work too, such as **S+D** mapped to
Ctrl or **A+S** mapped to Super.

Configure shortcuts under `[hints.keys]` and colors under `[hints]`.
`readability_color` controls the fill; `toggle_background` is the hold-to-hide
key despite its name. **F8** (`debug`) cycles normal hints, detected edges, and
color-coded accepted/rejected targets. Clicking is paused in debug views.

Requires a Wayland compositor with wlr layer-shell, screencopy, and virtual-pointer
support, such as Hyprland or Sway. Detection is visual, so it can miss controls
or label non-clickable text. Exit and re-enter after scrolling or changing the
screen. See [Hint detection](HINT-DETECTION.md) for tuning and debug details.


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
**96 raw input units per inch** (6,082,560 units per mile).
The default is a conventional reference, not detected device DPI.
You can choose a different reference scale when reporting:

```sh
fievel --odometer --odometer-units-per-inch 800
```


Totals are stored in `$XDG_STATE_HOME/fievel/odometer.toml`, or
`~/.local/state/fievel/odometer.toml` when `XDG_STATE_HOME` is unset or not absolute.

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

On startup, the application reads `~/.config/fievel/fievel.config`, unless
you select another file with `--config PATH`.
The built-in defaults below do not enable remappings. The bundled
[`fievel.config`](fievel.config) uses these defaults and adds the home-row
mappings described below.

```toml
mode = "hold" # "hold" (default) or "toggle"
notify = true # Persistent, faint Free Mouse Mode indicator
home_end_enabled = true # Scroll up+down sends Home; scroll left+right sends End

[speeds]
normal = 300
slow = 100
fast = 900
scroll = 6
scroll_slow = 1.5
scroll_fast = 24

[easing]
movement = 0.2
scroll = 0.3

[keys]
free_mouse = "f3" # Also accepts chords such as "leftalt + space"
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

[keycast]
enabled = false # Opt in to the caster hotkey; casting starts off
hotkey = "esc"
max_keys = 8
timeout = 3

[hints]
label_symbols = "abcdefghijklmnopqrstuvwxyz"
border_color = "#00ff00e0"
fill_color = "#00ff0018"
readability_color = "#404040e6"
label_color = "#ffffffff"
label_highlight_color = "#ffc107ff"
min_width = 8
max_width = 499
min_height = 4
max_height = 49

[hints.keys]
left = "leftmeta + space"
right = "leftmeta + i"
cancel = "enter"
toggle_background = "leftctrl"
debug = "f8"
```

All settings are optional; omitted values keep their defaults. If the default
file is missing, a message is printed and built-in defaults are used.
Unknown settings, unknown key names, duplicate control bindings, repeated keys
within an activation chord, and nonpositive,
nonfinite, or greater-than-100000 speeds are rejected.
Easing values must be finite numbers between 0 and 1. Hint labels must use 2-26
unique lowercase ASCII letters, and hint size limits must stay positive with
max >= min.

Key names are case-insensitive Linux keycodes: `h`, `space`, `f4`, `leftshift`,
`KEY_LEFTCTRL`, etc. `,`/`comma`, `.`/`dot`, `escape`/`esc`, and `return`/`enter`
are accepted. Controls must use distinct single keys. `free_mouse` and
`[hints.keys]` activators accept either a single distinct key or a `+`-separated
chord such as `"leftalt + space"` or `"leftctrl+leftalt+f3"`. All chord keys must
be held together, in either press order; extra held keys do not prevent
activation. For a buffered home-row activation chord such as D+F, use a
`free_mouse` action in `[remap.main]` as shown below.
The top-level `mode`, `notify`, and `home_end_enabled` settings must appear before any table
(`[speeds]`, `[easing]`, `[keys]`, `[keycast]`, or `[hints]`).
`"hold"` activates mouse mode only while every activation key is down; releasing
any chord member leaves the mode. `"toggle"` switches it on/off each time the
whole chord becomes held; releases and keyboard autorepeat do not toggle it.
Release and repress at least one member to toggle again. Slow/fast and
mouse-button bindings still use hold behavior in either mode.


### Home-row mods and navigation layers

Fievel provides chord-based home-row mods: press neighboring letter keys
together to hold a modifier, and use those letters normally when no chord
completes. It also supports timed keys and named navigation layers.
These mappings are optional and disabled in the built-in defaults, but enabled
in the bundled `fievel.config`. There is no separate `homerow.config` to load.
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
`fievel` appears in the bottom left of the screen.
It stays visible for the entire hold or toggled-on interval, with no timer.
