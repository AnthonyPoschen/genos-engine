//! Fixed reservoirs through the product spatial pipelines.
//!
//! `neighbors` is what the frame binds: 0–4 is `restir-spatial`, above 4 is
//! `restir-spatial-wide`. Header word 13 is what the shader merges. A scalar
//! bind with word 13 above 4 leaves the input. A wide bind with word 13 at 8
//! matches `balance_list`.

use genos_render::Renderer;

const EMPTY: u32 = u32::MAX;
const GRID: u32 = 8;
const CANDIDATES: u32 = 4;
const VIEW_Z: f32 = 3.0;
const LIFT: f32 = 0.02;
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    fn seeded(pixel: u32, frame: u32, lo: u32, hi: u32) -> Self {
        let mut seed = u64::from(lo) | (u64::from(hi) << 32);
        seed ^= u64::from(pixel).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        seed ^= u64::from(frame) << 17;
        seed ^= 2u64.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let mut rng = Self {
            state: seed,
            inc: (seed << 1) | 1,
        };
        let _ = rng.next_u32();
        rng
    }

    fn step(&mut self) -> u64 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        old
    }

    fn next_u32(&mut self) -> u32 {
        let old = self.step();
        let mixed = ((old >> 18) ^ old) >> 27;
        let bits = u32::try_from(mixed & 0xffff_ffff).unwrap_or(0);
        let rot = u32::try_from(old >> 59).unwrap_or(0);
        bits.rotate_right(rot)
    }

    fn next_f32(&mut self) -> f32 {
        let bits = (self.next_u32() >> 8) & 0xffff;
        f32::from(u16::try_from(bits).unwrap_or(0)) * (1.0 / 65536.0)
    }

    fn uniform(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let limit = u32::MAX - (u32::MAX % n);
        loop {
            let draw = self.next_u32();
            if draw < limit {
                return draw % n;
            }
        }
    }
}

struct Lamp {
    pos: [f32; 3],
    range: f32,
    color: [f32; 3],
}

struct World {
    lamps: Vec<Lamp>,
    eye: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    tan_half: f32,
    aspect: f32,
    normal_bits: u32,
}

fn uf(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(0))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len == 0.0 {
        a
    } else {
        scale(a, 1.0 / len)
    }
}

fn m_as_f(value: u32) -> f32 {
    let hi = uf(value >> 16);
    let lo = uf(value & 0xffff);
    hi * 65536.0 + lo
}

fn put_f(words: &mut [u32], index: usize, value: f32) {
    if let Some(slot) = words.get_mut(index) {
        *slot = value.to_bits();
    }
}

fn eye_dir(world: &World, x: u32, y: u32) -> [f32; 3] {
    let ndc_x = (uf(x) + 0.5) / uf(GRID) * 2.0 - 1.0;
    let ndc_y = 1.0 - (uf(y) + 0.5) / uf(GRID) * 2.0;
    norm(add(
        world.forward,
        add(
            scale(world.right, ndc_x * world.tan_half * world.aspect),
            scale(world.up, ndc_y * world.tan_half),
        ),
    ))
}

fn surface(world: &World, pixel: u32) -> [f32; 3] {
    let x = pixel % GRID;
    let y = pixel / GRID;
    let dir = eye_dir(world, x, y);
    let along = dot(dir, world.forward);
    let dist = if along == 0.0 { 0.0 } else { VIEW_Z / along };
    add(world.eye, scale(dir, dist))
}

fn lamp_light(lamp: &Lamp, origin: [f32; 3], normal: [f32; 3]) -> f32 {
    let toward = [
        lamp.pos[0] - origin[0],
        lamp.pos[1] - origin[1],
        lamp.pos[2] - origin[2],
    ];
    if lamp.range > 0.0 && dot(toward, toward) > lamp.range * lamp.range {
        return 0.0;
    }
    let dist = length(toward).max(1.0e-4);
    let unit = scale(toward, 1.0 / dist);
    let nd = dot(unit, normal);
    if nd <= 0.0 {
        return 0.0;
    }
    let dist2 = dist * dist;
    let falloff = 72.0 / dist2.max(0.1 * 0.1);
    let rgb = scale(lamp.color, nd * falloff);
    dot(rgb, LUMA)
}

