# Mesh workbench

Status: built in stages. In the tree: reading and showing a workspace, the selection and the pick over the agent endpoint, the selection highlight, and the command pass-through. Also in the tree: orbit, pan and zoom, selecting with a click, and mesh-workbench linked in for edits. Objects hide, show and solo; the shade modes are `shaded`, `wireframe`, `vertex_colors`, `object_ids` and `face_ids`; the axis views; replay of a session's recorded states; and menu controls for the pick mode, the shading and which object shows. Animation poses and bones are in too. The engine's `view` modes `normal`, `depth` and `albedo` work on a mesh scene. Clips and replays play. The note is typed in the window. Not yet: the orthographic view, textures, and an objects tree in the window. The stages are listed under Plan.

The scene program shows and edits the meshes that [mesh-workbench](https://github.com/NathanChambers/mesh-workbench) makes. A person looks at a mesh in the engine, selects part of it, and tells an agent what to change. The agent reads the selection from the engine's agent endpoint and sends the edit to mesh-workbench.

## Goal

A mesh-workbench workspace opens as a scene in the editor (`genos-editor`, `make editor` or `make mesh`). The editor runs the same frame loop as `genos-lighting` (the library `genos_scene_program`); it opens on the mesh scene, lists every scene, and serves MCP on port 8765. The picture follows the workspace as the agent edits it. The viewer gives the same experience as the mesh-workbench web viewer: orbit, pan and zoom, the render modes, the objects list, replay, and animation. The viewer also does what that web viewer does not: a person selects an object, faces, or vertices, and the agent can read that selection.

## What mesh-workbench is

Facts from its repository at commit `c88a38fb` (21 Nov 2026). It says its command API and workspace format may change.

- Rust, MIT, crate `gpt-mesh`. Binaries `mesh-server` and `mesh-cli`. Dependencies: `serde`, `serde_json`, `png`, `ufbx`.
- A mesh is an indexed polygon mesh with stable numeric IDs: vertices, n-gon faces, objects (sets of vertex IDs), collections, UVs, textures, vertex colours, an optional rig.
- A workspace is a directory. `project.json` holds `{"version", "mesh", "revision"}` and is replaced atomically after every state-changing command. `sessions/` holds the recorded commands and state snapshots.
- The editor server is `mesh-server --workspace DIR --port N`, loopback only. `POST /command` takes one JSON command or a batch. `GET /state` returns the mesh and the revision. There is no push channel.
- It has no MCP server. Its own reference says so.
- Its web viewer is read-only. It has no picking and no selection highlight. The only selection is a vertex set the agent sets with the `select` commands.

## Intent

The engine links mesh-workbench as a library (ADR 0042) and calls its session to edit. The engine reads the workspace files to draw, so a workspace another process edits is shown too.

The engine's agent endpoint is the one place an agent works. It carries three things:

- The mesh-workbench commands, run on the linked session (`mesh_command`).
- What the person selected in the viewer: the object, the face IDs, the vertex IDs, the point under the cursor, the camera, and a typed note (`mesh_selection`). This state lives in the engine. It is not the mesh-workbench vertex selection, which the agent's own `move`, `scale` and `rotate` use and which is recorded as an undo step.
- The viewer's state: the workspace, the revision shown, the render mode, hidden objects (`mesh_view`).

Any panel is a view over those commands ([agent-driven editing](agent-driven-editing.md)).

The viewer triangulates the n-gons itself, as the web viewer does: a fan for a convex face, ear clipping for a concave one. Normals follow the web viewer's rule (`geometry.js` `buildCornerNormals`): an object with no smooth angle, or a face in `flat_faces`, is flat; with a smooth angle a corner blends the faces around its vertex that are reached across edges no sharper than that angle, weighted by their corner angles. It keeps, for every triangle, the face ID and the object ID it came from, so a pick returns mesh-workbench IDs.

## Plan

1. **Read and show.** `crates/workbench` reads `project.json` into triangles with their face, vertex and object IDs. A scene file of `kind mesh` names a workspace. The scene program draws it on a ground plane under a sun and a sky, and reloads it when `project.json` changes. `mesh_view` reports the workspace, the revision and the counts.
2. **Look.** Orbit, pan and zoom about the mesh, frame an object, the axis snaps, perspective and orthographic.
3. **Select.** A click picks an object, a face, or a vertex. The selection is highlighted. `mesh_selection` reads and sets it, with the note.
4. **Edit.** `mesh_command` runs a command on the linked mesh-workbench session and returns the reply.
5. **Render modes and the objects list.** Wireframe, shaded, vertex colours, textured, and the debug modes (normals, depth, face IDs, object IDs). Show, hide, solo.
6. **Replay and animation.** The session scrubber and the clip player.

## Editing

The engine links mesh-workbench ([ADR 0042](../adr/0042-mesh-workbench-is-linked-into-the-editor.md)). `crates/meshbench` (`genos-meshbench`) wraps its session: `Bridge::open(workspace)`, `execute(command)`, `revision()`. It is the only crate that names `gpt-mesh` or `serde_json`.

Looking at a workspace writes nothing to it. The first `mesh_command` opens the session on the workspace. From then on mesh-workbench holds `project.json`, `workflow.json` and the recordings under `sessions/`, and writes `project.json` after every command that changes the mesh. The viewer follows that file, so the picture shows the edit on the next frame. An edit runs on the frame thread: a long command stalls the frame until it returns.

Until 22 Nov 2026 the scene program started `mesh-server` as a child process and posted commands over loopback HTTP (`--mesh-server`, `--mesh-port`). Those arguments are gone.

## Pointer

In a mesh scene the fly camera is off and the pointer works on the mesh:

- Left drag orbits about the point in front of the camera. Right drag pans. The wheel zooms. `F` frames the whole mesh.
- A click (a press that moves under 4 pixels) selects what is under the pointer and clears the rest. Shift and a click adds it, or removes it when it was selected.
- What a click selects is the pick mode: `face` (default), `object`, or `vertex`. `mesh_view` with `pick` sets it, and its reply reports it. The knob `mesh_pick` is the same value, and the menu's Scene tab shows it as "Click selects".
- A press on the menu is the menu's.
- `T` starts typing the note. Letters, digits and the space bar type into it (Shift gives capitals), Backspace removes a letter, Enter ends, and Escape ends and clears it. While typing, the hotkeys are off. The Scene tab shows the note. An agent reads it with `mesh_selection`.

When a checkpoint, a script, or an agent moves the camera, the orbit continues about the point in front of that new pose.

## Commands

On the agent endpoint (MCP, `POST /cmd/<name>`, and Rhai), while a mesh scene is open:

The names below are the script names. Over MCP and HTTP each is published as `mesh.<name>` (`mesh.view` for `mesh_view`); the script name is a deprecated alias there ([debugging](debugging.md)). Each tool publishes a JSON schema of its arguments (`mesh_schema`): the viewer's tools name every field, its type and its choices; a family tool requires `command` and leaves mesh-workbench's own fields open, because mesh-workbench publishes no schema for them.

- `mesh_view`: the workspace directory, the revision shown, the counts, the objects (id, name, origin, triangles, hidden), `pick`, `shade`, the server, and `lift`. `hide`, `show` and `solo` take object IDs and `show_all: true` shows every object. Hiding is local to the viewer, as in the web viewer. `shade` is `shaded`, `wireframe` (the polygon edges over the shaded mesh), `vertex_colors` (mesh-workbench's painted vertex colours; an object with none keeps the plain colour), `object_ids` or `face_ids` (a steady colour per ID), `collection_ids`, `silhouette` (the mesh in black), `textured` (each face's texture, tinted by the vertex paint), `uv_checker` (a checker through the face UVs; U runs to red and V to green), or `face_orientation` (blue seen from the front, red from behind). `look` is `+x`, `-x`, `+y`, `-y`, `+z`, `-z` (the side the camera stands on) or `frame`. `replay: {session, event}` shows the recorded state after that command of a session (`session` defaults to `latest`, event 0 is the session start), and `replay: "live"` returns to the live mesh. `replay: "next"` and `"previous"` step one command; stepping past the last command is the live mesh. While a state is replayed the picture does not follow `project.json`. `grid: false` hides the ground grid (a line every power of ten of metres that fits the mesh, 1 m for a mesh a few metres across). `turntable: true` turns the camera about the mesh, one turn in about 20 seconds; a press takes the camera back for that frame. `pose: {clip, time}` draws the mesh at that time of an animation clip, and `pose: "rest"` returns to rest. The posed vertices and joints come from mesh-workbench's `animation_sample`, 256 a page, so a pose opens the session and each pose is recorded there as commands. `pose: {clip, play: true}` plays the clip in a loop and `pose: "stop"` holds it. Playing samples the clip once, 12 poses a second of clip and at most 48, and blends between them, so a drawn frame does not record a command. `replay: "play"` steps through the session every 0.7 s and stops at the live mesh; `replay: "stop"` holds it. `solo_match` and `hide_match` take text and show only, or hide, the objects whose name contains it. The reply lists `collections` (id, name, objects); `solo_collection` and `hide_collection` take a collection ID, and `shade: collection_ids` colours each object by the first collection that holds it. `event: N` returns what command N of the session sent and its compact reply. `bones: true` draws each joint and a line to its parent, posed with the mesh. `surfaces: false` hides the mesh's surfaces, so the bones and the wireframe show. The reply lists `clips`, `pose` and `rig_bones`. A reload samples the pose again. The reply lists the sessions and their latest commands. The knobs `mesh_shade`, `mesh_solo`, `mesh_turntable` and `mesh_replay` are the menu's "Mesh shading", "Show object", "Turntable" and "Replay" (a slider: 0 is the session start, each step a command, the last step live). A mesh with clips also gets `mesh_clip` ("Clip": rest or a clip), `mesh_time` ("Time", in hundredths of the clip), `mesh_bones` and `mesh_surfaces`. The Scene tab also has an "Objects" section (collections and objects, each with a show box) and a "Replay" section (a session picker and the latest commands, each with a box that shows the mesh after it). `filter` is the text that narrows both; in the window Shift and T types it. `projection` is `perspective` or `orthographic` (the menu's "Orthographic"): the orthographic view is a 3 degree lens from far back that keeps the framing, not a parallel projection, so parallel edges draw within 2% of parallel. `grid` and `shadow` (the menu's "Grid" and "Contact shadow") turn the ground grid and the contact shadow on or off; the shadow is a soft dark patch drawn under each shown object, not a traced shadow. The picture raises the mesh by `lift` metres so its lowest point stands on the ground. Every position a command reports is in mesh coordinates.
- `mesh_pick`: the mesh under screen `[x, y]`. `mode` is `face`, `object`, or `vertex`. `select` is `set`, `add`, or `none`. The reply names the object, the face, the face's vertex IDs, the nearest vertex, and the point.
- `mesh_selection`: the selection (objects, faces, vertices, the last picked point, the note, the revision, the camera). `objects`, `faces` and `vertices` replace that part, `note` sets the note, `clear: true` empties it first. IDs that an edit removes leave the selection at the next reload.
- `mesh_command`: `command` is one mesh-workbench command object or a list of up to 1024. A list stops at the first command mesh-workbench refuses and replies `{ok, failed_command, replies}`. The reply is mesh-workbench's reply.

- The family tools `mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material`, `mesh_rig`, `mesh_animation`, `mesh_query` and `mesh_document` each take one mesh-workbench command object (`{"command": "rig_set", ...}`) and refuse a command of another family, naming its tool. The family is read from the command's name (`family_of`). `mesh_help` is mesh-workbench's help: `topic`, `all`, `offset`, `limit`.

The agent's loop: read `mesh_selection`, send `mesh_command`, and the picture reloads when `project.json` changes.

Checked on 22 Nov 2026 with mesh-workbench `c88a38fb`: on an empty workspace, two `object_create_box` commands through `mesh_command` made a crate and a lid, the viewer showed revision 2 with both objects, and a pick on the lid returned object 2, face 8, vertices 13 to 16. `debug/lighting/scripts/mesh_selection.rhai` checks the pick and the selection on the sample workspace without a server.

## Code

- `crates/workbench`, package `genos-workbench`: `load_workspace`, `Workspace`, `Part`, `triangulate`. It depends on `genos-json` only. It does not open a window or a device.
- `debug/scene-lib`: `Kind::Mesh { workspace }`.
- `crates/scene-program` (`genos-scene-program`), the frame loop `genos-editor` and `genos-lighting` run: `mesh_world` builds the world for a mesh scene and `follow_mesh` reloads it. `src/mesh_view.rs` holds the selection, the pick, the pointer, the highlight and wireframe triangles, replay, and the bridge.
- `crates/meshbench`, package `genos-meshbench`: the bridge to the linked mesh-workbench session. Only `editor/` (`genos-editor`) depends on it; it passes the scene program the session opener (`Program::mesh_session`, the `MeshSession` trait in `crates/scene-program`).
- `crates/debug`: `Host::scene_command` carries the `mesh_*` commands to the scene program; `commands.rs` lists them.

## Game use

None. This is authoring. A finished mesh leaves mesh-workbench as a GLB and enters a game through the asset path ([asset loading](asset-loading.md)).

## Limits

- The picture is the engine's raster and direct light. The mesh has no baked distance field, so it does not occlude bounce light.
- The orthographic view stands the camera up to 120 m back (the far plane is 200 m); a mesh too large for that gets a wider lens. Reflections and the reference tracer assume the 60 degree lens.
- The `textured` and `uv_checker` shades are lit, where mesh-workbench draws them unlit. A corner with no UV draws at the image's corner, not magenta. A part with several textures holds a tiling UV at the image's edge. `alpha_cutoff` is not read. A pose keeps the rest normals. Picking and the selection highlight follow the pose.
- The workspace format may change upstream. `load_workspace` reads versions 1 to 8 and refuses a newer one.
- Nothing is pushed: the viewer polls the file's modification time.

## Networking

- Replicated state: none. A workspace is local authoring data.
- Authority: the mesh-workbench session in the authoring program owns the mesh. The engine owns the selection and the view.
- Prediction / rollback: none.
- Determinism: the triangulation of a given `project.json` is the same on every run.
- Headless server: not involved. A headless scene program can still load a mesh scene and answer `mesh_view`.

## Decisions

- This adds a viewer panel to the scene program. [ADR 0019 agent-operable](../adr/0019-agent-operable.md) rejects editor GUI panels as the way to edit. The panel here is a view and a pointer: every edit is an agent command. ADR 0002 and ADR 0006 describe a separate Linux editor program; this work does not start one.
- [ADR 0020](../adr/0020-asset-pipeline-import-cook-stream.md) says there is no asset browser GUI. The objects list is a list of the parts of one open mesh, not a browser of assets.
- [ADR 0042](../adr/0042-mesh-workbench-is-linked-into-the-editor.md): mesh-workbench is a pinned git dependency of `genos-meshbench` only. It owns none of the five things ADR 0007 keeps native.
