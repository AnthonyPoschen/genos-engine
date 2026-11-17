//! X11 keycodes for the X11 shim.
//!
//! With the evdev XKB rules, the X keycode is the Linux evdev code plus 8.
//! Left and right modifiers stay on different codes.

/// Linux evdev code for an X keycode, or `-1` when the code is outside the table.
#[no_mangle]
#[cfg_attr(not(test), allow(dead_code, reason = "x11_shim.c calls this on Linux"))]
pub extern "C" fn genos_evdev_from_x_keycode(keycode: u32) -> i32 {
    if !(8..264).contains(&keycode) {
        return -1;
    }
    i32::try_from(keycode - 8).unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::genos_evdev_from_x_keycode;

    #[test]
    fn an_evdev_keycode_is_eight_above_the_linux_code() {
        assert_eq!(genos_evdev_from_x_keycode(9), 1);
        assert_eq!(genos_evdev_from_x_keycode(38), 30);
        assert_eq!(genos_evdev_from_x_keycode(0), -1);
        assert_eq!(genos_evdev_from_x_keycode(264), -1);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn grabbed_motion_keeps_the_fraction() {
        use std::ffi::c_double;
        use std::ffi::c_int;

        unsafe extern "C" {
            fn genos_axis_step(acc: *mut c_double, sample: c_double) -> c_int;
        }

        let mut acc = 0.0;
        let mut emitted = 0;
        for _ in 0..3 {
            emitted += unsafe { genos_axis_step(&mut acc, 0.4) };
        }
        assert_eq!(emitted, 1, "three 0.4 steps are one pixel, not three zeros");
        assert!((acc - (1.2 - 1.0)).abs() < 1.0e-9, "fraction left {acc}");
        emitted = 0;
        acc = 0.0;
        for _ in 0..3 {
            emitted += unsafe { genos_axis_step(&mut acc, -0.4) };
        }
        assert_eq!(emitted, -1, "three -0.4 steps are minus one pixel");
        assert!(
            (acc - (-1.2 - -1.0)).abs() < 1.0e-9,
            "negative fraction left {acc}"
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn an_emulated_wheel_notch_is_a_button() {
        use std::ffi::c_int;

        unsafe extern "C" {
            fn genos_xi_pointer_kind(evtype: c_int, flags: c_int, detail: c_int) -> c_int;
        }

        const PRESS: c_int = 4;
        const RELEASE: c_int = 5;
        const MOTION: c_int = 6;
        const EMULATED: c_int = 1 << 16;
        const IGNORE: c_int = 0;
        const MOTION_KIND: c_int = 1;
        const BUTTON_KIND: c_int = 2;

        assert_eq!(unsafe { genos_xi_pointer_kind(PRESS, 0, 1) }, BUTTON_KIND);
        assert_eq!(unsafe { genos_xi_pointer_kind(MOTION, 0, 0) }, MOTION_KIND);
        assert_eq!(unsafe { genos_xi_pointer_kind(PRESS, EMULATED, 1) }, IGNORE);
        assert_eq!(
            unsafe { genos_xi_pointer_kind(RELEASE, EMULATED, 1) },
            IGNORE
        );
        assert_eq!(
            unsafe { genos_xi_pointer_kind(MOTION, EMULATED, 0) },
            IGNORE
        );
        assert_eq!(unsafe { genos_xi_pointer_kind(PRESS, EMULATED, 2) }, IGNORE);
        assert_eq!(unsafe { genos_xi_pointer_kind(PRESS, EMULATED, 3) }, IGNORE);
        for detail in 4..=7 {
            assert_eq!(
                unsafe { genos_xi_pointer_kind(PRESS, EMULATED, detail) },
                BUTTON_KIND,
                "press {detail}"
            );
            assert_eq!(
                unsafe { genos_xi_pointer_kind(RELEASE, EMULATED, detail) },
                BUTTON_KIND,
                "release {detail}"
            );
        }
    }

    #[test]
    fn left_and_right_modifiers_stay_apart() {
        assert_eq!(genos_evdev_from_x_keycode(50), 42);
        assert_eq!(genos_evdev_from_x_keycode(62), 54);
        assert_eq!(genos_evdev_from_x_keycode(37), 29);
        assert_eq!(genos_evdev_from_x_keycode(105), 97);
        assert_eq!(genos_evdev_from_x_keycode(64), 56);
        assert_eq!(genos_evdev_from_x_keycode(108), 100);
    }
}