fn phat(world: &World, pixel: u32, lamp: u32) -> f32 {
    let Some(item) = world.lamps.get(usize::try_from(lamp).unwrap_or(usize::MAX)) else {
        return 0.0;
    };
    let pos = surface(world, pixel);
    let origin = add(pos, scale([0.0, 1.0, 0.0], LIFT));
    lamp_light(item, origin, [0.0, 1.0, 0.0])
}

fn window_cell(i: u32, x0: u32, y0: u32, span_x: u32, centre: u32) -> u32 {
    let cell = if i >= centre { i + 1 } else { i };
    (y0 + cell / span_x) * GRID + (x0 + cell % span_x)
}

fn pick(pixel: u32, take: u32, reach: u32, rng: &mut Rng) -> Vec<u32> {
    let x = pixel % GRID;
    let y = pixel / GRID;
    let x0 = x.saturating_sub(reach);
    let y0 = y.saturating_sub(reach);
    let x1 = (x + reach).min(GRID - 1);
    let y1 = (y + reach).min(GRID - 1);
    let span_x = x1 - x0 + 1;
    let span_y = y1 - y0 + 1;
    let cells = span_x * span_y;
    if cells <= 1 || take == 0 || reach == 0 {
        return Vec::new();
    }
    let pool_n = cells - 1;
    let centre = (y - y0) * span_x + (x - x0);
    let take = take.min(pool_n).min(16);
    let mut sw_i = [0u32; 32];
    let mut sw_v = [0u32; 32];
    let mut nswap = 0u32;
    let mut picked = Vec::new();
    for slot in 0..take {
        let pick = slot + rng.uniform(pool_n - slot);
        let mut a = 0u32;
        let mut b = 0u32;
        let mut a_hit = false;
        let mut b_hit = false;
        for entry in 0..nswap {
            let at = usize::try_from(entry).unwrap_or(0);
            if sw_i[at] == slot {
                a = sw_v[at];
                a_hit = true;
            }
            if sw_i[at] == pick {
                b = sw_v[at];
                b_hit = true;
            }
        }
        if !a_hit {
            a = window_cell(slot, x0, y0, span_x, centre);
        }
        if !b_hit {
            b = window_cell(pick, x0, y0, span_x, centre);
        }
        let mut wrote_slot = false;
        let mut wrote_pick = false;
        for entry in 0..nswap {
            let at = usize::try_from(entry).unwrap_or(0);
            if sw_i[at] == slot {
                sw_v[at] = b;
                wrote_slot = true;
            }
            if sw_i[at] == pick {
                sw_v[at] = a;
                wrote_pick = true;
            }
        }
        if !wrote_slot {
            let at = usize::try_from(nswap).unwrap_or(0);
            sw_i[at] = slot;
            sw_v[at] = b;
            nswap += 1;
        }
        if !wrote_pick && pick != slot {
            let at = usize::try_from(nswap).unwrap_or(0);
            sw_i[at] = pick;
            sw_v[at] = a;
            nswap += 1;
        }
        picked.push(b);
    }
    picked
}

struct Sample {
    lamp: u32,
    w: f32,
    m: u32,
    target: f32,
}

fn load_sample(words: &[u32], at: usize) -> Sample {
    let lamp = words.get(at).copied().unwrap_or(EMPTY);
    let w = f32::from_bits(words.get(at + 1).copied().unwrap_or(0));
    let m = words.get(at + 2).copied().unwrap_or(0);
    let target = f32::from_bits(words.get(at + 3).copied().unwrap_or(0));
    if m == 0 || lamp == EMPTY || target <= 0.0 || w <= 0.0 {
        Sample {
            lamp: EMPTY,
            w: 0.0,
            m: 0,
            target: 0.0,
        }
    } else {
        Sample { lamp, w, m, target }
    }
}

