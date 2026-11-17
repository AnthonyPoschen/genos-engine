//! Process-wide counts of Vulkan creates, allocates, and per-frame recording
//! calls. `vkDestroyDevice` on NVIDIA walks driver state that validation never
//! sees, so a flat steady-state count is the check that quit will not grow
//! with the frame count.
//!
//! The last device to load entry points owns the wrappers. Every device in
//! this process shares one counter set. Tests that open a device run in their
//! own process.

use std::ffi::c_void;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use super::{Fns, Handle, VkResult};

const ALLOC_MEM: usize = 0;
const FREE_MEM: usize = 1;
const CREATE_BUFFER: usize = 2;
const DESTROY_BUFFER: usize = 3;
const CREATE_IMAGE: usize = 4;
const DESTROY_IMAGE: usize = 5;
const CREATE_IMAGE_VIEW: usize = 6;
const DESTROY_IMAGE_VIEW: usize = 7;
const CREATE_FENCE: usize = 8;
const DESTROY_FENCE: usize = 9;
const CREATE_SEM: usize = 10;
const DESTROY_SEM: usize = 11;
const CREATE_QUERY: usize = 12;
const DESTROY_QUERY: usize = 13;
const CREATE_SWAPCHAIN: usize = 14;
const DESTROY_SWAPCHAIN: usize = 15;
const CREATE_FB: usize = 16;
const DESTROY_FB: usize = 17;
const CREATE_SHADER: usize = 18;
const DESTROY_SHADER: usize = 19;
const CREATE_PIPE: usize = 20;
const DESTROY_PIPE: usize = 21;
const CREATE_DESC_POOL: usize = 22;
const DESTROY_DESC_POOL: usize = 23;
const ALLOC_DESC: usize = 24;
const ALLOC_CMD: usize = 25;
const UPDATE_DESC: usize = 26;
const CMD_UPDATE: usize = 27;
const CMD_UPDATE_BYTES: usize = 28;
const BEGIN_CMD: usize = 29;
const ONE_TIME: usize = 30;
const RESET_CMD: usize = 31;
const MAP_MEM: usize = 32;
const UNMAP_MEM: usize = 33;
const N: usize = 34;

static COUNTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

/// Bytes per `VkDeviceMemory` handle, so the quit line can say how much the
/// driver handed out over the run, not just how many calls it took.
static MEMORY: Mutex<MemoryBook> = Mutex::new(MemoryBook::new());

struct MemoryBook {
    /// Address, size, memory type, and whether the allocate info had no
    /// `pNext` chain (only those can be parked and handed out again).
    live: Vec<(usize, u64, u32, bool)>,
    live_bytes: u64,
    peak_bytes: u64,
    total_bytes: u64,
    largest: u64,
    by_type: [u64; 32],
}

impl MemoryBook {
    const fn new() -> Self {
        Self {
            live: Vec::new(),
            live_bytes: 0,
            peak_bytes: 0,
            total_bytes: 0,
            largest: 0,
            by_type: [0; 32],
        }
    }
}

/// Info layout shared by every `VkMemoryAllocateInfo`.
#[repr(C)]
struct AllocInfo {
    _s_type: i32,
    next: *const c_void,
    size: u64,
    index: u32,
}

fn book_alloc(memory: Handle, size: u64, index: u32, plain: bool) {
    let mut book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    book.live.push((memory.addr(), size, index, plain));
    book.live_bytes = book.live_bytes.saturating_add(size);
    book.peak_bytes = book.peak_bytes.max(book.live_bytes);
    book.total_bytes = book.total_bytes.saturating_add(size);
    book.largest = book.largest.max(size);
    if let Some(slot) = usize::try_from(index)
        .ok()
        .and_then(|i| book.by_type.get_mut(i))
    {
        *slot = slot.saturating_add(size);
    }
}

/// Size and memory type of a plain allocation, `None` for one with a `pNext`
/// chain or one the book never saw.
fn book_free(memory: Handle) -> Option<(u64, u32)> {
    let mut book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    let at = book
        .live
        .iter()
        .position(|entry| entry.0 == memory.addr())?;
    let (_, size, index, plain) = book.live.swap_remove(at);
    book.live_bytes = book.live_bytes.saturating_sub(size);
    plain.then_some((size, index))
}

/// Freed device memory kept for the next allocate of the same size and type.
///
/// On the 4070 `vkDestroyDevice` took 16 to 25 s with nothing live, and the
/// time grew with the bytes allocated over the run (1.37 GB to 16 s, 2.56 GB
/// to 25 s), so the driver pays at teardown for every allocate and free the
/// run made. Streaming frees and reallocates the same page sizes over and
/// over. Parking a freed block and handing it back skips the driver for the
/// repeat. Compare against the commit before this one for an A/B run.
static PARKED: Mutex<Parked> = Mutex::new(Parked::new());

