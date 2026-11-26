//! GPU timing for the prototype (`--gpu-bench`): the cone march as a compute
//! shader over voxel textures, and the ray version on the hardware ray tracer
//! (`genos-gpuref`). It opens its own headless Vulkan device, as the offline
//! reference does, and shares nothing with the renderer.

#![expect(
    clippy::disallowed_types,
    clippy::undocumented_unsafe_blocks,
    reason = "offline benchmark on its own Vulkan device through ash, as genos-gpuref (ADR 0007 covers the renderer)"
)]

use ash::vk;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cascade.spv"));

/// Texels along x and y of one voxel level's clip volume by default; z holds
/// the levels.
pub const SIDE: u32 = 96;
pub const LEVELS: u32 = 8;
/// Cones whose per-interval results are kept: the shader writes cone `i` to
/// slot `i % RING`, so every write is paid without a buffer for all of them.
const RING: u64 = 1 << 20;
/// Workgroups along x a dispatch stays within (the minimum spec limit).
const GROUPS_X: u32 = 65_535;

pub struct CascadeInput<'a> {
    /// Two vec4 a cone: its start and its first interval's reach; its direction.
    pub jobs: &'a [[f32; 8]],
    /// Seven RGBA16F volumes over all levels: six axis ways, then the lamps.
    pub faces: &'a [Vec<u16>; 7],
    /// Twelve RGBA8 volumes over the plane levels: six A, then six B.
    pub planes: &'a [Vec<u8>; 12],
    pub plane_levels: u32,
    /// Texels along each side of a level's clip volume.
    pub side: u32,
    /// One R8 volume over all levels: 255 where a cell holds a surface or
    /// is next to one.
    pub occupancy: &'a [u8],
    pub descend: bool,
    pub clip: [[f32; 4]; 8],
    pub width0: f32,
    pub cascades: u32,
    pub refine_level: u32,
    pub cross_full: f32,
    pub box_margin: f32,
    pub sky: [f32; 3],
    /// Each cone marches only its own cascade's interval.
    pub own_interval: bool,
    /// Cascades below this take the crossing test and the refine walk.
    pub refine_cascades: u32,
    /// Above 0, every interval's cone width a metre out (the gather's).
    pub fixed_width: f32,
}

pub struct CascadeTiming {
    pub device: String,
    pub ms_median: f64,
    pub ms_min: f64,
    pub cones: u64,
    pub steps_per_cone: f64,
    pub reads_per_cone: f64,
    pub covered: f64,
    pub texture_mib: f64,
    /// Each cone's cover at the end of its march.
    pub covers: Vec<f32>,
}

fn err<E: std::fmt::Debug>(what: &'static str) -> impl Fn(E) -> String {
    move |e| format!("gpu bench: {what}: {e:?}")
}

struct Device {
    _entry: ash::Entry,
    instance: ash::Instance,
    logical: ash::Device,
    phys: vk::PhysicalDevice,
    queue: vk::Queue,
    pool: vk::CommandPool,
    name: String,
}

