//! Stage 2 of docs/systems/rendering/pbr-brdf-reference.md: dispatch
//! `pbr_check.comp` (which includes `pbr_brdf.glsl`) and compare `D`, `V`, `F`
//! and the full `f` over a grid of (n·l, n·v, roughness, metallic, albedo) with
//! `genos_math::brdf`. Gates: relative 1e-3 on `D`, `V`, `F`; 1e-2 on `f`.
//! A push constant that drops the dielectric `(1 − F)` weight must fail.
//!
//! Runs on the first non-llvmpipe GPU (the 4070). Set
//! `GENOS_PBR_PARITY_DEVICE=<name substring>` to pick another, e.g. `llvmpipe`.

#![expect(
    clippy::as_conversions,
    reason = "Vulkan handles and byte sizes are pointer-width"
)]
#![expect(
    clippy::borrow_as_ptr,
    reason = "Vulkan entry points take raw pointers"
)]
#![expect(
    clippy::items_after_statements,
    reason = "Vulkan struct layouts sit next to the call that fills them"
)]
#![expect(clippy::panic, reason = "a failed fixture aborts the test")]
#![expect(clippy::ptr_as_ptr, reason = "Vulkan create-info pointers")]
#![expect(clippy::too_many_lines, reason = "one headless compute dispatch")]
#![expect(
    clippy::undocumented_unsafe_blocks,
    reason = "Vulkan and dlopen FFI, same as gpu.rs"
)]
#![expect(clippy::unwrap_used, reason = "a failed fixture aborts the test")]
#![allow(
    clippy::disallowed_methods,
    reason = "the test reads its device filter from the environment"
)]

use std::ffi::{c_char, c_void};

use genos_math::brdf::{self, Cosines, Material, DIELECTRIC_F0};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pbr_check.comp.spv"));
// Grid. Same literals as pbr_check.comp.
const COSINES: [f32; 5] = [0.05, 0.25, 0.5, 0.75, 1.0];
const ROUGHNESS: [f64; 5] = [0.1, 0.3, 0.5, 0.75, 1.0];
const METALLIC: [f64; 3] = [0.0, 0.5, 1.0];
const ALBEDO: [[f64; 3]; 3] = [[1.0, 1.0, 1.0], [0.5, 0.25, 0.1], [0.04, 0.8, 0.3]];
const ROWS: usize = 5 * 5 * 5 * 3 * 3;
const STRIDE: usize = 6;
const WORDS: usize = ROWS * STRIDE;
/// Relative gate on `D`, `V`, `F` (doc stage 2).
const TERM_LIMIT: f64 = 1.0e-3;
/// Relative gate on the full `f` (doc stage 2).
const F_LIMIT: f64 = 1.0e-2;
/// Below this the comparison is absolute, so a zero reference is not a divide.
const FLOOR: f64 = 1.0e-6;

fn device_wanted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match std::env::var("GENOS_PBR_PARITY_DEVICE") {
        Ok(want) if !want.is_empty() => lower.contains(&want.to_ascii_lowercase()),
        _ => !lower.contains("llvmpipe"),
    }
}

