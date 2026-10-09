// GI v2 frame path (docs/systems/gi-v2-design.md), included into gpu.rs.
//
// With GI v2 on, the frame rasters a G-buffer (gbuffer.frag, same vertices and
// depth prepass) instead of the old shading, then runs small compute passes, each
// with at most one trace site:
//
//   place    one screen probe per tile on the G-buffer surface
//   trace    the probes' rays through the tracer (analytic shapes + mesh SDFs)
//   light    direct light with shadow rays at every pixel and every probe ray hit
//   gather   ray radiance to irradiance SH per probe
//   compose  direct + probe irradiance per pixel, tone curve, dither
//
// The result goes back into the colour image, so anti-aliasing, overlays, readback
// and present work as before. The old path (scene.frag, light.comp) is untouched and
// is what runs with GI v2 off: `GENOS_GI=v2` at start, or `Renderer::set_gi_v2`.

/// Screen probe tile edge in pixels (Medium and above in the design).
pub(crate) const GI2_TILE: u32 = 16;
/// Rays per screen probe (Low in the design; a square for the stratified pattern).
pub(crate) const GI2_RAYS: u32 = 64;

pub(crate) struct Gi2 {
    pub(crate) on: bool,
    ready: bool,
    desc_layout: Handle,
    layout: Handle,
    pool: Handle,
    sets: [Handle; 2],
    pipes: [Handle; 5],
    gbuffer_pipe: Handle,
    gbuf: Buffer,
    work: Buffer,
    out: Buffer,
    dims: [u32; 4],
}

impl Default for Gi2 {
    fn default() -> Self {
        Self {
            on: std::env::var("GENOS_GI").is_ok_and(|v| v == "v2"),
            ready: false,
            desc_layout: std::ptr::null_mut(),
            layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            sets: [std::ptr::null_mut(); 2],
            pipes: [std::ptr::null_mut(); 5],
            gbuffer_pipe: std::ptr::null_mut(),
            gbuf: Buffer::empty(),
            work: Buffer::empty(),
            out: Buffer::empty(),
            dims: [0; 4],
        }
    }
}

const GI2_PASS_NAMES: [&str; 5] = [
    "gi2 place",
    "gi2 trace",
    "gi2 light",
    "gi2 gather",
    "gi2 compose",
];

impl Gpu {
    /// Note how long a pipeline took to build (tests check it stays short).
    fn note_pipeline_time(&mut self, name: &str, start: Instant) {
        self.pipeline_times
            .push((name.to_string(), start.elapsed()));
    }

    /// Turn GI v2 on or off. The first time on builds its pipelines and buffers.
    pub(crate) fn set_gi2(&mut self, on: bool) -> Result<(), String> {
        self.gi2.on = on;
        if on && !self.gi2.ready {
            unsafe {
                (self.fns.device_wait)(self.device);
            }
            self.make_gi2()?;
            self.make_gi2_targets()?;
        }
        Ok(())
    }

