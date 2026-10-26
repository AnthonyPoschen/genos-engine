// Mirror and portal views, the water quad, and the grade buffer.
// Included from `gpu.rs`. An empty reflector list returns before any of this
// runs, so a Lambert frame does not bind these pipelines.

use genos_math::optics;
use genos_math::Vec3;
use genos_scene::look_direction;

use crate::reflections::{self, mirror_view_proj, portal_view_proj, GRADE_BYTES};
use crate::world::{Reflector, ReflectorKind};

const PLANES: usize = 4;

struct ReflectionGpu {
    planes: [Reflector; PLANES],
    plane_count: u32,
    preset: u32,
    eye: [f32; 3],
    forward: [f32; 3],
    aspect: f32,
    enabled: bool,
    water_on: bool,
    view_cap: u32,
    water_first: u32,
    water_count: u32,
    ran: bool,
    pass_override: Handle,
    color: Image,
    depth: Image,
    fb: Handle,
    later_pass: Handle,
    grade: Buffer,
    desc_layout: Handle,
    pipe_layout: Handle,
    pool: Handle,
    set: Handle,
    pipe: Handle,
    water_pipe: Handle,
    target_w: u32,
    target_h: u32,
}

impl ReflectionGpu {
    fn idle() -> Self {
        Self {
            planes: [Reflector {
                kind: ReflectorKind::Mirror,
                origin: [0.0; 3],
                normal: [0.0, 1.0, 0.0],
                portal_to: [0.0; 3],
            }; PLANES],
            plane_count: 0,
            preset: 3,
            eye: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            aspect: 1.0,
            enabled: false,
            water_on: false,
            view_cap: 0,
            water_first: 0,
            water_count: 0,
            ran: false,
            pass_override: std::ptr::null_mut(),
            color: Image::empty(),
            depth: Image::empty(),
            fb: std::ptr::null_mut(),
            later_pass: std::ptr::null_mut(),
            grade: Buffer::empty(),
            desc_layout: std::ptr::null_mut(),
            pipe_layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            set: std::ptr::null_mut(),
            pipe: std::ptr::null_mut(),
            water_pipe: std::ptr::null_mut(),
            target_w: 0,
            target_h: 0,
        }
    }
}

impl Gpu {
    fn note_reflections(
        &mut self,
        world: &crate::world::World,
        camera: &genos_scene::Camera,
        aspect: f32,
        water_first: u32,
        water_count: u32,
    ) {
        let settings = world.scene.reflections;
        let preset = reflections::lighting_reflection_preset(world.scene.lighting.preset);
        let mut planes = [self.reflection.planes[0]; PLANES];
        let mut count = 0u32;
        for reflector in world.reflectors.iter().take(PLANES) {
            let index = usize::try_from(count).unwrap_or(0);
            planes[index] = *reflector;
            count += 1;
        }
        let mirrors = world
            .reflectors
            .iter()
            .filter(|item| item.kind == ReflectorKind::Mirror)
            .count();
        let portals = world
            .reflectors
            .iter()
            .filter(|item| item.kind == ReflectorKind::Portal)
            .count();
        let water = if settings.water {
            world
                .reflectors
                .iter()
                .filter(|item| item.kind == ReflectorKind::Water)
                .count()
        } else {
            0
        };
        let active = settings.enabled
            && reflections::reflection_frame_active(
                u32::try_from(mirrors).unwrap_or(0),
                u32::try_from(portals).unwrap_or(0),
                u32::try_from(water).unwrap_or(0),
            );
        let forward = look_direction(camera.yaw, camera.pitch);
        self.reflection.planes = planes;
        self.reflection.plane_count = count;
        self.reflection.preset = preset;
        self.reflection.eye = [camera.position.x, camera.position.y, camera.position.z];
        self.reflection.forward = [forward.x, forward.y, forward.z];
        self.reflection.aspect = aspect;
        self.reflection.enabled = active;
        self.reflection.water_on = settings.water;
        self.reflection.view_cap = optics::reflection_view_cap(match preset {
            0 => optics::ReflectionPreset::Potato,
            1 => optics::ReflectionPreset::Low,
            2 => optics::ReflectionPreset::Medium,
            _ => optics::ReflectionPreset::High,
        })
        .min(settings.mirror_views);
        self.reflection.water_first = water_first;
        self.reflection.water_count = if settings.water { water_count } else { 0 };
        self.reflection.ran = false;
        self.reflection.pass_override = std::ptr::null_mut();
    }

