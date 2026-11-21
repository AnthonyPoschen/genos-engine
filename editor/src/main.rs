//! The editor: the scene program opened on the mesh scene, with every scene
//! in its list. A person looks and selects; an agent edits through the MCP
//! endpoint. The record is `docs/systems/mesh-workbench.md` and the running
//! list is `docs/systems/editor-checklist.md`.

fn main() {
    genos_scene_program::run_program(genos_scene_program::Program::EDITOR)
}
