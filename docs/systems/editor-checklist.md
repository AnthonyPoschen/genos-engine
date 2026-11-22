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
- [x] The viewer keeps reading `project.json`, by decision: the session rewrites that file as each command ends, and the viewer reloads it on the frame the reply arrives. One read path serves a session in this program, a workspace another process edits, a replayed state and a preview.
- [x] The session runs on its own thread (`Worker` in `mesh_view.rs`). A mesh tool's command is sent to it and the reply goes out when it finishes (`Host::scene_command_poll`, `Job::SceneCommand`), so a long command does not hold the frame. `mesh_preview` and a pose sample still wait for their own short requests.

### B. The editor binary (decision 1)

- [x] `editor/` (package `genos-editor`, `make editor`). It runs the same frame loop as `genos-lighting`: the library `genos_scene_program`, where a `Program` value gives each binary its name, title, MCP port, opening scene and scene list.
- [x] The editor opens on the mesh scene, lists every scene, and takes MCP port 8765. `genos-lighting` keeps port 8766 and does not list mesh scenes. `make mesh` runs the editor.
- [x] The frame loop is its own crate, `crates/scene-program` (`genos-scene-program`): the loop, the menu and its layout file, and the mesh viewer. `debug/lighting` and `editor` are two small binaries on it.
- [x] mesh-workbench is linked by the editor binary only. `genos-editor` gives the scene program the function that opens a session (`Program::mesh_session`); `genos-scene-program` and `genos-lighting` no longer depend on `genos-meshbench`, `gpt-mesh` or `serde_json` (`cargo tree -p genos-lighting`). A mesh command that edits says so on a program without it.
- [x] The mesh viewer is a build feature of the shared frame loop (`mesh` on `genos-scene-program`): `genos-editor` turns it on, and a build of `genos-lighting` alone (`cargo build -p genos-lighting`, `make stress`) has no mesh viewer and no mesh reader in it (`cargo tree -p genos-lighting` names neither). A build of the whole workspace has it in both, because Cargo gives a shared crate one set of features. Both configurations compile, lint and pass their tests (34 with the feature, 11 without). Run the viewer's own tests with `cargo test -p genos-scene-program --features mesh`; a workspace test run has the feature on.
- [x] `textured` looked missing from the window's "Mesh shading" list (23 Nov 2026, Anthony): the list shows six rows and scrolls, and `textured` was eighth. The texture shades now come first: `shaded`, `textured`, `material`, `wireframe`, `uv_checker`, `vertex_colors`.
- [x] Mesh workspaces found on disk are in the menu's "Workspace" picker.
- [x] `--scenes DIR` (mirror `GENOS_SCENES`) adds the `.scene` files of another directory to the scene picker: a game's own scenes. Paths inside a scene file are still under the repository.
- [x] ADR 0042 records that `genos-editor` starts the editor program of ADR 0002 and ADR 0006, as a thin view over agent commands (ADR 0019).

### C. The MCP server (decision 2)

