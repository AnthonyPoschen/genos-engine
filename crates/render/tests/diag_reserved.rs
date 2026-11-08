#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
#![expect(clippy::panic, reason = "a test aborts when a fixture fails")]

//! Owners of `gstats.diag`.
//!
//! Screen words 40, 41, and 61..63 are working state. Lamp-pick counters and
//! the ray-cut held visit are live counters in the same stats buffer. A write
//! through a named constant counts. Two owners of one index fail. `lighting_dg`
//! is compiled out (`LIGHTING_DIAG` is false) and must still stay off the
//! screen words and off the ray-cut word.

use std::collections::{BTreeMap, BTreeSet};

const RESERVED: [u32; 5] = [40, 41, 61, 62, 63];
const SHADERS: [(&str, &str); 6] = [
    ("lighting_cache.glsl", include_str!("../shaders/lighting_cache.glsl")),
    ("lighting_direct.comp", include_str!("../shaders/lighting_direct.comp")),
    ("lighting_compact.comp", include_str!("../shaders/lighting_compact.comp")),
    ("lighting_screen.comp", include_str!("../shaders/lighting_screen.comp")),
    ("lighting_screen_probe.glsl", include_str!("../shaders/lighting_screen_probe.glsl")),
    ("texel_ray_cut_prod.comp", include_str!("../shaders/texel_ray_cut_prod.comp")),
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    index: u32,
    group: String,
    file: String,
    line: usize,
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            out.push('\n');
            for n in chars.by_ref() {
                if n == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(n) = chars.next() {
                if n == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn constants(src: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for line in strip_comments(src).lines() {
        let Some(rest) = line.trim().strip_prefix("const uint ") else {
            continue;
        };
        let Some((name, expr)) = rest.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let expr = expr.trim().trim_end_matches(';').trim();
        let Some(number) = expr.strip_suffix('u') else {
            continue;
        };
        if let Ok(value) = number.parse() {
            out.insert(name.to_string(), value);
        }
    }
    out
}

fn group_of(name: &str) -> String {
    if name.starts_with("LIGHTING_DIAG_LAMP_") {
        "lamp".to_string()
    } else if name.starts_with("LIGHTING_DIAG_TEXEL_") {
        "texel".to_string()
    } else if name.starts_with("LIGHTING_DIAG_PROBE_") {
        "probe".to_string()
    } else if name.starts_with("LIGHTING_DIAG_SCREEN_") {
        "screen".to_string()
    } else {
        format!("name:{name}")
    }
}

fn assignments(src: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in strip_comments(src).lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("const ") || !trimmed.contains('=') {
            continue;
        }
        let Some((left, right)) = trimmed.split_once('=') else {
            continue;
        };
        let left = left.trim().trim_start_matches("uint ").trim();
        if !left.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || left.is_empty() {
            continue;
        }
        let right = right.trim().trim_end_matches(';').trim();
        if right.contains('?') || right.contains('(') {
            continue;
        }
        out.entry(left.to_string()).or_default().push(right.to_string());
    }
    out
}

fn split_ternary(expr: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut question = None;
    for (i, c) in expr.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '?' if depth == 0 && question.is_none() => question = Some(i),
            ':' if depth == 0 && question.is_some() => {
                let q = question.expect("question mark");
                return Some((expr[q + 1..i].trim(), expr[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

fn resolve(
    expr: &str,
    consts: &BTreeMap<String, u32>,
    assigned: &BTreeMap<String, Vec<String>>,
    seen: &mut BTreeSet<String>,
) -> Result<Vec<(u32, String)>, String> {
    if let Some((yes, no)) = split_ternary(expr) {
        let mut out = resolve(yes, consts, assigned, seen)?;
        out.extend(resolve(no, consts, assigned, seen)?);
        return Ok(out);
    }
    let expr = expr.trim();
    if let Some(number) = expr.strip_suffix('u') {
        if let Ok(value) = number.parse::<u32>() {
            return Ok(vec![(value, format!("literal:{value}"))]);
        }
    }
    if let Some(value) = consts.get(expr) {
        return Ok(vec![(*value, group_of(expr))]);
    }
    if expr.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !expr.is_empty() {
        if !seen.insert(expr.to_string()) {
            return Err(format!("diag index {expr} refers to itself"));
        }
        let Some(values) = assigned.get(expr) else {
            return Err(format!("unresolved diag index {expr}"));
        };
        let mut out = Vec::new();
        for value in values {
            out.extend(resolve(value, consts, assigned, seen)?);
        }
        return Ok(out);
    }
    Err(format!("unresolved diag index {expr}"))
}

fn live_sites(file: &str, src: &str, consts: &BTreeMap<String, u32>) -> Result<Vec<Site>, String> {
    let clean = strip_comments(src);
    let assigned = assignments(&clean);
    let mut sites = Vec::new();
    for (line_no, line) in clean.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains("void lighting_dg(") || trimmed.contains("if (LIGHTING_DIAG)") {
            continue;
        }
        let Some(open) = trimmed.find("gstats.diag[") else {
            continue;
        };
        let write = trimmed.contains("atomicAdd(gstats.diag[")
            || trimmed.contains("atomicMax(gstats.diag[")
            || trimmed.contains("gstats.diag[") && trimmed.contains("] =");
        if !write {
            continue;
        }
        let rest = &trimmed[open + "gstats.diag[".len()..];
        let Some(end) = rest.find(']') else {
            return Err(format!("{file}:{}: unclosed diag index", line_no + 1));
        };
        let mut seen = BTreeSet::new();
        for (index, group) in resolve(&rest[..end], consts, &assigned, &mut seen)? {
            sites.push(Site {
                index,
                group,
                file: file.to_string(),
                line: line_no + 1,
            });
        }
    }
    Ok(sites)
}

fn lighting_dg_indices(src: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for (line_no, line) in strip_comments(src).lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("lighting_dg(") {
            rest = &rest[at + "lighting_dg(".len()..];
            let mut depth = 0i32;
            let mut end = rest.len();
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 0 => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            let arg = &rest[..end];
            let mut number = String::new();
            for c in arg.chars() {
                if c.is_ascii_digit() {
                    number.push(c);
                    continue;
                }
                if !number.is_empty() {
                    if let Ok(value) = number.parse() {
                        out.push((line_no + 1, value));
                    }
                    number.clear();
                }
            }
            if let Ok(value) = number.parse() {
                out.push((line_no + 1, value));
            }
        }
    }
    out
}

fn collisions(sites: &[Site]) -> Vec<String> {
    let mut owners: BTreeMap<u32, BTreeMap<String, Vec<&Site>>> = BTreeMap::new();
    for site in sites {
        owners
            .entry(site.index)
            .or_default()
            .entry(site.group.clone())
            .or_default()
            .push(site);
    }
    let mut out = Vec::new();
    for (index, groups) in owners {
        if groups.len() < 2 {
            continue;
        }
        let names: Vec<_> = groups.keys().cloned().collect();
        out.push(format!("diag[{index}] owners {}", names.join(", ")));
    }
    out
}

#[test]
fn diagnostics_never_share_a_live_word() {
    let mut consts = BTreeMap::new();
    for (_, src) in SHADERS {
        for (name, value) in constants(src) {
            if let Some(previous) = consts.insert(name.clone(), value) {
                assert_eq!(previous, value, "{name} has two values");
            }
        }
    }
    assert_eq!(consts.get("LIGHTING_DIAG_TEXEL_HELD").copied(), Some(8));
    assert_eq!(consts.get("LIGHTING_DIAG_LAMP_PX").copied(), Some(38));
    assert_eq!(consts.get("LIGHTING_DIAG_LAMP_HIT").copied(), Some(39));
    let mut sites = Vec::new();
    for (name, src) in SHADERS {
        sites.extend(live_sites(name, src, &consts).unwrap_or_else(|err| panic!("{err}")));
    }
    let shared = collisions(&sites);
    assert!(shared.is_empty(), "live diag collision: {}", shared.join("; "));
    for site in &sites {
        if RESERVED.contains(&site.index) {
            assert_eq!(
                site.group, "screen",
                "{}:{} writes reserved diag[{}] as {}",
                site.file, site.line, site.index, site.group
            );
        }
    }
    let held = sites.iter().any(|site| site.index == 8 && site.group == "texel");
    let px = sites.iter().any(|site| site.index == 38 && site.group == "lamp");
    let hit = sites.iter().any(|site| site.index == 39 && site.group == "lamp");
    assert!(held && px && hit, "lamp counters and the held visit must both stay live");
}

#[test]
fn a_named_constant_collision_is_not_a_literal_regex() {
    let src = r"
        const uint LIGHTING_DIAG_TEXEL_HELD = 8u;
        const uint LIGHTING_DIAG_LAMP_PX = 8u;
        void main() {
            atomicAdd(gstats.diag[LIGHTING_DIAG_TEXEL_HELD], 1u);
            atomicAdd(gstats.diag[LIGHTING_DIAG_LAMP_PX], 1u);
        }
    ";
    let consts = constants(src);
    let sites = live_sites("sample", src, &consts).expect("sample resolves");
    let shared = collisions(&sites);
    assert!(
        shared.iter().any(|line| line.contains("diag[8]")),
        "named owners of one index must fail, got {shared:?}"
    );
    let ternary = r"
        const uint LIGHTING_DIAG_LAMP_PX = 38u;
        const uint LIGHTING_DIAG_LAMP_HIT = 39u;
        void main() {
            atomicAdd(gstats.diag[probe_hit ? LIGHTING_DIAG_LAMP_HIT : LIGHTING_DIAG_LAMP_PX], count);
        }
    ";
    let consts = constants(ternary);
    let sites = live_sites("ternary", ternary, &consts).expect("ternary resolves");
    assert!(collisions(&sites).is_empty(), "one lamp owner may write both of its words");
    let raw = r"
        const uint LIGHTING_DIAG_TEXEL_HELD = 8u;
        void main() {
            atomicAdd(gstats.diag[8u], 1u);
            atomicAdd(gstats.diag[LIGHTING_DIAG_TEXEL_HELD], 1u);
        }
    ";
    let consts = constants(raw);
    let sites = live_sites("raw", raw, &consts).expect("raw resolves");
    assert!(
        collisions(&sites).iter().any(|line| line.contains("diag[8]")),
        "a literal and the named owner of that word must fail"
    );
}

#[test]
fn lighting_dg_stays_off_the_live_ray_cut_and_screen_words() {
    let cache = SHADERS[0].1;
    assert!(cache.contains("const bool LIGHTING_DIAG = false"));
    assert!(cache.contains("if (LIGHTING_DIAG && !lighting_diag_reserved(i))"));
    let mut used = BTreeSet::new();
    for (name, src) in SHADERS {
        for (line, index) in lighting_dg_indices(src) {
            assert!(
                !RESERVED.contains(&index),
                "{name}:{line}: lighting_dg({index}) writes a screen working word"
            );
            assert_ne!(index, 8, "{name}:{line}: lighting_dg writes the held-visit word");
            assert_ne!(index, 38, "{name}:{line}: lighting_dg writes the lamp pixel counter");
            assert_ne!(index, 39, "{name}:{line}: lighting_dg writes the lamp hit counter");
            used.insert(index);
        }
    }
    assert!(used.contains(&34), "the dead near-list bin still uses 34 behind LIGHTING_DIAG");
}