impl Device {
    fn new() -> Result<Self, String> {
        unsafe {
            let entry = ash::Entry::load().map_err(err("load vulkan"))?;
            let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 2, 0));
            let instance = entry
                .create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None)
                .map_err(err("instance"))?;
            let mut pick = None;
            for phys in instance.enumerate_physical_devices().map_err(err("devices"))? {
                let props = instance.get_physical_device_properties(phys);
                let discrete = props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU;
                let family = instance
                    .get_physical_device_queue_family_properties(phys)
                    .iter()
                    .position(|f| f.queue_flags.contains(vk::QueueFlags::COMPUTE));
                if let Some(family) = family {
                    let name = props
                        .device_name_as_c_str()
                        .map_or_else(|_| String::new(), |c| c.to_string_lossy().into_owned());
                    if pick.is_none() || discrete {
                        pick = Some((phys, u32::try_from(family).unwrap_or(0), name));
                    }
                }
            }
            let (phys, family, name) = pick.ok_or("gpu bench: no device with a compute queue")?;
            let priority = [1.0f32];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priority)];
            let device = instance
                .create_device(phys, &vk::DeviceCreateInfo::default().queue_create_infos(&queues), None)
                .map_err(err("device"))?;
            let queue = device.get_device_queue(family, 0);
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(err("pool"))?;
            Ok(Self { _entry: entry, instance, logical: device, phys, queue, pool, name })
        }
    }

    fn memory_type(&self, bits: u32, want: vk::MemoryPropertyFlags) -> Option<u32> {
        let mem = unsafe { self.instance.get_physical_device_memory_properties(self.phys) };
        (0..mem.memory_type_count).find(|&i| {
            bits & (1 << i) != 0 && mem.memory_types[i as usize].property_flags.contains(want)
        })
    }

    /// A host-visible buffer, mapped.
    fn buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> Result<(vk::Buffer, *mut u8), String> {
        unsafe {
            let buffer = self
                .logical
                .create_buffer(&vk::BufferCreateInfo::default().size(size.max(16)).usage(usage), None)
                .map_err(err("buffer"))?;
            let req = self.logical.get_buffer_memory_requirements(buffer);
            let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let mut memory = None;
            for want in [host | vk::MemoryPropertyFlags::DEVICE_LOCAL, host] {
                if let Some(ty) = self.memory_type(req.memory_type_bits, want) {
                    if let Ok(m) = self.logical.allocate_memory(
                        &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),
                        None,
                    ) {
                        memory = Some(m);
                        break;
                    }
                }
            }
            let memory = memory.ok_or("gpu bench: no host-visible memory")?;
            self.logical.bind_buffer_memory(buffer, memory, 0).map_err(err("bind buffer"))?;
            let ptr = self
                .logical
                .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                .map_err(err("map"))?
                .cast::<u8>();
            Ok((buffer, ptr))
        }
    }

    /// Record with `f`, submit, wait. Returns the wall milliseconds of submit and wait.
    fn run(&self, f: impl FnOnce(vk::CommandBuffer)) -> Result<f64, String> {
        unsafe {
            let cmd = self
                .logical
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default().command_pool(self.pool).command_buffer_count(1),
                )
                .map_err(err("cmd"))?[0];
            self.logical
                .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
                .map_err(err("begin"))?;
            f(cmd);
            self.logical.end_command_buffer(cmd).map_err(err("end"))?;
            let fence = self.logical.create_fence(&vk::FenceCreateInfo::default(), None).map_err(err("fence"))?;
            let cmds = [cmd];
            let started = std::time::Instant::now();
            self.logical
                .queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], fence)
                .map_err(err("submit"))?;
            let waited = self.logical.wait_for_fences(&[fence], true, u64::MAX);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            self.logical.destroy_fence(fence, None);
            self.logical.free_command_buffers(self.pool, &cmds);
            waited.map_err(err("wait"))?;
            Ok(ms)
        }
    }

    /// One volume of `levels` stacked clip volumes, filled from `bytes`,
    /// ready to sample.
    fn volume(&self, bytes: &[u8], format: vk::Format, side: u32, levels: u32) -> Result<vk::ImageView, String> {
        unsafe {
            let extent = vk::Extent3D { width: side, height: side, depth: side * levels };
            let image = self
                .logical
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_3D)
                        .format(format)
                        .extent(extent)
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST),
                    None,
                )
                .map_err(err("image"))?;
            let req = self.logical.get_image_memory_requirements(image);
            let ty = self
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or("gpu bench: no device memory for a volume")?;
            let memory = self
                .logical
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
                .map_err(err("volume memory"))?;
            self.logical.bind_image_memory(image, memory, 0).map_err(err("bind image"))?;
            let (staging, ptr) = self.buffer(u64::try_from(bytes.len()).unwrap_or(u64::MAX), vk::BufferUsageFlags::TRANSFER_SRC)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);
            let barrier = |from, to, src, dst| {
                vk::ImageMemoryBarrier::default()
                    .old_layout(from)
                    .new_layout(to)
                    .src_access_mask(src)
                    .dst_access_mask(dst)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(range)
            };
            self.run(|cmd| {
                self.logical.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(
                        vk::ImageLayout::UNDEFINED,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::empty(),
                        vk::AccessFlags::TRANSFER_WRITE,
                    )],
                );
                self.logical.cmd_copy_buffer_to_image(
                    cmd,
                    staging,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(extent)],
                );
                self.logical.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        vk::AccessFlags::TRANSFER_WRITE,
                        vk::AccessFlags::SHADER_READ,
                    )],
                );
            })?;
            self.logical.destroy_buffer(staging, None);
            self.logical
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_3D)
                        .format(format)
                        .subresource_range(range),
                    None,
                )
                .map_err(err("view"))
        }
    }
}

