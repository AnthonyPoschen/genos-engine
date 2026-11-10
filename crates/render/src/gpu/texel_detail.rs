//! Dispatch `texel_detail_prod.comp`.
//!
//! The shader includes `lighting_common.glsl` and calls
//! `lighting_screen_probe_indirect`, the function compose calls. One module
//! is the writing specialization. The other defines `LIGHTING_SC_WRITES` false,
//! which is the read-only compose specialization. The buffers are the
//! production cache, key table, and work block at the production slot count.
//!
//! Lavapipe keeps the compiler heap for a compiled pipeline, and destroying
//! that pipeline does not return it. Both specializations on one device
//! exhaust a 16 GB software-test budget. The writing dispatch plants the
//! slots. The caller drops that device, returns the heap, and compiles the
//! reader on a new device with the planted cache and key table uploaded.
//! The production function is unchanged.

use std::ffi::c_void;

use super::{Buffer, Gpu, Handle, Memory, check, read_f32s};

const SLOTS: u64 = 1_048_576;
const CACHE_BYTES: u64 = SLOTS * 392;
// One retired grid slot shifts the change block past the cache-ray region. The
// extra words keep that head inside the buffer.
const WORK_BYTES: u64 = (98_304 * 16 + 64) * 16;
// Row 34 is the trust-0 pop. Rows 35 and 36 are the settled mover and the
// coarse miss. 8 head words, 37 rows of 17, the 40-word motion trailer, then
// the 32-word lattice parity.
const OUT_FLOATS: usize = 8 + 37 * 17 + 40 + 32;
const PC_BYTES: u32 = 48;
const HEAD_WORDS: usize = 8;
const HEAD_VARIANTS: usize = 3;

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
    pipe_ro: Handle,
    pub(super) cmd: Handle,
    fence: Handle,
}

pub(super) fn release_compiler_heap() {
    extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    // Lavapipe's compiler scratch is not the pipeline. Return it before the
    // dispatch, or the two together pass the software-test budget. A product
    // frame does not call this.
    unsafe {
        malloc_trim(0);
    }
}

/// Cache and key table left by the writing plant, stored on disk so the
/// read-only compile does not keep that copy in anonymous memory. The reader
/// uploads the files and deletes them before it dispatches.
pub struct TexelDetailPlant {
    cache: std::path::PathBuf,
    keys: std::path::PathBuf,
}

impl TexelDetailPlant {
    /// Hand the planted files to another process. Drop no longer deletes them.
    pub fn into_paths(mut self) -> (std::path::PathBuf, std::path::PathBuf) {
        (
            std::mem::take(&mut self.cache),
            std::mem::take(&mut self.keys),
        )
    }

    /// A plant written by [`Self::into_paths`]. Drop deletes the files.
    pub fn from_paths(cache: std::path::PathBuf, keys: std::path::PathBuf) -> Self {
        Self { cache, keys }
    }
}

impl Drop for TexelDetailPlant {
    fn drop(&mut self) {
        if !self.cache.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.cache);
        }
        if !self.keys.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.keys);
        }
    }
}

fn plant_file(kind: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("genos-texel-{kind}-{}.bin", std::process::id()))
}

fn ckeys_bytes() -> u64 {
    // The parity block writes a lattice record. The table sits past the
    // screen list, so the fixture buffer is the production key allocation.
    super::lighting_sh_keys_words() * 4
}

impl Gpu {
    pub(super) fn texel_detail_prod(&mut self) -> Result<Vec<f32>, String> {
        self.texel_detail_dispatch()
    }

    /// Writing specialization, `lamp.x = 1`. Copies the cache and the key table
    /// and destroys the pipeline. The caller drops the device before compiling
    /// the reader.
    pub(super) fn texel_detail_prod_plant(&mut self) -> Result<TexelDetailPlant, String> {
        self.ensure_texel_writer()?;
        self.zero_texel_buffers()?;
        let keys = self
            .texel_detail
            .as_ref()
            .ok_or("texel detail production check is not created")?
            .keys
            .memory;
        self.write_texel_detail_heads(keys)?;
        self.submit_texel(false, 1, 0, 0)?;
        let plant = self.snapshot_texel_plant()?;
        self.destroy_texel_detail();
        release_compiler_heap();
        Ok(plant)
    }

    /// Read-only compose specialization. `plant` is the writing device's cache
    /// and key table. This compiles only the reader, then drops the host copy
    /// before the dispatch.
    pub(super) fn texel_detail_prod_readonly(
        &mut self,
        plant: TexelDetailPlant,
    ) -> Result<Vec<f32>, String> {
        release_compiler_heap();
        if self.texel_detail.is_some() {
            self.destroy_texel_detail();
        }
        self.texel_detail = Some(self.create_texel_detail_pipe(super::TEXEL_DETAIL_PROD_RO_SPV)?);
        self.zero_texel_buffers()?;
        self.upload_texel_plant(&plant)?;
        drop(plant);
        release_compiler_heap();
        // A few rows at a time. Counts accumulate in the output and in gstats.
        // The read-only module is folded in build.rs; these chunks are the
        // dispatch the assertions count.
        for begin in (0u32..12).step_by(2) {
            self.submit_texel(false, 0, begin, 2)?;
            release_compiler_heap();
        }
        self.read_texel_out()
    }

