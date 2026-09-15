//! Compare immutable canvas uploads with a persistent Vello texture.
//! This is a measurement, not a native Canvas2D implementation.
use std::{error::Error, sync::Arc, time::Instant};
use vello::{Scene, kurbo::Affine, peniko};

const OUTPUT: u32 = 512;
const PATCH: u32 = 64;
const SAMPLES: usize = 120;

fn texture(device: &wgpu::Device, size: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Canvas upload measurement"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}
fn upload(queue: &wgpu::Queue, target: &wgpu::Texture, origin: u32, size: u32, bytes: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin,
                y: origin,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
}
fn scene(image: &peniko::ImageData, dimension: u32, rotated: bool) -> Scene {
    let mut scene = Scene::new();
    let brush = peniko::ImageBrush {
        image: image.clone(),
        sampler: peniko::ImageSampler {
            x_extend: peniko::Extend::Repeat,
            y_extend: peniko::Extend::Repeat,
            quality: peniko::ImageQuality::Medium,
            alpha: 0.73,
        },
    };
    let transform = if rotated {
        Affine::translate((256.25, 256.75))
            * Affine::rotate(0.17)
            * Affine::scale(0.8 * OUTPUT as f64 / dimension as f64)
            * Affine::translate((-(dimension as f64) / 2.0, -(dimension as f64) / 2.0))
    } else {
        Affine::scale(OUTPUT as f64 / dimension as f64)
    };
    scene.draw_image(brush.as_ref(), transform);
    scene
}
fn render(
    renderer: &mut vello::Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    renderer.render_to_texture(
        device,
        queue,
        scene,
        view,
        &vello::RenderParams {
            base_color: peniko::Color::TRANSPARENT,
            width: OUTPUT,
            height: OUTPUT,
            antialiasing_method: vello::AaConfig::Msaa16,
        },
    )?;
    Ok(())
}
fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Canvas comparison readback"),
        size: u64::from(OUTPUT * OUTPUT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(OUTPUT * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: OUTPUT,
            height: OUTPUT,
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
    let bytes = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    Ok(bytes)
}
fn statistics(values: &mut [f64]) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    serde_json::json!({ "meanMs": values.iter().sum::<f64>() / values.len() as f64,
        "p99Ms": values[(values.len() as f64 * 0.99).ceil() as usize - 1], "maxMs": values[values.len() - 1] })
}
fn main() -> Result<(), Box<dyn Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu {
        return Err("Software GPU is not permitted".into());
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))?;
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            use_cpu: false,
            num_init_threads: None,
            antialiasing_support: vello::AaSupport::all(),
            pipeline_cache: None,
        },
    )?;
    let output = texture(
        &device,
        OUTPUT,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let view = output.create_view(&Default::default());
    let mut results = Vec::new();
    let mut identity_differences = Vec::new();
    for dimension in [512, 2048, 4096] {
        let mut pixels = vec![0u8; dimension as usize * dimension as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[32, 36, 43, 255]);
        }
        let source = texture(
            &device,
            dimension,
            wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        );
        upload(&queue, &source, 0, dimension, &pixels);
        let registered = renderer.register_texture(source.clone());
        let mut patch = vec![0; (PATCH * PATCH * 4) as usize];
        let mut expected = Vec::new();
        for mode in ["immutable", "persistent"] {
            let mut setup = [0.0; SAMPLES];
            let mut complete = [0.0; SAMPLES];
            for iteration in 0..SAMPLES + 20 {
                let color = if iteration % 2 == 0 {
                    [255, 0, 0, 128]
                } else {
                    [0, 255, 0, 192]
                };
                for pixel in patch.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&color);
                }
                for row in 0..PATCH as usize {
                    let offset = ((row + PATCH as usize) * dimension as usize + PATCH as usize) * 4;
                    pixels[offset..offset + (PATCH * 4) as usize].copy_from_slice(
                        &patch[row * (PATCH * 4) as usize..(row + 1) * (PATCH * 4) as usize],
                    );
                }
                let start = Instant::now();
                let image = if mode == "immutable" {
                    peniko::ImageData {
                        data: peniko::Blob::new(Arc::new(pixels.clone())),
                        width: dimension,
                        height: dimension,
                        format: peniko::ImageFormat::Rgba8,
                        alpha_type: peniko::ImageAlphaType::Alpha,
                    }
                } else {
                    upload(&queue, &source, PATCH, PATCH, &patch);
                    renderer.mark_override_image_dirty(&registered);
                    registered.clone()
                };
                let scene = scene(&image, dimension, false);
                render(&mut renderer, &device, &queue, &scene, &view)?;
                let submitted = start.elapsed().as_secs_f64() * 1000.0;
                device.poll(wgpu::PollType::wait_indefinitely())?;
                if iteration >= 20 {
                    setup[iteration - 20] = submitted;
                    complete[iteration - 20] = start.elapsed().as_secs_f64() * 1000.0;
                }
            }
            let final_image = if mode == "immutable" {
                peniko::ImageData {
                    data: peniko::Blob::new(Arc::new(pixels.clone())),
                    width: dimension,
                    height: dimension,
                    format: peniko::ImageFormat::Rgba8,
                    alpha_type: peniko::ImageAlphaType::Alpha,
                }
            } else {
                registered.clone()
            };
            for (index, rotated) in [false, true].into_iter().enumerate() {
                render(
                    &mut renderer,
                    &device,
                    &queue,
                    &scene(&final_image, dimension, rotated),
                    &view,
                )?;
                let bytes = readback(&device, &queue, &output)?;
                if mode == "immutable" {
                    expected.push(bytes);
                } else if bytes != expected[index] {
                    let count = bytes
                        .iter()
                        .zip(&expected[index])
                        .filter(|(a, b)| a != b)
                        .count();
                    let maximum = bytes
                        .iter()
                        .zip(&expected[index])
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap_or(0);
                    let first = bytes
                        .iter()
                        .zip(&expected[index])
                        .position(|(a, b)| a != b)
                        .unwrap();
                    identity_differences.push(
                        serde_json::json!({ "dimension": dimension, "rotated": rotated,
                        "channels": count, "maximumByteDifference": maximum, "firstByte": first,
                        "expected": expected[index][first], "actual": bytes[first] }),
                    );
                }
            }
            results.push(serde_json::json!({ "dimension": dimension, "mode": mode,
                "cpuUploadBytes": if mode == "immutable" { pixels.len() } else { patch.len() },
                "setupAndSubmit": statistics(&mut setup), "completion": statistics(&mut complete) }));
        }
        renderer.unregister_texture(registered);
        // Use one image identity to keep atlas placement the same for the exact copy check.
        let image = peniko::ImageData {
            data: peniko::Blob::new(Arc::new(pixels)),
            width: dimension,
            height: dimension,
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
        };
        for rotated in [false, true] {
            renderer.override_image(&image, None);
            let scene = scene(&image, dimension, rotated);
            render(&mut renderer, &device, &queue, &scene, &view)?;
            let cpu = readback(&device, &queue, &output)?;
            renderer.override_image(
                &image,
                Some(wgpu::TexelCopyTextureInfoBase {
                    texture: source.clone(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                }),
            );
            render(&mut renderer, &device, &queue, &scene, &view)?;
            let gpu = readback(&device, &queue, &output)?;
            if cpu != gpu {
                return Err(format!(
                    "Same-identity pixel mismatch: dimension={dimension}, rotated={rotated}"
                )
                .into());
            }
        }
        renderer.override_image(&image, None);
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;
    if let Some(error) = pollster::block_on(errors.pop()) {
        return Err(error.to_string().into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({ "adapter": info.name, "samples": SAMPLES,
        "results": results, "exactSameIdentityComparisons": 6, "differentIdentityDifferences": identity_differences,
        "limits": "Blocking completion is not a GPU timestamp or frame timing. This is not native Canvas2D integration." })
        )?
    );
    Ok(())
}