fn reservoir_w(sample: &Sample) -> f32 {
    if sample.m == 0 || sample.lamp == EMPTY || sample.target <= 0.0 {
        0.0
    } else {
        sample.w / (m_as_f(sample.m) * sample.target)
    }
}

struct Slot {
    recv: u32,
    sample: Sample,
    tq: f32,
}

fn balance(world: &World, slots: &[Slot], unit: f32) -> Sample {
    if slots.len() == 2 && slots[0].sample.m == 0 {
        let other = &slots[1].sample;
        if other.m == 0 || other.lamp == EMPTY {
            return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
        }
        return Sample { lamp: other.lamp, w: other.w, m: other.m, target: other.target };
    }
    let mut mass = [0.0f32; 17];
    let mut area = 0.0f32;
    for (index, slot) in slots.iter().enumerate() {
        let weight = if slot.sample.m == 0 || slot.sample.lamp == EMPTY || slot.tq <= 0.0 {
            0.0
        } else {
            m_as_f(slot.sample.m) * reservoir_w(&slot.sample) * slot.tq
        };
        mass[index] = weight;
        area += weight;
    }
    if area <= 0.0 {
        return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
    }
    let selected = if slots.len() == 2 {
        if mass[1] > 0.0 && unit < mass[1] / area {
            Some(1)
        } else if mass[0] > 0.0 {
            Some(0)
        } else if mass[1] > 0.0 {
            Some(1)
        } else {
            None
        }
    } else {
        let mark = unit * area;
        let mut acc = 0.0f32;
        let mut last = None;
        let mut chosen = None;
        for (index, weight) in mass.iter().enumerate().take(slots.len()) {
            if *weight <= 0.0 {
                continue;
            }
            last = Some(index);
            acc += *weight;
            if mark < acc {
                chosen = Some(index);
                break;
            }
        }
        chosen.or(last)
    };
    let Some(sel) = selected else {
        return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
    };
    let slot = &slots[sel];
    let tq = slot.tq.max(0.0);
    let t_src = phat(world, slot.recv, slot.sample.lamp).max(0.0);
    if tq <= 0.0 || t_src <= 0.0 {
        return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
    }
    let mut denom = 0.0f32;
    let mut count = 0u32;
    for item in slots {
        if item.sample.m == 0 {
            continue;
        }
        count = count.wrapping_add(item.sample.m);
        denom += m_as_f(item.sample.m) * phat(world, item.recv, slot.sample.lamp).max(0.0);
    }
    if denom <= 0.0 || count == 0 {
        return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
    }
    let w_new = area * t_src / (tq * denom);
    Sample {
        lamp: slot.sample.lamp,
        w: w_new * m_as_f(count) * tq,
        m: count,
        target: tq,
    }
}

struct Built {
    world: World,
    scene: Vec<u32>,
    gbuf: Vec<u32>,
    restir: Vec<u32>,
    hist: usize,
}

