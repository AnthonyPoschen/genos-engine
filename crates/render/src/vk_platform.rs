// Vulkan loader and surface for the window that is already open.
//
// `genos_window_backend` says which native pair `display` and `surface` are.
// Wayland is `wl_display` and `wl_surface`. X11 is `Display*` and the window
// id. Win32 is `HINSTANCE` and `HWND`. Metal is a `CAMetalLayer` in both slots.

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
extern "C" {
    fn genos_window_backend() -> i32;
}

#[cfg(not(target_os = "windows"))]
extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> Pfn;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> Pfn;
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const WAYLAND_SURFACE: i32 = 1_000_006_000;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const XLIB_SURFACE: i32 = 1_000_004_000;
#[cfg(target_os = "windows")]
const WIN32_SURFACE: i32 = 1_000_009_000;
#[cfg(target_os = "macos")]
const METAL_SURFACE: i32 = 1_000_217_000;
#[cfg(target_os = "macos")]
const PORTABILITY_BIT: u32 = 0x0000_0001;

fn surface_extension_name() -> &'static [u8] {
    #[cfg(target_os = "windows")]
    {
        return b"VK_KHR_win32_surface\0";
    }
    #[cfg(target_os = "macos")]
    {
        return b"VK_EXT_metal_surface\0";
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // The window sets this before the renderer opens.
        if unsafe { genos_window_backend() } == 2 {
            b"VK_KHR_xlib_surface\0"
        } else {
            b"VK_KHR_wayland_surface\0"
        }
    }
}