/// Most bytes held parked. Older blocks go back to the driver past this.
const PARK_CAP_BYTES: u64 = 256 << 20;

struct Parked {
    /// Device, memory, size, memory type. Oldest first.
    blocks: Vec<(usize, usize, u64, u32)>,
    bytes: u64,
    /// Memory held mapped by the driver, and its whole-range base pointer.
    /// The first `vkMapMemory` maps the whole block and keeps it mapped until
    /// `vkFreeMemory`. Later maps return base plus offset and `vkUnmapMemory`
    /// never reaches the driver. The 16:40 4070 run had ~3 map/unmap pairs a
    /// frame as the only driver traffic growing with frames, alongside a
    /// vkDestroyDevice stall of ~73 ms per frame.
    mapped: Vec<(usize, usize)>,
    driver_allocs: u64,
    driver_frees: u64,
    driver_bytes: u64,
    reused: u64,
}

impl Parked {
    const fn new() -> Self {
        Self {
            blocks: Vec::new(),
            bytes: 0,
            mapped: Vec::new(),
            driver_allocs: 0,
            driver_frees: 0,
            driver_bytes: 0,
            reused: 0,
        }
    }
}

fn parked() -> std::sync::MutexGuard<'static, Parked> {
    PARKED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A parked block of exactly `size` bytes and memory type `index`.
fn take_parked(device: Handle, size: u64, index: u32) -> Option<Handle> {
    let mut park = parked();
    let at = park
        .blocks
        .iter()
        .position(|b| b.0 == device.addr() && b.2 == size && b.3 == index)?;
    let (_, memory, _, _) = park.blocks.remove(at);
    park.bytes = park.bytes.saturating_sub(size);
    park.reused += 1;
    Some(device.with_addr(memory))
}

/// Hand every parked block of `device` back to the driver.
pub(super) fn free_parked(device: Handle) {
    let freed: Vec<usize> = {
        let mut park = parked();
        let mut freed = Vec::new();
        let mut kept = Vec::new();
        for block in std::mem::take(&mut park.blocks) {
            if block.0 == device.addr() {
                park.bytes = park.bytes.saturating_sub(block.2);
                forget_mapping(&mut park, block.1);
                freed.push(block.1);
            } else {
                kept.push(block);
            }
        }
        park.blocks = kept;
        park.driver_frees += u64::try_from(freed.len()).unwrap_or(u64::MAX);
        freed
    };
    let free = real().free_mem;
    for memory in freed {
        unsafe { free(device, device.with_addr(memory), std::ptr::null()) };
    }
}

/// `vk_memory` quit line: bytes allocated over the run, the live peak, what
/// is still live, the largest single allocation, and totals per memory type.
#[must_use]
pub(super) fn memory_line() -> String {
    let book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    let mut line = format!(
        "total_bytes={} peak_live_bytes={} live_bytes={} live_allocs={} largest_bytes={}",
        book.total_bytes,
        book.peak_bytes,
        book.live_bytes,
        book.live.len(),
        book.largest
    );
    drop(book);
    let park = parked();
    let _ = write!(
        line,
        " driver_allocs={} driver_frees={} driver_bytes={} reused_allocs={} parked_bytes={}",
        park.driver_allocs, park.driver_frees, park.driver_bytes, park.reused, park.bytes
    );
    drop(park);
    let book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    for (index, bytes) in book.by_type.iter().enumerate() {
        if *bytes > 0 {
            let _ = write!(line, " type{index}_bytes={bytes}");
        }
    }
    line
}

// Calls the per-type creates above do not cover. The quit stall grows about
// 73 ms per frame on camera with flat creates, so these are the per-frame
// driver calls left to rule out.
const SUBMIT: usize = 0;
const SUBMIT_INFOS: usize = 1;
const SUBMIT_CMDS: usize = 2;
const SUBMIT_SIGNALS: usize = 3;
const PRESENT: usize = 4;
const ACQUIRE: usize = 5;
const RESET_QUERY: usize = 6;
const REAL_MAP: usize = 7;
const EXTRA_N: usize = 8;
const EXTRA_NAMES: [&str; EXTRA_N] = [
    "queue_submit",
    "submit_infos",
    "submit_cmds",
    "submit_signals",
    "present",
    "acquire",
    "cmd_reset_query",
    "driver_map",
];
static EXTRA: [AtomicU64; EXTRA_N] = [const { AtomicU64::new(0) }; EXTRA_N];

