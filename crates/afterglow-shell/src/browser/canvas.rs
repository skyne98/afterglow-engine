//! Mutable CPU pixels and persistent GPU images for connected native canvases.
use super::BrowserDocument;
use blitz_dom::node::RasterImageData;
use std::sync::Arc;
use vello::peniko::ImageData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl RasterRegion {
    pub fn validate(self, width: u32, height: u32, length: usize) -> Result<(), String> {
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || u64::from(width) * u64::from(height) * 4 != length as u64
            || self.width == 0
            || self.height == 0
            || self.x.checked_add(self.width).is_none_or(|end| end > width)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|end| end > height)
        {
            return Err("invalid native canvas raster region".into());
        }
        Ok(())
    }
    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }
}

pub(super) struct CanvasPixels {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    dirty: Option<RasterRegion>,
    gpu: Option<(ImageData, wgpu::Texture, afterglow_memory::Reservation)>,
    pub enabled: bool,
    _cpu_memory: afterglow_memory::Reservation,
    pending_gpu_memory: Option<afterglow_memory::Reservation>,
}
impl CanvasPixels {
    fn new(width: u32, height: u32, pixels: &[u8]) -> Result<Self, String> {
        Self::new_with_memory(width, height, pixels, afterglow_memory::process()?)
    }
    fn new_with_memory(width: u32, height: u32, pixels: &[u8], memory: &Arc<afterglow_memory::EngineMemory>) -> Result<Self, String> {
        // CPU mirror plus the page buffer. GPU admission includes atlas headroom.
        let cpu_memory = memory.reserve(pixels.len() as u64 * 2).ok_or("native canvas memory capacity reached")?;
        let gpu_memory = memory.reserve(pixels.len() as u64 * 2).ok_or("native canvas GPU memory capacity reached")?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(pixels.len())
            .map_err(|_| "native canvas allocation failed")?;
        copy.extend_from_slice(pixels);
        Ok(Self {
            width,
            height,
            pixels: copy,
            dirty: Some(RasterRegion {
                x: 0,
                y: 0,
                width,
                height,
            }),
            gpu: None,
            enabled: true,
            _cpu_memory: cpu_memory,
            pending_gpu_memory: Some(gpu_memory),
        })
    }
    fn update(&mut self, region: RasterRegion, source: &[u8]) {
        let stride = self.width as usize * 4;
        for row in region.y..region.y + region.height {
            let start = row as usize * stride + region.x as usize * 4;
            let end = start + region.width as usize * 4;
            self.pixels[start..end].copy_from_slice(&source[start..end]);
        }
        self.dirty = Some(self.dirty.map_or(region, |old| old.union(region)));
    }
}

impl BrowserDocument {
    /// The source is the full Canvas2D buffer. Only the specified rows are copied.
    pub fn update_canvas_raster(
        &mut self,
        native_id: u64,
        width: u32,
        height: u32,
        region: RasterRegion,
        rgba: &[u8],
    ) -> Result<(), String> {
        region.validate(width, height, rgba.len())?;
        let node = self
            .nodes
            .get(&native_id)
            .and_then(|id| self.document.as_ref()?.get_node(*id))
            .and_then(|node| node.element_data())
            .ok_or("native canvas is not connected")?;
        if node.name.local.as_ref() != "canvas" {
            return Err("native node is not a canvas".into());
        }
        if let Some(canvas) = self.canvas_pixels.get_mut(&native_id) {
            if canvas.width == width && canvas.height == height {
                canvas.update(region, rgba);
                canvas.enabled = true;
                return Ok(());
            }
            // Prepare the new CPU buffer before changing the current raster.
            let mut replacement = CanvasPixels::new(width, height, rgba)?;
            replacement.gpu = canvas.gpu.take();
            *canvas = replacement;
        } else {
            self.canvas_pixels
                .try_reserve(1)
                .map_err(|_| "native canvas table allocation failed")?;
            self.canvas_pixels
                .insert(native_id, CanvasPixels::new(width, height, rgba)?);
        }
        Ok(())
    }

