//! Froxel inject and integrate. The extinction image is 3D. The column buffer
//! is one vec4 per view column. History is reserved in the byte count and is
//! not read. A frame records the pass after the depth prepass and copies the
//! columns into the resident scene so the compose shader can apply them.

use std::ffi::c_void;

use genos_scene::{Camera, FogSettings, FogShape, Light, LightingPreset, Scene};

use super::{check, read_f32s, tick_delta, Buffer, Gpu, Handle, Image, Memory};
use crate::fog::{
    default_frame_sun, dispatched_grid, frame_fog_view, grid_size, normalize, DirectSun, FogColumn,
    FogView, GridSize,
};
use crate::fog_budget::allocation_bytes;
use crate::fog_pass::{integrate_traced, FogDispatch, FogGpu, FogPointLamp, FOG_SKIP_SHADOW};
use crate::pack::{self, SCENE_TAIL};

const MAX_VOLUMES: usize = 8;
const MAX_LAMPS: usize = 8;
const PARAM_BYTES: usize = 144 + 48 * MAX_VOLUMES + 32 * MAX_LAMPS;
const MAX_COLUMNS: u64 = 160 * 90;

pub(super) struct FrameJob {
    params: Vec<u8>,
    width: u32,
    height: u32,
    slices: u32,
    settings: FogSettings,
    view: FogView,
    sun: DirectSun,
    lamps: Vec<crate::fog_pass::FogPointLamp>,
}

pub(super) struct Targets {
    size: GridSize,
    image: Image,
    columns: Buffer,
    history: Buffer,
    params: Buffer,
    scene: Buffer,
    ext_read: Buffer,
    bytes: u64,
    desc_layout: Handle,
    desc_pool: Handle,
    set: Handle,
    layout: Handle,
    inject: Handle,
    integrate: Handle,
    query: Handle,
    cmd: Handle,
    fence: Handle,
    image_ready: bool,
}

impl Gpu {
    pub(super) fn set_froxel_frame(&mut self, scene: &Scene, camera: &Camera, aspect: f32) {
        if !scene.fog.dispatches() {
            self.froxel_frame = None;
            self.froxel_vram = 0;
            return;
        }
        let size = dispatched_grid(&scene.fog, scene.lighting.preset);
        if size.slices == 0 || size.width == 0 || size.height == 0 {
            self.froxel_frame = None;
            self.froxel_vram = 0;
            return;
        }
        let origin = [camera.position.x, camera.position.y, camera.position.z];
        let view = frame_fog_view(origin, camera.yaw, camera.pitch, aspect);
        let (sun, lamps) = sun_and_lamps(&scene.lights);
        self.froxel_frame = Some(FrameJob {
            params: encode_params(&scene.fog, &view, size, &sun, &lamps, 0, 0),
            width: size.width,
            height: size.height,
            slices: size.slices,
            settings: scene.fog.clone(),
            view,
            sun,
            lamps,
        });
    }

    /// Columns the last frame dispatch wrote. Empty when that frame did not dispatch.
    pub(super) fn read_frame_columns(&mut self) -> Result<Vec<FogColumn>, String> {
        if !self.froxel_wrote {
            return Ok(Vec::new());
        }
        self.wait_fence(self.fence, "frame columns")?;
        let Some(job) = self.froxel_frame.as_ref() else {
            return Ok(Vec::new());
        };
        let memory = self
            .froxel
            .as_ref()
            .map_or(std::ptr::null_mut(), |targets| targets.columns.memory);
        if memory.is_null() {
            return Ok(Vec::new());
        }
        let count = usize::try_from(job.width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(job.height).unwrap_or(0));
        let floats = read_f32s(
            self.fns.map_mem,
            self.fns.unmap_mem,
            self.device,
            memory,
            count.saturating_mul(4),
        )?;
        Ok(columns_from(&floats))
    }

    /// CPU shadowed integral of the view the last frame dispatch built.
    pub(super) fn frame_fog_reference(&self, scene: &Scene) -> Vec<FogColumn> {
        if !self.froxel_wrote {
            return Vec::new();
        }
        let Some(job) = self.froxel_frame.as_ref() else {
            return Vec::new();
        };
        let size = GridSize {
            width: job.width,
            height: job.height,
            slices: job.slices,
        };
        integrate_traced(&job.settings, &job.view, size, job.sun, &job.lamps, scene)
    }

    /// Compose reads `pad0`. A frame that did not dispatch leaves it at 0.
    pub(super) fn release_froxel_header(&self) {
        if self.froxel_wrote || self.frame_scene[0].buffer.is_null() {
            return;
        }
        let cmd = self.cmd;
        self.barrier_on(cmd, 0x8000 | 0x800, 0x1000, 0x20, 0x1000);
        let zero = 0u32.to_ne_bytes();
        unsafe {
            (self.fns.cmd_update_buffer)(
                cmd,
                self.frame_scene[0].buffer,
                12,
                4,
                zero.as_ptr().cast(),
            );
        }
        self.barrier_on(cmd, 0x1000, 0x800, 0x1000, 0x20);
    }

