#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::assigning_clones, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::bool_to_int_with_if, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::borrow_as_ptr, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_wrap, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_ptr_alignment, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::field_reassign_with_default, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::fn_params_excessive_bools, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::if_not_else, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::items_after_statements, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_assert, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_c_str_literals, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_slice_size_calculation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::many_single_char_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::needless_range_loop, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::panic, reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md")]
#![expect(clippy::precedence, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::ptr_as_ptr, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::redundant_locals, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::similar_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::struct_excessive_bools, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::struct_field_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::too_many_arguments, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::undocumented_unsafe_blocks, reason = "TODO(lint-2026-10-14): Vulkan or OS FFI; add a SAFETY comment. docs/LINT_RULES.md")]
#![expect(clippy::unnecessary_wraps, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::unreadable_literal, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::unwrap_used, reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md")]
#![expect(clippy::useless_vec, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

//! Vulkan loader and one forward pass. Entry points come from libvulkan.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use genos_scene::{view_proj, Camera, LightingPreset};

use crate::aa::Antialias;
use crate::budget::FrameMemory;
use crate::pack::{self, GpuVertex, Pack};
use crate::world::World;

include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

mod fog;
mod media_check;

#[path = "debug_view.rs"]
mod debug_view;
pub use crate::unfilled::{UnfilledControl, UnfilledStats};
pub use debug_view::{DebugView, LampReport, ViewMode};

/// Vulkan 1.0 (`VK_API_VERSION_1_0`). Every shader module has to be SPIR-V 1.0.
/// The plain a-trous kernel is the one built by glslang; its target env stays
/// `vulkan1.0` so it does not come out as SPIR-V 1.6.
const API_VERSION: u32 = 1 << 22;
const VK_SUCCESS: i32 = 0;
const VK_NOT_READY: i32 = 1;
const VK_TIMEOUT: i32 = 2;
/// `VK_ERROR_OUT_OF_DEVICE_MEMORY`.
const VK_ERROR_OUT_OF_DEVICE_MEMORY: i32 = -2;
const FRAME_BUDGET_MS: f32 = 16.0;
/// A fence wait on the frame thread before the seat pump takes over.
const FENCE_BUDGET_NS: u64 = 50_000_000;
/// A mapped copy above this runs while the seat pump answers pings.
const COPY_BUDGET: usize = 1_048_576;

thread_local! {
    static ENTER: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
    static LEAVE: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}

struct SeatLeave;

impl Drop for SeatLeave {
    fn drop(&mut self) {
        if let Some(leave) = LEAVE.with(std::cell::Cell::get) {
            leave();
        }
    }
}

/// Run `body` while the installed seat pump answers compositor pings.
fn block<T>(body: impl FnOnce() -> T) -> T {
    let Some(enter) = ENTER.with(std::cell::Cell::get) else {
        return body();
    };
    enter();
    let _leave = SeatLeave;
    body()
}

/// Copy `len` bytes into a mapped GPU allocation. A large copy yields the seat.
fn copy_to_mapped(dst: *mut u8, src: *const u8, len: usize) {
    if dst.is_null() || len == 0 {
        return;
    }
    if len > COPY_BUDGET {
        block(|| unsafe { std::ptr::copy_nonoverlapping(src, dst, len) });
    } else {
        unsafe { std::ptr::copy_nonoverlapping(src, dst, len) };
    }
}
const SUBOPTIMAL: i32 = 1000001003;
const OUT_OF_DATE: i32 = -1000001004;
type VkResult = i32;
type Handle = *mut c_void;
type Pfn = *const c_void;

/// Live streamer counters. `budget` is the texture target for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialStats {
    pub committed: u64,
    pub budget: u64,
    pub refused: u32,
    pub evictions: u32,
    pub thrash: u32,
    pub holes: u32,
    pub rate: u64,
    pub stream_ms: f32,
}

impl MaterialStats {
    /// One line for the stress log and the finish line.
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "materials committed={} budget={} refused={} evictions={} thrash={} holes={} rate={} stream_ms={:.3}",
            self.committed,
            self.budget,
            self.refused,
            self.evictions,
            self.thrash,
            self.holes,
            self.rate,
            self.stream_ms,
        )
    }
}

pub struct Renderer {
    gpu: Gpu,
    width: u32,
    height: u32,
    wire_on: bool,
    memory: FrameMemory,
    shapes: crate::pool::MeshPool,
    /// Last overlay list. The same list reuses its vertices.
    overlay_rects: Vec<ScreenRect>,
    /// Lines drawn with the overlay rectangles, in the same draw.
    overlay_lines: Vec<ScreenLine>,
    overlay_verts: Vec<crate::pack::GpuVertex>,
    /// Every frame copies its picture into host memory, for `read_earlier_frame`.
    keep_pictures: bool,
    /// Debug view and lighting overrides ([`debug_view`]).
    debug: debug_view::DebugState,
    /// One wait inside `poll_idle`, planted by the quit harness.
    ///
    /// Real fences signal immediately on lavapipe. This wait is the slow fence
    /// the harness uses so a blocked drain still fails the 100 ms budget.
    slow_fence: Option<Duration>,
}

impl Renderer {
    pub fn open(
        display: *mut c_void,
        surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<(Self, genos_window::ShutdownPermit), String> {
        let gpu = Gpu::open(display, surface, width, height)?;
        let width = gpu.extent_w;
        let height = gpu.extent_h;
        let unfilled = gpu.unfilled;
        let permit = genos_window::ShutdownPermit::issue();
        Ok((Self {
            gpu,
            width,
            height,
            wire_on: false,
            memory: FrameMemory::default(),
            shapes: crate::pool::MeshPool::default(),
            overlay_rects: Vec::new(),
            overlay_lines: Vec::new(),
            overlay_verts: Vec::new(),
            keep_pictures: false,
            debug: debug_view::DebugState {
                unfilled,
                ..debug_view::DebugState::default()
            },
            slow_fence: None,
        }, permit))
    }

    /// Answer compositor pings on a side thread during device waits and pipeline compiles.
    ///
    /// `enter` and `leave` are `genos_window::seat_enter` and `genos_window::seat_leave`.
    /// The window must already have installed its seat. Clear them before the window is dropped.
    pub fn set_blocker(enter: fn(), leave: fn()) {
        ENTER.with(|cell| cell.set(Some(enter)));
        LEAVE.with(|cell| cell.set(Some(leave)));
    }

    /// The next device wait runs on this thread only.
    pub fn clear_blocker() {
        ENTER.with(|cell| cell.set(None));
        LEAVE.with(|cell| cell.set(None));
    }

    /// Lines drawn over the picture with the next overlay rectangles, in the same
    /// draw. They stay until replaced.
    pub fn set_overlay_lines(&mut self, lines: &[ScreenLine]) {
        if self.overlay_lines.as_slice() != lines {
            self.overlay_lines.clear();
            self.overlay_lines.extend_from_slice(lines);
            // Rebuild the vertices on the next draw.
            self.overlay_rects.clear();
            self.overlay_rects.push(ScreenRect {
                x: f32::NAN,
                y: 0.0,
                w: 0.0,
                h: 0.0,
                color: [0.0; 3],
            });
        }
    }

    /// True when the last submit left its fence unsignaled. The caller can continue.
    pub fn submit_was_pending(&self) -> bool {
        self.gpu.submit_pending
    }

    /// Pixels from the frame before the latest submit. That frame keeps its own lamp.
    ///
    /// Needs `set_keep_pictures(true)` before both draws.
    pub fn read_earlier_frame(&mut self) -> Result<Vec<u8>, String> {
        let slot = self.gpu.flight;
        self.gpu.read_host(slot)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if !extent_needed(self.width, self.height, width, height) {
            return Ok(());
        }
        self.gpu.recreate(width, height)?;
        self.width = self.gpu.extent_w;
        self.height = self.gpu.extent_h;
        Ok(())
    }

    /// Replace the draw's lighting settings until [`Renderer::clear_lighting_override`].
    ///
    /// A game that already owns the [`genos_scene::Scene`] writes `scene.lighting`
    /// instead. The next draw applies either value, then the launch locks:
    /// `--preset`, `--bounces`, and `--ambient` win, and a `GENOS_LIGHTING_*`
    /// mirror applies only when its argument was omitted.
    pub fn set_lighting_settings(&mut self, settings: genos_scene::LightingSettings) {
        self.gpu.lighting.override_settings = Some(settings);
    }

    /// The scene's lighting settings win again.
    pub fn clear_lighting_override(&mut self) {
        self.gpu.lighting.override_settings = None;
    }

    /// Settings the last draw used, after the scene, the override, and the environment.
    pub fn lighting_settings(&self) -> genos_scene::LightingSettings {
        self.gpu.lighting.settings.clone()
    }

    /// EV100 the last exposure scale was built from. Zero while legacy bytes hold the scale at 1.
    pub fn hdr_ev100(&self) -> f64 {
        self.gpu.lighting.last_ev100
    }

    /// Exposure scale written into the compose params. Legacy bytes stay at 1.
    pub fn hdr_exposure(&self) -> f64 {
        f64::from(self.gpu.lighting.exposure_scale)
    }

    /// Last eight exposure scales, oldest first.
    pub fn hdr_scales(&self) -> Vec<f64> {
        self.gpu.lighting.exposure_log.clone()
    }

    /// Last histogram bin counts. Empty until an auto-exposure frame meters.
    pub fn hdr_histogram(&self) -> Vec<u32> {
        self.gpu.lighting.histogram_counts.clone()
    }

    /// Share of subsampled scene luma that moved by more than 0.01 since the previous copy.
    pub fn hdr_still_pop(&self) -> f64 {
        self.gpu.lighting.still_pop
    }

    /// Swapchain format and color space in use.
    pub fn hdr_surface(&self) -> (i32, i32) {
        (self.gpu.format, self.gpu.color_space)
    }

    /// HDR10 PQ code of graphics white, from `swapchain_nits`.
    pub fn hdr_pq_white(&self) -> f64 {
        self.gpu.pq_white
    }

    /// scRGB code of graphics white, from `swapchain_nits`.
    pub fn hdr_scrgb_white(&self) -> f64 {
        self.gpu.scrgb_white
    }

    /// Mode the swapchain was created for. SDR when the asked pair is not listed.
    pub fn hdr_resolved(&self) -> &'static str {
        match self.gpu.hdr_output {
            crate::hdr_output::HdrSwapchain::Sdr => "sdr",
            crate::hdr_output::HdrSwapchain::Hdr10 => "hdr10",
            crate::hdr_output::HdrSwapchain::Scrgb => "scrgb",
        }
    }

    /// GPU milliseconds of bloom plus the display pass on the last collected frame.
    ///
    /// One frame behind the draw that just returned. Zero when the device has no
    /// timestamp queries. Lavapipe time is not a 4070 score.
    pub fn hdr_ms(&self) -> f64 {
        self.gpu.lighting.hdr_ms
    }

    /// `hdr_ms`. The 1.50 ms ceiling is the 4070 at 1440p, not a lavapipe score.
    pub fn post_ms_device(&self) -> f64 {
        self.hdr_ms()
    }

    /// Bytes of the post buffers: scene image, host copy, bloom, encode, lattice, params, meter.
    pub fn hdr_vram_bytes(&self) -> f64 {
        let lighting = &self.gpu.lighting;
        let bytes = lighting
            .scene_lin
            .size
            .saturating_add(lighting.scene_host.size)
            .saturating_add(lighting.bloom.size)
            .saturating_add(lighting.hdr_encode.size)
            .saturating_add(lighting.agx_lut.size)
            .saturating_add(lighting.hdr_params.size)
            .saturating_add(lighting.meter_dev.size)
            .saturating_add(lighting.meter_host.size);
        bytes as f64
    }

    /// Post-chain bytes as MiB. The 96 MiB ceiling is the allocation budget.
    pub fn post_vram_device(&self) -> f64 {
        self.hdr_vram_bytes() / (1024.0 * 1024.0)
    }

    /// The legacy shoulder is the display curve, so a missing scene image may undo it.
    /// `AgX` and ACES are not inverted.
    pub fn hdr_legacy_display(&self) -> bool {
        self.gpu.lighting.hdr.legacy_bytes
            || matches!(
                self.gpu.lighting.hdr.display,
                genos_scene::DisplayTransform::Legacy
            )
    }

    /// Drop the surface cache and its work list on the next draw.
    ///
    /// A scene switch calls this. Recreating the renderer would compile the
    /// pipelines again; filling the cache buffers is enough.
    pub fn reset_lighting_cache(&mut self) {
        self.gpu.lighting.cache_clear = true;
        self.gpu.lighting.work_clear = true;
    }

    /// Replace the override with a named preset (`potato`, `low`, `medium`, `high`).
    pub fn apply_lighting_preset(&mut self, name: &str) -> Result<(), String> {
        let preset = genos_scene::LightingPreset::parse(name)
            .ok_or_else(|| format!("unknown lighting preset {name}"))?;
        self.gpu.lighting.override_settings = Some(genos_scene::LightingSettings::preset(preset));
        Ok(())
    }

    /// Dispatches recorded for each lighting bucket on the last collected frame.
    pub fn lighting_dispatches(&self) -> [u32; 12] {
        self.gpu.lighting.last_dispatches
    }

    /// GPU milliseconds of each lighting bucket on the last collected frame.
    ///
    /// One frame behind the draw that just returned. Zero when timestamps are off.
    pub fn lighting_pass_ms(&self) -> [f64; 12] {
        self.gpu.lighting.last_ms
    }

    /// `--gi-metrics` is on, so pass times and work counters are recorded.
    pub fn gi_metrics_enabled(&self) -> bool {
        self.gpu
            .gpu_times
            .as_ref()
            .is_some_and(|times| times.record)
    }

    /// Samples since the last clear, oldest first. One frame behind the draw.
    pub fn gi_metrics_samples(&mut self) -> &[GiSample] {
        let Some(times) = self.gpu.gpu_times.as_mut() else {
            return &[];
        };
        if !times.record {
            return &[];
        }
        times.samples.make_contiguous()
    }

    /// Drop the recorded window. The next sample starts at sequence 1.
    pub fn gi_metrics_clear(&mut self) {
        if let Some(times) = self.gpu.gpu_times.as_mut() {
            times.samples.clear();
            times.recorded = 0;
        }
    }

    /// Material knobs in force. `texture_quality` does not follow the VRAM size.
    pub fn material_settings(&self) -> genos_load::MaterialSettings {
        self.gpu.materials
    }

    /// Atlases the frame registered, and the finest resident mip of the first.
    ///
    /// The colour atlas is index 0 when it is loaded. `None` is the coarsest
    /// page only, or no texture at all when the count is 0.
    #[must_use]
    pub fn material_residency(&self) -> (usize, Option<u32>) {
        let count = self.gpu.streamer.texture_count();
        let finest = if count == 0 {
            None
        } else {
            self.gpu.streamer.finest_resident(0)
        };
        (count, finest)
    }

    /// Colour texel 0 of the device atlas, then the same texel of the page image.
    ///
    /// The atlas value is the blit of that page. The CPU chain and the staging
    /// buffer are not either byte. A staging copy into the atlas leaves the
    /// page at 0, or the atlas at 0 once the staging buffer has been cleared.
    #[must_use]
    pub fn material_texel(&self) -> [u8; 2] {
        [
            self.gpu.atlas_readback_texel(),
            self.gpu.page_readback_texel(),
        ]
    }

    /// Replace the material knobs. The next frame streams inside the new cap.
    pub fn set_material_settings(&mut self, settings: genos_load::MaterialSettings) {
        self.gpu.apply_material_settings(settings);
    }

    /// Device-local budget after headroom. Lighting can read this later.
    pub fn vram_budget(&self) -> genos_load::VramBudget {
        self.gpu.vram
    }

    /// Bytes other systems have allocated, on top of the render targets and the frame budget.
    pub fn set_non_texture_bytes(&mut self, bytes: u64) {
        self.gpu.non_texture_extra = bytes;
    }

    /// A finished disk read. The admit follows the slowest measured path.
    pub fn observe_material_disk(&mut self, bytes: u64, ms: u32) {
        self.gpu.streamer.observe_disk(bytes, ms);
    }

    /// A finished decode or transcode.
    pub fn observe_material_decode(&mut self, bytes: u64, ms: u32) {
        self.gpu.streamer.observe_decode(bytes, ms);
    }

    /// Texture bytes the streamer is holding, including a mip still in flight.
    #[must_use]
    pub fn material_committed(&self) -> u64 {
        self.gpu.streamer.committed()
    }

    /// Textures the frame has registered.
    #[must_use]
    pub fn material_texture_count(&self) -> usize {
        self.gpu.streamer.texture_count()
    }

    /// Finest resident mip of one registered texture.
    #[must_use]
    pub fn material_finest(&self, index: usize) -> Option<u32> {
        self.gpu.streamer.finest_resident(index)
    }

    /// Bytes per millisecond of the slowest measured path.
    #[must_use]
    pub fn material_bytes_per_ms(&self) -> u64 {
        self.gpu.streamer.bytes_per_ms()
    }

    /// Bytes the last step was willing to admit, after the sustainable cap.
    #[must_use]
    pub fn material_last_admit(&self) -> u64 {
        self.gpu.streamer.last_admit()
    }

    /// Base frame time the last step measured, in milliseconds.
    #[must_use]
    pub fn material_frame_ms(&self) -> f32 {
        self.gpu.streamer.frame_ms()
    }

    /// Eviction victims in the order they were dropped.
    #[must_use]
    pub fn material_evicted(&self) -> Vec<(usize, u32)> {
        self.gpu.streamer.evicted().to_vec()
    }

    /// Counters the next auto-tune reads: committed bytes, the texture target, and the streamer meters.
    #[must_use]
    pub fn material_stats(&self) -> MaterialStats {
        let stream = &self.gpu.streamer;
        MaterialStats {
            committed: stream.committed(),
            budget: self.gpu.vram.texture_target(),
            refused: stream.refused(),
            evictions: stream.evictions(),
            thrash: stream.thrash(),
            holes: stream.holes(),
            rate: stream.rate(),
            stream_ms: stream.frame_ms(),
        }
    }

    /// Replace the queried `heapBudget` and `heapUsage`. `None` uses the device again.
    ///
    /// The pair is `(heap_budget, heap_usage)`. The next frame's texture target
    /// is that budget minus non-texture bytes minus headroom.
    pub fn set_heap_budget_override(&mut self, budget_and_usage: Option<(u64, u64)>) {
        self.gpu.heap_override = budget_and_usage;
    }

    /// Device memory for a texture page. An out-of-device-memory error evicts,
    /// frees that mip's device memory, and retries.
    pub fn allocate_texture_pages(&mut self, size: u64, type_bits: u32) -> Result<(), String> {
        let memory = self.gpu.allocate_texture(size, type_bits)?;
        self.gpu.bind_next_unbacked(memory)
    }

    /// The next `count` device allocations return out of device memory and do
    /// not call the driver.
    pub fn fail_next_device_allocs(&self, count: u32) {
        self.gpu.fail_device_allocs.set(count);
    }

    /// Device bytes still held. This drops only when a free reaches the driver.
    #[must_use]
    pub fn device_bytes(&self) -> u64 {
        self.gpu.device_bytes.get()
    }

    /// Bytes of device memory still bound to streamed mip pages.
    #[must_use]
    pub fn material_page_bytes(&self) -> u64 {
        self.gpu
            .stream_pages
            .iter()
            .filter(|page| !page.memory.is_null())
            .map(|page| page.bytes)
            .sum()
    }

    /// Allocate one device page through the out-of-memory retry, then release
    /// that probe. A drop in `device_bytes` is the mip the retry freed.
    pub fn retry_texture_alloc(&mut self) -> Result<(), String> {
        let memory = self.gpu.allocate_texture(4096, u32::MAX)?;
        self.gpu.release_device(memory, 4096);
        Ok(())
    }

    fn fund_stream_pages(&mut self) -> Result<(), String> {
        if self.gpu.transfer_busy {
            return Ok(());
        }
        self.gpu.prepare_unbacked()?;
        while self.gpu.has_unbacked() {
            let (size, bits) = self.gpu.next_unbacked()?;
            self.allocate_texture_pages(size, bits)?;
        }
        Ok(())
    }

    /// Bucket names, in the order of [`Renderer::lighting_dispatches`].
    pub fn lighting_bucket_names() -> &'static [&'static str] {
        &LIGHTING_BUCKET_NAMES
    }

    /// Draw one frame: the visible objects, lit by the lighting passes.
    ///
    /// When `readback` is set, return tightly packed BGRA8 pixels.
    pub fn draw(
        &mut self,
        world: &World,
        camera: &Camera,
        readback: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        self.draw_with_overlay(world, camera, &[], readback, false)
    }

    /// Pixels of the last presented picture.
    ///
    /// Waits for that draw's graphics fence. Does not wait for a light gather.
    /// A `readback` draw already waited, and its pixels are the finished field.
    pub fn read_picture(&mut self) -> Result<Vec<u8>, String> {
        self.gpu.copy_last_picture()?;
        self.gpu.read_color()
    }

    /// Scene-referred radiance of the last compose, before the display transform.
    ///
    /// Empty when lighting has not allocated the image. Each texel goes through
    /// [`scene_read_from_rgba`], which calls [`scene_referred`].
    pub fn read_scene(&mut self) -> Result<crate::hdr_output::SceneRead, String> {
        self.gpu.copy_last_picture()?;
        self.gpu.read_scene_host()
    }

    /// Bloom the frame's pyramid wrote. RGBA32F, full resolution, before the
    /// intensity scale. The mip chain sits past this image and is not returned.
    pub fn read_bloom(&self) -> Result<Vec<f32>, String> {
        self.gpu.read_bloom_host()
    }

    /// HDR10 PQ or scRGB codes from the display pass. RGBA32F, one texel per pixel.
    /// Empty when that pass did not encode.
    pub fn read_hdr_encode(&self) -> Result<Vec<f32>, String> {
        self.gpu.read_hdr_encode_host()
    }

    /// Draw the world, then screen rectangles. The rectangles ignore the depth test.
    ///
    /// When `wireframe` is true, each floor, wall, and solid draws its vertices and its edges.
    /// The filled shaded triangles stay in use when `wireframe` is false.
    pub fn draw_with_overlay(
        &mut self,
        world: &World,
        camera: &Camera,
        overlay: &[ScreenRect],
        readback: bool,
        wireframe: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        self.gpu.sync_materials(world)?;
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let matrix = view_proj(camera, aspect);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        self.memory.begin_frame();
        self.shapes.begin_frame();
        let pack = pack::pack_frame(world, &matrix, eye, &mut self.memory);
        for key in self.memory.take_dropped() {
            self.shapes.release(key);
        }
        if self.overlay_rects.as_slice() != overlay {
            self.overlay_verts = screen_quads(overlay);
            self.overlay_verts.extend(screen_lines(&self.overlay_lines));
            self.overlay_rects.clear();
            self.overlay_rects.extend_from_slice(overlay);
        }
        let overlay_verts = self.overlay_verts.clone();
        self.wire_on = wireframe;
        let views = crate::material_stream::frame_views(world, camera, self.width, self.height, &matrix);
        self.gpu.tick_materials(self.memory.accounted(), &views)?;
        self.fund_stream_pages()?;
        self.gpu.submit_transfer()?;
        let overlay_count = overlay_verts.len() as u32;
        // The dynamic patch belongs to the frame that is about to record.
        let slot = self.gpu.flight;
        self.gpu.wait_flight(slot)?;
        self.gpu.bind_flight();
        let mut dynamic = Vec::new();
        let mut draws = Vec::new();
        let mut instances = vec![identity_instance()];
        for item in &pack.draws {
            match item {
                pack::PackedDraw::Shape {
                    key,
                    model,
                    color,
                    emission,
                    color_rect,
                    normal_rect,
                    cutoff,
                    source,
                } => {
                    if !self.memory.contains_shape(*key) {
                        continue;
                    }
                    let block = if self.shapes.contains(*key) {
                        self.shapes.use_mesh(*key, &[])
                    } else {
                        let local = pack::build_shape(source);
                        self.shapes.use_mesh(*key, &local)
                    };
                    let Some(block) = block else {
                        continue;
                    };
                    let instance = instances.len() as u32;
                    instances.push(InstanceRec {
                        model: *model,
                        // w: emitted light as shared-exponent bits (0: none) for the
                        // GI G-buffer (gbuffer.vert); the picture ignores it.
                        color: [
                            color[0],
                            color[1],
                            color[2],
                            f32::from_bits(crate::mesh_field::pack_rgb9e5(*emission)),
                        ],
                        atlas: *color_rect,
                        normal_atlas: *normal_rect,
                        extra: [*cutoff, 0.0, 0.0, 0.0],
                    });
                    draws.push(DrawSpan {
                        shapes: true,
                        first: block.offset,
                        count: block.count,
                        instance,
                    });
                }
                pack::PackedDraw::Dynamic(verts) => {
                    if verts.is_empty() {
                        continue;
                    }
                    let first = dynamic.len() as u32;
                    dynamic.extend_from_slice(verts);
                    draws.push(DrawSpan {
                        shapes: false,
                        first,
                        count: verts.len() as u32,
                        instance: 0,
                    });
                }
            }
        }
        self.shapes.finish_frame();
        for key in self.memory.finish_frame() {
            self.shapes.release(key);
        }
        self.gpu.upload_shapes(&self.shapes)?;
        if wireframe {
            let corners = pack::shape_corners(world, &matrix);
            let wire = crate::wire::mark_vertices(&crate::wire::marks_from_triangles(&corners));
            let count = wire.len() as u32;
            dynamic = wire;
            draws = if count == 0 {
                Vec::new()
            } else {
                vec![DrawSpan {
                    shapes: false,
                    first: 0,
                    count,
                    instance: 0,
                }]
            };
            instances = vec![identity_instance()];
        }
        let world_dynamic = dynamic.len() as u32;
        let mut dynamic = dynamic;
        dynamic.extend(overlay_verts);
        if !dynamic.is_empty() {
            self.gpu.upload(&dynamic)?;
        }
        self.gpu.write_instances(&instances)?;
        if self.memory.image_uploaded() {
            // The particle image is one shared buffer. Both frames can sample it.
            let (width, height) = self.memory.image_size();
            let bytes = pack::image_bytes(width, height, self.memory.image_pixels());
            self.gpu.wait_flight(1 - slot)?;
            self.gpu.upload_image(&bytes)?;
            self.memory.clear_image_upload();
        }
        let mut pack = pack;
        pack::apply_view(&mut pack, camera, aspect);
        self.gpu.unfilled_eye = [camera.position.x, camera.position.y, camera.position.z];
        self.gpu.unfilled_yaw = camera.yaw;
        self.gpu.unfilled_pitch = camera.pitch;
        self.debug.apply(&mut pack);
        self.sync_fields(world)?;
        self.gpu.upload_scene(&pack)?;
        self.gpu.note_vertex_count(world_dynamic);
        self.gpu.set_draws(draws);
        self.gpu.note_overlay_count(overlay_count);
        self.gpu.wire_on = wireframe;
        self.gpu.copy_picture = readback || self.keep_pictures;
        self.gpu.sync_lighting_settings(&world.scene)?;
        self.gpu.set_froxel_frame(&world.scene, camera, aspect);
        self.gpu.record_and_submit(&matrix)?;
        let pixels = if readback {
            self.gpu.wait_gpu()?;
            Some(self.gpu.read_color()?)
        } else {
            None
        };
        let result = self.gpu.present()?;
        if result == OUT_OF_DATE || result == SUBOPTIMAL {
            self.gpu.recreate(self.width, self.height)?;
            self.width = self.gpu.extent_w;
            self.height = self.gpu.extent_h;
        }
        Ok(pixels)
    }

    /// Draw one frame and record a Vulkan timestamp span around that GPU work.
    ///
    /// `cpu` is the host time of this call. `gpu` is the timestamp span when the
    /// fence has already signaled, for example after a readback wait. The fence
    /// wait is not the GPU time. [`Renderer::draw`] does not record timestamps.
    pub fn draw_profiled(
        &mut self,
        world: &World,
        camera: &Camera,
        overlay: &[ScreenRect],
        readback: bool,
        wireframe: bool,
    ) -> Result<(Option<Vec<u8>>, DrawProfile), String> {
        if self.gpu.timestamp_period <= 0.0 || self.gpu.timestamp_bits == 0 {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        let start = Instant::now();
        self.gpu.profile_submit = true;
        let drawn = self.draw_with_overlay(world, camera, overlay, readback, wireframe);
        let cpu = start.elapsed();
        self.gpu.profile_submit = false;
        let pixels = drawn?;
        let gpu = self.gpu.collect_submitted_if_ready()?;
        Ok((pixels, DrawProfile { cpu, gpu }))
    }

    /// GPU times whose fences have signaled, in submit order. This does not wait.
    pub fn poll_gpu_times(&mut self) -> Result<Vec<Duration>, String> {
        self.gpu.collect_ready(false)?;
        Ok(self.gpu.ready.drain(..).collect())
    }

    /// Wait for outstanding timestamp queries and return those GPU times.
    ///
    /// The wait is not added to the returned durations. Call this before the
    /// renderer is dropped so a finished run can store the device times.
    pub fn finish_gpu_times(&mut self) -> Result<Vec<Duration>, String> {
        self.gpu.collect_ready(true)?;
        Ok(self.gpu.ready.drain(..).collect())
    }

    /// Hand the GPU tracer the world's mesh SDF instances when they changed.
    fn sync_fields(&mut self, world: &World) -> Result<(), String> {
        let live = crate::mesh_field::live_instances(world);
        self.gpu.note_field_movers(&live);
        let key = crate::mesh_field::field_key(&live);
        self.gpu.sync_fields(key, &live)
    }

    /// Write the world into the resident scene buffer. This does not present.
    pub fn retain_scene(&mut self, world: &World) -> Result<(), String> {
        let camera = Camera::opening();
        let matrix = view_proj(&camera, 1.0);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        self.memory.begin_frame();
        let mut pack = pack::pack_frame(world, &matrix, eye, &mut self.memory);
        let aspect = self.width as f32 / self.height.max(1) as f32;
        pack::apply_view(&mut pack, &camera, aspect);
        self.gpu.wait_all_inflight()?;
        self.sync_fields(world)?;
        self.gpu.upload_scene(&pack)?;
        Ok(())
    }

    /// The picture has stopped changing: the lighting holds its light and a frame
    /// drawn from it is on screen.
    pub fn picture_settled(&self) -> bool {
        self.gpu.lighting_picture_held()
    }

    /// Whether the lighting has settled, for reports and the debug tools.
    pub fn settle_stats(&self) -> SettleStats {
        self.gpu.lighting_stats()
    }

    /// Five closed forms from `media_check.comp`. The parity test compares them
    /// with `genos_math::media`. This does not present.
    pub fn media_closed_forms(&mut self) -> Result<[f32; 5], String> {
        self.gpu.media_closed_forms()
    }

    /// Milliseconds of one `media_check.comp` dispatch, from device timestamps.
    /// The fog budget scales this span. Lavapipe is the cloud device.
    pub fn media_dispatch_ms(&mut self, groups: u32) -> Result<f64, String> {
        self.gpu.media_dispatch_ms(groups)
    }

    /// Run the froxel inject and integrate passes and read the columns back.
    ///
    /// # Errors
    ///
    /// Returns an error when the device rejects the dispatch.
    pub fn run_fog(&mut self, job: &crate::fog_pass::FogDispatch) -> Result<crate::fog_pass::FogGpu, String> {
        self.gpu.run_fog(job)
    }

    /// Lavapipe timestamp of one fitted froxel sample count. Not an RTX 4070 time.
    ///
    /// # Errors
    ///
    /// Returns an error when the device has no timestamp queries or the dispatch fails.
    pub fn froxel_sample_ms(&mut self, preset: LightingPreset, samples: u32) -> Result<f64, String> {
        self.gpu.froxel_sample_ms(preset, samples)
    }

    /// Lavapipe timestamp of one full froxel grid. Not an RTX 4070 time.
    ///
    /// # Errors
    ///
    /// Returns an error when the device has no timestamp queries or the dispatch fails.
    pub fn froxel_grid_ms(&mut self, size: crate::fog::GridSize) -> Result<f64, String> {
        self.gpu.froxel_grid_ms(size)
    }

    /// Bytes of the preset froxel allocation. `doubled` allocates a second copy
    /// and adds it, which is the planted VRAM miss.
    ///
    /// # Errors
    ///
    /// Returns an error when the image or buffer allocation fails.
    pub fn froxel_allocation(&mut self, preset: LightingPreset, doubled: bool) -> Result<u64, String> {
        self.gpu.froxel_allocation(preset, doubled)
    }

    /// Probe for the debug page. `pass_ms` is the last frame's froxel timestamp.
    /// History stays off, so accepts and rejects stay 0.
    #[must_use]
    pub fn fog_frame_probe(&self, fog: &genos_scene::FogSettings, preset: LightingPreset) -> crate::fog_budget::FogProbe {
        let mut probe = crate::fog_budget::fog_probe(fog, preset);
        if probe.dispatches == 0 || !self.gpu.froxel_wrote {
            probe.dispatches = 0;
            probe.pass_ms = 0.0;
            probe.vram_bytes = 0;
            return probe;
        }
        probe.pass_ms = self.gpu.froxel_pass_ms();
        probe.vram_bytes = self.gpu.froxel_vram();
        probe
    }

    /// Columns written by the last frame dispatch. Empty when that frame skipped fog.
    ///
    /// # Errors
    ///
    /// Returns an error when the column buffer cannot be mapped.
    pub fn frame_fog_columns(&mut self) -> Result<Vec<crate::fog::FogColumn>, String> {
        self.gpu.read_frame_columns()
    }

    /// CPU shadowed integral of the view the last frame dispatch built.
    ///
    /// `scene` is the scene that frame drew. Empty when that frame did not dispatch.
    #[must_use]
    pub fn frame_fog_reference(&self, scene: &genos_scene::Scene) -> Vec<crate::fog::FogColumn> {
        self.gpu.frame_fog_reference(scene)
    }

    /// One compute pass over the resident occluders. This does not present and does not upload them.
    pub fn transmission_gains(
        &mut self,
        listener: [f32; 3],
        sources: &[[f32; 3]],
    ) -> Result<Vec<f32>, String> {
        self.gpu.transmission_gains(listener, sources)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Picture filter for the next draw. The renderer starts at `Off`.
    pub fn set_antialias(&mut self, mode: Antialias) {
        self.gpu.antialias = mode;
    }

    pub fn antialias(&self) -> Antialias {
        self.gpu.antialias
    }

    /// Frames in a row GI has seen the same scene (camera, lights, objects): its
    /// screen probes average that many frames' rays (up to 32), and any change
    /// starts the count, and so the average, over at 0.
    pub fn lighting_still_frames(&self) -> u32 {
        self.gpu.lighting_still_frames()
    }

    /// GI's screen probes in the last frame drawn: the rays they held, kept
    /// strata included, and the probes placed. Rays over probes is each probe's
    /// effective sample count (up to 8 strata of 64). Waits for that frame.
    pub fn lighting_probe_samples(&mut self) -> [u32; 2] {
        self.gpu.lighting_probe_samples()
    }

    /// Dirty-fraction threshold for the a-trous work lists. Overrides
    /// `GENOS_LIGHTING_ATROUS_DIRTY` for later frames. `0` always runs the dense
    /// sweep, `1` keeps the lists up to the storage cap.
    pub fn set_lighting_atrous_dirty(&mut self, fraction: f32) {
        self.gpu.lighting.atrous_dirty = Some(fraction.clamp(0.0, 1.0));
    }

    /// A-trous dirty fraction, chosen mode and threshold for the last frame
    /// drawn. Waits for that frame.
    pub fn lighting_atrous_stats(&mut self) -> LightingAtrousStats {
        self.gpu.lighting_atrous_stats()
    }

    /// Shadow rays and spatial merges from the last frame. Waits for that frame.
    /// The counters are zero unless `GENOS_LIGHTING_LAMP_PICK_DIAG` was set.
    pub fn lighting_lamp_pick_counts(&mut self) -> LampPickCounts {
        self.gpu.lighting_lamp_pick_counts()
    }

    /// Sun-step mark from the last submitted frame. Waits for that frame.
    /// `gpu_sun` is the shader bit. `host_frames` counts frames the CPU took
    /// the small-sun path.
    pub fn lighting_sun_stats(&mut self) -> LightingSunStats {
        self.gpu.lighting_sun_stats()
    }

    /// How long each pipeline took to build, in build order.
    pub fn pipeline_times(&self) -> Vec<(String, Duration)> {
        self.gpu.pipeline_times.clone()
    }

    /// When on, every frame copies its picture into host memory, so
    /// `read_earlier_frame` can read a frame after the next one is submitted.
    /// Off by default: the copy is a whole picture over the bus each frame.
    pub fn set_keep_pictures(&mut self, on: bool) {
        self.keep_pictures = on;
    }
}

impl genos_window::ShutdownDevice for Renderer {
    fn poll_idle(&mut self) -> bool {
        if let Some(stall) = self.slow_fence.take() {
            // No seat thread. The drain phase's clock includes this wait, and
            // the next seat dispatch records it as the quit gap.
            wait_unpumped(stall);
        }
        self.gpu.poll_idle()
    }

    fn destroy_step(&mut self) -> bool {
        self.gpu.destroy_step()
    }

    fn next_phase(&self) -> &'static str {
        self.gpu.next_shutdown_phase()
    }

    fn plant_slow_fence(&mut self, stall: Duration) {
        self.slow_fence = Some(stall);
    }
}

