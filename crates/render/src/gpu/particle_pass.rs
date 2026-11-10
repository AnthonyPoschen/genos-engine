//! Stage-1 particle compute and the indirect draw in the keep pass.
//!
//! The compute buffer stays on the device. The CPU reads it only after a
//! readback frame, for the lavapipe agreement test. A frame that does not
//! ask for a picture does not map the positions.

use std::ffi::c_void;

use genos_scene::{look_direction, Camera, ParticleSettings};

use super::{check, Buffer, Gpu, Handle, Image, Memory};
use crate::particle_sim::{
    buffer_bytes, sim_generation, ParticleFault, ParticleStats, COMPACT_STRIDE, PARAMS_BYTES,
    scan_buffer_bytes, SLOT_STRIDE, STATE_BYTES,
};

const INDIRECT_BYTES: u64 = 16;
const DEPTH_FLAG: u32 = 8;
const DEPTH_IMAGE_FLAG: u32 = 16;
const FREEZE_FLAG: u32 = 1;
/// `VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`.
const COMPUTE_STAGE: u32 = 0x800;
/// `VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | LATE_FRAGMENT_TESTS_BIT`.
const DEPTH_STAGE: u32 = 0x100 | 0x200;
/// Depth-stencil attachment read and write.
const DEPTH_ACCESS: u32 = 0x200 | 0x400;
/// Shader read and write.
const SHADER_ACCESS: u32 = 0x20 | 0x40;
/// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
const TRANSFER_STAGE: u32 = 0x1000;
/// `VK_ACCESS_TRANSFER_WRITE_BIT`.
const TRANSFER_WRITE: u32 = 0x1000;
/// Shipping per-flight ring barrier. Compute is in both stage masks and both
/// access masks: the validation witness only orders a fill inside one recording
/// and cannot see the cross-submit dependency.
const RING_SRC_STAGE: u32 = COMPUTE_STAGE | TRANSFER_STAGE;
const RING_DST_STAGE: u32 = COMPUTE_STAGE;
const RING_SRC_ACCESS: u32 = SHADER_ACCESS | TRANSFER_WRITE;
const RING_DST_ACCESS: u32 = SHADER_ACCESS;
/// Threads per sim workgroup. Matches `particles.comp` `local_size_x`.
const SIM_GROUP: u32 = 64;
const MODE_INIT: u32 = 0;
const MODE_INIT_HEADER: u32 = 9;
const MODE_INTEGRATE: u32 = 1;
const MODE_REDUCE_FLAGS: u32 = 2;
const MODE_SCAN_LEVEL: u32 = 3;
const MODE_SCAN_ADD: u32 = 12;
const SCAN_HEADER: u32 = 1;
const SCAN_RECORD: u32 = 2;
const SCAN_INDIRECT: u32 = 4;
const SCAN_NO_SUM: u32 = u32::MAX;

#[derive(Clone, Copy, Default)]
struct ScanPush {
    count: u32,
    src: u32,
    dst: u32,
    sum: u32,
    parent: u32,
    flags: u32,
}
const MODE_APPLY_DEAD: u32 = 4;
const MODE_EMIT_PREP: u32 = 5;
const MODE_EMIT_BODY: u32 = 6;
const MODE_REDUCE_ALIVE: u32 = 7;
const MODE_SCATTER: u32 = 8;

fn sim_groups(budget: u32) -> u32 {
    if budget == 0 {
        0
    } else {
        budget.div_ceil(SIM_GROUP)
    }
}

/// One frame's emitter, camera axes, and the matrix the depth test uses.
#[derive(Clone, Copy, Debug)]
pub(super) struct Job {
    origin: [f32; 3],
    rate: f32,
    life: f32,
    rise: f32,
    spread: f32,
    size: f32,
    albedo: [f32; 3],
    seed: u32,
    budget: u32,
    count: u32,
    freeze: bool,
    pub(super) depth_collision: bool,
    soft_band: f32,
    generation: u32,
    view_proj: [f32; 16],
    right: [f32; 4],
    up: [f32; 4],
}

/// One flight's host inputs and compact output.
///
/// Compact and indirect ping-pong so this flight's draw can overlap the other
/// flight's compute. The persistent ring stays on [`Pass`].
struct Flight {
    params: Buffer,
    depth: Buffer,
    queries: Handle,
    compute_pool: Handle,
    compute_set: Handle,
    compact: Buffer,
    indirect: Buffer,
    /// Alive and spawned, copied at the end of this flight's compute.
    stats: Buffer,
    draw_pool: Handle,
    draw_set: Handle,
}

pub(super) struct Pass {
    budget: u32,
    /// Persistent sim. One copy. The next compute waits the previous one with a barrier.
    state: Buffer,
    slots: Buffer,
    dead: Buffer,
    /// Four bytes filled every dispatch. Its own transfer barrier orders it.
    /// The ring barrier is the compute memory barrier.
    canary: Buffer,
    /// Filled in the same recording, before the read. This layer records a
    /// storage-buffer shader access as a read, so the fill is the write it
    /// can check, and it reports that hazard against this recording.
    touch: Buffer,
    /// Destination of the read, so the load is not dropped.
    sink: Buffer,
    flights: [Flight; 2],
    /// 1x1 depth image kept in shader-read layout when this frame does not sample.
    dummy: Image,
    dummy_ready: bool,
    sampler: Handle,
    compute_desc_layout: Handle,
    compute_layout: Handle,
    compute_pipe: Handle,
    /// Spawn shader. Kept off `compute_pipe` so the wide passes do not carry the jump matrices.
    emit_pipe: Handle,
    read_desc_layout: Handle,
    read_layout: Handle,
    read_pipe: Handle,
    read_pool: Handle,
    read_set: Handle,
    draw_desc_layout: Handle,
    draw_layout: Handle,
    draw_pipe: Handle,
    /// Depth-tested twin of `draw_pipe`, used when the keep pass loaded scene depth.
    draw_pipe_depth: Handle,
    /// Flags, prefix, and block sums for the ordered compact. One copy: compute
    /// does not overlap the other flight's compute.
    scan: Buffer,
    bytes: u64,
}

impl Gpu {
    pub(super) fn note_particles(
        &mut self,
        settings: &ParticleSettings,
        camera: &Camera,
        aspect: f32,
    ) {
        if !settings.dispatches() {
            self.particle_job = None;
            return;
        }
        let forward = look_direction(camera.yaw, camera.pitch);
        let forward = [forward.x, forward.y, forward.z];
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        self.particle_job = Some(Job {
            origin: settings.origin,
            rate: settings.rate,
            life: settings.life,
            rise: settings.rise,
            spread: settings.spread,
            size: settings.size,
            albedo: settings.albedo,
            seed: settings.rng_seed(),
            budget: settings.max_particles,
            count: settings.live_cap(),
            freeze: settings.freeze,
            depth_collision: settings.depth_collision,
            soft_band: settings.soft_band,
            generation: sim_generation(settings.rng_seed(), settings.max_particles),
            view_proj: genos_scene::view_proj(camera, aspect),
            right: [right[0], right[1], right[2], settings.size],
            up: [up[0], up[1], up[2], 0.0],
        });
    }

