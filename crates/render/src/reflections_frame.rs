// Mirror and portal views, composited into the lit frame, then water.
// Included from `gpu.rs`. An empty reflector list returns before any of this
// runs, so a Lambert frame does not bind these pipelines.
//
// A mirror camera is `mirror_view_proj` (Rust oblique clip). A portal camera
// is `reflections_portal.vert`, which calls `reflection_portal_point`. Water
// samples the copied scene depth. The ray counter is `reflection_ray_cap`.

use genos_math::Vec3;
use genos_scene::look_direction;

use crate::reflections::{self, mirror_view_proj, GRADE_BYTES};
use crate::world::{Reflector, ReflectorKind, ShadePlane};

/// Reflectors and shade planes share [`crate::reflections::REFLECTOR_CAP`].
const PLANES: usize = crate::reflections::REFLECTOR_CAP;
const WATER_BYTES: u64 = 112 + 12 * 80;
const _: () = assert!(WATER_BYTES == 112 + (PLANES as u64) * 80);
const LINK_BYTES: u64 = 13 * 16;
const GGX_BYTES: u64 = 6 * 48;

struct ReflectionGpu {
    planes: [Reflector; PLANES],
    plane_count: u32,
    shade: [ShadePlane; PLANES],
    shade_count: u32,
    sky: [f32; 3],
    swatch: bool,
    /// Kept null. `raster_scene` still reads it for the main picture.
    pass_override: Handle,
    preset: u32,
    rays: u32,
    eye: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    aspect: f32,
    enabled: bool,
    water_on: bool,
    view_cap: u32,
    quad_first: u32,
    ran: bool,
    views_drawn: u32,
    gpu_ns: u64,
    vram_bytes: u64,
    query_slot: u32,
    depth_copied: bool,
    images_live: bool,
    color: Image,
    depth: Image,
    depth_copy: Image,
    fb: Handle,
    clear_pass: Handle,
    composite_pass: Handle,
    grade: Buffer,
    water_buf: Buffer,
    link_buf: Buffer,
    ggx_buf: Buffer,
    query_pool: Handle,
    desc_layout: Handle,
    pipe_layout: Handle,
    pool: Handle,
    set: Handle,
    pipe: Handle,
    albedo_pipe: Handle,
    portal_pipe: Handle,
    portal_layout: Handle,
    portal_set_layout: Handle,
    portal_pool: Handle,
    portal_set: Handle,
    composite_pipe: Handle,
    composite_layout: Handle,
    composite_set_layout: Handle,
    composite_pool: Handle,
    composite_set: Handle,
    water_pipe: Handle,
    water_layout: Handle,
    water_set_layout: Handle,
    water_pool: Handle,
    water_set: Handle,
    ggx_pipe: Handle,
    ggx_layout: Handle,
    sampler: Handle,
    target_w: u32,
    target_h: u32,
}

impl ReflectionGpu {
    fn idle() -> Self {
        Self {
            planes: [Reflector::default(); PLANES],
            plane_count: 0,
            shade: [blank_shade(); PLANES],
            shade_count: 0,
            sky: [0.45, 0.58, 0.72],
            swatch: false,
            pass_override: std::ptr::null_mut(),
            preset: 3,
            rays: 0,
            eye: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            aspect: 1.0,
            enabled: false,
            water_on: false,
            view_cap: 0,
            quad_first: 0,
            ran: false,
            views_drawn: 0,
            gpu_ns: 0,
            vram_bytes: 0,
            query_slot: 0,
            depth_copied: false,
            images_live: false,
            color: Image::empty(),
            depth: Image::empty(),
            depth_copy: Image::empty(),
            fb: std::ptr::null_mut(),
            clear_pass: std::ptr::null_mut(),
            composite_pass: std::ptr::null_mut(),
            grade: Buffer::empty(),
            water_buf: Buffer::empty(),
            link_buf: Buffer::empty(),
            ggx_buf: Buffer::empty(),
            query_pool: std::ptr::null_mut(),
            desc_layout: std::ptr::null_mut(),
            pipe_layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            set: std::ptr::null_mut(),
            pipe: std::ptr::null_mut(),
            albedo_pipe: std::ptr::null_mut(),
            portal_pipe: std::ptr::null_mut(),
            portal_layout: std::ptr::null_mut(),
            portal_set_layout: std::ptr::null_mut(),
            portal_pool: std::ptr::null_mut(),
            portal_set: std::ptr::null_mut(),
            composite_pipe: std::ptr::null_mut(),
            composite_layout: std::ptr::null_mut(),
            composite_set_layout: std::ptr::null_mut(),
            composite_pool: std::ptr::null_mut(),
            composite_set: std::ptr::null_mut(),
            water_pipe: std::ptr::null_mut(),
            water_layout: std::ptr::null_mut(),
            water_set_layout: std::ptr::null_mut(),
            water_pool: std::ptr::null_mut(),
            water_set: std::ptr::null_mut(),
            ggx_pipe: std::ptr::null_mut(),
            ggx_layout: std::ptr::null_mut(),
            sampler: std::ptr::null_mut(),
            target_w: 0,
            target_h: 0,
        }
    }
}

fn blank_shade() -> ShadePlane {
    ShadePlane {
        origin: [0.0; 3],
        normal: [0.0, 1.0, 0.0],
        tangent: [1.0, 0.0, 0.0],
        bitangent: [0.0, 0.0, 1.0],
        half_u: 1.0,
        half_v: 1.0,
        color: [1.0, 1.0, 1.0],
    }
}

fn preset_index(views: u32, rays: u32) -> u32 {
    match (views, rays) {
        (0, 0) => 0,
        (1, 0) => 1,
        (_, count) if count <= 32_000 => 2,
        _ => 3,
    }
}