    fn reflections_after_lighting(&mut self, matrix: &[f32; 16]) -> Result<(), String> {
        if !self.reflection.enabled {
            return Ok(());
        }
        // Potato and Low push a preset whose ray cap is 0. This pass does not
        // march reflection rays; the composite stores that cap.
        self.ensure_reflection_targets()?;
        self.raster_reflection_views();
        self.dispatch_reflection_grade()?;
        if self.reflection.water_count > 0 {
            self.draw_water(matrix);
        }
        self.reflection.ran = true;
        Ok(())
    }

    fn raster_reflection_views(&mut self) {
        let eye = Vec3::new(
            self.reflection.eye[0],
            self.reflection.eye[1],
            self.reflection.eye[2],
        );
        let forward = Vec3::new(
            self.reflection.forward[0],
            self.reflection.forward[1],
            self.reflection.forward[2],
        );
        if self.reflection.fb.is_null() || self.reflection.view_cap == 0 {
            return;
        }
        let mut drawn = 0u32;
        for index in 0..self.reflection.plane_count {
            if drawn >= self.reflection.view_cap {
                break;
            }
            let slot = usize::try_from(index).unwrap_or(0);
            let plane = self.reflection.planes[slot];
            if plane.kind == ReflectorKind::Water {
                continue;
            }
            let origin = Vec3::new(plane.origin[0], plane.origin[1], plane.origin[2]);
            let normal = Vec3::new(plane.normal[0], plane.normal[1], plane.normal[2]);
            let view = if plane.kind == ReflectorKind::Portal {
                let to = Vec3::new(plane.portal_to[0], plane.portal_to[1], plane.portal_to[2]);
                portal_view_proj(eye, forward, self.reflection.aspect, origin, normal, to)
            } else {
                mirror_view_proj(eye, forward, self.reflection.aspect, origin, normal)
            };
            if drawn > 0 {
                self.reflection.pass_override = self.reflection.later_pass;
            }
            self.raster_scene(
                self.reflection.fb,
                self.extent_w,
                self.extent_h,
                &view,
                false,
            );
            self.reflection.pass_override = std::ptr::null_mut();
            drawn += 1;
        }
    }