    pub(super) fn record_particle_compute(&mut self) -> Result<(), String> {
        self.particle_drawn = false;
        let Some(job) = self.particle_job else {
            self.particle_stamped = false;
            self.particle_stats.alive = 0;
            self.particle_stats.spawned = 0;
            self.particle_stats.draw_instances = 0;
            return Ok(());
        };
        let depth_bytes = self.particle_depth_bytes();
        let slot = self.flight.min(1);
        let other = 1 - slot;
        self.ensure_particle_pass(job.budget, depth_bytes)?;
        let sample_image = job.depth_collision
            && self.uploaded_depth_size() == (0, 0)
            && self.particle_depth_retained[other];
        let view = if sample_image {
            self.particle_view_proj[other]
        } else {
            job.view_proj
        };
        self.write_particle_inputs(slot, &job, &view, sample_image)?;
        self.prepare_dummy_depth()?;
        let sampled = if sample_image {
            self.acquire_depth_image(other)?;
            copy_image_handle(&self.depths[other])
        } else {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            copy_image_handle(&pass.dummy)
        };
        let (pipe, emit_pipe, layout, set, queries) = {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            let flight = &pass.flights[slot];
            (
                pass.compute_pipe,
                pass.emit_pipe,
                pass.compute_layout,
                flight.compute_set,
                flight.queries,
            )
        };
        self.write_depth_binding(set, sampled.view)?;
        self.particle_query_ready[slot] = false;
        self.particle_submitted = slot;
        let skip_compute =
            self.particle_fault == crate::particle_sim::ParticleFault::SkipFlightBarrier;
        let (canary, state, stats, touch, read_pipe, read_layout, read_set) = {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            (
                pass.canary.buffer,
                pass.state.buffer,
                pass.flights[slot].stats.buffer,
                pass.touch.buffer,
                pass.read_pipe,
                pass.read_layout,
                pass.read_set,
            )
        };
        // The canary fill keeps this barrier. It is not the ring dependency.
        self.buffer_barrier(canary, 0x1000, 0x1000, 0x1000, 0x1000);
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, canary, 0, 4, 1);
            // This fill is the write synchronization validation can see. The
            // layer records a storage-buffer shader access as a read, and it
            // treats a later queue submit as already ordered with this one, so
            // a cross-submit race never shows up. The read below is in this
            // recording, and only the ring barrier orders the fill to it.
            (self.fns.cmd_fill_buffer)(self.cmd, touch, 0, 4, 1);
            if !skip_compute {
                memory_barrier(
                    self,
                    RING_SRC_STAGE,
                    RING_DST_STAGE,
                    RING_SRC_ACCESS,
                    RING_DST_ACCESS,
                );
            }
            if !queries.is_null() {
                (self.fns.cmd_reset_query)(self.cmd, queries, 0, 8);
            }
            memory_barrier(self, 0x4000, COMPUTE_STAGE, 0x4000, 0x20);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, pipe);
            (self.fns.cmd_bind_set)(self.cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            write_particle_stamp(self, 0, COMPUTE_STAGE);
            self.dispatch_particle_sim(layout, job.budget, job.count, pipe, emit_pipe);
            // The 4-byte witness is not the sim. It stays in this recording,
            // after the ring barrier, so the sync child still sees the fill.
            write_particle_stamp(self, 1, COMPUTE_STAGE);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, read_pipe);
            (self.fns.cmd_bind_set)(
                self.cmd,
                1,
                read_layout,
                0,
                1,
                &read_set,
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
        }
        if sample_image {
            // The sampled image belongs to the other flight. Put it back in
            // depth-attachment layout before that flight's next pass.
            depth_barrier(
                self,
                sampled.image,
                5,
                3,
                COMPUTE_STAGE,
                DEPTH_STAGE,
                0x20,
                DEPTH_ACCESS,
            );
        }
        // Vertex and indirect for this flight's draw. Transfer for the stats
        // copy. The fence, not a host stage here, publishes that copy: a host
        // destination would chain into the next flight's host-to-compute
        // barrier and hide a missing ring barrier.
        memory_barrier(self, COMPUTE_STAGE, 0x8 | 0x2 | 0x1000, 0x40, 0x20 | 0x1 | 0x800);
        // The previous visit of this flight copied the same bytes. The fence
        // orders it; this barrier is the one synchronization validation sees.
        self.buffer_barrier(stats, 0x1000, 0x1000, 0x1000, 0x1000);
        #[repr(C)]
        struct Region {
            src: u64,
            dst: u64,
            size: u64,
        }
        let region = Region {
            src: 16,
            dst: 0,
            size: 8,
        };
        unsafe {
            (self.fns.cmd_copy_buffer)(
                self.cmd,
                state,
                stats,
                1,
                &region as *const Region as *const u8,
            );
        }
        self.particle_stamped = true;
        Ok(())
    }


    /// One thread per slot for integrate and compact. Emit jumps the rng so
    /// the spawn writes are parallel and still match the mirror's order.
    fn dispatch_particle_sim(
        &mut self,
        layout: Handle,
        budget: u32,
        count: u32,
        sim_pipe: Handle,
        emit_pipe: Handle,
    ) {
        let wide = sim_groups(budget).max(1);
        let emit = sim_groups(count.max(1)).max(1);
        self.sim_pass(layout, MODE_INIT, wide, wide);
        self.sim_pass_barrier();
        self.sim_pass(layout, MODE_INIT_HEADER, wide, 1);
        write_particle_stamp(self, 4, COMPUTE_STAGE);
        self.sim_pass_barrier();
        if self.particle_moving() {
            self.sim_pass(layout, MODE_INTEGRATE, wide, wide);
            self.sim_pass_barrier();
            self.sim_pass(layout, MODE_REDUCE_FLAGS, wide, wide);
            self.sim_pass_barrier();
            self.scan_tree(layout, budget, wide, false);
            self.sim_pass_barrier();
            self.sim_pass(layout, MODE_APPLY_DEAD, wide, wide);
            self.sim_pass_barrier();
        }
        write_particle_stamp(self, 5, COMPUTE_STAGE);
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, emit_pipe);
        }
        self.sim_pass(layout, MODE_EMIT_PREP, wide, 1);
        self.sim_pass_barrier();
        self.sim_pass(layout, MODE_EMIT_BODY, wide, emit);
        write_particle_stamp(self, 6, COMPUTE_STAGE);
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, sim_pipe);
        }
        self.sim_pass_barrier();
        self.sim_pass(layout, MODE_REDUCE_ALIVE, wide, wide);
        self.sim_pass_barrier();
        self.scan_tree(layout, budget, wide, true);
        self.sim_pass_barrier();
        self.sim_pass(layout, MODE_SCATTER, wide, wide);
        write_particle_stamp(self, 7, COMPUTE_STAGE);
    }

    fn particle_moving(&self) -> bool {
        let Some(job) = self.particle_job else {
            return false;
        };
        self.particle_dt.is_finite() && self.particle_dt > 0.0 && !job.freeze
    }

    fn sim_pass(&mut self, layout: Handle, mode: u32, wide: u32, dispatch: u32) {
        self.sim_words(layout, mode, wide, dispatch, ScanPush::default());
    }

    /// Block sums are scanned in workgroups of 64, then those totals are
    /// scanned the same way. One thread does not walk the group list.
    fn scan_tree(&mut self, layout: Handle, budget: u32, wide: u32, indirect: bool) {
        let level1 = sim_groups(wide).max(1);
        let level2 = sim_groups(level1).max(1);
        let sum = budget.saturating_mul(2);
        let excl = sum.saturating_add(wide);
        let level1_sum = excl.saturating_add(wide);
        let level1_excl = level1_sum.saturating_add(level1);
        let level2_sum = level1_excl.saturating_add(level1);
        let level2_excl = level2_sum.saturating_add(level2);
        let header = if indirect {
            SCAN_HEADER | SCAN_INDIRECT
        } else {
            SCAN_HEADER
        };
        if level1 <= 1 {
            self.scan_level(
                layout,
                wide,
                1,
                ScanPush {
                    count: wide,
                    src: sum,
                    dst: excl,
                    sum: SCAN_NO_SUM,
                    flags: header | SCAN_RECORD,
                    ..ScanPush::default()
                },
            );
            return;
        }
        self.scan_level(
            layout,
            wide,
            level1,
            ScanPush {
                count: wide,
                src: sum,
                dst: excl,
                sum: level1_sum,
                flags: SCAN_RECORD,
                ..ScanPush::default()
            },
        );
        self.sim_pass_barrier();
        if level2 <= 1 {
            self.scan_level(
                layout,
                wide,
                1,
                ScanPush {
                    count: level1,
                    src: level1_sum,
                    dst: level1_excl,
                    sum: SCAN_NO_SUM,
                    flags: header,
                    ..ScanPush::default()
                },
            );
            self.sim_pass_barrier();
        } else {
            self.scan_level(
                layout,
                wide,
                level2,
                ScanPush {
                    count: level1,
                    src: level1_sum,
                    dst: level1_excl,
                    sum: level2_sum,
                    ..ScanPush::default()
                },
            );
            self.sim_pass_barrier();
            self.scan_level(
                layout,
                wide,
                1,
                ScanPush {
                    count: level2,
                    src: level2_sum,
                    dst: level2_excl,
                    sum: SCAN_NO_SUM,
                    flags: header,
                    ..ScanPush::default()
                },
            );
            self.sim_pass_barrier();
            self.scan_add(layout, wide, level1, level1_excl, level2_excl);
            self.sim_pass_barrier();
        }
        self.scan_add(layout, wide, wide, excl, level1_excl);
    }

    fn scan_level(&mut self, layout: Handle, wide: u32, dispatch: u32, push: ScanPush) {
        self.sim_words(layout, MODE_SCAN_LEVEL, wide, dispatch, push);
    }

    fn scan_add(&mut self, layout: Handle, wide: u32, count: u32, dst: u32, parent: u32) {
        self.sim_words(
            layout,
            MODE_SCAN_ADD,
            wide,
            sim_groups(count).max(1),
            ScanPush {
                count,
                dst,
                parent,
                ..ScanPush::default()
            },
        );
    }

    fn sim_words(&mut self, layout: Handle, mode: u32, wide: u32, dispatch: u32, push: ScanPush) {
        let words = [
            mode,
            wide,
            push.count,
            push.src,
            push.dst,
            push.sum,
            push.parent,
            push.flags,
        ];
        unsafe {
            (self.fns.cmd_push)(
                self.cmd,
                layout,
                0x20,
                0,
                32,
                words.as_ptr().cast(),
            );
            (self.fns.cmd_dispatch)(self.cmd, dispatch.max(1), 1, 1);
        }
    }

    fn sim_pass_barrier(&mut self) {
        memory_barrier(
            self,
            COMPUTE_STAGE,
            COMPUTE_STAGE,
            SHADER_ACCESS,
            SHADER_ACCESS,
        );
    }

    pub(super) fn record_particle_draw(&mut self) {
        if !self.particle_stamped || self.particle_drawn || self.keep_pass.is_null() {
            return;
        }
        let (pipe, layout, set, indirect) = {
            let Some(pass) = self.particle_pass.as_ref() else {
                return;
            };
            let flight = &pass.flights[self.flight.min(1)];
            (
                if self.store_scene_depth && !pass.draw_pipe_depth.is_null() {
                    pass.draw_pipe_depth
                } else {
                    pass.draw_pipe
                },
                pass.draw_layout,
                flight.draw_set,
                flight.indirect.buffer,
            )
        };
        let Some(job) = self.particle_job else {
            return;
        };
        self.particle_drawn = true;
        self.prepare_kept_depth();
        let width = self.extent_w;
        let height = self.extent_h;
        #[repr(C)]
        struct Offset {
            x: i32,
            y: i32,
        }
        #[repr(C)]
        struct Extent {
            w: u32,
            h: u32,
        }
        #[repr(C)]
        struct Rect {
            offset: Offset,
            extent: Extent,
        }
        #[repr(C)]
        struct RpBegin {
            s_type: i32,
            next: *const c_void,
            pass: Handle,
            fb: Handle,
            area: Rect,
            clear_count: u32,
            clears: *const f32,
        }
        let rp = RpBegin {
            s_type: 43,
            next: std::ptr::null(),
            pass: if self.store_scene_depth && !self.keep_pass_store.is_null() {
                self.keep_pass_store
            } else {
                self.keep_pass
            },
            fb: self.framebuffer,
            area: Rect {
                offset: Offset { x: 0, y: 0 },
                extent: Extent {
                    w: width,
                    h: height,
                },
            },
            clear_count: 0,
            clears: std::ptr::null(),
        };
        let mut push = [0u8; 96];
        write_f32(&mut push, 0, &job.view_proj);
        write_f32(&mut push, 64, &job.right);
        write_f32(&mut push, 80, &job.up);
        unsafe {
            (self.fns.cmd_begin_rp)(self.cmd, &rp as *const RpBegin as *const u8, 0);
            (self.fns.cmd_bind_pipe)(self.cmd, 0, pipe);
            (self.fns.cmd_bind_set)(self.cmd, 0, layout, 0, 1, &set, 0, std::ptr::null());
            let viewport = [0.0f32, 0.0, width as f32, height as f32, 0.0, 1.0];
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
            let scissor = [0i32, 0, extent_i32(width), extent_i32(height)];
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
            (self.fns.cmd_push)(
                self.cmd,
                layout,
                1,
                0,
                96,
                push.as_ptr() as *const c_void,
            );
            // Bracket the indirect draw. The render-pass external dependency
            // is outside this span, so draw_ms is the draw itself.
            write_particle_stamp(self, 2, 0x8);
            (self.fns.cmd_draw_indirect)(self.cmd, indirect, 0, 1, 16);
            write_particle_stamp(self, 3, 0x400);
            (self.fns.cmd_end_rp)(self.cmd);
        }
    }

    pub(super) fn collect_particle_stats(&mut self) {
        let slot = self.flight.min(1);
        // `flight` is the slot about to be recorded. Its queries are the previous
        // visit, which this function's caller has already fenced. The other slot
        // may still be on the GPU.
        if !self.particle_query_ready[slot] {
            return;
        }
        self.collect_particle_stats_slot(slot);
    }

    pub(super) fn collect_particle_stats_slot(&mut self, slot: usize) {
        let Some(pass) = self.particle_pass.as_ref() else {
            return;
        };
        let slot = slot.min(1);
        let queries = pass.flights[slot].queries;
        let stats = copy_buffer_handle(&pass.flights[slot].stats);
        let indirect = copy_buffer_handle(&pass.flights[slot].indirect);
        let bytes = self.accounted_particle_bytes();
        let budget = pass.budget;
        let count = self.particle_job.map_or(0, |job| job.count);
        let sim_ms = self.particle_span_ms(queries, 0, 1);
        let draw_ms = self.particle_span_ms(queries, 2, 3);
        let sim_init_ms = self.particle_query_ms(queries, 0, 4);
        let sim_death_ms = self.particle_query_ms(queries, 4, 5);
        let sim_emit_ms = self.particle_query_ms(queries, 5, 6);
        let sim_compact_ms = self.particle_query_ms(queries, 6, 7);
        // The shared ring may still be written by the other flight. These two
        // words were copied at the end of this flight's compute.
        let alive = read_u32_at(self, &stats, 0).unwrap_or(0);
        let spawned = read_u32_at(self, &stats, 4).unwrap_or(0);
        let draw_instances = read_u32_at(self, &indirect, 4).unwrap_or(0);
        self.particle_stats = ParticleStats {
            sim_ms,
            sim_init_ms,
            sim_death_ms,
            sim_emit_ms,
            sim_compact_ms,
            sort_ms: 0.0,
            draw_ms,
            alive,
            spawned,
            overdraw_px: 0,
            vram_bytes: bytes,
            vram_mb: bytes as f64 / 1_000_000.0,
            max_particles: budget,
            count,
            draw_instances,
        };
    }

    /// `(workgroup size, group count)` the wide sim dispatch wrote into state.
    pub(super) fn particle_sim_launch(&self) -> Result<(u32, u32), String> {
        let Some(pass) = self.particle_pass.as_ref() else {
            return Err("particle pass missing".into());
        };
        let state = copy_buffer_handle(&pass.state);
        let size = read_u32_at(self, &state, 56)?;
        let groups = read_u32_at(self, &state, 60)?;
        Ok((size, groups))
    }

    /// `(workgroup size, group count)` of the block-sum scan.
    pub(super) fn particle_scan_launch(&self) -> Result<(u32, u32), String> {
        let Some(pass) = self.particle_pass.as_ref() else {
            return Err("particle pass missing".into());
        };
        let state = copy_buffer_handle(&pass.state);
        let size = read_u32_at(self, &state, 64)?;
        let groups = read_u32_at(self, &state, 68)?;
        Ok((size, groups))
    }

    /// Milliseconds for init, death scan, emit, and alive compact.
    ///
    /// Lavapipe timestamps. They are not a desktop-GPU score. The outer
    /// `sim_ms` is still queries 0 and 1, and it does not include the witness read.
    pub(super) fn particle_sim_parts(&self) -> [f64; 4] {
        let Some(pass) = self.particle_pass.as_ref() else {
            return [0.0; 4];
        };
        let pool = pass.flights[self.particle_submitted.min(1)].queries;
        [
            self.particle_query_ms(pool, 0, 4),
            self.particle_query_ms(pool, 4, 5),
            self.particle_query_ms(pool, 5, 6),
            self.particle_query_ms(pool, 6, 7),
        ]
    }

    fn particle_query_ms(&self, pool: Handle, start: u32, end: u32) -> f64 {
        if pool.is_null() || self.timestamp_period <= 0.0 || end <= start {
            return 0.0;
        }
        let Some(first) = read_timestamp(self, pool, start) else {
            return 0.0;
        };
        let Some(second) = read_timestamp(self, pool, end) else {
            return 0.0;
        };
        super::tick_delta(first, second, self.timestamp_bits) as f64
            * f64::from(self.timestamp_period)
            / 1.0e6
    }

    pub(super) fn particle_positions(&self) -> Result<Vec<[f32; 3]>, String> {
        let Some(pass) = self.particle_pass.as_ref() else {
            return Ok(Vec::new());
        };
        let budget = pass.budget;
        let state = copy_buffer_handle(&pass.state);
        let compact = copy_buffer_handle(&pass.flights[self.particle_submitted.min(1)].compact);
        let alive = read_u32_at(self, &state, 16).unwrap_or(0).min(budget);
        let mut out = Vec::with_capacity(usize::try_from(alive).unwrap_or(0));
        for index in 0..alive {
            let offset = u64::from(index) * COMPACT_STRIDE;
            let bytes = read_range(self, &compact, offset, 12)?;
            let mut position = [0.0; 3];
            for axis in 0..3 {
                let start = axis * 4;
                position[axis] = f32::from_ne_bytes(bytes[start..start + 4].try_into().unwrap_or([0; 4]));
            }
            out.push(position);
        }
        Ok(out)
    }

    pub(super) fn destroy_particle_pass(&mut self) {
        let Some(pass) = self.particle_pass.take() else {
            return;
        };
        unsafe {
            if !pass.emit_pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, pass.emit_pipe, std::ptr::null());
            }
        }
        destroy_pipe(
            self,
            pass.flights[0].queries,
            pass.compute_pipe,
            pass.compute_layout,
            pass.flights[0].compute_pool,
            pass.compute_desc_layout,
        );
        destroy_pipe(
            self,
            pass.flights[1].queries,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            pass.flights[1].compute_pool,
            std::ptr::null_mut(),
        );
        destroy_pipe(
            self,
            std::ptr::null_mut(),
            pass.draw_pipe,
            pass.draw_layout,
            pass.flights[0].draw_pool,
            pass.draw_desc_layout,
        );
        if !pass.flights[1].draw_pool.is_null() {
            unsafe {
                (self.fns.destroy_desc_pool)(self.device, pass.flights[1].draw_pool, std::ptr::null());
            }
        }
        if !pass.draw_pipe_depth.is_null() {
            unsafe {
                (self.fns.destroy_pipeline)(self.device, pass.draw_pipe_depth, std::ptr::null());
            }
        }
        destroy_pipe(
            self,
            std::ptr::null_mut(),
            pass.read_pipe,
            pass.read_layout,
            pass.read_pool,
            pass.read_desc_layout,
        );
        let [flight0, flight1] = pass.flights;
        if !pass.sampler.is_null() {
            unsafe {
                (self.fns.destroy_sampler)(self.device, pass.sampler, std::ptr::null());
            }
        }
        let mut buffers = [
            pass.state,
            pass.slots,
            pass.dead,
            pass.canary,
            pass.touch,
            pass.sink,
            pass.scan,
            flight0.stats,
            flight1.stats,
            flight0.params,
            flight0.depth,
            flight0.compact,
            flight0.indirect,
            flight1.params,
            flight1.depth,
            flight1.compact,
            flight1.indirect,
        ];
        for buffer in &mut buffers {
            self.destroy_buffer(buffer);
        }
        let mut dummy = pass.dummy;
        self.destroy_image(&mut dummy);
    }

    fn particle_depth_bytes(&self) -> u64 {
        let (width, height) = self.particle_depth_size;
        let texels = u64::from(width).saturating_mul(u64::from(height));
        let have = u64::try_from(self.particle_depth.len()).unwrap_or(0);
        if width == 0 || height == 0 || have < texels {
            4
        } else {
            texels.saturating_mul(4).max(4)
        }
    }

    fn wait_live_flights(&mut self) -> Result<(), String> {
        for slot in 0..2 {
            if self.flight_live[slot] {
                self.wait_flight(slot)?;
                self.flight_live[slot] = false;
            }
        }
        Ok(())
    }

    fn ensure_particle_pass(&mut self, budget: u32, depth_bytes: u64) -> Result<(), String> {
        let recreate = self
            .particle_pass
            .as_ref()
            .is_none_or(|pass| pass.budget != budget);
        if recreate {
            // The slot being recorded already had its fence reset. Waiting it
            // here never returns. Only a flight still on the GPU needs a wait.
            self.wait_live_flights()?;
            self.destroy_particle_pass();
            self.particle_pass = Some(self.create_particle_pass(budget, depth_bytes)?);
            return Ok(());
        }
        let too_small = self.particle_pass.as_ref().is_some_and(|pass| {
            pass.flights.iter().any(|flight| flight.depth.size < depth_bytes)
        });
        if too_small {
            self.wait_live_flights()?;
            self.grow_particle_depth(depth_bytes)?;
        }
        Ok(())
    }

    fn write_particle_inputs(
        &mut self,
        slot: usize,
        job: &Job,
        view_proj: &[f32; 16],
        sample_image: bool,
    ) -> Result<(), String> {
        let mut flags = self.particle_fault.bits();
        if job.freeze {
            flags |= FREEZE_FLAG;
        }
        if job.depth_collision {
            flags |= DEPTH_FLAG;
        }
        if sample_image {
            flags |= DEPTH_IMAGE_FLAG;
        }
        let (depth_w, depth_h) = if sample_image {
            (0, 0)
        } else {
            self.uploaded_depth_size()
        };
        let params = pack_params(job, view_proj, self.particle_dt, flags, depth_w, depth_h);
        let depth = self.particle_depth.clone();
        let (params_buf, depth_buf) = {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            let flight = &pass.flights[slot.min(1)];
            (
                copy_buffer_handle(&flight.params),
                copy_buffer_handle(&flight.depth),
            )
        };
        self.write_buffer(&params_buf, &params)?;
        if depth_w > 0 {
            let bytes = pod_f32(&depth);
            let count = usize::try_from(u64::from(depth_w) * u64::from(depth_h) * 4).unwrap_or(0);
            let count = count.min(bytes.len());
            self.write_buffer(&depth_buf, &bytes[..count])?;
        }
        Ok(())
    }

    fn uploaded_depth_size(&self) -> (u32, u32) {
        let (width, height) = self.particle_depth_size;
        let texels = u64::from(width).saturating_mul(u64::from(height));
        let have = u64::try_from(self.particle_depth.len()).unwrap_or(0);
        if width == 0 || height == 0 || have < texels {
            (0, 0)
        } else {
            (width, height)
        }
    }

    fn create_particle_pass(&mut self, budget: u32, depth_bytes: u64) -> Result<Pass, String> {
        let slots = u64::from(budget);
        let state = self.make_buffer(STATE_BYTES, 0x20 | 0x1, Memory::Readback)?;
        zero_host_buffer(self, &state)?;
        let slot_buf = self.make_buffer(slots * SLOT_STRIDE, 0x20, Memory::Readback)?;
        let dead = self.make_buffer(slots * 4, 0x20, Memory::Readback)?;
        let canary = self.make_buffer(4, 0x2, Memory::Device)?;
        let touch = self.make_buffer(4, 0x20 | 0x2, Memory::Device)?;
        let sink = self.make_buffer(4, 0x20, Memory::Device)?;
        let scan = self.make_buffer(scan_buffer_bytes(budget), 0x20, Memory::Device)?;
        self.write_buffer(&state, &zero_bytes(STATE_BYTES)?)?;
        let depth_bytes = depth_bytes.max(4);
        let mut built = Vec::with_capacity(2);
        for _ in 0..2 {
            let params = self.make_buffer(PARAMS_BYTES, 0x20, Memory::Upload)?;
            let depth = self.make_buffer(depth_bytes, 0x20, Memory::Upload)?;
            let compact = self.make_buffer(slots * COMPACT_STRIDE, 0x20, Memory::Readback)?;
            let indirect = self.make_buffer(INDIRECT_BYTES, 0x20 | 0x100, Memory::Readback)?;
            let stats = self.make_buffer(8, 0x2, Memory::Readback)?;
            self.write_buffer(&indirect, &zero_bytes(INDIRECT_BYTES)?)?;
            self.write_buffer(&stats, &zero_bytes(8)?)?;
            let queries = if self.timestamp_period > 0.0 {
                self.make_query_pool_n(8).unwrap_or(std::ptr::null_mut())
            } else {
                std::ptr::null_mut()
            };
            built.push((params, depth, compact, indirect, stats, queries));
        }
        let (params0, depth0, compact0, indirect0, stats0, queries0) = built.remove(0);
        let (params1, depth1, compact1, indirect1, stats1, queries1) = built.remove(0);
        let sampler = self.make_sampler()?;
        let dummy = self.make_image(126, 0x20 | 0x4, 2, 1, 1)?;
        let compute = self.particle_compute_pipeline(
            &[
                params0.buffer,
                state.buffer,
                slot_buf.buffer,
                dead.buffer,
                compact0.buffer,
                indirect0.buffer,
                depth0.buffer,
            ],
            scan.buffer,
        )?;
        let (pool1, set1) = alloc_compute_set(self, compute.0)?;
        write_storage_bindings(
            self,
            set1,
            &[
                params1.buffer,
                state.buffer,
                slot_buf.buffer,
                dead.buffer,
                compact1.buffer,
                indirect1.buffer,
                depth1.buffer,
            ],
        )?;
        write_storage_bindings_from(self, set1, 8, &[scan.buffer])?;
        write_depth_image(self, set1, dummy.view, sampler);
        write_depth_image(self, compute.2, dummy.view, sampler);
        let emit_pipe = self.particle_emit_pipeline(compute.3)?;
        let read_pipe = self.particle_touch_read_pipeline(touch.buffer, sink.buffer)?;
        let draw = self.particle_draw_pipeline(compact0.buffer)?;
        let (draw_pool1, draw_set1) = alloc_set(self, draw.0, 1)?;
        write_storage_bindings(self, draw_set1, &[compact1.buffer])?;
        let bytes = buffer_bytes(budget)
            .saturating_add(PARAMS_BYTES)
            .saturating_add(u64::from(budget) * COMPACT_STRIDE)
            .saturating_add(INDIRECT_BYTES)
            .saturating_add(depth0.size)
            .saturating_add(depth1.size)
            .saturating_add(4)
            .saturating_add(touch.size)
            .saturating_add(sink.size)
            .saturating_add(stats0.size)
            .saturating_add(stats1.size);
        Ok(Pass {
            budget,
            state,
            slots: slot_buf,
            dead,
            canary,
            touch,
            sink,
            flights: [
                Flight {
                    params: params0,
                    depth: depth0,
                    queries: queries0,
                    compute_pool: compute.1,
                    compute_set: compute.2,
                    compact: compact0,
                    indirect: indirect0,
                    stats: stats0,
                    draw_pool: draw.1,
                    draw_set: draw.2,
                },
                Flight {
                    params: params1,
                    depth: depth1,
                    queries: queries1,
                    compute_pool: pool1,
                    compute_set: set1,
                    compact: compact1,
                    indirect: indirect1,
                    stats: stats1,
                    draw_pool: draw_pool1,
                    draw_set: draw_set1,
                },
            ],
            dummy,
            dummy_ready: false,
            sampler,
            compute_desc_layout: compute.0,
            compute_layout: compute.3,
            compute_pipe: compute.4,
            emit_pipe,
            read_desc_layout: read_pipe.0,
            read_layout: read_pipe.3,
            read_pipe: read_pipe.4,
            read_pool: read_pipe.1,
            read_set: read_pipe.2,
            draw_desc_layout: draw.0,
            draw_layout: draw.3,
            draw_pipe: draw.4,
            draw_pipe_depth: draw.5,
            scan,
            bytes,
        })
    }

    fn grow_particle_depth(&mut self, depth_bytes: u64) -> Result<(), String> {
        for slot in 0..2 {
            let mut depth = self.make_buffer(depth_bytes, 0x20, Memory::Upload)?;
            let (set, handles) = {
                let Some(pass) = self.particle_pass.as_mut() else {
                    self.destroy_buffer(&mut depth);
                    return Err("particle pass missing".into());
                };
                std::mem::swap(&mut pass.flights[slot].depth, &mut depth);
                pass.bytes = flight_buffer_bytes(pass);
                let set = pass.flights[slot].compute_set;
                let handles = [
                    pass.flights[slot].params.buffer,
                    pass.state.buffer,
                    pass.slots.buffer,
                    pass.dead.buffer,
                    pass.flights[slot].compact.buffer,
                    pass.flights[slot].indirect.buffer,
                    pass.flights[slot].depth.buffer,
                ];
                (set, handles)
            };
            self.destroy_buffer(&mut depth);
            write_storage_bindings(self, set, &handles)?;
        }
        Ok(())
    }

    /// Wait both flights, then read the sim buffers. A frame without color readback
    /// still has to finish before the CPU mirror is compared.
    pub(super) fn finish_particles(&mut self) -> Result<(), String> {
        for slot in 0..2 {
            if self.flight_live[slot] {
                self.wait_flight(slot)?;
                self.flight_live[slot] = false;
            }
        }
        for slot in 0..2 {
            if self.particle_stamps_pending[slot] {
                self.particle_query_ready[slot] = true;
                self.particle_stamps_pending[slot] = false;
            }
        }
        let submitted = self.particle_submitted.min(1);
        if self.particle_query_ready[submitted] {
            self.collect_particle_stats_slot(submitted);
        }
        Ok(())
    }

    /// Pipelines, layouts, descriptor sets, and buffers this pass owns.
    pub(super) fn particle_validation_handles(&self) -> Vec<u64> {
        let Some(pass) = self.particle_pass.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        push_handle(&mut out, pass.state.buffer);
        push_handle(&mut out, pass.slots.buffer);
        push_handle(&mut out, pass.dead.buffer);
        push_handle(&mut out, pass.canary.buffer);
        push_handle(&mut out, pass.touch.buffer);
        push_handle(&mut out, pass.sink.buffer);
        push_handle(&mut out, pass.scan.buffer);
        push_handle(&mut out, pass.draw_pipe_depth);
        push_handle(&mut out, pass.dummy.image);
        push_handle(&mut out, pass.compute_desc_layout);
        push_handle(&mut out, pass.compute_layout);
        push_handle(&mut out, pass.compute_pipe);
        push_handle(&mut out, pass.emit_pipe);
        push_handle(&mut out, pass.read_desc_layout);
        push_handle(&mut out, pass.read_layout);
        push_handle(&mut out, pass.read_pipe);
        push_handle(&mut out, pass.read_set);
        push_handle(&mut out, pass.draw_desc_layout);
        push_handle(&mut out, pass.draw_layout);
        push_handle(&mut out, pass.draw_pipe);
        for flight in &pass.flights {
            push_handle(&mut out, flight.params.buffer);
            push_handle(&mut out, flight.depth.buffer);
            push_handle(&mut out, flight.compact.buffer);
            push_handle(&mut out, flight.indirect.buffer);
            push_handle(&mut out, flight.stats.buffer);
            push_handle(&mut out, flight.compute_set);
            push_handle(&mut out, flight.draw_set);
        }
        out
    }

    /// Bytes of the stage-1 buffers plus the scene depth retained for collision.
    fn accounted_particle_bytes(&self) -> u64 {
        let Some(pass) = self.particle_pass.as_ref() else {
            return 0;
        };
        let mut bytes = pass.bytes;
        if self.store_scene_depth || self.particle_depth_retained.iter().any(|kept| *kept) {
            let image = u64::from(self.extent_w).saturating_mul(u64::from(self.extent_h)) * 4;
            bytes = bytes.saturating_add(image.saturating_mul(2));
        }
        bytes
    }

    fn prepare_dummy_depth(&mut self) -> Result<(), String> {
        let ready = self
            .particle_pass
            .as_ref()
            .is_some_and(|pass| pass.dummy_ready);
        if ready {
            return Ok(());
        }
        let image = self
            .particle_pass
            .as_ref()
            .map(|pass| pass.dummy.image)
            .ok_or("particle pass missing")?;
        depth_barrier(self, image, 0, 5, 0x1, COMPUTE_STAGE, 0, 0x20);
        if let Some(pass) = self.particle_pass.as_mut() {
            pass.dummy_ready = true;
        }
        Ok(())
    }

    fn write_depth_binding(&self, set: Handle, view: Handle) -> Result<(), String> {
        let sampler = self
            .particle_pass
            .as_ref()
            .map(|pass| pass.sampler)
            .ok_or("particle pass missing")?;
        write_depth_image(self, set, view, sampler);
        Ok(())
    }

    /// The other flight's depth image, in shader-read layout.
    ///
    /// The image was stored by that flight's scene pass. This barrier's source
    /// scope is that earlier submission on the same queue.
    fn acquire_depth_image(&mut self, slot: usize) -> Result<(), String> {
        let image = self.depths[slot.min(1)].image;
        depth_barrier(
            self,
            image,
            3,
            5,
            DEPTH_STAGE,
            COMPUTE_STAGE,
            DEPTH_ACCESS,
            0x20,
        );
        Ok(())
    }

    /// The scene pass stored this flight's depth. The next flight samples it.
    pub(super) fn signal_particle_depth(&mut self, slot: usize) {
        if !self.store_scene_depth {
            return;
        }
        self.particle_depth_retained[slot.min(1)] = true;
    }

    /// One layout-preserving barrier so a keep pass can load the stored depth.
    pub(super) fn prepare_kept_depth(&self) {
        if !self.store_scene_depth || self.depth.image.is_null() {
            return;
        }
        depth_barrier(
            self,
            self.depth.image,
            3,
            3,
            0x200 | 0x1000,
            0x100,
            0x400 | 0x1000,
            0x200,
        );
    }

    fn particle_span_ms(&self, pool: Handle, start: u32, end: u32) -> f64 {
        if pool.is_null() || self.timestamp_period <= 0.0 {
            return 0.0;
        }
        let mut raw = [0u64; 2];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                pool,
                start,
                2,
                16,
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1,
            )
        };
        if got != 0 || end != start + 1 {
            return 0.0;
        }
        super::tick_delta(raw[0], raw[1], self.timestamp_bits) as f64
            * f64::from(self.timestamp_period)
            / 1.0e6
    }

    /// Compute pipeline: descriptor layout, pool, set, pipeline layout, pipeline.
    fn particle_compute_pipeline(
        &mut self,
        buffers: &[Handle; 7],
        scan: Handle,
    ) -> Result<(Handle, Handle, Handle, Handle, Handle), String> {
        let mut bindings = storage_bindings(7, 0x20);
        bindings.push(Binding {
            binding: 7,
            kind: 1,
            count: 1,
            stages: 0x20,
            samplers: std::ptr::null(),
        });
        bindings.push(storage_binding(8, 0x20));
        let desc_layout = create_desc_layout(self, &bindings)?;
        let (pool, set) = alloc_compute_set(self, desc_layout)?;
        write_storage_bindings(self, set, buffers)?;
        write_storage_bindings_from(self, set, 8, &[scan])?;
        let pipe_layout = create_pipe_layout(self, desc_layout, 32, 0x20)?;
        let module = self.shader(super::PARTICLES_COMP_SPV)?;
        let pipe = create_compute_pipe(self, module, pipe_layout)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok((desc_layout, pool, set, pipe_layout, pipe))
    }

    fn particle_emit_pipeline(&mut self, layout: Handle) -> Result<Handle, String> {
        let module = self.shader(super::PARTICLES_EMIT_SPV)?;
        let pipe = create_compute_pipe(self, module, layout)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok(pipe)
    }

    fn particle_touch_read_pipeline(
        &mut self,
        touch: Handle,
        sink: Handle,
    ) -> Result<(Handle, Handle, Handle, Handle, Handle), String> {
        let bindings = [storage_binding(0, 0x20), storage_binding(1, 0x20)];
        let desc_layout = create_desc_layout(self, &bindings)?;
        let (pool, set) = alloc_set(self, desc_layout, 2)?;
        write_storage_bindings(self, set, &[touch, sink])?;
        let pipe_layout = create_pipe_layout(self, desc_layout, 0, 0)?;
        let module = self.shader(super::PARTICLES_TOUCH_READ_SPV)?;
        let pipe = create_compute_pipe(self, module, pipe_layout)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok((desc_layout, pool, set, pipe_layout, pipe))
    }

    fn particle_draw_pipeline(
        &mut self,
        compact: Handle,
    ) -> Result<(Handle, Handle, Handle, Handle, Handle, Handle), String> {
        let bindings = [storage_binding(0, 0x1)];
        let desc_layout = create_desc_layout(self, &bindings)?;
        let (pool, set) = alloc_set(self, desc_layout, 1)?;
        write_storage_bindings(self, set, &[compact])?;
        let pipe_layout = create_pipe_layout(self, desc_layout, 96, 1)?;
        let vert = self.shader(super::PARTICLES_VERT_SPV)?;
        let frag = self.shader(super::PARTICLES_FRAG_SPV)?;
        let pipe = create_draw_pipe(self, vert, frag, pipe_layout, false)?;
        let pipe_depth = create_draw_pipe(self, vert, frag, pipe_layout, true)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, vert, std::ptr::null());
            (self.fns.destroy_shader)(self.device, frag, std::ptr::null());
        }
        Ok((desc_layout, pool, set, pipe_layout, pipe, pipe_depth))
    }
}