/// Wait out `duration` without dispatching the seat.
///
/// A condvar timeout is the wait. `std::thread::sleep` is denied, and this
/// must not run inside `block()`, or the seat thread would hide the stall.
fn wait_unpumped(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    let deadline = Instant::now() + duration;
    let pair = (Mutex::new(()), Condvar::new());
    while Instant::now() < deadline {
        let guard = match pair.0.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = pair.1.wait_timeout(guard, remaining);
    }
}

/// Host time and device time for one profiled draw.
///
/// `gpu` is `None` until that draw's fence has signaled. It is never a copy of `cpu`.
#[derive(Clone, Copy, Debug)]
pub struct DrawProfile {
    pub cpu: Duration,
    pub gpu: Option<Duration>,
}

/// One axis-aligned rectangle in window pixels. The origin is the top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 3],
}

/// A line in window pixels, drawn as a thin quad with the overlay rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLine {
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Pixels across.
    pub width: f32,
    pub color: [f32; 3],
}

struct DrawSpan {
    shapes: bool,
    first: u32,
    count: u32,
    instance: u32,
}

struct InstanceRec {
    model: [f32; 16],
    color: [f32; 4],
    /// Colour atlas rect (x, y, w, h). `w == 0` means no texture.
    atlas: [f32; 4],
    normal_atlas: [f32; 4],
    /// x is the alpha cutoff. 0 does not discard.
    extra: [f32; 4],
}

fn identity_instance() -> InstanceRec {
    InstanceRec {
        model: crate::world::identity_pose(),
        // w 0: no emitted light (gbuffer.vert).
        color: [1.0, 1.0, 1.0, 0.0],
        atlas: [0.0; 4],
        normal_atlas: [0.0; 4],
        extra: [0.0; 4],
    }
}

fn screen_quads(rects: &[ScreenRect]) -> Vec<GpuVertex> {
    let mut verts = Vec::with_capacity(rects.len() * 6);
    for rect in rects {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;
        let color = rect.color;
        let corner = |x: f32, y: f32| pack::overlay_vertex([x, y, 0.0], color);
        verts.push(corner(x0, y0));
        verts.push(corner(x1, y0));
        verts.push(corner(x1, y1));
        verts.push(corner(x0, y0));
        verts.push(corner(x1, y1));
        verts.push(corner(x0, y1));
    }
    verts
}

