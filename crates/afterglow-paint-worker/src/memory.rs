//! Native paint admission. The ceiling follows installed RAM, not free RAM.
const MIB: u64 = 1024 * 1024;
const BLOCK: u64 = 64 * MIB;
const TILE_BYTES: u64 = 32768;

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub maximum_bytes: u64,
    pub limit_bytes: u64,
    pub reserved_bytes: u64,
    pub maximum_tiles: usize,
}

pub use afterglow_memory::installed_bytes;

impl Budget {
    pub fn new(installed: u64, requested_mib: Option<f64>, width: u32, height: u32,
        threads: usize) -> Result<Self, String> {
        let maximum_bytes = (installed / 2 / BLOCK) * BLOCK;
        let requested = if let Some(value) = requested_mib {
            if !value.is_finite() || value <= 0.0 || value > u64::MAX as f64 / MIB as f64 {
                return Err("memoryLimitMiB must be finite and positive".into());
            }
            ((value as u64).saturating_mul(MIB) / BLOCK) * BLOCK
        } else { maximum_bytes };
        let limit_bytes = requested.min(maximum_bytes);
        let slots = u64::from(width.div_ceil(64)) * u64::from(height.div_ceil(64)) * 8;
        // Include hash arrays, coordinate/dirty/capture arrays and Vec growth.
        let metadata = slots.checked_mul(256).ok_or("paint metadata capacity overflow")?;
        let scale = if width.max(height) <= 4096 { 1 } else if width.max(height) <= 8192 { 2 } else { 4 };
        // Page raster, native raster and one bounded publication buffer.
        let display = u64::from(width.div_ceil(scale)) * u64::from(height.div_ceil(scale)) * 4 * 3;
        // History plus commit staging, renderer/codec reserve and pool scratch.
        let reserved_bytes = metadata + display + (128 + 128 + threads.min(16) as u64 * 8) * MIB;
        let tile_bytes = limit_bytes.checked_sub(reserved_bytes).ok_or("paint memory is below the fixed reserve")?;
        if tile_bytes < BLOCK { return Err("paint memory leaves no tile growth block".into()); }
        let maximum_tiles = usize::try_from((tile_bytes / TILE_BYTES).min(slots))
            .map_err(|_| "paint tile capacity exceeds the address space")?;
        Ok(Self { maximum_bytes, limit_bytes, reserved_bytes, maximum_tiles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_ram_is_a_ceiling_and_admission_keeps_all_reserves() {
        let budget = Budget::new(64 * 1024 * MIB, None, 16384, 16384, 16).unwrap();
        assert_eq!(budget.maximum_bytes, 32 * 1024 * MIB);
        assert_eq!(budget.limit_bytes, budget.maximum_bytes);
        assert_eq!(budget.maximum_tiles, 8 * 256 * 256);
        assert!(budget.reserved_bytes + budget.maximum_tiles as u64 * TILE_BYTES <= budget.limit_bytes);
        let smaller = Budget::new(64 * 1024 * MIB, Some(1536.0), 16384, 16384, 16).unwrap();
        assert_eq!(smaller.limit_bytes, 1536 * MIB);
        assert!(smaller.maximum_tiles < budget.maximum_tiles);
        assert!(smaller.reserved_bytes + smaller.maximum_tiles as u64 * TILE_BYTES <= smaller.limit_bytes);
    }
    #[test]
    fn rejects_invalid_or_insufficient_ram_and_clamps_to_installed_capacity() {
        for request in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(Budget::new(8 * 1024 * MIB, Some(request), 2048, 2048, 4).is_err());
        }
        assert!(Budget::new(0, None, 2048, 2048, 4).is_err());
        assert!(Budget::new(512 * MIB, None, 2048, 2048, 4).is_err());
        let budget = Budget::new(8 * 1024 * MIB, Some(65536.0), 2048, 2048, 4).unwrap();
        assert_eq!(budget.limit_bytes, 4096 * MIB);
    }
    #[test]
    fn native_system_reports_a_nonzero_capacity() {
        assert!(installed_bytes().unwrap() > 0);
    }
}
