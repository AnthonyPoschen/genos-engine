//! Headless lavapipe dispatch of `restir_initial.comp`.
//!
//! One compute invocation walks the receiver grid in order. The PCG stream is
//! the CPU reference stream. The loader is `libvulkan.so.1` (ADR 0007).

#![expect(clippy::as_conversions, reason = "Vulkan create-info pointers")]
#![expect(clippy::borrow_as_ptr, reason = "Vulkan entry points take raw pointers")]
#![expect(
    clippy::items_after_statements,
    reason = "Vulkan struct layouts sit next to the call that fills them"
)]
#![expect(clippy::ptr_as_ptr, reason = "Vulkan create-info pointers")]
#![expect(clippy::too_many_lines, reason = "one headless compute dispatch")]
#![expect(
    clippy::undocumented_unsafe_blocks,
    reason = "Vulkan and dlopen FFI, same as the reflections parity test"
)]

use std::ffi::{c_char, c_void, CStr};

use genos_math::f32_from_f64;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/restir_initial.spv"));

type Handle = *mut c_void;
type VkResult = i32;

/// Push constants. Field order matches `restir_initial.comp`.
#[derive(Clone, Copy, Debug)]
pub struct Push {
    pub seed: u64,
    pub receivers: u32,
    pub lamps: u32,
    pub candidates: u32,
    pub proposal: u32,
    pub lamp_base: u32,
    pub recv_base: u32,
    pub out_base: u32,
}

/// Device name and the dispatch duration in milliseconds.
pub struct GpuRun {
    pub device: String,
    pub dispatch_ms: f32,
    pub words: Vec<u32>,
}

/// Runs the initial-candidate shader on lavapipe.
///
/// # Errors
///
/// Returns a string when the loader, the device, or the dispatch fails.
pub fn dispatch(words: &[u32], push: Push) -> Result<GpuRun, String> {
    unsafe { dispatch_inner(words, push) }
}

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
        Err(format!("{what} failed ({result})"))
    }
}

fn device_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn low_high(seed: u64) -> Result<(u32, u32), String> {
    let lo = u32::try_from(seed & 0xffff_ffff).map_err(|_| "seed low half")?;
    let hi = u32::try_from((seed >> 32) & 0xffff_ffff).map_err(|_| "seed high half")?;
    Ok((lo, hi))
}

