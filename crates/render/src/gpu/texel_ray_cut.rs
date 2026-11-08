//! Dispatch `texel_ray_cut_prod.comp`.
//!
//! The shader includes `lighting_common.glsl` and calls
//! `lighting_compact_hold_texel_rays`, the function `lighting_compact.comp`
//! calls. One writing specialization. The buffers are the production cache,
//! key table, and work block. The screen-probe reader is not in this module.

use std::ffi::c_void;

use super::{check, read_f32s, Buffer, Gpu, Handle, Memory};

const SLOTS: u64 = 1_048_576;
const CACHE_BYTES: u64 = SLOTS * 392;
const WORK_BYTES: u64 = (98_304 * 16 + 64) * 16;
const ROWS: usize = 26;
const STRIDE: usize = 8;
const HEAD: usize = 8;
const TAIL: usize = 8;
const AGREE_FLOATS: usize = HEAD + ROWS * STRIDE + TAIL;
const LAMP_ROWS: usize = 3;
const LAMP_FLOATS: usize = HEAD + LAMP_ROWS * STRIDE;
const CMP_N: usize = 7;
const CMP_LISTS: usize = 6;
const CMP_BASE: usize = 16;
const CMP_BLOCK: usize = CMP_N * CMP_LISTS + 4;
const CMP_FLOATS: usize = CMP_BASE + 2 * CMP_BLOCK;
const OUT_FLOATS: usize = AGREE_FLOATS;
const PC_BYTES: u32 = 48;
const HEAD_WORDS: usize = 8;
const HEAD_VARIANTS: usize = 4;
/// `params.z`: bit 5 urgent, bit 10 light-quiet. The move row writes the only
/// change sphere. Quiet keeps a lamp ring from counting as a lamp touch.
const PARAMS_Z: u32 = (1 << 5) | (1 << 10);
const FRAME: u32 = 100;
/// Reaching-lamp rows. Bit 10 stays clear. The streak is the settle cap.
const LAMP_Z: u32 = 1 << 5;
const LAMP_FRAME: u32 = 200;
/// Production compact. Quiet so a settled interval is the slow rate.
const COMPACT_Z: u32 = 1 << 10;
const COMPACT_FRAME: u32 = 1000;
const COMPACT_GROUPS: u32 = 1_048_576 / 64;

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
    pipe: Handle,
    compact: Handle,
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

impl Gpu {
    pub(super) fn texel_ray_cut(&mut self) -> Result<Vec<f32>, String> {
        super::texel_detail::release_compiler_heap();
        if self.texel_ray_cut.is_none() {
            self.texel_ray_cut = Some(self.create_texel_ray_cut_pipe()?);
        }
        super::texel_detail::release_compiler_heap();
        let mut out = Vec::with_capacity(AGREE_FLOATS + LAMP_FLOATS + CMP_FLOATS);
        self.zero_texel_ray_cut()?;
        self.write_texel_ray_cut_heads_now()?;
        self.submit_texel_ray_cut(&[Step {
            compact: false,
            groups: 1,
            params_z: PARAMS_Z,
            frame: FRAME,
            lamp_x: 0,
            lamp_y: 0,
        }])?;
        let agree = self.read_texel_ray_cut()?;
        out.extend_from_slice(&agree[..AGREE_FLOATS]);
        self.zero_texel_ray_cut()?;
        self.write_texel_ray_cut_heads_now()?;
        self.submit_texel_ray_cut(&[Step {
            compact: false,
            groups: 1,
            params_z: LAMP_Z,
            frame: LAMP_FRAME,
            lamp_x: 1,
            lamp_y: 0,
        }])?;
        let lamp = self.read_texel_ray_cut()?;
        out.extend_from_slice(&lamp[..LAMP_FLOATS]);
        self.zero_texel_ray_cut()?;
        self.write_texel_ray_cut_heads_now()?;
        self.submit_texel_ray_cut(&[
            Step {
                compact: false,
                groups: 1,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 2,
                lamp_y: 0,
            },
            Step {
                compact: true,
                groups: COMPACT_GROUPS,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 0,
                lamp_y: 0,
            },
            Step {
                compact: false,
                groups: 1,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 3,
                lamp_y: 0,
            },
            Step {
                compact: false,
                groups: 1,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 4,
                lamp_y: 0,
            },
            Step {
                compact: true,
                groups: COMPACT_GROUPS,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 0,
                lamp_y: 0,
            },
            Step {
                compact: false,
                groups: 1,
                params_z: COMPACT_Z,
                frame: COMPACT_FRAME,
                lamp_x: 3,
                lamp_y: 1,
            },
        ])?;
        let compact = self.read_texel_ray_cut()?;
        out.extend_from_slice(&compact[..CMP_FLOATS]);
        Ok(out)
    }

