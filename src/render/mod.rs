pub(crate) mod placeholder;

use std::{error::Error, sync::Arc, time::Duration};

use winit::{event_loop::EventLoopProxy, window::Window};

use crate::{
    app::AppEvent,
    media::{
        frame::{Canvas, FrameView},
        playback::PageKey,
    },
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const ALPHA: wgpu::CompositeAlphaMode = wgpu::CompositeAlphaMode::PreMultiplied;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    sprite: Option<Sprite>,
    frame_rect: wgpu::Buffer,
    upload: Option<wgpu::SubmissionIndex>,
    canvas: Canvas,
    presented: bool,
}

struct Sprite {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    key: Option<PageKey>,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        frame: FrameView<'_>,
        enable_bc7: bool,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self, Box<dyn Error>> {
        let size = window.inner_size();
        let mut descriptor = wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..Default::default()
        };
        descriptor.backend_options.dx12.presentation_system =
            wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual;
        let instance = wgpu::Instance::new(&descriptor);
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        let capabilities = surface.get_capabilities(&adapter);
        eprintln!("Adapter: {info:?}\nSurface capabilities: {capabilities:?}");
        if !capabilities.formats.contains(&FORMAT) || !capabilities.alpha_modes.contains(&ALPHA) {
            return Err(
                "DirectComposition surface does not support BGRA8 premultiplied alpha".into(),
            );
        }
        let required_features = if enable_bc7 {
            adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features,
                ..Default::default()
            })
            .await?;
        device.set_device_lost_callback(move |reason, message| {
            if reason == wgpu::DeviceLostReason::Unknown {
                eprintln!("Pet device lost: {message}");
                let _ = proxy.send_event(AppEvent::PetDeviceLost);
            }
        });
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("no surface configuration for the DX12 adapter")?;
        config.format = FORMAT;
        config.alpha_mode = ALPHA;
        config.present_mode = wgpu::PresentMode::Fifo;
        eprintln!("Surface: DxgiFromVisual, {FORMAT:?}, {ALPHA:?}, Fifo");
        // Configure on the first non-zero redraw after the window is shown.
        config.width = 0;
        config.height = 0;

        let shader = device.create_shader_module(wgpu::include_wgsl!("first_frame.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Pet frame"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview: None,
            cache: None,
        });
        let frame_rect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame rectangle"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut renderer = Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            sprite: None,
            frame_rect,
            upload: None,
            canvas: frame.canvas,
            presented: false,
        };
        renderer.show(None, frame)?;
        Ok(renderer)
    }

    pub fn supports_bc7(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }

    pub fn show(&mut self, key: Option<PageKey>, frame: FrameView<'_>) -> Result<(), String> {
        if self.sprite.as_ref().is_none_or(|sprite| sprite.key != key) {
            let [width, height] = frame.page_size;
            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            if width.max(height) > self.device.limits().max_texture_dimension_2d {
                return Err("page exceeds device texture dimensions".into());
            }
            let (format, bytes) = if let Some(blocks) = frame.bc7 {
                (wgpu::TextureFormat::Bc7RgbaUnorm, blocks)
            } else {
                (wgpu::TextureFormat::Rgba8Unorm, frame.rgba)
            };
            if let Some(previous) = &self.upload {
                self.device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(previous.clone()),
                        timeout: Some(Duration::from_millis(250)),
                    })
                    .map_err(|error| error.to_string())?;
            }
            if self.sprite.as_ref().is_none_or(|sprite| {
                sprite.texture.size() != size || sprite.texture.format() != format
            }) {
                if self.sprite.is_some() {
                    // Drain submitted work before replacing the one-slot page pool.
                    self.device
                        .poll(wgpu::PollType::Wait {
                            submission_index: None,
                            timeout: Some(Duration::from_millis(250)),
                        })
                        .map_err(|error| error.to_string())?;
                    self.sprite = None;
                }
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Active sprite page"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Active sprite page"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.frame_rect.as_entire_binding(),
                        },
                    ],
                });
                self.sprite = Some(Sprite { texture, bind, key });
            }
            let sprite = self.sprite.as_mut().unwrap();
            sprite.key = key;
            self.queue.write_texture(
                sprite.texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // BC7 has 16 bytes per 4-pixel block, also width * 4.
                    bytes_per_row: Some(width * 4),
                    rows_per_image: None,
                },
                size,
            );
            self.upload = Some(self.queue.submit([]));
        }
        self.canvas = frame.canvas;
        let rect = frame.rect.map(u32::to_le_bytes);
        self.queue
            .write_buffer(&self.frame_rect, 0, rect.as_flattened());
        Ok(())
    }

    pub fn invalidate_surface(&mut self) {
        self.config.width = 0;
        self.config.height = 0;
    }

    pub fn draw(&mut self, width: u32, height: u32) -> Result<(), wgpu::SurfaceError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        let frame = self.surface.get_current_texture()?;
        let destination = self
            .canvas
            .destination([width, height])
            .map(i32::to_le_bytes);
        self.queue
            .write_buffer(&self.frame_rect, 16, destination.as_flattened());
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.sprite.as_ref().unwrap().bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        if !self.presented {
            self.presented = true;
            eprintln!("First frame presented: {width}x{height}");
        }
        Ok(())
    }
}
