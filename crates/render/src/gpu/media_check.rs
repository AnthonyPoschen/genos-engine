//! Dispatch `media_check.comp` and read the five closed forms.

use std::ffi::c_void;

use super::{check, read_f32s, Buffer, Gpu, Handle, Memory};

pub(super) struct Pipe {
    buffer: Buffer,
    desc_layout: Handle,
    desc_pool: Handle,
    set: Handle,
    layout: Handle,
    pipe: Handle,
    cmd: Handle,
    fence: Handle,
}

impl Gpu {
    pub(super) fn media_closed_forms(&mut self) -> Result<[f32; 5], String> {
        if self.media_check.is_none() {
            self.media_check = Some(self.create_media_check()?);
        }
        self.dispatch_media_check()
    }

    pub(super) fn destroy_media_check(&mut self) {
        let Some(pipe) = self.media_check.take() else {
            return;
        };
        unsafe {
            if !pipe.fence.is_null() {
                (self.fns.destroy_fence)(self.device, pipe.fence, std::ptr::null());
            }
            if !pipe.pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, pipe.pipe, std::ptr::null());
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
        let mut buffer = pipe.buffer;
        self.destroy_buffer(&mut buffer);
    }

    fn create_media_check(&mut self) -> Result<Pipe, String> {
        let buffer = self.make_buffer(64, 0x20, Memory::Readback)?;
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
        let bindings = [Binding {
            binding: 0,
            kind: 7,
            count: 1,
            stages: 0x20,
            samplers: std::ptr::null(),
        }];
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 1,
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
                "media descriptor layout",
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
        let sets = [desc_layout];
        let layout_info = LayoutInfo {
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
                    &layout_info as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "media pipeline layout",
            )?;
        }
        let module = self.shader(super::MEDIA_CHECK_SPV)?;
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
            check(result, "media pipeline")?;
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
        let size = Size { kind: 7, count: 1 };
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
                "media descriptor pool",
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
                "media descriptor set",
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
                "media command buffer",
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
                "media fence",
            )?;
        }
        let built = Pipe {
            buffer,
            desc_layout,
            desc_pool,
            set,
            layout,
            pipe,
            cmd,
            fence,
        };
        self.write_media_set(&built)?;
        Ok(built)
    }

    fn write_media_set(&self, pipe: &Pipe) -> Result<(), String> {
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
        let info = BufInfo {
            buffer: pipe.buffer.buffer,
            offset: 0,
            range: u64::MAX,
        };
        let write = Write {
            s_type: 35,
            next: std::ptr::null(),
            set: pipe.set,
            binding: 0,
            element: 0,
            count: 1,
            kind: 7,
            image: std::ptr::null(),
            buffer: &info,
            texel: std::ptr::null(),
        };
        unsafe {
            (self.fns.update_desc)(
                self.device,
                1,
                &write as *const Write as *const u8,
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn dispatch_media_check(&mut self) -> Result<[f32; 5], String> {
        let (memory, pipe, layout, set, cmd, fence) = {
            let Some(state) = self.media_check.as_ref() else {
                return Err("media check is not created".into());
            };
            (
                state.buffer.memory,
                state.pipe,
                state.layout,
                state.set,
                state.cmd,
                state.fence,
            )
        };
        let zeros = [0u8; 20];
        self.write_memory(memory, &zeros)?;
        self.wait_fence(fence, "media wait")?;
        let fences = [fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "media reset fence",
            )?;
            check((self.fns.reset_cmd)(cmd, 0), "media reset")?;
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
                (self.fns.begin_cmd)(cmd, &begin as *const BeginInfo as *const u8),
                "media begin",
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
            self.stamp_media(cmd, 0, 0x1);
            (self.fns.cmd_dispatch)(cmd, self.media_groups.max(1), 1, 1);
            self.stamp_media(cmd, 1, 0x2000);
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
            check((self.fns.end_cmd)(cmd), "media end")?;
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
                "media submit",
            )?;
        }
        self.wait_fence(fence, "media done")?;
        self.media_ms = self.read_span(0).ok().map(|duration| duration.as_secs_f64() * 1000.0);
        let values = read_f32s(self.fns.map_mem, self.fns.unmap_mem, self.device, memory, 5)?;
        let mut out = [0.0; 5];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value;
        }
        Ok(out)
    }

    fn stamp_media(&self, cmd: Handle, query: u32, stage: u32) {
        if self.query_pool.is_null() || self.timestamp_period <= 0.0 {
            return;
        }
        unsafe {
            if query == 0 {
                (self.fns.cmd_reset_query)(cmd, self.query_pool, 0, 2);
            }
            (self.fns.cmd_write_timestamp)(cmd, stage, self.query_pool, query);
        }
    }

    pub(super) fn media_dispatch_ms(&mut self, groups: u32) -> Result<f64, String> {
        self.media_groups = groups.max(1);
        let dispatched = self.media_closed_forms();
        self.media_groups = 1;
        let _ = dispatched?;
        self.media_ms
            .ok_or_else(|| "Vulkan timestamp queries are not available on this device".to_string())
    }
}
