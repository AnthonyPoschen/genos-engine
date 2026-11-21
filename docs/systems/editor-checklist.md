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
- [x] Menu: the Scene tab shows the mesh controls in a mesh scene. A test checks them. Until 22 Nov 2026 only the on/off ones were drawn: a slider or a dropdown needs its knob name in `SLIDER_KNOBS` or `DROP_KNOBS`.
- [x] mesh-workbench linked in (`genos-meshbench`); `mesh_command` runs on its session.
- [x] Checks: `debug/lighting/scripts/mesh_selection.rhai`; unit tests in `genos-workbench` and `crates/scene-program/src/mesh_view.rs`; clippy clean on both.

## To do, in order

### A. Link mesh-workbench (decision 4)

- [x] [ADR 0042](../adr/0042-mesh-workbench-is-linked-into-the-editor.md). `deny.toml` allows the one git source.
- [x] `crates/meshbench` over `gpt_mesh::session::Session`: `open`, `execute` (one command or a batch), `revision`. No other crate sees serde.
- [x] `mesh_command` calls the bridge. The child process, the HTTP client, `--mesh-server` and `--mesh-port` are removed.
- [ ] The viewer reads the mesh from the bridge's state when a session is open, in place of re-reading `project.json`. The file reader stays for a workspace another process owns, and for replay.
- [ ] Run the session on a worker so a long command does not stall the frame.

### B. The editor binary (decision 1)

- [x] `editor/` (package `genos-editor`, `make editor`). It runs the same frame loop as `genos-lighting`: the library `genos_scene_program`, where a `Program` value gives each binary its name, title, MCP port, opening scene and scene list.
- [x] The editor opens on the mesh scene, lists every scene, and takes MCP port 8765. `genos-lighting` keeps port 8766 and does not list mesh scenes. `make mesh` runs the editor.
- [x] The frame loop is its own crate, `crates/scene-program` (`genos-scene-program`): the loop, the menu and its layout file, and the mesh viewer. `debug/lighting` and `editor` are two small binaries on it.
- [ ] Compile the mesh viewer into the editor only. Today both binaries link it and only the editor lists mesh scenes.
- [ ] The scene picker lists game scenes and mesh workspaces found on disk, not only the files in `debug/scenes`.
- [ ] ADR note: this starts the editor program of ADR 0002 and ADR 0006, as a thin view over agent commands ([agent-driven editing](agent-driven-editing.md), ADR 0019).

### C. The MCP server (decision 2)

- [x] Mesh tools by family, each taking one mesh-workbench command object and refusing a command of another family: `mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material` (UVs, textures, vertex colours), `mesh_rig` (bones, skin weights), `mesh_animation`, `mesh_query`, `mesh_document` (undo, redo, transaction, import, export, workflow). `mesh_help` reads mesh-workbench's help. `mesh_command` still takes any command or a batch. The editor's MCP `tools/list` shows 81 tools, 13 of them `mesh_*`.
- [x] Editor state over MCP: `mesh_view` (the open workspace, revision, objects, display, replay) and `mesh_selection` (selection, note, camera).
- [ ] A JSON schema per mesh command. mesh-workbench has no machine-readable command list (its help is prose), so the family tools take a free-form object. See "For Nathan".
- [x] The catalogue is in six sections: `scene`, `lighting`, `render`, `capture`, `editor`, `mesh` (`section_of` in `crates/debug/src/commands.rs`). Every description starts with its section in brackets, and `list_commands` returns the catalogue grouped, or one section. The lighting commands are the `lighting` section.
- [ ] Section prefixes in the command names themselves (`lighting.get_settings`), with the old names as deprecated aliases that log a warning for one release (the public-names rule in `AGENTS.md`). The names are unchanged today: every script and test uses them.
- [x] Bones, skin weights and animation checked through `mesh_rig` and `mesh_animation`: a two-bone rig, four weighted vertices, a clip, and a sampled pose that moved those vertices 0.6 m.
- [ ] Recent edits in the editor state, and a context packet for each request: camera pose, a screenshot, the selection, the visible objects.

### D. Viewer parity with the mesh-workbench web viewer (decision 5)

