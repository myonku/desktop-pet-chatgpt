use std::{error::Error, sync::Arc};

use winit::{event_loop::EventLoopProxy, window::Window};

use crate::{platform::windows::InputEvent, sample::Sample};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const ALPHA: wgpu::CompositeAlphaMode = wgpu::CompositeAlphaMode::PreMultiplied;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    sprite: wgpu::BindGroup,
    presented: bool,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        sample: &Sample,
        proxy: EventLoopProxy<InputEvent>,
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
        let (device, queue) = adapter.request_device(&Default::default()).await?;
        device.set_device_lost_callback(move |reason, message| {
            if reason == wgpu::DeviceLostReason::Unknown {
                eprintln!("Pet device lost: {message}");
                let _ = proxy.send_event(InputEvent::PetDeviceLost);
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("render/first_frame.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("P1a first frame"),
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
        let view = upload_sample(&device, &queue, sample);
        let sprite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("P0 alpha-color PNG"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            sprite,
            presented: false,
        })
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
            pass.set_bind_group(0, &self.sprite, &[]);
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

fn upload_sample(device: &wgpu::Device, queue: &wgpu::Queue, sample: &Sample) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: sample.width,
        height: sample.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("P0 alpha-color PNG"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &sample.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(sample.width * 4),
            rows_per_image: None,
        },
        size,
    );
    texture.create_view(&Default::default())
}