impl super::Renderer {
    /// Seconds the next particle step integrates. Zero keeps the particles still.
    pub fn set_particle_dt(&mut self, dt: f32) {
        self.gpu.particle_dt = dt;
    }

    /// A stage-1 mutation. [`ParticleFault::None`] is the specified step.
    pub fn set_particle_fault(&mut self, fault: ParticleFault) {
        self.gpu.particle_fault = fault;
    }

    /// Depth samples the next step tests. `width` or `height` of zero disables it.
    ///
    /// This is the test override. A frame with `depth_collision` and no upload
    /// samples the previous frame's scene depth image instead.
    pub fn set_particle_depth(&mut self, width: u32, height: u32, samples: &[f32]) {
        self.gpu.particle_depth_size = (width, height);
        self.gpu.particle_depth.clear();
        self.gpu.particle_depth.extend_from_slice(samples);
    }

    /// Skip the lighting passes.
    ///
    /// The lighting header barrier orders compute by itself. The flight-barrier
    /// test turns lighting off so that barrier is not the one under test.
    pub fn suspend_lighting(&mut self) {
        self.gpu.lighting.ready = false;
    }

    /// Wait the other flight on the CPU before recording particles.
    ///
    /// The shipping path leaves this off. The lavapipe frame-time note turns it
    /// on to time the old fence.
    pub fn set_particle_cpu_fence(&mut self, on: bool) {
        self.gpu.particle_cpu_fence = on;
    }