const COUNT_NAMES: [&str; N] = [
    "alloc_mem",
    "free_mem",
    "create_buffer",
    "destroy_buffer",
    "create_image",
    "destroy_image",
    "create_image_view",
    "destroy_image_view",
    "create_fence",
    "destroy_fence",
    "create_sem",
    "destroy_sem",
    "create_query_pool",
    "destroy_query_pool",
    "create_swapchain",
    "destroy_swapchain",
    "create_framebuffer",
    "destroy_framebuffer",
    "create_shader",
    "destroy_shader",
    "create_pipeline",
    "destroy_pipeline",
    "create_desc_pool",
    "destroy_desc_pool",
    "alloc_desc",
    "alloc_cmd",
    "update_desc",
    "cmd_update_calls",
    "cmd_update_bytes",
    "begin_cmd",
    "one_time_begin",
    "reset_cmd",
    "map_mem",
    "unmap_mem",
];

/// Nanoseconds spent inside each destroy call, by `COUNTS` index.
static DESTROY_NS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

fn time_destroy(index: usize, start: Instant) {
    let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
    DESTROY_NS[index].fetch_add(ns, Ordering::Relaxed);
}

fn bump_extra(index: usize, n: u64) {
    EXTRA[index].fetch_add(n, Ordering::Relaxed);
}

/// Frames between `vk_traffic_rate` lines.
const RATE_EVERY: u64 = 100;

/// All counters at the last rate line, `COUNTS` then `EXTRA`.
static RATE_BASE: Mutex<Option<[u64; N + EXTRA_N]>> = Mutex::new(None);

fn all_counts() -> [u64; N + EXTRA_N] {
    let mut out = [0u64; N + EXTRA_N];
    for (slot, c) in out.iter_mut().zip(COUNTS.iter().chain(EXTRA.iter())) {
        *slot = c.load(Ordering::Relaxed);
    }
    out
}

fn count_name(i: usize) -> &'static str {
    if i < N {
        COUNT_NAMES[i]
    } else {
        EXTRA_NAMES[i - N]
    }
}

/// Hundredths as `whole.frac`, without a float cast.
fn hundredths(value: u64) -> String {
    format!("{}.{:02}", value / 100, value % 100)
}

/// Per-frame rate of every counter that moved over `frames` frames.
fn rate_text(now: &[u64; N + EXTRA_N], base: &[u64; N + EXTRA_N], frames: u64) -> String {
    let mut line = String::new();
    let frames = frames.max(1);
    for (i, (n, b)) in now.iter().zip(base.iter()).enumerate() {
        let delta = n.saturating_sub(*b);
        if delta > 0 {
            let rate = delta.saturating_mul(100) / frames;
            let _ = write!(line, " {}={}", count_name(i), hundredths(rate));
        }
    }
    line
}

/// After every [`RATE_EVERY`] presents, print what each call ran per frame
/// over that window.
fn maybe_rate_line(presents: u64) {
    if !presents.is_multiple_of(RATE_EVERY) {
        return;
    }
    let now = all_counts();
    let mut guard = RATE_BASE.lock().unwrap_or_else(PoisonError::into_inner);
    let base = guard.unwrap_or([0; N + EXTRA_N]);
    eprintln!(
        "vk_traffic_rate frame={presents} per_frame{}",
        rate_text(&now, &base, RATE_EVERY)
    );
    *guard = Some(now);
}

/// Per-frame rates over the whole run plus the uncovered call totals, for
/// the quit line.
#[must_use]
pub(super) fn rate_line() -> String {
    let now = all_counts();
    let frames = now[N + PRESENT];
    let mut line = format!("frames={frames}");
    for (i, name) in EXTRA_NAMES.iter().enumerate() {
        let _ = write!(line, " {name}={}", now[N + i]);
    }
    let _ = write!(
        line,
        " per_frame{}",
        rate_text(&now, &[0; N + EXTRA_N], frames)
    );
    line
}

/// Milliseconds spent destroying each object type over the run, so the
/// teardown can be split by type before `vkDestroyDevice`.
#[must_use]
pub(super) fn destroy_line() -> String {
    let mut line = String::new();
    for (i, (ns, count)) in DESTROY_NS.iter().zip(COUNTS.iter()).enumerate() {
        let n = count.load(Ordering::Relaxed);
        if COUNT_NAMES[i].starts_with("destroy_") && n > 0 {
            let centi_ms = ns.load(Ordering::Relaxed) / 10_000;
            let _ = write!(line, " {}={n}/{}ms", COUNT_NAMES[i], hundredths(centi_ms));
        }
    }
    line
}

#[repr(C)]
struct SubmitInfo {
    _s_type: i32,
    _next: *const c_void,
    _wait_count: u32,
    _waits: *const Handle,
    _stages: *const u32,
    cmd_count: u32,
    _cmds: *const Handle,
    signal_count: u32,
    _signals: *const Handle,
}

unsafe extern "system" fn counted_submit(
    queue: Handle,
    count: u32,
    infos: *const u8,
    fence: Handle,
) -> VkResult {
    bump_extra(SUBMIT, 1);
    bump_extra(SUBMIT_INFOS, u64::from(count));
    if !infos.is_null() {
        let infos = infos.cast::<SubmitInfo>();
        for i in 0..usize::try_from(count).unwrap_or(0) {
            let info = infos.add(i).read_unaligned();
            bump_extra(SUBMIT_CMDS, u64::from(info.cmd_count));
            bump_extra(SUBMIT_SIGNALS, u64::from(info.signal_count));
        }
    }
    (real().queue_submit)(queue, count, infos, fence)
}

