//! The regions a UI frame reports as changed.

use genos_scene::{Camera, Vec3};
use genos_ui::{id, lighting_frame, DamageTracker, Paint, Pointer, Rect, Segment, State, MAX_DAMAGE};

const VIEW: [f32; 2] = [1280.0, 720.0];

fn paint(x: f32, y: f32, w: f32, h: f32) -> Paint {
    Paint {
        x,
        y,
        w,
        h,
        color: [0.5, 0.5, 0.5],
    }
}

fn frame(tracker: &mut DamageTracker, groups: &[(u64, Vec<Paint>)]) -> Vec<Rect> {
    tracker.begin(VIEW);
    for (key, paints) in groups {
        tracker.paints(*key, paints);
    }
    tracker.finish()
}

fn covers(rects: &[Rect], x: f32, y: f32, w: f32, h: f32) -> bool {
    rects
        .iter()
        .any(|r| r.x <= x && r.y <= y && r.x + r.w >= x + w && r.y + r.h >= y + h)
}

fn area(rects: &[Rect]) -> f32 {
    rects.iter().map(|r| r.w * r.h).sum()
}

#[test]
fn the_first_frame_dirties_the_whole_viewport_and_a_same_frame_nothing() {
    let mut tracker = DamageTracker::default();
    let groups = vec![(1, vec![paint(10.0, 10.0, 40.0, 20.0)])];
    let first = frame(&mut tracker, &groups);
    assert_eq!(first.len(), 1);
    assert!(covers(&first, 0.0, 0.0, VIEW[0], VIEW[1]));
    assert!(frame(&mut tracker, &groups).is_empty());
}

#[test]
fn a_moved_group_dirties_its_old_and_new_bounds_only() {
    let mut tracker = DamageTracker::default();
    let still = (1, vec![paint(600.0, 400.0, 50.0, 50.0)]);
    frame(&mut tracker, &[still.clone(), (2, vec![paint(10.0, 10.0, 40.0, 20.0)])]);
    let damage = frame(&mut tracker, &[still, (2, vec![paint(200.0, 10.0, 40.0, 20.0)])]);
    assert!(covers(&damage, 10.0, 10.0, 40.0, 20.0), "old bounds: {damage:?}");
    assert!(covers(&damage, 200.0, 10.0, 40.0, 20.0), "new bounds: {damage:?}");
    assert!(!covers(&damage, 600.0, 400.0, 50.0, 50.0), "the still group: {damage:?}");
    assert!(area(&damage) < 2.0 * 44.0 * 24.0 + 1.0, "far apart, so not one box: {damage:?}");
}

#[test]
fn a_colour_change_in_place_dirties_that_group() {
    let mut tracker = DamageTracker::default();
    frame(&mut tracker, &[(1, vec![paint(10.0, 10.0, 40.0, 20.0)])]);
    let mut changed = paint(10.0, 10.0, 40.0, 20.0);
    changed.color = [1.0, 0.0, 0.0];
    let damage = frame(&mut tracker, &[(1, vec![changed])]);
    assert!(covers(&damage, 10.0, 10.0, 40.0, 20.0));
}

#[test]
fn a_removed_group_dirties_its_last_bounds() {
    let mut tracker = DamageTracker::default();
    frame(
        &mut tracker,
        &[(1, vec![paint(10.0, 10.0, 40.0, 20.0)]), (2, vec![paint(300.0, 300.0, 10.0, 10.0)])],
    );
    let damage = frame(&mut tracker, &[(1, vec![paint(10.0, 10.0, 40.0, 20.0)])]);
    assert!(covers(&damage, 300.0, 300.0, 10.0, 10.0));
    assert!(!covers(&damage, 10.0, 10.0, 40.0, 20.0));
}

#[test]
fn a_viewport_change_dirties_everything() {
    let mut tracker = DamageTracker::default();
    let groups = vec![(1, vec![paint(10.0, 10.0, 40.0, 20.0)])];
    frame(&mut tracker, &groups);
    tracker.begin([800.0, 600.0]);
    tracker.paints(1, &groups[0].1);
    let damage = tracker.finish();
    assert!(covers(&damage, 0.0, 0.0, 800.0, 600.0));
}

#[test]
fn many_scattered_changes_merge_to_the_cap_and_still_cover_each() {
    let mut tracker = DamageTracker::default();
    let spots: Vec<(f32, f32)> = (0..20).map(|i| (40.0 + 60.0 * i as f32, 30.0 + 31.0 * i as f32)).collect();
    let before: Vec<(u64, Vec<Paint>)> = spots.iter().enumerate().map(|(i, _)| (i as u64, Vec::new())).collect();
    frame(&mut tracker, &before);
    let after: Vec<(u64, Vec<Paint>)> =
        spots.iter().enumerate().map(|(i, &(x, y))| (i as u64, vec![paint(x, y, 8.0, 8.0)])).collect();
    let damage = frame(&mut tracker, &after);
    assert!(damage.len() <= MAX_DAMAGE, "{} rectangles", damage.len());
    for &(x, y) in &spots {
        assert!(covers(&damage, x, y, 8.0, 8.0), "spot {x} {y} left out");
    }
}

#[test]
fn lines_dirty_their_width_around_the_ends() {
    let mut tracker = DamageTracker::default();
    tracker.begin(VIEW);
    tracker.finish();
    tracker.begin(VIEW);
    tracker.segments(
        1,
        &[Segment {
            a: [100.0, 100.0],
            b: [200.0, 150.0],
            width: 4.0,
            color: [1.0; 3],
        }],
    );
    let damage = tracker.finish();
    assert!(covers(&damage, 98.0, 98.0, 104.0, 54.0), "{damage:?}");
}

#[test]
fn the_lighting_panel_reports_only_the_hovered_control() {
    let camera = Camera::opening();
    let lamp = Vec3::new(0.0, 4.0, 0.0);
    let lamp = Some([lamp.x, lamp.y, lamp.z]);
    let mut state = State::default();
    let idle = |state: &mut State, x: f32, y: f32| {
        lighting_frame(state, VIEW, &camera, lamp, Pointer { x, y, down: false })
    };
    let first = idle(&mut state, 1270.0, 710.0);
    assert!(covers(&first.damage, 0.0, 0.0, VIEW[0], VIEW[1]), "first frame");
    let same = idle(&mut state, 1270.0, 710.0);
    assert!(same.damage.is_empty(), "nothing changed: {:?}", same.damage);
    let button = same.shown.iter().find(|s| s.id == id::AA_FXAA).expect("FXAA button").clone();
    let hover = idle(&mut state, button.rect.x + 2.0, button.rect.y + 2.0);
    assert!(
        covers(&hover.damage, button.rect.x, button.rect.y, button.rect.w, button.rect.h),
        "hovered button: {:?}",
        hover.damage
    );
    let panel = same.shown.iter().find(|s| s.id == id::PANEL).expect("panel").rect;
    assert!(area(&hover.damage) < 0.5 * panel.w * panel.h, "hover redrew most of the panel: {:?}", hover.damage);
}
