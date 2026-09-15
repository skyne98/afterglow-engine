use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};

const BLOCK_BYTES: u64 = 64 * 1024 * 1024;
const MEMORY_QUERY_ERROR: &str = "cannot read installed RAM for native memory admission";

/// A fixed ceiling for bytes admitted by native engine code.
pub struct EngineMemory {
    limit: u64,
    used: AtomicU64,
    peak: AtomicU64,
}

/// A snapshot of one engine memory ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stats {
    pub limit: u64,
    pub used: u64,
    pub peak: u64,
}

/// An admitted byte count released when the token is dropped.
pub struct Reservation {
    memory: Arc<EngineMemory>,
    bytes: u64,
}

impl EngineMemory {
    /// Creates a shared byte-admission ceiling.
    pub fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        })
    }

    /// Admits bytes without allocating on failure.
    pub fn reserve(self: &Arc<Self>, bytes: u64) -> Option<Reservation> {
        if bytes > self.limit {
            return None;
        }

        let mut used = self.used.load(Ordering::Acquire);
        loop {
            let next = used.checked_add(bytes)?;
            if next > self.limit {
                return None;
            }

            match self.used.compare_exchange(
                used,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.record_peak(next);
                    return Some(Reservation {
                        memory: Arc::clone(self),
                        bytes,
                    });
                }
                Err(current) => used = current,
            }
        }
    }

    /// Returns the current usage and high-water mark.
    pub fn stats(&self) -> Stats {
        Stats {
            limit: self.limit,
            used: self.used.load(Ordering::Acquire),
            peak: self.peak.load(Ordering::Acquire),
        }
    }

    fn record_peak(&self, candidate: u64) {
        let mut peak = self.peak.load(Ordering::Relaxed);
        while candidate > peak {
            match self.peak.compare_exchange(
                peak,
                candidate,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(current) => peak = current,
            }
        }
    }
}

impl Reservation {
    /// Returns the exact admitted byte count.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.memory.used.fetch_sub(self.bytes, Ordering::Release);
    }
}

/// Returns installed physical memory for supported native platforms.
pub fn installed_bytes() -> Result<u64, &'static str> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // sysconf returns the physical page count and page size.
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if pages > 0 && page_size > 0 {
            return (pages as u64)
                .checked_mul(page_size as u64)
                .ok_or(MEMORY_QUERY_ERROR);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut bytes = 0u64;
        let mut length = std::mem::size_of::<u64>();
        // sysctlbyname writes the total into this live output buffer.
        let result = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if result == 0 && length == std::mem::size_of::<u64>() && bytes > 0 {
            return Ok(bytes);
        }
    }

    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{
            GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };

        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // GlobalMemoryStatusEx fills the initialized output structure.
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 && status.ullTotalPhys > 0 {
            return Ok(status.ullTotalPhys);
        }
    }

    Err(MEMORY_QUERY_ERROR)
}

static PROCESS_MEMORY: OnceLock<Result<Arc<EngineMemory>, &'static str>> = OnceLock::new();

/// Returns the process-wide half-installed-memory admission ceiling.
pub fn process() -> Result<&'static Arc<EngineMemory>, &'static str> {
    match PROCESS_MEMORY.get_or_init(|| {
        let installed = installed_bytes()?;
        let limit = (installed / 2 / BLOCK_BYTES) * BLOCK_BYTES;
        Ok(EngineMemory::new(limit))
    }) {
        Ok(memory) => Ok(memory),
        Err(error) => Err(*error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_admission_is_a_real_zero_token() {
        let memory = EngineMemory::new(0);
        let reservation = memory.reserve(0).expect("zero bytes fit");
        assert_eq!(reservation.bytes(), 0);
        assert_eq!(memory.stats(), Stats { limit: 0, used: 0, peak: 0 });
        assert!(memory.reserve(1).is_none());
    }

    #[test]
    fn overflow_and_limit_fail_without_usage_change() {
        let memory = EngineMemory::new(u64::MAX);
        let all = memory.reserve(u64::MAX).expect("maximum fits once");
        assert!(memory.reserve(1).is_none());
        assert!(memory.reserve(u64::MAX).is_none());
        assert_eq!(memory.stats().used, u64::MAX);
        drop(all);
        assert_eq!(memory.stats().used, 0);
    }

    #[test]
    fn release_returns_exact_bytes() {
        let memory = EngineMemory::new(10);
        let first = memory.reserve(3).unwrap();
        let second = memory.reserve(4).unwrap();
        assert_eq!(first.bytes(), 3);
        assert_eq!(second.bytes(), 4);
        assert_eq!(memory.stats().used, 7);
        drop(first);
        assert_eq!(memory.stats().used, 4);
        drop(second);
        assert_eq!(memory.stats().used, 0);
        assert_eq!(memory.stats().peak, 7);
    }

    #[test]
    fn shared_concurrent_admission_never_exceeds_limit() {
        let memory = EngineMemory::new(64);
        let mut threads = Vec::new();
        for _ in 0..8 {
            let memory = Arc::clone(&memory);
            threads.push(std::thread::spawn(move || {
                let mut reservations = Vec::new();
                for _ in 0..16 {
                    if let Some(reservation) = memory.reserve(1) {
                        reservations.push(reservation);
                    }
                }
                reservations
            }));
        }

        let mut held = Vec::new();
        for thread in threads {
            held.extend(thread.join().expect("admission thread completed"));
        }
        assert_eq!(held.len(), 64);
        assert_eq!(memory.stats().used, 64);
        assert_eq!(memory.stats().peak, 64);
        drop(held);
        assert_eq!(memory.stats().used, 0);
    }

    #[test]
    fn system_query_reports_installed_memory() {
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "windows"
        ))]
        assert!(installed_bytes().expect("installed memory query") > 0);

        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "windows"
        )))]
        assert!(installed_bytes().is_err());
    }

    #[test]
    fn process_uses_the_approved_capacity_floor() {
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "windows"
        ))]
        {
            let installed = installed_bytes().expect("installed memory query");
            let memory = process().expect("process memory query");
            assert_eq!(memory.stats().limit, (installed / 2 / BLOCK_BYTES) * BLOCK_BYTES);
        }
    }
}