    /// GPU images are only installed for this renderer. CPU paint uses snapshots.
    pub fn prepare_canvas_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
    ) -> Result<(), String> {
        self.canvas_pixels.retain(|id, canvas| {
            if canvas.enabled && self.nodes.contains_key(id) {
                return true;
            }
            if let Some((image, _, memory)) = canvas.gpu.take() {
                renderer.unregister_texture(image);
                // Submit pending texture writes before the retirement callback.
                queue.submit([]);
                queue.on_submitted_work_done(move || drop(memory));
            }
            false
        });
        let document = self
            .document
            .as_mut()
            .ok_or("browser document is not synchronized")?;
        for (id, canvas) in &mut self.canvas_pixels {
            if canvas.width > device.limits().max_texture_dimension_2d
                || canvas.height > device.limits().max_texture_dimension_2d
            {
                return Err("native canvas exceeds the GPU texture dimension".into());
            }
            if canvas.gpu.as_ref().is_some_and(|(_, texture, _)| {
                texture.width() != canvas.width || texture.height() != canvas.height
            }) {
                let (image, _, memory) = canvas.gpu.take().unwrap();
                renderer.unregister_texture(image);
                queue.submit([]);
                queue.on_submitted_work_done(move || drop(memory));
            }
            if canvas.gpu.is_none() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Native Canvas2D raster"),
                    size: wgpu::Extent3d {
                        width: canvas.width,
                        height: canvas.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let image = renderer.register_texture(texture.clone());
                canvas.gpu = Some((image, texture, canvas.pending_gpu_memory.take().ok_or("missing canvas GPU admission")?));
                canvas.dirty = Some(RasterRegion {
                    x: 0,
                    y: 0,
                    width: canvas.width,
                    height: canvas.height,
                });
            }
            // A resize back to the live texture dimensions can reuse its admission.
            canvas.pending_gpu_memory = None;
            let (image, texture, _) = canvas.gpu.as_ref().unwrap();
            if let Some(region) = canvas.dirty.take() {
                let start = ((region.y as usize * canvas.width as usize) + region.x as usize) * 4;
                let end = (((region.y + region.height - 1) as usize * canvas.width as usize)
                    + (region.x + region.width) as usize)
                    * 4;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: region.x,
                            y: region.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &canvas.pixels[start..end],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(canvas.width * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: region.width,
                        height: region.height,
                        depth_or_array_layers: 1,
                    },
                );
                renderer.mark_override_image_dirty(image);
            }
            document
                .mutate()
                .set_canvas_image(
                    self.nodes[id],
                    RasterImageData {
                        width: canvas.width,
                        height: canvas.height,
                        data: image.data.clone(),
                    },
                )
                .map_err(str::to_string)?;
        }
        Ok(())
    }

    /// CPU capture is a cold path and copies current pixels into immutable images.
    pub(super) fn prepare_canvas_snapshots(&mut self) -> Result<(), String> {
        let document = self
            .document
            .as_mut()
            .ok_or("browser document is not synchronized")?;
        for (id, canvas) in &self.canvas_pixels {
            if !canvas.enabled {
                continue;
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let memory = afterglow_memory::process()?.reserve(canvas.pixels.len() as u64)
                .ok_or("native canvas snapshot memory capacity reached")?;
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(canvas.pixels.len())
                .map_err(|_| "canvas snapshot allocation failed")?;
            pixels.extend_from_slice(&canvas.pixels);
            document
                .mutate()
                .set_canvas_image(*node, RasterImageData { width: canvas.width, height: canvas.height,
                    data: vello::peniko::Blob::new(Arc::new(CanvasSnapshot { pixels, _memory: memory })) })
                .map_err(str::to_string)?;
        }
        Ok(())
    }
}

// The reservation follows every immutable image clone, including cold render scenes.
struct CanvasSnapshot {
    pixels: Vec<u8>,
    _memory: afterglow_memory::Reservation,
}
impl AsRef<[u8]> for CanvasSnapshot { fn as_ref(&self) -> &[u8] { &self.pixels } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_admission_rejects_growth_without_losing_pixels() {
        let memory = afterglow_memory::EngineMemory::new(192);
        let canvas = CanvasPixels::new_with_memory(4, 3, &[7; 48], &memory).unwrap();
        assert_eq!(memory.stats().used, 192);
        assert!(CanvasPixels::new_with_memory(4, 4, &[9; 64], &memory).is_err());
        assert_eq!(canvas.pixels, [7; 48]);
        assert_eq!(memory.stats().used, 192);
        drop(canvas);
        assert_eq!(memory.stats().used, 0);
    }

