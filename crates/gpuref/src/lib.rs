//! GPU reference path tracer: the CPU proof's light transport (genos-debug
//! `reference.rs`) on the hardware ray tracer, for converged references in minutes
//! instead of hours. Offline only: it opens its own headless Vulkan device, builds
//! its own acceleration structures from the scene's triangles and shares nothing
//! with the renderer under test. See `shaders/path.comp` for what is traced.

use ash::vk;
use std::ffi::CStr;

/// One triangle: corners, corner normals, albedo, and whether its back is hit.
#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub positions: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub albedo: [f32; 3],
    pub two_sided: bool,
}

/// A lamp: a point at `position` or, when `direction` is non-zero, a sun whose rays
/// travel along `direction`.
#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Setup {
    pub tris: Vec<Tri>,
    pub lamps: Vec<Lamp>,
    pub sky: [f32; 3],
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub fov_y_deg: f32,
    pub width: u32,
    pub height: u32,
    pub max_bounces: u32,
    /// Stop when the noise (mean relative standard error, as the CPU proof
    /// measures it) is this low, or at `max_spp` paths, or after `seconds`.
    pub noise_target: f32,
    pub max_spp: u64,
    pub seconds: f32,
    /// Paths per pixel per half per dispatch.
    pub paths_per_pass: u32,
}

/// A finished trace: linear radiance per pixel (both halves together and each
/// half), which pixels met a surface, paths per pixel and the noise.
#[derive(Clone, Debug)]
pub struct Output {
    pub linear: Vec<[f32; 3]>,
    pub half_a: Vec<[f32; 3]>,
    pub half_b: Vec<[f32; 3]>,
    pub hit: Vec<bool>,
    pub spp: u64,
    pub seconds: f32,
    /// Mean over hit pixels of the standard error of luminance over its mean
    /// (the CPU proof's measure).
    pub noise: f32,
    /// The same from the two halves: mean |A - B| / 2 * sqrt(pi / 2) / mean.
    pub noise_halves: f32,
    pub device: String,
}

struct Buf {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
}

struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    phys: vk::PhysicalDevice,
    queue: vk::Queue,
    family: u32,
    pool: vk::CommandPool,
    asx: ash::khr::acceleration_structure::Device,
    name: String,
}

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/path.spv"));

