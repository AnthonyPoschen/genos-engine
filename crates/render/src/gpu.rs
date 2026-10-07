//! Vulkan loader and one forward pass. Entry points come from libvulkan.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::time::{Duration, Instant};

use genos_scene::{view_proj, Camera};

use crate::aa::Antialias;
use crate::budget::FrameMemory;
use crate::pack::{self, GpuVertex, Pack};
use crate::world::World;

include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

const API_VERSION: u32 = 1 << 22;
const VK_SUCCESS: i32 = 0;
const SUBOPTIMAL: i32 = 1000001003;
const OUT_OF_DATE: i32 = -1000001004;
/// Four bounces, and three cascades inside each bounce. Farther cascades run first.
const LIGHT_SLICES: u32 = crate::field::BOUNCES * 3;

type VkResult = i32;
type Handle = *mut c_void;
type Pfn = *const c_void;

pub struct Renderer {
    gpu: Gpu,
    width: u32,
    height: u32,
    /// Player and lamps the resident field was built for.
    field_anchor: Option<pack::FieldAnchor>,
    /// Grid the screen field was built for.
    screen_key: Option<crate::field::ScreenKey>,
    wire_on: bool,
    memory: FrameMemory,
    shapes: crate::pool::MeshPool,
    /// Last overlay list. The same list reuses its vertices.
    overlay_rects: Vec<ScreenRect>,
    overlay_verts: Vec<crate::pack::GpuVertex>,
    cascade_lines: Vec<crate::pack::GpuVertex>,
    /// Pinned hits for the two finest screen cascades. Cascade 2 is cast in the shader.
    screen_pins: crate::pins::ScreenPins,
    /// Floor and occluders the pin set was cast against. A geometry change starts a new cast.
    pin_geometry: Option<u64>,
}

