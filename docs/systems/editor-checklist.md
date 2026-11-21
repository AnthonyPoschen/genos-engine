# Editor and mesh workbench checklist

The running list for the editor work. A session reads this first, does the next open item, and updates it in the same change. The design is in [mesh workbench](mesh-workbench.md).

## Decisions from Anthony (22 Nov 2026)

1. The editor is its own binary, for making games and assets. It is not an extension of `genos-lighting`.
2. The engine keeps its own MCP server and extends it. The lighting commands sit under a lighting section. The editor adds its state, its commands, and every mesh command it needs: meshes, bones, weights, materials, animations.
3. Mesh edits go through an API bridge to mesh-workbench under the hood. Keep a list of refactors we want in mesh-workbench, to propose to Nathan.
4. mesh-workbench is an SDK dependency, linked in, not a separate process. If its structure prevents that, fork it under Anthony's GitHub, or set it up without a remote.
5. Build every remaining viewer feature. This file tracks done and to do.
6. Work is pushed to `lighting-v3`.
7. Nathan will add a selection or camera channel upstream if we want one.

Decisions 1 and 4 reverse what the first commits built: the viewer lives in `debug/lighting`, and edits go to a `mesh-server` child process over loopback HTTP.

## Done

- [x] Scene library: `stress` is `lighting-stress`; `make stress` opens the scene program on it; the old name is an alias.
- [x] `genos-workbench`: reads `project.json` into triangles with face, vertex and object IDs; polygon edges; sessions and state snapshots; the web viewer's normal rule.
- [x] `kind mesh` scenes: a workspace on a ground plane, reloaded when `project.json` changes. `make mesh`.
- [x] Pointer: orbit, pan, zoom, frame (`F`), click to select, Shift to add or remove.
- [x] Selection highlight: faces, objects, vertex markers.
- [x] Agent commands: `mesh_view`, `mesh_pick`, `mesh_selection` (with the note and the camera), `mesh_command`.
- [x] Display: hide, show, solo; `shaded`, `object_ids`, `wireframe`; axis views; replay of a recorded state.
- [x] Menu: "Click selects", "Mesh shading", "Show object".
- [x] mesh-workbench linked in (`genos-meshbench`); `mesh_command` runs on its session.
- [x] Checks: `debug/lighting/scripts/mesh_selection.rhai`; unit tests in `genos-workbench` and `debug/lighting/src/mesh_view.rs`; clippy clean on both.

## To do, in order

### A. Link mesh-workbench (decision 4)

- [x] [ADR 0042](../adr/0042-mesh-workbench-is-linked-into-the-editor.md). `deny.toml` allows the one git source.
- [x] `crates/meshbench` over `gpt_mesh::session::Session`: `open`, `execute` (one command or a batch), `revision`. No other crate sees serde.
- [x] `mesh_command` calls the bridge. The child process, the HTTP client, `--mesh-server` and `--mesh-port` are removed.
- [ ] The viewer reads the mesh from the bridge's state when a session is open, in place of re-reading `project.json`. The file reader stays for a workspace another process owns, and for replay.
- [ ] Run the session on a worker so a long command does not stall the frame.

### B. The editor binary (decision 1)

- [ ] `editor/` (package `genos-editor`) on the shared scaffolding. Move the frame loop pieces `debug/lighting` and the editor both need into a shared crate (`docs/architecture.md`: "When a second program needs the same loop, add a shared runtime crate").
- [ ] Move the mesh viewer (`mesh_view.rs`, `mesh_world`, the pointer) from `debug/lighting` to the editor. `genos-lighting` keeps the lighting scenes.
- [ ] `make editor`. The scene picker lists game scenes and mesh workspaces.
- [ ] ADR note: this starts the editor program of ADR 0002 and ADR 0006, as a thin view over agent commands ([agent-driven editing](agent-driven-editing.md), ADR 0019).

### C. The MCP server (decision 2)

- [ ] Group the command catalogue into sections: `lighting`, `scene`, `editor`, `mesh`. Old names stay as deprecated aliases that log a warning for one release (the public-names rule in `AGENTS.md`).
- [ ] Editor state: the open document, the selection, the camera, the pick mode, the note, recent edits.
- [ ] Mesh commands as first-class tools with schemas, not only the `mesh_command` pass-through: objects, faces and vertices, generators, UVs and textures (materials), rig and skin weights (bones, weights), animation clips and keys, import and export, undo and redo, transactions.
- [ ] A context packet for each request: camera pose, a screenshot, the selection, the visible objects.

### D. Viewer parity with the mesh-workbench web viewer (decision 5)

- [x] Orbit, pan, zoom, frame, axis views.
- [ ] Orthographic view. The engine's projection is perspective only (`genos_scene::view_proj`).
- [ ] Auto-rotate turntable.
- [x] Shaded, wireframe, object IDs.
- [ ] Shaded vertex colours; textured lit (material); base colour; unlit vertex colour.
- [ ] Debug views: normals, depth, silhouette, face orientation, face IDs, collection IDs, UV checker.
- [ ] Grid on the ground plane; studio lighting with contrast; contact shadow. The mesh is flat-lit now and casts no shadow.
- [x] Hide, show, solo, frame an object (commands and one menu dropdown).
- [ ] Objects panel: collections and objects as a tree, search, show all, hide matching, solo matching.
- [x] Replay: show the state after a command (`mesh_view replay`).
- [ ] Replay panel: session picker, previous, play, next, go live, scrubber, command filter, the event's input and reply.
- [ ] Animation: clip picker, play and scrub, bones, hierarchy and joint overlays, mesh toggle.
- [ ] Workspace picker; connection and revision readout; statistics footer.

### E. Beyond the web viewer

- [x] Select with a click; the selection over the agent endpoint.
- [ ] A text box in the window for the note.
- [ ] Box and lasso selection; select loop and select connected in the viewer.
- [ ] Preview of a pending edit (mesh-workbench `transaction` with `preview_id`) before it is committed.

## For Nathan

Changes we would like in mesh-workbench. Add to this list as the bridge meets them.

- A library entry point that does not take the workspace file lock for a read-only open, so a viewer can open a workspace another process edits.
- A change notification (a callback or a channel) in place of polling the revision.
- A user selection separate from the tool selection: faces and objects as well as vertices, not recorded as an undo step, with the viewer's camera.
- The workflow gates (`export` refused until requirements are met) switchable off for an editor-driven session.
- The crate, the repository and the docs under one name (`gpt-mesh`, `mesh-workbench`).
- A reference row for each of the roughly 40 commands that are in the code and not in `docs/command-reference.md`.

## Open notes

- `still_gi_pops.rhai` reads 0.06 to 0.09 on `genos-lighting` and 0.008 on `genos-stress`. The cause is auto exposure in the scene program's display path, not the lighting: with `--exposure-mode fixed` it reads 0.009. The script should fix the exposure, or run on both programs.
- `a_bright_lamp_does_not_cross_an_opaque_wall` fails with the doubled voxel sizes ([sparse cascade](lighting/sparse-radiance-cascades.md), Sizes).
- Repository lint was red before this work: clippy errors in `genos-mesh`, shader-constant counts over baseline, and a stale worktree under `.claude/`.