fn err<E: std::fmt::Debug>(what: &str) -> impl Fn(E) -> String + '_ {
    move |e| format!("gpuref: {what}: {e:?}")
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        unsafe {
            let entry = ash::Entry::load().map_err(err("load vulkan"))?;
            let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 3, 0));
            let instance = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default().application_info(&app),
                    None,
                )
                .map_err(err("instance"))?;
            let need: [&CStr; 4] = [
                ash::khr::acceleration_structure::NAME,
                ash::khr::ray_query::NAME,
                ash::khr::deferred_host_operations::NAME,
                ash::khr::spirv_1_4::NAME,
            ];
            let mut pick = None;
            for phys in instance
                .enumerate_physical_devices()
                .map_err(err("devices"))?
            {
                let exts = instance
                    .enumerate_device_extension_properties(phys)
                    .unwrap_or_default();
                let has = |n: &CStr| {
                    exts.iter()
                        .any(|e| e.extension_name_as_c_str().ok() == Some(n))
                };
                if !need.iter().all(|n| has(n)) {
                    continue;
                }
                let props = instance.get_physical_device_properties(phys);
                let discrete = props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU;
                let fam = instance
                    .get_physical_device_queue_family_properties(phys)
                    .iter()
                    .position(|f| f.queue_flags.contains(vk::QueueFlags::COMPUTE));
                if let Some(f) = fam {
                    let name = props
                        .device_name_as_c_str()
                        .map(|c| c.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if pick.is_none() || discrete {
                        pick = Some((phys, f as u32, name));
                    }
                }
            }
            let (phys, family, name) = pick.ok_or("gpuref: no device with VK_KHR_ray_query")?;
            let prio = [1.0f32];
            let qinfo = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&prio)];
            let ext_names: Vec<*const i8> = need.iter().map(|n| n.as_ptr()).collect();
            let mut f12 = vk::PhysicalDeviceVulkan12Features::default()
                .buffer_device_address(true)
                .scalar_block_layout(true);
            let mut fas = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
                .acceleration_structure(true);
            let mut frq = vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
            let info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&qinfo)
                .enabled_extension_names(&ext_names)
                .push_next(&mut f12)
                .push_next(&mut fas)
                .push_next(&mut frq);
            let device = instance
                .create_device(phys, &info, None)
                .map_err(err("device"))?;
            let queue = device.get_device_queue(family, 0);
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                    None,
                )
                .map_err(err("pool"))?;
            let asx = ash::khr::acceleration_structure::Device::new(&instance, &device);
            Ok(Gpu {
                _entry: entry,
                instance,
                device,
                phys,
                queue,
                family,
                pool,
                asx,
                name,
            })
        }
    }

    fn buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> Result<Buf, String> {
        unsafe {
            let size = size.max(16);
            let buffer = self
                .device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size)
                        .usage(usage | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS),
                    None,
                )
                .map_err(err("buffer"))?;
            let req = self.device.get_buffer_memory_requirements(buffer);
            let mem = self
                .instance
                .get_physical_device_memory_properties(self.phys);
            let want =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let pick = |extra: vk::MemoryPropertyFlags| {
                (0..mem.memory_type_count).find(|&i| {
                    req.memory_type_bits & (1 << i) != 0
                        && mem.memory_types[i as usize]
                            .property_flags
                            .contains(want | extra)
                })
            };
            let ty = pick(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| pick(vk::MemoryPropertyFlags::empty()))
                .ok_or("gpuref: no host-visible memory")?;
            let mut flags = vk::MemoryAllocateFlagsInfo::default()
                .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
            let memory = match self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(ty)
                    .push_next(&mut flags),
                None,
            ) {
                Ok(m) => m,
                Err(_) => {
                    // Device-local host-visible heaps can be small (no ReBAR): system memory.
                    let ty = pick(vk::MemoryPropertyFlags::empty()).ok_or("gpuref: no memory")?;
                    let mut flags = vk::MemoryAllocateFlagsInfo::default()
                        .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                    self.device
                        .allocate_memory(
                            &vk::MemoryAllocateInfo::default()
                                .allocation_size(req.size)
                                .memory_type_index(ty)
                                .push_next(&mut flags),
                            None,
                        )
                        .map_err(err("memory"))?
                }
            };
            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(err("bind"))?;
            let ptr = self
                .device
                .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                .map_err(err("map"))? as *mut u8;
            Ok(Buf {
                buffer,
                memory,
                ptr,
            })
        }
    }

    fn upload<T: Copy>(&self, data: &[T], usage: vk::BufferUsageFlags) -> Result<Buf, String> {
        let bytes = std::mem::size_of_val(data) as u64;
        let b = self.buffer(bytes, usage)?;
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, b.ptr, bytes as usize) };
        Ok(b)
    }

    fn address(&self, b: &Buf) -> u64 {
        unsafe {
            self.device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(b.buffer))
        }
    }

    fn free(&self, b: Buf) {
        unsafe {
            self.device.destroy_buffer(b.buffer, None);
            self.device.free_memory(b.memory, None);
        }
    }

    /// Record with `f` into a one-time command buffer, submit and wait.
    fn run(&self, f: impl FnOnce(vk::CommandBuffer)) -> Result<(), String> {
        unsafe {
            let cmd = self
                .device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .command_buffer_count(1),
                )
                .map_err(err("cmd"))?[0];
            self.device
                .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
                .map_err(err("begin"))?;
            f(cmd);
            self.device.end_command_buffer(cmd).map_err(err("end"))?;
            let fence = self
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(err("fence"))?;
            let cmds = [cmd];
            self.device
                .queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default().command_buffers(&cmds)],
                    fence,
                )
                .map_err(err("submit"))?;
            let r = self.device.wait_for_fences(&[fence], true, u64::MAX);
            self.device.destroy_fence(fence, None);
            self.device.free_command_buffers(self.pool, &cmds);
            r.map_err(err("wait (device lost?)"))
        }
    }

    /// Build an acceleration structure from `geometry` with `count` primitives.
    fn build_as(
        &self,
        ty: vk::AccelerationStructureTypeKHR,
        geometry: vk::AccelerationStructureGeometryKHR,
        count: u32,
    ) -> Result<(vk::AccelerationStructureKHR, Buf), String> {
        unsafe {
            let geoms = [geometry];
            let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(ty)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .geometries(&geoms);
            let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
            self.asx.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build,
                &[count],
                &mut sizes,
            );
            let store = self.buffer(
                sizes.acceleration_structure_size,
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
            )?;
            let acc = self
                .asx
                .create_acceleration_structure(
                    &vk::AccelerationStructureCreateInfoKHR::default()
                        .buffer(store.buffer)
                        .size(sizes.acceleration_structure_size)
                        .ty(ty),
                    None,
                )
                .map_err(err("create as"))?;
            let scratch = self.buffer(
                sizes.build_scratch_size + 256,
                vk::BufferUsageFlags::STORAGE_BUFFER,
            )?;
            let addr = (self.address(&scratch) + 255) & !255;
            build =
                build
                    .dst_acceleration_structure(acc)
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: addr,
                    });
            let range =
                [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(count)];
            let ranges: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&range];
            self.run(|cmd| {
                self.asx
                    .cmd_build_acceleration_structures(cmd, &[build], &ranges);
            })?;
            self.free(scratch);
            Ok((acc, store))
        }
    }
}