/// Floor and occluders. A lamp change does not start a new pin cast.
fn pin_geometry(scene: &genos_scene::Scene) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    let floor = &scene.floor;
    for value in [
        floor.position.x,
        floor.position.z,
        floor.half_x,
        floor.half_z,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    scene.walls.len().hash(&mut hasher);
    for wall in &scene.walls {
        for value in [
            wall.position.x,
            wall.position.z,
            wall.half_x,
            wall.half_z,
            wall.height,
        ] {
            value.to_bits().hash(&mut hasher);
        }
    }
    scene.solids.len().hash(&mut hasher);
    for solid in &scene.solids {
        let shape = match solid.shape {
            genos_scene::Shape::Square => 0u8,
            genos_scene::Shape::Circle => 1u8,
        };
        shape.hash(&mut hasher);
        // Position is not part of the key. A moving solid keeps the floor and
        // wall cells, and its own lattice follows it.
        for value in [solid.size, solid.height] {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

impl Renderer {
    pub fn open(
        display: *mut c_void,
        surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let gpu = Gpu::open(display, surface, width, height)?;
        let width = gpu.extent_w;
        let height = gpu.extent_h;
        Ok(Self {
            gpu,
            width,
            height,
            field_anchor: None,
            screen_key: None,
            wire_on: false,
            memory: FrameMemory::default(),
            shapes: crate::pool::MeshPool::default(),
            overlay_rects: Vec::new(),
            overlay_verts: Vec::new(),
            cascade_lines: Vec::new(),
            screen_pins: crate::pins::ScreenPins::default(),
            pin_geometry: None,
        })
    }

    /// Draw the selected cascade layers as unlit rays.
    /// Index 0 is the near screen cascade, 1 is the far screen cascade, and 2 is the 3D world volume.
    pub fn set_cascade_view(
        &mut self,
        scene: &genos_scene::Scene,
        camera: &genos_scene::Camera,
        show: [bool; 3],
    ) {
        if !show.iter().any(|on| *on) {
            self.cascade_lines.clear();
            return;
        }
        let geometry = pin_geometry(scene);
        if self.pin_geometry != Some(geometry) {
            self.screen_pins = crate::pins::ScreenPins::default();
            self.pin_geometry = Some(geometry);
        }
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let view = crate::pins::PinView::from_camera(camera, aspect, self.width, self.height);
        crate::pins::update_screen_pins(&mut self.screen_pins, scene, &view);
        let mut lines = Vec::new();
        if show[0] {
            lines.extend(crate::pins::pinned_debug_lines(
                &self.screen_pins.layers[0],
                &view,
                0,
                [0.25, 0.95, 1.0],
            ));
        }
        if show[1] {
            lines.extend(crate::pins::pinned_debug_lines(
                &self.screen_pins.layers[1],
                &view,
                1,
                [1.0, 0.82, 0.2],
            ));
        }
        if show[2] {
            lines.extend(crate::field::cascade_debug_lines(
                scene,
                None,
                [false, false, true],
            ));
        }
        self.cascade_lines = lines;
    }

    /// True when the last submit left its fence unsignaled. The caller can continue.
    pub fn submit_was_pending(&self) -> bool {
        self.gpu.submit_pending
    }

    /// Pixels from the frame before the latest submit. That frame keeps its own lamp.
    pub fn read_earlier_frame(&mut self) -> Result<Vec<u8>, String> {
        let slot = self.gpu.flight;
        self.gpu.read_host(slot)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if !extent_needed(self.width, self.height, width, height) {
            return Ok(());
        }
        self.gpu.recreate(width, height)?;
        self.width = self.gpu.extent_w;
        self.height = self.gpu.extent_h;
        Ok(())
    }

    /// Draw one frame. Cascades are rebuilt here, then only the visible objects are rasterized.
    ///
    /// When `readback` is set, return tightly packed BGRA8 pixels.
    pub fn draw(
        &mut self,
        world: &World,
        camera: &Camera,
        readback: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        self.draw_with_overlay(world, camera, &[], readback, false)
    }

    /// Pixels of the last presented picture.
    ///
    /// Waits for that draw's graphics fence. Does not wait for a light gather.
    /// A `readback` draw already waited, and its pixels are the finished field.
    pub fn read_picture(&mut self) -> Result<Vec<u8>, String> {
        self.gpu.wait_gpu()?;
        self.gpu.read_color()
    }

    /// Draw the world, then screen rectangles. The rectangles ignore the depth test.
    ///
    /// When `wireframe` is true, each floor, wall, and solid draws its vertices and its edges.
    /// The filled shaded triangles stay in use when `wireframe` is false.
    pub fn draw_with_overlay(
        &mut self,
        world: &World,
        camera: &Camera,
        overlay: &[ScreenRect],
        readback: bool,
        wireframe: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let matrix = view_proj(camera, aspect);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        self.memory.begin_frame();
        self.shapes.begin_frame();
        let pack = pack::pack_frame(world, &matrix, eye, &mut self.memory);
        for key in self.memory.take_dropped() {
            self.shapes.release(key);
        }
        if self.overlay_rects.as_slice() != overlay {
            self.overlay_verts = screen_quads(overlay);
            self.overlay_rects.clear();
            self.overlay_rects.extend_from_slice(overlay);
        }
        let overlay_verts = self.overlay_verts.clone();
        self.wire_on = wireframe;
        let overlay_count = overlay_verts.len() as u32;
        // Overlay rectangles are not in `pack`. A short player or lamp move keeps the field.
        // An unfinished cascade is not a reason to start again.
        let rewrite_light = self
            .field_anchor
            .as_ref()
            .map(|anchor| !anchor.holds(&pack))
            .unwrap_or(true);
        // The dynamic patch belongs to the frame that is about to record.
        let slot = self.gpu.flight;
        self.gpu.wait_flight(slot)?;
        self.gpu.bind_flight();
        let mut dynamic = Vec::new();
        let mut draws = Vec::new();
        let mut instances = vec![identity_instance()];
        for item in &pack.draws {
            match item {
                pack::PackedDraw::Shape {
                    key,
                    model,
                    color,
                    source,
                } => {
                    if !self.memory.contains_shape(*key) {
                        continue;
                    }
                    let block = if self.shapes.contains(*key) {
                        self.shapes.use_mesh(*key, &[])
                    } else {
                        let local = pack::build_shape(source);
                        self.shapes.use_mesh(*key, &local)
                    };
                    let Some(block) = block else {
                        continue;
                    };
                    let instance = instances.len() as u32;
                    instances.push(InstanceRec {
                        model: *model,
                        color: [color[0], color[1], color[2], 1.0],
                    });
                    draws.push(DrawSpan {
                        shapes: true,
                        first: block.offset,
                        count: block.count,
                        instance,
                    });
                }
                pack::PackedDraw::Dynamic(verts) => {
                    if verts.is_empty() {
                        continue;
                    }
                    let first = dynamic.len() as u32;
                    dynamic.extend_from_slice(verts);
                    draws.push(DrawSpan {
                        shapes: false,
                        first,
                        count: verts.len() as u32,
                        instance: 0,
                    });
                }
            }
        }
        self.shapes.finish_frame();
        for key in self.memory.finish_frame() {
            self.shapes.release(key);
        }
        self.gpu.upload_shapes(&self.shapes)?;
        if wireframe {
            let corners = pack::shape_corners(world, &matrix);
            let wire = crate::wire::mark_vertices(&crate::wire::marks_from_triangles(&corners));
            let count = wire.len() as u32;
            dynamic = wire;
            draws = if count == 0 {
                Vec::new()
            } else {
                vec![DrawSpan {
                    shapes: false,
                    first: 0,
                    count,
                    instance: 0,
                }]
            };
            instances = vec![identity_instance()];
        }
        if !self.cascade_lines.is_empty() {
            let first = dynamic.len() as u32;
            let count = self.cascade_lines.len() as u32;
            dynamic.extend_from_slice(&self.cascade_lines);
            draws.push(DrawSpan {
                shapes: false,
                first,
                count,
                instance: 0,
            });
        }
        let world_dynamic = dynamic.len() as u32;
        let mut dynamic = dynamic;
        dynamic.extend(overlay_verts);
        if !dynamic.is_empty() {
            self.gpu.upload(&dynamic)?;
        }
        self.gpu.write_instances(&instances)?;
        if self.memory.image_uploaded() {
            // The particle image is one shared buffer. Both frames can sample it.
            let (width, height) = self.memory.image_size();
            let bytes = pack::image_bytes(width, height, self.memory.image_pixels());
            self.gpu.wait_flight(1 - slot)?;
            self.gpu.upload_image(&bytes)?;
            self.memory.clear_image_upload();
        }
        let _ = (rewrite_light, self.screen_key);
        let mut pack = pack;
        pack::apply_view(&mut pack, camera, aspect, self.width, self.height);
        self.stage_pins(world, &mut pack);
        self.gpu.upload_scene(&pack)?;
        self.field_anchor = Some(pack::FieldAnchor::capture(&pack));
        self.gpu.poll_light()?;
        let wait_light = readback || !self.gpu.light_ready;
        // One cascade row when the build is already going. A readback finishes every row.
        self.gpu.kick_light(wait_light)?;
        self.gpu.note_vertex_count(world_dynamic);
        self.gpu.set_draws(draws);
        self.gpu.note_overlay_count(overlay_count);
        self.gpu.wire_on = wireframe;
        self.gpu.record_and_submit(&matrix)?;
        let pixels = if readback {
            self.gpu.wait_gpu()?;
            Some(self.gpu.read_color()?)
        } else {
            None
        };
        let result = self.gpu.present()?;
        if result == OUT_OF_DATE || result == SUBOPTIMAL {
            self.gpu.recreate(self.width, self.height)?;
            self.width = self.gpu.extent_w;
            self.height = self.gpu.extent_h;
        }
        Ok(pixels)
    }

    /// Draw one frame and record a Vulkan timestamp span around that GPU work.
    ///
    /// `cpu` is the host time of this call. `gpu` is the timestamp span when the
    /// fence has already signaled, for example after a readback wait. The fence
    /// wait is not the GPU time. [`Renderer::draw`] does not record timestamps.
    pub fn draw_profiled(
        &mut self,
        world: &World,
        camera: &Camera,
        overlay: &[ScreenRect],
        readback: bool,
        wireframe: bool,
    ) -> Result<(Option<Vec<u8>>, DrawProfile), String> {
        if self.gpu.timestamp_period <= 0.0 || self.gpu.timestamp_bits == 0 {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        let start = Instant::now();
        self.gpu.profile_submit = true;
        let drawn = self.draw_with_overlay(world, camera, overlay, readback, wireframe);
        let cpu = start.elapsed();
        self.gpu.profile_submit = false;
        let pixels = drawn?;
        let gpu = self.gpu.collect_submitted_if_ready()?;
        Ok((pixels, DrawProfile { cpu, gpu }))
    }

    /// GPU times whose fences have signaled, in submit order. This does not wait.
    pub fn poll_gpu_times(&mut self) -> Result<Vec<Duration>, String> {
        self.gpu.collect_ready(false)?;
        Ok(self.gpu.ready.drain(..).collect())
    }

    /// Wait for outstanding timestamp queries and return those GPU times.
    ///
    /// The wait is not added to the returned durations. Call this before the
    /// renderer is dropped so a finished run can store the device times.
    pub fn finish_gpu_times(&mut self) -> Result<Vec<Duration>, String> {
        self.gpu.collect_ready(true)?;
        Ok(self.gpu.ready.drain(..).collect())
    }

    /// Write the world into the resident scene buffer. This does not present.
    pub fn retain_scene(&mut self, world: &World) -> Result<(), String> {
        let camera = Camera::opening();
        let matrix = view_proj(&camera, 1.0);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        self.memory.begin_frame();
        let mut pack = pack::pack_frame(world, &matrix, eye, &mut self.memory);
        let aspect = self.width as f32 / self.height.max(1) as f32;
        pack::apply_view(&mut pack, &camera, aspect, self.width, self.height);
        self.stage_pins(world, &mut pack);
        self.gpu.wait_all_inflight()?;
        self.gpu.upload_scene(&pack)?;
        self.field_anchor = Some(pack::FieldAnchor::capture(&pack));
        Ok(())
    }

    /// Keep the two finest cascades on their world cells, then hand those hits to the gather.
    fn stage_pins(&mut self, world: &World, pack: &mut pack::Pack) {
        let geometry = pin_geometry(&world.scene);
        if self.pin_geometry != Some(geometry) {
            self.screen_pins = crate::pins::ScreenPins::default();
            self.pin_geometry = Some(geometry);
        }
        let view = crate::pins::PinView::from_pack(pack);
        crate::pins::update_screen_pins(&mut self.screen_pins, &world.scene, &view);
        pack.pin_count0 = self.screen_pins.layers[0].len() as u32;
        pack.pin_count1 = self.screen_pins.layers[1].len() as u32;
        let texels = crate::pins::pin_texels(&self.screen_pins, &view);
        let mut bytes = Vec::with_capacity(texels.len() * 16);
        for texel in texels {
            for value in texel {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        self.gpu.pin_bytes = bytes;
    }

    /// Ground position the gather is tracking.
    pub fn field_player(&self) -> [f32; 2] {
        self.field_anchor
            .as_ref()
            .map(pack::FieldAnchor::player)
            .unwrap_or([0.0, 0.0])
    }

    /// One compute pass over the resident occluders. This does not present and does not upload them.
    pub fn transmission_gains(
        &mut self,
        listener: [f32; 3],
        sources: &[[f32; 3]],
    ) -> Result<Vec<f32>, String> {
        self.gpu.transmission_gains(listener, sources)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Picture filter for the next draw. The renderer starts at `Off`.
    pub fn set_antialias(&mut self, mode: Antialias) {
        self.gpu.antialias = mode;
    }

    pub fn antialias(&self) -> Antialias {
        self.gpu.antialias
    }
}

/// Host time and device time for one profiled draw.
///
/// `gpu` is `None` until that draw's fence has signaled. It is never a copy of `cpu`.
#[derive(Clone, Copy, Debug)]
pub struct DrawProfile {
    pub cpu: Duration,
    pub gpu: Option<Duration>,
}

/// One axis-aligned rectangle in window pixels. The origin is the top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 3],
}

struct DrawSpan {
    shapes: bool,
    first: u32,
    count: u32,
    instance: u32,
}

struct InstanceRec {
    model: [f32; 16],
    color: [f32; 4],
}

fn identity_instance() -> InstanceRec {
    InstanceRec {
        model: crate::world::identity_pose(),
        color: [1.0, 1.0, 1.0, 1.0],
    }
}

fn screen_quads(rects: &[ScreenRect]) -> Vec<GpuVertex> {
    let mut verts = Vec::with_capacity(rects.len() * 6);
    for rect in rects {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;
        let color = rect.color;
        let corner = |x: f32, y: f32| pack::overlay_vertex([x, y, 0.0], color);
        verts.push(corner(x0, y0));
        verts.push(corner(x1, y0));
        verts.push(corner(x1, y1));
        verts.push(corner(x0, y0));
        verts.push(corner(x1, y1));
        verts.push(corner(x0, y1));
    }
    verts
}

/// Column-major map from pixel space to Vulkan NDC. (0, 0) is the top-left.
fn pixel_matrix(width: f32, height: f32) -> [f32; 16] {
    let mut matrix = [0.0; 16];
    matrix[0] = 2.0 / width.max(1.0);
    matrix[5] = 2.0 / height.max(1.0);
    matrix[10] = 1.0;
    matrix[12] = -1.0;
    matrix[13] = -1.0;
    matrix[15] = 1.0;
    matrix
}

pub(crate) fn extent_needed(old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> bool {
    new_w > 0 && new_h > 0 && (new_w != old_w || new_h != old_h)
}

#[allow(dead_code)]
struct Gpu {
    get_instance: GetProc,
    get_device: GetProc,
    instance: Handle,
    physical: Handle,
    device: Handle,
    queue: Handle,
    queue_family: u32,
    /// Gather queue. A graphics submit does not wait for this queue.
    compute_queue: Handle,
    compute_family: u32,
    surface: Handle,
    swapchain: Handle,
    format: i32,
    extent_w: u32,
    extent_h: u32,
    swap_images: Vec<Handle>,
    color: Image,
    depth: Image,
    framebuffer: Handle,
    host: Buffer,
    /// Alias of `vertices[flight]`. Drop destroys `vertices`, not this copy.
    vertex: Buffer,
    vertices: [Buffer; 2],
    /// Shared shape pool. Offsets stay put for the life of a block.
    shapes: Buffer,
    /// How many shape vertices have been read by a submitted frame.
    shapes_submitted: u32,
    instances: Buffer,
    instance_cache: Vec<u8>,
    draws: Vec<DrawSpan>,
    vertex_count: u32,
    overlay_count: u32,
    render_pass: Handle,
    pipeline: Handle,
    overlay_pipeline: Handle,
    wire_pipeline: Handle,
    wire_on: bool,
    layout: Handle,
    pool: Handle,
    cmd: Handle,
    image_ready: Handle,
    render_done: Handle,
    fence: Handle,
    present_index: u32,
    cmds: [Handle; 2],
    fences: [Handle; 2],
    image_readies: [Handle; 2],
    render_dones: [Handle; 2],
    colors: [Image; 2],
    depths: [Image; 2],
    framebuffers: [Handle; 2],
    hosts: [Buffer; 2],
    flight: usize,
    /// Latest occluders for audio. The raster reads `light_scene[light_shown]`.
    scene_buf: Buffer,
    particle_buf: Buffer,
    desc_layout: Handle,
    desc_pool: Handle,
    /// Raster set. This is `light_sets[light_shown]`.
    desc_set: Handle,
    light_scene: [Buffer; 2],
    light_field: [Buffer; 2],
    light_sets: [Handle; 2],
    /// Shown field index. The gather writes the other one.
    light_shown: usize,
    light_dst: usize,
    /// Field index each flight bound. `usize::MAX` means that flight has not drawn.
    flight_light: [usize; 2],
    light_cmd: Handle,
    light_fence: Handle,
    light_pool: Handle,
    light_busy: bool,
    /// True while a build still has slices to run or in flight.
    light_building: bool,
    /// Next gather slice. `LIGHT_SLICES` means the build has no slice left.
    light_pass: u32,
    /// Next workgroup row inside `light_pass`.
    light_row: u32,
    /// Workgroup columns and row-bands for the near, far, and world cascades.
    light_cols: [u32; 3],
    light_rows: [u32; 3],
    /// Coarse world-probe grid. One band advances per frame while the cache is dirty.
    world_cols: u32,
    world_bands: u32,
    world_row: u32,
    /// Bands still to write after a lamp or occluder change. Zero means the cache is current.
    world_pending: u32,
    /// A world-only band must not replace the screen field.
    swap_on_done: bool,
    /// Screen-probe grid from the last uploaded view.
    screen_w: u32,
    screen_h: u32,
    /// Live pins for cascades 0 and 1. Cascade 2 stays on the screen grid.
    pin_counts: [u32; 2],
    /// Pin positions, normals, and cell index. Written into the field before the gather.
    pin_bytes: Vec<u8>,
    floor_half_x: f32,
    floor_half_z: f32,
    /// Chains gather slices. The last slice leaves it signaled for one picture.
    light_sem: Handle,
    light_sem_hot: bool,
    light_wait_graphics: bool,
    /// Picture flight that waits `light_sem`, until that flight's fence signals.
    /// A new slice must not signal the semaphore while this is set.
    light_publish_flight: Option<usize>,
    light_publish_fence: Handle,
    pending_light: Option<Vec<u8>>,
    compute_layout: Handle,
    compute_pipe: Handle,
    audio_rays: Buffer,
    audio_gains: Buffer,
    audio_desc_layout: Handle,
    audio_desc_pool: Handle,
    audio_set: Handle,
    audio_layout: Handle,
    audio_pipe: Handle,
    audio_cmd: Handle,
    audio_fence: Handle,
    light_ready: bool,
    submit_pending: bool,
    timestamp_period: f32,
    timestamp_bits: u32,
    query_pool: Handle,
    profile_submit: bool,
    stamp_pending: [bool; 2],
    inflight: VecDeque<usize>,
    ready: VecDeque<Duration>,
    submitted_slot: Option<usize>,
    antialias: Antialias,
    /// Last known layout of each flight's color image. `0` is undefined. `6` is transfer source.
    color_layout: [u32; 2],
    keep_pass: Handle,
    aa_src: [Buffer; 2],
    aa_dst: [Buffer; 2],
    ssaa_color: [Image; 2],
    ssaa_depth: [Image; 2],
    ssaa_fb: [Handle; 2],
    aa_desc_layout: Handle,
    aa_layout: Handle,
    aa_fxaa: Handle,
    aa_ssaa: Handle,
    aa_pool: Handle,
    aa_sets: [Handle; 2],
    fns: Fns,
    memory_props: MemProps,
}

struct Image {
    image: Handle,
    view: Handle,
    memory: Handle,
}

struct Buffer {
    buffer: Handle,
    memory: Handle,
    size: u64,
}

#[derive(Clone, Copy)]
struct MemProps {
    count: u32,
    types: [u32; 32],
}

#[allow(dead_code)]
struct Fns {
    destroy_instance: FnDestroy,
    destroy_device: FnDestroy,
    create_wayland_surface: FnWayland,
    destroy_surface: Fn2,
    surface_support: FnSupport,
    surface_caps: FnCaps,
    surface_formats: FnCount,
    surface_modes: FnCount,
    create_swapchain: FnSwapchain,
    destroy_swapchain: Fn2,
    swapchain_images: FnCount,
    acquire: FnAcquire,
    present: FnPresent,
    create_image_view: FnCreateView,
    destroy_image_view: Fn2,
    create_shader: FnShader,
    destroy_shader: Fn2,
    create_layout: FnLayout,
    destroy_layout: Fn2,
    create_render_pass: FnRenderPass,
    destroy_render_pass: Fn2,
    create_pipelines: FnPipelines,
    destroy_pipeline: Fn2,
    create_framebuffer: FnFramebuffer,
    destroy_framebuffer: Fn2,
    create_pool: FnPool,
    destroy_pool: Fn2,
    alloc_cmd: FnAllocCmd,
    begin_cmd: FnBegin,
    end_cmd: FnCmdResult,
    reset_cmd: FnReset,
    cmd_begin_rp: FnBeginRp,
    cmd_end_rp: FnCmd,
    cmd_bind_pipe: FnBindPipe,
    cmd_bind_vb: FnBindVb,
    cmd_draw: FnDraw,
    cmd_push: FnPush,
    cmd_viewport: FnViewport,
    cmd_scissor: FnScissor,
    cmd_barrier: FnBarrier,
    cmd_copy_image: FnCopyImage,
    cmd_copy_to_buffer: FnCopyBuf,
    cmd_copy_to_image: FnCopyToImage,
    create_sem: FnCreateSem,
    destroy_sem: Fn2,
    create_fence: FnFence,
    destroy_fence: Fn2,
    wait_fences: FnWait,
    reset_fences: FnResetFences,
    queue_submit: FnSubmit,
    fence_status: FnFenceStatus,
    cmd_bind_set: FnBindSet,
    cmd_dispatch: FnDispatch,
    update_desc: FnUpdateDesc,
    create_desc_layout: FnDescLayout,
    destroy_desc_layout: Fn2,
    create_desc_pool: FnDescPool,
    destroy_desc_pool: Fn2,
    alloc_desc: FnAllocDesc,
    create_compute: FnPipelines,
    device_wait: FnDevice,
    create_buffer: FnBuffer,
    destroy_buffer: Fn2,
    buffer_reqs: FnBufReq,
    alloc_mem: FnAlloc,
    free_mem: Fn2,
    bind_buffer: FnBindBuf,
    map_mem: FnMap,
    unmap_mem: FnUnmap,
    create_image: FnImage,
    destroy_image: Fn2,
    image_reqs: FnImgReq,
    bind_image: FnBindImg,
    enumerate_devices: FnEnumDev,
    queue_families: FnQueues,
    mem_props: FnMemProps,
    create_query_pool: FnQueryPool,
    destroy_query_pool: Fn2,
    cmd_reset_query: FnResetQuery,
    cmd_write_timestamp: FnWriteTs,
    get_query_results: FnGetQuery,
}

type GetProc = unsafe extern "system" fn(Handle, *const c_char) -> Pfn;
type FnDestroy = unsafe extern "system" fn(Handle, *const c_void);
type Fn2 = unsafe extern "system" fn(Handle, Handle, *const c_void);
type FnDevice = unsafe extern "system" fn(Handle) -> VkResult;
type FnQueryPool =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnResetQuery = unsafe extern "system" fn(Handle, Handle, u32, u32);
type FnWriteTs = unsafe extern "system" fn(Handle, u32, Handle, u32);
type FnGetQuery =
    unsafe extern "system" fn(Handle, Handle, u32, u32, usize, *mut c_void, u64, u32) -> VkResult;
type FnCmd = unsafe extern "system" fn(Handle);
type FnCmdResult = unsafe extern "system" fn(Handle) -> VkResult;
type FnCreateDevice =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnGetQueue = unsafe extern "system" fn(Handle, u32, u32, *mut Handle);
type FnWayland =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnSupport = unsafe extern "system" fn(Handle, u32, Handle, *mut u32) -> VkResult;
type FnCaps = unsafe extern "system" fn(Handle, Handle, *mut u8) -> VkResult;
type FnCount = unsafe extern "system" fn(Handle, Handle, *mut u32, *mut u8) -> VkResult;
type FnSwapchain =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAcquire =
    unsafe extern "system" fn(Handle, Handle, u64, Handle, Handle, *mut u32) -> VkResult;
type FnPresent = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
type FnCreateView =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnShader =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnLayout =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnRenderPass =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnPipelines = unsafe extern "system" fn(
    Handle,
    Handle,
    u32,
    *const u8,
    *const c_void,
    *mut Handle,
) -> VkResult;
type FnFramebuffer =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnPool = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAllocCmd = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
type FnBegin = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
type FnReset = unsafe extern "system" fn(Handle, u32) -> VkResult;
type FnBeginRp = unsafe extern "system" fn(Handle, *const u8, u32);
type FnBindPipe = unsafe extern "system" fn(Handle, u32, Handle);
type FnBindVb = unsafe extern "system" fn(Handle, u32, u32, *const Handle, *const u64);
type FnDraw = unsafe extern "system" fn(Handle, u32, u32, u32, u32);
type FnPush = unsafe extern "system" fn(Handle, Handle, u32, u32, u32, *const c_void);
type FnViewport = unsafe extern "system" fn(Handle, u32, u32, *const f32);
type FnScissor = unsafe extern "system" fn(Handle, u32, u32, *const i32);
type FnBarrier = unsafe extern "system" fn(
    Handle,
    u32,
    u32,
    u32,
    u32,
    *const c_void,
    u32,
    *const c_void,
    u32,
    *const u8,
);
type FnCopyImage = unsafe extern "system" fn(Handle, Handle, i32, Handle, i32, u32, *const u8);
type FnCopyBuf = unsafe extern "system" fn(Handle, Handle, i32, Handle, u32, *const u8);
type FnCopyToImage = unsafe extern "system" fn(Handle, Handle, Handle, i32, u32, *const u8);
type FnCreateSem =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnFence = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnWait = unsafe extern "system" fn(Handle, u32, *const Handle, u32, u64) -> VkResult;
type FnResetFences = unsafe extern "system" fn(Handle, u32, *const Handle) -> VkResult;
type FnSubmit = unsafe extern "system" fn(Handle, u32, *const u8, Handle) -> VkResult;
type FnFenceStatus = unsafe extern "system" fn(Handle, Handle) -> VkResult;
type FnBindSet =
    unsafe extern "system" fn(Handle, u32, Handle, u32, u32, *const Handle, u32, *const u32);
type FnDispatch = unsafe extern "system" fn(Handle, u32, u32, u32);
type FnUpdateDesc = unsafe extern "system" fn(Handle, u32, *const u8, u32, *const u8);
type FnDescLayout =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnDescPool =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnAllocDesc = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
type FnBuffer =
    unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnBufReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
type FnAlloc = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnBindBuf = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
type FnMap = unsafe extern "system" fn(Handle, Handle, u64, u64, u32, *mut *mut c_void) -> VkResult;
type FnUnmap = unsafe extern "system" fn(Handle, Handle);
type FnImage = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
type FnImgReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
type FnBindImg = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
type FnEnumDev = unsafe extern "system" fn(Handle, *mut u32, *mut Handle) -> VkResult;
type FnQueues = unsafe extern "system" fn(Handle, *mut u32, *mut u8);
type FnMemProps = unsafe extern "system" fn(Handle, *mut u8);

impl Gpu {
    fn open(
        display: *mut c_void,
        wl_surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        unsafe {
            let lib = dlopen(b"libvulkan.so.1\0".as_ptr() as *const c_char, 2);
            if lib.is_null() {
                return Err("libvulkan.so.1 did not open".into());
            }
            let get_instance: GetProc = transmute(dlsym(
                lib,
                b"vkGetInstanceProcAddr\0".as_ptr() as *const c_char,
            ));
            let enumerate_ext: unsafe extern "system" fn(
                *const c_void,
                *mut u32,
                *mut u8,
            ) -> VkResult = transmute(get_instance(
                std::ptr::null_mut(),
                b"vkEnumerateInstanceExtensionProperties\0".as_ptr() as *const c_char,
            ));
            let create_instance: unsafe extern "system" fn(
                *const u8,
                *const c_void,
                *mut Handle,
            ) -> VkResult = transmute(get_instance(
                std::ptr::null_mut(),
                b"vkCreateInstance\0".as_ptr() as *const c_char,
            ));

            let mut ext_count = 0u32;
            check(
                enumerate_ext(std::ptr::null(), &mut ext_count, std::ptr::null_mut()),
                "extensions",
            )?;
            let surface_ext = b"VK_KHR_surface\0";
            let wayland_ext = b"VK_KHR_wayland_surface\0";
            let exts = [
                surface_ext.as_ptr() as *const c_char,
                wayland_ext.as_ptr() as *const c_char,
            ];
            let app_name = b"genos-camera\0";
            #[repr(C)]
            struct AppInfo {
                s_type: i32,
                next: *const c_void,
                name: *const c_char,
                app_version: u32,
                engine: *const c_char,
                engine_version: u32,
                api: u32,
            }
            let app = AppInfo {
                s_type: 0,
                next: std::ptr::null(),
                name: app_name.as_ptr() as *const c_char,
                app_version: 1,
                engine: app_name.as_ptr() as *const c_char,
                engine_version: 1,
                api: API_VERSION,
            };
            #[repr(C)]
            struct InstInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                app: *const AppInfo,
                enabled_layers: u32,
                layer_names: *const *const c_char,
                enabled_exts: u32,
                ext_names: *const *const c_char,
            }
            let info = InstInfo {
                s_type: 1,
                next: std::ptr::null(),
                flags: 0,
                app: &app,
                enabled_layers: 0,
                layer_names: std::ptr::null(),
                enabled_exts: exts.len() as u32,
                ext_names: exts.as_ptr(),
            };
            let mut instance = std::ptr::null_mut();
            check(
                create_instance(
                    &info as *const InstInfo as *const u8,
                    std::ptr::null(),
                    &mut instance,
                ),
                "vkCreateInstance",
            )?;
            let load = |name: &[u8]| get_instance(instance, name.as_ptr() as *const c_char);

            let enumerate_devices: FnEnumDev = transmute(load(b"vkEnumeratePhysicalDevices\0"));
            let queue_families: FnQueues =
                transmute(load(b"vkGetPhysicalDeviceQueueFamilyProperties\0"));
            let mem_props: FnMemProps = transmute(load(b"vkGetPhysicalDeviceMemoryProperties\0"));
            let create_device: FnCreateDevice = transmute(load(b"vkCreateDevice\0"));
            let get_device: GetProc = transmute(load(b"vkGetDeviceProcAddr\0"));
            let create_wayland_surface: FnWayland = transmute(load(b"vkCreateWaylandSurfaceKHR\0"));
            let destroy_surface: Fn2 = transmute(load(b"vkDestroySurfaceKHR\0"));
            let surface_support: FnSupport =
                transmute(load(b"vkGetPhysicalDeviceSurfaceSupportKHR\0"));
            let surface_caps: FnCaps =
                transmute(load(b"vkGetPhysicalDeviceSurfaceCapabilitiesKHR\0"));
            let surface_formats: FnCount =
                transmute(load(b"vkGetPhysicalDeviceSurfaceFormatsKHR\0"));
            let surface_modes: FnCount =
                transmute(load(b"vkGetPhysicalDeviceSurfacePresentModesKHR\0"));
            let destroy_instance: FnDestroy = transmute(load(b"vkDestroyInstance\0"));

            let mut dev_count = 0u32;
            check(
                enumerate_devices(instance, &mut dev_count, std::ptr::null_mut()),
                "devices",
            )?;
            if dev_count == 0 {
                return Err("no Vulkan physical device".into());
            }
            let mut devices = vec![std::ptr::null_mut(); dev_count as usize];
            check(
                enumerate_devices(instance, &mut dev_count, devices.as_mut_ptr()),
                "device list",
            )?;

            #[repr(C)]
            struct WaylandInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                display: *mut c_void,
                surface: *mut c_void,
            }
            let wayland = WaylandInfo {
                s_type: 1000006000,
                next: std::ptr::null(),
                flags: 0,
                display,
                surface: wl_surface,
            };
            let mut vk_surface = std::ptr::null_mut();
            check(
                create_wayland_surface(
                    instance,
                    &wayland as *const WaylandInfo as *const u8,
                    std::ptr::null(),
                    &mut vk_surface,
                ),
                "wayland surface",
            )?;

            let get_props: unsafe extern "system" fn(Handle, *mut u8) =
                transmute(load(b"vkGetPhysicalDeviceProperties\0"));
            let mut chosen = None;
            for physical in devices {
                let mut family_count = 0u32;
                queue_families(physical, &mut family_count, std::ptr::null_mut());
                let mut families = vec![0u8; family_count as usize * 24];
                queue_families(physical, &mut family_count, families.as_mut_ptr());
                for index in 0..family_count {
                    let base = index as usize * 24;
                    let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                    let timestamp_bits =
                        u32::from_ne_bytes(families[base + 8..base + 12].try_into().unwrap());
                    let mut supported = 0u32;
                    check(
                        surface_support(physical, index, vk_surface, &mut supported),
                        "present support",
                    )?;
                    if flags & 0x3 == 0x3 && supported == 1 {
                        chosen = Some((physical, index, timestamp_bits, families, family_count));
                        break;
                    }
                }
                if chosen.is_some() {
                    break;
                }
            }
            let Some((physical, queue_family, timestamp_bits, families, family_count)) = chosen
            else {
                return Err("no graphics and compute queue can present".into());
            };
            let mut compute_family = queue_family;
            for index in 0..family_count {
                let base = index as usize * 24;
                let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                // Compute without graphics can run the gather beside the picture.
                if flags & 0x1 == 0 && flags & 0x2 != 0 {
                    compute_family = index;
                    break;
                }
            }
            let gfx_queues = u32::from_ne_bytes(
                families[queue_family as usize * 24 + 4..queue_family as usize * 24 + 8]
                    .try_into()
                    .unwrap(),
            );
            let mut prop_bytes = vec![0u8; 1024];
            get_props(physical, prop_bytes.as_mut_ptr());
            let timestamp_flag = u32::from_ne_bytes(prop_bytes[716..720].try_into().unwrap());
            let mut timestamp_period = f32::from_ne_bytes(prop_bytes[720..724].try_into().unwrap());
            if timestamp_flag == 0
                || timestamp_bits == 0
                || !(timestamp_period > 0.0 && timestamp_period < 10_000.0)
            {
                timestamp_period = 0.0;
            }

            let mut raw_props = [0u8; 4 + 32 * 8 + 4 + 16 * 16];
            mem_props(physical, raw_props.as_mut_ptr());
            let memory_props = parse_mem_props(&raw_props);

            let priorities = [1.0f32, 0.0f32];
            #[repr(C)]
            struct QueueInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                family: u32,
                count: u32,
                priorities: *const f32,
            }
            let separate = compute_family != queue_family;
            let mut queue_infos = [
                QueueInfo {
                    s_type: 2,
                    next: std::ptr::null(),
                    flags: 0,
                    family: queue_family,
                    count: 1,
                    priorities: priorities.as_ptr(),
                },
                QueueInfo {
                    s_type: 2,
                    next: std::ptr::null(),
                    flags: 0,
                    family: compute_family,
                    count: 1,
                    priorities: priorities.as_ptr().add(1),
                },
            ];
            let queue_info_count = if separate {
                2
            } else if gfx_queues >= 2 {
                queue_infos[0].count = 2;
                1
            } else {
                1
            };
            let swap_ext = b"VK_KHR_swapchain\0";
            let ext_ptr = swap_ext.as_ptr() as *const c_char;
            #[repr(C)]
            struct DevInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                queue_count: u32,
                queues: *const QueueInfo,
                layer_count: u32,
                layers: *const c_void,
                ext_count: u32,
                exts: *const *const c_char,
                features: *const c_void,
            }
            let dev_info = DevInfo {
                s_type: 3,
                next: std::ptr::null(),
                flags: 0,
                queue_count: queue_info_count,
                queues: queue_infos.as_ptr(),
                layer_count: 0,
                layers: std::ptr::null(),
                ext_count: 1,
                exts: &ext_ptr,
                features: std::ptr::null(),
            };
            let mut device = std::ptr::null_mut();
            check(
                create_device(
                    physical,
                    &dev_info as *const DevInfo as *const u8,
                    std::ptr::null(),
                    &mut device,
                ),
                "vkCreateDevice",
            )?;
            let dload = |name: &[u8]| get_device(device, name.as_ptr() as *const c_char);
            let get_device_queue: FnGetQueue = transmute(dload(b"vkGetDeviceQueue\0"));
            let mut queue = std::ptr::null_mut();
            get_device_queue(device, queue_family, 0, &mut queue);
            let mut compute_queue = queue;
            if separate {
                get_device_queue(device, compute_family, 0, &mut compute_queue);
            } else if gfx_queues >= 2 {
                get_device_queue(device, queue_family, 1, &mut compute_queue);
            }

            let mut gpu = Self {
                get_instance,
                get_device,
                instance,
                physical,
                device,
                queue,
                queue_family,
                compute_queue,
                compute_family,
                surface: vk_surface,
                swapchain: std::ptr::null_mut(),
                format: 44,
                extent_w: width.max(1),
                extent_h: height.max(1),
                swap_images: Vec::new(),
                color: Image::empty(),
                depth: Image::empty(),
                framebuffer: std::ptr::null_mut(),
                host: Buffer::empty(),
                vertex: Buffer::empty(),
                vertices: [Buffer::empty(), Buffer::empty()],
                shapes: Buffer::empty(),
                shapes_submitted: 0,
                instances: Buffer::empty(),
                instance_cache: Vec::new(),
                draws: Vec::new(),
                vertex_count: 0,
                overlay_count: 0,
                render_pass: std::ptr::null_mut(),
                pipeline: std::ptr::null_mut(),
                overlay_pipeline: std::ptr::null_mut(),
                wire_pipeline: std::ptr::null_mut(),
                wire_on: false,
                layout: std::ptr::null_mut(),
                pool: std::ptr::null_mut(),
                cmd: std::ptr::null_mut(),
                image_ready: std::ptr::null_mut(),
                render_done: std::ptr::null_mut(),
                fence: std::ptr::null_mut(),
                present_index: 0,
                cmds: [std::ptr::null_mut(); 2],
                fences: [std::ptr::null_mut(); 2],
                image_readies: [std::ptr::null_mut(); 2],
                render_dones: [std::ptr::null_mut(); 2],
                colors: [Image::empty(), Image::empty()],
                depths: [Image::empty(), Image::empty()],
                framebuffers: [std::ptr::null_mut(); 2],
                hosts: [Buffer::empty(), Buffer::empty()],
                flight: 0,
                scene_buf: Buffer::empty(),
                particle_buf: Buffer::empty(),
                desc_layout: std::ptr::null_mut(),
                desc_pool: std::ptr::null_mut(),
                desc_set: std::ptr::null_mut(),
                light_scene: [Buffer::empty(), Buffer::empty()],
                light_field: [Buffer::empty(), Buffer::empty()],
                light_sets: [std::ptr::null_mut(); 2],
                light_shown: 0,
                light_dst: 1,
                flight_light: [usize::MAX; 2],
                light_cmd: std::ptr::null_mut(),
                light_fence: std::ptr::null_mut(),
                light_pool: std::ptr::null_mut(),
                light_busy: false,
                light_building: false,
                light_pass: LIGHT_SLICES,
                light_row: 0,
                light_cols: [1; 3],
                light_rows: [1; 3],
                world_cols: 1,
                world_bands: 1,
                world_row: 0,
                world_pending: 0,
                swap_on_done: true,
                screen_w: 1,
                screen_h: 1,
                pin_counts: [0, 0],
                pin_bytes: Vec::new(),
                floor_half_x: 1.0,
                floor_half_z: 1.0,
                light_sem: std::ptr::null_mut(),
                light_sem_hot: false,
                light_wait_graphics: false,
                light_publish_flight: None,
                light_publish_fence: std::ptr::null_mut(),
                pending_light: None,
                compute_layout: std::ptr::null_mut(),
                compute_pipe: std::ptr::null_mut(),
                audio_rays: Buffer::empty(),
                audio_gains: Buffer::empty(),
                audio_desc_layout: std::ptr::null_mut(),
                audio_desc_pool: std::ptr::null_mut(),
                audio_set: std::ptr::null_mut(),
                audio_layout: std::ptr::null_mut(),
                audio_pipe: std::ptr::null_mut(),
                audio_cmd: std::ptr::null_mut(),
                audio_fence: std::ptr::null_mut(),
                light_ready: false,
                submit_pending: false,
                timestamp_period,
                timestamp_bits,
                query_pool: std::ptr::null_mut(),
                profile_submit: false,
                stamp_pending: [false; 2],
                inflight: VecDeque::new(),
                ready: VecDeque::new(),
                submitted_slot: None,
                antialias: Antialias::Off,
                color_layout: [0, 0],
                keep_pass: std::ptr::null_mut(),
                aa_src: [Buffer::empty(), Buffer::empty()],
                aa_dst: [Buffer::empty(), Buffer::empty()],
                ssaa_color: [Image::empty(), Image::empty()],
                ssaa_depth: [Image::empty(), Image::empty()],
                ssaa_fb: [std::ptr::null_mut(); 2],
                aa_desc_layout: std::ptr::null_mut(),
                aa_layout: std::ptr::null_mut(),
                aa_fxaa: std::ptr::null_mut(),
                aa_ssaa: std::ptr::null_mut(),
                aa_pool: std::ptr::null_mut(),
                aa_sets: [std::ptr::null_mut(); 2],
                fns: load_fns(
                    dload,
                    create_wayland_surface,
                    destroy_surface,
                    surface_support,
                    surface_caps,
                    surface_formats,
                    surface_modes,
                    destroy_instance,
                    enumerate_devices,
                    queue_families,
                    mem_props,
                ),
                memory_props,
            };
            let _ = (destroy_instance, surface_support);
            gpu.create_static_objects()?;
            gpu.recreate(width, height)?;
            Ok(gpu)
        }
    }

    fn create_static_objects(&mut self) -> Result<(), String> {
        unsafe {
            self.render_pass = self.make_render_pass(false)?;
            self.keep_pass = self.make_render_pass(true)?;
            self.desc_layout = self.make_desc_layout()?;
            self.layout = self.make_layout()?;
            self.pipeline = self.make_pipeline(true, true, false)?;
            self.overlay_pipeline = self.make_pipeline(false, false, false)?;
            self.wire_pipeline = self.make_pipeline(false, false, true)?;
            #[repr(C)]
            struct PoolInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                family: u32,
            }
            let pool = PoolInfo {
                s_type: 39,
                next: std::ptr::null(),
                flags: 0x2,
                family: self.queue_family,
            };
            check(
                (self.fns.create_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.pool,
                ),
                "command pool",
            )?;
            #[repr(C)]
            struct AllocInfo {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                level: u32,
                count: u32,
            }
            let alloc = AllocInfo {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.pool,
                level: 0,
                count: 2,
            };
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &alloc as *const AllocInfo as *const u8,
                    self.cmds.as_mut_ptr(),
                ),
                "command buffer",
            )?;
            self.cmd = self.cmds[0];
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            for slot in 0..2 {
                self.image_readies[slot] = self.make_sem()?;
                self.render_dones[slot] = self.make_sem()?;
                check(
                    (self.fns.create_fence)(
                        self.device,
                        &fence as *const FenceInfo as *const u8,
                        std::ptr::null(),
                        &mut self.fences[slot],
                    ),
                    "fence",
                )?;
            }
            self.image_ready = self.image_readies[0];
            self.render_done = self.render_dones[0];
            self.fence = self.fences[0];
            if self.timestamp_period > 0.0 {
                self.query_pool = self.make_query_pool()?;
            }
            self.make_lighting()?;
            self.make_aa_pipes()?;
        }
        Ok(())
    }

    fn recreate(&mut self, width: u32, height: u32) -> Result<(), String> {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        self.destroy_targets();
        self.create_targets(width, height)
    }

    /// Mailbox (1) when the surface lists it, then immediate (0), then FIFO (2).
    fn choose_present(&self) -> Result<i32, String> {
        const MAILBOX: i32 = 1;
        const IMMEDIATE: i32 = 0;
        const FIFO: i32 = 2;
        let mut count = 0u32;
        unsafe {
            check(
                (self.fns.surface_modes)(
                    self.physical,
                    self.surface,
                    &mut count,
                    std::ptr::null_mut(),
                ),
                "present modes",
            )?;
            if count == 0 {
                return Ok(FIFO);
            }
            let mut raw = vec![0u8; count as usize * 4];
            check(
                (self.fns.surface_modes)(self.physical, self.surface, &mut count, raw.as_mut_ptr()),
                "present mode list",
            )?;
            let mut mailbox = false;
            let mut immediate = false;
            for chunk in raw.chunks(4).take(count as usize) {
                let mode = i32::from_ne_bytes(chunk.try_into().unwrap());
                if mode == MAILBOX {
                    mailbox = true;
                } else if mode == IMMEDIATE {
                    immediate = true;
                }
            }
            if mailbox {
                Ok(MAILBOX)
            } else if immediate {
                Ok(IMMEDIATE)
            } else {
                Ok(FIFO)
            }
        }
    }

    fn create_targets(&mut self, width: u32, height: u32) -> Result<(), String> {
        unsafe {
            let mut caps = [0u8; 64];
            check(
                (self.fns.surface_caps)(self.physical, self.surface, caps.as_mut_ptr()),
                "caps",
            )?;
            let min_images = u32::from_ne_bytes(caps[0..4].try_into().unwrap());
            let max_images = u32::from_ne_bytes(caps[4..8].try_into().unwrap());
            let current_w = u32::from_ne_bytes(caps[8..12].try_into().unwrap());
            let current_h = u32::from_ne_bytes(caps[12..16].try_into().unwrap());
            let mut image_count = min_images.max(2);
            if max_images > 0 && image_count > max_images {
                image_count = max_images;
            }
            if current_w != u32::MAX && current_w > 0 {
                self.extent_w = current_w;
                self.extent_h = current_h;
            } else {
                self.extent_w = width.max(1);
                self.extent_h = height.max(1);
            }
            let mut format_count = 0u32;
            check(
                (self.fns.surface_formats)(
                    self.physical,
                    self.surface,
                    &mut format_count,
                    std::ptr::null_mut(),
                ),
                "formats",
            )?;
            let mut formats = vec![0u8; format_count as usize * 8];
            check(
                (self.fns.surface_formats)(
                    self.physical,
                    self.surface,
                    &mut format_count,
                    formats.as_mut_ptr(),
                ),
                "format list",
            )?;
            self.format = 44;
            let mut found = false;
            for chunk in formats.chunks(8) {
                let format = i32::from_ne_bytes(chunk[0..4].try_into().unwrap());
                if format == 44 || format == 37 {
                    self.format = format;
                    found = true;
                    if format == 44 {
                        break;
                    }
                }
            }
            if !found && format_count > 0 {
                self.format = i32::from_ne_bytes(formats[0..4].try_into().unwrap());
            }

            #[repr(C)]
            struct SwapInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                surface: Handle,
                min_count: u32,
                format: i32,
                color_space: i32,
                width: u32,
                height: u32,
                layers: u32,
                usage: u32,
                sharing: u32,
                queue_count: u32,
                queues: *const u32,
                transform: u32,
                alpha: u32,
                present: i32,
                clipped: u32,
                old: Handle,
            }
            let present = self.choose_present()?;
            // Mailbox needs a free image while one image is queued and one is on screen.
            if present == 1 {
                image_count = min_images.max(3);
                if max_images > 0 && image_count > max_images {
                    image_count = max_images;
                }
            }
            let transform = u32::from_ne_bytes(caps[36..40].try_into().unwrap());
            let swap = SwapInfo {
                s_type: 1000001000,
                next: std::ptr::null(),
                flags: 0,
                surface: self.surface,
                min_count: image_count,
                format: self.format,
                color_space: 0,
                width: self.extent_w,
                height: self.extent_h,
                layers: 1,
                usage: 0x10 | 0x2,
                sharing: 0,
                queue_count: 0,
                queues: std::ptr::null(),
                transform,
                alpha: 1,
                present,
                clipped: 1,
                old: std::ptr::null_mut(),
            };
            check(
                (self.fns.create_swapchain)(
                    self.device,
                    &swap as *const SwapInfo as *const u8,
                    std::ptr::null(),
                    &mut self.swapchain,
                ),
                "swapchain",
            )?;
            let mut count = 0u32;
            check(
                (self.fns.swapchain_images)(
                    self.device,
                    self.swapchain,
                    &mut count,
                    std::ptr::null_mut(),
                ),
                "image count",
            )?;
            self.swap_images = vec![std::ptr::null_mut(); count as usize];
            check(
                (self.fns.swapchain_images)(
                    self.device,
                    self.swapchain,
                    &mut count,
                    self.swap_images.as_mut_ptr() as *mut u8,
                ),
                "image list",
            )?;

            let bytes = self.extent_w as u64 * self.extent_h as u64 * 4;
            for slot in 0..2 {
                self.colors[slot] = self.make_image(
                    self.format,
                    0x10 | 0x4 | 0x1 | 0x2,
                    1,
                    self.extent_w,
                    self.extent_h,
                )?;
                self.depths[slot] = self.make_image(126, 0x20, 2, self.extent_w, self.extent_h)?;
                self.color = copy_image(&self.colors[slot]);
                self.depth = copy_image(&self.depths[slot]);
                self.framebuffers[slot] = self.make_framebuffer()?;
                self.hosts[slot] = self.make_buffer(bytes, 0x2, true)?;
            }
            self.color = copy_image(&self.colors[0]);
            self.depth = copy_image(&self.depths[0]);
            self.framebuffer = self.framebuffers[0];
            self.host = copy_buffer(&self.hosts[0]);
            self.make_aa_targets()?;
            Ok(())
        }
    }

    fn upload(&mut self, verts: &[GpuVertex]) -> Result<(), String> {
        if verts.is_empty() {
            return Ok(());
        }
        let bytes = (verts.len() * std::mem::size_of::<GpuVertex>()) as u64;
        if self.vertex.size < bytes {
            unsafe {
                if !self.vertex.buffer.is_null() {
                    (self.fns.destroy_buffer)(self.device, self.vertex.buffer, std::ptr::null());
                    (self.fns.free_mem)(self.device, self.vertex.memory, std::ptr::null());
                }
            }
            self.vertex = self.make_buffer(bytes.max(1024), 0x80, true)?;
            self.vertices[self.flight] = copy_buffer(&self.vertex);
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, self.vertex.memory, 0, bytes, 0, &mut mapped),
                "map vertices",
            )?;
            std::ptr::copy_nonoverlapping(
                verts.as_ptr() as *const u8,
                mapped as *mut u8,
                bytes as usize,
            );
            (self.fns.unmap_mem)(self.device, self.vertex.memory);
        }
        Ok(())
    }

    fn set_draws(&mut self, draws: Vec<DrawSpan>) {
        self.draws = draws;
    }

    /// Copy dirty shape ranges. A new tail does not wait. A reused span waits for both frames.
    fn upload_shapes(&mut self, pool: &crate::pool::MeshPool) -> Result<(), String> {
        let verts = pool.vertices();
        if verts.is_empty() {
            return Ok(());
        }
        let stride = std::mem::size_of::<GpuVertex>() as u64;
        let bytes = verts.len() as u64 * stride;
        let reuse = pool
            .dirty()
            .iter()
            .any(|(offset, _)| *offset < self.shapes_submitted);
        if reuse || self.shapes.size < bytes {
            self.wait_all_inflight()?;
        }
        if self.shapes.size < bytes {
            let next = self.make_buffer(
                bytes.max(self.shapes.size.saturating_mul(2)).max(4096),
                0x80,
                true,
            )?;
            self.write_verts_at(&next, 0, verts)?;
            let mut old = std::mem::replace(&mut self.shapes, next);
            self.destroy_buffer(&mut old);
        } else {
            let shapes = copy_buffer(&self.shapes);
            for &(offset, count) in pool.dirty() {
                let start = offset as usize;
                let end = start + count as usize;
                self.write_verts_at(&shapes, offset, &verts[start..end])?;
            }
        }
        self.shapes_submitted = verts.len() as u32;
        Ok(())
    }

    fn write_verts_at(
        &self,
        buffer: &Buffer,
        first: u32,
        verts: &[GpuVertex],
    ) -> Result<(), String> {
        if verts.is_empty() {
            return Ok(());
        }
        let stride = std::mem::size_of::<GpuVertex>() as u64;
        let offset = first as u64 * stride;
        let bytes = verts.len() as u64 * stride;
        let end = offset + bytes;
        if buffer.size < end {
            return Err("shape buffer is smaller than the block".into());
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, buffer.memory, 0, end, 0, &mut mapped),
                "map shape block",
            )?;
            std::ptr::copy_nonoverlapping(
                verts.as_ptr() as *const u8,
                (mapped as *mut u8).add(offset as usize),
                bytes as usize,
            );
            (self.fns.unmap_mem)(self.device, buffer.memory);
        }
        Ok(())
    }

    fn write_instances(&mut self, items: &[InstanceRec]) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(items.len() * 80);
        for item in items {
            for value in item.model {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            for value in item.color {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        if bytes == self.instance_cache {
            return Ok(());
        }
        if !self.instance_cache.is_empty() {
            self.wait_all_inflight()?;
        }
        if self.instances.size < bytes.len() as u64 {
            let next = self.make_buffer((bytes.len() as u64).max(5120), 0x20, true)?;
            let mut old = std::mem::replace(&mut self.instances, next);
            self.destroy_buffer(&mut old);
            self.write_instance_binding()?;
        }
        self.write_buffer(&self.instances, &bytes)?;
        self.instance_cache = bytes;
        Ok(())
    }

    fn write_instance_binding(&self) -> Result<(), String> {
        self.write_light_set(0)?;
        self.write_light_set(1)
    }

    fn bind_flight(&mut self) {
        let slot = self.flight;
        self.cmd = self.cmds[slot];
        self.fence = self.fences[slot];
        self.image_ready = self.image_readies[slot];
        self.render_done = self.render_dones[slot];
        self.color = copy_image(&self.colors[slot]);
        self.depth = copy_image(&self.depths[slot]);
        self.framebuffer = self.framebuffers[slot];
        self.host = copy_buffer(&self.hosts[slot]);
        self.vertex = copy_buffer(&self.vertices[slot]);
    }

    fn record_and_submit(&mut self, matrix: &[f32; 16]) -> Result<(), String> {
        let profiled = self.profile_submit;
        self.profile_submit = false;
        if profiled && (self.query_pool.is_null() || self.timestamp_period <= 0.0) {
            return Err("Vulkan timestamp queries are not available on this device".into());
        }
        self.poll_light()?;
        self.desc_set = self.light_sets[self.light_shown];
        self.bind_flight();
        let slot = self.flight;
        unsafe {
            let fences = [self.fence];
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "wait fence",
            )?;
            if self.light_publish_flight == Some(slot) {
                self.light_publish_flight = None;
            }
            self.collect_slot_after_wait(slot)?;
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset fence",
            )?;
            let mut index = 0u32;
            let acquire = (self.fns.acquire)(
                self.device,
                self.swapchain,
                u64::MAX,
                self.image_ready,
                std::ptr::null_mut(),
                &mut index,
            );
            if acquire == OUT_OF_DATE {
                return Err("out of date".into());
            }
            check(acquire, "acquire")?;
            check((self.fns.reset_cmd)(self.cmd, 0), "reset cmd")?;
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
                (self.fns.begin_cmd)(self.cmd, &begin as *const BeginInfo as *const u8),
                "begin cmd",
            )?;
            if profiled {
                self.reset_queries(slot);
                // Top of pipe, before the GPU work. The value is device ticks.
                self.write_stamp(slot, 0, 1);
            }
            let swap = self.swap_images[index as usize];
            self.image_barrier(swap, 0, 7, 1, 0x1000, 0, 0x1000);
            let filtering = self.antialias != Antialias::Off;
            if self.antialias == Antialias::Ssaa {
                self.raster_scene(
                    self.ssaa_fb[slot],
                    self.extent_w * 2,
                    self.extent_h * 2,
                    matrix,
                    false,
                );
                self.resolve_ssaa(slot)?;
            } else {
                self.raster_scene(
                    self.framebuffer,
                    self.extent_w,
                    self.extent_h,
                    matrix,
                    !filtering,
                );
                self.color_layout[slot] = 6;
                if self.antialias == Antialias::Fxaa {
                    self.resolve_fxaa(slot)?;
                }
            }
            if filtering && self.overlay_count > 0 {
                self.raster_overlay();
            }
            self.copy_color_to_swapchain(swap);
            self.copy_color_to_buffer();
            self.image_barrier(swap, 7, 1000001002, 0x1000, 0x2000, 0x1000, 0);
            if profiled {
                // Bottom of pipe, after the draw and the copies.
                // The value is device ticks, not the CPU time spent in submit.
                self.write_stamp(slot, 1, 0x2000);
            }
            check((self.fns.end_cmd)(self.cmd), "end cmd")?;

            let wait_stage = [0x400u32, 0x80];
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
            let wait_sem = [self.image_ready, self.light_sem];
            let mut wait_count = 1u32;
            if self.light_wait_graphics {
                wait_count = 2;
                self.light_wait_graphics = false;
                self.light_sem_hot = false;
                // The signal stays with this flight until its fence signals.
                self.light_publish_flight = Some(slot);
            }
            let signal_sem = [self.render_done];
            let cmd = [self.cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count,
                waits: wait_sem.as_ptr(),
                stages: wait_stage.as_ptr(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 1,
                signals: signal_sem.as_ptr(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.fence,
                ),
                "submit",
            )?;
            self.present_index = index;
            self.flight_light[slot] = self.light_shown;
            let status = (self.fns.fence_status)(self.device, self.fence);
            self.submit_pending = status == 1;
            if profiled {
                self.stamp_pending[slot] = true;
                self.inflight.push_back(slot);
                self.submitted_slot = Some(slot);
            } else {
                self.submitted_slot = None;
            }
            self.flight = 1 - self.flight;
        }
        Ok(())
    }

    fn make_query_pool(&mut self) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            query_type: u32,
            query_count: u32,
            pipeline_statistics: u32,
        }
        let info = Info {
            s_type: 11,
            next: std::ptr::null(),
            flags: 0,
            // VK_QUERY_TYPE_TIMESTAMP is 2. Type 1 is pipeline statistics.
            query_type: 2,
            query_count: 4,
            pipeline_statistics: 0,
        };
        let mut pool = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_query_pool)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut pool,
                ),
                "query pool",
            )?;
        }
        Ok(pool)
    }

    fn reset_queries(&self, slot: usize) {
        unsafe {
            (self.fns.cmd_reset_query)(self.cmd, self.query_pool, (slot * 2) as u32, 2);
        }
    }

    fn write_stamp(&self, slot: usize, index: u32, stage: u32) {
        unsafe {
            (self.fns.cmd_write_timestamp)(
                self.cmd,
                stage,
                self.query_pool,
                (slot * 2) as u32 + index,
            );
        }
    }

    /// The slot fence has already been waited. Read the timestamp span if this slot still owes one.
    fn collect_slot_after_wait(&mut self, slot: usize) -> Result<(), String> {
        if !self.stamp_pending[slot] {
            return Ok(());
        }
        if self.inflight.front().copied() != Some(slot) {
            return Err("profile slot finished out of order".into());
        }
        let duration = self.read_span(slot)?;
        self.stamp_pending[slot] = false;
        self.inflight.pop_front();
        self.ready.push_back(duration);
        Ok(())
    }

    fn collect_submitted_if_ready(&mut self) -> Result<Option<Duration>, String> {
        let Some(slot) = self.submitted_slot else {
            return Ok(None);
        };
        if self.inflight.front().copied() != Some(slot) || !self.stamp_pending[slot] {
            return Ok(None);
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.fences[slot]) };
        if status == 1 {
            return Ok(None);
        }
        check(status, "timestamp fence")?;
        let duration = self.read_span(slot)?;
        self.stamp_pending[slot] = false;
        self.inflight.pop_front();
        Ok(Some(duration))
    }

    fn collect_ready(&mut self, wait: bool) -> Result<(), String> {
        while let Some(slot) = self.inflight.front().copied() {
            if !self.stamp_pending[slot] {
                self.inflight.pop_front();
                continue;
            }
            let fence = self.fences[slot];
            if wait {
                let fences = [fence];
                unsafe {
                    check(
                        (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                        "profile fence",
                    )?;
                }
            } else {
                let status = unsafe { (self.fns.fence_status)(self.device, fence) };
                if status == 1 {
                    break;
                }
                check(status, "profile fence")?;
            }
            let duration = self.read_span(slot)?;
            self.stamp_pending[slot] = false;
            self.inflight.pop_front();
            self.ready.push_back(duration);
        }
        Ok(())
    }

    fn read_span(&self, slot: usize) -> Result<Duration, String> {
        let mut raw = [0u64; 2];
        let waited = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.query_pool,
                (slot * 2) as u32,
                2,
                std::mem::size_of::<[u64; 2]>(),
                raw.as_mut_ptr() as *mut c_void,
                8,
                0x1 | 0x2,
            )
        };
        check(waited, "timestamp query")?;
        let delta = tick_delta(raw[0], raw[1], self.timestamp_bits);
        if raw[0] == 0 || raw[1] == 0 || delta == 0 {
            return Err("timestamp query returned no device time".into());
        }
        let nanos = delta as f64 * f64::from(self.timestamp_period);
        if !nanos.is_finite() || nanos < 0.0 || nanos > u64::MAX as f64 {
            return Err("timestamp period produced a bad duration".into());
        }
        Ok(Duration::from_nanos(nanos.round() as u64))
    }

    fn wait_flight(&mut self, slot: usize) -> Result<(), String> {
        unsafe {
            let fences = [self.fences[slot]];
            if fences[0].is_null() {
                return Ok(());
            }
            let status = (self.fns.fence_status)(self.device, fences[0]);
            if status == 1 {
                check(
                    (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                    "flight wait",
                )?;
            }
        }
        if self.light_publish_flight == Some(slot) {
            self.light_publish_flight = None;
        }
        Ok(())
    }

    fn wait_all_inflight(&mut self) -> Result<(), String> {
        for slot in 0..2 {
            self.wait_flight(slot)?;
        }
        Ok(())
    }

    fn read_host(&mut self, slot: usize) -> Result<Vec<u8>, String> {
        unsafe {
            let fences = [self.fences[slot]];
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "earlier frame",
            )?;
        }
        let saved = copy_buffer(&self.host);
        self.host = copy_buffer(&self.hosts[slot]);
        let pixels = self.read_color()?;
        self.host = saved;
        Ok(pixels)
    }

    fn wait_gpu(&mut self) -> Result<(), String> {
        unsafe {
            let fences = [self.fence];
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "present wait",
            )?;
        }
        Ok(())
    }

    fn present(&mut self) -> Result<VkResult, String> {
        unsafe {
            let swap = [self.swapchain];
            let index = [self.present_index];
            let wait = [self.render_done];
            #[repr(C)]
            struct Present {
                s_type: i32,
                next: *const c_void,
                wait_count: u32,
                waits: *const Handle,
                swap_count: u32,
                swaps: *const Handle,
                indices: *const u32,
                results: *mut i32,
            }
            let info = Present {
                s_type: 1000001001,
                next: std::ptr::null(),
                wait_count: 1,
                waits: wait.as_ptr(),
                swap_count: 1,
                swaps: swap.as_ptr(),
                indices: index.as_ptr(),
                results: std::ptr::null_mut(),
            };
            let result = (self.fns.present)(self.queue, &info as *const Present as *const u8);
            if result != VK_SUCCESS && result != SUBOPTIMAL && result != OUT_OF_DATE {
                return Err(format!("present failed: {result}"));
            }
            Ok(result)
        }
    }

    fn read_color(&self) -> Result<Vec<u8>, String> {
        let bytes = self.extent_w as usize * self.extent_h as usize * 4;
        let mut out = vec![0u8; bytes];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    self.host.memory,
                    0,
                    self.host.size,
                    0,
                    &mut mapped,
                ),
                "map readback",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u8, out.as_mut_ptr(), bytes);
            (self.fns.unmap_mem)(self.device, self.host.memory);
        }
        Ok(out)
    }

    fn make_sem(&mut self) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
        }
        let info = Info {
            s_type: 9,
            next: std::ptr::null(),
            flags: 0,
        };
        let mut sem = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_sem)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut sem,
                ),
                "semaphore",
            )?;
        }
        Ok(sem)
    }

    fn make_desc_layout(&self) -> Result<Handle, String> {
        #[repr(C)]
        struct Binding {
            binding: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        #[repr(C)]
        struct Info {
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
                stages: 0x10 | 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 1,
                kind: 7,
                count: 1,
                stages: 0x10 | 0x20,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 2,
                kind: 7,
                count: 1,
                stages: 0x10,
                samplers: std::ptr::null(),
            },
            Binding {
                binding: 3,
                kind: 7,
                count: 1,
                stages: 0x1,
                samplers: std::ptr::null(),
            },
        ];
        let info = Info {
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
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "descriptor layout",
            )?;
        }
        Ok(layout)
    }

    fn make_lighting(&mut self) -> Result<(), String> {
        self.scene_buf = self.make_buffer(4096, 0x20, true)?;
        let share = self.light_families();
        self.instances = self.make_buffer_queues(5120, 0x20, true, &share)?;
        self.particle_buf = self.make_buffer_queues(16 + 256 * 256 * 4, 0x20, true, &share)?;
        let field_bytes = vec![0u8; 2 * crate::field::FIELD_COPY as usize * 16];
        for index in 0..2 {
            self.light_scene[index] = self.make_buffer_queues(4096, 0x20, true, &share)?;
            self.light_field[index] =
                self.make_buffer_queues(field_bytes.len() as u64, 0x20, true, &share)?;
            self.write_buffer(&self.light_field[index], &field_bytes)?;
            self.write_buffer(&self.light_scene[index], &vec![0u8; 4096])?;
        }
        self.write_buffer(&self.particle_buf, &vec![0u8; 16])?;
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
        let sets = [self.desc_layout];
        let push = Range {
            stage: 0x20,
            offset: 0,
            size: 8,
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
                    &mut self.compute_layout,
                ),
                "compute layout",
            )?;
        }
        let module = self.shader(COMP_SPV)?;
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
            layout: self.compute_layout,
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
                &mut self.compute_pipe,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "compute pipeline")?;
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
        let size = Size { kind: 7, count: 8 };
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
                    &mut self.desc_pool,
                ),
                "descriptor pool",
            )?;
            #[repr(C)]
            struct Alloc {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                count: u32,
                layouts: *const Handle,
            }
            let layouts = [self.desc_layout, self.desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.desc_pool,
                count: 2,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    self.light_sets.as_mut_ptr(),
                ),
                "descriptor set",
            )?;
        }
        self.desc_set = self.light_sets[0];
        self.write_light_set(0)?;
        self.write_light_set(1)?;
        self.write_buffer(&self.scene_buf, &vec![0u8; 4096])?;
        self.make_light_queue()?;
        self.make_audio()?;
        Ok(())
    }

    fn light_families(&self) -> Vec<u32> {
        if self.compute_family == self.queue_family {
            Vec::new()
        } else {
            vec![self.queue_family, self.compute_family]
        }
    }

    fn write_light_set(&self, index: usize) -> Result<(), String> {
        self.write_descriptors(
            self.light_sets[index],
            &self.light_scene[index],
            &self.light_field[index],
        )
    }

    fn write_descriptors(&self, set: Handle, scene: &Buffer, field: &Buffer) -> Result<(), String> {
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
        let infos = [
            BufInfo {
                buffer: scene.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: field.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.particle_buf.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.instances.buffer,
                offset: 0,
                range: u64::MAX,
            },
        ];
        let writes = [
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
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
                set,
                binding: 1,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[1],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 2,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[2],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set,
                binding: 3,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[3],
                texel: std::ptr::null(),
            },
        ];
        unsafe {
            (self.fns.update_desc)(
                self.device,
                4,
                writes.as_ptr() as *const u8,
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn make_audio(&mut self) -> Result<(), String> {
        self.audio_rays = self.make_buffer(8192, 0x20, true)?;
        self.audio_gains = self.make_buffer(4096, 0x20, true)?;
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
            count: 3,
            bindings: bindings.as_ptr(),
        };
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_desc_layout,
                ),
                "audio descriptor layout",
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
        let sets = [self.audio_desc_layout];
        let layout = LayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr(),
            push_count: 0,
            push: std::ptr::null(),
        };
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_layout,
                ),
                "audio pipeline layout",
            )?;
        }
        let module = self.shader(AUDIO_SPV)?;
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
            layout: self.audio_layout,
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
                &mut self.audio_pipe,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, "audio pipeline")?;
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
        let size = Size { kind: 7, count: 3 };
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
                    &mut self.audio_desc_pool,
                ),
                "audio descriptor pool",
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
                pool: self.audio_desc_pool,
                count: 1,
                layouts: sets.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    &mut self.audio_set,
                ),
                "audio descriptor set",
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
                    &mut self.audio_cmd,
                ),
                "audio command buffer",
            )?;
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.audio_fence,
                ),
                "audio fence",
            )?;
        }
        self.write_audio_descriptors()
    }

    fn write_audio_descriptors(&self) -> Result<(), String> {
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
        let infos = [
            BufInfo {
                buffer: self.scene_buf.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.audio_rays.buffer,
                offset: 0,
                range: u64::MAX,
            },
            BufInfo {
                buffer: self.audio_gains.buffer,
                offset: 0,
                range: u64::MAX,
            },
        ];
        let writes = [
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: self.audio_set,
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
                set: self.audio_set,
                binding: 1,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[1],
                texel: std::ptr::null(),
            },
            Write {
                s_type: 35,
                next: std::ptr::null(),
                set: self.audio_set,
                binding: 2,
                element: 0,
                count: 1,
                kind: 7,
                image: std::ptr::null(),
                buffer: &infos[2],
                texel: std::ptr::null(),
            },
        ];
        unsafe {
            (self.fns.update_desc)(
                self.device,
                3,
                writes.as_ptr() as *const u8,
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn transmission_gains(
        &mut self,
        listener: [f32; 3],
        sources: &[[f32; 3]],
    ) -> Result<Vec<f32>, String> {
        if sources.len() > 256 {
            return Err("too many audio sources".into());
        }
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        self.write_buffer(&self.audio_rays, &ray_bytes(listener, sources))?;
        self.write_buffer(&self.audio_gains, &vec![0u8; sources.len() * 4])?;
        unsafe {
            let fences = [self.audio_fence];
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "audio wait",
            )?;
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "audio reset fence",
            )?;
            check((self.fns.reset_cmd)(self.audio_cmd, 0), "audio reset")?;
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
                (self.fns.begin_cmd)(self.audio_cmd, &begin as *const BeginInfo as *const u8),
                "audio begin",
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
                self.audio_cmd,
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
            (self.fns.cmd_bind_pipe)(self.audio_cmd, 1, self.audio_pipe);
            (self.fns.cmd_bind_set)(
                self.audio_cmd,
                1,
                self.audio_layout,
                0,
                1,
                &self.audio_set,
                0,
                std::ptr::null(),
            );
            let groups = (sources.len() as u32).div_ceil(64);
            (self.fns.cmd_dispatch)(self.audio_cmd, groups, 1, 1);
            let back = MemBar {
                s_type: 46,
                next: std::ptr::null(),
                src_access: 0x40,
                dst_access: 0x10,
            };
            (self.fns.cmd_barrier)(
                self.audio_cmd,
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
            check((self.fns.end_cmd)(self.audio_cmd), "audio end")?;
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
            let cmd = [self.audio_cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.audio_fence,
                ),
                "audio submit",
            )?;
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "audio done",
            )?;
        }
        let memory = self.audio_gains.memory;
        let device = self.device;
        let map = self.fns.map_mem;
        let unmap = self.fns.unmap_mem;
        read_f32s(map, unmap, device, memory, sources.len())
    }

    fn upload_scene(&mut self, pack: &Pack) -> Result<(), String> {
        let bytes = pack::scene_bytes(pack);
        self.write_buffer(&self.scene_buf, &bytes)?;
        for (index, cascade) in pack.cascades.iter().enumerate() {
            self.light_cols[index] = (cascade.count_x + 7) / 8;
            self.light_rows[index] = (cascade.count_z + 7) / 8;
        }
        let (world_x, world_z) = crate::field::world_counts(pack.floor_half_x, pack.floor_half_z);
        self.world_cols = (world_x + 7) / 8;
        self.world_bands = (world_z + 7) / 8;
        self.screen_w = pack.grid_w.max(1);
        self.screen_h = pack.grid_h.max(1);
        self.pin_counts = [pack.pin_count0, pack.pin_count1];
        self.floor_half_x = pack.floor_half_x;
        self.floor_half_z = pack.floor_half_z;
        self.pending_light = Some(bytes);
        Ok(())
    }

    /// Record one slice of the gather. A readback waits until that build is the field on screen.
    fn kick_light(&mut self, wait: bool) -> Result<(), String> {
        loop {
            self.poll_light()?;
            // The finished field's signal belongs to the next picture. A slice
            // that waits or signals it first makes that picture stall on the gather.
            if self.publish_blocks() && !wait {
                return Ok(());
            }
            if !self.light_busy && !self.light_building && self.pending_light.is_some() {
                if self.light_dst_free() {
                    if self.publish_blocks() {
                        self.drain_publish()?;
                    }
                    self.begin_light()?;
                } else if wait {
                    self.wait_all_inflight()?;
                    continue;
                }
            }
            if !self.light_busy && self.light_building {
                self.submit_slice()?;
            }
            let more = self.pending_light.is_some() || self.light_busy || self.light_building;
            if !wait || !more {
                return Ok(());
            }
            if self.light_busy {
                self.wait_light()?;
                continue;
            }
            self.wait_all_inflight()?;
        }
    }

    fn publish_blocks(&self) -> bool {
        self.light_wait_graphics || self.light_publish_flight.is_some()
    }

    /// The picture has not taken the publish signal yet, and a newer build has to signal it.
    fn drain_publish(&mut self) -> Result<(), String> {
        if let Some(slot) = self.light_publish_flight.take() {
            let fences = [self.fences[slot]];
            unsafe {
                check(
                    (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                    "light publish",
                )?;
            }
            self.light_wait_graphics = false;
            self.light_sem_hot = false;
            return Ok(());
        }
        if self.light_wait_graphics {
            self.consume_publish()?;
        }
        Ok(())
    }

    fn consume_publish(&mut self) -> Result<(), String> {
        let fences = [self.light_publish_fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset light publish",
            )?;
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
            let wait_sem = [self.light_sem];
            let wait_stage = 0x80u32;
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 1,
                waits: wait_sem.as_ptr(),
                stages: &wait_stage,
                cmd_count: 0,
                cmds: std::ptr::null(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.light_publish_fence,
                ),
                "light publish",
            )?;
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "light publish",
            )?;
        }
        self.light_wait_graphics = false;
        self.light_sem_hot = false;
        Ok(())
    }

    fn poll_light(&mut self) -> Result<(), String> {
        if !self.light_busy || self.light_fence.is_null() {
            return Ok(());
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.light_fence) };
        if status == 1 {
            return Ok(());
        }
        check(status, "light fence")?;
        self.light_busy = false;
        if self.light_pass >= LIGHT_SLICES {
            self.light_building = false;
            self.light_shown = self.light_dst;
            self.desc_set = self.light_sets[self.light_shown];
            self.light_ready = true;
            self.light_wait_graphics = true;
        }
        Ok(())
    }

    /// A lamp or occluder change restarts the world cache. The screen field does not wait for it.
    #[allow(dead_code)]
    fn mark_world_dirty(&mut self) {
        self.world_pending = self.world_bands.max(1);
        self.world_row = 0;
    }

    /// One world band into the cache the picture reads. The screen field stays.
    #[allow(dead_code)]
    fn kick_world(&mut self, wait: bool) -> Result<(), String> {
        if self.world_pending == 0 {
            return Ok(());
        }
        self.poll_light()?;
        if self.publish_blocks() && !wait {
            return Ok(());
        }
        if self.light_busy || self.light_building {
            if !wait {
                return Ok(());
            }
            self.wait_light()?;
        }
        if wait && self.publish_blocks() {
            self.drain_publish()?;
        }
        if self.world_pending == 0 {
            return Ok(());
        }
        self.light_dst = 1 - self.light_shown;
        self.submit_hybrid(false, false)?;
        if wait {
            self.wait_light()?;
        }
        Ok(())
    }

    fn light_dst_free(&self) -> bool {
        let dst = 1 - self.light_shown;
        (0..2).all(|slot| self.flight_light[slot] != dst || !self.fence_pending(slot))
    }

    fn fence_pending(&self, slot: usize) -> bool {
        let fence = self.fences[slot];
        if fence.is_null() {
            return false;
        }
        unsafe { (self.fns.fence_status)(self.device, fence) == 1 }
    }

    fn wait_light(&mut self) -> Result<(), String> {
        let fences = [self.light_fence];
        unsafe {
            check(
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX),
                "light wait",
            )?;
        }
        self.poll_light()
    }

    fn begin_light(&mut self) -> Result<(), String> {
        let Some(bytes) = self.pending_light.take() else {
            return Ok(());
        };
        let dst = 1 - self.light_shown;
        self.write_buffer(&self.light_scene[dst], &bytes)?;
        if !self.pin_bytes.is_empty() {
            let offset = crate::pins::PIN_SPAN_START as u64 * 16;
            self.write_buffer_at(&self.light_field[dst], offset, &self.pin_bytes)?;
        }
        self.light_dst = dst;
        self.light_pass = 0;
        self.light_row = 0;
        self.light_building = true;
        Ok(())
    }

    /// One workgroup row of the current cascade. The next call continues the build.
    fn submit_slice(&mut self) -> Result<(), String> {
        let dst = self.light_dst;
        let fences = [self.light_fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset light fence",
            )?;
            check((self.fns.reset_cmd)(self.light_cmd, 0), "reset light cmd")?;
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
                (self.fns.begin_cmd)(self.light_cmd, &begin as *const BeginInfo as *const u8),
                "begin light",
            )?;
        }
        self.dispatch_screen_field(self.light_cmd, self.light_sets[dst])?;
        unsafe {
            check((self.fns.end_cmd)(self.light_cmd), "end light")?;
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
            let cmd = [self.light_cmd];
            let wait_sem = [self.light_sem];
            let signal_sem = [self.light_sem];
            let wait_stage = 0x800u32;
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: u32::from(self.light_sem_hot),
                waits: wait_sem.as_ptr(),
                stages: &wait_stage,
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 1,
                signals: signal_sem.as_ptr(),
            };
            self.light_sem_hot = true;
            check(
                (self.fns.queue_submit)(
                    self.compute_queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.light_fence,
                ),
                "light submit",
            )?;
        }
        self.light_row = 0;
        self.light_pass = LIGHT_SLICES;
        self.light_busy = true;
        Ok(())
    }

    /// 3D world probes, one bounce, then the screen cascades from far to near.
    fn dispatch_screen_field(&mut self, cmd: Handle, set: Handle) -> Result<(), String> {
        let half_x = self.floor_half_x.max(0.5);
        let half_z = self.floor_half_z.max(0.5);
        let span_x = half_x * 2.0 + 2.5 * 4.0;
        let span_z = half_z * 2.0 + 2.5 * 4.0;
        let count_x = ((span_x / 2.5).ceil() as u32).clamp(1, 48);
        let count_z = ((span_z / 2.5).ceil() as u32).clamp(1, 48);
        let count_y = 3u32;
        let world_x = (count_x + 7) / 8;
        let world_y = (count_y * count_z + 7) / 8;
        // Pass 8 stores direct hits. Pass 9 adds one bounce from that volume.
        self.dispatch_light_slice(cmd, set, 8, 0, world_y.max(1), world_x.max(1))?;
        self.dispatch_light_slice(cmd, set, 9, 0, world_y.max(1), world_x.max(1))?;
        for pass in 0..3u32 {
            let level = 2 - pass;
            if level < 2 {
                let count = self.pin_counts[level as usize];
                if count == 0 {
                    continue;
                }
                // One workgroup is 8 by 8. The shader reads a linear pin index.
                let groups = (count + 63) / 64;
                self.dispatch_light_slice(cmd, set, pass, 0, 1, groups)?;
                continue;
            }
            // Screen cascade 2 is not read since the pinned cascades took over 0 and 1:
            // only screen_interval reads it, and only the screen path for 0 and 1 calls that.
        }
        // Bake the lit pins into the two lookup volumes the picture samples.
        for (level, cells) in [
            (0u32, crate::pins::VOL_XZ0 * crate::pins::VOL_XZ0 * crate::pins::VOL_Y0),
            (1u32, crate::pins::VOL_XZ1 * crate::pins::VOL_XZ1 * crate::pins::VOL_Y1),
        ] {
            self.dispatch_light_slice(cmd, set, 10 + level, 0, 1, (cells + 63) / 64)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn submit_hybrid(&mut self, all_world: bool, screen: bool) -> Result<(), String> {
        self.swap_on_done = screen;
        let dst = self.light_dst;
        let fences = [self.light_fence];
        unsafe {
            check(
                (self.fns.reset_fences)(self.device, 1, fences.as_ptr()),
                "reset light fence",
            )?;
            check((self.fns.reset_cmd)(self.light_cmd, 0), "reset light cmd")?;
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
                (self.fns.begin_cmd)(self.light_cmd, &begin as *const BeginInfo as *const u8),
                "begin light",
            )?;
        }
        self.record_hybrid(self.light_cmd, dst, all_world, screen)?;
        unsafe {
            check((self.fns.end_cmd)(self.light_cmd), "end light")?;
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
            let cmd = [self.light_cmd];
            let wait_sem = [self.light_sem];
            let signal_sem = [self.light_sem];
            let wait_stage = 0x800u32;
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: u32::from(self.light_sem_hot),
                waits: wait_sem.as_ptr(),
                stages: &wait_stage,
                cmd_count: 1,
                cmds: cmd.as_ptr(),
                signal_count: 1,
                signals: signal_sem.as_ptr(),
            };
            self.light_sem_hot = true;
            check(
                (self.fns.queue_submit)(
                    self.compute_queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.light_fence,
                ),
                "light submit",
            )?;
        }
        self.light_row = 0;
        self.light_pass = LIGHT_SLICES;
        self.light_busy = true;
        Ok(())
    }

    fn make_light_queue(&mut self) -> Result<(), String> {
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            family: u32,
        }
        let pool = PoolInfo {
            s_type: 39,
            next: std::ptr::null(),
            flags: 0x2,
            family: self.compute_family,
        };
        unsafe {
            check(
                (self.fns.create_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.light_pool,
                ),
                "light command pool",
            )?;
            #[repr(C)]
            struct AllocInfo {
                s_type: i32,
                next: *const c_void,
                pool: Handle,
                level: u32,
                count: u32,
            }
            let alloc = AllocInfo {
                s_type: 40,
                next: std::ptr::null(),
                pool: self.light_pool,
                level: 0,
                count: 1,
            };
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &alloc as *const AllocInfo as *const u8,
                    &mut self.light_cmd,
                ),
                "light command buffer",
            )?;
            #[repr(C)]
            struct FenceInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
            }
            let fence = FenceInfo {
                s_type: 8,
                next: std::ptr::null(),
                flags: 0x1,
            };
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.light_fence,
                ),
                "light fence",
            )?;
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.light_publish_fence,
                ),
                "light publish fence",
            )?;
        }
        self.light_sem = self.make_sem()?;
        Ok(())
    }

    fn upload_image(&self, bytes: &[u8]) -> Result<(), String> {
        self.write_buffer(&self.particle_buf, bytes)
    }

    fn write_buffer(&self, buffer: &Buffer, bytes: &[u8]) -> Result<(), String> {
        self.write_buffer_at(buffer, 0, bytes)
    }

    fn write_buffer_at(&self, buffer: &Buffer, offset: u64, bytes: &[u8]) -> Result<(), String> {
        if offset.saturating_add(bytes.len() as u64) > buffer.size {
            return Err("gpu buffer is too small".into());
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    buffer.memory,
                    offset,
                    bytes.len() as u64,
                    0,
                    &mut mapped,
                ),
                "map gpu buffer",
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped as *mut u8, bytes.len());
            (self.fns.unmap_mem)(self.device, buffer.memory);
        }
        Ok(())
    }

    /// World bands, then the screen field when `screen` is set.
    /// A readback fills every world band. An interactive frame writes one band while the cache is dirty.
    #[allow(dead_code)]
    fn record_hybrid(
        &mut self,
        cmd: Handle,
        dst: usize,
        all_world: bool,
        screen: bool,
    ) -> Result<(), String> {
        let _ = (all_world, self.world_cols, self.world_row);
        if !screen {
            return Ok(());
        }
        // Two bounces, farthest cascade first. The second bounce lands on the copy the pixel reads.
        for pass in 0..6u32 {
            let cascade = 2 - (pass % 3) as usize;
            let groups_x = self.light_cols[cascade].max(1);
            let groups_y = self.light_rows[cascade].max(1);
            self.dispatch_light_slice(cmd, self.light_sets[dst], pass, 0, groups_y, groups_x)?;
        }
        self.world_pending = 0;
        Ok(())
    }

    fn dispatch_light_slice(
        &self,
        cmd: Handle,
        set: Handle,
        pass: u32,
        row: u32,
        rows: u32,
        cols: u32,
    ) -> Result<(), String> {
        #[repr(C)]
        struct MemBar {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
        }
        unsafe {
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
            (self.fns.cmd_bind_pipe)(cmd, 1, self.compute_pipe);
            (self.fns.cmd_bind_set)(cmd, 1, self.compute_layout, 0, 1, &set, 0, std::ptr::null());
            // One band of probe rows. Later slices cover the rest of this pass, then the next pass.
            #[repr(C)]
            struct Push {
                pass: u32,
                y0: u32,
            }
            let push = Push { pass, y0: row * 8 };
            (self.fns.cmd_push)(
                cmd,
                self.compute_layout,
                0x20,
                0,
                8,
                &push as *const Push as *const c_void,
            );
            (self.fns.cmd_dispatch)(cmd, cols, rows, 1);
            let shade = MemBar {
                s_type: 46,
                next: std::ptr::null(),
                src_access: 0x40,
                dst_access: 0x20,
            };
            (self.fns.cmd_barrier)(
                cmd,
                0x800,
                0x800,
                0,
                1,
                &shade as *const MemBar as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
        }
        Ok(())
    }

    fn make_layout(&mut self) -> Result<Handle, String> {
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
            sets: *const c_void,
            push_count: u32,
            push: *const Range,
        }
        let range = Range {
            stage: 1,
            offset: 0,
            size: 64,
        };
        let sets = [self.desc_layout];
        let info = Info {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: sets.as_ptr() as *const c_void,
            push_count: 1,
            push: &range,
        };
        let mut layout = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_layout)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut layout,
                ),
                "layout",
            )?;
        }
        Ok(layout)
    }

    fn make_render_pass(&mut self, keep: bool) -> Result<Handle, String> {
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
        let color_load = if keep { 0 } else { 1 };
        let color_initial = if keep { 6 } else { 0 };
        let depth_load = if keep { 2 } else { 1 };
        // Final layout cannot be undefined. The overlay pass discards depth.
        let depth_final = 3;
        let in_stage = if keep { 0x1000 } else { 0x400 };
        // The filter leaves the color image visible to a transfer read.
        let in_access = if keep { 0x800 } else { 0 };
        let in_dst = if keep { 0x180 } else { 0x100 };
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
                final_layout: 6,
            },
            Att {
                flags: 0,
                format: 126,
                samples: 1,
                load: depth_load,
                store: 1,
                stencil_load: 1,
                stencil_store: 1,
                initial: 0,
                final_layout: depth_final,
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
                src_stage: in_stage,
                dst_stage: 0x400,
                src_access: in_access,
                dst_access: in_dst,
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
                "render pass",
            )?;
        }
        Ok(pass)
    }

    fn make_pipeline(
        &mut self,
        depth_test: bool,
        blend_on: bool,
        additive: bool,
    ) -> Result<Handle, String> {
        unsafe {
            let vert = self.shader(if additive { WIRE_VERT_SPV } else { VERT_SPV })?;
            let frag = self.shader(if additive { WIRE_FRAG_SPV } else { FRAG_SPV })?;
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
            struct Bind {
                binding: u32,
                stride: u32,
                rate: u32,
            }
            #[repr(C)]
            struct Attr {
                location: u32,
                binding: u32,
                format: i32,
                offset: u32,
            }
            #[repr(C)]
            struct VertInfo {
                s_type: i32,
                next: *const c_void,
                flags: u32,
                bind_count: u32,
                binds: *const Bind,
                attr_count: u32,
                attrs: *const Attr,
            }
            let bind = Bind {
                binding: 0,
                stride: 48,
                rate: 0,
            };
            let attrs = [
                Attr {
                    location: 0,
                    binding: 0,
                    format: 106,
                    offset: 0,
                },
                Attr {
                    location: 1,
                    binding: 0,
                    format: 106,
                    offset: 12,
                },
                Attr {
                    location: 2,
                    binding: 0,
                    format: 106,
                    offset: 24,
                },
                Attr {
                    location: 3,
                    binding: 0,
                    format: 100,
                    offset: 36,
                },
                Attr {
                    location: 4,
                    binding: 0,
                    format: 103,
                    offset: 40,
                },
            ];
            let vert_info = VertInfo {
                s_type: 19,
                next: std::ptr::null(),
                flags: 0,
                bind_count: 1,
                binds: &bind,
                attr_count: 5,
                attrs: attrs.as_ptr(),
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
            let vp = ViewportState {
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
            let depth_on = if depth_test { 1 } else { 0 };
            let depth = Depth {
                s_type: 25,
                next: std::ptr::null(),
                flags: 0,
                test: depth_on,
                write: depth_on,
                compare: 1,
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
            let batt = if additive {
                // Additive. A second copy of the same edge stays brighter than one copy.
                BlendAtt {
                    enable: 1,
                    src: 1,
                    dst: 1,
                    op: 0,
                    src_a: 1,
                    dst_a: 1,
                    op_a: 0,
                    mask: 0xf,
                }
            } else if blend_on {
                // Premultiplied. Opaque output uses alpha 1, so it replaces the target.
                // Fog output uses alpha = 1 - transmittance and rgb = in-scatter.
                BlendAtt {
                    enable: 1,
                    src: 1,
                    dst: 7,
                    op: 0,
                    src_a: 1,
                    dst_a: 7,
                    op_a: 0,
                    mask: 0xf,
                }
            } else {
                BlendAtt {
                    enable: 0,
                    src: 1,
                    dst: 0,
                    op: 0,
                    src_a: 1,
                    dst_a: 0,
                    op_a: 0,
                    mask: 0xf,
                }
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
            let dyn_state = Dyn {
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
                viewport: &vp,
                raster: &raster,
                ms: &ms,
                depth: &depth,
                blend: &blend,
                dynamic: &dyn_state,
                layout: self.layout,
                pass: self.render_pass,
                subpass: 0,
                base: std::ptr::null_mut(),
                base_index: -1,
            };
            let mut pipeline = std::ptr::null_mut();
            let result = (self.fns.create_pipelines)(
                self.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut pipeline,
            );
            (self.fns.destroy_shader)(self.device, vert, std::ptr::null());
            (self.fns.destroy_shader)(self.device, frag, std::ptr::null());
            check(result, "pipeline")?;
            Ok(pipeline)
        }
    }

    fn shader(&self, code: &[u8]) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: usize,
            code: *const u32,
        }
        let info = Info {
            s_type: 16,
            next: std::ptr::null(),
            flags: 0,
            size: code.len(),
            code: code.as_ptr() as *const u32,
        };
        let mut module = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_shader)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut module,
                ),
                "shader",
            )?;
        }
        Ok(module)
    }

    fn make_image(
        &self,
        format: i32,
        usage: u32,
        aspect: u32,
        width: u32,
        height: u32,
    ) -> Result<Image, String> {
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
            image_type: 1,
            format,
            extent: Extent {
                w: width,
                h: height,
                d: 1,
            },
            mips: 1,
            layers: 1,
            samples: 1,
            tiling: 0,
            usage,
            sharing: 0,
            queue_count: 0,
            queues: std::ptr::null(),
            initial: 0,
        };
        let mut image = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_image)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut image,
                ),
                "image",
            )?;
            let mut req = [0u8; 24];
            (self.fns.image_reqs)(self.device, image, req.as_mut_ptr());
            let size = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            let memory = self.alloc(size, bits, false)?;
            check(
                (self.fns.bind_image)(self.device, image, memory, 0),
                "bind image",
            )?;
            let view = self.make_view(image, format, aspect)?;
            Ok(Image {
                image,
                view,
                memory,
            })
        }
    }

    fn make_view(&self, image: Handle, format: i32, aspect: u32) -> Result<Handle, String> {
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
            view_type: 1,
            format,
            comp: Comp {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            range: Range {
                aspect,
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
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut view,
                ),
                "image view",
            )?;
        }
        Ok(view)
    }

    fn make_framebuffer(&self) -> Result<Handle, String> {
        let views = [self.color.view, self.depth.view];
        self.make_framebuffer_for(self.render_pass, &views, self.extent_w, self.extent_h)
    }

    fn make_framebuffer_for(
        &self,
        pass: Handle,
        views: &[Handle],
        width: u32,
        height: u32,
    ) -> Result<Handle, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            pass: Handle,
            count: u32,
            views: *const Handle,
            width: u32,
            height: u32,
            layers: u32,
        }
        let info = Info {
            s_type: 37,
            next: std::ptr::null(),
            flags: 0,
            pass,
            count: views.len() as u32,
            views: views.as_ptr(),
            width,
            height,
            layers: 1,
        };
        let mut fb = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_framebuffer)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut fb,
                ),
                "framebuffer",
            )?;
        }
        Ok(fb)
    }

    fn make_buffer(&self, size: u64, usage: u32, host: bool) -> Result<Buffer, String> {
        self.make_buffer_queues(size, usage, host, &[])
    }

    fn make_buffer_queues(
        &self,
        size: u64,
        usage: u32,
        host: bool,
        families: &[u32],
    ) -> Result<Buffer, String> {
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: u64,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
        }
        let shared = families.len() > 1;
        let info = Info {
            s_type: 12,
            next: std::ptr::null(),
            flags: 0,
            size,
            usage,
            sharing: u32::from(shared),
            queue_count: if shared { families.len() as u32 } else { 0 },
            queues: if shared {
                families.as_ptr()
            } else {
                std::ptr::null()
            },
        };
        let mut buffer = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_buffer)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut buffer,
                ),
                "buffer",
            )?;
            let mut req = [0u8; 24];
            (self.fns.buffer_reqs)(self.device, buffer, req.as_mut_ptr());
            let req_size = u64::from_ne_bytes(req[0..8].try_into().unwrap());
            let bits = u32::from_ne_bytes(req[16..20].try_into().unwrap());
            let memory = self.alloc(req_size, bits, host)?;
            check(
                (self.fns.bind_buffer)(self.device, buffer, memory, 0),
                "bind buffer",
            )?;
            Ok(Buffer {
                buffer,
                memory,
                size: req_size,
            })
        }
    }

    fn note_vertex_count(&mut self, count: u32) {
        self.vertex_count = count;
    }

    fn note_overlay_count(&mut self, count: u32) {
        self.overlay_count = count;
    }

    fn alloc(&self, size: u64, type_bits: u32, host: bool) -> Result<Handle, String> {
        let want = if host { 2 | 4 } else { 1 };
        let mut index = None;
        for i in 0..self.memory_props.count {
            if type_bits & (1 << i) != 0 && self.memory_props.types[i as usize] & want == want {
                index = Some(i);
                break;
            }
        }
        if index.is_none() && !host {
            for i in 0..self.memory_props.count {
                if type_bits & (1 << i) != 0 {
                    index = Some(i);
                    break;
                }
            }
        }
        let Some(index) = index else {
            return Err("no memory type".into());
        };
        #[repr(C)]
        struct Info {
            s_type: i32,
            next: *const c_void,
            size: u64,
            index: u32,
        }
        let info = Info {
            s_type: 5,
            next: std::ptr::null(),
            size,
            index,
        };
        let mut memory = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.alloc_mem)(
                    self.device,
                    &info as *const Info as *const u8,
                    std::ptr::null(),
                    &mut memory,
                ),
                "allocate",
            )?;
        }
        Ok(memory)
    }

    fn image_barrier(
        &self,
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

    fn copy_color_to_swapchain(&self, swap: Handle) {
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
            aspect: 1,
            mip: 0,
            layer: 0,
            count: 1,
        };
        let region = Region {
            src_layers: layers,
            src_offset: Offset { x: 0, y: 0, z: 0 },
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
                self.color.image,
                6,
                swap,
                7,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn copy_color_to_buffer(&self) {
        #[repr(C)]
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
            extent: Extent {
                w: self.extent_w,
                h: self.extent_h,
                d: 1,
            },
        };
        unsafe {
            (self.fns.cmd_copy_to_buffer)(
                self.cmd,
                self.color.image,
                6,
                self.host.buffer,
                1,
                &region as *const Region as *const u8,
            );
        }
    }

    fn destroy_targets(&mut self) {
        unsafe {
            if self.framebuffers[0].is_null() && self.framebuffer.is_null() {
                self.destroy_aa_targets();
                return;
            }
            self.destroy_aa_targets();
            for slot in 0..2 {
                if !self.framebuffers[slot].is_null() {
                    (self.fns.destroy_framebuffer)(
                        self.device,
                        self.framebuffers[slot],
                        std::ptr::null(),
                    );
                    self.framebuffers[slot] = std::ptr::null_mut();
                }
                let mut color = std::mem::replace(&mut self.colors[slot], Image::empty());
                let mut depth = std::mem::replace(&mut self.depths[slot], Image::empty());
                let mut host = std::mem::replace(&mut self.hosts[slot], Buffer::empty());
                self.destroy_image(&mut color);
                self.destroy_image(&mut depth);
                self.destroy_buffer(&mut host);
            }
            self.framebuffer = std::ptr::null_mut();
            self.color = Image::empty();
            self.depth = Image::empty();
            self.host = Buffer::empty();
            if !self.swapchain.is_null() {
                (self.fns.destroy_swapchain)(self.device, self.swapchain, std::ptr::null());
                self.swapchain = std::ptr::null_mut();
            }
            self.swap_images.clear();
        }
    }

    fn destroy_image(&self, image: &mut Image) {
        unsafe {
            if !image.view.is_null() {
                (self.fns.destroy_image_view)(self.device, image.view, std::ptr::null());
            }
            if !image.image.is_null() {
                (self.fns.destroy_image)(self.device, image.image, std::ptr::null());
            }
            if !image.memory.is_null() {
                (self.fns.free_mem)(self.device, image.memory, std::ptr::null());
            }
        }
        *image = Image::empty();
    }

    fn destroy_buffer(&self, buffer: &mut Buffer) {
        unsafe {
            if !buffer.buffer.is_null() {
                (self.fns.destroy_buffer)(self.device, buffer.buffer, std::ptr::null());
            }
            if !buffer.memory.is_null() {
                (self.fns.free_mem)(self.device, buffer.memory, std::ptr::null());
            }
        }
        *buffer = Buffer::empty();
    }
}

