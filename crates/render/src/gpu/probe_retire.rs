//! Dispatch the production probe passes on a planted screen.
//!
//! `probe_retire_prod.comp` writes the G-buffer, the cells, and the change-block
//! head. The command buffer then runs `lighting_place.comp`, `lighting_trace.comp`,
//! `lighting_gather.comp`, and `lighting_filter.comp`. A readback phase records
//! what those passes wrote. One writing specialization. The read-only compose
//! is not in this module.

use std::ffi::c_void;

use genos_scene::{LightingPreset, LightingSettings};

use super::{check, read_f32s, Buffer, Gpu, Handle, Memory};

const SLOTS: u64 = 1_048_576;
const CACHE_BYTES: u64 = SLOTS * 392;
const WORK_BYTES: u64 = (98_304 * 16 + 32_768) * 16;
const OUT_FLOATS: usize = 392;
const HEAD_SRC: usize = 360;
const PC_BYTES: u32 = 48;
const HEAD_WORDS: usize = 8;
const HEAD_VARIANTS: usize = 4;
const ROWS: u32 = 18;
const REAPPEAR: u32 = 14;
/// Mode 1 (probe rays only) and the quiet bit. Filter round and gather-pick stay clear.
const PARAMS_Z: u32 = (1 << 8) | (1 << 10);
const FRAME: u32 = 100;
const DIMS: [u32; 4] = [8, 4, 2, 1];

pub(super) struct Pipe {
    scene: Buffer,
    mesh: Buffer,
    gbuffer: Buffer,
    work: Buffer,
    out: Buffer,
    keys: Buffer,
    cache: Buffer,
    stats: Buffer,
    desc_layout: Handle,
    desc_pool: Handle,
    set: Handle,
    layout: Handle,
    plant: Handle,
    place: Handle,
    trace: Handle,
    gather: Handle,
    filter: Handle,
    cmd: Handle,
    fence: Handle,
}

fn ckeys_bytes() -> u64 {
    let compose_at = 2 * SLOTS
        + 2
        + 17_600
        + 16
        + 3 * 65_536
        + 1024
        + 32_768
        + 16
        + 5 * SLOTS
        + 5 * 65_537
        + 6 * 65_536
        + 16;
    (compose_at + 4 + 2 * 65_535 + 5 * SLOTS) * 4
}

fn retire_heads() -> [LightingSettings; HEAD_VARIANTS] {
    let mut both = LightingSettings::preset(LightingPreset::High);
    both.surface_cache.sh_cells = true;
    both.surface_cache.sh_texel_detail = true;
    both.surface_cache.sh_probe_retire = true;
    both.sanitize();
    let mut off = both.clone();
    off.surface_cache.sh_probe_retire = false;
    off.sanitize();
    let mut detail_off = both.clone();
    detail_off.surface_cache.sh_texel_detail = false;
    detail_off.sanitize();
    let mut low = LightingSettings::preset(LightingPreset::Low);
    low.surface_cache.sh_cells = true;
    low.surface_cache.sh_texel_detail = true;
    low.surface_cache.sh_probe_retire = true;
    low.sanitize();
    [both, off, detail_off, low]
}

impl Gpu {
    pub(super) fn probe_retire(&mut self) -> Result<Vec<f32>, String> {
        super::texel_detail::release_compiler_heap();
        if self.probe_retire.is_none() {
            self.probe_retire = Some(self.create_probe_retire_pipe()?);
        }
        super::texel_detail::release_compiler_heap();
        self.zero_probe_retire()?;
        let (out_mem, scene_mem) = {
            let pipe = self
                .probe_retire
                .as_ref()
                .ok_or("probe retire check is not created")?;
            (pipe.out.memory, pipe.scene.memory)
        };
        self.write_probe_retire_heads(out_mem)?;
        self.write_probe_retire_scene(scene_mem)?;
        self.submit_probe_retire()?;
        self.read_probe_retire()
    }

    pub(super) fn destroy_probe_retire(&mut self) {
        if let Some(pipe) = self.probe_retire.take() {
            self.destroy_probe_retire_pipe(pipe);
        }
    }

