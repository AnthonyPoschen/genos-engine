#![cfg_attr(test, expect(clippy::expect_used, clippy::panic, reason = "a test aborts when a fixture fails"))]

//! The editor: the scene program opened on the mesh scene, with every scene
//! in its list. A person looks and selects; an agent edits through the MCP
//! endpoint. The record is `docs/systems/mesh-workbench.md` and the running
//! list is `docs/systems/editor-checklist.md`.
//!
//! This binary is the one that links mesh-workbench (ADR 0042): it hands the
//! scene program the way to open a session.

use std::path::Path;

use genos_json::Value;
use genos_scene_program::{MeshSession, Program};

struct Session(genos_meshbench::Bridge);

impl MeshSession for Session {
    fn execute(&mut self, request: &Value) -> Result<Value, String> {
        self.0.execute(request)
    }
}

fn open(dir: &Path) -> Result<Box<dyn MeshSession>, String> {
    Ok(Box::new(Session(genos_meshbench::Bridge::open(dir)?)))
}

fn main() {
    genos_scene_program::run_program(Program { mesh_session: Some(open), ..Program::EDITOR })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch workspace for one test, under the system's temporary directory.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("genos-editor-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    /// The editing loop with no window: a selection and a note in the mesh
    /// viewer, one context call, a preview on the linked mesh-workbench, the
    /// commit, and the mesh checked. `mesh_edit_from_selection.rhai` runs the
    /// same loop on the running editor.
    #[test]
    fn a_selection_and_a_note_become_a_previewed_and_committed_edit() {
        use genos_scene_program::MeshView;
        let dir = scratch("edit-loop");
        let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../debug/scenes/mesh/sample/project.json");
        std::fs::copy(sample, dir.join("project.json")).expect("sample");
        genos_scene_program::use_program(Program { mesh_session: Some(open), ..Program::EDITOR });
        genos_scene_program::set_mesh_out_dir(dir.join("out"));
        let mut view = MeshView::new(dir.clone(), None);
        let mut camera = genos_scene::Camera::new(0.0, 4.0, 0.0);
        let mut call = |view: &mut MeshView, tool: &str, args: &str| -> Value {
            let args = genos_json::parse(args).expect("json");
            let reply = view.command(tool, &args, &mut camera, 1.0).unwrap_or_else(|err| panic!("{tool}: {err}"));
            // An edit answers with a ticket: wait for the session's thread.
            match reply.get("genos_pending_ticket").and_then(Value::as_u64) {
                Some(ticket) => loop {
                    if let Some(done) = view.poll_edit(ticket) {
                        break done.unwrap_or_else(|err| panic!("{tool}: {err}"));
                    }
                    std::thread::yield_now();
                },
                None => reply,
            }
        };
        let number = |value: &Value, path: &[&str]| path.iter().try_fold(value, |at, key| at.get(key)).and_then(Value::as_f64);
        let third = |value: &Value| value.as_array().and_then(|v| v.get(2)).and_then(Value::as_f64);

        // The person: the cube's +Z face, and what they want done with it.
        call(&mut view, "mesh_selection", r#"{"faces":[2],"note":"pull this face out by 30 cm"}"#);
        // The agent: one packet says what is wanted and what it is about.
        let context = call(&mut view, "mesh_context", r#"{"screenshot":false}"#);
        assert_eq!(context.get("intent").and_then(Value::as_str), Some("pull this face out by 30 cm"));
        let face = context.get("selected_faces").and_then(Value::as_array).and_then(|faces| faces.first()).cloned().expect("the selected face");
        assert_eq!(face.get("normal").and_then(third), Some(1.0), "it faces +Z");
        let before = face.get("centre").and_then(third).expect("centre");
        // Shown first: the workspace does not move, and the picture's mesh has the new side faces.
        let preview = call(&mut view, "mesh_preview", r#"{"commands":[{"command":"extrude","face":2,"value":[0,0,0.3]}]}"#);
        assert_eq!(preview.get("previewing"), Some(&Value::Bool(true)), "{}", genos_json::encode(&preview));
        view.reload(None);
        let shown = call(&mut view, "mesh_view", "{}");
        assert_eq!(number(&shown, &["faces"]), Some(18.0), "the preview's mesh: 14 faces and four sides");
        assert_eq!(number(&shown, &["preview", "commands"]), Some(1.0));
        let on_disk = std::fs::read_to_string(dir.join("project.json")).expect("project");
        assert!(on_disk.contains("\"revision\":1") || on_disk.contains("\"revision\": 1"), "the workspace is still at revision 1");
        // Agreed: one undo step.
        let committed = call(&mut view, "mesh_preview", r#"{"commit":true}"#);
        assert_eq!(number(&committed, &["result", "revision"]), Some(2.0), "{}", genos_json::encode(&committed));
        view.reload(None);
        let after = call(&mut view, "mesh_context", r#"{"screenshot":false}"#);
        let now = after.get("selected_faces").and_then(Value::as_array).and_then(|faces| faces.first()).and_then(|face| face.get("centre")).and_then(third).expect("centre");
        assert!((now - before - 0.3).abs() < 1.0e-6, "the face moved 0.3 m: {before} to {now}");
        let edits = after.get("recent_edits").and_then(Value::as_array).map_or_default(<[Value]>::to_vec);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].get("note").and_then(Value::as_str), Some("pull this face out by 30 cm"), "the edit carries the request");
        drop(view);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The catalogue `mesh_commands` serves, read from the linked
    /// mesh-workbench with no window: its commands, the tool each goes to,
    /// and the types its help and its own refusals give the fields.
    #[test]
    fn the_command_catalogue_reads_from_the_linked_mesh_workbench() {
        let dir = scratch("catalogue");
        let mut session = open(&dir).expect("session");
        let found = genos_scene_program::read_catalogue(session.as_mut()).expect("catalogue");
        let _ = std::fs::remove_dir_all(&dir);
        let of = |name: &str| found.iter().find(|doc| doc.command == name).unwrap_or_else(|| panic!("{name} is not listed"));
        let kind = |name: &str, field: &str| of(name).types.iter().find(|(named, _)| named == field).map(|(_, kind)| kind.as_str());
        // The pinned revision documents about 140 commands.
        assert!(found.len() >= 130, "{} commands", found.len());
        assert_eq!(of("extrude").fields, ["face", "value"]);
        // From a table's wording, from an example, from another command, and
        // from mesh-workbench's own refusal.
        assert_eq!(kind("extrude", "value"), Some("array"));
        assert_eq!(kind("animation_set", "clip"), Some("object"));
        assert_eq!(kind("sweep", "cap_start"), Some("boolean"));
        assert_eq!(kind("inspect_uv", "offset"), Some("number"));
        // The rig commands have no table row; their examples list them.
        assert!(of("rig_set").fields.contains(&"rig".to_string()));
        // A render mode is not a command.
        assert!(!found.iter().any(|doc| doc.command == "face_ids"));
        let fields: usize = found.iter().map(|doc| doc.fields.len()).sum();
        let typed: usize = found.iter().map(|doc| doc.types.len()).sum();
        eprintln!("catalogue: {} commands, {typed} of {fields} fields typed", found.len());
        assert!(typed * 100 >= fields * 84, "{typed} of {fields} fields typed");
    }
}
