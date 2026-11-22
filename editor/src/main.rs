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