fn extra_instance_extension() -> Option<&'static [u8]> {
    #[cfg(target_os = "macos")]
    {
        Some(b"VK_KHR_portability_enumeration\0")
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn instance_flags() -> u32 {
    #[cfg(target_os = "macos")]
    {
        PORTABILITY_BIT
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

#[cfg_attr(
    not(target_os = "macos"),
    allow(
        clippy::needless_pass_by_ref_mut,
        reason = "macOS appends VK_KHR_portability_subset when the device lists it"
    )
)]
fn push_device_extensions(dev_exts: &mut Vec<*const c_char>, listed: &[u8], count: u32) {
    #[cfg(target_os = "macos")]
    {
        static PORTABILITY: &[u8] = b"VK_KHR_portability_subset\0";
        if extension_listed(listed, count, b"VK_KHR_portability_subset") {
            dev_exts.push(PORTABILITY.as_ptr().cast::<c_char>());
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (dev_exts, listed, count);
    }
}

fn load_instance_proc() -> Result<GetProc, String> {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: the name is a null-terminated literal. A null module means the DLL is absent.
        let lib = unsafe { LoadLibraryA(b"vulkan-1.dll\0".as_ptr()) };
        if lib.is_null() {
            return Err("vulkan-1.dll did not open".into());
        }
        let proc = unsafe { GetProcAddress(lib, b"vkGetInstanceProcAddr\0".as_ptr()) };
        if proc.is_null() {
            return Err("vkGetInstanceProcAddr is missing from vulkan-1.dll".into());
        }
        return Ok(unsafe { transmute(proc) });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let names: &[&[u8]] = if cfg!(target_os = "macos") {
            &[
                b"libvulkan.dylib\0",
                b"libvulkan.1.dylib\0",
                b"libMoltenVK.dylib\0",
            ]
        } else {
            &[b"libvulkan.so.1\0"]
        };
        for name in names {
            // SAFETY: each name is a null-terminated literal. RTLD_NOW is 2.
            let lib = unsafe { dlopen(name.as_ptr().cast::<c_char>(), 2) };
            if lib.is_null() {
                continue;
            }
            let proc = unsafe { dlsym(lib, b"vkGetInstanceProcAddr\0".as_ptr().cast::<c_char>()) };
            if proc.is_null() {
                continue;
            }
            return Ok(unsafe { transmute(proc) });
        }
        Err("the Vulkan loader did not open".into())
    }
}

unsafe extern "system" fn unused_wayland_surface(
    _instance: Handle,
    _info: *const u8,
    _alloc: *const c_void,
    _out: *mut Handle,
) -> VkResult {
    -1
}

fn load_surface_proc(get_instance: GetProc, instance: Handle, symbol: &[u8]) -> Result<FnWayland, String> {
    let pfn = unsafe { get_instance(instance, symbol.as_ptr().cast::<c_char>()) };
    if pfn.is_null() {
        return Err("the Vulkan surface entry point is missing".into());
    }
    Ok(unsafe { transmute(pfn) })
}

fn finish_surface(
    create: FnWayland,
    instance: Handle,
    info: *const u8,
    what: &str,
) -> Result<Handle, String> {
    let mut vk_surface = std::ptr::null_mut();
    // SAFETY: `info` points at a repr(C) create-info that lives for this call. `create` is the loader entry.
    unsafe {
        check(
            create(instance, info, std::ptr::null(), &mut vk_surface),
            what,
        )?;
    }
    Ok(vk_surface)
}

#[cfg(target_os = "windows")]
unsafe fn create_surface(
    get_instance: GetProc,
    instance: Handle,
    display: *mut c_void,
    native: *mut c_void,
) -> Result<Handle, String> {
    #[repr(C)]
    struct Win32Info {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        hinstance: *mut c_void,
        hwnd: *mut c_void,
    }
    let info = Win32Info {
        s_type: WIN32_SURFACE,
        next: std::ptr::null(),
        flags: 0,
        hinstance: display,
        hwnd: native,
    };
    let create = load_surface_proc(get_instance, instance, b"vkCreateWin32SurfaceKHR\0")?;
    finish_surface(create, instance, std::ptr::from_ref(&info).cast::<u8>(), "win32 surface")
}

#[cfg(target_os = "macos")]
unsafe fn create_surface(
    get_instance: GetProc,
    instance: Handle,
    _display: *mut c_void,
    native: *mut c_void,
) -> Result<Handle, String> {
    #[repr(C)]
    struct MetalInfo {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        layer: *mut c_void,
    }
    let info = MetalInfo {
        s_type: METAL_SURFACE,
        next: std::ptr::null(),
        flags: 0,
        layer: native,
    };
    let create = load_surface_proc(get_instance, instance, b"vkCreateMetalSurfaceEXT\0")?;
    finish_surface(create, instance, std::ptr::from_ref(&info).cast::<u8>(), "metal surface")
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
unsafe fn create_surface(
    get_instance: GetProc,
    instance: Handle,
    display: *mut c_void,
    native: *mut c_void,
) -> Result<Handle, String> {
    if genos_window_backend() == 2 {
        #[repr(C)]
        struct XlibInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            dpy: *mut c_void,
            window: u64,
        }
        let info = XlibInfo {
            s_type: XLIB_SURFACE,
            next: std::ptr::null(),
            flags: 0,
            dpy: display,
            window: u64::try_from(native.addr()).unwrap_or(0),
        };
        let create = load_surface_proc(get_instance, instance, b"vkCreateXlibSurfaceKHR\0")?;
        return finish_surface(
            create,
            instance,
            std::ptr::from_ref(&info).cast::<u8>(),
            "x11 surface",
        );
    }
    #[repr(C)]
    struct WaylandInfo {
        s_type: i32,
        next: *const c_void,
        flags: u32,
        display: *mut c_void,
        surface: *mut c_void,
    }
    let info = WaylandInfo {
        s_type: WAYLAND_SURFACE,
        next: std::ptr::null(),
        flags: 0,
        display,
        surface: native,
    };
    let create = load_surface_proc(get_instance, instance, b"vkCreateWaylandSurfaceKHR\0")?;
    finish_surface(
        create,
        instance,
        std::ptr::from_ref(&info).cast::<u8>(),
        "wayland surface",
    )
}

#[cfg(test)]
mod surface_name {
    #[test]
    fn the_surface_extension_name_is_null_terminated() {
        let name = super::surface_extension_name();
        assert!(name.len() > 1);
        assert_eq!(name[name.len() - 1], 0);
        let text = std::str::from_utf8(&name[..name.len() - 1]).unwrap_or("");
        assert!(text.contains("surface"), "{text}");
    }
}
