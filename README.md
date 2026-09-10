# fievel

A Linux Rust application that grabs one keyboard through evdev and creates two
uinput devices: `fievel keyboard` for normal typing, and
`fievel pointer` for mouse events. Works below X11/Wayland and is visible
to tools such as `keyd monitor`. No keystrokes are recorded or sent anywhere.

Hold **F3** to enter **Free Mouse Mode**. Release F3 to leave it.

| Key while F3 is held | Action |
| --- | --- |
| H / J / K / L | Move left / down / up / right |
| Space | Left button down on press, up on release (supports dragging) |
| I | Right button down on press, up on release |
| N / M / , / . | Scroll left / down / up / right |
| S | Hold for fast movement; release for normal speed |
| A | Hold for slow movement; release for normal speed |
| Escape | Exit the application and release its input grab |

Movement is constant-speed, independent of keyboard repeat, with normalized
diagonals (not faster than horizontal/vertical movement). Opposite directions
cancel. Scrolling sends one notch immediately, then repeats at a constant rate
while held. Releasing F3 stops movement/scrolling and releases both mouse buttons,
even if Space or I is still held. F3 itself never reaches applications.
Other keys work normally, including modifiers and Ctrl+C.
Speed changes are immediate, not accelerated ramps. Slow takes priority if A
and S are both held; releasing A while S remains held returns to fast speed.
These modifiers only affect pointer movement, not scrolling. Outside Free Mouse
Mode, A and S type normally. All bindings and speeds can be changed in the config.

Mouse-control keys already held when F3 is pressed transfer to mouse control.
Keys used in mouse mode stay suppressed until released, so leaving the mode
does not accidentally type a held key. Bindings use Linux keycodes after any
upstream remapping (such as keyd), not desktop-layout-translated characters.

## Build and run

Install a stable Rust toolchain with Cargo, then:

```sh
cargo build --release
sudo ./target/release/fievel --list
sudo ./target/release/fievel
```

With no `--device`, the application prefers the accessible `keyd virtual keyboard`,
so keyd keeps ownership of the physical keyboard and all its remappings happen
first. If keyd's output is absent, it selects the sole accessible physical
keyboard. Multiple keyd outputs, or zero/multiple physical keyboards without
keyd, require an explicit choice. Other virtual devices are not automatically
selected, and this application's own outputs are always rejected.

To override automatic selection, use `--device` with an `/dev/input/eventN` path
from `--list`, or a physical keyboard's stable `/dev/input/by-id/...-event-kbd`
symlink. Set up the keyd exclusions below before running alongside keyd.

```sh
sudo ./target/release/fievel --device /dev/input/eventN --speed 800 --scroll-speed 8
```

`--speed` overrides the configured normal speed in relative input units per
second, not guaranteed screen pixels. `--scroll-speed` overrides the configured
scroll rate in notches per second. Slow/fast speeds remain as configured. The update interval
is 4 ms; fractional motion is retained between updates. A scheduling stall or
suspend is capped at 50 ms of movement to avoid large catch-up jumps.

Use **F3+Escape**, Ctrl+C, or SIGTERM to stop. Ctrl+Z also exits rather than
suspending with the keyboard still grabbed. On a normal exit, handled signal, or input
read error, held virtual keys/buttons are released and the keyboard is ungrabbed.
Unplugging the keyboard exits with an error; restart after reconnecting it.
This program does not install or start a service automatically.

It needs read access to the selected input node and write access to `/dev/uinput`.
Running with `sudo` is the simplest initial setup. If `/dev/uinput` does not exist,
load it with `sudo modprobe uinput`. For unprivileged operation, use narrowly
scoped device permissions; do not make all input devices world-readable/writable.
Input access can read passwords, and uinput access can inject system-wide input.

## Configuration

