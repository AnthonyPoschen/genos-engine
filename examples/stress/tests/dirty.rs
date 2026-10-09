//! The small stress doorway. A moving box marks its shaft and the probes that
//! can see it. It does not mark every live brick.

use genos_render::{scene_boxes, TierLight, TierState};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};

fn tier_lights(stage: &Stage) -> Vec<TierLight> {
    let mut lights: Vec<TierLight> = stage
        .world
        .scene
        .lights
        .iter()
        .map(|light| {
            let dir = [light.direction.x, light.direction.y, light.direction.z];
            let directional = dir.iter().any(|c| *c != 0.0);
            TierLight {
                pos: if directional {
                    dir
                } else {
                    [light.position.x, light.position.y, light.position.z]
                },
                color: light.color,
                directional,
            }
        })
        .collect();
    if let Some(sky) = &stage.world.scene.sky {
        lights.push(TierLight { pos: [0.0, -1.0, 0.0], color: sky.color, directional: true });
    }
    lights
}

#[test]
fn the_small_doorway_does_not_relight_every_brick() {
    let mix = LightMix { count: 5, dynamic_pct: 50, layout: Layout::First };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 1.0, 0.25, 120.0);
    stage.sun_frozen = true;
    stage.apply();
    // South doorway of section 0, beside the hall lamp. The eye is still inside.
    let eye = [12.5, 1.7, 23.5];
    let mut tier = TierState::default();
    tier.update(scene_boxes(&stage.world.scene), 1, eye, &tier_lights(&stage));
    while tier.has_work() {
        let batch = tier.batch(eye, None, 64);
        tier.commit(&batch);
    }
    let bricks = tier.stats().bricks;
    assert!(bricks > 40, "the small section should fill the tier, got {bricks}");
    stage.box_clock = 0.35;
    stage.apply();
    tier.update(scene_boxes(&stage.world.scene), 1, eye, &tier_lights(&stage));
    let changing = tier.stats().changing_bricks;
    assert!(changing > 0, "the moving boxes marked nothing");
    assert!(changing * 2 < bricks, "{changing} of {bricks} bricks changed");
}
