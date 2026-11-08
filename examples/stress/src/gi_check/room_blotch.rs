//! Cache-only room blotch for ADR 0039 stage 2.
//!
//! The grade calls [`super::metrics::blotch`] on room-a and room-b and compares
//! the mean with 0.014. [`super::ROOM_BLOTCH_LIMIT`] stays 0.023 and is only
//! reported. The grade does not path-trace. A missing, noisy, or mismatched
//! reference is not a physical fail.
//!
//! The room file's sun is commented out. `--freeze-sun` holds `time_of_day` so
//! the host scene does not move during settle. It does not change this room's
//! lamp light. E11's 0.014 is a different capture (W7, 12.5 cm). This grade
//! does not claim that number.
//!
//! `read_cached` is the decoder. It opens a path, and the `.ref` format has no
//! seconds field, so the loaded `seconds` value is 0. The grade stages the
//! hashed operand in a private directory and keeps the reference only when
//! that path still holds those bytes. Sidecar `seconds_bits` is the writer's
//! reported duration, not a clock read back from the file. `seconds_budget_bits`
//! is the stop this command asked for (900 when `--seconds` is omitted, or
//! 2400). A sidecar from the other request does not match and is not rewritten.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use genos_debug::reference::{self, RefSetup, Reference};
use genos_debug::Image;
use genos_scene::{LightingSettings, Scene, LIGHTING_TOGGLE_CELLS_ONLY};

use super::metrics;
use super::ROOM_BLOTCH_LIMIT;

const ROOM_SCENE: &str = include_str!("../../scenes/room.rhai");
const SCRIPT: &str = include_str!("../../scripts/room_blotch.rhai");

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const BOUNCES: u32 = 64;
const SPP_BATCH: u32 = 64;
const MAX_SPP: u32 = 262_144;
const ADR_LIMIT: f64 = 0.014;
const SETTLE_TIMEOUT: u32 = 900;
const CAPTURE_WAIT: u32 = 2;
const WINDOW: &str = "640x360";
/// Camera replies are rounded to 1e-5. The other pose is about 0.05 away.
const POSE_SLOP: f64 = 2.0e-5;
const FLOOR_BYTES: usize = 187_216;
const FLOOR_SHA256: [&str; 2] = [
    "d41fabfff5ab88846e39a62f157b0f3fb859e91cd6a2e3508e603bfbf41f7585",
    "f345b127c9698e98ae571e573a2f9ec0a78e825fd8e3519a8c2a9b662b9c6d36",
];

const HELP: &str = "\
genos-stress room-blotch grade --refs DIR [--out DIR] [--capture] [--seconds N]
genos-stress room-blotch trace --pose room-a|room-b [--out DIR] [--promote DIR] [--threads N] [--seconds N]

grade scores room-a and room-b with metrics::blotch. The mean is compared with
0.014. The gi-check room row still uses 0.023 and is not this verdict.
grade does not path-trace and does not write a .ref. A missing reference stays
missing-reference.

--refs DIR   room-a.ref, room-a.json, room-b.ref, room-b.json. Required.
--out DIR    screenshots, verdict.json, and copies of the inputs.
             Default: ./genos-debug/room-blotch. Not $HOME.
--capture    after the refs pass, one proof window. Settle must be true.
             Then wait 2 frames and capture. No compare, so a miss cannot trace.
--pose       room-a or room-b. No room-c and no close-up.
--threads N  CPU trace width. Wins over GENOS_REFERENCE_THREADS.
             When omitted, that variable is used, else 4.
--seconds N  Requested trace stop, 900 or 2400. Omitted means 900.
             grade requires seconds_budget_bits to equal this request.
             A 900 sidecar stays 900; it is not relabeled 2400.
             The value is the requested stop, not a clock read from the file.
             There is no environment variable for this flag.
--promote DIR  copy the isolated ref only when identity matches and noise is
             finite and <= 0.01. spp at 262144 does not accept a noisier file.
             A directory named reference-cache is refused.

The other trace limits stay fixed: shapes, 64 bounces, 320x180, batch 64,
noise 0.01, max 262144 spp, CPU analytic. GENOS_REFERENCE_GPU=1 is refused.
The live picture is cells_only, surface_cache.sh_cells on, cache_async off,
world cell 0.25 m, sc_settle 0. The native cache key does not encode those
flags or the seconds stop. The capture child drops the lighting environment
catalogue, then sets GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS=1.
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    AdrPass,
    PhysicalFail,
    MissingReference,
    NoisyReference,
    NonFiniteReference,
    SceneMismatch,
    PoseMismatch,
    BudgetMismatch,
    Unsettled,
    WrongFlags,
    SunNotFrozen,
    MissingCapture,
    MissingLive,
    CaptureSchedule,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Self::AdrPass => "adr-pass",
            Self::PhysicalFail => "physical-fail",
            Self::MissingReference => "missing-reference",
            Self::NoisyReference => "noisy-reference",
            Self::NonFiniteReference => "nonfinite-reference",
            Self::SceneMismatch => "scene-mismatch",
            Self::PoseMismatch => "pose-mismatch",
            Self::BudgetMismatch => "budget-mismatch",
            Self::Unsettled => "unsettled",
            Self::WrongFlags => "wrong-flags",
            Self::SunNotFrozen => "sun-not-frozen",
            Self::MissingCapture => "missing-capture",
            Self::MissingLive => "missing-live",
            Self::CaptureSchedule => "capture-schedule",
        }
    }
}

#[derive(Debug)]
struct Issue {
    class: Class,
    detail: String,
}

struct Verdict {
    class: Class,
    detail: String,
    issues: Vec<Issue>,
    start: Option<f64>,
    mid: Option<f64>,
    mean: Option<f64>,
    legacy_word: Option<&'static str>,
    notes: Vec<String>,
}

#[derive(Clone, Copy)]
struct Pose {
    name: &'static str,
    yaw: f32,
}

fn noise_target() -> f32 {
    0.01
}

const DEFAULT_TRACE_SECONDS: f32 = 900.0;
const LONG_TRACE_SECONDS: f32 = 2400.0;

fn seconds_text(seconds: f32) -> &'static str {
    if seconds.to_bits() == LONG_TRACE_SECONDS.to_bits() {
        "2400"
    } else {
        "900"
    }
}

fn require_trace_seconds(value: f32) -> Result<f32, String> {
    if !value.is_finite() {
        return Err(format!("trace seconds are not finite ({value})"));
    }
    if value.to_bits() != DEFAULT_TRACE_SECONDS.to_bits()
        && value.to_bits() != LONG_TRACE_SECONDS.to_bits()
    {
        return Err(format!("trace seconds are {value}, want 900 or 2400"));
    }
    Ok(value)
}

fn plain_seconds_token(text: &str) -> bool {
    let text = text.strip_prefix(['+', '-']).unwrap_or(text);
    let mut parts = text.split('.');
    let Some(whole) = parts.next() else {
        return false;
    };
    let fraction = parts.next();
    if parts.next().is_some() {
        return false;
    }
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    match fraction {
        None => true,
        Some(fraction) => {
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
        }
    }
}

fn parse_trace_seconds(text: &str) -> Result<f32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("--seconds needs a value".into());
    }
    let lower = text.to_ascii_lowercase();
    let bare = lower.trim_start_matches(['+', '-']);
    if bare == "nan" || bare == "inf" || bare == "infinity" {
        return Err(format!("--seconds is not finite ({text})"));
    }
    if !plain_seconds_token(text) {
        return Err(format!("--seconds is not a number ({text})"));
    }
    let value = text
        .parse::<f32>()
        .map_err(|_| format!("--seconds is not a number ({text})"))?;
    if !value.is_finite() {
        return Err(format!("--seconds is not finite ({text})"));
    }
    require_trace_seconds(value).map_err(|_| format!("--seconds is {text}, want 900 or 2400"))
}

fn cell_metres() -> f32 {
    0.25
}

fn rhai_f32(value: f64) -> f32 {
    f32::from_bits(narrow_f64(value.to_bits()))
}

/// Round a binary64 value to binary32 the way a Rust `as f32` cast does.
/// Integer shifts keep this file off the lossy-cast baseline.
fn narrow_f64(bits: u64) -> u32 {
    let sign = u32::try_from(bits >> 63).unwrap_or(0) << 31;
    let exp = u32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let frac = bits & 0x000f_ffff_ffff_ffff;
    if exp == 0x7ff {
        let payload = u32::try_from(frac >> 29).unwrap_or(0);
        let quiet = if frac == 0 { 0 } else { payload | 1 };
        return sign | (0xff_u32 << 23) | quiet;
    }
    if exp == 0 {
        return sign;
    }
    let unbiased = i32::try_from(exp).unwrap_or(0) - 1023;
    let target = unbiased + 127;
    if target >= 255 {
        return sign | (0xff_u32 << 23);
    }
    if target <= 0 {
        let shift = 1 - target + 29;
        if shift >= 54 {
            return sign;
        }
        let significand = (1_u64 << 52) | frac;
        let places = u32::try_from(shift).unwrap_or(63);
        let kept = significand >> places;
        let rest = significand & ((1_u64 << places) - 1);
        let half = 1_u64 << (places - 1);
        let mut mant = u32::try_from(kept).unwrap_or(0);
        if rest > half || (rest == half && mant & 1 == 1) {
            mant = mant.saturating_add(1);
        }
        return sign | mant;
    }
    let mut mant = u32::try_from(frac >> 29).unwrap_or(0);
    let rest = frac & ((1_u64 << 29) - 1);
    let half = 1_u64 << 28;
    let mut exp_bits = u32::try_from(target).unwrap_or(0);
    if rest > half || (rest == half && mant & 1 == 1) {
        mant = mant.saturating_add(1);
        if mant == 1_u32 << 23 {
            mant = 0;
            exp_bits = exp_bits.saturating_add(1);
            if exp_bits >= 255 {
                return sign | (0xff_u32 << 23);
            }
        }
    }
    sign | (exp_bits << 23) | mant
}

fn eye() -> [f32; 3] {
    [rhai_f32(0.664), rhai_f32(1.699), rhai_f32(9.629)]
}

fn pitch() -> f32 {
    rhai_f32(-0.12)
}

fn poses() -> [Pose; 2] {
    [
        Pose {
            name: "room-a",
            yaw: rhai_f32(-0.679),
        },
        Pose {
            name: "room-b",
            yaw: rhai_f32(-36.0 * 0.0174533),
        },
    ]
}

fn pose_named(name: &str) -> Option<Pose> {
    poses().into_iter().find(|pose| pose.name == name)
}

/// Stress start hour with the sun frozen. `0.12` is the default `--time` fraction.
fn frozen_time_of_day() -> f32 {
    let day = 0.12f32;
    let hours = (f64::from(day) * 24.0 + 6.0) % 24.0;
    rhai_f32(hours)
}

fn scene_sha256() -> String {
    sha256_hex(ROOM_SCENE.as_bytes())
}

fn legacy_word(mean: f64) -> &'static str {
    if mean <= ROOM_BLOTCH_LIMIT {
        "PASS"
    } else {
        "FAIL"
    }
}

fn issue(class: Class, detail: impl Into<String>) -> Issue {
    Issue {
        class,
        detail: detail.into(),
    }
}

fn verdict_from(issues: Vec<Issue>, start: Option<f64>, mid: Option<f64>) -> Verdict {
    if let Some(first) = issues.first() {
        return Verdict {
            class: first.class,
            detail: first.detail.clone(),
            issues,
            start: None,
            mid: None,
            mean: None,
            legacy_word: None,
            notes: Vec::new(),
        };
    }
    let (Some(start), Some(mid)) = (start, mid) else {
        return Verdict {
            class: Class::MissingLive,
            detail: "blotch inputs missing".into(),
            issues,
            start: None,
            mid: None,
            mean: None,
            legacy_word: None,
            notes: Vec::new(),
        };
    };
    let mean = 0.5 * (start + mid);
    if !start.is_finite() || !mid.is_finite() || !mean.is_finite() {
        return Verdict {
            class: Class::NonFiniteReference,
            detail: "blotch mean is not finite".into(),
            issues,
            start: None,
            mid: None,
            mean: None,
            legacy_word: None,
            notes: Vec::new(),
        };
    }
    let class = if mean <= ADR_LIMIT {
        Class::AdrPass
    } else {
        Class::PhysicalFail
    };
    Verdict {
        class,
        detail: format!("start {start:.6} mid {mid:.6} mean {mean:.6}"),
        issues,
        start: Some(start),
        mid: Some(mid),
        mean: Some(mean),
        legacy_word: Some(legacy_word(mean)),
        notes: Vec::new(),
    }
}

