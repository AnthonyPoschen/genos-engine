//! Scenes the checker knows. The stress driver builds the geometry.
//!
//! Sponza is listed so a run records that it was skipped. It is not traced
//! until a Sponza asset is in the tree.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneDesc {
    pub id: &'static str,
    pub title: &'static str,
    pub available: bool,
    /// Empty when the scene can be traced.
    pub skip_reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointDesc {
    pub scene: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    /// Set on the settled pose after a change. Live frames at +1/+2/+4/+8
    /// are compared with this picture.
    pub event: Option<&'static str>,
    /// A fixed camera. Flicker is the temporal variance of a live sequence.
    pub still: bool,
}

pub fn scenes() -> &'static [SceneDesc] {
    &[
        SceneDesc {
            id: "stress-still",
            title: "Stress building, camera and movers held",
            available: true,
            skip_reason: "",
        },
        SceneDesc {
            id: "stress-moving",
            title: "Stress building, boxes and lamps in motion",
            available: true,
            skip_reason: "",
        },
        SceneDesc {
            id: "bounce-corner",
            title: "Room A corner, lit by bounce only",
            available: true,
            skip_reason: "",
        },
        SceneDesc {
            id: "hell-100",
            title: "One section, 100 lamps, night",
            available: true,
            skip_reason: "",
        },
        SceneDesc {
            id: "direct-only",
            title: "Direct light only, the normalisation control",
            available: true,
            skip_reason: "",
        },
        SceneDesc {
            id: "sponza",
            title: "Sponza",
            available: false,
            skip_reason: "no Sponza asset in this repository",
        },
    ]
}

pub fn checkpoints() -> &'static [CheckpointDesc] {
    &[
        CheckpointDesc {
            scene: "stress-still",
            name: "hall",
            note: "Hall bench at noon, boxes held. The still-camera flicker pose.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "stress-still",
            name: "hall-morning",
            note: "Hall at 10.9 h, boxes held. The sunlit wall whose bounce settled dark.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "stress-still",
            name: "room-a",
            note: "Room A at noon, looking out of the bounce opening.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "stress-moving",
            name: "hall-moving",
            note: "Hall with moving boxes and a share of the lamps moving.",
            event: None,
            still: false,
        },
        CheckpointDesc {
            scene: "stress-moving",
            name: "box-step-before",
            note: "Hall just before the box step. A held stale frame looks like this.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "stress-moving",
            name: "box-step",
            note: "Hall after the boxes jump and hold. Convergence is scored here.",
            event: Some("box-step"),
            still: false,
        },
        CheckpointDesc {
            scene: "bounce-corner",
            name: "corner",
            note: "Inside room A, aimed at the corner the opening lights by bounce.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "hell-100",
            name: "night-hall",
            note: "Night, 100 lamps, most of them moving. One 25 m section.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "direct-only",
            name: "hall",
            note: "Hall bench. Path trace with zero bounces. Renderer with bounce count 0 and ambient off. The mean luminance ratio is the normalisation control and should sit near 1.",
            event: None,
            still: true,
        },
        CheckpointDesc {
            scene: "sponza",
            name: "atrium",
            note: "Not traced until a Sponza asset exists.",
            event: None,
            still: true,
        },
    ]
}

pub fn scene(id: &str) -> Option<&'static SceneDesc> {
    scenes().iter().find(|scene| scene.id == id)
}

pub fn available_checkpoints() -> impl Iterator<Item = &'static CheckpointDesc> {
    checkpoints()
        .iter()
        .filter(|cp| scene(cp.scene).is_some_and(|scene| scene.available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenes_are_listed_and_sponza_is_skipped() {
        let ids: Vec<_> = scenes().iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec![
                "stress-still",
                "stress-moving",
                "bounce-corner",
                "hell-100",
                "direct-only",
                "sponza"
            ]
        );
        let sponza = scene("sponza").unwrap();
        assert!(!sponza.available);
        assert!(sponza.skip_reason.contains("Sponza"));
        assert!(available_checkpoints().all(|cp| cp.scene != "sponza"));
        assert!(available_checkpoints().any(|cp| cp.name == "corner"));
        assert!(available_checkpoints().any(|cp| cp.scene == "direct-only"));
        assert!(available_checkpoints().any(|cp| cp.event == Some("box-step")));
    }
}
