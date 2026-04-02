# wayland-shadow-overlay

A full-screen, click-through Wayland overlay that renders a procedural
"golden-hour light through a window" effect on top of every
window on the desktop.

## Effect

The overlay composites three low-contrast layers so the desktop beneath remains
fully legible:

| Layer                | Description                                                                        |
| -------------------- | ---------------------------------------------------------------------------------- |
| Warm slanted beam    | A broad pale-amber projection falls across the screen at a gentle angle            |
| Window-frame shadows | Multiple muntins and frame shadows break the beam into calm geometric panes        |
| Depth blur           | Shadows start crisper near the source and soften as they travel across the surface |
| Slow drift           | Barely perceptible motion keeps the light from feeling frozen                      |

The result is a calm late-afternoon atmosphere: a soft window projection,
warm light, muted frame shadows, and barely perceptible motion.

## Requirements

| Requirement     | Details                                                                     |
| --------------- | --------------------------------------------------------------------------- |
| Compositor      | wlr-layer-shell support — **niri**, **Sway**, **Hyprland**, **river**, etc. |
| Wayland session | `WAYLAND_DISPLAY` must be set in the environment                            |
| Rust toolchain  | `rustc` + `cargo` ≥ 1.75 (edition 2021)                                     |
| GPU stack       | A working `wgpu` backend for your compositor and graphics driver            |

Note: GNOME (Mutter) and KDE Plasma (KWin) do **not** implement wlr-layer-shell.

## Building

```sh
# From the project root
cargo build --release
```

The compiled binary is placed at `target/release/wayland-shadow-overlay`.

## Running

```sh
# Run directly
./target/release/wayland-shadow-overlay

# Or via cargo
cargo run --release
```

The overlay starts immediately, covers all outputs at `Layer::Overlay`, and
blocks on a Wayland event loop. Press `Ctrl-C` or send `SIGTERM` to exit
(no keyboard events are consumed by the overlay itself).

## Autostart

### niri (example — `config.kdl`)

```kdl
spawn-at-startup "wayland-shadow-overlay"
```

### Sway / i3 (`config`)

```
exec --no-startup-id wayland-shadow-overlay
```

### Hyprland (`hyprland.conf`)

```
exec-once = wayland-shadow-overlay
```

## Technical notes

| Property               | Value                                                   |
| ---------------------- | ------------------------------------------------------- |
| Protocol               | `zwlr_layer_shell_v1`                                   |
| Layer                  | `Overlay` (above all application windows)               |
| Anchor                 | All four edges — stretches to fill the entire output    |
| Exclusive zone         | `0` — does not push panels or docks                     |
| Keyboard interactivity | `None` — never steals focus                             |
| Input region           | Empty `wl_region` — all mouse/touch events fall through |
| Rendering              | GPU-backed `wgpu` fragment shader                       |
| Presentation           | `Mailbox` when available, otherwise `Fifo`              |

## Adjusting the effect

Edit the WGSL shader in `src/main.rs` (`SHADER` constant):

- **`beam_center` math** — controls the slant and placement of the warm light band
- **`beam_width`** — widens or narrows the glow falloff
- **`window_footprint` / `inner_light`** — control the projected window shape
- **`travel`, `frame_blur`, `crossbar_blur`** — control how shadows soften with distance
- **`center_mullion` / `crossbar_*`** — tune the frame shadows and pane count inside the light
- **`ambient_warmth` / `shadow_alpha`** — tune overall intensity while keeping contrast soft
- **`mix(...)` colors** — shift between pale amber highlights and muted cool shadows

Rebuild with `cargo build --release` after any changes.
