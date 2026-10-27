//! Shared device-local VRAM budget.
//!
//! Texture streaming and any later GPU user read the same numbers. The texture
//! target is `heapBudget` minus the other engine allocations minus headroom.
//! `VK_EXT_memory_budget` supplies `heapBudget` and `heapUsage`. A device
//! without the extension uses half the device-local heap size.

/// Default headroom is 5% of `heapBudget`, and at least 256 MB.
pub const DEFAULT_HEADROOM_PERCENT: u32 = 5;
pub const DEFAULT_HEADROOM_FLOOR_MB: u32 = 256;
/// Missing `VK_EXT_memory_budget`: half the device-local heap.
pub const FALLBACK_HEAP_DIVISOR: u64 = 2;
/// `VK_ERROR_OUT_OF_DEVICE_MEMORY`.
pub const DEVICE_OOM: i32 = -2;

/// How much of `heapBudget` to leave unused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Headroom {
    pub percent: u32,
    pub floor_bytes: u64,
}

impl Default for Headroom {
    fn default() -> Self {
        Self {
            percent: DEFAULT_HEADROOM_PERCENT,
            floor_bytes: u64::from(DEFAULT_HEADROOM_FLOOR_MB) * 1024 * 1024,
        }
    }
}

impl Headroom {
    #[must_use]
    pub fn from_mb(percent: u32, floor_mb: u32) -> Self {
        Self {
            percent,
            floor_bytes: u64::from(floor_mb) * 1024 * 1024,
        }
    }
}

/// One heap from the budget query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapSlice {
    pub budget: u64,
    pub usage: u64,
    pub device_local: bool,
}

/// What the device will hold this frame, after headroom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub heap_budget: u64,
    pub heap_usage: u64,
    /// Lighting, render targets, meshes, and anything that is not a texture mip.
    pub non_texture: u64,
    pub headroom: u64,
    pub extension: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self::from_heap_size(0, 0, 0, Headroom::default())
    }
}

impl Budget {
    /// Sum the device-local heaps. `extension` is false on the half-heap fallback.
    #[must_use]
    pub fn from_heaps(
        heaps: &[HeapSlice],
        non_texture: u64,
        headroom: Headroom,
        extension: bool,
    ) -> Self {
        let mut heap_budget = 0u64;
        let mut heap_usage = 0u64;
        for heap in heaps {
            if heap.device_local {
                heap_budget = heap_budget.saturating_add(heap.budget);
                heap_usage = heap_usage.saturating_add(heap.usage);
            }
        }
        Self::assemble(heap_budget, heap_usage, non_texture, headroom, extension)
    }

    /// No `VK_EXT_memory_budget`. The budget is half the device-local heap.
    #[must_use]
    pub fn from_heap_size(
        device_local_bytes: u64,
        tracked_usage: u64,
        non_texture: u64,
        headroom: Headroom,
    ) -> Self {
        Self::assemble(
            device_local_bytes / FALLBACK_HEAP_DIVISOR,
            tracked_usage,
            non_texture,
            headroom,
            false,
        )
    }

    fn assemble(
        heap_budget: u64,
        heap_usage: u64,
        non_texture: u64,
        headroom: Headroom,
        extension: bool,
    ) -> Self {
        Self {
            heap_budget,
            heap_usage,
            non_texture,
            headroom: headroom_bytes(heap_budget, headroom),
            extension,
        }
    }

    /// Bytes left for texture mips.
    #[must_use]
    pub fn texture_target(self) -> u64 {
        self.heap_budget
            .saturating_sub(self.non_texture)
            .saturating_sub(self.headroom)
    }

    /// Bytes the whole engine may use. Later systems (the lighting cache) read this.
    #[must_use]
    pub fn ceiling(self) -> u64 {
        self.heap_budget.saturating_sub(self.headroom)
    }

    /// True when `texture_bytes + upload` still sits on the texture target.
    #[must_use]
    pub fn admits(self, texture_bytes: u64, upload: u64) -> bool {
        texture_bytes.saturating_add(upload) <= self.texture_target()
    }

    /// Usage is past `heapBudget`. Dwell does not hold an eviction back.
    #[must_use]
    pub fn critical(self, texture_bytes: u64) -> bool {
        let engine = texture_bytes.saturating_add(self.non_texture);
        engine > self.heap_budget || self.heap_usage > self.heap_budget
    }
}

#[must_use]
pub fn headroom_bytes(budget: u64, headroom: Headroom) -> u64 {
    let percent = budget.saturating_mul(u64::from(headroom.percent)) / 100;
    percent.max(headroom.floor_bytes)
}

/// A texture allocation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// `VK_ERROR_OUT_OF_DEVICE_MEMORY`.
    Oom,
    Failed,
}

#[must_use]
pub fn is_device_oom(code: i32) -> bool {
    code == DEVICE_OOM
}

/// Try `allocate`. On [`AllocError::Oom`], call `evict` and try again.
/// `evict` returns false when nothing more can leave. This does not panic.
pub fn retry_device_alloc<T>(
    mut allocate: impl FnMut() -> Result<T, AllocError>,
    mut evict: impl FnMut() -> bool,
    tries: u32,
) -> Result<T, AllocError> {
    let mut left = tries;
    loop {
        match allocate() {
            Ok(value) => return Ok(value),
            Err(AllocError::Oom) if left > 0 && evict() => left = left.saturating_sub(1),
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(n: u64) -> u64 {
        n * 1024 * 1024
    }

    #[test]
    fn headroom_is_the_larger_of_five_percent_and_the_floor() {
        let headroom = Headroom::default();
        assert_eq!(headroom_bytes(mb(1000), headroom), mb(256));
        assert_eq!(headroom_bytes(mb(10_000), headroom), mb(500));
    }

    #[test]
    fn a_smaller_heap_budget_lowers_the_texture_target() {
        let headroom = Headroom::from_mb(5, 0);
        let wide = Budget::from_heaps(
            &[HeapSlice {
                budget: mb(4096),
                usage: mb(512),
                device_local: true,
            }],
            mb(200),
            headroom,
            true,
        );
        let tight = Budget::from_heaps(
            &[HeapSlice {
                budget: mb(1024),
                usage: mb(800),
                device_local: true,
            }],
            mb(200),
            headroom,
            true,
        );
        assert!(tight.texture_target() < wide.texture_target());
        assert!(tight.heap_usage > tight.heap_budget / 2);
    }

    #[test]
    fn a_missing_extension_uses_half_the_heap() {
        let budget = Budget::from_heap_size(mb(4096), mb(100), mb(200), Headroom::from_mb(0, 0));
        assert!(!budget.extension);
        assert_eq!(budget.heap_budget, mb(2048));
        assert_eq!(budget.texture_target(), mb(2048 - 200));
    }

    #[test]
    fn an_oom_retries_after_an_eviction_and_does_not_panic() {
        let armed = std::cell::Cell::new(true);
        let evicted = std::cell::Cell::new(0u32);
        let got = retry_device_alloc(
            || {
                if armed.get() {
                    Err(AllocError::Oom)
                } else {
                    Ok(7u32)
                }
            },
            || {
                evicted.set(evicted.get().saturating_add(1));
                armed.set(false);
                true
            },
            4,
        );
        assert_eq!(got, Ok(7));
        assert_eq!(evicted.get(), 1);
        let stuck = retry_device_alloc(|| Err::<u32, _>(AllocError::Oom), || false, 4);
        assert_eq!(stuck, Err(AllocError::Oom));
    }
}
