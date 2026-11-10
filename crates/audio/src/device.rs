#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_is_multiple_of, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::undocumented_unsafe_blocks, reason = "TODO(lint-2026-10-14): Vulkan or OS FFI; add a SAFETY comment. docs/LINT_RULES.md")]

//! Held device outputs. Each one stays open and writes an already mixed buffer.

#[cfg(target_os = "linux")]
pub use alsa::AlsaDevice;
#[cfg(target_os = "windows")]
pub use wasapi::WasapiDevice;
#[cfg(target_os = "macos")]
pub use core_audio::CoreAudioDevice;

const PERIOD_FRAMES: u32 = 480;
const DEVICE_RATE: u32 = 48_000;

#[cfg(target_os = "linux")]
mod alsa {
    use std::ffi::c_void;
    use std::os::raw::c_char;

    use crate::path::{DevicePath, OpenedDevice};

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

    pub struct AlsaDevice {
        pcm: *mut c_void,
    }

    // SAFETY: the handle is used only on the `genos-audio` thread that calls `open`.
    unsafe impl Send for AlsaDevice {}

    impl AlsaDevice {
        #[must_use]
        pub fn new() -> Self {
            Self {
                pcm: std::ptr::null_mut(),
            }
        }
    }

    impl Default for AlsaDevice {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DevicePath for AlsaDevice {
        fn open(&mut self) -> Result<OpenedDevice, String> {
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
                let latency_us = super::PERIOD_FRAMES.saturating_mul(1_000_000) / super::DEVICE_RATE;
                let params = snd_pcm_set_params(
                    pcm,
                    S16_LE,
                    RW_INTERLEAVED,
                    2,
                    super::DEVICE_RATE,
                    0,
                    latency_us,
                );
                if params < 0 {
                    snd_pcm_close(pcm);
                    return Err(format!("ALSA params failed: {}", error_text(params)));
                }
                self.pcm = pcm;
                Ok(OpenedDevice {
                    rate: super::DEVICE_RATE,
                    period_frames: super::PERIOD_FRAMES,
                    channels: 2,
                })
            }
        }

        fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str> {
            if self.pcm.is_null() || interleaved.len() % 2 != 0 {
                return Err("xrun");
            }
            let frames = interleaved.len() / 2;
            let mut done = 0usize;
            while done < frames {
                let wrote = unsafe {
                    snd_pcm_writei(
                        self.pcm,
                        interleaved[done * 2..].as_ptr().cast(),
                        (frames - done) as u64,
                    )
                };
                if wrote < 0 {
                    let recovered = unsafe { snd_pcm_recover(self.pcm, wrote as i32, 1) };
                    if recovered < 0 {
                        return Err("xrun");
                    }
                    return Err("xrun");
                }
                done += wrote as usize;
            }
            Ok(())
        }

