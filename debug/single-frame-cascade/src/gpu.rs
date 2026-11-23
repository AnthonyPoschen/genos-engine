//! GPU timing for the prototype (`--gpu-bench`): the cone march as a compute
//! shader over voxel textures, and the ray version on the hardware ray tracer
//! (`genos-gpuref`). It opens its own headless Vulkan device, as the offline
//! reference does, and shares nothing with the renderer.

#![expect(
    clippy::disallowed_types,
    clippy::undocumented_unsafe_blocks,
    clippy::borrow_as_ptr,
    clippy::ptr_as_ptr,
    clippy::cast_ptr_alignment,
    reason = "offline benchmark on its own Vulkan device through ash, as genos-gpuref (ADR 0007 covers the renderer)"
)]

use ash::vk;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cone.spv"));

/// Texels along x and y of one voxel level's clip volume; z holds 8 levels.
pub const SIDE: u32 = 96;
pub const LEVELS: u32 = 8;

pub struct ConeInput<'a> {
    /// Two vec4 a pixel: point and cascade-0 ray length; normal and 1 when shown.
    pub surface: &'a [[f32; 8]],
    /// Seven RGBA16F volumes, `SIDE` x `SIDE` x `SIDE * LEVELS` texels.
    pub volumes: &'a [Vec<u16>; 7],
    pub clip: [[f32; 4]; 8],
    pub width0: f32,
    pub cascades: u32,
    pub sky: [f32; 3],
}

pub struct ConeTiming {
    pub device: String,
    pub ms_median: f64,
    pub ms_min: f64,
    pub steps_per_cone: f64,
    pub reads_per_cone: f64,
    pub covered: f64,
    pub cones: u64,
}

fn err<E: std::fmt::Debug>(what: &'static str) -> impl Fn(E) -> String {
    move |e| format!("gpu bench: {what}: {e:?}")
}

struct Device {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
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
            Ok(Self { _entry: entry, instance, device, phys, queue, pool, name })
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
                .device
                .create_buffer(&vk::BufferCreateInfo::default().size(size.max(16)).usage(usage), None)
                .map_err(err("buffer"))?;
            let req = self.device.get_buffer_memory_requirements(buffer);
            let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let mut memory = None;
            for want in [host | vk::MemoryPropertyFlags::DEVICE_LOCAL, host] {
                if let Some(ty) = self.memory_type(req.memory_type_bits, want) {
                    if let Ok(m) = self.device.allocate_memory(
                        &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),
                        None,
                    ) {
                        memory = Some(m);
                        break;
                    }
                }
            }
            let memory = memory.ok_or("gpu bench: no host-visible memory")?;
            self.device.bind_buffer_memory(buffer, memory, 0).map_err(err("bind buffer"))?;
            let ptr = self
                .device
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
                .device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default().command_pool(self.pool).command_buffer_count(1),
                )
                .map_err(err("cmd"))?[0];
            self.device
                .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
                .map_err(err("begin"))?;
            f(cmd);
            self.device.end_command_buffer(cmd).map_err(err("end"))?;
            let fence = self.device.create_fence(&vk::FenceCreateInfo::default(), None).map_err(err("fence"))?;
            let cmds = [cmd];
            let started = std::time::Instant::now();
            self.device
                .queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], fence)
                .map_err(err("submit"))?;
            let waited = self.device.wait_for_fences(&[fence], true, u64::MAX);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            self.device.destroy_fence(fence, None);
            self.device.free_command_buffers(self.pool, &cmds);
            waited.map_err(err("wait"))?;
            Ok(ms)
        }
    }

    /// One RGBA16F volume, filled from `texels`, ready to sample.
    fn volume(&self, texels: &[u16]) -> Result<vk::ImageView, String> {
        unsafe {
            let extent = vk::Extent3D { width: SIDE, height: SIDE, depth: SIDE * LEVELS };
            let image = self
                .device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_3D)
                        .format(vk::Format::R16G16B16A16_SFLOAT)
                        .extent(extent)
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST),
                    None,
                )
                .map_err(err("image"))?;
            let req = self.device.get_image_memory_requirements(image);
            let ty = self
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or("gpu bench: no device memory for a volume")?;
            let memory = self
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
                .map_err(err("volume memory"))?;
            self.device.bind_image_memory(image, memory, 0).map_err(err("bind image"))?;
            let bytes = std::mem::size_of_val(texels);
            let (staging, ptr) = self.buffer(bytes as u64, vk::BufferUsageFlags::TRANSFER_SRC)?;
            std::ptr::copy_nonoverlapping(texels.as_ptr().cast::<u8>(), ptr, bytes);
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
                self.device.cmd_pipeline_barrier(
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
                self.device.cmd_copy_buffer_to_image(
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
                self.device.cmd_pipeline_barrier(
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
            self.device.destroy_buffer(staging, None);
            self.device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_3D)
                        .format(vk::Format::R16G16B16A16_SFLOAT)
                        .subresource_range(range),
                    None,
                )
                .map_err(err("view"))
        }
    }
}