unsafe extern "system" fn counted_present(queue: Handle, info: *const u8) -> VkResult {
    let result = (real().present)(queue, info);
    let presents = EXTRA[PRESENT].fetch_add(1, Ordering::Relaxed) + 1;
    maybe_rate_line(presents);
    result
}

unsafe extern "system" fn counted_acquire(
    device: Handle,
    swapchain: Handle,
    timeout: u64,
    sem: Handle,
    fence: Handle,
    out: *mut u32,
) -> VkResult {
    bump_extra(ACQUIRE, 1);
    (real().acquire)(device, swapchain, timeout, sem, fence, out)
}

unsafe extern "system" fn counted_reset_query(cmd: Handle, pool: Handle, first: u32, count: u32) {
    bump_extra(RESET_QUERY, 1);
    (real().cmd_reset_query)(cmd, pool, first, count);
}

static REAL: Mutex<Option<Fns>> = Mutex::new(None);

fn bump(index: usize) {
    COUNTS[index].fetch_add(1, Ordering::Relaxed);
}

fn add(index: usize, n: u64) {
    COUNTS[index].fetch_add(n, Ordering::Relaxed);
}

fn real() -> Fns {
    let guard = REAL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(fns) = *guard {
        fns
    } else {
        std::process::abort();
    }
}

/// Replace the counted entry points. The pointers in `fns` on entry are the
/// driver ones and are saved before the wrappers are installed.
pub(super) fn install(fns: &mut Fns) {
    let mut guard = REAL.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(*fns);
    drop(guard);
    fns.alloc_mem = counted_alloc_mem;
    fns.free_mem = counted_free_mem;
    fns.create_buffer = counted_create_buffer;
    fns.destroy_buffer = counted_destroy_buffer;
    fns.create_image = counted_create_image;
    fns.destroy_image = counted_destroy_image;
    fns.create_image_view = counted_create_view;
    fns.destroy_image_view = counted_destroy_view;
    fns.create_fence = counted_create_fence;
    fns.destroy_fence = counted_destroy_fence;
    fns.create_sem = counted_create_sem;
    fns.destroy_sem = counted_destroy_sem;
    fns.create_query_pool = counted_create_query;
    fns.destroy_query_pool = counted_destroy_query;
    fns.create_swapchain = counted_create_swapchain;
    fns.destroy_swapchain = counted_destroy_swapchain;
    fns.create_framebuffer = counted_create_fb;
    fns.destroy_framebuffer = counted_destroy_fb;
    fns.create_shader = counted_create_shader;
    fns.destroy_shader = counted_destroy_shader;
    fns.create_pipelines = counted_create_gfx;
    fns.create_compute = counted_create_compute;
    fns.destroy_pipeline = counted_destroy_pipe;
    fns.create_desc_pool = counted_create_desc_pool;
    fns.destroy_desc_pool = counted_destroy_desc_pool;
    fns.alloc_desc = counted_alloc_desc;
    fns.alloc_cmd = counted_alloc_cmd;
    fns.update_desc = counted_update_desc;
    fns.cmd_update_buffer = counted_cmd_update;
    fns.begin_cmd = counted_begin;
    fns.reset_cmd = counted_reset;
    fns.map_mem = counted_map;
    fns.unmap_mem = counted_unmap;
    fns.queue_submit = counted_submit;
    fns.present = counted_present;
    fns.acquire = counted_acquire;
    fns.cmd_reset_query = counted_reset_query;
}

/// One sample of the process counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VkTraffic {
    pub alloc_mem: u64,
    pub free_mem: u64,
    pub create_buffer: u64,
    pub destroy_buffer: u64,
    pub create_image: u64,
    pub destroy_image: u64,
    pub create_image_view: u64,
    pub destroy_image_view: u64,
    pub create_fence: u64,
    pub destroy_fence: u64,
    pub create_sem: u64,
    pub destroy_sem: u64,
    pub create_query_pool: u64,
    pub destroy_query_pool: u64,
    pub create_swapchain: u64,
    pub destroy_swapchain: u64,
    pub create_framebuffer: u64,
    pub destroy_framebuffer: u64,
    pub create_shader: u64,
    pub destroy_shader: u64,
    pub create_pipeline: u64,
    pub destroy_pipeline: u64,
    pub create_desc_pool: u64,
    pub destroy_desc_pool: u64,
    pub alloc_desc: u64,
    pub alloc_cmd: u64,
    pub update_desc: u64,
    pub cmd_update_calls: u64,
    pub cmd_update_bytes: u64,
    pub begin_cmd: u64,
    pub one_time_begin: u64,
    pub reset_cmd: u64,
    pub map_mem: u64,
    pub unmap_mem: u64,
}