    #[test]
    fn regions_copy_only_changed_pixels_and_merge_pending_work() {
        let mut canvas = CanvasPixels::new(4, 3, &[7; 48]).unwrap();
        canvas.dirty = None;
        let a = RasterRegion {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        };
        let b = RasterRegion {
            x: 3,
            y: 2,
            width: 1,
            height: 1,
        };
        a.validate(4, 3, 48).unwrap();
        canvas.update(a, &[23; 48]);
        canvas.update(b, &[42; 48]);
        for index in 0..12 {
            assert_eq!(
                &canvas.pixels[index * 4..index * 4 + 4],
                &[if index == 5 {
                    23
                } else if index == 11 {
                    42
                } else {
                    7
                }; 4]
            );
        }
        assert_eq!(
            canvas.dirty,
            Some(RasterRegion {
                x: 1,
                y: 1,
                width: 3,
                height: 2
            })
        );
    }
    fn browser() -> BrowserDocument {
        let mut browser = BrowserDocument::new(32, 32);
        let snapshot = serde_json::from_value(serde_json::json!({ "nodes": [
            {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
            {"id":2,"kind":"element","localName":"head","attributes":[],"children":[]},
            {"id":3,"kind":"element","localName":"body","attributes":[{"localName":"style","value":"margin:0"}],"children":[4]},
            {"id":4,"kind":"element","localName":"canvas","attributes":[
                {"localName":"width","value":"32"},{"localName":"height","value":"32"},
                {"localName":"style","value":"display:block;width:32px;height:32px"}],"children":[]}
        ]})).unwrap();
        browser.sync(1, snapshot, "https://example.test/").unwrap();
        browser
    }

    #[test]
    fn cpu_capture_keeps_partial_pixels_and_rejects_invalid_updates() {
        let mut browser = browser();
        let mut pixels = [19, 31, 43, 255].repeat(32 * 32);
        browser
            .set_canvas_raster(4, 32, 32, pixels.clone())
            .unwrap();
        assert_eq!(browser.render_overlay().unwrap(), pixels);
        pixels[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let region = RasterRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        browser
            .update_canvas_raster(4, 32, 32, region, &pixels)
            .unwrap();
        assert_eq!(browser.render_overlay().unwrap(), pixels);
        assert!(
            browser
                .update_canvas_raster(3, 32, 32, region, &pixels)
                .is_err()
        );
        assert!(
            browser
                .update_canvas_raster(4, 32, 32, region, &pixels[..4])
                .is_err()
        );
        assert_eq!(browser.render_overlay().unwrap(), pixels);
    }

    #[test]
    #[ignore = "needs a hardware Vulkan adapter"]
    fn gpu_regions_keep_pixels_across_capture_resize_and_removal()
    -> Result<(), Box<dyn std::error::Error>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                use_cpu: false,
                num_init_threads: None,
                antialiasing_support: vello::AaSupport::all(),
                pipeline_cache: None,
            },
        )?;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        fn read_pixels(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            texture: &wgpu::Texture,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
            let width = texture.width();
            let height = texture.height();
            let stride = (width * 4).div_ceil(256) * 256;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(stride) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = send.send(result);
                });
            device.poll(wgpu::PollType::wait_indefinitely())?;
            receive.recv()??;
            let mut bytes = Vec::with_capacity((width * height * 4) as usize);
            {
                let mapped = buffer.slice(..).get_mapped_range();
                for row in mapped.chunks_exact(stride as usize) {
                    bytes.extend_from_slice(&row[..(width * 4) as usize]);
                }
            }
            buffer.unmap();
            Ok(bytes)
        }
        let mut browser = browser();
        let mut previous = None;
        let mut pixels = Vec::new();
        for (iteration, dimension) in [32, 32, 32, 64, 64, 32].into_iter().enumerate() {
            let full = RasterRegion {
                x: 0,
                y: 0,
                width: dimension,
                height: dimension,
            };
            let resized = pixels.len() != (dimension * dimension * 4) as usize;
            if resized {
                pixels = [19, 31, 43, 255].repeat(dimension as usize * dimension as usize);
                browser.update_canvas_raster(4, dimension, dimension, full, &pixels)?;
            }
            // Multiple publications before one frame must keep both changes.
            let y = iteration as u32 + 1;
            for x in [1, dimension - 2] {
                let offset = (y * dimension + x) as usize * 4;
                pixels[offset..offset + 4].copy_from_slice(&[255, iteration as u8 * 17, 0, 192]);
                browser.update_canvas_raster(
                    4,
                    dimension,
                    dimension,
                    RasterRegion {
                        x,
                        y,
                        width: 1,
                        height: 1,
                    },
                    &pixels,
                )?;
            }
            if !resized {
                assert_eq!(
                    browser.canvas_pixels[&4].dirty,
                    Some(RasterRegion {
                        x: 1,
                        y,
                        width: dimension - 2,
                        height: 1,
                    })
                );
            }
            let expected = browser.render_overlay()?;
            browser.prepare_canvas_textures(&device, &queue, &mut renderer)?;
            let id = browser.canvas_pixels[&4].gpu.as_ref().unwrap().0.data.id();
            if let Some((old_dimension, old_id)) = previous {
                assert_eq!(id == old_id, dimension == old_dimension);
            }
            previous = Some((dimension, id));
            let mut scene = vello::Scene::new();
            browser.paint_overlay(&mut anyrender_vello::VelloScenePainter::new(&mut scene))?;
            renderer.render_to_texture(
                &device,
                &queue,
                &scene,
                &target.create_view(&Default::default()),
                &vello::RenderParams {
                    width: 32,
                    height: 32,
                    base_color: vello::peniko::Color::TRANSPARENT,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )?;
            let actual = read_pixels(&device, &queue, &target)?;
            let (image, source, _) = browser.canvas_pixels[&4].gpu.as_ref().unwrap();
            assert_eq!(read_pixels(&device, &queue, source)?, pixels);
            let reference = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Full-upload reference"),
                size: wgpu::Extent3d {
                    width: dimension,
                    height: dimension,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                reference.as_image_copy(),
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dimension * 4),
                    rows_per_image: None,
                },
                reference.size(),
            );
            // Keep the image identity and GPU rasterizer the same for this exact comparison.
            let original = renderer.override_image(
                image,
                Some(wgpu::TexelCopyTextureInfoBase {
                    texture: reference,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                }),
            );
            renderer.render_to_texture(
                &device,
                &queue,
                &scene,
                &target.create_view(&Default::default()),
                &vello::RenderParams {
                    width: 32,
                    height: 32,
                    base_color: vello::peniko::Color::TRANSPARENT,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )?;
            assert_eq!(read_pixels(&device, &queue, &target)?, actual);
            renderer.override_image(image, original);
            assert_eq!(browser.render_overlay()?, expected);
        }
        browser.suppress_canvas_paint(4)?;
        browser.prepare_canvas_textures(&device, &queue, &mut renderer)?;
        assert!(browser.canvas_pixels.is_empty());
        device.poll(wgpu::PollType::wait_indefinitely())?;
        assert!(pollster::block_on(scope.pop()).is_none());
        Ok(())
    }

    #[test]
    fn rejects_malformed_region_before_copy() {
        for region in [
            RasterRegion {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            RasterRegion {
                x: 0,
                y: 3,
                width: 1,
                height: 1,
            },
            RasterRegion {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
        ] {
            assert!(region.validate(4, 3, 48).is_err());
        }
        let region = RasterRegion {
            x: 0,
            y: 0,
            width: 4,
            height: 3,
        };
        assert!(region.validate(4, 3, 47).is_err());
        assert!(region.validate(16385, 3, 16385 * 3 * 4).is_err());
    }
}
