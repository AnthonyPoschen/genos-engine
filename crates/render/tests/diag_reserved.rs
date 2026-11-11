//! The `gstats.diag` words the screen pass uses as working state (list
//! allocators and changed/fine/coarse counts) must never be written by a
//! diagnostic. Before this, `lighting_dg(40u/41u)` in `lighting_direct.comp` and
//! ad-hoc instrumentation on 61..63 changed the lit picture when `LIGHTING_DIAG` was on.

const RESERVED: [u32; 5] = [40, 41, 61, 62, 63];
const SHADERS: [(&str, &str); 5] = [
    ("lighting_cache.glsl", include_str!("../shaders/lighting_cache.glsl")),
    ("lighting_direct.comp", include_str!("../shaders/lighting_direct.comp")),
    ("lighting_compact.comp", include_str!("../shaders/lighting_compact.comp")),
    ("lighting_screen.comp", include_str!("../shaders/lighting_screen.comp")),
    ("restir_live.comp", include_str!("../shaders/restir_live.comp")),
];

fn literal_after(src: &str, pat: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find(pat) {
            rest = &rest[at + pat.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
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

/// Parent follow-up words. This slice writes 38 and 39 for texel and lamp
/// counts. It leaves 8 for that follow-up. The restir counters stay off all three.
const PARENT_DIAG: [u32; 3] = [8, 38, 39];

const DIAG_NAMES: [(&str, u32); 9] = [
    ("LIGHTING_DIAG_PROBE_SKIP", 0),
    ("LIGHTING_DIAG_PROBE_EVAL", 1),
    ("LIGHTING_DIAG_PROBE_READS", 2),
    ("LIGHTING_DIAG_TEXEL_HELD", 38),
    ("LIGHTING_DIAG_TEXEL_CUT", 39),
    ("LIGHTING_DIAG_SCREEN_LIST_COARSE", 40),
    ("LIGHTING_DIAG_SCREEN_LIST_FINE", 41),
    ("LIGHTING_DIAG_UNDER_TRUSTED", 57),
    ("LIGHTING_DIAG_SCREEN_CHANGED", 61),
];

/// `gstats.diag` indices a shader can write. `LIGHTING_DIAG` defaults to false
/// in source; this still collects every `lighting_dg` arm, so a true build is
/// the same set. Source text, not a SPIR-V disassembly.
#[allow(clippy::panic, reason = "a test aborts when a shader constant is missing")]
fn named_u32(src: &str, name: &str) -> u32 {
    let key = format!("const uint {name} = ");
    let Some(at) = src.find(&key) else {
        panic!("{name} has no constant");
    };
    let rest = &src[at + key.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or_else(|_| panic!("{name} is not a decimal word"))
}

fn occupied_diag(src: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let mut push = |v: u32| {
        if !found.contains(&v) {
            found.push(v);
        }
    };
    for line in src.lines() {
        let code = line.split("//").next().unwrap_or(line);
        if code.contains("lighting_dg(") {
            if code.contains("20u +") {
                for bin in 0..8 {
                    push(20 + bin);
                }
            }
            let mut rest = code;
            while let Some(at) = rest.find("lighting_dg(") {
                rest = &rest[at + "lighting_dg(".len()..];
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(v) = digits.parse() {
                    push(v);
                }
            }
            for mark in ["? ", ": "] {
                let mut rest = code;
                while let Some(at) = rest.find(mark) {
                    rest = &rest[at + mark.len()..];
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    if !digits.is_empty() && rest[digits.len()..].starts_with('u') {
                        if let Ok(v) = digits.parse() {
                            push(v);
                        }
                    }
                }
            }
        }
        if code.contains("gstats.diag[") {
            let mut rest = code;
            while let Some(at) = rest.find("gstats.diag[") {
                rest = &rest[at + "gstats.diag[".len()..];
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(v) = digits.parse() {
                    push(v);
                }
                for (name, value) in DIAG_NAMES {
                    if rest.starts_with(name) {
                        push(value);
                    }
                }
            }
        }
        if let Some(at) = code.find("slot_d = ") {
            let rest = &code[at + "slot_d = ".len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(v) = digits.parse() {
                push(v);
            }
        }
    }
    found
}

#[test]
fn restir_counters_stay_off_compact_diag_words() {
    // The default source says LIGHTING_DIAG is false. Compact and the cache
    // still name the arms a true build writes, including diag 5, 6, 13 and
    // diag 43, 44, 45. Those diagnostics stay. The new counters are restir
    // header words with those last three numbers: a different buffer, so the
    // shared integers are not a gstats layout collision. The guard fails if
    // the counters move back onto gstats.diag.
    let mut occupied = Vec::new();
    for (_, src) in SHADERS {
        for index in occupied_diag(src) {
            if !occupied.contains(&index) {
                occupied.push(index);
            }
        }
    }
    for index in [5u32, 6, 13, 14, 32, 36, 38, 39, 40, 41, 43, 44, 45, 48, 61] {
        assert!(
            occupied.contains(&index),
            "occupied set missed diag[{index}]; the scan no longer sees both compile modes"
        );
    }
    let live = SHADERS[4].1;
    let direct = SHADERS[1].1;
    let compact = SHADERS[2].1;
    assert!(compact.contains("lighting_dg(hinted ? 5u : 6u, 1u);"));
    assert!(compact.contains("on_screen ? 13u : 14u"));
    assert!(SHADERS[0].1.contains("lighting_dg(43u, 1u);"));
    assert!(SHADERS[0].1.contains("lighting_dg(44u, 1u);"));
    assert!(SHADERS[0].1.contains("lighting_dg(45u, 1u);"));
    assert!(SHADERS[0].1.contains("const bool LIGHTING_DIAG = false;"));
    let accept = named_u32(live, "RESTIR_DIAG_HIST_ACCEPT");
    let reject = named_u32(live, "RESTIR_DIAG_HIST_REJECT");
    let point = named_u32(direct, "RESTIR_DIAG_POINT_ADD");
    for index in [accept, reject, point] {
        assert!(
            !PARENT_DIAG.contains(&index),
            "restir header word {index} is parent diag 8, 38, or 39"
        );
        assert!(
            !RESERVED.contains(&index),
            "restir header word {index} is a screen working word"
        );
    }
    assert_eq!((accept, reject, point), (43, 44, 45));
    assert!(live.contains("atomicAdd(restir_words[RESTIR_DIAG_HIST_ACCEPT], 1u);"));
    assert!(live.contains("atomicAdd(restir_words[RESTIR_DIAG_HIST_REJECT], 1u);"));
    assert!(!live.contains("atomicAdd(gstats.diag"));
    assert!(!live.contains("gstats.diag[RESTIR_DIAG_HIST_ACCEPT]"));
    assert!(!live.contains("gstats.diag[RESTIR_DIAG_HIST_REJECT]"));
    assert!(direct.contains("atomicAdd(restir_words[RESTIR_DIAG_POINT_ADD], 1u);"));
    assert!(!direct.contains("atomicAdd(gstats.diag[13u], 1u);"));
}
