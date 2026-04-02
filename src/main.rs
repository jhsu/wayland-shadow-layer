/// wayland-shadow-overlay — main.rs
///
/// Renders a full-screen, click-through "golden-hour light through sheer
/// curtains" overlay on any Wayland compositor that implements
/// wlr-layer-shell (niri, Sway, Hyprland, river, ...).
use std::{
    process,
    ptr::NonNull,
    thread,
    time::{Duration, Instant},
};

use bytemuck::{Pod, Zeroable};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_region, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct Globals {
    resolution: vec2f,
    time: f32,
    premultiply_alpha: f32,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var out: VertexOut;
    var position = vec2f(-1.0, -1.0);

    if vertex_index == 1u {
        position = vec2f(3.0, -1.0);
    } else if vertex_index == 2u {
        position = vec2f(-1.0, 3.0);
    }

    out.position = vec4f(position, 0.0, 1.0);
    out.uv = position * 0.5 + vec2f(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    let frag_coord = in.position.xy;
    let uv = frag_coord / globals.resolution.xy;
    var p = uv * 2.0 - 1.0;
    p.x *= globals.resolution.x / globals.resolution.y;
    let projected_y = -p.y;

    var window_uv: vec2f;
    window_uv.x = p.x;
    let sun_angle = 0.6;
    window_uv.y = projected_y + p.x * sun_angle;
    window_uv.x = window_uv.x * 0.8 + 0.2;
    window_uv.y = window_uv.y * 1.2 - 0.2;

    let distance_to_window = p.x + 2.0;
    let travel = clamp(distance_to_window / 3.2, 0.0, 1.0);
    let blur = mix(0.006, 0.22, travel);
    let bar_blur = blur * mix(1.1, 2.9, travel);
    let shade_softness = mix(0.08, 0.42, travel);

    // Let the projection open up slightly as it travels, so the far end feels
    // more diffused and spread across the wall.
    window_uv.x *= mix(0.96, 1.20, travel);
    window_uv.y *= mix(0.98, 1.28, travel);

    let bounds_x = smoothstep(1.3 + blur, 1.3 - blur, abs(window_uv.x));
    let bounds_y = smoothstep(1.5 + blur, 1.5 - blur, abs(window_uv.y));
    var mask = bounds_x * bounds_y;

    let vertical_bar = smoothstep(0.0, bar_blur, abs(window_uv.x - 0.2));
    let horizontal_bar = smoothstep(0.0, bar_blur, abs(window_uv.y + 0.1));
    let shade_phase = window_uv.y * 12.0 + globals.time * 1.15 + sin(globals.time * 0.7) * 0.8;
    let shade_mask = smoothstep(
        0.5 - shade_softness,
        0.5 + shade_softness,
        0.5 + 0.5 * sin(shade_phase),
    );

    mask *= vertical_bar * horizontal_bar;
    mask *= mix(0.45, 1.0, shade_mask);
    mask = clamp(mask, 0.0, 1.0);

    let shadow_color = vec3f(0.08, 0.06, 0.07);
    let shadow_alpha = 0.42;
    let sunlight_color = vec3f(1.0, 0.85, 0.6);
    let sunlight_alpha = 0.09;

    var final_color = mix(shadow_color, sunlight_color, mask);
    var final_alpha = mix(shadow_alpha, sunlight_alpha, mask);

    var dust_glow = 0.0;
    for (var i = 0; i < 3; i++) {
        let fi = f32(i);
        let scale = 25.0 + fi * 10.0;
        var dust_uv = p * scale;
        dust_uv.y += globals.time * (0.15 + fi * 0.05);
        dust_uv.x -= globals.time * 0.05;

        let cell_id = floor(dust_uv);
        let local_uv = fract(dust_uv) - 0.5;
        let rand_val = hash21(cell_id + fi * 111.0);

        if (rand_val > 0.96) {
            let offset = vec2f(hash21(cell_id * 1.3), hash21(cell_id * 1.7)) - 0.5;
            let dist = length(local_uv - offset);
            let glow = 0.003 / (dist * dist + 0.001);
            let twinkle = sin(globals.time * 3.0 + rand_val * 100.0) * 0.5 + 0.5;
            dust_glow += glow * twinkle;
        }
    }

    let dust_final = dust_glow * mask * vec3f(1.0, 0.8, 0.5);
    final_color += dust_final;
    final_alpha = clamp(final_alpha + dust_glow * mask * 0.02, 0.0, 0.48);

    let rgb = select(final_color, final_color * final_alpha, globals.premultiply_alpha > 0.5);
    return vec4f(rgb, final_alpha);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    resolution: [f32; 2],
    time: f32,
    premultiply_alpha: f32,
}

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    premultiply_alpha: bool,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
}