fn copy_image(image: &Image) -> Image {
    Image {
        image: image.image,
        view: image.view,
        memory: image.memory,
    }
}

fn ray_bytes(listener: [f32; 3], sources: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 + 16 + 256 * 16);
    for value in [listener[0], listener[1], listener[2], 0.0] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for value in [sources.len() as u32, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for index in 0..256 {
        let source = sources.get(index).copied().unwrap_or([0.0; 3]);
        for value in [source[0], source[1], source[2], 0.0] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

fn read_f32s(
    map: FnMap,
    unmap: FnUnmap,
    device: Handle,
    memory: Handle,
    count: usize,
) -> Result<Vec<f32>, String> {
    let bytes = count * 4;
    let mut raw = vec![0u8; bytes];
    unsafe {
        let mut mapped = std::ptr::null_mut();
        check(
            map(device, memory, 0, bytes as u64, 0, &mut mapped),
            "read audio gains",
        )?;
        std::ptr::copy_nonoverlapping(mapped as *const u8, raw.as_mut_ptr(), bytes);
        unmap(device, memory);
    }
    Ok(raw
        .chunks_exact(4)
        .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn copy_buffer(buffer: &Buffer) -> Buffer {
    Buffer {
        buffer: buffer.buffer,
        memory: buffer.memory,
        size: buffer.size,
    }
}

impl Image {
    fn empty() -> Self {
        Self {
            image: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
            memory: std::ptr::null_mut(),
        }
    }
}

impl Buffer {
    fn empty() -> Self {
        Self {
            buffer: std::ptr::null_mut(),
            memory: std::ptr::null_mut(),
            size: 0,
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            if self.device.is_null() {
                return;
            }
            (self.fns.device_wait)(self.device);
            self.destroy_targets();
            self.vertex = Buffer::empty();
            let mut vertices =
                std::mem::replace(&mut self.vertices, [Buffer::empty(), Buffer::empty()]);
            let mut scene_buf = std::mem::replace(&mut self.scene_buf, Buffer::empty());
            let mut light_scene =
                std::mem::replace(&mut self.light_scene, [Buffer::empty(), Buffer::empty()]);
            let mut light_field =
                std::mem::replace(&mut self.light_field, [Buffer::empty(), Buffer::empty()]);
            let mut particle_buf = std::mem::replace(&mut self.particle_buf, Buffer::empty());
            let mut audio_rays = std::mem::replace(&mut self.audio_rays, Buffer::empty());
            let mut audio_gains = std::mem::replace(&mut self.audio_gains, Buffer::empty());
            self.destroy_buffer(&mut vertices[0]);
            self.destroy_buffer(&mut vertices[1]);
            let mut shapes = std::mem::replace(&mut self.shapes, Buffer::empty());
            let mut instances = std::mem::replace(&mut self.instances, Buffer::empty());
            self.destroy_buffer(&mut shapes);
            self.destroy_buffer(&mut instances);
            self.destroy_buffer(&mut scene_buf);
            self.destroy_buffer(&mut light_scene[0]);
            self.destroy_buffer(&mut light_scene[1]);
            self.destroy_buffer(&mut light_field[0]);
            self.destroy_buffer(&mut light_field[1]);
            self.destroy_buffer(&mut particle_buf);
            if !self.light_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.light_fence, std::ptr::null());
            }
            if !self.light_publish_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.light_publish_fence, std::ptr::null());
            }
            if !self.light_sem.is_null() {
                (self.fns.destroy_sem)(self.device, self.light_sem, std::ptr::null());
            }
            if !self.light_pool.is_null() {
                (self.fns.destroy_pool)(self.device, self.light_pool, std::ptr::null());
            }
            self.destroy_buffer(&mut audio_rays);
            self.destroy_buffer(&mut audio_gains);
            if !self.audio_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.audio_fence, std::ptr::null());
            }
            if !self.audio_pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.audio_pipe, std::ptr::null());
            }
            if !self.audio_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.audio_layout, std::ptr::null());
            }
            if !self.audio_desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.audio_desc_pool, std::ptr::null());
            }
            if !self.audio_desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(
                    self.device,
                    self.audio_desc_layout,
                    std::ptr::null(),
                );
            }
            for slot in 0..2 {
                if !self.fences[slot].is_null() {
                    (self.fns.destroy_fence)(self.device, self.fences[slot], std::ptr::null());
                }
                if !self.image_readies[slot].is_null() {
                    (self.fns.destroy_sem)(self.device, self.image_readies[slot], std::ptr::null());
                }
                if !self.render_dones[slot].is_null() {
                    (self.fns.destroy_sem)(self.device, self.render_dones[slot], std::ptr::null());
                }
            }
            if !self.compute_pipe.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.compute_pipe, std::ptr::null());
            }
            if !self.compute_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.compute_layout, std::ptr::null());
            }
            if !self.desc_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.desc_pool, std::ptr::null());
            }
            if !self.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.desc_layout, std::ptr::null());
            }
            if !self.pool.is_null() {
                (self.fns.destroy_pool)(self.device, self.pool, std::ptr::null());
            }
            if !self.query_pool.is_null() {
                (self.fns.destroy_query_pool)(self.device, self.query_pool, std::ptr::null());
            }
            if !self.wire_pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.wire_pipeline, std::ptr::null());
            }
            if !self.overlay_pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.overlay_pipeline, std::ptr::null());
            }
            if !self.pipeline.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.pipeline, std::ptr::null());
            }
            if !self.layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.layout, std::ptr::null());
            }
            if !self.keep_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.keep_pass, std::ptr::null());
            }
            if !self.render_pass.is_null() {
                (self.fns.destroy_render_pass)(self.device, self.render_pass, std::ptr::null());
            }
            if !self.aa_fxaa.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.aa_fxaa, std::ptr::null());
            }
            if !self.aa_ssaa.is_null() {
                (self.fns.destroy_pipeline)(self.device, self.aa_ssaa, std::ptr::null());
            }
            if !self.aa_layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.aa_layout, std::ptr::null());
            }
            if !self.aa_pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.aa_pool, std::ptr::null());
            }
            if !self.aa_desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.aa_desc_layout, std::ptr::null());
            }
            (self.fns.destroy_device)(self.device, std::ptr::null());
            if !self.surface.is_null() {
                (self.fns.destroy_surface)(self.instance, self.surface, std::ptr::null());
            }
            (self.fns.destroy_instance)(self.instance, std::ptr::null());
        }
    }
}

