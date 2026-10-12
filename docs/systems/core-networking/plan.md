# Entity and networking co-design

> **For agentic workers:** Do not start N1. D1–D49 are unanswered. The datagram bytes are the replay file and the live-edit path, so building N7 before those decisions are answered locks both. When they are answered, implement one step per commit, in order. Do not start the next step in the same commit. Each step has a deterministic headless test. Check the box when that test passes.

**Goal:** While a game is running in development, an author or an agent can inspect any entity and change it. Mods are a first-class way to change the game: data and sandboxed Rhai on the authority, the same bytes verified on every player runtime. Lockdown removes the debug tool from a release player build and does not remove the mod loader. The authority applies every change, the entity log records it, undo sends the inverse through that same path, and a save can write the result back to a sidecar or a prefab file. Every tool call goes through the same capability check. Interest limits what a player runtime can see.

**Architecture:** The simulation store is an entity id, a human-readable name, and plain components, updated by ordered tick passes. That store is what an agent queries and what the authority replicates. The authority runtime (the process Genos hosts) is the only writer of replicated state. A tool does not write a column. It sends a notice on an agent connection, and the authority applies it on a tick boundary. A player runtime predicts only the entity it has input authority for, and reconciles by restoring that entity and replaying its commands. Remote entities interpolate. Snapshots are unreliable deltas against the last acknowledged snapshot. A spatial grid and a byte budget decide who is in a player packet. They do not decide what an agent can read. A replay stores that stream. Seeking it is how an agent time-travels.

**Tech stack:** Rust, `std::net::UdpSocket` at the socket edge, no extra networking crate. `genos-math` stays `f32` and stays out of the replicated bytes. ADR 0007 forbids a crate that owns the network. ADR 0009 forbids a script that owns the network.

Written against `gi-v2` at `e61b89d`. This file does not add a crate, does not open a socket, and does not write a replay file.

## Agent observability and control

This is the test for every choice below, including the entity store. An agent has to manage entities and see what they are doing. Replication, prediction, interest, and replay stay, and they earn their place by how well they serve that.

The owner said an agent is another client or tool, with the same authority rules as network clients. In this repo the person is the player, and sources say "client" for the program that person runs. The plan's word is an **agent connection**: a connection with no person. It submits the same notices any connection submits. The authority applies them. The camera's agent endpoint (`crates/mcp`, loopback, scene handles such as `floor` and `wall:<id>`) stays the editor of the live scene. `CONTEXT.md` says to avoid calling a scene handle an entity id, and to avoid calling that endpoint a game network. Both avoids stay. An entity name is not a scene handle. The session tool is not that endpoint. `docs/systems/scene.md` already subscribes an agent to `genos://scene` for scene and camera edits. That subscription is not the entity log.

### What the agent can do

**Name.** The agent refers to an entity by a stable human-readable name, and by the numeric id when it already has one. The numeric id is `{ index: u32, generation: u32 }`, the same bits as the physics plan's `BodyId`, so a stale id cannot resurrect a despawned entity. The name is a component. D20–D22 are the open choices about whether the name is the wire id, how long it stays unique, and what happens when the caller omits it. The recommendation is both: the wire sorts and replicates the numeric id, and the agent may pass either the name or `index.generation`.

**Query.** By type, by tag, by region, and by component. Type and tag are components, so those queries are column scans. Region is an AABB in millimeters. The 32 m cell is only a fast reject inside that box, the same cell size interest uses for packets. D23 is whether the first tool also takes field predicates (`health < 10`) or only presence.

**Inspect.** One entity, every component, at the live tick or at a replay tick. Full state is the columns on that id. It is not a player snapshot, and it is not a scene handle's wall struct.

**Recent history.** An action and event log on the entity. "Action" in `CONTEXT.md` is already the named input intent, so this plan calls the record the **entity log**. A line is a command, a notice, or a component write, with the tick. The live ring is bounded. The replay holds the whole session. D24 is the live length.

**Subscribe.** The agent registers a filter (name, type, tag, region, or component) and receives a notice when a matching entity changes. Polling with query still works. D26 chooses subscribe, poll, or both. The recommendation is both, with subscribe on the reliable notice path, so there is no third channel.

**Causality.** Every write stores a **cause**: the tick pass that wrote it, the command that fed that pass, the notice that requested it, or the snapshot tick that a reconcile copied. The owner said "which system". A system in this repo is a system record. The cause names the tick pass. One hop is stored on the write. When the notice or the command names another entity, the log line names that id too. A general graph walk is not the first tool. D25 is whether the cause is only in authority memory, on every player packet, or on the authority recording and the agent connection.

**Control.** Spawn, edit, and despawn go through an MCP or CLI API. The API enqueues a notice. It does not call `set_pose` on the store. The authority applies the notice at the end of the tick, the same place queued spawns already land, and the datagram is in the replay. A player connection's spawn request is refused. Whether an agent connection's spawn request is accepted is D29. The recommendation is that the authority accepts it from an agent connection and still refuses it from a player connection. That is the same rule for both: the connection does not write state, and the authority decides who may request a spawn. D27 is whether this API is bolted onto the camera endpoint or is a separate session tool.

**Time-travel.** The agent seeks a replay to a tick and then uses the same query, inspect, and log calls. Seek is the keyframe-then-forward path in D4. The picture is the applied stream (D2), so time-travel works when this binary's sim would not resim the file. `--resim` remains the check. D28 is whether the agent may also edit that tick and fork a new session. The recommendation is read-only. Forking is a second authority. Live undo is not a seek. It is an inverse notice, below.

### Live edit and save-back

This is the gameplay-development workflow. The session keeps ticking. An author (the person making the game) and an agent both use the tool connection. Neither writes a column. Inspect reads the authority columns for one name: every component, the pin, and the entity log. A change is a notice. The authority queues it to the end of the tick, checks the generation, applies it or refuses it, and replicates the result. A stale id changes nothing and the log records the refusal. A predicted pawn reconciles the way it reconciles any other correction.

**Undo** is one stack of inverse notices on the authority, shared by every tool connection. Undo enqueues the inverse. Redo enqueues the forward notice again. Both are logged. The stack stores tool notices only. A tick pass and a player command are not undone by this stack, because undoing them would rewind the running game. That rewind is the replay seek, which stays read-only (D28). Depth and despawn identity are D31.

**A pin** is how a live tweak survives the next tick. Without it, the motion pass writes the pose again and the prop snaps back. The pin is a component, so the predicting runtime skips the same fields and does not false-correct every tick. Which fields pin by default is D30.

**Save-back** is optional and explicit. The edit is already live before anyone saves. Save copies the chosen columns at that tick and writes a file from the authority process. A failed write leaves the sim as it was. Players do not receive a file-write datagram, because only the authority has the disk. Undo reverts the live entity and does not, by recommendation, delete the file. D32–D35 are the open choices.

The scene file today is a Rhai script (`examples/camera/scene.rhai`, loaded by `crates/scene/src/script.rs`) of `floor`, `wall`, `solid`, and `light` calls, comments included. There is no prefab in the tree. The recommendation does not rewrite that script. A sidecar JSON, written with `genos-json` and loaded after the script, overlays entities by name. A prefab is the same schema for one template. Spawning it is a notice. The file is data. A tick pass does not eval Rhai to apply it. ADR 0009 stays: a script does not own the network, and a spawn does not run a script.

`examples/camera` still does not join a session. The author talks to the authority process through the session tool. The camera endpoint keeps editing scene handles and does not gain this save path.

### Lockdown

The inspect and edit tools are for development. A shipped multiplayer build has to be able to turn them off. The owner described that as build profiles, and as server-authoritative multiplayer where clients never directly edit authoritative entities. In this repo the authority runtime is the program sources call a server, and the player runtime is the program sources call a client. Genos, the other repository, is a host that starts the authority process. It is not a second writer.

**One surface.** Query, inspect, subscribe, spawn, edit, despawn, pin, undo, save, and seek are the same functions in dev, staging, and release. Each call enters `allow(connection, capability, target)`. The capability is compiled in or absent. The permission is a grant the authority assigned. A player datagram that asks for an edit is the same function a tool calls, and the grant refuses it. There is no second admin protocol and no client-side edit path.

**Profiles.** `dev`, `staging`, and `release`. D37 is whether a profile compiles the tool out of a binary, disables it at run time, or both. The recommendation is both, split by binary. The player binary compiles the tool API out in staging and release, so a patched player has no edit function to call. The authority binary keeps the functions and starts with an empty tool grant, so a host can turn inspect on without shipping a different protocol. Dev compiles the tool in and grants it on loopback.

**Players.** A player connection may send commands for the entity it has input authority for. It does not apply those commands to authoritative state. The authority does. Prediction of the owned pawn stays local and is corrected by the snapshot. A player packet is the interest set and the pose budget, and nothing beyond that: no entity log, no cause, no omniscient checksum, no query of the authority store. That is the anti-cheat exposure. D41 names the bytes. D11's full-sim hash stays on the authority recording. It does not ride the player snapshot.

**Admin.** Optional. Authenticated. Permissioned. Audit-logged. Executed on the authority only. A GM uses this grant. The player runtime does not apply an admin edit locally and then hope the authority agrees. The authority applies it, or refuses it, and the snapshot is what players see. D39 is the credential this repo checks. This repo does not grow an account system. Genos can hold the secret and pass it when it starts the process.

**Operators.** A host such as Genos may be granted inspect, and not edit, unless an admin grant is also configured. Inspect is the same `inspect` function, with that grant. Every inspect and every refusal is an audit line. The audit log is not replicated to players.

**The role byte is a request.** D29's join role does not grant anything by itself. The authority assigns the grant after the check. A player who sends the tool role is still a player.

**Lockdown does not remove mods.** The tool API and the mod loader are different. Staging and release player binaries compile out inspect, edit, undo, and save. They keep the loader that receives the mod manifest, checks hashes, and applies asset and prefab bytes. The authority keeps the loader in every profile, including release, and runs sanctioned scripts there. A mod author gets the tool API by running a dev build. A mod script cannot grant that tool to itself.

### Mods

Modding is a standing goal, not a layer added after the session. `CONTEXT.md` already defines a mod: scripts and data that change a game without a change to the engine. A native library is not a mod. ADR 0009 is the language decision: Rhai, the checked engine, registered functions only. The `unchecked` feature stays rejected. A script does not own the window, the input, the renderer, the audio, or the network.

Cheating and modding are different. Cheating is a player runtime writing authoritative state, calling the tool API, or running bytes that are not in the loaded manifest. Grants, compile-out, and the manifest hash block that. Modding is a change the authority loaded on purpose. Those changes are welcome. They go through the same notice path as a tool edit, so the tick, the log, the snapshot, and the replay see them.

**Data.** Entities, prefabs, and scenes stay data. A prefab is the JSON schema from D33. A scene overlay is a sidecar fragment keyed by entity name. A mod may add a prefab, replace one, or add an entity to the scene. It does not add a Rust component column. Extra fields are a named property map the script can read. Built-in components stay the typed columns. Asset overrides replace the bytes behind a logical path (`meshes/door.obj`, a PNG, a WAV). `genos-load` still decodes those bytes. A network payload and a file already share those decoders (`docs/systems/loading.md`). The mod loader only chooses which bytes.

