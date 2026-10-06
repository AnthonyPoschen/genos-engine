//! The lighting panel through the public UI entry, against a real lamp.

use genos_render::illuminate_facing;
use genos_scene::{viewport_uv, Camera, Floor, Light, Scene, Shape, Solid, Vec3};
use genos_ui::{
    apply_lamp, id, lighting_frame, Action, Frame, Paint, PictureMode, Pointer, Shown, State,
};

const VIEW: [f32; 2] = [1280.0, 720.0];

#[test]
fn the_panel_hover_press_and_drag_do_not_capture_the_camera() {
    let camera = Camera::opening();
    let scene = lamp_scene(Vec3::new(0.0, 4.0, 0.0));
    let mut state = State::default();
    let idle = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    let button = shown(&idle, id::X);
    assert_eq!(button.look, button.idle);
    let (x, y) = center(&button);
    let hover = ui(&mut state, &scene, &camera, pointer(x, y, false));
    let hovered = shown(&hover, id::X);
    assert_ne!(hovered.look, hovered.idle, "hover left the idle look");

    let press = ui(&mut state, &scene, &camera, pointer(x, y, true));
    let pressed = shown(&press, id::X);
    assert_ne!(pressed.look, pressed.idle, "press left the idle look");
    assert_ne!(pressed.look, hovered.look, "press matched hover");
    assert!(!press.look_capture, "a slider press captured look");

    let mut state = State::default();
    let idle = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    let title = shown(&idle, id::TITLE);
    let (x, y) = center(&title);
    let held = ui(&mut state, &scene, &camera, pointer(x, y, true));
    assert!(!held.look_capture);
    let moved = ui(
        &mut state,
        &scene,
        &camera,
        pointer(x + 30.0, y - 12.0, true),
    );
    let before = shown(&held, id::PANEL).rect;
    let after = shown(&moved, id::PANEL).rect;
    assert!(
        (after.x - before.x - 30.0).abs() < 0.05,
        "panel x {before:?} -> {after:?}"
    );
    assert!(
        (after.y - before.y + 12.0).abs() < 0.05,
        "panel y {before:?} -> {after:?}"
    );
    let child_before = shown(&held, id::X).rect;
    let child_after = shown(&moved, id::X).rect;
    assert!((child_after.x - child_before.x - 30.0).abs() < 0.05);
    assert!((child_after.y - child_before.y + 12.0).abs() < 0.05);

    let mut state = State::default();
    let outside = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, true));
    assert!(
        outside.look_capture,
        "a press off the panel did not allow look capture"
    );
}

#[test]
fn dim_and_bright_scale_the_lamp() {
    let camera = Camera::opening();
    let mut dimmed = lamp_scene(Vec3::new(0.0, 4.0, 0.0));
    let (before, after) = scale(id::DIM, &mut dimmed, &camera);
    assert!(
        after < before,
        "dim did not lower the lamp: {before} -> {after}"
    );

    let mut bright = lamp_scene(Vec3::new(0.0, 4.0, 0.0));
    let (before, after) = scale(id::BRIGHT, &mut bright, &camera);
    assert!(
        after > before,
        "bright did not raise the lamp: {before} -> {after}"
    );
}