/// Trace `setup` to convergence on the first GPU with hardware ray queries.
pub fn render(setup: &Setup) -> Result<Output, String> {
    let started = std::time::Instant::now();
    let gpu = Gpu::new()?;
    let (w, h) = (setup.width.max(1), setup.height.max(1));
    let n_px = (w * h) as usize;
    // Triangles: one-sided first (culled from behind), then two-sided.
    let mut order: Vec<&Tri> = setup.tris.iter().filter(|t| !t.two_sided).collect();
    let one_sided = order.len() as u32;
    order.extend(setup.tris.iter().filter(|t| t.two_sided));
    let verts: Vec<[f32; 3]> = order.iter().flat_map(|t| t.positions).collect();
    let shade: Vec<[f32; 12]> = order
        .iter()
        .map(|t| {
            let n = t.normals;
            [
                n[0][0],
                n[0][1],
                n[0][2],
                t.albedo[0],
                n[1][0],
                n[1][1],
                n[1][2],
                t.albedo[1],
                n[2][0],
                n[2][1],
                n[2][2],
                t.albedo[2],
            ]
        })
        .collect();
    let lamps: Vec<[f32; 8]> = setup
        .lamps
        .iter()
        .map(|l| {
            let sun = l.direction.iter().map(|c| c * c).sum::<f32>() > 1.0e-8;
            let p = if sun { l.direction } else { l.position };
            [
                p[0],
                p[1],
                p[2],
                if sun { 0.0 } else { 1.0 },
                l.color[0],
                l.color[1],
                l.color[2],
                0.0,
            ]
        })
        .collect();
    let input = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
    let vbuf = gpu.upload(&verts, input)?;
    let vaddr = gpu.address(&vbuf);
    let flip = std::env::var("GENOS_GPUREF_FLIP").is_ok_and(|v| v == "1");
    let mut blas = Vec::new();
    let mut instances = Vec::new();
    for (first, count, two_sided) in [
        (0u32, one_sided, false),
        (one_sided, order.len() as u32 - one_sided, true),
    ] {
        if count == 0 {
            continue;
        }
        let tris = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_data(vk::DeviceOrHostAddressConstKHR {
                device_address: vaddr + first as u64 * 36,
            })
            .vertex_stride(12)
            .max_vertex(count * 3 - 1)
            .index_type(vk::IndexType::NONE_KHR);
        let geom = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
            .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: tris })
            .flags(vk::GeometryFlagsKHR::OPAQUE);
        let (acc, store) =
            gpu.build_as(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL, geom, count)?;
        let addr = unsafe {
            gpu.asx.get_acceleration_structure_device_address(
                &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                    .acceleration_structure(acc),
            )
        };
        let mut flags = vk::GeometryInstanceFlagsKHR::empty();
        if two_sided {
            flags |= vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE;
        }
        if flip {
            flags |= vk::GeometryInstanceFlagsKHR::TRIANGLE_FLIP_FACING;
        }
        instances.push(vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR {
                matrix: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            },
            instance_custom_index_and_mask: vk::Packed24_8::new(first, 0xFF),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                0,
                flags.as_raw() as u8,
            ),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                device_handle: addr,
            },
        });
        blas.push((acc, store));
    }
    let ibuf = gpu.upload(&instances, input)?;
    let inst = vk::AccelerationStructureGeometryInstancesDataKHR::default()
        .array_of_pointers(false)
        .data(vk::DeviceOrHostAddressConstKHR {
            device_address: gpu.address(&ibuf),
        });
    let geom = vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::INSTANCES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { instances: inst })
        .flags(vk::GeometryFlagsKHR::OPAQUE);
    let (tlas, tlas_store) = gpu.build_as(
        vk::AccelerationStructureTypeKHR::TOP_LEVEL,
        geom,
        instances.len() as u32,
    )?;
    let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
    let tbuf = gpu.upload(&shade, storage)?;
    let lbuf = gpu.upload(
        if lamps.is_empty() {
            &[[0f32; 8]][..]
        } else {
            &lamps[..]
        },
        storage,
    )?;
    let groups = groups(n_px);
    let obuf = gpu.buffer(n_px as u64 * 32 * groups as u64, storage)?;

    let result = unsafe { trace(&gpu, setup, tlas, &tbuf, &lbuf, &obuf, groups, started) };
    unsafe {
        gpu.device.device_wait_idle().ok();
        for b in [vbuf, ibuf, tbuf, lbuf, obuf, tlas_store] {
            gpu.free(b);
        }
        gpu.asx.destroy_acceleration_structure(tlas, None);
        for (a, s) in blas {
            gpu.asx.destroy_acceleration_structure(a, None);
            gpu.free(s);
        }
        gpu.device.destroy_command_pool(gpu.pool, None);
        gpu.device.destroy_device(None);
        gpu.instance.destroy_instance(None);
    }
    let _ = gpu.family;
    result
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Push {
    eye: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sky: [f32; 4],
    dims: [u32; 4],
}