    /// Record inject and integrate on the frame command buffer, then copy the
    /// columns into the resident scene ahead of compose.
    pub(super) fn record_froxel(&mut self) -> Result<(), String> {
        let Some(job) = self.froxel_frame.clone_job() else {
            return Ok(());
        };
        let size = GridSize {
            width: job.width,
            height: job.height,
            slices: job.slices,
        };
        self.ensure_froxel(size)?;
        let (query, image, columns, column_mem, params, set, layout, inject, integrate) = {
            let targets = self.froxel.as_ref().ok_or("froxel targets missing")?;
            (
                targets.query,
                targets.image.image,
                targets.columns.buffer,
                targets.columns.memory,
                targets.params.memory,
                targets.set,
                targets.layout,
                targets.inject,
                targets.integrate,
            )
        };
        self.zero_columns(column_mem, job.width, job.height)?;
        self.write_memory(params, &job.params)?;
        self.write_froxel_set(self.frame_scene[0].buffer)?;
        let cmd = self.cmd;
        // The depth prepass has read the scene. Host writes cover the params and the cleared columns.
        self.barrier_on(cmd, 0x4000 | 0x8000, 0x800, 0x4000 | 0x20, 0x20);
        self.transition_froxel_image(cmd, image)?;
        self.stamp_froxel(cmd, query, 0, 0x1);
        self.dispatch_froxel(
            cmd, layout, set, inject, integrate, job.width, job.height, job.slices,
        );
        self.stamp_froxel(cmd, query, 1, 0x800);
        if columns_fit(job.width, job.height) {
            self.publish_frame_columns(cmd, columns, job.width, job.height, job.slices);
            self.froxel_wrote = true;
        }
        self.froxel_pending = !query.is_null() && self.timestamp_period > 0.0;
        if let Some(targets) = self.froxel.as_ref() {
            self.froxel_vram = targets.bytes;
        }
        Ok(())
    }

    /// Drop a previous dispatch's columns so a skipped integrate cannot be read back.
    fn zero_columns(&self, memory: Handle, width: u32, height: u32) -> Result<(), String> {
        if memory.is_null() {
            return Ok(());
        }
        let bytes = usize::try_from(column_bytes(width, height)).unwrap_or(0);
        self.write_memory(memory, &vec![0u8; bytes])
    }

    /// Device-side `pad0` plus the column tail. Compose sees fog only after this.
    fn publish_frame_columns(
        &self,
        cmd: Handle,
        columns: Handle,
        width: u32,
        height: u32,
        slices: u32,
    ) {
        self.barrier_on(cmd, 0x800, 0x1000 | 0x4000, 0x40 | 0x20, 0x800 | 0x1000 | 0x2000);
        let word = tail_word().to_ne_bytes();
        let head = column_header(width, height, slices);
        unsafe {
            (self.fns.cmd_update_buffer)(
                cmd,
                self.frame_scene[0].buffer,
                12,
                4,
                word.as_ptr().cast(),
            );
            (self.fns.cmd_update_buffer)(
                cmd,
                self.frame_scene[0].buffer,
                scene_offset(),
                16,
                head.as_ptr().cast(),
            );
            let region = CopyRegion {
                src: 0,
                dst: scene_offset().saturating_add(16),
                size: column_bytes(width, height),
            };
            (self.fns.cmd_copy_buffer)(
                cmd,
                columns,
                self.frame_scene[0].buffer,
                1,
                std::ptr::from_ref(&region).cast(),
            );
        }
        self.barrier_on(cmd, 0x1000, 0x800, 0x1000, 0x20);
    }

    pub(super) fn collect_froxel_time(&mut self) {
        if !self.froxel_pending {
            return;
        }
        self.froxel_pending = false;
        if let Some(ms) = self.read_froxel_ms() {
            self.froxel_pass_ms = ms;
        }
    }

    pub(super) fn froxel_pass_ms(&self) -> f64 {
        self.froxel_pass_ms
    }

    pub(super) fn froxel_vram(&self) -> u64 {
        self.froxel_vram
    }

