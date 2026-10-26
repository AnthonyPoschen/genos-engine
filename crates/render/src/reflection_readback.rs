// Headless dispatch of the frame grade shader. The presented composite runs
// this same SPIR-V after lighting and maps the same buffer. Preset 0 and 1
// store a ray count of 0. `fault` is `PictureFault::bits`.

use crate::reflections::{reflection_grade_from_bytes, ReflectionGrade};

/// Run `reflections_frame.comp` and read the grade buffer back.
pub fn read_frame_grade(preset: u32, fault: u32) -> Result<ReflectionGrade, String> {
    unsafe { dispatch_grade(preset, fault) }
}

fn wide(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

unsafe fn dispatch_grade(preset: u32, fault: u32) -> Result<ReflectionGrade, String> {
    let lib = dlopen(c"libvulkan.so.1".as_ptr(), 2);
    if lib.is_null() {
        return Err("libvulkan.so.1 did not open".into());
    }
    let get_instance: GetProc = transmute(dlsym(lib, c"vkGetInstanceProcAddr".as_ptr()));
    let create_instance: unsafe extern "system" fn(*const u8, *const c_void, *mut Handle) -> VkResult =
        transmute(get_instance(
            std::ptr::null_mut(),
            c"vkCreateInstance".as_ptr(),
        ));
    let app_name = c"genos-reflection-grade";
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
        name: app_name.as_ptr(),
        app_version: 1,
        engine: app_name.as_ptr(),
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
        enabled_exts: 0,
        ext_names: std::ptr::null(),
    };
    let mut instance = std::ptr::null_mut();
    check(
        create_instance(&raw const info as *const u8, std::ptr::null(), &mut instance),
        "vkCreateInstance",
    )?;
    let load = |name: &std::ffi::CStr| get_instance(instance, name.as_ptr());
    let enumerate: FnEnumDev = transmute(load(c"vkEnumeratePhysicalDevices"));
    let get_props: unsafe extern "system" fn(Handle, *mut u8) =
        transmute(load(c"vkGetPhysicalDeviceProperties"));
    let queue_families: FnQueues =
        transmute(load(c"vkGetPhysicalDeviceQueueFamilyProperties"));
    let mem_props_fn: FnMemProps = transmute(load(c"vkGetPhysicalDeviceMemoryProperties"));
    let create_device: FnCreateDevice = transmute(load(c"vkCreateDevice"));
    let get_device: GetProc = transmute(load(c"vkGetDeviceProcAddr"));
    let destroy_instance: FnDestroy = transmute(load(c"vkDestroyInstance"));

    let mut dev_count = 0u32;
    check(
        enumerate(instance, &mut dev_count, std::ptr::null_mut()),
        "devices",
    )?;
    if dev_count == 0 {
        destroy_instance(instance, std::ptr::null());
        return Err("no Vulkan physical device".into());
    }
    let mut devices = vec![std::ptr::null_mut(); wide(dev_count)];
    check(
        enumerate(instance, &mut dev_count, devices.as_mut_ptr()),
        "device list",
    )?;
    let mut chosen: Option<(Handle, u32, bool)> = None;
    for physical in devices {
        let mut prop = [0u8; 1024];
        get_props(physical, prop.as_mut_ptr());
        let name = device_name(&prop[20..276]);
        let lavapipe = name.to_ascii_lowercase().contains("llvmpipe");
        let mut family_count = 0u32;
        queue_families(physical, &mut family_count, std::ptr::null_mut());
        let mut families = vec![0u8; wide(family_count).saturating_mul(24)];
        queue_families(physical, &mut family_count, families.as_mut_ptr());
        for index in 0..family_count {
            let base = wide(index).saturating_mul(24);
            let flags = u32::from_ne_bytes(
                families[base..base + 4]
                    .try_into()
                    .unwrap_or([0, 0, 0, 0]),
            );
            if flags & 0x2 != 0 && (lavapipe || chosen.is_none()) {
                chosen = Some((physical, index, lavapipe));
                if lavapipe {
                    break;
                }
            }
        }
        if chosen.is_some_and(|(_, _, pipe)| pipe) {
            break;
        }
    }
    let Some((physical, family, _)) = chosen else {
        destroy_instance(instance, std::ptr::null());
        return Err("no compute queue".into());
    };
    let mut raw_props = [0u8; 520];
    mem_props_fn(physical, raw_props.as_mut_ptr());
    let mem = parse_mem_props(&raw_props);
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
    check(
        create_device(
            physical,
            &raw const dev_info as *const u8,
            std::ptr::null(),
            &mut device,
        ),
        "vkCreateDevice",
    )?;
    let dload = |entry: &std::ffi::CStr| get_device(device, entry.as_ptr());
    let destroy_device: FnDestroy = transmute(dload(c"vkDestroyDevice"));
    let get_queue: FnGetQueue = transmute(dload(c"vkGetDeviceQueue"));
    let create_buffer: FnBuffer = transmute(dload(c"vkCreateBuffer"));
    let buffer_req: FnBufReq = transmute(dload(c"vkGetBufferMemoryRequirements"));
    let alloc_mem: FnAlloc = transmute(dload(c"vkAllocateMemory"));
    let bind_buf: FnBindBuf = transmute(dload(c"vkBindBufferMemory"));
    let map_mem: FnMap = transmute(dload(c"vkMapMemory"));
    let unmap_mem: FnUnmap = transmute(dload(c"vkUnmapMemory"));
    let free_mem: Fn2 = transmute(dload(c"vkFreeMemory"));
    let destroy_buf: Fn2 = transmute(dload(c"vkDestroyBuffer"));
    let desc_layout: FnLayout = transmute(dload(c"vkCreateDescriptorSetLayout"));
    let destroy_set_layout: Fn2 = transmute(dload(c"vkDestroyDescriptorSetLayout"));
    let pipe_layout_fn: FnLayout = transmute(dload(c"vkCreatePipelineLayout"));
    let destroy_pipe_layout: Fn2 = transmute(dload(c"vkDestroyPipelineLayout"));
    let shader_mod: FnShader = transmute(dload(c"vkCreateShaderModule"));
    let destroy_shader: Fn2 = transmute(dload(c"vkDestroyShaderModule"));
    let create_pipes: FnPipelines = transmute(dload(c"vkCreateComputePipelines"));
    let destroy_pipe: Fn2 = transmute(dload(c"vkDestroyPipeline"));
    let desc_pool: FnPool = transmute(dload(c"vkCreateDescriptorPool"));
    let destroy_pool: Fn2 = transmute(dload(c"vkDestroyDescriptorPool"));
    let alloc_desc: FnAllocCmd = transmute(dload(c"vkAllocateDescriptorSets"));
    let update_desc: FnUpdateDesc = transmute(dload(c"vkUpdateDescriptorSets"));
    let cmd_pool_fn: FnPool = transmute(dload(c"vkCreateCommandPool"));
    let destroy_cmd_pool: Fn2 = transmute(dload(c"vkDestroyCommandPool"));
    let alloc_cmd: FnAllocCmd = transmute(dload(c"vkAllocateCommandBuffers"));
    let begin_cmd: FnBegin = transmute(dload(c"vkBeginCommandBuffer"));
    let end_cmd: FnCmdResult = transmute(dload(c"vkEndCommandBuffer"));
    let bind_pipe: FnBindPipe = transmute(dload(c"vkCmdBindPipeline"));
    let bind_set: FnBindSet = transmute(dload(c"vkCmdBindDescriptorSets"));
    let push: FnPush = transmute(dload(c"vkCmdPushConstants"));
    let dispatch_cmd: FnDispatch = transmute(dload(c"vkCmdDispatch"));
    let barrier: FnBarrier = transmute(dload(c"vkCmdPipelineBarrier"));
    let submit: FnSubmit = transmute(dload(c"vkQueueSubmit"));
    let idle: FnDevice = transmute(dload(c"vkQueueWaitIdle"));
    let mut queue = std::ptr::null_mut();
    get_queue(device, family, 0, &mut queue);

    let grade = run_grade(
        device,
        queue,
        family,
        &mem,
        preset,
        fault,
        create_buffer,
        buffer_req,
        alloc_mem,
        bind_buf,
        map_mem,
        unmap_mem,
        free_mem,
        destroy_buf,
        desc_layout,
        destroy_set_layout,
        pipe_layout_fn,
        destroy_pipe_layout,
        shader_mod,
        destroy_shader,
        create_pipes,
        destroy_pipe,
        desc_pool,
        destroy_pool,
        alloc_desc,
        update_desc,
        cmd_pool_fn,
        destroy_cmd_pool,
        alloc_cmd,
        begin_cmd,
        end_cmd,
        bind_pipe,
        bind_set,
        push,
        dispatch_cmd,
        barrier,
        submit,
        idle,
    );
    destroy_device(device, std::ptr::null());
    destroy_instance(instance, std::ptr::null());
    grade
}