    fn write_texel_ray_cut_heads_now(&self) -> Result<(), String> {
        let keys = self
            .texel_ray_cut
            .as_ref()
            .ok_or("texel ray cut check is not created")?
            .keys
            .memory;
        self.write_texel_ray_cut_heads(keys)
    }

    pub(super) fn destroy_texel_ray_cut(&mut self) {
        if let Some(pipe) = self.texel_ray_cut.take() {
            self.destroy_texel_ray_cut_pipe(pipe);
        }
    }

    fn destroy_texel_ray_cut_pipe(&mut self, mut pipe: Pipe) {
        unsafe {
            if !pipe.fence.is_null() {
                (self.fns.destroy_fence)(self.device, pipe.fence, std::ptr::null());
            }
            if !pipe.pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, pipe.pipe, std::ptr::null());
            }
            if !pipe.compact.is_null() {
                (self.fns.destroy_pipeline)(self.device, pipe.compact, std::ptr::null());
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

    fn create_texel_ray_cut_pipe(&mut self) -> Result<Pipe, String> {
        let scene = self.make_buffer(256, 0x20, Memory::Readback)?;
        let mesh = self.make_buffer(256, 0x20, Memory::Readback)?;
        let gbuffer = self.make_buffer(256, 0x20, Memory::Readback)?;
        let work = self.make_buffer(WORK_BYTES, 0x20, Memory::Readback)?;
        let out_bytes = u64::try_from(OUT_FLOATS * 4)
            .map_err(|err| format!("texel ray cut output size: {err}"))?;
        let out = self.make_buffer(out_bytes, 0x20, Memory::Readback)?;
        let keys = self.make_buffer(ckeys_bytes(), 0x20, Memory::Readback)?;
        let cache = self.make_buffer(CACHE_BYTES, 0x20, Memory::Readback)?;
        let stats = self.make_buffer(4096, 0x20, Memory::Readback)?;
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
            .map_err(|err| format!("texel ray cut binding count: {err}"))?;
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
                "texel ray cut descriptor layout",
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
                "texel ray cut pipeline layout",
            )?;
        }
        let code_pipe = self.texel_ray_cut_compute(layout, super::TEXEL_RAY_CUT_SPV, "texel ray cut")?;
        let compact_pipe =
            self.texel_ray_cut_compute(layout, super::LIGHTING_COMPACT_SPV, "lighting compact")?;
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
            count: binding_count,
        };
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
                "texel ray cut descriptor pool",
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
                pool: desc_pool,
                count: 1,
                layouts: sets.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(self.device, &alloc as *const Alloc as *const u8, &mut set),
                "texel ray cut descriptor set",
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
                "texel ray cut command buffer",
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
                "texel ray cut fence",
            )?;
        }
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
            pipe: code_pipe,
            compact: compact_pipe,
            cmd,
            fence,
        };
        self.write_texel_ray_cut_set(&built)?;
        Ok(built)
    }

    fn texel_ray_cut_compute(
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
            check(result, what)?;
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
        }
        Ok(pipe)
    }

    fn write_texel_ray_cut_set(&self, pipe: &Pipe) -> Result<(), String> {
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
            .map_err(|err| format!("texel ray cut descriptor writes: {err}"))?;
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

    fn write_texel_ray_cut_heads(&self, memory: Handle) -> Result<(), String> {
        let mut words = [0u32; HEAD_WORDS * HEAD_VARIANTS];
        for (i, settings) in ray_cut_heads().into_iter().enumerate() {
            let head = super::lighting_change_head(&settings);
            let at = i * HEAD_WORDS;
            words[at] = 0;
            words[at + 1] = 1.0f32.to_bits();
            words[at + 2] = ((2 * head.flags) as f32).to_bits();
            words[at + 3] = (head.mode_cap as f32).to_bits();
            words[at + 7] = (head.head_hi as f32).to_bits();
        }
        let bytes = u64::try_from(std::mem::size_of_val(&words))
            .map_err(|err| format!("texel ray cut head size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, bytes, 0, &mut mapped),
                "map texel ray cut heads",
            )?;
            std::ptr::copy_nonoverlapping(
                words.as_ptr().cast::<u8>(),
                mapped.cast::<u8>(),
                usize::try_from(bytes).map_err(|err| format!("texel ray cut head copy: {err}"))?,
            );
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn zero_texel_ray_cut(&self) -> Result<(), String> {
        let pipe = self
            .texel_ray_cut
            .as_ref()
            .ok_or("texel ray cut check is not created")?;
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
            self.zero_ray_cut_memory(memory, size)?;
        }
        Ok(())
    }

    fn zero_ray_cut_memory(&self, memory: Handle, size: u64) -> Result<(), String> {
        let bytes =
            usize::try_from(size).map_err(|err| format!("texel ray cut buffer size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, size, 0, &mut mapped),
                "map texel ray cut buffer",
            )?;
            std::ptr::write_bytes(mapped.cast::<u8>(), 0, bytes);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn read_texel_ray_cut(&self) -> Result<Vec<f32>, String> {
        let out_mem = self
            .texel_ray_cut
            .as_ref()
            .ok_or("texel ray cut check is not created")?
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

    fn submit_texel_ray_cut(&mut self, steps: &[Step]) -> Result<(), String> {
        let (pipe, compact, layout, set, cmd, fence) = {
            let Some(state) = self.texel_ray_cut.as_ref() else {
                return Err("texel ray cut check is not created".into());
            };
            (
                state.pipe,
                state.compact,
                state.layout,
                state.set,
                state.cmd,
                state.fence,
            )
        };
        self.wait_fence(fence, "texel ray cut wait")?;
        let fences = [fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "texel ray cut reset fence",
            )?;
            check((self.fns.reset_cmd)(cmd, 0), "texel ray cut reset")?;
            self.begin_reused(cmd)?;
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
                cmd,
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
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            #[repr(C)]
            struct Pc {
                dims: [u32; 4],
                params: [u32; 4],
                lamp: [u32; 4],
            }
            for (i, step) in steps.iter().enumerate() {
                if i > 0 {
                    let pass = MemBar {
                        s_type: 46,
                        next: std::ptr::null(),
                        src_access: 0x40,
                        dst_access: 0x60,
                    };
                    (self.fns.cmd_barrier)(
                        cmd,
                        0x800,
                        0x800,
                        0,
                        1,
                        &pass as *const MemBar as *const c_void,
                        0,
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                    );
                }
                let which = if step.compact { compact } else { pipe };
                (self.fns.cmd_bind_pipe)(cmd, 1, which);
                let pc = Pc {
                    dims: [1, 1, 1, 1],
                    params: [0, 1, step.params_z, step.frame],
                    lamp: [step.lamp_x, step.lamp_y, 0, 0],
                };
                (self.fns.cmd_push)(
                    cmd,
                    layout,
                    0x20,
                    0,
                    PC_BYTES,
                    std::ptr::from_ref(&pc).cast(),
                );
                (self.fns.cmd_dispatch)(cmd, step.groups, 1, 1);
            }
            let back = MemBar {
                s_type: 46,
                next: std::ptr::null(),
                src_access: 0x40,
                dst_access: 0x2000,
            };
            (self.fns.cmd_barrier)(
                cmd,
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
            check((self.fns.end_cmd)(cmd), "texel ray cut end")?;
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
                "texel ray cut submit",
            )?;
        }
        self.wait_fence(fence, "texel ray cut done")
    }
}

struct Step {
    compact: bool,
    groups: u32,
    params_z: u32,
    frame: u32,
    lamp_x: u32,
    lamp_y: u32,
}

fn ray_cut_heads() -> [genos_scene::LightingSettings; HEAD_VARIANTS] {
    let mut both = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
    both.surface_cache.sh_cells = true;
    both.surface_cache.sh_texel_detail = true;
    both.surface_cache.sh_texel_ray_cut = true;
    both.sanitize();
    let mut cut_off = both.clone();
    cut_off.surface_cache.sh_texel_ray_cut = false;
    cut_off.sanitize();
    let mut detail_off = both.clone();
    detail_off.surface_cache.sh_texel_detail = false;
    detail_off.sanitize();
    let mut low = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Low);
    low.surface_cache.sh_cells = true;
    low.surface_cache.sh_texel_detail = true;
    low.surface_cache.sh_texel_ray_cut = true;
    low.sanitize();
    [both, cut_off, detail_off, low]
}