#[test]
fn notched_sliders_set_one_axis_and_wait_for_a_drag() {
    let camera = Camera::opening();
    let start = Vec3::new(0.25, 4.2, -0.4);
    let mut scene = lamp_scene(start);
    let mut state = State::default();
    let idle = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    apply_actions(&mut scene, &idle);
    assert_eq!(
        scene.lights[0].position, start,
        "showing the panel moved the lamp"
    );
    assert!(
        idle.actions
            .iter()
            .all(|action| !matches!(action, Action::SetLamp { .. })),
        "an idle frame wrote the lamp"
    );
    let labels: Vec<&str> = idle
        .shown
        .iter()
        .filter_map(|item| item.text.as_deref())
        .collect();
    for gone in ["X+", "X-", "Y+", "Y-", "Z+", "Z-"] {
        assert!(!labels.contains(&gone), "{gone} is still a control");
    }

    for axis_id in [id::X, id::Y, id::Z] {
        let track = shown(&idle, axis_id).rect;
        let marks = notch_marks(&idle, track);
        assert!(
            marks.len() >= 8,
            "slider {axis_id} painted {} notches",
            marks.len()
        );
        let mut xs: Vec<f32> = marks.iter().map(center_x).collect();
        xs.sort_by(f32::total_cmp);
        assert!(
            xs[0] < track.x + track.w * 0.25,
            "notches do not start across the track: {xs:?}"
        );
        assert!(
            *xs.last().unwrap() > track.x + track.w * 0.75,
            "notches do not finish across the track: {xs:?}"
        );
        let thumb = thumb(&idle, track);
        let thumb_x = center_x(&thumb);
        assert!(
            marks
                .iter()
                .any(|mark| (center_x(mark) - thumb_x).abs() < 1.0),
            "thumb {thumb_x} is off the notches {xs:?}"
        );
    }

    let mut scene = lamp_scene(start);
    let x_values = sweep(id::X, 0, &mut scene, &camera, start);
    assert_one_meter_steps(&x_values);
    assert!(
        x_values[0] <= -8.0,
        "X does not reach the floor: {x_values:?}"
    );
    assert!(
        *x_values.last().unwrap() >= 8.0,
        "X does not reach the floor: {x_values:?}"
    );
    assert_eq!(scene.lights[0].position.y, start.y);
    assert_eq!(scene.lights[0].position.z, start.z);

    let mut scene = lamp_scene(start);
    let z_values = sweep(id::Z, 2, &mut scene, &camera, start);
    assert_one_meter_steps(&z_values);
    assert!(
        z_values[0] <= -8.0,
        "Z does not reach the floor: {z_values:?}"
    );
    assert!(
        *z_values.last().unwrap() >= 8.0,
        "Z does not reach the floor: {z_values:?}"
    );
    assert_eq!(scene.lights[0].position.x, start.x);
    assert_eq!(scene.lights[0].position.y, start.y);

    let mut scene = lamp_scene(start);
    let y_samples = sweep_samples(id::Y, 1, &mut scene, &camera, start);
    let y_values = unique_values(&y_samples);
    assert_one_meter_steps(&y_values);
    assert!(
        y_values.iter().any(|value| *value < 0.0),
        "Y has no underground notch: {y_values:?}"
    );
    assert!(
        y_values.iter().any(|value| (*value - 7.0).abs() < 1.0e-4),
        "Y is missing the scripted height 7: {y_values:?}"
    );
    assert_eq!(scene.lights[0].position.x, start.x);
    assert_eq!(scene.lights[0].position.z, start.z);

    let buried = y_samples
        .iter()
        .find(|sample| sample.value < 0.0)
        .expect("underground sample");
    let mut buried_scene = lamp_scene(start);
    choose_notch(buried.x, buried.y, &mut buried_scene, &camera);
    assert!(
        buried_scene.lights[0].position.y < 0.0,
        "the underground notch left the lamp at {}",
        buried_scene.lights[0].position.y
    );
    assert!(buried_scene.lights[0].position.y <= 0.05);
    assert_eq!(buried_scene.lights[0].position.x, start.x);
    assert_eq!(buried_scene.lights[0].position.z, start.z);

    let height = y_samples
        .iter()
        .find(|sample| (sample.value - 7.0).abs() < 1.0e-4)
        .expect("height 7 sample");
    let mut high = lamp_scene(start);
    choose_notch(height.x, height.y, &mut high, &camera);
    assert!((high.lights[0].position.y - 7.0).abs() < 1.0e-4);
    assert_eq!(high.lights[0].position.x, start.x);
    assert_eq!(high.lights[0].position.z, start.z);
}

