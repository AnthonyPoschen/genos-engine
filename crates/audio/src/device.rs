//! Device backends. Each one writes an already mixed stereo buffer.

/// Submit interleaved 16-bit stereo PCM. The return value is frames the device consumed.
pub fn submit(interleaved: &[i16], device_rate: u32) -> Result<u64, String> {
    if device_rate == 0 || interleaved.len() % 2 != 0 {
        return Err("audio submit needs a stereo buffer and a device rate".into());
    }
    #[cfg(target_os = "linux")]
    {
        return alsa::play(interleaved, device_rate);
    }
    #[cfg(target_os = "windows")]
    {
        return wasapi::play(interleaved, device_rate);
    }
    #[cfg(target_os = "macos")]
    {
        return core_audio::play(interleaved, device_rate);
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = interleaved;
        Err("this operating system has no audio device backend".into())
    }
}

#[cfg(target_os = "linux")]
mod alsa {
    use std::ffi::c_void;
    use std::os::raw::c_char;

    const PLAYBACK: i32 = 0;
    const RW_INTERLEAVED: i32 = 3;
    const S16_LE: i32 = 2;

    #[link(name = "asound")]
    extern "C" {
        fn snd_pcm_open(pcm: *mut *mut c_void, name: *const c_char, stream: i32, mode: i32) -> i32;
        fn snd_pcm_set_params(
            pcm: *mut c_void,
            format: i32,
            access: i32,
            channels: u32,
            rate: u32,
            soft_resample: i32,
            latency: u32,
        ) -> i32;
        fn snd_pcm_writei(pcm: *mut c_void, buffer: *const c_void, size: u64) -> i64;
        fn snd_pcm_recover(pcm: *mut c_void, err: i32, silent: i32) -> i32;
        fn snd_pcm_drain(pcm: *mut c_void) -> i32;
        fn snd_pcm_close(pcm: *mut c_void) -> i32;
        fn snd_strerror(err: i32) -> *const c_char;
    }

    pub fn play(interleaved: &[i16], device_rate: u32) -> Result<u64, String> {
        unsafe {
            let name = b"default\0";
            let mut pcm = std::ptr::null_mut();
            let opened = snd_pcm_open(
                std::ptr::addr_of_mut!(pcm),
                name.as_ptr().cast(),
                PLAYBACK,
                0,
            );
            if opened < 0 {
                return Err(format!("ALSA open failed: {}", error_text(opened)));
            }
            let params =
                snd_pcm_set_params(pcm, S16_LE, RW_INTERLEAVED, 2, device_rate, 0, 200_000);
            if params < 0 {
                snd_pcm_close(pcm);
                return Err(format!("ALSA params failed: {}", error_text(params)));
            }
            let frames = interleaved.len() / 2;
            let mut done = 0usize;
            while done < frames {
                let wrote = snd_pcm_writei(
                    pcm,
                    interleaved[done * 2..].as_ptr().cast(),
                    (frames - done) as u64,
                );
                if wrote < 0 {
                    let recovered = snd_pcm_recover(pcm, wrote as i32, 1);
                    if recovered < 0 {
                        snd_pcm_close(pcm);
                        return Err(format!("ALSA write failed: {}", error_text(wrote as i32)));
                    }
                    continue;
                }
                done += wrote as usize;
            }
            let drained = snd_pcm_drain(pcm);
            snd_pcm_close(pcm);
            if drained < 0 {
                return Err(format!("ALSA drain failed: {}", error_text(drained)));
            }
            Ok(done as u64)
        }
    }

    unsafe fn error_text(code: i32) -> String {
        let text = snd_strerror(code);
        if text.is_null() {
            return format!("{code}");
        }
        std::ffi::CStr::from_ptr(text)
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(target_os = "windows")]
mod wasapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    struct WaveFormat {
        tag: u16,
        channels: u16,
        rate: u32,
        byte_rate: u32,
        align: u16,
        bits: u16,
        extra: u16,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            ctx: u32,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
        fn CoUninitialize();
    }

    const CLSID_ENUMERATOR: Guid = Guid {
        data1: 0xBCDE0395,
        data2: 0xE52F,
        data3: 0x467C,
        data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
    };
    const IID_ENUMERATOR: Guid = Guid {
        data1: 0xA95664D2,
        data2: 0x9614,
        data3: 0x4F35,
        data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
    };
    const IID_AUDIO_CLIENT: Guid = Guid {
        data1: 0x1CB9AD4C,
        data2: 0xDBFA,
        data3: 0x4C32,
        data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
    };
    const IID_RENDER_CLIENT: Guid = Guid {
        data1: 0xF294ACFC,
        data2: 0x3146,
        data3: 0x4483,
        data4: [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2],
    };

    pub fn play(interleaved: &[i16], device_rate: u32) -> Result<u64, String> {
        unsafe {
            let start = CoInitializeEx(std::ptr::null_mut(), 0);
            if start < 0 {
                return Err(format!("CoInitializeEx failed: {start}"));
            }
            let result = play_inner(interleaved, device_rate);
            CoUninitialize();
            result
        }
    }

