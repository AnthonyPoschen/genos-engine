impl Gpu {
    fn make_aa_pipes(&mut self) -> Result<(), String> {
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
        ];
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 2,
            bindings: bindings.as_ptr(),
        };
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.aa_desc_layout,
                ),
                "aa descriptor layout",
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
        let sets = [self.aa_desc_layout];
        let push = Range {
            stage: 0x20,
            offset: 0,
            size: 16,
        };
        let layout = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr(),
            push_count: 1,
            push: &push,
        };
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut self.aa_layout,
                ),
                "aa pipeline layout",
            )?;
        }
        self.aa_fxaa = self.make_aa_pipe(FXAA_SPV)?;
        self.aa_ssaa = self.make_aa_pipe(SSAA_SPV)?;
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
        let size = Size { kind: 7, count: 4 };
        let pool = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 2,
            size_count: 1,
            sizes: &size,
        };
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.aa_pool,
                ),
                "aa descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [self.aa_desc_layout, self.aa_desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.aa_pool,
                count: 2,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    self.aa_sets.as_mut_ptr(),
                ),
                "aa descriptor sets",
            )?;
        }
        Ok(())
    }

    fn make_aa_pipe(&self, code: &[u8]) -> Result<Handle, String> {
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
            layout: self.aa_layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        let mut pipeline = std::ptr::null_mut();
        unsafe {
            let result = (self.fns.create_compute)(
                self.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut pipeline,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "aa compute pipeline")?;
        }
        Ok(pipeline)
    }

    fn make_aa_targets(&mut self) -> Result<(), String> {
        self.destroy_aa_targets();
        let w = self.extent_w;
        let h = self.extent_h;
        let presented = w as u64 * h as u64 * 4;
        let hi = presented * 4;
        for slot in 0..2 {
            self.ssaa_color[slot] = self.make_image(self.format, 0x10 | 0x1, 1, w * 2, h * 2)?;
            self.ssaa_depth[slot] = self.make_image(126, 0x20, 2, w * 2, h * 2)?;
            let views = [self.ssaa_color[slot].view, self.ssaa_depth[slot].view];
            self.ssaa_fb[slot] =
                self.make_framebuffer_for(self.render_pass, &views, w * 2, h * 2)?;
            self.aa_src[slot] = self.make_buffer(hi, 0x20 | 0x1 | 0x2, false)?;
            self.aa_dst[slot] = self.make_buffer(presented, 0x20 | 0x1 | 0x2, false)?;
            self.color_layout[slot] = 0;
        }
        self.write_aa_sets()?;
        Ok(())
    }

    fn write_aa_sets(&self) -> Result<(), String> {
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
        for slot in 0..2 {
            let infos = [
                BufInfo {
                    buffer: self.aa_src[slot].buffer,
                    offset: 0,
                    range: u64::MAX,
                },
                BufInfo {
                    buffer: self.aa_dst[slot].buffer,
                    offset: 0,
                    range: u64::MAX,
                },
            ];
            let writes = [
                Write {
                    s_type: 35,
                    next: std::ptr::null(),
                    set: self.aa_sets[slot],
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
                    set: self.aa_sets[slot],
                    binding: 1,
                    element: 0,
                    count: 1,
                    kind: 7,
                    image: std::ptr::null(),
                    buffer: &infos[1],
                    texel: std::ptr::null(),
                },
            ];
            unsafe {
                (self.fns.update_desc)(
                    self.device,
                    2,
                    writes.as_ptr() as *const u8,
                    0,
                    std::ptr::null(),
                );
            }
        }
        Ok(())
    }

    fn destroy_aa_targets(&mut self) {
        unsafe {
            for slot in 0..2 {
                if !self.ssaa_fb[slot].is_null() {
                    (self.fns.destroy_framebuffer)(
                        self.device,
                        self.ssaa_fb[slot],
                        std::ptr::null(),
                    );
                    self.ssaa_fb[slot] = std::ptr::null_mut();
                }
                let mut color = std::mem::replace(&mut self.ssaa_color[slot], Image::empty());
                let mut depth = std::mem::replace(&mut self.ssaa_depth[slot], Image::empty());
                let mut src = std::mem::replace(&mut self.aa_src[slot], Buffer::empty());
                let mut dst = std::mem::replace(&mut self.aa_dst[slot], Buffer::empty());
                self.destroy_image(&mut color);
                self.destroy_image(&mut depth);
                self.destroy_buffer(&mut src);
                self.destroy_buffer(&mut dst);
                self.color_layout[slot] = 0;
            }
        }
    }

    fn raster_scene(&self, fb: Handle, width: u32, height: u32, matrix: &[f32; 16], overlay: bool) {
        let [r, g, b] = self.background;
        let mut clears = [[r, g, b, 1.0], [1.0, 0.0, 0.0, 0.0]];
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
            pass: self.render_pass,
            fb,
            area: Rect {
                offset: Offset { x: 0, y: 0 },
                extent: Extent {
                    w: width,
                    h: height,
                },
            },
            clear_count: 2,
            clears: clears.as_mut_ptr() as *const f32,
        };
        unsafe {
            (self.fns.cmd_begin_rp)(self.cmd, &rp as *const RpBegin as *const u8, 0);
            let world_pipe = if self.wire_on {
                self.wire_pipeline
            } else {
                self.pipeline
            };
            (self.fns.cmd_bind_pipe)(self.cmd, 0, world_pipe);
            (self.fns.cmd_bind_set)(
                self.cmd,
                0,
                self.layout,
                0,
                1,
                &self.desc_set,
                0,
                std::ptr::null(),
            );
            let viewport = [0.0f32, 0.0, width as f32, height as f32, 0.0, 1.0];
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
            let scissor = [0i32, 0, width as i32, height as i32];
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
            (self.fns.cmd_push)(
                self.cmd,
                self.layout,
                1,
                0,
                64,
                matrix.as_ptr() as *const c_void,
            );
            let mut bound = std::ptr::null_mut();
            for draw in &self.draws {
                if draw.count == 0 {
                    continue;
                }
                let buffer = if draw.shapes {
                    self.shapes.buffer
                } else {
                    self.vertex.buffer
                };
                if buffer != bound {
                    let vb = [buffer];
                    let off = [0u64];
                    (self.fns.cmd_bind_vb)(self.cmd, 0, 1, vb.as_ptr(), off.as_ptr());
                    bound = buffer;
                }
                (self.fns.cmd_draw)(self.cmd, draw.count, 1, draw.first, draw.instance);
            }
            if overlay && self.overlay_count > 0 && !self.overlay_pipeline.is_null() {
                self.draw_overlay_quads(width, height);
            }
            (self.fns.cmd_end_rp)(self.cmd);
        }
    }

    fn raster_overlay(&self) {
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
                    w: self.extent_w,
                    h: self.extent_h,
                },
            },
            clear_count: 0,
            clears: std::ptr::null(),
        };
        unsafe {
            (self.fns.cmd_begin_rp)(self.cmd, &rp as *const RpBegin as *const u8, 0);
            self.draw_overlay_quads(self.extent_w, self.extent_h);
            (self.fns.cmd_end_rp)(self.cmd);
        }
    }

    fn draw_overlay_quads(&self, width: u32, height: u32) {
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 0, self.overlay_pipeline);
            let viewport = [0.0f32, 0.0, width as f32, height as f32, 0.0, 1.0];
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
            let scissor = [0i32, 0, width as i32, height as i32];
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
            let ortho = pixel_matrix(width as f32, height as f32);
            (self.fns.cmd_push)(
                self.cmd,
                self.layout,
                1,
                0,
                64,
                ortho.as_ptr() as *const c_void,
            );
            let vb = [self.vertex.buffer];
            let off = [0u64];
            (self.fns.cmd_bind_vb)(self.cmd, 0, 1, vb.as_ptr(), off.as_ptr());
            (self.fns.cmd_draw)(self.cmd, self.overlay_count, 1, self.vertex_count, 0);
        }
    }

    fn resolve_fxaa(&mut self, slot: usize) -> Result<(), String> {
        let image = self.color.image;
        self.copy_image_buffer(
            image,
            6,
            self.aa_src[slot].buffer,
            self.extent_w,
            self.extent_h,
            true,
        );
        self.buffer_barrier(self.aa_src[slot].buffer, 0x1000, 0x800, 0x1000, 0x20);
        self.dispatch_aa(
            self.aa_fxaa,
            self.aa_sets[slot],
            self.extent_w,
            self.extent_h,
            self.extent_w,
        );
        self.buffer_barrier(self.aa_dst[slot].buffer, 0x800, 0x1000, 0x40, 0x800);
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        self.copy_image_buffer(
            image,
            7,
            self.aa_dst[slot].buffer,
            self.extent_w,
            self.extent_h,
            false,
        );
        self.image_barrier(image, 7, 6, 0x1000, 0x1000, 0x1000, 0x800);
        self.color_layout[slot] = 6;
        Ok(())
    }

    fn resolve_ssaa(&mut self, slot: usize) -> Result<(), String> {
        if self.ssaa_color[slot].image.is_null() {
            return Err("ssaa target is missing".into());
        }
        let hi_w = self.extent_w * 2;
        let hi_h = self.extent_h * 2;
        self.copy_image_buffer(
            self.ssaa_color[slot].image,
            6,
            self.aa_src[slot].buffer,
            hi_w,
            hi_h,
            true,
        );
        self.buffer_barrier(self.aa_src[slot].buffer, 0x1000, 0x800, 0x1000, 0x20);
        self.dispatch_aa(
            self.aa_ssaa,
            self.aa_sets[slot],
            self.extent_w,
            self.extent_h,
            hi_w,
        );
        self.buffer_barrier(self.aa_dst[slot].buffer, 0x800, 0x1000, 0x40, 0x800);
        let old = self.color_layout[slot];
        let src_stage = if old == 0 { 1 } else { 0x1000 };
        let src_access = if old == 0 { 0 } else { 0x800 };
        self.image_barrier(
            self.color.image,
            old,
            7,
            src_stage,
            0x1000,
            src_access,
            0x1000,
        );
        self.copy_image_buffer(
            self.color.image,
            7,
            self.aa_dst[slot].buffer,
            self.extent_w,
            self.extent_h,
            false,
        );
        self.image_barrier(self.color.image, 7, 6, 0x1000, 0x1000, 0x1000, 0x800);
        self.color_layout[slot] = 6;
        Ok(())
    }

    fn dispatch_aa(&self, pipe: Handle, set: Handle, width: u32, height: u32, src_width: u32) {
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        let pc = [width, height, src_width, bgra];
        let groups_x = width.div_ceil(8);
        let groups_y = height.div_ceil(8);
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, pipe);
            (self.fns.cmd_bind_set)(self.cmd, 1, self.aa_layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_push)(
                self.cmd,
                self.aa_layout,
                0x20,
                0,
                16,
                pc.as_ptr() as *const c_void,
            );
            (self.fns.cmd_dispatch)(self.cmd, groups_x, groups_y, 1);
        }
    }

    fn buffer_barrier(
        &self,
        buffer: Handle,
        src_stage: u32,
        dst_stage: u32,
        src_access: u32,
        dst_access: u32,
    ) {
        #[repr(C)]
        struct Barrier {
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
        let barrier = Barrier {
            s_type: 44,
            next: std::ptr::null(),
            src_access,
            dst_access,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            buffer,
            offset: 0,
            size: u64::MAX,
        };
        unsafe {
            (self.fns.cmd_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                0,
                std::ptr::null(),
                1,
                &barrier as *const Barrier as *const c_void,
                0,
                std::ptr::null(),
            );
        }
    }

    fn copy_image_buffer(
        &self,
        image: Handle,
        layout: i32,
        buffer: Handle,
        w: u32,
        h: u32,
        to_buffer: bool,
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
            extent: Extent { w, h, d: 1 },
        };
        unsafe {
            if to_buffer {
                (self.fns.cmd_copy_to_buffer)(
                    self.cmd,
                    image,
                    layout,
                    buffer,
                    1,
                    &region as *const Region as *const u8,
                );
            } else {
                (self.fns.cmd_copy_to_image)(
                    self.cmd,
                    buffer,
                    image,
                    layout,
                    1,
                    &region as *const Region as *const u8,
                );
            }
        }
    }
}