impl VkTraffic {
    /// Counts added since `earlier`.
    #[must_use]
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            alloc_mem: self.alloc_mem.saturating_sub(earlier.alloc_mem),
            free_mem: self.free_mem.saturating_sub(earlier.free_mem),
            create_buffer: self.create_buffer.saturating_sub(earlier.create_buffer),
            destroy_buffer: self.destroy_buffer.saturating_sub(earlier.destroy_buffer),
            create_image: self.create_image.saturating_sub(earlier.create_image),
            destroy_image: self.destroy_image.saturating_sub(earlier.destroy_image),
            create_image_view: self
                .create_image_view
                .saturating_sub(earlier.create_image_view),
            destroy_image_view: self
                .destroy_image_view
                .saturating_sub(earlier.destroy_image_view),
            create_fence: self.create_fence.saturating_sub(earlier.create_fence),
            destroy_fence: self.destroy_fence.saturating_sub(earlier.destroy_fence),
            create_sem: self.create_sem.saturating_sub(earlier.create_sem),
            destroy_sem: self.destroy_sem.saturating_sub(earlier.destroy_sem),
            create_query_pool: self
                .create_query_pool
                .saturating_sub(earlier.create_query_pool),
            destroy_query_pool: self
                .destroy_query_pool
                .saturating_sub(earlier.destroy_query_pool),
            create_swapchain: self
                .create_swapchain
                .saturating_sub(earlier.create_swapchain),
            destroy_swapchain: self
                .destroy_swapchain
                .saturating_sub(earlier.destroy_swapchain),
            create_framebuffer: self
                .create_framebuffer
                .saturating_sub(earlier.create_framebuffer),
            destroy_framebuffer: self
                .destroy_framebuffer
                .saturating_sub(earlier.destroy_framebuffer),
            create_shader: self.create_shader.saturating_sub(earlier.create_shader),
            destroy_shader: self.destroy_shader.saturating_sub(earlier.destroy_shader),
            create_pipeline: self.create_pipeline.saturating_sub(earlier.create_pipeline),
            destroy_pipeline: self
                .destroy_pipeline
                .saturating_sub(earlier.destroy_pipeline),
            create_desc_pool: self
                .create_desc_pool
                .saturating_sub(earlier.create_desc_pool),
            destroy_desc_pool: self
                .destroy_desc_pool
                .saturating_sub(earlier.destroy_desc_pool),
            alloc_desc: self.alloc_desc.saturating_sub(earlier.alloc_desc),
            alloc_cmd: self.alloc_cmd.saturating_sub(earlier.alloc_cmd),
            update_desc: self.update_desc.saturating_sub(earlier.update_desc),
            cmd_update_calls: self
                .cmd_update_calls
                .saturating_sub(earlier.cmd_update_calls),
            cmd_update_bytes: self
                .cmd_update_bytes
                .saturating_sub(earlier.cmd_update_bytes),
            begin_cmd: self.begin_cmd.saturating_sub(earlier.begin_cmd),
            one_time_begin: self.one_time_begin.saturating_sub(earlier.one_time_begin),
            reset_cmd: self.reset_cmd.saturating_sub(earlier.reset_cmd),
            map_mem: self.map_mem.saturating_sub(earlier.map_mem),
            unmap_mem: self.unmap_mem.saturating_sub(earlier.unmap_mem),
        }
    }

    /// Steady state: no new driver objects and no recording that embeds a
    /// fresh host upload or a one-time command buffer.
    ///
    /// `map_mem`, `unmap_mem`, `reset_cmd`, and `begin_cmd` stay off this
    /// list. A reused host buffer and a reused command buffer do those every
    /// frame. `one_time_begin` is the begin that asks the driver to drop the
    /// recording after submit.
    #[must_use]
    pub fn is_flat(&self) -> bool {
        self.alloc_mem == 0
            && self.free_mem == 0
            && self.create_buffer == 0
            && self.destroy_buffer == 0
            && self.create_image == 0
            && self.destroy_image == 0
            && self.create_image_view == 0
            && self.destroy_image_view == 0
            && self.create_fence == 0
            && self.destroy_fence == 0
            && self.create_sem == 0
            && self.destroy_sem == 0
            && self.create_query_pool == 0
            && self.destroy_query_pool == 0
            && self.create_swapchain == 0
            && self.destroy_swapchain == 0
            && self.create_framebuffer == 0
            && self.destroy_framebuffer == 0
            && self.create_shader == 0
            && self.destroy_shader == 0
            && self.create_pipeline == 0
            && self.destroy_pipeline == 0
            && self.create_desc_pool == 0
            && self.destroy_desc_pool == 0
            && self.alloc_desc == 0
            && self.alloc_cmd == 0
            && self.update_desc == 0
            && self.cmd_update_calls == 0
            && self.one_time_begin == 0
    }

    /// `key=value` pairs, one line, no prefix.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "alloc_mem={} free_mem={} create_buffer={} destroy_buffer={} create_image={} destroy_image={} create_image_view={} destroy_image_view={} create_fence={} destroy_fence={} create_sem={} destroy_sem={} create_query_pool={} destroy_query_pool={} create_swapchain={} destroy_swapchain={} create_framebuffer={} destroy_framebuffer={} create_shader={} destroy_shader={} create_pipeline={} destroy_pipeline={} create_desc_pool={} destroy_desc_pool={} alloc_desc={} alloc_cmd={} update_desc={} cmd_update_calls={} cmd_update_bytes={} begin_cmd={} one_time_begin={} reset_cmd={} map_mem={} unmap_mem={}",
            self.alloc_mem,
            self.free_mem,
            self.create_buffer,
            self.destroy_buffer,
            self.create_image,
            self.destroy_image,
            self.create_image_view,
            self.destroy_image_view,
            self.create_fence,
            self.destroy_fence,
            self.create_sem,
            self.destroy_sem,
            self.create_query_pool,
            self.destroy_query_pool,
            self.create_swapchain,
            self.destroy_swapchain,
            self.create_framebuffer,
            self.destroy_framebuffer,
            self.create_shader,
            self.destroy_shader,
            self.create_pipeline,
            self.destroy_pipeline,
            self.create_desc_pool,
            self.destroy_desc_pool,
            self.alloc_desc,
            self.alloc_cmd,
            self.update_desc,
            self.cmd_update_calls,
            self.cmd_update_bytes,
            self.begin_cmd,
            self.one_time_begin,
            self.reset_cmd,
            self.map_mem,
            self.unmap_mem,
        )
    }
}