    pub(super) fn run_fog(&mut self, job: &FogDispatch) -> Result<FogGpu, String> {
        // The frame path shares this query pool and descriptor set.
        // A standalone dispatch replaces the frame's columns, so the frame readback ends.
        self.froxel_wrote = false;
        self.wait_all_inflight()?;
        if job.size.slices == 0 || job.size.width == 0 || job.size.height == 0 {
            return Ok(FogGpu {
                columns: Vec::new(),
                extinction: Vec::new(),
                pass_ms: 0.0,
                vram_bytes: 0,
            });
        }
        self.ensure_froxel(job.size)?;
        let mut flags = job.flags;
        if job.scene.is_none() {
            flags |= FOG_SKIP_SHADOW;
        }
        let params = encode_params(
            &job.settings,
            &job.view,
            job.size,
            &job.sun,
            &job.lamps,
            flags,
            job.stop_slices,
        );
        let scene = scene_bytes_for(job);
        let (
            params_mem,
            scene_buf,
            query,
            image,
            set,
            layout,
            inject,
            integrate,
            cmd,
            fence,
            columns_mem,
        ) = {
            let targets = self.froxel.as_ref().ok_or("froxel targets missing")?;
            (
                targets.params.memory,
                targets.scene.buffer,
                targets.query,
                targets.image.image,
                targets.set,
                targets.layout,
                targets.inject,
                targets.integrate,
                targets.cmd,
                targets.fence,
                targets.columns.memory,
            )
        };
        self.write_memory(params_mem, &params)?;
        self.write_buffer_handle(&scene)?;
        if job.read_extinction {
            self.ensure_ext_read(job.size)?;
        }
        self.write_froxel_set(scene_buf)?;
        self.wait_fence(fence, "froxel wait")?;
        let fences = [fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "froxel reset fence",
            )?;
            check((self.fns.reset_cmd)(cmd, 0), "froxel reset")?;
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
                (self.fns.begin_cmd)(cmd, std::ptr::from_ref(&begin).cast()),
                "froxel begin",
            )?;
        }
        self.barrier_on(cmd, 0x4000, 0x800, 0x4000, 0x20);
        self.transition_froxel_image(cmd, image)?;
        self.stamp_froxel(cmd, query, 0, 0x1);
        self.dispatch_froxel(
            cmd,
            layout,
            set,
            inject,
            integrate,
            job.size.width,
            job.size.height,
            job.size.slices,
        );
        self.stamp_froxel(cmd, query, 1, 0x800);
        if job.read_extinction {
            self.copy_extinction(cmd, image, job.size)?;
        }
        self.barrier_on(cmd, 0x800 | 0x1000, 0x4000, 0x40 | 0x1000, 0x2000);
        unsafe {
            check((self.fns.end_cmd)(cmd), "froxel end")?;
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
            let cmds = [cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmds.as_ptr(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(self.queue, 1, std::ptr::from_ref(&submit).cast(), fence),
                "froxel submit",
            )?;
        }
        self.wait_fence(fence, "froxel done")?;
        let pass_ms = self.read_froxel_ms().unwrap_or(0.0);
        self.froxel_pass_ms = pass_ms;
        let vram_bytes = self.froxel.as_ref().map_or(0, |targets| targets.bytes);
        self.froxel_vram = vram_bytes;
        let count = usize::try_from(job.size.width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(job.size.height).unwrap_or(0));
        let floats = read_f32s(
            self.fns.map_mem,
            self.fns.unmap_mem,
            self.device,
            columns_mem,
            count.saturating_mul(4),
        )?;
        let columns = columns_from(&floats);
        let extinction = if job.read_extinction {
            let cells = job.size.cells();
            let memory = self
                .froxel
                .as_ref()
                .map_or(std::ptr::null_mut(), |targets| targets.ext_read.memory);
            read_f32s(
                self.fns.map_mem,
                self.fns.unmap_mem,
                self.device,
                memory,
                cells,
            )?
        } else {
            Vec::new()
        };
        Ok(FogGpu {
            columns,
            extinction,
            pass_ms,
            vram_bytes,
        })
    }

    /// Timestamp of one inject plus integrate at `size`. Not an RTX 4070 time.
    pub(super) fn froxel_grid_ms(&mut self, size: GridSize) -> Result<f64, String> {
        if size.width == 0 || size.height == 0 || size.slices == 0 {
            return Ok(0.0);
        }
        let job = FogDispatch {
            settings: height_settings(),
            view: FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0),
            size,
            sun: DirectSun {
                toward: normalize([0.2, 0.9, 0.1]),
                color: [1.0, 0.95, 0.85],
            },
            lamps: Vec::new(),
            scene: None,
            flags: 0,
            stop_slices: 0,
            read_extinction: false,
        };
        let gpu = self.run_fog(&job)?;
        if self.timestamp_period <= 0.0 {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        Ok(gpu.pass_ms)
    }

    pub(super) fn froxel_sample_ms(
        &mut self,
        preset: LightingPreset,
        samples: u32,
    ) -> Result<f64, String> {
        let size = grid_for_samples(preset, samples);
        if size.slices == 0 {
            return Ok(0.0);
        }
        let job = FogDispatch {
            settings: height_settings(),
            view: FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0),
            size,
            sun: DirectSun {
                toward: normalize([0.2, 0.9, 0.1]),
                color: [1.0, 0.95, 0.85],
            },
            lamps: Vec::new(),
            scene: Some(empty_scene()),
            flags: 0,
            stop_slices: 0,
            read_extinction: false,
        };
        let gpu = self.run_fog(&job)?;
        if self.timestamp_period <= 0.0 {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        Ok(gpu.pass_ms)
    }

    pub(super) fn froxel_allocation(
        &mut self,
        preset: LightingPreset,
        doubled: bool,
    ) -> Result<u64, String> {
        let size = grid_size(preset);
        if size.slices == 0 {
            return Ok(0);
        }
        self.wait_all_inflight()?;
        self.ensure_froxel(size)?;
        let once = self.froxel.as_ref().map_or(0, |targets| targets.bytes);
        if !doubled {
            return Ok(once);
        }
        let extra = self.measure_extra(size)?;
        Ok(once.saturating_add(extra))
    }

    pub(super) fn destroy_froxel(&mut self) {
        let Some(mut targets) = self.froxel.take() else {
            return;
        };
        unsafe {
            if !targets.fence.is_null() {
                (self.fns.destroy_fence)(self.device, targets.fence, std::ptr::null());
            }
            if !targets.inject.is_null() {
                (self.fns.destroy_pipeline)(self.device, targets.inject, std::ptr::null());
            }
            if !targets.integrate.is_null() {
                (self.fns.destroy_pipeline)(self.device, targets.integrate, std::ptr::null());
            }
            if !targets.layout.is_null() {
                (self.fns.destroy_layout)(self.device, targets.layout, std::ptr::null());
            }
            if !targets.desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, targets.desc_pool, std::ptr::null());
            }
            if !targets.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, targets.desc_layout, std::ptr::null());
            }
            if !targets.query.is_null() {
                (self.fns.destroy_query_pool)(self.device, targets.query, std::ptr::null());
            }
        }
        self.destroy_image(&mut targets.image);
        self.destroy_buffer(&mut targets.columns);
        self.destroy_buffer(&mut targets.history);
        self.destroy_buffer(&mut targets.params);
        self.destroy_buffer(&mut targets.scene);
        self.destroy_buffer(&mut targets.ext_read);
    }

    fn ensure_froxel(&mut self, size: GridSize) -> Result<(), String> {
        if self.froxel.is_none() {
            self.froxel = Some(self.create_froxel_pipes()?);
        }
        let same = self
            .froxel
            .as_ref()
            .is_some_and(|targets| targets.size == size);
        if same
            && self
                .froxel
                .as_ref()
                .is_some_and(|targets| !targets.image.image.is_null())
        {
            return Ok(());
        }
        let (image_bytes, image) = self.make_image_3d(size)?;
        let columns = self.make_buffer(
            column_bytes(size.width, size.height).max(16),
            0x20 | 0x1,
            Memory::Readback,
        )?;
        let history = self.make_history(size, image_bytes, columns.size)?;
        let bytes = image_bytes
            .saturating_add(columns.size)
            .saturating_add(history.size);
        let mut retired = {
            let targets = self.froxel.as_mut().ok_or("froxel targets missing")?;
            let retired = (
                std::mem::replace(&mut targets.image, image),
                std::mem::replace(&mut targets.columns, columns),
                std::mem::replace(&mut targets.history, history),
                std::mem::replace(&mut targets.ext_read, Buffer::empty()),
            );
            targets.size = size;
            targets.bytes = bytes;
            targets.image_ready = false;
            retired
        };
        self.destroy_image(&mut retired.0);
        self.destroy_buffer(&mut retired.1);
        self.destroy_buffer(&mut retired.2);
        self.destroy_buffer(&mut retired.3);
        Ok(())
    }

    fn measure_extra(&mut self, size: GridSize) -> Result<u64, String> {
        let (image_bytes, mut image) = self.make_image_3d(size)?;
        let mut columns = self.make_buffer(
            column_bytes(size.width, size.height).max(16),
            0x20 | 0x1,
            Memory::Readback,
        )?;
        let mut history = self.make_history(size, image_bytes, columns.size)?;
        let bytes = image_bytes
            .saturating_add(columns.size)
            .saturating_add(history.size);
        self.destroy_image(&mut image);
        self.destroy_buffer(&mut columns);
        self.destroy_buffer(&mut history);
        Ok(bytes)
    }

    fn make_history(
        &self,
        size: GridSize,
        image_bytes: u64,
        column_bytes: u64,
    ) -> Result<Buffer, String> {
        let design = if matches!(
            (size.width, size.height, size.slices),
            (64, 36, 24) | (96, 54, 32) | (160, 90, 48)
        ) {
            allocation_bytes(preset_of(size))
        } else {
            design_bytes(size)
        };
        let used = image_bytes.saturating_add(column_bytes);
        let pad = design.saturating_sub(used);
        if pad == 0 {
            return Ok(Buffer::empty());
        }
        self.make_buffer(pad, 0x20, Memory::Device)
    }

    fn ensure_ext_read(&mut self, size: GridSize) -> Result<(), String> {
        let bytes = u64::try_from(size.cells().saturating_mul(4)).unwrap_or(u64::MAX);
        let have = self
            .froxel
            .as_ref()
            .map_or(0, |targets| targets.ext_read.size);
        if have >= bytes && have > 0 {
            return Ok(());
        }
        let buffer = self.make_buffer(bytes.max(4), 0x2, Memory::Readback)?;
        let mut old = {
            let targets = self.froxel.as_mut().ok_or("froxel targets missing")?;
            std::mem::replace(&mut targets.ext_read, buffer)
        };
        self.destroy_buffer(&mut old);
        Ok(())
    }

    fn create_froxel_pipes(&mut self) -> Result<Targets, String> {
        let desc_layout = self.froxel_desc_layout()?;
        let layout = self.froxel_pipe_layout(desc_layout)?;
        let inject = self.froxel_pipe(layout, super::FROXEL_INJECT_SPV)?;
        let integrate = self.froxel_pipe(layout, super::FROXEL_INTEGRATE_SPV)?;
        let (desc_pool, set) = self.froxel_desc_pool(desc_layout)?;
        let (cmd, fence) = self.froxel_cmd_fence()?;
        let query = if self.timestamp_period > 0.0 {
            self.make_query_pool_n(2)?
        } else {
            std::ptr::null_mut()
        };
        let params = self.make_buffer(
            u64::try_from(PARAM_BYTES).unwrap_or(784),
            0x20,
            Memory::Upload,
        )?;
        let scene = self.make_buffer(
            u64::try_from(pack::SCENE_CAPACITY).unwrap_or(0),
            0x20,
            Memory::Upload,
        )?;
        Ok(Targets {
            size: GridSize {
                width: 0,
                height: 0,
                slices: 0,
            },
            image: Image::empty(),
            columns: Buffer::empty(),
            history: Buffer::empty(),
            params,
            scene,
            ext_read: Buffer::empty(),
            bytes: 0,
            desc_layout,
            desc_pool,
            set,
            layout,
            inject,
            integrate,
            query,
            cmd,
            fence,
            image_ready: false,
        })
    }

    fn froxel_desc_layout(&self) -> Result<Handle, String> {
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
                kind: 3,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 3,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            },
        ];
        let info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 4,
            bindings: bindings.as_ptr(),
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    std::ptr::from_ref(&info).cast(),
                    std::ptr::null(),
                    &mut layout,
                ),
                "froxel descriptor layout",
            )?;
        }
        Ok(layout)
    }

    fn froxel_pipe_layout(&self, set_layout: Handle) -> Result<Handle, String> {
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
        let sets = [set_layout];
        let info = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr(),
            push_count: 0,
            push: std::ptr::null(),
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    std::ptr::from_ref(&info).cast(),
                    std::ptr::null(),
                    &mut layout,
                ),
                "froxel pipeline layout",
            )?;
        }
        Ok(layout)
    }

    fn froxel_pipe(&self, layout: Handle, code: &[u8]) -> Result<Handle, String> {
        let module = self.shader(code)?;
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
        struct PipeInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            stage: Stage,
            layout: Handle,
            base: Handle,
            base_index: i32,
        }
        let info = PipeInfo {
            s_type: 29,
            next: std::ptr::null(),
            flags: 0,
            stage: Stage {
                s_type: 18,
                next: std::ptr::null(),
                flags: 0,
                stage: 0x20,
                module,
                name: c"main".as_ptr(),
                spec: std::ptr::null(),
            },
            layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        let mut pipe = std::ptr::null_mut();
        unsafe {
            let result = (self.fns.create_compute)(
                self.device,
                std::ptr::null_mut(),
                1,
                std::ptr::from_ref(&info).cast(),
                std::ptr::null(),
                &mut pipe,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "froxel pipeline")?;
        }
        Ok(pipe)
    }

    fn froxel_desc_pool(&self, layout: Handle) -> Result<(Handle, Handle), String> {
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
        let sizes = [Size { kind: 7, count: 3 }, Size { kind: 3, count: 1 }];
        let info = PoolInfo {
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
                (self.fns.create_desc_pool)(
                    self.device,
                    std::ptr::from_ref(&info).cast(),
                    std::ptr::null(),
                    &mut pool,
                ),
                "froxel descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool,
                count: 1,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(self.device, std::ptr::from_ref(&alloc).cast(), &mut set),
                "froxel descriptor set",
            )?;
        }
        Ok((pool, set))
    }

    fn froxel_cmd_fence(&self) -> Result<(Handle, Handle), String> {
        let mut cmd = std::ptr::null_mut();
        let mut fence = std::ptr::null_mut();
        unsafe {
            #[repr(C)]
            struct CmdAlloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                level: u32,
                count: u32,
            }
            let alloc = CmdAlloc {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.pool,
                level: 0,
                count: 1,
            };
            check(
                (self.fns.alloc_cmd)(self.device, std::ptr::from_ref(&alloc).cast(), &mut cmd),
                "froxel command buffer",
            )?;
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence_info = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            check(
                (self.fns.create_fence)(
                    self.device,
                    std::ptr::from_ref(&fence_info).cast(),
                    std::ptr::null(),
                    &mut fence,
                ),
                "froxel fence",
            )?;
        }
        Ok((cmd, fence))
    }

    fn write_froxel_set(&self, scene: Handle) -> Result<(), String> {
        let targets = self.froxel.as_ref().ok_or("froxel targets missing")?;
        #[repr(C)]
        struct BufInfo {
            buffer: Handle,
            offset: u64,
            range: u64,
        }
        #[repr(C)]
        struct ImageInfo {
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
            image: *const ImageInfo,
            buffer: *const BufInfo,
            texel: *const c_void,
        }
        let scene_info = BufInfo {
            buffer: scene,
            offset: 0,
            range: u64::MAX,
        };
        let params_info = BufInfo {
            buffer: targets.params.buffer,
            offset: 0,
            range: u64::MAX,
        };
        let column_info = BufInfo {
            buffer: targets.columns.buffer,
            offset: 0,
            range: u64::MAX,
        };
        let image_info = ImageInfo {
            sampler: std::ptr::null_mut(),
            view: targets.image.view,
            layout: 1,
        };
        let writes = [
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: targets.set,
                binding: 0,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &scene_info,
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: targets.set,
                binding: 1,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &params_info,
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: targets.set,
                binding: 2,
                element: 0,
                count: 1,
                kind: 3,
                image: &image_info,
                buffer: std::ptr::null(),
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: targets.set,
                binding: 3,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &column_info,
                texel: std::ptr::null(),
            },
        ];
        unsafe {
            (self.fns.update_desc)(self.device, 4, writes.as_ptr().cast(), 0, std::ptr::null());
        }
        Ok(())
    }

    fn write_buffer_handle(&self, bytes: &[u8]) -> Result<(), String> {
        let scene = {
            let targets = self.froxel.as_ref().ok_or("froxel targets missing")?;
            Buffer {
                buffer: targets.scene.buffer,
                memory: targets.scene.memory,
                size: targets.scene.size,
            }
        };
        self.write_buffer(&scene, bytes)
    }

    fn transition_froxel_image(&mut self, cmd: Handle, image: Handle) -> Result<(), String> {
        let ready = self
            .froxel
            .as_ref()
            .is_some_and(|targets| targets.image_ready);
        let old = if ready { 1 } else { 0 };
        self.image_barrier_cmd(cmd, image, old, 1, 0x800, 0x800, 0, 0x20 | 0x40);
        if let Some(targets) = self.froxel.as_mut() {
            targets.image_ready = true;
        }
        Ok(())
    }

    fn copy_extinction(&self, cmd: Handle, image: Handle, size: GridSize) -> Result<(), String> {
        let buffer = self
            .froxel
            .as_ref()
            .map_or(std::ptr::null_mut(), |targets| targets.ext_read.buffer);
        self.image_barrier_cmd(cmd, image, 1, 1, 0x800, 0x1000, 0x40, 0x800);
        #[repr(C)]
        struct Region {
            offset: u64,
            row: u32,
            height: u32,
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
            offset: 0,
            row: 0,
            height: 0,
            aspect: 1,
            mip: 0,
            layer: 0,
            layers: 1,
            x: 0,
            y: 0,
            z: 0,
            w: size.width,
            h: size.height,
            d: size.slices,
        };
        unsafe {
            (self.fns.cmd_copy_to_buffer)(
                cmd,
                image,
                1,
                buffer,
                1,
                std::ptr::from_ref(&region).cast(),
            );
        }
        self.image_barrier_cmd(cmd, image, 1, 1, 0x1000, 0x800, 0x800, 0x20);
        Ok(())
    }

    fn image_barrier_cmd(
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
                std::ptr::from_ref(&barrier).cast(),
            );
        }
    }

    fn dispatch_froxel(
        &self,
        cmd: Handle,
        layout: Handle,
        set: Handle,
        inject: Handle,
        integrate: Handle,
        width: u32,
        height: u32,
        slices: u32,
    ) {
        unsafe {
            (self.fns.cmd_bind_pipe)(cmd, 1, inject);
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_dispatch)(cmd, groups(width, 4), groups(height, 4), groups(slices, 4));
        }
        self.image_barrier_cmd(cmd, self.froxel_image(), 1, 1, 0x800, 0x800, 0x40, 0x20);
        unsafe {
            (self.fns.cmd_bind_pipe)(cmd, 1, integrate);
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_dispatch)(cmd, groups(width, 8), groups(height, 8), 1);
        }
    }

    fn froxel_image(&self) -> Handle {
        self.froxel
            .as_ref()
            .map_or(std::ptr::null_mut(), |targets| targets.image.image)
    }

    fn stamp_froxel(&self, cmd: Handle, query: Handle, index: u32, stage: u32) {
        if query.is_null() || self.timestamp_period <= 0.0 {
            return;
        }
        unsafe {
            if index == 0 {
                (self.fns.cmd_reset_query)(cmd, query, 0, 2);
            }
            (self.fns.cmd_write_timestamp)(cmd, stage, query, index);
        }
    }

    fn read_froxel_ms(&self) -> Option<f64> {
        let query = self.froxel.as_ref().map(|targets| targets.query)?;
        if query.is_null() || self.timestamp_period <= 0.0 {
            return None;
        }
        let mut raw = [0u64; 2];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                query,
                0,
                2,
                16,
                raw.as_mut_ptr().cast(),
                8,
                0x1 | 0x2,
            )
        };
        if got != 0 {
            return None;
        }
        let delta = tick_delta(raw[0], raw[1], self.timestamp_bits);
        if delta == 0 {
            return None;
        }
        Some(delta as f64 * f64::from(self.timestamp_period) / 1.0e6)
    }

    fn make_image_3d(&self, size: GridSize) -> Result<(u64, Image), String> {
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
            image_type: 2,
            format: 100,
            extent: Extent {
                w: size.width.max(1),
                h: size.height.max(1),
                d: size.slices.max(1),
            },
            mips: 1,
            layers: 1,
            samples: 1,
            tiling: 0,
            usage: 0x8 | 0x1,
            sharing: 0,
            queue_count: 0,
            queues: std::ptr::null(),
            initial: 0,
        };
        unsafe {
            let mut image = std::ptr::null_mut();
            check(
                (self.fns.create_image)(
                    self.device,
                    std::ptr::from_ref(&info).cast(),
                    std::ptr::null(),
                    &mut image,
                ),
                "froxel image",
            )?;
            let mut req = [0u8; 24];
            (self.fns.image_reqs)(self.device, image, req.as_mut_ptr());
            let bytes = u64::from_ne_bytes(req[0..8].try_into().unwrap_or([0; 8]));
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap_or([0; 4]));
            let memory = match self.alloc(bytes, bits, Memory::Device) {
                Ok(memory) => memory,
                Err(err) => {
                    (self.fns.destroy_image)(self.device, image, std::ptr::null());
                    return Err(err);
                }
            };
            if (self.fns.bind_image)(self.device, image, memory, 0) != 0 {
                (self.fns.free_mem)(self.device, memory, std::ptr::null());
                (self.fns.destroy_image)(self.device, image, std::ptr::null());
                return Err("bind froxel image".into());
            }
            let view = match self.make_view_3d(image) {
                Ok(view) => view,
                Err(err) => {
                    (self.fns.free_mem)(self.device, memory, std::ptr::null());
                    (self.fns.destroy_image)(self.device, image, std::ptr::null());
                    return Err(err);
                }
            };
            Ok((
                bytes,
                Image {
                    image,
                    view,
                    memory,
                },
            ))
        }
    }

    fn make_view_3d(&self, image: Handle) -> Result<Handle, String> {
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
            view_type: 2,
            format: 100,
            comp: Comp {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            range: Range {
                aspect: 1,
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
                    std::ptr::from_ref(&info).cast(),
                    std::ptr::null(),
                    &mut view,
                ),
                "froxel image view",
            )?;
        }
        Ok(view)
    }
}

