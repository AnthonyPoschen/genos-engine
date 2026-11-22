# Editor status

Where the editor stands on 23 Nov 2026 (branch `lighting-v3`), and what is missing. Work is on hold at this point. The design is in [mesh workbench](mesh-workbench.md). The item-by-item record, with the evidence for each, is the [editor checklist](editor-checklist.md).

## What it is

`genos-editor` (`make editor`) is the scene program opened on the mesh scene. A person looks at a mesh-workbench workspace, selects parts of it, and types a note that says what they want. An agent reads that over MCP and makes the edit with mesh-workbench, which is linked into the editor ([ADR 0042](../adr/0042-mesh-workbench-is-linked-into-the-editor.md)). `genos-lighting` (`make stress`) is the same frame loop on the lighting scenes, with no mesh viewer.

## What works

**Scenes.** Scene files in `debug/scenes` open by name. The old stress scene is `lighting-stress`; `stress` still opens it. `--scenes DIR` adds another directory's scene files.

**Viewing a mesh.**

- A workspace opens as the `mesh` scene, and the picture follows `project.json` as it changes.
- Orbit, pan, zoom, frame (`F`), axis views, a turntable, and a perspective or orthographic (parallel) projection.
- Fifteen shades, named as mesh-workbench names its render modes: `shaded`, `textured`, `material`, `wireframe`, `uv_checker`, `vertex_colors`, `vertex_color_shaded`, `object_ids`, `face_ids`, `collection_ids`, `face_orientation`, `tessellation`, `normals`, `depth`, `silhouette`.
- The look of the web viewer: its three lights, a dark ground that fades into a dark background, a faint grid, a contact shadow.
- Hide, show and solo objects; collections; replay of recorded sessions; animation clips with bones; ID labels in the wireframe shade.
- On the picture: the workspace line, the counts, the pointer hints, an axis gizmo with the projection toggle, and an Objects and Replay panel.
- A workspace picker. A directory with no `project.json` opens as an empty workspace.

**Selecting.** A click selects a face, an object or a vertex; Shift adds. Control and a drag draws a box, Alt and a drag a lasso. `T` types the note.

**Editing through MCP.** The tools are in six sections (scene, lighting, render, capture, editor, mesh); 16 are mesh tools. A Claude Code session sees 90 tools in all.

- `mesh_context` gives one packet: the note, the selection, each selected face's vertices, centre and normal, the camera, the objects in view, the latest edits, and a screenshot.
- `mesh_commands` lists mesh-workbench's 143 commands with their fields, read from the linked version's help.
- `mesh_preview` shows an edit before it is committed, then commits or discards it.
- A tool for each command family (`mesh_object`, `mesh_edit`, `mesh_select`, `mesh_material`, `mesh_rig`, `mesh_animation`, `mesh_query`, `mesh_document`), and `mesh_command` for any command or a batch.
- `.mcp.json` at the repository root gives a Claude Code session in this repository these tools while the editor runs.

## How it was checked

- **Against the web viewer.** mesh-workbench's browser viewer was drawn in headless Chromium in twelve modes and the editor captured in the same modes: [the sheet](img/mesh-viewer-vs-web.png). The same was done against mesh-workbench's `capture` renderer. An object's ID colour reads 241, 53, 54 in the editor and 242, 53, 53 in mesh-workbench.
- **The editing loop.** A separate Claude Code session, given only `.mcp.json` and the task, read a selection and a note, previewed an extrude and committed it. `cargo test -p genos-editor` runs the same loop with no window. `debug/lighting/scripts/mesh_edit_from_selection.rhai` runs it on the running editor.
- **Tests.** 34 in the viewer (`cargo test -p genos-scene-program --features mesh`), 2 in the editor crate, 6 in the reader.

## Missing or not confirmed

**Not confirmed with a real mouse.** These are covered by tests that feed the pointer by hand, and by nothing else:

- A click on a face selects it.
- The wheel zooms and a right drag pans. Both were broken until the last pointer fix (`pointer_over_ui`), and the fix has not been seen in the window.
- Box and lasso drags, and clicks on the gizmo.

`debug/lighting/scripts/mesh_pointer_check.sh` checks the click, the drag and the gizmo with the desktop's own pointer. It takes the pointer and the focus for about ten seconds, so a person runs it on purpose.

**Not done.**

- The editing loop has not been run from a person's own Claude chat, only from a scripted session and the tests.
- 72 of 518 command field names have no type in `mesh_commands`. Many are not fields: the table parser also picks up a blend mode's values and the names of reply fields. A complete list needs a machine-readable command list from mesh-workbench.
- "Select connected" and "select loop" (`mesh_selection grow`) have no key in the window.
- The Objects panel lists at most 200 lines.
- The typed note reads a US keyboard layout only.
- The mesh casts no traced shadow: it has no distance field. The contact shadow is drawn, and does not follow the light.
- The picture is re-read from `project.json` after each command. That is by decision: one read path serves a session in this program, another process's workspace, a replay and a preview.

**Different from the web viewer, by decision.**

- Its toolbar is the Mesh tab of the engine's menu.
- The wireframe shade has ID labels, as mesh-workbench's `capture` has. The browser viewer has none.

**Limits of what is built.**

- A build of the whole workspace has the mesh viewer in `genos-lighting` too. Only a build of that program alone leaves it out, because Cargo gives a shared crate one set of features.
- In the orthographic view the fog, particles and reflections are not drawn, and the reference tracer refuses the view.
- The window list of shades shows six rows and scrolls.
- Every launch of the editor opens a window that takes the pointer on a Hyprland desktop with no no-focus rule for `genos-editor`. The rule in use covers `genos-camera` only.

## Open in the lighting engine

Found while this work was done, and not fixed:

- `a_bright_lamp_does_not_cross_an_opaque_wall` fails with the doubled voxel sizes ([sparse cascade](lighting/sparse-radiance-cascades.md), Sizes).
- `bounce_counts_brighten_the_shadow_and_skip_work` fails on this branch.
- `still_gi_pops.rhai` reads high on the scene program because of auto exposure; the script should fix the exposure.
- The `legacy_shader` check of `hdr_output_gate` fails, with and without this work.

## Asked of mesh-workbench

The list is in the checklist, under "For Nathan". The ones that matter most here:

- A machine-readable command list: each command's name, family, fields and types.
- The prepared transaction's mesh, so a preview is not made by running the commands again on a copy.
- A user selection kept apart from the tool selection, and a change notification in place of polling.

## To pick the work up

1. Run `make editor`. Click a face, scroll, right-drag. Fix what fails from the code.
2. Start a Claude Code session in this repository with the editor running, select a face, type a note, and ask for the edit.
3. Then the "Not done" list above, in the order that matters to the game.