fn device_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[allow(clippy::too_many_arguments)]
unsafe fn run_grade(
    device: Handle,
    queue: Handle,
    family: u32,
    mem: &MemProps,
    preset: u32,
    fault: u32,
    create_buffer: FnBuffer,
    buffer_req: FnBufReq,
    alloc_mem: FnAlloc,
    bind_buf: FnBindBuf,
    map_mem: FnMap,
    unmap_mem: FnUnmap,
    free_mem: Fn2,
    destroy_buf: Fn2,
    desc_layout: FnLayout,
    destroy_set_layout: Fn2,
    pipe_layout_fn: FnLayout,
    destroy_pipe_layout: Fn2,
    shader_mod: FnShader,
    destroy_shader: Fn2,
    create_pipes: FnPipelines,
    destroy_pipe: Fn2,
    desc_pool: FnPool,
    destroy_pool: Fn2,
    alloc_desc: FnAllocCmd,
    update_desc: FnUpdateDesc,
    cmd_pool_fn: FnPool,
    destroy_cmd_pool: Fn2,
    alloc_cmd: FnAllocCmd,
    begin_cmd: FnBegin,
    end_cmd: FnCmdResult,
    bind_pipe: FnBindPipe,
    bind_set: FnBindSet,
    push: FnPush,
    dispatch_cmd: FnDispatch,
    barrier: FnBarrier,
    submit: FnSubmit,
    idle: FnDevice,
) -> Result<ReflectionGrade, String> {
    let bytes = u64::try_from(GRADE_BYTES).unwrap_or(0);
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
    check(
        create_buffer(device, &raw const buf_info as *const u8, std::ptr::null(), &mut buffer),
        "grade buffer",
    )?;
    let mut reqs = [0u8; 24];
    buffer_req(device, buffer, reqs.as_mut_ptr());
    let alloc_size = u64::from_ne_bytes(reqs[0..8].try_into().unwrap_or([0; 8]));
    let type_bits = u32::from_ne_bytes(reqs[16..20].try_into().unwrap_or([0; 4]));
    let Some(mem_index) = host_index(type_bits, mem) else {
        destroy_buf(device, buffer, std::ptr::null());
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
    check(
        alloc_mem(device, &raw const alloc_info as *const u8, std::ptr::null(), &mut memory),
        "allocate grade",
    )?;
    check(bind_buf(device, buffer, memory, 0), "bind grade")?;
    let mut mapped = std::ptr::null_mut();
    check(map_mem(device, memory, 0, alloc_size, 0, &mut mapped), "map grade")?;
    let mapped_len = usize::try_from(alloc_size).unwrap_or(0);
    std::ptr::write_bytes(mapped, 0, mapped_len);

    let (set_layout, layout, pipeline, pool, set, commands) = grade_pipe(
        device,
        family,
        buffer,
        desc_layout,
        pipe_layout_fn,
        shader_mod,
        destroy_shader,
        create_pipes,
        desc_pool,
        alloc_desc,
        update_desc,
        cmd_pool_fn,
    )?;
    let recorded = record_grade(
        commands,
        pipeline,
        layout,
        set,
        buffer,
        preset,
        fault,
        alloc_cmd,
        begin_cmd,
        end_cmd,
        bind_pipe,
        bind_set,
        push,
        dispatch_cmd,
        barrier,
        device,
    );
    let submitted = recorded.and_then(|cmd| {
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
        check(
            submit(queue, 1, &raw const submit_info as *const u8, std::ptr::null_mut()),
            "submit grade",
        )?;
        check(idle(queue), "wait grade")?;
        let mut copied = vec![0u8; GRADE_BYTES];
        std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), copied.as_mut_ptr(), GRADE_BYTES);
        reflection_grade_from_bytes(&copied).ok_or_else(|| "grade buffer was short".to_string())
    });
    unmap_mem(device, memory);
    destroy_cmd_pool(device, commands, std::ptr::null());
    destroy_pool(device, pool, std::ptr::null());
    destroy_pipe(device, pipeline, std::ptr::null());
    destroy_pipe_layout(device, layout, std::ptr::null());
    destroy_set_layout(device, set_layout, std::ptr::null());
    destroy_buf(device, buffer, std::ptr::null());
    free_mem(device, memory, std::ptr::null());
    submitted
}

