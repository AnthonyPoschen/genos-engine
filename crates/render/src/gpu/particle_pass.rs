//! Stage-1 particle compute and the indirect draw in the keep pass.
//!
//! The compute buffer stays on the device. The CPU reads it only after a
//! readback frame, for the lavapipe agreement test. A frame that does not
//! ask for a picture does not map the positions.

use std::ffi::c_void;

use genos_scene::{look_direction, Camera, ParticleSettings};

use super::{check, Buffer, Gpu, Handle, Memory};
use crate::particle_sim::{
    buffer_bytes, sim_generation, ParticleFault, ParticleStats, COMPACT_STRIDE, PARAMS_BYTES,
    SLOT_STRIDE, STATE_BYTES,
};

const INDIRECT_BYTES: u64 = 16;
const DEPTH_FLAG: u32 = 8;
const FREEZE_FLAG: u32 = 1;

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
    depth_collision: bool,
    soft_band: f32,
    generation: u32,
    view_proj: [f32; 16],
    right: [f32; 4],
    up: [f32; 4],
}

pub(super) struct Pass {
    budget: u32,
    params: Buffer,
    state: Buffer,
    slots: Buffer,
    dead: Buffer,
    compact: Buffer,
    indirect: Buffer,
    depth: Buffer,
    compute_desc_layout: Handle,
    compute_pool: Handle,
    compute_set: Handle,
    compute_layout: Handle,
    compute_pipe: Handle,
    draw_desc_layout: Handle,
    draw_pool: Handle,
    draw_set: Handle,
    draw_layout: Handle,
    draw_pipe: Handle,
    queries: Handle,
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
            return Ok(());
        };
        let depth_bytes = self.particle_depth_bytes();
        self.ensure_particle_pass(job.budget, depth_bytes)?;
        self.write_particle_inputs(&job)?;
        let (pipe, layout, set, queries) = {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            (pass.compute_pipe, pass.compute_layout, pass.compute_set, pass.queries)
        };
        unsafe {
            if !queries.is_null() {
                (self.fns.cmd_reset_query)(self.cmd, queries, 0, 4);
            }
            memory_barrier(self, 0x4000, 0x800, 0x4000, 0x20);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, pipe);
            (self.fns.cmd_bind_set)(self.cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            write_particle_stamp(self, 0, 0x800);
            (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
            write_particle_stamp(self, 1, 0x800);
            memory_barrier(self, 0x800, 0x8 | 0x2 | 0x4000, 0x40, 0x20 | 0x1 | 0x2000);
        }
        self.particle_stamped = true;
        Ok(())
    }

    pub(super) fn record_particle_draw(&mut self) {
        if !self.particle_stamped || self.particle_drawn || self.keep_pass.is_null() {
            return;
        }
        let (pipe, layout, set, indirect) = {
            let Some(pass) = self.particle_pass.as_ref() else {
                return;
            };
            (pass.draw_pipe, pass.draw_layout, pass.draw_set, pass.indirect.buffer)
        };
        let Some(job) = self.particle_job else {
            return;
        };
        self.particle_drawn = true;
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
            pass: self.keep_pass,
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
            write_particle_stamp(self, 2, 0x2);
            (self.fns.cmd_begin_rp)(self.cmd, &rp as *const RpBegin as *const u8, 0);
            (self.fns.cmd_bind_pipe)(self.cmd, 0, pipe);
            (self.fns.cmd_bind_set)(self.cmd, 0, layout, 0, 1, &set, 0, std::ptr::null());
            let viewport = [0.0f32, 0.0, width as f32, height as f32, 0.0, 1.0];
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
            let scissor = [0i32, 0, width as i32, height as i32];
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
            (self.fns.cmd_push)(
                self.cmd,
                layout,
                1,
                0,
                96,
                push.as_ptr() as *const c_void,
            );
            (self.fns.cmd_draw_indirect)(self.cmd, indirect, 0, 1, 16);
            (self.fns.cmd_end_rp)(self.cmd);
            write_particle_stamp(self, 3, 0x400);
        }
    }

    pub(super) fn collect_particle_stats(&mut self) {
        if !self.particle_stamped {
            return;
        }
        let Some(pass) = self.particle_pass.as_ref() else {
            return;
        };
        let queries = pass.queries;
        let state = copy_buffer_handle(&pass.state);
        let indirect = copy_buffer_handle(&pass.indirect);
        let bytes = pass.bytes;
        let budget = pass.budget;
        let count = self.particle_job.map_or(0, |job| job.count);
        let sim_ms = self.particle_span_ms(queries, 0, 1);
        let draw_ms = self.particle_span_ms(queries, 2, 3);
        let alive = read_u32_at(self, &state, 16).unwrap_or(0);
        let spawned = read_u32_at(self, &state, 20).unwrap_or(0);
        let draw_instances = read_u32_at(self, &indirect, 4).unwrap_or(0);
        self.particle_stats = ParticleStats {
            sim_ms,
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

    pub(super) fn particle_positions(&self) -> Result<Vec<[f32; 3]>, String> {
        let Some(pass) = self.particle_pass.as_ref() else {
            return Ok(Vec::new());
        };
        let budget = pass.budget;
        let state = copy_buffer_handle(&pass.state);
        let compact = copy_buffer_handle(&pass.compact);
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
        destroy_pipe(
            self,
            pass.queries,
            pass.compute_pipe,
            pass.compute_layout,
            pass.compute_pool,
            pass.compute_desc_layout,
        );
        destroy_pipe(
            self,
            std::ptr::null_mut(),
            pass.draw_pipe,
            pass.draw_layout,
            pass.draw_pool,
            pass.draw_desc_layout,
        );
        let mut buffers = [
            pass.params, pass.state, pass.slots, pass.dead, pass.compact, pass.indirect,
            pass.depth,
        ];
        for buffer in &mut buffers {
            self.destroy_buffer(buffer);
        }
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

    fn ensure_particle_pass(&mut self, budget: u32, depth_bytes: u64) -> Result<(), String> {
        let recreate = self
            .particle_pass
            .as_ref()
            .is_none_or(|pass| pass.budget != budget);
        if recreate {
            self.destroy_particle_pass();
            self.particle_pass = Some(self.create_particle_pass(budget, depth_bytes)?);
            return Ok(());
        }
        let too_small = self
            .particle_pass
            .as_ref()
            .is_some_and(|pass| pass.depth.size < depth_bytes);
        if too_small {
            self.grow_particle_depth(depth_bytes)?;
        }
        Ok(())
    }

    fn write_particle_inputs(&mut self, job: &Job) -> Result<(), String> {
        let mut flags = self.particle_fault.bits();
        if job.freeze {
            flags |= FREEZE_FLAG;
        }
        if job.depth_collision {
            flags |= DEPTH_FLAG;
        }
        let (depth_w, depth_h) = self.uploaded_depth_size();
        let params = pack_params(job, self.particle_dt, flags, depth_w, depth_h);
        let depth = self.particle_depth.clone();
        let (params_buf, depth_buf) = {
            let pass = self.particle_pass.as_ref().ok_or("particle pass missing")?;
            (
                copy_buffer_handle(&pass.params),
                copy_buffer_handle(&pass.depth),
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
        let params = self.make_buffer(PARAMS_BYTES, 0x20, Memory::Upload)?;
        let state = self.make_buffer(STATE_BYTES, 0x20, Memory::Readback)?;
        let slot_buf = self.make_buffer(slots * SLOT_STRIDE, 0x20, Memory::Readback)?;
        let dead = self.make_buffer(slots * 4, 0x20, Memory::Readback)?;
        let compact = self.make_buffer(slots * COMPACT_STRIDE, 0x20, Memory::Readback)?;
        let indirect = self.make_buffer(INDIRECT_BYTES, 0x20 | 0x100, Memory::Readback)?;
        let depth = self.make_buffer(depth_bytes.max(4), 0x20, Memory::Upload)?;
        self.write_buffer(&state, &[0u8; STATE_BYTES as usize])?;
        self.write_buffer(&indirect, &[0u8; INDIRECT_BYTES as usize])?;
        let compute = self.particle_compute_pipeline(&[
            params.buffer, state.buffer, slot_buf.buffer, dead.buffer, compact.buffer,
            indirect.buffer, depth.buffer,
        ])?;
        let draw = self.particle_draw_pipeline(compact.buffer)?;
        let queries = if self.timestamp_period > 0.0 {
            self.make_query_pool_n(4).unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        };
        let bytes = buffer_bytes(budget).saturating_add(depth.size);
        Ok(Pass {
            budget,
            params,
            state,
            slots: slot_buf,
            dead,
            compact,
            indirect,
            depth,
            compute_desc_layout: compute.0,
            compute_pool: compute.1,
            compute_set: compute.2,
            compute_layout: compute.3,
            compute_pipe: compute.4,
            draw_desc_layout: draw.0,
            draw_pool: draw.1,
            draw_set: draw.2,
            draw_layout: draw.3,
            draw_pipe: draw.4,
            queries,
            bytes,
        })
    }

    fn grow_particle_depth(&mut self, depth_bytes: u64) -> Result<(), String> {
        let mut depth = self.make_buffer(depth_bytes, 0x20, Memory::Upload)?;
        let handles = {
            let Some(pass) = self.particle_pass.as_mut() else {
                self.destroy_buffer(&mut depth);
                return Err("particle pass missing".into());
            };
            std::mem::swap(&mut pass.depth, &mut depth);
            pass.bytes = buffer_bytes(pass.budget).saturating_add(pass.depth.size);
            [
                pass.params.buffer,
                pass.state.buffer,
                pass.slots.buffer,
                pass.dead.buffer,
                pass.compact.buffer,
                pass.indirect.buffer,
                pass.depth.buffer,
            ]
        };
        self.destroy_buffer(&mut depth);
        let set = self
            .particle_pass
            .as_ref()
            .map(|pass| pass.compute_set)
            .ok_or("particle pass missing")?;
        write_storage_bindings(self, set, &handles)?;
        Ok(())
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
    ) -> Result<(Handle, Handle, Handle, Handle, Handle), String> {
        let bindings = storage_bindings(7, 0x20);
        let desc_layout = create_desc_layout(self, &bindings)?;
        let (pool, set) = alloc_set(self, desc_layout, 7)?;
        write_storage_bindings(self, set, buffers)?;
        let pipe_layout = create_pipe_layout(self, desc_layout, 0)?;
        let module = self.shader(super::PARTICLES_COMP_SPV)?;
        let pipe = create_compute_pipe(self, module, pipe_layout)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok((desc_layout, pool, set, pipe_layout, pipe))
    }

    fn particle_draw_pipeline(
        &mut self,
        compact: Handle,
    ) -> Result<(Handle, Handle, Handle, Handle, Handle), String> {
        let bindings = [storage_binding(0, 0x1)];
        let desc_layout = create_desc_layout(self, &bindings)?;
        let (pool, set) = alloc_set(self, desc_layout, 1)?;
        write_storage_bindings(self, set, &[compact])?;
        let pipe_layout = create_pipe_layout(self, desc_layout, 96)?;
        let vert = self.shader(super::PARTICLES_VERT_SPV)?;
        let frag = self.shader(super::PARTICLES_FRAG_SPV)?;
        let pipe = create_draw_pipe(self, vert, frag, pipe_layout)?;
        unsafe {
            (self.fns.destroy_shader)(self.device, vert, std::ptr::null());
            (self.fns.destroy_shader)(self.device, frag, std::ptr::null());
        }
        Ok((desc_layout, pool, set, pipe_layout, pipe))
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
    /// The scene depth image stays in the raster pass, so this upload is the
    /// collision buffer. The centre of the particle samples one texel.
    pub fn set_particle_depth(&mut self, width: u32, height: u32, samples: &[f32]) {
        self.gpu.particle_depth_size = (width, height);
        self.gpu.particle_depth.clear();
        self.gpu.particle_depth.extend_from_slice(samples);
    }

    /// Last completed particle step. Times are 0 before a frame, or when the
    /// device has no timestamp queries. They are lavapipe timestamps, not a
    /// score against a desktop GPU.
    #[must_use]
    pub fn particle_stats(&self) -> ParticleStats {
        self.gpu.particle_stats
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

fn pack_params(job: &Job, dt: f32, flags: u32, depth_w: u32, depth_h: u32) -> [u8; 256] {
    let mut bytes = [0u8; 256];
    write_f32(&mut bytes, 0, &[job.origin[0], job.origin[1], job.origin[2], job.rate]);
    write_f32(&mut bytes, 16, &[job.rise, job.life, job.spread, job.size]);
    write_f32(&mut bytes, 32, &[job.albedo[0], job.albedo[1], job.albedo[2], dt]);
    write_u32(&mut bytes, 48, &[job.seed, job.budget, job.count, flags]);
    write_u32(&mut bytes, 64, &[job.generation, depth_w, depth_h, 0]);
    write_f32(&mut bytes, 80, &[job.soft_band, 0.0, 0.0, 0.0]);
    write_f32(&mut bytes, 96, &job.view_proj);
    write_f32(&mut bytes, 160, &job.right);
    write_f32(&mut bytes, 176, &job.up);
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
    if pass.queries.is_null() || gpu.timestamp_period <= 0.0 {
        return;
    }
    unsafe {
        (gpu.fns.cmd_write_timestamp)(gpu.cmd, stage, pass.queries, query);
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
            binding: u32::try_from(binding).unwrap_or(0),
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

fn create_pipe_layout(gpu: &Gpu, set_layout: Handle, push_size: u32) -> Result<Handle, String> {
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
        stage: 1,
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
        test: 0,
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

fn copy_buffer_handle(buffer: &Buffer) -> Buffer {
    Buffer {
        buffer: buffer.buffer,
        memory: buffer.memory,
        size: buffer.size,
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