    fn dispatch_reflection_grade(&mut self) -> Result<(), String> {
        let pc = [self.reflection.preset, 0u32, 0, 0];
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, self.reflection.pipe);
            (self.fns.cmd_bind_set)(
                self.cmd,
                1,
                self.reflection.pipe_layout,
                0,
                1,
                &self.reflection.set,
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_push)(
                self.cmd,
                self.reflection.pipe_layout,
                0x20,
                0,
                16,
                pc.as_ptr() as *const c_void,
            );
            (self.fns.cmd_dispatch)(self.cmd, 3, 1, 1);
        }
        self.buffer_barrier(self.reflection.grade.buffer, 0x800, 0x4000, 0x40, 0x2000);
        Ok(())
    }

    fn draw_water(&mut self, matrix: &[f32; 16]) {
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
            (self.fns.cmd_bind_pipe)(self.cmd, 0, self.reflection.water_pipe);
            let viewport = [
                0.0f32,
                0.0,
                self.extent_w as f32,
                self.extent_h as f32,
                0.0,
                1.0,
            ];
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
            let scissor = [0i32, 0, self.extent_w as i32, self.extent_h as i32];
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
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
            (self.fns.cmd_push)(
                self.cmd,
                self.layout,
                1,
                0,
                64,
                matrix.as_ptr() as *const c_void,
            );
            let vb = [self.vertex.buffer];
            let off = [0u64];
            (self.fns.cmd_bind_vb)(self.cmd, 0, 1, vb.as_ptr(), off.as_ptr());
            (self.fns.cmd_draw)(
                self.cmd,
                self.reflection.water_count,
                1,
                self.reflection.water_first,
                0,
            );
            (self.fns.cmd_end_rp)(self.cmd);
        }
    }

    fn ensure_reflection_targets(&mut self) -> Result<(), String> {
        if self.reflection.pipe.is_null() {
            self.make_reflection_pipe()?;
        }
        if self.reflection.water_pipe.is_null() && self.reflection.water_count > 0 {
            self.reflection.water_pipe = self.make_water_pipeline()?;
        }
        let needs_view = self.reflection.view_cap > 0
            && self.reflection.planes[..usize::try_from(self.reflection.plane_count).unwrap_or(0)]
                .iter()
                .any(|plane| plane.kind != ReflectorKind::Water);
        if !needs_view {
            return Ok(());
        }
        if self.reflection.later_pass.is_null() {
            self.reflection.later_pass = self.make_reflect_reload_pass()?;
        }
        if self.reflection.target_w == self.extent_w
            && self.reflection.target_h == self.extent_h
            && !self.reflection.fb.is_null()
        {
            return Ok(());
        }
        self.destroy_reflection_targets();
        self.reflection.color = self.make_image(
            self.format,
            0x10 | 0x1 | 0x2,
            1,
            self.extent_w,
            self.extent_h,
        )?;
        self.reflection.depth = self.make_image(126, 0x20, 2, self.extent_w, self.extent_h)?;
        let views = [self.reflection.color.view, self.reflection.depth.view];
        self.reflection.fb = self.make_framebuffer_for(
            self.render_pass,
            &views,
            self.extent_w,
            self.extent_h,
        )?;
        self.reflection.target_w = self.extent_w;
        self.reflection.target_h = self.extent_h;
        Ok(())
    }

    fn make_water_pipeline(&mut self) -> Result<Handle, String> {
        let saved = self.frag_override;
        self.frag_override = Some(REFLECTION_WATER_SPV);
        let pipe = self.make_raster_pipeline(false, true, false, false);
        self.frag_override = saved;
        pipe
    }

    fn make_reflection_pipe(&mut self) -> Result<(), String> {
        self.reflection.grade = self.make_buffer(GRADE_BYTES as u64, 0x20, Memory::Readback)?;
        #[repr(C)]
        struct Binding {
            binding: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        let binding = Binding {
            binding: 0,
            kind: 7,
            count: 1,
            stages: 0x20,
            samplers: std::ptr::null(),
        };
        #[repr(C)]
        struct SetInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            count: u32,
            bindings: *const Binding,
        }
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 1,
            bindings: &binding,
        };
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.reflection.desc_layout,
                ),
                "reflection descriptor layout",
            )?;
        }
        #[repr(C)]
        struct Range {
            stage: u32,
            offset: u32,
            size: u32,
        }
        let push = Range {
            stage: 0x20,
            offset: 0,
            size: 16,
        };
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
        let sets = [self.reflection.desc_layout];
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
                    &mut self.reflection.pipe_layout,
                ),
                "reflection pipeline layout",
            )?;
        }
        let module = self.shader(REFLECTION_FRAME_SPV)?;
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
            layout: self.reflection.pipe_layout,
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
                &mut self.reflection.pipe,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "reflection compute pipeline")?;
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
                    &mut self.reflection.pool,
                ),
                "reflection descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [self.reflection.desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.reflection.pool,
                count: 1,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    &mut self.reflection.set,
                ),
                "reflection descriptor set",
            )?;
        }
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
            buffer: self.reflection.grade.buffer,
            offset: 0,
            range: u64::MAX,
        };
        let write = Write {
            s_type: 35,
            next: std::ptr::null(),
            set: self.reflection.set,
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

    fn make_reflect_reload_pass(&mut self) -> Result<Handle, String> {
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
        let atts = [
            Att {
                flags: 0,
                format: self.format,
                samples: 1,
                load: 1,
                store: 0,
                stencil_load: 1,
                stencil_store: 1,
                initial: 6,
                final_layout: 6,
            },
            Att {
                flags: 0,
                format: 126,
                samples: 1,
                load: 1,
                store: 1,
                stencil_load: 1,
                stencil_store: 1,
                initial: 3,
                final_layout: 3,
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
                src_stage: 0x1000,
                dst_stage: 0x400,
                src_access: 0x800,
                dst_access: 0x180,
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
                "reflection reload pass",
            )?;
        }
        Ok(pass)
    }

    fn read_reflection_grade(&self) -> Result<reflections::ReflectionGrade, String> {
        if !self.reflection.ran || self.reflection.grade.buffer.is_null() {
            return Ok(reflections::ReflectionGrade {
                ran: false,
                color: Vec::new(),
                depth: Vec::new(),
                rays: 0,
                views: 0,
            });
        }
        let mut bytes = vec![0u8; GRADE_BYTES];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    self.reflection.grade.memory,
                    0,
                    self.reflection.grade.size,
                    0,
                    &mut mapped,
                ),
                "map reflection grade",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u8, bytes.as_mut_ptr(), GRADE_BYTES);
            (self.fns.unmap_mem)(self.device, self.reflection.grade.memory);
        }
        Ok(reflections::reflection_grade_from_bytes(&bytes).unwrap_or(
            reflections::ReflectionGrade {
                ran: false,
                color: Vec::new(),
                depth: Vec::new(),
                rays: 0,
                views: 0,
            },
        ))
    }

    fn destroy_reflection_targets(&mut self) {
        unsafe {
            if !self.reflection.fb.is_null() {
                (self.fns.destroy_framebuffer)(self.device, self.reflection.fb, std::ptr::null());
                self.reflection.fb = std::ptr::null_mut();
            }
        }
        let mut color = std::mem::replace(&mut self.reflection.color, Image::empty());
        let mut depth = std::mem::replace(&mut self.reflection.depth, Image::empty());
        self.destroy_image(&mut color);
        self.destroy_image(&mut depth);
        self.reflection.target_w = 0;
        self.reflection.target_h = 0;
    }

    fn destroy_reflection(&mut self) {
        self.destroy_reflection_targets();
        unsafe {
            if !self.reflection.water_pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.reflection.water_pipe, std::ptr::null());
            }
            if !self.reflection.pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.reflection.pipe, std::ptr::null());
            }
            if !self.reflection.pipe_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.reflection.pipe_layout, std::ptr::null());
            }
            if !self.reflection.pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.reflection.pool, std::ptr::null());
            }
            if !self.reflection.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(
                    self.device,
                    self.reflection.desc_layout,
                    std::ptr::null(),
                );
            }
            if !self.reflection.later_pass.is_null() {
                (self.fns.destroy_render_pass)(
                    self.device,
                    self.reflection.later_pass,
                    std::ptr::null(),
                );
            }
        }
        let mut grade = std::mem::replace(&mut self.reflection.grade, Buffer::empty());
        self.destroy_buffer(&mut grade);
        self.reflection.water_pipe = std::ptr::null_mut();
        self.reflection.pipe = std::ptr::null_mut();
        self.reflection.pipe_layout = std::ptr::null_mut();
        self.reflection.pool = std::ptr::null_mut();
        self.reflection.desc_layout = std::ptr::null_mut();
        self.reflection.set = std::ptr::null_mut();
        self.reflection.later_pass = std::ptr::null_mut();
    }
}