/// Each line as two triangles around its segment, `width` pixels across.
fn screen_lines(lines: &[ScreenLine]) -> Vec<GpuVertex> {
    let mut verts = Vec::with_capacity(lines.len() * 6);
    for line in lines {
        let d = [line.b[0] - line.a[0], line.b[1] - line.a[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len <= 1.0e-6 || line.width <= 0.0 || !len.is_finite() {
            continue;
        }
        let h = 0.5 * line.width / len;
        let n = [-d[1] * h, d[0] * h];
        let corner = |p: [f32; 2], s: f32| {
            pack::overlay_vertex([p[0] + n[0] * s, p[1] + n[1] * s, 0.0], line.color)
        };
        let (a0, a1, b0, b1) = (
            corner(line.a, 1.0),
            corner(line.a, -1.0),
            corner(line.b, 1.0),
            corner(line.b, -1.0),
        );
        verts.extend([a0, b0, b1, a0, b1, a1]);
    }
    verts
}

/// Column-major map from pixel space to Vulkan NDC. (0, 0) is the top-left.
fn pixel_matrix(width: f32, height: f32) -> [f32; 16] {
    let mut matrix = [0.0; 16];
    matrix[0] = 2.0 / width.max(1.0);
    matrix[5] = 2.0 / height.max(1.0);
    matrix[10] = 1.0;
    matrix[12] = -1.0;
    matrix[13] = -1.0;
    matrix[15] = 1.0;
    matrix
}

pub(crate) fn extent_needed(old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> bool {
    new_w > 0 && new_h > 0 && (new_w != old_w || new_h != old_h)
}

#[allow(dead_code)]
struct Gpu {
    get_instance: GetProc,
    get_device: GetProc,
    instance: Handle,
    physical: Handle,
    device: Handle,
    queue: Handle,
    queue_family: u32,
    /// Gather queue. A graphics submit does not wait for this queue.
    compute_queue: Handle,
    compute_family: u32,
    surface: Handle,
    swapchain: Handle,
    format: i32,
    /// Swapchain color space. sRGB nonlinear until an HDR pair is selected.
    color_space: i32,
    /// Resolved swapchain. SDR until `create_targets` runs.
    hdr_output: crate::hdr_output::HdrSwapchain,
    /// `swapchain_nits` of graphics white for HDR10 and for scRGB.
    pq_white: f64,
    scrgb_white: f64,
    /// `VK_EXT_hdr_metadata` was enabled on the device.
    hdr_metadata: bool,
    extent_w: u32,
    extent_h: u32,
    /// Swapchain present mode: 0 immediate, 1 mailbox, 2 FIFO.
    present_mode: i32,
    swap_images: Vec<Handle>,
    color: Image,
    depth: Image,
    framebuffer: Handle,
    host: Buffer,
    /// Alias of `vertices[flight]`. Drop destroys `vertices`, not this copy.
    vertex: Buffer,
    vertices: [Buffer; 2],
    /// Shared shape pool. Offsets stay put for the life of a block.
    shapes: Buffer,
    /// How many shape vertices have been read by a submitted frame.
    shapes_submitted: u32,
    /// One instance buffer per frame in flight. The vertex shader reads the
    /// buffer for its flight. A moving box can update the idle flight without
    /// waiting for the frame still on the GPU.
    instance_bufs: [Buffer; 2],
    draws: Vec<DrawSpan>,
    vertex_count: u32,
    overlay_count: u32,
    render_pass: Handle,
    pipeline: Handle,
    /// Depth only, over the opaque shapes, before `pipeline` (aa_gpu.rs raster_scene).
    depth_pipeline: Handle,
    overlay_pipeline: Handle,
    wire_pipeline: Handle,
    wire_on: bool,
    /// This frame also copies its picture into host memory, for a readback.
    copy_picture: bool,
    /// Unfilled-on-screen detector. Off does no atomics and no mask copy.
    unfilled: UnfilledControl,
    unfilled_saved: u32,
    last_unfilled: Option<UnfilledStats>,
    unfilled_eye: [f32; 3],
    unfilled_yaw: f32,
    unfilled_pitch: f32,
    /// `--out` from the process arguments. It wins over the environment mirror.
    out_cli: Option<String>,
    /// Directory the next unfilled event is written into.
    debug_out: String,
    layout: Handle,
    pool: Handle,
    cmd: Handle,
    image_ready: Handle,
    render_done: Handle,
    fence: Handle,
    present_index: u32,
    cmds: [Handle; 2],
    fences: [Handle; 2],
    image_readies: [Handle; 2],
    render_dones: [Handle; 2],
    colors: [Image; 2],
    depths: [Image; 2],
    framebuffers: [Handle; 2],
    hosts: [Buffer; 2],
    flight: usize,
    /// Occluders for audio: the latest scene, written only when audio asks.
    scene_buf: Buffer,
    /// The scene as this frame draws it: lamps, occluders and the view.
    frame_scene: [Buffer; 2],
    /// This frame's scene bytes, written into `frame_scene` once its slot is free.
    frame_bytes: Vec<u8>,
    /// S70C: what the resident scene buffer holds (`frame_scene[0]`, device-local;
    /// `[1]` names the same buffer). A frame records only the 256-byte blocks that
    /// differ (vkCmdUpdateBuffer, in the frame's command stream).
    scene_shadow: Vec<u8>,
    particle_buf: Buffer,
    /// Device-local atlases the fragment shader loads (bindings 4 and 7).
    /// The transfer queue copies a resident mip and blits it here. The CPU
    /// does not write these texels.
    sampled_color: Image,
    sampled_normal: Image,
    sampled_size: [(u32, u32); 2],
    sampled_layout: [u32; 2],
    sampled_cleared: [bool; 2],
    /// One texel copied from the colour atlas by the transfer. The CPU never
    /// stores a mip byte here.
    atlas_readback: Buffer,
    /// The same texel copied from the device page that was blitted. The CPU
    /// never stores a mip byte here either.
    page_readback: Buffer,
    material_key: u64,
    normal_key: u64,
    /// Keys of [`crate::world::World::stream_textures`], in register order.
    stream_keys: Vec<u64>,
    /// Mip chains registered with the streamer. Parallel to its texture indices.
    stream_sources: Vec<StreamSource>,
    /// Device images that received a real transfer copy.
    stream_pages: Vec<StreamPage>,
    /// Signalled by a transfer submit the next graphics submit must see.
    transfer_done: Handle,
    /// The graphics submit waits on `transfer_done` once.
    transfer_wait: bool,
    /// Mesh SDF instances for the software tracer (mesh_field.rs; binding 5).
    /// Device-local and resident: what it holds is `mesh_field_shadow`, and a frame
    /// records only `mesh_field_dirty` (byte ranges) with vkCmdUpdateBuffer, so a
    /// moving instance costs its 128-byte record and no device wait.
    mesh_field: Buffer,
    mesh_field_cap: u64,
    mesh_field_key: u64,
    mesh_field_layout: u64,
    mesh_field_shadow: Vec<u8>,
    mesh_field_dirty: Vec<(usize, usize)>,
    /// GI frame path (lighting_gpu.rs).
    lighting: Lighting,
    /// Fragment shader for the next raster pipeline built, instead of overlay.frag.
    frag_override: Option<&'static [u8]>,
    /// The vertex shader the next raster pipeline uses instead of scene.vert.
    vert_override: Option<&'static [u8]>,
    /// How long each pipeline took to build.
    pipeline_times: Vec<(String, Duration)>,
    desc_layout: Handle,
    desc_pool: Handle,
    /// Raster set of the frame being recorded: `raster_sets[flight]`.
    desc_set: Handle,
    raster_sets: [Handle; 2],
    /// Command pool on the compute family (the async light cache rounds).
    compute_pool: Handle,
    /// Clear colour of the picture: the sky's radiance through the tone curve, so a
    /// pixel that meets nothing shows the light a ray that leaves the scene brings.
    background: [f32; 3],
    audio_rays: Buffer,
    audio_gains: Buffer,
    audio_desc_layout: Handle,
    audio_desc_pool: Handle,
    audio_set: Handle,
    audio_layout: Handle,
    audio_pipe: Handle,
    audio_cmd: Handle,
    audio_fence: Handle,
    /// Lazy `media_check.comp` dispatch. Absent until the parity test asks.
    media_check: Option<media_check::Pipe>,
    /// Last `media_check.comp` timestamp span, milliseconds. `None` before a dispatch.
    media_ms: Option<f64>,
    /// Workgroups for the next media dispatch. The parity check uses 1.
    media_groups: u32,
    /// Froxel targets, pipelines, and the standalone command buffer.
    froxel: Option<fog::Targets>,
    /// What the next frame's froxel pass runs. None skips the pass.
    froxel_frame: Option<fog::FrameJob>,
    /// Last froxel timestamp span, milliseconds. Zero before a dispatch.
    froxel_pass_ms: f64,
    /// Bytes of the live froxel allocation. Zero when the pass is off.
    froxel_vram: u64,
    /// The frame command buffer wrote froxel timestamps that have not been read.
    froxel_pending: bool,
    /// This frame's `record_froxel` copied columns and wrote the compose header.
    froxel_wrote: bool,
    submit_pending: bool,
    /// Frame-slot fence has a submit that has not been waited. A reset fence
    /// with no submit stays false: polling it would wait forever.
    flight_live: [bool; 2],
    /// The lifecycle is draining and destroying. Lighting waits must not block.
    lifecycle_release: bool,
    /// The instance has been destroyed.
    torn_down: bool,
    /// 0 yields, 1 destroys lighting, 2 destroys the swapchain, 3 releases
    /// images and pipelines, 4 destroys the device, 5 the surface, then the instance.
    teardown: u8,
    /// The drain submits have been queued.
    shutdown_armed: bool,
    /// Fence for the graphics-queue drain submit.
    shutdown_fence: Handle,
    /// Fence for the compute-queue drain submit, when that queue is distinct.
    shutdown_compute: Handle,
    timestamp_period: f32,
    timestamp_bits: u32,
    query_pool: Handle,
    /// Two timestamps per frame, bloom start and display end. Independent of `--gpu-times`.
    hdr_time_pool: Handle,
    /// `--gpu-times N`, or `GENOS_GPU_TIMES` when that argument was omitted:
    /// per-pass GPU times of every frame and light build,
    /// averaged and printed every `frames` frames ([`GPU_TIMES_FRAMES`] by default).
    gpu_times: Option<Box<GpuTimes>>,
    profile_submit: bool,
    stamp_pending: [bool; 2],
    inflight: VecDeque<usize>,
    ready: VecDeque<Duration>,
    submitted_slot: Option<usize>,
    antialias: Antialias,
    /// Last known layout of each flight's color image. `0` is undefined. `6` is transfer source.
    color_layout: [u32; 2],
    keep_pass: Handle,
    aa_src: [Buffer; 2],
    aa_dst: [Buffer; 2],
    aa_desc_layout: Handle,
    aa_layout: Handle,
    aa_fxaa: Handle,
    aa_pool: Handle,
    aa_sets: [Handle; 2],
    fns: Fns,
    memory_props: MemProps,
    /// Upload buffers that went to device-local memory, and all upload buffers.
    upload_local: std::cell::Cell<[u32; 2]>,
    /// Device allocations to fail with out-of-device-memory before the driver.
    fail_device_allocs: std::cell::Cell<u32>,
    /// Device bytes allocated minus the bytes passed to the free that calls
    /// the driver. Bookkeeping that drops a mip without that free does not move.
    device_bytes: std::cell::Cell<u64>,
    /// Dedicated transfer queue when the device has one, otherwise the graphics queue.
    transfer_queue: Handle,
    transfer_family: u32,
    transfer_pool: Handle,
    transfer_cmd: Handle,
    transfer_fence: Handle,
    transfer_busy: bool,
    transfer_bytes: u64,
    transfer_submitted: Option<Instant>,
    materials: genos_load::MaterialSettings,
    vram: genos_load::VramBudget,
    streamer: genos_load::Streamer,
    non_texture_extra: u64,
    budget_tick: u32,
    cached_budget: u64,
    cached_usage: u64,
    /// `(heapBudget, heapUsage)` in place of the device query.
    heap_override: Option<(u64, u64)>,
    budget_extension: bool,
    memory_budget_query: Option<FnMemProps>,
    frame_mark: Option<Instant>,
}

struct Image {
    image: Handle,
    view: Handle,
    memory: Handle,
}

struct Buffer {
    buffer: Handle,
    memory: Handle,
    size: u64,
}

struct StreamSource {
    index: usize,
    color: bool,
    chain: crate::material_stream::Chain,
    staging: Buffer,
    mip_offset: Vec<u64>,
    /// Mip last blitted into the device atlas. `None` until the first copy.
    shown: Option<u32>,
}

struct StreamPage {
    index: usize,
    level: u32,
    image: Handle,
    memory: Handle,
    bytes: u64,
    type_bits: u32,
    copied: bool,
    /// Layout after the last transfer command that touched this image.
    layout: u32,
}

/// Device mip image after the staging copy. [`Gpu::publish_resident`] is the
/// only atlas write, and it accepts this handle rather than a buffer.
struct ResidentPage {
    image: Handle,
    width: u32,
    height: u32,
}

struct UnboundImage {
    image: Handle,
    bytes: u64,
    type_bits: u32,
}

#[derive(Clone, Copy)]
struct MemProps {
    count: u32,
    types: [u32; 32],
    device_local_bytes: u64,
}

#[allow(dead_code)]
struct Fns {
    destroy_instance: FnDestroy,
    destroy_device: FnDestroy,
    create_wayland_surface: FnWayland,
    destroy_surface: Fn2,
    surface_support: FnSupport,
    surface_caps: FnCaps,
    surface_formats: FnCount,
    surface_modes: FnCount,
    create_swapchain: FnSwapchain,
    destroy_swapchain: Fn2,
    swapchain_images: FnCount,
    acquire: FnAcquire,
    present: FnPresent,
    create_image_view: FnCreateView,
    destroy_image_view: Fn2,
    create_shader: FnShader,
    destroy_shader: Fn2,
    create_layout: FnLayout,
    destroy_layout: Fn2,
    create_render_pass: FnRenderPass,
    destroy_render_pass: Fn2,
    create_pipelines: FnPipelines,
    destroy_pipeline: Fn2,
    create_framebuffer: FnFramebuffer,
    destroy_framebuffer: Fn2,
    create_pool: FnPool,
    destroy_pool: Fn2,
    alloc_cmd: FnAllocCmd,
    begin_cmd: FnBegin,
    end_cmd: FnCmdResult,
    reset_cmd: FnReset,
    cmd_begin_rp: FnBeginRp,
    cmd_end_rp: FnCmd,
    cmd_bind_pipe: FnBindPipe,
    cmd_bind_vb: FnBindVb,
    cmd_draw: FnDraw,
    cmd_push: FnPush,
    cmd_viewport: FnViewport,
    cmd_scissor: FnScissor,
    cmd_barrier: FnBarrier,
    cmd_copy_image: FnCopyImage,
    cmd_copy_to_buffer: FnCopyBuf,
    cmd_copy_buffer: FnCopyBuffer,
    cmd_copy_to_image: FnCopyToImage,
    cmd_blit: FnBlit,
    cmd_clear_color: FnClearColor,
    create_sem: FnCreateSem,
    destroy_sem: Fn2,
    create_fence: FnFence,
    destroy_fence: Fn2,
    wait_fences: FnWait,
    reset_fences: FnResetFences,
    queue_submit: FnSubmit,
    fence_status: FnFenceStatus,
    cmd_bind_set: FnBindSet,
    cmd_dispatch: FnDispatch,
    cmd_dispatch_indirect: FnDispatchIndirect,
    cmd_update_buffer: FnUpdateBuffer,
    cmd_fill_buffer: FnFillBuffer,
    update_desc: FnUpdateDesc,
    create_desc_layout: FnDescLayout,
    destroy_desc_layout: Fn2,
    create_desc_pool: FnDescPool,
    destroy_desc_pool: Fn2,
    alloc_desc: FnAllocDesc,
    create_compute: FnPipelines,
    device_wait: FnDevice,
    create_buffer: FnBuffer,
    destroy_buffer: Fn2,
    buffer_reqs: FnBufReq,
    alloc_mem: FnAlloc,
    free_mem: Fn2,
    bind_buffer: FnBindBuf,
    map_mem: FnMap,
    unmap_mem: FnUnmap,
    create_image: FnImage,
    destroy_image: Fn2,
    image_reqs: FnImgReq,
    bind_image: FnBindImg,
    enumerate_devices: FnEnumDev,
    queue_families: FnQueues,
    mem_props: FnMemProps,
    create_query_pool: FnQueryPool,
    destroy_query_pool: Fn2,
    cmd_reset_query: FnResetQuery,
    cmd_write_timestamp: FnWriteTs,
    get_query_results: FnGetQuery,
}

type GetProc = unsafe extern "system" fn(Handle, *const c_char) -> Pfn;
type FnDestroy = unsafe extern "system" fn(Handle, *const c_void);
type Fn2 = unsafe extern "system" fn(Handle, Handle, *const c_void);
type FnDevice = unsafe extern "system" fn(Handle) -> VkResult;
type FnQueryPool =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnResetQuery = unsafe extern "system" fn(Handle, Handle, u32, u32);
type FnWriteTs = unsafe extern "system" fn(Handle, u32, Handle, u32);
type FnGetQuery =
    unsafe extern "system" fn(Handle, Handle, u32, u32, usize, *mut c_void, u64, u32) -> VkResult;
type FnCmd = unsafe extern "system" fn(Handle);
type FnUpdateBuffer = unsafe extern "system" fn(Handle, Handle, u64, u64, *const c_void);
type FnFillBuffer = unsafe extern "system" fn(Handle, Handle, u64, u64, u32);
type FnCmdResult = unsafe extern "system" fn(Handle) -> VkResult;
type FnCreateDevice =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnGetQueue = unsafe extern "system" fn(Handle, u32, u32, *mut Handle);
type FnWayland =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnSupport = unsafe extern "system" fn(Handle, u32, Handle, *mut u32) -> VkResult;
type FnCaps = unsafe extern "system" fn(Handle, Handle, *mut u8) -> VkResult;
type FnCount = unsafe extern "system" fn(Handle, Handle, *mut u32, *mut u8) -> VkResult;
type FnSwapchain =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAcquire =
    unsafe extern "system" fn(Handle, Handle, u64, Handle, Handle, *mut u32) -> VkResult;
type FnPresent = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
type FnCreateView =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnShader =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnLayout =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnRenderPass =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnPipelines = unsafe extern "system" fn(
    Handle,
    Handle,
    u32,
    *const u8,
    *const c_void,
    *mut Handle,
) -> VkResult;
type FnFramebuffer =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnPool = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAllocCmd = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
type FnBegin = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
type FnReset = unsafe extern "system" fn(Handle, u32) -> VkResult;
type FnBeginRp = unsafe extern "system" fn(Handle, *const u8, u32);
type FnBindPipe = unsafe extern "system" fn(Handle, u32, Handle);
type FnBindVb = unsafe extern "system" fn(Handle, u32, u32, *const Handle, *const u64);
type FnDraw = unsafe extern "system" fn(Handle, u32, u32, u32, u32);
type FnPush = unsafe extern "system" fn(Handle, Handle, u32, u32, u32, *const c_void);
type FnViewport = unsafe extern "system" fn(Handle, u32, u32, *const f32);
type FnScissor = unsafe extern "system" fn(Handle, u32, u32, *const i32);
type FnBarrier = unsafe extern "system" fn(
    Handle,
    u32,
    u32,
    u32,
    u32,
    *const c_void,
    u32,
    *const c_void,
    u32,
    *const u8,
);
type FnCopyImage = unsafe extern "system" fn(Handle, Handle, i32, Handle, i32, u32, *const u8);
type FnCopyBuf = unsafe extern "system" fn(Handle, Handle, i32, Handle, u32, *const u8);
type FnCopyBuffer = unsafe extern "system" fn(Handle, Handle, Handle, u32, *const u8);
type FnCopyToImage = unsafe extern "system" fn(Handle, Handle, Handle, i32, u32, *const u8);
type FnBlit = unsafe extern "system" fn(Handle, Handle, i32, Handle, i32, u32, *const u8, u32);
type FnClearColor = unsafe extern "system" fn(Handle, Handle, i32, *const u8, u32, *const u8);
type FnCreateSem =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnFence = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnWait = unsafe extern "system" fn(Handle, u32, *const Handle, u32, u64) -> VkResult;
type FnResetFences = unsafe extern "system" fn(Handle, u32, *const Handle) -> VkResult;
type FnSubmit = unsafe extern "system" fn(Handle, u32, *const u8, Handle) -> VkResult;
type FnFenceStatus = unsafe extern "system" fn(Handle, Handle) -> VkResult;
type FnBindSet =
    unsafe extern "system" fn(Handle, u32, Handle, u32, u32, *const Handle, u32, *const u32);
type FnDispatch = unsafe extern "system" fn(Handle, u32, u32, u32);
type FnDispatchIndirect = unsafe extern "system" fn(Handle, Handle, u64);
type FnUpdateDesc = unsafe extern "system" fn(Handle, u32, *const u8, u32, *const u8);
type FnDescLayout =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnDescPool =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAllocDesc = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
type FnBuffer =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnBufReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
type FnAlloc = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnBindBuf = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
type FnMap = unsafe extern "system" fn(Handle, Handle, u64, u64, u32, *mut *mut c_void) -> VkResult;
type FnUnmap = unsafe extern "system" fn(Handle, Handle);
type FnImage = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnImgReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
type FnBindImg = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
type FnEnumDev = unsafe extern "system" fn(Handle, *mut u32, *mut Handle) -> VkResult;
type FnQueues = unsafe extern "system" fn(Handle, *mut u32, *mut u8);
type FnMemProps = unsafe extern "system" fn(Handle, *mut u8);

impl Gpu {
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn open(
        display: *mut c_void,
        wl_surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        unsafe {
            let lib = dlopen(b"libvulkan.so.1\0".as_ptr() as *const c_char, 2);
            if lib.is_null() {
                return Err("libvulkan.so.1 did not open".into());
            }
            let get_instance: GetProc = transmute(dlsym(
                lib,
                b"vkGetInstanceProcAddr\0".as_ptr() as *const c_char,
            ));
            let enumerate_ext: unsafe extern "system" fn(
                *const c_void,
                *mut u32,
                *mut u8,
            ) -> VkResult = transmute(get_instance(
                std::ptr::null_mut(),
                b"vkEnumerateInstanceExtensionProperties\0".as_ptr() as *const c_char,
            ));
            let create_instance: unsafe extern "system" fn(
                *const u8,
                *const c_void,
                *mut Handle,
            ) -> VkResult = transmute(get_instance(
                std::ptr::null_mut(),
                b"vkCreateInstance\0".as_ptr() as *const c_char,
            ));

            let mut ext_count = 0u32;
            check(
                enumerate_ext(std::ptr::null(), &mut ext_count, std::ptr::null_mut()),
                "extensions",
            )?;
            let mut inst_listed = vec![0u8; ext_count as usize * 260];
            if ext_count > 0 {
                check(
                    enumerate_ext(std::ptr::null(), &mut ext_count, inst_listed.as_mut_ptr()),
                    "instance extension list",
                )?;
            }
            let surface_ext = b"VK_KHR_surface\0";
            let wayland_ext = b"VK_KHR_wayland_surface\0";
            let color_ext = b"VK_EXT_swapchain_colorspace\0";
            let props2_ext = b"VK_KHR_get_physical_device_properties2\0";
            let has_props2 = extension_listed(
                &inst_listed,
                ext_count,
                b"VK_KHR_get_physical_device_properties2",
            );
            let mut ext_ptrs = vec![
                surface_ext.as_ptr() as *const c_char,
                wayland_ext.as_ptr() as *const c_char,
            ];
            if has_props2 {
                ext_ptrs.push(props2_ext.as_ptr() as *const c_char);
            }
            if extension_listed(&inst_listed, ext_count, b"VK_EXT_swapchain_colorspace") {
                ext_ptrs.push(color_ext.as_ptr() as *const c_char);
            }
            let app_name = b"genos-camera\0";
            #[repr(C)]
            struct AppInfo {
                s_type: i32,
                next: *const c_void,
                name: *const c_char,
                app_version: u32,
                engine: *const c_char,
                engine_version: u32,
                api: u32,
            }
            let app = AppInfo {
                s_type: 0,
                next: std::ptr::null(),
                name: app_name.as_ptr() as *const c_char,
                app_version: 1,
                engine: app_name.as_ptr() as *const c_char,
                engine_version: 1,
                api: API_VERSION,
            };
            #[repr(C)]
            struct InstInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                app: *const AppInfo,
                enabled_layers: u32,
                layer_names: *const *const c_char,
                enabled_exts: u32,
                ext_names: *const *const c_char,
            }
            let info = InstInfo {
                s_type: 1,
                next: std::ptr::null(),
                flags: 0,
                app: &app,
                enabled_layers: 0,
                layer_names: std::ptr::null(),
                enabled_exts: u32::try_from(ext_ptrs.len()).unwrap_or(0),
                ext_names: ext_ptrs.as_ptr(),
            };
            let mut instance = std::ptr::null_mut();
            check(
                create_instance(
                    &info as *const InstInfo as *const u8,
                    std::ptr::null(),
                    &mut instance,
                ),
                "vkCreateInstance",
            )?;
            let load = |name: &[u8]| get_instance(instance, name.as_ptr() as *const c_char);

            let enumerate_devices: FnEnumDev = transmute(load(b"vkEnumeratePhysicalDevices\0"));
            let queue_families: FnQueues =
                transmute(load(b"vkGetPhysicalDeviceQueueFamilyProperties\0"));
            let mem_props: FnMemProps = transmute(load(b"vkGetPhysicalDeviceMemoryProperties\0"));
            let create_device: FnCreateDevice = transmute(load(b"vkCreateDevice\0"));
            let get_device: GetProc = transmute(load(b"vkGetDeviceProcAddr\0"));
            let create_wayland_surface: FnWayland = transmute(load(b"vkCreateWaylandSurfaceKHR\0"));
            let destroy_surface: Fn2 = transmute(load(b"vkDestroySurfaceKHR\0"));
            let surface_support: FnSupport =
                transmute(load(b"vkGetPhysicalDeviceSurfaceSupportKHR\0"));
            let surface_caps: FnCaps =
                transmute(load(b"vkGetPhysicalDeviceSurfaceCapabilitiesKHR\0"));
            let surface_formats: FnCount =
                transmute(load(b"vkGetPhysicalDeviceSurfaceFormatsKHR\0"));
            let surface_modes: FnCount =
                transmute(load(b"vkGetPhysicalDeviceSurfacePresentModesKHR\0"));
            let destroy_instance: FnDestroy = transmute(load(b"vkDestroyInstance\0"));

            let mut dev_count = 0u32;
            check(
                enumerate_devices(instance, &mut dev_count, std::ptr::null_mut()),
                "devices",
            )?;
            if dev_count == 0 {
                return Err("no Vulkan physical device".into());
            }
            let mut devices = vec![std::ptr::null_mut(); dev_count as usize];
            check(
                enumerate_devices(instance, &mut dev_count, devices.as_mut_ptr()),
                "device list",
            )?;

            #[repr(C)]
            struct WaylandInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                display: *mut c_void,
                surface: *mut c_void,
            }
            let wayland = WaylandInfo {
                s_type: 1000006000,
                next: std::ptr::null(),
                flags: 0,
                display,
                surface: wl_surface,
            };
            let mut vk_surface = std::ptr::null_mut();
            check(
                create_wayland_surface(
                    instance,
                    &wayland as *const WaylandInfo as *const u8,
                    std::ptr::null(),
                    &mut vk_surface,
                ),
                "wayland surface",
            )?;

            let get_props: unsafe extern "system" fn(Handle, *mut u8) =
                transmute(load(b"vkGetPhysicalDeviceProperties\0"));
            let mut chosen = None;
            for physical in devices {
                let mut family_count = 0u32;
                queue_families(physical, &mut family_count, std::ptr::null_mut());
                let mut families = vec![0u8; family_count as usize * 24];
                queue_families(physical, &mut family_count, families.as_mut_ptr());
                for index in 0..family_count {
                    let base = index as usize * 24;
                    let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                    let timestamp_bits =
                        u32::from_ne_bytes(families[base + 8..base + 12].try_into().unwrap());
                    let mut supported = 0u32;
                    check(
                        surface_support(physical, index, vk_surface, &mut supported),
                        "present support",
                    )?;
                    if flags & 0x3 == 0x3 && supported == 1 {
                        chosen = Some((physical, index, timestamp_bits, families, family_count));
                        break;
                    }
                }
                if chosen.is_some() {
                    break;
                }
            }
            let Some((physical, queue_family, timestamp_bits, families, family_count)) = chosen
            else {
                return Err("no graphics and compute queue can present".into());
            };
            let mut compute_family = queue_family;
            for index in 0..family_count {
                let base = index as usize * 24;
                let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                // Compute without graphics can run the gather beside the picture.
                if flags & 0x1 == 0 && flags & 0x2 != 0 {
                    compute_family = index;
                    break;
                }
            }
            let gfx_queues = u32::from_ne_bytes(
                families[queue_family as usize * 24 + 4..queue_family as usize * 24 + 8]
                    .try_into()
                    .unwrap(),
            );
            let mut prop_bytes = vec![0u8; 1024];
            get_props(physical, prop_bytes.as_mut_ptr());
            let timestamp_flag = u32::from_ne_bytes(prop_bytes[716..720].try_into().unwrap());
            let mut timestamp_period = f32::from_ne_bytes(prop_bytes[720..724].try_into().unwrap());
            if timestamp_flag == 0
                || timestamp_bits == 0
                || !(timestamp_period > 0.0 && timestamp_period < 10_000.0)
            {
                timestamp_period = 0.0;
            }

            let mut raw_props = [0u8; 4 + 32 * 8 + 4 + 16 * 16];
            mem_props(physical, raw_props.as_mut_ptr());
            let memory_props = parse_mem_props(&raw_props);

            let mut transfer_family = queue_family;
            let mut transfer_dedicated = false;
            for index in 0..family_count {
                if index == queue_family || index == compute_family {
                    continue;
                }
                let base = index as usize * 24;
                let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                let queues =
                    u32::from_ne_bytes(families[base + 4..base + 8].try_into().unwrap());
                // Transfer without graphics. A shared graphics family keeps the graphics queue.
                if flags & 0x4 != 0 && flags & 0x1 == 0 && queues > 0 {
                    transfer_family = index;
                    transfer_dedicated = true;
                    break;
                }
            }
            let priorities = [1.0f32, 0.0f32, 1.0f32];
            #[repr(C)]
            struct QueueInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                family: u32,
                count: u32,
                priorities: *const f32,
            }
            let separate = compute_family != queue_family;
            let mut queue_infos = [
                QueueInfo {
                    s_type: 2,
                    next: std::ptr::null(),
                    flags: 0,
                    family: queue_family,
                    count: 1,
                    priorities: priorities.as_ptr(),
                },
                QueueInfo {
                    s_type: 2,
                    next: std::ptr::null(),
                    flags: 0,
                    family: compute_family,
                    count: 1,
                    priorities: priorities.as_ptr().add(1),
                },
                QueueInfo {
                    s_type: 2,
                    next: std::ptr::null(),
                    flags: 0,
                    family: transfer_family,
                    count: 1,
                    priorities: priorities.as_ptr().add(2),
                },
            ];
            let mut queue_info_count = if separate {
                2
            } else if gfx_queues >= 2 {
                queue_infos[0].count = 2;
                1
            } else {
                1
            };
            if transfer_dedicated {
                queue_info_count += 1;
            }
            let enumerate_device_ext: unsafe extern "system" fn(
                Handle,
                *const c_char,
                *mut u32,
                *mut u8,
            ) -> VkResult = transmute(load(b"vkEnumerateDeviceExtensionProperties\0"));
            let mut dev_ext_count = 0u32;
            check(
                enumerate_device_ext(
                    physical,
                    std::ptr::null(),
                    &mut dev_ext_count,
                    std::ptr::null_mut(),
                ),
                "device extensions",
            )?;
            let mut dev_listed = vec![0u8; dev_ext_count as usize * 260];
            if dev_ext_count > 0 {
                check(
                    enumerate_device_ext(
                        physical,
                        std::ptr::null(),
                        &mut dev_ext_count,
                        dev_listed.as_mut_ptr(),
                    ),
                    "device extension list",
                )?;
            }
            let budget_query_ptr = load(if has_props2 {
                b"vkGetPhysicalDeviceMemoryProperties2KHR\0"
            } else {
                b"vkGetPhysicalDeviceMemoryProperties2\0"
            });
            let budget_extension =
                extension_listed(&dev_listed, dev_ext_count, b"VK_EXT_memory_budget")
                    && !budget_query_ptr.is_null();
            let memory_budget_query = if budget_extension {
                Some(transmute::<FnMemProps>(budget_query_ptr))
            } else {
                None
            };
            // VkPhysicalDeviceFeatures: only fragmentStoresAndAtomics (26), for the GI
            // v2 G-buffer. Every desktop driver (and lavapipe, MoltenVK) has it.
            let mut features = [0u32; 55];
            features[26] = 1;
            let swap_ext = b"VK_KHR_swapchain\0";
            let budget_ext = b"VK_EXT_memory_budget\0";
            let hdr_meta_ext = b"VK_EXT_hdr_metadata\0";
            let mut dev_exts = vec![swap_ext.as_ptr() as *const c_char];
            if budget_extension {
                dev_exts.push(budget_ext.as_ptr() as *const c_char);
            }
            let hdr_meta =
                device_extension_present(get_instance, instance, physical, b"VK_EXT_hdr_metadata");
            if hdr_meta {
                dev_exts.push(hdr_meta_ext.as_ptr() as *const c_char);
            }
            #[repr(C)]
            struct DevInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                queue_count: u32,
                queues: *const QueueInfo,
                layer_count: u32,
                layers: *const c_void,
                ext_count: u32,
                exts: *const *const c_char,
                features: *const c_void,
            }
            let dev_info = DevInfo {
                s_type: 3,
                next: std::ptr::null(),
                flags: 0,
                queue_count: queue_info_count,
                queues: queue_infos.as_ptr(),
                layer_count: 0,
                layers: std::ptr::null(),
                ext_count: u32::try_from(dev_exts.len()).unwrap_or(0),
                exts: dev_exts.as_ptr(),
                features: features.as_ptr() as *const c_void,
            };
            let mut device = std::ptr::null_mut();
            check(
                create_device(
                    physical,
                    &dev_info as *const DevInfo as *const u8,
                    std::ptr::null(),
                    &mut device,
                ),
                "vkCreateDevice",
            )?;
            let dload = |name: &[u8]| get_device(device, name.as_ptr() as *const c_char);
            let get_device_queue: FnGetQueue = transmute(dload(b"vkGetDeviceQueue\0"));
            let mut queue = std::ptr::null_mut();
            get_device_queue(device, queue_family, 0, &mut queue);
            let mut compute_queue = queue;
            if separate {
                get_device_queue(device, compute_family, 0, &mut compute_queue);
            } else if gfx_queues >= 2 {
                get_device_queue(device, queue_family, 1, &mut compute_queue);
            }
            let mut transfer_queue = queue;
            if transfer_dedicated {
                get_device_queue(device, transfer_family, 0, &mut transfer_queue);
            }
            let materials = genos_load::MaterialSettings::default();

            let mut gpu = Self {
                get_instance,
                get_device,
                instance,
                physical,
                device,
                queue,
                queue_family,
                compute_queue,
                compute_family,
                surface: vk_surface,
                swapchain: std::ptr::null_mut(),
                format: 44,
                color_space: crate::hdr_output::COLOR_SPACE_SRGB,
                hdr_output: crate::hdr_output::HdrSwapchain::Sdr,
                pq_white: 0.0,
                scrgb_white: 0.0,
                hdr_metadata: hdr_meta,
                extent_w: width.max(1),
                extent_h: height.max(1),
                present_mode: 2,
                swap_images: Vec::new(),
                color: Image::empty(),
                depth: Image::empty(),
                framebuffer: std::ptr::null_mut(),
                host: Buffer::empty(),
                vertex: Buffer::empty(),
                vertices: [Buffer::empty(), Buffer::empty()],
                shapes: Buffer::empty(),
                shapes_submitted: 0,
                instance_bufs: [Buffer::empty(), Buffer::empty()],
                draws: Vec::new(),
                vertex_count: 0,
                overlay_count: 0,
                render_pass: std::ptr::null_mut(),
                pipeline: std::ptr::null_mut(),
                depth_pipeline: std::ptr::null_mut(),
                overlay_pipeline: std::ptr::null_mut(),
                wire_pipeline: std::ptr::null_mut(),
                wire_on: false,
                copy_picture: false,
                unfilled: UnfilledControl::from_env()?,
                unfilled_saved: 0,
                last_unfilled: None,
                unfilled_eye: [0.0; 3],
                unfilled_yaw: 0.0,
                unfilled_pitch: 0.0,
                out_cli: genos_scene::DebugSettings::cli_out(),
                debug_out: String::new(),
                layout: std::ptr::null_mut(),
                pool: std::ptr::null_mut(),
                cmd: std::ptr::null_mut(),
                image_ready: std::ptr::null_mut(),
                render_done: std::ptr::null_mut(),
                fence: std::ptr::null_mut(),
                present_index: 0,
                cmds: [std::ptr::null_mut(); 2],
                fences: [std::ptr::null_mut(); 2],
                image_readies: [std::ptr::null_mut(); 2],
                render_dones: [std::ptr::null_mut(); 2],
                colors: [Image::empty(), Image::empty()],
                depths: [Image::empty(), Image::empty()],
                framebuffers: [std::ptr::null_mut(); 2],
                hosts: [Buffer::empty(), Buffer::empty()],
                flight: 0,
                scene_buf: Buffer::empty(),
                particle_buf: Buffer::empty(),
                sampled_color: Image::empty(),
                sampled_normal: Image::empty(),
                sampled_size: [(0, 0), (0, 0)],
                sampled_layout: [0, 0],
                sampled_cleared: [false, false],
                atlas_readback: Buffer::empty(),
                page_readback: Buffer::empty(),
                material_key: 0,
                normal_key: 0,
                stream_keys: Vec::new(),
                stream_sources: Vec::new(),
                stream_pages: Vec::new(),
                transfer_done: std::ptr::null_mut(),
                transfer_wait: false,
                mesh_field: Buffer::empty(),
                mesh_field_cap: 0,
                mesh_field_key: 0,
                mesh_field_layout: 0,
                mesh_field_shadow: Vec::new(),
                mesh_field_dirty: Vec::new(),
                lighting: Lighting::default(),
                frag_override: None,
                vert_override: None,
                pipeline_times: Vec::new(),
                desc_layout: std::ptr::null_mut(),
                desc_pool: std::ptr::null_mut(),
                desc_set: std::ptr::null_mut(),
                raster_sets: [std::ptr::null_mut(); 2],
                frame_scene: [Buffer::empty(), Buffer::empty()],
                frame_bytes: Vec::new(),
                scene_shadow: Vec::new(),
                compute_pool: std::ptr::null_mut(),
                background: [0.0; 3],
                audio_rays: Buffer::empty(),
                audio_gains: Buffer::empty(),
                audio_desc_layout: std::ptr::null_mut(),
                audio_desc_pool: std::ptr::null_mut(),
                audio_set: std::ptr::null_mut(),
                audio_layout: std::ptr::null_mut(),
                audio_pipe: std::ptr::null_mut(),
                audio_cmd: std::ptr::null_mut(),
                audio_fence: std::ptr::null_mut(),
                media_check: None,
                media_ms: None,
                media_groups: 1,
                froxel: None,
                froxel_frame: None,
                froxel_pass_ms: 0.0,
                froxel_vram: 0,
                froxel_pending: false,
                froxel_wrote: false,
                submit_pending: false,
                flight_live: [false, false],
                lifecycle_release: false,
                torn_down: false,
                teardown: 0,
                shutdown_armed: false,
                shutdown_fence: std::ptr::null_mut(),
                shutdown_compute: std::ptr::null_mut(),
                timestamp_period,
                timestamp_bits,
                query_pool: std::ptr::null_mut(),
                hdr_time_pool: std::ptr::null_mut(),
                gpu_times: {
                    let args: Vec<String> = std::env::args().skip(1).collect();
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    let env = std::env::var("GENOS_GPU_TIMES").ok();
                    let metrics_env = std::env::var("GENOS_GI_METRICS").ok();
                    let every = gpu_times_every(&refs, env.as_deref())?;
                    let record = gi_metrics_enabled(&refs, metrics_env.as_deref())?;
                    if every.is_none() && !record {
                        None
                    } else {
                        Some(Box::new(GpuTimes {
                            every: every.unwrap_or(u32::MAX),
                            print: every.is_some(),
                            record,
                            ..GpuTimes::default()
                        }))
                    }
                },
                profile_submit: false,
                stamp_pending: [false; 2],
                inflight: VecDeque::new(),
                ready: VecDeque::new(),
                submitted_slot: None,
                antialias: Antialias::Off,
                color_layout: [0, 0],
                keep_pass: std::ptr::null_mut(),
                aa_src: [Buffer::empty(), Buffer::empty()],
                aa_dst: [Buffer::empty(), Buffer::empty()],
                aa_desc_layout: std::ptr::null_mut(),
                aa_layout: std::ptr::null_mut(),
                aa_fxaa: std::ptr::null_mut(),
                aa_pool: std::ptr::null_mut(),
                aa_sets: [std::ptr::null_mut(); 2],
                fns: load_fns(
                    dload,
                    create_wayland_surface,
                    destroy_surface,
                    surface_support,
                    surface_caps,
                    surface_formats,
                    surface_modes,
                    destroy_instance,
                    enumerate_devices,
                    queue_families,
                    mem_props,
                ),
                memory_props,
                upload_local: std::cell::Cell::new([0, 0]),
                fail_device_allocs: std::cell::Cell::new(0),
                device_bytes: std::cell::Cell::new(0),
                transfer_queue,
                transfer_family,
                transfer_pool: std::ptr::null_mut(),
                transfer_cmd: std::ptr::null_mut(),
                transfer_fence: std::ptr::null_mut(),
                transfer_busy: false,
                transfer_bytes: 0,
                transfer_submitted: None,
                materials,
                vram: genos_load::VramBudget::default(),
                streamer: genos_load::Streamer::new(
                    materials.quality_floor(),
                    materials.mip_bias,
                    materials.dwell_frames,
                ),
                non_texture_extra: 0,
                budget_tick: 0,
                cached_budget: 0,
                cached_usage: 0,
                heap_override: None,
                budget_extension,
                memory_budget_query,
                frame_mark: None,
            };
            let _ = (destroy_instance, surface_support);
            gpu.create_static_objects()?;
            gpu.recreate(width, height)?;
            block(|| gpu.make_lighting())?;
            gpu.make_lighting_targets()?;
            let launch = genos_scene::DebugSettings::default().effective(gpu.out_cli.as_deref());
            gpu.debug_out = launch.out.clone();
            crate::unfilled::enforce_unfilled_cap(std::path::Path::new(&gpu.debug_out))?;
            Ok(gpu)
        }
    }

    fn create_static_objects(&mut self) -> Result<(), String> {
        unsafe {
            self.render_pass = self.make_render_pass(false)?;
            self.keep_pass = self.make_render_pass(true)?;
            self.desc_layout = self.make_desc_layout()?;
            self.layout = self.make_layout()?;
            let start = Instant::now();
            self.pipeline = block(|| self.make_pipeline(true, true, false))?;
            self.note_pipeline_time("scene", start);
            self.depth_pipeline = block(|| self.make_depth_pipeline())?;
            self.overlay_pipeline = block(|| self.make_pipeline(false, false, false))?;
            self.wire_pipeline = block(|| self.make_pipeline(false, false, true))?;
            #[repr(C)]
            struct PoolInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                family: u32,
            }
            let pool = PoolInfo {
                s_type: 39,
                next: std::ptr::null(),
                flags: 0x2,
                family: self.queue_family,
            };
            check(
                (self.fns.create_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.pool,
                ),
                "command pool",
            )?;
            #[repr(C)]
            struct AllocInfo {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                level: u32,
                count: u32,
            }
            let alloc = AllocInfo {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.pool,
                level: 0,
                count: 2,
            };
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &alloc as *const AllocInfo as *const u8,
                    self.cmds.as_mut_ptr(),
                ),
                "command buffer",
            )?;
            self.cmd = self.cmds[0];
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            for slot in 0..2 {
                self.image_readies[slot] = self.make_sem()?;
                self.render_dones[slot] = self.make_sem()?;
                check(
                    (self.fns.create_fence)(
                        self.device,
                        &fence as *const FenceInfo as *const u8,
                        std::ptr::null(),
                        &mut self.fences[slot],
                    ),
                    "fence",
                )?;
            }
            self.image_ready = self.image_readies[0];
            self.render_done = self.render_dones[0];
            self.fence = self.fences[0];
            let transfer_pool = PoolInfo {
                s_type: 39,
                next: std::ptr::null(),
                flags: 0x2,
                family: self.transfer_family,
            };
            check(
                (self.fns.create_pool)(
                    self.device,
                    &transfer_pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.transfer_pool,
                ),
                "transfer pool",
            )?;
            let transfer_alloc = AllocInfo {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.transfer_pool,
                level: 0,
                count: 1,
            };
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &transfer_alloc as *const AllocInfo as *const u8,
                    &mut self.transfer_cmd,
                ),
                "transfer command buffer",
            )?;
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.transfer_fence,
                ),
                "transfer fence",
            )?;
            self.transfer_done = self.make_sem()?;
            if self.timestamp_period > 0.0 {
                self.query_pool = self.make_query_pool()?;
                self.hdr_time_pool = self.make_query_pool_n(4)?;
                if self.gpu_times.is_some() {
                    let pool = self.make_query_pool_n(2 * FRAME_STAMPS)?;
                    let lighting_pool = self.make_query_pool_n(2 * LIGHTING_STAMPS)?;
                    if let Some(times) = self.gpu_times.as_mut() {
                        times.pool = pool;
                        times.lighting_pool = lighting_pool;
                    }
                }
            } else if self.gpu_times.take().is_some() {
                eprintln!("GI metrics: this device has no timestamp queries");
            }
            let start = Instant::now();
            self.make_scene_buffers()?;
            self.note_pipeline_time("scene buffers", start);
            self.make_aa_pipes()?;
        }
        Ok(())
    }

    fn recreate(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.idle_device();
        self.destroy_targets();
        self.destroy_swapchain();
        self.create_targets(width, height)
    }

    /// Mailbox (1) when the surface lists it, then immediate (0), then FIFO (2).
    fn choose_present(&self) -> Result<i32, String> {
        const MAILBOX: i32 = 1;
        const IMMEDIATE: i32 = 0;
        const FIFO: i32 = 2;
        let mut count = 0u32;
        unsafe {
            check(
                (self.fns.surface_modes)(
                    self.physical,
                    self.surface,
                    &mut count,
                    std::ptr::null_mut(),
                ),
                "present modes",
            )?;
            if count == 0 {
                return Ok(FIFO);
            }
            let mut raw = vec![0u8; count as usize * 4];
            check(
                (self.fns.surface_modes)(self.physical, self.surface, &mut count, raw.as_mut_ptr()),
                "present mode list",
            )?;
            let mut mailbox = false;
            let mut immediate = false;
            for chunk in raw.chunks(4).take(count as usize) {
                let mode = i32::from_ne_bytes(chunk.try_into().unwrap());
                if mode == MAILBOX {
                    mailbox = true;
                } else if mode == IMMEDIATE {
                    immediate = true;
                }
            }
            if mailbox {
                Ok(MAILBOX)
            } else if immediate {
                Ok(IMMEDIATE)
            } else {
                Ok(FIFO)
            }
        }
    }

    fn query_surface_formats(&self) -> Result<Vec<u8>, String> {
        unsafe {
            let mut format_count = 0u32;
            check(
                (self.fns.surface_formats)(
                    self.physical,
                    self.surface,
                    &mut format_count,
                    std::ptr::null_mut(),
                ),
                "formats",
            )?;
            let mut formats = vec![0u8; format_count as usize * 8];
            if format_count > 0 {
                check(
                    (self.fns.surface_formats)(
                        self.physical,
                        self.surface,
                        &mut format_count,
                        formats.as_mut_ptr(),
                    ),
                    "format list",
                )?;
            }
            Ok(formats)
        }
    }

    /// 44, then 37, ignoring color space. An HDR pair replaces that only when listed.
    fn choose_surface(&self, formats: &[u8]) -> (i32, i32, crate::hdr_output::HdrSwapchain) {
        let mut sdr_format = 44;
        let mut found = false;
        for chunk in formats.chunks(8) {
            let Ok(bytes) = chunk[0..4].try_into() else {
                continue;
            };
            let format = i32::from_ne_bytes(bytes);
            if format == 44 || format == 37 {
                sdr_format = format;
                found = true;
                if format == 44 {
                    break;
                }
            }
        }
        if !found && formats.len() >= 4 {
            sdr_format = i32::from_ne_bytes(formats[0..4].try_into().unwrap());
        }
        let mut listed = Vec::new();
        for chunk in formats.chunks(8) {
            if chunk.len() < 8 {
                continue;
            }
            let format = i32::from_ne_bytes(chunk[0..4].try_into().unwrap());
            let color_space = i32::from_ne_bytes(chunk[4..8].try_into().unwrap());
            listed.push(crate::hdr_output::ListedFormat { format, color_space });
        }
        let force_sdr = self.lighting.hdr.legacy_bytes
            || matches!(
                self.lighting.settings.preset,
                genos_scene::LightingPreset::Potato | genos_scene::LightingPreset::Low
            );
        crate::hdr_output::surface_choice(
            sdr_format,
            &listed,
            crate::hdr_output::present_host(),
            self.lighting.hdr.output,
            force_sdr,
        )
    }

    /// Recreate the color targets when the listed pair is not the one in use.
    fn refresh_hdr_surface(&mut self) -> Result<(), String> {
        let formats = self.query_surface_formats()?;
        let (format, space, mode) = self.choose_surface(&formats);
        if format == self.format && space == self.color_space && mode == self.hdr_output {
            return Ok(());
        }
        self.idle_device();
        self.destroy_targets();
        self.destroy_swapchain();
        self.destroy_color_pipelines();
        self.format = format;
        self.color_space = space;
        self.hdr_output = mode;
        self.render_pass = self.make_render_pass(false)?;
        self.keep_pass = self.make_render_pass(true)?;
        self.pipeline = block(|| self.make_pipeline(true, true, false))?;
        self.depth_pipeline = block(|| self.make_depth_pipeline())?;
        self.overlay_pipeline = block(|| self.make_pipeline(false, false, false))?;
        self.wire_pipeline = block(|| self.make_pipeline(false, false, true))?;
        self.remake_gbuffer_pipe()?;
        self.create_targets(self.extent_w, self.extent_h)
    }

    fn destroy_color_pipelines(&mut self) {
        unsafe {
            for pipe in [
                &mut self.pipeline,
                &mut self.depth_pipeline,
                &mut self.overlay_pipeline,
                &mut self.wire_pipeline,
                &mut self.lighting.gbuffer_pipe,
            ] {
                if !pipe.is_null() {
                    (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null());
                    *pipe = std::ptr::null_mut();
                }
            }
            if !self.render_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.render_pass, std::ptr::null());
                self.render_pass = std::ptr::null_mut();
            }
            if !self.keep_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.keep_pass, std::ptr::null());
                self.keep_pass = std::ptr::null_mut();
            }
        }
    }

    /// BT.2020 primaries, peak 1000 nits. Only when the created image is HDR10.
    fn write_hdr_metadata(&self) {
        if !self.hdr_metadata || self.hdr_output != crate::hdr_output::HdrSwapchain::Hdr10 {
            return;
        }
        unsafe {
            let raw = (self.get_device)(
                self.device,
                b"vkSetHdrMetadataEXT\0".as_ptr() as *const c_char,
            );
            if raw.is_null() {
                return;
            }
            type SetMeta = unsafe extern "system" fn(Handle, u32, *const Handle, *const HdrMeta);
            let set: SetMeta = transmute(raw);
            #[repr(C)]
            struct Xy {
                x: f32,
                y: f32,
            }
            #[repr(C)]
            struct HdrMeta {
                s_type: i32,
                next: *const c_void,
                red: Xy,
                green: Xy,
                blue: Xy,
                white: Xy,
                max_luminance: f32,
                min_luminance: f32,
                max_cll: f32,
                max_fall: f32,
            }
            let meta = HdrMeta {
                s_type: 1_000_105_000,
                next: std::ptr::null(),
                red: Xy { x: 0.708, y: 0.292 },
                green: Xy { x: 0.170, y: 0.797 },
                blue: Xy { x: 0.131, y: 0.046 },
                white: Xy {
                    x: 0.3127,
                    y: 0.3290,
                },
                max_luminance: 1000.0,
                min_luminance: 0.0,
                max_cll: 1000.0,
                max_fall: 1000.0,
            };
            let swap = self.swapchain;
            set(self.device, 1, &swap, &meta);
        }
    }

    fn create_targets(&mut self, width: u32, height: u32) -> Result<(), String> {
        unsafe {
            let mut caps = [0u8; 64];
            check(
                (self.fns.surface_caps)(self.physical, self.surface, caps.as_mut_ptr()),
                "caps",
            )?;
            let min_images = u32::from_ne_bytes(caps[0..4].try_into().unwrap());
            let max_images = u32::from_ne_bytes(caps[4..8].try_into().unwrap());
            let current_w = u32::from_ne_bytes(caps[8..12].try_into().unwrap());
            let current_h = u32::from_ne_bytes(caps[12..16].try_into().unwrap());
            let mut image_count = min_images.max(2);
            if max_images > 0 && image_count > max_images {
                image_count = max_images;
            }
            if current_w != u32::MAX && current_w > 0 {
                self.extent_w = current_w;
                self.extent_h = current_h;
            } else {
                self.extent_w = width.max(1);
                self.extent_h = height.max(1);
            }
            let mut format_count = 0u32;
            check(
                (self.fns.surface_formats)(
                    self.physical,
                    self.surface,
                    &mut format_count,
                    std::ptr::null_mut(),
                ),
                "formats",
            )?;
            let mut formats = vec![0u8; format_count as usize * 8];
            check(
                (self.fns.surface_formats)(
                    self.physical,
                    self.surface,
                    &mut format_count,
                    formats.as_mut_ptr(),
                ),
                "format list",
            )?;
            let (format, space, mode) = self.choose_surface(&formats);
            self.format = format;
            self.color_space = space;
            self.hdr_output = mode;
            // HDR replaces the SDR search only when that pair is listed. The search
            // itself still takes 44, then 37, and does not look at the color space.
            self.pq_white = crate::hdr_output::swapchain_nits(
                crate::hdr_output::HdrSwapchain::Hdr10,
                crate::hdr_output::GRAPHICS_WHITE_NITS,
            );
            self.scrgb_white = crate::hdr_output::swapchain_nits(
                crate::hdr_output::HdrSwapchain::Scrgb,
                crate::hdr_output::GRAPHICS_WHITE_NITS,
            );

            #[repr(C)]
            struct SwapInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                surface: Handle,
                min_count: u32,
                format: i32,
                color_space: i32,
                width: u32,
                height: u32,
                layers: u32,
                usage: u32,
                sharing: u32,
                queue_count: u32,
                queues: *const u32,
                transform: u32,
                alpha: u32,
                present: i32,
                clipped: u32,
                old: Handle,
            }
            // One wl_surface carries one wp_fifo. A second live swapchain on
            // the same surface is a protocol error on drivers that use
            // fifo-v1, and the surface is lost.
            if !self.swapchain.is_null() {
                return Err("swapchain: the previous swapchain is still live".into());
            }
            let present = self.choose_present()?;
            self.present_mode = present;
            // Mailbox needs a free image while one image is queued and one is on screen.
            if present == 1 {
                image_count = min_images.max(3);
                if max_images > 0 && image_count > max_images {
                    image_count = max_images;
                }
            }
            let transform = u32::from_ne_bytes(caps[36..40].try_into().unwrap());
            let swap = SwapInfo {
                s_type: 1000001000,
                next: std::ptr::null(),
                flags: 0,
                surface: self.surface,
                min_count: image_count,
                format: self.format,
                color_space: self.color_space,
                width: self.extent_w,
                height: self.extent_h,
                layers: 1,
                usage: 0x10 | 0x2,
                sharing: 0,
                queue_count: 0,
                queues: std::ptr::null(),
                transform,
                alpha: 1,
                present,
                clipped: 1,
                old: std::ptr::null_mut(),
            };
            check(
                (self.fns.create_swapchain)(
                    self.device,
                    &swap as *const SwapInfo as *const u8,
                    std::ptr::null(),
                    &mut self.swapchain,
                ),
                "swapchain",
            )?;
            self.write_hdr_metadata();
            let mut count = 0u32;
            check(
                (self.fns.swapchain_images)(
                    self.device,
                    self.swapchain,
                    &mut count,
                    std::ptr::null_mut(),
                ),
                "image count",
            )?;
            self.swap_images = vec![std::ptr::null_mut(); count as usize];
            check(
                (self.fns.swapchain_images)(
                    self.device,
                    self.swapchain,
                    &mut count,
                    self.swap_images.as_mut_ptr() as *mut u8,
                ),
                "image list",
            )?;

            let bpp = if self.format == crate::hdr_output::FORMAT_R16F {
                8
            } else {
                4
            };
            let bytes = self.extent_w as u64 * self.extent_h as u64 * bpp;
            for slot in 0..2 {
                self.colors[slot] = self.make_image(
                    self.format,
                    0x10 | 0x4 | 0x1 | 0x2,
                    1,
                    self.extent_w,
                    self.extent_h,
                )?;
                self.depths[slot] = self.make_image(126, 0x20, 2, self.extent_w, self.extent_h)?;
                self.color = copy_image(&self.colors[slot]);
                self.depth = copy_image(&self.depths[slot]);
                self.framebuffers[slot] = self.make_framebuffer()?;
                self.hosts[slot] = self.make_buffer(bytes, 0x2, Memory::Readback)?;
            }
            self.color = copy_image(&self.colors[0]);
            self.depth = copy_image(&self.depths[0]);
            self.framebuffer = self.framebuffers[0];
            self.host = copy_buffer(&self.hosts[0]);
            self.make_aa_targets()?;
            self.make_lighting_targets()?;
            Ok(())
        }
    }

    fn upload(&mut self, verts: &[GpuVertex]) -> Result<(), String> {
        if verts.is_empty() {
            return Ok(());
        }
        let bytes = (verts.len() * std::mem::size_of::<GpuVertex>()) as u64;
        if self.vertex.size < bytes {
            unsafe {
                if !self.vertex.buffer.is_null() {
                    (self.fns.destroy_buffer)(self.device, self.vertex.buffer, std::ptr::null());
                    (self.fns.free_mem)(self.device, self.vertex.memory, std::ptr::null());
                }
            }
            self.vertex = self.make_buffer(bytes.max(1024), 0x80, Memory::Upload)?;
            self.vertices[self.flight] = copy_buffer(&self.vertex);
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, self.vertex.memory, 0, bytes, 0, &mut mapped),
                "map vertices",
            )?;
            copy_to_mapped(
                mapped as *mut u8,
                verts.as_ptr() as *const u8,
                bytes as usize,
            );
            (self.fns.unmap_mem)(self.device, self.vertex.memory);
        }
        Ok(())
    }

    fn set_draws(&mut self, draws: Vec<DrawSpan>) {
        self.draws = draws;
    }

    /// Copy dirty shape ranges. A new tail does not wait. A reused span waits for both frames.
    fn upload_shapes(&mut self, pool: &crate::pool::MeshPool) -> Result<(), String> {
        let verts = pool.vertices();
        if verts.is_empty() {
            return Ok(());
        }
        let stride = std::mem::size_of::<GpuVertex>() as u64;
        let bytes = verts.len() as u64 * stride;
        let reuse = pool
            .dirty()
            .iter()
            .any(|(offset, _)| *offset < self.shapes_submitted);
        if reuse || self.shapes.size < bytes {
            self.wait_all_inflight()?;
        }
        if self.shapes.size < bytes {
            let next = self.make_buffer(
                bytes.max(self.shapes.size.saturating_mul(2)).max(4096),
                0x80,
                Memory::Upload,
            )?;
            self.write_verts_at(&next, 0, verts)?;
            let mut old = std::mem::replace(&mut self.shapes, next);
            self.destroy_buffer(&mut old);
        } else {
            let shapes = copy_buffer(&self.shapes);
            for &(offset, count) in pool.dirty() {
                let start = offset as usize;
                let end = start + count as usize;
                self.write_verts_at(&shapes, offset, &verts[start..end])?;
            }
        }
        self.shapes_submitted = verts.len() as u32;
        Ok(())
    }

    fn write_verts_at(
        &self,
        buffer: &Buffer,
        first: u32,
        verts: &[GpuVertex],
    ) -> Result<(), String> {
        if verts.is_empty() {
            return Ok(());
        }
        let stride = std::mem::size_of::<GpuVertex>() as u64;
        let offset = first as u64 * stride;
        let bytes = verts.len() as u64 * stride;
        let end = offset + bytes;
        if buffer.size < end {
            return Err("shape buffer is smaller than the block".into());
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, buffer.memory, 0, end, 0, &mut mapped),
                "map shape block",
            )?;
            copy_to_mapped(
                (mapped as *mut u8).add(offset as usize),
                verts.as_ptr() as *const u8,
                bytes as usize,
            );
            (self.fns.unmap_mem)(self.device, buffer.memory);
        }
        Ok(())
    }

    fn write_instances(&mut self, items: &[InstanceRec]) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(items.len() * 128);
        for item in items {
            for value in item
                .model
                .into_iter()
                .chain(item.color)
                .chain(item.atlas)
                .chain(item.normal_atlas)
                .chain(item.extra)
            {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        // GI holds its light while nothing it sees changes (lighting_gpu.rs).
        self.lighting.draw_key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            hasher.finish()
        };
        let slot = self.flight;
        if self.instance_bufs[slot].size < bytes.len() as u64 {
            let share = self.light_families();
            let next = self.make_buffer_queues(
                (bytes.len() as u64).max(5120),
                0x20,
                Memory::Upload,
                &share,
            )?;
            let mut old = std::mem::replace(&mut self.instance_bufs[slot], next);
            self.destroy_buffer(&mut old);
            self.write_raster_sets()?;
        }
        self.write_buffer(&self.instance_bufs[slot], &bytes)?;
        Ok(())
    }

    fn bind_flight(&mut self) {
        let slot = self.flight;
        self.cmd = self.cmds[slot];
        self.fence = self.fences[slot];
        self.image_ready = self.image_readies[slot];
        self.render_done = self.render_dones[slot];
        self.color = copy_image(&self.colors[slot]);
        self.depth = copy_image(&self.depths[slot]);
        self.framebuffer = self.framebuffers[slot];
        self.host = copy_buffer(&self.hosts[slot]);
        self.vertex = copy_buffer(&self.vertices[slot]);
    }

    fn record_and_submit(&mut self, matrix: &[f32; 16]) -> Result<(), String> {
        if self.unfilled.dump_pct.is_some() {
            self.copy_picture = true;
        }
        let profiled = self.profile_submit;
        self.profile_submit = false;
        if profiled && (self.query_pool.is_null() || self.timestamp_period <= 0.0) {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        self.bind_flight();
        let slot = self.flight;
        self.wait_fence(self.fence, "wait fence")?;
        self.flight_live[slot] = false;
        let fences = [self.fence];
        unsafe {
            self.collect_slot_after_wait(slot)?;
            if self.lighting_active() {
                self.take_unfilled(slot);
            }
            self.collect_hdr_time(slot);
            self.collect_froxel_time();
            self.collect_frame_times(slot);
            self.desc_set = self.raster_sets[slot];
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset fence",
            )?;
            let mut index = 0u32;
            let acquire = (self.fns.acquire)(
                self.device,
                self.swapchain,
                u64::MAX,
                self.image_ready,
                std::ptr::null_mut(),
                &mut index,
            );
            if acquire == OUT_OF_DATE {
                return Err("out of date".into());
            }
            check(acquire, "acquire")?;
            check((self.fns.reset_cmd)(self.cmd, 0), "reset cmd")?;
            #[repr(C)]
            struct BeginInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                inherit: *const c_void,
            }
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 1,
                inherit: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(self.cmd, &begin as *const BeginInfo as *const u8),
                "begin cmd",
            )?;
            if profiled {
                self.reset_queries(slot);
                // Top of pipe, before the GPU work. The value is device ticks.
                self.write_stamp(slot, 0, 1);
            }
            self.frame_stamp(slot, FrameStamp::Start);
            self.update_resident_scene();
            self.update_resident_fields();
            let swap = self.swap_images[index as usize];
            self.image_barrier(swap, 0, 7, 1, 0x1000, 0, 0x1000);
            let filtering = self.antialias != Antialias::Off;
            {
                let lighting = self.lighting_active();
                if lighting {
                    self.lighting_begin(slot);
                }
                self.frame_stamp(slot, FrameStamp::Setup);
                self.raster_scene(
                    self.framebuffer,
                    self.extent_w,
                    self.extent_h,
                    matrix,
                    !filtering && !lighting,
                );
                self.color_layout[slot] = 6;
                self.frame_stamp(slot, FrameStamp::Gbuffer);
                self.froxel_wrote = false;
                self.record_froxel()?;
                if !self.froxel_wrote {
                    self.release_froxel_header();
                }
                if lighting {
                    self.lighting_passes(slot);
                }
                self.frame_stamp(slot, FrameStamp::Lighting);
                // No antialias: the overlay is its own pass, after lighting, so its
                // time stays out of the lighting span.
                if lighting && !filtering && self.overlay_count > 0 {
                    self.raster_overlay();
                }
                self.frame_stamp(slot, FrameStamp::EarlyOverlay);
                if self.antialias == Antialias::Fxaa {
                    self.resolve_fxaa(slot)?;
                }
            }
            self.frame_stamp(slot, FrameStamp::Resolve);
            if filtering && self.overlay_count > 0 {
                self.raster_overlay();
            }
            self.frame_stamp(slot, FrameStamp::Overlay);
            self.copy_color_to_swapchain(swap);
            // The host copy is a whole picture over the bus; only a readback pays it.
            if self.copy_picture {
                self.copy_color_to_buffer();
            }
            self.image_barrier(swap, 7, 1000001002, 0x1000, 0x2000, 0x1000, 0);
            self.frame_stamp(slot, FrameStamp::Copy);
            if profiled {
                // Bottom of pipe, after the draw and the copies.
                // The value is device ticks, not the CPU time spent in submit.
                self.write_stamp(slot, 1, 0x2000);
            }
            check((self.fns.end_cmd)(self.cmd), "end cmd")?;

            let mut wait_stage = [0x400u32, 0x80];
            #[repr(C)]
            struct Submit {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                stages: *const u32,
                cmd_count: u32,
                cmds: *const Handle,
                signal_count: u32,
                signals: *const Handle,
            }
            let wait_sem = [self.image_ready, self.transfer_done];
            let mut wait_count = 1u32;
            if self.transfer_wait {
                wait_stage[1] = 0x80;
                wait_count = 2;
                self.transfer_wait = false;
            }
            let signal_sem = [self.render_done];
            let cmd = [self.cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count,
                waits: wait_sem.as_ptr(),
                stages: wait_stage.as_ptr(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 1,
                signals: signal_sem.as_ptr(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.fence,
                ),
                "submit",
            )?;
            self.flight_live[slot] = true;
            self.present_index = index;
            let status = (self.fns.fence_status)(self.device, self.fence);
            self.submit_pending = status == 1;
            if profiled {
                self.stamp_pending[slot] = true;
                self.inflight.push_back(slot);
                self.submitted_slot = Some(slot);
            } else {
                self.submitted_slot = None;
            }
            self.flight = 1 - self.flight;
        }
        Ok(())
    }

    fn make_query_pool(&mut self) -> Result<Handle, String> {
        self.make_query_pool_n(4)
    }

    fn make_query_pool_n(&mut self, count: u32) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            query_type: u32,
            query_count: u32,
            pipeline_statistics: u32,
        }
        let info = Info {
            s_type: 11,
            next: std::ptr::null(),
            flags: 0,
            // VK_QUERY_TYPE_TIMESTAMP is 2. Type 1 is pipeline statistics.
            query_type: 2,
            query_count: count,
            pipeline_statistics: 0,
        };
        let mut pool = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_query_pool)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut pool,
                ),
                "query pool",
            )?;
        }
        Ok(pool)
    }

    /// A frame timestamp for `GENOS_GPU_TIMES`, at the end of everything recorded so far.
    fn frame_stamp(&self, slot: usize, stamp: FrameStamp) {
        let Some(times) = self.gpu_times.as_ref() else {
            return;
        };
        let first = slot as u32 * FRAME_STAMPS;
        unsafe {
            if stamp == FrameStamp::Start {
                (self.fns.cmd_reset_query)(self.cmd, times.pool, first, FRAME_STAMPS);
                (self.fns.cmd_write_timestamp)(self.cmd, 0x1, times.pool, first);
            } else {
                (self.fns.cmd_write_timestamp)(self.cmd, 0x2000, times.pool, first + stamp as u32);
            }
        }
    }

    /// Bloom and display timestamps from the previous frame. Zero when the device
    /// has no timestamp queries or the pair was not written.
    fn collect_hdr_time(&mut self, slot: usize) {
        let base = self.lighting.hdr_query[slot];
        if self.hdr_time_pool.is_null() || base == u32::MAX || self.timestamp_period <= 0.0 {
            self.lighting.hdr_ms = 0.0;
            return;
        }
        let mut raw = [0u64; 2];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.hdr_time_pool,
                base,
                2,
                16,
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1,
            )
        };
        self.lighting.hdr_ms = if got == 0 {
            tick_delta(raw[0], raw[1], self.timestamp_bits) as f64
                * f64::from(self.timestamp_period)
                / 1.0e6
        } else {
            0.0
        };
    }

    /// Read a finished frame's stamps (its fence has been waited) and print the averages
    /// every `every` frames.
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn collect_frame_times(&mut self, slot: usize) {
        let bits = self.timestamp_bits;
        let period = f64::from(self.timestamp_period);
        let antialias = self.antialias;
        // The print below borrows `gpu_times`. Read the diag counters first.
        let lamp_pick_line = if genos_scene::lighting_env_os("GENOS_LIGHTING_LAMP_PICK_DIAG")
            .is_some()
            && self.gpu_times.as_ref().is_some_and(|times| {
                times.print && times.written[slot] && times.frames.saturating_add(1) >= times.every
            }) {
            let c = self.lighting_lamp_pick_counts();
            format!(
                " lamp_px={} lamp_kept={} lamp_fill={} lamp_cache={} lamp_spatial={}",
                c.pixels, c.kept, c.fills, c.cache, c.spatial
            )
        } else {
            String::new()
        };
        let stats_words = if self
            .gpu_times
            .as_ref()
            .is_some_and(|times| times.record && times.written[slot])
        {
            self.read_stats16(self.lighting.stats[slot].memory)
                .unwrap_or([0; LIGHTING_STATS_WORDS])
        } else {
            [0; LIGHTING_STATS_WORDS]
        };
        let Some(times) = self.gpu_times.as_mut() else {
            return;
        };
        if !times.written[slot] {
            times.written[slot] = true;
            return;
        }
        let mut raw = [0u64; FRAME_STAMPS as usize];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                times.pool,
                slot as u32 * FRAME_STAMPS,
                FRAME_STAMPS,
                8 * raw.len(),
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1,
            )
        };
        if got != 0 {
            return;
        }
        if times.print {
            for (k, w) in raw.windows(2).enumerate() {
                times.frame[k] += tick_delta(w[0], w[1], bits) as f64 * period / 1.0e6;
            }
            times
                .frame_spans
                .push((raw[0], raw[FRAME_STAMPS as usize - 1]));
        }
        let ids = std::mem::take(&mut times.lighting_ids[slot]);
        let tags = std::mem::take(&mut self.lighting.frame_tags[slot]);
        let shadow = self.lighting.frame_shadow[slot];
        let prefill = self.lighting.frame_prefill[slot];
        let counts = self.lighting.frame_counts[slot];
        let mut bucket_ms = [0.0f64; BUCKETS];
        if !ids.is_empty() && !times.lighting_pool.is_null() {
            let mut g = vec![0u64; ids.len() + 1];
            let got = unsafe {
                (self.fns.get_query_results)(
                    self.device,
                    times.lighting_pool,
                    slot as u32 * LIGHTING_STAMPS,
                    g.len() as u32,
                    8 * g.len(),
                    g.as_mut_ptr() as *mut c_void,
                    8,
                    0x1,
                )
            };
            if got == 0 {
                if times.print && times.lighting_disp.len() != ids.len() {
                    times.lighting_disp = ids.iter().map(|&p| (p, 0.0)).collect();
                }
                for (i, (&pass, w)) in ids.iter().zip(g.windows(2)).enumerate() {
                    let ms = tick_delta(w[0], w[1], bits) as f64 * period / 1.0e6;
                    if times.print && pass < times.lighting.len() {
                        times.lighting[pass] += ms;
                    }
                    if times.print {
                        times.lighting_disp[i].1 += ms;
                    }
                    let tag = tags.get(i).copied().unwrap_or(pass as u8) as usize;
                    if tag < BUCKETS {
                        bucket_ms[tag] += ms;
                        if times.print {
                            times.lighting_bucket[tag] += ms;
                        }
                    }
                    if shadow && (tag == B_DIRECT as usize || tag == B_HIT as usize) {
                        bucket_ms[B_SHADOWS as usize] += ms;
                        if times.print {
                            times.lighting_bucket[B_SHADOWS as usize] += ms;
                        }
                    }
                }
                if times.print {
                    times.lighting_frames += 1;
                    if prefill {
                        times.prefill_frames += 1;
                    }
                }
                self.lighting.last_ms = bucket_ms;
                self.lighting.last_dispatches = counts;
                if times.record {
                    let world_rays = std::mem::take(&mut times.pending_world_rays);
                    let world_relit = std::mem::take(&mut times.pending_world_relit);
                    times.recorded += 1;
                    let sample = gi_sample_from(
                        times.recorded,
                        &bucket_ms,
                        &stats_words,
                        world_rays,
                        world_relit,
                    );
                    times.samples.push_back(sample);
                    while times.samples.len() > GI_SAMPLE_CAP {
                        times.samples.pop_front();
                    }
                }
            }
        }
        if !times.print {
            return;
        }
        times.frames += 1;
        let since = *times.since.get_or_insert_with(std::time::Instant::now);
        if times.frames < times.every {
            return;
        }
        let n = f64::from(times.frames);
        let seconds = since.elapsed().as_secs_f64().max(1.0e-6);
        let f = times.frame.map(|v| v / n);
        let passes = pass_times(&f);
        eprintln!(
            "GPU_MS frames={} size={}x{} present={} setup={:.3} gbuffer={:.3} aa({:?})={:.3} overlay={:.3} copy={:.3} nonlight={:.3} lighting={:.3} frame={:.3} hdr={:.3} | fps={:.0}",
            times.frames,
            self.extent_w,
            self.extent_h,
            present_name(self.present_mode),
            passes.setup,
            passes.gbuffer,
            antialias,
            passes.aa,
            passes.overlay,
            passes.copy,
            passes.nonlight,
            passes.lighting,
            passes.frame,
            self.lighting.hdr_ms,
            n / seconds,
        );
        if times.lighting_frames > 0 && std::env::var_os("GENOS_GPU_DISPATCH").is_some() {
            let lf = f64::from(times.lighting_frames);
            let parts: Vec<String> = times
                .lighting_disp
                .iter()
                .enumerate()
                .map(|(i, (p, ms))| format!("{i}:{p}={:.3}", ms / lf))
                .collect();
            eprintln!("LIGHTING_DISPATCH {}", parts.join(" "));
            times.lighting_disp.clear();
        }
        if times.lighting_frames > 0 {
            let g = times.lighting.map(|v| v / f64::from(times.lighting_frames));
            eprintln!(
                "LIGHTING_MS frames={} compact={:.3} place={:.3} trace={:.3} light={:.3} gather={:.3} filter={:.3} cache={:.3} compose={:.3} atrous={:.3} total={:.3} async_rounds={} async_ms={:.3} async_compact={:.3} async_trace={:.3} async_light={:.3} async_cache={:.3} cache_rays={:.3} shadows={:.3} prefill={:.3}{}{}",
                times.lighting_frames,
                g[0],
                g[1],
                g[2],
                g[3],
                g[4],
                g[5],
                g[6],
                g[7],
                g[8],
                g.iter().sum::<f64>(),
                times.world_runs,
                times.world_ms / f64::from(times.world_runs.max(1)),
                times.world_pass[0] / f64::from(times.world_runs.max(1)),
                times.world_pass[1] / f64::from(times.world_runs.max(1)),
                times.world_pass[2] / f64::from(times.world_runs.max(1)),
                times.world_pass[3] / f64::from(times.world_runs.max(1)),
                times.lighting_bucket[B_CACHE_RAYS as usize] / f64::from(times.lighting_frames),
                times.lighting_bucket[B_SHADOWS as usize] / f64::from(times.lighting_frames),
                times.prefill_frames as f64 / f64::from(times.lighting_frames),
                self.last_unfilled
                    .map(|stats| format!(" | {}", stats.line()))
                    .unwrap_or_default(),
                lamp_pick_line
            );
        }
        if !times.light_spans.is_empty() {
            let (light, inside) = overlap_ms(&times.light_spans, &times.frame_spans, period);
            eprintln!(
                "GPU_OVERLAP async_light_ms={light:.2} while_a_frame_ran_ms={inside:.2} share={:.2}",
                inside / light.max(1.0e-9)
            );
        }
        if !times.placed {
            let [local, all] = self.upload_local.get();
            eprintln!("GPU_MEM upload buffers in device-local memory: {local} of {all}");
        }
        let pool = times.pool;
        let lighting_pool = times.lighting_pool;
        let print = times.print;
        let record = times.record;
        let samples = std::mem::take(&mut times.samples);
        let recorded = times.recorded;
        let pending_world_rays = times.pending_world_rays;
        let pending_world_relit = times.pending_world_relit;
        **times = GpuTimes {
            pool,
            lighting_pool,
            written: times.written,
            every: times.every,
            placed: true,
            print,
            record,
            samples,
            recorded,
            pending_world_rays,
            pending_world_relit,
            ..GpuTimes::default()
        };
    }

    fn reset_queries(&self, slot: usize) {
        unsafe {
            (self.fns.cmd_reset_query)(self.cmd, self.query_pool, (slot * 2) as u32, 2);
        }
    }

    fn write_stamp(&self, slot: usize, index: u32, stage: u32) {
        unsafe {
            (self.fns.cmd_write_timestamp)(
                self.cmd,
                stage,
                self.query_pool,
                (slot * 2) as u32 + index,
            );
        }
    }

    /// The slot fence has already been waited. Read the timestamp span if this slot still owes one.
    fn collect_slot_after_wait(&mut self, slot: usize) -> Result<(), String> {
        if !self.stamp_pending[slot] {
            return Ok(());
        }
        if self.inflight.front().copied() != Some(slot) {
            return Err("profile slot finished out of order".into());
        }
        let duration = self.read_span(slot)?;
        self.stamp_pending[slot] = false;
        self.inflight.pop_front();
        self.ready.push_back(duration);
        Ok(())
    }

    fn collect_submitted_if_ready(&mut self) -> Result<Option<Duration>, String> {
        let Some(slot) = self.submitted_slot else {
            return Ok(None);
        };
        if self.inflight.front().copied() != Some(slot) || !self.stamp_pending[slot] {
            return Ok(None);
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.fences[slot]) };
        if status == 1 {
            return Ok(None);
        }
        check(status, "timestamp fence")?;
        let duration = self.read_span(slot)?;
        self.stamp_pending[slot] = false;
        self.inflight.pop_front();
        Ok(Some(duration))
    }

    fn collect_ready(&mut self, wait: bool) -> Result<(), String> {
        while let Some(slot) = self.inflight.front().copied() {
            if !self.stamp_pending[slot] {
                self.inflight.pop_front();
                continue;
            }
            let fence = self.fences[slot];
            if wait {
                self.wait_fence(fence, "profile fence")?;
            } else {
                let status = unsafe { (self.fns.fence_status)(self.device, fence) };
                if status == 1 {
                    break;
                }
                check(status, "profile fence")?;
            }
            let duration = self.read_span(slot)?;
            self.stamp_pending[slot] = false;
            self.inflight.pop_front();
            self.ready.push_back(duration);
        }
        Ok(())
    }

    fn read_span(&self, slot: usize) -> Result<Duration, String> {
        let mut raw = [0u64; 2];
        let waited = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.query_pool,
                (slot * 2) as u32,
                2,
                std::mem::size_of::<[u64; 2]>(),
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1 | 0x2,
            )
        };
        check(waited, "timestamp query")?;
        let delta = tick_delta(raw[0], raw[1], self.timestamp_bits);
        if raw[0] == 0 || raw[1] == 0 || delta == 0 {
            return Err("timestamp query returned no device time".into());
        }
        let nanos = delta as f64 * f64::from(self.timestamp_period);
        if !nanos.is_finite() || nanos < 0.0 || nanos > u64::MAX as f64 {
            return Err("timestamp period produced a bad duration".into());
        }
        Ok(Duration::from_nanos(nanos.round() as u64))
    }

    fn idle_device(&self) {
        let wait = self.fns.device_wait;
        let device = self.device;
        block(|| unsafe { wait(device) });
    }

    /// Wait until `fence` signals. A short wait stays on this thread. A longer
    /// one dispatches the seat so a ping is not missed.
    fn wait_fence(&self, fence: Handle, what: &str) -> Result<(), String> {
        if fence.is_null() {
            return Ok(());
        }
        let status = unsafe { (self.fns.fence_status)(self.device, fence) };
        if status == VK_SUCCESS {
            return Ok(());
        }
        let timed = unsafe {
            (self.fns.wait_fences)(self.device, 1, &fence, 1, FENCE_BUDGET_NS)
        };
        if timed == VK_SUCCESS {
            return Ok(());
        }
        if timed != VK_TIMEOUT && timed != VK_NOT_READY {
            return check(timed, what);
        }
        let wait = self.fns.wait_fences;
        let device = self.device;
        let mut result = VK_SUCCESS;
        block(|| {
            result = unsafe { wait(device, 1, &fence, 1, u64::MAX) };
        });
        check(result, what)
    }

    fn wait_flight(&mut self, slot: usize) -> Result<(), String> {
        self.wait_fence(self.fences[slot], "flight wait")
    }

    fn wait_all_inflight(&mut self) -> Result<(), String> {
        for slot in 0..2 {
            self.wait_flight(slot)?;
        }
        Ok(())
    }

    fn read_host(&mut self, slot: usize) -> Result<Vec<u8>, String> {
        self.wait_fence(self.fences[slot], "earlier frame")?;
        let saved = copy_buffer(&self.host);
        self.host = copy_buffer(&self.hosts[slot]);
        let pixels = self.read_color()?;
        self.host = saved;
        Ok(pixels)
    }

    fn wait_gpu(&mut self) -> Result<(), String> {
        self.wait_fence(self.fence, "present wait")?;
        self.collect_froxel_time();
        Ok(())
    }

    fn present(&mut self) -> Result<VkResult, String> {
        unsafe {
            let swap = [self.swapchain];
            let index = [self.present_index];
            let wait = [self.render_done];
            #[repr(C)]
            struct Present {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                swap_count: u32,
                swaps: *const Handle,
                indices: *const u32,
                results: *mut i32,
            }
            let info = Present {
                s_type: 1000001001,
                next: std::ptr::null(),
                wait_count: 1,
                waits: wait.as_ptr(),
                swap_count: 1,
                swaps: swap.as_ptr(),
                indices: index.as_ptr(),
                results: std::ptr::null_mut(),
            };
            let result = (self.fns.present)(self.queue, &info as *const Present as *const u8);
            if result != VK_SUCCESS && result != SUBOPTIMAL && result != OUT_OF_DATE {
                return Err(format!("present failed: {result}"));
            }
            Ok(result)
        }
    }

    /// Copy the last frame's picture into host memory, after that frame's fence.
    ///
    /// Reuses that frame's command buffer and fence; the next frame on this flight
    /// waits for the fence as usual.
    fn copy_last_picture(&mut self) -> Result<(), String> {
        self.wait_gpu()?;
        unsafe {
            let fences = [self.fence];
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset fence",
            )?;
            check((self.fns.reset_cmd)(self.cmd, 0), "reset cmd")?;
            #[repr(C)]
            struct BeginInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                inherit: *const c_void,
            }
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 1,
                inherit: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(self.cmd, &begin as *const BeginInfo as *const u8),
                "begin cmd",
            )?;
            self.copy_color_to_buffer();
            self.copy_scene_for_readback();
            check((self.fns.end_cmd)(self.cmd), "end cmd")?;
            #[repr(C)]
            struct Submit {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                stages: *const u32,
                cmd_count: u32,
                cmds: *const Handle,
                signal_count: u32,
                signals: *const Handle,
            }
            let cmd = [self.cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.fence,
                ),
                "submit copy",
            )?;
        }
        self.wait_gpu()
    }

    fn read_color(&self) -> Result<Vec<u8>, String> {
        let bytes = self.extent_w as usize * self.extent_h as usize * 4;
        let mut out = vec![0u8; bytes];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    self.host.memory,
                    0,
                    self.host.size,
                    0,
                    &mut mapped,
                ),
                "map readback",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u8, out.as_mut_ptr(), bytes);
            (self.fns.unmap_mem)(self.device, self.host.memory);
        }
        Ok(out)
    }

    /// Device scene image to the host buffer. The fence has already waited.
    fn copy_scene_for_readback(&mut self) {
        let src = self.lighting.scene_lin.buffer;
        let dst = self.lighting.scene_host.buffer;
        if src.is_null() || dst.is_null() {
            return;
        }
        let size = self.lighting.scene_lin.size.min(self.lighting.scene_host.size);
        if size == 0 {
            return;
        }
        #[repr(C)]
        struct Region {
            src: u64,
            dst: u64,
            size: u64,
        }
        let region = Region { src: 0, dst: 0, size };
        unsafe {
            (self.fns.cmd_copy_buffer)(
                self.cmd,
                src,
                dst,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn read_scene_host(&self) -> Result<crate::hdr_output::SceneRead, String> {
        let pixels = self.extent_w as usize * self.extent_h as usize;
        let bytes = pixels.saturating_mul(16);
        if bytes == 0
            || self.lighting.scene_host.memory.is_null()
            || self.lighting.scene_host.size < bytes as u64
        {
            return Ok(crate::hdr_output::SceneRead {
                width: 0,
                height: 0,
                rgb: Vec::new(),
                distance_m: Vec::new(),
            });
        }
        let values = read_f32s(
            self.fns.map_mem,
            self.fns.unmap_mem,
            self.device,
            self.lighting.scene_host.memory,
            pixels.saturating_mul(4),
        )?;
        Ok(crate::hdr_output::scene_read_from_rgba(
            self.extent_w,
            self.extent_h,
            &values,
        ))
    }

    fn make_sem(&mut self) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
        }
        let info = Info {
            s_type: 9,
            next: std::ptr::null(),
            flags: 0,
        };
        let mut sem = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_sem)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut sem,
                ),
                "semaphore",
            )?;
        }
        Ok(sem)
    }

    fn make_desc_layout(&self) -> Result<Handle, String> {
        #[repr(C)]
        struct Binding {
            binding: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            count: u32,
            bindings: *const Binding,
        }
        let bindings = [
            Binding {
                binding: 0,
                kind: 7,
                count: 1,
                stages: 0x10 | 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 2,
                kind: 7,
                count: 1,
                stages: 0x10,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 3,
                kind: 7,
                count: 1,
                stages: 0x1,
                samplers: std::ptr::null(),
            },
            // Device-local colour atlas. Fragment only: the depth mask and the G-buffer.
            Binding {
                binding: 4,
                kind: 3,
                count: 1,
                stages: 0x10,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 5,
                kind: 7,
                count: 1,
                stages: 0x10 | 0x20,
                samplers: std::ptr::null(),
            },
            // GI G-buffer (gbuffer.frag writes it).
            Binding {
                binding: 6,
                kind: 7,
                count: 1,
                stages: 0x10,
                samplers: std::ptr::null(),
            },
            // Device-local normal atlas. Same readers as binding 4.
            Binding {
                binding: 7,
                kind: 3,
                count: 1,
                stages: 0x10,
                samplers: std::ptr::null(),
            },
        ];
        let info = Info {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: bindings.len() as u32,
            bindings: bindings.as_ptr(),
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "descriptor layout",
            )?;
        }
        Ok(layout)
    }

    fn make_scene_buffers(&mut self) -> Result<(), String> {
        self.scene_buf = self.make_buffer(pack::SCENE_CAPACITY as u64, 0x20, Memory::Upload)?;
        let share = self.light_families();
        self.instance_bufs = [
            self.make_buffer_queues(5120, 0x20, Memory::Upload, &share)?,
            self.make_buffer_queues(5120, 0x20, Memory::Upload, &share)?,
        ];
        self.particle_buf =
            self.make_buffer_queues(16 + 256 * 256 * 4, 0x20, Memory::Upload, &share)?;
        self.sampled_color = self.make_atlas_image(1, 1)?;
        self.sampled_normal = self.make_atlas_image(1, 1)?;
        self.sampled_size = [(1, 1), (1, 1)];
        self.atlas_readback = self.make_buffer(4, 0x2, Memory::Readback)?;
        self.page_readback = self.make_buffer(4, 0x2, Memory::Readback)?;
        // The CPU never stores a mip here. A skipped copy reads back as zero.
        self.write_buffer(&self.atlas_readback, &[0, 0, 0, 0])?;
        self.write_buffer(&self.page_readback, &[0, 0, 0, 0])?;
        // S70C: one resident scene, written by deltas in the frame's command stream.
        self.frame_scene[0] =
            self.make_buffer(pack::SCENE_CAPACITY as u64, 0x20 | 0x2, Memory::Device)?;
        self.frame_scene[1] = copy_buffer(&self.frame_scene[0]);
        self.scene_shadow.clear();
        self.write_buffer(&self.particle_buf, &vec![0u8; 16])?;
        self.mesh_field_cap = 1 << 16;
        self.mesh_field =
            self.make_buffer_queues(self.mesh_field_cap, 0x20 | 0x2, Memory::Device, &share)?;
        // The empty set, recorded by the first frame.
        self.mesh_field_layout = 0;
        self.mesh_field_shadow = vec![0u8; 16];
        self.mesh_field_dirty = vec![(0, 16)];
        #[repr(C)]
        struct Size {
            kind: u32,
            count: u32,
        }
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            max_sets: u32,
            size_count: u32,
            sizes: *const Size,
        }
        // Two raster sets: five storage buffers plus the two material images.
        let sizes = [Size { kind: 7, count: 16 }, Size { kind: 3, count: 8 }];
        let pool = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 2,
            size_count: sizes.len() as u32,
            sizes: sizes.as_ptr(),
        };
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.desc_pool,
                ),
                "descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [self.desc_layout; 2];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.desc_pool,
                count: 2,
                layouts: layouts.as_ptr(),
            };
            let mut sets = [std::ptr::null_mut(); 2];
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    sets.as_mut_ptr(),
                ),
                "descriptor set",
            )?;
            self.raster_sets = sets;
        }
        self.desc_set = self.raster_sets[0];
        self.write_raster_sets()?;
        self.write_buffer(&self.scene_buf, &vec![0u8; pack::SCENE_TAIL])?;
        self.make_compute_pool()?;
        self.make_audio()?;
        Ok(())
    }

    /// The command pool on the compute family, for the async light cache rounds.
    fn make_compute_pool(&mut self) -> Result<(), String> {
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            family: u32,
        }
        let pool = PoolInfo {
            s_type: 39,
            next: std::ptr::null(),
            flags: 0x2,
            family: self.compute_family,
        };
        unsafe {
            check(
                (self.fns.create_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.compute_pool,
                ),
                "compute command pool",
            )
        }
    }

    /// Queue families that share the lighting's buffers (graphics, and compute when
    /// it is a separate family).
    fn light_families(&self) -> Vec<u32> {
        if self.compute_family == self.queue_family {
            Vec::new()
        } else {
            vec![self.queue_family, self.compute_family]
        }
    }

    /// Point both frames' raster sets at the scene, the particle image, that frame's
    /// instances, the traced meshes, the G-buffer, and the device atlases.
    fn write_raster_sets(&self) -> Result<(), String> {
        for slot in 0..2 {
            self.write_descriptors(
                self.raster_sets[slot],
                &self.frame_scene[slot],
                &self.instance_bufs[slot],
            )?;
        }
        Ok(())
    }

    fn write_descriptors(
        &self,
        set: Handle,
        scene: &Buffer,
        instances: &Buffer,
    ) -> Result<(), String> {
        #[repr(C)]
        struct BufInfo {
            buffer: Handle,
            offset: u64,
            range: u64,
        }
        #[repr(C)]
        struct Write {
            s_type: i32,
            next: *const c_void,
            set: Handle,
            binding: u32,
            element: u32,
            count: u32,
            kind: u32,
            image: *const c_void,
            buffer: *const BufInfo,
            texel: *const c_void,
        }
        let infos = [
            BufInfo {
                buffer: scene.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.particle_buf.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: instances.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.mesh_field.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                // Any buffer until GI sizes its G-buffer; only gbuffer.frag reads it.
                buffer: if self.lighting.gbuf.buffer.is_null() {
                    self.mesh_field.buffer
                } else {
                    self.lighting.gbuf.buffer
                },
                offset: 0,
                range: u64::MAX,
            },
        ];
        #[repr(C)]
        struct ImageInfo {
            sampler: Handle,
            view: Handle,
            layout: u32,
        }
        let color_info = ImageInfo {
            sampler: std::ptr::null_mut(),
            view: self.sampled_color.view,
            layout: 1,
        };
        let normal_info = ImageInfo {
            sampler: std::ptr::null_mut(),
            view: self.sampled_normal.view,
            layout: 1,
        };
        let writes = [
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 0,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[0],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 2,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[1],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 3,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[2],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 5,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[3],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 6,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[4],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 4,
                element: 0,
                count: 1,
                kind: 3,
                image: &color_info as *const ImageInfo as *const c_void,
                buffer: std::ptr::null(),
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 7,
                element: 0,
                count: 1,
                kind: 3,
                image: &normal_info as *const ImageInfo as *const c_void,
                buffer: std::ptr::null(),
                texel: std::ptr::null(),
            },
        ];
        unsafe {
            (self.fns.update_desc)(
                self.device,
                writes.len() as u32,
                writes.as_ptr() as *const u8,
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn make_audio(&mut self) -> Result<(), String> {
        self.audio_rays = self.make_buffer(8192, 0x20, Memory::Upload)?;
        self.audio_gains = self.make_buffer(4096, 0x20, Memory::Readback)?;
        #[repr(C)]
        struct Binding {
            binding: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        #[repr(C)]
        struct SetInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            count: u32,
            bindings: *const Binding,
        }
        let bindings = [
            Binding {
                binding: 0,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 1,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 2,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            },
        ];
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 3,
            bindings: bindings.as_ptr(),
        };
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_desc_layout,
                ),
                "audio descriptor layout",
            )?;
        }
        #[repr(C)]
        struct LayoutInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            set_count: u32,
            sets: *const Handle,
            push_count: u32,
            push: *const c_void,
        }
        let sets = [self.audio_desc_layout];
        let layout = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr(),
            push_count: 0,
            push: std::ptr::null(),
        };
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_layout,
                ),
                "audio pipeline layout",
            )?;
        }
        let module = self.shader(AUDIO_SPV)?;
        #[repr(C)]
        struct Stage {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            stage: u32,
            module: Handle,
            name: *const i8,
            spec: *const c_void,
        }
        #[repr(C)]
        struct Pipe {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            stage: Stage,
            layout: Handle,
            base: Handle,
            base_index: i32,
        }
        let pipe = Pipe {
            s_type: 29,
            next: std::ptr::null(),
            flags: 0,
            stage: Stage {
                s_type: 18,
                next: std::ptr::null(),
                flags: 0,
                stage: 0x20,
                module,
                name: b"main\0".as_ptr() as *const i8,
                spec: std::ptr::null(),
            },
            layout: self.audio_layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        unsafe {
            let result = (self.fns.create_compute)(
                self.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut self.audio_pipe,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "audio pipeline")?;
        }
        #[repr(C)]
        struct Size {
            kind: u32,
            count: u32,
        }
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            max_sets: u32,
            size_count: u32,
            sizes: *const Size,
        }
        let size = Size { kind: 7, count: 3 };
        let pool = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            size_count: 1,
            sizes: &size,
        };
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_desc_pool,
                ),
                "audio descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.audio_desc_pool,
                count: 1,
                layouts: sets.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    &mut self.audio_set,
                ),
                "audio descriptor set",
            )?;
            #[repr(C)]
            struct CmdAlloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                level: u32,
                count: u32,
            }
            let cmd_alloc = CmdAlloc {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.pool,
                level: 0,
                count: 1,
            };
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &cmd_alloc as *const CmdAlloc as *const u8,
                    &mut self.audio_cmd,
                ),
                "audio command buffer",
            )?;
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_fence,
                ),
                "audio fence",
            )?;
        }
        self.write_audio_descriptors()
    }

    fn write_audio_descriptors(&self) -> Result<(), String> {
        #[repr(C)]
        struct BufInfo {
            buffer: Handle,
            offset: u64,
            range: u64,
        }
        #[repr(C)]
        struct Write {
            s_type: i32,
            next: *const c_void,
            set: Handle,
            binding: u32,
            element: u32,
            count: u32,
            kind: u32,
            image: *const c_void,
            buffer: *const BufInfo,
            texel: *const c_void,
        }
        let infos = [
            BufInfo {
                buffer: self.scene_buf.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.audio_rays.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.audio_gains.buffer,
                offset: 0,
                range: u64::MAX,
            },
        ];
        let writes = [
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: self.audio_set,
                binding: 0,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[0],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: self.audio_set,
                binding: 1,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[1],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: self.audio_set,
                binding: 2,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[2],
                texel: std::ptr::null(),
            },
        ];
        unsafe {
            (self.fns.update_desc)(
                self.device,
                3,
                writes.as_ptr() as *const u8,
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn transmission_gains(
        &mut self,
        listener: [f32; 3],
        sources: &[[f32; 3]],
    ) -> Result<Vec<f32>, String> {
        if sources.len() > 256 {
            return Err("too many audio sources".into());
        }
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        self.sync_audio_scene()?;
        self.write_buffer(&self.audio_rays, &ray_bytes(listener, sources))?;
        self.write_buffer(&self.audio_gains, &vec![0u8; sources.len() * 4])?;
        self.wait_fence(self.audio_fence, "audio wait")?;
        let fences = [self.audio_fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "audio reset fence",
            )?;
            check((self.fns.reset_cmd)(self.audio_cmd, 0), "audio reset")?;
            #[repr(C)]
            struct BeginInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                inherit: *const c_void,
            }
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 1,
                inherit: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(self.audio_cmd, &begin as *const BeginInfo as *const u8),
                "audio begin",
            )?;
            #[repr(C)]
            struct MemBar {
                s_type: i32,
                next: *const c_void,
                src_access: u32,
                dst_access: u32,
            }
            let host = MemBar {
                s_type: 46,
                next: std::ptr::null(),
                src_access: 0x4000,
                dst_access: 0x20,
            };
            (self.fns.cmd_barrier)(
                self.audio_cmd,
                0x4000,
                0x800,
                0,
                1,
                &host as *const MemBar as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_bind_pipe)(self.audio_cmd, 1, self.audio_pipe);
            (self.fns.cmd_bind_set)(
                self.audio_cmd,
                1,
                self.audio_layout,
                0,
                1,
                &self.audio_set,
                0,
                std::ptr::null(),
            );
            let groups = (sources.len() as u32).div_ceil(64);
            (self.fns.cmd_dispatch)(self.audio_cmd, groups, 1, 1);
            let back = MemBar {
                s_type: 46,
                next: std::ptr::null(),
                src_access: 0x40,
                dst_access: 0x10,
            };
            (self.fns.cmd_barrier)(
                self.audio_cmd,
                0x800,
                0x4000,
                0,
                1,
                &back as *const MemBar as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
            check((self.fns.end_cmd)(self.audio_cmd), "audio end")?;
            #[repr(C)]
            struct Submit {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                stages: *const u32,
                cmd_count: u32,
                cmds: *const Handle,
                signal_count: u32,
                signals: *const Handle,
            }
            let cmd = [self.audio_cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.audio_fence,
                ),
                "audio submit",
            )?;
            self.wait_fence(self.audio_fence, "audio done")?;
        }
        let memory = self.audio_gains.memory;
        let device = self.device;
        let map = self.fns.map_mem;
        let unmap = self.fns.unmap_mem;
        read_f32s(map, unmap, device, memory, sources.len())
    }

    /// Write the traced mesh instances when they changed (`key`). Waits for the GPU
    /// first: every pass in flight reads the buffer.
    /// Bring the resident mesh field up to `live`. Same layout (only poses, albedo or
    /// emission changed): the changed records are marked for the next frame, no wait.
    /// A new layout repacks everything; only growing the buffer waits for the device.
    fn sync_fields(
        &mut self,
        key: u64,
        live: &[&crate::mesh_field::FieldInstance],
    ) -> Result<(), String> {
        if key == self.mesh_field_key {
            return Ok(());
        }
        let quick = crate::mesh_field::field_parts(live, false);
        let head = crate::mesh_field::words_bytes(&quick.head);
        let tail_at = (quick.head.len() + quick.data_len) * 4;
        let tail = crate::mesh_field::words_bytes(&quick.emission);
        if quick.layout == self.mesh_field_layout
            && self.mesh_field_shadow.len() == tail_at + tail.len()
        {
            self.mark_field(0, &head);
            self.mark_field(tail_at, &tail);
            self.mesh_field_key = key;
            return Ok(());
        }
        let bytes = crate::mesh_field::words_bytes(&crate::mesh_field::field_words(live));
        if bytes.len() as u64 > self.mesh_field_cap {
            // The old buffer may be in use by frames in flight.
            self.idle_device();
            let share = self.light_families();
            let cap = (bytes.len() as u64).next_power_of_two();
            let mut old = std::mem::replace(&mut self.mesh_field, Buffer::empty());
            self.destroy_buffer(&mut old);
            self.mesh_field = self.make_buffer_queues(cap, 0x20 | 0x2, Memory::Device, &share)?;
            self.mesh_field_cap = cap;
            self.write_raster_sets()?;
            if self.lighting.ready {
                self.write_lighting_sets()?;
            }
            self.mesh_field_shadow.clear();
        }
        self.mark_field(0, &bytes);
        self.mesh_field_shadow.truncate(bytes.len());
        self.mesh_field_layout = quick.layout;
        self.mesh_field_key = key;
        Ok(())
    }

    /// Put `bytes` at `at` in the field's shadow and mark the 128-byte records that
    /// changed for the next frame.
    fn mark_field(&mut self, at: usize, bytes: &[u8]) {
        const BLOCK: usize = 128;
        let end = at + bytes.len();
        if self.mesh_field_shadow.len() < end {
            self.mesh_field_shadow.resize(end, 0);
            self.mesh_field_dirty.push((at, end));
            self.mesh_field_shadow[at..end].copy_from_slice(bytes);
            return;
        }
        let mut b = at;
        while b < end {
            let e = (b + BLOCK).min(end);
            if self.mesh_field_shadow[b..e] != bytes[b - at..e - at] {
                self.mesh_field_shadow[b..e].copy_from_slice(&bytes[b - at..e - at]);
                match self.mesh_field_dirty.last_mut() {
                    Some((_, stop)) if *stop == b => *stop = e,
                    _ => self.mesh_field_dirty.push((b, e)),
                }
            }
            b = e;
        }
    }

    /// Record the field's marked ranges into this frame (before any pass reads it).
    fn update_resident_fields(&mut self) {
        const RUN: usize = 65536;
        if self.mesh_field_dirty.is_empty() || self.mesh_field.buffer.is_null() {
            return;
        }
        if self.lighting.async_on {
            // The async light cache reads the field too: let its round finish, and
            // hold the next one until this frame is done.
            self.wait_lighting_world();
            self.lighting.world_hold = self.lighting.world_hold.max(2);
        }
        let mut ranges = std::mem::take(&mut self.mesh_field_dirty);
        ranges.sort_unstable();
        self.memory_barrier(0x10000, 0x1000, 0x20 | 0x8, 0x1000);
        let mut sent = 0usize;
        for &(start, stop) in &ranges {
            let stop = stop.min(self.mesh_field_shadow.len());
            let mut at = start & !3;
            while at < stop {
                let end = (at + RUN)
                    .min(stop + 3 & !3)
                    .min(self.mesh_field_shadow.len());
                if end <= at {
                    break;
                }
                unsafe {
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.mesh_field.buffer,
                        at as u64,
                        (end - at) as u64,
                        self.mesh_field_shadow[at..end].as_ptr() as *const c_void,
                    );
                }
                sent += end - at;
                at = end;
            }
        }
        self.memory_barrier(0x1000, 0x10000, 0x1000, 0x20 | 0x8);
        if crate::scene_delta_trace() {
            eprintln!(
                "field delta runs {} bytes {} of {}",
                ranges.len(),
                sent,
                self.mesh_field_shadow.len()
            );
        }
    }

    fn upload_scene(&mut self, pack: &Pack) -> Result<(), String> {
        let bytes = pack::scene_bytes(pack);
        self.frame_bytes.clone_from(&bytes);
        self.lighting.light_scene = lighting_scene_key(pack, &bytes);
        self.lighting.trace_key = lighting_trace_key(pack);
        self.note_lighting_sun(pack);
        self.background = pack.sky.map(crate::mesh::tone);
        Ok(())
    }

    /// Bring audio's copy of the occluders up to the latest scene (only when audio
    /// asks: the frame itself reads the resident scene).
    fn sync_audio_scene(&self) -> Result<(), String> {
        if self.frame_bytes.is_empty() {
            return Ok(());
        }
        let len = self.frame_bytes.len().min(self.scene_buf.size as usize);
        self.write_buffer(&self.scene_buf, &self.frame_bytes[..len])
    }

    fn upload_image(&self, bytes: &[u8]) -> Result<(), String> {
        self.write_buffer(&self.particle_buf, bytes)
    }

    /// S70C: bring the resident scene up to this frame's bytes, recording only the
    /// 256-byte blocks that changed (runs up to vkCmdUpdateBuffer's 64 KiB).
    fn update_resident_scene(&mut self) {
        const BLOCK: usize = 256;
        const RUN: usize = 65536;
        let bytes = std::mem::take(&mut self.frame_bytes);
        if bytes.is_empty() || self.frame_scene[0].buffer.is_null() {
            self.frame_bytes = bytes;
            return;
        }
        let len = bytes.len().min(self.frame_scene[0].size as usize) & !3;
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut at = 0;
        while at < len {
            let end = (at + BLOCK).min(len);
            let same =
                self.scene_shadow.len() >= end && self.scene_shadow[at..end] == bytes[at..end];
            if !same {
                match runs.last_mut() {
                    Some((start, stop)) if *stop == at && end - *start <= RUN => *stop = end,
                    _ => runs.push((at, end)),
                }
            }
            at = end;
        }
        if !runs.is_empty() {
            // The last frame's reads are done before the writes (in queue order).
            self.memory_barrier(0x10000, 0x1000, 0x20 | 0x8, 0x1000);
            for &(start, stop) in &runs {
                unsafe {
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.frame_scene[0].buffer,
                        start as u64,
                        (stop - start) as u64,
                        bytes[start..stop].as_ptr() as *const c_void,
                    );
                }
            }
            self.memory_barrier(0x1000, 0x10000, 0x1000, 0x20 | 0x8);
            if self.scene_shadow.len() < len {
                self.scene_shadow.resize(len, 0);
            }
            for &(start, stop) in &runs {
                self.scene_shadow[start..stop].copy_from_slice(&bytes[start..stop]);
            }
            if crate::scene_delta_trace() {
                let sent: usize = runs.iter().map(|(a, b)| b - a).sum();
                eprintln!("scene delta runs {} bytes {} of {}", runs.len(), sent, len);
            }
        }
        self.frame_bytes = bytes;
    }

    fn sync_materials(&mut self, world: &World) -> Result<(), String> {
        if self.materials_current(world) {
            return Ok(());
        }
        self.idle_device();
        self.drop_stream_resources();
        self.streamer.clear();
        self.bind_atlas(true, &world.color_atlas, true)?;
        self.bind_atlas(false, &world.normal_atlas, true)?;
        self.stream_keys.clear();
        for streamed in &world.stream_textures {
            self.bind_atlas(true, &streamed.atlas, false)?;
            self.stream_keys.push(streamed.atlas.key);
        }
        self.material_key = world.color_atlas.key;
        self.normal_key = world.normal_atlas.key;
        self.write_raster_sets()
    }

    fn materials_current(&self, world: &World) -> bool {
        self.material_key == world.color_atlas.key
            && self.normal_key == world.normal_atlas.key
            && self.stream_keys.len() == world.stream_textures.len()
            && self
                .stream_keys
                .iter()
                .zip(&world.stream_textures)
                .all(|(key, streamed)| *key == streamed.atlas.key)
    }

    /// Register the atlas mip chain. The device image starts empty; the transfer fills it.
    /// `sampled` is the colour or normal image the fragment shader loads. Extra
    /// chains still get device mips; they do not replace that image.
    fn bind_atlas(
        &mut self,
        color: bool,
        atlas: &crate::world::MaterialAtlas,
        sampled: bool,
    ) -> Result<(), String> {
        let Some(chain) = crate::material_stream::mip_chain(atlas.width, atlas.height, &atlas.rgba)
        else {
            return Ok(());
        };
        if sampled {
            self.ensure_sampled(color, chain.width, chain.height)?;
        }
        let started = Instant::now();
        let bytes = chain.level_bytes();
        let total = chain.total_bytes();
        let index = self.streamer.texture_count();
        self.streamer
            .add_texture(chain.width, chain.height, &bytes);
        let elapsed = started.elapsed().as_millis().max(1);
        let ms = u32::try_from(elapsed).unwrap_or(u32::MAX);
        self.streamer.observe_disk(total, ms);
        self.streamer.observe_decode(total, ms);
        let (staging, mip_offset) = self.stage_chain(&chain)?;
        self.stream_sources.push(StreamSource {
            index,
            color,
            chain,
            staging,
            mip_offset,
            shown: None,
        });
        Ok(())
    }

    fn ensure_sampled(&mut self, color: bool, width: u32, height: u32) -> Result<(), String> {
        let slot = usize::from(!color);
        if self.sampled_size[slot] == (width, height) && !self.sampled_image(color).image.is_null()
        {
            return Ok(());
        }
        let image = self.make_atlas_image(width, height)?;
        let mut old = if color {
            std::mem::replace(&mut self.sampled_color, image)
        } else {
            std::mem::replace(&mut self.sampled_normal, image)
        };
        self.destroy_image(&mut old);
        self.sampled_size[slot] = (width, height);
        self.sampled_layout[slot] = 0;
        self.sampled_cleared[slot] = false;
        Ok(())
    }

    fn sampled_image(&self, color: bool) -> &Image {
        if color {
            &self.sampled_color
        } else {
            &self.sampled_normal
        }
    }

    fn stage_chain(
        &self,
        chain: &crate::material_stream::Chain,
    ) -> Result<(Buffer, Vec<u64>), String> {
        let mut packed = Vec::new();
        let mut mip_offset = Vec::new();
        for level in &chain.levels {
            mip_offset.push(packed.len() as u64);
            packed.extend_from_slice(&level.rgba);
        }
        if packed.is_empty() {
            packed.resize(4, 0);
        }
        // TRANSFER_SRC for the page copy, TRANSFER_DST so the submit can wipe it
        // before the atlas blit. The wipe is what makes a staging-to-atlas copy black.
        let staging = self.make_buffer(packed.len() as u64, 0x1 | 0x2, Memory::Upload)?;
        self.write_buffer(&staging, &packed)?;
        Ok((staging, mip_offset))
    }

    fn atlas_readback_texel(&self) -> u8 {
        self.readback_texel(&self.atlas_readback)
    }

    fn page_readback_texel(&self) -> u8 {
        self.readback_texel(&self.page_readback)
    }

    fn readback_texel(&self, buffer: &Buffer) -> u8 {
        if buffer.memory.is_null() || buffer.size < 4 {
            return 0;
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(self.device, buffer.memory, 0, 4, 0, &mut mapped) != 0
                || mapped.is_null()
            {
                return 0;
            }
            let byte = *mapped.cast::<u8>();
            (self.fns.unmap_mem)(self.device, buffer.memory);
            byte
        }
    }

    fn write_memory(&self, memory: Handle, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    memory,
                    0,
                    bytes.len() as u64,
                    0,
                    &mut mapped,
                ),
                "map gpu buffer",
            )?;
            copy_to_mapped(mapped as *mut u8, bytes.as_ptr(), bytes.len());
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn write_buffer(&self, buffer: &Buffer, bytes: &[u8]) -> Result<(), String> {
        self.write_buffer_at(buffer, 0, bytes)
    }

    fn write_buffer_at(&self, buffer: &Buffer, offset: u64, bytes: &[u8]) -> Result<(), String> {
        if offset.saturating_add(bytes.len() as u64) > buffer.size {
            return Err("gpu buffer is too small".into());
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    buffer.memory,
                    offset,
                    bytes.len() as u64,
                    0,
                    &mut mapped,
                ),
                "map gpu buffer",
            )?;
            copy_to_mapped(mapped as *mut u8, bytes.as_ptr(), bytes.len());
            (self.fns.unmap_mem)(self.device, buffer.memory);
        }
        Ok(())
    }

    fn make_layout(&mut self) -> Result<Handle, String> {
        #[repr(C)]
        struct Range {
            stage: u32,
            offset: u32,
            size: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            set_count: u32,
            sets: *const c_void,
            push_count: u32,
            push: *const Range,
        }
        let range = Range {
            stage: 1,
            offset: 0,
            size: 64,
        };
        let sets = [self.desc_layout];
        let info = Info {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr() as *const c_void,
            push_count: 1,
            push: &range,
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "layout",
            )?;
        }
        Ok(layout)
    }

    fn make_render_pass(&mut self, keep: bool) -> Result<Handle, String> {
        #[repr(C)]
        struct Att {
            flags: u32,
            format: i32,
            samples: u32,
            load: u32,
            store: u32,
            stencil_load: u32,
            stencil_store: u32,
            initial: u32,
            final_layout: u32,
        }
        #[repr(C)]
        struct Ref {
            attachment: u32,
            layout: u32,
        }
        #[repr(C)]
        struct Subpass {
            flags: u32,
            pipeline: u32,
            input_count: u32,
            input: *const Ref,
            color_count: u32,
            color: *const Ref,
            resolve: *const Ref,
            depth: *const Ref,
            preserve_count: u32,
            preserve: *const u32,
        }
        #[repr(C)]
        struct Dep {
            src: u32,
            dst: u32,
            src_stage: u32,
            dst_stage: u32,
            src_access: u32,
            dst_access: u32,
            flags: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            att_count: u32,
            atts: *const Att,
            sub_count: u32,
            subs: *const Subpass,
            dep_count: u32,
            deps: *const Dep,
        }
        let color_load = if keep { 0 } else { 1 };
        let color_initial = if keep { 6 } else { 0 };
        let depth_load = if keep { 2 } else { 1 };
        // Final layout cannot be undefined. The overlay pass discards depth.
        let depth_final = 3;
        let in_stage = if keep { 0x1000 } else { 0x400 };
        // The filter leaves the color image visible to a transfer read.
        let in_access = if keep { 0x800 } else { 0 };
        let in_dst = if keep { 0x180 } else { 0x100 };
        let atts = [
            Att {
                flags: 0,
                format: self.format,
                samples: 1,
                load: color_load,
                store: 0,
                stencil_load: 1,
                stencil_store: 1,
                initial: color_initial,
                final_layout: 6,
            },
            Att {
                flags: 0,
                format: 126,
                samples: 1,
                load: depth_load,
                store: 1,
                stencil_load: 1,
                stencil_store: 1,
                initial: 0,
                final_layout: depth_final,
            },
        ];
        let color = Ref {
            attachment: 0,
            layout: 2,
        };
        let depth = Ref {
            attachment: 1,
            layout: 3,
        };
        let sub = Subpass {
            flags: 0,
            pipeline: 0,
            input_count: 0,
            input: std::ptr::null(),
            color_count: 1,
            color: &color,
            resolve: std::ptr::null(),
            depth: &depth,
            preserve_count: 0,
            preserve: std::ptr::null(),
        };
        let deps = [
            Dep {
                src: u32::MAX,
                dst: 0,
                src_stage: in_stage,
                dst_stage: 0x400,
                src_access: in_access,
                dst_access: in_dst,
                flags: 0,
            },
            Dep {
                src: 0,
                dst: u32::MAX,
                src_stage: 0x400,
                dst_stage: 0x1000,
                src_access: 0x100,
                dst_access: 0x800,
                flags: 0,
            },
        ];
        let info = Info {
            s_type: 38,
            next: std::ptr::null(),
            flags: 0,
            att_count: 2,
            atts: atts.as_ptr(),
            sub_count: 1,
            subs: &sub,
            dep_count: 2,
            deps: deps.as_ptr(),
        };
        let mut pass = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_render_pass)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut pass,
                ),
                "render pass",
            )?;
        }
        Ok(pass)
    }

    fn make_pipeline(
        &mut self,
        depth_test: bool,
        blend_on: bool,
        additive: bool,
    ) -> Result<Handle, String> {
        self.make_raster_pipeline(depth_test, blend_on, additive, false)
    }

    /// The depth of the opaque shapes alone: no fragment shader, no color. The scene
    /// pass after it tests equal-or-nearer, so each pixel shades one opaque surface
    /// however the shapes overlap or are ordered.
    fn make_depth_pipeline(&mut self) -> Result<Handle, String> {
        self.make_raster_pipeline(true, false, false, true)
    }

    fn make_raster_pipeline(
        &mut self,
        depth_test: bool,
        blend_on: bool,
        additive: bool,
        depth_only: bool,
    ) -> Result<Handle, String> {
        unsafe {
            let vert = self.shader(if additive {
                WIRE_VERT_SPV
            } else {
                self.vert_override.unwrap_or(VERT_SPV)
            })?;
            let frag = self.shader(if depth_only {
                DEPTH_MASK_SPV
            } else if additive {
                WIRE_FRAG_SPV
            } else {
                self.frag_override.unwrap_or(FRAG_SPV)
            })?;
            #[repr(C)]
            struct Stage {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                stage: u32,
                module: Handle,
                name: *const i8,
                spec: *const c_void,
            }
            let entry = b"main\0".as_ptr() as *const i8;
            let stages = [
                Stage {
                    s_type: 18,
                    next: std::ptr::null(),
                    flags: 0,
                    stage: 1,
                    module: vert,
                    name: entry,
                    spec: std::ptr::null(),
                },
                Stage {
                    s_type: 18,
                    next: std::ptr::null(),
                    flags: 0,
                    stage: 16,
                    module: frag,
                    name: entry,
                    spec: std::ptr::null(),
                },
            ];
            #[repr(C)]
            struct Bind {
                binding: u32,
                stride: u32,
                rate: u32,
            }
            #[repr(C)]
            struct Attr {
                location: u32,
                binding: u32,
                format: i32,
                offset: u32,
            }
            #[repr(C)]
            struct VertInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                bind_count: u32,
                binds: *const Bind,
                attr_count: u32,
                attrs: *const Attr,
            }
            let bind = Bind {
                binding: 0,
                stride: 48,
                rate: 0,
            };
            let attrs = [
                Attr {
                    location: 0,
                    binding: 0,
                    format: 106,
                    offset: 0,
                },
                Attr {
                    location: 1,
                    binding: 0,
                    format: 106,
                    offset: 12,
                },
                Attr {
                    location: 2,
                    binding: 0,
                    format: 106,
                    offset: 24,
                },
                Attr {
                    location: 3,
                    binding: 0,
                    format: 100,
                    offset: 36,
                },
                Attr {
                    location: 4,
                    binding: 0,
                    format: 103,
                    offset: 40,
                },
            ];
            let vert_info = VertInfo {
                s_type: 19,
                next: std::ptr::null(),
                flags: 0,
                bind_count: 1,
                binds: &bind,
                attr_count: 5,
                attrs: attrs.as_ptr(),
            };
            #[repr(C)]
            struct Ia {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                topo: u32,
                restart: u32,
            }
            let ia = Ia {
                s_type: 20,
                next: std::ptr::null(),
                flags: 0,
                topo: 3,
                restart: 0,
            };
            #[repr(C)]
            struct ViewportState {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                vp_count: u32,
                vps: *const c_void,
                sc_count: u32,
                scs: *const c_void,
            }
            let vp = ViewportState {
                s_type: 22,
                next: std::ptr::null(),
                flags: 0,
                vp_count: 1,
                vps: std::ptr::null(),
                sc_count: 1,
                scs: std::ptr::null(),
            };
            #[repr(C)]
            struct Raster {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                depth_clamp: u32,
                discard: u32,
                poly: u32,
                cull: u32,
                front: u32,
                depth_bias: u32,
                bias_const: f32,
                bias_clamp: f32,
                bias_slope: f32,
                line: f32,
            }
            let raster = Raster {
                s_type: 23,
                next: std::ptr::null(),
                flags: 0,
                depth_clamp: 0,
                discard: 0,
                poly: 0,
                cull: 0,
                front: 0,
                depth_bias: 0,
                bias_const: 0.0,
                bias_clamp: 0.0,
                bias_slope: 0.0,
                line: 1.0,
            };
            #[repr(C)]
            struct Ms {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                samples: u32,
                sample_shading: u32,
                min_sample: f32,
                mask: *const u32,
                alpha_to_cov: u32,
                alpha_to_one: u32,
            }
            let ms = Ms {
                s_type: 24,
                next: std::ptr::null(),
                flags: 0,
                samples: 1,
                sample_shading: 0,
                min_sample: 0.0,
                mask: std::ptr::null(),
                alpha_to_cov: 0,
                alpha_to_one: 0,
            };
            #[repr(C)]
            struct Depth {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                test: u32,
                write: u32,
                compare: u32,
                bounds: u32,
                stencil: u32,
                front_fail: u32,
                front_pass: u32,
                front_depth_fail: u32,
                front_compare: u32,
                front_cmp_mask: u32,
                front_write_mask: u32,
                front_ref: u32,
                back_fail: u32,
                back_pass: u32,
                back_depth_fail: u32,
                back_compare: u32,
                back_cmp_mask: u32,
                back_write_mask: u32,
                back_ref: u32,
                min: f32,
                max: f32,
            }
            let depth_on = if depth_test { 1 } else { 0 };
            let depth = Depth {
                s_type: 25,
                next: std::ptr::null(),
                flags: 0,
                test: depth_on,
                write: depth_on,
                // LESS for the depth pass, LESS_OR_EQUAL after it: the same shape
                // (scene.vert's invariant position) passes at the depth it wrote.
                compare: if depth_only { 1 } else { 3 },
                bounds: 0,
                stencil: 0,
                front_fail: 0,
                front_pass: 0,
                front_depth_fail: 0,
                front_compare: 0,
                front_cmp_mask: 0,
                front_write_mask: 0,
                front_ref: 0,
                back_fail: 0,
                back_pass: 0,
                back_depth_fail: 0,
                back_compare: 0,
                back_cmp_mask: 0,
                back_write_mask: 0,
                back_ref: 0,
                min: 0.0,
                max: 1.0,
            };
            #[repr(C)]
            struct BlendAtt {
                enable: u32,
                src: u32,
                dst: u32,
                op: u32,
                src_a: u32,
                dst_a: u32,
                op_a: u32,
                mask: u32,
            }
            #[repr(C)]
            struct Blend {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                logic: u32,
                logic_op: u32,
                att_count: u32,
                atts: *const BlendAtt,
                blend: [f32; 4],
            }
            let batt = if additive {
                // Additive. A second copy of the same edge stays brighter than one copy.
                BlendAtt {
                    enable: 1,
                    src: 1,
                    dst: 1,
                    op: 0,
                    src_a: 1,
                    dst_a: 1,
                    op_a: 0,
                    mask: 0xf,
                }
            } else if blend_on {
                // Premultiplied. Opaque output uses alpha 1, so it replaces the target.
                // Fog output uses alpha = 1 - transmittance and rgb = in-scatter.
                BlendAtt {
                    enable: 1,
                    src: 1,
                    dst: 7,
                    op: 0,
                    src_a: 1,
                    dst_a: 7,
                    op_a: 0,
                    mask: 0xf,
                }
            } else {
                BlendAtt {
                    enable: 0,
                    src: 1,
                    dst: 0,
                    op: 0,
                    src_a: 1,
                    dst_a: 0,
                    op_a: 0,
                    mask: if depth_only { 0 } else { 0xf },
                }
            };
            let blend = Blend {
                s_type: 26,
                next: std::ptr::null(),
                flags: 0,
                logic: 0,
                logic_op: 0,
                att_count: 1,
                atts: &batt,
                blend: [0.0; 4],
            };
            #[repr(C)]
            struct Dyn {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                count: u32,
                states: *const u32,
            }
            let states = [0u32, 1];
            let dyn_state = Dyn {
                s_type: 27,
                next: std::ptr::null(),
                flags: 0,
                count: 2,
                states: states.as_ptr(),
            };
            #[repr(C)]
            struct Pipe {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                stage_count: u32,
                stages: *const Stage,
                vertex: *const VertInfo,
                ia: *const Ia,
                tess: *const c_void,
                viewport: *const ViewportState,
                raster: *const Raster,
                ms: *const Ms,
                depth: *const Depth,
                blend: *const Blend,
                dynamic: *const Dyn,
                layout: Handle,
                pass: Handle,
                subpass: u32,
                base: Handle,
                base_index: i32,
            }
            let pipe = Pipe {
                s_type: 28,
                next: std::ptr::null(),
                flags: 0,
                // The depth pass samples masked materials, so it runs a fragment
                // shader. Colour writes stay masked off.
                stage_count: 2,
                stages: stages.as_ptr(),
                vertex: &vert_info,
                ia: &ia,
                tess: std::ptr::null(),
                viewport: &vp,
                raster: &raster,
                ms: &ms,
                depth: &depth,
                blend: &blend,
                dynamic: &dyn_state,
                layout: self.layout,
                pass: self.render_pass,
                subpass: 0,
                base: std::ptr::null_mut(),
                base_index: -1,
            };
            let mut pipeline = std::ptr::null_mut();
            let result = (self.fns.create_pipelines)(
                self.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut pipeline,
            );
            (self.fns.destroy_shader)(self.device, vert, std::ptr::null());
            (self.fns.destroy_shader)(self.device, frag, std::ptr::null());
            check(result, "pipeline")?;
            Ok(pipeline)
        }
    }

    fn shader(&self, code: &[u8]) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: usize,
            code: *const u32,
        }
        let info = Info {
            s_type: 16,
            next: std::ptr::null(),
            flags: 0,
            size: code.len(),
            code: code.as_ptr() as *const u32,
        };
        let mut module = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_shader)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut module,
                ),
                "shader",
            )?;
        }
        Ok(module)
    }

    fn make_image(
        &self,
        format: i32,
        usage: u32,
        aspect: u32,
        width: u32,
        height: u32,
    ) -> Result<Image, String> {
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            image_type: u32,
            format: i32,
            extent: Extent,
            mips: u32,
            layers: u32,
            samples: u32,
            tiling: u32,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
            initial: u32,
        }
        let info = Info {
            s_type: 14,
            next: std::ptr::null(),
            flags: 0,
            image_type: 1,
            format,
            extent: Extent {
                w: width,
                h: height,
                d: 1,
            },
            mips: 1,
            layers: 1,
            samples: 1,
            tiling: 0,
            usage,
            sharing: 0,
            queue_count: 0,
            queues: std::ptr::null(),
            initial: 0,
        };
        let mut image = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_image)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut image,
                ),
                "image",
            )?;
            let mut req = [0u8; 24];
            (self.fns.image_reqs)(self.device, image, req.as_mut_ptr());
            let size = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            let memory = self.alloc(size, bits, Memory::Device)?;
            check(
                (self.fns.bind_image)(self.device, image, memory, 0),
                "bind image",
            )?;
            let view = self.make_view(image, format, aspect)?;
            Ok(Image {
                image,
                view,
                memory,
            })
        }
    }

    /// Device-local atlas the fragment shader loads. Transfer is the only writer.
    fn make_atlas_image(&self, width: u32, height: u32) -> Result<Image, String> {
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            image_type: u32,
            format: i32,
            extent: Extent,
            mips: u32,
            layers: u32,
            samples: u32,
            tiling: u32,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
            initial: u32,
        }
        let families = [self.queue_family, self.transfer_family];
        let shared = self.queue_family != self.transfer_family;
        let info = Info {
            s_type: 14,
            next: std::ptr::null(),
            flags: 0,
            image_type: 1,
            format: 37,
            extent: Extent {
                w: width.max(1),
                h: height.max(1),
                d: 1,
            },
            mips: 1,
            layers: 1,
            samples: 1,
            tiling: 0,
            usage: 0x1 | 0x2 | 0x8,
            sharing: u32::from(shared),
            queue_count: if shared { 2 } else { 0 },
            queues: if shared {
                families.as_ptr()
            } else {
                std::ptr::null()
            },
            initial: 0,
        };
        let mut image = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_image)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut image,
                ),
                "atlas image",
            )?;
            let mut req = [0u8; 24];
            (self.fns.image_reqs)(self.device, image, req.as_mut_ptr());
            let size = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            let memory = self.alloc(size, bits, Memory::Device)?;
            check(
                (self.fns.bind_image)(self.device, image, memory, 0),
                "bind atlas",
            )?;
            let view = self.make_view(image, 37, 1)?;
            Ok(Image {
                image,
                view,
                memory,
            })
        }
    }

    fn make_view(&self, image: Handle, format: i32, aspect: u32) -> Result<Handle, String> {
        #[repr(C)]
        struct Comp {
            r: u32,
            g: u32,
            b: u32,
            a: u32,
        }
        #[repr(C)]
        struct Range {
            aspect: u32,
            mip: u32,
            levels: u32,
            layer: u32,
            layers: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            image: Handle,
            view_type: u32,
            format: i32,
            comp: Comp,
            range: Range,
        }
        let info = Info {
            s_type: 15,
            next: std::ptr::null(),
            flags: 0,
            image,
            view_type: 1,
            format,
            comp: Comp {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            range: Range {
                aspect,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        let mut view = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_image_view)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut view,
                ),
                "image view",
            )?;
        }
        Ok(view)
    }

    fn make_framebuffer(&self) -> Result<Handle, String> {
        let views = [self.color.view, self.depth.view];
        self.make_framebuffer_for(self.render_pass, &views, self.extent_w, self.extent_h)
    }

    fn make_framebuffer_for(
        &self,
        pass: Handle,
        views: &[Handle],
        width: u32,
        height: u32,
    ) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            pass: Handle,
            count: u32,
            views: *const Handle,
            width: u32,
            height: u32,
            layers: u32,
        }
        let info = Info {
            s_type: 37,
            next: std::ptr::null(),
            flags: 0,
            pass,
            count: views.len() as u32,
            views: views.as_ptr(),
            width,
            height,
            layers: 1,
        };
        let mut fb = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_framebuffer)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut fb,
                ),
                "framebuffer",
            )?;
        }
        Ok(fb)
    }

    fn make_buffer(&self, size: u64, usage: u32, memory: Memory) -> Result<Buffer, String> {
        self.make_buffer_queues(size, usage, memory, &[])
    }

    fn make_buffer_queues(
        &self,
        size: u64,
        usage: u32,
        memory: Memory,
        families: &[u32],
    ) -> Result<Buffer, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: u64,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
        }
        let shared = families.len() > 1;
        let info = Info {
            s_type: 12,
            next: std::ptr::null(),
            flags: 0,
            size,
            usage,
            sharing: u32::from(shared),
            queue_count: if shared { families.len() as u32 } else { 0 },
            queues: if shared {
                families.as_ptr()
            } else {
                std::ptr::null()
            },
        };
        let mut buffer = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_buffer)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut buffer,
                ),
                "buffer",
            )?;
            let mut req = [0u8; 24];
            (self.fns.buffer_reqs)(self.device, buffer, req.as_mut_ptr());
            let req_size = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            let memory = self.alloc(req_size, bits, memory)?;
            check(
                (self.fns.bind_buffer)(self.device, buffer, memory, 0),
                "bind buffer",
            )?;
            Ok(Buffer {
                buffer,
                memory,
                size: req_size,
            })
        }
    }

    fn note_vertex_count(&mut self, count: u32) {
        self.vertex_count = count;
    }

    fn note_overlay_count(&mut self, count: u32) {
        self.overlay_count = count;
    }

    fn alloc(&self, size: u64, type_bits: u32, memory: Memory) -> Result<Handle, String> {
        if memory == Memory::Device {
            let pending = self.fail_device_allocs.get();
            if pending > 0 {
                self.fail_device_allocs.set(pending - 1);
                return Err("out of device memory".into());
            }
        }
        // Buffers the CPU only writes go in memory the GPU reads at full speed when the
        // device has it (resizable BAR, or the 256 MB BAR window): from plain host memory
        // every probe, scene and tier read crossed PCIe. Readbacks want cached host memory.
        let first = match memory {
            Memory::Device => None,
            Memory::Upload => Some(1 | 2 | 4),
            Memory::Readback => Some(2 | 4 | 8),
        };
        if memory == Memory::Upload {
            let [local, all] = self.upload_local.get();
            self.upload_local.set([local, all + 1]);
        }
        if let Some(want) = first {
            if let Some(index) = self.memory_index(type_bits, want) {
                if let Ok(handle) = self.alloc_index(size, index) {
                    if memory == Memory::Upload {
                        let [local, all] = self.upload_local.get();
                        self.upload_local.set([local + 1, all]);
                    }
                    return Ok(handle);
                }
            }
        }
        let host = memory != Memory::Device;
        let want = if host { 2 | 4 } else { 1 };
        let mut index = self.memory_index(type_bits, want);
        if index.is_none() && !host {
            for i in 0..self.memory_props.count {
                if type_bits & (1 << i) != 0 {
                    index = Some(i);
                    break;
                }
            }
        }
        let Some(index) = index else {
            return Err("no memory type".into());
        };
        let handle = self.alloc_index(size, index)?;
        if memory == Memory::Device {
            self.device_bytes
                .set(self.device_bytes.get().saturating_add(size));
        }
        Ok(handle)
    }

    /// The first memory type allowed by `type_bits` with every flag in `want`.
    fn memory_index(&self, type_bits: u32, want: u32) -> Option<u32> {
        (0..self.memory_props.count).find(|&i| {
            type_bits & (1 << i) != 0 && self.memory_props.types[i as usize] & want == want
        })
    }

    fn alloc_index(&self, size: u64, index: u32) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            size: u64,
            index: u32,
        }
        let info = Info {
            s_type: 5,
            next: std::ptr::null(),
            size,
            index,
        };
        let mut memory = std::ptr::null_mut();
        unsafe {
            let result = (self.fns.alloc_mem)(
                self.device,
                &info as *const Info as *const u8,
                std::ptr::null(),
                &mut memory,
            );
            if result == VK_ERROR_OUT_OF_DEVICE_MEMORY {
                return Err("out of device memory".into());
            }
            check(result, "allocate")?;
        }
        Ok(memory)
    }

    fn apply_material_settings(&mut self, settings: genos_load::MaterialSettings) {
        self.streamer.set_quality_floor(settings.quality_floor());
        self.streamer.set_bias(settings.mip_bias);
        self.streamer.set_dwell(settings.dwell_frames);
        self.materials = settings;
    }

    /// Poll the transfer fence, refresh the budget, and admit what the frame can hold.
    fn tick_materials(
        &mut self,
        accounted: u64,
        views: &[genos_load::TextureView],
    ) -> Result<(), String> {
        let now = Instant::now();
        let base_ms = self.frame_mark.map_or(8.0, |then| {
            now.saturating_duration_since(then).as_secs_f32() * 1000.0
        });
        self.frame_mark = Some(now);
        let pixels = u64::from(self.extent_w)
            .saturating_mul(u64::from(self.extent_h))
            .saturating_mul(16);
        let non_texture = accounted
            .saturating_add(self.non_texture_extra)
            .saturating_add(pixels);
        self.refresh_vram(non_texture);
        let ready = self.transfer_signaled();
        if self.transfer_busy && ready {
            if self.transfer_bytes > 0 {
                let elapsed = self
                    .transfer_submitted
                    .map_or(1, |start| start.elapsed().as_millis());
                let ms = u32::try_from(elapsed).unwrap_or(u32::MAX).max(1);
                self.streamer.observe_transfer(self.transfer_bytes, ms);
            }
            self.transfer_busy = false;
            self.transfer_bytes = 0;
        }
        self.streamer
            .set_transfer_ready(!self.transfer_busy || ready);
        let budget = self.vram;
        self.streamer.step(&budget, views, base_ms, FRAME_BUDGET_MS);
        self.reclaim_unheld();
        Ok(())
    }

    fn refresh_vram(&mut self, non_texture: u64) {
        self.budget_tick = self.budget_tick.wrapping_add(1);
        if self.heap_override.is_none()
            && (self.budget_tick == 1 || self.budget_tick.is_multiple_of(4))
        {
            self.query_heaps();
        }
        let headroom = genos_load::Headroom::from_mb(
            self.materials.headroom_percent,
            self.materials.headroom_floor_mb,
        );
        if let Some((budget, usage)) = self.heap_override {
            self.vram = genos_load::VramBudget::from_heaps(
                &[genos_load::HeapSlice {
                    budget,
                    usage,
                    device_local: true,
                }],
                non_texture,
                headroom,
                true,
            );
            return;
        }
        self.vram = if self.budget_extension {
            genos_load::VramBudget::from_heaps(
                &[genos_load::HeapSlice {
                    budget: self.cached_budget,
                    usage: self.cached_usage,
                    device_local: true,
                }],
                non_texture,
                headroom,
                true,
            )
        } else {
            genos_load::VramBudget::from_heap_size(
                self.memory_props.device_local_bytes,
                self.cached_usage,
                non_texture,
                headroom,
            )
        };
    }

    fn query_heaps(&mut self) {
        let Some(query) = self.memory_budget_query else {
            self.budget_extension = false;
            return;
        };
        #[repr(C)]
        struct BudgetProps {
            s_type: i32,
            pad: u32,
            next: *const c_void,
            heap_budget: [u64; 16],
            heap_usage: [u64; 16],
        }
        let mut budget = BudgetProps {
            s_type: 1000237000,
            pad: 0,
            next: std::ptr::null(),
            heap_budget: [0; 16],
            heap_usage: [0; 16],
        };
        let mut raw = vec![0u8; 16 + 520];
        raw[..4].copy_from_slice(&1_000_059_006i32.to_ne_bytes());
        let next = &mut budget as *mut BudgetProps as usize;
        raw[8..16].copy_from_slice(&next.to_ne_bytes());
        unsafe { query(self.physical, raw.as_mut_ptr()) };
        let props = parse_mem_props(&raw[16..]);
        let heap_count = u32::from_ne_bytes(raw[16 + 260..16 + 264].try_into().unwrap_or([0; 4])).min(16);
        let mut heap_budget = 0u64;
        let mut heap_usage = 0u64;
        for index in 0..heap_count {
            let base = 16 + 264 + index as usize * 16;
            if raw.len() < base + 12 {
                break;
            }
            let flags = u32::from_ne_bytes(raw[base + 8..base + 12].try_into().unwrap_or([0; 4]));
            if flags & 1 == 0 {
                continue;
            }
            let slot = index as usize;
            heap_budget = heap_budget.saturating_add(budget.heap_budget[slot]);
            heap_usage = heap_usage.saturating_add(budget.heap_usage[slot]);
        }
        let _ = props;
        self.cached_budget = heap_budget;
        self.cached_usage = heap_usage;
        self.budget_extension = true;
    }

    fn transfer_signaled(&self) -> bool {
        if self.transfer_fence.is_null() {
            return true;
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.transfer_fence) };
        status == VK_SUCCESS
    }

    /// Copy staging mip bytes into device images, then blit the finest resident
    /// mip into the device atlas. The fence is polled on the next tick.
    fn submit_transfer(&mut self) -> Result<(), String> {
        if self.transfer_cmd.is_null() || self.transfer_fence.is_null() {
            return Ok(());
        }
        if self.transfer_busy && !self.transfer_signaled() {
            return Ok(());
        }
        let due = self.uncopied_pages();
        let republish = self.ready_to_show();
        if due.is_empty() && republish.is_empty() {
            return Ok(());
        }
        // The previous fill finished with the fence. Put the chain back so this
        // submit's page copy still has bytes, then the command buffer wipes it.
        self.restore_staging()?;
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, &self.transfer_fence),
                "transfer reset fence",
            )?;
            check((self.fns.reset_cmd)(self.transfer_cmd, 0), "transfer reset")?;
            #[repr(C)]
            struct BeginInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                inheritance: *const c_void,
            }
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 0x1,
                inheritance: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(self.transfer_cmd, &begin as *const BeginInfo as *const u8),
                "transfer begin",
            )?;
        }
        self.staging_barrier(0x4000, 0x1000, 0x4000, 0x800);
        let mut copied = 0u64;
        let mut filled = Vec::new();
        for (index, level) in &due {
            let bytes = self.record_mip_copy(*index, *level);
            if bytes > 0 {
                copied = copied.saturating_add(bytes);
                filled.push((*index, *level));
            }
        }
        for (index, level) in filled {
            if let Some(page) = self
                .stream_pages
                .iter_mut()
                .find(|page| page.index == index && page.level == level)
            {
                page.copied = true;
                page.layout = 6;
            }
        }
        // The page images now hold the texels. Wipe staging before the atlas
        // write so a buffer copy into the atlas cannot supply them.
        self.staging_barrier(0x1000, 0x1000, 0x800, 0x1000);
        self.scrub_staging();
        self.staging_barrier(0x1000, 0x1000, 0x1000, 0x800);
        self.prepare_atlas_draw(true);
        self.prepare_atlas_draw(false);
        let show = self.ready_to_show();
        for (index, level) in &show {
            self.record_mip_blit(*index, *level);
        }
        for (index, level) in show {
            if let Some(source) = self
                .stream_sources
                .iter_mut()
                .find(|source| source.index == index)
            {
                source.shown = Some(level);
            }
        }
        self.finish_atlas_draw(true, true);
        self.finish_atlas_draw(false, false);
        unsafe {
            check((self.fns.end_cmd)(self.transfer_cmd), "transfer end")?;
            #[repr(C)]
            struct Submit {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                stages: *const u32,
                cmd_count: u32,
                cmds: *const Handle,
                signal_count: u32,
                signals: *const Handle,
            }
            let cmd = [self.transfer_cmd];
            let signal = [self.transfer_done];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 1,
                signals: signal.as_ptr(),
            };
            check(
                (self.fns.queue_submit)(
                    self.transfer_queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.transfer_fence,
                ),
                "transfer submit",
            )?;
        }
        self.transfer_wait = true;
        self.transfer_busy = true;
        self.transfer_bytes = copied;
        self.transfer_submitted = Some(Instant::now());
        Ok(())
    }

    /// Finest resident mips whose device image already holds a copy and whose
    /// atlas blit is stale.
    fn ready_to_show(&self) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for source in &self.stream_sources {
            let Some(level) = self.streamer.finest_resident(source.index) else {
                continue;
            };
            if source.shown == Some(level) {
                continue;
            }
            if self.page_copied(source.index, level) {
                out.push((source.index, level));
            }
        }
        out
    }

    fn page_copied(&self, index: usize, level: u32) -> bool {
        self.stream_pages
            .iter()
            .any(|page| page.index == index && page.level == level && page.copied)
    }

    fn uncopied_pages(&self) -> Vec<(usize, u32)> {
        self.stream_pages
            .iter()
            .filter(|page| !page.memory.is_null() && !page.copied)
            .map(|page| (page.index, page.level))
            .collect()
    }

    fn copies_due(&self) -> Vec<(usize, u32)> {
        let mut due = Vec::new();
        for copy in self.streamer.pending_copies() {
            if !self.page_has(copy.index, copy.level) {
                due.push((copy.index, copy.level));
            }
        }
        let count = self.streamer.texture_count();
        for index in 0..count {
            let levels = self.streamer.level_count(index);
            for level in 0..levels {
                if self.streamer.is_resident(index, level)
                    && !self.page_has(index, level)
                    && !due.contains(&(index, level))
                {
                    due.push((index, level));
                }
            }
        }
        due
    }

    fn page_has(&self, index: usize, level: u32) -> bool {
        self.stream_pages
            .iter()
            .any(|page| page.index == index && page.level == level)
    }

    fn record_mip_copy(&self, index: usize, level: u32) -> u64 {
        let Some(source) = self.stream_sources.iter().find(|source| source.index == index) else {
            return 0;
        };
        let Some(slot) = usize::try_from(level).ok() else {
            return 0;
        };
        let Some(mip) = source.chain.levels.get(slot) else {
            return 0;
        };
        let Some(page) = self
            .stream_pages
            .iter()
            .find(|page| page.index == index && page.level == level)
        else {
            return 0;
        };
        let offset = source.mip_offset.get(slot).copied().unwrap_or(0);
        let width = mip.width.max(1);
        let height = mip.height.max(1);
        self.transfer_to_image(
            source.staging.buffer,
            offset,
            page.image,
            page.layout,
            width,
            height,
        );
        u64::from(width) * u64::from(height) * 4
    }

    /// Move the atlas to transfer-dest and clear it once. A later blit overwrites
    /// the texels a copy actually delivered. A skipped copy leaves this clear.
    fn prepare_atlas_draw(&mut self, color: bool) {
        let slot = usize::from(!color);
        let image = self.sampled_image(color).image;
        if image.is_null() {
            return;
        }
        let old = self.sampled_layout[slot];
        if old != 7 {
            let src_access = if old == 0 { 0 } else { 0x20 };
            self.cmd_image_barrier(
                self.transfer_cmd,
                image,
                old,
                7,
                0x1000,
                0x1000,
                src_access,
                0x1000,
            );
            self.sampled_layout[slot] = 7;
        }
        if !self.sampled_cleared[slot] {
            self.clear_color(self.transfer_cmd, image, 7);
            self.sampled_cleared[slot] = true;
        }
    }

    /// Leave the atlas in general layout so the fragment shader can load it.
    /// The colour atlas also copies texel (0, 0) into the readback buffer.
    fn finish_atlas_draw(&mut self, color: bool, readback: bool) {
        let slot = usize::from(!color);
        if self.sampled_layout[slot] != 7 {
            return;
        }
        let image = self.sampled_image(color).image;
        if image.is_null() {
            return;
        }
        self.cmd_image_barrier(
            self.transfer_cmd,
            image,
            7,
            1,
            0x1000,
            0x1000,
            0x1000,
            0x800 | 0x20,
        );
        self.sampled_layout[slot] = 1;
        if readback {
            self.copy_atlas_texel(image);
        }
    }

    fn copy_page_texel(&self, image: Handle) {
        self.copy_image_texel(image, 6, &self.page_readback);
    }

    fn record_mip_blit(&self, index: usize, level: u32) {
        let Some(source) = self.stream_sources.iter().find(|source| source.index == index) else {
            return;
        };
        let Some(slot) = usize::try_from(level).ok() else {
            return;
        };
        let Some(mip) = source.chain.levels.get(slot) else {
            return;
        };
        let Some(page) = self.copied_page(index, level, mip.width.max(1), mip.height.max(1))
        else {
            return;
        };
        self.publish_resident(&page, source.color);
    }

    /// A device page after `vkCmdCopyBufferToImage`. An atlas write takes this
    /// handle and not a staging buffer.
    fn copied_page(&self, index: usize, level: u32, width: u32, height: u32) -> Option<ResidentPage> {
        let page = self.stream_pages.iter().find(|page| {
            page.index == index && page.level == level && page.copied && !page.image.is_null()
        })?;
        Some(ResidentPage {
            image: page.image,
            width,
            height,
        })
    }

    /// Blit `page` into the device atlas. This is the only atlas write.
    fn publish_resident(&self, page: &ResidentPage, color: bool) {
        let atlas = self.sampled_image(color).image;
        let (atlas_w, atlas_h) = self.sampled_size[usize::from(!color)];
        if atlas.is_null() || page.image.is_null() {
            return;
        }
        self.blit_nearest(
            page.image,
            atlas,
            page.width,
            page.height,
            atlas_w.max(1),
            atlas_h.max(1),
        );
        if color {
            self.copy_page_texel(page.image);
        }
    }

    fn restore_staging(&self) -> Result<(), String> {
        for source in &self.stream_sources {
            if source.staging.buffer.is_null() {
                continue;
            }
            let mut packed = Vec::new();
            for level in &source.chain.levels {
                packed.extend_from_slice(&level.rgba);
            }
            if packed.is_empty() {
                continue;
            }
            self.write_buffer(&source.staging, &packed)?;
        }
        Ok(())
    }

    fn scrub_staging(&self) {
        for source in &self.stream_sources {
            let buffer = source.staging.buffer;
            let size = source.staging.size & !3;
            if buffer.is_null() || size < 4 {
                continue;
            }
            unsafe {
                (self.fns.cmd_fill_buffer)(self.transfer_cmd, buffer, 0, size, 0);
            }
        }
    }

    fn staging_barrier(&self, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
        for source in &self.stream_sources {
            let buffer = source.staging.buffer;
            let size = source.staging.size & !3;
            if buffer.is_null() || size < 4 {
                continue;
            }
            self.transfer_buffer_barrier(buffer, size, src_stage, dst_stage, src_access, dst_access);
        }
    }

    fn transfer_buffer_barrier(
        &self,
        buffer: Handle,
        size: u64,
        src_stage: u32,
        dst_stage: u32,
        src_access: u32,
        dst_access: u32,
    ) {
        #[repr(C)]
        struct Bar {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
            src_queue: u32,
            dst_queue: u32,
            buffer: Handle,
            offset: u64,
            size: u64,
        }
        let bar = Bar {
            s_type: 44,
            next: std::ptr::null(),
            src_access,
            dst_access,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            buffer,
            offset: 0,
            size,
        };
        unsafe {
            (self.fns.cmd_barrier)(
                self.transfer_cmd,
                src_stage,
                dst_stage,
                0,
                0,
                std::ptr::null(),
                1,
                &bar as *const Bar as *const c_void,
                0,
                std::ptr::null(),
            );
        }
    }

    fn clear_color(&self, cmd: Handle, image: Handle, layout: u32) {
        let color = [0.0f32; 4];
        #[repr(C)]
        struct Range {
            aspect: u32,
            mip: u32,
            levels: u32,
            layer: u32,
            layers: u32,
        }
        let range = Range {
            aspect: 1,
            mip: 0,
            levels: 1,
            layer: 0,
            layers: 1,
        };
        unsafe {
            (self.fns.cmd_clear_color)(
                cmd,
                image,
                layout as i32,
                color.as_ptr().cast(),
                1,
                &range as *const Range as *const u8,
            );
        }
    }

    fn blit_nearest(
        &self,
        src: Handle,
        dst: Handle,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Layers {
            aspect: u32,
            mip: u32,
            layer: u32,
            count: u32,
        }
        #[repr(C)]
        struct Offset {
            x: i32,
            y: i32,
            z: i32,
        }
        #[repr(C)]
        struct Blit {
            src_layers: Layers,
            src_off: [Offset; 2],
            dst_layers: Layers,
            dst_off: [Offset; 2],
        }
        let layers = Layers {
            aspect: 1,
            mip: 0,
            layer: 0,
            count: 1,
        };
        let blit = Blit {
            src_layers: layers,
            src_off: [
                Offset { x: 0, y: 0, z: 0 },
                Offset {
                    x: src_w as i32,
                    y: src_h as i32,
                    z: 1,
                },
            ],
            dst_layers: layers,
            dst_off: [
                Offset { x: 0, y: 0, z: 0 },
                Offset {
                    x: dst_w as i32,
                    y: dst_h as i32,
                    z: 1,
                },
            ],
        };
        unsafe {
            (self.fns.cmd_blit)(
                self.transfer_cmd,
                src,
                6,
                dst,
                7,
                1,
                &blit as *const Blit as *const u8,
                0,
            );
        }
    }

    fn copy_atlas_texel(&self, image: Handle) {
        self.copy_image_texel(image, 1, &self.atlas_readback);
    }

    fn copy_image_texel(&self, image: Handle, layout: i32, buffer: &Buffer) {
        #[repr(C)]
        struct Layers {
            aspect: u32,
            mip: u32,
            layer: u32,
            count: u32,
        }
        #[repr(C)]
        struct Offset {
            x: i32,
            y: i32,
            z: i32,
        }
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Region {
            offset: u64,
            row: u32,
            height: u32,
            layers: Layers,
            image_offset: Offset,
            extent: Extent,
        }
        let region = Region {
            offset: 0,
            row: 0,
            height: 0,
            layers: Layers {
                aspect: 1,
                mip: 0,
                layer: 0,
                count: 1,
            },
            image_offset: Offset { x: 0, y: 0, z: 0 },
            extent: Extent { w: 1, h: 1, d: 1 },
        };
        unsafe {
            (self.fns.cmd_copy_to_buffer)(
                self.transfer_cmd,
                image,
                layout,
                buffer.buffer,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn create_unbound_image(&self, width: u32, height: u32) -> Result<UnboundImage, String> {
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            image_type: u32,
            format: i32,
            extent: Extent,
            mips: u32,
            layers: u32,
            samples: u32,
            tiling: u32,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
            initial: u32,
        }
        let info = Info {
            s_type: 14,
            next: std::ptr::null(),
            flags: 0,
            image_type: 1,
            format: 37,
            extent: Extent {
                w: width,
                h: height,
                d: 1,
            },
            mips: 1,
            layers: 1,
            samples: 1,
            tiling: 0,
            usage: 0x1 | 0x2,
            sharing: 0,
            queue_count: 0,
            queues: std::ptr::null(),
            initial: 0,
        };
        let mut image = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_image)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut image,
                ),
                "mip image",
            )?;
            let mut req = [0u8; 24];
            (self.fns.image_reqs)(self.device, image, req.as_mut_ptr());
            let bytes = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let type_bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            Ok(UnboundImage {
                image,
                bytes,
                type_bits,
            })
        }
    }

    fn transfer_to_image(
        &self,
        buffer: Handle,
        offset: u64,
        image: Handle,
        old_layout: u32,
        width: u32,
        height: u32,
    ) {
        #[repr(C)]
        struct Range {
            aspect: u32,
            mip: u32,
            levels: u32,
            layer: u32,
            layers: u32,
        }
        #[repr(C)]
        struct Barrier {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
            old: u32,
            new: u32,
            src_queue: u32,
            dst_queue: u32,
            image: Handle,
            range: Range,
        }
        let barrier = Barrier {
            s_type: 45,
            next: std::ptr::null(),
            src_access: 0,
            dst_access: 0x1000,
            old: old_layout,
            new: 7,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            image,
            range: Range {
                aspect: 1,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        let order = Barrier {
            s_type: 45,
            next: std::ptr::null(),
            src_access: 0x1000,
            dst_access: 0x1000,
            old: 7,
            new: 7,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            image,
            range: Range {
                aspect: 1,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        let to_src = Barrier {
            s_type: 45,
            next: std::ptr::null(),
            src_access: 0x1000,
            dst_access: 0x800,
            old: 7,
            new: 6,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            image,
            range: Range {
                aspect: 1,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        #[repr(C)]
        struct Region {
            buffer_offset: u64,
            row_length: u32,
            image_height: u32,
            aspect: u32,
            mip: u32,
            layer: u32,
            layers: u32,
            x: i32,
            y: i32,
            z: i32,
            w: u32,
            h: u32,
            d: u32,
        }
        let region = Region {
            buffer_offset: offset,
            row_length: 0,
            image_height: 0,
            aspect: 1,
            mip: 0,
            layer: 0,
            layers: 1,
            x: 0,
            y: 0,
            z: 0,
            w: width,
            h: height,
            d: 1,
        };
        unsafe {
            if old_layout != 7 {
                (self.fns.cmd_barrier)(
                    self.transfer_cmd,
                    0x1,
                    0x1000,
                    0,
                    0,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    1,
                    &barrier as *const Barrier as *const u8,
                );
            }
            // Zero first. The buffer copy is the only writer of a real texel.
            // Skipping that copy leaves the clear, and the blit publishes black.
            self.clear_color(self.transfer_cmd, image, 7);
            (self.fns.cmd_barrier)(
                self.transfer_cmd,
                0x1000,
                0x1000,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &order as *const Barrier as *const u8,
            );
            (self.fns.cmd_copy_to_image)(
                self.transfer_cmd,
                buffer,
                image,
                7,
                1,
                &region as *const Region as *const u8,
            );
            (self.fns.cmd_barrier)(
                self.transfer_cmd,
                0x1000,
                0x1000,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_src as *const Barrier as *const u8,
            );
        }
    }

    fn drop_stream_resources(&mut self) {
        let pages = std::mem::take(&mut self.stream_pages);
        for mut page in pages {
            self.free_page(&mut page);
        }
        let sources = std::mem::take(&mut self.stream_sources);
        for mut source in sources {
            self.destroy_buffer(&mut source.staging);
        }
    }

    fn free_page(&self, page: &mut StreamPage) {
        unsafe {
            if !page.image.is_null() {
                (self.fns.destroy_image)(self.device, page.image, std::ptr::null());
                page.image = std::ptr::null_mut();
            }
            if !page.memory.is_null() {
                self.release_device(page.memory, page.bytes);
                page.memory = std::ptr::null_mut();
            }
        }
    }

    /// Free device memory and drop those bytes from the running total.
    fn release_device(&self, memory: Handle, bytes: u64) {
        if memory.is_null() {
            return;
        }
        unsafe { (self.fns.free_mem)(self.device, memory, std::ptr::null()) };
        self.device_bytes
            .set(self.device_bytes.get().saturating_sub(bytes));
    }

    fn allocate_texture(&mut self, size: u64, type_bits: u32) -> Result<Handle, String> {
        let mut left = 4u32;
        loop {
            match self.alloc(size, type_bits, Memory::Device) {
                Ok(memory) => return Ok(memory),
                Err(err)
                    if err == "out of device memory" && left > 0 && self.evict_for_device() =>
                {
                    left -= 1;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Drop the lowest-priority mip and release its device memory.
    fn evict_for_device(&mut self) -> bool {
        if !self.streamer.evict_for_oom() {
            return false;
        }
        self.reclaim_unheld();
        true
    }

    fn reclaim_unheld(&mut self) {
        let pages = std::mem::take(&mut self.stream_pages);
        let mut keep = Vec::new();
        for mut page in pages {
            if self.streamer.holds_memory(page.index, page.level) {
                keep.push(page);
            } else {
                self.free_page(&mut page);
            }
        }
        self.stream_pages = keep;
    }

    fn has_unbacked(&self) -> bool {
        self.stream_pages.iter().any(|page| page.memory.is_null())
    }

    fn next_unbacked(&self) -> Result<(u64, u32), String> {
        self.stream_pages
            .iter()
            .find(|page| page.memory.is_null())
            .map(|page| (page.bytes, page.type_bits))
            .ok_or_else(|| "no mip is waiting for memory".to_string())
    }

    fn bind_next_unbacked(&mut self, memory: Handle) -> Result<(), String> {
        let Some(page) = self
            .stream_pages
            .iter_mut()
            .find(|page| page.memory.is_null())
        else {
            unsafe { (self.fns.free_mem)(self.device, memory, std::ptr::null()) };
            return Err("no mip is waiting for memory".into());
        };
        let image = page.image;
        page.memory = memory;
        unsafe {
            check(
                (self.fns.bind_image)(self.device, image, memory, 0),
                "bind mip",
            )?;
        }
        Ok(())
    }

    fn prepare_unbacked(&mut self) -> Result<(), String> {
        let due = self.copies_due();
        for (index, level) in due {
            if self
                .stream_pages
                .iter()
                .any(|page| page.index == index && page.level == level)
            {
                continue;
            }
            let Some(source) = self
                .stream_sources
                .iter()
                .find(|source| source.index == index)
            else {
                continue;
            };
            let Some(slot) = usize::try_from(level).ok() else {
                continue;
            };
            let Some(mip) = source.chain.levels.get(slot) else {
                continue;
            };
            let image = self.create_unbound_image(mip.width.max(1), mip.height.max(1))?;
            self.stream_pages.push(StreamPage {
                index,
                level,
                image: image.image,
                memory: std::ptr::null_mut(),
                bytes: image.bytes,
                type_bits: image.type_bits,
                copied: false,
                layout: 0,
            });
        }
        Ok(())
    }

    fn image_barrier(
        &self,
        image: Handle,
        old: u32,
        new: u32,
        src_stage: u32,
        dst_stage: u32,
        src_access: u32,
        dst_access: u32,
    ) {
        self.cmd_image_barrier(
            self.cmd,
            image,
            old,
            new,
            src_stage,
            dst_stage,
            src_access,
            dst_access,
        );
    }

    fn cmd_image_barrier(
        &self,
        cmd: Handle,
        image: Handle,
        old: u32,
        new: u32,
        src_stage: u32,
        dst_stage: u32,
        src_access: u32,
        dst_access: u32,
    ) {
        #[repr(C)]
        struct Range {
            aspect: u32,
            mip: u32,
            levels: u32,
            layer: u32,
            layers: u32,
        }
        #[repr(C)]
        struct Barrier {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
            old: u32,
            new: u32,
            src_queue: u32,
            dst_queue: u32,
            image: Handle,
            range: Range,
        }
        let barrier = Barrier {
            s_type: 45,
            next: std::ptr::null(),
            src_access,
            dst_access,
            old,
            new,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            image,
            range: Range {
                aspect: 1,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        unsafe {
            (self.fns.cmd_barrier)(
                cmd,
                src_stage,
                dst_stage,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &barrier as *const Barrier as *const u8,
            );
        }
    }

    fn copy_color_to_swapchain(&self, swap: Handle) {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Layers {
            aspect: u32,
            mip: u32,
            layer: u32,
            count: u32,
        }
        #[repr(C)]
        struct Offset {
            x: i32,
            y: i32,
            z: i32,
        }
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Region {
            src_layers: Layers,
            src_offset: Offset,
            dst_layers: Layers,
            dst_offset: Offset,
            extent: Extent,
        }
        let layers = Layers {
            aspect: 1,
            mip: 0,
            layer: 0,
            count: 1,
        };
        let region = Region {
            src_layers: layers,
            src_offset: Offset { x: 0, y: 0, z: 0 },
            dst_layers: layers,
            dst_offset: Offset { x: 0, y: 0, z: 0 },
            extent: Extent {
                w: self.extent_w,
                h: self.extent_h,
                d: 1,
            },
        };
        unsafe {
            (self.fns.cmd_copy_image)(
                self.cmd,
                self.color.image,
                6,
                swap,
                7,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn copy_color_to_buffer(&self) {
        #[repr(C)]
        struct Layers {
            aspect: u32,
            mip: u32,
            layer: u32,
            count: u32,
        }
        #[repr(C)]
        struct Offset {
            x: i32,
            y: i32,
            z: i32,
        }
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
            d: u32,
        }
        #[repr(C)]
        struct Region {
            offset: u64,
            row: u32,
            height: u32,
            layers: Layers,
            image_offset: Offset,
            extent: Extent,
        }
        let region = Region {
            offset: 0,
            row: 0,
            height: 0,
            layers: Layers {
                aspect: 1,
                mip: 0,
                layer: 0,
                count: 1,
            },
            image_offset: Offset { x: 0, y: 0, z: 0 },
            extent: Extent {
                w: self.extent_w,
                h: self.extent_h,
                d: 1,
            },
        };
        unsafe {
            (self.fns.cmd_copy_to_buffer)(
                self.cmd,
                self.color.image,
                6,
                self.host.buffer,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn destroy_targets(&mut self) {
        unsafe {
            if self.framebuffers[0].is_null() && self.framebuffer.is_null() {
                self.destroy_aa_targets();
                return;
            }
            self.destroy_aa_targets();
            for slot in 0..2 {
                if !self.framebuffers[slot].is_null() {
                    (self.fns.destroy_framebuffer)(
                        self.device,
                        self.framebuffers[slot],
                        std::ptr::null(),
                    );
                    self.framebuffers[slot] = std::ptr::null_mut();
                }
                let mut color = std::mem::replace(&mut self.colors[slot], Image::empty());
                let mut depth = std::mem::replace(&mut self.depths[slot], Image::empty());
                let mut host = std::mem::replace(&mut self.hosts[slot], Buffer::empty());
                self.destroy_image(&mut color);
                self.destroy_image(&mut depth);
                self.destroy_buffer(&mut host);
            }
            self.framebuffer = std::ptr::null_mut();
            self.color = Image::empty();
            self.depth = Image::empty();
            self.host = Buffer::empty();
        }
    }

    /// Presentation images. The seat is dispatched around this call: the
    /// driver round-trips the Wayland socket before it returns, and a client
    /// that does not read the socket waits out the driver timeout.
    fn destroy_swapchain(&mut self) {
        if self.device.is_null() || self.swapchain.is_null() {
            self.swapchain = std::ptr::null_mut();
            self.swap_images.clear();
            return;
        }
        let swapchain = self.swapchain;
        self.swapchain = std::ptr::null_mut();
        unsafe {
            (self.fns.destroy_swapchain)(self.device, swapchain, std::ptr::null());
        }
        self.swap_images.clear();
    }

    fn destroy_image(&self, image: &mut Image) {
        unsafe {
            if !image.view.is_null() {
                (self.fns.destroy_image_view)(self.device, image.view, std::ptr::null());
            }
            if !image.image.is_null() {
                (self.fns.destroy_image)(self.device, image.image, std::ptr::null());
            }
            if !image.memory.is_null() {
                (self.fns.free_mem)(self.device, image.memory, std::ptr::null());
            }
        }
        *image = Image::empty();
    }

    fn destroy_buffer(&self, buffer: &mut Buffer) {
        unsafe {
            if !buffer.buffer.is_null() {
                (self.fns.destroy_buffer)(self.device, buffer.buffer, std::ptr::null());
            }
            if !buffer.memory.is_null() {
                (self.fns.free_mem)(self.device, buffer.memory, std::ptr::null());
            }
        }
        *buffer = Buffer::empty();
    }
}

fn copy_image(image: &Image) -> Image {
    Image {
        image: image.image,
        view: image.view,
        memory: image.memory,
    }
}

fn ray_bytes(listener: [f32; 3], sources: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 + 16 + 256 * 16);
    for value in [listener[0], listener[1], listener[2], 0.0] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for value in [sources.len() as u32, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for index in 0..256 {
        let source = sources.get(index).copied().unwrap_or([0.0; 3]);
        for value in [source[0], source[1], source[2], 0.0] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

fn read_f32s(
    map: FnMap,
    unmap: FnUnmap,
    device: Handle,
    memory: Handle,
    count: usize,
) -> Result<Vec<f32>, String> {
    let bytes = count * 4;
    let mut raw = vec![0u8; bytes];
    unsafe {
        let mut mapped = std::ptr::null_mut();
        check(
            map(device, memory, 0, bytes as u64, 0, &mut mapped),
            "read audio gains",
        )?;
        std::ptr::copy_nonoverlapping(mapped as *const u8, raw.as_mut_ptr(), bytes);
        unmap(device, memory);
    }
    Ok(raw
        .chunks_exact(4)
        .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn copy_buffer(buffer: &Buffer) -> Buffer {
    Buffer {
        buffer: buffer.buffer,
        memory: buffer.memory,
        size: buffer.size,
    }
}

impl Image {
    fn empty() -> Self {
        Self {
            image: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
            memory: std::ptr::null_mut(),
        }
    }
}

impl Buffer {
    fn empty() -> Self {
        Self {
            buffer: std::ptr::null_mut(),
            memory: std::ptr::null_mut(),
            size: 0,
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }
        if !self.lifecycle_release {
            // A null device was never opened. Already unwinding: leak. An idle wait is the stall.
            if self.device.is_null() || std::thread::panicking() {
                return;
            }
            panic!("genos-render: the device was dropped outside the app lifecycle");
        }
        // The device pointer is cleared before the surface and the instance,
        // so a null device here still has those two calls left.
        self.finish_teardown();
    }
}

impl Gpu {
    /// `true` when the queues that presented and built lighting have retired.
    ///
    /// An empty submit on each queue, after the last present, is ordered after
    /// that present. Frame fences are polled only when a submit is in flight.
    /// A reset fence with no submit stays unsignaled, so it is not polled.
    /// The transfer queue is separate from graphics and compute.
    pub(crate) fn poll_idle(&mut self) -> bool {
        if self.device.is_null() || self.torn_down {
            return true;
        }
        if !self.shutdown_armed && !self.arm_shutdown() {
            return false;
        }
        self.frames_retired()
            && self.transfer_retired()
            && self.fence_retired(self.shutdown_fence)
            && self.fence_retired(self.shutdown_compute)
            && (!self.lighting.world_busy || self.fence_retired(self.lighting.world_fence))
    }

    fn frames_retired(&self) -> bool {
        (0..2).all(|slot| !self.flight_live[slot] || self.fence_retired(self.fences[slot]))
    }

    fn transfer_retired(&self) -> bool {
        !self.transfer_busy || self.fence_retired(self.transfer_fence)
    }

    /// Name of the group the next [`Self::destroy_step`] call destroys.
    pub(crate) fn next_shutdown_phase(&self) -> &'static str {
        match self.teardown {
            0 => "destroy_yield",
            1 => "destroy_lighting",
            2 => "destroy_swapchain",
            3 => "destroy_objects",
            4 => "destroy_device",
            5 => "destroy_surface",
            _ => "destroy_instance",
        }
    }

    /// Destroy one group and return. `true` when the instance is gone.
    pub(crate) fn destroy_step(&mut self) -> bool {
        if self.torn_down {
            return true;
        }
        self.lifecycle_release = true;
        match self.teardown {
            0 => {
                self.teardown = 1;
                false
            }
            1 => {
                // The seat thread dispatches while the driver call runs. On a
                // real device these calls wait for the compositor, and a missed
                // ping makes that wait hit the driver timeout.
                block(|| self.destroy_lighting());
                self.teardown = 2;
                false
            }
            2 => {
                block(|| self.destroy_swapchain());
                self.teardown = 3;
                false
            }
            3 => {
                block(|| self.destroy_gpu_objects());
                self.teardown = 4;
                false
            }
            4 => {
                block(|| self.destroy_device_handle());
                self.teardown = 5;
                false
            }
            5 => {
                block(|| self.destroy_surface_handle());
                self.teardown = 6;
                false
            }
            _ => {
                block(|| self.destroy_instance_handle());
                true
            }
        }
    }

    fn finish_teardown(&mut self) {
        if self.teardown < 2 {
            self.destroy_lighting();
            self.teardown = 2;
        }
        if self.teardown < 3 {
            self.destroy_swapchain();
            self.teardown = 3;
        }
        if self.teardown < 4 {
            self.destroy_gpu_objects();
            self.teardown = 4;
        }
        if self.teardown < 5 {
            self.destroy_device_handle();
            self.teardown = 5;
        }
        if self.teardown < 6 {
            self.destroy_surface_handle();
            self.teardown = 6;
        }
        self.destroy_instance_handle();
    }

    fn fence_retired(&self, fence: Handle) -> bool {
        if fence.is_null() || self.device.is_null() {
            return true;
        }
        let status = unsafe { (self.fns.fence_status)(self.device, fence) };
        status != VK_NOT_READY
    }

    fn arm_shutdown(&mut self) -> bool {
        if self.shutdown_armed {
            return true;
        }
        let Ok(graphics) = self.make_plain_fence() else {
            return false;
        };
        self.shutdown_fence = graphics;
        let distinct = !self.compute_queue.is_null() && !std::ptr::eq(self.compute_queue, self.queue);
        if distinct {
            let Ok(fence) = self.make_plain_fence() else {
                self.destroy_shutdown_fences();
                return false;
            };
            self.shutdown_compute = fence;
        }
        // The queue can block in the driver until the compositor releases a
        // buffer. The seat thread answers that while this thread waits.
        let graphics_submitted = block(|| self.submit_empty(self.queue, self.shutdown_fence));
        if !graphics_submitted {
            self.destroy_shutdown_fences();
            return false;
        }
        let compute_submitted = !distinct
            || block(|| self.submit_empty(self.compute_queue, self.shutdown_compute));
        if distinct && !compute_submitted {
            unsafe {
                (self.fns.destroy_fence)(self.device, self.shutdown_compute, std::ptr::null());
            }
            self.shutdown_compute = std::ptr::null_mut();
        }
        self.shutdown_armed = true;
        true
    }

    fn make_plain_fence(&mut self) -> Result<Handle, String> {
        #[repr(C)]
        struct FenceInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
        }
        let info = FenceInfo {
            s_type: 8,
            next: std::ptr::null(),
            flags: 0,
        };
        let mut fence = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_fence)(
                    self.device,
                    &info as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut fence,
                ),
                "shutdown fence",
            )?;
        }
        Ok(fence)
    }

    fn submit_empty(&self, queue: Handle, fence: Handle) -> bool {
        if queue.is_null() || fence.is_null() {
            return false;
        }
        #[repr(C)]
        struct Submit {
            s_type: i32,
            next: *const c_void,
            wait_count: u32,
            waits: *const Handle,
            stages: *const u32,
            cmd_count: u32,
            cmds: *const Handle,
            signal_count: u32,
            signals: *const Handle,
        }
        let submit = Submit {
            s_type: 4,
            next: std::ptr::null(),
            wait_count: 0,
            waits: std::ptr::null(),
            stages: std::ptr::null(),
            cmd_count: 0,
            cmds: std::ptr::null(),
            signal_count: 0,
            signals: std::ptr::null(),
        };
        let result = unsafe {
            (self.fns.queue_submit)(queue, 1, &submit as *const Submit as *const u8, fence)
        };
        result == VK_SUCCESS
    }

    fn destroy_shutdown_fences(&mut self) {
        if self.device.is_null() {
            self.shutdown_fence = std::ptr::null_mut();
            self.shutdown_compute = std::ptr::null_mut();
            return;
        }
        unsafe {
            if !self.shutdown_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.shutdown_fence, std::ptr::null());
                self.shutdown_fence = std::ptr::null_mut();
            }
            if !self.shutdown_compute.is_null() {
                (self.fns.destroy_fence)(self.device, self.shutdown_compute, std::ptr::null());
                self.shutdown_compute = std::ptr::null_mut();
            }
        }
    }

    /// Images, buffers, and pipelines. The device, surface, and instance stay.
    fn destroy_gpu_objects(&mut self) {
        if self.device.is_null() {
            return;
        }
        self.lifecycle_release = true;
        // Stream pages and staging buffers are device memory. Free them here,
        // on the lifecycle step, instead of waiting idle in Drop.
        self.drop_stream_resources();
        unsafe {
            self.destroy_targets();
            self.vertex = Buffer::empty();
            let mut vertices =
                std::mem::replace(&mut self.vertices, [Buffer::empty(), Buffer::empty()]);
            let mut scene_buf = std::mem::replace(&mut self.scene_buf, Buffer::empty());
            let mut frame_scene =
                std::mem::replace(&mut self.frame_scene, [Buffer::empty(), Buffer::empty()]);
            let mut particle_buf = std::mem::replace(&mut self.particle_buf, Buffer::empty());
            let mut mesh_field = std::mem::replace(&mut self.mesh_field, Buffer::empty());
            let mut audio_rays = std::mem::replace(&mut self.audio_rays, Buffer::empty());
            let mut audio_gains = std::mem::replace(&mut self.audio_gains, Buffer::empty());
            self.destroy_buffer(&mut vertices[0]);
            self.destroy_buffer(&mut vertices[1]);
            let mut shapes = std::mem::replace(&mut self.shapes, Buffer::empty());
            let mut instance_bufs =
                std::mem::replace(&mut self.instance_bufs, [Buffer::empty(), Buffer::empty()]);
            self.destroy_buffer(&mut shapes);
            self.destroy_buffer(&mut instance_bufs[0]);
            self.destroy_buffer(&mut instance_bufs[1]);
            self.destroy_buffer(&mut scene_buf);
            // [1] names the same buffer as [0] (S70C).
            self.destroy_buffer(&mut frame_scene[0]);
            self.destroy_buffer(&mut particle_buf);
            let mut sampled_color = std::mem::replace(&mut self.sampled_color, Image::empty());
            let mut sampled_normal = std::mem::replace(&mut self.sampled_normal, Image::empty());
            let mut atlas_readback = std::mem::replace(&mut self.atlas_readback, Buffer::empty());
            let mut page_readback = std::mem::replace(&mut self.page_readback, Buffer::empty());
            self.destroy_image(&mut sampled_color);
            self.destroy_image(&mut sampled_normal);
            self.destroy_buffer(&mut atlas_readback);
            self.destroy_buffer(&mut page_readback);
            if !self.transfer_done.is_null() {
                (self.fns.destroy_sem)(self.device, self.transfer_done, std::ptr::null());
                self.transfer_done = std::ptr::null_mut();
            }
            self.destroy_buffer(&mut mesh_field);
            if let Some(times) = self.gpu_times.as_ref() {
                if !times.pool.is_null() {
                    (self.fns.destroy_query_pool)(self.device, times.pool, std::ptr::null());
                }
                if !times.lighting_pool.is_null() {
                    (self.fns.destroy_query_pool)(self.device, times.lighting_pool, std::ptr::null());
                }
            }
            if !self.compute_pool.is_null() {
                (self.fns.destroy_pool)(self.device, self.compute_pool, std::ptr::null());
            }
            if !self.transfer_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.transfer_fence, std::ptr::null());
            }
            if !self.transfer_pool.is_null() {
                (self.fns.destroy_pool)(self.device, self.transfer_pool, std::ptr::null());
            }
            self.destroy_buffer(&mut audio_rays);
            self.destroy_buffer(&mut audio_gains);
            self.destroy_media_check();
            self.destroy_froxel();
            if !self.audio_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.audio_fence, std::ptr::null());
            }
            if !self.audio_pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.audio_pipe, std::ptr::null());
            }
            if !self.audio_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.audio_layout, std::ptr::null());
            }
            if !self.audio_desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.audio_desc_pool, std::ptr::null());
            }
            if !self.audio_desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(
                    self.device,
                    self.audio_desc_layout,
                    std::ptr::null(),
                );
            }
            for slot in 0..2 {
                if !self.fences[slot].is_null() {
                    (self.fns.destroy_fence)(self.device, self.fences[slot], std::ptr::null());
                }
                if !self.image_readies[slot].is_null() {
                    (self.fns.destroy_sem)(self.device, self.image_readies[slot], std::ptr::null());
                }
                if !self.render_dones[slot].is_null() {
                    (self.fns.destroy_sem)(self.device, self.render_dones[slot], std::ptr::null());
                }
            }
            if !self.desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.desc_pool, std::ptr::null());
            }
            if !self.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.desc_layout, std::ptr::null());
            }
            if !self.pool.is_null() {
                (self.fns.destroy_pool)(self.device, self.pool, std::ptr::null());
            }
            if !self.query_pool.is_null() {
                (self.fns.destroy_query_pool)(self.device, self.query_pool, std::ptr::null());
            }
            if !self.hdr_time_pool.is_null() {
                (self.fns.destroy_query_pool)(self.device, self.hdr_time_pool, std::ptr::null());
            }
            if !self.wire_pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.wire_pipeline, std::ptr::null());
            }
            if !self.overlay_pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.overlay_pipeline, std::ptr::null());
            }
            if !self.depth_pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.depth_pipeline, std::ptr::null());
            }
            if !self.pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.pipeline, std::ptr::null());
            }
            if !self.layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.layout, std::ptr::null());
            }
            if !self.keep_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.keep_pass, std::ptr::null());
            }
            if !self.render_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.render_pass, std::ptr::null());
            }
            if !self.aa_fxaa.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.aa_fxaa, std::ptr::null());
            }
            if !self.aa_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.aa_layout, std::ptr::null());
            }
            if !self.aa_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.aa_pool, std::ptr::null());
            }
            if !self.aa_desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.aa_desc_layout, std::ptr::null());
            }
            if !self.shutdown_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.shutdown_fence, std::ptr::null());
                self.shutdown_fence = std::ptr::null_mut();
            }
            if !self.shutdown_compute.is_null() {
                (self.fns.destroy_fence)(self.device, self.shutdown_compute, std::ptr::null());
                self.shutdown_compute = std::ptr::null_mut();
            }
        }
    }

    fn destroy_device_handle(&mut self) {
        if self.device.is_null() {
            return;
        }
        let device = self.device;
        self.device = std::ptr::null_mut();
        unsafe {
            (self.fns.destroy_device)(device, std::ptr::null());
        }
    }

    fn destroy_surface_handle(&mut self) {
        if self.surface.is_null() || self.instance.is_null() {
            self.surface = std::ptr::null_mut();
            return;
        }
        let surface = self.surface;
        self.surface = std::ptr::null_mut();
        unsafe {
            (self.fns.destroy_surface)(self.instance, surface, std::ptr::null());
        }
    }

    fn destroy_instance_handle(&mut self) {
        if self.instance.is_null() {
            self.torn_down = true;
            return;
        }
        let instance = self.instance;
        self.instance = std::ptr::null_mut();
        unsafe {
            (self.fns.destroy_instance)(instance, std::ptr::null());
        }
        self.torn_down = true;
    }
}

