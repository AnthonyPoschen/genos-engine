//! Dispatch `reflections_check.comp` on lavapipe and compare every optics output
//! with `genos_math::optics`. The limit is `1e-5`. A push constant that forgets
//! to invert the below-water index ratio must fail that limit.

#![expect(
    clippy::as_conversions,
    reason = "Vulkan handles and byte sizes are pointer-width"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "Vulkan struct fields are the API's u32"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "mirror view count is a small integer stored as f32"
)]
#![expect(
    clippy::borrow_as_ptr,
    reason = "Vulkan entry points take raw pointers"
)]
#![expect(
    clippy::excessive_precision,
    reason = "these literals are the same bits the parity shader writes"
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

use std::ffi::{c_char, c_void};

use genos_math::optics::{self, PortalLink, MIRROR_FRESNEL_CUTOFF};
use genos_math::{Mat3, Vec3};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/reflections_check.comp.spv"));
const ROWS: usize = 4;
const STRIDE: usize = 19;
const WORDS: usize = ROWS * STRIDE;
const LIMIT: f32 = 1.0e-5;

#[derive(Clone, Copy)]
struct Inputs {
    normal: Vec3,
    incident: Vec3,
    point: Vec3,
    cos_i: f32,
    eta_refract: f32,
    n_dot_h: f32,
    alpha: f32,
    sigma: f32,
    distance: f32,
    ior: f32,
    from_basis: Mat3,
    to_basis: Mat3,
    portal_point: Vec3,
    chain: [f32; 8],
    chain_count: usize,
    view_cap: u32,
}

fn yaw_basis() -> Mat3 {
    Mat3::from_cols([0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0])
}

