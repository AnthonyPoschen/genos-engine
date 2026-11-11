//! Set-1 scan codes and absolute mouse positions for the Win32 shim.
//!
//! `win32_shim.c` calls these. A base scan code matches the Linux evdev code.
//! An extended scan is a different key, so left and right modifiers stay apart.

/// Set-1 scan code to a Linux evdev code.
///
/// `extended` is the Windows extended-key bit (`KF_EXTENDED` / raw `RI_KEY_E0`).
#[no_mangle]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "win32_shim.c calls this on Windows")
)]
pub extern "C" fn genos_evdev_from_set1(scan: u32, extended: i32) -> i32 {
    if extended != 0 {
        return match scan {
            0x1C => 96,
            0x1D => 97,
            0x35 => 98,
            0x37 => 99,
            0x38 => 100,
            0x47 => 102,
            0x48 => 103,
            0x49 => 104,
            0x4B => 105,
            0x4D => 106,
            0x4F => 107,
            0x50 => 108,
            0x51 => 109,
            0x52 => 110,
            0x53 => 111,
            0x5B => 125,
            0x5C => 126,
            0x5D => 127,
            _ => -1,
        };
    }
    if (1..256).contains(&scan) {
        i32::try_from(scan).unwrap_or(-1)
    } else {
        -1
    }
}

/// Map a `MOUSE_MOVE_ABSOLUTE` sample (`0..=65535`) onto a pixel span.
#[no_mangle]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "win32_shim.c calls this on Windows")
)]
pub extern "C" fn genos_absolute_pixel(norm: i32, origin: i32, span: i32) -> i32 {
    if span <= 0 {
        return origin;
    }
    let norm = norm.clamp(0, 65535);
    origin.saturating_add(norm.saturating_mul(span) / 65535)
}

/// Pixel delta from the previous absolute sample. The first sample is zero.
#[no_mangle]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "win32_shim.c calls this on Windows")
)]
pub extern "C" fn genos_absolute_delta(ready: i32, prev: i32, next: i32) -> i32 {
    if ready == 0 {
        0
    } else {
        next.wrapping_sub(prev)
    }
}

#[cfg(test)]
mod tests {
    use super::{genos_absolute_delta, genos_absolute_pixel, genos_evdev_from_set1};

    #[test]
    fn left_and_right_modifiers_use_different_evdev_codes() {
        assert_eq!(genos_evdev_from_set1(0x2A, 0), 42);
        assert_eq!(genos_evdev_from_set1(0x36, 0), 54);
        assert_eq!(genos_evdev_from_set1(0x1D, 0), 29);
        assert_eq!(genos_evdev_from_set1(0x1D, 1), 97);
        assert_eq!(genos_evdev_from_set1(0x38, 0), 56);
        assert_eq!(genos_evdev_from_set1(0x38, 1), 100);
        assert_ne!(
            genos_evdev_from_set1(0x1D, 0),
            genos_evdev_from_set1(0x1D, 1)
        );
    }

    #[test]
    fn letters_escape_and_arrows_match_evdev() {
        assert_eq!(genos_evdev_from_set1(0x11, 0), 17);
        assert_eq!(genos_evdev_from_set1(0x1E, 0), 30);
        assert_eq!(genos_evdev_from_set1(0x01, 0), 1);
        assert_eq!(genos_evdev_from_set1(0x48, 1), 103);
        assert_eq!(genos_evdev_from_set1(0x50, 1), 108);
        assert_eq!(genos_evdev_from_set1(0x4B, 1), 105);
        assert_eq!(genos_evdev_from_set1(0x4D, 1), 106);
        assert_eq!(genos_evdev_from_set1(0x48, 0), 0x48);
        assert_eq!(genos_evdev_from_set1(0, 0), -1);
        assert_eq!(genos_evdev_from_set1(0x5B, 0), 0x5B);
        assert_eq!(genos_evdev_from_set1(0x5B, 1), 125);
    }

    #[test]
    fn an_absolute_sample_is_a_pixel_and_the_first_delta_is_zero() {
        assert_eq!(genos_absolute_pixel(0, 10, 1000), 10);
        assert_eq!(genos_absolute_pixel(65535, 10, 1000), 1010);
        assert_eq!(genos_absolute_pixel(32768, 0, 65535), 32768);
        assert_eq!(genos_absolute_delta(0, 0, 40000), 0);
        assert_eq!(genos_absolute_delta(1, 100, 140), 40);
        assert_eq!(genos_absolute_delta(1, 140, 100), -40);
    }
}
