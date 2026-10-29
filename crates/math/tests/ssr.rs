//! CPU reference for hierarchical-Z screen hits (ADR 0029).
//! The grade is `ssr_depth_rel` against the analytic ray on the built-in scenes.

#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when a fixture fails")
)]

use genos_math::optics::{self, ReflectionPreset};
use genos_math::ssr::{
    grade_reference, rasterize, reflection_dir, step_cap, trace_reference, trunc_u32, CoarseTest,
    DepthImage, HierarchicalZ, MissKind, Plane, Primitive, Query, Trace, View, THICKNESS_M,
};
use genos_math::Vec3;

mod ssr {
    use super::{
        grade_reference, optics, rasterize, reflection_dir, step_cap, trace_reference, trunc_u32,
        CoarseTest, DepthImage, HierarchicalZ, MissKind, Plane, Primitive, Query, ReflectionPreset,
        Trace, Vec3, View, THICKNESS_M,
    };

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn view_128() -> View {
        View {
            width: 128,
            height: 128,
            fovy: 60.0_f64.to_radians(),
            near: 0.05,
        }
    }

    fn floor_and_wall(z: f64) -> Vec<Primitive> {
        vec![
            Primitive::Plane(Plane {
                point: [0.0, -1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                min: [-8.0, -1.05, -12.0],
                max: [8.0, -0.95, -0.3],
                surface: 1,
            }),
            Primitive::Plane(Plane {
                point: [0.0, 0.0, z],
                normal: [0.0, 0.0, 1.0],
                min: [-2.0, -1.5, z - 0.05],
                max: [2.0, 2.5, z + 0.05],
                surface: 2,
            }),
        ]
    }

    fn march_wall(z: f64) -> Trace {
        let view = view_128();
        let primitives = floor_and_wall(z);
        let origin = [0.2, -1.0, -2.0];
        let direction = reflection_dir(origin, [0.0, 1.0, 0.0]);
        let image = rasterize(&view, &primitives);
        let pyramid = HierarchicalZ::build(&image);
        pyramid.march(
            &image,
            &view,
            &Query {
                origin,
                direction,
                skip_surface: 1,
                max_steps: 2048,
                coarse: CoarseTest::MinMax,
            },
        )
    }

    #[test]
    fn reference_scenes_stay_inside_the_depth_gate() {
        let rows = grade_reference(2048);
        let mut hits = 0u32;
        let mut misses = 0u32;
        for row in &rows {
            println!(
                "{:<18} rel={:.6} steps={} min_plane={} analytic={} march={} px={}",
                row.name,
                row.depth_rel,
                row.steps,
                row.min_plane_steps,
                row.analytic_depth
                    .map_or_else(|| "-".to_owned(), |z| format!("{z:.4}")),
                row.march_depth
                    .map_or_else(|| "-".to_owned(), |z| format!("{z:.4}")),
                row.pixel_delta
                    .map_or_else(|| "-".to_owned(), |d| d.to_string()),
            );
            assert!(
                row.within_gate(),
                "{} ssr_depth_rel {} exceeds 0.02",
                row.name,
                row.depth_rel
            );
            match row.name {
                "floor_to_wall" | "floor_to_sphere" | "near_contact" | "near_occludes_far"
                | "behind_post" => {
                    assert!(row.analytic_depth.is_some(), "{}", row.name);
                    assert!(row.march_depth.is_some(), "{}", row.name);
                    hits += 1;
                }
                "leaves_screen" | "beside_wall" => {
                    assert!(row.analytic_depth.is_none(), "{}", row.name);
                    assert!(row.march_depth.is_none(), "{}", row.name);
                    assert!(close(row.depth_rel, 0.0, 0.0), "{}", row.name);
                    misses += 1;
                }
                other => panic!("unexpected scene {other}"),
            }
        }
        assert_eq!(hits, 5);
        assert_eq!(misses, 2);
        let post = rows
            .iter()
            .find(|row| row.name == "behind_post")
            .expect("behind_post");
        assert!(
            post.min_plane_steps > post.steps,
            "min-plane {} vs min-max {}",
            post.min_plane_steps,
            post.steps
        );
    }

    #[test]
    fn near_contact_hits_inside_the_low_step_cap() {
        let trace = trace_reference("near_contact", step_cap(ReflectionPreset::Low));
        let Some(Trace::Hit(hit)) = trace else {
            panic!("expected a hit, got {trace:?}");
        };
        assert_eq!(hit.surface, 2);
        assert!(hit.steps <= 16, "steps {}", hit.steps);
        println!("near_contact steps={}", hit.steps);
    }

    #[test]
    fn the_same_buffer_returns_the_same_trace() {
        let first = trace_reference("floor_to_wall", 2048);
        let second = trace_reference("floor_to_wall", 2048);
        assert_eq!(first, second);
        assert!(matches!(first, Some(Trace::Hit(_))));
    }

    #[test]
    fn a_two_centimetre_wall_move_tracks_the_hit_depth() {
        let near = march_wall(-4.0);
        let far = march_wall(-4.02);
        let (Trace::Hit(a), Trace::Hit(b)) = (near, far) else {
            panic!("both walls should hit: {near:?} {far:?}");
        };
        assert_eq!(a.surface, b.surface);
        assert!(
            close(b.depth - a.depth, 0.02, 1.0e-9),
            "depths {} {}",
            a.depth,
            b.depth
        );
        println!("wall_move depths {} -> {}", a.depth, b.depth);
    }

    #[test]
    fn step_caps_match_the_preset_table() {
        assert_eq!(step_cap(ReflectionPreset::Potato), 0);
        assert_eq!(step_cap(ReflectionPreset::Low), 16);
        assert_eq!(step_cap(ReflectionPreset::Medium), 32);
        assert_eq!(step_cap(ReflectionPreset::High), 48);
        assert!(close(THICKNESS_M, 0.25, 0.0));
    }

    #[test]
    fn reflection_matches_the_optics_mirror_on_unit_vectors() {
        let incident = Vec3::new(0.2, -0.4, -0.8);
        let len =
            (incident.x * incident.x + incident.y * incident.y + incident.z * incident.z).sqrt();
        let incident = Vec3::new(incident.x / len, incident.y / len, incident.z / len);
        let normal = Vec3::Y;
        let mirrored = optics::mirror_dir(incident, normal);
        let reflected = reflection_dir(
            [
                f64::from(incident.x),
                f64::from(incident.y),
                f64::from(incident.z),
            ],
            [
                f64::from(normal.x),
                f64::from(normal.y),
                f64::from(normal.z),
            ],
        );
        assert!(close(reflected[0], f64::from(mirrored.x), 1.0e-5));
        assert!(close(reflected[1], f64::from(mirrored.y), 1.0e-5));
        assert!(close(reflected[2], f64::from(mirrored.z), 1.0e-5));
    }

    #[test]
    fn a_floor_point_projects_to_the_lower_half() {
        let view = View {
            width: 128,
            height: 64,
            fovy: 60.0_f64.to_radians(),
            near: 0.05,
        };
        let projected = view
            .project([0.0, -1.0, -2.0])
            .expect("point is in front of the near plane");
        assert!(close(projected.px, f64::from(view.width) * 0.5, 1.0e-9));
        assert!(projected.py > f64::from(view.height) * 0.5);
        assert!(close(projected.depth, 2.0, 1.0e-12));
        let x = trunc_u32(projected.px).expect("pixel x");
        let y = trunc_u32(projected.py).expect("pixel y");
        let dir = view.pixel_dir(x, y).expect("pixel dir");
        let scale = projected.depth / -dir[2];
        let back = view
            .project([dir[0] * scale, dir[1] * scale, dir[2] * scale])
            .expect("round trip");
        assert!(close(back.px, f64::from(x) + 0.5, 1.0e-6));
        assert!(close(back.py, f64::from(y) + 0.5, 1.0e-6));
    }

    #[test]
    fn an_offscreen_ray_and_a_beside_wall_both_miss() {
        let left = trace_reference("leaves_screen", 2048);
        assert!(
            matches!(
                left,
                Some(Trace::Miss {
                    kind: MissKind::LeftScreen,
                    ..
                })
            ),
            "{left:?}"
        );
        let beside = trace_reference("beside_wall", 2048);
        assert!(matches!(beside, Some(Trace::Miss { .. })), "{beside:?}");
    }

    #[test]
    fn the_floor_receiver_is_not_the_hit() {
        let wall = trace_reference("floor_to_wall", 2048);
        let Some(Trace::Hit(hit)) = wall else {
            panic!("expected the wall, got {wall:?}");
        };
        assert_eq!(hit.surface, 2);
        let near = trace_reference("near_occludes_far", 2048);
        let Some(Trace::Hit(hit)) = near else {
            panic!("expected the near wall, got {near:?}");
        };
        assert_eq!(hit.surface, 2);
    }

    #[test]
    fn an_empty_march_reports_a_bad_image() {
        let view = view_128();
        let image = DepthImage::empty(0, 0);
        let pyramid = HierarchicalZ::build(&image);
        let trace = pyramid.march(
            &image,
            &view,
            &Query {
                origin: [0.0, 0.0, -1.0],
                direction: [0.0, 0.0, -1.0],
                skip_surface: 0,
                max_steps: 8,
                coarse: CoarseTest::MinMax,
            },
        );
        assert_eq!(
            trace,
            Trace::Miss {
                kind: MissKind::BadImage,
                steps: 0
            }
        );
    }
}
