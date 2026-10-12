//! The display curve the picture and the CPU references share.

/// Lit colour of a surface: albedo times the default BRDF times its irradiance,
/// through the display curve.
pub(crate) fn compose(albedo: [f32; 3], direct: f32, bounce: [f32; 3], _level: bool) -> [f32; 3] {
    let reflect = genos_scene::DEFAULT_BRDF;
    [
        tone(albedo[0] * reflect * (direct + bounce[0])),
        tone(albedo[1] * reflect * (direct + bounce[1])),
        tone(albedo[2] * reflect * (direct + bounce[2])),
    ]
}

/// A shadow stays linear. A hot corridor wall bends instead of clipping.
pub(crate) fn tone(channel: f32) -> f32 {
    if channel <= 0.64 {
        return channel.max(0.0);
    }
    let extra = channel - 0.64;
    0.64 + 0.14 * (extra / (extra + 1.1))
}