/// The four rows `reflections_check.comp` writes. Same literals as that shader.
fn inputs() -> [Inputs; ROWS] {
    let up = Vec3::Y;
    let diag = Vec3::new(0.0, 1.0, 1.0).normalize();
    let id = Mat3::IDENTITY;
    let yaw = yaw_basis();
    let water = [0.020_373_188, 0.059_690_919, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let bright = [0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0];
    let one = [0.020_373_188, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    [
        Inputs {
            normal: up,
            incident: Vec3::new(0.6, -0.8, 0.0),
            point: Vec3::new(1.0, 2.0, 3.0),
            cos_i: 0.5,
            eta_refract: 1.0 / 1.333,
            n_dot_h: 1.0,
            alpha: 0.5,
            sigma: 0.5,
            distance: 1.0,
            ior: 1.333,
            from_basis: id,
            to_basis: id,
            portal_point: Vec3::new(1.0, 2.0, 3.0),
            chain: water,
            chain_count: 2,
            view_cap: 3,
        },
        Inputs {
            normal: up,
            incident: Vec3::new(0.0, -1.0, 0.0),
            point: Vec3::new(0.0, 1.0, 0.0),
            cos_i: 1.0,
            eta_refract: 1.0 / 1.333,
            n_dot_h: 0.5,
            alpha: 0.25,
            sigma: 0.5,
            distance: 2.0,
            ior: 1.333,
            from_basis: id,
            to_basis: id,
            portal_point: Vec3::new(0.0, 0.0, 1.0),
            chain: water,
            chain_count: 2,
            view_cap: 0,
        },
        Inputs {
            normal: up,
            incident: Vec3::new(0.95, -0.05, 0.0).normalize(),
            point: Vec3::new(2.0, -1.0, 4.0),
            cos_i: 0.0,
            eta_refract: 1.333,
            n_dot_h: 0.25,
            alpha: 0.7,
            sigma: 1.0,
            distance: 0.25,
            ior: 1.5,
            from_basis: yaw,
            to_basis: yaw,
            portal_point: Vec3::new(1.0, 2.0, 3.0),
            chain: bright,
            chain_count: 5,
            view_cap: 3,
        },
        Inputs {
            normal: diag,
            incident: Vec3::new(0.2, -0.4, 0.4).normalize(),
            point: Vec3::new(-1.0, 0.5, 2.0),
            cos_i: 0.25,
            eta_refract: 1.0 / 1.5,
            n_dot_h: 0.8,
            alpha: 0.4,
            sigma: 0.2,
            distance: 4.0,
            ior: 1.5,
            from_basis: id,
            to_basis: id,
            portal_point: Vec3::new(4.0, 0.0, -2.0),
            chain: one,
            chain_count: 1,
            view_cap: 1,
        },
    ]
}

fn cpu_words(broken: bool) -> [f32; WORDS] {
    let mut out = [0.0; WORDS];
    let to_origin = Vec3::new(10.0, 0.0, 0.0);
    for (row, input) in inputs().into_iter().enumerate() {
        let reflected = optics::mirror_dir(input.incident, input.normal);
        let mirrored = optics::mirror_point(input.point, input.normal);
        let above = optics::fresnel_dielectric(input.ior, input.cos_i);
        let eta_below = if broken { input.ior } else { 1.0 / input.ior };
        let below = optics::fresnel_dielectric(eta_below, input.cos_i);
        let hit = optics::refract_dir(input.incident, input.normal, input.eta_refract);
        let (tx, ty, tz, ok) = hit.map_or((0.0, 0.0, 0.0, 0.0), |dir| (dir.x, dir.y, dir.z, 1.0));
        let portal = optics::portal_point(
            PortalLink {
                from_origin: Vec3::ZERO,
                from_basis: input.from_basis,
                to_origin,
                to_basis: input.to_basis,
            },
            input.portal_point,
        );
        let views = optics::mirror_views(
            &input.chain[..input.chain_count],
            input.view_cap,
            MIRROR_FRESNEL_CUTOFF,
        );
        let base = row * STRIDE;
        let packed = [
            reflected.x,
            reflected.y,
            reflected.z,
            mirrored.x,
            mirrored.y,
            mirrored.z,
            above,
            below,
            tx,
            ty,
            tz,
            ok,
            optics::ggx_ndf(input.n_dot_h, input.alpha),
            optics::beer(input.sigma, input.distance),
            optics::flat_water_transmittance(input.ior),
            portal.x,
            portal.y,
            portal.z,
            views as f32,
        ];
        out[base..base + STRIDE].copy_from_slice(&packed);
    }
    out
}

fn max_abs(gpu: &[f32], cpu: &[f32]) -> f32 {
    gpu.iter()
        .zip(cpu)
        .map(|(got, expect)| (got - expect).abs())
        .fold(0.0, f32::max)
}

fn max_abs_slots(gpu: &[f32], cpu: &[f32], slot: usize) -> f32 {
    (0..ROWS)
        .map(|row| (gpu[row * STRIDE + slot] - cpu[row * STRIDE + slot]).abs())
        .fold(0.0, f32::max)
}

fn max_abs_except(gpu: &[f32], cpu: &[f32], skip: usize) -> f32 {
    let mut max_err: f32 = 0.0;
    for (index, (got, expect)) in gpu.iter().zip(cpu).enumerate() {
        if index % STRIDE == skip {
            continue;
        }
        max_err = max_err.max((got - expect).abs());
    }
    max_err
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
        let app_name = c"genos-reflections-parity";
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
            if !name.to_ascii_lowercase().contains("llvmpipe") {
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
        std::ptr::write_bytes(mapped, 0, alloc_size as usize);

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
fn lavapipe_matches_optics_and_a_broken_fresnel_fails() {
    let cpu = cpu_words(false);
    let (name, gpu) = dispatch(0).unwrap_or_else(|err| panic!("{err}"));
    assert!(
        name.to_ascii_lowercase().contains("llvmpipe"),
        "dispatched on {name}"
    );
    let err = max_abs(&gpu, &cpu);
    assert!(
        err <= LIMIT,
        "optics parity {err} exceeds {LIMIT} on {name}: gpu={gpu:?} cpu={cpu:?}"
    );

    let (_name, broken_gpu) = dispatch(1).unwrap_or_else(|err| panic!("{err}"));
    let below = max_abs_slots(&broken_gpu, &cpu, 7);
    let rest = max_abs_except(&broken_gpu, &cpu, 7);
    assert!(
        rest <= LIMIT,
        "broken fresnel moved another output by {rest}"
    );
    assert!(
        below > LIMIT,
        "forgetting to invert eta still matched within {LIMIT} (error {below})"
    );

    let mut nudged = gpu.clone();
    nudged[0] += 2.0e-5;
    let nudged_err = max_abs(&nudged, &cpu);
    assert!(
        nudged_err > LIMIT,
        "a 2e-5 miss still passed the 1e-5 gate ({nudged_err})"
    );
}