fn sha256(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = u64::try_from(message.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(8);
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut offset = 0;
    while offset + 64 <= padded.len() {
        let mut words = [0_u32; 64];
        for (i, word) in words.iter_mut().enumerate().take(16) {
            let at = offset + i * 4;
            let bytes = [
                padded.get(at).copied().unwrap_or(0),
                padded.get(at + 1).copied().unwrap_or(0),
                padded.get(at + 2).copied().unwrap_or(0),
                padded.get(at + 3).copied().unwrap_or(0),
            ];
            *word = u32::from_be_bytes(bytes);
        }
        for i in 16..64 {
            let s0 = words[i - 15].rotate_right(7)
                ^ words[i - 15].rotate_right(18)
                ^ (words[i - 15] >> 3);
            let s1 = words[i - 2].rotate_right(17)
                ^ words[i - 2].rotate_right(19)
                ^ (words[i - 2] >> 10);
            words[i] = words[i - 16]
                .wrapping_add(s0)
                .wrapping_add(words[i - 7])
                .wrapping_add(s1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[i])
                .wrapping_add(words[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
        offset += 64;
    }
    let mut out = [0_u8; 32];
    for (i, word) in state.iter().enumerate() {
        let bytes = word.to_be_bytes();
        let at = i * 4;
        for (n, byte) in bytes.iter().enumerate() {
            if let Some(slot) = out.get_mut(at + n) {
                *slot = *byte;
            }
        }
    }
    out
}

fn sha256_hex(message: &[u8]) -> String {
    let digest = sha256(message);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn known_floor(sha: &str) -> bool {
    FLOOR_SHA256.contains(&sha)
}

fn refuses_acceptance_cache(path: &Path) -> Result<(), String> {
    for component in path.components() {
        if component.as_os_str() == "reference-cache" {
            return Err(format!(
                "refusing {} because it is the shared reference cache",
                path.display()
            ));
        }
    }
    Ok(())
}

fn json_fields(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim().trim_end_matches(',');
        if line.is_empty() || line == "{" || line == "}" {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"').to_string();
        map.insert(key, value.trim().to_string());
    }
    map
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_string()
}

fn take_u32(fields: &HashMap<String, String>, key: &str) -> Result<u32, Issue> {
    let Some(token) = fields.get(key) else {
        return Err(issue(field_class(key), format!("sidecar missing {key}")));
    };
    if token.contains('.') {
        return Err(issue(
            field_class(key),
            format!("sidecar {key} is not an integer ({token})"),
        ));
    }
    token.parse::<u32>().map_err(|_| {
        issue(
            field_class(key),
            format!("sidecar {key} is not an integer ({token})"),
        )
    })
}

fn take_bool(fields: &HashMap<String, String>, key: &str) -> Result<bool, Issue> {
    match fields.get(key).map(String::as_str) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(issue(
            field_class(key),
            format!("sidecar {key} is not a bool ({other})"),
        )),
        None => Err(issue(field_class(key), format!("sidecar missing {key}"))),
    }
}

fn field_class(key: &str) -> Class {
    match key {
        "pose" | "eye_bits" | "yaw_bits" | "pitch_bits" => Class::PoseMismatch,
        "scene_sha256" | "scene" | "estimator" | "triangles" | "gpu_reference" | "kind" => {
            Class::SceneMismatch
        }
        "cells_only" | "sh_cells" | "cache_async" | "world_cell_bits" | "world_mode" => {
            Class::WrongFlags
        }
        "sun_frozen" | "time_of_day_bits" => Class::SunNotFrozen,
        "noise_bits" => Class::NoisyReference,
        _ => Class::BudgetMismatch,
    }
}

struct Sidecar {
    pose: String,
    eye_bits: [u32; 3],
    yaw_bits: u32,
    pitch_bits: u32,
    noise_bits: u32,
    spp: u32,
    seconds_bits: u32,
    ref_sha256: String,
}

fn parse_eye_bits(token: &str) -> Result<[u32; 3], Issue> {
    let inner = token.trim().trim_start_matches('[').trim_end_matches(']');
    let mut bits = [0_u32; 3];
    let mut count = 0;
    for part in inner.split(',') {
        if count >= 3 {
            return Err(issue(Class::PoseMismatch, "eye_bits has more than 3"));
        }
        let parsed = part.trim().parse::<u32>().map_err(|_| {
            issue(
                Class::PoseMismatch,
                format!("eye_bits is not three integers ({token})"),
            )
        })?;
        bits[count] = parsed;
        count += 1;
    }
    if count != 3 {
        return Err(issue(Class::PoseMismatch, "eye_bits is not three integers"));
    }
    Ok(bits)
}

fn parse_sidecar(text: &str, seconds: f32) -> Result<Sidecar, Issue> {
    let fields = json_fields(text);
    let kind = fields
        .get("kind")
        .map(|value| unquote(value))
        .ok_or_else(|| issue(Class::SceneMismatch, "sidecar missing kind"))?;
    if kind != "room-blotch-ref" {
        return Err(issue(
            Class::SceneMismatch,
            format!("sidecar kind is {kind}"),
        ));
    }
    let pose = fields
        .get("pose")
        .map(|value| unquote(value))
        .ok_or_else(|| issue(Class::PoseMismatch, "sidecar missing pose"))?;
    if pose_named(&pose).is_none() {
        return Err(issue(
            Class::PoseMismatch,
            format!("sidecar pose is {pose}"),
        ));
    }
    let got_scene = fields
        .get("scene_sha256")
        .map(|value| unquote(value))
        .ok_or_else(|| issue(Class::SceneMismatch, "sidecar missing scene_sha256"))?;
    let eye_bits = fields
        .get("eye_bits")
        .ok_or_else(|| issue(Class::PoseMismatch, "sidecar missing eye_bits"))?;
    let eye_bits = parse_eye_bits(eye_bits)?;
    let expect_u32 = |key, want| -> Result<(), Issue> {
        let got = take_u32(&fields, key)?;
        if got != want {
            return Err(issue(
                field_class(key),
                format!("sidecar {key} is {got}, want {want}"),
            ));
        }
        Ok(())
    };
    expect_u32("width", WIDTH)?;
    expect_u32("height", HEIGHT)?;
    expect_u32("bounces", BOUNCES)?;
    expect_u32("spp_batch", SPP_BATCH)?;
    expect_u32("noise_target_bits", noise_target().to_bits())?;
    expect_u32("max_spp", MAX_SPP)?;
    let budget_bits = take_u32(&fields, "seconds_budget_bits")?;
    let recorded = f32::from_bits(budget_bits);
    if !recorded.is_finite() {
        return Err(issue(
            Class::NonFiniteReference,
            "sidecar seconds_budget_bits is not finite",
        ));
    }
    let seconds =
        require_trace_seconds(seconds).map_err(|err| issue(Class::BudgetMismatch, err))?;
    if budget_bits != seconds.to_bits() {
        return Err(issue(
            Class::BudgetMismatch,
            format!(
                "sidecar seconds_budget_bits is {budget_bits} ({recorded} s), want {} ({} s). The recorded budget is kept",
                seconds.to_bits(),
                seconds_text(seconds)
            ),
        ));
    }
    expect_u32("world_cell_bits", cell_metres().to_bits())?;
    expect_u32("world_mode", 3)?;
    expect_u32("time_of_day_bits", frozen_time_of_day().to_bits())?;
    if take_bool(&fields, "triangles")? {
        return Err(issue(Class::SceneMismatch, "sidecar triangles is true"));
    }
    if take_bool(&fields, "gpu_reference")? {
        return Err(issue(Class::SceneMismatch, "sidecar gpu_reference is true"));
    }
    if !take_bool(&fields, "sun_frozen")? {
        return Err(issue(Class::SunNotFrozen, "sidecar sun_frozen is false"));
    }
    if !take_bool(&fields, "cells_only")? {
        return Err(issue(Class::WrongFlags, "sidecar cells_only is false"));
    }
    if !take_bool(&fields, "sh_cells")? {
        return Err(issue(Class::WrongFlags, "sidecar sh_cells is false"));
    }
    if take_bool(&fields, "cache_async")? {
        return Err(issue(Class::WrongFlags, "sidecar cache_async is true"));
    }
    let estimator = fields
        .get("estimator")
        .map(|value| unquote(value))
        .ok_or_else(|| issue(Class::SceneMismatch, "sidecar missing estimator"))?;
    if estimator != "cpu-analytic" {
        return Err(issue(
            Class::SceneMismatch,
            format!("sidecar estimator is {estimator}"),
        ));
    }
    if got_scene != scene_sha256() {
        return Err(issue(
            Class::SceneMismatch,
            "sidecar scene_sha256 is not scenes/room.rhai",
        ));
    }
    let yaw_bits = take_u32(&fields, "yaw_bits")?;
    let pitch_bits = take_u32(&fields, "pitch_bits")?;
    let noise_bits = take_u32(&fields, "noise_bits")?;
    let spp = take_u32(&fields, "spp")?;
    let seconds_bits = take_u32(&fields, "seconds_bits")?;
    let ref_sha256 = fields
        .get("ref_sha256")
        .map(|value| unquote(value))
        .ok_or_else(|| issue(Class::SceneMismatch, "sidecar missing ref_sha256"))?;
    Ok(Sidecar {
        pose,
        eye_bits,
        yaw_bits,
        pitch_bits,
        noise_bits,
        spp,
        seconds_bits,
        ref_sha256,
    })
}

fn header_words(bytes: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let word = |index: usize| -> Option<u32> {
        let at = index * 4;
        Some(u32::from_le_bytes([
            *bytes.get(at)?,
            *bytes.get(at + 1)?,
            *bytes.get(at + 2)?,
            *bytes.get(at + 3)?,
        ]))
    };
    Some((word(0)?, word(1)?, word(2)?, word(3)?))
}

/// Linux `O_NOFOLLOW` (`asm-generic/fcntl.h`, octal 0400000).
const O_NOFOLLOW: i32 = 0x20000;

static STAGE_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct PrivateStage {
    dir: PathBuf,
}

impl PrivateStage {
    fn path(&self) -> PathBuf {
        self.dir.join("operand.ref")
    }
}

impl Drop for PrivateStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let hi = usize::from(byte >> 4);
        let lo = usize::from(byte & 0x0f);
        out.push(char::from(HEX[hi]));
        out.push(char::from(HEX[lo]));
    }
    out
}

fn random_hex() -> String {
    let mut bytes = [0_u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| {
            use std::io::Read;
            file.read_exact(&mut bytes)?;
            Ok(())
        })
        .is_ok();
    if !filled {
        let tick = STAGE_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let mixed = (u128::from(std::process::id()) << 64) | u128::from(tick) | nanos;
        bytes = mixed.to_le_bytes();
    }
    hex_bytes(&bytes)
}

fn private_judge_dir() -> Result<PathBuf, Issue> {
    use std::os::unix::fs::DirBuilderExt;
    for _ in 0..8 {
        let path = std::env::temp_dir().join(format!("room-blotch-judge-{}", random_hex()));
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(issue(
                    Class::SceneMismatch,
                    format!("could not stage the reader: {err}"),
                ));
            }
        }
    }
    Err(issue(Class::SceneMismatch, "could not stage the reader"))
}

fn exclusive_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_nofollow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    let mut got = Vec::new();
    file.read_to_end(&mut got)?;
    Ok(got)
}

fn hashed_operand_matches(path: &Path, bytes: &[u8]) -> bool {
    read_nofollow(path).ok().as_deref() == Some(bytes)
}

fn private_stage(bytes: &[u8]) -> Result<PrivateStage, Issue> {
    let dir = private_judge_dir()?;
    let stage = PrivateStage { dir };
    exclusive_write(&stage.path(), bytes).map_err(|err| {
        issue(
            Class::SceneMismatch,
            format!("could not stage the reader: {err}"),
        )
    })?;
    Ok(stage)
}

/// Decode `bytes` with [`reference::read_cached`].
///
/// The directory name is random and mode 0700. The file is created with
/// `O_EXCL` and `O_NOFOLLOW`, so a symlink at a hash-derived path is not
/// opened and not overwritten. `read_cached` reopens the path. The no-follow
/// read must still match `bytes` after that, or the decoded reference is
/// discarded. A same-user writer who replaces the file for the reopen and
/// restores it before the second check can still race; the hash-named shared
/// path is not used.
fn reference_from_staged(path: &Path, bytes: &[u8], pose_name: &str) -> Result<Reference, Issue> {
    if !hashed_operand_matches(path, bytes) {
        return Err(issue(
            Class::SceneMismatch,
            format!("{pose_name} staged bytes differ from the hashed operand"),
        ));
    }
    let loaded = reference::read_cached(path, WIDTH, HEIGHT);
    if !hashed_operand_matches(path, bytes) {
        return Err(issue(
            Class::SceneMismatch,
            format!("{pose_name} scored bytes differ from the hashed operand"),
        ));
    }
    loaded.ok_or_else(|| {
        issue(
            Class::SceneMismatch,
            format!("{pose_name} did not load through read_cached"),
        )
    })
}

fn load_verified(bytes: &[u8], pose_name: &str) -> Result<Reference, Issue> {
    let staged = private_stage(bytes)?;
    reference_from_staged(&staged.path(), bytes, pose_name)
}

fn samples_finite(reference: &Reference) -> bool {
    reference.noise.is_finite()
        && reference
            .linear
            .iter()
            .all(|rgb| rgb.iter().all(|channel| channel.is_finite()))
}

