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
