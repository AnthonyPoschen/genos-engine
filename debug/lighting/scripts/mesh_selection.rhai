// The mesh viewer: the sample workspace opens as the mesh scene, a pick at a
// screen point names mesh-workbench IDs, and the selection (with the typed
// note) reads back for an agent. Run on genos-lighting.
load_scene(#{ name: "mesh", checkpoint: "front" });
wait(4);
let view = mesh_view();
check_gt(view.objects.len(), 1, "the sample workspace holds the cube and the bracket");
check_gt(view.faces, 13, "the sample has 14 faces");
// Stand in front of the cube and pick the face that looks at the camera.
camera(#{ position: [0.0, 0.5, 4.0], look_at: [0.0, 0.5, 0.0] });
wait(2);
let pick = mesh_pick(#{ screen: [0.5, 0.5], mode: "face" });
check_gt(pick.hit.face, 1, "the +Z face of the cube is face 2");
check_lt(pick.hit.face, 3, "the +Z face of the cube is face 2");
check_gt(pick.hit.face_vertices.len(), 3, "a quad has four vertices");
let sky = mesh_pick(#{ screen: [0.5, 0.02], select: "none" });
check_lt(mesh_selection().faces.len(), 2, "a report-only pick leaves the selection");
mesh_pick(#{ screen: [0.5, 0.5], mode: "object", select: "add" });
let sel = mesh_selection(#{ note: "make this taller" });
log(`selection: objects ${sel.objects} faces ${sel.faces} note ${sel.note}`);
check_gt(sel.objects.len(), 0, "the cube object joined the selection");
check_gt(sel.faces.len(), 0, "the face stayed selected");
let cleared = mesh_selection(#{ clear: true });
check_lt(cleared.faces.len() + cleared.objects.len(), 1, "clear empties the selection");