impl Device {
    /// A device-local storage buffer of `size` bytes, filled from `bytes` if given.
    fn device_buffer(&self, size: u64, bytes: Option<&[u8]>) -> Result<vk::Buffer, String> {
        unsafe {
            let usage = vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::TRANSFER_SRC;
            let buffer = self
                .logical
                .create_buffer(&vk::BufferCreateInfo::default().size(size.max(16)).usage(usage), None)
                .map_err(err("device buffer"))?;
            let req = self.logical.get_buffer_memory_requirements(buffer);
            let ty = self
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or("gpu bench: no device memory for a buffer")?;
            let memory = self
                .logical
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
                .map_err(err("device buffer memory"))?;
            self.logical.bind_buffer_memory(buffer, memory, 0).map_err(err("bind device buffer"))?;
            if let Some(bytes) = bytes {
                let (staging, ptr) = self.buffer(u64::try_from(bytes.len()).unwrap_or(u64::MAX), vk::BufferUsageFlags::TRANSFER_SRC)?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
                self.run(|cmd| {
                    self.logical.cmd_copy_buffer(cmd, staging, buffer, &[vk::BufferCopy::default().size(u64::try_from(bytes.len()).unwrap_or(u64::MAX))]);
                })?;
                self.logical.destroy_buffer(staging, None);
            }
            Ok(buffer)
        }
    }

    /// The first `size` bytes of a device buffer, copied to the host.
    fn read_back(&self, buffer: vk::Buffer, size: u64) -> Result<Vec<u8>, String> {
        unsafe {
            let (host, ptr) = self.buffer(size, vk::BufferUsageFlags::TRANSFER_DST)?;
            self.run(|cmd| {
                self.logical.cmd_copy_buffer(cmd, buffer, host, &[vk::BufferCopy::default().size(size)]);
            })?;
            let out = std::slice::from_raw_parts(ptr, usize::try_from(size).unwrap_or(0)).to_vec();
            self.logical.destroy_buffer(host, None);
            Ok(out)
        }
    }
}

fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