    unsafe fn play_inner(interleaved: &[i16], device_rate: u32) -> Result<u64, String> {
        let mut enumerator = std::ptr::null_mut();
        let created = CoCreateInstance(
            &CLSID_ENUMERATOR,
            std::ptr::null_mut(),
            23,
            &IID_ENUMERATOR,
            std::ptr::addr_of_mut!(enumerator),
        );
        if created < 0 {
            return Err(format!("CoCreateInstance failed: {created}"));
        }
        let mut device = std::ptr::null_mut();
        let endpoint: unsafe extern "system" fn(*mut c_void, i32, i32, *mut *mut c_void) -> i32 =
            vcall(enumerator, 4);
        let got = endpoint(enumerator, 0, 0, std::ptr::addr_of_mut!(device));
        if got < 0 {
            release(enumerator);
            return Err(format!("GetDefaultAudioEndpoint failed: {got}"));
        }
        let mut client = std::ptr::null_mut();
        let activate: unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            u32,
            *mut c_void,
            *mut *mut c_void,
        ) -> i32 = vcall(device, 3);
        let activated = activate(
            device,
            &IID_AUDIO_CLIENT,
            23,
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(client),
        );
        if activated < 0 {
            release(device);
            release(enumerator);
            return Err(format!("Activate IAudioClient failed: {activated}"));
        }
        let frames = interleaved.len() / 2;
        let align = 4u32;
        let format = WaveFormat {
            tag: 1,
            channels: 2,
            rate: device_rate,
            byte_rate: device_rate * align,
            align: align as u16,
            bits: 16,
            extra: 0,
        };
        let duration = i64::from(frames as u32) * 10_000_000 / i64::from(device_rate);
        let init: unsafe extern "system" fn(
            *mut c_void,
            u32,
            u32,
            i64,
            i64,
            *const WaveFormat,
            *const Guid,
        ) -> i32 = vcall(client, 3);
        let initialized = init(
            client,
            0,
            0,
            duration.max(10_000_000),
            0,
            &format,
            std::ptr::null(),
        );
        if initialized < 0 {
            release(client);
            release(device);
            release(enumerator);
            return Err(format!("IAudioClient::Initialize failed: {initialized}"));
        }
        let mut capacity = 0u32;
        let buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = vcall(client, 4);
        let sized = buffer_size(client, std::ptr::addr_of_mut!(capacity));
        if sized < 0 || capacity == 0 {
            release(client);
            release(device);
            release(enumerator);
            return Err(format!("GetBufferSize failed: {sized}"));
        }
        let mut render = std::ptr::null_mut();
        let service: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            vcall(client, 14);
        let served = service(client, &IID_RENDER_CLIENT, std::ptr::addr_of_mut!(render));
        if served < 0 {
            release(client);
            release(device);
            release(enumerator);
            return Err(format!("GetService IAudioRenderClient failed: {served}"));
        }
        let written = fill(client, render, interleaved, capacity)?;
        let start: unsafe extern "system" fn(*mut c_void) -> i32 = vcall(client, 10);
        let started = start(client);
        if started < 0 {
            release(render);
            release(client);
            release(device);
            release(enumerator);
            return Err(format!("IAudioClient::Start failed: {started}"));
        }
        let drained = wait_until_empty(client, capacity);
        let stop: unsafe extern "system" fn(*mut c_void) -> i32 = vcall(client, 11);
        let _ = stop(client);
        release(render);
        release(client);
        release(device);
        release(enumerator);
        drained?;
        Ok(written)
    }

    unsafe fn fill(
        client: *mut c_void,
        render: *mut c_void,
        interleaved: &[i16],
        capacity: u32,
    ) -> Result<u64, String> {
        let frames = interleaved.len() / 2;
        let mut done = 0usize;
        let padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = vcall(client, 6);
        let get_buffer: unsafe extern "system" fn(*mut c_void, u32, *mut *mut i16) -> i32 =
            vcall(render, 3);
        let release_buffer: unsafe extern "system" fn(*mut c_void, u32, u32) -> i32 =
            vcall(render, 4);
        while done < frames {
            let mut pad = 0u32;
            let padded = padding(client, std::ptr::addr_of_mut!(pad));
            if padded < 0 {
                return Err(format!("GetCurrentPadding failed: {padded}"));
            }
            let room = capacity.saturating_sub(pad) as usize;
            if room == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            let count = room.min(frames - done);
            let mut dest = std::ptr::null_mut();
            let got = get_buffer(render, count as u32, std::ptr::addr_of_mut!(dest));
            if got < 0 || dest.is_null() {
                return Err(format!("IAudioRenderClient::GetBuffer failed: {got}"));
            }
            std::ptr::copy_nonoverlapping(interleaved[done * 2..].as_ptr(), dest, count * 2);
            let released = release_buffer(render, count as u32, 0);
            if released < 0 {
                return Err(format!("ReleaseBuffer failed: {released}"));
            }
            done += count;
        }
        Ok(done as u64)
    }

    unsafe fn wait_until_empty(client: *mut c_void, capacity: u32) -> Result<(), String> {
        let padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = vcall(client, 6);
        for _ in 0..400 {
            let mut pad = 0u32;
            let padded = padding(client, std::ptr::addr_of_mut!(pad));
            if padded < 0 {
                return Err(format!("GetCurrentPadding failed: {padded}"));
            }
            if pad == 0 {
                return Ok(());
            }
            let _ = capacity;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("WASAPI did not consume the buffer".into())
    }

    unsafe fn vcall<T>(this: *mut c_void, index: usize) -> T {
        let table = *(this as *const *const usize);
        std::mem::transmute_copy(&*table.add(index))
    }

    unsafe fn release(this: *mut c_void) {
        if this.is_null() {
            return;
        }
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = vcall(this, 2);
        let _ = release(this);
    }
}

