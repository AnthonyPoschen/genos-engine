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
    live: Vec<(usize, u64, u32)>,
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
    _next: *const c_void,
    size: u64,
    index: u32,
}

fn book_alloc(memory: Handle, size: u64, index: u32) {
    let mut book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    book.live.push((memory.addr(), size, index));
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

fn book_free(memory: Handle) {
    let mut book = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = book.live.iter().position(|entry| entry.0 == memory.addr()) {
        let (_, size, _) = book.live.swap_remove(at);
        book.live_bytes = book.live_bytes.saturating_sub(size);
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
    for (index, bytes) in book.by_type.iter().enumerate() {
        if *bytes > 0 {
            let _ = write!(line, " type{index}_bytes={bytes}");
        }
    }
    line
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
    let result = (real().alloc_mem)(device, info, alloc, out);
    if result == 0 && !info.is_null() && !out.is_null() {
        let info = std::ptr::read_unaligned(info.cast::<AllocInfo>());
        book_alloc(*out, info.size, info.index);
    }
    result
}

unsafe extern "system" fn counted_free_mem(device: Handle, memory: Handle, alloc: *const c_void) {
    bump(FREE_MEM);
    book_free(memory);
    (real().free_mem)(device, memory, alloc);
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
    (real().destroy_buffer)(device, buffer, alloc);
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

unsafe extern "system" fn counted_destroy_image(device: Handle, image: Handle, alloc: *const c_void) {
    bump(DESTROY_IMAGE);
    (real().destroy_image)(device, image, alloc);
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
    (real().destroy_image_view)(device, view, alloc);
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

unsafe extern "system" fn counted_destroy_fence(device: Handle, fence: Handle, alloc: *const c_void) {
    bump(DESTROY_FENCE);
    (real().destroy_fence)(device, fence, alloc);
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
    (real().destroy_sem)(device, sem, alloc);
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

unsafe extern "system" fn counted_destroy_query(device: Handle, pool: Handle, alloc: *const c_void) {
    bump(DESTROY_QUERY);
    (real().destroy_query_pool)(device, pool, alloc);
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
    (real().destroy_swapchain)(device, swapchain, alloc);
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
    (real().destroy_framebuffer)(device, fb, alloc);
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
    (real().destroy_shader)(device, shader, alloc);
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
    (real().destroy_pipeline)(device, pipe, alloc);
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
    (real().destroy_desc_pool)(device, pool, alloc);
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
    (real().map_mem)(device, memory, offset, size, flags, out)
}

unsafe extern "system" fn counted_unmap(device: Handle, memory: Handle) {
    bump(UNMAP_MEM);
    (real().unmap_mem)(device, memory);
}