fn camera_axes(forward: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let helper = if forward[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let right = normalize(cross(forward, helper));
    let up = normalize(cross(right, forward));
    (right, up)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1.0e-8 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

impl Gpu {
    fn note_reflections(
        &mut self,
        world: &crate::world::World,
        camera: &genos_scene::Camera,
        aspect: f32,
        quad_first: u32,
        _quad_count: u32,
    ) {
        let settings = world.scene.reflections;
        let mut planes = [Reflector::default(); PLANES];
        let mut count = 0u32;
        for reflector in world.reflectors.iter().take(PLANES) {
            let index = usize::try_from(count).unwrap_or(0);
            planes[index] = *reflector;
            count += 1;
        }
        let mut shade = [blank_shade(); PLANES];
        let mut shade_count = 0u32;
        for plane in world.shade_planes.iter().take(PLANES) {
            let index = usize::try_from(shade_count).unwrap_or(0);
            shade[index] = *plane;
            shade_count += 1;
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
        let forward = [forward.x, forward.y, forward.z];
        let (right, up) = camera_axes(forward);
        self.reflection.planes = planes;
        self.reflection.plane_count = count;
        self.reflection.shade = shade;
        self.reflection.shade_count = shade_count;
        self.reflection.sky = world.reflection_sky;
        self.reflection.swatch = world.reflection_swatch;
        self.reflection.preset = preset_index(settings.mirror_views, settings.rays);
        self.reflection.rays = settings.rays;
        self.reflection.eye = [camera.position.x, camera.position.y, camera.position.z];
        self.reflection.forward = forward;
        self.reflection.right = right;
        self.reflection.up = up;
        self.reflection.aspect = aspect;
        self.reflection.enabled = active;
        self.reflection.water_on = settings.water;
        // The budget is the reflection settings, not the lighting preset.
        self.reflection.view_cap = settings.mirror_views;
        self.reflection.quad_first = quad_first;
        self.reflection.ran = false;
        self.reflection.views_drawn = 0;
        self.reflection.gpu_ns = 0;
    }

    fn reflections_after_lighting(&mut self, matrix: &[f32; 16]) -> Result<(), String> {
        if !self.reflection.enabled {
            return Ok(());
        }
        self.ensure_reflection_targets()?;
        self.reflection.query_slot = u32::try_from(self.flight).unwrap_or(0) * 2;
        self.stamp_reflection(0, 1);
        self.copy_scene_depth();
        self.upload_water_buffer()?;
        let mut drawn = 0u32;
        let count = self.reflection.plane_count;
        for index in 0..count {
            let slot = usize::try_from(index).unwrap_or(0);
            let plane = self.reflection.planes[slot];
            if plane.kind == ReflectorKind::Water {
                continue;
            }
            // View cap. A later mirror keeps its albedo.
            let sample = if drawn < self.reflection.view_cap {
                self.raster_reflection_view(slot)?;
                drawn += 1;
                1.0
            } else {
                0.0
            };
            self.composite_reflector(matrix, index, sample);
        }
        self.reflection.views_drawn = drawn;
        if self.reflection.water_on {
            self.draw_water_surfaces(matrix);
        }
        if self.reflection.swatch {
            self.draw_ggx_swatch()?;
        }
        self.dispatch_reflection_rays()?;
        self.stamp_reflection(1, 0x2000);
        self.reflection.ran = true;
        Ok(())
    }

    fn raster_reflection_view(&mut self, slot: usize) -> Result<(), String> {
        let plane = self.reflection.planes[slot];
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
        let origin = Vec3::new(plane.origin[0], plane.origin[1], plane.origin[2]);
        let normal = Vec3::new(plane.normal[0], plane.normal[1], plane.normal[2]);
        let portal = plane.kind == ReflectorKind::Portal;
        if portal {
            self.upload_portal_link(&plane)?;
        }
        let view = if portal {
            [0.0; 16]
        } else {
            mirror_view_proj(eye, forward, self.reflection.aspect, origin, normal)
        };
        self.prepare_reflection_target();
        // The mirror image stays the displayed frame. The guard lives on the scene target.
        self.begin_target_pass(
            self.reflection.clear_pass,
            self.reflection.fb,
            true,
            self.extent_w,
            self.extent_h,
        );
        unsafe {
            let pipe = if portal {
                self.reflection.portal_pipe
            } else {
                self.reflection.albedo_pipe
            };
            (self.fns.cmd_bind_pipe)(self.cmd, 0, pipe);
            if portal {
                (self.fns.cmd_bind_set)(
                    self.cmd,
                    0,
                    self.reflection.portal_layout,
                    0,
                    1,
                    &self.reflection.portal_set,
                    0,
                    std::ptr::null(),
                );
            } else {
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
                    view.as_ptr() as *const c_void,
                );
            }
            self.set_viewport(self.extent_w, self.extent_h);
            self.set_scissor(0, 0, self.extent_w, self.extent_h);
            self.draw_scene_spans();
            (self.fns.cmd_end_rp)(self.cmd);
        }
        self.barrier_reflection_sample();
        Ok(())
    }

    fn composite_reflector(&self, matrix: &[f32; 16], index: u32, sample: f32) {
        let slot = usize::try_from(index).unwrap_or(0);
        let plane = self.reflection.planes[slot];
        let (lit_w, lit_h) = self.lit_extent();
        // The scene target is the lit picture. A display-sized render area would
        // drop the crop's far edge, and the presented frame would keep the base colour.
        self.begin_target_pass(
            self.reflection.composite_pass,
            self.framebuffer,
            false,
            lit_w,
            lit_h,
        );
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 0, self.reflection.composite_pipe);
            (self.fns.cmd_bind_set)(
                self.cmd,
                0,
                self.reflection.composite_layout,
                0,
                1,
                &self.reflection.composite_set,
                0,
                std::ptr::null(),
            );
            self.set_viewport(lit_w, lit_h);
            let scissor = plane.scissor;
            if scissor[2] == 0 || scissor[3] == 0 {
                self.set_display_scissor(0, 0, self.extent_w, self.extent_h);
            } else {
                self.set_display_scissor(scissor[0], scissor[1], scissor[2], scissor[3]);
            }
            (self.fns.cmd_push)(
                self.cmd,
                self.reflection.composite_layout,
                1,
                0,
                64,
                matrix.as_ptr() as *const c_void,
            );
            let params = [self.extent_w as f32, self.extent_h as f32, sample, 0.0];
            (self.fns.cmd_push)(
                self.cmd,
                self.reflection.composite_layout,
                0x10,
                64,
                16,
                params.as_ptr() as *const c_void,
            );
            let (ox, oy) = self.display_origin();
            let frame = [ox as f32, oy as f32, 0.0, 0.0];
            (self.fns.cmd_push)(
                self.cmd,
                self.reflection.composite_layout,
                0x10,
                80,
                16,
                frame.as_ptr() as *const c_void,
            );
            self.bind_dynamic_verts();
            (self.fns.cmd_draw)(self.cmd, 6, 1, self.reflection.quad_first + index * 6, 0);
            (self.fns.cmd_end_rp)(self.cmd);
        }
    }

    fn draw_water_surfaces(&self, matrix: &[f32; 16]) {
        let count = self.reflection.plane_count;
        let mut opened = false;
        for index in 0..count {
            let slot = usize::try_from(index).unwrap_or(0);
            let plane = self.reflection.planes[slot];
            if plane.kind != ReflectorKind::Water {
                continue;
            }
            if !opened {
                let (lit_w, lit_h) = self.lit_extent();
                self.begin_target_pass(
                    self.reflection.composite_pass,
                    self.framebuffer,
                    false,
                    lit_w,
                    lit_h,
                );
                unsafe {
                    (self.fns.cmd_bind_pipe)(self.cmd, 0, self.reflection.water_pipe);
                    (self.fns.cmd_bind_set)(
                        self.cmd,
                        0,
                        self.reflection.water_layout,
                        0,
                        1,
                        &self.reflection.water_set,
                        0,
                        std::ptr::null(),
                    );
                    self.set_viewport(lit_w, lit_h);
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.reflection.water_layout,
                        1,
                        0,
                        64,
                        matrix.as_ptr() as *const c_void,
                    );
                    self.bind_dynamic_verts();
                }
                opened = true;
            }
            let scissor = plane.scissor;
            if scissor[2] == 0 || scissor[3] == 0 {
                self.set_display_scissor(0, 0, self.extent_w, self.extent_h);
            } else {
                self.set_display_scissor(scissor[0], scissor[1], scissor[2], scissor[3]);
            }
            unsafe {
                (self.fns.cmd_draw)(self.cmd, 6, 1, self.reflection.quad_first + index * 6, 0);
            }
        }
        if opened {
            unsafe {
                (self.fns.cmd_end_rp)(self.cmd);
            }
        }
    }

    fn draw_ggx_swatch(&mut self) -> Result<(), String> {
        self.upload_ggx_quad()?;
        let (lit_w, lit_h) = self.lit_extent();
        self.begin_target_pass(
            self.reflection.composite_pass,
            self.framebuffer,
            false,
            lit_w,
            lit_h,
        );
        let identity = crate::world::identity_pose();
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 0, self.reflection.ggx_pipe);
            let (ox, oy) = self.display_origin();
            self.set_viewport_rect(
                ox as f32,
                oy as f32,
                self.extent_w as f32,
                self.extent_h as f32,
            );
            self.set_scissor(ox, oy, 8, 8);
            (self.fns.cmd_push)(
                self.cmd,
                self.reflection.ggx_layout,
                1,
                0,
                64,
                identity.as_ptr() as *const c_void,
            );
            let vb = [self.reflection.ggx_buf.buffer];
            let off = [0u64];
            (self.fns.cmd_bind_vb)(self.cmd, 0, 1, vb.as_ptr(), off.as_ptr());
            (self.fns.cmd_draw)(self.cmd, 6, 1, 0, 0);
            (self.fns.cmd_end_rp)(self.cmd);
        }
        Ok(())
    }

    fn dispatch_reflection_rays(&mut self) -> Result<(), String> {
        unsafe {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.reflection.grade.buffer,
                0,
                GRADE_BYTES as u64,
                0,
            );
        }
        self.buffer_barrier(self.reflection.grade.buffer, 0x1000, 0x800, 0x1000, 0x40);
        // One group still runs when the cap is 0, so the shader's cap is the count.
        let launch = self.reflection.rays.max(64);
        let groups = launch.div_ceil(64);
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
            (self.fns.cmd_dispatch)(self.cmd, groups, 1, 1);
        }
        self.buffer_barrier(self.reflection.grade.buffer, 0x800, 0x4000, 0x40, 0x2000);
        Ok(())
    }

    fn upload_water_buffer(&mut self) -> Result<(), String> {
        let mut bytes = vec![0u8; usize::try_from(WATER_BYTES).unwrap_or(0)];
        let mut at = 0usize;
        let push = |bytes: &mut [u8], at: &mut usize, value: f32| {
            let next = *at + 4;
            if next <= bytes.len() {
                bytes[*at..next].copy_from_slice(&value.to_le_bytes());
            }
            *at = next;
        };
        let push4 = |bytes: &mut [u8], at: &mut usize, value: [f32; 4]| {
            for channel in value {
                push(bytes, at, channel);
            }
        };
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.eye[0],
                self.reflection.eye[1],
                self.reflection.eye[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.forward[0],
                self.reflection.forward[1],
                self.reflection.forward[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.right[0],
                self.reflection.right[1],
                self.reflection.right[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.up[0],
                self.reflection.up[1],
                self.reflection.up[2],
                self.reflection.aspect,
            ],
        );
        let tan_half = (30.0_f32.to_radians()).tan();
        push4(
            &mut bytes,
            &mut at,
            [0.55, tan_half, self.extent_w as f32, self.extent_h as f32],
        );
        let (ox, oy) = self.display_origin();
        let (lit_w, lit_h) = self.lit_extent();
        push4(
            &mut bytes,
            &mut at,
            [ox as f32, oy as f32, lit_w as f32, lit_h as f32],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.sky[0],
                self.reflection.sky[1],
                self.reflection.sky[2],
                self.reflection.shade_count as f32,
            ],
        );
        for index in 0..PLANES {
            let plane = self.reflection.shade[index];
            push4(
                &mut bytes,
                &mut at,
                [
                    plane.origin[0],
                    plane.origin[1],
                    plane.origin[2],
                    plane.half_u,
                ],
            );
            push4(
                &mut bytes,
                &mut at,
                [
                    plane.normal[0],
                    plane.normal[1],
                    plane.normal[2],
                    plane.half_v,
                ],
            );
            push4(
                &mut bytes,
                &mut at,
                [plane.color[0], plane.color[1], plane.color[2], 0.0],
            );
            push4(
                &mut bytes,
                &mut at,
                [plane.tangent[0], plane.tangent[1], plane.tangent[2], 0.0],
            );
            push4(
                &mut bytes,
                &mut at,
                [
                    plane.bitangent[0],
                    plane.bitangent[1],
                    plane.bitangent[2],
                    0.0,
                ],
            );
        }
        let dst = self.reflection.water_buf.buffer;
        self.update_buffer(dst, &bytes)?;
        self.buffer_barrier(self.reflection.water_buf.buffer, 0x1000, 0x80, 0x1000, 0x20);
        Ok(())
    }

    fn upload_portal_link(&mut self, plane: &Reflector) -> Result<(), String> {
        let mut bytes = vec![0u8; usize::try_from(LINK_BYTES).unwrap_or(0)];
        let mut at = 0usize;
        let push4 = |bytes: &mut [u8], at: &mut usize, value: [f32; 4]| {
            for channel in value {
                let next = *at + 4;
                if next <= bytes.len() {
                    bytes[*at..next].copy_from_slice(&channel.to_le_bytes());
                }
                *at = next;
            }
        };
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.eye[0],
                self.reflection.eye[1],
                self.reflection.eye[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.forward[0],
                self.reflection.forward[1],
                self.reflection.forward[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [plane.origin[0], plane.origin[1], plane.origin[2], 0.0],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                plane.normal[0],
                plane.normal[1],
                plane.normal[2],
                self.reflection.aspect,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                plane.portal_to[0],
                plane.portal_to[1],
                plane.portal_to[2],
                0.0,
            ],
        );
        for basis in [plane.from_basis, plane.to_basis] {
            for column in 0..3 {
                let o = column * 3;
                push4(
                    &mut bytes,
                    &mut at,
                    [basis[o], basis[o + 1], basis[o + 2], 0.0],
                );
            }
        }
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.right[0],
                self.reflection.right[1],
                self.reflection.right[2],
                0.0,
            ],
        );
        push4(
            &mut bytes,
            &mut at,
            [
                self.reflection.up[0],
                self.reflection.up[1],
                self.reflection.up[2],
                0.0,
            ],
        );
        let dst = self.reflection.link_buf.buffer;
        self.update_buffer(dst, &bytes)?;
        self.buffer_barrier(self.reflection.link_buf.buffer, 0x1000, 0x8, 0x1000, 0x20);
        self.write_storage(
            self.reflection.portal_set,
            1,
            &self.instance_bufs[self.flight],
        );
        Ok(())
    }

    fn upload_ggx_quad(&mut self) -> Result<(), String> {
        let width = self.extent_w.max(1) as f32;
        let height = self.extent_h.max(1) as f32;
        let x1 = -1.0 + 16.0 / width;
        let y1 = -1.0 + 16.0 / height;
        let vert = |x: f32, y: f32| GpuVertex {
            pos: [x, y, 0.0],
            albedo: [1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            shade: 1.0,
            uv: [0.0, 0.0],
        };
        let verts = [
            vert(-1.0, -1.0),
            vert(x1, -1.0),
            vert(x1, y1),
            vert(-1.0, -1.0),
            vert(x1, y1),
            vert(-1.0, y1),
        ];
        let bytes = unsafe {
            std::slice::from_raw_parts(verts.as_ptr() as *const u8, std::mem::size_of_val(&verts))
        };
        let dst = self.reflection.ggx_buf.buffer;
        self.update_buffer(dst, bytes)?;
        self.buffer_barrier(self.reflection.ggx_buf.buffer, 0x1000, 0x4, 0x1000, 0x4);
        Ok(())
    }

    fn update_buffer(&mut self, buffer: Handle, bytes: &[u8]) -> Result<(), String> {
        let cmd = self.cmd;
        self.write_device(cmd, buffer, 0, bytes)
    }

    fn copy_scene_depth(&mut self) {
        if self.reflection.depth_copy.image.is_null() {
            return;
        }
        self.barrier_aspect(self.depth.image, 2, 3, 6, 0x200, 0x1000, 0x400, 0x800);
        let old = if self.reflection.depth_copied { 5 } else { 0 };
        self.barrier_aspect(
            self.reflection.depth_copy.image,
            2,
            old,
            7,
            1,
            0x1000,
            0,
            0x1000,
        );
        self.copy_depth(self.depth.image, 6, self.reflection.depth_copy.image, 7);
        self.barrier_aspect(
            self.reflection.depth_copy.image,
            2,
            7,
            5,
            0x1000,
            0x80,
            0x1000,
            0x20,
        );
        self.barrier_aspect(
            self.depth.image,
            2,
            6,
            3,
            0x1000,
            0x100,
            0x800,
            0x200 | 0x400,
        );
        self.reflection.depth_copied = true;
    }

    fn copy_depth(&self, src: Handle, src_layout: i32, dst: Handle, dst_layout: i32) {
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
            aspect: 2,
            mip: 0,
            layer: 0,
            count: 1,
        };
        let (ox, oy) = self.display_origin();
        let region = Region {
            src_layers: layers,
            src_offset: Offset {
                x: i32::try_from(ox).unwrap_or(0),
                y: i32::try_from(oy).unwrap_or(0),
                z: 0,
            },
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
                src,
                src_layout,
                dst,
                dst_layout,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn prepare_reflection_target(&mut self) {
        if self.reflection.images_live {
            return;
        }
        self.barrier_aspect(self.reflection.color.image, 1, 0, 5, 1, 0x80, 0, 0x20);
        self.barrier_aspect(self.reflection.depth.image, 2, 0, 5, 1, 0x80, 0, 0x20);
        self.reflection.images_live = true;
    }

    fn barrier_reflection_sample(&self) {
        self.barrier_aspect(
            self.reflection.color.image,
            1,
            6,
            5,
            0x1000,
            0x80,
            0x800,
            0x20,
        );
        self.barrier_aspect(
            self.reflection.depth.image,
            2,
            3,
            5,
            0x200,
            0x80,
            0x400,
            0x20,
        );
    }

    fn barrier_aspect(
        &self,
        image: Handle,
        aspect: u32,
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
                aspect,
                mip: 0,
                levels: 1,
                layer: 0,
                layers: 1,
            },
        };
        unsafe {
            (self.fns.cmd_barrier)(
                self.cmd,
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

    fn begin_target_pass(&self, pass: Handle, fb: Handle, clear: bool, width: u32, height: u32) {
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
        let [r, g, b] = self.background;
        let mut clears = [[r, g, b, 1.0], [1.0, 0.0, 0.0, 0.0]];
        let rp = RpBegin {
            s_type: 43,
            next: std::ptr::null(),
            pass,
            fb,
            area: Rect {
                offset: Offset { x: 0, y: 0 },
                extent: Extent {
                    w: width.max(1),
                    h: height.max(1),
                },
            },
            clear_count: if clear { 2 } else { 0 },
            clears: if clear {
                clears.as_mut_ptr() as *const f32
            } else {
                std::ptr::null()
            },
        };
        unsafe {
            (self.fns.cmd_begin_rp)(self.cmd, &rp as *const RpBegin as *const u8, 0);
        }
    }

    fn set_viewport(&self, width: u32, height: u32) {
        self.set_viewport_rect(0.0, 0.0, width as f32, height as f32);
    }

    fn set_viewport_rect(&self, x: f32, y: f32, width: f32, height: f32) {
        let viewport = [x, y, width, height, 0.0, 1.0];
        unsafe {
            (self.fns.cmd_viewport)(self.cmd, 0, 1, viewport.as_ptr());
        }
    }

    /// Display size inside the lit picture. A zero lit target is the swapchain.
    fn lit_extent(&self) -> (u32, u32) {
        if self.lit_w == 0 || self.lit_h == 0 {
            (self.extent_w.max(1), self.extent_h.max(1))
        } else {
            (self.lit_w, self.lit_h)
        }
    }

    /// Crop origin. The presented pixel `(x, y)` is lit pixel `(x + ox, y + oy)`.
    fn display_origin(&self) -> (u32, u32) {
        if self.lit_w == 0 || self.lit_h == 0 {
            (0, 0)
        } else {
            (self.crop_x, self.crop_y)
        }
    }

    /// A display-space scissor, shifted onto the lit picture.
    fn set_display_scissor(&self, x: u32, y: u32, w: u32, h: u32) {
        let (ox, oy) = self.display_origin();
        self.set_scissor(x.saturating_add(ox), y.saturating_add(oy), w, h);
    }

    fn set_scissor(&self, x: u32, y: u32, w: u32, h: u32) {
        let scissor = [
            i32::try_from(x).unwrap_or(i32::MAX),
            i32::try_from(y).unwrap_or(i32::MAX),
            i32::try_from(w).unwrap_or(i32::MAX),
            i32::try_from(h).unwrap_or(i32::MAX),
        ];
        unsafe {
            (self.fns.cmd_scissor)(self.cmd, 0, 1, scissor.as_ptr());
        }
    }

    fn bind_dynamic_verts(&self) {
        let vb = [self.vertex.buffer];
        let off = [0u64];
        unsafe {
            (self.fns.cmd_bind_vb)(self.cmd, 0, 1, vb.as_ptr(), off.as_ptr());
        }
    }

    fn draw_scene_spans(&self) {
        let mut bound = std::ptr::null_mut();
        unsafe {
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
        }
    }

    fn stamp_reflection(&self, index: u32, stage: u32) {
        if self.reflection.query_pool.is_null() || self.timestamp_period <= 0.0 {
            return;
        }
        let query = self.reflection.query_slot + index;
        if index == 0 {
            unsafe {
                (self.fns.cmd_reset_query)(self.cmd, self.reflection.query_pool, query, 2);
            }
        }
        unsafe {
            (self.fns.cmd_write_timestamp)(self.cmd, stage, self.reflection.query_pool, query);
        }
    }

    fn ensure_reflection_targets(&mut self) -> Result<(), String> {
        if self.reflection.pipe.is_null() {
            self.make_reflection_pipes()?;
        }
        let pixels = u64::from(self.extent_w) * u64::from(self.extent_h);
        self.reflection.vram_bytes =
            pixels * 4 * 3 + GRADE_BYTES as u64 + WATER_BYTES + LINK_BYTES + GGX_BYTES;
        if self.reflection.target_w == self.extent_w
            && self.reflection.target_h == self.extent_h
            && !self.reflection.fb.is_null()
        {
            return Ok(());
        }
        self.destroy_reflection_targets();
        self.reflection.color = self.make_image(
            self.format,
            0x10 | 0x4 | 0x1 | 0x2,
            1,
            self.extent_w,
            self.extent_h,
        )?;
        self.reflection.depth =
            self.make_image(126, 0x20 | 0x4, 2, self.extent_w, self.extent_h)?;
        self.reflection.depth_copy =
            self.make_image(126, 0x4 | 0x2, 2, self.extent_w, self.extent_h)?;
        let views = [self.reflection.color.view, self.reflection.depth.view];
        self.reflection.fb = self.make_framebuffer_for(
            self.reflection.clear_pass,
            &views,
            self.extent_w,
            self.extent_h,
        )?;
        self.reflection.target_w = self.extent_w;
        self.reflection.target_h = self.extent_h;
        self.reflection.depth_copied = false;
        self.reflection.images_live = false;
        self.write_combined(self.reflection.composite_set, 0, self.reflection.color.view);
        self.write_combined(self.reflection.composite_set, 1, self.reflection.depth.view);
        self.write_storage(self.reflection.water_set, 0, &self.reflection.water_buf);
        self.write_combined(
            self.reflection.water_set,
            1,
            self.reflection.depth_copy.view,
        );
        Ok(())
    }

    fn make_reflection_pipes(&mut self) -> Result<(), String> {
        self.reflection.clear_pass = self.make_reflect_pass(1, 5, 1, 5, 0)?;
        self.reflection.composite_pass = self.make_reflect_pass(0, 6, 0, 3, 0)?;
        self.reflection.grade =
            self.make_buffer(GRADE_BYTES as u64, 0x20 | 0x2, Memory::Readback)?;
        self.reflection.water_buf = self.make_buffer(WATER_BYTES, 0x20 | 0x2, Memory::Device)?;
        self.reflection.link_buf = self.make_buffer(LINK_BYTES, 0x20 | 0x2, Memory::Device)?;
        self.reflection.ggx_buf = self.make_buffer(GGX_BYTES, 0x80 | 0x2, Memory::Device)?;
        if self.timestamp_period > 0.0 {
            self.reflection.query_pool = self.make_query_pool_n(4)?;
        }
        self.reflection.sampler = self.make_nearest_sampler()?;
        self.make_ray_pipeline()?;
        self.make_portal_pipeline()?;
        self.make_composite_pipeline()?;
        self.make_water_pipeline()?;
        self.make_ggx_pipeline()?;
        self.reflection.albedo_pipe = self.make_reflect_raster(
            std::ptr::null_mut(),
            self.reflection.clear_pass,
            None,
            Some(REFLECTION_ALBEDO_SPV),
            true,
            true,
        )?;
        Ok(())
    }

    fn make_reflect_raster(
        &mut self,
        layout: Handle,
        pass: Handle,
        vert: Option<&'static [u8]>,
        frag: Option<&'static [u8]>,
        depth_test: bool,
        depth_write: bool,
    ) -> Result<Handle, String> {
        let saved_layout = self.layout_override;
        let saved_pass = self.pipe_pass;
        let saved_vert = self.vert_override;
        let saved_frag = self.frag_override;
        let saved_write = self.depth_write_force;
        self.layout_override = layout;
        self.pipe_pass = pass;
        self.vert_override = vert;
        self.frag_override = frag;
        self.depth_write_force = Some(depth_write);
        let pipe = self.make_raster_pipeline(depth_test, false, false, false);
        self.layout_override = saved_layout;
        self.pipe_pass = saved_pass;
        self.vert_override = saved_vert;
        self.frag_override = saved_frag;
        self.depth_write_force = saved_write;
        pipe
    }

    fn make_ray_pipeline(&mut self) -> Result<(), String> {
        self.reflection.desc_layout = self.storage_layout(0x20)?;
        self.reflection.pipe_layout = self.push_layout(self.reflection.desc_layout, 0x20, 0, 16)?;
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
        self.reflection.pool = self.desc_pool_one(7)?;
        self.reflection.set = self.alloc_one(self.reflection.pool, self.reflection.desc_layout)?;
        self.write_storage(self.reflection.set, 0, &self.reflection.grade);
        Ok(())
    }

    fn make_portal_pipeline(&mut self) -> Result<(), String> {
        self.reflection.portal_set_layout = self.two_storage_layout(0x1)?;
        self.reflection.portal_layout = self.set_only_layout(self.reflection.portal_set_layout)?;
        self.reflection.portal_pool = self.desc_pool_kind(7, 2)?;
        self.reflection.portal_set = self.alloc_one(
            self.reflection.portal_pool,
            self.reflection.portal_set_layout,
        )?;
        self.write_storage(self.reflection.portal_set, 0, &self.reflection.link_buf);
        self.reflection.portal_pipe = self.make_reflect_raster(
            self.reflection.portal_layout,
            self.reflection.clear_pass,
            Some(REFLECTION_PORTAL_SPV),
            Some(REFLECTION_ALBEDO_SPV),
            true,
            true,
        )?;
        Ok(())
    }

    fn make_composite_pipeline(&mut self) -> Result<(), String> {
        self.reflection.composite_set_layout = self.two_sampler_layout()?;
        self.reflection.composite_layout =
            self.push_layout(self.reflection.composite_set_layout, 0x11, 0, 96)?;
        self.reflection.composite_pool = self.desc_pool_kind(1, 2)?;
        self.reflection.composite_set = self.alloc_one(
            self.reflection.composite_pool,
            self.reflection.composite_set_layout,
        )?;
        self.reflection.composite_pipe = self.make_reflect_raster(
            self.reflection.composite_layout,
            self.reflection.composite_pass,
            Some(REFLECTION_QUAD_SPV),
            Some(REFLECTION_COMPOSITE_SPV),
            false,
            true,
        )?;
        Ok(())
    }

    fn make_water_pipeline(&mut self) -> Result<(), String> {
        self.reflection.water_set_layout = self.storage_sampler_layout()?;
        self.reflection.water_layout =
            self.push_layout(self.reflection.water_set_layout, 1, 0, 64)?;
        self.reflection.water_pool = self.desc_pool_mixed()?;
        self.reflection.water_set =
            self.alloc_one(self.reflection.water_pool, self.reflection.water_set_layout)?;
        self.reflection.water_pipe = self.make_reflect_raster(
            self.reflection.water_layout,
            self.reflection.composite_pass,
            Some(REFLECTION_QUAD_SPV),
            Some(REFLECTION_WATER_SPV),
            false,
            false,
        )?;
        Ok(())
    }

    fn make_ggx_pipeline(&mut self) -> Result<(), String> {
        self.reflection.ggx_layout = self.push_only_layout(1, 64)?;
        self.reflection.ggx_pipe = self.make_reflect_raster(
            self.reflection.ggx_layout,
            self.reflection.composite_pass,
            Some(REFLECTION_QUAD_SPV),
            Some(REFLECTION_GGX_SPV),
            false,
            false,
        )?;
        Ok(())
    }

    fn make_nearest_sampler(&self) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            mag: u32,
            min: u32,
            mip: u32,
            addr_u: u32,
            addr_v: u32,
            addr_w: u32,
            bias: f32,
            aniso_on: u32,
            max_aniso: f32,
            compare_on: u32,
            compare: u32,
            min_lod: f32,
            max_lod: f32,
            border: u32,
            unnorm: u32,
        }
        let info = Info {
            s_type: 31,
            next: std::ptr::null(),
            flags: 0,
            mag: 0,
            min: 0,
            mip: 0,
            addr_u: 2,
            addr_v: 2,
            addr_w: 2,
            bias: 0.0,
            aniso_on: 0,
            max_aniso: 1.0,
            compare_on: 0,
            compare: 0,
            min_lod: 0.0,
            max_lod: 0.0,
            border: 0,
            unnorm: 0,
        };
        type FnCreateSampler =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> i32;
        let create: FnCreateSampler = unsafe {
            std::mem::transmute((self.get_device)(
                self.device,
                b"vkCreateSampler\0".as_ptr() as *const c_char,
            ))
        };
        let mut sampler = std::ptr::null_mut();
        unsafe {
            check(
                create(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut sampler,
                ),
                "sampler",
            )?;
        }
        Ok(sampler)
    }

    fn storage_layout(&self, stages: u32) -> Result<Handle, String> {
        self.binding_layout(&[(0, 7, stages)])
    }

    fn two_storage_layout(&self, stages: u32) -> Result<Handle, String> {
        self.binding_layout(&[(0, 7, stages), (1, 7, stages)])
    }

    fn two_sampler_layout(&self) -> Result<Handle, String> {
        self.binding_layout(&[(0, 1, 0x10), (1, 1, 0x10)])
    }

    fn storage_sampler_layout(&self) -> Result<Handle, String> {
        self.binding_layout(&[(0, 7, 0x10), (1, 1, 0x10)])
    }

    fn binding_layout(&self, bindings: &[(u32, u32, u32)]) -> Result<Handle, String> {
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
        let items: Vec<Binding> = bindings
            .iter()
            .map(|(binding, kind, stages)| Binding {
                binding: *binding,
                kind: *kind,
                count: 1,
                stages: *stages,
                samplers: std::ptr::null(),
            })
            .collect();
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: u32::try_from(items.len()).unwrap_or(0),
            bindings: items.as_ptr(),
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "reflection descriptor layout",
            )?;
        }
        Ok(layout)
    }

    fn push_layout(
        &self,
        set: Handle,
        stage: u32,
        offset: u32,
        size: u32,
    ) -> Result<Handle, String> {
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
        let push = Range {
            stage,
            offset,
            size,
        };
        let sets = [set];
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
                "reflection pipeline layout",
            )?;
        }
        Ok(layout)
    }

    fn set_only_layout(&self, set: Handle) -> Result<Handle, String> {
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
        let sets = [set];
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
                "portal pipeline layout",
            )?;
        }
        Ok(layout)
    }

    fn push_only_layout(&self, stage: u32, size: u32) -> Result<Handle, String> {
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
        let push = Range {
            stage,
            offset: 0,
            size,
        };
        let layout_info = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 0,
            sets: std::ptr::null(),
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
                "ggx pipeline layout",
            )?;
        }
        Ok(layout)
    }

    fn desc_pool_one(&self, kind: u32) -> Result<Handle, String> {
        self.desc_pool_kind(kind, 1)
    }

    fn desc_pool_kind(&self, kind: u32, count: u32) -> Result<Handle, String> {
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
        let size = Size { kind, count };
        let pool = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            size_count: 1,
            sizes: &size,
        };
        let mut handle = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut handle,
                ),
                "reflection descriptor pool",
            )?;
        }
        Ok(handle)
    }

    fn desc_pool_mixed(&self) -> Result<Handle, String> {
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
        let sizes = [Size { kind: 7, count: 1 }, Size { kind: 1, count: 1 }];
        let pool = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            size_count: 2,
            sizes: sizes.as_ptr(),
        };
        let mut handle = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut handle,
                ),
                "water descriptor pool",
            )?;
        }
        Ok(handle)
    }

    fn alloc_one(&self, pool: Handle, layout: Handle) -> Result<Handle, String> {
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
        let mut set = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.alloc_desc)(self.device, &alloc as *const Alloc as *const u8, &mut set),
                "reflection descriptor set",
            )?;
        }
        Ok(set)
    }

    fn write_storage(&self, set: Handle, binding: u32, buffer: &Buffer) {
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
            buffer: buffer.buffer,
            offset: 0,
            range: u64::MAX,
        };
        let write = Write {
            s_type: 35,
            next: std::ptr::null(),
            set,
            binding,
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
    }

    fn write_combined(&self, set: Handle, binding: u32, view: Handle) {
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
            buffer: *const c_void,
            texel: *const c_void,
        }
        let info = ImageInfo {
            sampler: self.reflection.sampler,
            view,
            layout: 5,
        };
        let write = Write {
            s_type: 35,
            next: std::ptr::null(),
            set,
            binding,
            element: 0,
            count: 1,
            kind: 1,
            image: &info,
            buffer: std::ptr::null(),
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
    }

    fn make_reflect_pass(
        &mut self,
        color_load: u32,
        color_initial: u32,
        depth_load: u32,
        depth_initial: u32,
        depth_store: u32,
    ) -> Result<Handle, String> {
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
        let color_final = 6;
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
                final_layout: color_final,
            },
            Att {
                flags: 0,
                format: 126,
                samples: 1,
                load: depth_load,
                store: depth_store,
                stencil_load: 1,
                stencil_store: 1,
                initial: depth_initial,
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
                src_stage: 0x80 | 0x1000,
                dst_stage: 0x400,
                src_access: 0x20 | 0x800,
                dst_access: 0x100 | 0x400,
                flags: 0,
            },
            Dep {
                src: 0,
                dst: u32::MAX,
                src_stage: 0x400,
                dst_stage: 0x80 | 0x1000,
                src_access: 0x100 | 0x400,
                dst_access: 0x20 | 0x800,
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
                "reflection pass",
            )?;
        }
        Ok(pass)
    }

    fn read_reflection_grade(&self) -> Result<reflections::ReflectionGrade, String> {
        if !self.reflection.ran || self.reflection.grade.buffer.is_null() {
            return Ok(reflections::ReflectionGrade {
                ran: false,
                rays: 0,
                views: 0,
                gpu_ns: 0,
                vram_bytes: 0,
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
        let mut grade = reflections::reflection_grade_from_bytes(&bytes).unwrap_or(
            reflections::ReflectionGrade {
                ran: false,
                rays: 0,
                views: 0,
                gpu_ns: 0,
                vram_bytes: 0,
            },
        );
        grade.views = self.reflection.views_drawn;
        grade.gpu_ns = self.read_reflection_ns();
        grade.vram_bytes = self.reflection.vram_bytes;
        Ok(grade)
    }

    fn read_reflection_ns(&self) -> u64 {
        if self.reflection.query_pool.is_null() || self.timestamp_period <= 0.0 {
            return 0;
        }
        let mut raw = [0u64; 2];
        let waited = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.reflection.query_pool,
                self.reflection.query_slot,
                2,
                std::mem::size_of::<[u64; 2]>(),
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1 | 0x2,
            )
        };
        if waited != 0 {
            return 0;
        }
        let delta = tick_delta(raw[0], raw[1], self.timestamp_bits);
        let nanos = delta as f64 * f64::from(self.timestamp_period);
        if !nanos.is_finite() || nanos < 0.0 {
            return 0;
        }
        format!("{:.0}", nanos.round()).parse::<u64>().unwrap_or(u64::MAX)
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
        let mut copy = std::mem::replace(&mut self.reflection.depth_copy, Image::empty());
        self.destroy_image(&mut color);
        self.destroy_image(&mut depth);
        self.destroy_image(&mut copy);
        self.reflection.target_w = 0;
        self.reflection.target_h = 0;
        self.reflection.images_live = false;
        self.reflection.depth_copied = false;
    }

    fn destroy_reflection(&mut self) {
        self.destroy_reflection_targets();
        let pipes = [
            self.reflection.water_pipe,
            self.reflection.pipe,
            self.reflection.albedo_pipe,
            self.reflection.portal_pipe,
            self.reflection.composite_pipe,
            self.reflection.ggx_pipe,
        ];
        let layouts = [
            self.reflection.pipe_layout,
            self.reflection.portal_layout,
            self.reflection.composite_layout,
            self.reflection.water_layout,
            self.reflection.ggx_layout,
        ];
        let pools = [
            self.reflection.pool,
            self.reflection.portal_pool,
            self.reflection.composite_pool,
            self.reflection.water_pool,
        ];
        let sets = [
            self.reflection.desc_layout,
            self.reflection.portal_set_layout,
            self.reflection.composite_set_layout,
            self.reflection.water_set_layout,
        ];
        unsafe {
            for pipe in pipes {
                if !pipe.is_null() {
                    (self.fns.destroy_pipeline)(self.device, pipe, std::ptr::null());
                }
            }
            for layout in layouts {
                if !layout.is_null() {
                    (self.fns.destroy_layout)(self.device, layout, std::ptr::null());
                }
            }
            for pool in pools {
                if !pool.is_null() {
                    (self.fns.destroy_desc_pool)(self.device, pool, std::ptr::null());
                }
            }
            for layout in sets {
                if !layout.is_null() {
                    (self.fns.destroy_desc_layout)(self.device, layout, std::ptr::null());
                }
            }
            for pass in [self.reflection.clear_pass, self.reflection.composite_pass] {
                if !pass.is_null() {
                    (self.fns.destroy_render_pass)(self.device, pass, std::ptr::null());
                }
            }
            if !self.reflection.query_pool.is_null() {
                (self.fns.destroy_query_pool)(
                    self.device,
                    self.reflection.query_pool,
                    std::ptr::null(),
                );
            }
            if !self.reflection.sampler.is_null() {
                type FnDestroySampler = unsafe extern "system" fn(Handle, Handle, *const c_void);
                let destroy: FnDestroySampler = std::mem::transmute((self.get_device)(
                    self.device,
                    b"vkDestroySampler\0".as_ptr() as *const c_char,
                ));
                destroy(self.device, self.reflection.sampler, std::ptr::null());
            }
        }
        let mut grade = std::mem::replace(&mut self.reflection.grade, Buffer::empty());
        let mut water = std::mem::replace(&mut self.reflection.water_buf, Buffer::empty());
        let mut link = std::mem::replace(&mut self.reflection.link_buf, Buffer::empty());
        let mut ggx = std::mem::replace(&mut self.reflection.ggx_buf, Buffer::empty());
        self.destroy_buffer(&mut grade);
        self.destroy_buffer(&mut water);
        self.destroy_buffer(&mut link);
        self.destroy_buffer(&mut ggx);
        self.reflection = ReflectionGpu::idle();
    }
}