#[must_use]
pub(super) fn snapshot() -> VkTraffic {
    let n = |index: usize| COUNTS[index].load(Ordering::Relaxed);
    VkTraffic {
        alloc_mem: n(ALLOC_MEM),
        free_mem: n(FREE_MEM),
        create_buffer: n(CREATE_BUFFER),
        destroy_buffer: n(DESTROY_BUFFER),
        create_image: n(CREATE_IMAGE),
        destroy_image: n(DESTROY_IMAGE),
        create_image_view: n(CREATE_IMAGE_VIEW),
        destroy_image_view: n(DESTROY_IMAGE_VIEW),
        create_fence: n(CREATE_FENCE),
        destroy_fence: n(DESTROY_FENCE),
        create_sem: n(CREATE_SEM),
        destroy_sem: n(DESTROY_SEM),
        create_query_pool: n(CREATE_QUERY),
        destroy_query_pool: n(DESTROY_QUERY),
        create_swapchain: n(CREATE_SWAPCHAIN),
        destroy_swapchain: n(DESTROY_SWAPCHAIN),
        create_framebuffer: n(CREATE_FB),
        destroy_framebuffer: n(DESTROY_FB),
        create_shader: n(CREATE_SHADER),
        destroy_shader: n(DESTROY_SHADER),
        create_pipeline: n(CREATE_PIPE),
        destroy_pipeline: n(DESTROY_PIPE),
        create_desc_pool: n(CREATE_DESC_POOL),
        destroy_desc_pool: n(DESTROY_DESC_POOL),
        alloc_desc: n(ALLOC_DESC),
        alloc_cmd: n(ALLOC_CMD),
        update_desc: n(UPDATE_DESC),
        cmd_update_calls: n(CMD_UPDATE),
        cmd_update_bytes: n(CMD_UPDATE_BYTES),
        begin_cmd: n(BEGIN_CMD),
        one_time_begin: n(ONE_TIME),
        reset_cmd: n(RESET_CMD),
        map_mem: n(MAP_MEM),
        unmap_mem: n(UNMAP_MEM),
    }
}

unsafe extern "system" fn counted_alloc_mem(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(ALLOC_MEM);
    let request = (!info.is_null()).then(|| std::ptr::read_unaligned(info.cast::<AllocInfo>()));
    let plain = request.as_ref().is_some_and(|r| r.next.is_null()) && alloc.is_null();
    if let Some(request) = request.as_ref().filter(|_| plain && !out.is_null()) {
        if let Some(memory) = take_parked(device, request.size, request.index) {
            *out = memory;
            book_alloc(memory, request.size, request.index, true);
            return 0;
        }
    }
    let mut result = (real().alloc_mem)(device, info, alloc, out);
    if result == super::VK_ERROR_OUT_OF_DEVICE_MEMORY {
        // Parked blocks hold device memory the app already gave back.
        free_parked(device);
        result = (real().alloc_mem)(device, info, alloc, out);
    }
    if let Some(request) = request.as_ref().filter(|_| result == 0) {
        if !out.is_null() {
            book_alloc(*out, request.size, request.index, plain);
            let mut park = parked();
            park.driver_allocs += 1;
            park.driver_bytes = park.driver_bytes.saturating_add(request.size);
        }
    }
    result
}

