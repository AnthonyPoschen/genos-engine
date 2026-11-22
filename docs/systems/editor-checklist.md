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
- [x] Menu: the Mesh tab shows the mesh controls (their own tab, drawn only in a mesh scene; the editor opens on it) in a mesh scene. A test checks them. Until 22 Nov 2026 only the on/off ones were drawn: a slider or a dropdown needs its knob name in `SLIDER_KNOBS` or `DROP_KNOBS`.
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
- [x] mesh-workbench is linked by the editor binary only. `genos-editor` gives the scene program the function that opens a session (`Program::mesh_session`); `genos-scene-program` and `genos-lighting` no longer depend on `genos-meshbench`, `gpt-mesh` or `serde_json` (`cargo tree -p genos-lighting`). A mesh command that edits says so on a program without it.
- [ ] The viewer's own code (`mesh_view.rs`, the reader `genos-workbench`) is still in the shared library; `genos-lighting` lists no mesh scene, so it never runs there.
- [x] Mesh workspaces found on disk are in the menu's "Workspace" picker.
- [ ] The scene picker lists the scene files in `debug/scenes` only; a game's own scenes are not listed.
- [x] ADR 0042 records that `genos-editor` starts the editor program of ADR 0002 and ADR 0006, as a thin view over agent commands (ADR 0019).

### C. The MCP server (decision 2)

- [x] Mesh tools by family, each taking one mesh-workbench command object and refusing a command of another family: `mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material` (UVs, textures, vertex colours), `mesh_rig` (bones, skin weights), `mesh_animation`, `mesh_query`, `mesh_document` (undo, redo, transaction, import, export, workflow). `mesh_help` reads mesh-workbench's help. `mesh_command` still takes any command or a batch. The editor's MCP `tools/list` shows 81 tools, 13 of them `mesh_*`.
- [x] Editor state over MCP: `mesh_view` (the open workspace, revision, objects, display, replay) and `mesh_selection` (selection, note, camera).
- [x] A JSON schema for each of the 13 mesh tools (`mesh_schema` in `crates/debug/src/commands.rs`). The viewer's own tools (`mesh_view`, `mesh_pick`, `mesh_selection`, `mesh_help`) name every field with its type and its choices and refuse another; a test keeps the schema and the fields the viewer reads the same. A family tool and `mesh_command` require `command`.
- [ ] A schema for the fields of each mesh-workbench command inside a family tool. mesh-workbench has no machine-readable command list (its help is prose), so those fields are free-form. See "For Nathan".
- [x] The catalogue is in six sections: `scene`, `lighting`, `render`, `capture`, `editor`, `mesh` (`section_of` in `crates/debug/src/commands.rs`). Every description starts with its section in brackets, and `list_commands` returns the catalogue grouped, or one section. The lighting commands are the `lighting` section.
- [x] Section prefixes in the command names: a client sees `lighting.get_settings`, `scene.load`, `mesh.view`. The old names are deprecated aliases that log a warning on first use over MCP or HTTP. Scripts keep the old names (a Rhai name has no dot). `fx.*` and `fx_*` were already published twice and still are.
- [x] Bones, skin weights and animation checked through `mesh_rig` and `mesh_animation`: a two-bone rig, four weighted vertices, a clip, and a sampled pose that moved those vertices 0.6 m.
- [x] `mesh_context`: one reply with the intent (the note), the selection, each selected face's vertices, centre and normal, the camera, the objects in view with their bounds, how the picture is shown, the latest edits (each with its request, whether it ran, and the note it ran under), and a screenshot saved as a file.

### D. Viewer parity with the mesh-workbench web viewer (decision 5)