/// Time the voxel-layer cascade's cones: every cone of one frame, each through
/// all its intervals, `frames` dispatches after a warm-up.
pub fn time_cascade(input: &CascadeInput, frames: u32) -> Result<CascadeTiming, String> {
    let gpu = Device::new()?;
    let cones = u64::try_from(input.jobs.len()).unwrap_or(u64::MAX);
    unsafe {
        let mut views = Vec::new();
        for faces in input.faces {
            views.push(gpu.volume(as_bytes(faces), vk::Format::R16G16B16A16_SFLOAT, input.side, LEVELS)?);
        }
        for planes in input.planes {
            views.push(gpu.volume(planes, vk::Format::R8G8B8A8_UNORM, input.side, input.plane_levels)?);
        }
        views.push(gpu.volume(input.occupancy, vk::Format::R8_UNORM, input.side, LEVELS)?);
        let texture_bytes = input.faces.iter().map(|f| std::mem::size_of_val(f.as_slice())).sum::<usize>()
            + input.planes.iter().map(Vec::len).sum::<usize>()
            + input.occupancy.len();
        let linear = gpu
            .logical
            .create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
            .map_err(err("sampler"))?;
        let jobs = gpu.device_buffer(std::mem::size_of_val(input.jobs) as u64, Some(as_bytes(input.jobs)))?;
        let ring = cones.min(RING) * u64::from(input.cascades) * 16;
        let results = gpu.device_buffer(ring, None)?;
        let stats = gpu.device_buffer(cones * 16, None)?;
        let mut params = [0f32; 48];
        for (level, clip) in input.clip.iter().enumerate() {
            params[level * 4..level * 4 + 4].copy_from_slice(clip);
        }
        params[32] = input.width0;
        params[33] = input.cascades as f32;
        params[34] = cones as f32;
        params[35] = input.refine_level as f32;
        params[36] = 1.0;
        params[37] = input.cross_full;
        params[38] = input.box_margin;
        params[39] = input.side as f32;
        params[40..43].copy_from_slice(&input.sky);
        params[43] = if input.own_interval { 1.0 } else { 0.0 };
        params[44] = input.refine_cascades.min(64) as f32;
        params[45] = if input.descend { 1.0 } else { 0.0 };
        params[46] = input.fixed_width;
        let (uniform, uniform_ptr) = gpu.buffer(192, vk::BufferUsageFlags::UNIFORM_BUFFER)?;
        std::ptr::copy_nonoverlapping(params.as_ptr().cast::<u8>(), uniform_ptr, 192);

        let binding = |b: u32, ty: vk::DescriptorType, count: u32| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(b)
                .descriptor_type(ty)
                .descriptor_count(count)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        };
        let image = vk::DescriptorType::COMBINED_IMAGE_SAMPLER;
        let storage = vk::DescriptorType::STORAGE_BUFFER;
        let bindings = [
            binding(0, image, 7),
            binding(1, image, 6),
            binding(2, image, 6),
            binding(3, storage, 1),
            binding(4, storage, 1),
            binding(5, storage, 1),
            binding(6, vk::DescriptorType::UNIFORM_BUFFER, 1),
            binding(7, image, 1),
        ];
        let layout = gpu
            .logical
            .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)
            .map_err(err("set layout"))?;
        let sizes = [
            vk::DescriptorPoolSize { ty: image, descriptor_count: 20 },
            vk::DescriptorPoolSize { ty: storage, descriptor_count: 3 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::UNIFORM_BUFFER, descriptor_count: 1 },
        ];
        let pool = gpu
            .logical
            .create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes), None)
            .map_err(err("descriptor pool"))?;
        let layouts = [layout];
        let set = gpu
            .logical
            .allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts))
            .map_err(err("set"))?[0];
        let images: Vec<vk::DescriptorImageInfo> = views
            .iter()
            .map(|view| {
                vk::DescriptorImageInfo::default()
                    .sampler(linear)
                    .image_view(*view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let whole = |buffer| [vk::DescriptorBufferInfo::default().buffer(buffer).range(vk::WHOLE_SIZE)];
        let (jobs_info, results_info, stats_info, uniform_info) = (whole(jobs), whole(results), whole(stats), whole(uniform));
        let write = |b: u32, ty: vk::DescriptorType| vk::WriteDescriptorSet::default().dst_set(set).dst_binding(b).descriptor_type(ty);
        gpu.logical.update_descriptor_sets(
            &[
                write(0, image).image_info(&images[0..7]),
                write(1, image).image_info(&images[7..13]),
                write(2, image).image_info(&images[13..19]),
                write(3, storage).buffer_info(&jobs_info),
                write(4, storage).buffer_info(&results_info),
                write(5, storage).buffer_info(&stats_info),
                write(6, vk::DescriptorType::UNIFORM_BUFFER).buffer_info(&uniform_info),
                write(7, image).image_info(&images[19..20]),
            ],
            &[],
        );
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV)).map_err(err("spirv"))?;
        let module = gpu
            .logical
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            .map_err(err("shader"))?;
        let pipe_layout = gpu
            .logical
            .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts), None)
            .map_err(err("pipeline layout"))?;
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module)
            .name(c"main");
        let pipeline = gpu
            .logical
            .create_compute_pipelines(
                vk::PipelineCache::null(),
                &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(pipe_layout)],
                None,
            )
            .map_err(|(_, e)| format!("gpu bench: pipeline: {e:?}"))?[0];
        let groups = u32::try_from(cones.div_ceil(64)).map_err(err("groups"))?;
        let (gx, gy) = (groups.min(GROUPS_X), groups.div_ceil(GROUPS_X).max(1));
        let mut times = Vec::new();
        let warm = 3u32;
        for frame in 0..warm + frames.max(1) {
            let ms = gpu.run(|cmd| {
                gpu.logical.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipeline);
                gpu.logical.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[set], &[]);
                gpu.logical.cmd_dispatch(cmd, gx, gy, 1);
            })?;
            if frame >= warm {
                times.push(ms);
            }
        }
        times.sort_by(f64::total_cmp);
        let bytes = gpu.read_back(stats, cones * 16)?;
        let per_cone: Vec<[f32; 4]> = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|c| std::array::from_fn(|i| f32::from_le_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]])))
            .collect();
        let n = (cones.max(1)) as f64;
        let sum = |k: usize| per_cone.iter().map(|s| f64::from(s[k])).sum::<f64>() / n;
        gpu.logical.device_wait_idle().ok();
        Ok(CascadeTiming {
            device: gpu.name.clone(),
            ms_median: times[times.len() / 2],
            ms_min: times[0],
            cones,
            steps_per_cone: sum(0),
            reads_per_cone: sum(1),
            covered: sum(2),
            texture_mib: texture_bytes as f64 / (1024.0 * 1024.0),
            covers: per_cone.iter().map(|s| s[2]).collect(),
        })
    }
}

/// Milliseconds the hardware ray tracer takes for one path a pixel: a camera
/// ray, a bounce ray, and a shadow ray to every lamp at both hits. The time is
/// the dispatches alone (submit and wait), without the host's sums.
pub fn time_rays(setup: &genos_gpuref::Setup) -> Result<(f64, String), String> {
    let mut s = setup.clone();
    s.paths_per_pass = 1;
    s.max_spp = 128;
    s.noise_target = 0.0;
    s.seconds = 600.0;
    let out = genos_gpuref::render(&s)?;
    Ok((f64::from(out.dispatch_ms) / f64::from(out.paths_per_dispatch.max(1)), out.device))
}