    /// Last completed particle step. Times are 0 before a frame, or when the
    /// device has no timestamp queries. They are lavapipe timestamps, not a
    /// score against a desktop GPU.
    #[must_use]
    pub fn particle_stats(&self) -> ParticleStats {
        self.gpu.particle_stats
    }

    /// Wait both frames in flight so a later position read is not racing a submit.
    ///
    /// # Errors
    ///
    /// Returns an error when a fence wait fails.
    pub fn finish_particles(&mut self) -> Result<(), String> {
        self.gpu.finish_particles()
    }

    /// Pipelines, layouts, sets, and buffers the particle pass owns.
    ///
    /// The reflect sync child prints these so a validation line can be tied to
    /// a particle object. Handles are the values Vulkan returns.
    #[must_use]
    pub fn particle_validation_handles(&self) -> Vec<u64> {
        self.gpu.particle_validation_handles()
    }

    /// Workgroup size and group count of the sim dispatch the shader ran.
    ///
    /// # Errors
    ///
    /// Returns an error when the state buffer cannot be mapped.
    pub fn particle_sim_launch(&self) -> Result<(u32, u32), String> {
        self.gpu.particle_sim_launch()
    }

    /// Block-sum scan launch: workgroup size and group count.
    ///
    /// # Errors
    ///
    /// Returns an error when the state buffer cannot be mapped.
    pub fn particle_scan_launch(&self) -> Result<(u32, u32), String> {
        self.gpu.particle_scan_launch()
    }