**Scripts.** A mod script runs in the checked Rhai engine, on the authority, inside one mod pass. It calls registered functions. Those functions enqueue notices. They do not write a column and they do not open a socket. A fuel limit stops a runaway script, which is the checked build ADR 0009 already requires. The player runtime does not run that pass. It renders the snapshot and plays the assets. A client-realm script may drive presentation. It cannot enqueue an authoritative edit. Factorio runs the same mod code on every peer because the simulation is lockstep ([Factorio modding](https://wiki.factorio.com/Tutorial:Modding)). This plan does not copy the lockstep. It copies the rule that a peer with different mod bytes does not enter the session. Garry's Mod splits a server realm from a client realm. The authority pass is the server realm. Presentation is the client realm.

**Order.** A manifest lists mods and their dependencies (`before`, `after`). The loader sorts that list. Two mods that provide the same prefab name, the same scene key, the same asset path, or the same script hook are a conflict. If the later mod declares `overrides` for that key, it wins and the session report names both mods. If neither declares it, loading fails and names both mods and the key. The failure happens before the first tick.

**Distribution.** The authority hashes the canonical manifest and every file. A joining player runtime sends its manifest hash. On a match, the tick may start. On a mismatch, the authority sends the missing files, the player runtime checks each file hash and then the manifest hash, and a failed check never ticks. The gameplay datagram stays the entity stream. File bytes are a separate reliable transfer, still on `std` sockets, still not a crate that owns the network. Genos hosts this by pointing the authority at a directory. `--mods PATH` is the whole integration. The session report includes the manifest hash. This repo does not add a workshop.

**Authors.** A mod author runs the authority as a dev build and uses the same inspect, edit, undo, and save tools as an engine author. A release player build joined to that session still has no tool API. Saving a prefab writes data a mod can ship. The script does not call `allow` and cannot widen its own permissions.

**Replay.** The manifest hash is a header field beside the map hash (D13). A replay whose hash does not match the loaded mods refuses to open. Snapshot playback would otherwise show entities the local scripts do not know.

### Settled by this direction

- The agent test above is how the choices in this file are judged. Live edit, undo, and save-back are part of that test.
- The session keeps ticking during an edit. Inspect, change, log, undo, and save do not pause the tick and do not rewind player commands.
- A tool does not write a column. Spawn, edit, and despawn are notices. The authority accepts or refuses them, and it is the only writer. The bytes are the session stream, so a replay can show the request and the result. Who may spawn is D29.
- An agent reads the authority store, or one applied replay tick. Interest and `POSE_BUDGET` limit player packets. They do not limit that read.
- Scene handles stay scene handles. The camera agent endpoint stays the scene editor. The session tool is a different API, even though both are loopback tools an agent can call.
- D20 is open. The recommendation is that the numeric id stays the replication id and the physics id, with a human-readable name beside it. A string as the only wire id is the option that recommendation refuses.
- The tool API is a development surface. A player runtime never writes authoritative state. Admin and operator calls run on the authority, through `allow`, or they do not run.
- Interest is the player's exposure. The entity log, the cause, and a full-sim checksum are not player datagrams.
- Mods stay in every profile. Lockdown compiles the debug and agent tool out of the release player binary and leaves the mod loader in place. A mod changes the game through data and through authority-side scripts. A player runtime that writes authoritative state is cheating, and it is refused.

### How the choices score

| Choice | Agent test | Verdict |
| --- | --- | --- |
| Numeric id plus a name component | The log and the physics body stay on a stable id. The agent types a name. | Recommend. D20–D22. |
| String as the only id | Readable. Deltas, sort order, and `BodyId` all get worse. A rename becomes an identity change. | Refuse as the wire id. |
| `Scene` arrays and scene handles | An index shifts when a neighbor is removed, so a log cannot stay attached. The camera endpoint writes the scene directly, past the authority. | Refuse. |
| Bevy, lightyear, renet, bevy_replicon | Queries and observers match the agent well. Entity ids are still opaque, so a name is still a component. The stack would own the schedule and the socket, which ADR 0007 rejects, and the agent would talk to that runtime instead of to the session notices. Time-travel would be a second replay. | Refuse. Sources only. |
| Actors | One named object is easy to point at. Query by component, and a cause that is a tick pass rather than "the object decided", both get harder. | Refuse. |
| Authority is the only writer. Connections request. | The agent and a player connection follow one rule. | Recommend. D29 sets who may request a spawn. |
| Interest and the 1024-byte pose budget | A player packet stays small at thousands of entities. The agent does not use that packet as its view. | Keep for players. Refuse as the agent's read path. |
| Predict only the owned pawn | The authority view the agent reads is the true state. A reconcile writes the pawn and the log's cause is the snapshot tick, so the agent can see the correction. | Keep. |
| Commands and notices | Commands are the input half of the entity log. Notices are spawn, edit, despawn, and subscribe. | Keep. |
| Snapshot deltas and keyframes | The agent seeks to a tick and reads state without reverse simulation and without a cross-machine deterministic sim. | Recommend. D1–D4. |
| Input-only replay | A seek on another machine can show a different world, so the agent cannot trust the picture. | Refuse as the way time-travel looks. |
| Full-world GGPO | Every peer resims every entity. The agent has no single authority log to ask. | Refuse. The owned-pawn restore stays. |
| Per-tick state hash | The agent can see the tick that diverged. | Recommend. D11. |
| Cause stored on the authority stream | Seek can answer why without resim, which is what a sim-epoch mismatch still allows. | Recommend. D25. |
| Cause on every player packet | Players pay bytes so a tool can read a reason. The tool can read the authority instead. | Refuse. |
| Camera MCP gains entity spawn | The scene handle and the entity name become the same string. | Refuse. D27. |
| Edit a replay and resume | Time-travel becomes a second simulation. | Refuse until the owner asks. D28. Live undo is a separate path. |
| Edit applied once, then the tick pass overwrites it | A moved prop snaps back. The author cannot trust the tweak. | Refuse as the default. D30. |
| Pin the edited component | The tweak sticks on the authority and on the predictor. | Recommend. D30. |
| Undo by seeking backward | Rewinds the running game and every player. | Refuse. D28 stays read-only. |
| Undo by an inverse notice | The live entity returns to the previous tool value. The tick continues. | Recommend. D31. |
| Rewrite `scene.rhai` on save | The hand-written room and its comments are replaced. | Refuse. D33. |
| Sidecar plus a data prefab | The script stays. Save overlays by name. Spawn-from-prefab is a notice. | Recommend. D32–D35. |
| Tool API always linked, trust the caller | A release player contains the edit function. A patch can skip the check. | Refuse for the player binary. D37. |
| Compile the tool out of the player binary, keep it on the authority behind grants | Players have no edit symbol. A host can still grant inspect. | Recommend. D37, D38. |
| A separate admin protocol | Two surfaces drift, and a check on one misses the other. | Refuse. One `allow` gate. D42. |
| Trust the join role byte | The player runtime would promote itself to a tool. | Refuse. The authority assigns the grant. |
| Full-sim checksum on the player snapshot | A far change is visible as a hash change, past the interest set. | Refuse. D41. The authority file still stores it. D11. |
| Player packet is the interest set only | The anti-cheat exposure is the relevancy cut already in this plan. | Keep. D41. |
| Compile mods out of release | Lockdown would remove the first-class mod path. | Refuse. |
| Mods as native libraries | ADR 0009 rejects that. A native library is not a mod. | Refuse. |
| Every peer runs mod scripts (lockstep) | Factorio can do this. This sim has no cross-machine deterministic script. | Refuse as the authority model. D43. |
| Authority runs mod scripts through notices | Sanctioned changes use the same path as a tool edit. Players receive the result. | Recommend. D43, D44. |
| Data overrides in load order, with a hard conflict | Prefabs, scenes, and assets extend without a silent last-writer. | Recommend. D45, D46. |
| Join only when the mod hash matches | The player runtime cannot bring a private rule set. | Recommend. D47. |

D20–D49, below, are the questions this section opens. D1–D19 stay open. Where this test adds weight to one of them, the line is in that decision.

## Open decisions

These are the questions to settle before any step is built. Each one is **Open**. The recommendation is a proposal for that discussion.

**Settled, and kept out of the list.** A replay is a recorded network stream. Commands, notices, and snapshots in the file are the datagram bytes a `Link` or a `UdpSocket` would carry. The reader feeds those bytes to the same apply path a live player runtime uses. Inputs are commands. Notices are the event stream, including the notices an agent connection sends. Component results (`Health`, `StunUntil`, the name, tags) ride the snapshot. There is no second gameplay codec, no dump of `Sim` memory as the file, and no replay-only component. If a value must be in the recording, it is in the live datagram. The agent path is that datagram, not a side door. The store question is judged again in [Agent observability and control](#agent-observability-and-control) and [Decision](#decision). The recommendation is still the small in-repo store. The reason is the agent test, not a closed door.

**Working proposal.** Record the authority's full stream and each player runtime's own stream (D8, D9). Keep commands, notices, snapshot deltas, and a baseline-0 keyframe every 120 ticks (D1, D3). Watching applies those datagrams (D2). A seek jumps to the previous keyframe and applies forward (D4). A debug flag resims from that keyframe and compares per-tick hashes (D11, D12, D15). Public spectators receive the authority stream 10 seconds late (D6, D7). A killcam is the last 10 seconds of the authority stream, played through the same path, with the camera chosen locally (D5, D19). The file adds a header and a length prefix so a datagram boundary survives the disk; the slice inside each frame is the datagram (D14). A protocol mismatch refuses the file. A sim-epoch mismatch still plays snapshots and refuses resim (D13).

D1 and D2 move together. D1.C is what makes D2.C possible. D1.A leaves playback as resim. D1.B leaves playback as apply. D11 and D14 move together. D11.C stores the live snapshot hash inside the datagram and the per-tick hash in the D14 frame. D14.B (raw concatenation) cannot represent a tick that sent nothing, so D11.C's empty frames would be dropped and the hash story becomes D11.A or D11.B. D3, D5, D7, and D18 share one cost: a full baseline of 4096 entities is about 34 bytes each in the [Locked names](#locked-names) layout (index, generation, flags, pose, input authority, stun, health), about 140 KB. That figure is a layout estimate, not a measurement.

### D1. Which datagrams the stream keeps

**Question.** Playback has to reconstruct actions, inputs, and events. Which subset of the session datagrams is the recording?

- **A. Commands and notices only.** The file is small. Playback resims every tick from the command log. A sim that is not deterministic on the machine doing the playback produces a different picture. Seeking from the start of a long file resims the whole prefix.
- **B. Snapshots and notices only.** Playback applies datagrams and matches what the network said, including after a sim change that stays inside the same protocol. The command that caused a tick is absent, so a debug resim has nothing to rerun.
- **C. Commands, notices, snapshot deltas, and periodic baseline-0 keyframes.** The file is the union. A keyframe is a snapshot datagram whose baseline tick is 0, which the codec already defines. Watching applies snapshots. Debugging resims commands and checks them against the stored hashes.

**Recommendation.** C. A cannot show a correct picture when the physics step diverges across machines. B cannot rerun the command that produced a bug. The owner asked for actions, inputs, and the event stream together. The agent test agrees: time-travel has to show the command and the notice, not only the pose.

### D2. How playback advances a tick

**Question.** Given C from D1, what does the player runtime do while a replay is open?

- **A. Always resim.** Snapshots are a check, not the picture. This requires the sim that recorded the file. The physics plan refuses a cross-machine golden hash, so another machine can fail the check on a correct file.
- **B. Always apply snapshots and notices.** Commands stay in the file and are ignored during playback. The picture matches the recording inside one protocol. The question "which command diverged" has no run to point at.
- **C. Apply snapshots to watch.** Resim from the last keyframe only when a debug flag is set, or when a stored hash mismatches. Shipping playback does not depend on input-only resim.

**Recommendation.** C. Input replay is the check. Snapshot apply is the picture. An agent seeking a file on a different sim epoch still sees the recorded entities. Resim is available when the epoch matches.

### D3. Keyframe interval

**Question.** How often is a baseline-0 datagram written?

- **Every snapshot** (every 3 ticks, 20 Hz). A seek is one datagram. Baselines of a full 4096-entity world are about 2.8 MB/s before deltas.
- **60 ticks** (1 s). About 140 KB/s of baselines.
- **120 ticks** (2 s). About 70 KB/s of baselines. A seek reads at most about 2 seconds of deltas.
- **600 ticks** (10 s). Smaller file. A killcam that starts mid-interval still applies up to 10 seconds of deltas.
- **1800 ticks** (30 s). Unreal's documented default for `demo.CVarCheckpointUploadDelayInSeconds` is 30 seconds ([DemoNetDriver and Streamers](https://docs.unrealengine.com/4.27/en-US/TestingAndOptimization/ReplaySystem/Streamers/)). Scrubbing backward gets slower as the gap grows. Thirty seconds is coarse for inspecting one tick.

**Recommendation.** 120 ticks. The killcam window then contains a handful of keyframes, and tick inspection never applies more than about 2 seconds of deltas. The interval stays a multiple of `SNAPSHOT_EVERY` (3) so a keyframe lands on a snapshot tick. 120 is a multiple of 3.

### D4. Seek, rewind, and scrub

**Question.** How does playback land on tick N, including a tick behind the playhead?

- **A. Keyframe, then forward.** Load the greatest keyframe with tick `<= N`, apply it, apply deltas through N. Rewind is the same seek to an earlier tick. The kinematic pass and the physics step have no inverse, so nothing simulates backward.
- **B. Decoded bracket in memory.** After A, keep the decoded poses for that keyframe bracket. A scrub that stays inside the 120 ticks does not read the file again.
- **C. Reverse simulation.** Run the tick pass backward from the current tick.

**Recommendation.** A on disk, for a cold seek. B inside `genos-net-view` and inside the session tool after the first seek into a bracket, so a scrub or an agent's next query inside those 120 ticks does not read the file again. C is a bad fit for this sim. This is the agent's time-travel.

### D5. Killcam

**Question.** When a pawn's `Health` drops, what is played, and how long is the window?

- **A. The victim's received stream**, the last N seconds. Playback shows the datagrams that player runtime actually applied. An attacker outside that interest set is absent.
- **B. The attacker's view only.** Smaller. The victim's surroundings are whatever that view's interest set contained.
- **C. A slice of the authority's omniscient recording.** The attacker is present even when the victim's live packet culled them. Playback uses the same apply path. The camera starts on the attacker's pawn, which is already an entity in the slice, and can be moved locally (D19).

The Overwatch written notes describe about 8–12 seconds of network data fed to replay as if it were live ([GDC 2017 notes](https://www.lfzxb.top/ow-gdc-gameplay-architecture-and-netcode/)). That range is a design input from those notes, not a number from a public paper. Duration choices: 8 s, 10 s, 12 s.

**Recommendation.** C, with a 600-tick (10 s) window. The ring stores datagrams and includes a keyframe at the start of the window, so the killcam does not resim the match. An upper bound, if every snapshot in the window is a full baseline, is the 2.8 MB/s figure from D3, about 28 MB for 10 seconds. Deltas of clean fields are smaller. The number 10 is the part to argue.

### D6. Live spectators

**Question.** What does a spectator connection receive while the match is running?

- **A. Player-like interest.** The connection follows one pawn, uses the 32 m grid, and stays inside `POSE_BUDGET`. A coach sees the packet that pawn's player would see. The connection sends no commands.
- **B. Omniscient, delay 0.** Every replicated entity, as it happens. A spectator can relay that picture back to a player in the match.
- **C. Omniscient, delayed.** The authority sim writes the stream immediately. The spectator socket receives copies after a delay. SourceTV broadcasts this way (`tv_delay`). Competitive TF2 leagues require a long delay so a spectator cannot ghost for a team; RGL's floor is 90 seconds ([RGL SourceTV](https://docs.rgl.gg/guides/servers/sourcetv/)). That 90 is a league rule, not an engine default.

**Recommendation.** C for a public spectator. A stays available as a coach connection: a normal connection, a chosen view entity, no input authority. B is a poor default on a match anyone can join.

### D7. Spectator delay

**Question.** How long is the public delay, and does the file wait too?

Choices: 0, 3 s (180 ticks), 10 s (600 ticks), 30 s (1800 ticks), 90 s (5400 ticks).

**Recommendation.** 10 seconds, stored in the header, configurable per spectator session. The authority file is written immediately, with delay 0 in its header. Stopping the process keeps the tail, because the tail is already on disk. The spectator socket is what waits. A killcam plays the recorded slice at once; it is not a live spectator. An operator can set 90 seconds for a broadcast. At the D5 upper bound, 90 seconds of that full-baseline stream is about 250 MB of queued datagrams.

### D8. Who records

**Question.** Which runtime opens the file?

- **A. The authority runtime only.** One omniscient file. A bug that existed only in the bytes a player runtime applied, after loss, is not in the file.
- **B. Each player runtime**, writing the datagrams it sent and the datagrams it applied.
- **C. Both.** The header says which.

**Recommendation.** C. Killcams and public replays need the authority file. A bug report from one machine needs the bytes that machine actually applied.

### D9. Interest in the authority file

**Question.** Does the authority recording use the live interest set and the pose budget?

- **Authority file.** Every replicated entity. Interest and `POSE_BUDGET` do not apply. The datagram layout is unchanged.
- **Player file.** The datagrams that runtime sent and applied, including loss and the budget.

**Recommendation.** That split. Playing a player file shows the holes that player had. Playing the authority file shows the match. D18 is the budget half of the same split. An agent time-travels the authority file. A player file would hide entities the agent might name.

### D10. The map inside the file

**Question.** Playback has to load the same static geometry the session loaded before the loop. What does the file store?

- **A. The scene script bytes, plus a hash of those bytes.** The session loader runs before the first tick, as it does for a live session. The script is not an entity codec. The body of the file stays datagrams. A caller that also passes a path must hash-match the embedded bytes, or the open fails.
- **B. The hash and a path.** Opening the file on a machine without that path fails. A bug file does not travel.
- **C. The hash only.** The caller has already loaded a map. A mismatch refuses the open. The smallest header, and a file that cannot be attached to a bug report on its own.

**Recommendation.** A.

### D11. Per-tick state hashes

**Question.** The checksum under [Components, dirty state, deltas](#components-dirty-state-deltas) is the state hash (FNV-1a over generation, pose, origin sectors, input authority, stun, health). When N24 has linked physics, `hash_world` is a second `u64`. Where are they written?

- **A. Snapshot ticks only** (every 3rd tick), inside the snapshot datagram. Live and file match. A resim can diverge on the two ticks between snapshots and look clean at the next snapshot.
- **B. Every tick, on the live stream and in the file**, as its own datagram. The diverging tick is visible immediately. Every connection pays for a datagram 60 times a second even when no pose is dirty.
- **C. Every tick in the file, in the frame beside the datagram.** Authority and inspect datagrams carry the checksum of that snapshot tick as a trailing `u64`, and a second `u64` for `hash_world` (zero until N24). A tick that sent nothing stores `len = 0` and still stores the hash. Intra-snapshot divergence is visible in the recording and in a resim. A player snapshot omits the hash (D41).

**Recommendation.** C for the authority recording and for an inspect grant. Accepting it adds those two `u64` words to the end of that datagram in [Locked names](#locked-names). A player snapshot omits them (D41). The in-memory checksum function stays the one N5 already specifies.

### D12. What a hash mismatch does

**Question.** Playback has a stored hash and a hash it just computed. They differ.

- **A. Stop** at that tick.
- **B. Mark the tick and keep resimulating.** The next mismatch is measured against a sim that is already wrong.
- **C. Stop the resim, keep applying recorded snapshots from that tick, and keep the tick in the report.** A `--strict` flag uses A.

**Recommendation.** C for watching a bug. A for a test that must fail on divergence.

### D13. Versioning and compatibility

**Question.** Which header fields decide whether a binary will open a file, and how many old protocols one binary reads?

Header fields in the proposal:

- magic `GNOS` (4 bytes)
- `protocol: u16` (the datagram layout)
- `sim_epoch: u32` (bumped when tick-pass results change and the datagram layout does not)
- `tick_rate: u32` (60)
- `keyframe_interval: u32` (ticks)
- `recorder_role: u8` (`1` authority, `2` player)
- `spectator_delay_ticks: u32` (0 on an authority file)
- `map_hash: u64`
- git hash as `u16` length plus bytes, for a person reading the file
- `mod_hash`: 32 bytes, SHA-256 of the mod manifest from D47, or 32 zero bytes when no mod is loaded. A replay refuses to open when the loaded manifest hash differs.

A protocol mismatch refuses the open. A sim-epoch mismatch allows snapshot playback and refuses resim. The git hash is not a compatibility check. It changes every commit.

How many old protocols:

- **A. Current only.**
- **B. Current, plus one previous, snapshot playback only.**
- **C. Every protocol the project has ever written.**

**Recommendation.** A until a recording has been given to someone other than the person who recorded it. Then B. The RTS command-log lesson (StarCraft, Warcraft III) is why `sim_epoch` is its own field: those games store commands plus a version, and a later sim does not promise to reproduce the match. This plan does not cite a primary URL for those formats. The lesson is the split. Snapshot playback still works when resim is refused, which is what an input-only file cannot do.

### D14. Framing around the datagram

**Question.** A UDP datagram has a length the socket provides. A file of concatenated payloads does not. How is the file cut back into datagrams?

- **A. A header, then length-prefixed frames.** Each frame is `tick: u32`, `state_hash: u64`, `physics_hash: u64`, `len: u32`, then `len` datagram bytes. A trailer maps each baseline-0 tick to a file offset and is rewritten on close. A missing trailer (a crash) still scans the length prefixes of the flushed prefix. The datagram slice is what `Link` delivered or would have delivered.
- **B. Raw concatenated payloads.** The reader must parse the codec to find the next datagram. A corrupt byte shifts the rest of the file. A tick that sent nothing cannot be stored.

Source demos are the precedent for A. A `dem_packet` wraps raw network packets in a demo command header, and playback feeds those packets to the client net channel, which is the player runtime's apply path in this repo ([DEM format](https://github.com/SizzlingStats/demboyz/blob/master/docs/DemFormat.md)). The header is framing. The packet is the packet.

**Recommendation.** A. The length prefix is the file's copy of the socket's datagram length. A test compares the slice to the bytes `Link` sent, byte for byte. Framing is not a gameplay format.

### D15. What the debug binary shows at tick N

**Question.** The default inspect path, before any flag.

- **Apply.** Load the keyframe at or before N, apply deltas through N, print the components. The picture is what the network said.
- **Resim.** Rerun stored commands from that keyframe and print the resim hash next to the stored hash.

**Recommendation.** Apply by default. `--resim` prints both hashes. The session tool's inspect at a tick is this same path, plus the entity log.

### D16. Recording with no socket

**Question.** Does a session that never binds a port still write a replay?

**Recommendation.** Yes. `genos-session --bind off` and `genos-net-view` run the same codec into the recorder. A single-player bug in those programs is a file. `examples/camera` still does not open a session and does not write a replay in this plan. Recording from the camera loop is a later plan.

### D17. Playback controls

**Question.** Which controls exist on the first debug binary?

**Recommendation.** Pause, step one tick, fast-forward, and a tick integer for scrub, all in `genos-net-view`. Headless flags call the same functions: `--replay PATH --tick N`, `--step`, `--resim`. The window's scrub control is that tick integer. There is no third binary.

### D18. Pose budget on the recorder

**Question.** `POSE_BUDGET` is 1024 bytes for a live player link. Does a keyframe, or an omniscient authority datagram, have to fit in it?

**Recommendation.** No. The budget stays on live player links. A keyframe and an omniscient datagram may be larger. Spawns and despawns already ignore the budget. The cost is the size in D3 and D7.

### D19. Camera during playback and killcam

**Question.** Is the camera a track in the file?

The view target is an entity that already has a pose in the stream. A free camera during playback is local UI state and is not written back. Choosing which pawn to follow is local and is not a notice. The strike command already identifies the attacking connection, and that connection's pawn is the entity the killcam starts on.

**Recommendation.** Both. Playback can follow any entity with a pose, and it can fly a free camera. The file does not store a camera track. An agent's time-travel is a query at a tick, not a camera track.

### D20. What the agent types as the id

**Question.** The owner asked for stable human-readable entity ids. The replication id in this plan is `{ index, generation }`. What does the tool accept?

- **A. The numeric id only**, printed `index.generation` (`4.1`). Stable across removal. Opaque to a person and to an agent reading a log.
- **B. A name string is the only id.** The datagram carries the string. A rename is an identity change. Snapshot sort order follows string bytes. The physics `BodyId` would no longer be these bits.
- **C. Both.** Snapshots keep the numeric id and sort by index. The name is a component. The tool accepts the name or `index.generation`.

**Recommendation.** C. A fails the readability test. B fails the packet, the sort, and the shared body id. The name is not a scene handle.

### D21. How long a name stays unique

**Question.** After despawn, or after a rename, may a new entity take the string?

- **A. Unique among live entities.** Despawn frees the string. A replay query for `door` at two ticks can mean two ids.
- **B. Unique for the session and the recording.** Despawn does not free the string. A rename keeps the old string reserved and still resolving to the same numeric id. `find` at a replay tick and `find` live agree on which id owned that string.

**Recommendation.** B. Time-travel is unambiguous. The log stores the numeric id either way, so a renamed entity's history stays one chain. The cost is a name table that grows with every spawn for the life of the session.

### D22. The name when the caller omits one

**Question.** Spawn with no name.

- **A. Refuse.**
- **B. The authority mints `<type>-<index>` when a type was set, otherwise `e-<index>`.** The string is reserved for the session, same as D21.

**Recommendation.** B. A tool can spawn in one call and then query the minted name.

### D23. The first query

**Question.** Which filters exist on the first tool?

- **A. Type, tag, region, and component presence.**
- **B. A, plus field predicates** (`health < 10`, `x_mm > 1000`).

Region is an AABB in millimeters, tested with the same cell size as interest (`CELL_MM`). Type and tag are components. Component presence means the column has a value for that id.

**Recommendation.** A. Field predicates are a real parser and a second query language. Presence, type, tag, and region cover "what is in this room" and "what has health". Add predicates after those four have tests.

### D24. How long the live entity log is

**Question.** The replay holds the session. The live authority also keeps a ring so a tool can ask without opening a file.

- **16 ticks**, the hit-rewind window.
- **120 ticks**, one keyframe interval.
- **600 ticks**, the killcam window.
- **Unbounded** until the process exits.

**Recommendation.** 120 ticks. An agent watching a live change sees about two seconds of cause. Older than that, it seeks the replay. Sixteen ticks is shorter than a stun an agent might want to explain. Unbounded growth is the replay file's job.

### D25. Where the cause is stored

**Question.** A cause is a tick-pass id, a command `(connection, tick)`, a notice sequence, or a reconcile snapshot tick. Where does it go?

- **A. Authority memory only**, in the 120-tick ring. A replay seek shows state and not why, unless the agent resims.
- **B. On every player snapshot**, beside each dirty component. Every player pays for tick-pass names.
- **C. On the authority recording and on agent-connection datagrams.** Player snapshots leave the cause flag clear. The format has the flag. Players do not spend the bytes. Seek on the authority file rebuilds the log by apply, with no resim.

**Recommendation.** C. It matches the agent read path. The working flag field in [Locked names](#locked-names) is a `u8`, and bits `1` through `64` are already named. Bit `128` would be the last one, and D30's pin needs a bit too. Accepting D25 or D30 widens that field to `u16` in the same commit that first writes the bit. Cause is bit `128`. Pin is bit `256`. After the flags, a cause writes a `u16` kind and a `u32` word (pass id, notice seq, command tick, or snapshot tick). Kind `1` tick pass, `2` command, `3` notice, `4` reconcile. Neither bit is in the locked layout until its decision is accepted.

### D26. Subscribe or poll

**Question.** How an agent tracks changes.

- **A. Poll.** The tool calls query and inspect on its own clock.
- **B. Subscribe.** A notice registers a filter. A later matching write enqueues a notice to that agent connection: entity id, component, cause. The notice stays small. The tool reads full state with inspect if it wants it.
- **C. Both.**

**Recommendation.** C. Subscribe is how the agent hears a change without scanning. Poll remains for a one-shot query. Delivery is the reliable ordered notice path. There is no third channel.

### D27. Which process the tool talks to

**Question.** MCP, CLI, or both, and in which binary?

- **A. Add entity tools to the camera agent endpoint** in `crates/mcp`. One URL for scene handles and entities.
- **B. A session tool.** CLI arguments on `genos-session`, and a loopback MCP in that same process. Both call one set of Rust functions. Those functions enqueue notices. The camera endpoint is unchanged.
- **C. CLI only.**

**Recommendation.** B. A puts the entity name in the same map as `wall:<id>` and writes from the camera process, which is not the authority runtime. C makes an agent that already speaks MCP grow a second habit. The headless test calls the Rust functions and feeds one JSON body to the handler. It does not need a display.

### D28. Time-travel writes

**Question.** After a seek, may the agent edit and continue?

- **A. Read only.** Query, inspect, and the entity log at that tick. A write returns an error. The live session is untouched.
- **B. Fork.** Edit the sought tick and start a new authority session from there.

**Recommendation.** A. The owner asked to time-travel through replays, which is seek and read. Fork is a second simulation and a second recording. It can be a later plan. Live undo of an author edit is D31. It does not seek.

### D29. Who may request a spawn

**Question.** The owner said the same authority rules as network clients.

- **A. An agent connection is no stronger than a player connection.** Players cannot spawn in this plan, so the agent cannot either. Spawn stays an internal authority call. The tool can edit and despawn only what already exists, and even that is a notice.
- **B. Neither connection writes state.** The authority applies notices. A player connection's spawn notice is refused. An agent connection's spawn, edit, and despawn notices are applied at the tick boundary. Edit of a predicted pawn still replicates back and reconciles, like any other correction.

**Recommendation.** B. A reads "same rules" as "same privileges" and leaves the agent unable to manage entities, which fails the test that opened this plan. B reads "same rules" as one path: request, authority applies, the datagram is recorded. The agent is not a backdoor. The player still cannot mint an id. Join already uses notice name `1`. The recommended payload adds a `u8` role: `1` player, `2` tool. That byte is how the authority tells the two requests apart. It is part of this decision, not a separate channel. An author at the CLI and an agent on MCP both use role `2` in dev. The byte is a request. [Lockdown](#lockdown) assigns the grant. Sending role `2` does not make a player into a tool.

### D30. Does a live edit stick

**Question.** The motion pass writes pose every tick it has a command. An author sets a new pose. What happens on the next tick?

- **A. The edit is applied once.** The pass may overwrite it. The log shows the edit, then the pass. A moved prop snaps back when something owns that pose.
- **B. The edit pins that component.** Tick passes skip pinned components until an unpin notice. The pin replicates. The predicting runtime skips the same fields, so it does not correct every tick. Pose, name, type, and tags pin by default. `Health`, `StunUntil`, and input authority stay unpinned unless the notice says pin.
- **C. The pass always wins on the following tick.** The edit is visible for one tick.

**Recommendation.** B. A and C make a live tweak look like it worked and then take it away. The pin is a component, not a pause. On the wire it is flag bit `256` in the `u16` flag field from D25, so it does not take the last `u8` bit from the cause.

### D31. What undo covers

**Question.** Undo has to be safe in a running game.

- **A. Seek the replay backward.** Players and tick passes rewind. That is D28's fork.
- **B. Inverse of the last tool notice.** The authority keeps one stack, shared by every tool connection, depth 64. Undo enqueues the inverse notice. Redo enqueues the forward notice. A tick-pass write and a player command are not on the stack. Despawn undo spawns a new generation under the reserved name and the log links the old id to the new one. D21 still refuses a different spawn of that name.
- **C. Snapshot the whole entity on every change, including simulation, and restore that blob.**

**Recommendation.** B. The tick continues. Other machines see the undo as a new applied notice, not as a rewind. C would undo a player's movement and a hit.

### D32. Which entities and components a save includes

**Question.** Save-back should not dump the whole fight onto disk.

- **A. Every component of every entity**, including health and projectiles.
- **B. Entities marked authored**, plus any entity named in the save request. Components are name, type, tags, pose, and pins. Map entities from N18 are authored. A tool spawn is authored. A gameplay spawn is not, until a save names it. Combat fields stay out unless the request says full.
- **C. A checksum blob**, with no fields an author can read.

**Recommendation.** B.

### D33. Scene file, sidecar, or prefab script

**Question.** Where the saved fields go. The room today is `examples/camera/scene.rhai`: `floor`, `wall`, `solid`, and `light`, with comments. Nothing in the tree is a prefab.

- **A. Rewrite the Rhai script** from the live entities. Comments and hand layout go away. Rhai has no source map back to those calls.
- **B. A sidecar JSON next to the script**, loaded after `load_path`. Names in the sidecar overlay those entities. A prefab is that same schema for one template, in its own file. Spawn-from-prefab is a notice. The authority reads the file and spawns. The tick does not eval the file.
- **C. A prefab is a Rhai fragment** eval'd on spawn.

**Recommendation.** B. The hand-written script stays the room. C runs a script on the spawn path. ADR 0009 keeps the script off the socket and this plan keeps it off the tick. `genos-sim` does not open the file. The session tool does, with `genos-json`.

### D34. When save runs

**Question.** How a live edit becomes a file.

- **A. Every applied edit writes the file.**
- **B. An explicit save, on the authority process only.** The edit is already applied and replicated. The save copies columns at that tick and writes after the tick returns. Players are not asked to write the file. A failed write is reported. The sim is unchanged by the failure.
- **C. Save when the process exits.**

**Recommendation.** B. A and C write the disk on a cadence the author did not ask for, and they race a hand-edited script.

### D35. Undo against the file

**Question.** After a save, an undo of the live edit.

- **A. Undo also reverts the file** to the previous sidecar bytes.
- **B. Undo reverts the live entity only.** The file changes on the next explicit save.

**Recommendation.** B. Disk revert surprises an author who saved on purpose. The live game and the file are two steps.

### D36. What the author uses

**Question.** The human and the agent need the same edits.

- **A. The author uses the session CLI and the same JSON handler as the agent.** A later window calls those functions. It is not a step in this plan.
- **B. This plan adds an inspector window.**
- **C. The camera MCP gains the inspector and the save.**

**Recommendation.** A. One path, two callers. C puts entity names into the scene-handle endpoint and saves from the camera process, which is not the authority. In staging and release the player binary does not contain this handler. D37.

### D37. How a profile removes the tool

**Question.** Dev, staging, and release need a lockdown. The owner asked to compile the debug and agent APIs out, or to disable them.

- **A. Compile out only.** A Cargo feature per profile. Release does not contain the tool functions in any binary. An operator cannot inspect a release authority, because the function is gone.
- **B. Disable only.** The functions are in every binary. A config flag starts them off. A patched player can skip the flag.
- **C. Both, split by binary.** Staging and release player binaries compile the tool API out. The authority binary keeps the functions in every profile and starts with an empty tool grant. Dev compiles the tool into the player-side session tool and grants it on `127.0.0.1` with no token. `genos-net-view` is a dev binary. `examples/camera` and its scene MCP are not linked into the player binary.

**Recommendation.** C. Compile-out is what makes a player patch unable to call edit. A grant on the authority is what lets Genos turn inspect on without a second protocol. The feature names are `profile-dev`, `profile-staging`, and `profile-release`. Workspace tests default to `profile-dev`. The mod loader and the Rhai host are not behind these features. Release still loads mods.

### D38. Grants

**Question.** Which capabilities exist, and which profile grants them before anyone configures a host.

Capabilities in the recommendation: `command`, `inspect`, `edit`, `save`, `subscribe`, `undo`, `debug`. `edit` covers spawn, despawn, pin, and component writes. `debug` is the loss sliders, the desync view, and replay resim.

| Profile | Player binary | Authority default grant |
| --- | --- | --- |
| dev | Tool API compiled in. Loopback gets every capability. A remote player gets `command` only. | Same process, so the loopback grant is the tool grant. |
| staging | Tool API compiled out. | `command` for players. `inspect` only if an operator token is configured. `edit` only if an admin token is configured. |
| release | Tool API compiled out. | `command` for players. Tool grants empty unless the host sets a token. |

- **A. Coarse capabilities, as in that table.** One inspect grant reads any entity. One edit grant writes any entity.
- **B. A per-entity access list** on top of the capabilities.

**Recommendation.** A. B is a second product. Players still only receive the interest set, which is narrower than the inspect grant. An operator inspect may read entities a player packet omits.

### D39. Who proves an operator or an admin

**Question.** Admin and operator access are authenticated. This repo has no accounts.

- **A. A secret the host passes in when it starts the authority** (`--operator-token`, `--admin-token`, or an environment variable). The join notice carries the secret. The authority compares it and assigns the grant. An empty token means that grant cannot be taken over the network. Dev loopback does not need a token. A mismatch refuses the join.
- **B. Mutual TLS**, with certificates this repo would have to issue.
- **C. An account system** in this repo.

**Recommendation.** A. Genos, in the other repository, holds the secret and starts the process. This repo checks the bytes. It does not store users. B and C are later plans. The operator secret and the admin secret are different, so inspect does not imply edit.

### D40. The audit log

**Question.** Admin access is audit-logged. The entity log already records applied writes.

- **A. The entity log is the audit.** Refusals and inspects that change nothing are missing.
- **B. A separate authority-side audit.** Each line is tick, connection, capability, target id, allow or deny. Inspect, edit, save, undo, subscribe, and a refused player edit all write a line. The audit is not in a player datagram. The entity log still records the applied cause.
- **C. Audit lines are replicated** to every player.

**Recommendation.** B. A ring of 4096 lines, plus the same lines appended to the authority recording so a replay can show who asked. C would publish the audit to the people it is about. Players do not receive it.

### D41. What a player packet may contain

**Question.** Anti-cheat exposure is the datagram, not a promise in a comment.

- **A. The interest set only.** Relevant entities, the components a remote entity needs to render, and the owned pawn's predicted components. No entity log. No cause word. No full-sim checksum. No name of an entity outside the interest set. Pin replicates only to the runtime that predicts that entity.
- **B. The full authority snapshot**, and trust the player not to draw it.
- **C. A plus the world checksum** from D11, so the player can detect its own desync against the whole sim.

**Recommendation.** A. B is the omniscient recording, which is the authority file and an inspect grant. C lets a player watch a far fight by watching the hash change. D11.C stays for the authority file and for agent frames. The player snapshot drops those two trailing `u64`s. Desync of the owned pawn is still the pose compare in N13.

### D42. Where the check sits

**Question.** The same API has to be gated everywhere.

- **A. One function, `allow(connection, capability, target)`.** Tool handlers, the join path, and the command path call it. A deny returns an error, writes the audit line, and does not change the sim. Release player code does not contain the tool handlers, so the call is absent there rather than checked.
- **B. Each handler has its own if-statement.**
- **C. The network boundary checks once, and in-process tool calls skip the check.**

**Recommendation.** A. C would make the dev CLI a backdoor around the grant. In-process calls from the session tool use a connection id and the same `allow`. A mod script does not call `allow`. Its registered functions are the narrower set in D44.

### D43. Where a mod script runs

**Question.** Multiplayer stays authoritative. Factorio runs mods on every peer. Garry's Mod runs a server realm and a client realm ([realms](https://wiki.facepunch.com/gmod/States)).

- **A. Authority only for anything that changes an entity.** The player runtime applies snapshots and may run a presentation script. That presentation script cannot spawn, edit, or despawn. Custom movement a mod wants is an authority notice, and the pawn reconciles.
- **B. Every runtime runs every script.** Joining requires the same bytes, and the tick is lockstep. A script that is not deterministic across machines desyncs the session.
- **C. The player runtime also runs the mod script for the owned pawn**, so prediction stays smooth, and the authority runs every other script.

**Recommendation.** A. B needs the cross-machine determinism the physics plan refuses, and a script with a fuel limit can still diverge if anything outside the engine differs. C is a later plan for one pawn, after A is tested. The manifest hash is required in all three. A does not skip it.

### D44. How a script is allowed to change state

**Question.** ADR 0009 gives a script registered functions and the checked engine. The `unchecked` feature stays rejected.

- **A. The script receives the store and writes columns.**
- **B. The script calls registered functions that enqueue the same notices as a tool.** The authority applies them at the end of the tick. The mod permission list is the functions actually registered: read a component, enqueue a spawn, enqueue an edit of a property or a built-in component, play a presentation event. No socket, no window, no path outside that mod's directory. Fuel is a fixed operation budget per tick. A script that exceeds it is stopped for that tick and the session report names the mod. The entity log's cause is the mod id and the hook name.
- **C. The script may also open a socket if the mod permission says so.**

**Recommendation.** B. A skips the authority path. C breaks ADR 0009. The mod permission is not a connection grant. Loading the mod is the operator's choice. The script cannot add permissions while it runs.

### D45. Load order and conflicts

**Question.** Two mods both ship `prefabs/door.json`, or both hook `on_tick`.

- **A. Last file on disk wins.** Silent.
- **B. A manifest orders mods by `before` and `after`.** The same key from two mods, and neither sets `overrides` for that key, fails the load and names both mod ids and the key. The later mod may set `overrides` and then it wins, and the report records the loser. A cycle in `before` / `after` fails the load.
- **C. Numeric priorities only**, with equal priorities undefined.

**Recommendation.** B. The host can still append an explicit order in the manifest when two mods do not declare a relation. That explicit order is the tie-break, and an undeclared duplicate key still fails.

### D46. What a mod may override or extend

**Question.** How data-driven the entity is.

- **A. New Rust component columns** from a mod.
- **B. Prefabs, scene overlays, asset paths, and a property map.** A prefab carries name, type, tags, pose, pins, and string or integer properties. A scene overlay sets those fields on an entity name from the base scene or adds an entity. An asset override replaces the bytes at a logical path. The decoder stays `genos-load`. A mod does not add a column type.
- **C. A mod may replace `scene.rhai` outright** and also do B.

**Recommendation.** B, plus one map replacement when a single mod declares the `scene` key. Two mods that both declare `scene` are a D45 conflict. C as an unconditional rewrite would drop the base scene without a record. Properties are how a mod extends an entity without a new column. A built-in pass ignores a property it does not know. A script reads it.

### D47. Distributing mods and checking them

**Question.** Players must run the sanctioned set, automatically.

- **A. The operator tells each player where to download, out of band.** A wrong download is a support problem.
- **B. The authority owns a manifest.** Each file has a SHA-256. The manifest hash is SHA-256 over the canonical listing (mod id, version, path, file hash, load order). The implementation of SHA-256 is in-tree. No new crate. The player sends its manifest hash at join. Equal hashes start the tick. A mismatch transfers the missing files on a reliable stream, then hashes them again. A failed hash refuses the join and never ticks. Gameplay datagrams stay the entity stream. The replay header stores the same manifest hash next to the map hash.
- **C. Trust the filename.** No hash.

**Recommendation.** B. A is easy to get wrong on a Genos host with many players. C does not stop a swapped file. The reliable stream is file bytes, not a second entity protocol.

### D48. What Genos has to do

**Question.** A hosted authority should run a modded game without a special build.

- **A. `--mods PATH` on `genos-session`.** The directory holds one folder per mod and a `manifest.json`. The process loads it before the tick, prints `mod_hash` in the session report, and serves that set to joiners. Genos, in the other repository, sets the argument. An empty path is the base game.
- **B. A workshop client inside this repo.**
- **C. A separate modded binary.**

**Recommendation.** A. B is a product in the hosting repository. C would make modded and unmodded two protocols. The release authority already keeps the loader (D37). No profile compiles it out.

### D49. Mod authors and the dev tool

**Question.** Authors should inspect a running modded game. Lockdown still applies to players.

- **A. The author runs a `profile-dev` authority and uses the same inspect, edit, undo, and save functions.** A release player joined to it has the tool compiled out. A script cannot call those functions. Save writes a prefab or a sidecar the author can put in the mod folder.
- **B. Every release player gains inspect if a mod is loaded.**
- **C. A separate mod-debug protocol.**

**Recommendation.** A. B would undo D37 as soon as a server runs mods. C splits the tool the author was promised.

## Global constraints

- ADR 0007. Network code uses the Rust standard library and BSD sockets or Winsock. `tokio` stays rejected. A third-party crate does not own the network. `renet`, `lightyear`, `bevy_replicon`, and any QUIC or WebTransport crate stay off the dependency list. They are design sources.
- ADR 0009. Rhai stays the game script and the mod script. A script does not own a socket. The checked Rhai build stays the mod evaluator. The `unchecked` feature and the Grain VM stay out. A mod script enqueues notices. It does not write a column. The authority's mod pass is the only tick pass that calls Rhai.
- ADR 0001 and ADR 0004. The input system stays a pull. A tick samples the action map once into a command. The device is not replayed.
- `genos-math` stays `f32`. There is no `f64` path in that crate. Replicated positions are integer millimeters. Wide worlds use a sector, which is the same shift the physics plan's origin work publishes.
- Y is up. The ground is the XZ plane. Yaw 0 looks along -Z. Positive yaw turns toward +X. Positive strafe is +X. Lengths are metres. The tick is `1/60` s, the same step `update` already accumulates in `crates/scene/src/camera.rs`.
- The character controller is the action map in `crates/input/src/action.rs`. It is not a body. The simulated capsule is a body. Do not rename one to the other.
- A scene handle (`floor`, `wall:<id>`, `solid:<id>`, `light:<id>`) stays the agent-endpoint name. It is not the entity id. `CONTEXT.md` says to avoid calling a scene handle an entity id.
- The agent endpoint stays loopback MCP on `127.0.0.1`. It is not the game session. `CONTEXT.md` says to avoid calling that endpoint a game network.
- The person is the player. Sources say "client" for the program that person runs. In this repo that program is the player runtime. Sources say "server" for the authoritative program. In this repo that program is the authority runtime, and it is still a runtime. The game is not a server. Genos, the other repository, starts the authority process.
- "System" in this repo is a system record. The ECS behavior is a tick pass.
- New public words (`EntityId`, entity name, entity log, cause, agent connection, pin, prefab, capability, grant, audit log, profile, mod manifest, override, mod permission, tick pass, authority runtime, player runtime, connection, snapshot, command, notice, replay, keyframe, spectator) are added to `CONTEXT.md` in the commit that first publishes them (step N1). N1 waits until D1–D49 are answered. The camera agent endpoint and the scene handle keep the definitions they already have. Prefab is a new word. It is not a scene handle. Admin and operator are grants, not a second kind of player. Mod keeps the definition already in `CONTEXT.md`. A mod permission is not a connection grant.
- A new crate sets `[lints] workspace = true`, joins the workspace `Cargo.toml`, and is named in `docs/architecture.md` in that same commit.
- `examples/camera` does not open a game socket in any step of this plan. The first proof stays the lit scene.
- Headless tests use a fixed command list, a fixed tick, a seeded link, and no window. Two runs in one process match on the integers the step names. Do not check wall-clock time, thread scheduling, or a hash from another operating system.
- Every step keeps `cargo test -p genos-sim` green, plus the tests of any other crate that step touches. From N20 on, `cargo test -p genos-net` is included. `cargo build --workspace` passes. The workspace denies warnings.
- Do not add a job scheduler. Tick passes run on the calling thread, in the order they were registered.

## Words

These are the words N1 adds to `CONTEXT.md`. This plan uses them from here on. When a sentence cites another project, it uses that project's word and then the word above.

**Entity**:
A stable simulation id and the components stored under it. The same id is used in memory and in a snapshot.
_Avoid_: Scene handle, entity index that shifts when a neighbor is removed.

**Tick pass**:
A function the tick calls in a fixed order. It reads and writes components. It stores no simulation state of its own.
_Avoid_: System record, script.

**Authority runtime**:
The headless runtime that writes replicated components. Genos hosts this process.
_Avoid_: The game, the agent endpoint.

**Player runtime**:
The runtime a player is playing. It predicts one owned entity and interpolates the rest.
_Avoid_: The player (the person), the agent endpoint.

**Connection**:
One player runtime joined to an authority runtime.
_Avoid_: Gamepad slot, MCP session.

**Snapshot**:
The replicated components one connection needs for one tick, delta-compressed against an earlier snapshot.
_Avoid_: The whole `Scene` struct, a lighting cache.

**Command**:
One tick of sampled actions for one connection. The redundancy window retransmits unacknowledged commands. A lost command is filled by a later packet.
_Avoid_: Action (the named intent on the device), notice.

**Notice**:
A reliable ordered message that is not itself the replicated state. Join, leave, and a request use a notice. The result of a request is a component in a snapshot.
_Avoid_: Command, MCP tool call.

**Replay**:
The session datagram stream, written to a file or sent to a spectator connection, and read back through the same apply path. Prediction still says "replay the commands" for the owned-entity reconcile in N13. That verb is the pawn correction. This noun is the recording.
_Avoid_: A second gameplay codec, a dump of simulation memory.

**Keyframe**:
A snapshot datagram whose baseline tick is 0, written into the replay on an interval, so a seek can start there.
_Avoid_: A copy of the whole `Sim`, a checkpoint that skips later datagrams while it saves.

**Spectator**:
A connection that receives the session stream and does not hold input authority.
_Avoid_: A second protocol, the player (the person).

**Entity name**:
The session-unique human-readable label for one entity. A tool may pass it instead of the numeric id. It is a component. A rename keeps the old label reserved for that same id.
_Avoid_: Scene handle, a string used as the only replication id.

**Entity log**:
The recent commands, notices, and component writes for one entity. Each line has a tick and a cause. The live ring is bounded. The replay holds the session.
_Avoid_: Action (the named input intent), the action map.

**Cause**:
The tick pass, command, notice, or reconcile snapshot that wrote a component.
_Avoid_: System record.

**Agent connection**:
A connection with no player. A tool uses it to request spawn, edit, despawn, query, and subscribe. An author at the CLI uses this same connection. The authority applies those requests when the grant allows them. The connection does not write a column.
_Avoid_: The player (the person), the camera's agent endpoint, a game network.

**Capability**:
One named thing a connection may ask the authority to do, such as `command`, `inspect`, or `edit`. The profile decides whether the function exists in that binary.
_Avoid_: A second protocol, a role byte the player runtime assigns to itself.

**Grant**:
The capabilities the authority has assigned to one connection. A join role is a request. The grant is the answer.
_Avoid_: An account, a scene handle.

**Audit log**:
The authority's record of tool and admin calls, including refusals. It is not replicated to player runtimes.
_Avoid_: The entity log, a player datagram.

**Profile**:
`dev`, `staging`, or `release`. It selects what is compiled into a binary and which grants start empty.
_Avoid_: A runtime mode the player can switch on.

**Mod manifest**:
The ordered list of mods for one session, with a hash of every file. The authority and every player runtime must have the same hash before the tick.
_Avoid_: A workshop catalog, a native library list.

**Override**:
A later mod's explicit claim on a prefab name, a scene key, an asset path, or a script hook that an earlier mod also provides. Without that claim, the duplicate is a load error.
_Avoid_: Last file on disk wins.

**Mod permission**:
The registered Rhai functions one mod script may call. The list is fixed when the mod loads.
_Avoid_: A connection grant, a capability the script can add to itself.

**Pin**:
A replicated mark on a component. Tick passes skip a pinned component until an unpin notice. The predicting runtime honors the same mark.
_Avoid_: A pause of the session, a lock file.

**Prefab**:
A data file of components the authority can spawn as a new entity. The spawn is a notice. The file is not executed.
_Avoid_: Scene handle, a Rhai script, a second entity store.

## Current state

No branch in this repository implements a game session. `git fetch` of `origin` on 2026-10-12 shows `master`, `gi-v2`, `gi-v2-prefill`, `mesh-loading`, and cursor branches for lighting, the agent rubric, environment notes, and one physics plan. The only related design is [the physics pipeline plan](https://github.com/AnthonyPoschen/genos-engine/blob/cursor/physics-architecture-plan-a077/docs/systems/physics/plan.md) on `origin/cursor/physics-architecture-plan-a077`. It is not in this tree. A search of those branches finds no entity store, no snapshot codec, and no game socket. `UdpSocket` does not appear. The sockets that do exist are the loopback MCP listener.

### Scene model

`Scene` (`crates/scene/src/types.rs`) is one `Floor`, a `Vec<Wall>`, a `Vec<Solid>`, a `Vec<Light>`, an optional `Ceiling`, and an optional `Sky`. None of those structs stores an id. `Shape` is `Square` or `Circle`. `solid_by_color` finds a solid by a color epsilon, which is a search key, not an identity.

`Camera` (`crates/scene/src/camera.rs`) owns a `genos_physics::World`. `view` is a `usize` index of the capsule. `attach_scene` inserts the floor, every wall, every solid, and then the capsule, and remembers `solid_origin` as `1 + walls.len()`. A solid's body is "that many slots after the walls." Removing a wall from the middle of the `Vec` would change every later index. Codimation binds a `solid: usize` (`BoundCodimation` in the same file).

`update` samples `Actions` (forward, strafe, mouse deltas, stick look, capture, escape), writes a yaw-relative wish at `MOVE_SPEED` (4 m/s), and steps physics when an accumulator reaches `const STEP: f32 = 1.0 / 60.0`. Mouse look is applied as a pointer delta even when the interval is not a step. The frame interval is the measured `dt`, capped at 0.25 s by the camera loop. There is no input history and no tick number.

The renderer caps are `MAX_LAMPS` (4096) and `MAX_OCCLUDERS` (8192) in `types.rs`. Those caps are lighting limits. They are not a replication budget. `docs/systems/scene.md` and `docs/systems/physics.md` still say the physics world holds `MAX_BODIES` (32) and drops the rest. `crates/physics/src/lib.rs` has no such cap: `World::insert` pushes onto `bodies: Vec<Body>`. The physics plan records that correction and it has not landed here. Do not design the session around a 32-body world, and do not treat the stale sentence as the code.

### Physics, as it stands on this branch

`Body` is `Copy`: position, orientation, velocity, inverse mass, restitution, friction, shape, an optional spring, `motor`, and `wish`. There is no body id, no angular velocity, and no island. `insert` returns the `Vec` index.

`step` (`lib.rs`) applies gravity and a compressed spring, copies `wish.x` and `wish.z` onto the velocity when `motor` is set, integrates `velocity * dt` for dynamic bodies, and runs `contact::collide` four times. Orientation does not change. The first step may allocate the contact scratch list. `timed_math_and_physics_batch_does_not_allocate` (`crates/physics/tests/bench.rs`) asserts that a later step of a prepared world allocates nothing.

`contact::collide` (`crates/physics/src/contact.rs`) builds an AABB, expands it by `BOUNDS_MARGIN` (0.02), sorts by minimum X with `total_cmp`, sweeps, and sorts pairs before solving them. Pair order follows body index. The same bodies in the same order are deterministic inside one process. `atan2` is used while building a hull (`mesh.rs`), so a hull built on another platform is not a golden hash. `genos-math` is `f32` (`docs/systems/math.md`). There is no cross-machine determinism claim, and ADR 0007 keeps `libm` out.

### What the physics plan already decided

On `origin/cursor/physics-architecture-plan-a077`, `docs/systems/physics/plan.md` (parent `e61b89d`) locks:

- `BodyId { index: u32, generation: u32 }`, stable across removal, generation bumped on reuse (its step P2).
- A fixed `1/60` s step inside the accumulated frame. Sub-steps divide that step. They are not extra network ticks.
- `hash_world`, FNV-1a over body id order, in-process bitwise replay. A cross-OS golden hash is explicitly not claimed (its step P14). `std` trig stays off the solver path.
- `step_parallel` may run only when its hash equals `step` (its steps P23 and P24). The parallel work uses `std::thread::scope`. There is still no general job system.
- `WidePos` as `f64` metres inside the physics crate, with `shift_origin` so contact runs in `f32` near the origin (its step P21). `genos-math::Vec3` stays `f32`.
- Ray and shape casts keyed by `BodyId` (its step P13), which lag compensation will call once they exist.
- Thousands of bodies, with asleep islands absent from the solver (its steps P12 and P26).

That plan does not mention a session, a snapshot, or a player runtime. This plan is the other half of the same id. `EntityId` has the same layout as that `BodyId`. One allocator, in `genos-sim`, assigns it. A body that is an entity uses those bits as its `BodyId`. N24 is the commit that wires them, and it waits until `BodyId` and `hash_world` are public in `genos-physics`.

### Scripting

The host is `crates/scene/src/script.rs`. `load_path` and `load_str` run Rhai 1.26.0 (`crates/scene/Cargo.toml`, `rhai = "=1.26.0"`) and return a `Scene`. The registered names place a floor, walls, solids, lights, and a sun. `docs/systems/scripting.md` says there is no mod folder, no hot reload, and no gameplay script. ADR 0009: a script calls registered functions and does not own the window, the input, the renderer, the audio, or the network.

Today a tick pass does not call Rhai. [Mods](#mods) is the decision that changes that for one authority pass: a checked script enqueues notices, and the player runtime does not run it. A script inside prediction is still refused until that pass is proven. The checked engine and the fuel limit are what keep a runaway mod off the tick.

### Input

`genos-input` (`crates/input/src/lib.rs`, `system.rs`, `action.rs`) is a pull. Each frame writes the keyboard, the mouse, and the gamepad slots, then reads an `ActionMap`. `character_controller` binds `move`, `look`, `capture`, and `release`. Stick values inside `DEFAULT_STICK_DEADZONE` (0.08) are zero. There is no sequence number, no ring of past actions, and no remote command. Mouse motion is a delta: sampling it twice in one tick would consume it twice, so the command path samples it once.

### Jobs

There is no gameplay job system.

- `crates/debug/src/tools.rs` has a `Job` enum (frames, shots, a reference trace). The frame thread drains it. That is the debug command queue.
- `crates/bake/src/sdf.rs` `parallel_map` spreads offline SDF and card work across `std::thread::scope`, then sorts the chunks back into order. It does not tick a game.
- `crates/debug/src/reference.rs` traces paths on a worker thread. It does not simulate bodies.
- The physics plan's later steps color constraints and solve one color in parallel, then require the same hash as the serial step. Those steps are not in this tree.

Replication, the snapshot ring, and prediction replay stay on the tick thread. A future job system may parallelize tick passes that write disjoint components. It must join before the snapshot is built, and the replay path must call the same serial order the hash was defined on. This plan does not add that scheduler.

### Networking that exists

`genos-mcp` (`crates/mcp/src/lib.rs`) binds `127.0.0.1` port 8765 (`GENOS_MCP_PORT` overrides it) and speaks MCP over HTTP (`crates/mcp/src/http.rs`, `TcpListener`). `Handles` in `crates/mcp/src/live.rs` assigns a process-local `u64` starting at 1, separately for walls, solids, and lights, and formats `wall:<id>`, `solid:<id>`, `light:<id>`. The floor handle is the string `floor`. The handle lives in the endpoint, not on `Wall` or `Solid`. It stays until removal. It is not sent to another machine. `docs/architecture.md` and `docs/goals/mvp-scene.md` both say gameplay networking waits. ADR 0007 already names the socket rule for when it stops waiting.

`docs/systems/loading.md` says a network payload and a disc payload share the decoders after the bytes arrive. Loading does not open a socket.

Nothing in the tree sends a pose, a command, or a delta. Nothing records that stream to a file or to a spectator.

## Gaps

| Piece | In the tree today | This plan adds |
| --- | --- | --- |
| Identity | `Vec` index in the scene and in physics. MCP handle on the side, process-local. | Numeric `EntityId` shared with `BodyId`, plus a session-unique entity name for tools. D20–D22. |
| Simulation store | `Scene` fields and `Body` fields. Behavior sits on `Camera`. | Components as data. Tick passes as the behavior. |
| Tick | Accumulator inside `Camera::update`. No tick number. | A `1/60` s command frame with a `u32` tick. |
| Replication | None. | Dirty tick per component, delta against the last acked snapshot, 32-deep history. |
| Interest | The renderer culls draws. Every solid is still in the `Scene`. | A 32 m grid, an always-relevant set, priority, a byte budget. |
| Authority | One process, one player, trusted by construction. | Headless authority runtime. Player runtime predicts only its pawn. |
| Reconciliation | None. | Restore the owned entity at the snapshot tick and replay commands. Remote entities are not resimulated. |
| Lag compensation | None. | Rewind hit volumes to the aim tick, query, restore. |
| Determinism | In-process contact order. No hash. Physics plan P14 not merged. | Integer poses in `genos-sim`. Physics hash stays the physics gate. The net checksum is the quantized pose. |
| Ownership | The local camera is the only mover. | `InputAuthority(ConnectionId)` assigned by the authority, transferable at a tick boundary. |
| Messages | MCP JSON-RPC on loopback. | Commands (unreliable, redundant) and notices (reliable, ordered). Results are components. |
| Interpolation | Codimation eases a local path. | A snapshot buffer. Render remote entities 100 ms behind. Hold on a gap. |
| Headless process | `examples/stress` can render without a visible monitor. It is still a picture tool. | `genos-session`, no window crate, the process Genos starts. |
| Scale test | Hundreds of static pillars in one physics test. | 4096 entities, 8 connections, a far entity absent from the packet. |
| Debug view | Lighting and MCP screenshots. | One process, authority plus player runtimes, seeded loss, desync rows. |
| Replay | None. No file, no spectator stream, no per-tick hash on the wire. | The session datagram stream, to a file or to a spectator connection, with keyframes and per-tick hashes. An agent seeks that file. D1–D19 are open. |
| Agent control | The camera endpoint edits scene handles and can subscribe to `genos://scene`. It does not see a simulation entity, a cause, or a replay tick. | An agent connection. Query, inspect, entity log, subscribe, and spawn, edit, despawn as notices. D20–D29 are open. |
| Live edit | The camera endpoint mutates the `Scene` in the camera process. There is no pin, no undo stack, and no save of entity components. | Tool notices applied at a tick boundary, pinned so the next pass does not erase them, undo by inverse notice. D30–D31, D36. |
| Save-back | A `.rhai` scene is hand-written and loaded once. Nothing writes it back. There is no prefab. | An explicit authority-side sidecar and a data prefab. The script stays. D32–D35. |
| Lockdown | No profile, no grant, no audit. Any future tool would be linked into every binary. | Dev, staging, and release. The player binary compiles the tool out. The authority checks a grant. The mod loader stays. D37–D42. |
| Mods | A scene script places a room. `docs/systems/scripting.md` says there is no mod folder and no gameplay script. | Data overrides, a checked Rhai pass on the authority, load order, hash-checked distribution. D43–D49. |

## Decision

Use a small in-repo entity store. An entity is an id plus a name. A component is state with no functions. A tick pass is behavior with no stored simulation state. The id in the store is the id in the snapshot. This is an ECS in the sense of the Overwatch talk. It is not a Bevy app, and it is not an actor graph. The owner asked to judge that choice by agent observability. The score is in [How the choices score](#how-the-choices-score). The recommendation stands. A recording is the replication stream of this store, and it is the stream the agent seeks.

Three shapes were compared again against that test.

**Typed arrays, replicated as the scene.** `Scene` and `World::bodies` would be the snapshot. Quake and Source also ship a flat entity list, so the packet idea fits. The store does not. Indices change when a wall is removed, so an entity log and a name cannot stay attached. MCP handles are a second map that physics does not read, and the camera endpoint writes those arrays directly, which skips the authority. A prediction replay would have to clone the camera, the script, and every body to correct one pawn. Interest management would be a filter bolted on after the `Vec` exists. The physics plan is already replacing the index with `BodyId`. Building replication on the index would throw that away. This shape fails the agent test.

**Bevy, with lightyear, renet, and bevy_replicon.** Queries and observers are the closest public match to "find entities and hear changes". They are listed below as sources. They do not fit this repository. ADR 0007 says a third-party crate does not own the network, and it rejects `tokio`. Those crates own the socket, the reliability layer, and, in Bevy's case, the schedule, which would become a second runtime beside `examples/camera`. Lightyear 0.28 moved its replication backend onto bevy_replicon, so depending on "the Bevy stack" is one ecosystem, not three small libraries. The engine would inherit Bevy's entity ids, which are still opaque index-and-generation values, and then still have to reconcile them with `BodyId` and still add a name component. An agent would talk to that runtime instead of sending a session notice, so control would not be the same path as a connection, and time-travel would be a second replay. This shape serves the queries and fails the authority path.

**This store.** `EntityId` matches the physics plan's `BodyId` layout so a body and its replicated pose cannot disagree about identity. The name is a component on that id, which is what the agent types. Columns are the query: type, tag, pose, health. The entity log and the cause hang off the same id, and a tick pass name is a cause an agent can read. Components are also the unit of dirty state, of a snapshot field, of a rewind, and of an interpolation sample. Tick passes are the unit of prediction: the same function runs on the authority and, for the owned entity, on the player runtime. Overwatch used that split so a rewind of `MovementState` did not have to rewind unrelated behavior. We have the same need at a smaller size: one pawn is predicted, a door that was shot is rewound for the query, and a lamp is neither. The agent reads the authority columns, not the predicted pawn and not the culled packet.

What we refuse inside that choice:

- A general archetype framework and a job scheduler. Thousands of entities need a dense column per component and a grid for interest. They do not need a scheduler before the first session. Overwatch's own store was a map from a 32-bit id to components, and systems scanned for the tuple they required. Columns here are `Vec`s indexed by `EntityId.index`, with the generation checked on every read.
- Peer-to-peer GGPO of the whole world. GGPO saves the full simulation, predicts remote inputs, and resimulates when the real input arrives. That requires every peer to simulate every entity, bit-identically, across machines. We do not have cross-machine determinism, and a large world should not ship every entity to every player. We take GGPO's restore-and-replay loop and apply it to the owned entity.
- Client-predicted spawns. A player connection's spawn notice is refused. The authority is the only writer. An agent connection may request a spawn, and the authority applies it at the tick boundary (D29). The agent does not mint an id locally. A temporary player-side id is a later plan.
- Shared-mode state authority, where a player runtime owns the true state of an object. Photon Fusion's dedicated-server mode keeps state authority on the server. That is the mode we copy. Input authority can move. State authority stays on the authority runtime.

## Sources

The packet and the prediction loop are old. The entity store is what makes them one design instead of a codec wrapped around `Scene`.

### Overwatch (Timothy Ford, GDC 2017)

Talk: ["'Overwatch' Gameplay Architecture and Netcode"](https://gdcvault.com/play/1024001), GDC Vault. The notes below follow the written record of that talk at <https://www.lfzxb.top/ow-gdc-gameplay-architecture-and-netcode/>, checked against the Vault summary. The Vault video is the primary source. Where a number comes from the written record and not from a public paper, the step below treats it as a design input, not as a constant we must copy.

- An entity is an id. Components store state and have no behavior. Systems (our tick passes) have behavior and have no simulation state. A connection is a component on the player's entity, which is how a pass knows "this is a player" without a class hierarchy.
- The command frame is fixed: 16 ms in the retail game, 7 ms for their esports setting. The client runs ahead by about half a round trip plus a small buffer. A ring of predicted states and a ring of inputs make a misprediction a restore plus a replay, not a pop to a stale pose.
- Inputs are redundant. Each packet carries every command since the last acknowledged state, the same idea as QuakeWorld. The server, if it has no fresh command, repeats the last one for a short buffer. We take the redundant window. We do not take their clock slewing (running the client at 15.2 ms for a while to refill that buffer). The tick stays `1/60` s. The limit is recorded under Limits.
- Remote entities interpolate `MovementState`. The shooter rewinds those components to the command's frame for the hit query, then the damage queue runs separately. Damage is not predicted, because the client is not trusted. A high ping (they cite about 220 ms) stops hit prediction. Our rewind cap is 16 ticks, about 267 ms, and the hit result is a component the authority writes.
- Structural changes are deferred so a pass does not mutate the entity set while it is scanning. `Sim::spawn` during a tick pass is queued and applied at the end of the tick. N3 tests that.

### Quake 3 snapshot delta

Fabien Sanglard, [Quake 3 network model](https://fabiensanglard.net/quake3/network.php). Protocol notes: <https://www.jfedor.org/quake3/>.

- The server keeps the last 32 snapshots per client. A packet is a delta against the newest snapshot that client has acknowledged. If none is in the ring, the delta is against a zero baseline, which is a full update.
- A lost snapshot is not retransmitted. The next delta uses the older ack and is larger, and it still rebuilds the current state.
- Entity numbers are sorted before the delta is written, so the encoding does not depend on hash iteration order.
- There is also a Huffman pass on the datagram. We do not take it. The first codec is the explicit byte layout under Locked names. Bit packing can be a later plan once the tests pin the fields.

### Valve Source

[Source Multiplayer Networking](https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking) and [Lag Compensation](https://developer.valvesoftware.com/wiki/Lag_compensation). The implementation lives in the Source SDK (`player_lagcompensation.cpp`, `interpolatedvar.h`).

- Simulation can run faster than the snapshot rate. Source clients default to about 20 snapshots per second (`cl_updaterate`). We simulate at 60 Hz and send a snapshot every third tick.
- Interpolation delay defaults to 100 ms (`cl_interp 0.1`) so one lost snapshot still leaves two samples. We use 6 ticks, which is 100 ms. Source will extrapolate for a short gap (`cl_extrapolate_amount`, about 0.25 s). We hold the last pose. Extrapolation hides loss and then pops. It can be added after the hold is tested.
- Lag compensation stores recent player positions (on the order of a second, `sv_maxunlag`) and moves hitboxes back to the command's estimated time, including the interpolation delay, then restores them. A disagreement above about 200 ms falls back to a latency estimate. Our cap is the 16-tick history, and the aim tick is what the player runtime puts in the command. We do not resimulate the world to answer the hit.

### Unreal Iris and the replication graph

- Replication Graph, Epic's write-up: [Replication Graph overview](https://www.unrealengine.com/en-US/tech-blog/replication-graph-overview-and-proper-replication-methods) and the [4.27 docs](https://docs.unrealengine.com/4.27/en-US/InteractiveExperiences/Networking/ReplicationGraph/). A spatialization node stores the actors visible from each grid cell, so a connection gathers a list instead of distance-testing every actor. Always-relevant actors sit on their own list. Dormant actors stay off the per-connection list until they wake. Fortnite used this for large matches.
- Iris, [Epic's replication system docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/iris-replication-system-in-unreal-engine) and [prioritization](https://dev.epicgames.com/documentation/en-us/unreal-engine/iris-prioritization-in-unreal-engine). The 5.7 docs still mark Iris experimental; the 5.8 docs still document it. Unreal Fest Orlando 2025 ("Developing and Launching a New Replication System") describes the server tick as filter, poll and copy, quantize, prioritize, serialize. Priority accumulates until the object is sent, then resets, so a low priority still ships eventually. A per-tick bit budget stops the packet. Push-model dirty state avoids comparing every property.
- We take the grid, the always-relevant set, accumulated priority, the byte budget, and writer-marked dirty state. We do not build a graph of nodes, dormancy lists, or NetGUIDs. An entity whose dirty tick is older than the acked baseline is already absent from the delta, which is what dormancy was buying. Iris exists because an actor poll did not scale; we do not start with actors and then replace them.

### Photon Fusion

[Fusion 2 player input](https://doc.photonengine.com/fusion/current/manual/input/player-input) and [network objects](https://doc.photonengine.com/fusion/current/manual/network-object).

- In client-server mode the server is always the state authority. The client that produces input is the input authority. `GetInput` succeeds for those two and fails for everyone else, who interpolate.
- The tick is fixed. On a corrected state the input authority rolls back and resimulates `FixedUpdateNetwork` from that tick. Proxies do not.
- Shared mode lets a peer take state authority. We do not copy shared mode. Input authority is the only thing that moves, and only the authority runtime writes that component.

### Bevy crates

- [lightyear](https://github.com/cBournhonesque/lightyear) (docs.rs `lightyear`): server-authoritative replication, prediction with rollback, snapshot interpolation, and an in-process link for tests. Authority transfer is a server-side broker; a peer does not assign itself the true state. Releases in the 0.28 line moved replication onto bevy_replicon. Deterministic replication (replicate inputs only, simulate everywhere) is a mode they added and we do not use, because it requires the cross-machine determinism the physics plan refuses.
- [bevy_replicon](https://docs.rs/bevy_replicon): replication flows from the server to clients. Clients send messages, not state. Structural updates (spawn, despawn, insert) go on a reliable ordered channel. Component mutations go on an unreliable channel that carries the latest values since the last ack, because a lost reliable mutation would resend stale bytes. Visibility is a filter. The transport is swappable.
- [renet](https://github.com/lucaspoffo/renet) and `bevy_replicon_renet`: reliable and unreliable channels on UDP. `bevy_replicon` also documents quinnet, aeronet, and matchbox backends. renet would own our socket, so it stays a source. We copy the split: snapshots are unreliable and not retransmitted; notices are reliable and ordered; the socket type is `std::net::UdpSocket`.

### GGPO

[pond3r/ggpo](https://github.com/pond3r/ggpo). Save state, predict inputs you have not received, and resimulate from the first wrong frame when they arrive. It is the right loop for two deterministic peers that both simulate the whole match. It is the wrong topology for a hosted authority and a large world. N13 uses the loop on one entity.

## Architecture

```text
authority tick                         player runtime tick
--------------                         --------------------
take commands for this tick            sample the action map once
run tick passes                        send the unacked command suffix
mark dirty ticks                       run the tick pass on the owned entity only
every 3rd tick:                        store the predicted pose at this tick
  interest, priority, budget           apply a snapshot:
  delta vs last acked snapshot           remote poses -> interpolation buffer
  send, store in a 32-deep ring          owned pose disagrees -> restore and replay
store hit volumes for 16 ticks         render owned pose; lerp others at now - 6 ticks
```

The map is the scene script, loaded by every process from the same bytes. Static geometry is not in the per-tick snapshot. Dynamic components are.

### Identity

`EntityId` is `{ index: u32, generation: u32 }`. The authority runtime calls `spawn`. The index may be reused; the generation increments, so a stale snapshot cannot resurrect a removed entity. A read whose generation does not match returns "absent."

Index 0, generation 1, is reserved for the origin entity. Its component is the sector. Every other pose is millimeters relative to that sector. Reconstructing a world position in millimeters is `sector_x * SECTOR_MM + x_mm` (and the same on Z). Shifting the sector subtracts the same delta from the stored millimeters, so a pawn near the player stays a small integer. That is the replicated form of the physics plan's `shift_origin` (P21). Until N24, nothing calls the physics shift. The sector math still exists, and its test does not need physics.

The player runtime creates an entity only when a snapshot includes a spawn of that id and generation. It does not mint ids. Scene handles stay in `genos-mcp`. A debug label may print `solid` next to an entity; the label is not the id.

`BodyId` on the physics branch, once it exists, is these same bits for an entity that has a body. `genos-sim` owns the allocator. Physics does not allocate a second id for a replicated body. N24 is invalid until that API exists. Earlier steps do not link `genos-physics`.

### Components, dirty state, deltas

A component is a plain value in a column `Vec`, indexed by `EntityId.index`, with the generation stored beside it. The first components are `Pose`, `Origin` (on entity 0 only), `InputAuthority`, `StunUntil`, and `Health`.

The writer sets `changed_tick` to the current tick. A reader does not. The snapshot builder skips a component whose `changed_tick` is less than or equal to the baseline tick. That is the dirty set. It is not a `memcmp` of an `f32`.

The delta is against the connection's last acknowledged snapshot, or against a zero baseline when that snapshot has fallen out of the 32-deep ring or was never acked. Entities are encoded in ascending index order. A lost datagram is not retransmitted. The next delta uses whichever ack is still in the ring and still rebuilds the latest tick. Applying a delta is idempotent for the same baseline and the same payload.

Quantized pose fields are the replicated fields. `Pose` is already integer, so the kinematic stand-in has no `f32` to quantize. N24 quantizes a physics `f32` position into that `Pose` at the end of the physics step. A poke smaller than a millimeter does not change the checksum.

Checksum: FNV-1a 64-bit, offset basis `14695981039346656037`, prime `1099511628211`, over entities in index order, skipping absent slots. Fold generation, `x_mm`, `y_mm`, `z_mm`, yaw, the origin sectors, input-authority connection (or `0xffff`), `stun_until`, and health. Two sims that took the same commands produce the same checksum. The physics `hash_world` is a different hash of the `f32` body. Both are checked in N24. A cross-machine golden is not claimed.

### Interest, priority, bandwidth

Cell size is 32 m, computed in millimeters: world millimetre `sector * 1_024_000 + x_mm`, cell `= world_mm / 32_000`. A connection is relevant to its view cell and the eight neighbors, plus any entity marked always-relevant, plus the entity it has input authority for.

Always-relevant is a flag, not a second id space. The origin entity is always-relevant.

Among the relevant entities that are dirty against the baseline, each has a `u32` priority. It starts at 0. Each snapshot opportunity, before selection, every such entity gains 1. The entity this connection has input authority for gains 1 more. Selection sorts by priority descending, then by snapshots skipped since the last send descending, then by index ascending. A written entity resets both numbers to 0. A moving pawn therefore sends often, and a neighbor that has waited still overtakes it after a reset. Iris also scales priority by distance inside the relevant set. The grid already did that cut. Distance inside the cell is a later knob, not a step.

The pose budget is 1024 bytes per snapshot per connection. Spawns and despawns are outside that budget: a new id is in the packet even when the pose budget is full, and its later pose updates compete normally. Notices are capped at 4 per datagram and do not consume the pose budget.

A far entity, outside the neighborhood and not always-relevant, is absent from the packet even if it is dirty. That is the scale rule. 4096 stored entities are cheap if a connection's packet names the neighborhood. An agent query does not use this cut. It reads the authority columns, or a replay tick, and can ask for a region the player packet would omit. That read is an `inspect` grant. A player connection does not get the query. The interest set is the anti-cheat boundary for what a player datagram may contain (D41).

### Authority, prediction, reconciliation, rollback

The authority runtime runs every tick pass on every entity. It applies the command whose connection matches `InputAuthority` at the start of the tick. A missing command repeats the last command for at most 4 ticks, then applies a zero command. The repeat is the small server buffer Ford describes. The bound is so a disconnected player does not walk forever.

The player runtime runs the motion tick pass only on the entity it owns. It stores the `Pose` it produced at each tick in a ring of 128. When a snapshot for tick `T` arrives, it compares the snapshot pose to the stored pose at `T`. Equal checksums for that entity drop the predicted states through `T`. An unequal pose copies the snapshot components onto the entity, sets the tick cursor to `T`, and runs the motion tick pass for `T+1 ..= local_tick` with the stored commands. Other entities are not visited by that replay.

`StunUntil` zeroes motion while `tick_count <= stun_until`. The player runtime does not invent that value. The snapshot copies it onto the owned entity, and the replay honors it. A stun is how the test shows a fact the predictor had not seen, such as a hit or a closed door.

Rollback does not resimulate remote entities, does not rerun interest, and does not rewind the authority. GGPO's full-state rollback is this same restore-and-replay, scoped to one entity, which is the scope that stays cheap at thousands of entities.

Damage is not predicted. `Health` changes only on the authority, and the player runtime displays the snapshot value. Ford's rule, and the one we can test without a combat game.

### Lag compensation

The authority stores the `Pose` of every entity that has `Health` for the last 16 ticks. A command with the strike bit set carries `aim_tick`, the tick the player runtime was rendering (the interpolation time), not its predicted tick. The authority clamps `aim_tick` into `[now - 16, now]`, copies those stored poses over a scratch list, runs a segment test, writes `Health` on a hit, and leaves the live poses untouched. The segment test in N15 is an integer ray in the sim crate. When physics P13 `ray_cast` exists, N24's follow-through calls that cast against bodies placed at the rewound poses. The live `step` result is unchanged either way.

A strike whose aim tick is older than 16 ticks does not rewind. The command still arrives; the hit does not. That is the high-ping cutoff, in ticks instead of the 220 ms figure.

### Determinism and the physics plan

Inside `genos-sim` the tick is integer. The same commands produce the same checksum in one process, including across a seeded lossy link, because the link decides delivery and the tick consumes commands, not arrival time.

The kinematic stand-in used until N24, at yaw 0:

- `strafe` of `127` adds `67` mm on X per tick. `forward` of `127` adds `-67` mm on Z per tick.
- `delta_mm = axis as i32 * 67 / 127`, truncating toward zero.
- Any other yaw leaves translation on those axes. Yaw itself replicates. N24 replaces this translation with the physics step. The stand-in exists so prediction tests do not wait on the physics branch.

N24, once `BodyId` and `hash_world` are public:

- The pawn's body id equals its `EntityId`.
- The authority and the predicting runtime both call `genos_physics::step` at `1/60`, then quantize the body's `f32` position into `Pose` (millimeters, truncation toward zero). Sub-steps stay inside `step`. They are not extra ticks.
- Two serial runs hash equal under `hash_world`, and their `Pose` checksums match.
- A `1e-6` m poke of a body position does not change the `Pose` checksum.
- If `step_parallel` exists, the session may call it on the authority only when its `hash_world` equals `step` for that world. The predicting runtime calls `step`. A mismatch means the session calls `step` on both. Prediction that used a different parallel schedule would false-correct every tick.
- `shift_origin` and the origin entity's sector move in the same tick. The reconstructed world position is unchanged across the shift.
- If `ray_cast` is public, the strike path places bodies at the rewound poses, calls `ray_cast`, and asserts the live body position is unchanged. If it is not public, the hit path stays the integer segment test from N15.

Cross-OS bitwise physics is not a goal. ADR 0007 and the physics plan already say why. The player runtime is allowed to be wrong. The snapshot corrects it.

### Ownership and transfer

`InputAuthority` is `Option<ConnectionId>`, written only by the authority runtime, replicated in the snapshot. `None` means every player runtime interpolates the entity. `Some(id)` means that connection predicts it and sends its commands.

A transfer written during tick `T` is visible to command selection at tick `T+1`. Commands from the old connection with tick greater than `T` are dropped. Commands from the new connection apply from `T+1`. State authority does not move.

Join assigns a `ConnectionId` (`u16`, from 1) and input authority on that player's pawn. Leave clears the component and stops accepting that connection's commands. Both are notices plus a component write. The component is what the next snapshot says. The notice is how the peer learns the id assigned to it.

### Commands, notices, events

A command is the sampled action for one tick: `tick`, `forward: i8`, `strafe: i8`, `look_x: i16`, `look_y: i16`, `buttons: u8`, `aim_tick: u32`. Bit 0 of `buttons` is the strike. The authority calls the rewind query when that bit is set. The device float is converted at the sample: full deflection is `127` or `-127`, deadzone output stays `0`. Look deltas are the mouse and stick sample for that tick, clamped into `i16`. The action map is not stored in the ring. The command is.

Each player-to-authority datagram carries every command after the authority's last ack, the acked snapshot tick, and the acked notice sequence. That is the QuakeWorld window Ford also describes.

A notice is `{ seq, name: u16, entity, bytes }` with at most 256 bytes of payload. Names in this plan: `1` join, `2` leave, `3` request. A request does not change `Health`. The authority's tick pass changes `Health`, and the snapshot publishes it. Notices resend until the ack covers their sequence, oldest first, at most 4 per datagram. They are ordered: sequence `N+1` is not applied before `N`.

Snapshots do not use that resend list.

### Snapshots and interpolation

A snapshot is sent every third tick (20 Hz) if the connection's dirty relevant set is non-empty, and also whenever a spawn or despawn is pending. The datagram layout is under Locked names.

The player runtime keeps the decoded poses for the last 32 snapshot ticks. The render tick is `newest_snapshot_tick.saturating_sub(6)`. Remote entities lerp millimeters between the two stored ticks that bracket the render tick. Yaw lerps on the short arc of the `u16`, splitting at `32768`. If the bracket has a hole, the pose holds at the older sample. The owned entity is not in this buffer's output; it renders the predicted pose.

### Headless authority

`examples/session`, package `genos-session`, binary `genos-session`. It links `genos-sim`, `genos-net`, `genos-scene`, and `genos-json`. It does not link `genos-window`, `genos-render`, or `genos-audio`.

Arguments: `--scene PATH --bind IP:PORT --frames N`. `N` of 0 runs until stdin reaches end of file. `--frames N` with `N > 0` runs that many ticks on the in-process loop when `--bind` is `off`, and on a real socket when `--bind` is an address. The hosted process uses a real bind. The test uses `--bind off` so it does not need a port, and one separate test binds `127.0.0.1:0` and exchanges one datagram.

The report is one `genos-json` object: `ticks`, `checksum`, `bytes_sent`, `connections`. Genos, in the other repository, is the program that starts this process. This repository does not define a control protocol, a matchmaker, or a lifetime beyond the arguments and stdin. When Genos wants the process to stop, it closes stdin or sends a signal the binary already treats as exit.

`examples/camera` stays a single-player picture. It does not depend on `genos-net`.

### Scale

The storage test inserts 4096 entities with poses spread over a kilometre. Eight connections stand in different cells. Each connection's steady-state snapshot, after the initial spawns have been acked, names only its neighborhood, stays within 1024 pose bytes, and is byte-identical across two runs with the same seed and the same zero loss. A dirty entity outside the neighborhood does not appear.

That is the "thousands of entities and a large world" bar for this plan. It is a relevancy bar, not a claim that the renderer can draw 4096 dynamic occluders. The renderer caps stay where they are.

### Sockets

`genos-sim` does not call `std::net`. The codec writes bytes. `genos-net` owns `Link`, which is the in-process lossy path, and `UdpEdge`, which is `UdpSocket` in nonblocking mode. The session loop reads and writes datagrams. Tests of the codec and of prediction use `Link` only.

`LinkConfig` is `latency_ms`, `jitter_ms`, `loss_percent` (`0..=100`), and `seed`. Time is in ticks inside the test: `latency_ms` converts as `(latency_ms * 60 + 999) / 1000` ticks, so 0 ms is 0 ticks and 100 ms is 6 ticks. Jitter adds `0..=jitter_ticks` from the xorshift below. A later sequence can overtake an earlier one. The receiver drops a datagram whose snapshot tick or packet sequence is older than or equal to the last one applied. Loss rolls the same generator: drop when `(next % 100) < loss_percent`.

```rust
fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}
```

Seed `0` still moves, because of the `| 1`. The test pins behavior (which datagrams drop), not a published random stream.

### Scripting and input, in the tick

The session binary loads the map with `genos_scene::load_path` before the loop. The bridge in N18 builds one entity per solid and one static pose per wall and floor. Those static poses are in the sim on every peer because every peer loaded the script. They are not snapshotted after the join snapshot, unless a tick pass marks them dirty.

No tick pass evaluates Rhai. A gameplay request from a script, later, would be a registered function that enqueues a notice. That function is not in this plan.

The player runtime, once per tick, reads `character_controller` through the action map and writes one `InputCommand`. `examples/camera` is unchanged and still reads the action map itself.

### Debug view

`examples/net-view`, package `genos-net-view`, binary `genos-net-view`. One process holds one authority `Sim`, up to four player `Sim`s, and one `Link` per player. The cap of four is the picture. The sim's scale test still uses eight connections without this window.

Headless mode: `--headless --players N --frames N --latency MS --loss PERCENT --jitter MS --seed U --metrics PATH`. It runs the in-process links, writes a `genos-json` report, and does not open a window. The report's `rows` are the desync rows: entity id, authority millimeters, that player's millimeters, and the correction count. Two runs with the same arguments write the same bytes.

Window mode, the same binary without `--headless`: a `genos-ui` `button_panel` (`crates/ui/src/panel.rs`) with one `PanelSlider` per knob.

| Row | Slider | Range | Step |
| --- | --- | --- | --- |
| Latency | `latency_ms` | 0..=500 | 10 |
| Loss | `loss_percent` | 0..=100 | 1 |
| Jitter | `jitter_ms` | 0..=100 | 5 |
| Players | `players` | 1..=4 | 1 |

A button reseeds from a counter the test can set, so the seed is not taken from the clock. The picture uses the UI crate's rectangles: for each entity, a mark at the authority XZ and a mark at each player runtime's rendered XZ. A row whose millimeters differ by 1 or more uses the panel's selected look. The correction count is the slider readout text, not a new text renderer. No Vulkan debug pass and no new shader. The window mode is N23. Its test does not open a display: it drives `LinkConfig` from the same slider values `button_panel` would produce, runs one session, and checks the report.

Desync means the predicted pose at the acknowledged tick differed and the owned entity was restored and its commands were rerun. The view shows that count. It does not show a physics `hash_world` unless N24 has filled that field in the report. When it has, the row gains `physics_hash` for the authority and for the player, and a mismatch is a row even if the millimeters were quantized equal. That last case is the "sub-millimeter dust" the quantization is there to ignore, so the view shows it and the test asserts the net checksum still matches.

N28 adds replay controls to this same binary. The scrub control is a tick integer. Headless mode takes `--replay`, `--tick`, `--step`, and `--resim`. The slider test still does not open a window.

## Replay

D1–D19 are the recording decisions. D20–D49 are the agent, live-edit, lockdown, and mod decisions. This section is the argument, the sources, and the way the recommendations sit on the datagram layout above. An agent seeks this same file. Nothing here is built.

### One stream

A live authority tick already produces the bytes a replay needs. Commands arrive from the player. Notices carry join, leave, and request. Snapshots carry the component results, including `Health` and `StunUntil`. Writing those datagrams to a file, or sending them to a spectator connection, is the recording. Reading them back calls the same `apply` the player runtime calls for a socket. `genos-sim` does not open files and does not call `std::net`. `genos-net` owns the recorder beside `Link` and `UdpEdge`, and it uses `std::fs` for the file. ADR 0007 still holds: the standard library, no crate that owns the network.

Unreal's replay system is the same shape at a larger size. A `DemoNetDriver` reads the replication stream the way a live `NetDriver` does, hands it to a Replay Streamer, and playback reconstructs the session from that data. `demoscrub` jumps to a time. A game with no remote peer can still record, as long as the data is replicated ([Using the Replay System](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-the-replay-system-in-unreal-engine)). That is the reason D16 records `--bind off`. Unreal also allows fields that are relevant only to replays. This plan does not. A replay-only component would be a second stream.

Source demos wrap the same idea in a file header (demo protocol, network protocol, map name) and `dem_packet` records of raw network packets. Playback feeds those packets through the client net channel (the player runtime's apply path). The map name in that header is why D10 embeds the scene script: a bug file has to open on a machine that does not already have the path loaded. The script runs in the existing loader before the tick loop. It does not encode entities.

### Input log, snapshot log, or both

An input log is the RTS replay and the fighting-game training mode. StarCraft and Warcraft III store the commands and a version. GGPO's training use is the same loop in miniature: an input log plus savestates, which is a keyframe ([pond3r/ggpo](https://github.com/pond3r/ggpo)). Playback is cheap to store and exact when every machine runs the same sim. This repository does not have that sim. The physics plan's `hash_world` is an in-process bitwise check, and it refuses a cross-OS golden hash. A file recorded on one machine and resimmed on another can diverge with every command still intact. Shipping playback cannot depend on input-only resim. The input log stays in the file because the debug question is "rerun these commands."

A snapshot log is a Quake or Source demo: the deltas the protocol already sends, applied in order. A lost baseline becomes a zero-baseline full update, which this plan's codec already writes. Playback survives a sim-epoch change. It cannot tell you which command diverged, because the commands were not kept. Seeking without a keyframe means applying every delta from the start of the file. Rewind means doing that again from the start. That is correct and slow, and it is the whole of option D1.B.

The hybrid keeps both, and writes a zero-baseline datagram on an interval. Watching applies snapshots, so the picture is what the network said. A debug resim starts at the last keyframe, reruns the stored commands, and compares hashes. The keyframe is not a new codec. Baseline tick `0` is already the full update in [Locked names](#locked-names).

### Keyframes, seek, scrub

Unreal writes checkpoints on a timer. The documented default of `demo.CVarCheckpointUploadDelayInSeconds` is 30 seconds. A longer gap makes the file smaller and makes a backward scrub apply more deltas. Thirty seconds is a long way to walk to look at one tick, which is why D3 recommends 120 ticks.

Unreal can also spread a checkpoint across later frames (`demo.CheckpointSaveMaxMSPerFrame`). Actors that miss the time limit are queued, and the checkpoint then mixes frames. A forum report of killcam recording describes `TickDemoRecordFrame` stopping while a checkpoint saves, which leaves gaps in the recorded traffic ([TickDemoRecordFrame during checkpoint save](https://forums.unrealengine.com/t/demonetdriver-tickdemorecordframe-not-called-during-checkpoint-saving-ultimately-resulting-in-missing-network-traffic/2600457)). This plan writes the keyframe in the tick that owns it. Later ticks keep their own datagrams. A hitch is preferable to a hole.

Seek to tick N loads the greatest keyframe with tick `<= N`, applies it, and applies deltas through N. Rewind is that seek aimed backward. Scrub is that seek driven by a tick integer. Inside the debug binary, once a bracket is decoded, moving within those 120 ticks uses the in-memory poses. The file has no reverse-simulation path.

### Killcam and live spectators

A killcam is a short replay played as if it were live. The Overwatch notes describe about 8–12 seconds of network data handed to replay. D5 recommends 10 seconds from the authority's omniscient stream, so the attacker is in the slice even when the victim's interest set omitted them. The victim's own file remains the record of what that runtime was sent (D8). The camera starts on the attacker's pawn and is local (D19).

A public spectator is a connection with no input authority. The authority writes the stream at once. The spectator socket receives it after the header's delay. SourceTV's `tv_delay` is this queue. A league such as RGL sets the delay on the order of 90 seconds so a spectator cannot ghost. The engine default in this plan is 10 seconds, and the operator can raise it. The file does not wait. A recorder that stored only the delayed bytes would hold the tail in memory and drop it on a crash; the immediate authority file is what makes a stop keep the tail.

A coach connection is the cheap form: player-like interest, the pose budget, no commands. It exists beside the public stream. It is not a second protocol.

The omniscient queue is the memory cost. A world in which every snapshot is a full baseline is the 2.8 MB/s figure from D3: about 28 MB for a 10 second killcam and about 250 MB for a 90 second delay. A quiet world is far smaller, because clean entities stay out of the delta. If that upper bound is unacceptable, the answer to change is D6, toward interest-limited spectators, or D7, toward a shorter delay. A spectator delay queue is not a numbered step. Numbering it before D6 and D7 are answered would let an implementer start it.

### Hashes, desync, versions

The state hash is the checksum N5 defines. It folds the replicated columns in index order. `hash_world`, once N24 can see it, is a second word and a different hash. The two answers to "did this tick match" are not interchangeable. The net checksum can match while the physics hash differs; the debug row already plans to show that.

Hashes on snapshot ticks only hide a divergence on the ticks in between. The owner asked for per-tick hashes. D11.C puts one in every file frame, and puts the snapshot tick's hash on the live datagram so a player runtime can compare when a snapshot arrives. Live bandwidth does not grow by 60 empty datagrams a second.

On a mismatch while watching, resim stops and snapshot apply continues, and the report keeps the tick. `--strict` stops. A test that is supposed to catch a bad command uses `--strict`. A person watching a bug uses the continuing picture.

The header separates the datagram protocol from the sim epoch. A layout change refuses the file. A tick-pass change, with the same bytes, still applies snapshots and refuses resim. That is the RTS lesson applied to a hybrid file: an input-only recording would have nothing left to show. The git hash is a label for humans. Compatibility is `protocol` and `sim_epoch`. Until a file has been handed to someone else, a binary reads only the protocol it writes.

### Debug

The path for a bug is: run `genos-net-view` or `genos-session`, write the file, open the same binary with `--replay` and `--tick`, print the components at that tick. `--resim` prints the hash from the commands and the hash stored in the frame. `--step` advances one tick. Pause and fast-forward call the same functions. Fast-forward applies datagrams in a loop and does not sleep. The desync rows already defined for the live session gain the stored hash, the applied hash, and the tick.

No third binary. No window in the headless test.

### What acceptance changes in the locked layout

If D11.C is accepted, the authority datagram gains `u64 checksum` and `u64 physics_hash` after the notice block. `physics_hash` is 0 until N24. The rest of [Locked names](#locked-names) stays the working layout. D14 does not change those bytes. It stores them in a frame:

- `u32` tick
- `u64` state hash
- `u64` physics hash
- `u32` length
- that many datagram bytes

On a snapshot tick the frame hash equals the checksum inside the datagram. On a tick that sent nothing, the length is 0. The trailer is a list of `(tick, file offset)` for frames whose datagram baseline is 0, rewritten when the file closes.

N25–N28 state the tests for these recommendations. If an open decision moves, edit the step before implementing it.

## Locked names

N1 publishes `EntityId` and `Sim`. Later steps add the rest in the files named on the step. Signatures the tests call:

```rust
pub struct EntityId { pub index: u32, pub generation: u32 }

pub struct Sim { /* columns, tick, rings */ }

impl Sim {
    pub fn new() -> Self;
    pub fn spawn(&mut self) -> EntityId;
    pub fn despawn(&mut self, id: EntityId);
    pub fn get(&self, id: EntityId) -> Option<Pose>;
    pub fn set_pose(&mut self, id: EntityId, pose: Pose);
    pub fn tick_count(&self) -> u32;
    pub fn checksum(&self) -> u64;
}

pub struct Pose { pub x_mm: i32, pub y_mm: i32, pub z_mm: i32, pub yaw: u16 }

pub struct Origin { pub sector_x: i32, pub sector_z: i32 }

pub struct StunUntil { pub tick: u32 }

pub const SIM_HZ: u32 = 60;
pub const SNAPSHOT_EVERY: u32 = 3;
pub const SNAPSHOT_HISTORY: usize = 32;
pub const PREDICT_HISTORY: usize = 128;
pub const REWIND_TICKS: u32 = 16;
pub const INTERP_TICKS: u32 = 6;
pub const CELL_MM: i32 = 32_000;
pub const SECTOR_MM: i32 = 1_024_000;
pub const POSE_BUDGET: usize = 1024;
pub const MM_PER_FULL_AXIS: i32 = 67;

pub struct InputCommand {
    pub tick: u32,
    pub forward: i8,
    pub strafe: i8,
    pub look_x: i16,
    pub look_y: i16,
    pub buttons: u8,
    pub aim_tick: u32,
}

pub struct ConnectionId(pub u16);

pub struct LinkConfig {
    pub latency_ms: u32,
    pub jitter_ms: u32,
    pub loss_percent: u8,
    pub seed: u64,
}
```

Datagram, authority to player, little-endian:

- `u32` packet sequence
- `u32` snapshot tick
- `u32` baseline tick (`0` means a zero baseline)
- `u16` entity records
- records in ascending `index`: `u32` index, `u32` generation, `u8` flags (`1` spawn, `2` despawn, `4` pose, `8` input authority, `16` stun, `32` health, `64` origin)
- if pose: `u8` field mask (`1` x, `2` y, `4` z, `8` yaw), then each masked field (`i32`, `i32`, `i32`, `u16`). A field equal to the baseline snapshot is clear in the mask. A zero baseline writes all four fields. Values are absolute millimeters and yaw, not a numeric subtraction. Omitting the unchanged field is the delta.
- if input authority: `u16`, or `0xffff` for none
- if stun: `u32` `stun_until`
- if health: `i32`
- if origin: `i32` sector_x, `i32` sector_z (flag `64` on the origin entity)
- The flag field stays `u8` until D25 or D30 is accepted. Those decisions widen it to `u16`, with cause at bit `128` and pin at bit `256`. Do not spend bit `128` on both.
- A player snapshot omits the trailing checksum and `physics_hash` (D41). An authority or inspect datagram may include them (D11).
- `u16` notice count, then each notice: `u32` seq, `u16` name, `u32` entity index, `u32` generation, `u16` len, `len` bytes

Player to authority:

- `u32` packet sequence
- `u32` acked snapshot tick
- `u32` acked notice seq
- `u16` command count, then each `InputCommand` as `u32` tick, `i8`, `i8`, `i16`, `i16`, `u8`, `u32` aim_tick

Spawn and despawn records are written even when the pose budget is spent. Pose records stop before the pose section would pass `POSE_BUDGET`.

## File map

| Path | Owns |
| --- | --- |
| `docs/adr/0013-entity-id-is-the-replication-id.md` | The decision. From N1. |
| `crates/sim/src/lib.rs` | `Sim`, `EntityId`, spawn, checksum. Package `genos-sim`. From N1. |
| `crates/sim/src/name.rs` | Entity name, lookup, reserved strings. From N29. |
| `crates/sim/src/query.rs` | Type, tag, region, component presence. From N30. |
| `crates/sim/src/log.rs` | Entity log and cause. From N31. |
| `crates/sim/src/pose.rs` | `Pose`, sector, millimeters. From N5. |
| `crates/sim/src/tick.rs` | Accumulator, tick passes, command application. From N3. |
| `crates/sim/src/snapshot.rs` | Ring, delta, apply. From N7. |
| `crates/sim/src/interest.rs` | Grid, always-relevant, priority, budget. From N9. |
| `crates/sim/src/predict.rs` | Predicted ring, restore, replay. From N13. |
| `crates/sim/src/interp.rs` | Interpolation buffer. From N14. |
| `crates/sim/src/rewind.rs` | Hit-volume history and the segment test. From N15. |
| `crates/net/src/lib.rs` | `Link`, datagram codec, notices. Package `genos-net`. From N8. |
| `crates/net/src/udp.rs` | `UdpSocket` edge. From N20. |
| `crates/net/src/replay.rs` | File header, length-prefixed frames, keyframe trailer. From N25. The datagram slice is the session codec. |
| `crates/net/src/agent.rs` | Agent-connection notices: spawn, edit, despawn, subscribe, undo. From N32. |
| `crates/net/src/allow.rs` | Capabilities, grants, profiles, audit lines. From N40. |
| `crates/mods/src/lib.rs` | Manifest, load order, conflicts, override map. Package `genos-mods`. From N43. No socket. |
| `crates/mods/src/script.rs` | Checked Rhai host. Registered functions only. From N45. |
| `crates/net/src/mods_sync.rs` | Manifest hash and missing-file transfer. From N46. |
| `crates/sim/src/pin.rs` | Pinned components. From N36. |
| `examples/session/src/tool.rs` | Query, inspect, seek, CLI parse, one JSON handler. From N34. |
| `examples/session/src/save.rs` | Sidecar and prefab files. From N38. |
| `examples/session/` | Headless authority. Package `genos-session`. From N21. |
| `examples/net-view/` | Debug binary. Package `genos-net-view`. From N22. |

`genos-sim` depends on nothing in this workspace at N1, and it never depends on `rhai`. `genos-net` depends on `genos-sim`. `genos-mods` depends on `rhai` from N45 and does not depend on `genos-net`. `genos-session` depends on `genos-net`, `genos-scene`, `genos-json`, and, from N47, `genos-mods`. `genos-net-view` depends on `genos-net` and `genos-ui`. The window is not linked into `genos-session`.

## Steps

Do not start N1. D1–D49 are unanswered. The datagram layout, the replay file, and the live-edit notices are the same bytes, so N7 is the commit that would lock the recording and the tool. N25–N28 describe the recommended replay answers. N29–N47 describe the recommended author, agent, lockdown, and mod answers. If a decision above moves, edit that step before implementing it. The steps stay in order: N25 follows N24, and N29 follows N28.

### N1. Entity id

- [ ] **Files:** `docs/adr/0013-entity-id-is-the-replication-id.md`, `CONTEXT.md`, `docs/architecture.md`, `Cargo.toml`, `crates/sim/Cargo.toml`, `crates/sim/src/lib.rs`. Test `crates/sim/tests/ids.rs`.
- **Behavior:** The ADR records the decision in this plan: the entity id is the replication id, the authority runtime writes replicated state, tick passes are Rust, sockets stay in `genos-net` on `std::net`, and `examples/camera` does not open a game socket. It names ADR 0007 and ADR 0009 and does not override them. Rejected in the ADR: a Bevy stack, replicating `Scene` indices and MCP handles, peer-to-peer rollback of every entity, and a tool that writes a column instead of sending a notice. The ADR also records that an agent connection is a connection, and that the camera agent endpoint is not that connection. `CONTEXT.md` gains the words in [Words](#words). `docs/architecture.md` gains one sentence under the crate list: `genos-sim` stores entities, and the game session is `docs/systems/core-networking/plan.md`. The sentence that gameplay networking waits stays true for `examples/camera`. `spawn` returns an `EntityId` with generation 1. Index 0 is the origin and is already spawned. `despawn` makes `get` return `None`. A new `spawn` may reuse the index and must bump the generation. `set_pose` on a despawned id does nothing.
- **Test:** `entity_id_survives_removal_of_another_entity`
  - Spawn three entities and set poses with `x_mm` 1, 2, and 3. Despawn the middle id. The other two still read 1 and 3. The despawned id reads `None`. A new spawn does not compare equal to the despawned id.
- **Commit:** `sim: address entities with generational ids`

### N2. Pose column

- [ ] **Files:** `crates/sim/src/pose.rs`, `lib.rs`. Test `crates/sim/tests/pose.rs`.
- **Behavior:** `set_pose` / `get` round-trip `x_mm`, `y_mm`, `z_mm`, and `yaw`. Clearing a pose (a `None` write, `clear_pose`) leaves the entity spawned and `get` returns a zero pose. Despawning removes the pose. The column does not grow when the same id is written twice.
- **Test:** `a_pose_round_trips_and_a_removed_entity_has_none`
  - Set `(10, -20, 30, yaw 40000)`, read it back equal. `clear_pose`, read zeros, `spawn` identity unchanged. Despawn, `get` is `None`.
- **Commit:** `sim: store a pose per entity`

### N3. Tick passes in a fixed order

- [ ] **Files:** `crates/sim/src/tick.rs`. Test `crates/sim/tests/tick.rs`.
- **Behavior:** `add_pass` appends a pass. `run_tick` runs them in registration order and then applies queued spawns. A pass that calls `spawn` does not make the new id visible until `run_tick` returns. Passes after it in the same tick do not see the new id either. `tick_count` starts at 0 and increments by 1 per `run_tick`.
- **Test:** `passes_run_in_order_and_spawns_land_after_the_tick`
  - Pass A adds 1 mm to entity X. Pass B reads X and writes that value into entity Y. After one tick, Y equals X. A pass that spawns Z and a later pass that writes Z: after the tick, Z exists and still has a zero pose (the write in that tick missed it). A second `run_tick` can see Z.
- **Commit:** `sim: run tick passes in order`

### N4. Fixed step accumulator

- [ ] **Files:** `tick.rs`. Test `crates/sim/tests/tick.rs`.
- **Behavior:** `accumulate(dt_seconds)` runs `run_tick` once per full `1.0/60.0` stored, and keeps the remainder. A non-positive or non-finite `dt` runs nothing. The remainder is the only `f32` in the tick path. It is not in the checksum. This is the same step size as `const STEP` inside `Camera::update`.
- **Test:** `three_steps_of_one_sixtieth_are_three_ticks`
  - `accumulate(3.0 / 60.0)` makes `tick_count == 3`. `accumulate(0.0)` and `accumulate(f32::NAN)` leave it at 3. Two calls of `accumulate(1.0/120.0)` raise it to 4.
- **Commit:** `sim: step the world at one sixtieth`

### N5. Sector and checksum

- [ ] **Files:** `pose.rs`. Test `crates/sim/tests/checksum.rs`.
- **Behavior:** The origin entity stores `Origin { sector_x, sector_z }`, not a pawn pose. `world_mm(id)` on X is `sector_x * SECTOR_MM + pose.x_mm`, and the same on Z. `shift_sector(dx, dz)` adds `dx` and `dz` to that origin and subtracts `dx * SECTOR_MM` from every other entity's `x_mm` (and `dz * SECTOR_MM` from `z_mm`). `checksum` is the FNV-1a fold in [Architecture](#components-dirty-state-deltas), including the origin sectors. Two sims with the same writes match. A 1 mm change changes the checksum.
- **Test:** `a_sector_shift_keeps_the_world_position_and_the_checksum_sees_a_millimetre`
  - Entity at local `x_mm = 1_024_500` with `Origin` at 0. `world_mm` is `1_024_500`. `shift_sector(1, 0)`. `world_mm` is still `1_024_500`, local `x_mm` is `500`, `sector_x` is `1`.
  - A second sim written the same way has the same checksum. Adding 1 mm to one pawn makes the checksums differ.
- **Commit:** `sim: shift sectors and checksum poses`

### N6. Dirty tick

- [ ] **Files:** `pose.rs`, `snapshot.rs` (only the dirty query). Test `crates/sim/tests/dirty.rs`.
- **Behavior:** `set_pose` sets that component's `changed_tick` to `tick_count` (the tick about to be entered; after `run_tick`, a write from outside a pass uses the current `tick_count`). `dirty_since(tick)` lists ids whose pose changed after `tick`, in ascending index order. `get` does not mark dirty. A write of the same millimeters still marks dirty: the writer said it wrote. N7 decides whether the bytes go out.
- **Test:** `a_write_is_dirty_and_a_read_is_not`
  - After one `set_pose`, `dirty_since(0)` is that id. `get` leaves the list unchanged. A second entity that was never written is absent.
- **Commit:** `sim: mark a pose dirty when it is written`

### N7. Snapshot delta

- [ ] **Files:** `crates/sim/src/snapshot.rs`. Test `crates/sim/tests/snapshot.rs`.
- **Behavior:** `take_snapshot(baseline_tick)` writes the datagram in [Locked names](#locked-names) for every entity dirty since `baseline_tick`, plus spawns and despawns since that tick. Inside a dirty pose, a field that equals the baseline snapshot is left out of the field mask. `baseline_tick == 0` writes every live entity and all four pose fields (the zero baseline). The ring stores 32 snapshots. `ack(tick)` sets the baseline. If the acked tick is not in the ring, the next take uses baseline 0. `apply(bytes)` reproduces `checksum` when the receiver already has that baseline. An entity that was not dirty is not in the record list. Records are in ascending index order. If D11.C is the accepted answer, this datagram ends with the `u64` checksum and the `u64` physics hash from [Replay](#replay), and the test below checks the checksum word. Do not land a datagram here that N25 then extends. Physics hash is 0 in this step.
- **Test:** `a_delta_omits_a_clean_field_and_a_lost_baseline_sends_the_full_set`
  - Two entities, change one entity's `x_mm` only. The delta from an ack of the previous snapshot contains that entity, the field mask has bit `1` set, and bits `2`, `4`, and `8` clear. Apply it on a sim that already had the acked state: checksums match, and that entity's `y_mm` is unchanged.
  - Force the ack to a tick older than the ring. The next datagram's baseline is 0 and applying it on an empty sim reproduces the checksum.
  - If D11.C was accepted: the last 16 bytes are the sim checksum at that snapshot tick, then `physics_hash` 0.
- **Commit:** `sim: delta snapshots against the last ack`

### N8. Seeded lossy link

- [ ] **Files:** `crates/net/Cargo.toml`, `crates/net/src/lib.rs`, workspace `Cargo.toml`, `docs/architecture.md` crate list. Test `crates/net/tests/link.rs`.
- **Behavior:** `Link::new(config)` uses the xorshift in [Sockets](#sockets). `send(tick, bytes)` may drop, or deliver at `tick + latency_ticks + jitter_ticks`. `recv(tick)` returns the datagrams due at that tick, and drops any whose sequence is less than or equal to the last returned sequence. Loss 0 drops nothing. Loss 100 drops everything. The same seed and the same send pattern produce the same delivery list. `genos-sim` still has no `std::net` use.
- **Test:** `a_seeded_link_drops_the_same_datagrams_and_keeps_the_newest`
  - Seed `1`, latency 0, jitter 0, loss 0: five sends are received in order.
  - Loss 100: `recv` is empty.
  - Latency 100 ms, jitter 0: a send at tick 0 is received at tick 6 and not at tick 5.
  - Jitter 100 ms, seed `7`: build the delivery set twice and compare it. Swap is allowed. A datagram with an older sequence delivered after a newer one is not returned.
  - Seed `3`, loss 30, latency 50, jitter 20, 40 ticks, one pawn strafing: the authority sends a snapshot every third tick, the player sim applies every datagram `recv` returns and acks the snapshot tick it applied, and the authority's next delta uses that ack. After tick 40 both checksums match. That is the client/server harness: two `Sim` values and one `Link`, no socket.
- **Commit:** `net: simulate loss latency and jitter`

### N9. Interest grid

- [ ] **Files:** `crates/sim/src/interest.rs`. Test `crates/sim/tests/interest.rs`.
- **Behavior:** `relevant(view_entity)` returns the view entity, the origin, entities flagged always-relevant, the input-authority target of this view, and every entity whose cell is in the 3 by 3 neighborhood of the view. Cell is `world_mm / CELL_MM` on X and Z. The list is ascending index order. 4096 entities are inserted on a line 100 m apart. A view at the origin does not see an entity 200 m away.
- **Test:** `a_far_entity_is_outside_the_neighborhood`
  - Place the view at world 0 and one pawn at 10 m (same cell) and one at 200 m. `relevant` contains the pawn at 10 m and the origin, and does not contain the pawn at 200 m. An always-relevant flag on the far pawn puts it back in the list.
- **Commit:** `sim: gather entities in a spatial neighborhood`

### N10. Priority and the byte budget

- [ ] **Files:** `interest.rs`, `snapshot.rs`. Test `crates/sim/tests/budget.rs`.
- **Behavior:** `take_snapshot_for(connection, budget)` filters to `relevant`, then writes spawns and despawns first, then dirty poses in the priority order from [Interest, priority, bandwidth](#interest-priority-bandwidth), stopping before the pose bytes would exceed `budget`. The default budget is `POSE_BUDGET` (1024). A written entity's priority returns to 0.
- **Test:** `the_budget_rotates_three_dirty_neighbors_and_still_spawns`
  - Three entities in the view cell, each with all four pose fields changed so each record is a full pose. Pass a budget equal to the encoded size of one full pose record. Over three snapshots each id appears once.
  - A spawn outside the cell is still present in the next datagram. Its pose is absent from the following datagram while it stays outside the neighborhood and the budget is already spent by an in-cell dirty pose.
- **Commit:** `sim: spend a pose budget by accumulated priority`

### N11. Redundant commands

- [ ] **Files:** `crates/net/src/command.rs`, `crates/sim/src/tick.rs`. Test `crates/net/tests/command.rs`.
- **Behavior:** `push_command` stores the command at its tick. `datagram_for(acked_tick)` writes every stored command with `tick > acked_tick`, capped at 64 commands (the oldest unacked ones if the window is longer). `apply_datagram` on the authority appends any command tick it does not already have. After a dropped datagram, the next one fills the hole. The motion pass consumes the command at the current tick for the connection that holds input authority. A tick with no command repeats the previous command for at most 4 ticks, then uses a zero command.
- **Test:** `a_dropped_command_packet_is_filled_by_the_next_one`
  - Push ticks 1, 2, and 3. Drop the first datagram (loss 100 for one send, then loss 0). The second datagram contains ticks 1, 2, and 3. The authority's command list is 1, 2, 3 with no gap.
  - Starve the authority for 5 ticks. Ticks +1 through +4 repeat the last strafe. Tick +5 has strafe 0.
- **Commit:** `net: resend unacknowledged commands`

### N12. Authority and a predicting runtime share the tick pass

- [ ] **Files:** `tick.rs`. Test `crates/sim/tests/motion.rs`.
- **Behavior:** The kinematic pass implements the millimeter rule in [Determinism](#determinism-and-the-physics-plan). `Sim::authority()` and `Sim::player()` are the same type. Given the same spawns and the same command list, 60 ticks, the checksums match. `strafe = 127` for 10 ticks from the origin moves `x_mm` to `670`. `forward = 127` for 10 ticks moves `z_mm` to `-670`.
- **Test:** `two_sims_with_the_same_commands_match`
  - The 10-tick strafe case equals 670 on both sims. The checksums are equal. A third sim that missed one command does not match, and matches again when that command is inserted and the ticks are rerun from the start.
- **Commit:** `sim: step a pawn from commands`

### N13. Restore and replay the owned entity

- [ ] **Files:** `crates/sim/src/predict.rs`. Test `crates/sim/tests/predict.rs`.
- **Behavior:** The player sim records `Pose` and `StunUntil` after each tick in a 128-deep ring. `reconcile(snapshot_tick, snapshot_pose, stun_until)` compares the recorded pose at that tick. On a mismatch it writes the snapshot pose and `StunUntil`, then runs the kinematic pass from the next tick through the local tick using stored commands. Motion is zero while `tick <= stun_until`. Entities other than the owned one are not written by that replay.
- **Test:** `a_stun_replays_the_owned_pawn_and_leaves_the_bystander`
  - Local tick 16, strafe 127 every tick, `stun_until` 0 in the prediction. Predicted `x_mm` is `16 * 67 = 1072`. The snapshot of tick 11 says `x_mm = 670` (moved on ticks 1..=10, stunned on tick 11) and `stun_until = 15`. After reconcile, ticks 12..=15 add 0 and tick 16 adds 67, so `x_mm` is `737`. A bystander at `x_mm = 50` is still at 50. A second reconcile of tick 16 with pose 737 and `stun_until` 15 does not change 737 and does not increment the correction count.
- **Commit:** `sim: replay the owned entity after a mismatch`

### N14. Interpolation buffer

- [ ] **Files:** `crates/sim/src/interp.rs`. Test `crates/sim/tests/interp.rs`.
- **Behavior:** `push(tick, id, pose)` stores remote poses. `sample(render_tick, id)` lerps between the greatest stored tick `ta <= render_tick` and the least stored tick `tb > render_tick`, and only when `tb - ta <= SNAPSHOT_EVERY`. Millimeters lerp in integer math: `a + (b - a) * (render - ta) / (tb - ta)`. Yaw uses the signed short arc, then wraps in `u16`. If there is no greater tick, or `tb - ta > SNAPSHOT_EVERY`, the sample is the pose at `ta`. The owned entity is refused by `push`. Render tick helper: `newest.saturating_sub(INTERP_TICKS)`.
- **Test:** `a_remote_pose_lerps_and_a_gap_holds`
  - Tick 0 at `x_mm = 0`, tick 3 at `x_mm = 6000`. `sample(1)` is `2000`. Ticks 0 and 12 only, `sample(6)` is the tick-0 pose.
  - Tick 0 yaw `65500`, tick 2 yaw `100`. `sample(1)` is `32` (`65500 + 68`, wrapped).
- **Commit:** `sim: interpolate remote poses`

### N15. Rewind hit volumes

- [ ] **Files:** `crates/sim/src/rewind.rs`. Test `crates/sim/tests/rewind.rs`.
- **Behavior:** `store_hit(tick, id, pose)` keeps 16 ticks. `strike(aim_tick, from_x_mm, to_x_mm)` clamps `aim_tick` to the window and tests the scratch poses, not the live column. A hit is a stored pose whose `x_mm` lies on the closed segment and within 100 mm of it in Z (the test uses Z = 0). A hit subtracts 1 from live `Health`. Live `Pose` is unchanged. `aim_tick` older than 16 ticks returns no hit and does not change `Health`.
- **Test:** `a_strike_hits_where_the_pawn_was_and_restores_the_present`
  - `Health` starts at 10. Live pose of the target is `x_mm = 0`. Stored pose at tick `now - 5` is `x_mm = 5000`. `strike` from `0` to `10_000` at that tick hits. Live `x_mm` is still 0. `Health` is 9. The same call with `aim_tick = now - 17` leaves `Health` at 9.
- **Commit:** `sim: rewind hit volumes for one query`

### N16. Input authority transfer

- [ ] **Files:** `tick.rs`. Test `crates/sim/tests/authority.rs`.
- **Behavior:** `set_input_authority(id, Some(connection))` during tick `T` affects command selection at `T+1`. The motion pass ignores a command whose connection does not hold authority for that tick. Leave (`None`) stops motion from commands. The component is included in the snapshot when it changes, via the dirty tick.
- **Test:** `commands_follow_the_connection_that_holds_authority`
  - Connection 1 strafes at 127 for ticks 1..=5. At the end of tick 5 the authority moves to connection 2. Connection 1 keeps sending strafe 127. Connection 2 sends strafe -127. Ticks 6..=10 move by `-67` mm each. The pose at tick 10 is `5 * 67 + 5 * -67 = 0`.
- **Commit:** `sim: move input authority at a tick boundary`

### N17. Notices

- [ ] **Files:** `crates/net/src/notice.rs`. Test `crates/net/tests/notice.rs`.
- **Behavior:** `enqueue(notice)` assigns the next sequence. A datagram carries at most 4 unacked notices, oldest first. `ack(seq)` drops every notice with `seq <= ack`. Apply is in order: a datagram that skips a sequence is held until the missing one arrives. A snapshot datagram is not stored on this list. A request notice does not change `Health`; a following snapshot that contains `Health` does.
- **Test:** `a_dropped_notice_is_resent_and_a_snapshot_is_not`
  - Enqueue six notices. The first datagram contains sequences 1..=4. Drop it. The next datagram contains 1..=4 again. Ack 4. The next contains 5..=6.
  - Deliver sequence 2 before sequence 1. The applied list is empty until 1 arrives, then 1 then 2.
  - A snapshot datagram sent once is not in the resend list after a drop. The next snapshot's apply still reaches the same checksum, using N7's delta rule.
- **Commit:** `net: resend ordered notices`

### N18. Map bridge

- [ ] **Files:** `examples/session` is not created yet. Put the bridge in `crates/sim/src/map.rs` as data in, entities out, so `genos-sim` does not depend on Rhai. Test `crates/sim/tests/map.rs`. The session binary calls it in N21.
- **Behavior:** `MapSolid { x_mm, y_mm, z_mm }` and `MapWall` with the same fields. `load_map(&[MapSolid], &[MapWall])` spawns one entity per item, sets the pose, and leaves input authority empty. The function takes millimeters, not handle strings. Ids are whatever `spawn` returns. Loading the same lists into two sims produces the same checksum.
- **Test:** `a_map_spawns_entities_without_using_a_scene_handle`
  - Two solids and one wall. The sim's spawned count is the origin plus three. Poses match the millimeters passed in. A second `load_map` on a fresh sim matches the checksum. The function signature has no `&str` parameter.
- **Commit:** `sim: spawn a map from plain poses`

### N19. Thousands of entities

- [ ] **Files:** `crates/sim/tests/scale.rs`, `crates/net/tests/scale.rs`.
- **Behavior:** No new API. Insert 4096 entities. Eight views in eight cells. After a full baseline and an ack, dirty one far entity and one neighbor per view. Each view's snapshot contains the neighbor and not the far id, the pose section is `<= POSE_BUDGET`, and the bytes match a second run.
- **Test:** `eight_views_of_four_thousand_entities_stay_inside_the_budget`
  - The assertions above. The test does not read the clock.
- **Commit:** `sim: keep a large world inside the pose budget`

### N20. UDP edge

- [ ] **Files:** `crates/net/src/udp.rs`. Test `crates/net/tests/udp.rs`.
- **Behavior:** `UdpEdge::bind("127.0.0.1:0")` binds one `UdpSocket`, nonblocking. `send_to` / `recv_from` move one datagram. The codec from N8 is what the bytes contain. `genos-sim` still does not reference `std::net`. A test that cannot bind skips with a message that names the error; on this environment it should bind. The happy path is the assertion.
- **Test:** `one_datagram_round_trips_on_loopback`
  - Bind two sockets on port 0. Send a 32-byte snapshot header. The peer reads those 32 bytes. No second thread sleeps; use a short nonblocking poll loop capped at 50 tries.
- **Commit:** `net: send one udp datagram`

### N21. Headless authority process

- [ ] **Files:** `examples/session/Cargo.toml`, `examples/session/src/main.rs`, `examples/session/src/run.rs`. Test `examples/session/tests/headless.rs`. `docs/architecture.md` lists the binary.
- **Behavior:** `run_session(SessionConfig) -> Report` loads nothing from the network. With `bind: None` it runs `frames` ticks of one authority sim and zero connections, writes the checksum, and returns. `main` parses `--scene --bind --frames`. `--bind off` calls `run_session` and prints one JSON object through `genos-json`. The package dependencies are `genos-sim`, `genos-net`, `genos-scene`, `genos-json`. They do not include `genos-window`. `frames == 0` returns after the stdin flag `stop` is set; the test sets the flag rather than closing a real pipe. `--scene` is accepted and `load_path` feeds `load_map` with the solid and wall positions converted to millimeters (truncation toward zero). A missing scene path returns an error report and does not panic.
- **Test:** `a_headless_session_ticks_without_a_window`
  - `run_session` for 30 frames from a two-solid map built in memory. `report.ticks == 30`. `report.checksum` equals a second call. The test binary does not link a window type: it is a `genos-session` test, and `genos-session`'s `Cargo.toml` has no `genos-window` dependency (the test reads that `Cargo.toml` as text and asserts the substring `genos-window` is absent).
- **Commit:** `session: run a headless authority`

### N22. Debug session in one process

- [ ] **Files:** `examples/net-view/Cargo.toml`, `examples/net-view/src/report.rs`, `examples/net-view/src/main.rs`. Test `examples/net-view/tests/report.rs`.
- **Behavior:** `run_view(ViewConfig) -> Report` builds one authority sim, `players` player sims (1..=4), and one `Link` each from `LinkConfig`. Scripted strafe commands drive player 0's pawn. The authority applies them after the link delay. Each player reconciles. The report rows are `{ index, generation, authority_x_mm, player_x_mm, corrections }` for the owned pawn of each player. Same config, same report bytes via `genos-json`. `--headless` is the `main` path that calls `run_view` and writes `--metrics`. This step does not open a window. `genos-ui` is not a dependency yet.
- **Test:** `a_delayed_link_corrects_once_and_then_matches`
  - Latency 100 ms, loss 0, jitter 0, seed 1, 60 frames, one player, strafe 127. The final row's `authority_x_mm` equals `player_x_mm`. `corrections` is 0 when every snapshot matches the prediction (the kinematic rule is shared, so a delay alone does not require a correction). A second case sets the authority to stun at tick 20. `corrections >= 1`, and the final millimeters still match.
  - Two calls write identical JSON.
- **Commit:** `net-view: report a desync between authority and player`

### N23. Slider model and the window path

- [ ] **Files:** `examples/net-view/src/sliders.rs`, `examples/net-view/src/main.rs`. Test `examples/net-view/tests/sliders.rs`. `genos-ui` becomes a dependency of `genos-net-view` only.
- **Behavior:** `sliders_from(config)` builds four `PanelRow` values whose `PanelSlider` ranges match the table in [Debug view](#debug-view). `config_from(sliders)` reads them back into a `LinkConfig` and a player count. `main` without `--headless` calls `button_panel` and then `run_view`. The test does not call `button_panel` and does not open a window: it changes a slider value, maps it through `config_from`, and runs `run_view`. Marks for the picture are a function `marks(report) -> Vec<Mark>` of world XZ in millimeters, one authority mark and one player mark, with `differ` true when the millimeters differ. The draw loop, when a window exists, places a UI rectangle on those marks. The test calls `marks`.
- **Test:** `slider_values_drive_the_same_link_the_headless_mode_runs`
  - A latency slider at 80 maps to `latency_ms == 80` (step 10, so 80 is on the step; 84 maps to 80). Loss 100, seed 1, 30 frames: the report is identical to `run_view` configured without the slider types.
  - A stun case produces a mark with `differ` true at the correction frame and `differ` false at the last frame.
- **Commit:** `net-view: drive latency loss and jitter from sliders`

### N24. Physics pawn, when BodyId exists

- [ ] **Files:** `examples/session/src/physics_pawn.rs`, `examples/session/Cargo.toml` (add `genos-physics`). Test `examples/session/tests/physics_pawn.rs`. `genos-sim` does not depend on `genos-physics`.
- **Behavior:** Start this step only when `genos_physics::BodyId` and `genos_physics::hash_world` are public. If they are not, stop. The adapter spawns a physics body whose `BodyId` index and generation equal the pawn `EntityId`. Each tick calls `step(&mut world, 1.0/60.0)` and writes `Pose` from the body position in millimeters, truncation toward zero. Two adapters fed the same commands match on `hash_world` and on `Sim::checksum`. Adding `1e-6` to the body position without a new `step` leaves the checksum unchanged. `shift_origin` by 1024 m on X and `shift_sector(1, 0)` in the same tick leave `world_mm` unchanged. If `step_parallel` is public and its hash differs from `step` on this world, the adapter calls `step`. The predicting runtime uses this same adapter for the owned pawn only. If `ray_cast` is public, one extra assertion places bodies at the rewound poses, calls `ray_cast`, and checks the live body position is unchanged. If it is not public, N15's segment test remains the hit path.
- **Test:** `the_physics_pawn_hashes_match_and_dust_stays_off_the_checksum`
  - The assertions above, 60 steps, strafe command converted to a wish of `4` m/s on X (the camera's `MOVE_SPEED`). The test tolerates the pose: both sides equal each other. It does not hard-code the millimeter, because the physics step is allowed to slide along a wall. With no wall, both checksums match.
- **Commit:** `session: drive the pawn from the physics step`

### N25. Record the session stream and read it back

- [ ] **Files:** `crates/net/src/replay.rs`. Test `crates/net/tests/replay.rs`.
- **Behavior:** `Recorder` writes the D14 frame around datagram bytes the N8 codec already produced. `Reader` returns those slices. `apply` on a fresh sim consumes them in tick order. `genos-sim` still does not reference `std::fs` or `std::net`. A tick with nothing to send writes length 0 and still writes the checksum. The authority datagram's trailing checksum matches the frame hash on snapshot ticks.
- **Test:** `recorded_datagrams_match_the_link_and_replay_the_checksum`
  - Run the N8 harness (seed 3, loss 30, latency 50, jitter 20, 40 ticks, one strafing pawn) and record both directions. For every frame with length greater than 0, the slice equals the bytes `Link` sent on that tick, byte for byte. Applying the authority slices on a fresh sim reproduces the authority checksum at tick 40. A second record with the same seed writes the same file bytes.
- **Commit:** `net: record the session datagrams`

### N26. Seek to a keyframe

- [ ] **Files:** `replay.rs`. Test `crates/net/tests/replay.rs`.
- **Behavior:** Tick 0 is a baseline-0 datagram. Further keyframes fall every 120 ticks, which is a multiple of `SNAPSHOT_EVERY`. Each keyframe is appended to the trailer. `seek(tick)` loads the greatest keyframe with tick `<=` the target, applies it, then applies later frames through the target. It does not run tick passes for ticks before the keyframe.
- **Test:** `a_seek_matches_playback_from_the_start`
  - Record 240 ticks of strafe 127 with loss 0. `seek(200)` and a full apply from tick 0 leave the same checksum and the same `x_mm`. The seek path's applied frame ticks are the keyframe tick and the frames after it, not tick 0, when the keyframe tick is greater than 0.
- **Commit:** `net: seek a replay to a keyframe`

### N27. Resim from a keyframe and compare hashes

- [ ] **Files:** `replay.rs`, `crates/sim/src/predict.rs` only if the owned-entity replay helper needs a tick range. Test `crates/net/tests/replay.rs`.
- **Behavior:** `resim(tick)` loads the keyframe, runs the kinematic pass with the stored commands through `tick`, and compares `checksum` to the stored frame hash at each tick. An equal hash continues. A mismatch returns that tick and does not keep resimulating. Snapshot apply of the same file still reaches the end.
- **Test:** `an_altered_command_mismatches_at_that_tick`
  - An unaltered recording resims equal at every tick through 200. Replacing the command at tick 50 with a different strafe makes `resim` return 50. Applying the recorded snapshots anyway still reproduces the original checksum at tick 200.
- **Commit:** `net: compare a resim to the stored hashes`

### N28. Inspect one tick in the debug binary

- [ ] **Files:** `examples/net-view/src/replay.rs`, `examples/net-view/src/main.rs`. Test `examples/net-view/tests/replay.rs`.
- **Behavior:** `inspect(path, tick, resim) -> Report` loads the keyframe at or before `tick`, applies frames through `tick`, and returns the pawn pose, the stored hash, and the applied hash. `resim: true` also runs N27 and fills `resim_hash`. `step(report)` advances one tick with the same functions. `--replay PATH --tick N` is the headless `main` path. `--step` and `--resim` call `step` and `inspect`. The scrub control writes the same tick integer. This step does not open a window.
- **Test:** `inspect_tick_prints_the_recorded_pose`
  - Record 90 ticks, strafe 127, loss 0. `inspect(path, 60, false)` returns `x_mm == 60 * 67` and equal stored and applied hashes. `inspect(path, 60, true)` returns the same pose and a `resim_hash` equal to the stored hash. `step` from that report moves to tick 61 and `x_mm == 61 * 67`. Two calls write identical JSON.
- **Commit:** `net-view: inspect a replay at a tick`

### N29. Entity name

- [ ] **Files:** `crates/sim/src/name.rs`. Test `crates/sim/tests/name.rs`.
- **Behavior:** `set_name(id, name)` stores the string when no live or reserved string equals it. `find(name)` returns the id that holds it, including a string reserved by a rename. `rename` keeps the old string resolving to the same id. Spawn with an empty name mints `e-<index>`. A typed mint (`<type>-<index>`) waits until N32, when a spawn notice can carry a type. The checksum fold gains the current name's bytes, so a rename changes the checksum. A scene-handle string is not special here: `wall:1` is only a name if a caller set that name, and this step does not read the camera endpoint.
- **Test:** `a_name_stays_on_the_same_id_after_rename_and_despawn`
  - Spawn, name `door`. `find("door")` is that id. Rename to `gate`. `find("door")` and `find("gate")` are that id. A second spawn named `door` is refused and does not allocate. Despawn the first id. A third spawn named `door` is still refused. `find` of that id's `index.generation` text returns the id before despawn and `None` after.
- **Commit:** `sim: name an entity for tools`

### N30. Query

- [ ] **Files:** `crates/sim/src/query.rs`. Test `crates/sim/tests/query.rs`.
- **Behavior:** `query_type`, `query_tag`, `query_component`, and `query_region(min_x_mm, min_z_mm, max_x_mm, max_z_mm)` return ids in ascending index order. Region uses `CELL_MM` only as a fast reject; the result is the entities whose world millimeters lie inside the AABB, so a cell edge does not drop an entity that is inside the box. Type and tag are components. Component presence is "the column has a value". No field predicates in this step.
- **Test:** `queries_select_type_tag_region_and_component`
  - Two entities with type `pawn`, one tagged `red`, both with `Health`, one at `x_mm = 0` and one at `x_mm = 100_000`. Tag `red` returns one id. Type `pawn` returns both. Component `Health` returns both. A region of 1 m around the origin returns the near id and not the far id. An entity with no `Health` is absent from the component query.
- **Commit:** `sim: query entities by type tag region and component`

### N31. Entity log and cause

- [ ] **Files:** `crates/sim/src/log.rs`. Test `crates/sim/tests/log.rs`.
- **Behavior:** A component write appends a log line: tick, cause kind, cause word, component id. The ring keeps 120 ticks and drops older lines. Kind `1` is a tick pass id, kind `3` is a notice sequence, kind `4` is a reconcile snapshot tick. `log(id)` returns the lines still in the ring, oldest first. A reconcile write's cause is the snapshot tick, not the motion pass. If D25.C is the accepted answer, the flag field is the `u16` from that decision, the authority datagram sets bit `128`, and writes the cause. A player snapshot leaves the bit clear. Do not put the cause on the player packet in this step. Bit `128` is not also the pin.
- **Test:** `a_write_records_the_tick_pass_and_a_reconcile_records_the_snapshot`
  - A motion pass writes `x_mm`. The log line names that pass and the tick. A notice write to `Health` names the notice sequence. A reconcile from snapshot tick 11 writes the pose and the log kind is reconcile with word 11. Line 121 of a one-line-per-tick run drops the oldest.
- **Commit:** `sim: record why a component changed`

### N32. Agent notices

- [ ] **Files:** `crates/net/src/agent.rs`. Test `crates/net/tests/agent.rs`.
- **Behavior:** `request_spawn`, `request_edit`, and `request_despawn` enqueue a notice. They do not change the sim. `run_tick` applies an agent connection's notices and refuses a player connection's spawn notice with no state change. An edit sets the named component and the log cause is that notice. A spawned id uses the requested name. If the notice omits the name and carries a type, the minted name is `<type>-<index>`. If it omits both, the name is `e-<index>` from N29. The datagram that carries the notice is the same notice encoding as N17. Join's payload role byte is the `u8` from D29.
- **Test:** `an_agent_spawn_applies_and_a_player_spawn_does_not`
  - A player connection's spawn notice leaves the spawned count unchanged. An agent connection's spawn of `door` adds one id after the tick, `find("door")` returns it, and the log cause is the notice. `request_edit` of `x_mm` to 10 leaves `x_mm` unchanged until `run_tick`. After that tick, `x_mm` is 10.
- **Commit:** `net: apply agent spawn through a notice`

### N33. Subscribe

- [ ] **Files:** `agent.rs`. Test `crates/net/tests/agent.rs`.
- **Behavior:** `subscribe(connection, filter)` stores a filter for an agent connection. A matching write enqueues one notice to that connection: entity index, generation, component id, cause kind, cause word. A write that does not match enqueues nothing. The notice uses the N17 resend and order rules. Query still works without a subscription.
- **Test:** `a_subscription_hears_the_matching_write_only`
  - Subscribe to tag `red`. A write on the tagged entity enqueues one notice. A write on an untagged entity does not. Dropping the datagram and sending again resends that notice until acked.
- **Commit:** `net: notify an agent connection of a matching change`

### N34. Inspect and seek from the tool

- [ ] **Files:** `examples/session/src/tool.rs`. Test `examples/session/tests/tool.rs`.
- **Behavior:** `inspect_live(name) -> Components` reads the authority columns. `inspect_replay(path, tick, name)` seeks with N26 and returns the components and the log lines at that tick. A write function called while a replay inspect is in progress returns an error and does not change the live sim. The report is `genos-json`.
- **Test:** `a_replay_seek_reads_the_log_and_refuses_a_write`
  - Record 90 ticks of an agent edit that sets `x_mm` to 50 at tick 40. `inspect_replay(path, 40, name)` returns `x_mm == 50` and a log line whose cause is that notice. `inspect_replay` then `request_edit` returns an error. The live sim's pose is unchanged from before the call.
- **Commit:** `session: inspect an entity at a replay tick`

### N35. CLI and one JSON handler

- [ ] **Files:** `examples/session/src/tool.rs`, `examples/session/src/main.rs`. Test `examples/session/tests/tool.rs`.
- **Behavior:** `parse_tool_line("query tag red")` returns the same query N30 runs. `handle_json` accepts one JSON-RPC-shaped body, `method` `query` or `inspect` or `spawn` or `seek`, and calls those functions. The test passes the body as a string. It does not bind a port and does not link the camera `genos-mcp` crate. `genos-session`'s `Cargo.toml` still has no `genos-window` dependency. Spawn through `handle_json` enqueues the same notice as N32.
- **Test:** `a_cli_line_and_a_json_body_call_the_same_query`
  - `parse_tool_line("query tag red")` and `handle_json` of a query for tag `red` return the same id list. A spawn JSON body followed by one `run_tick` makes `find` return the new name. The camera crate name `genos-mcp` does not appear in `genos-session`'s `Cargo.toml`.
- **Commit:** `session: query entities from a cli line and a json body`

### N36. Pin an edited component

- [ ] **Files:** `crates/sim/src/pin.rs`. Test `crates/sim/tests/pin.rs`.
- **Behavior:** A tool edit of pose, name, type, or tags sets a pin on that component. `Health` does not pin unless the notice says so. The motion pass skips a pinned pose. Unpin is a notice. The pin is included in the snapshot when it changes. A predicting runtime that applies the pin also skips the pose, and the checksums match.
- **Test:** `a_pinned_pose_stays_while_strafe_continues`
  - Pose at `x_mm = 0`. A tool edit sets `x_mm` to 10 and pins pose. Ten ticks of strafe 127 leave `x_mm` at 10. Unpin. One tick of strafe 127 leaves `x_mm` at 77. A second sim that only applied the snapshot, including the pin, matches the checksum after those ticks.
- **Commit:** `sim: pin a component an author edited`

### N37. Undo the last tool notice

- [ ] **Files:** `crates/net/src/agent.rs`. Test `crates/net/tests/undo.rs`.
- **Behavior:** Each applied tool notice pushes an inverse, up to 64. `undo` enqueues that inverse and pushes the forward notice onto a redo stack. `redo` enqueues it again. A player command and a motion-pass write do not push. Undo of despawn spawns a new generation, reuses the reserved name, copies the stored components, and appends a log line that names the old id. A different spawn of that name is still refused. The tick count advances. Player poses that the undo did not name stay on their own motion.
- **Test:** `undo_restores_the_tool_edit_and_not_the_player_motion`
  - Tool edit sets `x_mm` from 0 to 10. `undo` then one `run_tick` leaves `x_mm` at 0. The log contains both notices. A bystander already at 67, with a zero command on the undo tick, is still at 67. Despawn of `door` then undo: `find("door")` is a new generation, and the log names the old id. The 65th tool edit drops the oldest inverse: undoing 64 times does not restore that first edit.
- **Commit:** `net: undo a tool edit with an inverse notice`

### N38. Save a sidecar

- [ ] **Files:** `examples/session/src/save.rs`. Test `examples/session/tests/save.rs`. The same commit adds one sentence to `docs/systems/scripting.md` limits: a sidecar may overlay a loaded scene, and the script file is not rewritten.
- **Behavior:** `save(path)` copies authored entities at the current tick: name, type, tags, pose, pins. It writes that list with `genos-json` after `run_tick` returns. It does not open the `.rhai` file for write. A gameplay-spawned entity is absent unless the request names it. `load_sidecar` on a fresh sim, after the same map spawn, overlays those poses. A failed write returns an error and leaves the sim checksum unchanged. `genos-sim` does not reference `std::fs`.
- **Test:** `a_save_overlays_by_name_and_leaves_the_script_bytes`
  - An authored entity `door` at `x_mm` 10. A second entity spawned by a test pass with no authored mark, at `x_mm` 99. Save. The JSON contains `door` and 10, and does not contain 99. The bytes of a fixture script file are equal before and after. Load the sidecar onto a fresh sim that already spawned `door` at 0. `door` is at 10. Point the writer at a path that cannot be created. The function returns an error and the checksum is the pre-call checksum.
- **Commit:** `session: save authored entities to a sidecar`

### N39. Spawn from a prefab

- [ ] **Files:** `save.rs`, `agent.rs`. Test `examples/session/tests/save.rs`.
- **Behavior:** `save_prefab(entity, path)` writes one entity's authored components. `request_spawn_prefab(path)` enqueues a notice. After the tick the authority reads the file and spawns a new id with a minted name, the prefab pose, and the authored mark. The source entity is unchanged. A player connection's prefab spawn is refused. The new name is session-unique.
- **Test:** `a_prefab_spawn_copies_components_under_a_new_name`
  - `door` at `x_mm` 10, saved as a prefab. Spawn it. The new id's pose is 10, its name is not `door`, and `door` is still at 10. A player connection's request leaves the spawned count unchanged.
- **Commit:** `session: spawn an entity from a prefab file`

### N40. One allow gate

- [ ] **Files:** `crates/net/src/allow.rs`, `crates/net/Cargo.toml` features `profile-dev` (default), `profile-staging`, `profile-release`. Test `crates/net/tests/allow.rs`.
- **Behavior:** `allow(connection, capability, target) -> Result` is the gate from D42. Tool handlers from N32 onward call it, including in-process CLI calls. Under `profile-dev`, a loopback connection is granted every capability, and a remote player connection is granted `command` only. Under `profile-staging` and `profile-release`, the player-tool module is not compiled into the player entry (the test builds `genos-net` with `--no-default-features --features profile-release` and asserts the tool handler symbol's source file is cfg'd out by calling a `tool_api_compiled() -> bool` that is false). The authority entry still has `allow`. A deny writes an audit line and returns an error. The sim is unchanged. Audit lines are not part of a player snapshot.
- **Test:** `dev_loopback_may_edit_and_a_player_may_not`
  - Dev grant: an edit notice from the loopback connection changes `x_mm` after the tick. The same notice from a player connection leaves `x_mm` unchanged and appends a deny line with that connection and the `edit` capability. `command` from the player is allowed. Two calls with the same inputs produce the same audit bytes.
- **Commit:** `net: gate every tool call on a capability`

### N41. Player packets stay inside interest

- [ ] **Files:** `crates/sim/src/snapshot.rs`, `crates/net/src/allow.rs`. Test `crates/net/tests/expose.rs`.
- **Behavior:** A player snapshot contains only the interest set from N9, inside `POSE_BUDGET`. It has no cause field, no entity-log bytes, and no trailing checksum. An inspect grant's datagram may include the cause and the checksum. Pin is present on a player snapshot only for the entity that connection predicts.
- **Test:** `a_player_datagram_omits_the_far_entity_and_the_cause`
  - Two entities, one in the view cell and one 200 m away. The far id is absent from the player datagram. The near entity's record has the cause bit clear. The inspect datagram for the same tick has the cause bit set and the trailing checksum equal to `Sim::checksum`. The player datagram's last bytes are not that checksum.
- **Commit:** `net: keep the player packet inside interest`

### N42. Operator inspect and admin edit

- [ ] **Files:** `allow.rs`, `examples/session/src/tool.rs`. Test `examples/session/tests/grants.rs`.
- **Behavior:** `--operator-token` and `--admin-token` store the secrets on the authority. A join that presents the operator secret is granted `inspect` and not `edit`. A join that presents the admin secret is granted `inspect` and `edit`. A wrong secret is refused and audited. An empty configured secret means that grant cannot be taken. Dev loopback still needs no token. Inspect reads a far entity the player packet omitted. Edit from the operator grant is denied. Edit from the admin grant follows N32. Neither grant is applied on a player runtime. The authority applies the admin edit, and the player learns it from the next snapshot.
- **Test:** `an_operator_can_inspect_and_an_admin_can_edit`
  - An entity at `x_mm` 200_000. Operator inspect returns 200_000. That id is absent from the player datagram. Operator edit leaves the pose unchanged and writes a deny line. A second entity in the view cell at `x_mm` 5. Admin edit sets it to 8. After the tick, inspect returns 8 and the player snapshot of that relevant entity is 8. A join with the wrong admin token does not receive `edit`. The release-player assertion stays in N40.
- **Commit:** `session: grant inspect to an operator and edit to an admin`

### N43. Mod load order and conflicts

- [ ] **Files:** `crates/mods/Cargo.toml`, `crates/mods/src/lib.rs`, workspace `Cargo.toml`, `docs/architecture.md`. Test `crates/mods/tests/order.rs`. `[lints] workspace = true`. The crate depends on nothing in the workspace and does not depend on `rhai` yet. The same commit adds one sentence to `docs/systems/scripting.md` limits: mods load from a manifest, and a duplicate key without `overrides` fails the load.
- **Behavior:** `load_manifest` reads mod ids, versions, `before`, `after`, and provided keys. It returns an ordered list or an error. A cycle is an error. The same key from two mods without `overrides` is an error that names both ids and the key. With `overrides` on the later mod, the order stands and the result records the loser. An explicit host order is the tie-break when two mods declare no relation and no shared key.
- **Test:** `two_mods_conflict_until_one_overrides`
  - Mods `base` and `extra` both provide `prefabs/door`. The error names both and that key. `extra` sets `overrides` for that key and `after: base`. The order is `base`, then `extra`, and the loser is `base`. A third mod `loop` with `before` and `after` that cycle returns the cycle error. `genos-sim`'s `Cargo.toml` does not gain a `rhai` dependency.
- **Commit:** `mods: order a manifest and fail a silent clash`

### N44. Resolve prefabs, scenes, and assets

- [ ] **Files:** `crates/mods/src/resolve.rs`. Test `crates/mods/tests/resolve.rs`.
- **Behavior:** Given the ordered mods, `resolve(path)` returns the winning bytes for a logical asset path. `scene_overlay` merges entity fragments by name, later override wins. A prefab's property map keeps keys the built-in columns do not use. The function returns bytes and fields. It does not decode PNG or OBJ. The session later passes those bytes to `genos-load`.
- **Test:** `an_override_replaces_the_asset_and_the_scene_name`
  - Base provides `meshes/door.obj` bytes `A` and entity `door` at `x_mm` 0. A later mod overrides the mesh with bytes `B` and the entity with `x_mm` 10, plus property `hp` 3. `resolve` returns `B`. The overlay's `door` is at 10 and `hp` is 3. A third mod that provides the mesh without `overrides` is the N43 error, not a silent winner.
- **Commit:** `mods: resolve overrides for assets and scenes`

### N45. Sandboxed script enqueues a notice

- [ ] **Files:** `crates/mods/src/script.rs`, `crates/mods/Cargo.toml` (`rhai = "=1.26.0"`). Test `crates/mods/tests/script.rs`. `genos-sim` still does not depend on `rhai`.
- **Behavior:** The engine is the checked Rhai build. The host registers `spawn_named` and `set_prop` only when the mod permission list includes them. A call enqueues a notice and does not touch a store. The session applies the notice in the mod pass, after the other tick passes, with the same end-of-tick rule as a tool spawn. The cause on the new entity is the mod id and the hook name. A script that exceeds the fuel budget contributes no notices for that tick. A presentation engine does not register `spawn_named`. A script has no socket API and no function that writes a column.
- **Test:** `a_mod_script_spawns_through_a_notice_and_a_presentation_script_cannot`
  - Script `spawn_named("crate")` with permission. After one tick the entity exists and the log cause names the mod. The same source without the permission fails the call and leaves the spawned count unchanged. A loop past the fuel budget leaves the count unchanged even if the source called `spawn_named` before the loop. A presentation registration of the same source fails the call. Two runs enqueue the same notice bytes.
- **Commit:** `mods: let a checked script enqueue a spawn`

### N46. Hash the manifest and refuse a mismatch

- [ ] **Files:** `crates/mods/src/hash.rs`, `crates/net/src/mods_sync.rs`. Test `crates/mods/tests/hash.rs` and `crates/net/tests/mods_sync.rs`.
- **Behavior:** SHA-256 is implemented in `genos-mods`. No new crate. The manifest hash covers mod id, version, path, file hash, and order, in that canonical listing. `join_mods(local_hash)` on the authority returns `Ready` when the hashes match. It returns `Need` plus the missing file ids when they differ. `accept_files` then re-hashes. A bad file hash returns `Refuse` and does not advance the tick. Gameplay datagrams are not used for the file bytes.
- **Test:** `a_mismatched_mod_hash_never_ticks`
  - SHA-256 of the empty input is `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. Two identical one-file manifests produce the same manifest hash. Flip one file byte. `join_mods` returns `Need`. Feeding the original bytes returns `Ready`. Feeding a different byte returns `Refuse`, and a following `run_tick` is not called by the join helper. The helper's tick count stays 0 on `Refuse`.
- **Commit:** `mods: refuse a session whose mod hash differs`

### N47. A hosted authority loads a mod directory

- [ ] **Files:** `examples/session/src/mods.rs`, `examples/session/src/run.rs`. Test `examples/session/tests/mods.rs`.
- **Behavior:** `run_session` with `mods: Some(path)` loads that directory before the tick and sets `report.mod_hash`. `mods: None` sets a zero hash and still ticks. The dev tool's inspect from N40 still reads an entity the mod script spawned. This step does not open a socket and does not link `genos-window`.
- **Test:** `a_mod_directory_sets_the_report_hash_and_spawns`
  - A temp directory with one mod whose script is the N45 spawn. Thirty ticks. `report.mod_hash` equals `join_mods` on those same files. The entity `crate` exists. A second call with `mods: None` has a zero hash and no `crate`. Inspect of `crate` on the first run returns the entity. The release compile-out check stays in N40.
- **Commit:** `session: load mods from a directory`

## Limits

- The authority runtime is the only state authority. Input authority can move. A player runtime does not keep the true state of a pawn after it loses input authority.
- Spawns happen on the authority. There is no predicted spawn and no temporary id minted by the player runtime.
- The kinematic stand-in ignores yaw when it translates. N24 replaces that motion for the pawn. Other entities stay on the integer pose until a later plan gives them a body.
- Remote entities are interpolated or held. They are not extrapolated, and they are not resimulated on the player runtime.
- Damage and `Health` are authority writes. The player runtime does not predict them.
- The rewind window is 16 ticks. Older aim ticks do not hit.
- The command repeat on the authority is at most 4 ticks. There is no clock slew.
- Snapshots omit unchanged entities. They do not bit-pack, and they do not Huffman-code the datagram.
- Notices are at most 256 bytes and at most 4 per datagram. They are not a general RPC system for scripts.
- The only tick pass that calls Rhai is the authority mod pass. The player runtime does not run it. A script cannot widen its permissions or open a socket. Lockdown does not compile that pass out of a release authority.
- There is no job scheduler. Parallel physics, if it lands, is allowed on the authority only when its hash matches `step`.
- `examples/camera` does not join a session.
- The debug window shows four player runtimes. The scale test's eight connections are headless.
- Cross-machine bitwise identity is not claimed. Snapshot playback is the picture. Resim is a debug check against the stored hashes.
- The replay file has no reverse-simulation path. Rewind seeks to an earlier keyframe and applies forward.
- A keyframe is written in the tick that owns it. Later ticks are not dropped to pay for it.
- There is no replay-only component. A value in the file is a value in the live datagram, or a frame field from D14 (tick, hashes, length).
- Public spectators are delayed. The authority file is written immediately.
- `examples/camera` does not write a replay.
- D1–D42 are open. The recommendations are not a decision record.
- The camera agent endpoint does not spawn, edit, or despawn simulation entities. Scene handles stay on that endpoint.
- A tool does not write a column. Time-travel does not fork a session. Live undo does not seek backward and does not undo a player command.
- Save does not rewrite the Rhai scene. A prefab is data. A failed save does not change the sim. Undo does not revert the file.
- Staging and release player binaries do not contain the tool API. The authority starts with tool grants empty. A join role byte does not grant a capability. This repo does not store accounts. Admin edits run on the authority. The audit log is not sent to players.
- A mod is data plus a checked script. It is not a native library. A duplicate key without `overrides` does not load. A mismatched manifest hash does not tick. A mod author inspects from a dev build. A release player does not gain that tool because mods are loaded.
- The first query has no field predicates. The cause stores one hop, not a graph walk.
- Genos's process supervisor, matchmaking, and account system are out of this repository. The contract is the binary, the bind address, the scene path, and stdin.

## Records each step touches

N1 is the one that edits `CONTEXT.md`, `docs/architecture.md`, and the new ADR. N8 and N21 edit `docs/architecture.md` again for the new crate and the binary. No step edits `docs/systems/scene.md` or `docs/systems/physics.md` until N24, and N24 adds one sentence to each: the pawn's `BodyId` is its `EntityId`, and the net checksum is the quantized pose. N38 adds one sentence to `docs/systems/scripting.md`: a sidecar may overlay a loaded scene. The camera loop does not change.

## Coverage

| Requirement | Where it is decided | Step that tests it |
| --- | --- | --- |
| Entity identity across peers | Decision, Identity | N1, N7, N18 |
| Dirty tracking | Components | N6, N7 |
| Delta compression | Quake 3, Snapshots | N7, N8 |
| Interest and relevancy | Iris, replication graph | N9, N19 |
| Priority and bandwidth | Iris | N10, N19 |
| Server authority | Fusion dedicated mode | N12, N21 |
| Client prediction | Overwatch, Fusion | N12, N13 |
| Reconciliation and rollback | Overwatch, GGPO scoped to one entity | N13, N22 |
| Lag compensation | Source, Overwatch rewind | N15 |
| Determinism and the physics plan | P14, P21, P23 | N5, N12, N24 |
| Ownership transfer | Lightyear broker, Fusion input authority | N16 |
| RPCs and events | bevy_replicon messages, notices | N17 |
| Snapshot and interpolation buffers | Source 100 ms, Quake | N7, N14 |
| Headless server Genos can host | Headless authority | N21 |
| Thousands of entities, large worlds | Scale | N19 |
| Lossy client/server harness | Sockets, debug view | N8, N22 |
| Debug binary with sliders and desync marks | Debug view | N22, N23 |
| ECS decision | Decision | N1 (the ADR) |
| Job system left alone | Jobs, Global constraints | No step adds one |
| Rhai stays off the socket | ADR 0009, Scripting | No step links Rhai into `genos-sim`. N45 registers no socket function |
| Replay is the recorded network stream | Open decisions, Replay | N25 |
| Input replay, snapshot deltas, and keyframes | D1, D2, D3 | N25, N26, N27 |
| Seek, rewind, scrub | D4, D17 | N26, N28 |
| Killcam and live spectators | D5, D6, D7, D19 | N25 records the stream a killcam would slice. The window and the delay wait on those decisions |
| Per-tick hashes and desync | D11, D12, D15 | N27, N28 |
| Recording version and map bytes | D10, D13, D14 | N25 |
| Record a bug and inspect a tick | D8, D15, D16, D17 | N28 |
| Agent names, query, log, cause | Agent observability, D20–D26 | N29, N30, N31 |
| Spawn, edit, despawn as a connection | D27, D29 | N32, N35 |
| Subscribe | D26 | N33 |
| Time-travel through a replay | D4, D28 | N34 |
| Live edit that sticks | D30, D36 | N36 |
| Undo a tool edit | D31, D35 | N37 |
| Save back to a sidecar and a prefab | D32, D33, D34 | N38, N39 |
| Profiles, grants, and the one allow gate | D37, D38, D42 | N40 |
| Anti-cheat exposure is the interest set | D41 | N41 |
| Operator inspect and admin edit | D39, D40 | N42 |
| Mod load order and conflicts | D45, D46 | N43, N44 |
| Sandboxed Rhai on the authority | ADR 0009, D43, D44 | N45 |
| Hash-checked mod distribution | D47, D48 | N46, N47 |
| Mod authors keep the dev tool | D49 | N47, and N40 still compiles the tool out of a release player |

## What this plan does not do

It does not implement the steps. It does not settle D1–D49. It does not change `step`, the camera loop, or the camera MCP endpoint. It does not rewrite a scene script. It does not add accounts or a workshop. It does not merge the physics plan. N24 is the seam, and it waits for that plan's `BodyId` and `hash_world`. N1 waits until the open decisions are answered, because the datagram layout is the replay file and the live-edit path.
