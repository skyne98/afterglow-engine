use std::{
    mem::size_of,
    ptr, slice,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use glam::{Vec3A, vec3a};
use obvhs::{
    BvhBuildParams,
    cwbvh::{CwBvh, builder::build_cwbvh_from_tris},
    ray::{Ray, RayHit},
    triangle::Triangle,
};

const STATUS_OK: i32 = 0;
const STATUS_INVALID_ARGUMENT: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct IplVector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IplMaterial {
    pub absorption: [f32; 3],
    pub scattering: f32,
    pub transmission: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IplRay {
    pub origin: IplVector3,
    pub direction: IplVector3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IplHit {
    pub distance: f32,
    pub triangle_index: i32,
    pub object_index: i32,
    pub material_index: i32,
    pub normal: IplVector3,
    pub material: *mut IplMaterial,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputTriangle {
    pub v0: IplVector3,
    pub v1: IplVector3,
    pub v2: IplVector3,
    pub material_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TracerStats {
    pub static_node_count: u32,
    pub static_primitive_count: u32,
    pub door_node_count: u32,
    pub door_primitive_count: u32,
    pub owned_bytes: u64,
    pub build_milliseconds: f64,
}

struct TraceMesh {
    bvh: CwBvh,
    triangles: Box<[Triangle]>,
    material_indices: Box<[u32]>,
}

impl TraceMesh {
    fn build(input: &[InputTriangle], build_time: &mut Duration) -> Option<Self> {
        if input.is_empty() {
            return None;
        }
        let mut triangles = Vec::with_capacity(input.len());
        let mut material_indices = Vec::with_capacity(input.len());
        for triangle in input {
            triangles.push(Triangle {
                v0: to_vec3a(triangle.v0),
                v1: to_vec3a(triangle.v1),
                v2: to_vec3a(triangle.v2),
            });
            material_indices.push(u32::try_from(triangle.material_index).ok()?);
        }
        let bvh = build_cwbvh_from_tris(&triangles, BvhBuildParams::medium_build(), build_time);
        Some(Self {
            bvh,
            triangles: triangles.into_boxed_slice(),
            material_indices: material_indices.into_boxed_slice(),
        })
    }

    #[inline(always)]
    fn closest(&self, ray: Ray) -> Option<MeshHit> {
        let mut hit = RayHit::none();
        if !self
            .bvh
            .ray_traverse(ray, &mut hit, |candidate_ray, primitive_id| {
                let original_id = self.bvh.primitive_indices[primitive_id] as usize;
                intersect_triangle(&self.triangles[original_id], candidate_ray)
            })
        {
            return None;
        }
        let original_id = self.bvh.primitive_indices[hit.primitive_id as usize] as usize;
        Some(MeshHit {
            distance: hit.t,
            primitive_id: original_id,
        })
    }

    #[inline(always)]
    fn any(&self, ray: Ray) -> bool {
        !self
            .bvh
            .ray_traverse_miss(ray, |candidate_ray, primitive_id| {
                let original_id = self.bvh.primitive_indices[primitive_id] as usize;
                intersect_triangle(&self.triangles[original_id], candidate_ray)
            })
    }

    fn owned_bytes(&self) -> usize {
        self.bvh.nodes.len() * size_of_val_or_zero(&self.bvh.nodes)
            + self.bvh.primitive_indices.len() * size_of::<u32>()
            + self.triangles.len() * size_of::<Triangle>()
            + self.material_indices.len() * size_of::<u32>()
    }
}

#[inline]
fn size_of_val_or_zero<T>(values: &[T]) -> usize {
    if values.is_empty() { 0 } else { size_of::<T>() }
}

#[derive(Clone, Copy)]
struct MeshHit {
    distance: f32,
    primitive_id: usize,
}

pub struct Tracer {
    static_mesh: TraceMesh,
    door_mesh: Option<TraceMesh>,
    materials: Box<[IplMaterial]>,
    door_y_bits: AtomicU32,
    stats: TracerStats,
}

#[derive(Clone, Copy)]
struct SceneHit {
    distance: f32,
    primitive_id: usize,
    object_id: i32,
}

impl Tracer {
    #[inline(always)]
    fn ray(&self, ray: &IplRay, min_distance: f32, max_distance: f32) -> Option<Ray> {
        if !min_distance.is_finite()
            || max_distance.is_nan()
            || max_distance <= min_distance
            || !vector_is_finite(ray.origin)
            || !vector_is_finite(ray.direction)
        {
            return None;
        }
        Some(Ray::new(
            to_vec3a(ray.origin),
            to_vec3a(ray.direction),
            min_distance,
            max_distance,
        ))
    }

    #[inline(always)]
    fn closest(&self, input: &IplRay, min_distance: f32, max_distance: f32) -> Option<SceneHit> {
        let ray = self.ray(input, min_distance, max_distance)?;
        let static_hit = self.static_mesh.closest(ray).map(|hit| SceneHit {
            distance: hit.distance,
            primitive_id: hit.primitive_id,
            object_id: 0,
        });
        let door_hit = self.door_mesh.as_ref().and_then(|door_mesh| {
            let door_y = f32::from_bits(self.door_y_bits.load(Ordering::Relaxed));
            let mut door_ray = ray;
            door_ray.origin.y -= door_y;
            door_mesh.closest(door_ray).map(|hit| SceneHit {
                distance: hit.distance,
                primitive_id: hit.primitive_id,
                object_id: 1,
            })
        });
        match (static_hit, door_hit) {
            (Some(left), Some(right)) => Some(if left.distance <= right.distance {
                left
            } else {
                right
            }),
            (Some(hit), None) | (None, Some(hit)) => Some(hit),
            (None, None) => None,
        }
    }

    #[inline(always)]
    fn any(&self, input: &IplRay, min_distance: f32, max_distance: f32) -> bool {
        let Some(ray) = self.ray(input, min_distance, max_distance) else {
            return max_distance <= min_distance;
        };
        if self.static_mesh.any(ray) {
            return true;
        }
        self.door_mesh.as_ref().is_some_and(|door_mesh| {
            let door_y = f32::from_bits(self.door_y_bits.load(Ordering::Relaxed));
            let mut door_ray = ray;
            door_ray.origin.y -= door_y;
            door_mesh.any(door_ray)
        })
    }

    #[inline(always)]
    fn fill_hit(&self, scene_hit: SceneHit, output: &mut IplHit) {
        let (mesh, global_primitive_id) = if scene_hit.object_id == 0 {
            (&self.static_mesh, scene_hit.primitive_id)
        } else {
            let Some(door_mesh) = self.door_mesh.as_ref() else {
                return;
            };
            (
                door_mesh,
                self.static_mesh.triangles.len() + scene_hit.primitive_id,
            )
        };
        let material_id = mesh.material_indices[scene_hit.primitive_id] as usize;
        let normal = mesh.triangles[scene_hit.primitive_id].compute_normal();
        output.distance = scene_hit.distance;
        output.triangle_index = i32::try_from(global_primitive_id).unwrap_or(-1);
        output.object_index = scene_hit.object_id;
        output.material_index = i32::try_from(material_id).unwrap_or(-1);
        output.normal = from_vec3a(normal);
        output.material = self.materials.as_ptr().wrapping_add(material_id) as *mut IplMaterial;
    }
}

#[inline(always)]
fn intersect_triangle(triangle: &Triangle, ray: &Ray) -> f32 {
    // Two-sided Möller–Trumbore with inclusive edge tests. obvhs 0.3.2's
    // built-in Triangle::intersect rejects negative zero, which can create a
    // crack when an acoustic ray lands exactly on a shared triangle edge.
    let edge1 = triangle.v1 - triangle.v0;
    let edge2 = triangle.v2 - triangle.v0;
    let p = ray.direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() <= 1.0e-8 {
        return f32::INFINITY;
    }
    let inverse_determinant = determinant.recip();
    let offset = ray.origin - triangle.v0;
    let u = offset.dot(p) * inverse_determinant;
    if !(0.0..=1.0).contains(&u) {
        return f32::INFINITY;
    }
    let q = offset.cross(edge1);
    let v = ray.direction.dot(q) * inverse_determinant;
    if v < 0.0 || u + v > 1.0 {
        return f32::INFINITY;
    }
    let distance = edge2.dot(q) * inverse_determinant;
    if distance >= ray.tmin && distance <= ray.tmax {
        distance
    } else {
        f32::INFINITY
    }
}

#[inline(always)]
fn to_vec3a(value: IplVector3) -> Vec3A {
    vec3a(value.x, value.y, value.z)
}

#[inline(always)]
fn from_vec3a(value: Vec3A) -> IplVector3 {
    IplVector3 {
        x: value.x,
        y: value.y,
        z: value.z,
    }
}

#[inline(always)]
fn vector_is_finite(value: IplVector3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

#[inline(always)]
fn miss() -> IplHit {
    IplHit {
        distance: f32::INFINITY,
        triangle_index: -1,
        object_index: -1,
        material_index: -1,
        normal: IplVector3::default(),
        material: ptr::null_mut(),
    }
}

/// # Safety
/// All input arrays must contain the declared number of initialized elements and
/// remain readable for this call. `tracer_out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_create(
    static_triangles: *const InputTriangle,
    static_triangle_count: u32,
    door_triangles: *const InputTriangle,
    door_triangle_count: u32,
    materials: *const IplMaterial,
    material_count: u32,
    tracer_out: *mut *mut Tracer,
) -> i32 {
    if static_triangles.is_null()
        || (door_triangle_count > 0 && door_triangles.is_null())
        || materials.is_null()
        || tracer_out.is_null()
        || static_triangle_count == 0
        || material_count == 0
    {
        return STATUS_INVALID_ARGUMENT;
    }
    // SAFETY: Validated non-null; lengths are part of this function's caller contract.
    let static_input =
        unsafe { slice::from_raw_parts(static_triangles, static_triangle_count as usize) };
    let door_input = if door_triangle_count == 0 {
        &[]
    } else {
        // SAFETY: Non-null when the declared count is non-zero.
        unsafe { slice::from_raw_parts(door_triangles, door_triangle_count as usize) }
    };
    // SAFETY: Same as above.
    let material_input = unsafe { slice::from_raw_parts(materials, material_count as usize) };
    let valid_material = |triangle: &InputTriangle| {
        triangle.material_index >= 0 && (triangle.material_index as u32) < material_count
    };
    if !static_input.iter().all(valid_material)
        || !door_input.iter().all(valid_material)
        || !material_input.iter().all(|material| {
            material.absorption.iter().all(|value| value.is_finite())
                && material.scattering.is_finite()
                && material.transmission.iter().all(|value| value.is_finite())
        })
    {
        return STATUS_INVALID_ARGUMENT;
    }

    let started = Instant::now();
    let mut measured_build_time = Duration::ZERO;
    let Some(static_mesh) = TraceMesh::build(static_input, &mut measured_build_time) else {
        return STATUS_INVALID_ARGUMENT;
    };
    let door_mesh = if door_input.is_empty() {
        None
    } else {
        TraceMesh::build(door_input, &mut measured_build_time)
    };
    let materials = material_input.to_vec().into_boxed_slice();
    let stats = TracerStats {
        static_node_count: static_mesh.bvh.nodes.len() as u32,
        static_primitive_count: static_mesh.triangles.len() as u32,
        door_node_count: door_mesh
            .as_ref()
            .map_or(0, |mesh| mesh.bvh.nodes.len() as u32),
        door_primitive_count: door_mesh
            .as_ref()
            .map_or(0, |mesh| mesh.triangles.len() as u32),
        owned_bytes: (static_mesh.owned_bytes()
            + door_mesh.as_ref().map_or(0, TraceMesh::owned_bytes)
            + materials.len() * size_of::<IplMaterial>()) as u64,
        build_milliseconds: started.elapsed().as_secs_f64() * 1_000.0,
    };
    let tracer = Box::new(Tracer {
        static_mesh,
        door_mesh,
        materials,
        door_y_bits: AtomicU32::new(0.0f32.to_bits()),
        stats,
    });
    // SAFETY: `tracer_out` is writable by contract and receives ownership.
    unsafe { tracer_out.write(Box::into_raw(tracer)) };
    STATUS_OK
}

/// # Safety
/// `tracer` must be null or a live pointer returned by `afterglow_obvhs_create`
/// that has not already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_destroy(tracer: *mut Tracer) {
    if !tracer.is_null() {
        // SAFETY: Guaranteed by this function's contract.
        drop(unsafe { Box::from_raw(tracer) });
    }
}

/// # Safety
/// `tracer` must point to a live tracer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_set_door_y(tracer: *mut Tracer, door_y: f32) {
    if !tracer.is_null() && door_y.is_finite() {
        // SAFETY: Validated non-null and guaranteed live by the caller.
        unsafe { &*tracer }
            .door_y_bits
            .store(door_y.to_bits(), Ordering::Relaxed);
    }
}

/// # Safety
/// `tracer` must point to a live tracer and `stats` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_get_stats(tracer: *const Tracer, stats: *mut TracerStats) {
    if !tracer.is_null() && !stats.is_null() {
        // SAFETY: Pointers are valid by contract.
        unsafe { stats.write((&*tracer).stats) };
    }
}

/// # Safety
/// Steam Audio guarantees callback pointers are valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_closest_hit(
    ray: *const IplRay,
    min_distance: f32,
    max_distance: f32,
    hit: *mut IplHit,
    user_data: *mut Tracer,
) {
    if ray.is_null() || hit.is_null() || user_data.is_null() {
        return;
    }
    // SAFETY: Validated non-null and guaranteed live by the callback contract.
    let tracer = unsafe { &*user_data };
    let mut output = miss();
    // SAFETY: Validated non-null and readable by the callback contract.
    if let Some(scene_hit) = tracer.closest(unsafe { &*ray }, min_distance, max_distance) {
        tracer.fill_hit(scene_hit, &mut output);
    }
    // SAFETY: Validated non-null and writable by the callback contract.
    unsafe { hit.write(output) };
}

/// # Safety
/// Steam Audio guarantees callback pointers are valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_any_hit(
    ray: *const IplRay,
    min_distance: f32,
    max_distance: f32,
    occluded: *mut u8,
    user_data: *mut Tracer,
) {
    if ray.is_null() || occluded.is_null() || user_data.is_null() {
        return;
    }
    // SAFETY: Callback pointer contract.
    let value = unsafe { &*user_data }.any(unsafe { &*ray }, min_distance, max_distance) as u8;
    // SAFETY: Validated writable callback output.
    unsafe { occluded.write(value) };
}