/// Time the cone march: one cone a pixel, `frames` dispatches after a warm-up.
pub fn time_cones(input: &ConeInput, frames: u32) -> Result<ConeTiming, String> {
    let gpu = Device::new()?;
    let pixels = input.surface.len();
    unsafe {
        let views: Vec<vk::ImageView> = input.volumes.iter().map(|v| gpu.volume(v)).collect::<Result<_, _>>()?;
        let sampler = gpu
            .device
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
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let surface_bytes = std::mem::size_of_val(input.surface);
        let (surface, surface_ptr) = gpu.buffer(surface_bytes as u64, storage)?;
        std::ptr::copy_nonoverlapping(input.surface.as_ptr().cast::<u8>(), surface_ptr, surface_bytes);
        let out_bytes = pixels * 5 * 16;
        let (out, out_ptr) = gpu.buffer(out_bytes as u64, storage)?;
        let mut params = [0f32; 40];
        for (level, clip) in input.clip.iter().enumerate() {
            params[level * 4..level * 4 + 4].copy_from_slice(clip);
        }
        params[32] = input.width0;
        params[33] = input.cascades as f32;
        params[34] = pixels as f32;
        params[36..39].copy_from_slice(&input.sky);
        let (uniform, uniform_ptr) = gpu.buffer(160, vk::BufferUsageFlags::UNIFORM_BUFFER)?;

        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(7)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let layout = gpu
            .device
            .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)
            .map_err(err("set layout"))?;
        let sizes = [
            vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 7 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_BUFFER, descriptor_count: 2 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::UNIFORM_BUFFER, descriptor_count: 1 },
        ];
        let pool = gpu
            .device
            .create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes), None)
            .map_err(err("descriptor pool"))?;
        let layouts = [layout];
        let set = gpu
            .device
            .allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts))
            .map_err(err("set"))?[0];
        let images: Vec<vk::DescriptorImageInfo> = views
            .iter()
            .map(|view| {
                vk::DescriptorImageInfo::default()
                    .sampler(sampler)
                    .image_view(*view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let whole = |buffer| [vk::DescriptorBufferInfo::default().buffer(buffer).range(vk::WHOLE_SIZE)];
        let (surface_info, out_info, uniform_info) = (whole(surface), whole(out), whole(uniform));
        gpu.device.update_descriptor_sets(
            &[
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&images),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&surface_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&out_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(3)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&uniform_info),
            ],
            &[],
        );
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV)).map_err(err("spirv"))?;
        let module = gpu
            .device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            .map_err(err("shader"))?;
        let pipe_layout = gpu
            .device
            .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts), None)
            .map_err(err("pipeline layout"))?;
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module)
            .name(c"main");
        let pipeline = gpu
            .device
            .create_compute_pipelines(
                vk::PipelineCache::null(),
                &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(pipe_layout)],
                None,
            )
            .map_err(|(_, e)| format!("gpu bench: pipeline: {e:?}"))?[0];
        let groups = u32::try_from(pixels.div_ceil(64)).map_err(err("groups"))?;
        let mut times = Vec::new();
        let warm = 5u32;
        for frame in 0..warm + frames.max(1) {
            params[35] = frame as f32;
            std::ptr::copy_nonoverlapping(params.as_ptr().cast::<u8>(), uniform_ptr, 160);
            let ms = gpu.run(|cmd| {
                gpu.device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipeline);
                gpu.device.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[set], &[]);
                gpu.device.cmd_dispatch(cmd, groups, 1, 1);
            })?;
            if frame >= warm {
                times.push(ms);
            }
        }
        times.sort_by(f64::total_cmp);
        let result = std::slice::from_raw_parts(out_ptr.cast::<[f32; 4]>(), pixels * 5);
        let (mut cones, mut steps, mut reads, mut covered) = (0u64, 0.0f64, 0.0f64, 0.0f64);
        for (p, surface) in input.surface.iter().enumerate() {
            if surface[7] > 0.5 {
                let stats = result[p * 5 + 4];
                cones += 1;
                steps += f64::from(stats[0]);
                reads += f64::from(stats[1]);
                covered += f64::from(stats[2]);
            }
        }
        let n = (cones.max(1)) as f64;
        gpu.device.device_wait_idle().ok();
        Ok(ConeTiming {
            device: gpu.name.clone(),
            ms_median: times[times.len() / 2],
            ms_min: times[0],
            steps_per_cone: steps / n,
            reads_per_cone: reads / n,
            covered: covered / n,
            cones,
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