fn extension_name_eq(row: &[u8], name: &[u8]) -> bool {
    row.len() >= name.len()
        && row.get(..name.len()) == Some(name)
        && row.get(name.len()).copied().unwrap_or(1) == 0
}

fn extension_present(bytes: &[u8], count: u32, name: &[u8]) -> bool {
    let stride = 260usize;
    let listed = usize::try_from(count).unwrap_or(0);
    for index in 0..listed {
        let start = index.saturating_mul(stride);
        let Some(row) = bytes.get(start..start.saturating_add(256)) else {
            break;
        };
        if extension_name_eq(row, name) {
            return true;
        }
    }
    false
}

fn device_extension_present(
    get_instance: GetProc,
    instance: Handle,
    physical: Handle,
    name: &[u8],
) -> bool {
    unsafe {
        let raw = get_instance(
            instance,
            b"vkEnumerateDeviceExtensionProperties\0".as_ptr() as *const c_char,
        );
        if raw.is_null() {
            return false;
        }
        type Enumerate =
            unsafe extern "system" fn(Handle, *const c_void, *mut u32, *mut u8) -> VkResult;
        let enumerate: Enumerate = transmute(raw);
        let mut count = 0u32;
        if enumerate(physical, std::ptr::null(), &mut count, std::ptr::null_mut()) != 0 || count == 0
        {
            return false;
        }
        let bytes_len = usize::try_from(count).unwrap_or(0).saturating_mul(260);
        let mut bytes = vec![0u8; bytes_len];
        if enumerate(physical, std::ptr::null(), &mut count, bytes.as_mut_ptr()) != 0 {
            return false;
        }
        extension_present(&bytes, count, name)
    }
}