    /// Init, death, emit, and compact spans inside `sim_ms`, in milliseconds.
    #[must_use]
    pub fn particle_sim_parts(&self) -> [f64; 4] {
        self.gpu.particle_sim_parts()
    }

    /// Compacted positions from the last waited frame, metres.
    ///
    /// # Errors
    ///
    /// Returns an error when the buffer cannot be mapped.
    pub fn particle_positions(&self) -> Result<Vec<[f32; 3]>, String> {
        self.gpu.particle_positions()
    }
}

fn pack_params(
    job: &Job,
    view_proj: &[f32; 16],
    dt: f32,
    flags: u32,
    depth_w: u32,
    depth_h: u32,
) -> [u8; 256] {
    let mut bytes = [0u8; 256];
    write_f32(&mut bytes, 0, &[job.origin[0], job.origin[1], job.origin[2], job.rate]);
    write_f32(&mut bytes, 16, &[job.rise, job.life, job.spread, job.size]);
    write_f32(&mut bytes, 32, &[job.albedo[0], job.albedo[1], job.albedo[2], dt]);
    write_u32(&mut bytes, 48, &[job.seed, job.budget, job.count, flags]);
    write_u32(&mut bytes, 64, &[job.generation, depth_w, depth_h, 0]);
    write_f32(&mut bytes, 80, &[job.soft_band, 0.0, 0.0, 0.0]);
    write_f32(&mut bytes, 96, view_proj);
    bytes
}