impl FrameJob {
    fn clone_job(&self) -> Option<FrameJob> {
        Some(FrameJob {
            params: self.params.clone(),
            width: self.width,
            height: self.height,
            slices: self.slices,
            settings: self.settings.clone(),
            view: self.view,
            sun: self.sun,
            lamps: self.lamps.clone(),
        })
    }
}

trait CloneJob {
    fn clone_job(&self) -> Option<FrameJob>;
}

impl CloneJob for Option<FrameJob> {
    fn clone_job(&self) -> Option<FrameJob> {
        self.as_ref().and_then(FrameJob::clone_job)
    }
}

#[repr(C)]
struct CopyRegion {
    src: u64,
    dst: u64,
    size: u64,
}

fn groups(n: u32, local: u32) -> u32 {
    n.saturating_add(local.saturating_sub(1)) / local.max(1)
}

fn column_bytes(width: u32, height: u32) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(16)
}

fn columns_fit(width: u32, height: u32) -> bool {
    u64::from(width).saturating_mul(u64::from(height)) <= MAX_COLUMNS
}

fn tail_bytes() -> u64 {
    16_u64.saturating_add(MAX_COLUMNS.saturating_mul(16))
}

fn scene_offset() -> u64 {
    u64::try_from(pack::SCENE_CAPACITY)
        .unwrap_or(0)
        .saturating_sub(tail_bytes())
}