fn check(result: VkResult, what: &str) -> Result<(), String> {
    if result == VK_SUCCESS {
        Ok(())
    } else {
        Err(format!("{what} failed: {result}"))
    }
}

unsafe fn transmute<T>(ptr: Pfn) -> T {
    if ptr.is_null() {
        panic!("missing Vulkan entry point");
    }
    std::mem::transmute_copy(&ptr)
}

fn parse_mem_props(raw: &[u8]) -> MemProps {
    if raw.len() < 4 {
        return MemProps {
            count: 0,
            types: [0; 32],
            device_local_bytes: 0,
        };
    }
    let count = u32::from_ne_bytes(raw[0..4].try_into().unwrap()).min(32);
    let mut types = [0u32; 32];
    for i in 0..count as usize {
        let offset = 4 + i * 8;
        if raw.len() < offset + 4 {
            break;
        }
        types[i] = u32::from_ne_bytes(raw[offset..offset + 4].try_into().unwrap());
    }
    let mut device_local_bytes = 0u64;
    if raw.len() >= 264 {
        let heap_count = u32::from_ne_bytes(raw[260..264].try_into().unwrap()).min(16);
        for index in 0..heap_count as usize {
            let base = 264 + index * 16;
            if raw.len() < base + 12 {
                break;
            }
            let size = u64::from_ne_bytes(raw[base..base + 8].try_into().unwrap());
            let flags = u32::from_ne_bytes(raw[base + 8..base + 12].try_into().unwrap());
            if flags & 1 != 0 {
                device_local_bytes = device_local_bytes.saturating_add(size);
            }
        }
    }
    MemProps {
        count,
        types,
        device_local_bytes,
    }
}