- [x] Orbit, pan, zoom, frame, axis views.
- [x] Orthographic view (`mesh_view projection`, the menu's "Orthographic"): a parallel projection with the same framing. `Camera::parallel_half` (half the picture's height in metres; 0 is perspective) gives the view-projection its parallel form; the scene block carries it (`view_up.w`), the G-buffer stores depth along the view, and the lighting rebuilds and projects positions with it. `Camera::view_tan` is the perspective field of view. Orbit, zoom, pan, picking, box selection and the face-orientation shade work in it.
- [ ] In the parallel view the fog, reflections, particles and the reference tracer still assume a perspective eye; a mesh scene uses none of them. The lighting sizes its cells by the distance to the eye, which stands up to 120 m back.
- [x] Auto-rotate turntable (`mesh_view turntable`, the menu's "Turntable").
- [x] Shaded, wireframe, object IDs.
- [x] Shaded vertex colours (`shade: vertex_colors`). The mesh draw carries a colour per vertex (`DrawKind::Mesh::colors`).
- [x] Textured (`shade: textured`) and the UV checker (`shade: uv_checker`). A mesh scene's colour atlas holds the checker and every texture of the workspace. A part with one texture, and the checker, repeat as in mesh-workbench. The vertex paint tints the texture; a face with no texture is grey.
- [x] Textured as mesh-workbench draws it: `textured`, `uv_checker` and `vertex_colors_unlit` switch the view to the base colour (unlit) and back; a face with incomplete UVs is magenta and a face with no texture grey; a part with several textures is drawn as one piece a texture, so each repeats.
- [x] Texture alpha as mesh-workbench shows it: a texture with `alpha_cutoff` drops the texels under it; one without stands over mid grey by its alpha.
- [x] The engine drew every texture at its coarsest mip in the scene program (fixed 22 Nov 2026, `Streamer::load`'s starvation floor and the draw call's own time as the streamer's frame time). Sponza draws textured after about 20 seconds.
- [x] Unlit vertex colour (`shade: vertex_colors_unlit`).
- [x] Face IDs (`shade: face_ids`).
- [x] Normals, depth and base colour: the engine's `view` modes `normal`, `depth` and `albedo`. They showed nothing in the scene program until the HDR compose stored the view's colour in the scene image the display pass draws.
- [x] Collection IDs (`shade: collection_ids`).
- [x] Silhouette (`shade: silhouette`): the mesh in black.
- [x] Face orientation (`shade: face_orientation`): a face seen from its front is blue, from behind red. The raster culls nothing, so a face wound the wrong way shows red from outside. The colours are set on the CPU and the mesh is rebuilt only when a face turns over.
- [x] Lighting with contrast: the mesh scene's sun is 5 and its sky 0.1 to 0.16, so faces read apart.
- [x] Grid on the ground (`mesh_view grid`, the menu's "Grid"): a line every power of ten of metres that fits the mesh.
- [x] Contact shadow (`mesh_view shadow`, the menu's "Contact shadow"): a soft dark patch on the ground under each shown object, fainter for an object off the ground. It is drawn, not traced, so it does not follow the light.
- [x] Bloom is off in a mesh scene.
- [ ] Auto exposure still sets the picture's brightness in a mesh scene; `--exposure-mode fixed` holds it.
- [x] Hide, show, solo, frame an object (commands and one menu dropdown).
- [x] Show all, hide matching and solo matching by name (`mesh_view show_all`, `hide_match`, `solo_match`).
- [x] Collections: listed by `mesh_view`, with `solo_collection` and `hide_collection`.
- [x] Objects panel in the window (the Mesh tab's "Objects" section): each collection with its objects under it, then the objects in no collection, each with a box that shows or hides it; a collection's box acts on all its objects. A filter narrows it: Shift and T types it, or `mesh_view filter`.
- [x] Replay: show the state after a command (`mesh_view replay`).
- [x] Replay steps and scrubber: `replay: "next"`, `"previous"`, `"live"`, and the menu's "Replay" slider (0 is the session start, the last step is live).
- [x] Replay play (`replay: "play"`, `"stop"`, the menu's "Play replay"): a step every 0.7 s to the live mesh.
- [x] A session by ID (`replay: {session, event}`) and a recorded command's input and reply (`mesh_view event: N`).
- [x] Replay panel in the window (the Mesh tab's "Replay" section): a session picker, then "live" and the session's latest 12 commands, newest first, each with a box that shows the mesh after it. The same filter narrows the commands by name.
- [ ] The panels are menu rows, not a tree widget: no fold per collection, no scroll of its own, one filter for both, and at most 200 object lines.
- [x] Animation: clip picker and time scrub (`mesh_view pose`, the menu's "Clip" and "Time"), bones with their joints and the line to each parent (`bones`), and the mesh toggle (`surfaces`). The wireframe follows the pose.
- [x] Animation play (`pose: {clip, play: true}`, `"stop"`, the menu's "Play clip"). The clip is sampled once, 12 poses a second of clip and at most 48, and the frames between are blended, so playing does not record a command a frame.
- [x] Picking and the selection highlight follow the pose.
- [x] Posed normals: the normals follow the pose, smooth where the rest normals are.
- [x] Workspace picker (the menu's "Workspace", `mesh_view workspace`): the workspaces found beside the open one, under `debug/scenes/mesh` and under the output directory. A directory with no `project.json` opens as an empty workspace to build in. Readouts: "Mesh" (revision, objects, faces, triangles, vertices) and "Showing" (live, replay or preview, and whether the session is open).

### E. Beyond the web viewer

- [x] Select with a click; the selection over the agent endpoint.
- [x] The note typed in the window: `T` starts, letters, digits and the space bar type, Backspace removes, Enter ends, Escape clears. Hotkeys are off while typing. The Mesh tab shows it as "Note".
- [ ] Punctuation in the typed note (the window gives key codes, not text).
- [x] Box and lasso selection: Control and a left drag draws a box, Alt and a left drag a lasso, and the release selects what is inside by the pick mode (Shift adds). `mesh_pick box` and `mesh_pick lasso` do the same; only what the camera sees unless `through: true`.
- [x] Select connected and select loop: `mesh_selection grow: connected` (everything joined to the selection) and `grow: loop` (the ring of four-sided faces through two selected neighbours). They have no key in the window yet. mesh-workbench's own `select_loop`, `select_box` and `select_related` act on its vertex selection through `mesh_select`.
- [x] Preview of a pending edit (`mesh_preview`): mesh-workbench prepares the commands as one transaction and does not commit; the picture shows the result; `commit: true` commits it as one undo step and `discard: true` drops it. The picture comes from running the same commands on a copy of the workspace under `./genos-debug/mesh-preview/`, because mesh-workbench keeps its prepared mesh private.

### F. Checked in the window

- [x] The menu's Mesh tab was read from a picture of the editor's own window (22 Nov 2026): the folder picker, the readouts, the note, the shading and object dropdowns, the four check boxes, the replay slider, the Objects section with a show box per object, and the Replay section with the session picker and a row per command. A `capture` with `ui: true` does not include the menu; the picture came from the desktop (`grim` on the window's rectangle).
- [x] A readout (the note, the filter, the counts, the state) draws as words. It drew as a slider with a thumb and its text ran off the panel.

## For Nathan

Changes we would like in mesh-workbench. Add to this list as the bridge meets them.

- A library entry point that does not take the workspace file lock for a read-only open, so a viewer can open a workspace another process edits.
- A change notification (a callback or a channel) in place of polling the revision.
- A user selection separate from the tool selection: faces and objects as well as vertices, not recorded as an undo step, with the viewer's camera.
- The workflow gates (`export` refused until requirements are met) switchable off for an editor-driven session.
- The crate, the repository and the docs under one name (`gpt-mesh`, `mesh-workbench`).
- A read path that is not recorded as a session event (or a flag to skip recording), for queries a viewer repeats: `animation_sample` for playback, `query_rig`, `inspect`.
- A page limit above 256 for `animation_sample` and `query_mesh`, or a whole-mesh pose in one reply.
- Access to the prepared transaction's mesh (a `preview_state` query, or the preview written as a state file), so a viewer shows the candidate without running the commands a second time on a copy.
- A machine-readable command list: each command's name, family, parameters and types, so the engine can publish a schema per tool.
- A reference row for each of the roughly 40 commands that are in the code and not in `docs/command-reference.md`.

## Open notes

- `still_gi_pops.rhai` reads 0.06 to 0.09 on `genos-lighting` and 0.008 on `genos-stress`. The cause is auto exposure in the scene program's display path, not the lighting: with `--exposure-mode fixed` it reads 0.009. The script should fix the exposure, or run on both programs.
- `a_bright_lamp_does_not_cross_an_opaque_wall` fails with the doubled voxel sizes ([sparse cascade](lighting/sparse-radiance-cascades.md), Sizes).
- Repository lint was red before this work: clippy errors in `genos-mesh`, shader-constant counts over baseline, and a stale worktree under `.claude/`.