fn host_index(type_bits: u32, props: &MemProps) -> Option<u32> {
    let mut index = 0u32;
    while index < props.count {
        let bit = 1u32.checked_shl(index).unwrap_or(0);
        let slot = wide(index);
        let flags = props.types[slot];
        if type_bits & bit != 0 && flags & (2 | 4) == (2 | 4) {
            return Some(index);
        }
        index += 1;
    }
    None
}

#[allow(clippy::type_complexity)]
unsafe fn grade_pipe(
    device: Handle,
    family: u32,
    buffer: Handle,
    desc_layout: FnLayout,
    pipe_layout_fn: FnLayout,
    shader_mod: FnShader,
    destroy_shader: Fn2,
    create_pipes: FnPipelines,
    desc_pool: FnPool,
    alloc_desc: FnAllocCmd,
    update_desc: FnUpdateDesc,
    cmd_pool_fn: FnPool,
) -> Result<(Handle, Handle, Handle, Handle, Handle, Handle), String> {
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
    let mut set_layout = std::ptr::null_mut();
    check(
        desc_layout(device, &raw const set_info as *const u8, std::ptr::null(), &mut set_layout),
        "grade set layout",
    )?;
    #[repr(C)]
    struct Range {
        stage: u32,
        offset: u32,
        size: u32,
    }
    let push_range = Range {
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
    let layout_info = LayoutInfo {
        s_type: 30,
        next: std::ptr::null(),
        flags: 0,
        set_count: 1,
        sets: &set_layout,
        push_count: 1,
        push: &push_range,
    };
    let mut layout = std::ptr::null_mut();
    check(
        pipe_layout_fn(device, &raw const layout_info as *const u8, std::ptr::null(), &mut layout),
        "grade pipeline layout",
    )?;
    let (words, rest) = REFLECTION_FRAME_SPV.as_chunks::<4>();
    if !rest.is_empty() {
        return Err("reflection frame SPIR-V is not a multiple of 4".into());
    }
    let spirv: Vec<u32> = words.iter().copied().map(u32::from_le_bytes).collect();
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
    check(
        shader_mod(device, &raw const shader_info as *const u8, std::ptr::null(), &mut module),
        "grade shader",
    )?;
    #[repr(C)]
    struct Stage {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        stage: u32,
        module: Handle,
        name: *const c_char,
        spec: *const c_void,
    }
    let stage = Stage {
        s_type: 18,
        next: std::ptr::null(),
        flags: 0,
        stage: 0x20,
        module,
        name: c"main".as_ptr(),
        spec: std::ptr::null(),
    };
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
        stage,
        layout,
        base: std::ptr::null_mut(),
        base_index: -1,
    };
    let mut pipeline = std::ptr::null_mut();
    let created = create_pipes(
        device,
        std::ptr::null_mut(),
        1,
        &raw const pipe as *const u8,
        std::ptr::null(),
        &mut pipeline,
    );
    destroy_shader(device, module, std::ptr::null());
    check(created, "grade compute pipeline")?;
    #[repr(C)]
    struct Size {
        kind: u32,
        count: u32,
    }
    let size = Size { kind: 7, count: 1 };
    #[repr(C)]
    struct PoolInfo {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        max_sets: u32,
        size_count: u32,
        sizes: *const Size,
    }
    let pool_info = PoolInfo {
        s_type: 33,
        next: std::ptr::null(),
        flags: 0,
        max_sets: 1,
        size_count: 1,
        sizes: &size,
    };
    let mut pool = std::ptr::null_mut();
    check(
        desc_pool(device, &raw const pool_info as *const u8, std::ptr::null(), &mut pool),
        "grade descriptor pool",
    )?;
    #[repr(C)]
    struct AllocSet {
        s_type: i32,
        next: *const c_void,
        pool: Handle,
        count: u32,
        layouts: *const Handle,
    }
    let alloc_sets = AllocSet {
        s_type: 34,
        next: std::ptr::null(),
        pool,
        count: 1,
        layouts: &set_layout,
    };
    let mut set = std::ptr::null_mut();
    check(
        alloc_desc(device, &raw const alloc_sets as *const u8, &mut set),
        "grade descriptor set",
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
    check(
        cmd_pool_fn(device, &raw const cmd_pool_info as *const u8, std::ptr::null(), &mut commands),
        "grade command pool",
    )?;
    Ok((set_layout, layout, pipeline, pool, set, commands))
}

unsafe fn record_grade(
    commands: Handle,
    pipeline: Handle,
    layout: Handle,
    set: Handle,
    buffer: Handle,
    preset: u32,
    fault: u32,
    alloc_cmd: FnAllocCmd,
    begin_cmd: FnBegin,
    end_cmd: FnCmdResult,
    bind_pipe: FnBindPipe,
    bind_set: FnBindSet,
    push: FnPush,
    dispatch_cmd: FnDispatch,
    barrier: FnBarrier,
    device: Handle,
) -> Result<Handle, String> {
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
    check(
        alloc_cmd(device, &raw const cmd_alloc as *const u8, &mut cmd),
        "grade command buffer",
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
    check(begin_cmd(cmd, &raw const begin as *const u8), "begin grade")?;
    bind_pipe(cmd, 1, pipeline);
    bind_set(cmd, 1, layout, 0, 1, &set, 0, std::ptr::null());
    let pc = [preset, fault, 0u32, 0];
    push(cmd, layout, 0x20, 0, 16, pc.as_ptr().cast::<c_void>());
    dispatch_cmd(cmd, 3, 1, 1);
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
        &raw const buf_barrier as *const c_void,
        0,
        std::ptr::null(),
    );
    check(end_cmd(cmd), "end grade")?;
    Ok(cmd)
}
