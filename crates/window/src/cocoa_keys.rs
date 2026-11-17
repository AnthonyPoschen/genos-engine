//! Per-side Cocoa modifier flags. `cocoa_shim.m` calls [`genos_modifier_down`].
//!
//! The public Shift, Control, and Option flags are either side. The device masks
//! below are the left and right bits still present on `NSEvent.modifierFlags`.

/// `1` when that physical modifier is down, `0` when it is up, `-1` for another key.
#[no_mangle]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "cocoa_shim.m calls this on macOS")
)]
pub extern "C" fn genos_modifier_down(key_code: u32, flags: u64) -> i32 {
    let mask: u64 = match key_code {
        56 => 0x0002,  // left shift
        60 => 0x0004,  // right shift
        59 => 0x0001,  // left control
        62 => 0x2000,  // right control
        58 => 0x0020,  // left option
        61 => 0x0040,  // right option
        55 => 0x0008,  // left command
        54 => 0x0010,  // right command
        _ => return -1,
    };
    i32::from(flags & mask != 0)
}

#[cfg(test)]
mod tests {
    use super::genos_modifier_down;

    #[test]
    fn each_side_follows_its_own_flag() {
        assert_eq!(genos_modifier_down(56, 0x0002), 1);
        assert_eq!(genos_modifier_down(60, 0x0002), 0);
        assert_eq!(genos_modifier_down(60, 0x0004), 1);
        assert_eq!(genos_modifier_down(59, 0x0001), 1);
        assert_eq!(genos_modifier_down(62, 0x0001), 0);
        assert_eq!(genos_modifier_down(62, 0x2000), 1);
        assert_eq!(genos_modifier_down(58, 0x0020), 1);
        assert_eq!(genos_modifier_down(61, 0x0040), 1);
        assert_eq!(genos_modifier_down(55, 0x0008), 1);
        assert_eq!(genos_modifier_down(54, 0x0010), 1);
        assert_eq!(genos_modifier_down(13, 0x0002), -1);
    }
}