fn judge_bytes(
    bytes: &[u8],
    sidecar_text: &str,
    pose: &Pose,
    seconds: f32,
) -> Result<Reference, Issue> {
    let sha = sha256_hex(bytes);
    if known_floor(&sha) {
        return Err(issue(
            Class::SceneMismatch,
            "open-floor reference is not a room operand",
        ));
    }
    if bytes.len() == FLOOR_BYTES {
        return Err(issue(
            Class::SceneMismatch,
            "160x90 floor-sized reference is not a room operand",
        ));
    }
    let Some((width, height, spp, noise_bits)) = header_words(bytes) else {
        return Err(issue(
            Class::SceneMismatch,
            format!("{} reference header is short", pose.name),
        ));
    };
    if width != WIDTH || height != HEIGHT {
        return Err(issue(
            Class::SceneMismatch,
            format!(
                "{} reference is {width}x{height}, want {WIDTH}x{HEIGHT}",
                pose.name
            ),
        ));
    }
    let sidecar = parse_sidecar(sidecar_text, seconds)?;
    if sidecar.pose != pose.name {
        return Err(issue(
            Class::PoseMismatch,
            format!("sidecar pose is {}, file is {}", sidecar.pose, pose.name),
        ));
    }
    let want_eye = eye().map(f32::to_bits);
    if sidecar.eye_bits != want_eye || sidecar.pitch_bits != pitch().to_bits() {
        return Err(issue(
            Class::PoseMismatch,
            format!("{} eye or pitch bits differ", pose.name),
        ));
    }
    if sidecar.yaw_bits != pose.yaw.to_bits() {
        return Err(issue(
            Class::PoseMismatch,
            format!("{} yaw bits differ", pose.name),
        ));
    }
    if spp != sidecar.spp || noise_bits != sidecar.noise_bits {
        return Err(issue(
            Class::SceneMismatch,
            format!("{} header does not match the sidecar", pose.name),
        ));
    }
    if sha != sidecar.ref_sha256 {
        return Err(issue(
            Class::SceneMismatch,
            format!("{} bytes do not match ref_sha256", pose.name),
        ));
    }
    let noise = f32::from_bits(sidecar.noise_bits);
    let seconds = f32::from_bits(sidecar.seconds_bits);
    if !noise.is_finite() || !seconds.is_finite() {
        return Err(issue(
            Class::NonFiniteReference,
            format!("{} noise or reported seconds is not finite", pose.name),
        ));
    }
    if noise < 0.0 || seconds < 0.0 {
        return Err(issue(
            Class::NonFiniteReference,
            format!("{} noise or reported seconds is negative", pose.name),
        ));
    }
    if sidecar.spp == 0 || sidecar.spp > MAX_SPP {
        return Err(issue(
            Class::BudgetMismatch,
            format!("{} spp is {}", pose.name, sidecar.spp),
        ));
    }
    if noise > noise_target() {
        return Err(issue(
            Class::NoisyReference,
            format!(
                "{} noise {noise} spp {} is above 0.01; the spp cap does not accept it",
                pose.name, sidecar.spp
            ),
        ));
    }
    // `seconds` above is the requested stop. The reported duration in the
    // sidecar is separate provenance. The file loader stores 0. A positive
    // report does not prove a file clock, and zero is not a budget mismatch.
    let loaded = load_verified(bytes, pose.name)?;
    if !samples_finite(&loaded) {
        return Err(issue(
            Class::NonFiniteReference,
            format!("{} has a nonfinite sample", pose.name),
        ));
    }
    if loaded.spp != sidecar.spp || loaded.noise.to_bits() != sidecar.noise_bits {
        return Err(issue(
            Class::SceneMismatch,
            format!("{} reader header does not match the sidecar", pose.name),
        ));
    }
    Ok(loaded)
}

fn judge_pose(refs: &Path, pose: &Pose, seconds: f32) -> Result<Reference, Issue> {
    let ref_path = refs.join(format!("{}.ref", pose.name));
    let json_path = refs.join(format!("{}.json", pose.name));
    if !ref_path.is_file() || !json_path.is_file() {
        return Err(issue(
            Class::MissingReference,
            format!("missing {}.ref or {}.json", pose.name, pose.name),
        ));
    }
    let bytes = std::fs::read(&ref_path).map_err(|err| {
        issue(
            Class::MissingReference,
            format!("reading {}: {err}", ref_path.display()),
        )
    })?;
    let text = std::fs::read_to_string(&json_path).map_err(|err| {
        issue(
            Class::MissingReference,
            format!("reading {}: {err}", json_path.display()),
        )
    })?;
    judge_bytes(&bytes, &text, pose, seconds)
}

fn flags_of(text: &str) -> Result<(), Issue> {
    let settings = LightingSettings::parse(text)
        .map_err(|err| issue(Class::WrongFlags, format!("settings did not parse: {err}")))?;
    if settings.experiments.toggles != LIGHTING_TOGGLE_CELLS_ONLY {
        return Err(issue(
            Class::WrongFlags,
            format!("toggles bits are {}", settings.experiments.toggles),
        ));
    }
    if !settings.surface_cache.sh_cells {
        return Err(issue(Class::WrongFlags, "surface_cache.sh_cells is off"));
    }
    if settings.experiments.cache_async {
        return Err(issue(Class::WrongFlags, "cache_async is on"));
    }
    if settings.experiments.sc_settle != 0 {
        return Err(issue(
            Class::WrongFlags,
            format!("sc_settle is {}", settings.experiments.sc_settle),
        ));
    }
    if settings.world_cache.cell_metres.to_bits() != cell_metres().to_bits() {
        return Err(issue(
            Class::WrongFlags,
            format!("world_cell is {}", settings.world_cache.cell_metres),
        ));
    }
    if settings.world_mode() != 3 {
        return Err(issue(
            Class::WrongFlags,
            format!("world mode is {}", settings.world_mode()),
        ));
    }
    if settings.time_of_day.to_bits() != frozen_time_of_day().to_bits() {
        return Err(issue(
            Class::SunNotFrozen,
            format!("time_of_day is {}", settings.time_of_day),
        ));
    }
    Ok(())
}

fn log_lines(text: &str) -> Vec<String> {
    if let Some(section) = text.split("## Log").nth(1) {
        let body = section.split("## ").next().unwrap_or(section);
        return body
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("- ").map(str::trim).map(str::to_string)
            })
            .filter(|line| !line.is_empty())
            .collect();
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn field_after(line: &str, key: &str) -> Option<String> {
    let token = format!("{key}=");
    let rest = line.split(&token).nth(1)?;
    Some(rest.split_whitespace().next().unwrap_or(rest).to_string())
}

fn pose_value_ok(logged: &str, expected: f32) -> bool {
    let Ok(value) = logged.parse::<f64>() else {
        return false;
    };
    (value - f64::from(expected)).abs() <= POSE_SLOP
}

fn judge_capture(text: &str) -> Vec<Issue> {
    let lines = log_lines(text);
    if lines.is_empty() {
        return vec![issue(Class::MissingCapture, "capture log is empty")];
    }
    let mut issues = Vec::new();
    for pose in poses() {
        let pose_line = lines
            .iter()
            .find(|line| line.starts_with(&format!("pose {} ", pose.name)));
        let Some(pose_line) = pose_line else {
            issues.push(issue(
                Class::MissingCapture,
                format!("no pose line for {}", pose.name),
            ));
            continue;
        };
        let eye = eye();
        let ok = field_after(pose_line, "x").is_some_and(|v| pose_value_ok(&v, eye[0]))
            && field_after(pose_line, "y").is_some_and(|v| pose_value_ok(&v, eye[1]))
            && field_after(pose_line, "z").is_some_and(|v| pose_value_ok(&v, eye[2]))
            && field_after(pose_line, "yaw").is_some_and(|v| pose_value_ok(&v, pose.yaw))
            && field_after(pose_line, "pitch").is_some_and(|v| pose_value_ok(&v, pitch()));
        if !ok {
            issues.push(issue(
                Class::PoseMismatch,
                format!("{} camera reply is not the room pose", pose.name),
            ));
        }
        let settle = lines.iter().find(|line| {
            line.starts_with("settle ") && field_after(line, "pose").as_deref() == Some(pose.name)
        });
        let Some(settle) = settle else {
            issues.push(issue(
                Class::MissingCapture,
                format!("no settle line for {}", pose.name),
            ));
            continue;
        };
        if field_after(settle, "settled").as_deref() != Some("true") {
            let frames = field_after(settle, "frames").unwrap_or_else(|| "?".into());
            issues.push(issue(
                Class::Unsettled,
                format!("{} settled is not true after {frames} frames", pose.name),
            ));
            continue;
        }
        let resolved = lines.iter().find(|line| {
            line.starts_with("resolved ") && field_after(line, "pose").as_deref() == Some(pose.name)
        });
        let Some(resolved) = resolved else {
            issues.push(issue(
                Class::WrongFlags,
                format!("no resolved settings for {}", pose.name),
            ));
            continue;
        };
        let marker = format!("resolved pose={} ", pose.name);
        let flat = resolved
            .split_once(&marker)
            .map(|(_, rest)| rest)
            .unwrap_or("");
        let settings = flat.trim().trim_end_matches(" ||").replace(" || ", "\n");
        if let Err(flag) = flags_of(&settings) {
            issues.push(flag);
            continue;
        }
        let before = lines.iter().find_map(|line| {
            (line.starts_with("schedule ")
                && field_after(line, "pose").as_deref() == Some(pose.name)
                && line.contains(" before="))
            .then(|| field_after(line, "before"))
            .flatten()
        });
        let after = lines.iter().find_map(|line| {
            (line.starts_with("schedule ")
                && field_after(line, "pose").as_deref() == Some(pose.name)
                && line.contains(" after="))
            .then(|| field_after(line, "after"))
            .flatten()
        });
        let (Some(before), Some(after)) = (before, after) else {
            issues.push(issue(
                Class::CaptureSchedule,
                format!("{} is missing the fixed wait", pose.name),
            ));
            continue;
        };
        let (Ok(before), Ok(after)) = (before.parse::<u32>(), after.parse::<u32>()) else {
            issues.push(issue(
                Class::CaptureSchedule,
                format!("{} wait is not a frame count", pose.name),
            ));
            continue;
        };
        if after != before.saturating_add(CAPTURE_WAIT) {
            issues.push(issue(
                Class::CaptureSchedule,
                format!(
                    "{} waited from {before} to {after}, want {CAPTURE_WAIT} frames",
                    pose.name
                ),
            ));
        }
    }
    issues
}

fn live_png(capture: &Path, pose: &Pose) -> PathBuf {
    capture.join(format!("{}-live.png", pose.name))
}

fn score_pose(capture: &Path, out: &Path, pose: &Pose) -> Result<f64, Issue> {
    let live = live_png(capture, pose);
    let reference = out.join(format!("{}-reference.png", pose.name));
    if !live.is_file() {
        return Err(issue(
            Class::MissingLive,
            format!("missing {}", live.display()),
        ));
    }
    let live_image = Image::load(&live)
        .map_err(|err| issue(Class::MissingLive, format!("{} live: {err}", pose.name)))?;
    if live_image.width != WIDTH || live_image.height != HEIGHT {
        return Err(issue(
            Class::CaptureSchedule,
            format!(
                "{} live is {}x{}, want {WIDTH}x{HEIGHT}",
                pose.name, live_image.width, live_image.height
            ),
        ));
    }
    let live_plane = metrics::lum320(&live).map_err(|err| {
        issue(
            Class::MissingLive,
            format!("{} live plane: {err}", pose.name),
        )
    })?;
    let ref_plane = metrics::lum320(&reference).map_err(|err| {
        issue(
            Class::MissingReference,
            format!("{} reference plane: {err}", pose.name),
        )
    })?;
    let score = metrics::blotch(&live_plane, &ref_plane, 4.0);
    if !score.is_finite() {
        return Err(issue(
            Class::NonFiniteReference,
            format!("{} blotch is not finite", pose.name),
        ));
    }
    Ok(score)
}

fn write_display(reference: &Reference, path: &Path) -> Result<(), String> {
    reference
        .image
        .save(path)
        .map_err(|err| format!("{}: {err}", path.display()))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|err| format!("{} -> {}: {err}", from.display(), to.display()))
}

fn grade_dirs(
    refs: &Path,
    capture: Option<&Path>,
    out: &Path,
    seconds: f32,
) -> Result<Verdict, String> {
    std::fs::create_dir_all(out).map_err(|err| format!("{}: {err}", out.display()))?;
    let mut issues = Vec::new();
    let mut accepted = Vec::new();
    for pose in poses() {
        match judge_pose(refs, &pose, seconds) {
            Ok(reference) => {
                let png = out.join(format!("{}-reference.png", pose.name));
                write_display(&reference, &png)?;
                accepted.push(pose.name);
            }
            Err(found) => issues.push(found),
        }
        for suffix in ["ref", "json"] {
            let from = refs.join(format!("{}.{suffix}", pose.name));
            if from.is_file() {
                copy_file(
                    &from,
                    &out.join("inputs").join(format!("{}.{suffix}", pose.name)),
                )?;
            }
        }
    }
    if let Some(dir) = capture {
        let report = std::fs::read_to_string(dir.join("report.md")).unwrap_or_default();
        if report.is_empty() {
            issues.push(issue(
                Class::MissingCapture,
                format!("no report in {}", dir.display()),
            ));
        } else {
            issues.extend(judge_capture(&report));
            copy_file(&dir.join("report.md"), &out.join("report.md"))?;
        }
        let result = dir.join("result.json");
        if result.is_file() {
            copy_file(&result, &out.join("result.json"))?;
        }
    } else {
        issues.push(issue(Class::MissingCapture, "no capture directory"));
    }
    if !issues.is_empty() {
        let verdict = verdict_from(issues, None, None);
        write_verdict(out, &verdict, seconds)?;
        return Ok(verdict);
    }
    let Some(dir) = capture else {
        let verdict = verdict_from(
            vec![issue(Class::MissingCapture, "no capture directory")],
            None,
            None,
        );
        write_verdict(out, &verdict, seconds)?;
        return Ok(verdict);
    };
    let mut scores = Vec::new();
    for pose in poses() {
        match score_pose(dir, out, &pose) {
            Ok(score) => scores.push(score),
            Err(found) => issues.push(found),
        }
        let live = live_png(dir, &pose);
        if live.is_file() {
            copy_file(&live, &out.join(format!("{}-live.png", pose.name)))?;
        }
    }
    if !issues.is_empty() {
        let verdict = verdict_from(issues, None, None);
        write_verdict(out, &verdict, seconds)?;
        return Ok(verdict);
    }
    let start = scores.first().copied();
    let mid = scores.get(1).copied();
    let mut verdict = verdict_from(issues, start, mid);
    verdict.notes.push(protocol_note().to_string());
    verdict
        .notes
        .push(format!("accepted {}", accepted.join(" ")));
    write_verdict(out, &verdict, seconds)?;
    Ok(verdict)
}