#[allow(clippy::many_single_char_names)] // n, l, v, f are the textbook names.
/// The reference at the shader's f32 inputs: same plane, same half vector.
fn cpu_words() -> Vec<f64> {
    let mut out = Vec::with_capacity(WORDS);
    for nl in COSINES {
        for nv in COSINES {
            let nl = f64::from(nl);
            let nv = f64::from(nv);
            let l = [(1.0 - nl * nl).max(0.0).sqrt(), 0.0, nl];
            let v = [-(1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv];
            let c = Cosines::from_dirs([0.0, 0.0, 1.0], l, v);
            for rough in ROUGHNESS {
                for metal in METALLIC {
                    for base in ALBEDO {
                        let m = Material {
                            metallic: metal,
                            ..Material::metal(base, rough)
                        };
                        let alpha = m.alpha();
                        let f = brdf::eval(&m, c);
                        out.push(brdf::d_ggx(c.n_dot_h, alpha));
                        out.push(brdf::v_smith_ggx_correlated(c.n_dot_l, c.n_dot_v, alpha));
                        out.push(brdf::fresnel_schlick(DIELECTRIC_F0, c.v_dot_h));
                        out.extend(f);
                    }
                }
            }
        }
    }
    out
}

fn rel(gpu: f32, cpu: f64) -> f64 {
    (f64::from(gpu) - cpu).abs() / cpu.abs().max(FLOOR)
}

/// Worst relative error over the slots `pick` selects, and the row it sits in.
fn worst(gpu: &[f32], cpu: &[f64], pick: impl Fn(usize) -> bool) -> (f64, usize) {
    let mut out = (0.0, 0);
    for (i, (g, c)) in gpu.iter().zip(cpu).enumerate() {
        if pick(i % STRIDE) {
            let e = rel(*g, *c);
            if e > out.0 || e.is_nan() {
                out = (e, i / STRIDE);
            }
        }
    }
    out
}

type Handle = *mut c_void;
type VkResult = i32;

#[repr(C)]
struct StageLayout {
    s_type: i32,
    next: *const c_void,
    flags: u32,
    stage: u32,
    module: Handle,
    name: *const c_char,
    spec: *const c_void,
}

const _: () = assert!(std::mem::size_of::<StageLayout>() == 48);
const _: () = assert!(std::mem::offset_of!(StageLayout, stage) == 20);
const _: () = assert!(std::mem::offset_of!(StageLayout, module) == 24);

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

fn vk_check(result: VkResult, what: &str) -> Result<(), String> {
    if result == 0 {
        Ok(())
    } else {
        Err(format!("{what} returned {result}"))
    }
}

fn device_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn dispatch(broken: u32) -> Result<(String, Vec<f32>), String> {
    unsafe {
        let lib = dlopen(c"libvulkan.so.1".as_ptr(), 2);
        if lib.is_null() {
            return Err("libvulkan.so.1 did not open".into());
        }
        let get_instance: unsafe extern "system" fn(Handle, *const c_char) -> *const c_void =
            std::mem::transmute(dlsym(lib, c"vkGetInstanceProcAddr".as_ptr()));
        let create_instance: unsafe extern "system" fn(
            *const u8,
            *const c_void,
            *mut Handle,
        ) -> VkResult = std::mem::transmute(get_instance(
            std::ptr::null_mut(),
            c"vkCreateInstance".as_ptr(),
        ));
        let app_name = c"genos-pbr-parity";
        let app = AppInfo {
            s_type: 0,
            next: std::ptr::null(),
            name: app_name.as_ptr(),
            app_version: 1,
            engine: app_name.as_ptr(),
            engine_version: 1,
            api: 1 << 22,
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
            enabled_exts: 0,
            ext_names: std::ptr::null(),
        };
        let mut instance = std::ptr::null_mut();
        vk_check(
            create_instance(
                &raw const info as *const u8,
                std::ptr::null(),
                &mut instance,
            ),
            "vkCreateInstance",
        )?;
        let load = |name: &std::ffi::CStr| get_instance(instance, name.as_ptr());
        type EnumDev = unsafe extern "system" fn(Handle, *mut u32, *mut Handle) -> VkResult;
        type Props = unsafe extern "system" fn(Handle, *mut u8);
        type Queues = unsafe extern "system" fn(Handle, *mut u32, *mut u8);
        type MemProps = unsafe extern "system" fn(Handle, *mut u8);
        type CreateDevice =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type GetDev = unsafe extern "system" fn(Handle, *const c_char) -> *const c_void;
        type Destroy = unsafe extern "system" fn(Handle, *const c_void);
        let enumerate: EnumDev = std::mem::transmute(load(c"vkEnumeratePhysicalDevices"));
        let get_props: Props = std::mem::transmute(load(c"vkGetPhysicalDeviceProperties"));
        let queue_families: Queues =
            std::mem::transmute(load(c"vkGetPhysicalDeviceQueueFamilyProperties"));
        let mem_props: MemProps = std::mem::transmute(load(c"vkGetPhysicalDeviceMemoryProperties"));
        let create_device: CreateDevice = std::mem::transmute(load(c"vkCreateDevice"));
        let get_device: GetDev = std::mem::transmute(load(c"vkGetDeviceProcAddr"));
        let destroy_instance: Destroy = std::mem::transmute(load(c"vkDestroyInstance"));

        let mut dev_count = 0u32;
        vk_check(
            enumerate(instance, &mut dev_count, std::ptr::null_mut()),
            "devices",
        )?;
        let mut devices = vec![std::ptr::null_mut(); dev_count as usize];
        vk_check(
            enumerate(instance, &mut dev_count, devices.as_mut_ptr()),
            "device list",
        )?;
        let mut chosen: Option<(Handle, u32, String)> = None;
        for physical in devices {
            let mut prop = [0u8; 1024];
            get_props(physical, prop.as_mut_ptr());
            let name = device_name(&prop[20..276]);
            if !device_wanted(&name) {
                continue;
            }
            let mut family_count = 0u32;
            queue_families(physical, &mut family_count, std::ptr::null_mut());
            let mut families = vec![0u8; family_count as usize * 24];
            queue_families(physical, &mut family_count, families.as_mut_ptr());
            for index in 0..family_count {
                let base = index as usize * 24;
                let flags = u32::from_ne_bytes(families[base..base + 4].try_into().unwrap());
                if flags & 0x2 != 0 {
                    chosen = Some((physical, index, name));
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let Some((physical, family, name)) = chosen else {
            destroy_instance(instance, std::ptr::null());
            return Err("no lavapipe device with a compute queue".into());
        };

        let mut raw_props = [0u8; 4 + 32 * 8 + 4 + 16 * 16];
        mem_props(physical, raw_props.as_mut_ptr());
        let mem_count = u32::from_ne_bytes(raw_props[0..4].try_into().unwrap()).min(32);
        let priorities = [1.0f32];
        #[repr(C)]
        struct QueueInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            family: u32,
            count: u32,
            priorities: *const f32,
        }
        let queue_info = QueueInfo {
            s_type: 2,
            next: std::ptr::null(),
            flags: 0,
            family,
            count: 1,
            priorities: priorities.as_ptr(),
        };
        #[repr(C)]
        struct DevInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            queue_count: u32,
            queues: *const QueueInfo,
            layer_count: u32,
            layers: *const *const c_char,
            ext_count: u32,
            exts: *const *const c_char,
            features: *const u8,
        }
        let dev_info = DevInfo {
            s_type: 3,
            next: std::ptr::null(),
            flags: 0,
            queue_count: 1,
            queues: &queue_info,
            layer_count: 0,
            layers: std::ptr::null(),
            ext_count: 0,
            exts: std::ptr::null(),
            features: std::ptr::null(),
        };
        let mut device = std::ptr::null_mut();
        vk_check(
            create_device(
                physical,
                &raw const dev_info as *const u8,
                std::ptr::null(),
                &mut device,
            ),
            "vkCreateDevice",
        )?;
        let dload = |entry: &std::ffi::CStr| get_device(device, entry.as_ptr());
        type FnDestroyDev = unsafe extern "system" fn(Handle, *const c_void);
        type FnQueue = unsafe extern "system" fn(Handle, u32, u32, *mut Handle);
        type FnBuf =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type FnReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
        type FnAlloc =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type FnBind = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
        type FnMap =
            unsafe extern "system" fn(Handle, Handle, u64, u64, u32, *mut *mut c_void) -> VkResult;
        type FnUnmap = unsafe extern "system" fn(Handle, Handle);
        type FnLayout =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type FnShader =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type FnPool =
            unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
        type FnAllocCmd = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
        type FnBegin = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
        type FnEnd = unsafe extern "system" fn(Handle) -> VkResult;
        type FnBindPipe = unsafe extern "system" fn(Handle, u32, Handle);
        type FnBindSet = unsafe extern "system" fn(
            Handle,
            u32,
            Handle,
            u32,
            u32,
            *const Handle,
            u32,
            *const u32,
        );
        type FnPush = unsafe extern "system" fn(Handle, Handle, u32, u32, u32, *const c_void);
        type FnDispatch = unsafe extern "system" fn(Handle, u32, u32, u32);
        type FnBarrier = unsafe extern "system" fn(
            Handle,
            u32,
            u32,
            u32,
            u32,
            *const c_void,
            u32,
            *const u8,
            u32,
            *const c_void,
        );
        type FnSubmit = unsafe extern "system" fn(Handle, u32, *const u8, Handle) -> VkResult;
        type FnIdle = unsafe extern "system" fn(Handle) -> VkResult;
        type FnUpdate = unsafe extern "system" fn(Handle, u32, *const u8, u32, *const u8);
        type FnFreeMem = unsafe extern "system" fn(Handle, Handle, *const c_void);
        type FnDestroyBuf = unsafe extern "system" fn(Handle, Handle, *const c_void);

        let destroy_device: FnDestroyDev = std::mem::transmute(dload(c"vkDestroyDevice"));
        let get_queue: FnQueue = std::mem::transmute(dload(c"vkGetDeviceQueue"));
        let create_buffer: FnBuf = std::mem::transmute(dload(c"vkCreateBuffer"));
        let buffer_req: FnReq = std::mem::transmute(dload(c"vkGetBufferMemoryRequirements"));
        let alloc_mem: FnAlloc = std::mem::transmute(dload(c"vkAllocateMemory"));
        let bind_buf: FnBind = std::mem::transmute(dload(c"vkBindBufferMemory"));
        let map_mem: FnMap = std::mem::transmute(dload(c"vkMapMemory"));
        let unmap_mem: FnUnmap = std::mem::transmute(dload(c"vkUnmapMemory"));
        let free_mem: FnFreeMem = std::mem::transmute(dload(c"vkFreeMemory"));
        let destroy_buf: FnDestroyBuf = std::mem::transmute(dload(c"vkDestroyBuffer"));
        let desc_layout: FnLayout = std::mem::transmute(dload(c"vkCreateDescriptorSetLayout"));
        let destroy_layout: FnDestroyBuf =
            std::mem::transmute(dload(c"vkDestroyDescriptorSetLayout"));
        let pipe_layout: FnLayout = std::mem::transmute(dload(c"vkCreatePipelineLayout"));
        let destroy_pipe_layout: FnDestroyBuf =
            std::mem::transmute(dload(c"vkDestroyPipelineLayout"));
        let shader_mod: FnShader = std::mem::transmute(dload(c"vkCreateShaderModule"));
        let destroy_shader: FnDestroyBuf = std::mem::transmute(dload(c"vkDestroyShaderModule"));
        let create_pipes: unsafe extern "system" fn(
            Handle,
            Handle,
            u32,
            *const u8,
            *const c_void,
            *mut Handle,
        ) -> VkResult = std::mem::transmute(dload(c"vkCreateComputePipelines"));
        let destroy_pipe: FnDestroyBuf = std::mem::transmute(dload(c"vkDestroyPipeline"));
        let desc_pool: FnPool = std::mem::transmute(dload(c"vkCreateDescriptorPool"));
        let destroy_pool: FnDestroyBuf = std::mem::transmute(dload(c"vkDestroyDescriptorPool"));
        let alloc_desc: FnAllocCmd = std::mem::transmute(dload(c"vkAllocateDescriptorSets"));
        let update_desc: FnUpdate = std::mem::transmute(dload(c"vkUpdateDescriptorSets"));
        let cmd_pool: FnPool = std::mem::transmute(dload(c"vkCreateCommandPool"));
        let destroy_cmd_pool: FnDestroyBuf = std::mem::transmute(dload(c"vkDestroyCommandPool"));
        let alloc_cmd: FnAllocCmd = std::mem::transmute(dload(c"vkAllocateCommandBuffers"));
        let begin_cmd: FnBegin = std::mem::transmute(dload(c"vkBeginCommandBuffer"));
        let end_cmd: FnEnd = std::mem::transmute(dload(c"vkEndCommandBuffer"));
        let bind_pipe: FnBindPipe = std::mem::transmute(dload(c"vkCmdBindPipeline"));
        let bind_set: FnBindSet = std::mem::transmute(dload(c"vkCmdBindDescriptorSets"));
        let push: FnPush = std::mem::transmute(dload(c"vkCmdPushConstants"));
        let dispatch_cmd: FnDispatch = std::mem::transmute(dload(c"vkCmdDispatch"));
        let barrier: FnBarrier = std::mem::transmute(dload(c"vkCmdPipelineBarrier"));
        let submit: FnSubmit = std::mem::transmute(dload(c"vkQueueSubmit"));
        let idle: FnIdle = std::mem::transmute(dload(c"vkQueueWaitIdle"));
        let mut queue = std::ptr::null_mut();
        get_queue(device, family, 0, &mut queue);

        let bytes = (WORDS * 4) as u64;
        #[repr(C)]
        struct BufInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: u64,
            usage: u32,
            sharing: u32,
            queue_count: u32,
            queues: *const u32,
        }
        let buf_info = BufInfo {
            s_type: 12,
            next: std::ptr::null(),
            flags: 0,
            size: bytes,
            usage: 0x20,
            sharing: 0,
            queue_count: 0,
            queues: std::ptr::null(),
        };
        let mut buffer = std::ptr::null_mut();
        vk_check(
            create_buffer(
                device,
                &raw const buf_info as *const u8,
                std::ptr::null(),
                &mut buffer,
            ),
            "buffer",
        )?;
        let mut reqs = [0u8; 24];
        buffer_req(device, buffer, reqs.as_mut_ptr());
        let alloc_size = u64::from_ne_bytes(reqs[0..8].try_into().unwrap());
        let type_bits = u32::from_ne_bytes(reqs[16..20].try_into().unwrap());
        let mut mem_index = None;
        for index in 0..mem_count {
            let offset = 4 + index as usize * 8;
            let flags = u32::from_ne_bytes(raw_props[offset..offset + 4].try_into().unwrap());
            if type_bits & (1u32 << index) != 0 && flags & (2 | 4) == (2 | 4) {
                mem_index = Some(index);
                break;
            }
        }
        let Some(mem_index) = mem_index else {
            return Err("no host-visible coherent memory".into());
        };
        #[repr(C)]
        struct AllocInfo {
            s_type: i32,
            next: *const c_void,
            size: u64,
            index: u32,
        }
        let alloc_info = AllocInfo {
            s_type: 5,
            next: std::ptr::null(),
            size: alloc_size,
            index: mem_index,
        };
        let mut memory = std::ptr::null_mut();
        vk_check(
            alloc_mem(
                device,
                &raw const alloc_info as *const u8,
                std::ptr::null(),
                &mut memory,
            ),
            "allocate",
        )?;
        vk_check(bind_buf(device, buffer, memory, 0), "bind")?;
        let mut mapped = std::ptr::null_mut();
        vk_check(
            map_mem(device, memory, 0, alloc_size, 0, &mut mapped),
            "map",
        )?;
        let bytes = usize::try_from(alloc_size).unwrap();
        std::ptr::write_bytes(mapped, 0, bytes);

        #[repr(C)]
        struct DescBinding {
            slot: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        let binding = DescBinding {
            slot: 0,
            kind: 7,
            count: 1,
            stages: 0x20,
            samplers: std::ptr::null(),
        };
        #[repr(C)]
        struct LayoutInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            count: u32,
            bindings: *const DescBinding,
        }
        let layout_info = LayoutInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: 1,
            bindings: &binding,
        };
        let mut set_layout = std::ptr::null_mut();
        vk_check(
            desc_layout(
                device,
                &raw const layout_info as *const u8,
                std::ptr::null(),
                &mut set_layout,
            ),
            "set layout",
        )?;
        #[repr(C)]
        struct PushRange {
            stages: u32,
            offset: u32,
            size: u32,
        }
        let push_range = PushRange {
            stages: 0x20,
            offset: 0,
            size: 4,
        };
        #[repr(C)]
        struct PipeLayoutInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            set_count: u32,
            sets: *const Handle,
            push_count: u32,
            pushes: *const PushRange,
        }
        let pipe_layout_info = PipeLayoutInfo {
            s_type: 30,
            next: std::ptr::null(),
            flags: 0,
            set_count: 1,
            sets: &set_layout,
            push_count: 1,
            pushes: &push_range,
        };
        let mut layout = std::ptr::null_mut();
        vk_check(
            pipe_layout(
                device,
                &raw const pipe_layout_info as *const u8,
                std::ptr::null(),
                &mut layout,
            ),
            "pipeline layout",
        )?;
        let (spirv_words, rest) = SPV.as_chunks::<4>();
        assert!(rest.is_empty(), "SPIR-V length is not a multiple of 4");
        let spirv: Vec<u32> = spirv_words
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        #[repr(C)]
        struct ShaderInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            size: usize,
            code: *const u32,
        }
        let shader_info = ShaderInfo {
            s_type: 16,
            next: std::ptr::null(),
            flags: 0,
            size: spirv.len() * 4,
            code: spirv.as_ptr(),
        };
        let mut module = std::ptr::null_mut();
        vk_check(
            shader_mod(
                device,
                &raw const shader_info as *const u8,
                std::ptr::null(),
                &mut module,
            ),
            "shader",
        )?;
        #[repr(C)]
        struct Stage {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            kind: u32,
            module: Handle,
            name: *const c_char,
            spec: *const c_void,
        }
        let stage = Stage {
            s_type: 18,
            next: std::ptr::null(),
            flags: 0,
            kind: 0x20,
            module,
            name: c"main".as_ptr(),
            spec: std::ptr::null(),
        };
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
        let pipe_info = PipeInfo {
            s_type: 29,
            next: std::ptr::null(),
            flags: 0,
            stage,
            layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        let mut pipeline = std::ptr::null_mut();
        vk_check(
            create_pipes(
                device,
                std::ptr::null_mut(),
                1,
                &raw const pipe_info as *const u8,
                std::ptr::null(),
                &mut pipeline,
            ),
            "pipeline",
        )?;

        #[repr(C)]
        struct PoolSize {
            kind: u32,
            count: u32,
        }
        let pool_size = PoolSize { kind: 7, count: 1 };
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            max_sets: u32,
            size_count: u32,
            sizes: *const PoolSize,
        }
        let pool_info = PoolInfo {
            s_type: 33,
            next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            size_count: 1,
            sizes: &pool_size,
        };
        let mut pool = std::ptr::null_mut();
        vk_check(
            desc_pool(
                device,
                &raw const pool_info as *const u8,
                std::ptr::null(),
                &mut pool,
            ),
            "descriptor pool",
        )?;
        #[repr(C)]
        struct AllocSets {
            s_type: i32,
            next: *const c_void,
            pool: Handle,
            count: u32,
            layouts: *const Handle,
        }
        let alloc_sets = AllocSets {
            s_type: 34,
            next: std::ptr::null(),
            pool,
            count: 1,
            layouts: &set_layout,
        };
        let mut set = std::ptr::null_mut();
        vk_check(
            alloc_desc(device, &raw const alloc_sets as *const u8, &mut set),
            "descriptor set",
        )?;
        #[repr(C)]
        struct BufDesc {
            buffer: Handle,
            offset: u64,
            range: u64,
        }
        let buf_desc = BufDesc {
            buffer,
            offset: 0,
            range: u64::MAX,
        };
        #[repr(C)]
        struct WriteSet {
            s_type: i32,
            next: *const c_void,
            set: Handle,
            binding: u32,
            element: u32,
            count: u32,
            kind: u32,
            image: *const c_void,
            buffer: *const BufDesc,
            texel: *const c_void,
        }
        let write = WriteSet {
            s_type: 35,
            next: std::ptr::null(),
            set,
            binding: 0,
            element: 0,
            count: 1,
            kind: 7,
            image: std::ptr::null(),
            buffer: &buf_desc,
            texel: std::ptr::null(),
        };
        update_desc(
            device,
            1,
            &raw const write as *const u8,
            0,
            std::ptr::null(),
        );

        #[repr(C)]
        struct CmdPoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            family: u32,
        }
        let cmd_pool_info = CmdPoolInfo {
            s_type: 39,
            next: std::ptr::null(),
            flags: 0,
            family,
        };
        let mut commands = std::ptr::null_mut();
        vk_check(
            cmd_pool(
                device,
                &raw const cmd_pool_info as *const u8,
                std::ptr::null(),
                &mut commands,
            ),
            "command pool",
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
            pool: commands,
            level: 0,
            count: 1,
        };
        let mut cmd = std::ptr::null_mut();
        vk_check(
            alloc_cmd(device, &raw const cmd_alloc as *const u8, &mut cmd),
            "command buffer",
        )?;
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
        vk_check(begin_cmd(cmd, &raw const begin as *const u8), "begin")?;
        bind_pipe(cmd, 1, pipeline);
        bind_set(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
        push(cmd, layout, 0x20, 0, 4, &raw const broken as *const c_void);
        dispatch_cmd(cmd, 1, 1, 1);
        #[repr(C)]
        struct BufBarrier {
            s_type: i32,
            next: *const c_void,
            src: u32,
            dst: u32,
            src_queue: u32,
            dst_queue: u32,
            buffer: Handle,
            offset: u64,
            size: u64,
        }
        let buf_barrier = BufBarrier {
            s_type: 44,
            next: std::ptr::null(),
            src: 0x40,
            dst: 0x2000,
            src_queue: u32::MAX,
            dst_queue: u32::MAX,
            buffer,
            offset: 0,
            size: u64::MAX,
        };
        barrier(
            cmd,
            0x800,
            0x4000,
            0,
            0,
            std::ptr::null(),
            1,
            &raw const buf_barrier as *const u8,
            0,
            std::ptr::null(),
        );
        vk_check(end_cmd(cmd), "end")?;
        #[repr(C)]
        struct SubmitInfo {
            s_type: i32,
            next: *const c_void,
            wait_count: u32,
            waits: *const Handle,
            wait_stages: *const u32,
            cmd_count: u32,
            cmds: *const Handle,
            signal_count: u32,
            signals: *const Handle,
        }
        let submit_info = SubmitInfo {
            s_type: 4,
            next: std::ptr::null(),
            wait_count: 0,
            waits: std::ptr::null(),
            wait_stages: std::ptr::null(),
            cmd_count: 1,
            cmds: &cmd,
            signal_count: 0,
            signals: std::ptr::null(),
        };
        vk_check(
            submit(
                queue,
                1,
                &raw const submit_info as *const u8,
                std::ptr::null_mut(),
            ),
            "submit",
        )?;
        vk_check(idle(queue), "wait idle")?;
        let slice = std::slice::from_raw_parts(mapped.cast::<f32>(), WORDS);
        let words = slice.to_vec();
        unmap_mem(device, memory);
        destroy_cmd_pool(device, commands, std::ptr::null());
        destroy_pool(device, pool, std::ptr::null());
        destroy_pipe(device, pipeline, std::ptr::null());
        destroy_shader(device, module, std::ptr::null());
        destroy_pipe_layout(device, layout, std::ptr::null());
        destroy_layout(device, set_layout, std::ptr::null());
        destroy_buf(device, buffer, std::ptr::null());
        free_mem(device, memory, std::ptr::null());
        destroy_device(device, std::ptr::null());
        destroy_instance(instance, std::ptr::null());
        Ok((name, words))
    }
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[test]
fn gpu_brdf_matches_the_cpu_reference_and_a_broken_shade_fails() {
    let cpu = cpu_words();
    assert_eq!(cpu.len(), WORDS);
    let (name, gpu) = dispatch(0).unwrap_or_else(|err| panic!("{err}"));
    for (slot, label) in [(0, "D"), (1, "V"), (2, "F")] {
        let (err, row) = worst(&gpu, &cpu, |s| s == slot);
        assert!(
            err <= TERM_LIMIT,
            "{label} relative error {err} > {TERM_LIMIT} at row {row} on {name}"
        );
    }
    let (f_err, f_row) = worst(&gpu, &cpu, |s| s >= 3);
    assert!(
        f_err <= F_LIMIT,
        "f relative error {f_err} > {F_LIMIT} at row {f_row} on {name}: gpu={:?} cpu={:?}",
        &gpu[f_row * STRIDE..f_row * STRIDE + STRIDE],
        &cpu[f_row * STRIDE..f_row * STRIDE + STRIDE]
    );
    println!("pbr parity on {name}: f worst {f_err:.3e} (gate {F_LIMIT})");

    let (_name, broken) = dispatch(1).unwrap_or_else(|err| panic!("{err}"));
    let (terms, _) = worst(&broken, &cpu, |s| s < 3);
    assert!(
        terms <= TERM_LIMIT,
        "the broken shade moved D, V or F by {terms}"
    );
    let (broken_f, _) = worst(&broken, &cpu, |s| s >= 3);
    assert!(
        broken_f > F_LIMIT,
        "dropping the (1 - F) diffuse weight still matched within {F_LIMIT} ({broken_f})"
    );
}