unsafe extern "system" fn counted_free_mem(device: Handle, memory: Handle, alloc: *const c_void) {
    bump(FREE_MEM);
    if memory.is_null() {
        return;
    }
    let known = book_free(memory);
    if let Some((size, index)) = known.filter(|_| alloc.is_null()) {
        if size <= PARK_CAP_BYTES {
            park(device, memory, size, index);
            return;
        }
    }
    {
        let mut park = parked();
        park.driver_frees += 1;
        forget_mapping(&mut park, memory.addr());
    }
    (real().free_mem)(device, memory, alloc);
}

/// Drop the kept mapping of `memory`; `vkFreeMemory` unmaps implicitly.
fn forget_mapping(park: &mut Parked, memory: usize) {
    if let Some(at) = park.mapped.iter().position(|m| m.0 == memory) {
        park.mapped.swap_remove(at);
    }
}

/// Keep `memory` for a later allocate of the same size and type, and give
/// the oldest parked blocks back to the driver past [`PARK_CAP_BYTES`].
unsafe fn park(device: Handle, memory: Handle, size: u64, index: u32) {
    let evicted = {
        let mut park = parked();
        // A parked block stays mapped; the next owner's vkMapMemory gets the
        // same base back without a driver call.
        park.blocks
            .push((device.addr(), memory.addr(), size, index));
        park.bytes = park.bytes.saturating_add(size);
        let mut evicted = Vec::new();
        while park.bytes > PARK_CAP_BYTES && !park.blocks.is_empty() {
            let block = park.blocks.remove(0);
            park.bytes = park.bytes.saturating_sub(block.2);
            evicted.push(block);
        }
        park.driver_frees += u64::try_from(evicted.len()).unwrap_or(u64::MAX);
        for block in &evicted {
            forget_mapping(&mut park, block.1);
        }
        evicted
    };
    let fns = real();
    for (dev, mem, _, _) in evicted {
        (fns.free_mem)(
            device.with_addr(dev),
            device.with_addr(mem),
            std::ptr::null(),
        );
    }
}

unsafe extern "system" fn counted_create_buffer(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_BUFFER);
    (real().create_buffer)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_buffer(
    device: Handle,
    buffer: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_BUFFER);
    let start = Instant::now();
    (real().destroy_buffer)(device, buffer, alloc);
    time_destroy(DESTROY_BUFFER, start);
}

unsafe extern "system" fn counted_create_image(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_IMAGE);
    (real().create_image)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_image(
    device: Handle,
    image: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_IMAGE);
    let start = Instant::now();
    (real().destroy_image)(device, image, alloc);
    time_destroy(DESTROY_IMAGE, start);
}

unsafe extern "system" fn counted_create_view(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_IMAGE_VIEW);
    (real().create_image_view)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_view(device: Handle, view: Handle, alloc: *const c_void) {
    bump(DESTROY_IMAGE_VIEW);
    let start = Instant::now();
    (real().destroy_image_view)(device, view, alloc);
    time_destroy(DESTROY_IMAGE_VIEW, start);
}

unsafe extern "system" fn counted_create_fence(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_FENCE);
    (real().create_fence)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_fence(
    device: Handle,
    fence: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_FENCE);
    let start = Instant::now();
    (real().destroy_fence)(device, fence, alloc);
    time_destroy(DESTROY_FENCE, start);
}

unsafe extern "system" fn counted_create_sem(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_SEM);
    (real().create_sem)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_sem(device: Handle, sem: Handle, alloc: *const c_void) {
    bump(DESTROY_SEM);
    let start = Instant::now();
    (real().destroy_sem)(device, sem, alloc);
    time_destroy(DESTROY_SEM, start);
}

unsafe extern "system" fn counted_create_query(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_QUERY);
    (real().create_query_pool)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_query(
    device: Handle,
    pool: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_QUERY);
    let start = Instant::now();
    (real().destroy_query_pool)(device, pool, alloc);
    time_destroy(DESTROY_QUERY, start);
}

unsafe extern "system" fn counted_create_swapchain(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_SWAPCHAIN);
    (real().create_swapchain)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_swapchain(
    device: Handle,
    swapchain: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_SWAPCHAIN);
    let start = Instant::now();
    (real().destroy_swapchain)(device, swapchain, alloc);
    time_destroy(DESTROY_SWAPCHAIN, start);
}

unsafe extern "system" fn counted_create_fb(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_FB);
    (real().create_framebuffer)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_fb(device: Handle, fb: Handle, alloc: *const c_void) {
    bump(DESTROY_FB);
    let start = Instant::now();
    (real().destroy_framebuffer)(device, fb, alloc);
    time_destroy(DESTROY_FB, start);
}