        fn close(&mut self) {
            if self.pcm.is_null() {
                return;
            }
            unsafe {
                let _ = snd_pcm_drain(self.pcm);
                snd_pcm_close(self.pcm);
            }
            self.pcm = std::ptr::null_mut();
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

    use crate::path::{DevicePath, OpenedDevice};

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

    pub struct WasapiDevice {
        enumerator: *mut c_void,
        endpoint: *mut c_void,
        client: *mut c_void,
        render: *mut c_void,
        capacity: u32,
        com: bool,
    }

    // SAFETY: the COM pointers are used only on the thread that calls `open`.
    unsafe impl Send for WasapiDevice {}

    impl WasapiDevice {
        #[must_use]
        pub fn new() -> Self {
            Self {
                enumerator: std::ptr::null_mut(),
                endpoint: std::ptr::null_mut(),
                client: std::ptr::null_mut(),
                render: std::ptr::null_mut(),
                capacity: 0,
                com: false,
            }
        }

        fn fail(&mut self, err: String) -> Result<OpenedDevice, String> {
            self.close();
            Err(err)
        }
    }

    impl Default for WasapiDevice {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DevicePath for WasapiDevice {
        fn open(&mut self) -> Result<OpenedDevice, String> {
            unsafe {
                let start = CoInitializeEx(std::ptr::null_mut(), 0);
                if start < 0 {
                    return Err(format!("CoInitializeEx failed: {start}"));
                }
                self.com = true;
                let created = CoCreateInstance(
                    &CLSID_ENUMERATOR,
                    std::ptr::null_mut(),
                    23,
                    &IID_ENUMERATOR,
                    std::ptr::addr_of_mut!(self.enumerator),
                );
                if created < 0 {
                    return self.fail(format!("CoCreateInstance failed: {created}"));
                }
                let endpoint: unsafe extern "system" fn(
                    *mut c_void,
                    i32,
                    i32,
                    *mut *mut c_void,
                ) -> i32 = vcall(self.enumerator, 4);
                let got = endpoint(self.enumerator, 0, 0, std::ptr::addr_of_mut!(self.endpoint));
                if got < 0 {
                    return self.fail(format!("GetDefaultAudioEndpoint failed: {got}"));
                }
                let activate: unsafe extern "system" fn(
                    *mut c_void,
                    *const Guid,
                    u32,
                    *mut c_void,
                    *mut *mut c_void,
                ) -> i32 = vcall(self.endpoint, 3);
                let activated = activate(
                    self.endpoint,
                    &IID_AUDIO_CLIENT,
                    23,
                    std::ptr::null_mut(),
                    std::ptr::addr_of_mut!(self.client),
                );
                if activated < 0 {
                    return self.fail(format!("Activate IAudioClient failed: {activated}"));
                }
                let align = 4u32;
                let format = WaveFormat {
                    tag: 1,
                    channels: 2,
                    rate: super::DEVICE_RATE,
                    byte_rate: super::DEVICE_RATE * align,
                    align: align as u16,
                    bits: 16,
                    extra: 0,
                };
                let duration = i64::from(super::PERIOD_FRAMES) * 10_000_000
                    / i64::from(super::DEVICE_RATE);
                let init: unsafe extern "system" fn(
                    *mut c_void,
                    u32,
                    u32,
                    i64,
                    i64,
                    *const WaveFormat,
                    *const Guid,
                ) -> i32 = vcall(self.client, 3);
                let initialized = init(
                    self.client,
                    0,
                    0,
                    duration,
                    0,
                    &format,
                    std::ptr::null(),
                );
                if initialized < 0 {
                    return self.fail(format!("IAudioClient::Initialize failed: {initialized}"));
                }
                let buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 =
                    vcall(self.client, 4);
                let sized = buffer_size(self.client, std::ptr::addr_of_mut!(self.capacity));
                if sized < 0 || self.capacity == 0 {
                    return self.fail(format!("GetBufferSize failed: {sized}"));
                }
                let service: unsafe extern "system" fn(
                    *mut c_void,
                    *const Guid,
                    *mut *mut c_void,
                ) -> i32 = vcall(self.client, 14);
                let served = service(
                    self.client,
                    &IID_RENDER_CLIENT,
                    std::ptr::addr_of_mut!(self.render),
                );
                if served < 0 {
                    return self.fail(format!("GetService IAudioRenderClient failed: {served}"));
                }
                let start: unsafe extern "system" fn(*mut c_void) -> i32 = vcall(self.client, 10);
                let started = start(self.client);
                if started < 0 {
                    return self.fail(format!("IAudioClient::Start failed: {started}"));
                }
                Ok(OpenedDevice {
                    rate: super::DEVICE_RATE,
                    period_frames: super::PERIOD_FRAMES,
                    channels: 2,
                })
            }
        }

        fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str> {
            if self.client.is_null() || self.render.is_null() {
                return Err("xrun");
            }
            unsafe { write_quantum(self.client, self.render, interleaved, self.capacity) }
        }

        fn close(&mut self) {
            unsafe {
                if !self.client.is_null() {
                    let stop: unsafe extern "system" fn(*mut c_void) -> i32 = vcall(self.client, 11);
                    let _ = stop(self.client);
                }
                release(self.render);
                release(self.client);
                release(self.endpoint);
                release(self.enumerator);
                if self.com {
                    CoUninitialize();
                }
            }
            self.render = std::ptr::null_mut();
            self.client = std::ptr::null_mut();
            self.endpoint = std::ptr::null_mut();
            self.enumerator = std::ptr::null_mut();
            self.com = false;
        }
    }

    unsafe fn write_quantum(
        client: *mut c_void,
        render: *mut c_void,
        interleaved: &[i16],
        capacity: u32,
    ) -> Result<(), &'static str> {
        let frames = interleaved.len() / 2;
        let padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = vcall(client, 6);
        let mut pad = 0u32;
        if padding(client, std::ptr::addr_of_mut!(pad)) < 0 {
            return Err("xrun");
        }
        let room = capacity.saturating_sub(pad);
        if usize::try_from(room).unwrap_or(0) < frames {
            return Err("xrun");
        }
        let get_buffer: unsafe extern "system" fn(*mut c_void, u32, *mut *mut i16) -> i32 =
            vcall(render, 3);
        let release_buffer: unsafe extern "system" fn(*mut c_void, u32, u32) -> i32 =
            vcall(render, 4);
        let mut dest = std::ptr::null_mut();
        let count = u32::try_from(frames).unwrap_or(0);
        if get_buffer(render, count, std::ptr::addr_of_mut!(dest)) < 0 || dest.is_null() {
            return Err("xrun");
        }
        std::ptr::copy_nonoverlapping(interleaved.as_ptr(), dest, frames * 2);
        if release_buffer(render, count, 0) < 0 {
            return Err("xrun");
        }
        Ok(())
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::path::{DevicePath, OpenedDevice};

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

    extern "C" fn finished(user: *mut c_void, _queue: *mut c_void, _buffer: *mut QueueBuffer) {
        if user.is_null() {
            return;
        }
        unsafe {
            let slot = &*(user as *const AtomicBool);
            slot.store(true, Ordering::Release);
        }
    }

    pub struct CoreAudioDevice {
        queue: *mut c_void,
        buffer: *mut QueueBuffer,
        free: AtomicBool,
    }

    // SAFETY: the queue is used only on the thread that calls `open`.
    unsafe impl Send for CoreAudioDevice {}

    impl CoreAudioDevice {
        #[must_use]
        pub fn new() -> Self {
            Self {
                queue: std::ptr::null_mut(),
                buffer: std::ptr::null_mut(),
                free: AtomicBool::new(true),
            }
        }
    }

    impl Default for CoreAudioDevice {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DevicePath for CoreAudioDevice {
        fn open(&mut self) -> Result<OpenedDevice, String> {
            unsafe {
                let format = StreamDesc {
                    sample_rate: f64::from(super::DEVICE_RATE),
                    format_id: 0x6c70_636d,
                    format_flags: 4 | 8,
                    bytes_per_packet: 4,
                    frames_per_packet: 1,
                    bytes_per_frame: 4,
                    channels: 2,
                    bits: 16,
                    reserved: 0,
                };
                let made = AudioQueueNewOutput(
                    &format,
                    finished,
                    std::ptr::addr_of!(self.free) as *mut c_void,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::addr_of_mut!(self.queue),
                );
                if made != 0 {
                    return Err(format!("AudioQueueNewOutput failed: {made}"));
                }
                let bytes = super::PERIOD_FRAMES.saturating_mul(4);
                let allocated = AudioQueueAllocateBuffer(
                    self.queue,
                    bytes,
                    std::ptr::addr_of_mut!(self.buffer),
                );
                if allocated != 0 || self.buffer.is_null() {
                    AudioQueueDispose(self.queue, 1);
                    self.queue = std::ptr::null_mut();
                    return Err(format!("AudioQueueAllocateBuffer failed: {allocated}"));
                }
                let started = AudioQueueStart(self.queue, std::ptr::null());
                if started != 0 {
                    AudioQueueDispose(self.queue, 1);
                    self.queue = std::ptr::null_mut();
                    return Err(format!("AudioQueueStart failed: {started}"));
                }
                Ok(OpenedDevice {
                    rate: super::DEVICE_RATE,
                    period_frames: super::PERIOD_FRAMES,
                    channels: 2,
                })
            }
        }

        fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str> {
            if self.queue.is_null() || self.buffer.is_null() {
                return Err("xrun");
            }
            if !self.free.swap(false, Ordering::AcqRel) {
                return Err("xrun");
            }
            let bytes = interleaved.len() * 2;
            unsafe {
                let buffer = &mut *self.buffer;
                if buffer.data.is_null() || usize::try_from(buffer.capacity).unwrap_or(0) < bytes {
                    self.free.store(true, Ordering::Release);
                    return Err("xrun");
                }
                std::ptr::copy_nonoverlapping(
                    interleaved.as_ptr() as *const u8,
                    buffer.data,
                    bytes,
                );
                buffer.size = u32::try_from(bytes).unwrap_or(0);
                let queued = AudioQueueEnqueueBuffer(self.queue, self.buffer, 0, std::ptr::null());
                if queued != 0 {
                    self.free.store(true, Ordering::Release);
                    return Err("xrun");
                }
            }
            Ok(())
        }

        fn close(&mut self) {
            unsafe {
                if !self.queue.is_null() {
                    let _ = AudioQueueStop(self.queue, 1);
                    AudioQueueDispose(self.queue, 1);
                }
            }
            self.queue = std::ptr::null_mut();
            self.buffer = std::ptr::null_mut();
        }
    }
}