#[cfg(target_os = "macos")]
mod core_audio {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[repr(C)]
    struct StreamDesc {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels: u32,
        bits: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct QueueBuffer {
        capacity: u32,
        data: *mut u8,
        size: u32,
        user: *mut c_void,
        packet_capacity: u32,
        packets: *mut c_void,
    }

    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioQueueNewOutput(
            format: *const StreamDesc,
            callback: extern "C" fn(*mut c_void, *mut c_void, *mut QueueBuffer),
            user: *mut c_void,
            run_loop: *mut c_void,
            mode: *mut c_void,
            flags: u32,
            queue: *mut *mut c_void,
        ) -> i32;
        fn AudioQueueAllocateBuffer(
            queue: *mut c_void,
            size: u32,
            buffer: *mut *mut QueueBuffer,
        ) -> i32;
        fn AudioQueueEnqueueBuffer(
            queue: *mut c_void,
            buffer: *mut QueueBuffer,
            packets: u32,
            descriptions: *const c_void,
        ) -> i32;
        fn AudioQueueStart(queue: *mut c_void, start_time: *const c_void) -> i32;
        fn AudioQueueStop(queue: *mut c_void, immediate: u8) -> i32;
        fn AudioQueueDispose(queue: *mut c_void, immediate: u8) -> i32;
    }

    extern "C" fn finished(user: *mut c_void, _queue: *mut c_void, buffer: *mut QueueBuffer) {
        if user.is_null() || buffer.is_null() {
            return;
        }
        unsafe {
            let frames = (*buffer).size / 4;
            let slot = &*(user as *const AtomicU32);
            slot.store(frames, Ordering::Release);
        }
    }

    pub fn play(interleaved: &[i16], device_rate: u32) -> Result<u64, String> {
        unsafe {
            let format = StreamDesc {
                sample_rate: f64::from(device_rate),
                format_id: 0x6c70_636d,
                format_flags: 4 | 8,
                bytes_per_packet: 4,
                frames_per_packet: 1,
                bytes_per_frame: 4,
                channels: 2,
                bits: 16,
                reserved: 0,
            };
            let done = AtomicU32::new(u32::MAX);
            let mut queue = std::ptr::null_mut();
            let made = AudioQueueNewOutput(
                &format,
                finished,
                std::ptr::addr_of!(done) as *mut c_void,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                std::ptr::addr_of_mut!(queue),
            );
            if made != 0 {
                return Err(format!("AudioQueueNewOutput failed: {made}"));
            }
            let bytes = interleaved.len() * 2;
            let mut buffer = std::ptr::null_mut();
            let allocated =
                AudioQueueAllocateBuffer(queue, bytes as u32, std::ptr::addr_of_mut!(buffer));
            if allocated != 0 || buffer.is_null() {
                AudioQueueDispose(queue, 1);
                return Err(format!("AudioQueueAllocateBuffer failed: {allocated}"));
            }
            std::ptr::copy_nonoverlapping(interleaved.as_ptr() as *const u8, (*buffer).data, bytes);
            (*buffer).size = bytes as u32;
            let queued = AudioQueueEnqueueBuffer(queue, buffer, 0, std::ptr::null());
            if queued != 0 {
                AudioQueueDispose(queue, 1);
                return Err(format!("AudioQueueEnqueueBuffer failed: {queued}"));
            }
            let started = AudioQueueStart(queue, std::ptr::null());
            if started != 0 {
                AudioQueueDispose(queue, 1);
                return Err(format!("AudioQueueStart failed: {started}"));
            }
            let frames = interleaved.len() / 2;
            let wait_ms = (u64::from(frames) * 1000 / u64::from(device_rate)).saturating_add(500);
            let mut saw = u32::MAX;
            for _ in 0..wait_ms / 10 {
                saw = done.load(Ordering::Acquire);
                if saw != u32::MAX {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let _ = AudioQueueStop(queue, 0);
            AudioQueueDispose(queue, 1);
            if saw == u32::MAX {
                return Err("CoreAudio did not consume the buffer".into());
            }
            Ok(u64::from(saw))
        }
    }
}