fn tail_word() -> u32 {
    let offset = scene_offset();
    let tail = u64::try_from(SCENE_TAIL).unwrap_or(0);
    u32::try_from(offset.saturating_sub(tail) / 16).unwrap_or(0)
}

fn column_header(width: u32, height: u32, slices: u32) -> [u8; 16] {
    let mut head = [0u8; 16];
    head[0..4].copy_from_slice(&width.to_ne_bytes());
    head[4..8].copy_from_slice(&height.to_ne_bytes());
    head[8..12].copy_from_slice(&slices.to_ne_bytes());
    head[12..16].copy_from_slice(&1u32.to_ne_bytes());
    head
}

fn design_bytes(size: GridSize) -> u64 {
    let width = u64::from(size.width);
    let height = u64::from(size.height);
    let slices = u64::from(size.slices);
    width
        .saturating_mul(height)
        .saturating_mul(slices)
        .saturating_mul(16)
        .saturating_add(width.saturating_mul(height).saturating_mul(8))
}

fn preset_of(size: GridSize) -> LightingPreset {
    match (size.width, size.height, size.slices) {
        (96, 54, 32) => LightingPreset::Medium,
        (160, 90, 48) => LightingPreset::High,
        _ => LightingPreset::Low,
    }
}

fn grid_for_samples(preset: LightingPreset, samples: u32) -> GridSize {
    let full = grid_size(preset);
    let cap = crate::fog_budget::sample_cap(preset).max(1);
    if samples == 0 || full.slices == 0 {
        return GridSize {
            width: 0,
            height: 0,
            slices: 0,
        };
    }
    let cells = full.cells().max(1);
    let mut target = cells.saturating_mul(usize::try_from(samples).unwrap_or(0))
        / usize::try_from(cap).unwrap_or(1);
    // One sample is a trivial grid. The proportional count is still tens of cells.
    if samples <= 1 {
        target = target.min(4);
    }
    let target = target.max(1);
    let mut width = full.width.max(1);
    let mut height = full.height.max(1);
    let mut slices = full.slices.max(1);
    while usize::try_from(width).unwrap_or(1)
        * usize::try_from(height).unwrap_or(1)
        * usize::try_from(slices).unwrap_or(1)
        > target
        && slices > 1
    {
        slices -= 1;
    }
    while usize::try_from(width).unwrap_or(1)
        * usize::try_from(height).unwrap_or(1)
        * usize::try_from(slices).unwrap_or(1)
        > target
        && height > 1
    {
        height -= 1;
    }
    while usize::try_from(width).unwrap_or(1)
        * usize::try_from(height).unwrap_or(1)
        * usize::try_from(slices).unwrap_or(1)
        > target
        && width > 1
    {
        width -= 1;
    }
    GridSize {
        width,
        height,
        slices,
    }
}

