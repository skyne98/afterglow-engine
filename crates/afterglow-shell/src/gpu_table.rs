//! GPU resource table: a shared, thread-safe, generational table of `wgpu`
//! textures and buffers. Workers upload to the shared `wgpu_core::Global` and
//! register the resulting resource; the renderer retrieves it by handle and
//! binds it. Zero bytes cross to JS — only the 8-byte [`SlotHandle`] does.
//!
//! Built on the generic [`afterglow_rpc::handle::SlotMap`]: a stale handle
//! (slot reused after `take`/`remove`) is rejected, never silently aliased.

use std::sync::Arc;

use afterglow_rpc::handle::{SlotHandle, SlotMap};

pub struct GpuResourceTable {
    textures: Arc<SlotMap<wgpu::Texture>>,
    buffers: Arc<SlotMap<wgpu::Buffer>>,
}

impl GpuResourceTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            textures: SlotMap::new(),
            buffers: SlotMap::new(),
        })
    }

    /// Register a texture; returns a handle the renderer binds by.
    pub fn insert_texture(&self, texture: wgpu::Texture) -> SlotHandle {
        self.textures.insert(texture)
    }
    /// Take a texture out of the table for binding (one-shot). Returns `None`
    /// for a stale handle or an already-taken slot.
    pub fn take_texture(&self, handle: SlotHandle) -> Option<wgpu::Texture> {
        self.textures.take(handle)
    }

    /// Register a buffer.
    pub fn insert_buffer(&self, buffer: wgpu::Buffer) -> SlotHandle {
        self.buffers.insert(buffer)
    }
    /// Take a buffer out of the table.
    pub fn take_buffer(&self, handle: SlotHandle) -> Option<wgpu::Buffer> {
        self.buffers.take(handle)
    }
}

// Compile-time proof that `wgpu::Texture` / `wgpu::Buffer` are `Send` (required
// by `SlotMap<T: Send>` and to cross into a worker thread).
const _: () = {
    fn _assert_send<T: Send>() {}
    fn _gpu() {
        _assert_send::<wgpu::Texture>();
        _assert_send::<wgpu::Buffer>();
    }
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A worker thread creates a texture on a shared device (the worker→GPU
    /// upload path) and registers it in the table; the host retrieves it by
    /// handle. Proves a worker thread can drive the shared `wgpu` device
    /// (Global is Send+Sync) and the table resolves the handle. Requires a
    /// Vulkan-capable GPU (NVIDIA RTX 3090 on this workstation).
    #[test]
    fn worker_registers_texture_on_shared_device_and_host_retrieves() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::from_build_config(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .expect("request a Vulkan adapter (needs a GPU)");
        assert!(
            !adapter.get_info().name.contains("llvmpipe"),
            "software adapter; this test needs a real GPU"
        );
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("gpu-table test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            },
        ))
        .expect("request device");

        let table = GpuResourceTable::new();
        let table_c = table.clone();
        let device_c = device.clone();
        let queue_c = queue.clone();
        let worker = std::thread::spawn(move || {
            // The worker creates a texture + uploads to it through the shared
            // device/queue, then registers the texture in the table.
            let tex = device_c.create_texture(&wgpu::TextureDescriptor {
                label: Some("worker texture"),
                size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            queue_c.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[0xAB; 4 * 4 * 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * 4),
                    rows_per_image: Some(4),
                },
                wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            );
            table_c.insert_texture(tex)
        });

        let handle = worker.join().expect("worker thread");
        // Host retrieves the worker-created texture by handle.
        let tex = table.take_texture(handle).expect("texture handle resolves");
        assert_eq!(tex.width(), 4);
        assert_eq!(tex.height(), 4);
        // A stale handle (after take) is rejected.
        assert!(table.take_texture(handle).is_none());
    }
}
