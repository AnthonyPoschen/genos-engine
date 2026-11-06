//! The oracle scripts grade the live renderer, which follows light for an
//! unlimited number of bounces, against the CPU path tracer. A reference
//! capped below the tracer's default depth drops the light those extra
//! bounces carry, so every oracle number would carry that missing energy as
//! error. This lint fails when an oracle script caps `bounces` below it.

use std::path::Path;

/// `crates/debug/src/tools.rs` uses this when `bounces` is omitted.
const FULL_DEPTH: u32 = 64;

const ORACLE_SCRIPTS: &[&str] = &[
    "lighting_oracle.rhai",
    "lighting_oracle_lamps.rhai",
    "lamp_toggle_frames.rhai",
];

fn caps(source: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for (n, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let mut rest = code;
        while let Some(i) = rest.find("bounces:") {
            rest = &rest[i + "bounces:".len()..];
            let digits: String = rest
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(v) = digits.parse() {
                out.push((n + 1, v));
            }
        }
    }
    out
}

#[test]
fn oracle_references_are_not_bounce_capped() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let mut bad = Vec::new();
    for name in ORACLE_SCRIPTS {
        let path = dir.join(name);
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (line, v) in caps(&source) {
            if v < FULL_DEPTH {
                bad.push(format!("{name}:{line}: bounces: {v} < {FULL_DEPTH}"));
            }
        }
    }
    assert!(bad.is_empty(), "bounce-capped oracle references:\n{}", bad.join("\n"));
}

#[test]
fn lint_reads_bounce_caps() {
    assert_eq!(caps("x(#{ bounces: 3 }); // bounces: 1"), vec![(1, 3)]);
    assert_eq!(caps("bounces: 0, max_spp: 4"), vec![(1, 0)]);
}
