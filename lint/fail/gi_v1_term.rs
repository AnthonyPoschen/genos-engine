// The radiance field is not a lightmap, and a light probe is not a screen probe.
// Do not restore GI v2 product labels, archive-v1.md, ADR 0008, or gi_v2_floor.
fn retired() {
    let _ = "docs/systems/lighting/archive-v1.md";
    let _ = "docs/adr/0008-lighting-uses-radiance-cascades.md";
    let _ = "examples/stress/tests/gi_v2_floor.rs";
}