fn build(header_neighbors: u32) -> Built {
    let world = World {
        lamps: vec![
            Lamp { pos: [0.0, 4.0, 0.0], range: 40.0, color: [1.0, 0.85, 0.45] },
            Lamp { pos: [1.2, 3.6, -0.4], range: 40.0, color: [0.35, 0.55, 1.0] },
        ],
        eye: [0.0, 5.0, 0.0],
        forward: [0.0, -1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        tan_half: 0.5,
        aspect: 1.0,
        normal_bits: 32_767u32 << 16,
    };
    let pixels = usize::try_from(GRID * GRID).unwrap_or(0);
    let lamp_base = 64usize;
    let hist0 = lamp_base + world.lamps.len() * 8;
    let hist1 = hist0 + pixels * 6;
    let scratch = hist1 + pixels * 6;
    let total = scratch + pixels * 4;
    let mut restir = vec![0u32; total];
    restir[0] = u32::try_from(world.lamps.len()).unwrap_or(0);
    restir[1] = u32::try_from(lamp_base).unwrap_or(0);
    restir[7] = GRID;
    restir[8] = GRID;
    restir[9] = CANDIDATES;
    restir[10] = 2;
    restir[13] = header_neighbors;
    restir[14] = 2;
    restir[16] = 3;
    restir[17] = 0xC5A1_7E11;
    restir[18] = 0x0051_5EED;
    restir[19] = 6;
    restir[46] = 1;
    restir[48] = GRID;
    restir[49] = GRID;
    if let Some(slot) = restir.get_mut(4) {
        *slot = u32::try_from(hist0).unwrap_or(0);
    }
    if let Some(slot) = restir.get_mut(5) {
        *slot = u32::try_from(hist1).unwrap_or(0);
    }
    if let Some(slot) = restir.get_mut(6) {
        *slot = u32::try_from(scratch).unwrap_or(0);
    }
    for (index, lamp) in world.lamps.iter().enumerate() {
        let at = lamp_base + index * 8;
        put_f(&mut restir, at, lamp.pos[0]);
        put_f(&mut restir, at + 1, lamp.pos[1]);
        put_f(&mut restir, at + 2, lamp.pos[2]);
        put_f(&mut restir, at + 3, lamp.range);
        put_f(&mut restir, at + 4, lamp.color[0]);
        put_f(&mut restir, at + 5, lamp.color[1]);
        put_f(&mut restir, at + 6, lamp.color[2]);
        if let Some(slot) = restir.get_mut(at + 7) {
            *slot = u32::try_from(index).unwrap_or(0);
        }
    }
    for pixel in 0..pixels {
        let at = hist1 + pixel * 6;
        let sample = planted(u32::try_from(pixel).unwrap_or(0));
        if let Some(slot) = restir.get_mut(at) {
            *slot = sample.lamp;
        }
        put_f(&mut restir, at + 1, sample.w);
        if let Some(slot) = restir.get_mut(at + 2) {
            *slot = sample.m;
        }
        put_f(&mut restir, at + 3, sample.target);
        put_f(&mut restir, at + 4, VIEW_Z);
        if let Some(slot) = restir.get_mut(at + 5) {
            *slot = world.normal_bits;
        }
    }
    let mut scene = vec![0u32; 64];
    put_f(&mut scene, 4, world.eye[0]);
    put_f(&mut scene, 5, world.eye[1]);
    put_f(&mut scene, 6, world.eye[2]);
    put_f(&mut scene, 24, world.right[0]);
    put_f(&mut scene, 25, world.right[1]);
    put_f(&mut scene, 26, world.right[2]);
    put_f(&mut scene, 27, world.tan_half);
    put_f(&mut scene, 28, world.up[0]);
    put_f(&mut scene, 29, world.up[1]);
    put_f(&mut scene, 30, world.up[2]);
    put_f(&mut scene, 32, world.forward[0]);
    put_f(&mut scene, 33, world.forward[1]);
    put_f(&mut scene, 34, world.forward[2]);
    put_f(&mut scene, 35, world.aspect);
    let mut gbuf = vec![0u32; 4 + pixels * 5];
    for pixel in 0..pixels {
        let dir = eye_dir(&world, u32::try_from(pixel % 8).unwrap_or(0), u32::try_from(pixel / 8).unwrap_or(0));
        let along = dot(dir, world.forward);
        let dist = if along == 0.0 { 0.0 } else { VIEW_Z / along };
        let at = 4 + pixel * 5;
        put_f(&mut gbuf, at, dist);
        if let Some(slot) = gbuf.get_mut(at + 1) {
            *slot = world.normal_bits;
        }
    }
    Built { world, scene, gbuf, restir, hist: hist1 }
}

fn planted(pixel: u32) -> Sample {
    if pixel.is_multiple_of(7) {
        return Sample { lamp: EMPTY, w: 0.0, m: 0, target: 0.0 };
    }
    Sample {
        lamp: pixel % 2,
        w: 0.2 + uf(pixel % 4) * 0.05,
        m: 4 + pixel % 5,
        target: 0.35 + uf(pixel % 3) * 0.1,
    }
}

fn reference(built: &Built, header: u32, merge: bool) -> Vec<Sample> {
    let pixels = usize::try_from(GRID * GRID).unwrap_or(0);
    let mut out = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let px = u32::try_from(pixel).unwrap_or(0);
        let loaded = load_sample(&built.restir, built.hist + pixel * 6);
        if !merge {
            out.push(loaded);
            continue;
        }
        let mut rng = Rng::seeded(px, built.restir[16], built.restir[17], built.restir[18]);
        let picked = pick(px, header.min(16), 2, &mut rng);
        let unit = rng.next_f32();
        let mut slots = vec![canon(&built.world, px, px, &loaded)];
        for source in picked {
            let sample = load_sample(
                &built.restir,
                built.hist + usize::try_from(source).unwrap_or(0) * 6,
            );
            slots.push(canon(&built.world, px, source, &sample));
        }
        out.push(balance(&built.world, &slots, unit));
    }
    out
}

