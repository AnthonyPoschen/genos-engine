//! Points and edges from the triangles the draw already has.
//! A second copy of the same triangles keeps its own points and edges.

use crate::pack::GpuVertex;

/// One point per triangle corner and one line per triangle edge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WireMarks {
    pub points: Vec<[f32; 3]>,
    pub edges: Vec<[[f32; 3]; 2]>,
}

/// Build marks from triangle corners. `corners` is groups of three.
/// A repeated triangle adds three more points and three more edges.
pub(crate) fn marks_from_triangles(corners: &[[f32; 3]]) -> WireMarks {
    let mut points = Vec::new();
    let mut edges = Vec::new();
    for tri in corners.chunks_exact(3) {
        points.push(tri[0]);
        points.push(tri[1]);
        points.push(tri[2]);
        edges.push([tri[0], tri[1]]);
        edges.push([tri[1], tri[2]]);
        edges.push([tri[2], tri[0]]);
    }
    WireMarks { points, edges }
}

const EDGE_HALF: f32 = 0.025;
const POINT_HALF: f32 = 0.04;

/// Triangle ribbons for every mark. Depth is off at draw time, so copies add.
pub(crate) fn mark_vertices(marks: &WireMarks) -> Vec<GpuVertex> {
    let mut out = Vec::new();
    for edge in &marks.edges {
        push_edge(&mut out, edge[0], edge[1], EDGE_HALF);
    }
    for point in &marks.points {
        push_point(&mut out, *point, POINT_HALF);
    }
    out
}

fn push_edge(out: &mut Vec<GpuVertex>, a: [f32; 3], b: [f32; 3], half: f32) {
    let dir = sub(b, a);
    let len = length(dir);
    if len < 1.0e-6 {
        return;
    }
    let dir = scale(dir, 1.0 / len);
    let side = perpendicular(dir);
    let up = cross(dir, side);
    push_ribbon(out, a, b, side, half);
    push_ribbon(out, a, b, up, half);
}

fn push_point(out: &mut Vec<GpuVertex>, point: [f32; 3], half: f32) {
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for axis in axes {
        let a = sub(point, scale(axis, half));
        let b = add(point, scale(axis, half));
        push_ribbon(out, a, b, perpendicular(axis), half);
    }
}

fn push_ribbon(out: &mut Vec<GpuVertex>, a: [f32; 3], b: [f32; 3], side: [f32; 3], half: f32) {
    let offset = scale(side, half);
    let a0 = sub(a, offset);
    let a1 = add(a, offset);
    let b0 = sub(b, offset);
    let b1 = add(b, offset);
    push_tri(out, a0, b0, b1);
    push_tri(out, a0, b1, a1);
}

fn push_tri(out: &mut Vec<GpuVertex>, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    out.push(vertex(a));
    out.push(vertex(b));
    out.push(vertex(c));
}

fn vertex(pos: [f32; 3]) -> GpuVertex {
    GpuVertex {
        pos,
        albedo: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        shade: 0.0,
        uv: [-1.0, -1.0],
    }
}

fn perpendicular(dir: [f32; 3]) -> [f32; 3] {
    let axis = if dir[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let side = cross(dir, axis);
    let len = length(side).max(1.0e-6);
    scale(side, 1.0 / len)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], factor: f32) -> [f32; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::marks_from_triangles;

    #[test]
    fn a_second_copy_of_a_face_keeps_its_points_and_edges() {
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let c = [1.0, 1.0, 0.0];
        let d = [0.0, 1.0, 0.0];
        let face = [a, b, c, a, c, d];
        let mut corners = Vec::new();
        corners.extend_from_slice(&face);
        corners.extend_from_slice(&face);
        let marks = marks_from_triangles(&corners);
        assert_eq!(marks.points.len(), 12, "points merged a copied face");
        assert_eq!(marks.edges.len(), 12, "edges merged a copied face");
        for corner in [a, b, c, d] {
            assert!(
                marks.points.iter().any(|point| *point == corner),
                "missing vertex {corner:?}"
            );
        }
        assert_eq!(
            count_point(&marks.points, a),
            4,
            "shared corner dropped a copy"
        );
        assert_eq!(
            count_point(&marks.points, b),
            2,
            "copied face dropped a corner"
        );
        assert_eq!(
            count_edge(&marks.edges, a, b),
            2,
            "copied face dropped an edge"
        );
        assert_eq!(
            count_edge(&marks.edges, b, c),
            2,
            "copied face dropped an edge"
        );
        assert_eq!(
            count_edge(&marks.edges, c, d),
            2,
            "copied face dropped an edge"
        );
        assert_eq!(
            count_edge(&marks.edges, d, a),
            2,
            "copied face dropped an edge"
        );
        assert_eq!(
            count_edge(&marks.edges, a, c),
            4,
            "copied face dropped the split"
        );
    }

    fn count_point(points: &[[f32; 3]], want: [f32; 3]) -> usize {
        points.iter().filter(|point| **point == want).count()
    }

    fn count_edge(edges: &[[[f32; 3]; 2]], a: [f32; 3], b: [f32; 3]) -> usize {
        edges
            .iter()
            .filter(|edge| (edge[0] == a && edge[1] == b) || (edge[0] == b && edge[1] == a))
            .count()
    }
}
