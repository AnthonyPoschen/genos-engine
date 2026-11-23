// The whole editing loop, as an agent runs it. A person picks a face and
// types what they want. The agent reads one context packet, turns the note
// and the face's normal into a mesh-workbench command, shows it as a preview,
// commits it, and checks the mesh. Run on genos-editor (it links
// mesh-workbench). The script builds its own workspace under the output
// directory, so the sample workspace is not changed.
load_scene(#{ name: "mesh", checkpoint: "front" });
wait(4);
mesh_view(#{ workspace: "genos-debug/mesh-edit-check" });
wait(4);
mesh_object(#{ command: "cube" });
wait(4);
let made = mesh_view();
check_gt(made.faces, 5, "the new workspace holds a cube");
check_lt(made.faces, 7, "the new workspace holds a cube");

// The person: looks at the cube, clicks a face, types the request.
mesh_view(#{ look: "+z" });
wait(2);
let pick = mesh_pick(#{ screen: [0.5, 0.5], mode: "face" });
check_gt(pick.hit.face, 0, "the click is on a face");
mesh_selection(#{ note: "pull this face out by 30 cm" });

// The agent: one call says what is wanted and what it is about.
let context = mesh_context(#{ screenshot: false });
check_gt(context.intent.len(), 10, "the packet carries the note");
check_gt(context.selected_faces.len(), 0, "the packet describes the selected face");
check_lt(context.selected_faces.len(), 2, "one face is selected");
let face = context.selected_faces[0];
let before = face.centre;
// "out by 30 cm": along the face's normal.
let amount = 0.3;
let value = [face.normal[0] * amount, face.normal[1] * amount, face.normal[2] * amount];
let known = mesh_commands(#{ command: "extrude" });
check_gt(known.fields.len(), 1, "the catalogue names extrude's fields");

// Show it first. The live mesh is not changed by a preview.
let revision = mesh_view().revision;
let preview = mesh_preview(#{ commands: [#{ command: "extrude", face: face.face, value: value }] });
wait(4);
let shown = mesh_view();
check_gt(shown.faces, made.faces, "the picture shows the extruded mesh");
check_gt(shown.preview.commands, 0, "the view says it is a preview");

// The person agrees: commit, as one undo step.
mesh_preview(#{ commit: true });
wait(4);
let after = mesh_view();
check_gt(after.revision, revision, "the commit moved the revision");
check_gt(after.faces, made.faces, "the committed mesh has the side faces");

// The face is 0.3 m further along its normal. It is still selected.
let moved = mesh_context(#{ screenshot: false });
let now = moved.selected_faces[0].centre;
let along = (now[0] - before[0]) * face.normal[0] + (now[1] - before[1]) * face.normal[1] + (now[2] - before[2]) * face.normal[2];
log(`face ${face.face} moved ${along} m along its normal`);
check_gt(along, 0.29, "the face moved out by 30 cm");
check_lt(along, 0.31, "the face moved out by 30 cm");
check_gt(moved.recent_edits.len(), 1, "the edits are on record");
let last = moved.recent_edits[moved.recent_edits.len() - 1];
check_gt(last.note.len(), 10, "the edit is recorded with the request it answered");
