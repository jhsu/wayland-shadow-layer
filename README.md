# wayland-shadow-overlay

A full-screen, click-through Wayland overlay that renders a procedural
"golden-hour light through sheer curtains" effect on top of the desktop.

## Demo


https://github.com/user-attachments/assets/48017be0-22c2-4954-9d45-b7c2632f16a9


## Effect

The overlay composites several low-contrast layers so the desktop beneath
remains fully legible:

| Layer                | Description                                                                 |
| -------------------- | --------------------------------------------------------------------------- |
| Warm slanted beam    | A pale amber projection falls across the screen at a gentle angle            |
| Curtain bands        | Soft vertical and horizontal occlusion suggests light filtering through sheers |
| Foliage shadows      | Slow branch-like shadows drift across the lit area                           |
| Depth blur           | The projection softens and spreads as it travels across the surface          |
| Dust glints          | Sparse warm specks shimmer inside the lit portion                            |

The result is a calm late-afternoon atmosphere: diffused sunlight, soft curtain
filtering, faint foliage motion, and subtle dust suspended in the light.

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

The overlay starts immediately, covers the compositor space at `Layer::Overlay`,
then continuously redraws at roughly 60 FPS on the Wayland event loop. Press
`Ctrl-C` or send `SIGTERM` to exit (no keyboard events are consumed by the
overlay itself).

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

- **`sun_angle` and `window_uv` remap** — control the projection angle and placement
- **`travel`, `blur`, `bar_blur`** — control how the light spreads and softens with distance
- **`vertical_bar`, `horizontal_bar`, `shade_phase`** — shape the curtain-like filtering
- **`branch_sway` and `branch_curve_*`** — tune the drifting foliage shadow pattern
- **`shadow_color`, `sunlight_color`, `shadow_alpha`, `sunlight_alpha`** — set the overall contrast and warmth
- **`dust_glow` loop** — adjusts how often bright particles appear and how strongly they twinkle

Rebuild with `cargo build --release` after any changes.
