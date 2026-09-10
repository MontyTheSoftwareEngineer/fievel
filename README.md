# fievel

A Linux Rust application that grabs one keyboard through evdev and creates two
uinput devices: `fievel keyboard` for normal typing, and
`fievel pointer` for mouse events. Works below X11/Wayland and is visible
to tools such as `keyd monitor`. No keystrokes are recorded or sent anywhere.

https://github.com/user-attachments/assets/cdcef352-618e-4b6a-afa2-5eb3f40cdee6

By default, hold **F3** to enter **Free Mouse Mode** and release F3 to leave it.
With `mode = "toggle"`, press F3 once to enter and again to leave.
By default, a faint **fievel** rectangle remains at the bottom-left while Free
Mouse Mode is active (Wayland/Hyprland/Sway; see [Mode indicator](#mode-indicator)).

| Key while Free Mouse Mode is active | Action |
| --- | --- |
| H / J / K / L | Move left / down / up / right |
| Space | Left button down on press, up on release (supports dragging) |
| I | Right button down on press, up on release |
| N / M / , / . | Scroll left / down / up / right |
| S | Hold for fast movement and scrolling; release for normal speed |
| A | Hold for slow movement and scrolling; release for normal speed |

Movement and scrolling ease toward their configured speeds, independently of
keyboard repeat. Movement has normalized diagonal target speeds (not faster
than horizontal/vertical movement). Opposite directions cancel the target
velocity. Releasing direction keys gently slows movement/scrolling to a stop;
changing directions produces smooth turns. Scrolling uses high-resolution wheel
events (1/120 notch) to ease smoothly from rest without an initial full-notch jump.
Leaving Free Mouse Mode immediately stops movement/scrolling and releases both mouse buttons,
even if Space or I is still held. F3 itself never reaches applications.
Other keys work normally, including modifiers and Ctrl+C.
Speed changes also ease toward the new target. Slow takes priority if A
and S are both held; releasing A while S remains held returns to fast speed.
These modifiers affect both pointer movement and scrolling, with separately
configurable rates. Outside Free Mouse
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
normal scroll rate in notches per second. Slow/fast pointer and scroll speeds
remain as configured. The update interval
is 4 ms; fractional motion is retained between updates. A scheduling stall or
suspend is capped at 50 ms of movement to avoid large catch-up jumps.

Use Ctrl+C or SIGTERM to stop. Escape passes through normally and does not quit
the application or leave Free Mouse Mode. Ctrl+Z also exits rather than
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
mode = "hold" # "hold" (default) or "toggle"
notify = true # Persistent, faint Free Mouse Mode indicator

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
```

All settings are optional; omitted values keep their defaults. If the default
file is missing, a message is printed and built-in defaults are used. An invalid
or unreadable file stops startup before any input device is grabbed.
Unknown settings, unknown key names, duplicate control bindings, repeated keys
within an activation chord, and nonpositive,
nonfinite, or greater-than-100000 speeds are rejected.
Easing values must be finite numbers between 0 and 1.
The old `keys.exit` setting has been removed; delete it from existing configs.

Key names are case-insensitive Linux keycodes: `h`, `space`, `f4`, `leftshift`,
`KEY_LEFTCTRL`, etc. `,`/`comma`, `.`/`dot`, `escape`/`esc`, and `return`/`enter`
are accepted. Controls must use distinct single keys. `free_mouse` accepts either
a single distinct key or a `+`-separated chord such as `"leftalt + space"` or
`"leftctrl+leftalt+f3"`. All chord keys must be held together, in either press
order; extra held keys do not prevent activation. If keyd already remaps D+F
to F3, leave `free_mouse = "f3"`.
The top-level `mode` and `notify` settings must appear before any table
(`[speeds]`, `[easing]`, or `[keys]`).
`"hold"` activates mouse mode only while every activation key is down; releasing
any chord member leaves the mode. `"toggle"` switches it on/off each time the
whole chord becomes held; releases and keyboard autorepeat do not toggle it.
Release and repress at least one member to toggle again. Slow/fast and
mouse-button bindings still use hold behavior in either mode.

Partial chords pass through normally before completion. When a chord completes,
previously forwarded members are released on the virtual keyboard (including
modifiers); the completing press is consumed. Chord members stay suppressed and
cannot also perform mouse actions until released. Already-forwarded partial
keypresses cannot be undone, so press modifiers first for shortcuts like Alt+Space.
Chord members may overlap controls: with `"leftalt + space"`, activation does not
click. In hold mode, bind `left_click` to another key (for example `"u"`) since
Space must remain held to keep the mode active. In toggle mode, release the chord
and then use Space normally for clicking.

`speeds.scroll`, `speeds.scroll_slow`, and `speeds.scroll_fast` set normal, slow,
and fast scrolling in notches per second (defaults: 6, 1.5, and 24). Speed changes
update the target rate of held scrolling, with slow taking priority over fast.
With scroll easing disabled, a scroll press still sends one immediate notch,
followed by whole-notch repeats.

Default pointer speeds are 300 / 100 / 900 input units per second for normal /
slow / fast. These and the scroll defaults match the steady-state rates of a
Mouseless configuration with `base_move_speed = 5`, `move_speed_multiplier = 3`,
`base_wheel_speed = 0.1`, and `wheel_speed_multiplier = 4`: its movement loop
converts base speeds with a factor of 60 per second, multiplying for fast and
dividing for slow. Fievel keeps normalized diagonals; desktop scaling and
acceleration can still make the on-screen feel differ. Existing explicit speed
settings override these defaults.

`easing.movement` and `easing.scroll` control acceleration, deceleration after
direction-key release, and transitions between normal/slow/fast speeds.
Defaults are 0.2 and 0.3, matching the easing factors in that Mouseless config.
Each is the fraction of the gap to the target velocity closed per 1/60 second,
adjusted for elapsed time so it does not depend on keyboard repeat or tick rate.
Smaller positive values give gentler, longer transitions; values nearer 1 feel
sharper. Set either value to **0** to disable its easing (1 is also instantaneous).
Setting both to 0 restores the previous immediate start/stop behavior.

At the defaults, movement closes about 95% of the velocity gap in 224 ms and
scrolling in 140 ms. With easing enabled, fractional scroll deltas are emitted
through Linux high-resolution wheel events, with corresponding whole-notch events
for legacy consumers. Modern input stacks use the high-resolution stream instead
of adding both streams together. Actual visual smoothness depends on the
compositor and application; legacy consumers still scroll in whole notches.
Short taps produce a small eased scroll rather than a guaranteed full notch,
and a short scroll tail can continue after releasing a direction.
Mouse-button releases, leaving mouse mode, and
shutdown are always immediate, with no residual movement on reactivation.

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

## Mode indicator

`notify = true` (the default) enables a persistent graphical indicator, **not**
a desktop notification. While Free Mouse Mode is active, an 88×30 logical-pixel
rectangle labeled `fievel` appears 12 logical pixels from the bottom-left edge.
The background is approximately 8% opaque white; the small bitmap lettering is
35% opaque white. It is click-through, never requests keyboard focus, reserves
no desktop space, and uses the overlay layer so it can appear above fullscreen
windows. The compositor chooses the output on each activation (normally the
currently focused monitor); the indicator does not follow the pointer between
monitors while active. Desktop effects may alter its appearance.

It stays visible for the entire hold or toggled-on interval, with no timer.
Leaving the mode destroys the surface immediately via a worker wakeup (subject
to compositor scheduling). Normal shutdown,
handled signals, and input errors also remove it. Display work runs on a separate
thread, never in the input polling loop.

The indicator requires a Wayland compositor implementing `zwlr_layer_shell_v1`,
such as Hyprland or Sway, and access to the current user's Wayland socket through
`WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`. It is built into the binary: no notification
daemon, GUI toolkit, font installation, or external overlay helper is needed.
There is no X11 or non-layer-shell desktop fallback. Display startup/protocol
failures print a warning and disable the indicator for that run, **without
disabling keyboard/mouse control**; restart after fixing the display environment.
Unresponsive Wayland setup is limited to three seconds in the worker.

Prefer running as the logged-in desktop user with appropriate device permissions.
If using sudo, explicitly retain the display environment as well as selecting
your own configuration:

```sh
sudo --preserve-env=WAYLAND_DISPLAY,XDG_RUNTIME_DIR ./target/release/fievel \
  --config "$HOME/.config/fievel/fievel.config"
```

Socket access still depends on local permissions and sudo policy. Set the
top-level `notify = false` for headless operation: no display connection or worker
is created. `--list` and `--check-config` never initialize the indicator either.

## Disable desktop pointer acceleration

Fievel's easing controls velocity over time; compositor/X server pointer
acceleration is separate and can further alter relative uinput events.
**Set a flat acceleration profile for `fievel pointer` to get predictable
on-screen speeds and easing.** Set a fixed sensitivity
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

For example, if keyd maps the D+F chord to F3, Fievel sees the remapped F3,
not the original D and F. With `mode = "hold"`, hold the chord to activate
Free Mouse Mode; keyd's F3 release exits it, so the mapping must hold F3 down.
With `mode = "toggle"`, each new chord press switches Free Mouse Mode on/off;
a mapping that emits an F3 tap also works.
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
An optional display-only smoke test briefly shows/hides the real indicator twice,
without opening or grabbing any input device:

```sh
cargo test wayland_indicator_lifecycle -- --ignored
```

An optional kernel round-trip test creates temporary virtual devices and grabs
their outputs before emitting anything, isolating its events from the desktop:

```sh
cargo test uinput_round_trip -- --ignored
```

This requires access to `/dev/uinput` and the created `/dev/input/event*` nodes.
The keyboard output forwards key events, not keyboard LED feedback or non-key
features of combined devices (for example, a touchpad on the same event node).