- [x] Orbit, pan, zoom, frame, axis views.
- [ ] Orthographic view. Blocked on the engine: the field of view is a fixed 60 degrees in `genos_scene::view_proj`, in the scene block the lighting passes rebuild positions from (`view_right.w`), in the fog passes, and in the references. It needs a field of view (or a projection kind) on `Camera` carried through those, in its own change with the lighting gates run.
- [x] Auto-rotate turntable (`mesh_view turntable`, the menu's "Turntable").
- [x] Shaded, wireframe, object IDs.
- [x] Shaded vertex colours (`shade: vertex_colors`). The mesh draw carries a colour per vertex (`DrawKind::Mesh::colors`).
- [ ] Textured lit (material) and the UV checker. `genos-workbench` reads the textures (RGBA from `rgba_base64`), each face's texture, and each corner's UV with V turned to the image's top row. Drawing them is blocked on the engine: built as a hand-made material scene through `World::add_gltf`, the object reaches the atlas correctly on the CPU (128 by 128 rect, the draw carries its UVs) and draws black, and Sponza draws with flat colours and no textures in this program. What is known: `gbuffer.frag` takes the textured branch when the vertex has a UV and the surface has an atlas rect, and reads the atlas with `texelFetch` at the streamer's resident mip (`atlas_mode.color_lod`, `material_atlas.glsl`). The atlas key is a hash of its content (`MaterialAtlas::rehash`), so a world built at run time does reach `sync_materials`. `get_material_stats` reported 4 pages committed and no holes while the object drew black. Next: check which mip `atlas_lod` names for an atlas bound after start-up and whether that mip of the device image was written.
- [ ] Unlit vertex colour.
- [x] Face IDs (`shade: face_ids`).
- [x] Normals, depth and base colour: the engine's `view` modes `normal`, `depth` and `albedo`. They showed nothing in the scene program until the HDR compose stored the view's colour in the scene image the display pass draws.
- [x] Collection IDs (`shade: collection_ids`).
- [x] Silhouette (`shade: silhouette`): the mesh in black.
- [ ] Face orientation. Back faces are culled, so a face wound the wrong way is a hole, not a red face; it needs a two-sided draw.
- [x] Lighting with contrast: the mesh scene's sun is 5 and its sky 0.1 to 0.16, so faces read apart.
- [x] Grid on the ground (`mesh_view grid`, the menu's "Grid"): a line every power of ten of metres that fits the mesh.
- [ ] Contact shadow (the mesh casts no shadow). Bloom and auto exposure soften the picture and could be off for a mesh scene.
- [x] Hide, show, solo, frame an object (commands and one menu dropdown).
- [x] Show all, hide matching and solo matching by name (`mesh_view show_all`, `hide_match`, `solo_match`).
- [x] Collections: listed by `mesh_view`, with `solo_collection` and `hide_collection`.
- [ ] Objects panel in the window: collections and objects as a tree with a search box. The menu has one "Show object" dropdown; the rest is commands.
- [x] Replay: show the state after a command (`mesh_view replay`).
- [x] Replay steps and scrubber: `replay: "next"`, `"previous"`, `"live"`, and the menu's "Replay" slider (0 is the session start, the last step is live).
- [x] Replay play (`replay: "play"`, `"stop"`, the menu's "Play replay"): a step every 0.7 s to the live mesh.
- [x] A session by ID (`replay: {session, event}`) and a recorded command's input and reply (`mesh_view event: N`).
- [ ] Replay panel in the window: a session picker and a command filter.
- [x] Animation: clip picker and time scrub (`mesh_view pose`, the menu's "Clip" and "Time"), bones with their joints and the line to each parent (`bones`), and the mesh toggle (`surfaces`). The wireframe follows the pose.
- [x] Animation play (`pose: {clip, play: true}`, `"stop"`, the menu's "Play clip"). The clip is sampled once, 12 poses a second of clip and at most 48, and the frames between are blended, so playing does not record a command a frame.
- [x] Picking and the selection highlight follow the pose.
- [ ] Posed normals (the pose keeps the rest normals).
- [ ] Workspace picker; connection and revision readout; statistics footer.

### E. Beyond the web viewer

- [x] Select with a click; the selection over the agent endpoint.
- [x] The note typed in the window: `T` starts, letters, digits and the space bar type, Backspace removes, Enter ends, Escape clears. Hotkeys are off while typing. The Scene tab shows it as "Note".
- [ ] Punctuation in the typed note (the window gives key codes, not text).
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