- [x] Mesh tools by family, each taking one mesh-workbench command object and refusing a command of another family: `mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material` (UVs, textures, vertex colours), `mesh_rig` (bones, skin weights), `mesh_animation`, `mesh_query`, `mesh_document` (undo, redo, transaction, import, export, workflow). `mesh_help` reads mesh-workbench's help. `mesh_command` still takes any command or a batch. The editor's MCP `tools/list` shows 81 tools, 13 of them `mesh_*`.
- [x] Editor state over MCP: `mesh_view` (the open workspace, revision, objects, display, replay) and `mesh_selection` (selection, note, camera).
- [x] A JSON schema for each of the 13 mesh tools (`mesh_schema` in `crates/debug/src/commands.rs`). The viewer's own tools (`mesh_view`, `mesh_pick`, `mesh_selection`, `mesh_help`) name every field with its type and its choices and refuse another; a test keeps the schema and the fields the viewer reads the same. A family tool and `mesh_command` require `command`.
- [x] `mesh_commands`: the mesh-workbench commands as its own help documents them, read from the linked version: 143 commands, the tool each goes to and its fields. `command: NAME` gives the reference text, an example and a JSON schema. A field's type comes from the JSON examples in the help (85 commands have at least one typed field), and a command with no table row (the rig and animation commands) is listed from its examples. The help is read once, on a scratch workspace, and a name is kept only if mesh-workbench accepts it.
- [x] A field's type also comes from how the reference writes it (`value:[x,y,z]`, `kind:"reference"`, `clean:true`): 118 of the 143 commands have a typed field, 314 of 518 fields.
- [x] A field with no type of its own takes the type it has in mesh-workbench's other commands when they all agree (marked as such in the schema): 435 of 518 fields are typed, and 98 of the 143 commands have every field typed.
- [x] A field still untyped is asked of mesh-workbench itself: sent as a value no field takes, it is refused by name when the command reaches it ("cap_start must be boolean", "offset must be an unsigned integer"). 446 of 518 fields are typed. A test in the editor crate builds the catalogue from the linked mesh-workbench with no window (`the_command_catalogue_reads_from_the_linked_mesh_workbench`).
- [ ] The other 72 names have their form in the text only. Many are not fields: the table parser also picks up a blend mode's values (`multiply`, `screen`) and the names of reply fields (`glb_base64`). A complete list waits on a machine-readable one from Nathan.
- [x] The catalogue is in six sections: `scene`, `lighting`, `render`, `capture`, `editor`, `mesh` (`section_of` in `crates/debug/src/commands.rs`). Every description starts with its section in brackets, and `list_commands` returns the catalogue grouped, or one section. The lighting commands are the `lighting` section.
- [x] Section prefixes in the command names: a client sees `lighting.get_settings`, `scene.load`, `mesh.view`. The old names are deprecated aliases that log a warning on first use over MCP or HTTP. Scripts keep the old names (a Rhai name has no dot). `fx.*` and `fx_*` were already published twice and still are.
- [x] Bones, skin weights and animation checked through `mesh_rig` and `mesh_animation`: a two-bone rig, four weighted vertices, a clip, and a sampled pose that moved those vertices 0.6 m.
- [x] `mesh_context`: one reply with the intent (the note), the selection, each selected face's vertices, centre and normal, the camera, the objects in view with their bounds, how the picture is shown, the latest edits (each with its request, whether it ran, and the note it ran under), and a screenshot saved as a file.

### D. Viewer parity with the mesh-workbench web viewer (decision 5)

