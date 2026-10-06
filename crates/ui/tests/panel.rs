//! The lighting panel through the public UI entry, against a real lamp.

use genos_render::illuminate_facing;
use genos_scene::{Camera, Floor, Light, Scene, Vec3};
use genos_ui::{apply_lamp, id, lighting_frame, Action, Frame, Pointer, Shown, State};

const VIEW: [f32; 2] = [1280.0, 720.0];

#[test]
fn the_panel_hover_press_and_drag_do_not_capture_the_camera() {
    let camera = Camera::opening();
    let mut state = State::default();
    let idle = lighting_frame(&mut state, VIEW, &camera, pointer(0.0, 0.0, false));
    let button = shown(&idle, id::X_POS);
    assert_eq!(button.look, button.idle);
    let (x, y) = center(&button);
    let hover = lighting_frame(&mut state, VIEW, &camera, pointer(x, y, false));
    let hovered = shown(&hover, id::X_POS);
    assert_ne!(hovered.look, hovered.idle, "hover left the idle look");

    let press = lighting_frame(&mut state, VIEW, &camera, pointer(x, y, true));
    let pressed = shown(&press, id::X_POS);
    assert_ne!(pressed.look, pressed.idle, "press left the idle look");
    assert_ne!(pressed.look, hovered.look, "press matched hover");
    assert!(!press.look_capture, "a panel press captured look");

    let mut state = State::default();
    let idle = lighting_frame(&mut state, VIEW, &camera, pointer(0.0, 0.0, false));
    let title = shown(&idle, id::TITLE);
    let (x, y) = center(&title);
    let held = lighting_frame(&mut state, VIEW, &camera, pointer(x, y, true));
    assert!(!held.look_capture);
    let moved = lighting_frame(&mut state, VIEW, &camera, pointer(x + 30.0, y - 12.0, true));
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
    let child_before = shown(&held, id::X_POS).rect;
    let child_after = shown(&moved, id::X_POS).rect;
    assert!((child_after.x - child_before.x - 30.0).abs() < 0.05);
    assert!((child_after.y - child_before.y + 12.0).abs() < 0.05);

    let mut state = State::default();
    let outside = lighting_frame(&mut state, VIEW, &camera, pointer(0.0, 0.0, true));
    assert!(
        outside.look_capture,
        "a press off the panel did not allow look capture"
    );
}

#[test]
fn lamp_controls_move_the_light_and_change_illumination() {
    let camera = Camera::opening();
    let mut scene = lamp_scene();
    let mut state = State::default();
    let idle = lighting_frame(&mut state, VIEW, &camera, pointer(0.0, 0.0, false));
    let (x, y) = center(&shown(&idle, id::X_POS));
    let press = lighting_frame(&mut state, VIEW, &camera, pointer(x, y, true));
    let started = scene.lights[0].clone_xy();
    apply_actions(&mut scene, &press);
    assert!(
        scene.lights[0].position.x > started.0,
        "X+ did not move the lamp"
    );
    assert_eq!(scene.lights[0].position.y, started.1);
    assert_eq!(scene.lights[0].position.z, started.2);
    assert!(!press.look_capture);

    let mut state = State::default();
    let idle = lighting_frame(&mut state, VIEW, &camera, pointer(0.0, 0.0, false));
    let (x, y) = center(&shown(&idle, id::BRIGHT));
    let before = illuminate_facing(&scene, 0.0, 1.0, 0.0, [0.0, 1.0, 0.0]);
    let press = lighting_frame(&mut state, VIEW, &camera, pointer(x, y, true));
    apply_actions(&mut scene, &press);
    let after = illuminate_facing(&scene, 0.0, 1.0, 0.0, [0.0, 1.0, 0.0]);
    assert!(
        after > before,
        "brighter control dimmed the lamp: {before} -> {after}"
    );
    assert!(matches!(
        press.actions.as_slice(),
        [Action::ScaleIntensity(_)]
    ));
}

fn pointer(x: f32, y: f32, down: bool) -> Pointer {
    Pointer { x, y, down }
}

fn shown(frame: &Frame, id: u32) -> Shown {
    frame
        .shown
        .iter()
        .copied()
        .find(|item| item.id == id)
        .expect("panel item")
}

fn center(item: &Shown) -> (f32, f32) {
    (
        item.rect.x + item.rect.w * 0.5,
        item.rect.y + item.rect.h * 0.5,
    )
}

fn apply_actions(scene: &mut Scene, frame: &Frame) {
    for action in &frame.actions {
        apply_lamp(scene, *action);
    }
}

fn lamp_scene() -> Scene {
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
            position: Vec3::new(0.0, 4.0, 0.0),
            color: [0.5, 0.5, 0.5],
        }],
    }
}

trait LampPoint {
    fn clone_xy(&self) -> (f32, f32, f32);
}

impl LampPoint for Light {
    fn clone_xy(&self) -> (f32, f32, f32) {
        (self.position.x, self.position.y, self.position.z)
    }
}