/// `centre` is the pixel being shaded. The shaders store `in_tq` as p-hat of
/// the slot's lamp there, and recompute p-hat at `recv` only for the denominator.
fn canon(world: &World, centre: u32, recv: u32, sample: &Sample) -> Slot {
    let empty = sample.m == 0 || sample.lamp == EMPTY;
    let m = if empty { CANDIDATES } else { sample.m };
    let lamp = if empty { EMPTY } else { sample.lamp };
    Slot {
        recv,
        sample: Sample {
            lamp,
            w: sample.w,
            m,
            target: sample.target,
        },
        tq: phat(world, centre, lamp),
    }
}

fn close(a: f32, b: f32) -> bool {
    if a.to_bits() == b.to_bits() {
        return true;
    }
    let scale = a.abs().max(b.abs()).max(1.0e-6);
    (a - b).abs() <= scale * 1.0e-5
}

fn check_case(renderer: &mut Renderer, host: u32, header: u32) {
    let built = build(header);
    let merge = header > 0 && host == header;
    let expect = reference(&built, header, merge);
    let got = renderer
        .restir_merge_oracle(host, &built.scene, &built.gbuf, &built.restir)
        .expect("spatial oracle dispatch");
    let scratch = usize::try_from(built.restir.get(6).copied().unwrap_or(0)).unwrap_or(0);
    for (pixel, want) in expect.iter().enumerate() {
        let at = scratch + pixel * 4;
        let lamp = got.get(at).copied().unwrap_or(EMPTY);
        let w = f32::from_bits(got.get(at + 1).copied().unwrap_or(0));
        let m = got.get(at + 2).copied().unwrap_or(0);
        assert_eq!(lamp, want.lamp, "pixel {pixel} lamp, host {host} header {header}");
        assert_eq!(m, want.m, "pixel {pixel} M, host {host} header {header}");
        assert!(
            close(w, want.w),
            "pixel {pixel} W {w} vs {} host {host} header {header}",
            want.w
        );
    }
}

#[test]
fn fixed_reservoirs_match_balance_list_on_both_spatial_pipelines() {
    let (_gpu, mut owner) = super::open();
    let renderer = &mut *owner.device;
    check_case(renderer, 0, 0);
    check_case(renderer, 1, 1);
    check_case(renderer, 4, 4);
    let wide = build(8);
    let merged = reference(&wide, 8, true);
    let kept = reference(&wide, 8, false);
    assert!(
        merged.iter().zip(&kept).any(|(a, b)| a.lamp != b.lamp || a.m != b.m || !close(a.w, b.w)),
        "eight neighbours must change a planted reservoir"
    );
    check_case(renderer, 4, 8);
    check_case(renderer, 8, 8);
}
