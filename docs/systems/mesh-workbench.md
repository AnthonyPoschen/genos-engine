# Mesh workbench

Status: built in stages. In the tree: reading and showing a workspace, the selection and the pick over the agent endpoint, the selection highlight, and the command pass-through. Also in the tree: orbit, pan and zoom, selecting with a click, and starting `mesh-server` with the scene. Not yet: the axis snaps and the orthographic view, render modes, the objects list, replay, animation. The stages are listed under Plan.

The scene program shows and edits the meshes that [mesh-workbench](https://github.com/NathanChambers/mesh-workbench) makes. A person looks at a mesh in the engine, selects part of it, and tells an agent what to change. The agent reads the selection from the engine's agent endpoint and sends the edit to mesh-workbench.

## Goal

A mesh-workbench workspace opens as a scene in the scene program (`genos-lighting`). The picture follows the workspace as the agent edits it. The viewer gives the same experience as the mesh-workbench web viewer: orbit, pan and zoom, the render modes, the objects list, replay, and animation. The viewer also does what that web viewer does not: a person selects an object, faces, or vertices, and the agent can read that selection.

## What mesh-workbench is

Facts from its repository at commit `c88a38fb` (21 Nov 2026). It says its command API and workspace format may change.

- Rust, MIT, crate `gpt-mesh`. Binaries `mesh-server` and `mesh-cli`. Dependencies: `serde`, `serde_json`, `png`, `ufbx`.
- A mesh is an indexed polygon mesh with stable numeric IDs: vertices, n-gon faces, objects (sets of vertex IDs), collections, UVs, textures, vertex colours, an optional rig.
- A workspace is a directory. `project.json` holds `{"version", "mesh", "revision"}` and is replaced atomically after every state-changing command. `sessions/` holds the recorded commands and state snapshots.
- The editor server is `mesh-server --workspace DIR --port N`, loopback only. `POST /command` takes one JSON command or a batch. `GET /state` returns the mesh and the revision. There is no push channel.
- It has no MCP server. Its own reference says so.
- Its web viewer is read-only. It has no picking and no selection highlight. The only selection is a vertex set the agent sets with the `select` commands.

## Intent

mesh-workbench stays a separate program. The engine does not link it: its dependencies are third-party crates the engine does not carry, and its API is days old. The engine reads the workspace files to draw, and talks to `mesh-server` over loopback HTTP to edit.

The engine's agent endpoint is the one place an agent works. It carries three things:

- The mesh-workbench commands, passed through to `mesh-server` (`mesh_command`).
- What the person selected in the viewer: the object, the face IDs, the vertex IDs, the point under the cursor, the camera, and a typed note (`mesh_selection`). This state lives in the engine. It is not the mesh-workbench vertex selection, which the agent's own `move`, `scale` and `rotate` use and which is recorded as an undo step.
- The viewer's state: the workspace, the revision shown, the render mode, hidden objects (`mesh_view`).

Any panel is a view over those commands ([agent-driven editing](agent-driven-editing.md)).

The viewer triangulates the n-gons itself, as the web viewer does: a fan for a convex face, ear clipping for a concave one. It keeps, for every triangle, the face ID and the object ID it came from, so a pick returns mesh-workbench IDs.

## Plan

1. **Read and show.** `crates/workbench` reads `project.json` into triangles with their face, vertex and object IDs. A scene file of `kind mesh` names a workspace. The scene program draws it on a ground plane under a sun and a sky, and reloads it when `project.json` changes. `mesh_view` reports the workspace, the revision and the counts.
2. **Look.** Orbit, pan and zoom about the mesh, frame an object, the axis snaps, perspective and orthographic.
3. **Select.** A click picks an object, a face, or a vertex. The selection is highlighted. `mesh_selection` reads and sets it, with the note.
4. **Edit.** The scene program starts `mesh-server` on the workspace when it is on the path. `mesh_command` passes a command through and returns the reply.
5. **Render modes and the objects list.** Wireframe, shaded, vertex colours, textured, and the debug modes (normals, depth, face IDs, object IDs). Show, hide, solo.
6. **Replay and animation.** The session scrubber and the clip player.

## Server

When a mesh scene opens, the scene program starts `mesh-server --workspace DIR --port N` on that workspace and stops it when the scene closes or the program quits. `--mesh-server PATH` (`GENOS_MESH_SERVER`) names the program; the default is `mesh-server` on the path, and `off` starts none. `--mesh-port N` (`GENOS_MESH_PORT`) is the port, default 7879. When a server already answers on the port, none is started. When the program is missing, the viewer still shows the workspace and `mesh_view` reports why in `server`. A workspace with no `project.json` yet is an empty workspace: the server writes the file on the first edit and the viewer picks it up.

Install mesh-workbench with `cargo install --path . --locked` in its checkout.

## Pointer

In a mesh scene the fly camera is off and the pointer works on the mesh:

- Left drag orbits about the point in front of the camera. Right drag pans. The wheel zooms. `F` frames the whole mesh.
- A click (a press that moves under 4 pixels) selects what is under the pointer and clears the rest. Shift and a click adds it, or removes it when it was selected.
- What a click selects is the pick mode: `face` (default), `object`, or `vertex`. `mesh_view` with `pick` sets it, and its reply reports it. The knob `mesh_pick` is the same value, and the menu's Scene tab shows it as "Click selects".
- A press on the menu is the menu's.

When a checkpoint, a script, or an agent moves the camera, the orbit continues about the point in front of that new pose.

## Commands

On the agent endpoint (MCP, `POST /cmd/<name>`, and Rhai), while a mesh scene is open:

- `mesh_view`: the workspace directory, the revision shown, the counts, the objects (id, name, origin, triangles), the server port, and `lift`. The picture raises the mesh by `lift` metres so its lowest point stands on the ground. Every position a command reports is in mesh coordinates.
- `mesh_pick`: the mesh under screen `[x, y]`. `mode` is `face`, `object`, or `vertex`. `select` is `set`, `add`, or `none`. The reply names the object, the face, the face's vertex IDs, the nearest vertex, and the point.
- `mesh_selection`: the selection (objects, faces, vertices, the last picked point, the note, the revision, the camera). `objects`, `faces` and `vertices` replace that part, `note` sets the note, `clear: true` empties it first. IDs that an edit removes leave the selection at the next reload.
- `mesh_command`: `command` is one mesh-workbench command object or a list. It goes to `POST /command` on `mesh-server` at `port` (default `GENOS_MESH_PORT`, else 7879). The reply is mesh-workbench's reply. With no server the error says how to start one.

The agent's loop: read `mesh_selection`, send `mesh_command`, and the picture reloads when `project.json` changes.

Checked on 22 Nov 2026 with mesh-workbench `c88a38fb`: on an empty workspace, two `object_create_box` commands through `mesh_command` made a crate and a lid, the viewer showed revision 2 with both objects, and a pick on the lid returned object 2, face 8, vertices 13 to 16. `debug/lighting/scripts/mesh_selection.rhai` checks the pick and the selection on the sample workspace without a server.

## Code

- `crates/workbench`, package `genos-workbench`: `load_workspace`, `Workspace`, `Part`, `triangulate`. It depends on `genos-json` only. It does not open a window or a device.
- `debug/scene-lib`: `Kind::Mesh { workspace }`.
- `debug/lighting`: `mesh_world` builds the world for a mesh scene and `follow_mesh` reloads it. `src/mesh_view.rs` holds the selection, the pick, the highlight triangles, and the loopback HTTP client.
- `crates/debug`: `Host::scene_command` carries the `mesh_*` commands to the scene program; `commands.rs` lists them.

## Game use

None. This is authoring. A finished mesh leaves mesh-workbench as a GLB and enters a game through the asset path ([asset loading](asset-loading.md)).

## Limits

- The picture is the engine's raster and direct light. The mesh has no baked distance field, so it does not occlude bounce light.
- Textures, the rig, and animation are not drawn until stage 5 and stage 6.
- The workspace format may change upstream. `load_workspace` reads versions 1 to 8 and refuses a newer one.
- Nothing is pushed: the viewer polls the file's modification time.

## Networking

- Replicated state: none. A workspace is local authoring data.
- Authority: the `mesh-server` process owns the mesh. The engine owns the selection and the view.
- Prediction / rollback: none.
- Determinism: the triangulation of a given `project.json` is the same on every run.
- Headless server: not involved. A headless scene program can still load a mesh scene and answer `mesh_view`.

## Decisions

- This adds a viewer panel to the scene program. [ADR 0019 agent-operable](../adr/0019-agent-operable.md) rejects editor GUI panels as the way to edit. The panel here is a view and a pointer: every edit is an agent command. ADR 0002 and ADR 0006 describe a separate Linux editor program; this work does not start one.
- [ADR 0020](../adr/0020-asset-pipeline-import-cook-stream.md) says there is no asset browser GUI. The objects list is a list of the parts of one open mesh, not a browser of assets.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md): the loopback HTTP client is engine code on the standard library's sockets.