    fn ensure_texel_writer(&mut self) -> Result<(), String> {
        if self.texel_detail.is_none() {
            self.texel_detail = Some(self.create_texel_detail_pipe(super::TEXEL_DETAIL_PROD_SPV)?);
        }
        Ok(())
    }

    fn texel_detail_dispatch(&mut self) -> Result<Vec<f32>, String> {
        self.ensure_texel_writer()?;
        self.zero_texel_buffers()?;
        let keys = self
            .texel_detail
            .as_ref()
            .ok_or("texel detail production check is not created")?
            .keys
            .memory;
        self.write_texel_detail_heads(keys)?;
        self.submit_texel(false, 0, 0, 0)?;
        self.read_texel_out()
    }

    pub(super) fn destroy_texel_detail(&mut self) {
        if let Some(pipe) = self.texel_detail.take() {
            self.destroy_texel_pipe(pipe);
        }
    }

    fn destroy_texel_pipe(&mut self, mut pipe: Pipe) {
        unsafe {
            if !pipe.fence.is_null() {
                (self.fns.destroy_fence)(self.device, pipe.fence, std::ptr::null());
            }
            if !pipe.pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, pipe.pipe, std::ptr::null());
            }
            if !pipe.pipe_ro.is_null() {
                (self.fns.destroy_pipeline)(self.device, pipe.pipe_ro, std::ptr::null());
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

    fn create_texel_detail_pipe(&mut self, code: &[u8]) -> Result<Pipe, String> {
        let scene = self.make_buffer(256, 0x20, Memory::Readback)?;
        let mesh = self.make_buffer(256, 0x20, Memory::Readback)?;
        let gbuffer = self.make_buffer(256, 0x20, Memory::Readback)?;
        let work = self.make_buffer(WORK_BYTES, 0x20, Memory::Readback)?;
        let out_bytes = u64::try_from(OUT_FLOATS * 4)
            .map_err(|err| format!("texel detail output size: {err}"))?;
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
            .map_err(|err| format!("texel detail binding count: {err}"))?;
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
                "texel detail descriptor layout",
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
                "texel detail pipeline layout",
            )?;
        }
        let pipe = self.texel_compute(code, layout, "texel detail pipeline")?;
        // One specialization per device. The other compile happens after this
        // device is gone.
        let pipe_ro = std::ptr::null_mut();
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
                "texel detail descriptor pool",
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
                "texel detail descriptor set",
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
                "texel detail command buffer",
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
                "texel detail fence",
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
            pipe,
            pipe_ro,
            cmd,
            fence,
        };
        self.write_texel_detail_set(&built)?;
        Ok(built)
    }

    fn write_texel_detail_set(&self, pipe: &Pipe) -> Result<(), String> {
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
            .map_err(|err| format!("texel detail descriptor writes: {err}"))?;
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

    fn write_texel_detail_heads(&self, memory: Handle) -> Result<(), String> {
        let mut words = [0u32; HEAD_WORDS * HEAD_VARIANTS];
        let variants = [
            (true, true, false),
            (false, true, false),
            (true, true, true),
        ];
        for (i, (detail, cells, low)) in variants.into_iter().enumerate() {
            let mut settings =
                genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
            settings.surface_cache.sh_cells = cells;
            settings.surface_cache.sh_texel_detail = detail;
            if low {
                settings.bounce.count = 0;
            }
            settings.sanitize();
            let head = super::lighting_change_head(&settings);
            let at = i * HEAD_WORDS;
            words[at] = 0;
            words[at + 1] = 1.0f32.to_bits();
            words[at + 2] = ((2 * head.flags) as f32).to_bits();
            words[at + 3] = (head.mode_cap as f32).to_bits();
            words[at + 7] = (head.head_hi as f32).to_bits();
        }
        let bytes = u64::try_from(std::mem::size_of_val(&words))
            .map_err(|err| format!("texel detail head size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, bytes, 0, &mut mapped),
                "map texel detail heads",
            )?;
            std::ptr::copy_nonoverlapping(
                words.as_ptr().cast::<u8>(),
                mapped.cast::<u8>(),
                usize::try_from(bytes).map_err(|err| format!("texel detail head copy: {err}"))?,
            );
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn zero_memory(&self, memory: Handle, size: u64) -> Result<(), String> {
        let bytes =
            usize::try_from(size).map_err(|err| format!("texel detail buffer size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, size, 0, &mut mapped),
                "map texel detail buffer",
            )?;
            std::ptr::write_bytes(mapped.cast::<u8>(), 0, bytes);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(())
    }

    fn texel_compute(&self, code: &[u8], layout: Handle, what: &str) -> Result<Handle, String> {
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
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, what)?;
        }
        // The compiler's scratch is not the pipeline. Return it before the
        // dispatch, or the two together pass the software-test budget.
        release_compiler_heap();
        Ok(pipe)
    }

    fn zero_texel_buffers(&mut self) -> Result<(), String> {
        let zeros = {
            let Some(state) = self.texel_detail.as_ref() else {
                return Err("texel detail production check is not created".into());
            };
            [
                (state.scene.memory, state.scene.size),
                (state.mesh.memory, state.mesh.size),
                (state.gbuffer.memory, state.gbuffer.size),
                (state.work.memory, state.work.size),
                (state.out.memory, state.out.size),
                (state.keys.memory, state.keys.size),
                (state.cache.memory, state.cache.size),
                (state.stats.memory, state.stats.size),
            ]
        };
        for (memory, size) in zeros {
            self.zero_memory(memory, size)?;
        }
        Ok(())
    }

    fn snapshot_texel_plant(&self) -> Result<TexelDetailPlant, String> {
        let Some(state) = self.texel_detail.as_ref() else {
            return Err("texel detail production check is not created".into());
        };
        let cache = plant_file("cache");
        let keys = plant_file("keys");
        self.write_mapped_file(state.cache.memory, state.cache.size, &cache)?;
        self.write_mapped_file(state.keys.memory, state.keys.size, &keys)?;
        Ok(TexelDetailPlant { cache, keys })
    }

    fn upload_texel_plant(&self, plant: &TexelDetailPlant) -> Result<(), String> {
        let Some(state) = self.texel_detail.as_ref() else {
            return Err("texel detail production check is not created".into());
        };
        self.read_mapped_file(state.cache.memory, state.cache.size, &plant.cache)?;
        self.read_mapped_file(state.keys.memory, state.keys.size, &plant.keys)?;
        Ok(())
    }

    fn write_mapped_file(
        &self,
        memory: Handle,
        size: u64,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let bytes =
            usize::try_from(size).map_err(|err| format!("texel detail buffer size: {err}"))?;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, size, 0, &mut mapped),
                "map texel detail buffer",
            )?;
            let slice = std::slice::from_raw_parts(mapped.cast::<u8>(), bytes);
            let result = std::fs::write(path, slice);
            (self.fns.unmap_mem)(self.device, memory);
            result.map_err(|err| format!("write {}: {err}", path.display()))
        }
    }

    fn read_mapped_file(
        &self,
        memory: Handle,
        size: u64,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let bytes =
            usize::try_from(size).map_err(|err| format!("texel detail buffer size: {err}"))?;
        let mut file =
            std::fs::File::open(path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let file_len = file
            .metadata()
            .map_err(|err| format!("stat {}: {err}", path.display()))?
            .len();
        if file_len != size {
            return Err(format!(
                "{} is {file_len} bytes, the buffer is {size}",
                path.display()
            ));
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, 0, size, 0, &mut mapped),
                "map texel detail buffer",
            )?;
            let dest = std::slice::from_raw_parts_mut(mapped.cast::<u8>(), bytes);
            let result = std::io::Read::read_exact(&mut file, dest);
            (self.fns.unmap_mem)(self.device, memory);
            result.map_err(|err| format!("read {}: {err}", path.display()))?;
        }
        Ok(())
    }

    fn read_texel_out(&self) -> Result<Vec<f32>, String> {
        let out_mem = self
            .texel_detail
            .as_ref()
            .ok_or("texel detail production check is not created")?
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

    fn submit_texel(
        &mut self,
        reader: bool,
        lamp_x: u32,
        row: u32,
        rows: u32,
    ) -> Result<(), String> {
        let (pipe, layout, set, cmd, fence) = {
            let Some(state) = self.texel_detail.as_ref() else {
                return Err("texel detail production check is not created".into());
            };
            let pipe = if reader { state.pipe_ro } else { state.pipe };
            if pipe.is_null() {
                return Err("texel detail pipeline is not created".into());
            }
            (pipe, state.layout, state.set, state.cmd, state.fence)
        };
        self.wait_fence(fence, "texel detail wait")?;
        let fences = [fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "texel detail reset fence",
            )?;
            check((self.fns.reset_cmd)(cmd, 0), "texel detail reset")?;
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
            (self.fns.cmd_bind_pipe)(cmd, 1, pipe);
            (self.fns.cmd_bind_set)(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
            #[repr(C)]
            struct Pc {
                dims: [u32; 4],
                params: [u32; 4],
                lamp: [u32; 4],
            }
            let pc = Pc {
                dims: [1, 1, 1, 1],
                params: [0, 1, 0, 10],
                lamp: [lamp_x, row, rows, 0],
            };
            (self.fns.cmd_push)(
                cmd,
                layout,
                0x20,
                0,
                PC_BYTES,
                std::ptr::from_ref(&pc).cast(),
            );
            (self.fns.cmd_dispatch)(cmd, 1, 1, 1);
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
            check((self.fns.end_cmd)(cmd), "texel detail end")?;
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
                "texel detail submit",
            )?;
        }
        self.wait_fence(fence, "texel detail done")
    }
}