    fn destroy_probe_retire_pipe(&mut self, mut pipe: Pipe) {
        unsafe {
            if !pipe.fence.is_null() {
                (self.fns.destroy_fence)(self.device, pipe.fence, std::ptr::null());
            }
            for handle in [pipe.plant, pipe.place, pipe.trace, pipe.gather, pipe.filter] {
                if !handle.is_null() {
                    (self.fns.destroy_pipeline)(self.device, handle, std::ptr::null());
                }
            }
            if !pipe.layout.is_null() {
                (self.fns.destroy_layout)(self.device, pipe.layout, std::ptr::null());
            }
            if !pipe.desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, pipe.desc_pool, std::ptr::null());
            }
            if !pipe.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, pipe.desc_layout, std::ptr::null());
            }
        }
        self.destroy_buffer(&mut pipe.scene);
        self.destroy_buffer(&mut pipe.mesh);
        self.destroy_buffer(&mut pipe.gbuffer);
        self.destroy_buffer(&mut pipe.work);
        self.destroy_buffer(&mut pipe.out);
        self.destroy_buffer(&mut pipe.keys);
        self.destroy_buffer(&mut pipe.cache);
        self.destroy_buffer(&mut pipe.stats);
    }

    fn create_probe_retire_pipe(&mut self) -> Result<Pipe, String> {
        let scene = self.make_buffer(256, 0x20, Memory::Readback)?;
        let mesh = self.make_buffer(256, 0x20, Memory::Readback)?;
        let gbuffer = self.make_buffer(4096, 0x20, Memory::Readback)?;
        let work = self.make_buffer(WORK_BYTES, 0x20, Memory::Readback)?;
        let out_bytes = u64::try_from(OUT_FLOATS * 4)
            .map_err(|err| format!("probe retire output size: {err}"))?;
        let out = self.make_buffer(out_bytes, 0x20, Memory::Readback)?;
        let keys = self.make_buffer(ckeys_bytes(), 0x20, Memory::Readback)?;
        let cache = self.make_buffer(CACHE_BYTES, 0x20, Memory::Readback)?;
        let stats = self.make_buffer(4096, 0x20, Memory::Readback)?;
        let (desc_layout, layout) = self.probe_retire_layout()?;
        super::texel_detail::release_compiler_heap();
        let plant = self.probe_retire_compute(layout, super::PROBE_RETIRE_SPV, "plant")?;
        super::texel_detail::release_compiler_heap();
        let place = self.probe_retire_compute(layout, super::LIGHTING_PLACE_SPV, "place")?;
        super::texel_detail::release_compiler_heap();
        let trace = self.probe_retire_compute(layout, super::LIGHTING_TRACE_SPV, "trace")?;
        super::texel_detail::release_compiler_heap();
        let gather = self.probe_retire_compute(layout, super::LIGHTING_GATHER_SPV, "gather")?;
        super::texel_detail::release_compiler_heap();
        let filter = self.probe_retire_compute(layout, super::LIGHTING_FILTER_SPV, "filter")?;
        super::texel_detail::release_compiler_heap();
        let (desc_pool, set, cmd, fence) = self.probe_retire_commands(desc_layout)?;
        let built = Pipe {
            scene,
            mesh,
            gbuffer,
            work,
            out,
            keys,
            cache,
            stats,
            desc_layout,
            desc_pool,
            set,
            layout,
            plant,
            place,
            trace,
            gather,
            filter,
            cmd,
            fence,
        };
        self.write_probe_retire_set(&built)?;
        Ok(built)
    }

    fn probe_retire_layout(&self) -> Result<(Handle, Handle), String> {
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
        let bind_at = [0, 5, 6, 7, 8, 9, 10, 11];
        let bindings: Vec<Binding> = bind_at
            .into_iter()
            .map(|binding| Binding {
                binding,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            })
            .collect();
        let binding_count = u32::try_from(bindings.len())
            .map_err(|err| format!("probe retire binding count: {err}"))?;
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: binding_count,
            bindings: bindings.as_ptr(),
        };
        let mut desc_layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut desc_layout,
                ),
                "probe retire descriptor layout",
            )?;
        }
        #[repr(C)]
        struct Range {
            stage: u32,
            offset: u32,
            size: u32,
        }
        #[repr(C)]
        struct LayoutInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            set_count: u32,
            sets: *const Handle,
            push_count: u32,
            push: *const Range,
        }
        let sets = [desc_layout];
        let push = Range {
            stage: 0x20,
            offset: 0,
            size: PC_BYTES,
        };
        let layout_info = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr(),
            push_count: 1,
            push: &push,
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout_info as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "probe retire pipeline layout",
            )?;
        }
        Ok((desc_layout, layout))
    }

    fn probe_retire_compute(
        &self,
        layout: Handle,
        code: &[u8],
        what: &str,
    ) -> Result<Handle, String> {
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
                name: b"main\0".as_ptr().cast(),
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
                &info as *const PipeInfo as *const u8,
                std::ptr::null(),
                &mut pipe,
            );
            check(result, &format!("probe retire {what} pipeline"))?;
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok(pipe)
    }

    fn probe_retire_commands(
        &self,
        desc_layout: Handle,
    ) -> Result<(Handle, Handle, Handle, Handle), String> {
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
        let size = Size { kind: 7, count: 8 };
        let pool_info = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            size_count: 1,
            sizes: &size,
        };
        let mut desc_pool = std::ptr::null_mut();
        let mut set = std::ptr::null_mut();
        let mut cmd = std::ptr::null_mut();
        let mut fence = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool_info as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut desc_pool,
                ),
                "probe retire descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: desc_pool,
                count: 1,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(self.device, &alloc as *const Alloc as *const u8, &mut set),
                "probe retire descriptor set",
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
                    &mut cmd,
                ),
                "probe retire command buffer",
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
                    &fence_info as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut fence,
                ),
                "probe retire fence",
            )?;
        }
        Ok((desc_pool, set, cmd, fence))
    }

    fn write_probe_retire_set(&self, pipe: &Pipe) -> Result<(), String> {
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
        let buffers = [
            &pipe.scene,
            &pipe.mesh,
            &pipe.gbuffer,
            &pipe.work,
            &pipe.out,
            &pipe.keys,
            &pipe.cache,
            &pipe.stats,
        ];
        let bind_at = [0, 5, 6, 7, 8, 9, 10, 11];
        let infos: Vec<BufInfo> = buffers
            .iter()
            .map(|buffer| BufInfo {
                buffer: buffer.buffer,
                offset: 0,
                range: u64::MAX,
            })
            .collect();
        let writes: Vec<Write> = bind_at
            .into_iter()
            .zip(infos.iter())
            .map(|(binding, info)| Write {
                s_type: 35,
                next: std::ptr::null(),
                set: pipe.set,
                binding,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: info,
                texel: std::ptr::null(),
            })
            .collect();
        let write_count = u32::try_from(writes.len())
            .map_err(|err| format!("probe retire descriptor writes: {err}"))?;
        unsafe {
            (self.fns.update_desc)(
                self.device,
                write_count,
                writes.as_ptr().cast(),
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn zero_probe_retire(&self) -> Result<(), String> {
        let pipe = self
            .probe_retire
            .as_ref()
            .ok_or("probe retire check is not created")?;
        let buffers = [
            (pipe.scene.memory, pipe.scene.size),
            (pipe.mesh.memory, pipe.mesh.size),
            (pipe.gbuffer.memory, pipe.gbuffer.size),
            (pipe.work.memory, pipe.work.size),
            (pipe.out.memory, pipe.out.size),
            (pipe.keys.memory, pipe.keys.size),
            (pipe.cache.memory, pipe.cache.size),
            (pipe.stats.memory, pipe.stats.size),
        ];
        for (memory, size) in buffers {
            self.zero_probe_retire_memory(memory, size)?;
        }
        Ok(())
    }

    fn zero_probe_retire_memory(&self, memory: Handle, size: u64) -> Result<(), String> {
        let bytes =
            usize::try_from(size).map_err(|err| format!("probe retire buffer size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, size, 0, &mut mapped),
                "map probe retire buffer",
            )?;
            std::ptr::write_bytes(mapped.cast::<u8>(), 0, bytes);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn write_probe_retire_heads(&self, memory: Handle) -> Result<(), String> {
        let mut words = [0u32; HEAD_WORDS * HEAD_VARIANTS];
        for (i, settings) in retire_heads().into_iter().enumerate() {
            let head = super::lighting_change_head(&settings);
            let at = i * HEAD_WORDS;
            words[at] = 0;
            words[at + 1] = 1.0f32.to_bits();
            words[at + 2] = ((2 * head.flags) as f32).to_bits();
            words[at + 3] = (head.mode_cap as f32).to_bits();
            words[at + 7] = (head.head_hi as f32).to_bits();
        }
        self.write_probe_retire_bytes(memory, HEAD_SRC * 4, &words)
    }

    fn write_probe_retire_scene(&self, memory: Handle) -> Result<(), String> {
        let mut words = [0u32; 40];
        words[4] = 1.17f32.to_bits();
        words[5] = 4.375f32.to_bits();
        words[6] = 4.19f32.to_bits();
        words[24] = 1.0f32.to_bits();
        words[27] = 1.0e-6f32.to_bits();
        words[30] = 1.0f32.to_bits();
        words[33] = (-1.0f32).to_bits();
        words[35] = 1.0f32.to_bits();
        self.write_probe_retire_bytes(memory, 0, &words)
    }

    fn write_probe_retire_bytes<const N: usize>(
        &self,
        memory: Handle,
        offset: usize,
        words: &[u32; N],
    ) -> Result<(), String> {
        let bytes = std::mem::size_of_val(words);
        let offset_u =
            u64::try_from(offset).map_err(|err| format!("probe retire map offset: {err}"))?;
        let span = offset_u
            + u64::try_from(bytes).map_err(|err| format!("probe retire map size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, span, 0, &mut mapped),
                "map probe retire words",
            )?;
            std::ptr::copy_nonoverlapping(
                words.as_ptr().cast::<u8>(),
                mapped.cast::<u8>().add(offset),
                bytes,
            );
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn read_probe_retire(&self) -> Result<Vec<f32>, String> {
        let out_mem = self
            .probe_retire
            .as_ref()
            .ok_or("probe retire check is not created")?
            .out
            .memory;
        read_f32s(
            self.fns.map_mem,
            self.fns.unmap_mem,
            self.device,
            out_mem,
            OUT_FLOATS,
        )
    }

    fn submit_probe_retire(&mut self) -> Result<(), String> {
        let (plant, place, trace, gather, filter, layout, set, cmd, fence) = {
            let Some(state) = self.probe_retire.as_ref() else {
                return Err("probe retire check is not created".into());
            };
            (
                state.plant,
                state.place,
                state.trace,
                state.gather,
                state.filter,
                state.layout,
                state.set,
                state.cmd,
                state.fence,
            )
        };
        self.wait_fence(fence, "probe retire wait")?;
        let fences = [fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "probe retire reset fence",
            )?;
            check((self.fns.reset_cmd)(cmd, 0), "probe retire reset")?;
            self.begin_reused(cmd)?;
            self.probe_retire_barrier(cmd, 0x4000, 0x800, 0x4000, 0x20);
            for row in 0..ROWS {
                self.probe_retire_pass(cmd, layout, set, plant, row, 0, 1, 1, 1);
                self.probe_retire_passes(cmd, layout, set, place, trace, gather, filter);
                self.probe_retire_pass(cmd, layout, set, plant, row, 1, 1, 1, 1);
                if row == REAPPEAR {
                    self.probe_retire_pass(cmd, layout, set, plant, row, 2, 1, 1, 1);
                    self.probe_retire_passes(cmd, layout, set, place, trace, gather, filter);
                    self.probe_retire_pass(cmd, layout, set, plant, row, 1, 1, 1, 1);
                }
            }
            self.probe_retire_barrier(cmd, 0x800, 0x4000, 0x40, 0x2000);
            check((self.fns.end_cmd)(cmd), "probe retire end")?;
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
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    fence,
                ),
                "probe retire submit",
            )?;
        }
        self.wait_fence(fence, "probe retire done")
    }

    fn probe_retire_passes(
        &self,
        cmd: Handle,
        layout: Handle,
        set: Handle,
        place: Handle,
        trace: Handle,
        gather: Handle,
        filter: Handle,
    ) {
        // The row is already in the push constants from the plant dispatch.
        // These passes ignore lamp. Place is one group, trace is one group in
        // mode 1, gather is one group per probe, filter is one group.
        self.probe_retire_bind(cmd, layout, set, place, 1, 1, 1);
        self.probe_retire_bind(cmd, layout, set, trace, 1, 1, 1);
        self.probe_retire_bind(cmd, layout, set, gather, 2, 1, 1);
        self.probe_retire_bind(cmd, layout, set, filter, 1, 1, 1);
    }

    fn probe_retire_pass(
        &self,
        cmd: Handle,
        layout: Handle,
        set: Handle,
        pipe: Handle,
        row: u32,
        phase: u32,
        x: u32,
        y: u32,
        z: u32,
    ) {
        unsafe {
            (self.fns.cmd_bind_pipe)(cmd, 1, pipe);
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            #[repr(C)]
            struct Pc {
                dims: [u32; 4],
                params: [u32; 4],
                lamp: [u32; 4],
            }
            let pc = Pc {
                dims: DIMS,
                params: [4, 4, PARAMS_Z, FRAME],
                lamp: [row, phase, 0, 0],
            };
            (self.fns.cmd_push)(
                cmd,
                layout,
                0x20,
                0,
                PC_BYTES,
                std::ptr::from_ref(&pc).cast(),
            );
            (self.fns.cmd_dispatch)(cmd, x, y, z);
        }
        self.probe_retire_barrier(cmd, 0x800, 0x800, 0x40, 0x60);
    }

    fn probe_retire_bind(
        &self,
        cmd: Handle,
        layout: Handle,
        set: Handle,
        pipe: Handle,
        x: u32,
        y: u32,
        z: u32,
    ) {
        unsafe {
            (self.fns.cmd_bind_pipe)(cmd, 1, pipe);
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_dispatch)(cmd, x, y, z);
        }
        self.probe_retire_barrier(cmd, 0x800, 0x800, 0x40, 0x60);
    }

    fn probe_retire_barrier(
        &self,
        cmd: Handle,
        src_stage: u32,
        dst_stage: u32,
        src: u32,
        dst: u32,
    ) {
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
            src_access: src,
            dst_access: dst,
        };
        unsafe {
            (self.fns.cmd_barrier)(
                cmd,
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
}