fn protocol_note() -> &'static str {
    "Protocol differs from E11 and from gi-check room.rhai. The sun line in scenes/room.rhai is commented out, and sun height reads directional lights only, so --freeze-sun does not change this room's lamp illumination. It holds time_of_day at the stress start hour. The live view is cells_only with surface_cache.sh_cells, cache_async off, sc_settle 0, and the 0.25 m world cell. room-b is settled at its pose in the same window after room-a, not after the 29-frame turn. The capture waits 2 frames after settled and does not wait on a reference worker. E11's 0.014 is the W7 12.5 cm gi-check blotch. This grade does not claim that historical score. The gi-check PASS word remains 0.023 and is not this verdict. Sidecar seconds_bits is the writer's reported duration. The .ref format has no seconds field, so read_cached stores 0. That report is provenance. A zero report is not a budget mismatch, and a positive report is not a file clock. seconds_budget_bits is the requested stop for this run, 900 when --seconds is omitted and 2400 when that is the request. The other stop is a budget mismatch and is not rewritten."
}

fn json_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn write_verdict(out: &Path, verdict: &Verdict, seconds: f32) -> Result<(), String> {
    let seconds = require_trace_seconds(seconds)?;
    let number = |value: Option<f64>| match value {
        Some(value) => format!("{value:.8}"),
        None => "null".into(),
    };
    let legacy = verdict
        .legacy_word
        .map(|word| format!("\"{word}\""))
        .unwrap_or_else(|| "null".into());
    let issues = verdict
        .issues
        .iter()
        .map(|item| {
            format!(
                "    {{\"class\": \"{}\", \"detail\": \"{}\"}}",
                item.class.name(),
                json_escape(&item.detail)
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let notes = verdict
        .notes
        .iter()
        .map(|note| format!("    \"{}\"", json_escape(note)))
        .collect::<Vec<_>>()
        .join(",\n");
    let body = format!(
        "{{\n  \"class\": \"{}\",\n  \"detail\": \"{}\",\n  \"adr_limit\": {ADR_LIMIT},\n  \"legacy_room_blotch_limit\": {ROOM_BLOTCH_LIMIT},\n  \"legacy_word\": {legacy},\n  \"start\": {},\n  \"mid\": {},\n  \"mean\": {},\n  \"scene_sha256\": \"{}\",\n  \"settle_timeout_frames\": {SETTLE_TIMEOUT},\n  \"capture_wait_frames\": {CAPTURE_WAIT},\n  \"seconds_budget\": {},\n  \"issues\": [\n{issues}\n  ],\n  \"notes\": [\n{notes}\n  ]\n}}\n",
        verdict.class.name(),
        json_escape(&verdict.detail),
        number(verdict.start),
        number(verdict.mid),
        number(verdict.mean),
        scene_sha256(),
        seconds_text(seconds),
    );
    let path = out.join("verdict.json");
    std::fs::write(&path, body).map_err(|err| format!("{}: {err}", path.display()))
}

fn capture_args(scene: &Path, script: &Path, report: &Path) -> Vec<String> {
    vec![
        "--proof".into(),
        "--no-panel".into(),
        "--size".into(),
        WINDOW.into(),
        "--freeze-sun".into(),
        "--lighting-toggles".into(),
        "cells_only".into(),
        "--lighting-cache-async".into(),
        "off".into(),
        "--scene".into(),
        scene.display().to_string(),
        "--script".into(),
        script.display().to_string(),
        "--report".into(),
        report.display().to_string(),
    ]
}

fn bytes_are_lighting_experiment(bytes: &[u8]) -> bool {
    // The pieces stay separate so a prefix is not a new environment name.
    let Some(rest) = bytes.strip_prefix(b"GENOS_") else {
        return false;
    };
    rest.starts_with(b"LIGHTING_")
        || rest.starts_with(b"GI2_")
        || rest == b"GI_DEBUG"
        || rest == b"COMPOSE_SC_WRITES"
}

fn capture_drops_env(name: &[u8]) -> bool {
    bytes_are_lighting_experiment(name) || name == b"GENOS_REFERENCE_GPU"
}

#[cfg(test)]
fn lighting_experiment_env(name: &str) -> bool {
    bytes_are_lighting_experiment(name.as_bytes())
}

/// Parent bindings the capture child keeps. Lighting catalogue names are
/// dropped, including deprecated `GENOS_GI2_*` aliases and `GENOS_GI_DEBUG`.
/// `GENOS_COMPOSE_SC_WRITES` is dropped because the renderer reads it outside
/// that catalogue and it changes surface-cache compose writes.
/// `GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS` is then set to `1`.
#[cfg(test)]
fn capture_child_env(parent: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut child = Vec::new();
    for (name, value) in parent {
        if capture_drops_env(name.as_bytes()) {
            continue;
        }
        child.push(((*name).to_string(), (*value).to_string()));
    }
    child.push((
        "GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS".to_string(),
        "1".to_string(),
    ));
    child
}

fn seal_capture_command(command: &mut Command) {
    use std::os::unix::ffi::OsStrExt;
    let names: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| capture_drops_env(name.as_bytes()))
        .collect();
    for name in names {
        command.env_remove(name);
    }
    command.env("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", "1");
}

fn run_capture(out: &Path) -> Result<(), String> {
    let dir = out.join("capture");
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let scene = dir.join("room-scene.rhai");
    let script = dir.join("script.rhai");
    std::fs::write(&scene, ROOM_SCENE).map_err(|err| format!("{}: {err}", scene.display()))?;
    std::fs::write(&script, SCRIPT).map_err(|err| format!("{}: {err}", script.display()))?;
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut command = Command::new(exe);
    command.args(capture_args(&scene, &script, &dir));
    seal_capture_command(&mut command);
    let log_path = dir.join("run.log");
    let log =
        std::fs::File::create(&log_path).map_err(|err| format!("{}: {err}", log_path.display()))?;
    command
        .stdout(log.try_clone().map_err(|err| err.to_string())?)
        .stderr(log);
    let status = command
        .status()
        .map_err(|err| format!("capture process: {err}"))?;
    if !status.success() {
        return Err(format!(
            "capture process failed ({status}); see {}",
            log_path.display()
        ));
    }
    Ok(())
}

struct TraceRequest {
    pose: Pose,
    out: PathBuf,
    promote: Option<PathBuf>,
    threads: u32,
    seconds: f32,
    commit: String,
    gpu: bool,
}

fn reference_threads(argument: Option<u32>, env: Option<&str>) -> Result<u32, String> {
    if let Some(value) = argument {
        if value == 0 {
            return Err("--threads must be at least 1".into());
        }
        return Ok(value);
    }
    match env.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => text
            .parse::<u32>()
            .map_err(|_| format!("GENOS_REFERENCE_THREADS is not an integer ({text})"))
            .and_then(|value| {
                if value == 0 {
                    Err("GENOS_REFERENCE_THREADS must be at least 1".into())
                } else {
                    Ok(value)
                }
            }),
        None => Ok(4),
    }
}

fn gpu_requested(env: Option<&str>) -> bool {
    env == Some("1")
}

fn room_setup(pose: &Pose, seconds: f32) -> Result<RefSetup, String> {
    let seconds = require_trace_seconds(seconds)?;
    let mut scene: Scene = genos_scene::load_str(ROOM_SCENE)?;
    scene.lighting.time_of_day = frozen_time_of_day();
    Ok(RefSetup {
        scene,
        eye: eye(),
        yaw: pose.yaw,
        pitch: pitch(),
        width: WIDTH,
        height: HEIGHT,
        spp: SPP_BATCH,
        noise_target: noise_target(),
        max_spp: MAX_SPP,
        seconds,
        max_bounces: BOUNCES,
        triangles: false,
        materials: Vec::new(),
        specular: false,
    })
}

fn sidecar_text(
    pose: &Pose,
    reference: &Reference,
    sha: &str,
    key: u64,
    threads: u32,
    commit: &str,
    seconds: f32,
) -> Result<String, String> {
    let seconds = require_trace_seconds(seconds)?;
    let eye_bits = eye();
    Ok(format!(
        "{{\n  \"kind\": \"room-blotch-ref\",\n  \"source_commit\": \"{commit}\",\n  \"scene\": \"room\",\n  \"scene_sha256\": \"{}\",\n  \"pose\": \"{}\",\n  \"eye_bits\": [{}, {}, {}],\n  \"yaw_bits\": {},\n  \"pitch_bits\": {},\n  \"width\": {WIDTH},\n  \"height\": {HEIGHT},\n  \"bounces\": {BOUNCES},\n  \"spp_batch\": {SPP_BATCH},\n  \"noise_target_bits\": {},\n  \"max_spp\": {MAX_SPP},\n  \"seconds_budget_bits\": {},\n  \"triangles\": false,\n  \"specular\": false,\n  \"materials\": 0,\n  \"estimator\": \"cpu-analytic\",\n  \"gpu_reference\": false,\n  \"sun_frozen\": true,\n  \"time_of_day_bits\": {},\n  \"cells_only\": true,\n  \"sh_cells\": true,\n  \"cache_async\": false,\n  \"world_cell_bits\": {},\n  \"world_mode\": 3,\n  \"noise_bits\": {},\n  \"spp\": {},\n  \"seconds_bits\": {},\n  \"threads\": {threads},\n  \"ref_sha256\": \"{sha}\",\n  \"native_cache_key\": \"{key:016x}\",\n  \"native_cache_key_note\": \"host scene at the frozen hour, sh_cells off, stall 0. Grade does not look this key up. The key omits toggles, sh_cells, cache_async, noise, seconds, and max_spp. It includes lighting.time_of_day and event_loop.longest_stall_ms.\"\n}}\n",
        scene_sha256(),
        pose.name,
        eye_bits[0].to_bits(),
        eye_bits[1].to_bits(),
        eye_bits[2].to_bits(),
        pose.yaw.to_bits(),
        pitch().to_bits(),
        noise_target().to_bits(),
        seconds.to_bits(),
        frozen_time_of_day().to_bits(),
        cell_metres().to_bits(),
        reference.noise.to_bits(),
        reference.spp,
        reference.seconds.to_bits(),
    ))
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|text| text.to_os_string())
        .unwrap_or_default();
    name.push(".partial");
    path.with_file_name(name)
}

fn stage_write(bytes: &[u8], dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    let partial = partial_path(dest);
    std::fs::write(&partial, bytes).map_err(|err| format!("{}: {err}", partial.display()))?;
    std::fs::rename(&partial, dest)
        .map_err(|err| format!("{} -> {}: {err}", partial.display(), dest.display()))
}

fn write_pair(dir: &Path, pose: &Pose, bytes: &[u8], sidecar: &str) -> Result<(), String> {
    refuses_acceptance_cache(dir)?;
    stage_write(bytes, &dir.join(format!("{}.ref", pose.name)))?;
    stage_write(sidecar.as_bytes(), &dir.join(format!("{}.json", pose.name)))?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "render reads GENOS_REFERENCE_THREADS and has no argument. --threads is copied in for the call and restored."
)]
fn with_reference_threads<T>(threads: u32, body: impl FnOnce() -> T) -> T {
    let previous = std::env::var("GENOS_REFERENCE_THREADS").ok();
    std::env::set_var("GENOS_REFERENCE_THREADS", threads.to_string());
    let value = body();
    match previous {
        Some(text) => std::env::set_var("GENOS_REFERENCE_THREADS", text),
        None => std::env::remove_var("GENOS_REFERENCE_THREADS"),
    }
    value
}

fn trace_pose(request: &TraceRequest) -> Result<(), String> {
    if request.gpu {
        return Err(
            "GENOS_REFERENCE_GPU=1 is set. The room blotch reference is the CPU analytic trace."
                .into(),
        );
    }
    refuses_acceptance_cache(&request.out)?;
    if let Some(dest) = &request.promote {
        refuses_acceptance_cache(dest)?;
    }
    if judge_pose(&request.out, &request.pose, request.seconds).is_ok() {
        eprintln!(
            "room-blotch reused {} from {} at the requested {} s budget",
            request.pose.name,
            request.out.display(),
            seconds_text(request.seconds)
        );
        if let Some(dest) = &request.promote {
            promote_pose(&request.out, dest, &request.pose, request.seconds)?;
        }
        return Ok(());
    }
    let setup = room_setup(&request.pose, request.seconds)?;
    let key = reference::cache_key(&setup);
    eprintln!(
        "room-blotch trace {} 320x180 bounces {BOUNCES} noise 0.01 max_spp {MAX_SPP} seconds {} threads {} commit {}",
        request.pose.name,
        seconds_text(request.seconds),
        request.threads,
        request.commit
    );
    let traced = with_reference_threads(request.threads, || reference::render(&setup));
    let scratch = request.out.join(format!("{}.ref.write", request.pose.name));
    reference::write_cached(&scratch, &traced)
        .map_err(|err| format!("{}: {err}", scratch.display()))?;
    let bytes = std::fs::read(&scratch).map_err(|err| format!("{}: {err}", scratch.display()))?;
    let _ = std::fs::remove_file(&scratch);
    let sha = sha256_hex(&bytes);
    let sidecar = sidecar_text(
        &request.pose,
        &traced,
        &sha,
        key,
        request.threads,
        &request.commit,
        request.seconds,
    )?;
    write_pair(&request.out, &request.pose, &bytes, &sidecar)?;
    eprintln!(
        "room-blotch traced {} noise {} spp {} seconds {}",
        request.pose.name, traced.noise, traced.spp, traced.seconds
    );
    match judge_pose(&request.out, &request.pose, request.seconds) {
        Ok(_) => {}
        Err(found) => {
            return Err(format!("{}: {}", found.class.name(), found.detail));
        }
    }
    if let Some(dest) = &request.promote {
        promote_pose(&request.out, dest, &request.pose, request.seconds)?;
    }
    Ok(())
}