/// # Safety
/// Steam Audio guarantees each callback array has `num_rays` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_batched_closest_hit(
    num_rays: i32,
    rays: *const IplRay,
    min_distances: *const f32,
    max_distances: *const f32,
    hits: *mut IplHit,
    user_data: *mut Tracer,
) {
    if num_rays <= 0
        || rays.is_null()
        || min_distances.is_null()
        || max_distances.is_null()
        || hits.is_null()
        || user_data.is_null()
    {
        return;
    }
    for index in 0..num_rays as usize {
        // SAFETY: Callback array contract guarantees all indexed elements.
        unsafe {
            afterglow_obvhs_closest_hit(
                rays.add(index),
                *min_distances.add(index),
                *max_distances.add(index),
                hits.add(index),
                user_data,
            );
        }
    }
}

/// # Safety
/// Steam Audio guarantees each callback array has `num_rays` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn afterglow_obvhs_batched_any_hit(
    num_rays: i32,
    rays: *const IplRay,
    min_distances: *const f32,
    max_distances: *const f32,
    occluded: *mut u8,
    user_data: *mut Tracer,
) {
    if num_rays <= 0
        || rays.is_null()
        || min_distances.is_null()
        || max_distances.is_null()
        || occluded.is_null()
        || user_data.is_null()
    {
        return;
    }
    for index in 0..num_rays as usize {
        // SAFETY: Callback array contract guarantees all indexed elements.
        unsafe {
            afterglow_obvhs_any_hit(
                rays.add(index),
                *min_distances.add(index),
                *max_distances.add(index),
                occluded.add(index),
                user_data,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
        sync::atomic::AtomicUsize,
    };

    struct CountingAllocator;
    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    }

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            TRACK_ALLOCATIONS.with(|tracking| {
                if tracking.get() {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                }
            });
            // SAFETY: Delegated to the system allocator.
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: Delegated to the system allocator.
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    fn point(x: f32, y: f32, z: f32) -> IplVector3 {
        IplVector3 { x, y, z }
    }

    fn wall(material_index: i32) -> [InputTriangle; 2] {
        [
            InputTriangle {
                v0: point(0.0, -2.0, -2.0),
                v1: point(0.0, 2.0, -2.0),
                v2: point(0.0, 2.0, 2.0),
                material_index,
            },
            InputTriangle {
                v0: point(0.0, -2.0, -2.0),
                v1: point(0.0, 2.0, 2.0),
                v2: point(0.0, -2.0, 2.0),
                material_index,
            },
        ]
    }

    fn create() -> *mut Tracer {
        let static_triangles = wall(0);
        let mut door_triangles = wall(1);
        for triangle in &mut door_triangles {
            triangle.v0.x = 2.0;
            triangle.v1.x = 2.0;
            triangle.v2.x = 2.0;
        }
        let materials = [
            IplMaterial {
                absorption: [0.1; 3],
                scattering: 0.2,
                transmission: [0.3; 3],
            },
            IplMaterial {
                absorption: [0.4; 3],
                scattering: 0.5,
                transmission: [0.6; 3],
            },
        ];
        let mut tracer = ptr::null_mut();
        // SAFETY: All arrays and the output pointer are valid for this call.
        let status = unsafe {
            afterglow_obvhs_create(
                static_triangles.as_ptr(),
                2,
                door_triangles.as_ptr(),
                2,
                materials.as_ptr(),
                2,
                &mut tracer,
            )
        };
        assert_eq!(status, STATUS_OK);
        assert!(!tracer.is_null());
        tracer
    }

    #[test]
    fn closest_hit_reports_geometry_normal_and_material() {
        let tracer = create();
        let ray = IplRay {
            origin: point(-1.0, 0.25, 0.0),
            direction: point(1.0, 0.0, 0.0),
        };
        let mut hit = miss();
        // SAFETY: Test pointers are live.
        unsafe { afterglow_obvhs_closest_hit(&ray, 0.001, 10.0, &mut hit, tracer) };
        assert!((hit.distance - 1.0).abs() < 1.0e-5);
        assert_eq!(hit.object_index, 0);
        assert_eq!(hit.material_index, 0);
        assert!(!hit.material.is_null());
        assert!((hit.normal.x - 1.0).abs() < 1.0e-5);
        // SAFETY: Ownership belongs to this test.
        unsafe { afterglow_obvhs_destroy(tracer) };
    }

    #[test]
    fn shared_triangle_edge_has_no_acoustic_crack() {
        let tracer = create();
        let ray = IplRay {
            origin: point(-1.0, 0.0, 0.0),
            direction: point(1.0, 0.0, 0.0),
        };
        let mut hit = miss();
        unsafe { afterglow_obvhs_closest_hit(&ray, 0.001, 10.0, &mut hit, tracer) };
        assert!((hit.distance - 1.0).abs() < 1.0e-5);
        unsafe { afterglow_obvhs_destroy(tracer) };
    }

    #[test]
    fn interval_and_any_hit_semantics_match_steam_audio() {
        let tracer = create();
        let ray = IplRay {
            origin: point(-1.0, 0.25, 0.0),
            direction: point(1.0, 0.0, 0.0),
        };
        let mut occluded = 0;
        // SAFETY: Test pointers are live.
        unsafe { afterglow_obvhs_any_hit(&ray, 0.001, 0.5, &mut occluded, tracer) };
        assert_eq!(occluded, 0);
        unsafe { afterglow_obvhs_any_hit(&ray, 0.001, 1.5, &mut occluded, tracer) };
        assert_eq!(occluded, 1);
        unsafe { afterglow_obvhs_any_hit(&ray, 2.0, 1.0, &mut occluded, tracer) };
        assert_eq!(occluded, 1);
        unsafe { afterglow_obvhs_destroy(tracer) };
    }

    #[test]
    fn translated_door_blas_moves_without_rebuild() {
        let tracer = create();
        let ray = IplRay {
            origin: point(1.0, 0.25, 0.0),
            direction: point(1.0, 0.0, 0.0),
        };
        let mut hit = miss();
        unsafe { afterglow_obvhs_closest_hit(&ray, 0.001, 10.0, &mut hit, tracer) };
        assert_eq!(hit.object_index, 1);
        unsafe { afterglow_obvhs_set_door_y(tracer, 5.0) };
        unsafe { afterglow_obvhs_closest_hit(&ray, 0.001, 10.0, &mut hit, tracer) };
        assert!(hit.distance.is_infinite());
        unsafe { afterglow_obvhs_destroy(tracer) };
    }

    #[test]
    fn callbacks_allocate_nothing_after_build() {
        let tracer = create();
        let rays = [
            IplRay {
                origin: point(-1.0, 0.25, 0.0),
                direction: point(1.0, 0.0, 0.0),
            },
            IplRay {
                origin: point(-1.0, 8.0, 0.0),
                direction: point(1.0, 0.0, 0.0),
            },
        ];
        let mins = [0.001; 2];
        let maxes = [10.0; 2];
        let mut hits = [miss(); 2];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(true));
        unsafe {
            afterglow_obvhs_batched_closest_hit(
                2,
                rays.as_ptr(),
                mins.as_ptr(),
                maxes.as_ptr(),
                hits.as_mut_ptr(),
                tracer,
            )
        };
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
        unsafe { afterglow_obvhs_destroy(tracer) };
    }
}