impl GpuState {
    fn new(conn: &Connection, layer_surface: &LayerSurface) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(conn.backend().display_ptr() as *mut _)
                .expect("Wayland display pointer is null"),
        ));
        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(layer_surface.wl_surface().id().as_ptr() as *mut _)
                .expect("Wayland surface pointer is null"),
        ));

        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle,
                    raw_window_handle,
                })
                .expect("failed to create wgpu Wayland surface")
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("failed to find compatible GPU adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None))
            .expect("failed to request GPU device");

        let capabilities = surface.get_capabilities(&adapter);
        let surface_format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| {
                matches!(
                    mode,
                    wgpu::CompositeAlphaMode::PreMultiplied
                        | wgpu::CompositeAlphaMode::PostMultiplied
                )
            })
            .unwrap_or(capabilities.alpha_modes[0]);
        let present_mode = capabilities
            .present_modes
            .iter()
            .copied()
            .find(|mode| matches!(mode, wgpu::PresentMode::Mailbox))
            .unwrap_or(wgpu::PresentMode::Fifo);

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay globals"),
            contents: bytemuck::bytes_of(&GlobalsUniform {
                resolution: [1.0, 1.0],
                time: 0.0,
                premultiply_alpha: matches!(alpha_mode, wgpu::CompositeAlphaMode::PreMultiplied)
                    as u32 as f32,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            surface,
            device,
            queue,
            surface_config: wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: 1,
                height: 1,
                present_mode,
                alpha_mode,
                view_formats: vec![surface_format],
                desired_maximum_frame_latency: 2,
            },
            premultiply_alpha: matches!(alpha_mode, wgpu::CompositeAlphaMode::PreMultiplied),
            uniform_buffer,
            uniform_bind_group,
            render_pipeline,
        }
    }

    fn configure_surface(&mut self, width: u32, height: u32) {
        self.surface_config.width = width.max(1);
        self.surface_config.height = height.max(1);
        self.surface.configure(&self.device, &self.surface_config);
    }
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    gpu: GpuState,
    layer_surface: LayerSurface,
    width: u32,
    height: u32,
    animation_started_at: Instant,
    should_exit: bool,
}

impl App {
    fn render_frame(&mut self, _qh: &QueueHandle<Self>) {
        if self.width == 0 || self.height == 0 {
            return;
        }

        let surface_texture = match self.gpu.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.gpu.configure_surface(self.width, self.height);
                return;
            }
            Err(wgpu::SurfaceError::Timeout) => return,
            Err(wgpu::SurfaceError::OutOfMemory) => {
                eprintln!("wgpu surface ran out of memory");
                self.should_exit = true;
                return;
            }
        };

        let globals = GlobalsUniform {
            resolution: [self.width as f32, self.height as f32],
            time: self.animation_started_at.elapsed().as_secs_f32(),
            premultiply_alpha: self.gpu.premultiply_alpha as u32 as f32,
        };
        self.gpu
            .queue
            .write_buffer(&self.gpu.uniform_buffer, 0, bytemuck::bytes_of(&globals));

        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overlay encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.gpu.render_pipeline);
            render_pass.set_bind_group(0, &self.gpu.uniform_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.gpu.queue.submit(Some(encoder.finish()));
        surface_texture.present();
    }

    fn update_click_through_region(&self, qh: &QueueHandle<Self>) {
        let compositor = self.compositor_state.wl_compositor();
        let empty_region: wl_region::WlRegion = compositor.create_region(qh, ());
        self.layer_surface
            .wl_surface()
            .set_input_region(Some(&empty_region));
        empty_region.destroy();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = if configure.new_size.0 == 0 {
            1920
        } else {
            configure.new_size.0
        };
        self.height = if configure.new_size.1 == 0 {
            1080
        } else {
            configure.new_size.1
        };

        self.update_click_through_region(qh);
        self.gpu.configure_surface(self.width, self.height);
        self.render_frame(qh);

        eprintln!(
            "Overlay rendered: {}x{} pixels via wgpu",
            self.width, self.height
        );
    }

    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        eprintln!("Layer surface closed by compositor - exiting.");
        self.should_exit = true;
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

impl Dispatch<wl_region::WlRegion, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        unreachable!("wl_region emits no events")
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);

fn main() {
    let conn = Connection::connect_to_env().unwrap_or_else(|e| {
        eprintln!(
            "Cannot connect to the Wayland display: {e}\n\
             Make sure WAYLAND_DISPLAY is set and you are running in a Wayland session."
        );
        process::exit(1);
    });

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap_or_else(|e| {
        eprintln!("Failed to initialise global registry: {e}");
        process::exit(1);
    });
    let qh = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh).unwrap_or_else(|e| {
        eprintln!("wl_compositor not available: {e}");
        process::exit(1);
    });

    let layer_shell = LayerShell::bind(&globals, &qh).unwrap_or_else(|e| {
        eprintln!(
            "zwlr_layer_shell_v1 not available: {e}\n\
             Your compositor must support wlr-layer-shell \
             (niri, Sway, Hyprland, river, ...)."
        );
        process::exit(1);
    });

    let surface = compositor_state.create_surface(&qh);
    let layer_surface = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("wayland-shadow-overlay"),
        None,
    );

    layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_size(0, 0);
    layer_surface.set_exclusive_zone(0);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    let gpu = GpuState::new(&conn, &layer_surface);

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        gpu,
        layer_surface,
        width: 0,
        height: 0,
        animation_started_at: Instant::now(),
        should_exit: false,
    };

    eprintln!("Overlay initialised with wgpu - waiting for compositor configure...");

    loop {
        if app.width == 0 || app.height == 0 {
            event_queue.blocking_dispatch(&mut app).unwrap_or_else(|e| {
                eprintln!("Event queue error: {e}");
                process::exit(1);
            });
        } else {
            event_queue.dispatch_pending(&mut app).unwrap_or_else(|e| {
                eprintln!("Event queue error: {e}");
                process::exit(1);
            });

            if !app.should_exit {
                app.render_frame(&qh);
                event_queue.flush().unwrap_or_else(|e| {
                    eprintln!("Wayland flush error: {e}");
                    process::exit(1);
                });
                thread::sleep(Duration::from_millis(16));
            }
        }

        if app.should_exit {
            eprintln!("Exiting cleanly.");
            break;
        }
    }
}