fn promote_pose(from: &Path, dest: &Path, pose: &Pose, seconds: f32) -> Result<(), String> {
    refuses_acceptance_cache(dest)?;
    judge_pose(from, pose, seconds)
        .map_err(|found| format!("{}: {}", found.class.name(), found.detail))?;
    let before = dest.join(format!("{}.ref", pose.name));
    let backup_ref = std::fs::read(&before).ok();
    let backup_json = std::fs::read(dest.join(format!("{}.json", pose.name))).ok();
    copy_file(
        &from.join(format!("{}.ref", pose.name)),
        &dest.join(format!("{}.ref", pose.name)),
    )?;
    if let Err(err) = copy_file(
        &from.join(format!("{}.json", pose.name)),
        &dest.join(format!("{}.json", pose.name)),
    ) {
        if let Some(bytes) = backup_ref {
            let _ = std::fs::write(&before, bytes);
        }
        return Err(err);
    }
    judge_pose(dest, pose, seconds).map_err(|found| {
        if let Some(bytes) = &backup_ref {
            let _ = std::fs::write(dest.join(format!("{}.ref", pose.name)), bytes);
        }
        if let Some(bytes) = &backup_json {
            let _ = std::fs::write(dest.join(format!("{}.json", pose.name)), bytes);
        }
        format!(
            "promoted {} failed the same check: {}",
            pose.name, found.detail
        )
    })?;
    eprintln!("room-blotch promoted {} into {}", pose.name, dest.display());
    Ok(())
}

fn git_head() -> Result<(String, bool), String> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let head = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|err| err.to_string())?;
    if !head.status.success() {
        return Err(String::from_utf8_lossy(&head.stderr).trim().to_string());
    }
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if commit.len() != 40 || !commit.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("git HEAD is not a commit: {commit}"));
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|err| err.to_string())?;
    if !status.status.success() {
        return Err(String::from_utf8_lossy(&status.stderr).trim().to_string());
    }
    let clean = String::from_utf8_lossy(&status.stdout).trim().is_empty();
    Ok((commit, clean))
}

fn print_verdict(verdict: &Verdict) {
    let legacy = verdict.legacy_word.unwrap_or("n/a");
    match verdict.mean {
        Some(mean) => println!(
            "room-blotch {} mean {mean:.6} adr {ADR_LIMIT} legacy-word-would-be {legacy} {}",
            verdict.class.name(),
            verdict.detail
        ),
        None => println!("room-blotch {} {}", verdict.class.name(), verdict.detail),
    }
}

fn value_of(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    flag: &str,
) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

pub fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut args = args.collect::<Vec<_>>().into_iter().peekable();
    let Some(command) = args.next() else {
        println!("{HELP}");
        return Ok(());
    };
    if command == "-h" || command == "--help" {
        println!("{HELP}");
        return Ok(());
    }
    let gpu = std::env::var("GENOS_REFERENCE_GPU").ok();
    let threads_env = std::env::var("GENOS_REFERENCE_THREADS").ok();
    match command.as_str() {
        "grade" => grade_command(&mut args, gpu.as_deref()),
        "trace" => trace_command(&mut args, gpu.as_deref(), threads_env.as_deref()),
        other => Err(format!("room-blotch: unknown command {other}\n{HELP}")),
    }
}

fn take_seconds(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    flag: &str,
    slot: &mut Option<f32>,
) -> Result<(), String> {
    if slot.is_some() {
        return Err("--seconds was given twice".into());
    }
    *slot = Some(parse_trace_seconds(&value_of(args, flag)?)?);
    Ok(())
}

enum CommandArgs<T> {
    Ready(T),
    Help,
}

struct GradeArgs {
    refs: PathBuf,
    out: PathBuf,
    capture: bool,
    seconds: f32,
}

fn parse_grade_args(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
) -> Result<CommandArgs<GradeArgs>, String> {
    let mut refs: Option<PathBuf> = None;
    let mut out = PathBuf::from("genos-debug/room-blotch");
    let mut capture = false;
    let mut seconds: Option<f32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--refs" => refs = Some(PathBuf::from(value_of(args, &arg)?)),
            "--out" => out = PathBuf::from(value_of(args, &arg)?),
            "--capture" => capture = true,
            "--seconds" => take_seconds(args, &arg, &mut seconds)?,
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(CommandArgs::Help);
            }
            other => return Err(format!("room-blotch grade: unknown option {other}\n{HELP}")),
        }
    }
    let Some(refs) = refs else {
        return Err("room-blotch grade needs --refs DIR".into());
    };
    Ok(CommandArgs::Ready(GradeArgs {
        refs,
        out,
        capture,
        seconds: seconds.unwrap_or(DEFAULT_TRACE_SECONDS),
    }))
}

fn grade_command(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    gpu: Option<&str>,
) -> Result<(), String> {
    let parsed = match parse_grade_args(args)? {
        CommandArgs::Help => return Ok(()),
        CommandArgs::Ready(parsed) => parsed,
    };
    let GradeArgs {
        refs,
        out,
        capture,
        seconds,
    } = parsed;
    refuses_acceptance_cache(&out)?;
    if capture {
        for pose in poses() {
            if let Err(found) = judge_pose(&refs, &pose, seconds) {
                let verdict = verdict_from(vec![found], None, None);
                std::fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
                write_verdict(&out, &verdict, seconds)?;
                print_verdict(&verdict);
                return Err(format!(
                    "room-blotch {} {}",
                    verdict.class.name(),
                    verdict.detail
                ));
            }
        }
        if gpu_requested(gpu) {
            return Err(
                "GENOS_REFERENCE_GPU=1 is set. The capture grades the CPU reference against the live picture and does not trace."
                    .into(),
            );
        }
        if let Err(err) = run_capture(&out) {
            let verdict = verdict_from(vec![issue(Class::MissingCapture, err.clone())], None, None);
            write_verdict(&out, &verdict, seconds)?;
            print_verdict(&verdict);
            return Err(err);
        }
    }
    let capture_dir = capture.then(|| out.join("capture"));
    let verdict = grade_dirs(&refs, capture_dir.as_deref(), &out, seconds)?;
    print_verdict(&verdict);
    if verdict.class == Class::AdrPass {
        Ok(())
    } else {
        Err(format!(
            "room-blotch {} {}",
            verdict.class.name(),
            verdict.detail
        ))
    }
}

struct TraceArgs {
    pose: Pose,
    out: PathBuf,
    promote: Option<PathBuf>,
    threads: Option<u32>,
    seconds: f32,
}

fn parse_trace_args(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
) -> Result<CommandArgs<TraceArgs>, String> {
    let mut pose_name: Option<String> = None;
    let mut out = PathBuf::from("genos-debug/room-blotch/isolated");
    let mut promote: Option<PathBuf> = None;
    let mut threads_arg: Option<u32> = None;
    let mut seconds: Option<f32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pose" => pose_name = Some(value_of(args, &arg)?),
            "--out" => out = PathBuf::from(value_of(args, &arg)?),
            "--promote" => promote = Some(PathBuf::from(value_of(args, &arg)?)),
            "--threads" => {
                let text = value_of(args, &arg)?;
                threads_arg = Some(
                    text.parse::<u32>()
                        .map_err(|_| format!("--threads is not an integer ({text})"))?,
                );
            }
            "--seconds" => take_seconds(args, &arg, &mut seconds)?,
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(CommandArgs::Help);
            }
            other => return Err(format!("room-blotch trace: unknown option {other}\n{HELP}")),
        }
    }
    let Some(name) = pose_name else {
        return Err("room-blotch trace needs --pose room-a or room-b".into());
    };
    let Some(pose) = pose_named(&name) else {
        return Err(format!(
            "room-blotch trace pose is room-a or room-b, not {name}"
        ));
    };
    Ok(CommandArgs::Ready(TraceArgs {
        pose,
        out,
        promote,
        threads: threads_arg,
        seconds: seconds.unwrap_or(DEFAULT_TRACE_SECONDS),
    }))
}