fn extension_listed(bytes: &[u8], count: u32, name: &[u8]) -> bool {
    for index in 0..count as usize {
        let start = index * 260;
        let end = start + name.len();
        if bytes.len() <= end {
            return false;
        }
        if &bytes[start..end] == name && bytes[end] == 0 {
            return true;
        }
    }
    false
}

fn load_fns(
    dload: impl Fn(&[u8]) -> Pfn,
    create_wayland_surface: FnWayland,
    destroy_surface: Fn2,
    surface_support: FnSupport,
    surface_caps: FnCaps,
    surface_formats: FnCount,
    surface_modes: FnCount,
    destroy_instance: FnDestroy,
    enumerate_devices: FnEnumDev,
    queue_families: FnQueues,
    mem_props: FnMemProps,
) -> Fns {
    unsafe {
        macro_rules! d {
            ($name:literal) => {
                transmute(dload(concat!($name, "\0").as_bytes()))
            };
        }
        Fns {
            destroy_instance,
            destroy_device: d!("vkDestroyDevice"),
            create_wayland_surface,
            destroy_surface,
            surface_support,
            surface_caps,
            surface_formats,
            surface_modes,
            create_swapchain: d!("vkCreateSwapchainKHR"),
            destroy_swapchain: d!("vkDestroySwapchainKHR"),
            swapchain_images: d!("vkGetSwapchainImagesKHR"),
            acquire: d!("vkAcquireNextImageKHR"),
            present: d!("vkQueuePresentKHR"),
            create_image_view: d!("vkCreateImageView"),
            destroy_image_view: d!("vkDestroyImageView"),
            create_shader: d!("vkCreateShaderModule"),
            destroy_shader: d!("vkDestroyShaderModule"),
            create_layout: d!("vkCreatePipelineLayout"),
            destroy_layout: d!("vkDestroyPipelineLayout"),
            create_render_pass: d!("vkCreateRenderPass"),
            destroy_render_pass: d!("vkDestroyRenderPass"),
            create_pipelines: d!("vkCreateGraphicsPipelines"),
            destroy_pipeline: d!("vkDestroyPipeline"),
            create_framebuffer: d!("vkCreateFramebuffer"),
            destroy_framebuffer: d!("vkDestroyFramebuffer"),
            create_pool: d!("vkCreateCommandPool"),
            destroy_pool: d!("vkDestroyCommandPool"),
            alloc_cmd: d!("vkAllocateCommandBuffers"),
            begin_cmd: d!("vkBeginCommandBuffer"),
            end_cmd: d!("vkEndCommandBuffer"),
            reset_cmd: d!("vkResetCommandBuffer"),
            cmd_begin_rp: d!("vkCmdBeginRenderPass"),
            cmd_end_rp: d!("vkCmdEndRenderPass"),
            cmd_bind_pipe: d!("vkCmdBindPipeline"),
            cmd_bind_vb: d!("vkCmdBindVertexBuffers"),
            cmd_draw: d!("vkCmdDraw"),
            cmd_push: d!("vkCmdPushConstants"),
            cmd_viewport: d!("vkCmdSetViewport"),
            cmd_scissor: d!("vkCmdSetScissor"),
            cmd_barrier: d!("vkCmdPipelineBarrier"),
            cmd_copy_image: d!("vkCmdCopyImage"),
            cmd_copy_to_buffer: d!("vkCmdCopyImageToBuffer"),
            cmd_copy_buffer: d!("vkCmdCopyBuffer"),
            cmd_copy_to_image: d!("vkCmdCopyBufferToImage"),
            cmd_blit: d!("vkCmdBlitImage"),
            cmd_clear_color: d!("vkCmdClearColorImage"),
            create_sem: d!("vkCreateSemaphore"),
            destroy_sem: d!("vkDestroySemaphore"),
            create_fence: d!("vkCreateFence"),
            destroy_fence: d!("vkDestroyFence"),
            wait_fences: d!("vkWaitForFences"),
            reset_fences: d!("vkResetFences"),
            queue_submit: d!("vkQueueSubmit"),
            fence_status: d!("vkGetFenceStatus"),
            cmd_bind_set: d!("vkCmdBindDescriptorSets"),
            cmd_dispatch: d!("vkCmdDispatch"),
            cmd_dispatch_indirect: d!("vkCmdDispatchIndirect"),
            cmd_update_buffer: d!("vkCmdUpdateBuffer"),
            cmd_fill_buffer: d!("vkCmdFillBuffer"),
            update_desc: d!("vkUpdateDescriptorSets"),
            create_desc_layout: d!("vkCreateDescriptorSetLayout"),
            destroy_desc_layout: d!("vkDestroyDescriptorSetLayout"),
            create_desc_pool: d!("vkCreateDescriptorPool"),
            destroy_desc_pool: d!("vkDestroyDescriptorPool"),
            alloc_desc: d!("vkAllocateDescriptorSets"),
            create_compute: d!("vkCreateComputePipelines"),
            device_wait: d!("vkDeviceWaitIdle"),
            create_buffer: d!("vkCreateBuffer"),
            destroy_buffer: d!("vkDestroyBuffer"),
            buffer_reqs: d!("vkGetBufferMemoryRequirements"),
            alloc_mem: d!("vkAllocateMemory"),
            free_mem: d!("vkFreeMemory"),
            bind_buffer: d!("vkBindBufferMemory"),
            map_mem: d!("vkMapMemory"),
            unmap_mem: d!("vkUnmapMemory"),
            create_image: d!("vkCreateImage"),
            destroy_image: d!("vkDestroyImage"),
            image_reqs: d!("vkGetImageMemoryRequirements"),
            bind_image: d!("vkBindImageMemory"),
            enumerate_devices,
            queue_families,
            mem_props,
            create_query_pool: d!("vkCreateQueryPool"),
            destroy_query_pool: d!("vkDestroyQueryPool"),
            cmd_reset_query: d!("vkCmdResetQueryPool"),
            cmd_write_timestamp: d!("vkCmdWriteTimestamp"),
            get_query_results: d!("vkGetQueryPoolResults"),
        }
    }
}

