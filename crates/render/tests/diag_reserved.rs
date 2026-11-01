//! The gstats.diag words the screen pass uses as working state (list
//! allocators and changed/fine/coarse counts) must never be written by a
//! diagnostic. Before this, `lighting_dg(40u/41u)` in direct.comp and ad-hoc
//! instrumentation on 61..63 changed the lit picture when LIGHTING_DIAG was on.

const RESERVED: [u32; 5] = [40, 41, 61, 62, 63];
const SHADERS: [(&str, &str); 4] = [
    ("lighting_cache.glsl", include_str!("../shaders/lighting_cache.glsl")),
    ("lighting_direct.comp", include_str!("../shaders/lighting_direct.comp")),
    ("lighting_compact.comp", include_str!("../shaders/lighting_compact.comp")),
    ("lighting_screen.comp", include_str!("../shaders/lighting_screen.comp")),
];

fn literal_after<'a>(src: &'a str, pat: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find(pat) {
            rest = &rest[at + pat.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = digits.parse() {
                out.push((n + 1, v));
            }
        }
    }
    out
}

#[test]
fn diagnostics_never_target_working_counters() {
    for (name, src) in SHADERS {
        for (line, i) in literal_after(src, "lighting_dg(") {
            assert!(!RESERVED.contains(&i), "{name}:{line}: lighting_dg({i}u) writes a working counter");
        }
        for (line, i) in literal_after(src, "gstats.diag[") {
            assert!(!RESERVED.contains(&i), "{name}:{line}: raw gstats.diag[{i}]; use the LIGHTING_DIAG_SCREEN_* name");
        }
    }
}

#[test]
fn lighting_dg_guards_reserved_words() {
    let cache = SHADERS[0].1;
    assert!(cache.contains("if (LIGHTING_DIAG && !lighting_diag_reserved(i))"));
    for i in RESERVED {
        assert!(cache.contains(&format!("= {i}u;")), "reserved word {i} has no named constant");
    }
}