unsafe extern "system" fn counted_create_shader(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_SHADER);
    (real().create_shader)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_shader(
    device: Handle,
    shader: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_SHADER);
    let start = Instant::now();
    (real().destroy_shader)(device, shader, alloc);
    time_destroy(DESTROY_SHADER, start);
}

unsafe extern "system" fn counted_create_gfx(
    device: Handle,
    cache: Handle,
    count: u32,
    infos: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_PIPE);
    (real().create_pipelines)(device, cache, count, infos, alloc, out)
}

unsafe extern "system" fn counted_create_compute(
    device: Handle,
    cache: Handle,
    count: u32,
    infos: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_PIPE);
    (real().create_compute)(device, cache, count, infos, alloc, out)
}

unsafe extern "system" fn counted_destroy_pipe(device: Handle, pipe: Handle, alloc: *const c_void) {
    bump(DESTROY_PIPE);
    let start = Instant::now();
    (real().destroy_pipeline)(device, pipe, alloc);
    time_destroy(DESTROY_PIPE, start);
}

unsafe extern "system" fn counted_create_desc_pool(
    device: Handle,
    info: *const u8,
    alloc: *const c_void,
    out: *mut Handle,
) -> VkResult {
    bump(CREATE_DESC_POOL);
    (real().create_desc_pool)(device, info, alloc, out)
}

unsafe extern "system" fn counted_destroy_desc_pool(
    device: Handle,
    pool: Handle,
    alloc: *const c_void,
) {
    bump(DESTROY_DESC_POOL);
    let start = Instant::now();
    (real().destroy_desc_pool)(device, pool, alloc);
    time_destroy(DESTROY_DESC_POOL, start);
}

unsafe extern "system" fn counted_alloc_desc(
    device: Handle,
    info: *const u8,
    out: *mut Handle,
) -> VkResult {
    bump(ALLOC_DESC);
    (real().alloc_desc)(device, info, out)
}

unsafe extern "system" fn counted_alloc_cmd(
    device: Handle,
    info: *const u8,
    out: *mut Handle,
) -> VkResult {
    bump(ALLOC_CMD);
    (real().alloc_cmd)(device, info, out)
}

unsafe extern "system" fn counted_update_desc(
    device: Handle,
    writes: u32,
    write_ptr: *const u8,
    copies: u32,
    copy_ptr: *const u8,
) {
    bump(UPDATE_DESC);
    (real().update_desc)(device, writes, write_ptr, copies, copy_ptr);
}

unsafe extern "system" fn counted_cmd_update(
    cmd: Handle,
    buffer: Handle,
    offset: u64,
    size: u64,
    data: *const c_void,
) {
    bump(CMD_UPDATE);
    add(CMD_UPDATE_BYTES, size);
    (real().cmd_update_buffer)(cmd, buffer, offset, size, data);
}

unsafe extern "system" fn counted_begin(cmd: Handle, info: *const u8) -> VkResult {
    bump(BEGIN_CMD);
    if !info.is_null() {
        #[repr(C)]
        struct BeginInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            inherit: *const c_void,
        }
        let flags = info.cast::<BeginInfo>().read_unaligned().flags;
        if flags & 1 != 0 {
            bump(ONE_TIME);
        }
    }
    (real().begin_cmd)(cmd, info)
}

unsafe extern "system" fn counted_reset(cmd: Handle, flags: u32) -> VkResult {
    bump(RESET_CMD);
    (real().reset_cmd)(cmd, flags)
}

unsafe extern "system" fn counted_map(
    device: Handle,
    memory: Handle,
    offset: u64,
    size: u64,
    flags: u32,
    out: *mut *mut c_void,
) -> VkResult {
    bump(MAP_MEM);
    let Ok(skip) = usize::try_from(offset) else {
        return (real().map_mem)(device, memory, offset, size, flags, out);
    };
    let kept = parked()
        .mapped
        .iter()
        .find(|m| m.0 == memory.addr())
        .map(|m| m.1);
    let base = if let Some(base) = kept {
        base
    } else {
        let mut base: *mut c_void = std::ptr::null_mut();
        // VK_WHOLE_SIZE from offset 0, so any later range is inside it.
        let result = (real().map_mem)(device, memory, 0, u64::MAX, flags, &mut base);
        bump_extra(REAL_MAP, 1);
        if result != 0 {
            return result;
        }
        parked().mapped.push((memory.addr(), base.expose_provenance()));
        base.expose_provenance()
    };
    if !out.is_null() {
        *out = std::ptr::with_exposed_provenance_mut::<c_void>(base.saturating_add(skip));
    }
    0
}

/// The mapping stays until `vkFreeMemory`; see [`Parked::mapped`].
unsafe extern "system" fn counted_unmap(_device: Handle, _memory: Handle) {
    bump(UNMAP_MEM);
}