fn tick_delta(start: u64, end: u64, bits: u32) -> u64 {
    if bits == 0 || bits >= 64 {
        return end.wrapping_sub(start);
    }
    let mask = (1u64 << bits) - 1;
    end.wrapping_sub(start) & mask
}

include!("aa_gpu.rs");
include!("lighting_gpu.rs");

extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> Pfn;
}

/// `Some(true)` when `arg` is `--gpu-times` and the count is the next argument.
/// `Some(false)` when the flag already includes its value. `None` otherwise.
pub fn gpu_times_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = gpu_times_dash(arg)?;
    (name == "gpu-times").then_some(inline.is_none())
}

/// Reject a bad `--gpu-times` argument. `next` is the following argument when
/// [`gpu_times_flag_needs_value`] is `Some(true)`.
pub fn check_gpu_times_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    parse_gpu_times_flag(arg, next).map(|_| ())
}

/// Frames per printed line. `Some` turns the timestamps on. An argument wins.
/// `GENOS_GPU_TIMES` applies only when `--gpu-times` was omitted. A mirror that
/// is not a number uses [`GPU_TIMES_FRAMES`].
fn gpu_times_every(args: &[&str], env: Option<&str>) -> Result<Option<u32>, String> {
    let mut chosen = None;
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).copied();
        match parse_gpu_times_flag(args[i], next)? {
            None => i += 1,
            Some(every) => {
                let extra = gpu_times_flag_needs_value(args[i]).unwrap_or(false);
                chosen = Some(every);
                i += 1 + usize::from(extra);
            }
        }
    }
    if let Some(every) = chosen {
        return Ok(Some(every.max(1)));
    }
    Ok(env.map(|value| value.parse().unwrap_or(GPU_TIMES_FRAMES).max(1)))
}