unsafe fn trace(
    gpu: &Gpu,
    setup: &Setup,
    tlas: vk::AccelerationStructureKHR,
    tbuf: &Buf,
    lbuf: &Buf,
    obuf: &Buf,
    groups: u32,
    started: std::time::Instant,
) -> Result<Output, String> {
    let d = &gpu.device;
    let (w, h) = (setup.width.max(1), setup.height.max(1));
    let n_px = (w * h) as usize;
    let bindings: Vec<vk::DescriptorSetLayoutBinding> = (0..4)
        .map(|b| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(b)
                .descriptor_type(if b == 0 {
                    vk::DescriptorType::ACCELERATION_STRUCTURE_KHR
                } else {
                    vk::DescriptorType::STORAGE_BUFFER
                })
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect();
    let dsl = d
        .create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
        .map_err(err("dsl"))?;
    let ranges = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
        .size(std::mem::size_of::<Push>() as u32)];
    let dsls = [dsl];
    let layout = d
        .create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&dsls)
                .push_constant_ranges(&ranges),
            None,
        )
        .map_err(err("layout"))?;
    let words: Vec<u32> = SPV
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let module = d
        .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
        .map_err(err("module"))?;
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(c"main");
    let pipe = d
        .create_compute_pipelines(
            vk::PipelineCache::null(),
            &[vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(layout)],
            None,
        )
        .map_err(|e| format!("gpuref: pipeline: {:?}", e.1))?[0];
    let sizes = [
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1),
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(3),
    ];
    let dpool = d
        .create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&sizes),
            None,
        )
        .map_err(err("dpool"))?;
    let set = d
        .allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(dpool)
                .set_layouts(&dsls),
        )
        .map_err(err("set"))?[0];
    let tlases = [tlas];
    let mut as_write =
        vk::WriteDescriptorSetAccelerationStructureKHR::default().acceleration_structures(&tlases);
    let mut writes = vec![vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
        .descriptor_count(1)
        .push_next(&mut as_write)];
    let infos: Vec<[vk::DescriptorBufferInfo; 1]> = [tbuf, lbuf, obuf]
        .iter()
        .map(|b| {
            [vk::DescriptorBufferInfo::default()
                .buffer(b.buffer)
                .range(vk::WHOLE_SIZE)]
        })
        .collect();
    for (i, info) in infos.iter().enumerate() {
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(i as u32 + 1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(info),
        );
    }
    d.update_descriptor_sets(&writes, &[]);

    let tan_y = (setup.fov_y_deg.to_radians() * 0.5).tan();
    let aspect = w as f32 / h as f32;
    let paths = setup.paths_per_pass.max(1);
    let seed = 0x5EEDu32;
    // Per pixel per half: f64 sums of radiance and of squared luminance.
    let mut sum = vec![[0.0f64; 4]; 2 * n_px];
    let mut hit = vec![true; n_px];
    let mut done: u64 = 0;
    let mut pass = 0u32;
    let (mut noise, mut noise_halves);
    loop {
        let push = Push {
            eye: [setup.eye[0], setup.eye[1], setup.eye[2], tan_y],
            forward: [setup.forward[0], setup.forward[1], setup.forward[2], aspect],
            right: [
                setup.right[0],
                setup.right[1],
                setup.right[2],
                setup.lamps.len() as f32,
            ],
            up: [
                setup.up[0],
                setup.up[1],
                setup.up[2],
                setup.max_bounces as f32,
            ],
            sky: [setup.sky[0], setup.sky[1], setup.sky[2], pass as f32],
            dims: [w, h, paths, seed],
        };
        // Tiles of 64 rows so no one submit runs long enough to trip the driver's watchdog.
        gpu.run(|cmd| {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
            d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, layout, 0, &[set], &[]);
            let bytes = std::slice::from_raw_parts(
                &push as *const Push as *const u8,
                std::mem::size_of::<Push>(),
            );
            d.cmd_push_constants(cmd, layout, vk::ShaderStageFlags::COMPUTE, 0, bytes);
            d.cmd_dispatch(cmd, w.div_ceil(8), h.div_ceil(8), groups);
        })?;
        let px =
            std::slice::from_raw_parts(obuf.ptr as *const [f32; 4], 2 * n_px * groups as usize);
        for g in 0..groups as usize {
            let px = &px[2 * n_px * g..];
            for i in 0..n_px {
                if px[2 * i][0] < 0.0 {
                    hit[i] = false;
                    continue;
                }
                for half in 0..2 {
                    for c in 0..4 {
                        sum[2 * i + half][c] += px[2 * i + half][c] as f64;
                    }
                }
            }
        }
        done += 2 * paths as u64 * groups as u64;
        pass += 1;
        (noise, noise_halves) = measure(&sum, &hit, done);
        let out_of_time = started.elapsed().as_secs_f32() >= setup.seconds;
        if noise <= setup.noise_target
            || done + 2 * paths as u64 * groups as u64 > setup.max_spp
            || out_of_time
        {
            break;
        }
    }
    let half = done as f64 / 2.0;
    let mut linear = vec![[0.0f32; 3]; n_px];
    let mut half_a = vec![[0.0f32; 3]; n_px];
    let mut half_b = vec![[0.0f32; 3]; n_px];
    for i in 0..n_px {
        if !hit[i] {
            linear[i] = setup.sky;
            half_a[i] = setup.sky;
            half_b[i] = setup.sky;
            continue;
        }
        for c in 0..3 {
            let (a, b) = (sum[2 * i][c], sum[2 * i + 1][c]);
            linear[i][c] = ((a + b) / done as f64) as f32;
            half_a[i][c] = (a / half) as f32;
            half_b[i][c] = (b / half) as f32;
        }
    }
    d.destroy_descriptor_pool(dpool, None);
    d.destroy_pipeline(pipe, None);
    d.destroy_shader_module(module, None);
    d.destroy_pipeline_layout(layout, None);
    d.destroy_descriptor_set_layout(dsl, None);
    Ok(Output {
        linear,
        half_a,
        half_b,
        hit,
        spp: done,
        seconds: started.elapsed().as_secs_f32(),
        noise,
        noise_halves,
        device: gpu.name.clone(),
    })
}