- [x] Orbit, pan, zoom, frame, axis views.
- [x] Orthographic view (`mesh_view projection`, the menu's "Orthographic"): a parallel projection with the same framing. `Camera::parallel_half` (half the picture's height in metres; 0 is perspective) gives the view-projection its parallel form; the scene block carries it (`view_up.w`), the G-buffer stores depth along the view, and the lighting rebuilds and projects positions with it. `Camera::view_tan` is the perspective field of view. Orbit, zoom, pan, picking, box selection and the face-orientation shade work in it.
- [x] The parallel view has defined behaviour in every pass: the picture, the lighting and picking use it; the frame fog, the particles and the reflections are not drawn in it (each is built for a perspective eye); the reference tracer (`reference`, `compare`) refuses with a message that says why. A mesh scene uses none of the four.
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
- [x] A mesh scene holds a fixed exposure (EV 1.2) with bloom off, so the picture is as bright from every side. The Output tab still changes both, and another scene gets its own settings back. (The first attempt set bloom in the world builder, where the preset then replaced it; it is set when the mesh view opens.)
- [x] Hide, show, solo, frame an object (commands and one menu dropdown).
- [x] Show all, hide matching and solo matching by name (`mesh_view show_all`, `hide_match`, `solo_match`).
- [x] Collections: listed by `mesh_view`, with `solo_collection` and `hide_collection`.
- [x] Objects panel in the window (the Mesh tab's "Objects" section): each collection with its objects under it, then the objects in no collection, each with a box that shows or hides it; a collection's box acts on all its objects. A filter narrows it: Shift and T types it, or `mesh_view filter`.
- [x] Replay: show the state after a command (`mesh_view replay`).
- [x] Replay steps and scrubber: `replay: "next"`, `"previous"`, `"live"`, and the menu's "Replay" slider (0 is the session start, the last step is live).
- [x] Replay play (`replay: "play"`, `"stop"`, the menu's "Play replay"): a step every 0.7 s to the live mesh.
- [x] A session by ID (`replay: {session, event}`) and a recorded command's input and reply (`mesh_view event: N`).
- [x] Replay panel in the window (the Mesh tab's "Replay" section): a session picker, then "live" and the session's latest 12 commands, newest first, each with a box that shows the mesh after it. The same filter narrows the commands by name.
- [x] A collection's row has an "open" box that folds its objects away. The Replay panel has its own filter (`mesh_view command_filter`; Control and T types it).
- [x] The Objects and Replay lists are a panel of their own: a second panel on the right under the gizmo, with its own place (it drags) and folds. Its HIDE button and the OBJECTS button under the gizmo close it, the button opens it again, and `mesh_view panel` does both. The Objects list holds at most 200 lines (the filter narrows it). Read from a picture of the window.
- [x] Animation: clip picker and time scrub (`mesh_view pose`, the menu's "Clip" and "Time"), bones with their joints and the line to each parent (`bones`), and the mesh toggle (`surfaces`). The wireframe follows the pose.
- [x] Animation play (`pose: {clip, play: true}`, `"stop"`, the menu's "Play clip"). The clip is sampled once, 12 poses a second of clip and at most 48, and the frames between are blended, so playing does not record a command a frame.
- [x] Picking and the selection highlight follow the pose.
- [x] Posed normals: the normals follow the pose, smooth where the rest normals are.
- [x] Workspace picker (the menu's "Workspace", `mesh_view workspace`): the workspaces found beside the open one, under `debug/scenes/mesh` and under the output directory. A directory with no `project.json` opens as an empty workspace to build in. Readouts: "Mesh" (revision, objects, faces, triangles, vertices) and "Showing" (live, replay or preview, and whether the session is open).

### E. Beyond the web viewer

- [x] Select with a click; the selection over the agent endpoint.
- [x] The note typed in the window: `T` starts, letters, digits and the space bar type, Backspace removes, Enter ends, Escape clears. Hotkeys are off while typing. The Mesh tab shows it as "Note".
- [x] The typed note takes the signs on the digit keys (Shift), the brackets, the backtick, and the number pad's point and signs.
- [x] The window types the punctuation row: `InputCode` gained `key_comma`, `key_period`, `key_minus`, `key_equal`, `key_semicolon`, `key_quote`, `key_slash` and `key_backslash` (the Windows OEM virtual keys, as `key_grave` and the brackets are), with their evdev codes. Shift gives the upper mark on a US layout; another layout's marks are not read.
- [x] Box and lasso selection: Control and a left drag draws a box, Alt and a left drag a lasso, and the release selects what is inside by the pick mode (Shift adds). `mesh_pick box` and `mesh_pick lasso` do the same; only what the camera sees unless `through: true`.
- [x] Select connected and select loop: `mesh_selection grow: connected` (everything joined to the selection) and `grow: loop` (the ring of four-sided faces through two selected neighbours). They have no key in the window yet. mesh-workbench's own `select_loop`, `select_box` and `select_related` act on its vertex selection through `mesh_select`.
- [x] Preview of a pending edit (`mesh_preview`): mesh-workbench prepares the commands as one transaction and does not commit; the picture shows the result; `commit: true` commits it as one undo step and `discard: true` drops it. The picture comes from running the same commands on a copy of the workspace under `./genos-debug/mesh-preview/`, because mesh-workbench keeps its prepared mesh private.

### F. Checked in the window

- [x] The menu's Mesh tab was read from a picture of the editor's own window (22 Nov 2026): the folder picker, the readouts, the note, the shading and object dropdowns, the four check boxes, the replay slider, the Objects section with a show box per object, and the Replay section with the session picker and a row per command. A `capture` with `ui: true` does not include the menu; the picture came from the desktop (`grim` on the window's rectangle).
- [x] A readout (the note, the filter, the counts, the state) draws as words. It drew as a slider with a thumb and its text ran off the panel.

### H. The editing loop, end to end

- [x] Run over an MCP session on 23 Nov 2026 with the assistant as the agent: a face picked in the viewer and the note "pull this face out by 30 cm"; `mesh.context` gave the note and the face's normal; `mesh.commands` gave `extrude`'s fields; `mesh.preview` showed the result with the workspace still at revision 1; `commit: true` moved it to revision 2; the face's vertices read z 0.8, from 0.5; the edit record held the command with the note.
- [x] The same loop is a check: `debug/lighting/scripts/mesh_edit_from_selection.rhai` on `genos-editor` (15 checks). It builds its own workspace under the output directory.
- [x] The loop is also a test that needs no window and no GPU: `a_selection_and_a_note_become_a_previewed_and_committed_edit` in the editor crate drives the mesh viewer and the linked mesh-workbench directly (the selection and the note, `mesh_context`, `mesh_preview`, the commit, the face 0.3 m further out, the edit on record with its note). `cargo test -p genos-editor` runs it.
- [x] Through Claude Code itself: `.mcp.json` at the repository root names the editor's endpoint (`genos-editor`, port 8765) and the lighting program's (`genos-lighting`, 8766), so a Claude Code session in this repository has the tools. Checked on 23 Nov 2026 with a separate `claude -p` session that was given only that config and the task: it called `mesh_context`, `mesh_commands`, `mesh_preview` and the commit, and the selected face moved 0.25 m (z 0.5 to 0.75, revision 1 to 2). Claude Code shows a tool as `mcp__genos-editor__mesh_context`: it writes the dot of `mesh.context` as an underscore.
- [ ] For Anthony to run, on purpose: `debug/lighting/scripts/mesh_pointer_check.sh` drives the real pointer (click a face, drag to orbit, click the projection's name) and prints PASS or FAIL for each. It takes the pointer and the focus for about ten seconds, so an agent does not run it.
- The one real-pointer run on 23 Nov 2026 is not evidence either way: Anthony was moving the mouse while it ran (it took his pointer, and he stopped it). In it a drag turned the camera, and the clicks did not land. Read from the code, a press and a release with no movement between is a click (`CLICK_PIXELS`), on the picture and on the gizmo.
- [x] Found by reading that path: the viewer was told the pointer was over the UI on every frame with no left button held (`over_ui: !look_capture`, and the panel reports `look_capture` only while a press that missed it is held). So with a real mouse the wheel did not zoom and a right drag did not pan; the pointer tests set that flag by hand and passed. `pointer_over_ui` now asks where the pointer is: on something a panel paints. A test covers it.
- [ ] Not yet seen with a real mouse: the wheel zoom and the right-drag pan after that fix, and a click on a face. `mesh_pointer_check.sh` checks the click, the drag and the gizmo; run it with the mouse left alone.

### G. Compared with mesh-workbench's own renderer

A workspace with three objects, an extruded face, box UVs scaled to tile, a texture on one object and painted vertices on another was drawn in each mode by mesh-workbench's `capture` and by the editor, side by side (23 Nov 2026; `genos-debug/cmp/sheet.png` on the machine that ran it).

- [x] `object_ids`, `face_ids`, `collection_ids`: the same colours. The editor used its own palette by ID; it now uses mesh-workbench's (`categorical_color`: the colour of the Nth ID in order). An object's red reads 241, 53, 54 in the editor and 242, 53, 53 in mesh-workbench.
- [x] `uv_checker`: the same checker (eight squares a UV unit, 224 and 48). The editor's was tinted.
- [x] `textured`: the same image and tiling; a face with no UVs is magenta whether or not it has a texture, as in mesh-workbench.
- [x] `vertex_colors`: an unpainted object is white, and paint is read as display values.
- [x] `silhouette`, `orientation` (grey, red from behind), `tessellation` (every triangle edge) and `material` (the texture, lit) match in what they show.
- [x] The unlit modes show exact colours: a debug view is no longer exposed and tone-mapped under the HDR display path (white read 184).
- [x] The shade names are mesh-workbench's: `vertex_colors` is unlit and `vertex_color_shaded` lit; `orientation` and `topology` are accepted for `face_orientation` and `wireframe`.
- [x] Compared with the browser viewer itself: mesh-workbench's `mesh-server` was built from the pinned revision and its page drawn in headless Chromium on the same workspace (`genos-debug/cmp/web-default.png`). Its look is a dark background (23, 26, 32), a dark ground with a faint grid and a soft shadow under the model, and a neutral grey mesh. The mesh scene now has that look: a dark sky and ground, grid lines a little lighter than the ground, the contact shadow, and the same grey (the front of the cube reads about 150 in both).
- [x] Every mode against the browser viewer: a script pressed each of the page's render-mode buttons over the DevTools protocol and saved a picture, and the editor was captured in the same twelve modes from a like angle ([the sheet](img/mesh-viewer-vs-web.png), web viewer on the left of each pair). `shaded`, `topology`, `object_ids`, `face_ids`, `uv_checker`, `textured`, `vertex_colors`, `orientation`, `silhouette`, `normals`, `depth` and `material` show the same thing in both.
- [x] The backdrops are the web viewer's: white for `silhouette`, black for `depth`, its background (23, 26, 32) for the other unlit shades, read back exactly; the grid and the shadow are left out of `silhouette` and `depth`. In the base-colour view the sky is shown as its own colour, not tone-mapped.
- [x] In the lit `material` shade a face with no UVs gives off one flat magenta from every side. The tone curve keeps it from pure magenta: it reads about (160, 60, 160).
- [ ] The browser's `topology` has no ID labels; mesh-workbench's `capture` has them, and so has the editor (`labels: false` hides them).
- [x] The mesh scene is lit as the web viewer lights a mesh: the scene's sun from the web viewer's key direction, and two weaker directional lights from its other two directions at its strengths (`MESH_FILL_LIGHTS`).
- [x] `topology` (the `wireframe` shade) labels each visible face and vertex with its ID, as mesh-workbench's does; `mesh_view labels: false` hides them. Read from a picture of the window.
- [x] Contact shadows that would overlap are one patch: two patches at the same height flickered into rings (seen in the window).
- [x] The sky is the lit ground's colour, so the ground has no visible edge: it fades into the background as the web viewer's does.
- [x] `normals` and `depth` are shades in mesh-workbench's encodings (RGB = (N + 1) / 2; 255 at the nearest shown vertex and 32 at the farthest), compared with its `capture` side by side. The engine's `view` modes `normal` and `depth` remain as they were.
- [x] The contact shadow is a smooth gradient. It showed steps on the dark ground because the G-buffer keeps base colour in 8 linear bits; `gbuffer.frag` now adds an ordered dither of under half a step, fixed to the pixel, so a dark gradient anywhere in the engine has no steps. A colour that is exactly a byte is unchanged, and the render suite fails the same tests with and without it.
- [x] The web viewer's corners are on the picture: the workspace with what is shown and its revision (top), the counts (bottom left), the pointer hints (bottom right), and the axis gizmo (top right). A click on a gizmo end looks from that side, and a click on the projection's name under it changes the projection. Read from a picture of the window.
- Decision: the web viewer's toolbar is the Mesh tab of the engine's menu here (a folder picker, dropdowns, check boxes). The editor stays basic: a person looks, selects and types a note, and an agent edits.

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
