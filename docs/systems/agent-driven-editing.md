# Agent-driven editing (future direction)

Status: **Future direction. Not scheduled. No implementation now.** This note records intent so later design docs point the same way. It adds no backlog items.

## Vision

Genos has no Unreal- or Unity-style editor GUI. A person flies around the running game, looks at or selects something, and says or types what should change. An agent knows what is on screen, what is selected, and the scene around it. It makes the change, can undo it, and saves it to the scene data files. GUIs exist only for players.

## What already points this way

- The MCP live-control server (`crates/mcp`). It reads and edits the scene (`floor`, `wall:<id>`, `solid:<id>`, `light:<id>`) and returns `take_screenshot` and `lighting_status`.
- One command set across MCP, `POST /cmd/<name>`, and Rhai ([debugging](debugging.md)).
- `.rhai` scene files ([scripting](scripting.md)), data-driven settings ([settings principles](../engine/settings-principles.md)), the deterministic scene library in `debug/scenes/`, per-frame metrics, and screenshot plus metrics output.

## Later, when it is scheduled

Each of these needs its own design doc before any code:

- A context packet for each request: camera pose, frame screenshot, the entity under the crosshair or cursor, the visible entities with compact components, and recent edits.
- Pointing: crosshair pick, click, and spoken references such as "that lamp".
- Edit transactions: preview, undo and redo, saving to scene and prefab files, and a record of who or what made each change.
- Verification: the agent takes a new screenshot and checks the metrics after each edit.
- Permissions: dev builds allow inspect and edit on a sliding scale. Release builds lock editing off.
- Any panel is a thin view over these same text commands, never a separate editor.

## Conflicts named

[ADR 0002](../adr/0002-runtime-and-editor-share-one-repository.md) and [ADR 0006](../adr/0006-editor-is-linux-first.md) assume a separate Linux editor program. This direction replaces that editor with agent tooling around the running game. Those ADRs stand until a later ADR supersedes them.

## Principle that applies today

[ADR 0019](../adr/0019-agent-operable.md): every feature must be drivable and verifiable through text interfaces.