On startup, the application reads
`~/.config/fievel/fievel.config` (`~` is the current process's `$HOME`).
The format is TOML. A complete example is included as `fievel.config`
in this project:

```toml
[speeds]
normal = 800
slow = 200
fast = 1600
scroll = 8

[keys]
free_mouse = "f3"
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
exit = "esc"
```

All settings are optional; omitted values keep their defaults. If the default
file is missing, a message is printed and built-in defaults are used. An invalid
or unreadable file stops startup before any input device is grabbed.
Unknown settings, unknown key names, duplicate bindings, and nonpositive,
nonfinite, or greater-than-100000 speeds are rejected.

Key names are case-insensitive Linux keycodes: `h`, `space`, `f4`, `leftshift`,
`KEY_LEFTCTRL`, etc. `,`/`comma`, `.`/`dot`, `escape`/`esc`, and `return`/`enter`
are accepted. Each action must use a distinct key. Chords still belong in keyd:
for your D+F -> F3 remap, leave `free_mouse = "f3"`.
The exit action is held together with the configured Free Mouse Mode key.

Restart the application to apply edits. Inspect the effective configuration
without grabbing a keyboard:

```sh
./target/release/fievel --check-config
```

Use `--config PATH` to select a different file (it must exist). When running with
sudo, pass your own config explicitly so it does not look in root's home:

```sh
sudo ./target/release/fievel --config "$HOME/.config/fievel/fievel.config"
```

## Disable desktop pointer acceleration

The application never adds acceleration, but a compositor/X server can still
accelerate relative uinput events. **Set a flat acceleration profile for
`fievel pointer` to get constant on-screen speed.** Set a fixed sensitivity
to taste. Display scaling can also change the input-unit-to-pixel ratio.

For Sway, add this to your Sway config and reload:

```text
input "4617:62210:fievel_pointer" {
    accel_profile flat
    pointer_accel 0
    natural_scroll disabled
}
```

Confirm the identifier with `swaymsg -t get_inputs` while the app is running.

For Hyprland, confirm the device name with `hyprctl devices`, then configure:

```text
device {
    name = fievel-pointer
    accel_profile = flat
    sensitivity = 0
    natural_scroll = false
}
```

For other desktops, choose the flat/no-acceleration mouse profile in their
settings. On X11 with the libinput driver, while the application is running:

```sh
xinput set-prop 'fievel pointer' 'libinput Accel Profile Enabled' 0 1
xinput set-prop 'fievel pointer' 'libinput Accel Speed' 0
xinput set-prop 'fievel pointer' 'libinput Natural Scrolling Enabled' 0
```

Disable natural scrolling for this device if you want the documented scroll
directions; the desktop can otherwise reverse them.

## Using alongside keyd

Only one program can exclusively grab an input device. By default, fievel
selects **keyd's virtual keyboard output**, leaving the physical keyboard owned
by keyd. The pipeline is physical keyboard -> keyd -> fievel -> desktop.
You can also explicitly select keyd's output using `--device`.

For example, if keyd maps the D+F chord to F3, holding that chord activates
Free Mouse Mode, and keyd's F3 release exits it. Hifam-mouse sees the remapped F3,
not the original D and F. The mapping must hold F3 down, not emit only a tap.
The other mouse controls likewise operate on keycodes emitted by keyd.

Prevent keyd from processing fievel's outputs again: for each keyd
configuration whose `[ids]` section matches all devices (`*`), add:

```ini
[ids]
*
-1209:f301
-1209:f302
```

Merge these exclusions into the existing section; do not replace your other
configuration. Reload keyd before starting fievel. The keyboard has ID
`1209:f301`; the pointer has ID `1209:f302`. These are application-local virtual
identifiers, not claims of registered USB product IDs.

Alternatively, stop the conflicting remapper and select the physical keyboard
directly. Do not run multiple mouse remappers against the same keyboard.

## Development

```sh
cargo test
```

The state-machine tests do not require root, input devices, or a desktop.
An optional kernel round-trip test creates temporary virtual devices and grabs
their outputs before emitting anything, isolating its events from the desktop:

```sh
cargo test uinput_round_trip -- --ignored
```

This requires access to `/dev/uinput` and the created `/dev/input/event*` nodes.
The keyboard output forwards key events, not keyboard LED feedback or non-key
features of combined devices (for example, a touchpad on the same event node).