fn check(result: VkResult, what: &str) -> Result<(), String> {
    if result == VK_SUCCESS {
        Ok(())
    } else {
        Err(format!("{what} failed: {result}"))
    }
}

unsafe fn transmute<T>(ptr: Pfn) -> T {
    if ptr.is_null() {
        panic!("missing Vulkan entry point");
    }
    std::mem::transmute_copy(&ptr)
}

fn parse_mem_props(raw: &[u8]) -> MemProps {
    let count = u32::from_ne_bytes(raw[0..4].try_into().unwrap()).min(32);
    let mut types = [0u32; 32];
    for i in 0..count as usize {
        let offset = 4 + i * 8;
        types[i] = u32::from_ne_bytes(raw[offset..offset + 4].try_into().unwrap());
    }
    MemProps { count, types }
}

fn load_fns(
    dload: impl Fn(&[u8]) -> Pfn,
    create_wayland_surface: FnWayland,
    destroy_surface: Fn2,
    surface_support: FnSupport,
    surface_caps: FnCaps,
    surface_formats: FnCount,
    surface_modes: FnCount,
    destroy_instance: FnDestroy,
    enumerate_devices: FnEnumDev,
    queue_families: FnQueues,
    mem_props: FnMemProps,
) -> Fns {
    unsafe {
        macro_rules! d {
            ($name:literal) => {
                transmute(dload(concat!($name, "\0").as_bytes()))
            };
        }
        Fns {
            destroy_instance,
            destroy_device: d!("vkDestroyDevice"),
            create_wayland_surface,
            destroy_surface,
            surface_support,
            surface_caps,
            surface_formats,
            surface_modes,
            create_swapchain: d!("vkCreateSwapchainKHR"),
            destroy_swapchain: d!("vkDestroySwapchainKHR"),
            swapchain_images: d!("vkGetSwapchainImagesKHR"),
            acquire: d!("vkAcquireNextImageKHR"),
            present: d!("vkQueuePresentKHR"),
            create_image_view: d!("vkCreateImageView"),
            destroy_image_view: d!("vkDestroyImageView"),
            create_shader: d!("vkCreateShaderModule"),
            destroy_shader: d!("vkDestroyShaderModule"),
            create_layout: d!("vkCreatePipelineLayout"),
            destroy_layout: d!("vkDestroyPipelineLayout"),
            create_render_pass: d!("vkCreateRenderPass"),
            destroy_render_pass: d!("vkDestroyRenderPass"),
            create_pipelines: d!("vkCreateGraphicsPipelines"),
            destroy_pipeline: d!("vkDestroyPipeline"),
            create_framebuffer: d!("vkCreateFramebuffer"),
            destroy_framebuffer: d!("vkDestroyFramebuffer"),
            create_pool: d!("vkCreateCommandPool"),
            destroy_pool: d!("vkDestroyCommandPool"),
            alloc_cmd: d!("vkAllocateCommandBuffers"),
            begin_cmd: d!("vkBeginCommandBuffer"),
            end_cmd: d!("vkEndCommandBuffer"),
            reset_cmd: d!("vkResetCommandBuffer"),
            cmd_begin_rp: d!("vkCmdBeginRenderPass"),
            cmd_end_rp: d!("vkCmdEndRenderPass"),
            cmd_bind_pipe: d!("vkCmdBindPipeline"),
            cmd_bind_vb: d!("vkCmdBindVertexBuffers"),
            cmd_draw: d!("vkCmdDraw"),
            cmd_push: d!("vkCmdPushConstants"),
            cmd_viewport: d!("vkCmdSetViewport"),
            cmd_scissor: d!("vkCmdSetScissor"),
            cmd_barrier: d!("vkCmdPipelineBarrier"),
            cmd_copy_image: d!("vkCmdCopyImage"),
            cmd_copy_to_buffer: d!("vkCmdCopyImageToBuffer"),
            cmd_copy_to_image: d!("vkCmdCopyBufferToImage"),
            create_sem: d!("vkCreateSemaphore"),
            destroy_sem: d!("vkDestroySemaphore"),
            create_fence: d!("vkCreateFence"),
            destroy_fence: d!("vkDestroyFence"),
            wait_fences: d!("vkWaitForFences"),
            reset_fences: d!("vkResetFences"),
            queue_submit: d!("vkQueueSubmit"),
            fence_status: d!("vkGetFenceStatus"),
            cmd_bind_set: d!("vkCmdBindDescriptorSets"),
            cmd_dispatch: d!("vkCmdDispatch"),
            update_desc: d!("vkUpdateDescriptorSets"),
            create_desc_layout: d!("vkCreateDescriptorSetLayout"),
            destroy_desc_layout: d!("vkDestroyDescriptorSetLayout"),
            create_desc_pool: d!("vkCreateDescriptorPool"),
            destroy_desc_pool: d!("vkDestroyDescriptorPool"),
            alloc_desc: d!("vkAllocateDescriptorSets"),
            create_compute: d!("vkCreateComputePipelines"),
            device_wait: d!("vkDeviceWaitIdle"),
            create_buffer: d!("vkCreateBuffer"),
            destroy_buffer: d!("vkDestroyBuffer"),
            buffer_reqs: d!("vkGetBufferMemoryRequirements"),
            alloc_mem: d!("vkAllocateMemory"),
            free_mem: d!("vkFreeMemory"),
            bind_buffer: d!("vkBindBufferMemory"),
            map_mem: d!("vkMapMemory"),
            unmap_mem: d!("vkUnmapMemory"),
            create_image: d!("vkCreateImage"),
            destroy_image: d!("vkDestroyImage"),
            image_reqs: d!("vkGetImageMemoryRequirements"),
            bind_image: d!("vkBindImageMemory"),
            enumerate_devices,
            queue_families,
            mem_props,
            create_query_pool: d!("vkCreateQueryPool"),
            destroy_query_pool: d!("vkDestroyQueryPool"),
            cmd_reset_query: d!("vkCmdResetQueryPool"),
            cmd_write_timestamp: d!("vkCmdWriteTimestamp"),
            get_query_results: d!("vkGetQueryPoolResults"),
        }
    }
}

fn tick_delta(start: u64, end: u64, bits: u32) -> u64 {
    if bits == 0 || bits >= 64 {
        return end.wrapping_sub(start);
    }
    let mask = (1u64 << bits) - 1;
    end.wrapping_sub(start) & mask
}

include!("aa_gpu.rs");

extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> Pfn;
}