fn write_f32(bytes: &mut [u8], offset: usize, values: &[f32]) {
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * 4;
        if let Some(slot) = bytes.get_mut(start..start + 4) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
    }
}

fn write_u32(bytes: &mut [u8], offset: usize, values: &[u32]) {
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * 4;
        if let Some(slot) = bytes.get_mut(start..start + 4) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
    }
}

fn pod_f32(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1.0e-6);
    [v[0] / len, v[1] / len, v[2] / len]
}

fn flight_buffer_bytes(pass: &Pass) -> u64 {
    buffer_bytes(pass.budget)
        .saturating_add(PARAMS_BYTES)
        .saturating_add(u64::from(pass.budget) * COMPACT_STRIDE)
        .saturating_add(INDIRECT_BYTES)
        .saturating_add(pass.flights[0].depth.size)
        .saturating_add(pass.flights[1].depth.size)
        .saturating_add(pass.flights[0].stats.size)
        .saturating_add(pass.flights[1].stats.size)
        .saturating_add(pass.touch.size)
        .saturating_add(pass.sink.size)
        .saturating_add(4)
}

fn depth_barrier(
    gpu: &Gpu,
    image: Handle,
    old: u32,
    new: u32,
    src_stage: u32,
    dst_stage: u32,
    src_access: u32,
    dst_access: u32,
) {
    if image.is_null() {
        return;
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
            aspect: 2,
            mip: 0,
            levels: 1,
            layer: 0,
            layers: 1,
        },
    };
    unsafe {
        (gpu.fns.cmd_barrier)(
            gpu.cmd,
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

fn copy_image_handle(image: &Image) -> Image {
    Image {
        image: image.image,
        view: image.view,
        memory: image.memory,
    }
}

fn memory_barrier(gpu: &Gpu, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
    #[repr(C)]
    struct MemBar {
        s_type: i32,
        next: *const c_void,
        src_access: u32,
        dst_access: u32,
    }
    let bar = MemBar {
        s_type: 46,
        next: std::ptr::null(),
        src_access,
        dst_access,
    };
    unsafe {
        (gpu.fns.cmd_barrier)(
            gpu.cmd,
            src_stage,
            dst_stage,
            0,
            1,
            &bar as *const MemBar as *const c_void,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
}

fn write_particle_stamp(gpu: &Gpu, query: u32, stage: u32) {
    let Some(pass) = gpu.particle_pass.as_ref() else {
        return;
    };
    let queries = pass.flights[gpu.flight.min(1)].queries;
    if queries.is_null() || gpu.timestamp_period <= 0.0 {
        return;
    }
    unsafe {
        (gpu.fns.cmd_write_timestamp)(gpu.cmd, stage, queries, query);
    }
}

#[repr(C)]
struct Binding {
    binding: u32,
    kind: u32,
    count: u32,
    stages: u32,
    samplers: *const c_void,
}

fn storage_binding(binding: u32, stages: u32) -> Binding {
    Binding {
        binding,
        kind: 7,
        count: 1,
        stages,
        samplers: std::ptr::null(),
    }
}

fn storage_bindings(count: u32, stages: u32) -> Vec<Binding> {
    (0..count).map(|binding| storage_binding(binding, stages)).collect()
}

fn create_desc_layout(gpu: &Gpu, bindings: &[Binding]) -> Result<Handle, String> {
    #[repr(C)]
    struct SetInfo {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        count: u32,
        bindings: *const Binding,
    }
    let info = SetInfo {
        s_type: 32,
        next: std::ptr::null(),
        flags: 0,
        count: u32::try_from(bindings.len()).unwrap_or(0),
        bindings: bindings.as_ptr(),
    };
    let mut layout = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_desc_layout)(
                gpu.device,
                &info as *const SetInfo as *const u8,
                std::ptr::null(),
                &mut layout,
            ),
            "particle descriptor layout",
        )?;
    }
    Ok(layout)
}

fn alloc_set(gpu: &Gpu, layout: Handle, storage_count: u32) -> Result<(Handle, Handle), String> {
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
    let size = Size {
        kind: 7,
        count: storage_count,
    };
    let pool_info = PoolInfo {
        s_type: 33,
        next: std::ptr::null(),
        flags: 0,
        max_sets: 1,
        size_count: 1,
        sizes: &size,
    };
    let mut pool = std::ptr::null_mut();
    let mut set = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_desc_pool)(
                gpu.device,
                &pool_info as *const PoolInfo as *const u8,
                std::ptr::null(),
                &mut pool,
            ),
            "particle descriptor pool",
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
            pool,
            count: 1,
            layouts: &layout,
        };
        check(
            (gpu.fns.alloc_desc)(gpu.device, &alloc as *const Alloc as *const u8, &mut set),
            "particle descriptor set",
        )?;
    }
    Ok((pool, set))
}

