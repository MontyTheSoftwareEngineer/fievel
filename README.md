# fievel

A Linux Rust application for keyboard-driven mouse control and configurable
**home-row mods**. Keep your hands on the home row with modifier chords,
navigation layers, and mouse movement, clicking, and scrolling.
The optional [home-row profile](homerow.config) maps A+S to Super, S+D to Ctrl,
and D+F to Free Mouse Mode, with timed Caps Lock navigation.

Fievel reads all suitable keyboards through evdev by default and creates two uinput devices:
`fievel keyboard` for typing and keyboard mappings, and `fievel pointer` for
mouse events. Input handling works below X11/Wayland and does not require
programmable keyboard firmware or a separate remapping tool.
No keystrokes are saved or sent anywhere. The opt-in caster overlay holds only
recent Free Mouse Mode control presses temporarily in memory.

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

To customize quickly, download `fievel.config` from the source at the **same
release tag** (the binary archive does not include it). From the directory
containing the downloaded config and extracted binary, copy it to the default
location, edit it, and rerun the binary:

```sh
mkdir -p ~/.config/fievel
cp -i fievel.config ~/.config/fievel/fievel.config
nano ~/.config/fievel/fievel.config
./fievel
```

Use your preferred editor instead of `nano` if needed. Stop any running fievel
with Ctrl+C before restarting; config edits take effect on startup. Use the
matching release's config to avoid settings unsupported by an older binary.
See [Configuration](#configuration) for all settings.

For home-row mods, download [`homerow.config`](homerow.config) from the same
release tag and run `./fievel --config homerow.config` instead. See
[Home-row mods and navigation layers](#home-row-mods-and-navigation-layers)
for the bindings and timing behavior.

## Controls

https://github.com/user-attachments/assets/cdcef352-618e-4b6a-afa2-5eb3f40cdee6

By default, hold **F3** to enter **Free Mouse Mode** and release F3 to leave it.
With `mode = "toggle"`, press F3 once to enter and again to leave.
By default, a faint **fievel** rectangle remains at the bottom-left while Free
Mouse Mode is active (Wayland/Hyprland/Sway; see [Mode indicator](#mode-indicator)).
Press **Super+Space** to enter hint mode for a left click or **Super+I** for a
right click: fievel captures the visible wlroots output, draws labeled target
boxes, and clicks as soon as you type a full label.
Press the same activation shortcut again to cancel without clicking. Press the
other shortcut to switch the click button without losing your selection.
Hold **Left Ctrl** for a more opaque background behind hint text; release to hide it.

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
Mode, A and S type normally unless used in a configured modifier chord.
All bindings and speeds can be changed in the config.

Home/End shortcuts are enabled by default and only work in Free Mouse Mode.
Hold the configured scroll up+down keys together to send Home, or scroll
left+right to send End, in either press order. Each chord sends one key tap, not autorepeats; release and
repress either member to fire again. A jump immediately clears scrolling momentum
on both axes and suppresses scrolling from all currently held scroll keys until
they are released, so releasing the chord cannot scroll away from the page edge.
Press a scroll key again after releasing it to resume normal scrolling.
Set `home_end_enabled = false` to disable
these shortcuts (opposite directions still cancel scrolling). Movement keys do
not trigger these shortcuts. These are ordinary
Home/End key events sent to the focused application; in text fields they may
move the caret rather than scroll the page, and held modifiers still apply.

Mouse-control keys already held when F3 is pressed transfer to mouse control.
Keys used in mouse mode stay suppressed until released, so leaving the mode
does not accidentally type a held key. Bindings use Linux keycodes, not
desktop-layout-translated characters. Native keyboard mappings are applied
before mouse handling; while mouse mode is active, new mouse-control presses
take precedence over those mappings.

## Build and run

With [Rust and Cargo](https://rustup.rs/) installed, build from the project directory:

```sh
cargo build --release
```

Set up [device permissions](#device-permissions) once, then run the binary:

```sh
./target/release/fievel
```

Or copy it to a directory on your `PATH`:

```sh
sudo install -m 755 target/release/fievel /usr/local/bin/fievel
fievel
```

Use **Ctrl+C** to stop and release the keyboards. Escape passes through normally
unless configured as the enabled [caster hotkey](#caster-mode-keycast) in Free Mouse Mode;
Ctrl+Z exits rather than suspending. In automatic mode, newly connected or
reconnected keyboards are discovered about every two seconds in the background,
without pausing typing or mouse updates. Unplugging one keyboard releases its
held keys without interrupting the others; fievel waits for reconnection even
if all keyboards disconnect. With `--device PATH`, fievel exits when that
keyboard disconnects and must be restarted.
No service is installed or started automatically.

### Movement odometer

While fievel is running, it automatically keeps separate totals since reset for
movement emitted by fievel and movement from physical mice/trackpads, including
when Free Mouse Mode is inactive. In another terminal, run:

```sh
fievel --odometer
```

This prints both totals and exits without opening input devices, grabbing a
keyboard, loading configuration, or disturbing the running instance. It also
works when fievel is stopped. Totals survive restarts and are saved atomically
once per second and on clean shutdown; an abrupt termination may lose the last
second. Before the first run, the command reports zero.

Reset both totals from any terminal:

```sh
fievel --reset-odometer
```

This permanently clears both counters. It works with fievel stopped or running;
a running instance saves and acknowledges the reset without releasing the
keyboard or interrupting mouse control, then continues counting from zero.
Restart an older running binary with the updated version before using live reset.

Distances are displayed in **estimated miles**, using a reference scale of
**96 raw input units per inch** (6,082,560 units per mile). This is a display
conversion, not measured physical travel or on-screen cursor distance: fievel's
relative motion, mouse hardware counts, and trackpad coordinates have different
scales. The default is a conventional reference, not detected device DPI.
You can choose a different reference scale when reporting:

```sh
fievel --odometer --odometer-units-per-inch 800
```

The same scale applies to both totals, so it cannot calibrate a mixture of devices.
Raw totals remain stored without rounding; existing history is automatically
displayed in miles and changing the report scale does not alter stored data.
Your odometer notification shortcut also displays miles with the updated binary.
Each input frame
adds `sqrt(dx*dx + dy*dy)`. Clicks and wheel events are excluded; trackpads count
single-contact motion, not finger lifts/repositioning or multi-finger gestures.
Touchscreens and virtual devices (including fievel's own pointer) are excluded.
These are input-level measurements, not the desktop's final gesture/palm
classification or accelerated/clipped cursor motion.

Physical devices are read **without grabbing them**, and newly connected devices
are discovered within about two seconds. Fievel needs read access to their
`/dev/input/event*` nodes in addition to the keyboard; the `input` group setup
below generally provides this. Inaccessible devices are reported on stderr and
cannot contribute to the physical total. Devices exclusively grabbed by another
program cannot be observed. Nothing is counted while fievel is stopped.

Totals are stored in `$XDG_STATE_HOME/fievel/odometer.toml`, or
`~/.local/state/fievel/odometer.toml` when `XDG_STATE_HOME` is unset or not absolute.
Run the main instance and the reporting command as the same user with the same
state directory. Only one tracking instance can write to that directory; any
number of `--odometer` readers can run alongside it. Only aggregate distances
are saved, not event histories or coordinates.

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

Only grant these permissions to trusted users: the `input` group can read other
input devices, including password keystrokes, and `fievel` group members can
inject system-wide input. Do not make input devices world-readable/writable.

### Choosing a keyboard

By default, Fievel uses **all accessible, suitable keyboards connected at startup**,
including built-in, USB, and Bluetooth keyboards. No `--device` option is needed
to use several keyboards. They share one set of mappings and mouse-mode state:
you can hold a modifier or mouse-mode key on one and use keys on another.
Releasing a key on one keyboard does not release it while another still holds it.

Automatic selection excludes Fievel's own outputs, unknown virtual inputs,
non-keyboards, and combined keyboard/pointer event nodes whose pointer events
would otherwise be swallowed. Separate keyboard nodes on those devices still work.
The known `keyd virtual keyboard` output is included alongside available physical
keyboards; inputs already grabbed by keyd or another program are skipped with a
diagnostic rather than preventing the other keyboards from working.
Inaccessible inputs are also reported and skipped. Startup fails if no suitable
keyboard can be grabbed.

To restrict Fievel to **one keyboard**, list devices and select it explicitly:

```sh
./target/release/fievel --list
./target/release/fievel --device /dev/input/eventN
```

A physical keyboard's stable `/dev/input/by-id/...-event-kbd` path also works.
Virtual inputs can be selected with `--device`; fievel's own outputs are
always rejected. Only one program can exclusively grab an input device, so
stop any tool that already owns an explicitly selected keyboard before starting
Fievel. Unlike automatic selection, `--device` fails if that device cannot be
opened or grabbed; it never falls back to other keyboards.

Optional overrides: `--speed 800` sets normal pointer speed in relative input
units per second (not guaranteed screen pixels), and `--scroll-speed 8` sets
normal scrolling in notches per second. Slow/fast speeds remain as configured.

## Configuration

On startup, the application reads
`~/.config/fievel/fievel.config` (`~` is the current process's `$HOME`).
The format is TOML. [`fievel.config`](fievel.config) documents the default
mouse controls; [`homerow.config`](homerow.config) adds optional home-row mods
and navigation layers while retaining the default mouse speeds and controls.
The default settings are:

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
timeout = 3 # Seconds of inactivity

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
```

All settings are optional; omitted values keep their defaults. If the default
file is missing, a message is printed and built-in defaults are used. An invalid
or unreadable file stops startup before any input device is grabbed.
Unknown settings, unknown key names, duplicate control bindings, repeated keys
within an activation chord, and nonpositive,
nonfinite, or greater-than-100000 speeds are rejected.
Easing values must be finite numbers between 0 and 1. Hint labels must use 2-26
unique lowercase ASCII letters, and hint size limits must stay positive with
max >= min.
The old `keys.exit` setting has been removed; delete it from existing configs.

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

For `keys.free_mouse`, partial chords pass through normally before completion. When a chord completes,
previously forwarded members are released on the virtual keyboard (including
modifiers); the completing press is consumed. Chord members stay suppressed and
cannot also perform mouse actions until released. Already-forwarded partial
keypresses cannot be undone, so press modifiers first for shortcuts like Alt+Space.
Chord members may overlap controls: with `"leftalt + space"`, activation does not
click. In hold mode, bind `left_click` to another key (for example `"u"`) since
Space must remain held to keep the mode active. In toggle mode, release the chord
and then use Space normally for clicking.

For buffered letter chords that must not type a partial key, use the native
`free_mouse` action described below instead of `keys.free_mouse`.

Hint mode is optional, like the indicator: outside a wlroots Wayland session it
prints a warning and immediately returns to normal keyboard passthrough. While
hints are visible, letter keys matching `label_symbols` extend the current
selection, Backspace deletes one character (or cancels when empty), Enter cancels
by default, and Escape always cancels. Any matching full label immediately warps
the pointer to that region's center and emits the configured left or right click.
`border_color` and `fill_color` control the target box, while `label_color` and
`label_highlight_color` control the untyped and already-typed label text.
Hold **Left Ctrl** while hints are visible for a more opaque grey fill
inside the boxes, behind the text. Release it to restore the normal fill.
The background is hidden unless this key is held. Set
`hints.readability_color` (`#RRGGBB` or `#RRGGBBAA`, including opacity) and
`hints.keys.toggle_background` to customize the color and hold key
(for example, `"rightctrl"`). The existing setting name is retained for config
compatibility; its behavior is now hold-to-show, not toggle. Autorepeat has no effect.
The background preserves typed letters and filtering, and cannot use a hint-label
letter, Escape, Backspace, or the configured cancel key.
Hint activation shortcuts are recognized after Fievel's remapping, so a
home-row chord mapped to Super works with Space/I just like physical Super.
While hints are active, re-press the same shortcut to cancel without clicking,
or press the other shortcut to switch between left and right click. Switching
preserves labels, typed characters, and the readability background.
The completing key is consumed rather than forwarded to desktop shortcuts.
No compositor keybinding is needed; remove old Super+Space/Super+I bindings
that launch wl-kbptr to avoid opening its separate overlay when Fievel is not
handling input. While hints are active, remappings needed for the activation
shortcuts or readability hold key are applied. For example, holding S+D mapped
to Ctrl shows the grey background; releasing either key hides it. If another
Ctrl-producing key or chord is still held, the background stays visible until
the last one is released. Ordinary navigation and mouse remappings do not replace
hint letters. Partial shortcut chords use `remap.chord_timeout`, so a letter
shared with a shortcut may wait briefly before appearing. Escape and the cancel
key cancel immediately without replaying pending labels.
`min_width`/`max_width` and `min_height`/`max_height` filter detected regions in
logical clickable-element units, matching wl-kbptr-style defaults.
Detection runs natively in Rust on an in-memory screencopy: connected edges
find unboxed text, links and open icons as well as outlined or filled controls.
Nearby character strokes are grouped, and redundant nested targets are filtered.
It is a visual heuristic, not application accessibility data: some non-clickable
text may be labeled and low-contrast or tightly packed targets can still be missed.
Captures are oriented and resampled to the compositor-configured logical output
size; detection, overlay drawing and output-local clicks share those coordinates,
including fractional scaling and rotated/flipped outputs. No screenshots are saved
and no OpenCV runtime is required.

### Home-row mods and navigation layers

Fievel provides chord-based home-row mods: press neighboring letter keys
together to hold a modifier, and use those letters normally when no chord
completes. It also supports timed keys and named navigation layers.
These mappings are optional; the default configuration remains mouse-only.
The ready-to-use [`homerow.config`](homerow.config) in the project root
contains:

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

Add these tables to your configuration, or use the example with `--config`.
Binding names are single Linux keys or `+`-separated chords, in either press
order. Actions are key names (`escape`, `f3`, `left`, etc.), held modifiers
(`ctrl`, `super`), `layer(name)`, `free_mouse`, or
`timeout(short_action, milliseconds, held_action)`. `layer(control)` and
`layer(meta)` are aliases for holding left Ctrl and left Super.
Actions are not recursively remapped.
Undefined layers, duplicate normalized triggers, nested timeouts, nonpositive
timeouts, and ambiguous strict chord subsets (such as A+S alongside A+S+D)
are rejected. Layer names and the `layer(...)`/`timeout(...)` syntax are
case-sensitive; key names are case-insensitive.

Chord members are buffered for up to `chord_timeout` milliseconds (default 25).
A completed chord consumes its members, so letters do not leak into the
application. If no chord completes, pending keys are replayed in press order
through their single-key mappings. Only potential chord members incur this
delay. Shared-member chords such as A+S and S+D are supported; the first
completed chord wins. Releasing any member releases the chord action; its
remaining members stay reserved until released and must be pressed again for
another chord. Holding a modifier chord holds the modifier, not just a tap.

The Caps Lock binding provides a timed Ctrl/navigation choice: releasing
before 175 ms taps Ctrl; pressing another key before that deadline chooses
Ctrl first, allowing a quick Caps+C to send Ctrl+C. Holding Caps without
another press for 175 ms chooses navigation instead. That choice lasts until
Caps is released, so a quick Ctrl shortcut does not turn into navigation
mid-shortcut. W+E is another way to hold the same navigation layer. Layers
fall through to the main mappings for keys they do not override, and a key's
chosen mapping is retained through its release even if the layer changes.

`free_mouse` follows the top-level `mode`: hold the chord in hold mode, or
press it again to switch off in toggle mode. Mouse controls take precedence
over keyboard mappings while mouse mode is active, so H/J/K/L move the pointer
even with navigation held and I clicks rather than waiting for I+O. Activation
members remain reserved until release. With this profile, hold D+F to control
the mouse, then hold S for fast movement or A for slow movement; neither
speed key is occupied by the activation chord. The `keys.free_mouse` binding
(F3 by default) remains available too.

From the project directory, build Fievel and inspect the home-row profile:

```bash
cargo build --release
./target/release/fievel --config homerow.config --check-config
./target/release/fievel --list
```

Stop any running Fievel instance before changing profiles, then run:

```bash
./target/release/fievel --config homerow.config
```

Add `--device /dev/input/by-id/YOUR-KEYBOARD-event-kbd` if needed, replacing
the placeholder with your physical keyboard's path. To make the profile your
default, copy or merge `homerow.config` into `~/.config/fievel/fievel.config`.
Keep any pointer tuning or other settings you already customized.

### Pointer tuning

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
Bindings are Linux keycode labels, not desktop-layout-translated text.
The displayed keys are **white** at normal speed, **green** in fast mode, and
**yellow** in slow mode. The whole history updates as you press or release your
configured speed keys; slow takes priority when both are held.

`max_keys` keeps the newest **8** presses by default (allowed: 1-32), removing
the oldest when full. The whole history clears after **3 seconds** without a
new control press; `timeout` accepts seconds greater than zero and at most 60,
including fractions. Long histories wrap. Toggling casting off or leaving Free
Mouse Mode immediately clears the display. The casting toggle is remembered
between Free Mouse Mode sessions, including when entering Hint Mode, until you
toggle it off. It starts off each time fievel launches.

The hotkey is a configurable single key and, when enabled, must differ from
every mouse control and activation-chord member. It is consumed only in Free
Mouse Mode and takes precedence over native remappings there; outside the mode
it behaves normally. A hotkey held while leaving the mode remains suppressed
until released. `enabled = false` (the default) disables the feature completely.

The overlay is click-through, never takes focus, and uses the same Wayland
layer-shell requirements and compositor-selected monitor behavior as the
[mode indicator](#mode-indicator). It works independently of `notify`, so you can
use `notify = false` with keycast enabled. Display failures warn without
interrupting keyboard or mouse control. No history is written to disk.

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
top-level `notify = false` and leave `keycast.enabled = false` for headless
operation: no display connection or worker is created. `--list` and
`--check-config` never initialize either overlay.

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