    fn make_gi2(&mut self) -> Result<(), String> {
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
        let bindings: Vec<Binding> = [0u32, 5, 6, 7, 8]
            .iter()
            .map(|&b| Binding {
                binding: b,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            })
            .collect();
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: bindings.len() as u32,
            bindings: bindings.as_ptr(),
        };
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
        #[repr(C)]
        struct Alloc {
            s_type: i32,
            next: *const c_void,
            pool: Handle,
            count: u32,
            layouts: *const Handle,
        }
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.desc_layout,
                ),
                "gi2 descriptor layout",
            )?;
            let sets = [self.gi2.desc_layout];
            let push = Range {
                stage: 0x20,
                offset: 0,
                size: 32,
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
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.layout,
                ),
                "gi2 pipeline layout",
            )?;
            let size = Size { kind: 7, count: 10 };
            let pool = PoolInfo {
                s_type: 33,
                next: std::ptr::null(),
                flags: 0,
                max_sets: 2,
                size_count: 1,
                sizes: &size,
            };
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.pool,
                ),
                "gi2 descriptor pool",
            )?;
            let layouts = [self.gi2.desc_layout, self.gi2.desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.gi2.pool,
                count: 2,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    self.gi2.sets.as_mut_ptr(),
                ),
                "gi2 descriptor sets",
            )?;
        }
        let codes: [&[u8]; 5] = [
            GI2_PLACE_SPV,
            GI2_TRACE_SPV,
            GI2_LIGHT_SPV,
            GI2_GATHER_SPV,
            GI2_COMPOSE_SPV,
        ];
        for (k, code) in codes.iter().enumerate() {
            let start = Instant::now();
            self.gi2.pipes[k] = self.make_compute_pipe(code, self.gi2.layout, GI2_PASS_NAMES[k])?;
            self.note_pipeline_time(GI2_PASS_NAMES[k], start);
        }
        let start = Instant::now();
        self.frag_override = Some(GBUFFER_FRAG_SPV);
        let made = self.make_pipeline(true, false, false);
        self.frag_override = None;
        self.gi2.gbuffer_pipe = made?;
        self.note_pipeline_time("gi2 gbuffer", start);
        self.gi2.ready = true;
        Ok(())
    }

    fn make_compute_pipe(&self, code: &[u8], layout: Handle, what: &str) -> Result<Handle, String> {
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
            layout,
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
            check(result, what)?;
        }
        Ok(pipeline)
    }

    /// Size the G-buffer, work and output buffers for the picture and point every
    /// descriptor at them.
    fn make_gi2_targets(&mut self) -> Result<(), String> {
        if !self.gi2.ready {
            return Ok(());
        }
        let (w, h) = (self.extent_w.max(1), self.extent_h.max(1));
        let cols = w.div_ceil(GI2_TILE);
        let rows = h.div_ceil(GI2_TILE);
        let probes = (cols * rows) as u64;
        let rays = probes * GI2_RAYS as u64;
        let pixels = w as u64 * h as u64;
        let work_vec4 = 2 * probes + 4 * rays + pixels + 7 * probes;
        let mut gbuf = std::mem::replace(&mut self.gi2.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.gi2.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.gi2.out, Buffer::empty());
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.gi2.gbuf = self.make_buffer(16 + pixels * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.work = self.make_buffer(work_vec4 * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.out = self.make_buffer(pixels * 4, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.dims = [w, h, cols, rows];
        self.write_light_set(0)?;
        self.write_light_set(1)?;
        self.write_gi2_sets()
    }

    fn write_gi2_sets(&self) -> Result<(), String> {
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
            let buffers = [
                (0u32, self.frame_scene[slot].buffer),
                (5, self.mesh_field.buffer),
                (6, self.gi2.gbuf.buffer),
                (7, self.gi2.work.buffer),
                (8, self.gi2.out.buffer),
            ];
            let infos: Vec<BufInfo> = buffers
                .iter()
                .map(|(_, b)| BufInfo {
                    buffer: *b,
                    offset: 0,
                    range: u64::MAX,
                })
                .collect();
            let writes: Vec<Write> = buffers
                .iter()
                .zip(&infos)
                .map(|((binding, _), info)| Write {
                    s_type: 35,
                    next: std::ptr::null(),
                    set: self.gi2.sets[slot],
                    binding: *binding,
                    element: 0,
                    count: 1,
                    kind: 7,
                    image: std::ptr::null(),
                    buffer: info,
                    texel: std::ptr::null(),
                })
                .collect();
            unsafe {
                (self.fns.update_desc)(
                    self.device,
                    writes.len() as u32,
                    writes.as_ptr() as *const u8,
                    0,
                    std::ptr::null(),
                );
            }
        }
        Ok(())
    }

    /// GI v2 runs this frame: on, built, sized for the picture, no supersampling.
    fn gi2_active(&self) -> bool {
        self.gi2.on
            && self.gi2.ready
            && self.antialias != Antialias::Ssaa
            && self.gi2.dims[0] == self.extent_w
            && self.gi2.dims[1] == self.extent_h
    }

    /// Before the raster: the G-buffer size header, and the last frame's passes done
    /// with the buffers this frame's raster writes.
    fn gi2_begin(&self) {
        let header = [self.gi2.dims[0], self.gi2.dims[1], 0, 0];
        unsafe {
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.gi2.gbuf.buffer,
                0,
                16,
                header.as_ptr() as *const c_void,
            );
        }
        // Compute reads/writes and the header update before fragment writes.
        self.memory_barrier(
            0x800 | 0x1000,
            0x80 | 0x800,
            0x20 | 0x40 | 0x1000,
            0x20 | 0x40,
        );
    }

    /// After the raster: the passes, then the result into the colour image.
    fn gi2_passes(&mut self, slot: usize) {
        let [w, h, cols, rows] = self.gi2.dims;
        let probes = cols * rows;
        let rays = probes * GI2_RAYS;
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        let pc = [w, h, cols, rows, GI2_RAYS, GI2_TILE, bgra, 0];
        let image = self.color.image;
        // The raster colour as the base: pixels with no surface keep it.
        self.copy_image_buffer(image, 6, self.gi2.out.buffer, w, h, true);
        self.memory_barrier(0x80 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let set = self.gi2.sets[slot];
        let groups = [
            (0usize, probes.div_ceil(64), 1u32),
            (1, rays.div_ceil(64), 1),
            (2, (w * h + rays).div_ceil(64), 1),
            (3, probes.div_ceil(64), 1),
            (4, w.div_ceil(8), h.div_ceil(8)),
        ];
        unsafe {
            (self.fns.cmd_bind_set)(
                self.cmd,
                1,
                self.gi2.layout,
                0,
                1,
                &set,
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_push)(
                self.cmd,
                self.gi2.layout,
                0x20,
                0,
                32,
                pc.as_ptr() as *const c_void,
            );
            for (pass, gx, gy) in groups {
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.gi2.pipes[pass]);
                (self.fns.cmd_dispatch)(self.cmd, gx.max(1), gy.max(1), 1);
                self.memory_barrier(0x800, 0x800 | 0x1000, 0x40, 0x20 | 0x40 | 0x800);
            }
        }
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        self.copy_image_buffer(image, 7, self.gi2.out.buffer, w, h, false);
        self.image_barrier(image, 7, 6, 0x1000, 0x1000 | 0x800, 0x1000, 0x800 | 0x20);
    }

    fn memory_barrier(&self, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
        #[repr(C)]
        struct Barrier {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
        }
        let barrier = Barrier {
            s_type: 46,
            next: std::ptr::null(),
            src_access,
            dst_access,
        };
        unsafe {
            (self.fns.cmd_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                1,
                &barrier as *const Barrier as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
        }
    }

    fn destroy_gi2(&mut self) {
        let mut gbuf = std::mem::replace(&mut self.gi2.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.gi2.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.gi2.out, Buffer::empty());
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        unsafe {
            for pipe in self
                .gi2
                .pipes
                .iter_mut()
                .chain(std::iter::once(&mut self.gi2.gbuffer_pipe))
            {
                if !pipe.is_null() {
                    (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null());
                    *pipe = std::ptr::null_mut();
                }
            }
            if !self.gi2.layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.gi2.layout, std::ptr::null());
                self.gi2.layout = std::ptr::null_mut();
            }
            if !self.gi2.pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.gi2.pool, std::ptr::null());
                self.gi2.pool = std::ptr::null_mut();
            }
            if !self.gi2.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.gi2.desc_layout, std::ptr::null());
                self.gi2.desc_layout = std::ptr::null_mut();
            }
        }
        self.gi2.ready = false;
    }
}
