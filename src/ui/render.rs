use crate::SpeechVizState;
use crate::ui::painter::OverlayPainter;
use crate::ui::state::OverlayState;
use bytemuck::{Pod, Zeroable};
use std::mem;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

const VERTICES: [Vertex; 6] = [
    Vertex {
        pos: [-1.0, -1.0],
        uv: [0.0, 1.0],
    },
    Vertex {
        pos: [1.0, -1.0],
        uv: [1.0, 1.0],
    },
    Vertex {
        pos: [1.0, 1.0],
        uv: [1.0, 0.0],
    },
    Vertex {
        pos: [-1.0, -1.0],
        uv: [0.0, 1.0],
    },
    Vertex {
        pos: [1.0, 1.0],
        uv: [1.0, 0.0],
    },
    Vertex {
        pos: [-1.0, 1.0],
        uv: [0.0, 0.0],
    },
];

pub struct OverlayRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    painter: OverlayPainter,
}

impl OverlayRenderer {
    pub fn new(window: &Arc<Window>, speech_viz: Arc<SpeechVizState>) -> Result<Self, String> {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor();

        let instance = wgpu::Instance::default();
        let surface_target = unsafe { wgpu::SurfaceTargetUnsafe::from_window(window.as_ref()) }
            .map_err(|e| format!("Failed to get surface target: {e}"))?;
        let surface = unsafe { instance.create_surface_unsafe(surface_target) }
            .map_err(|e| format!("Failed to create surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("Failed to request adapter: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("overlay-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::default(),
        }))
        .map_err(|e| format!("Failed to request device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| *f == wgpu::TextureFormat::Bgra8UnormSrgb)
            .unwrap_or(caps.formats[0]);

        let present_mode = caps
            .present_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::PresentMode::Mailbox)
            .unwrap_or(wgpu::PresentMode::Fifo);

        let alpha_mode = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
        {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            return Err(format!(
                "Transparent overlay unsupported: required PreMultiplied or PostMultiplied alpha mode, available: {:?}",
                caps.alpha_modes
            ));
        };
        let post_multiplied_surface = alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied;

        eprintln!(
            "Overlay surface config: format={:?}, alpha_mode={:?}, supported_alpha_modes={:?}",
            format, alpha_mode, caps.alpha_modes
        );

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: physical_size.width.max(1),
            height: physical_size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader_src = format!(
            r#"
@group(0) @binding(0)
var tex: texture_2d<f32>;
@group(0) @binding(1)
var samp: sampler;

struct VsOut {{
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
}};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {{
  var out: VsOut;
  out.position = vec4<f32>(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {{
  let c = textureSample(tex, samp, in.uv);
  if ({}) {{
    if (c.a <= 0.0) {{
      return c;
    }}
    let straight_rgb = clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(straight_rgb, c.a);
  }}
  return c;
}}
"#,
            if post_multiplied_surface {
                "true"
            } else {
                "false"
            }
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: mem::size_of::<[f32; 2]>() as u64,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay-vertex-buffer"),
            contents: bytemuck::cast_slice(&VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (texture, texture_view, sampler, bind_group) =
            Self::create_texture_resources(&device, &bind_group_layout, physical_size)?;

        let painter = OverlayPainter::new(physical_size, scale_factor, speech_viz);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            texture,
            texture_view,
            sampler,
            bind_group_layout,
            bind_group,
            painter,
        })
    }

    fn create_texture_resources(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
        size: PhysicalSize<u32>,
    ) -> Result<
        (
            wgpu::Texture,
            wgpu::TextureView,
            wgpu::Sampler,
            wgpu::BindGroup,
        ),
        String,
    > {
        let width = size.width.max(1);
        let height = size.height.max(1);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("overlay-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay-bind-group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Ok((texture, texture_view, sampler, bind_group))
    }

    pub fn resize(&mut self, physical_size: PhysicalSize<u32>, scale_factor: f64) {
        if physical_size.width == 0 || physical_size.height == 0 {
            return;
        }

        self.painter.resize(physical_size, scale_factor);

        self.config.width = physical_size.width;
        self.config.height = physical_size.height;
        self.surface.configure(&self.device, &self.config);

        let (texture, texture_view, sampler, bind_group) =
            Self::create_texture_resources(&self.device, &self.bind_group_layout, physical_size)
                .expect("Failed to recreate texture resources");
        self.texture = texture;
        self.texture_view = texture_view;
        self.sampler = sampler;
        self.bind_group = bind_group;
    }

    pub fn draw_frame(
        &mut self,
        state: OverlayState,
        now: std::time::Instant,
        _started_at: std::time::Instant,
    ) {
        self.painter.draw_frame(state, now, _started_at);
        let size = self.painter.physical_size();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.painter.data_u8(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width.max(1)),
                rows_per_image: Some(size.height.max(1)),
            },
            wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn render(&mut self) -> Result<(), String> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Outdated) => return Ok(()),
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err("Surface out of memory".to_string());
            }
            Err(wgpu::SurfaceError::Other) => return Ok(()),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overlay-encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
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
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.draw(0..VERTICES.len() as u32, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}