fn height_settings() -> FogSettings {
    FogSettings {
        enabled: true,
        density: 0.08,
        height: 0.0,
        thickness: 40.0,
        albedo: 1.0,
        phase_g: 0.4,
        ..FogSettings::default()
    }
}

fn empty_scene() -> Scene {
    Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 30.0,
            half_z: 30.0,
            color: [0.4, 0.4, 0.4],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: FogSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    }
}

fn columns_from(floats: &[f32]) -> Vec<FogColumn> {
    let mut columns = Vec::with_capacity(floats.len() / 4);
    for chunk in floats.chunks_exact(4) {
        columns.push(FogColumn {
            inscatter: [chunk[0], chunk[1], chunk[2]],
            transmittance: chunk[3],
        });
    }
    columns
}

fn sun_and_lamps(lights: &[Light]) -> (DirectSun, Vec<FogPointLamp>) {
    let mut sun = default_frame_sun();
    let mut lamps = Vec::new();
    let mut sun_set = false;
    for light in lights {
        let span = light.direction.x * light.direction.x
            + light.direction.y * light.direction.y
            + light.direction.z * light.direction.z;
        if span > 1.0e-8 {
            if !sun_set {
                let len = span.sqrt();
                sun = DirectSun {
                    toward: [
                        -light.direction.x / len,
                        -light.direction.y / len,
                        -light.direction.z / len,
                    ],
                    color: light.color,
                };
                sun_set = true;
            }
        } else if lamps.len() < MAX_LAMPS {
            lamps.push(FogPointLamp {
                position: [light.position.x, light.position.y, light.position.z],
                color: light.color,
            });
        }
    }
    (sun, lamps)
}

