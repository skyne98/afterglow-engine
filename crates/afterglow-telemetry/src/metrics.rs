//! Fixed metric cells for always-on health telemetry.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::descriptor::{CategoryId, Unit};

pub const HISTOGRAM_BUCKETS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetricId(pub u32);

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    Counter = 1,
    Gauge = 2,
    Maximum = 3,
    HistogramLog2 = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricDescriptor {
    pub category: CategoryId,
    pub category_name: &'static str,
    pub name: &'static str,
    pub kind: MetricKind,
    pub unit: Unit,
}

impl MetricDescriptor {
    pub const fn new(
        category: CategoryId,
        category_name: &'static str,
        name: &'static str,
        kind: MetricKind,
        unit: Unit,
    ) -> Self {
        Self {
            category,
            category_name,
            name,
            kind,
            unit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricStatus {
    Updated,
    InvalidMetric,
    WrongMetricKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricSample {
    pub metric: u32,
    /// Zero for scalar metrics; 0..31 for logarithmic histograms.
    pub bucket: u8,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricSnapshotError {
    OutputTooSmall { needed: usize, provided: usize },
}

/// Fixed atomic storage. Construction computes one direct cell offset per
/// metric; updates never allocate or lock.
pub struct MetricBank {
    descriptors: &'static [MetricDescriptor],
    offsets: Box<[u32]>,
    cells: Box<[AtomicU64]>,
    sample_count: usize,
}

impl MetricBank {
    pub fn new(descriptors: &'static [MetricDescriptor]) -> Self {
        let mut offsets = Vec::with_capacity(descriptors.len());
        let mut cell_count = 0_usize;
        for descriptor in descriptors {
            offsets.push(cell_count as u32);
            cell_count += if descriptor.kind == MetricKind::HistogramLog2 {
                HISTOGRAM_BUCKETS
            } else {
                1
            };
        }
        let cells = (0..cell_count)
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            descriptors,
            offsets: offsets.into_boxed_slice(),
            cells,
            sample_count: cell_count,
        }
    }

    pub fn descriptors(&self) -> &'static [MetricDescriptor] {
        self.descriptors
    }

    pub fn required_sample_capacity(&self) -> usize {
        self.sample_count
    }

    #[inline]
    pub fn counter_add(&self, metric: MetricId, delta: u64) -> MetricStatus {
        let Some(cell) = self.scalar_cell(metric, MetricKind::Counter) else {
            return self.invalid_or_wrong(metric, MetricKind::Counter);
        };
        cell.fetch_add(delta, Ordering::Relaxed);
        MetricStatus::Updated
    }

    #[inline]
    pub fn gauge_set(&self, metric: MetricId, value: i64) -> MetricStatus {
        let Some(cell) = self.scalar_cell(metric, MetricKind::Gauge) else {
            return self.invalid_or_wrong(metric, MetricKind::Gauge);
        };
        cell.store(value as u64, Ordering::Relaxed);
        MetricStatus::Updated
    }

    #[inline]
    pub fn gauge_set_f64(&self, metric: MetricId, value: f64) -> MetricStatus {
        let Some(cell) = self.scalar_cell(metric, MetricKind::Gauge) else {
            return self.invalid_or_wrong(metric, MetricKind::Gauge);
        };
        cell.store(value.to_bits(), Ordering::Relaxed);
        MetricStatus::Updated
    }

    #[inline]
    pub fn maximum(&self, metric: MetricId, value: u64) -> MetricStatus {
        let Some(cell) = self.scalar_cell(metric, MetricKind::Maximum) else {
            return self.invalid_or_wrong(metric, MetricKind::Maximum);
        };
        cell.fetch_max(value, Ordering::Relaxed);
        MetricStatus::Updated
    }

    #[inline]
    pub fn histogram_log2(&self, metric: MetricId, value: u64) -> MetricStatus {
        let index = metric.0 as usize;
        let Some(descriptor) = self.descriptors.get(index) else {
            return MetricStatus::InvalidMetric;
        };
        if descriptor.kind != MetricKind::HistogramLog2 {
            return MetricStatus::WrongMetricKind;
        }
        let bucket = if value == 0 {
            0
        } else {
            (63 - value.leading_zeros() as usize).min(HISTOGRAM_BUCKETS - 1)
        };
        let offset = self.offsets[index] as usize + bucket;
        self.cells[offset].fetch_add(1, Ordering::Relaxed);
        MetricStatus::Updated
    }

    pub fn snapshot_into(&self, output: &mut [MetricSample]) -> Result<usize, MetricSnapshotError> {
        if output.len() < self.sample_count {
            return Err(MetricSnapshotError::OutputTooSmall {
                needed: self.sample_count,
                provided: output.len(),
            });
        }
        let mut cursor = 0;
        for (metric, descriptor) in self.descriptors.iter().enumerate() {
            let count = if descriptor.kind == MetricKind::HistogramLog2 {
                HISTOGRAM_BUCKETS
            } else {
                1
            };
            let offset = self.offsets[metric] as usize;
            for bucket in 0..count {
                output[cursor] = MetricSample {
                    metric: metric as u32,
                    bucket: bucket as u8,
                    value: self.cells[offset + bucket].load(Ordering::Relaxed),
                };
                cursor += 1;
            }
        }
        Ok(cursor)
    }

    fn scalar_cell(&self, metric: MetricId, kind: MetricKind) -> Option<&AtomicU64> {
        let index = metric.0 as usize;
        let descriptor = self.descriptors.get(index)?;
        if descriptor.kind != kind {
            return None;
        }
        self.cells.get(self.offsets[index] as usize)
    }

    fn invalid_or_wrong(&self, metric: MetricId, expected: MetricKind) -> MetricStatus {
        match self.descriptors.get(metric.0 as usize) {
            None => MetricStatus::InvalidMetric,
            Some(descriptor) if descriptor.kind != expected => MetricStatus::WrongMetricKind,
            Some(_) => MetricStatus::InvalidMetric,
        }
    }
}