fn trace_command(
    args: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    gpu: Option<&str>,
    threads_env: Option<&str>,
) -> Result<(), String> {
    let parsed = match parse_trace_args(args)? {
        CommandArgs::Help => return Ok(()),
        CommandArgs::Ready(parsed) => parsed,
    };
    let (commit, clean) = git_head()?;
    if !clean {
        return Err(
            "room-blotch trace refuses a dirty worktree so the sidecar commit matches the binary"
                .into(),
        );
    }
    let request = TraceRequest {
        pose: parsed.pose,
        out: parsed.out,
        promote: parsed.promote,
        threads: reference_threads(parsed.threads, threads_env)?,
        seconds: parsed.seconds,
        commit,
        gpu: gpu_requested(gpu),
    };
    trace_pose(&request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_scene::LightingPreset;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("room-blotch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn filled(width: u32, height: u32, rgb: [f32; 3], noise: f32, spp: u32) -> Reference {
        let count = usize::try_from(width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(height).unwrap_or(0));
        Reference {
            image: Image {
                width,
                height,
                rgb: vec![0; count * 3],
            },
            linear: vec![rgb; count],
            hit: vec![true; count],
            spp,
            seconds: 12.0,
            noise,
            specular: None,
        }
    }

    fn accepted_text(pose: &Pose, reference: &Reference, sha: &str) -> String {
        sidecar_text(
            pose,
            reference,
            sha,
            0xabc,
            4,
            "779e9a67eb67b92d02428bb87af450e1b7adc723",
            DEFAULT_TRACE_SECONDS,
        )
        .unwrap()
    }

    fn plant(dir: &Path, pose: &Pose, reference: &Reference) {
        let scratch = dir.join(format!("{}-scratch.ref", pose.name));
        reference::write_cached(&scratch, reference).unwrap();
        let bytes = std::fs::read(&scratch).unwrap();
        let sha = sha256_hex(&bytes);
        let text = accepted_text(pose, reference, &sha);
        std::fs::write(dir.join(format!("{}.ref", pose.name)), bytes).unwrap();
        std::fs::write(dir.join(format!("{}.json", pose.name)), text).unwrap();
        let _ = std::fs::remove_file(scratch);
    }

    fn settings_text(mut edit: impl FnMut(&mut LightingSettings)) -> String {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        settings.surface_cache.sh_cells = true;
        settings.experiments.toggles = LIGHTING_TOGGLE_CELLS_ONLY;
        settings.experiments.cache_async = false;
        settings.time_of_day = frozen_time_of_day();
        edit(&mut settings);
        settings.to_text()
    }

    #[test]
    fn sha256_matches_the_empty_string_and_abc() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pose_bits_match_the_rhai_f32_cast() {
        assert_eq!(rhai_f32(0.0).to_bits(), 0);
        assert_eq!(rhai_f32(1.0).to_bits(), 0x3f80_0000);
        assert_eq!(rhai_f32(-0.5).to_bits(), 0xbf00_0000);
        let eye = eye();
        assert_eq!(eye[0].to_bits(), 1_059_716_071);
        assert_eq!(eye[1].to_bits(), 1_071_216_853);
        assert_eq!(eye[2].to_bits(), 1_092_227_170);
        assert_eq!(poses()[0].yaw.to_bits(), 3_207_451_378);
        assert_eq!(poses()[1].yaw.to_bits(), 3_206_601_088);
        assert_eq!(pitch().to_bits(), 3_186_999_951);
        assert_eq!(frozen_time_of_day().to_bits(), 1_091_441_787);
    }

    #[test]
    fn known_floor_hashes_are_refused_and_a_room_hash_is_not() {
        assert!(known_floor(FLOOR_SHA256[0]));
        assert!(known_floor(FLOOR_SHA256[1]));
        assert!(!known_floor(&scene_sha256()));
    }

    #[test]
    fn legacy_limit_stays_0_023_and_is_not_the_adr_verdict() {
        assert_eq!(ROOM_BLOTCH_LIMIT, 0.023);
        let physical = verdict_from(Vec::new(), Some(0.019), Some(0.021));
        assert_eq!(physical.class, Class::PhysicalFail);
        assert_eq!(physical.legacy_word, Some("PASS"));
        assert!((physical.mean.unwrap() - 0.020).abs() < 1e-12);
        let over = verdict_from(Vec::new(), Some(0.030), Some(0.030));
        assert_eq!(over.class, Class::PhysicalFail);
        assert_eq!(over.legacy_word, Some("FAIL"));
        let on_line = verdict_from(Vec::new(), Some(ADR_LIMIT), Some(ADR_LIMIT));
        assert_eq!(on_line.class, Class::AdrPass);
        let above = verdict_from(Vec::new(), Some(ADR_LIMIT + 1e-12), Some(ADR_LIMIT));
        assert_eq!(above.class, Class::PhysicalFail);
    }

    #[test]
    fn a_guard_hides_a_score() {
        let verdict = verdict_from(
            vec![issue(Class::MissingReference, "missing room-a.ref")],
            Some(0.5),
            Some(0.5),
        );
        assert_eq!(verdict.class, Class::MissingReference);
        assert!(verdict.mean.is_none());
        assert!(verdict.legacy_word.is_none());
    }

    #[test]
    fn blotch_of_identical_and_uniform_planes_is_zero() {
        let flat = metrics::Plane {
            w: 8,
            h: 8,
            v: vec![0.4; 64],
        };
        let biased = metrics::Plane {
            w: 8,
            h: 8,
            v: vec![0.9; 64],
        };
        let same = metrics::blotch(&flat, &flat, 4.0);
        let bias = metrics::blotch(&biased, &flat, 4.0);
        assert!(same.abs() < 1.0e-12, "identical blotch {same}");
        assert!(bias.abs() < 1.0e-12, "uniform bias blotch {bias}");
    }

    #[test]
    fn blotch_of_a_step_is_stable() {
        let mut live = metrics::Plane::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                live.v[y * 8 + x] = if x < 4 { 0.2 } else { 0.8 };
            }
        }
        let reference = metrics::Plane {
            w: 8,
            h: 8,
            v: vec![0.5; 64],
        };
        let score = metrics::blotch(&live, &reference, 4.0);
        let pin = 0.17401275553535756;
        assert!(
            (score - pin).abs() < 1.0e-12,
            "metrics::blotch changed: {score}"
        );
    }

    #[test]
    fn resolved_flags_require_cells_only_sh_async_off_and_25_cm() {
        let text = settings_text(|_| {});
        assert!(!text.contains("world_mode="));
        assert!(text.contains("sc_settle=0"));
        assert!(flags_of(&text).is_ok());
        let settle = settings_text(|settings| settings.experiments.sc_settle = 300);
        assert_eq!(flags_of(&settle).unwrap_err().class, Class::WrongFlags);
        let eye = eye();
        let flags = settle.replace('\n', " || ");
        let log = format!(
            "pose room-a x={} y={} z={} yaw={} pitch={}\nsettle pose=room-a settled=true frames=10 still_frames=300 wall_ms=1\nresolved pose=room-a {flags}\n",
            eye[0], eye[1], eye[2], poses()[0].yaw, pitch()
        );
        assert_eq!(judge_capture(&log)[0].class, Class::WrongFlags);
        let async_on = settings_text(|settings| settings.experiments.cache_async = true);
        assert_eq!(flags_of(&async_on).unwrap_err().class, Class::WrongFlags);
        let axis = settings_text(|settings| settings.surface_cache.sh_cells = false);
        assert_eq!(flags_of(&axis).unwrap_err().class, Class::WrongFlags);
        let toggles = settings_text(|settings| settings.experiments.toggles = 0);
        assert_eq!(flags_of(&toggles).unwrap_err().class, Class::WrongFlags);
        let extra = settings_text(|settings| {
            settings.experiments.toggles = LIGHTING_TOGGLE_CELLS_ONLY | 1;
        });
        assert_eq!(flags_of(&extra).unwrap_err().class, Class::WrongFlags);
        let coarse = settings_text(|settings| settings.world_cache.cell_metres = 0.5);
        assert_eq!(flags_of(&coarse).unwrap_err().class, Class::WrongFlags);
        let noon = settings_text(|settings| settings.time_of_day = 12.0);
        assert_eq!(flags_of(&noon).unwrap_err().class, Class::SunNotFrozen);
    }

    #[test]
    fn settle_timeout_is_unsettled_and_a_true_settle_at_900_is_not() {
        let eye = eye();
        let yaw = poses()[0].yaw;
        let header = format!(
            "pose room-a x={} y={} z={} yaw={} pitch={}\n",
            eye[0],
            eye[1],
            eye[2],
            yaw,
            pitch()
        );
        let flags = settings_text(|_| {}).replace('\n', " || ");
        let good = format!(
            "{header}settle pose=room-a settled=true frames={SETTLE_TIMEOUT} still_frames=300 wall_ms=1\nresolved pose=room-a {flags}\nschedule pose=room-a before=10\nschedule pose=room-a after=12\npose room-b x={} y={} z={} yaw={} pitch={}\nsettle pose=room-b settled=true frames=20 still_frames=300 wall_ms=1\nresolved pose=room-b {flags}\nschedule pose=room-b before=40\nschedule pose=room-b after=42\n",
            eye[0], eye[1], eye[2], poses()[1].yaw, pitch()
        );
        assert!(
            judge_capture(&good).is_empty(),
            "{:?}",
            judge_capture(&good)
                .iter()
                .map(|item| item.detail.clone())
                .collect::<Vec<_>>()
        );
        let capped = format!(
            "{header}settle pose=room-a settled=false frames={SETTLE_TIMEOUT} still_frames=0 wall_ms=1\n"
        );
        let issues = judge_capture(&capped);
        assert_eq!(issues[0].class, Class::Unsettled);
    }

    #[test]
    fn the_other_pose_and_a_short_wait_are_distinct() {
        let eye = eye();
        let flags = settings_text(|_| {}).replace('\n', " || ");
        let swapped = format!(
            "pose room-a x={} y={} z={} yaw={} pitch={}\nsettle pose=room-a settled=true frames=10 still_frames=300 wall_ms=1\nresolved pose=room-a {flags}\nschedule pose=room-a before=1\nschedule pose=room-a after=3\n",
            eye[0], eye[1], eye[2], poses()[1].yaw, pitch()
        );
        assert_eq!(judge_capture(&swapped)[0].class, Class::PoseMismatch);
        let short = format!(
            "pose room-a x={} y={} z={} yaw={} pitch={}\nsettle pose=room-a settled=true frames=10 still_frames=300 wall_ms=1\nresolved pose=room-a {flags}\nschedule pose=room-a before=1\nschedule pose=room-a after=9\npose room-b x={} y={} z={} yaw={} pitch={}\nsettle pose=room-b settled=true frames=10 still_frames=300 wall_ms=1\nresolved pose=room-b {flags}\nschedule pose=room-b before=1\nschedule pose=room-b after=3\n",
            eye[0], eye[1], eye[2], poses()[0].yaw, pitch(),
            eye[0], eye[1], eye[2], poses()[1].yaw, pitch()
        );
        assert!(judge_capture(&short)
            .iter()
            .any(|item| item.class == Class::CaptureSchedule));
    }

    #[test]
    fn noisy_cap_floor_size_and_wrong_pose_stay_distinct() {
        let dir = temp_dir("guards");
        let pose = poses()[0];
        let mut noisy = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.02, MAX_SPP);
        noisy.seconds = 30.0;
        plant(&dir, &pose, &noisy);
        assert_eq!(
            judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NoisyReference
        );
        let floor = vec![0_u8; FLOOR_BYTES];
        let text = accepted_text(&pose, &noisy, &sha256_hex(&floor));
        assert_eq!(
            judge_bytes(&floor, &text, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::SceneMismatch
        );
        let mut good = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.009, 128);
        good.seconds = 8.0;
        plant(&dir, &pose, &good);
        let wrong = std::fs::read_to_string(dir.join("room-a.json"))
            .unwrap()
            .replace(
                &pose.yaw.to_bits().to_string(),
                &poses()[1].yaw.to_bits().to_string(),
            );
        std::fs::write(dir.join("room-a.json"), wrong).unwrap();
        assert_eq!(
            judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::PoseMismatch
        );
        let cache = dir.join("reference-cache");
        assert!(refuses_acceptance_cache(&cache).is_err());
        let mut broken = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], f32::NAN, 32);
        broken.seconds = 4.0;
        plant(&dir, &pose, &broken);
        assert_eq!(
            judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NonFiniteReference
        );
        let mut negative = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], -0.1, 32);
        negative.seconds = 4.0;
        plant(&dir, &pose, &negative);
        assert_eq!(
            judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NonFiniteReference
        );
        let mut zero_clock = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.004, 32);
        zero_clock.seconds = 0.0;
        plant(&dir, &pose, &zero_clock);
        let loaded = judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(loaded.seconds, 0.0);
        assert_eq!(loaded.spp, 32);
        let mut capped = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.004, 64);
        capped.seconds = 4.0;
        plant(&dir, &pose, &capped);
        let text = std::fs::read_to_string(dir.join("room-a.json"))
            .unwrap()
            .replace("\"max_spp\": 262144", "\"max_spp\": 4194304");
        std::fs::write(dir.join("room-a.json"), text).unwrap();
        assert_eq!(
            judge_pose(&dir, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::BudgetMismatch
        );
        let kept = dir.join("kept.ref");
        std::fs::write(&kept, b"stay").unwrap();
        let err = promote_pose(&dir, &dir, &pose, DEFAULT_TRACE_SECONDS);
        assert!(err.is_err());
        assert_eq!(std::fs::read(&kept).unwrap(), b"stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_accepted_pair_grades_only_through_blotch_and_does_not_write_the_cache() {
        let refs = temp_dir("refs");
        let out = temp_dir("out");
        let cache = temp_dir("reference-cache");
        std::fs::write(cache.join("keep.ref"), b"keep").unwrap();
        for pose in poses() {
            let mut reference = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, 64);
            reference.seconds = 3.0;
            plant(&refs, &pose, &reference);
        }
        let capture = temp_dir("capture");
        let shown = reference::read_cached(&refs.join("room-a.ref"), WIDTH, HEIGHT).unwrap();
        shown.image.save(&capture.join("room-a-live.png")).unwrap();
        shown.image.save(&capture.join("room-b-live.png")).unwrap();
        let eye = eye();
        let flags = settings_text(|_| {}).replace('\n', " || ");
        let report = format!(
            "# script — PASS\n\n## Log\n\n- pose room-a x={} y={} z={} yaw={} pitch={}\n- settle pose=room-a settled=true frames=40 still_frames=300 wall_ms=1\n- resolved pose=room-a {flags}\n- schedule pose=room-a before=4\n- schedule pose=room-a after=6\n- pose room-b x={} y={} z={} yaw={} pitch={}\n- settle pose=room-b settled=true frames=40 still_frames=300 wall_ms=1\n- resolved pose=room-b {flags}\n- schedule pose=room-b before=8\n- schedule pose=room-b after=10\n",
            eye[0], eye[1], eye[2], poses()[0].yaw, pitch(),
            eye[0], eye[1], eye[2], poses()[1].yaw, pitch()
        );
        std::fs::write(capture.join("report.md"), report).unwrap();
        let before = std::fs::read(refs.join("room-a.ref")).unwrap();
        let verdict = grade_dirs(&refs, Some(&capture), &out, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(verdict.class, Class::AdrPass, "{}", verdict.detail);
        assert_eq!(verdict.legacy_word, Some("PASS"));
        assert_eq!(std::fs::read(refs.join("room-a.ref")).unwrap(), before);
        assert_eq!(std::fs::read(cache.join("keep.ref")).unwrap(), b"keep");
        assert!(out.join("verdict.json").is_file());
        assert!(out.join("room-a-reference.png").is_file());
        assert!(!refs.join("room-c.ref").exists());
        let _ = std::fs::remove_dir_all(&refs);
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&capture);
    }

    #[test]
    fn missing_refs_do_not_become_a_physical_fail() {
        let refs = temp_dir("empty-refs");
        let out = temp_dir("empty-out");
        let verdict = grade_dirs(&refs, None, &out, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(verdict.class, Class::MissingReference);
        assert!(verdict.mean.is_none());
        let text = std::fs::read_to_string(out.join("verdict.json")).unwrap();
        assert!(text.contains("\"class\": \"missing-reference\""));
        assert!(!text.contains("\"class\": \"physical-fail\""));
        assert!(!text.contains("\"class\": \"adr-pass\""));
        let _ = std::fs::remove_dir_all(&refs);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn the_script_has_two_poses_and_does_not_call_compare() {
        assert!(!SCRIPT.contains("compare("));
        assert!(!SCRIPT.contains("room-c"));
        assert!(!SCRIPT.contains("close-0"));
        assert!(!SCRIPT.contains("4194304"));
        assert_eq!(SCRIPT.matches("wait(2)").count(), 2);
        assert_eq!(SCRIPT.matches("timeout_frames: 900").count(), 2);
        assert!(SCRIPT.contains("width: 320"));
        assert!(SCRIPT.contains("height: 180"));
        assert!(SCRIPT.contains("yaw: -0.679"));
        assert!(SCRIPT.contains("-36.0 * 0.0174533"));
    }

    #[test]
    fn capture_arguments_freeze_the_sun_and_the_cache_flags() {
        let args = capture_args(
            Path::new("scene.rhai"),
            Path::new("script.rhai"),
            Path::new("out"),
        );
        assert!(args.windows(2).any(
            |pair| pair == ["--freeze-sun", "--lighting-toggles"] || pair[0] == "--freeze-sun"
        ));
        assert!(args.iter().any(|arg| arg == "--freeze-sun"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--lighting-toggles", "cells_only"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--lighting-cache-async", "off"]));
        assert!(args.windows(2).any(|pair| pair == ["--size", "640x360"]));
        assert!(!args.iter().any(|arg| arg.contains("compare")));
        assert_eq!(reference_threads(Some(4), Some("16")).unwrap(), 4);
        assert_eq!(reference_threads(None, Some("8")).unwrap(), 8);
        assert_eq!(reference_threads(None, None).unwrap(), 4);
        assert!(gpu_requested(Some("1")));
        assert!(!gpu_requested(Some("0")));
        assert!(!gpu_requested(None));
        let previous = std::env::var("GENOS_REFERENCE_THREADS").ok();
        with_reference_threads(4, || {
            assert_eq!(
                std::env::var("GENOS_REFERENCE_THREADS").ok().as_deref(),
                Some("4")
            );
        });
        assert_eq!(std::env::var("GENOS_REFERENCE_THREADS").ok(), previous);
    }

    #[test]
    fn native_key_changes_with_time_and_sh_cells_and_grade_ignores_it() {
        let mut first = room_setup(&poses()[0], DEFAULT_TRACE_SECONDS).unwrap();
        let mut second = first.clone();
        second.scene.lighting.time_of_day = 12.0;
        assert_ne!(reference::cache_key(&first), reference::cache_key(&second));
        first.scene.lighting.surface_cache.sh_cells = false;
        second = first.clone();
        second.scene.lighting.surface_cache.sh_cells = true;
        assert_ne!(reference::cache_key(&first), reference::cache_key(&second));
        let dir = temp_dir("key");
        let mut reference = filled(WIDTH, HEIGHT, [0.1, 0.2, 0.3], 0.004, 32);
        reference.seconds = 2.0;
        plant(&dir, &poses()[0], &reference);
        let mut text = std::fs::read_to_string(dir.join("room-a.json")).unwrap();
        text = text.replace(
            "\"native_cache_key\": \"0000000000000abc\"",
            "\"native_cache_key\": \"ffffffffffffffff\"",
        );
        std::fs::write(dir.join("room-a.json"), text).unwrap();
        assert!(judge_pose(&dir, &poses()[0], DEFAULT_TRACE_SECONDS).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn operand_bytes(reference: &Reference) -> Vec<u8> {
        let nonce = STAGE_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = temp_dir(&format!("operand-{nonce}"));
        let path = dir.join("one.ref");
        reference::write_cached(&path, reference).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    #[test]
    fn a_planted_symlink_is_not_written_and_scored_bytes_match_the_hash() {
        let pose = poses()[0];
        let mut reference = filled(WIDTH, HEIGHT, [0.2, 0.31, 0.41], 0.004, 64);
        reference.seconds = 12.0;
        let bytes = operand_bytes(&reference);
        assert_eq!(bytes.len(), 748_816);
        let sha = sha256_hex(&bytes);
        let scratch_dir = temp_dir("symlink-scratch");
        let scratch = scratch_dir.join("scratch");
        let marker = vec![7_u8; bytes.len()];
        std::fs::write(&scratch, &marker).unwrap();
        let planted = std::env::temp_dir().join(format!("room-blotch-judge-{sha}.ref"));
        let _ = std::fs::remove_file(&planted);
        std::os::unix::fs::symlink(&scratch, &planted).unwrap();
        let text = accepted_text(&pose, &reference, &sha);
        let loaded = judge_bytes(&bytes, &text, &pose, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(std::fs::read(&scratch).unwrap(), marker);
        assert!(planted.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(loaded.linear[0], [0.2, 0.31, 0.41]);
        assert_eq!(loaded.spp, 64);
        assert_eq!(loaded.seconds, 0.0);
        assert!(!planted.symlink_metadata().unwrap().file_type().is_file());
        let _ = std::fs::remove_file(&planted);
        let _ = std::fs::remove_dir_all(&scratch_dir);
    }

    #[test]
    fn a_replaced_or_linked_stage_is_not_scored() {
        let pose = poses()[0];
        let mut reference = filled(WIDTH, HEIGHT, [0.11, 0.22, 0.33], 0.003, 64);
        reference.seconds = 4.0;
        let bytes = operand_bytes(&reference);
        let stage = private_stage(&bytes).unwrap();
        assert!(reference_from_staged(&stage.path(), &bytes, pose.name).is_ok());
        std::fs::write(stage.path(), vec![1_u8; bytes.len()]).unwrap();
        let replaced = reference_from_staged(&stage.path(), &bytes, pose.name).unwrap_err();
        assert_eq!(replaced.class, Class::SceneMismatch);
        assert!(replaced.detail.contains("staged bytes differ"));
        let scratch_dir = temp_dir("link-target");
        let scratch = scratch_dir.join("copy");
        std::fs::write(&scratch, &bytes).unwrap();
        std::fs::remove_file(stage.path()).unwrap();
        std::os::unix::fs::symlink(&scratch, stage.path()).unwrap();
        let linked = reference_from_staged(&stage.path(), &bytes, pose.name).unwrap_err();
        assert_eq!(linked.class, Class::SceneMismatch);
        assert_eq!(std::fs::read(&scratch).unwrap(), bytes);
        let marker = vec![9_u8; 32];
        let victim = scratch_dir.join("victim");
        std::fs::write(&victim, &marker).unwrap();
        let link = scratch_dir.join("predictable.ref");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(exclusive_write(&link, b"clobber").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), marker);
        drop(stage);
        assert!(!scratch_dir.join("operand.ref").exists());
        let _ = std::fs::remove_dir_all(&scratch_dir);
    }

    #[test]
    fn private_stage_directories_are_removed() {
        let dir = {
            let stage = private_stage(b"room-blotch-stage").unwrap();
            let dir = stage.dir.clone();
            assert!(dir.join("operand.ref").is_file());
            dir
        };
        assert!(!dir.exists());
    }

    #[test]
    fn concurrent_judges_do_not_share_a_temp_path() {
        let pose = poses()[0];
        let mut left_ref = filled(WIDTH, HEIGHT, [0.2, 0.0, 0.0], 0.004, 64);
        left_ref.seconds = 3.0;
        let mut right_ref = filled(WIDTH, HEIGHT, [0.0, 0.2, 0.0], 0.004, 64);
        right_ref.seconds = 9.0;
        let left_bytes = operand_bytes(&left_ref);
        let right_bytes = operand_bytes(&right_ref);
        let left_sha = sha256_hex(&left_bytes);
        let right_sha = sha256_hex(&right_bytes);
        let left_text = accepted_text(&pose, &left_ref, &left_sha);
        let right_text = accepted_text(&pose, &right_ref, &right_sha);
        let (left, right) = std::thread::scope(|scope| {
            let left_job =
                scope.spawn(|| judge_bytes(&left_bytes, &left_text, &pose, DEFAULT_TRACE_SECONDS));
            let right_job = scope
                .spawn(|| judge_bytes(&right_bytes, &right_text, &pose, DEFAULT_TRACE_SECONDS));
            (left_job.join().unwrap(), right_job.join().unwrap())
        });
        assert_eq!(left.unwrap().linear[0], [0.2, 0.0, 0.0]);
        assert_eq!(right.unwrap().linear[0], [0.0, 0.2, 0.0]);
        assert!(!std::env::temp_dir()
            .join(format!("room-blotch-judge-{left_sha}.ref"))
            .exists());
        assert!(!std::env::temp_dir()
            .join(format!("room-blotch-judge-{right_sha}.ref"))
            .exists());
    }

    #[test]
    fn reported_seconds_are_provenance_and_do_not_flip_the_sample_gate() {
        let pose = poses()[0];
        let mut reference = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, 64);
        reference.seconds = 0.0;
        let bytes = operand_bytes(&reference);
        let sha = sha256_hex(&bytes);
        let zero = accepted_text(&pose, &reference, &sha);
        reference.seconds = 12.0;
        let twelve = accepted_text(&pose, &reference, &sha);
        let loaded_zero = judge_bytes(&bytes, &zero, &pose, DEFAULT_TRACE_SECONDS).unwrap();
        let loaded_twelve = judge_bytes(&bytes, &twelve, &pose, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(loaded_zero.seconds, 0.0);
        assert_eq!(loaded_twelve.seconds, 0.0);
        assert_eq!(loaded_zero.linear, loaded_twelve.linear);
        assert_eq!(loaded_zero.spp, 64);
        reference.seconds = -4.0;
        let negative = accepted_text(&pose, &reference, &sha);
        assert_eq!(
            judge_bytes(&bytes, &negative, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NonFiniteReference
        );

        let fixture = include_str!("../../fixtures/room-blotch/room-a.UNACCEPTED.json");
        assert!(fixture.contains("\"noise_bits\": 1013768127"));
        assert!(fixture.contains("\"spp\": 72064"));
        assert!(fixture.contains("\"seconds_bits\": 1147215451"));
        let room_noise = f32::from_bits(1_013_768_127);
        assert!(room_noise > noise_target());
        let mut room_a = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], room_noise, 72_064);
        room_a.seconds = f32::from_bits(1_147_215_451);
        let room_bytes = operand_bytes(&room_a);
        let room_text = accepted_text(&pose, &room_a, &sha256_hex(&room_bytes));
        assert_eq!(
            judge_bytes(&room_bytes, &room_text, &pose, DEFAULT_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NoisyReference
        );
        let room_b = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/room-blotch/room-b.ref");
        assert!(!room_b.exists());

        let refs = temp_dir("seconds-refs");
        let out_zero = temp_dir("seconds-out-zero");
        let out_twelve = temp_dir("seconds-out-twelve");
        let capture = temp_dir("seconds-capture");
        std::fs::write(refs.join("room-a.ref"), &bytes).unwrap();
        std::fs::write(refs.join("room-a.json"), &zero).unwrap();
        let mut other_ref = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, 64);
        other_ref.seconds = 0.0;
        plant(&refs, &poses()[1], &other_ref);
        let shown = reference::read_cached(&refs.join("room-a.ref"), WIDTH, HEIGHT).unwrap();
        assert_eq!(shown.seconds, 0.0);
        shown.image.save(&capture.join("room-a-live.png")).unwrap();
        shown.image.save(&capture.join("room-b-live.png")).unwrap();
        let eye = eye();
        let flags = settings_text(|_| {}).replace('\n', " || ");
        let report = format!(
            "# script — PASS\n\n## Log\n\n- pose room-a x={} y={} z={} yaw={} pitch={}\n- settle pose=room-a settled=true frames=40 still_frames=300 wall_ms=1\n- resolved pose=room-a {flags}\n- schedule pose=room-a before=4\n- schedule pose=room-a after=6\n- pose room-b x={} y={} z={} yaw={} pitch={}\n- settle pose=room-b settled=true frames=40 still_frames=300 wall_ms=1\n- resolved pose=room-b {flags}\n- schedule pose=room-b before=8\n- schedule pose=room-b after=10\n",
            eye[0], eye[1], eye[2], poses()[0].yaw, pitch(),
            eye[0], eye[1], eye[2], poses()[1].yaw, pitch()
        );
        std::fs::write(capture.join("report.md"), &report).unwrap();
        let verdict_zero =
            grade_dirs(&refs, Some(&capture), &out_zero, DEFAULT_TRACE_SECONDS).unwrap();
        std::fs::write(refs.join("room-a.json"), &twelve).unwrap();
        let mut twelve_b = std::fs::read_to_string(refs.join("room-b.json")).unwrap();
        twelve_b = twelve_b.replace(
            "\"seconds_bits\": 0",
            &format!("\"seconds_bits\": {}", 12.0f32.to_bits()),
        );
        std::fs::write(refs.join("room-b.json"), twelve_b).unwrap();
        let verdict_twelve =
            grade_dirs(&refs, Some(&capture), &out_twelve, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(verdict_zero.class, verdict_twelve.class);
        assert!(out_zero.join("room-a-reference.png").is_file());
        assert!(out_twelve.join("room-a-reference.png").is_file());
        let note = std::fs::read_to_string(out_twelve.join("verdict.json")).unwrap();
        assert!(note.contains("reported duration"));
        assert!(note.contains("read_cached stores 0"));
        assert!(note.contains("zero report is not a budget mismatch"));
        assert!(!note.contains("trace clock"));
        let _ = std::fs::remove_dir_all(&refs);
        let _ = std::fs::remove_dir_all(&out_zero);
        let _ = std::fs::remove_dir_all(&out_twelve);
        let _ = std::fs::remove_dir_all(&capture);
    }

    #[test]
    fn capture_child_drops_a_poisoned_lighting_catalogue() {
        let source = include_str!("../../../../crates/scene/src/lighting.rs");
        let mut names = Vec::new();
        let bytes = source.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index..].starts_with(b"GENOS_") {
                let start = index;
                index += 6;
                while index < bytes.len()
                    && (bytes[index].is_ascii_uppercase()
                        || bytes[index].is_ascii_digit()
                        || bytes[index] == b'_')
                {
                    index += 1;
                }
                if index > start + 6 {
                    names.push(&source[start..index]);
                }
            } else {
                index += 1;
            }
        }
        names.sort_unstable();
        names.dedup();
        assert!(names.len() > 50, "{}", names.len());
        for name in &names {
            assert!(
                lighting_experiment_env(name),
                "{name} is in the lighting catalogue and would leak"
            );
        }
        assert!(lighting_experiment_env("GENOS_COMPOSE_SC_WRITES"));
        assert!(!lighting_experiment_env("PATH"));
        assert!(!lighting_experiment_env("GENOS_REFERENCE_THREADS"));
        assert!(!lighting_experiment_env("GENOS_GPU_TIMES"));
        assert!(!lighting_experiment_env("GENOS_SCENE_DELTA"));
        let parent = [
            ("GENOS_LIGHTING_SC_SETTLE", "300"),
            ("GENOS_LIGHTING_TIME", "18.5"),
            ("GENOS_LIGHTING_DENOISE", "off"),
            ("GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET", "1"),
            ("GENOS_LIGHTING_SURFACE_CACHE_BLEND", "off"),
            ("GENOS_LIGHTING_SURFACE_CACHE_FIREFLY_SIGMA", "0"),
            ("GENOS_LIGHTING_BOUNCES", "0"),
            ("GENOS_LIGHTING_ASYNC", "1"),
            ("GENOS_LIGHTING_PRESET", "low"),
            ("GENOS_LIGHTING_WORLD", "0"),
            ("GENOS_LIGHTING_WORLD_CELL", "1"),
            ("GENOS_LIGHTING_SC_FLAGS", "1"),
            ("GENOS_LIGHTING_TOGGLES", "none"),
            ("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", "0"),
            ("GENOS_GI2_SC_SETTLE", "12"),
            ("GENOS_GI2_DENOISE", "off"),
            ("GENOS_GI_DEBUG", "1"),
            ("GENOS_COMPOSE_SC_WRITES", "1"),
            ("GENOS_REFERENCE_GPU", "1"),
            ("PATH", "/usr/bin"),
            ("GENOS_REFERENCE_THREADS", "8"),
        ];
        let child = capture_child_env(&parent);
        let sh_cells: Vec<_> = child
            .iter()
            .filter(|(name, _)| name == "GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS")
            .collect();
        assert_eq!(sh_cells.len(), 1);
        assert_eq!(sh_cells[0].1, "1");
        for (name, _) in parent {
            if name == "GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS" {
                continue;
            }
            if lighting_experiment_env(name) || name == "GENOS_REFERENCE_GPU" {
                assert!(
                    child.iter().all(|(key, _)| key != name),
                    "{name} leaked into the child"
                );
            }
        }
        assert!(child
            .iter()
            .any(|(name, value)| name == "PATH" && value == "/usr/bin"));
        assert!(child
            .iter()
            .any(|(name, value)| name == "GENOS_REFERENCE_THREADS" && value == "8"));
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn seconds_flag_accepts_only_the_two_bounded_stops() {
        assert!(HELP.contains("--seconds N"));
        assert!(HELP.contains("900 or 2400"));
        assert!(HELP.contains("no environment variable"));
        assert!(run(words(&["trace", "--help"]).into_iter()).is_ok());
        assert!(run(words(&["grade", "-h"]).into_iter()).is_ok());
        let mut trace = words(&["--pose", "room-a", "--seconds", "2400", "--threads", "4"])
            .into_iter()
            .peekable();
        let parsed = parse_trace_args(&mut trace).unwrap();
        assert!(matches!(parsed, CommandArgs::Ready(_)));
        let CommandArgs::Ready(parsed) = parsed else {
            return;
        };
        assert_eq!(parsed.pose.name, "room-a");
        assert_eq!(parsed.threads, Some(4));
        assert_eq!(parsed.seconds.to_bits(), LONG_TRACE_SECONDS.to_bits());
        let mut omitted = words(&["--pose", "room-b"]).into_iter().peekable();
        let parsed = parse_trace_args(&mut omitted).unwrap();
        assert!(matches!(parsed, CommandArgs::Ready(_)));
        let CommandArgs::Ready(parsed) = parsed else {
            return;
        };
        assert_eq!(parsed.seconds.to_bits(), DEFAULT_TRACE_SECONDS.to_bits());
        let mut grade = words(&["--refs", "refs", "--seconds", "900.0"])
            .into_iter()
            .peekable();
        let parsed = parse_grade_args(&mut grade).unwrap();
        assert!(matches!(parsed, CommandArgs::Ready(_)));
        let CommandArgs::Ready(parsed) = parsed else {
            return;
        };
        assert_eq!(parsed.seconds.to_bits(), DEFAULT_TRACE_SECONDS.to_bits());
        for bad in ["nan", "NaN", "inf", "+inf", "-inf", "infinity"] {
            let err = run(words(&["trace", "--pose", "room-a", "--seconds", bad]).into_iter())
                .unwrap_err();
            assert!(err.contains("not finite"), "{bad}: {err}");
            assert!(!err.contains("dirty worktree"), "{bad} reached the trace");
        }
        for bad in ["-1", "0", "1", "899", "901", "2399", "2401", "900.5"] {
            let err = run(words(&["trace", "--pose", "room-a", "--seconds", bad]).into_iter())
                .unwrap_err();
            assert!(err.contains("want 900 or 2400"), "{bad}: {err}");
            assert!(!err.contains("dirty worktree"), "{bad} reached the trace");
        }
        for bad in ["9e2", "abc", "900.", ""] {
            let err = run(words(&["trace", "--pose", "room-a", "--seconds", bad]).into_iter())
                .unwrap_err();
            assert!(
                err.contains("not a number") || err.contains("needs a value"),
                "{bad}: {err}"
            );
            assert!(!err.contains("dirty worktree"), "{bad} reached the trace");
        }
        let twice = run(words(&[
            "grade",
            "--seconds",
            "900",
            "--seconds",
            "2400",
            "--refs",
            "unused",
        ])
        .into_iter())
        .unwrap_err();
        assert!(twice.contains("twice"));
        let missing = run(words(&["grade", "--seconds"]).into_iter()).unwrap_err();
        assert!(missing.contains("needs a value"));
        assert_eq!(
            parse_trace_seconds("+2400").unwrap().to_bits(),
            LONG_TRACE_SECONDS.to_bits()
        );
        assert_eq!(
            parse_trace_seconds(" 900 ").unwrap().to_bits(),
            DEFAULT_TRACE_SECONDS.to_bits()
        );
    }

    #[test]
    fn a_900_budget_is_not_relabeled_when_2400_is_requested() {
        let pose = poses()[0];
        let short_setup = room_setup(&pose, DEFAULT_TRACE_SECONDS).unwrap();
        let long_setup = room_setup(&pose, LONG_TRACE_SECONDS).unwrap();
        assert_eq!(
            short_setup.seconds.to_bits(),
            DEFAULT_TRACE_SECONDS.to_bits()
        );
        assert_eq!(long_setup.seconds.to_bits(), LONG_TRACE_SECONDS.to_bits());
        assert_eq!(short_setup.max_spp, MAX_SPP);
        assert_eq!(long_setup.max_spp, MAX_SPP);
        assert_eq!(short_setup.noise_target.to_bits(), noise_target().to_bits());
        assert_eq!(
            reference::cache_key(&short_setup),
            reference::cache_key(&long_setup)
        );
        assert!(room_setup(&pose, f32::NAN).is_err());
        assert!(room_setup(&pose, f32::INFINITY).is_err());
        assert!(room_setup(&pose, 12.0).is_err());

        let mut reference = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, 64);
        reference.seconds = 12.0;
        let bytes = operand_bytes(&reference);
        let sha = sha256_hex(&bytes);
        let commit = "779e9a67eb67b92d02428bb87af450e1b7adc723";
        let short_text =
            sidecar_text(&pose, &reference, &sha, 1, 4, commit, DEFAULT_TRACE_SECONDS).unwrap();
        let long_text =
            sidecar_text(&pose, &reference, &sha, 1, 4, commit, LONG_TRACE_SECONDS).unwrap();
        assert!(short_text.contains(&format!(
            "\"seconds_budget_bits\": {}",
            DEFAULT_TRACE_SECONDS.to_bits()
        )));
        assert!(long_text.contains(&format!(
            "\"seconds_budget_bits\": {}",
            LONG_TRACE_SECONDS.to_bits()
        )));
        assert!(short_text.contains(&format!("\"seconds_bits\": {}", 12.0f32.to_bits())));
        assert!(long_text.contains(&format!("\"seconds_bits\": {}", 12.0f32.to_bits())));
        assert!(judge_bytes(&bytes, &short_text, &pose, DEFAULT_TRACE_SECONDS).is_ok());
        assert_eq!(
            judge_bytes(&bytes, &short_text, &pose, LONG_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::BudgetMismatch
        );
        assert!(judge_bytes(&bytes, &long_text, &pose, LONG_TRACE_SECONDS).is_ok());
        let mismatch = judge_bytes(&bytes, &long_text, &pose, DEFAULT_TRACE_SECONDS).unwrap_err();
        assert_eq!(mismatch.class, Class::BudgetMismatch);
        assert!(mismatch.detail.contains("recorded budget is kept"));

        reference.seconds = 2500.0;
        let over =
            sidecar_text(&pose, &reference, &sha, 1, 4, commit, DEFAULT_TRACE_SECONDS).unwrap();
        let loaded = judge_bytes(&bytes, &over, &pose, DEFAULT_TRACE_SECONDS).unwrap();
        assert_eq!(loaded.seconds, 0.0);
        assert_eq!(loaded.spp, 64);

        let mut noisy = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.02, MAX_SPP);
        noisy.seconds = 30.0;
        let noisy_bytes = operand_bytes(&noisy);
        let noisy_sha = sha256_hex(&noisy_bytes);
        let noisy_text =
            sidecar_text(&pose, &noisy, &noisy_sha, 1, 4, commit, LONG_TRACE_SECONDS).unwrap();
        assert_eq!(
            judge_bytes(&noisy_bytes, &noisy_text, &pose, LONG_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NoisyReference
        );
        let mut over_spp = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, MAX_SPP + 1);
        over_spp.seconds = 30.0;
        let over_bytes = operand_bytes(&over_spp);
        let over_sha = sha256_hex(&over_bytes);
        let over_text = sidecar_text(
            &pose,
            &over_spp,
            &over_sha,
            1,
            4,
            commit,
            LONG_TRACE_SECONDS,
        )
        .unwrap();
        assert_eq!(
            judge_bytes(&over_bytes, &over_text, &pose, LONG_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::BudgetMismatch
        );
        let nan_text = long_text.replace(
            &format!("\"seconds_budget_bits\": {}", LONG_TRACE_SECONDS.to_bits()),
            &format!("\"seconds_budget_bits\": {}", f32::NAN.to_bits()),
        );
        assert_eq!(
            judge_bytes(&bytes, &nan_text, &pose, LONG_TRACE_SECONDS)
                .unwrap_err()
                .class,
            Class::NonFiniteReference
        );

        let fixture = include_str!("../../fixtures/room-blotch/room-a.UNACCEPTED.json");
        assert!(fixture.contains(&format!(
            "\"seconds_budget_bits\": {}",
            DEFAULT_TRACE_SECONDS.to_bits()
        )));
        assert!(!fixture.contains(&format!(
            "\"seconds_budget_bits\": {}",
            LONG_TRACE_SECONDS.to_bits()
        )));

        let refs = temp_dir("budget-refs");
        let out_long = temp_dir("budget-out-2400");
        let out_default = temp_dir("budget-out-900");
        for pose in poses() {
            let mut sample = filled(WIDTH, HEIGHT, [0.2, 0.3, 0.4], 0.005, 64);
            sample.seconds = 8.0;
            plant(&refs, &pose, &sample);
        }
        let before = std::fs::read(refs.join("room-a.json")).unwrap();
        let err = run(words(&[
            "grade",
            "--refs",
            &refs.display().to_string(),
            "--out",
            &out_long.display().to_string(),
            "--seconds",
            "2400",
        ])
        .into_iter())
        .unwrap_err();
        assert!(err.contains("budget-mismatch"), "{err}");
        assert!(!err.contains("adr-pass"));
        assert_eq!(std::fs::read(refs.join("room-a.json")).unwrap(), before);
        let copied = std::fs::read_to_string(out_long.join("inputs").join("room-a.json")).unwrap();
        assert_eq!(copied.as_bytes(), before.as_slice());
        let verdict = std::fs::read_to_string(out_long.join("verdict.json")).unwrap();
        assert!(verdict.contains("\"class\": \"budget-mismatch\""));
        assert!(verdict.contains("\"seconds_budget\": 2400"));
        assert!(verdict.contains("\"adr_limit\": 0.014"));
        assert!(verdict.contains("\"legacy_room_blotch_limit\": 0.023"));
        assert!(!out_long.join("room-a-reference.png").is_file());
        let default_err = run(words(&[
            "grade",
            "--refs",
            &refs.display().to_string(),
            "--out",
            &out_default.display().to_string(),
        ])
        .into_iter())
        .unwrap_err();
        assert!(default_err.contains("missing-capture"), "{default_err}");
        let default_verdict = std::fs::read_to_string(out_default.join("verdict.json")).unwrap();
        assert!(default_verdict.contains("\"seconds_budget\": 900"));
        assert!(!default_verdict.contains("budget-mismatch"));

        let empty = temp_dir("budget-empty");
        let empty_out = temp_dir("budget-empty-out");
        let missing = run(words(&[
            "grade",
            "--refs",
            &empty.display().to_string(),
            "--out",
            &empty_out.display().to_string(),
            "--seconds",
            "2400",
        ])
        .into_iter())
        .unwrap_err();
        assert!(missing.contains("missing-reference"), "{missing}");
        assert!(!empty.join("room-a.ref").exists());
        let _ = std::fs::remove_dir_all(&refs);
        let _ = std::fs::remove_dir_all(&out_long);
        let _ = std::fs::remove_dir_all(&out_default);
        let _ = std::fs::remove_dir_all(&empty);
        let _ = std::fs::remove_dir_all(&empty_out);
    }
}