#[test]
fn the_sun_tracks_the_lamp_and_a_press_does_not_move_it() {
    let mut scene = lamp_scene(Vec3::new(0.0, 7.0, 0.0));
    scene.solids.push(Solid {
        shape: Shape::Square,
        position: Vec3::new(3.0, 0.0, 3.0),
        size: 1.0,
        height: 1.0,
        color: [0.8, 0.2, 0.1],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    let solid = scene.solids[0].position;
    let camera = aim_at(lamp_of(&scene));
    let mut state = State::default();
    let frame = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    let expected = project(&camera, lamp_of(&scene));
    assert_on_screen(expected);
    let (center, longer, fill) = sun_metrics(&frame);
    assert!(
        (center[0] - expected[0]).abs() < 3.0 && (center[1] - expected[1]).abs() < 3.0,
        "sun {center:?} is off the lamp {expected:?}"
    );
    assert!(
        (10.0..=72.0).contains(&longer),
        "sun span {longer} is outside 10 to 72"
    );
    assert!(fill >= 2.0, "sun fill {fill} is dim");
    let panel = shown(&frame, id::PANEL);
    assert_ne!(
        sun_fill_color(&frame),
        panel.idle.background,
        "the sun used the panel chrome"
    );

    let mut turned = camera.clone();
    turned.yaw += 0.12;
    let moved_camera = ui(&mut state, &scene, &turned, pointer(0.0, 0.0, false));
    let turned_point = project(&turned, lamp_of(&scene));
    assert_on_screen(turned_point);
    let (turned_center, _, _) = sun_metrics(&moved_camera);
    assert!(
        (turned_center[0] - turned_point[0]).abs() < 3.0
            && (turned_center[1] - turned_point[1]).abs() < 3.0,
        "sun did not follow the camera: {turned_center:?} vs {turned_point:?}"
    );
    assert!(
        (turned_center[0] - center[0]).abs() > 1.0 || (turned_center[1] - center[1]).abs() > 1.0,
        "the camera move left the sun in place"
    );

    scene.lights[0].position.y = 5.0;
    let lowered = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    let lowered_point = project(&camera, lamp_of(&scene));
    assert_on_screen(lowered_point);
    let (lowered_center, _, _) = sun_metrics(&lowered);
    assert!(
        (lowered_center[0] - lowered_point[0]).abs() < 3.0
            && (lowered_center[1] - lowered_point[1]).abs() < 3.0,
        "sun did not follow the lamp: {lowered_center:?} vs {lowered_point:?}"
    );

    scene.lights[0].position = Vec3::new(0.0, -2.0, 0.0);
    let underground_camera = aim_at(lamp_of(&scene));
    let underground = ui(
        &mut state,
        &scene,
        &underground_camera,
        pointer(0.0, 0.0, false),
    );
    let under_point = project(&underground_camera, lamp_of(&scene));
    assert_on_screen(under_point);
    let (under_center, under_span, under_fill) = sun_metrics(&underground);
    assert!(
        (under_center[0] - under_point[0]).abs() < 3.0
            && (under_center[1] - under_point[1]).abs() < 3.0,
        "sun left the underground lamp: {under_center:?} vs {under_point:?}"
    );
    assert!((10.0..=72.0).contains(&under_span));
    assert!(under_fill >= 2.0);

    let mut behind = underground_camera.clone();
    behind.yaw += std::f32::consts::PI;
    assert!(
        viewport_uv(&behind, VIEW[0] / VIEW[1], lamp_of(&scene)).is_none(),
        "the lamp stayed in front of the camera"
    );
    let hidden = ui(&mut state, &scene, &behind, pointer(0.0, 0.0, false));
    assert!(
        sun_paints(&hidden).is_empty(),
        "the sun stayed on screen behind the camera"
    );

    let mut state = State::default();
    let before = scene.lights[0].position;
    let press = ui(
        &mut state,
        &scene,
        &underground_camera,
        pointer(under_center[0], under_center[1], true),
    );
    assert!(press.look_capture, "a press on the sun blocked look");
    assert!(
        press.actions.is_empty(),
        "a press on the sun wrote an action"
    );
    let drag = ui(
        &mut state,
        &scene,
        &underground_camera,
        pointer(under_center[0] + 40.0, under_center[1] - 18.0, true),
    );
    assert!(drag.look_capture, "the drag on the sun blocked look");
    apply_actions(&mut scene, &press);
    apply_actions(&mut scene, &drag);
    assert_eq!(scene.lights[0].position, before);
    assert_eq!(scene.solids[0].position, solid);
}

#[test]
fn the_panel_shows_picture_modes_and_a_press_selects_one() {
    let camera = Camera::opening();
    let scene = lamp_scene(Vec3::new(0.0, 4.0, 0.0));
    let mut state = State::default();
    let frame = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
    let texts: Vec<&str> = frame
        .shown
        .iter()
        .filter_map(|item| item.text.as_deref())
        .collect();
    for label in [
        "Light", "X", "Y", "Z", "Dim", "Bright", "off", "FXAA", "SSAA",
    ] {
        assert!(texts.contains(&label), "{label} is missing from {texts:?}");
    }
    assert_eq!(texts.iter().filter(|text| **text == "off").count(), 1);

    for (control, mode, label) in [
        (id::AA_OFF, PictureMode::Off, "off"),
        (id::AA_FXAA, PictureMode::Fxaa, "FXAA"),
        (id::AA_SSAA, PictureMode::Ssaa, "SSAA"),
    ] {
        let mut state = State::default();
        let idle = ui(&mut state, &scene, &camera, pointer(0.0, 0.0, false));
        let button = shown(&idle, control);
        assert_eq!(button.text.as_deref(), Some(label));
        let (x, y) = center(&button);
        let mut state = State::default();
        let press = ui(&mut state, &scene, &camera, pointer(x, y, true));
        assert_eq!(press.actions, vec![Action::SetAntialias(mode)]);
        assert!(!press.look_capture, "a picture-mode press captured look");
    }
}

fn scale(control: u32, scene: &mut Scene, camera: &Camera) -> (f32, f32) {
    let mut state = State::default();
    let idle = ui(&mut state, scene, camera, pointer(0.0, 0.0, false));
    let (x, y) = center(&shown(&idle, control));
    let before = illuminate_facing(scene, 0.0, 1.0, 0.0, [0.0, 1.0, 0.0]);
    let press = ui(&mut state, scene, camera, pointer(x, y, true));
    assert!(!press.look_capture, "a panel press captured look");
    assert!(matches!(
        press.actions.as_slice(),
        [Action::ScaleIntensity(_)]
    ));
    apply_actions(scene, &press);
    let after = illuminate_facing(scene, 0.0, 1.0, 0.0, [0.0, 1.0, 0.0]);
    (before, after)
}

struct Sample {
    x: f32,
    y: f32,
    value: f32,
}

fn sweep(axis_id: u32, axis: usize, scene: &mut Scene, camera: &Camera, start: Vec3) -> Vec<f32> {
    unique_values(&sweep_samples(axis_id, axis, scene, camera, start))
}

fn sweep_samples(
    axis_id: u32,
    axis: usize,
    scene: &mut Scene,
    camera: &Camera,
    start: Vec3,
) -> Vec<Sample> {
    let mut state = State::default();
    let idle = ui(&mut state, scene, camera, pointer(0.0, 0.0, false));
    let track = shown(&idle, axis_id).rect;
    let marks = notch_marks(&idle, track);
    let y = track.y + track.h * 0.5;
    let mut samples = Vec::new();
    let mut x = track.x + 1.0;
    let end = track.x + track.w - 1.0;
    while x <= end {
        let frame = ui(&mut state, scene, camera, pointer(x, y, true));
        assert!(!frame.look_capture, "a slider drag captured look");
        assert!(
            matches!(
                frame.actions.as_slice(),
                [Action::SetLamp { axis: written, .. }] if *written as usize == axis
            ),
            "slider {axis_id} wrote {:?}",
            frame.actions
        );
        apply_actions(scene, &frame);
        let position = scene.lights[0].position;
        assert_eq!(other_axes(position, axis), other_axes(start, axis));
        samples.push(Sample {
            x,
            y,
            value: component(position, axis),
        });
        x += 2.0;
    }
    let values = unique_values(&samples);
    assert!(
        values.len() >= 8,
        "slider {axis_id} snapped to {} positions",
        values.len()
    );
    assert_eq!(
        values.len(),
        marks.len(),
        "slider {axis_id} values {values:?} do not match {} notches",
        marks.len()
    );
    assert!(
        samples.len() > values.len() * 2,
        "the slider did not snap: {} samples, {} values",
        samples.len(),
        values.len()
    );
    let _ = ui(&mut state, scene, camera, pointer(end, y, false));
    samples
}

fn choose_notch(x: f32, y: f32, scene: &mut Scene, camera: &Camera) {
    let mut state = State::default();
    let _ = ui(&mut state, scene, camera, pointer(0.0, 0.0, false));
    let frame = ui(&mut state, scene, camera, pointer(x, y, true));
    assert!(!frame.look_capture);
    apply_actions(scene, &frame);
}

fn assert_one_meter_steps(values: &[f32]) {
    assert!(values.len() >= 2, "a slider needs two notches");
    for pair in values.windows(2) {
        assert!(
            (pair[1] - pair[0] - 1.0).abs() < 1.0e-4,
            "notches {values:?} are not 1 meter apart"
        );
    }
}

fn unique_values(samples: &[Sample]) -> Vec<f32> {
    let mut values: Vec<f32> = samples.iter().map(|sample| sample.value).collect();
    values.sort_by(f32::total_cmp);
    values.dedup();
    values
}

fn component(position: Vec3, axis: usize) -> f32 {
    match axis {
        0 => position.x,
        1 => position.y,
        _ => position.z,
    }
}

fn other_axes(position: Vec3, axis: usize) -> (f32, f32) {
    match axis {
        0 => (position.y, position.z),
        1 => (position.x, position.z),
        _ => (position.x, position.y),
    }
}

fn notch_marks(frame: &Frame, track: genos_ui::Rect) -> Vec<Paint> {
    frame
        .paints
        .iter()
        .copied()
        .filter(|paint| {
            let cx = center_x(paint);
            let cy = paint.y + paint.h * 0.5;
            track.contains(cx, cy)
                && paint.w <= 4.0
                && paint.h >= 4.0
                && paint.h < track.h - 4.0
                && paint.w < track.w * 0.5
        })
        .collect()
}

fn thumb(frame: &Frame, track: genos_ui::Rect) -> Paint {
    frame
        .paints
        .iter()
        .copied()
        .find(|paint| {
            let cx = center_x(paint);
            let cy = paint.y + paint.h * 0.5;
            track.contains(cx, cy)
                && paint.w >= 8.0
                && paint.w < track.w * 0.5
                && paint.h > track.h * 0.5
                && paint.h < track.h - 1.0
        })
        .expect("slider thumb")
}

fn sun_paints(frame: &Frame) -> Vec<Paint> {
    let panel = shown(frame, id::PANEL).rect;
    frame
        .paints
        .iter()
        .copied()
        .filter(|paint| {
            let cx = center_x(paint);
            let cy = paint.y + paint.h * 0.5;
            let sum = paint.color[0] + paint.color[1] + paint.color[2];
            !panel.contains(cx, cy)
                && cx >= 0.0
                && cy >= 0.0
                && cx < VIEW[0]
                && cy < VIEW[1]
                && sum >= 2.0
        })
        .collect()
}

fn sun_metrics(frame: &Frame) -> ([f32; 2], f32, f32) {
    let paints = sun_paints(frame);
    assert!(!paints.is_empty(), "the sun is missing from the view");
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for paint in &paints {
        min_x = min_x.min(paint.x);
        min_y = min_y.min(paint.y);
        max_x = max_x.max(paint.x + paint.w);
        max_y = max_y.max(paint.y + paint.h);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    assert!(
        (width - height).abs() < 2.0,
        "sun bounds {width} by {height} are not a circle"
    );
    let main = paints
        .iter()
        .max_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap())
        .expect("sun paint");
    let fill = main.color[0] + main.color[1] + main.color[2];
    (
        [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5],
        width.max(height),
        fill,
    )
}

fn sun_fill_color(frame: &Frame) -> [f32; 3] {
    let paints = sun_paints(frame);
    paints
        .iter()
        .max_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap())
        .expect("sun paint")
        .color
}

fn aim_at(lamp: [f32; 3]) -> Camera {
    let ahead = 6.0;
    let mut camera = Camera::new(lamp[0], lamp[2] + ahead, 0.0);
    camera.pitch = (lamp[1] - camera.position.y).atan2(ahead);
    camera
}

fn project(camera: &Camera, lamp: [f32; 3]) -> [f32; 2] {
    let uv = viewport_uv(camera, VIEW[0] / VIEW[1], lamp).expect("lamp is in front of the camera");
    [uv[0] * VIEW[0], uv[1] * VIEW[1]]
}

fn assert_on_screen(point: [f32; 2]) {
    assert!(
        point[0] > VIEW[0] * 0.15
            && point[0] < VIEW[0] * 0.85
            && point[1] > VIEW[1] * 0.15
            && point[1] < VIEW[1] * 0.85,
        "the aimed lamp is outside the safe view: {point:?}"
    );
}

fn lamp_of(scene: &Scene) -> [f32; 3] {
    let light = &scene.lights[0];
    [light.position.x, light.position.y, light.position.z]
}

fn ui(state: &mut State, scene: &Scene, camera: &Camera, pointer: Pointer) -> Frame {
    lighting_frame(state, VIEW, camera, Some(lamp_of(scene)), pointer)
}

fn pointer(x: f32, y: f32, down: bool) -> Pointer {
    Pointer { x, y, down }
}

fn shown(frame: &Frame, id: u32) -> Shown {
    frame
        .shown
        .iter()
        .find(|item| item.id == id)
        .expect("panel item")
        .clone()
}

fn center(item: &Shown) -> (f32, f32) {
    (
        item.rect.x + item.rect.w * 0.5,
        item.rect.y + item.rect.h * 0.5,
    )
}

fn center_x(paint: &Paint) -> f32 {
    paint.x + paint.w * 0.5
}

fn apply_actions(scene: &mut Scene, frame: &Frame) {
    for action in &frame.actions {
        apply_lamp(scene, *action);
    }
}

fn lamp_scene(position: Vec3) -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position,
            color: [0.5, 0.5, 0.5],

            direction: Vec3::ZERO,
        }],
    }
}