fn encode_params(
    settings: &FogSettings,
    view: &FogView,
    size: GridSize,
    sun: &DirectSun,
    lamps: &[FogPointLamp],
    flags: u32,
    stop_slices: u32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PARAM_BYTES);
    push_u32(&mut bytes, size.width);
    push_u32(&mut bytes, size.height);
    push_u32(&mut bytes, size.slices);
    push_u32(&mut bytes, flags);
    push_f32(&mut bytes, view.origin[0]);
    push_f32(&mut bytes, view.origin[1]);
    push_f32(&mut bytes, view.origin[2]);
    push_f32(&mut bytes, 0.0);
    push_f32(&mut bytes, view.forward[0]);
    push_f32(&mut bytes, view.forward[1]);
    push_f32(&mut bytes, view.forward[2]);
    push_f32(&mut bytes, view.tan_half_x);
    push_f32(&mut bytes, view.right[0]);
    push_f32(&mut bytes, view.right[1]);
    push_f32(&mut bytes, view.right[2]);
    push_f32(&mut bytes, view.tan_half_y);
    push_f32(&mut bytes, view.up[0]);
    push_f32(&mut bytes, view.up[1]);
    push_f32(&mut bytes, view.up[2]);
    push_f32(&mut bytes, 0.0);
    push_f32(&mut bytes, sun.toward[0]);
    push_f32(&mut bytes, sun.toward[1]);
    push_f32(&mut bytes, sun.toward[2]);
    push_f32(&mut bytes, 0.0);
    push_f32(&mut bytes, sun.color[0]);
    push_f32(&mut bytes, sun.color[1]);
    push_f32(&mut bytes, sun.color[2]);
    push_f32(&mut bytes, settings.albedo.clamp(0.0, 1.0));
    push_f32(&mut bytes, settings.density);
    push_f32(&mut bytes, settings.height);
    push_f32(&mut bytes, settings.thickness);
    push_f32(&mut bytes, settings.phase_g);
    let volumes = settings.volumes.len().min(MAX_VOLUMES);
    let lamp_count = lamps.len().min(MAX_LAMPS);
    push_u32(&mut bytes, u32::try_from(volumes).unwrap_or(0));
    push_u32(&mut bytes, u32::try_from(lamp_count).unwrap_or(0));
    push_u32(&mut bytes, stop_slices);
    push_u32(&mut bytes, 0);
    for volume in settings.volumes.iter().take(MAX_VOLUMES) {
        push_f32(&mut bytes, volume.position[0]);
        push_f32(&mut bytes, volume.position[1]);
        push_f32(&mut bytes, volume.position[2]);
        push_f32(
            &mut bytes,
            if volume.shape == FogShape::Sphere {
                1.0
            } else {
                0.0
            },
        );
        push_f32(&mut bytes, volume.extents[0]);
        push_f32(&mut bytes, volume.extents[1]);
        push_f32(&mut bytes, volume.extents[2]);
        push_f32(&mut bytes, volume.density);
        push_f32(&mut bytes, volume.albedo);
        push_f32(&mut bytes, volume.phase_g);
        push_f32(&mut bytes, volume.falloff);
        push_f32(&mut bytes, 0.0);
    }
    for _ in volumes..MAX_VOLUMES {
        for _ in 0..12 {
            push_f32(&mut bytes, 0.0);
        }
    }
    for lamp in lamps.iter().take(MAX_LAMPS) {
        push_f32(&mut bytes, lamp.position[0]);
        push_f32(&mut bytes, lamp.position[1]);
        push_f32(&mut bytes, lamp.position[2]);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, lamp.color[0]);
        push_f32(&mut bytes, lamp.color[1]);
        push_f32(&mut bytes, lamp.color[2]);
        push_f32(&mut bytes, 0.0);
    }
    for _ in lamp_count..MAX_LAMPS {
        for _ in 0..8 {
            push_f32(&mut bytes, 0.0);
        }
    }
    debug_assert_eq!(bytes.len(), PARAM_BYTES);
    bytes
}