fn gpu_times_dash(arg: &str) -> Option<(&str, Option<&str>)> {
    let rest = arg.strip_prefix("--")?;
    if rest.is_empty() {
        return None;
    }
    match rest.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((rest, None)),
    }
}

fn parse_gpu_times_flag(arg: &str, next: Option<&str>) -> Result<Option<u32>, String> {
    let Some((name, inline)) = gpu_times_dash(arg) else {
        return Ok(None);
    };
    if name != "gpu-times" {
        return Ok(None);
    }
    let value = match inline {
        Some(value) => value,
        None => next.ok_or_else(|| "--gpu-times needs a value".to_string())?,
    };
    if value.is_empty() {
        return Err("--gpu-times needs a value".into());
    }
    let every = value
        .parse::<u32>()
        .map_err(|_| format!("--gpu-times {value} is not a number"))?;
    Ok(Some(every))
}

/// Frame timestamps per flight slot for `GENOS_GPU_TIMES`.
const FRAME_STAMPS: u32 = 8;
/// Intervals between [`FRAME_STAMPS`] (one fewer than the stamps).
const FRAME_SPANS: usize = FRAME_STAMPS as usize - 1;
/// Frames averaged per `GENOS_GPU_TIMES` line unless it gives a number.
const GPU_TIMES_FRAMES: u32 = 120;

/// `Some(true)` when `arg` is `--gi-metrics` and the switch is the next argument.
/// `Some(false)` when the flag already includes its value. `None` otherwise.
///
/// A bare `--gi-metrics` is on and takes no value. `--gi-metrics=on` and
/// `--gi-metrics=off` carry the switch.
pub fn gi_metrics_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, _) = gpu_times_dash(arg)?;
    (name == "gi-metrics").then_some(false)
}

/// Reject a bad `--gi-metrics` argument.
pub fn check_gi_metrics_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    parse_gi_metrics_flag(arg, next).map(|_| ())
}

/// `--gi-metrics` wins. `GENOS_GI_METRICS` applies only when the argument was
/// omitted. `0`, `off`, `false`, and `no` are off. Anything else set is on.
fn gi_metrics_enabled(args: &[&str], env: Option<&str>) -> Result<bool, String> {
    let mut chosen = None;
    for arg in args {
        if let Some(on) = parse_gi_metrics_flag(arg, None)? {
            chosen = Some(on);
        }
    }
    if let Some(on) = chosen {
        return Ok(on);
    }
    Ok(env.is_some_and(gi_metrics_env_on))
}

fn gi_metrics_env_on(value: &str) -> bool {
    !matches!(value.trim(), "" | "0" | "off" | "false" | "no")
}

fn parse_gi_metrics_flag(arg: &str, _next: Option<&str>) -> Result<Option<bool>, String> {
    let Some((name, inline)) = gpu_times_dash(arg) else {
        return Ok(None);
    };
    if name != "gi-metrics" {
        return Ok(None);
    }
    // A bare `--gi-metrics` is on. It does not take the next argument.
    let Some(value) = inline else {
        return Ok(Some(true));
    };
    if value.is_empty() {
        return Err("--gi-metrics needs on or off".into());
    }
    match value {
        "1" | "on" | "true" | "yes" => Ok(Some(true)),
        "0" | "off" | "false" | "no" => Ok(Some(false)),
        other => Err(format!("--gi-metrics {other} is not on or off")),
    }
}

/// Where a frame timestamp sits. Lighting is its own span so `GPU_MS` does not
/// fold it into the raster.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameStamp {
    Start = 0,
    /// After the resident updates, the swapchain barrier, and `lighting_begin`.
    Setup = 1,
    /// After the depth prepass and the shaded scene raster.
    Gbuffer = 2,
    /// After `lighting_passes` (no overlay, no antialias).
    Lighting = 3,
    /// After the overlay that runs when lighting is on and antialias is off.
    EarlyOverlay = 4,
    /// After the antialias resolve.
    Resolve = 5,
    /// After the overlay that runs when antialias is on.
    Overlay = 6,
    /// After the swapchain copy and the optional host copy.
    Copy = 7,
}

/// `GPU_MS` buckets. `nonlight` is every pass except the lighting span.
struct PassTimes {
    setup: f64,
    gbuffer: f64,
    aa: f64,
    overlay: f64,
    copy: f64,
    nonlight: f64,
    lighting: f64,
    frame: f64,
}

fn present_name(mode: i32) -> &'static str {
    match mode {
        0 => "immediate",
        1 => "mailbox",
        _ => "fifo",
    }
}

/// Split one frame's stamp intervals into the `GPU_MS` line.
///
/// `span` is setup, gbuffer, lighting, early overlay, antialias, late overlay, copy.
fn pass_times(span: &[f64; FRAME_SPANS]) -> PassTimes {
    let setup = span[0];
    let gbuffer = span[1];
    let lighting = span[2];
    let overlay = span[3] + span[5];
    let aa = span[4];
    let copy = span[6];
    let nonlight = setup + gbuffer + aa + overlay + copy;
    PassTimes {
        setup,
        gbuffer,
        aa,
        overlay,
        copy,
        nonlight,
        lighting,
        frame: nonlight + lighting,
    }
}

/// Running sums for `GENOS_GPU_TIMES`.
/// Where a buffer lives: GPU-only, written by the CPU and read by the GPU, or read back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Memory {
    Device,
    Upload,
    Readback,
}

struct GpuTimes {
    pool: Handle,
    /// Frames per printed line.
    every: u32,
    /// `--gpu-times` asked for the `GPU_MS` line. Metrics-only runs stay quiet.
    print: bool,
    /// `--gi-metrics` records one [`GiSample`] per collected lighting frame.
    record: bool,
    samples: VecDeque<GiSample>,
    recorded: u64,
    /// Async cache-round rays and relit entries not yet folded into a sample.
    pending_world_rays: u32,
    pending_world_relit: u32,
    /// A slot's stamps hold a frame that has not been read yet.
    written: [bool; 2],
    frames: u32,
    /// Setup, gbuffer, lighting, early overlay, antialias, late overlay, copy.
    frame: [f64; FRAME_SPANS],
    /// When this line's first frame was read: frames per second count from there.
    since: Option<std::time::Instant>,
    /// The memory placement line has been printed.
    placed: bool,
    /// GI pass stamps (2 slots x LIGHTING_STAMPS), whether a slot holds a full frame's
    /// passes, and the per-pass sums over `lighting_frames`.
    lighting_pool: Handle,
    lighting_ids: [Vec<usize>; 2],
    // GENOS_GPU_DISPATCH: per-dispatch ms (pass id, summed ms) in dispatch order.
    lighting_disp: Vec<(usize, f64)>,
    lighting_frames: u32,
    lighting: [f64; 9],
    /// Per-bucket sums, including cache rays, hit light, and shadows.
    lighting_bucket: [f64; 12],
    /// Frames in this line whose prefill scout was due. Printed as a fraction,
    /// not a millisecond: the scout shares the cache-ray dispatch.
    prefill_frames: u32,
    /// Async light cache rounds (GI) and their GPU time.
    world_runs: u32,
    world_ms: f64,
    /// The async round's passes: compact, trace, light, cache (sums over world_runs).
    world_pass: [f64; 4],
    /// Absolute GPU spans (ticks) of frames and async light cache rounds since the last
    /// line, to show how much of that work ran while a frame was on the GPU.
    frame_spans: Vec<(u64, u64)>,
    light_spans: Vec<(u64, u64)>,
}

/// Stamps per frame for the GI passes, plus two for bloom and the display pass.
/// One before the passes, one after each dispatch, then the HDR pair when it fits.
const LIGHTING_STAMPS: u32 = 28;

/// Frames kept for a gate's p50 and p99. One scenario is shorter than this.
const GI_SAMPLE_CAP: usize = 16_384;

/// Milliseconds of `spans` that fall inside any of `within` (ticks, `period` ns each).
fn overlap_ms(spans: &[(u64, u64)], within: &[(u64, u64)], period: f64) -> (f64, f64) {
    let mut total = 0.0;
    let mut inside = 0.0;
    for &(a, b) in spans {
        if b <= a {
            continue;
        }
        total += (b - a) as f64;
        for &(c, d) in within {
            let lo = a.max(c);
            let hi = b.min(d);
            if hi > lo {
                inside += (hi - lo) as f64;
            }
        }
    }
    (total * period / 1.0e6, inside * period / 1.0e6)
}

impl Default for GpuTimes {
    fn default() -> Self {
        Self {
            pool: std::ptr::null_mut(),
            every: GPU_TIMES_FRAMES,
            print: true,
            record: false,
            samples: VecDeque::new(),
            recorded: 0,
            pending_world_rays: 0,
            pending_world_relit: 0,
            written: [false; 2],
            frames: 0,
            frame: [0.0; FRAME_SPANS],
            since: None,
            placed: false,
            lighting_pool: std::ptr::null_mut(),
            lighting_ids: [Vec::new(), Vec::new()],
            lighting_disp: Vec::new(),
            world_runs: 0,
            world_ms: 0.0,
            world_pass: [0.0; 4],
            lighting_frames: 0,
            lighting: [0.0; 9],
            lighting_bucket: [0.0; 12],
            prefill_frames: 0,
            frame_spans: Vec::new(),
            light_spans: Vec::new(),
        }
    }
}

#[cfg(test)]
mod pass_time_tests {
    use super::pass_times;

    #[test]
    fn nonlight_leaves_out_the_lighting_span() {
        // setup, gbuffer, lighting, early overlay, aa, late overlay, copy.
        let span = [0.20, 1.50, 8.00, 0.05, 0.30, 0.10, 0.15];
        let times = pass_times(&span);
        assert!((times.lighting - 8.0).abs() < 1e-9);
        assert!((times.overlay - 0.15).abs() < 1e-9);
        assert!((times.gbuffer - 1.50).abs() < 1e-9);
        assert!((times.aa - 0.30).abs() < 1e-9);
        assert!((times.copy - 0.15).abs() < 1e-9);
        assert!((times.setup - 0.20).abs() < 1e-9);
        let nonlight = 0.20 + 1.50 + 0.30 + 0.15 + 0.15;
        assert!((times.nonlight - nonlight).abs() < 1e-9);
        assert!((times.frame - (nonlight + 8.0)).abs() < 1e-9);
    }
}


#[cfg(test)]
mod gpu_times_launch {
    use super::{check_gpu_times_arg, gpu_times_every, gpu_times_flag_needs_value};

    #[test]
    fn arguments_win_and_env_is_the_mirror() {
        assert_eq!(gpu_times_every(&[], None).unwrap(), None);
        assert_eq!(gpu_times_every(&[], Some("30")).unwrap(), Some(30));
        assert_eq!(gpu_times_every(&[], Some("nope")).unwrap(), Some(120));
        assert_eq!(
            gpu_times_every(&["--gpu-times", "10"], Some("30")).unwrap(),
            Some(10)
        );
        assert_eq!(
            gpu_times_every(&["--gpu-times=4"], Some("30")).unwrap(),
            Some(4)
        );
        assert_eq!(
            gpu_times_every(&["--scale", "big"], Some("8")).unwrap(),
            Some(8)
        );
        assert_eq!(
            gpu_times_every(&["--gpu-times", "0"], None).unwrap(),
            Some(1)
        );
        assert!(gpu_times_flag_needs_value("--gpu-times").unwrap());
        assert!(gpu_times_flag_needs_value("--gpu-times=3").is_some_and(|needs| !needs));
        assert!(gpu_times_flag_needs_value("--preset").is_none());
        assert_eq!(
            check_gpu_times_arg("--gpu-times", None).unwrap_err(),
            "--gpu-times needs a value"
        );
        assert_eq!(
            check_gpu_times_arg("--gpu-times", Some("nope")).unwrap_err(),
            "--gpu-times nope is not a number"
        );
        assert_eq!(
            check_gpu_times_arg("--gpu-times=", None).unwrap_err(),
            "--gpu-times needs a value"
        );
        assert!(check_gpu_times_arg("--gpu-times", Some("30")).is_ok());
    }
}

#[cfg(test)]
mod gi_metrics_launch {
    use super::{check_gi_metrics_arg, gi_metrics_enabled, gi_metrics_flag_needs_value};

    #[test]
    fn the_argument_wins_and_the_env_is_the_mirror() {
        assert!(!gi_metrics_enabled(&[], None).unwrap());
        assert!(gi_metrics_enabled(&[], Some("1")).unwrap());
        assert!(gi_metrics_enabled(&[], Some("on")).unwrap());
        assert!(!gi_metrics_enabled(&[], Some("0")).unwrap());
        assert!(!gi_metrics_enabled(&[], Some("off")).unwrap());
        assert!(!gi_metrics_enabled(&[], Some("false")).unwrap());
        assert!(gi_metrics_enabled(&["--gi-metrics"], Some("0")).unwrap());
        assert!(!gi_metrics_enabled(&["--gi-metrics=off"], Some("1")).unwrap());
        assert!(gi_metrics_enabled(&["--gi-metrics=on"], Some("0")).unwrap());
        assert!(gi_metrics_enabled(&["--preset", "high", "--gi-metrics"], None).unwrap());
        assert!(!gi_metrics_enabled(&["--preset", "high"], Some("no")).unwrap());
        assert!(gi_metrics_flag_needs_value("--gi-metrics").is_some_and(|needs| !needs));
        assert!(gi_metrics_flag_needs_value("--gi-metrics=on").is_some_and(|needs| !needs));
        assert!(gi_metrics_flag_needs_value("--preset").is_none());
        assert_eq!(
            check_gi_metrics_arg("--gi-metrics=maybe", None).unwrap_err(),
            "--gi-metrics maybe is not on or off"
        );
        assert_eq!(
            check_gi_metrics_arg("--gi-metrics=", None).unwrap_err(),
            "--gi-metrics needs on or off"
        );
        assert!(check_gi_metrics_arg("--gi-metrics", None).is_ok());
        assert!(check_gi_metrics_arg("--gi-metrics=off", None).is_ok());
    }
}

#[cfg(test)]
mod memory_props {
    use super::{extension_listed, parse_mem_props};

    #[test]
    fn a_device_local_heap_is_counted_and_a_host_heap_is_not() {
        let mut raw = vec![0u8; 520];
        raw[0..4].copy_from_slice(&1u32.to_ne_bytes());
        raw[4..8].copy_from_slice(&1u32.to_ne_bytes());
        raw[260..264].copy_from_slice(&2u32.to_ne_bytes());
        raw[264..272].copy_from_slice(&1000u64.to_ne_bytes());
        raw[272..276].copy_from_slice(&1u32.to_ne_bytes());
        raw[280..288].copy_from_slice(&50u64.to_ne_bytes());
        let props = parse_mem_props(&raw);
        assert_eq!(props.count, 1);
        assert_eq!(props.types[0], 1);
        assert_eq!(props.device_local_bytes, 1000);
        let name = b"VK_EXT_memory_budget";
        let mut listed = vec![0u8; 260];
        listed[..name.len()].copy_from_slice(name);
        assert!(extension_listed(&listed, 1, name));
        assert!(!extension_listed(&listed, 1, b"VK_KHR_swapchain"));
    }
}