/// Sample groups per dispatch: enough threads (about 512 K) to keep the GPU busy.
fn groups(n_px: usize) -> u32 {
    (524288usize.div_ceil(n_px.max(1))).clamp(1, 256) as u32
}

/// (the CPU proof's noise measure, the two-half measure) over hit pixels.
fn measure(sum: &[[f64; 4]], hit: &[bool], done: u64) -> (f32, f32) {
    let n = done as f64;
    let half = n / 2.0;
    let (mut total, mut total_h, mut count) = (0.0f64, 0.0f64, 0usize);
    let luma = |s: &[f64; 4]| 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2];
    for i in 0..hit.len() {
        if !hit[i] {
            continue;
        }
        let (a, b) = (&sum[2 * i], &sum[2 * i + 1]);
        let mean = (luma(a) + luma(b)) / n;
        let sq = (a[3] + b[3]) / n;
        let var = (sq - mean * mean).max(0.0);
        total += (var / n).sqrt() / mean.max(1.0e-4);
        let (ma, mb) = (luma(a) / half, luma(b) / half);
        total_h += (ma - mb).abs() / 2.0 * (std::f64::consts::PI / 2.0).sqrt() / mean.max(1.0e-4);
        count += 1;
    }
    let c = count.max(1) as f64;
    ((total / c) as f32, (total_h / c) as f32)
}