fn write_storage_bindings(gpu: &Gpu, set: Handle, buffers: &[Handle]) -> Result<(), String> {
    write_storage_bindings_from(gpu, set, 0, buffers)
}

fn write_storage_bindings_from(
    gpu: &Gpu,
    set: Handle,
    first: u32,
    buffers: &[Handle],
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
    let infos: Vec<BufInfo> = buffers
        .iter()
        .map(|buffer| BufInfo {
            buffer: *buffer,
            offset: 0,
            range: u64::MAX,
        })
        .collect();
    let writes: Vec<Write> = infos
        .iter()
        .enumerate()
        .map(|(binding, info)| Write {
            s_type: 35,
            next: std::ptr::null(),
            set,
            binding: first + u32::try_from(binding).unwrap_or(0),
            element: 0,
            count: 1,
            kind: 7,
            image: std::ptr::null(),
            buffer: info,
            texel: std::ptr::null(),
        })
        .collect();
    unsafe {
        (gpu.fns.update_desc)(
            gpu.device,
            u32::try_from(writes.len()).unwrap_or(0),
            writes.as_ptr() as *const u8,
            0,
            std::ptr::null(),
        );
    }
    Ok(())
}

fn alloc_compute_set(gpu: &Gpu, layout: Handle) -> Result<(Handle, Handle), String> {
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
    let sizes = [Size { kind: 7, count: 8 }, Size { kind: 1, count: 1 }];
    let pool_info = PoolInfo {
        s_type: 33,
        next: std::ptr::null(),
        flags: 0,
        max_sets: 1,
        size_count: 2,
        sizes: sizes.as_ptr(),
    };
    let mut pool = std::ptr::null_mut();
    let mut set = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_desc_pool)(
                gpu.device,
                &pool_info as *const PoolInfo as *const u8,
                std::ptr::null(),
                &mut pool,
            ),
            "particle descriptor pool",
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
            pool,
            count: 1,
            layouts: &layout,
        };
        check(
            (gpu.fns.alloc_desc)(gpu.device, &alloc as *const Alloc as *const u8, &mut set),
            "particle descriptor set",
        )?;
    }
    Ok((pool, set))
}

fn write_depth_image(gpu: &Gpu, set: Handle, view: Handle, sampler: Handle) {
    #[repr(C)]
    struct Img {
        sampler: Handle,
        view: Handle,
        layout: u32,
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
        image: *const Img,
        buffer: *const c_void,
        texel: *const c_void,
    }
    let info = Img {
        sampler,
        view,
        layout: 5,
    };
    let write = Write {
        s_type: 35,
        next: std::ptr::null(),
        set,
        binding: 7,
        element: 0,
        count: 1,
        kind: 1,
        image: &info,
        buffer: std::ptr::null(),
        texel: std::ptr::null(),
    };
    unsafe {
        (gpu.fns.update_desc)(
            gpu.device,
            1,
            &write as *const Write as *const u8,
            0,
            std::ptr::null(),
        );
    }
}