unsafe fn dispatch_inner(words: &[u32], push: Push) -> Result<GpuRun, String> {
    let lib = dlopen(c"libvulkan.so.1".as_ptr(), 2);
    if lib.is_null() {
        return Err("libvulkan.so.1 did not open".to_string());
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
    let app_name = c"genos-restir-gpu";
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
        create_instance(&raw const info as *const u8, std::ptr::null(), &mut instance),
        "vkCreateInstance",
    )?;
    let load = |name: &CStr| get_instance(instance, name.as_ptr());
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
    let mut devices = vec![std::ptr::null_mut(); usize::try_from(dev_count).unwrap_or(0)];
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
        let stride = 24usize;
        let bytes = stride.saturating_mul(usize::try_from(family_count).unwrap_or(0));
        let mut families = vec![0u8; bytes];
        queue_families(physical, &mut family_count, families.as_mut_ptr());
        for index in 0..family_count {
            let base = stride.saturating_mul(usize::try_from(index).unwrap_or(0));
            let Some(flag_bytes) = families.get(base..base + 4) else {
                continue;
            };
            let Ok(flag_arr) = <[u8; 4]>::try_from(flag_bytes) else {
                continue;
            };
            if u32::from_ne_bytes(flag_arr) & 0x2 != 0 {
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
        return Err("no lavapipe device with a compute queue".to_string());
    };

    let mut raw_props = [0u8; 4 + 32 * 8];
    mem_props(physical, raw_props.as_mut_ptr());
    let mem_count = u32::from_ne_bytes(raw_props[0..4].try_into().unwrap_or([0; 4])).min(32);
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
    let mut features = [0u32; 55];
    features[40] = 1;
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
        features: *const u32,
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
        features: features.as_ptr(),
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
    let dload = |entry: &CStr| get_device(device, entry.as_ptr());
    let started = std::time::Instant::now();
    let result = run_shader(dload, device, family, words, push, &raw_props, mem_count);
    let millis = started.elapsed().as_secs_f64() * 1000.0;
    type FnDestroyDev = unsafe extern "system" fn(Handle, *const c_void);
    let destroy_device: FnDestroyDev = std::mem::transmute(dload(c"vkDestroyDevice"));
    destroy_device(device, std::ptr::null());
    destroy_instance(instance, std::ptr::null());
    let mut run = result?;
    run.device = name;
    run.dispatch_ms = f32_from_f64(millis);
    Ok(run)
}

unsafe fn run_shader(
    dload: impl Fn(&CStr) -> *const c_void,
    device: Handle,
    family: u32,
    words: &[u32],
    push: Push,
    raw_props: &[u8],
    mem_count: u32,
) -> Result<GpuRun, String> {
    type FnQueue = unsafe extern "system" fn(Handle, u32, u32, *mut Handle);
    type FnBuf = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
    type FnReq = unsafe extern "system" fn(Handle, Handle, *mut u8);
    type FnAlloc = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
    type FnBind = unsafe extern "system" fn(Handle, Handle, Handle, u64) -> VkResult;
    type FnMap = unsafe extern "system" fn(Handle, Handle, u64, u64, u32, *mut *mut c_void) -> VkResult;
    type FnUnmap = unsafe extern "system" fn(Handle, Handle);
    type FnLayout = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
    type FnShader = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
    type FnPool = unsafe extern "system" fn(Handle, *const u8, *const c_void, *mut Handle) -> VkResult;
    type FnAllocCmd = unsafe extern "system" fn(Handle, *const u8, *mut Handle) -> VkResult;
    type FnBegin = unsafe extern "system" fn(Handle, *const u8) -> VkResult;
    type FnEnd = unsafe extern "system" fn(Handle) -> VkResult;
    type FnBindPipe = unsafe extern "system" fn(Handle, u32, Handle);
    type FnBindSet = unsafe extern "system" fn(Handle, u32, Handle, u32, u32, *const Handle, u32, *const u32);
    type FnPush = unsafe extern "system" fn(Handle, Handle, u32, u32, u32, *const c_void);
    type FnDispatch = unsafe extern "system" fn(Handle, u32, u32, u32);
    type FnBarrier = unsafe extern "system" fn(Handle, u32, u32, u32, u32, *const c_void, u32, *const u8, u32, *const c_void);
    type FnSubmit = unsafe extern "system" fn(Handle, u32, *const u8, Handle) -> VkResult;
    type FnIdle = unsafe extern "system" fn(Handle) -> VkResult;
    type FnUpdate = unsafe extern "system" fn(Handle, u32, *const u8, u32, *const u8);

    let get_queue: FnQueue = std::mem::transmute(dload(c"vkGetDeviceQueue"));
    let create_buffer: FnBuf = std::mem::transmute(dload(c"vkCreateBuffer"));
    let buffer_req: FnReq = std::mem::transmute(dload(c"vkGetBufferMemoryRequirements"));
    let alloc_mem: FnAlloc = std::mem::transmute(dload(c"vkAllocateMemory"));
    let bind_buf: FnBind = std::mem::transmute(dload(c"vkBindBufferMemory"));
    let map_mem: FnMap = std::mem::transmute(dload(c"vkMapMemory"));
    let unmap_mem: FnUnmap = std::mem::transmute(dload(c"vkUnmapMemory"));
    let desc_layout: FnLayout = std::mem::transmute(dload(c"vkCreateDescriptorSetLayout"));
    let pipe_layout: FnLayout = std::mem::transmute(dload(c"vkCreatePipelineLayout"));
    let shader_mod: FnShader = std::mem::transmute(dload(c"vkCreateShaderModule"));
    let create_pipes: unsafe extern "system" fn(Handle, Handle, u32, *const u8, *const c_void, *mut Handle) -> VkResult =
        std::mem::transmute(dload(c"vkCreateComputePipelines"));
    let desc_pool: FnPool = std::mem::transmute(dload(c"vkCreateDescriptorPool"));
    let alloc_desc: FnAllocCmd = std::mem::transmute(dload(c"vkAllocateDescriptorSets"));
    let update_desc: FnUpdate = std::mem::transmute(dload(c"vkUpdateDescriptorSets"));
    let cmd_pool: FnPool = std::mem::transmute(dload(c"vkCreateCommandPool"));
    let alloc_cmd: FnAllocCmd = std::mem::transmute(dload(c"vkAllocateCommandBuffers"));
    let begin_cmd: FnBegin = std::mem::transmute(dload(c"vkBeginCommandBuffer"));
    let end_cmd: FnEnd = std::mem::transmute(dload(c"vkEndCommandBuffer"));
    let bind_pipe: FnBindPipe = std::mem::transmute(dload(c"vkCmdBindPipeline"));
    let bind_set: FnBindSet = std::mem::transmute(dload(c"vkCmdBindDescriptorSets"));
    let push_cmd: FnPush = std::mem::transmute(dload(c"vkCmdPushConstants"));
    let dispatch_cmd: FnDispatch = std::mem::transmute(dload(c"vkCmdDispatch"));
    let barrier: FnBarrier = std::mem::transmute(dload(c"vkCmdPipelineBarrier"));
    let submit: FnSubmit = std::mem::transmute(dload(c"vkQueueSubmit"));
    let idle: FnIdle = std::mem::transmute(dload(c"vkQueueWaitIdle"));

    let mut queue = std::ptr::null_mut();
    get_queue(device, family, 0, &mut queue);
    let byte_len = u64::try_from(words.len().saturating_mul(4)).unwrap_or(0);
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
        size: byte_len,
        usage: 0x20,
        sharing: 0,
        queue_count: 0,
        queues: std::ptr::null(),
    };
    let mut buffer = std::ptr::null_mut();
    vk_check(
        create_buffer(device, &raw const buf_info as *const u8, std::ptr::null(), &mut buffer),
        "buffer",
    )?;
    let mut reqs = [0u8; 24];
    buffer_req(device, buffer, reqs.as_mut_ptr());
    let alloc_size = u64::from_ne_bytes(reqs[0..8].try_into().unwrap_or([0; 8]));
    let type_bits = u32::from_ne_bytes(reqs[16..20].try_into().unwrap_or([0; 4]));
    let mut mem_index = None;
    for index in 0..mem_count {
        let offset = 4 + usize::try_from(index).unwrap_or(0) * 8;
        let Some(flag_bytes) = raw_props.get(offset..offset + 4) else {
            continue;
        };
        let Ok(flag_arr) = <[u8; 4]>::try_from(flag_bytes) else {
            continue;
        };
        let flags = u32::from_ne_bytes(flag_arr);
        if type_bits & (1u32 << index) != 0 && flags & (2 | 4) == (2 | 4) {
            mem_index = Some(index);
            break;
        }
    }
    let Some(mem_index) = mem_index else {
        return Err("no host-visible coherent memory".to_string());
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
        alloc_mem(device, &raw const alloc_info as *const u8, std::ptr::null(), &mut memory),
        "allocate",
    )?;
    vk_check(bind_buf(device, buffer, memory, 0), "bind")?;
    let mut mapped = std::ptr::null_mut();
    vk_check(map_mem(device, memory, 0, alloc_size, 0, &mut mapped), "map")?;
    let dst = std::slice::from_raw_parts_mut(mapped.cast::<u32>(), words.len());
    dst.copy_from_slice(words);

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
        desc_layout(device, &raw const layout_info as *const u8, std::ptr::null(), &mut set_layout),
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
        size: 36,
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
        pipe_layout(device, &raw const pipe_layout_info as *const u8, std::ptr::null(), &mut layout),
        "pipeline layout",
    )?;
    let (spirv_words, rest) = SPV.as_chunks::<4>();
    if !rest.is_empty() {
        return Err("SPIR-V length is not a multiple of 4".to_string());
    }
    let spirv: Vec<u32> = spirv_words.iter().map(|word| u32::from_le_bytes(*word)).collect();
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
        size: spirv.len().saturating_mul(4),
        code: spirv.as_ptr(),
    };
    let mut module = std::ptr::null_mut();
    vk_check(
        shader_mod(device, &raw const shader_info as *const u8, std::ptr::null(), &mut module),
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
        create_pipes(device, std::ptr::null_mut(), 1, &raw const pipe_info as *const u8, std::ptr::null(), &mut pipeline),
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
        desc_pool(device, &raw const pool_info as *const u8, std::ptr::null(), &mut pool),
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
    vk_check(alloc_desc(device, &raw const alloc_sets as *const u8, &mut set), "descriptor set")?;
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
    update_desc(device, 1, &raw const write as *const u8, 0, std::ptr::null());

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
        cmd_pool(device, &raw const cmd_pool_info as *const u8, std::ptr::null(), &mut commands),
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
    vk_check(alloc_cmd(device, &raw const cmd_alloc as *const u8, &mut cmd), "command buffer")?;
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
        flags: 0,
        inherit: std::ptr::null(),
    };
    vk_check(begin_cmd(cmd, &raw const begin as *const u8), "begin")?;
    #[repr(C)]
    struct MemBarrier {
        s_type: i32,
        next: *const c_void,
        src: u32,
        dst: u32,
    }
    let host_to_shader = MemBarrier {
        s_type: 46,
        next: std::ptr::null(),
        src: 0x4000,
        dst: 0x20,
    };
    barrier(cmd, 0x4000, 0x800, 0, 1, &raw const host_to_shader as *const c_void, 0, std::ptr::null(), 0, std::ptr::null());
    bind_pipe(cmd, 1, pipeline);
    bind_set(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
    let (seed_lo, seed_hi) = low_high(push.seed)?;
    #[repr(C)]
    struct GpuPush {
        seed_lo: u32,
        seed_hi: u32,
        receivers: u32,
        lamps: u32,
        candidates: u32,
        proposal: u32,
        lamp_base: u32,
        recv_base: u32,
        out_base: u32,
    }
    let gpu_push = GpuPush {
        seed_lo,
        seed_hi,
        receivers: push.receivers,
        lamps: push.lamps,
        candidates: push.candidates,
        proposal: push.proposal,
        lamp_base: push.lamp_base,
        recv_base: push.recv_base,
        out_base: push.out_base,
    };
    push_cmd(cmd, layout, 0x20, 0, 36, &raw const gpu_push as *const c_void);
    dispatch_cmd(cmd, 1, 1, 1);
    let shader_to_host = MemBarrier {
        s_type: 46,
        next: std::ptr::null(),
        src: 0x40,
        dst: 0x2000,
    };
    barrier(cmd, 0x800, 0x4000, 0, 1, &raw const shader_to_host as *const c_void, 0, std::ptr::null(), 0, std::ptr::null());
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
    vk_check(submit(queue, 1, &raw const submit_info as *const u8, std::ptr::null_mut()), "submit")?;
    vk_check(idle(queue), "wait")?;
    let src = std::slice::from_raw_parts(mapped.cast::<u32>(), words.len());
    let out = src.to_vec();
    unmap_mem(device, memory);
    Ok(GpuRun {
        device: String::new(),
        dispatch_ms: 0.0,
        words: out,
    })
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