fn scene_bytes_for(job: &FogDispatch) -> Vec<u8> {
    let Some(scene) = job.scene.as_ref() else {
        return vec![0u8; SCENE_TAIL];
    };
    let world = crate::world::World::from_scene(scene.clone());
    let mut memory = crate::budget::FrameMemory::default();
    let mut pack = pack::pack_frame(
        &world,
        &crate::world::identity_pose(),
        job.view.origin,
        &mut memory,
    );
    pack.eye = job.view.origin;
    pack.view_right = job.view.right;
    pack.view_up = job.view.up;
    pack.view_forward = job.view.forward;
    pack.view_tan = job.view.tan_half_y;
    pack.view_aspect = if job.view.tan_half_y.abs() > 1.0e-6 {
        job.view.tan_half_x / job.view.tan_half_y
    } else {
        1.0
    };
    pack::scene_bytes(&pack)
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_ne_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_high_grid_fits_in_the_scene_tail() {
        let offset = scene_offset();
        let tail = tail_bytes();
        let cap = u64::try_from(pack::SCENE_CAPACITY).unwrap_or(0);
        assert_eq!(offset + tail, cap);
        assert_eq!(offset % 16, 0);
        assert!(tail_word() > 0);
        assert!(columns_fit(160, 90));
    }

    #[test]
    fn a_full_sample_cap_keeps_the_preset_grid() {
        let high = grid_for_samples(LightingPreset::High, 32_000);
        assert_eq!((high.width, high.height, high.slices), (160, 90, 48));
        let one = grid_for_samples(LightingPreset::High, 1);
        assert!(one.width * one.height * one.slices <= 4, "{one:?}");
        assert_eq!(grid_for_samples(LightingPreset::Potato, 4).slices, 0);
    }
}
