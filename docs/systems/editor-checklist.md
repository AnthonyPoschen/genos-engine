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

- [x] `editor/` (package `genos-editor`, `make editor`). It runs the same frame loop as `genos-lighting`: that loop is now the library `genos_scene_program` in `debug/lighting`, and a `Program` value gives each binary its name, title, MCP port, opening scene and scene list.
- [x] The editor opens on the mesh scene, lists every scene, and takes MCP port 8765. `genos-lighting` keeps port 8766 and does not list mesh scenes. `make mesh` runs the editor.
- [ ] Move the library out of `debug/lighting` into its own crate (`crates/scene-program`), and the mesh viewer module with it into an editor-only module. Today the mesh code is compiled into both binaries and only listed by the editor.
- [ ] The scene picker lists game scenes and mesh workspaces found on disk, not only the files in `debug/scenes`.
- [ ] ADR note: this starts the editor program of ADR 0002 and ADR 0006, as a thin view over agent commands ([agent-driven editing](agent-driven-editing.md), ADR 0019).

### C. The MCP server (decision 2)

- [x] Mesh tools by family, each taking one mesh-workbench command object and refusing a command of another family: `mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material` (UVs, textures, vertex colours), `mesh_rig` (bones, skin weights), `mesh_animation`, `mesh_query`, `mesh_document` (undo, redo, transaction, import, export, workflow). `mesh_help` reads mesh-workbench's help. `mesh_command` still takes any command or a batch. The editor's MCP `tools/list` shows 81 tools, 13 of them `mesh_*`.
- [x] Editor state over MCP: `mesh_view` (the open workspace, revision, objects, display, replay) and `mesh_selection` (selection, note, camera).
- [ ] A JSON schema per mesh command. mesh-workbench has no machine-readable command list (its help is prose), so the family tools take a free-form object. See "For Nathan".
- [ ] Group the whole catalogue into sections (`lighting`, `scene`, `editor`, `mesh`) with the lighting commands under the lighting section. Old names stay as deprecated aliases that log a warning for one release (the public-names rule in `AGENTS.md`). Today only the mesh tools say their section, in their descriptions.
- [x] Bones, skin weights and animation checked through `mesh_rig` and `mesh_animation`: a two-bone rig, four weighted vertices, a clip, and a sampled pose that moved those vertices 0.6 m.
- [ ] Recent edits in the editor state, and a context packet for each request: camera pose, a screenshot, the selection, the visible objects.

### D. Viewer parity with the mesh-workbench web viewer (decision 5)

- [x] Orbit, pan, zoom, frame, axis views.
- [ ] Orthographic view. The engine's projection is perspective only (`genos_scene::view_proj`).
- [x] Auto-rotate turntable (`mesh_view turntable`, the menu's "Turntable").
- [x] Shaded, wireframe, object IDs.
- [x] Shaded vertex colours (`shade: vertex_colors`). The mesh draw carries a colour per vertex (`DrawKind::Mesh::colors`).
- [ ] Textured lit (material); base colour; unlit vertex colour.
- [x] Face IDs (`shade: face_ids`).
- [x] Normals, depth and base colour: the engine's `view` modes `normal`, `depth` and `albedo`. They showed nothing in the scene program until the HDR compose stored the view's colour in the scene image the display pass draws.
- [ ] Debug views: silhouette, face orientation, collection IDs, UV checker.
- [x] Lighting with contrast: the mesh scene's sun is 5 and its sky 0.1 to 0.16, so faces read apart.
- [ ] Grid on the ground plane; contact shadow (the mesh casts no shadow); bloom and auto exposure soften the picture and could be off for a mesh scene.
- [x] Hide, show, solo, frame an object (commands and one menu dropdown).
- [x] Show all, hide matching and solo matching by name (`mesh_view show_all`, `hide_match`, `solo_match`).
- [ ] Objects panel in the window: collections and objects as a tree with a search box.
- [x] Replay: show the state after a command (`mesh_view replay`).
- [x] Replay steps and scrubber: `replay: "next"`, `"previous"`, `"live"`, and the menu's "Replay" slider (0 is the session start, the last step is live).
- [x] Replay play (`replay: "play"`, `"stop"`, the menu's "Play replay"): a step every 0.7 s to the live mesh.
- [ ] Replay panel: session picker, command filter, the event's input and reply.
- [x] Animation: clip picker and time scrub (`mesh_view pose`, the menu's "Clip" and "Time"), bones with their joints and the line to each parent (`bones`), and the mesh toggle (`surfaces`). The wireframe follows the pose.
- [x] Animation play (`pose: {clip, play: true}`, `"stop"`, the menu's "Play clip"). The clip is sampled once, 12 poses a second of clip and at most 48, and the frames between are blended, so playing does not record a command a frame.
- [ ] Posed normals (the pose keeps the rest normals), and picking and the selection highlight on the posed mesh (they use the rest positions).
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
- A read path that is not recorded as a session event (or a flag to skip recording), for queries a viewer repeats: `animation_sample` for playback, `query_rig`, `inspect`.
- A page limit above 256 for `animation_sample` and `query_mesh`, or a whole-mesh pose in one reply.
- A machine-readable command list: each command's name, family, parameters and types, so the engine can publish a schema per tool.
- A reference row for each of the roughly 40 commands that are in the code and not in `docs/command-reference.md`.

## Open notes

- `still_gi_pops.rhai` reads 0.06 to 0.09 on `genos-lighting` and 0.008 on `genos-stress`. The cause is auto exposure in the scene program's display path, not the lighting: with `--exposure-mode fixed` it reads 0.009. The script should fix the exposure, or run on both programs.
- `a_bright_lamp_does_not_cross_an_opaque_wall` fails with the doubled voxel sizes ([sparse cascade](lighting/sparse-radiance-cascades.md), Sizes).
- Repository lint was red before this work: clippy errors in `genos-mesh`, shader-constant counts over baseline, and a stale worktree under `.claude/`.