fn create_pipe_layout(
    gpu: &Gpu,
    set_layout: Handle,
    push_size: u32,
    push_stage: u32,
) -> Result<Handle, String> {
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
        sets: *const Handle,
        push_count: u32,
        push: *const Range,
    }
    let range = Range {
        stage: push_stage,
        offset: 0,
        size: push_size,
    };
    let info = Info {
        s_type: 30,
        next: std::ptr::null(),
        flags: 0,
        set_count: 1,
        sets: &set_layout,
        push_count: u32::from(push_size > 0),
        push: if push_size > 0 {
            &range
        } else {
            std::ptr::null()
        },
    };
    let mut layout = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_layout)(
                gpu.device,
                &info as *const Info as *const u8,
                std::ptr::null(),
                &mut layout,
            ),
            "particle pipeline layout",
        )?;
    }
    Ok(layout)
}

fn create_compute_pipe(gpu: &Gpu, module: Handle, layout: Handle) -> Result<Handle, String> {
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
    struct Info {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        stage: Stage,
        layout: Handle,
        base: Handle,
        base_index: i32,
    }
    let info = Info {
        s_type: 29,
        next: std::ptr::null(),
        flags: 0,
        stage: Stage {
            s_type: 18,
            next: std::ptr::null(),
            flags: 0,
            stage: 0x20,
            module,
            name: b"main\0".as_ptr().cast(),
            spec: std::ptr::null(),
        },
        layout,
        base: std::ptr::null_mut(),
        base_index: -1,
    };
    let mut pipe = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_compute)(
                gpu.device,
                std::ptr::null_mut(),
                1,
                &info as *const Info as *const u8,
                std::ptr::null(),
                &mut pipe,
            ),
            "particle compute pipeline",
        )?;
    }
    Ok(pipe)
}

fn create_draw_pipe(
    gpu: &Gpu,
    vert: Handle,
    frag: Handle,
    layout: Handle,
    depth_test: bool,
) -> Result<Handle, String> {
    if gpu.keep_pass.is_null() {
        return Err("particle draw has no keep pass".into());
    }
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
    struct VertInfo {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        bind_count: u32,
        binds: *const c_void,
        attr_count: u32,
        attrs: *const c_void,
    }
    let vert_info = VertInfo {
        s_type: 19,
        next: std::ptr::null(),
        flags: 0,
        bind_count: 0,
        binds: std::ptr::null(),
        attr_count: 0,
        attrs: std::ptr::null(),
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
    let viewport = ViewportState {
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
    let depth = Depth {
        s_type: 25,
        next: std::ptr::null(),
        flags: 0,
        test: u32::from(depth_test),
        write: 0,
        compare: 3,
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
    let batt = BlendAtt {
        enable: 1,
        src: 1,
        dst: 7,
        op: 0,
        src_a: 1,
        dst_a: 7,
        op_a: 0,
        mask: 0xf,
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
    let dynamic = Dyn {
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
        stage_count: 2,
        stages: stages.as_ptr(),
        vertex: &vert_info,
        ia: &ia,
        tess: std::ptr::null(),
        viewport: &viewport,
        raster: &raster,
        ms: &ms,
        depth: &depth,
        blend: &blend,
        dynamic: &dynamic,
        layout,
        pass: gpu.keep_pass,
        subpass: 0,
        base: std::ptr::null_mut(),
        base_index: -1,
    };
    let mut pipeline = std::ptr::null_mut();
    unsafe {
        check(
            (gpu.fns.create_pipelines)(
                gpu.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut pipeline,
            ),
            "particle draw pipeline",
        )?;
    }
    Ok(pipeline)
}

fn destroy_pipe(
    gpu: &Gpu,
    queries: Handle,
    pipe: Handle,
    layout: Handle,
    pool: Handle,
    desc_layout: Handle,
) {
    unsafe {
        if !queries.is_null() {
            (gpu.fns.destroy_query_pool)(gpu.device, queries, std::ptr::null());
        }
        if !pipe.is_null() {
            (gpu.fns.destroy_pipeline)(gpu.device, pipe, std::ptr::null());
        }
        if !layout.is_null() {
            (gpu.fns.destroy_layout)(gpu.device, layout, std::ptr::null());
        }
        if !pool.is_null() {
            (gpu.fns.destroy_desc_pool)(gpu.device, pool, std::ptr::null());
        }
        if !desc_layout.is_null() {
            (gpu.fns.destroy_desc_layout)(gpu.device, desc_layout, std::ptr::null());
        }
    }
}

fn extent_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn zero_bytes(bytes: u64) -> Result<Vec<u8>, String> {
    let len = usize::try_from(bytes).map_err(|_| {
        "particle clear is larger than addressable memory".to_string()
    })?;
    Ok(vec![0u8; len])
}

fn push_handle(out: &mut Vec<u64>, handle: Handle) {
    if handle.is_null() {
        return;
    }
    if let Ok(value) = u64::try_from(handle.addr()) {
        out.push(value);
    }
}

fn copy_buffer_handle(buffer: &Buffer) -> Buffer {
    Buffer {
        buffer: buffer.buffer,
        memory: buffer.memory,
        size: buffer.size,
    }
}

fn zero_host_buffer(gpu: &Gpu, buffer: &Buffer) -> Result<(), String> {
    let count = usize::try_from(buffer.size).unwrap_or(0);
    unsafe {
        let mut mapped = std::ptr::null_mut();
        check(
            (gpu.fns.map_mem)(
                gpu.device,
                buffer.memory,
                0,
                buffer.size,
                0,
                &mut mapped,
            ),
            "map particle state",
        )?;
        std::ptr::write_bytes(mapped.cast::<u8>(), 0u8, count);
        (gpu.fns.unmap_mem)(gpu.device, buffer.memory);
    }
    Ok(())
}

fn read_timestamp(gpu: &Gpu, pool: Handle, query: u32) -> Option<u64> {
    let mut raw = 0u64;
    let got = unsafe {
        (gpu.fns.get_query_results)(
            gpu.device,
            pool,
            query,
            1,
            8,
            &mut raw as *mut u64 as *mut c_void,
            8,
            0x1,
        )
    };
    if got != 0 {
        None
    } else {
        Some(raw)
    }
}

fn read_u32_at(gpu: &Gpu, buffer: &Buffer, offset: u64) -> Result<u32, String> {
    let bytes = read_range(gpu, buffer, offset, 4)?;
    Ok(u32::from_ne_bytes(bytes.try_into().unwrap_or([0; 4])))
}

fn read_range(gpu: &Gpu, buffer: &Buffer, offset: u64, count: usize) -> Result<Vec<u8>, String> {
    if offset.saturating_add(count as u64) > buffer.size {
        return Err("particle read is past the buffer".into());
    }
    let mut raw = vec![0u8; count];
    unsafe {
        let mut mapped = std::ptr::null_mut();
        check(
            (gpu.fns.map_mem)(
                gpu.device,
                buffer.memory,
                offset,
                count as u64,
                0,
                &mut mapped,
            ),
            "map particle buffer",
        )?;
        std::ptr::copy_nonoverlapping(mapped as *const u8, raw.as_mut_ptr(), count);
        (gpu.fns.unmap_mem)(gpu.device, buffer.memory);
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::{
        COMPUTE_STAGE, RING_DST_ACCESS, RING_DST_STAGE, RING_SRC_ACCESS, RING_SRC_STAGE,
        SHADER_ACCESS,
    };

    #[test]
    fn the_particle_ring_barrier_keeps_compute_in_both_masks() {
        assert_ne!(RING_SRC_STAGE & COMPUTE_STAGE, 0, "src stage dropped compute");
        assert_ne!(RING_DST_STAGE & COMPUTE_STAGE, 0, "dst stage dropped compute");
        assert_eq!(
            RING_SRC_ACCESS & SHADER_ACCESS,
            SHADER_ACCESS,
            "src access dropped shader read or write"
        );
        assert_eq!(
            RING_DST_ACCESS & SHADER_ACCESS,
            SHADER_ACCESS,
            "dst access dropped shader read or write"
        );
    }
}
