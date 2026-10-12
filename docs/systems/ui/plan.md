# UI toolkit implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `genos-ui` into the retained UI toolkit a game or a general app uses to show information and take input, on a screen root and on a world root, without replacing the lighting panel or the profile graph.

**Architecture:** Keep the retained `Node` tree and resolve it every frame with the Clay sizing rules already in `layout`. Interaction state stays in `State`, keyed by element id. `frame` paints axis-aligned rectangles and segments, diffs them with `DamageTracker` (**requires #5**), and returns at most eight dirty rectangles. Text starts on the built-in bitmap and later opts into outlines from `genos-load`. The crate still does not open a window and does not call Vulkan.

**Tech Stack:** Rust, `genos-ui`, `genos-scene`, `genos-input`, `genos-load` (font outlines and images, consumed by the gallery), `genos-window` (Wayland pointer axis and text-input), `genos-render` (overlay blend and one atlas). No new crate.

This plan is based on `gi-v2`. PR #5 (`cursor/frame-output-reuse-6556`) adds `DamageTracker` and `Frame::damage`. A step that reads that tracker is marked **requires #5**. Do that part after #5 has merged. The renderer half of frame reuse is PR #5's `docs/frame-reuse-plan.md`. Do not implement it here.

## Global Constraints

- [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md): engine code calls the operating system or Vulkan. A third-party crate does not own the window, the input, the renderer, the audio, or the network. The Rust standard library stays. When the standard library has no equivalent, the helper lives in this repository. Dear ImGui, egui, Slint, Yoga, Taffy, HarfBuzz, Parley, AccessKit, and any UI markup crate stay out.
- [ADR 0009](../../adr/0009-rhai-is-the-scripting-language.md): Rhai is the scripting language. This plan does not add a second UI language.
- The workspace denies compiler warnings. `cargo build --workspace` passes. Run `cargo test -p <crate>` for every crate a step touches. A new file in an existing crate inherits the workspace lints.
- `genos-ui` does not open a window and does not call Vulkan. `docs/architecture.md` states that. Headless tests compare paint lists. They do not boot Weston or a GPU.
- Words: a **UI element** is the rectangle in the tree (`CONTEXT.md`). A **screen root** stays on the viewport. A **world root** follows a world point. **Window focus** is the display-server state in `CONTEXT.md`. **UI focus** is `State.focus`, the element id that receives keys. The input system, the input code, and the action keep those names.
- A step that changes the goal, the intent, the game use, or a limit of a system edits that system record in the same commit (`docs/systems/ui.md`, and `window.md` or `renderer.md` when those crates change).
- Lighting-panel ids stay in `genos_ui::id` (1 through 15). `button_panel` chrome stays at `u32::MAX - 4095` and above. Every other id in one `frame` call is unique. The same id in one frame joins one damage group.
- `Action` stays the lighting-panel enum. New elements emit `Event`. Existing `match` arms in `examples/camera` and `examples/stress` keep compiling.
- Do not route `ProfileGraph` through the chart element. The profile tests stay on `profile.rs`.
- Every new system is designed with networking in mind. This plan states the UI's replicated state, authority, prediction, determinism, and headless authority behavior in [Networking](#networking). `genos-ui` does not open a socket. The session, the codec, and the socket stay in [core-networking/plan.md](../core-networking/plan.md). When a name there differs from this plan, that file wins.

---

## Current state

The crate is `crates/ui`, package `genos-ui`. It depends on `genos-scene` only (`crates/ui/Cargo.toml`). The living record is [ui.md](../ui.md).

| Piece | Where | What it does |
| --- | --- | --- |
| Tree and layout | `crates/ui/src/layout.rs` | `Node`, `Sizing` (`Fit`, `Grow`, `Fixed`, `Percent`), `Direction`, `Align`, `Pad`, `Place`, `Space::{Screen, World}`, `layout`. Grow siblings share leftover space until they match. A sibling that is already larger stays there. A child with `Place` leaves the flow. A screen root ignores the camera. A world root projects with `viewport_uv`. |
| Text | `crates/ui/src/text.rs` | `CELL_W` 12, `CELL_H` 16. Whitespace wrap. A 5×7 bitmap, ASCII, uppercased. `measure` and `blots` share the cell. |
| Pointer panels | `crates/ui/src/panel.rs` | `lighting_frame`, `button_panel`, `Pointer`, `State`, `Paint`, `Frame`. Hit test walks items in reverse. `choose` picks pressed, hover, or idle. `State.focus` is the id of the last press. A miss sets `look_capture`. `PanelSlider` snaps to a step. Lamp sliders are notched at 1 meter. |
| Theme | `crates/ui/src/omarchy.rs` | `Palette` from `colors.toml` when the Omarchy `current` directory exists. A missing file keeps the previous palette. |
| Profile graph | `crates/ui/src/profile.rs` | `profile_overlay` paints a readout and `Segment` lines. Pause, drag a 250 ms bucket, reset. This is the only chart. |
| Damage | not on `gi-v2` | PR #5 adds `crates/ui/src/damage.rs`: `DamageTracker`, `MAX_DAMAGE` 8, merge gap 16 px, bounds outset to whole pixels, and `crates/ui/tests/damage.rs`. **requires #5.** |
| Public API | `crates/ui/src/lib.rs` | `Look`, `Segment`, `Action`, `PictureMode`, and the re-exports above. `Frame` has no `damage` field until #5. |
| Tests | `crates/ui/tests/` | `panel.rs`, `slider.rs`, `omarchy.rs`, `profile.rs`, `aa_camera.rs`. Layout tests live at the bottom of `layout.rs`. |

The frame that uses it:

- `examples/camera/src/main.rs` calls `lighting_frame`, copies `Paint` into `ScreenRect`, and draws with `Renderer::draw_with_overlay`. Look capture is suppressed while the pointer is on the panel.
- `examples/stress/src/main.rs` calls `button_panel` with rows built from `genos-debug` knobs (`Knob` in `crates/debug/src/knobs.rs` is a named number, not a rotary control).
- The overlay is opaque. `Gpu::open` builds it with `make_pipeline(false, false, false)` in `crates/render/src/gpu.rs` (depth off, blend off, additive off). `ScreenRect` and `ScreenLine` carry `[f32; 3]` color. Vertices are rebuilt only when the rectangle list changes. `crates/render/shaders/overlay.frag` is that pass: untextured when `uv.x < 0`, particle atlas on binding 2 when `uv` is in range, discard when `shade >= 0.5`. `GpuVertex` is 48 bytes.
- The window pointer-axis callbacks in `crates/window/src/wayland_shim.c` (`pointer_axis` and the discrete handlers) discard the value. There is no text commit and no clipboard on `Window::Frame`.
- `genos-load` decodes RGBA8 and a simple TrueType outline (`Font::glyph` in `crates/load/src/ttf.rs`, fixture `crates/load/fixtures/glyph.ttf`). It does not kern, hint, or shape. Composite glyphs error.
- `InputSystem` (`crates/input/src/system.rs`) exposes `keyboard`, `mouse`, and `gamepads`. `GamepadDevice::down` already treats the d-pad, the face buttons, and the left-stick digital codes. It has no `pressed` edge for those codes. `character_controller()` is the camera map (`move`, `look`, `capture`, `release`).

PR #5's `docs/frame-reuse.md` is the contract for dirty regions. **requires #5.** The UI reports changed rectangles, the renderer repairs those regions over a held scene picture, and a cached UI layer texture waits until `overlay` time is measured. That file is not on `gi-v2`.

## Gaps

What a game can do today: a screen root, a world-root marker, a draggable column of labels, buttons, and sliders, hover and press looks, an Omarchy palette, and a frame-time graph.

What is missing, against the toolkits below:

| Need | Today |
| --- | --- |
| Labels and rich text | One bitmap style. No spans, no font file, no shaping. |
| Button, toggle, checkbox, radio | `button_panel` has a labelled button and a selected look. No toggle, checkbox, or radio group. A focused box uses the pressed look (`choose` in `panel.rs`). |
| Slider | `PanelSlider` and the lamp notches. No vertical slider, no general element. |
| Dial and knob | Absent. `genos-debug` `Knob` is a number in a script. |
| Progress, linear and radial | Absent. The sun is a decorative disc, not a value. |
| Text field | Absent. No caret, selection, multiline, or IME. The window has no commit string. |
| Dropdown and selector | The picture mode is two buttons. No popup list. |
| List, tree, table | Absent. No virtualization. |
| Tabs | Absent. |
| Scroll | Absent. The Wayland axis event is discarded. |
| Modal, popup, tooltip, context menu | Absent. One tree, no layer that eats hits. |
| Color picker | Absent. |
| Number spinner | Absent. `Knob::number` is data. The panel draws it as a slider. |
| Image and icon | Absent. The overlay has no texture. |
| Stack, grid, flex, anchors | Column and row exist (`Direction`). Anchors exist (`Place`). No wrap, no main-axis justify, no grid. |
| Theme and style | Omarchy `Palette` plus per-node `Look`. No role names shared by every element. |
| Animation | Absent. `genos-load` animation keys are mesh transforms. |
| UI focus, keyboard, gamepad | A press stores `State.focus`. Keys do not move it. The camera map is the only action map the examples read. |
| Localisation | Strings are baked into the tree. |
| Accessibility | No role, name, or value beside the paints. |
| World root and screen root | Both exist. The world root is a screen-sized overlay at the projected point. It does not scale with distance, and the scene does not occlude it. |
| Charts | The profile graph only. |

## Comparison

The notes below are the parts that decide the Genos shape. Versions were checked in October 2026.

**Dear ImGui** is immediate mode. Each frame calls the widgets again. Keyboard and gamepad navigation are `ImGuiConfigFlags_NavEnableKeyboard` and `NavEnableGamepad`, with an activate key and a back key. Tables sort, resize, freeze, and scroll. Docking lives on the docking branch. Text input is `AddInputCharacter` plus `WantTextInput`. The library redraws the whole UI every frame. Genos keeps the navigation idea (direction, activate, back) and the debug-panel use. Genos does not take the immediate call style. `Node` ids name elements across frames. PR #5's `DamageTracker` hashes paints per id. **requires #5.**

**egui** (0.32) is immediate mode with id-keyed memory. `AtomLayout` places text and images inside a widget. `TextEdit` has a galley, grapheme caret movement, and selection. AccessKit exports roles for buttons, sliders, text, and progress. Layout is a cursor, a row, columns, and wrap, not a flex engine. Genos keeps id-keyed state in `State` and a semantic tree of its own. AccessKit is a third-party crate and stays out under ADR 0007.

**Slint** (1.14, October 2025) is a retained `.slint` tree. Layout is horizontal, vertical, and grid. Text goes through Fontique and Parley. The software renderer tracks dirty geometry and repaints the region a removed element left behind. Widgets include button, checkbox, switch, slider, spin box, line edit, text edit, combo box, list view, tabs, scroll, dialog, and popup. Accessibility exposes text selection. Translations use gettext. PR #5 reports dirty rectangles from the CPU. **requires #5.** Genos does not take the markup language or Parley.

**Unity UI Toolkit** (Unity 6) is retained. Yoga flexbox is the layout engine. UXML and USS are the document and the style. `ListView` virtualizes rows. World-space panels arrived in Unity 6.2 as a panel on a scene object. Runtime navigation is wired to the input system. Genos extends the existing sizing rules with wrap, justify, and a grid, and virtualizes lists. A world root stays an overlay projection. A textured quad in the scene is a later renderer change, outside this plan.

**Unreal UMG and Slate** are retained. Slate builds the tree in code. UMG is the editor over that tree. Canvas anchors match `Place`. `InvalidationBox` caches child geometry and skips the layout when nothing changed. A volatile widget still paints every frame. Common UI routes the d-pad and the face buttons to UI focus, with an activate target and a back handler. PR #5 invalidates by paint hash per id. **requires #5.** The profile graph is the volatile case once that tracker exists: it feeds its own `DamageTracker`. A cached UI layer texture stays out until PR #5's `docs/frame-reuse.md` has measured `overlay`.

**Noesis** is retained XAML. The visual tree and the draw list are cached. Compatible primitives batch. `UpdateRenderTree` reports whether anything changed so the host can skip the GPU. Genos already batches every solid rectangle into one overlay draw (`screen_quads` in `crates/render/src/gpu.rs`). Skipping a still frame is the renderer plan, not this one.

**Flutter** splits a widget description from a retained element and a render object. Layout is flex, stack, and grid. `RepaintBoundary` isolates a layer. Semantics are a parallel tree. IME comes from the platform. Genos uses the same split in a smaller form: the `Node` is data, `frame` is the pass that lays out, hits, and paints. Repaint boundaries are the damage groups. IME is a Wayland text-input commit on the window, which ADR 0007 allows as a native call.

**Clay** (Nic Barker) is the sizing model `layout.rs` already implements: fit, grow, fixed, percent, and the rule that open grow siblings meet in the middle while a larger sibling stays. This plan extends that file. It does not add the Clay crate.

The rejected shape is an immediate-mode API beside the retained tree. Two ways to build one button will drift, and damage keys will depend on call order. The rejected shape is also a `.slint` / UXML / XAML document. ADR 0009 already picked Rhai for scripts, and a markup parser is a second language. Rust constructors build the tree. A Rhai binding can call those constructors later. It is not a step in this plan.

## Architecture

### Retained tree, resolved every frame

`Node` remains the element. A `Kind` says which control it is. `Kind::Box` is the default, so every existing tree stays a box and `choose` stays as it is for those nodes.

```rust
pub enum Kind {
    Box,
    Label,
    Button { enabled: bool },
    Toggle { on: bool },
    Checkbox { checked: bool },
    Radio { group: u32, selected: bool },
    Slider { value: f64, min: f64, max: f64, step: f64, vertical: bool },
    Dial { value: f64, min: f64, max: f64, step: f64, knob: bool },
    Progress { value: f32, radial: bool },
    Spinner { value: f64, min: f64, max: f64, step: f64 },
    TextField { text: String, multiline: bool },
    Scroll { offset: [f32; 2] },
    List { rows: Vec<ListRow>, selected: Option<u32>, scroll: [f32; 2] },
    Tree { nodes: Vec<TreeNode> },
    Table { columns: Vec<Column>, rows: Vec<Vec<String>>, sort: Option<Sort> },
    Tabs { titles: Vec<String>, selected: u32 },
    Dropdown { choices: Vec<String>, selected: u32, open: bool },
    Tooltip { text: String },
    Menu { items: Vec<MenuItem> },
    Dialog { title: String },
    Image { source: u32, uv: [f32; 4] },
    Icon { name: IconName },
    ColorField { rgba: [f32; 4] },
    Chart { series: Vec<Series> },
}
```

The game owns the values inside `Kind` and copies them in each frame, the way `PanelSlider` already does. `State` owns the pointer gesture, UI focus, caret, selection, scroll velocity, popup layer, and animation clock, keyed by id.

```rust
pub enum Event {
    Press { id: u32 },
    Toggle { id: u32, on: bool },
    Change { id: u32, value: f64 },
    Text { id: u32, text: String },
    Select { id: u32, index: u32 },
    Submit { id: u32 },
    Dismiss { id: u32 },
    Sort { id: u32, column: u32, ascending: bool },
    Color { id: u32, rgba: [f32; 4] },
}

pub struct Frame {
    pub shown: Vec<Shown>,
    pub paints: Vec<Paint>,
    pub segments: Vec<Segment>,
    pub images: Vec<ImagePaint>,
    pub actions: Vec<Action>,
    pub events: Vec<Event>,
    pub look_capture: bool,
    pub keys_captured: bool,
    pub damage: Vec<Rect>,
    pub semantics: Vec<Semantic>,
}
```

`lighting_frame` and `button_panel` keep returning `Frame`. New fields default empty so old callers still compile. `images` is empty until the image step. `semantics` is empty until the semantics step.

```rust
pub struct FrameArgs<'a> {
    pub pointer: Pointer,
    pub dt: f32,
    pub keys: Option<&'a genos_input::InputSystem>,
    pub text: TextInput,
    pub strings: Option<&'a StringTable>,
}

pub fn frame(
    root: &Node,
    state: &mut State,
    space: Space,
    viewport: [f32; 2],
    camera: Option<&Camera>,
    args: FrameArgs<'_>,
) -> Frame
```

Task 2 introduces `frame` with `pointer` and `dt` only, as two trailing arguments, so the early tests stay short. Task 17 adds `keys`. Task 28 adds `text`. Task 41 adds `strings`. When a step adds a field, it folds the earlier arguments into `FrameArgs` if they are still loose, and every existing call passes the new field as `None` or `TextInput::default()`. The signature above is the shape after Task 41. `dt` is seconds. A test passes `0.0`.

Hit testing stays reverse tree order, then the open layer in front of the root. A press on a layer does not reach the root.

### Layout

`layout` stays the one pass. Stack is `Direction::LeftToRight` or `TopToBottom`. Flex adds `Justify` on the main axis and `wrap: bool`. Grid adds `columns: u32` on the node. Anchors stay `Place`. Clip is a flag: children paint only inside the padding box, and damage uses the clipped bounds.

`Place` is the anchor container. `Place::point` pins a pivot. `Place::stretch` sizes from split anchors. Y grows down. The origin is the parent's top-left. That matches the comment on `Place` in `layout.rs`.

### Text

`TextSource` on the node replaces the bare `Option<String>` once rich text and the string table arrive. Until then `Node.text` remains.

Shaping stays in this repository. The bitmap path remains the default so the lighting panel's tests keep their cells. `TextFace::Outline` rasterizes `genos_load::Glyph` contours into an R8 atlas on the CPU. Advances come from `Glyph.advance`. There is no kerning, no bidi, and no HarfBuzz. A composite glyph still fails in the loader. Wrapping breaks on whitespace, then on the advance width.

IME is a commit string and a preedit string. The UI paints the preedit in the accent color and inserts the commit at the caret. The window learns those strings from Wayland `text-input-v3`. The headless test injects them. It does not open a seat.

### Paint, batching, dirty regions

`Paint` stays an axis-aligned rectangle. `Segment` stays a line the renderer draws as a thin quad. One overlay draw holds every solid rectangle, every line, and, after the atlas step, every image quad.

`GpuVertex` is 48 bytes (`crates/render/src/pack.rs` asserts the size) and stays 48 bytes. `overlay_vertex` writes `uv: [-1, -1]` and `shade: 0`. `overlay.frag` treats `uv.x < 0` as untextured, discards `shade >= 0.5`, and writes premultiplied `albedo * texel.a`. The particle atlas is binding 2. `particle_texel` blurs by 6 pixels for embers. UI images do not use that buffer. Unlit particle cards use `shade = 0` and a geometric normal, so UI alpha does not reuse `shade` or `normal`. Untextured UI alpha travels in `uv.y`: a negative `uv.y` means alpha 1, which is every vertex the renderer writes today.

**requires #5.** Every element passes its paints to `DamageTracker` under `u64::from(id)`. A clip cuts the rectangles before the hash, so a row scrolled out of view does not dirty. The first frame after a viewport change dirties the whole viewport. Tests assert stability on the second frame, then a one-element damage on the change after that. Until #5 has merged, `frame` omits `damage` and those assertions stay skipped.

Alpha arrives as `Paint.alpha` defaulting to `1.0`. The overlay pipeline turns blend on. The scene pipeline stays as it is. `0` alpha paints nothing and hashes as absent. The fragment reads UI alpha from `uv.y` as the paint section describes.

Animation lerps a recorded rect and color toward the layout result over `dt`. `State.reduced_motion` snaps. Opacity joins after alpha exists.

### Theme

```rust
pub struct Theme {
    pub surface: [f32; 3],
    pub surface_raised: [f32; 3],
    pub text: [f32; 3],
    pub muted: [f32; 3],
    pub accent: [f32; 3],
    pub danger: [f32; 3],
    pub focus: [f32; 3],
    pub mode: omarchy::Mode,
}
```

`theme_dark()` and `theme_light()` are the built-ins. `theme_from_palette` copies `background`, `foreground`, and `accent`. The lighting panel keeps the looks it builds today. New elements call `looks(&theme)`.

### Input

Navigation reads the input system. It does not poll devices itself.

```rust
pub fn ui_actions() -> genos_input::ActionMap
```

Default bindings:

| Action | Codes |
| --- | --- |
| `ui_left` | `key_left`, `key_a` is not bound, `gamepad_dpad_left`, `gamepad_left_stick_left` |
| `ui_right` | `key_right`, `gamepad_dpad_right`, `gamepad_left_stick_right` |
| `ui_up` | `key_up`, `gamepad_dpad_up`, `gamepad_left_stick_up` |
| `ui_down` | `key_down`, `gamepad_dpad_down`, `gamepad_left_stick_down` |
| `ui_activate` | `key_enter`, `key_space`, `gamepad_face_south` |
| `ui_back` | `key_escape`, `gamepad_face_east` |
| `ui_next` | `key_tab` |
| `ui_prev` | `key_tab` with `key_shift_left` or `key_shift_right` held |

`key_a` stays on the camera `move` action. While `Frame.keys_captured` is true the frame skips `character_controller` for that frame. The gallery does this. `examples/camera` and `examples/stress` stay on `look_capture` until a later change. WASD is not a UI direction, so a lighting panel that has UI focus does not eat camera movement.

`GamepadDevice::pressed` is the edge, matching `KeyboardDevice::pressed`. Stick digital directions snapshot inside `begin_frame`.

A text field consumes character keys, arrows, home, end, backspace, and delete while it has UI focus, and sets `keys_captured`. Other focused controls set `keys_captured` only for the navigation codes they used this frame.

### Localisation and semantics

A string table is `HashMap<(locale, key), String>`. `TextSource::Key` resolves during `frame`. A missing key paints the key. Layout sees the resolved text, so a longer translation grows a fit element.

```rust
pub struct Semantic {
    pub id: u32,
    pub role: Role,
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub focused: bool,
}
```

`Role` is `Label`, `Button`, `Toggle`, `Checkbox`, `Radio`, `Slider`, `Dial`, `Progress`, `Spinner`, `TextField`, `List`, `Tree`, `Table`, `Tab`, `Dropdown`, `Dialog`, `Menu`, `Image`, `Chart`. The tree is paint order, back to front, so a dialog is last. No AT-SPI socket in this plan. The semantic list is the headless accessibility surface.

### Networking

The name source is [core-networking/plan.md](../core-networking/plan.md). This section uses the words that file locks.

An **entity** is the stable simulation id and the components stored under it. `EntityId` is `{ index: u32, generation: u32 }`. The **authority runtime** writes replicated components and is headless. A **player runtime** predicts the one entity its `InputAuthority` names and interpolates the rest. A **connection** is one player runtime joined to an authority runtime (`ConnectionId`). A **snapshot** is the replicated components one connection needs for one tick, delta-compressed against an earlier snapshot. A **command** is one tick of sampled actions. `InputCommand` carries `tick`, `forward`, `strafe`, `look_x`, `look_y`, `buttons`, and `aim_tick`. A **notice** is a reliable ordered message. The result of a notice comes back as a component on a later snapshot.

The sim tick is 60 Hz. A snapshot goes out every 3rd tick. Remote entities render at `newest_snapshot_tick` minus 6 ticks. The owned entity renders its predicted pose. `Health` changes only on the authority runtime, and the player runtime displays the snapshot value. The same rule covers `StunUntil`. The player runtime may be wrong about the owned pose. The snapshot corrects it.

`genos-ui` keeps `index` and `generation` as two `u32` fields with that `EntityId` layout. It does not depend on `genos-sim` or `genos-net`. It does not call `std::net`. `genos-session` does not link this crate. The caller copies component values into the tree once per UI frame. `frame` does not read a snapshot ring.

| | Local element | Replicated element |
| --- | --- | --- |
| Examples | UI focus, caret, scroll offset, popup layer, tooltip delay, theme, locale, the lighting panel, the profile graph | A readout or a slider bound to `Pose`, `Health`, or `StunUntil` on one entity |
| Where the value lives | `State`, or the caller's own model | A component the authority runtime writes |
| Snapshot | Absent | Present when the component is dirty and in interest |
| Who writes the shown number | This player runtime | The authority runtime. The player runtime supplies a predicted pose only for the entity it has input authority for |

```rust
pub enum ReplicaField {
    PoseX,
    PoseY,
    PoseZ,
    Yaw,
    Health,
    StunUntil,
}

pub struct ReplicaView {
    pub index: u32,
    pub generation: u32,
    pub field: ReplicaField,
    /// Owned pose, in the element's display unit. Empty for health, stun, and every entity this connection lacks input authority for.
    pub predicted: Option<f64>,
    /// Authority value on the newest applied snapshot, in the same display unit. This is the confirmed value.
    pub snapshot: f64,
    /// A local request waiting on a command or a notice. Empty once the snapshot has caught up.
    pub pending: Option<f64>,
}
```

`Node.replica` defaults to `None`, which marks a local element. The lighting panel and the profile graph leave it empty. The caller fills `ReplicaView` for a replicated element. Pose values are already quantized to the snapshot's millimetres and `u16` yaw, then converted into the display unit, so sub-millimetre noise does not move a mark. The UI does not quantize.

Paint, for a slider, a dial, a progress bar, a spinner, or a label that carries `ReplicaView`:

- The thumb, fill, or readout uses `predicted` when it is `Some`.
- Otherwise it uses `snapshot`.
- `Health` and `StunUntil` clear `predicted` before paint. Those fields show the snapshot value.
- When a kept `predicted` differs from `snapshot`, a 2 px mark in `theme.muted` sits on the snapshot value. That mark is the confirmed value.
- When `pending` differs from the shown value, a 2 px mark in `theme.accent` sits on the pending value.
- A label has no track. Its readout is the shown number. A predicted pose that differs from the snapshot adds the snapshot number as a second run in `theme.muted`. A pending value adds a second run in `theme.accent`.
- A local element paints the value inside `Kind` and draws neither mark.

A press still emits `Event` on that frame, with the logical value. The thumb stays on the predicted pose or on the snapshot. It does not jump to `pending`. The caller copies a pawn button into `InputCommand.buttons`. The caller sends any other request as a notice. The UI crate builds neither. The next snapshot the caller copies is what moves `snapshot`.

`inspect` walks the tree in paint order and returns one row per element.

```rust
pub struct InspectRow {
    pub id: u32,
    pub local: bool,
    pub index: Option<u32>,
    pub generation: Option<u32>,
    pub field: Option<ReplicaField>,
    pub predicted: Option<f64>,
    pub snapshot: Option<f64>,
    pub pending: Option<f64>,
    pub diverged: bool,
}

pub fn inspect(root: &Node) -> Vec<InspectRow>
```

`diverged` is true when the row kept a predicted value and that value differs from `snapshot`. A health row whose caller passed `predicted` still reports `predicted: None` and `diverged: false`.

`frame` and `inspect` read the tree, `State`, and `FrameArgs`. The same inputs produce the same paints and the same rows. The authority runtime draws no UI. Gallery tests stay in-process and do not bind a UDP socket.

`genos-net-view` is the debug view of one authority runtime and N player runtimes, with link latency, loss, and jitter. This plan does not build that binary. The gallery inspector is the view of one UI tree: local rows beside replicated rows, predicted beside snapshot.

### Files

| Path | Responsibility |
| --- | --- |
| `crates/ui/src/lib.rs` | Public types and `frame`. |
| `crates/ui/src/layout.rs` | Sizing, wrap, justify, grid, clip, anchors. |
| `crates/ui/src/text.rs` | Bitmap measure and blots. Outline measure joins in the font step. |
| `crates/ui/src/theme.rs` | `Theme`, built-ins, palette mapping, `looks`. |
| `crates/ui/src/snapshot.rs` | Canonical paint text for tests. |
| `crates/ui/src/interact.rs` | Hit test, UI focus, gestures, `frame`. |
| `crates/ui/src/controls.rs` | Kind-specific behavior and paints. Split a file when one kind passes about 400 lines. |
| `crates/ui/src/layers.rs` | Popup, menu, dialog, tooltip stacking. |
| `crates/ui/src/nav.rs` | `ui_actions` and focus movement. |
| `crates/ui/src/damage.rs` | **requires #5.** PR #5 adds the tracker. Callers group by id. |
| `crates/ui/src/panel.rs` | Lighting panel and `button_panel`, calling `frame`. |
| `crates/ui/src/profile.rs` | Unchanged. |
| `crates/ui/src/replica.rs` | `ReplicaView`, `inspect`, predicted and snapshot marks. |
| `crates/ui/tests/*.rs` | One integration test file per step that adds behavior. |
| `examples/ui/` | Gallery binary and its state inspector, last step. |

---

## Ordered steps

Each step is one commit. Do them in order. The test is the spec. Run it and expect the failure named in the step before writing the implementation, then expect `cargo test -p genos-ui` to pass. Add `--test <file>` when the step names a file. Steps that touch `genos-input`, `genos-window`, or `genos-render` also run that crate's tests.

A step marked **requires #5** reads `DamageTracker` or `Frame::damage`. Skip that assertion until #5 has merged. The rest of the step runs on `gi-v2`. After #5, an unchanged second `frame` returns `damage: []`, and the first `begin` on a new viewport dirties the whole viewport.

### Task 1: Paint snapshot

The still-panel test reads `Frame::damage`. **requires #5.** `paint_snapshot` does not.

**Files:**
- Create: `crates/ui/src/snapshot.rs`
- Modify: `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/snapshot.rs`

**Produces:** `pub fn paint_snapshot(paints: &[Paint]) -> String`. One line per rectangle, `x y w h r g b` with two decimals, sorted by `y`, then `x`, then `color`. Zero-area rectangles are omitted.

- [ ] **Step 1: Write the failing test**

```rust
use genos_scene::Camera;
use genos_ui::{lighting_frame, paint_snapshot, Paint, Pointer, State};

#[test]
fn paint_snapshot_orders_rects_and_drops_empty_ones() {
    let paints = vec![
        Paint { x: 10.0, y: 2.0, w: 4.0, h: 5.0, color: [1.0, 0.0, 0.0] },
        Paint { x: 1.0, y: 2.0, w: 3.0, h: 4.0, color: [0.0, 1.0, 0.5] },
        Paint { x: 0.0, y: 0.0, w: 0.0, h: 4.0, color: [0.0, 0.0, 0.0] },
    ];
    assert_eq!(
        paint_snapshot(&paints),
        "1.00 2.00 3.00 4.00 0.00 1.00 0.50\n10.00 2.00 4.00 5.00 1.00 0.00 0.00\n"
    );
}

#[test]
fn a_still_lighting_panel_matches_itself_and_then_reports_no_damage() {
    let mut state = State::default();
    state.omarchy_current = Some(std::path::PathBuf::from("/no/such/omarchy"));
    let camera = Camera::opening();
    let pointer = Pointer { x: 0.0, y: 0.0, down: false };
    let view = [1280.0, 720.0];
    let lamp = Some([0.0, 7.0, 0.0]);
    let first = lighting_frame(&mut state, view, &camera, lamp, pointer);
    let second = lighting_frame(&mut state, view, &camera, lamp, pointer);
    assert_eq!(paint_snapshot(&first.paints), paint_snapshot(&second.paints));
    assert!(second.damage.is_empty(), "{:?}", second.damage);
}
```

- [ ] **Step 2: Run** `cargo test -p genos-ui --test snapshot`

Expected: fail to compile, `paint_snapshot` is missing.

- [ ] **Step 3: Implement** `snapshot.rs` and `pub use snapshot::paint_snapshot`.

- [ ] **Step 4: Run** `cargo test -p genos-ui`

Expected: the two new tests pass, and the existing UI tests pass.

- [ ] **Step 5: Commit** `ui: canonical paint snapshot for headless checks`

### Task 2: Shared frame pass

Moving the damage loop and asserting `Frame.damage` **requires #5.** On `gi-v2`, `lighting_frame` has no damage loop. `frame` leaves `damage` empty until #5 lands. The hit test, looks, and drag move on this branch.

**Files:**
- Create: `crates/ui/src/interact.rs`
- Modify: `crates/ui/src/panel.rs`, `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/frame.rs`

**Produces:** `frame` as in Architecture, with `keys: Option` omitted until Task 17. `Frame.events` exists and is empty for the lighting panel. `Node.kind` exists and defaults to `Kind::Box`. `lighting_frame` and `button_panel` build their trees as they do now, then call `frame` for hit testing, looks, drag, damage, and `Node.action`.

- [ ] **Step 1: Write the failing test**

```rust
use genos_ui::{frame, Action, Node, Place, Pointer, Sizing, Space, State};

#[test]
fn a_press_on_a_box_with_an_action_fires_once() {
    let mut node = Node::new(7);
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 10.0]));
    node.action = Some(Action::Press(7));
    let mut state = State::default();
    let down = Pointer { x: 12.0, y: 12.0, down: true };
    let first = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, down, 0.0);
    assert_eq!(first.actions, vec![Action::Press(7)]);
    assert!(!first.look_capture);
    let held = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, down, 0.0);
    assert!(held.actions.is_empty());
    assert!(held.damage.is_empty());
}

#[test]
fn a_press_that_misses_sets_look_capture() {
    let node = Node::new(1);
    let mut state = State::default();
    let miss = Pointer { x: 180.0, y: 80.0, down: true };
    let frame = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, miss, 0.0);
    assert!(frame.look_capture);
    assert!(state.focus.is_none());
}
```

Make `State.focus` public in this step. The test reads it.

- [ ] **Step 2: Run** `cargo test -p genos-ui --test frame` — expected: compile failure.

- [ ] **Step 3: Move** the pointer edge, drag, slider hold, `choose`, and damage loop out of the two panel functions into `frame`. Keep lamp notches and `push_bar` in `panel.rs`. Existing panel, slider, and damage tests are the regression.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: one frame pass for hit testing, looks, and damage`

Update `docs/systems/ui.md` Game use: a game can call `frame` on its own tree.

### Task 3: Theme roles

**Files:**
- Create: `crates/ui/src/theme.rs`
- Modify: `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/theme.rs`

**Produces:** `Theme`, `theme_dark`, `theme_light`, `theme_from_palette`, `looks(theme) -> Looks` with `idle`, `hover`, `pressed`, and `focus` fields of type `Look`.

Built-in dark: surface `[0.16, 0.16, 0.18]`, surface_raised `[0.22, 0.22, 0.25]`, text `[0.93, 0.95, 0.92]`, muted `[0.62, 0.64, 0.66]`, accent `[0.36, 0.58, 0.95]`, danger `[0.85, 0.32, 0.28]`, focus `[0.95, 0.86, 0.40]`. Light swaps surface and text. `theme_from_palette` copies background, foreground, and accent, and keeps danger and focus from the matching mode.

- [ ] **Step 1: Test** that `theme_from_palette` of a palette with background `[0.1, 0.1, 0.1]`, foreground `[0.9, 0.9, 0.9]`, accent `[1.0, 0.0, 0.0]`, and `Mode::Dark` stores those three channels, and that `looks(&theme_dark()).hover.border` equals `theme_dark().accent`.

- [ ] **Step 2: Run** `cargo test -p genos-ui --test theme` — expected: compile failure.

- [ ] **Step 3: Implement.** Do not change `lighting_frame` colors.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass, including `omarchy`.

- [ ] **Step 5: Commit** `ui: theme roles beside the Omarchy palette`

### Task 4: Label

The second-frame empty-damage assertion **requires #5.**

**Files:**
- Modify: `crates/ui/src/interact.rs`, `crates/ui/src/lib.rs` (`Kind::Label`)
- Test: `crates/ui/tests/label.rs`

A label lays out with `text::measure`. A press on it does not push `Action` or `Event`. The snapshot of `"AB"` at a fixed 200×32 box contains more than one paint (the glyphs) and a second frame has no damage.

- [ ] **Step 1: Test** those two facts with `Node { id: 3, kind: Kind::Label, text: Some("AB".into()), width: Sizing::fixed(80.0), height: Sizing::fit(), .. }`.

- [ ] **Step 2: Run** the test — expected: `Kind::Label` missing, then after the enum exists, the press assertion fails if labels still emit actions.

- [ ] **Step 3: Implement.** `Kind::Box` with `action: Some` still fires. That is Task 2.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: a label paints text and ignores a press`

### Task 5: Main-axis justify

**Files:**
- Modify: `crates/ui/src/layout.rs` (`pub enum Justify { Start, Center, End, SpaceBetween }`, field `Node.justify`, default `Start`)
- Test: `crates/ui/src/layout.rs` unit tests

`Start` matches today's cursor. `SpaceBetween` with two fixed children and a known gap puts the first at the content origin and the last flush with the content end. `Center` centers a single child. `End` pins the row to the far side.

- [ ] **Step 1: Test** a 200×20 left-to-right node, gap 0, two 20×10 children, `Justify::SpaceBetween`. Expect x of 0 and 180.

- [ ] **Step 2: Run** `cargo test -p genos-ui layout::` — expected: fail.

- [ ] **Step 3: Implement** in `distribute` by placing leftover space according to `Justify` when no child is still growing. Grow still consumes leftover first. Justify applies to what remains.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: main-axis justify on the flow`

Document in `ui.md` Intent: the main axis can start, center, end, or split the leftover. The default is start.

### Task 6: Wrap

**Files:**
- Modify: `crates/ui/src/layout.rs` (`Node.wrap: bool`, default false)
- Test: unit test in `layout.rs`

Three 100×10 children in a left-to-right node of width 250, gap 0, `wrap: true`. The third child's y is the first child's height. The parent's fit height is 20. With `wrap: false` the third stays on the first row and overflows, which is today's behavior.

- [ ] **Step 1: Test** both flags.

- [ ] **Step 2: Run** the test — expected: the wrapped third child is still on y 0.

- [ ] **Step 3: Implement** by breaking the flow when the next basis does not fit the remaining main space. A child wider than the content box sits alone on a row.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: optional wrap on a row or a column`

### Task 7: Grid

**Files:**
- Modify: `crates/ui/src/layout.rs` (`Node.columns: u32`, `0` means flow)
- Test: unit test in `layout.rs`

`columns: 2`, four fixed 40×16 children, gap 4, in a node of width 84. Child ids 2 and 3 share y 0 at x 0 and 44. Child ids 4 and 5 share y 20. The parent fit height is 36.

- [ ] **Step 1: Test** that geometry.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.** A grid ignores `Direction` for placement and uses `columns` as the row length. `columns: 0` keeps the flow.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: fixed-column grid`

### Task 8: Clip

The dirty-rectangle assertion **requires #5.** The clip of the child's paint does not.

**Files:**
- Modify: `crates/ui/src/layout.rs` (`Node.clip: bool`), `crates/ui/src/interact.rs`
- Test: `crates/ui/tests/clip.rs`

A clip node at `(0, 0, 50, 20)` with a child placed at `(40, 0)` sized `30×10` paints the child only where it overlaps the parent. The child's paint `x + w` is at most 50. A second frame has no damage. Moving the child to x 10 dirties a rectangle that covers both the old and the new visible bounds, and does not extend to x 80.

- [ ] **Step 1: Test** that.

- [ ] **Step 2: Run** the test — expected: the child paint still has width 30.

- [ ] **Step 3: Implement** by intersecting each paint and segment with the clip rect before hashing and before pushing to `Frame`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: clip children to the parent and damage the visible part`

### Task 9: Button

The empty-damage frame and the third-frame dirty rect **require #5.**

**Files:**
- Modify: `crates/ui/src/interact.rs`
- Test: `crates/ui/tests/button.rs`

`Kind::Button { enabled: true }` with `looks(&theme_dark())`. A press edge emits `Event::Press { id }` once. A held pointer emits nothing. `enabled: false` emits nothing and uses the muted text color. Hover changes the border to `theme.accent`. UI focus after the press draws the focus border from `looks.focus`. The second still frame has empty damage. Disabling on the third frame dirties that button's rect.

- [ ] **Step 1: Test** press, hold, disabled, hover color, and damage.

- [ ] **Step 2: Run** the test — expected: no `Event::Press`.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: button press, hover, and disabled`

### Task 10: Toggle

The stable second frame and the dirty track **require #5.**

**Files:**
- Test: `crates/ui/tests/toggle.rs`
- Modify: `crates/ui/src/interact.rs`

`Kind::Toggle { on }`. A press emits `Event::Toggle { id, on: !on }` and does not mutate the caller's `Kind` (the game writes the next frame). The thumb paint sits on the right half when `on` is true and the left half when false. Track width 48, height 24, fixed. Second frame stable. Flipping `on` dirties the track.

- [ ] **Step 1: Test** both positions by comparing the thumb's x to the track's midpoint.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: toggle reports the next on state`

### Task 11: Checkbox

**Files:**
- Test: `crates/ui/tests/checkbox.rs`
- Modify: `crates/ui/src/interact.rs`

`Kind::Checkbox { checked }`. Press emits `Event::Toggle` with the inverted flag. The checked snapshot contains a mark (a paint in the accent color inside the box, inset by 4 px). The unchecked snapshot does not. The box is 16×16.

- [ ] **Step 1: Test** the mark's presence and the event.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: checkbox mark and toggle event`

### Task 12: Radio

**Files:**
- Test: `crates/ui/tests/radio.rs`
- Modify: `crates/ui/src/interact.rs`

Three `Kind::Radio { group: 1, selected }` in one column, ids 1, 2, 3. Pressing id 2 emits one `Event::Select { id: 2, index: 1 }` where index is the position among radios of that group in tree order. The selected look is the accent fill. A radio in group 9 is unchanged. A press on the already selected radio emits `Select` again and stays selected.

- [ ] **Step 1: Test** that.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: radio group selection`

### Task 13: Slider

The damage assertion **requires #5.** The value tests do not.

**Files:**
- Test: `crates/ui/tests/slider_element.rs`
- Modify: `crates/ui/src/interact.rs`

`Kind::Slider { value: 0.0, min: 0.0, max: 10.0, step: 1.0, vertical: false }` on a fixed 100×24 track placed at `(0, 0)`. A press at the track's midpoint emits `Event::Change { id, value: 5.0 }`. A press at the left end emits `0.0`. `step: 0.0` emits a continuous value within `1e-3` of the fraction. `vertical: true` uses pointer y, top of the track as `max`. The lamp notches and `PanelSlider` stay. Damage on a value change covers the track and not a sibling label placed 200 px to the right.

- [ ] **Step 1: Test** horizontal snap, continuous, vertical, and damage.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** with the same track-end inset `button_panel` uses (`SLIDER_INSET` 12, thumb 10×16).

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass, including `slider`.

- [ ] **Step 5: Commit** `ui: horizontal and vertical slider element`

### Task 14: Dial

The second-frame empty-damage assertion **requires #5.**

**Files:**
- Create: `crates/ui/src/arc.rs`
- Test: `crates/ui/tests/dial.rs`
- Modify: `crates/ui/src/lib.rs`, `crates/ui/src/interact.rs`

**Produces:** `pub fn arc_segments(center: [f32; 2], radius: f32, from_turns: f32, to_turns: f32, width: f32, color: [f32; 3]) -> Vec<Segment>`. Turns run clockwise from the top. At least 8 segments for a quarter turn.

`Kind::Dial` on a fixed 64×64 box. Value `0.0` at min `0.0` max `1.0` points up. A drag upward by 20 px from the center increases the value. `step: 0.25` snaps. The value clamps at `min` and `max`. `knob: true` adds a radial segment from the center to the rim. `Frame.segments` carries the arc. A second frame with the same value has no damage.

- [ ] **Step 1: Test** the up vector at value 0 (the marker's end y is less than the center y), the clamp, and the snap.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: dial and knob with a shared arc`

### Task 15: Progress

Dirtying the element when the value changes **requires #5.**

**Files:**
- Test: `crates/ui/tests/progress.rs`
- Modify: `crates/ui/src/interact.rs`

`Kind::Progress { value, radial }`. Value clamps to `0.0..=1.0`. Linear: the fill width is `value * track_width` on a 100×12 track. Radial: `arc_segments` spans `value` turns on a 48×48 box. A press emits no event. Value `0.5` then `0.75` dirties that element.

- [ ] **Step 1: Test** fill width, arc presence, and the ignored press.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: linear and radial progress`

### Task 16: Spinner

**Files:**
- Test: `crates/ui/tests/spinner.rs`
- Modify: `crates/ui/src/interact.rs`

`Kind::Spinner { value: 1.0, min: 0.0, max: 3.0, step: 1.0 }`. A press on the up half emits `Change` `2.0`. A press on the down half emits `0.0`. At `max`, up emits `3.0`. The readout text paints the value with the bitmap.

- [ ] **Step 1: Test** the three presses.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** as a row: down button, label, up button, one id for the whole spinner. Hits on the halves map to the buttons.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: number spinner`

### Task 17: UI focus and navigation

**Files:**
- Create: `crates/ui/src/nav.rs`
- Modify: `crates/ui/Cargo.toml` (add `genos-input`), `crates/ui/src/lib.rs`, `crates/ui/src/interact.rs`, `crates/input/src/device.rs`
- Test: `crates/ui/tests/nav.rs`, and a unit test in `crates/input/src/lib.rs`

**Produces:** `ui_actions()` and `GamepadDevice::pressed`.

`frame` gains `keys: Option<&InputSystem>` (see `FrameArgs`). Focusable kinds are button, toggle, checkbox, radio, slider, dial, spinner, and later text, list, table, tab, dropdown. Disabled buttons are skipped. `ui_down` moves UI focus to the next focusable item in tree order, wrapping. `ui_up` moves backward. `ui_next` and `ui_prev` do the same. `ui_activate` on a button emits `Press`. On a slider, `ui_left` and `ui_right` step the value and emit `Change`. A radio group's left and right stay inside the group. `keys_captured` is true on a frame that consumed a navigation code. A pointer miss clears UI focus and then sets `look_capture`, which is the lighting panel's current miss path. UI focus survives frames where the pointer is up. `keys_captured` does not by itself force `look_capture` false.

- [ ] **Step 1: Write the input test and the UI test**

```rust
#[test]
fn a_stick_up_edge_fires_once() {
    let mut input = genos_input::InputSystem::new();
    let pad = input.gamepad_mut(0).unwrap();
    pad.view.connected = true;
    pad.left_stick.y = 1.0;
    assert!(pad.pressed(genos_input::InputCode::gamepad_left_stick_up));
    input.begin_frame();
    assert!(!pad.pressed(genos_input::InputCode::gamepad_left_stick_up));
    assert!(pad.down(genos_input::InputCode::gamepad_left_stick_up));
}
```

UI test: two enabled buttons, ids 1 and 2, stacked. `set_key(key_down, true)` after `begin_frame` moves `state.focus` from 1 to 2 and sets `keys_captured`. A second frame with the key still down does not move again. `key_enter` emits `Press` for id 2.

- [ ] **Step 2: Run** `cargo test -p genos-input stick_up` and `cargo test -p genos-ui --test nav` — expected: fail.

- [ ] **Step 3: Implement** `pressed` by storing the previous digital stick and button sample in `begin_frame`. Build `ui_actions` with `ActionMap::set`. Call it from `frame` when `keys` is `Some`.

- [ ] **Step 4: Run** `cargo test -p genos-input` and `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: keyboard and gamepad focus through the input system`

Update `docs/systems/ui.md` and `docs/systems/input.md` Game use: UI navigation reads `ui_actions()` on the same `InputSystem` as the camera map.

### Task 18: Layers

Including the layer's old bounds in `Frame.damage`, and dirtying both layers on replace, **requires #5.**

**Files:**
- Create: `crates/ui/src/layers.rs`
- Modify: `crates/ui/src/interact.rs`, `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/layers.rs`

**Produces:** `State::open_layer(&mut self, node: Node)` and `State::close_layer(&mut self)`. The open layer is hit first. A press on a layer button emits that button's event and does not emit the root button under the same pointer. A press outside the layer emits `Event::Dismiss { id }` for the layer root and closes it. Damage includes the layer's old bounds when it closes.

- [ ] **Step 1: Test** the covered root button and the outside dismiss.

- [ ] **Step 2: Run** the test — expected: the root button also fires.

- [ ] **Step 3: Implement.** One layer is enough. A second `open_layer` replaces the first and dirties both.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: one popup layer in front of the root`

### Task 19: Scroll

Dirtying the scroll rect **requires #5.** Clipping and the wheel offset do not.

**Files:**
- Modify: `crates/ui/src/interact.rs`, `crates/ui/src/lib.rs` (`Pointer.wheel_y`)
- Test: `crates/ui/tests/scroll.rs`

`Kind::Scroll { offset }` clips. A drag on the content moves `offset` and emits `Event::Change` only through the offset the next frame reads: the event is `Change` with the y offset in `value`. Wheel `pointer.wheel_y` of `16.0` subtracts 16 from `offset[1]`, clamped so content cannot scroll past its end. A child that starts above the clip produces no paints. Dragging it into view paints it and dirties the scroll rect.

`Pointer` gains `wheel_y: f32`, default `0.0`. Existing struct literals in the camera, the stress example, and the tests gain `wheel_y: 0.0`.

- [ ] **Step 1: Test** clamp, clip, and wheel.

- [ ] **Step 2: Run** the test — expected: fail to compile on `wheel_y`, then fail the clip assertion.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` and `cargo build -p genos-camera -p genos-stress` — expected: pass.

- [ ] **Step 5: Commit** `ui: scroll view clips and follows drag and wheel`

### Task 20: Wayland wheel

**Files:**
- Modify: `crates/window/src/wayland_shim.c`, `crates/window/src/lib.rs`
- Test: `crates/window` unit test if one can feed the shim; otherwise a Rust decode test

`Window::Frame` gains `wheel_y: f32`. `pointer_axis` for vertical axis accumulates `wl_fixed_to_double` into the frame and clears it after `pump` copies it out. Horizontal axis is ignored. `pointer_axis_value120` adds `value / 120.0` when the continuous event did not already arrive for that frame (prefer value120 when the source is a click wheel).

- [ ] **Step 1: Test** a small C-free helper if the shim exposes one. If the shim cannot be called headless, test the Rust accumulator `fn take_wheel(pending: &mut f32) -> f32` that `pump` uses, and document the Wayland callback as the only writer. The Rust test: two adds of `1.5` and `-0.5` return `1.0` and leave the pending value at `0.0`.

- [ ] **Step 2: Run** `cargo test -p genos-window` — expected: fail.

- [ ] **Step 3: Implement** the callbacks and the field. Update `docs/systems/window.md` Game use.

- [ ] **Step 4: Run** `cargo test -p genos-window` — expected: pass.

- [ ] **Step 5: Commit** `window: report vertical pointer-axis wheel on the frame`

The gallery copies `frame.wheel_y` onto `Pointer.wheel_y`. The camera and the stress example pass `0.0` until someone scrolls a panel there.

### Task 21: List

The damage-bounds assertion **requires #5.** Virtualization and select do not.

**Files:**
- Test: `crates/ui/tests/list.rs`
- Modify: `crates/ui/src/controls.rs` (create), `crates/ui/src/interact.rs`

```rust
pub struct ListRow { pub id: u32, pub text: String }
```

`Kind::List` of 1000 rows, row height 16, viewport height 64, shows four rows. Paints mention the visible labels and no later row's text. `scroll: [0.0, 16.0 * 10.0]` shows rows 10 through 13. A press emits `Event::Select { id: row_id, index }`. Row ids stay stable and differ from the list node's id, so a one-row text change dirties one row height. The list damage bounds stay inside the viewport.

- [ ] **Step 1: Test** visible count, scroll window, select, and damage area.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** by building temporary child nodes for the visible range only. Do not allocate 1000 `Node`s.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: virtualized list`

### Task 22: Tree

Dirtying the tree viewport on collapse **requires #5.**

**Files:**
- Test: `crates/ui/tests/tree.rs`
- Modify: `crates/ui/src/controls.rs`

```rust
pub struct TreeNode { pub id: u32, pub text: String, pub children: Vec<TreeNode>, pub open: bool }
```

A closed parent paints its label and hides its children. A press on the disclosure emits `Event::Toggle { id, on: true }` and does not select. A press on the label emits `Select`. An open parent indents children by 16 px. Collapsing dirties the tree viewport.

- [ ] **Step 1: Test** hidden text, indent, and the two events.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** as a flattened visible list of `(depth, node)`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: tree expand and collapse`

### Task 23: Table

**Files:**
- Test: `crates/ui/tests/table.rs`
- Modify: `crates/ui/src/controls.rs`

```rust
pub struct Column { pub title: String, pub width: f32 }
pub struct Sort { pub column: u32, pub ascending: bool }
```

```rust
pub struct Column { pub id: u32, pub title: String, pub width: f32 }
```

Three columns, five rows, viewport shows the header plus two rows. Header press emits `Event::Sort { id: column.id, column, ascending: true }` and a second press on the same header emits `ascending: false`. The game reorders `rows` on the next frame. Row press emits `Select` with the row index. Cells clip to the column width. A drag on the header's right 4 px emits `Event::Change { id: column.id, value: new_width }` and does not emit `Sort`.

- [ ] **Step 1: Test** sort events, clip, and virtualization (row 4's text is absent until scrolled).

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** the header, the rows, the clip, and the resize drag from the spec above.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: table columns, sort, and row select`

### Task 24: Tabs

The page-rect damage assertion **requires #5.**

**Files:**
- Test: `crates/ui/tests/tabs.rs`
- Modify: `crates/ui/src/controls.rs`

`Kind::Tabs { titles, selected }`. The selected title uses the accent look. One child of the tabs node is visible: the child at `selected`. A press on a title emits `Event::Select { id, index }`. Switching from 0 to 1 removes the first child's paints and adds the second's, and damage covers the page rect.

- [ ] **Step 1: Test** with two children that contain the labels `"ONE"` and `"TWO"`.

- [ ] **Step 2: Run** the test — expected: both labels paint.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: tabs show the selected page`

### Task 25: Dropdown

**Files:**
- Test: `crates/ui/tests/dropdown.rs`
- Modify: `crates/ui/src/controls.rs`, `crates/ui/src/layers.rs`

`Kind::Dropdown { choices, selected, open }`. Closed, the closed box paints `choices[selected]` and a press emits nothing except setting the layer: the event is `Event::Press { id }` and `state` opens a layer of the choices. The test calls `frame` twice, the second time with `open: true`, and expects the choice labels in the paints. A press on choice index 1 emits `Event::Select { id, index: 1 }` and `Dismiss`. A press outside emits `Dismiss` and no `Select`.

- [ ] **Step 1: Test** closed text, open list, select, and outside dismiss.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** using Task 18's layer.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: dropdown list on the popup layer`

This is the selector. A radio group remains the always-visible selector.

### Task 26: Single-line text and selection

**Files:**
- Create: `crates/ui/src/edit.rs`
- Test: `crates/ui/tests/text_field.rs`
- Modify: `crates/ui/src/interact.rs`

**Produces:**

```rust
pub struct Caret { pub anchor: usize, pub head: usize }
```

`anchor == head` is a caret. The field stores `Caret` in `State` under the element id. `Kind::TextField { text, multiline: false }`.

Keys, with UI focus on the field: `key_a` inserts `"a"` at the head and emits `Event::Text` with the new string. Shift plus `key_right` grows the selection. Backspace deletes the selection or the character before the head. `key_left` at 0 stays at 0. Home and end jump. The game replaces `text` with the event's string on the next frame. The test drives `InputSystem` the way Task 17 does. `keys_captured` is true. A caret paint is a 2×cell-height rectangle. A selection paint uses the accent color behind the selected cells.

- [ ] **Step 1: Test** insert, selection delete, and the home clamp.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** against the bitmap cell. Character index is a byte index only for ASCII. Reject a non-ASCII `text` by ignoring those scalars in this step (the outline step lifts that).

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: single-line text field with selection`

### Task 27: Multiline

**Files:**
- Test: extend `crates/ui/tests/text_field.rs`
- Modify: `crates/ui/src/edit.rs`

`multiline: true` wraps with `text::measure` rules and lets `key_enter` insert `\n`. `key_up` and `key_down` move the head by one visual line, keeping the column. A line past the clip scrolls the field by one cell height.

- [ ] **Step 1: Test** `"ab cd"` in a width of 24 (the layout test's wrap) and `key_up` from the second line landing on the first.

- [ ] **Step 2: Run** the test — expected: enter does not insert a newline.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: multiline text field`

### Task 28: IME model

**Files:**
- Modify: `crates/ui/src/edit.rs`, `crates/ui/src/lib.rs`
- Test: `crates/ui/tests/text_field.rs`

```rust
pub struct TextInput { pub commit: String, pub preedit: String }
```

`FrameArgs.text` is `TextInput::default()` at every existing call. A commit `"é"` inserts at the caret and clears the preedit in the returned event's text. A preedit `"ni"` paints those glyphs in the accent color and does not emit `Text` until commit. The preedit is not part of `Kind::TextField.text`.

- [ ] **Step 1: Test** commit insertion and preedit paint without an event.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: text commit and preedit`

### Task 29: Wayland text input

**Files:**
- Modify: `crates/window/src/wayland_shim.c`, `crates/window/src/lib.rs`
- Test: Rust-side `fn apply_text_event(commit: &mut String, preedit: &mut String, op: TextOp)`

`Window::Frame` gains `text_commit: String` and `text_preedit: String`. Bind `zwp_text_input_v3` when the global exists. Enable it while the app has asked: `Window::set_text_input(&mut self, on: bool)`. `commit_string` appends to `text_commit` for that pump and clears after read. `preedit_string` replaces `text_preedit`. A missing global leaves both strings empty and `set_text_input` returns `Ok(())`.

The headless test covers `apply_text_event`: a preedit op sets the preedit, a commit op appends and clears the preedit. Update `docs/systems/window.md`.

- [ ] **Step 1: Test** `apply_text_event`.

- [ ] **Step 2: Run** `cargo test -p genos-window` — expected: fail.

- [ ] **Step 3: Implement** the protocol callbacks writing the same helper.

- [ ] **Step 4: Run** `cargo test -p genos-window` — expected: pass.

- [ ] **Step 5: Commit** `window: Wayland text-input commit and preedit`

The gallery calls `set_text_input(true)` while a text field has UI focus, and copies the frame strings into `TextInput`.

### Task 30: Clipboard

**Files:**
- Modify: `crates/ui/src/edit.rs`
- Test: `crates/ui/tests/text_field.rs`

`State` holds one `String` clipboard. `key_control_left` plus `key_c` copies the selection. `key_control_left` plus `key_v` inserts it and emits `Text`. `key_control_left` plus `key_x` copies and deletes. An empty selection copies nothing. This clipboard is inside the process. A Wayland data-device paste is out of this plan. The limit goes in `docs/systems/ui.md`.

- [ ] **Step 1: Test** copy and paste of `"ab"` inside one field.

- [ ] **Step 2: Run** the test — expected: paste leaves the text unchanged.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: in-process copy and paste for text fields`

### Task 31: Tooltip

Reporting the tip rect as damage **requires #5.**

**Files:**
- Test: `crates/ui/tests/tooltip.rs`
- Modify: `crates/ui/src/layers.rs`

A node with `tooltip: Option<String>` (add the field, default `None`). Hover for `dt` accumulating to `0.4` seconds opens a layer whose text is the tip, placed under the pointer and clamped to the viewport. Hover of `0.1` does not. Leaving the target closes it. The tip is not a press target. Damage is the tip rect. A tooltip does not open while a dialog owns the layer.

- [ ] **Step 1: Test** the delay, the leave, and the clamp at the bottom edge (pointer y near the viewport height places the tip above the pointer).

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: tooltip after a hover delay`

### Task 32: Context menu

**Files:**
- Test: `crates/ui/tests/menu.rs`
- Modify: `crates/ui/src/layers.rs`, `crates/ui/src/lib.rs`

```rust
pub struct MenuItem { pub id: u32, pub text: String, pub enabled: bool }
```

`Pointer` gains `right_down: bool` and `right_pressed: bool` (edge). The camera and stress literals set both false. A right press on a node with `menu: Vec<MenuItem>` opens the layer at the pointer. A press on an enabled item emits `Event::Press { id: item.id }` and `Dismiss`. A disabled item emits nothing. A left press outside dismisses.

- [ ] **Step 1: Test** open, item press, and disabled.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` and `cargo build -p genos-camera -p genos-stress` — expected: pass.

- [ ] **Step 5: Commit** `ui: context menu on right press`

### Task 33: Modal

**Files:**
- Test: `crates/ui/tests/modal.rs`
- Modify: `crates/ui/src/layers.rs`

`State::open_dialog(node)` is a layer that does not dismiss on an outside press. The outside press emits no root event. `ui_back` emits `Event::Dismiss` and closes the dialog. UI focus on open moves to the first focusable child. On close it returns to the id that was focused before. Tab wraps inside the dialog.

- [ ] **Step 1: Test** the blocked root, the back key, and the restored focus id.

- [ ] **Step 2: Run** the test — expected: the root still receives the press.

- [ ] **Step 3: Implement.** The scrim is an opaque surface-colored rectangle until Task 34. It covers the viewport.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: modal dialog traps focus`

### Task 34: Overlay alpha

**Files:**
- Modify: `crates/ui/src/panel.rs` (`Paint.alpha: f32`), `crates/ui/src/snapshot.rs`, `crates/render/src/gpu.rs`, `crates/render/src/pack.rs`
- Test: `crates/ui/tests/snapshot.rs`, `crates/render/src/gpu.rs` unit test of the blend description

`Paint.alpha` defaults to `1.0`. Snapshot gains a trailing alpha only when it is below `1.0`, so the Task 1 strings stay valid. `0.0` omits the rectangle. The modal scrim uses alpha `0.6`.

`GpuVertex` stays 48 bytes. `overlay_vertex` grows a parameter `alpha: f32`. Alpha `1.0` still writes `uv: [-1.0, -1.0]`. A lower alpha writes `uv: [-1.0, alpha]`. `overlay.frag`, in the `uv.x < 0` branch, uses `v_uv.y` as alpha when `v_uv.y >= 0`, and uses `1.0` when `v_uv.y < 0`. The output stays premultiplied: `vec4(albedo * alpha, alpha)`.

`overlay_pipeline` is created with blend on (`make_pipeline(false, true, false)` at the call that is `make_pipeline(false, false, false)` today). The scene pipeline call stays `make_pipeline(true, true, false)`. Add `fn overlay_blend() -> (u32, u32)` returning `(1, 7)`, the source and destination color factors in the premultiplied branch (`src: 1`, `dst: 7`, one and one-minus-source-alpha). A unit test asserts `(1, 7)`. Do not open a device.

`ScreenRect` gains `alpha: f32`. Callers in the camera and the stress example set `1.0`. `screen_quads` passes it to `overlay_vertex`.

- [ ] **Step 1: Test** snapshot omission of alpha 1, omission of alpha 0, and `overlay_blend()` factors.

- [ ] **Step 2: Run** `cargo test -p genos-ui --test snapshot` and `cargo test -p genos-render overlay_blend` — expected: fail.

- [ ] **Step 3: Implement.** Update `docs/systems/renderer.md` Limits: the overlay blends, the scene pass does not change.

- [ ] **Step 4: Run** `cargo test -p genos-ui` and `cargo test -p genos-render overlay_blend` — expected: pass. Do not run the GPU frame tests.

- [ ] **Step 5: Commit** `ui, render: overlay rectangles carry alpha`

### Task 35: Image element

Dirtying the rect when `source` changes **requires #5.**

**Files:**
- Test: `crates/ui/tests/image.rs`
- Modify: `crates/ui/src/lib.rs`, `crates/ui/src/interact.rs`

```rust
pub struct ImagePaint { pub source: u32, pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub uv: [f32; 4] }
```

`Kind::Image { source, uv }` emits one `ImagePaint` and one flat placeholder `Paint` in `muted` so a build without the atlas step still shows a box. `uv` of `[0.0, 0.0, 1.0, 1.0]` passes through. A changed `source` dirties the rect. The element is not a press target.

- [ ] **Step 1: Test** the image record and the ignored press.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: image element describes a textured rect`

### Task 36: Overlay atlas

**Files:**
- Modify: `crates/render/src/gpu.rs`, `crates/render/src/pack.rs`
- Test: `crates/render/src/pack.rs` or a `#[cfg(test)]` module

**Produces:** `pub fn pack_atlas(images: &[(u32, u32, u32)]) -> Vec<Rect>` where each tuple is `(source, width, height)` and the rect is in the atlas. A shelf packer: rows 256 px tall, atlas width 512, a new shelf when the row overflows. Two 32×32 images sit at x 0 and x 32 of y 0.

`Renderer::set_overlay_images(&mut self, pixels: &[(u32, u32, u32, &[u8])])` replaces a UI atlas buffer. The overlay draw samples it for `ImagePaint` quads. Those quads stay in the overlay vertex list, with `shade` set to `0.25` and `uv` in the atlas. `overlay.frag` samples the new buffer only when `v_shade` is `0.25`, bilinear, with no ember blur. Binding 2 stays the particle image. The new buffer is the next binding. Do not renumber the existing writes in `write_descriptors`. Missing sources draw the placeholder and skip the sample. The packer test does not open Vulkan. The upload path may no-op when the device is absent.

Update `docs/systems/renderer.md` Game use.

- [ ] **Step 1: Test** the shelf positions, including a 300×40 image wrapping to the next shelf.

- [ ] **Step 2: Run** `cargo test -p genos-render pack_atlas` — expected: fail.

- [ ] **Step 3: Implement** the packer and the upload path beside the existing overlay vertex build.

- [ ] **Step 4: Run** `cargo test -p genos-render pack_atlas` and `cargo build -p genos-render` — expected: pass. Do not run GPU tests.

- [ ] **Step 5: Commit** `render: one atlas for overlay images`

### Task 37: Icon

**Files:**
- Modify: `crates/ui/src/text.rs`, `crates/ui/src/interact.rs`
- Test: `crates/ui/tests/icon.rs`

```rust
pub enum IconName { Close, Check, Chevron }
```

Each name has a 5×7 bitmask next to `glyph`. `Kind::Icon` paints those blots in the text color inside a 16×16 box. `IconName::Check` matches the checkbox mark's bits so the two stay visually one mark. A press does not emit.

- [ ] **Step 1: Test** that `Close` produces at least one blot and `Check` produces the same blot count as a checked checkbox's mark.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: built-in icons`

### Task 38: Color picker

Keeping the paints in one damage group **requires #5.**

**Files:**
- Test: `crates/ui/tests/color.rs`
- Modify: `crates/ui/src/controls.rs`

`Kind::ColorField { rgba }` in a fixed 120×80 box. The top 64 px are the saturation-value square. The bottom 16 px are the hue bar. A press in the middle of the square at hue 0 (the bar's left end, `rgba` starting at `[1, 0, 0, 1]`) emits `Event::Color` with green and blue near `0.5` (tolerance `0.08`) and alpha `1.0`. A press on the hue bar's right end emits red near `1` and green and blue near `0` only after the hue wraps: the right end is hue 1, which is red again. The midpoint of the bar emits a color whose max channel is 1 and whose min channel is 0. The field clamps. It does not allocate a texture. Paints are a grid of at most 8×8 quads plus the hue bar as 8 quads, so damage stays one group.

- [ ] **Step 1: Test** the center sample and the hue midpoint.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** HSV to RGB in `controls.rs`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: color picker square and hue bar`

### Task 39: Chart

The damage-split assertion **requires #5.**

**Files:**
- Create: `crates/ui/src/chart.rs`
- Test: `crates/ui/tests/chart.rs`

```rust
pub struct Series { pub name: String, pub color: [f32; 3], pub points: Vec<[f32; 2]> }
```

`Kind::Chart { series }` in a fixed 200×100 plot. Points are in data space. The axis fits the min and max of all points, with a 10% pad. A series of `(0, 0)` and `(1, 10)` produces one `Segment` whose second endpoint is above the first in screen space (smaller y). A hover inside the plot paints the nearest point's values with the bitmap. No event. Changing a point dirties the plot and not a sibling outside it. Leave `profile.rs` on its own painter.

- [ ] **Step 1: Test** the segment direction, the hover text, and the damage split.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass, including `profile`.

- [ ] **Step 5: Commit** `ui: chart series for debug tools`

### Task 40: Motion

The following-frame empty-damage assertion **requires #5.**

**Files:**
- Create: `crates/ui/src/motion.rs`
- Test: `crates/ui/tests/motion.rs`
- Modify: `crates/ui/src/interact.rs`

`Node.motion: f32` is the duration in seconds, default `0.0`. A duration of `0.2` lerps the painted rect and the background from the previous frame's result toward the layout result. At `dt = 0.1` the x is halfway. At `dt = 0.2` it equals the target and the following frame has no damage. `State.reduced_motion = true` snaps on the first frame. Color channels lerp the same way.

- [ ] **Step 1: Test** the halfway position, the settle, and reduced motion.

- [ ] **Step 2: Run** the test — expected: the rect jumps on the first dt.

- [ ] **Step 3: Implement.** Store the previous painted rect on `State` by id.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: lerp rect and color, with reduced motion`

Document in `ui.md`: motion is cosmetic. A value `Event` still reports the logical value on the press frame, not the lerped paint.

### Task 41: String table

**Files:**
- Create: `crates/ui/src/strings.rs`
- Test: `crates/ui/tests/strings.rs`
- Modify: `crates/ui/src/lib.rs`, `crates/ui/src/interact.rs`

```rust
pub enum TextSource { Plain(String), Key(String) }
pub struct StringTable { /* locale, map */ }
impl StringTable {
    pub fn resolve(&self, source: &TextSource) -> String
}
```

`Node.text` remains for `Plain`. `Node.source: Option<TextSource>` overrides it when set. `FrameArgs.strings` is `None` at every existing call, which resolves like an empty table. An empty table resolves a key to the key. Locale `"fr"` and key `"run"` resolve to the stored French string. A longer resolution increases the fit width of a label (`"Go"` is 24 px, `"Demarrer"` is 96 px, at `CELL_W` 12).

- [ ] **Step 1: Test** the missing key, the locale hit, and the fit width.

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement.** The default argument is `StringTable::empty()`, threaded like `TextInput`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: locale string table for labels`

### Task 42: Semantics

**Files:**
- Create: `crates/ui/src/semantics.rs`
- Test: `crates/ui/tests/semantics.rs`
- Modify: `crates/ui/src/interact.rs`

`Frame.semantics` lists every visible element that has a role. A button named `"Run"` is `Role::Button`, `enabled` follows `Kind`, `focused` follows `State.focus`. A disabled button is `enabled: false`. An open dialog is the last semantic. A clipped-away list row is absent.

- [ ] **Step 1: Test** those three cases.

- [ ] **Step 2: Run** the test — expected: `semantics` is empty.

- [ ] **Step 3: Implement** by filling the vec at the end of `frame` from `shown` and `Kind`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: semantic tree for accessibility checks`

Update `ui.md`: the accessibility surface is this list. A platform bridge is a later native call, not a crate.

### Task 43: World-root distance scale

**Files:**
- Modify: `crates/ui/src/layout.rs`
- Test: extend the world-root test in `layout.rs`

`Space::World` gains a scale: the rect's width and height multiply by `near / distance`, clamped to `0.25..=1.5`, where `near` is `4.0` world units. A screen root under the same camera move keeps its rect. A world root moved from 4 units ahead to 8 units ahead halves its width. A point behind the camera still lands off-screen as it does today (`project` returns `None`).

- [ ] **Step 1: Test** the half-size and the stable screen root. Reuse `Camera::opening` and the existing `world_ahead` helper.

- [ ] **Step 2: Run** `cargo test -p genos-ui world_root` — expected: the far rect matches the near rect.

- [ ] **Step 3: Implement** inside `world_rect`.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: world roots shrink with distance`

Update `ui.md` Limits: a world root is an overlay. The scene does not occlude it.

### Task 44: Rich text

**Files:**
- Modify: `crates/ui/src/strings.rs`, `crates/ui/src/text.rs`
- Test: `crates/ui/tests/rich_text.rs`

```rust
pub struct Run { pub text: String, pub color: [f32; 3] }
```

`TextSource::Runs(Vec<Run>)`. The runs paint in order on the bitmap baseline, each in its color. A wrap can break between runs and inside a run on whitespace. The fit width is the sum of the run widths on the widest line. A press still does nothing on a label.

- [ ] **Step 1: Test** two runs `"AA"` red and `"BB"` blue. The snapshot contains both colors. The fit width equals eight cells (`CELL_W * 4` is wrong: four characters are `48`).

- [ ] **Step 2: Run** the test — expected: fail.

- [ ] **Step 3: Implement** in `blots` by taking a color per character span. Plain text stays one run of the theme text color.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: rich text runs on the bitmap`

### Task 45: Outline face

**Files:**
- Modify: `crates/ui/src/text.rs`, `crates/ui/Cargo.toml` (add `genos-load`)
- Test: `crates/ui/tests/font.rs`

```rust
pub enum TextFace { Bitmap, Outline }
pub struct AtlasGlyph { pub advance: f32, pub w: u32, pub h: u32, pub pixels: Vec<u8> }
pub fn raster_outline(contours: &[Vec<genos_load::OutlinePoint>], units_per_em: u16, px: f32) -> AtlasGlyph
```

`Node.face` defaults to `TextFace::Bitmap`. `Outline` measures with the advance in pixels (`advance / units_per_em * px`) and paints one quad per glyph. The quad's alpha is the fraction of mask pixels that are set. The test loads `crates/load/fixtures/glyph.ttf` through `genos_load::load_font` and `FileSource`, rasterizes one glyph at 16 px, and checks `advance > 0` and a non-empty pixel vec. A label with `TextFace::Outline` and that atlas on `State` paints a rect wider than 0. The lighting panel does not set `face`, so a still lighting frame still matches itself, as in Task 1.

Coverage raster: even-odd fill on a grid of `px` by `px`. Off-curve points become midpoints between their neighbors, which matches a quadratic TrueType contour without a curve library.

- [ ] **Step 1: Test** the fixture glyph and the unchanged lighting snapshot.

- [ ] **Step 2: Run** `cargo test -p genos-ui --test font` — expected: fail.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: optional outline face from a loaded glyph`

Update `ui.md` Limits: shaping is left to right. No kerning and no bidi.

### Task 46: Replicated binding

The empty second-frame damage assertion, and dirtying the track when `snapshot` changes, **require #5.** The inspect rows and the predicted versus snapshot paint do not.

**Files:**
- Create: `crates/ui/src/replica.rs`
- Test: `crates/ui/tests/replica.rs`
- Modify: `crates/ui/src/lib.rs`, `crates/ui/src/layout.rs`, `crates/ui/src/interact.rs`, `docs/systems/ui.md`

`Node.replica` defaults to `None`. `inspect` and the paint rule are the ones in [Networking](#networking). A local slider still emits `Change` from its `Kind` value.

The pose test uses a 100×24 slider, min `0.0`, max `10.0`, entity `{ index: 1, generation: 1 }`, field `PoseX`, `predicted: Some(8.0)`, `snapshot: 2.0`. The thumb center is to the right of the muted mark. `inspect` reports `local: false`, that index and generation, `predicted: Some(8.0)`, `snapshot: Some(2.0)`, and `diverged: true`. A second identical frame has empty damage. Setting `snapshot` to `8.0` clears the mark and dirties the track.

The health test uses a label, entity `{ index: 2, generation: 1 }`, `predicted: Some(0.0)`, `snapshot: 40.0`, `pending: Some(100.0)`. The readout paints `40`. A second run paints `100` in `theme.accent`. `inspect` reports `predicted: None`, `snapshot: Some(40.0)`, `pending: Some(100.0)`, and `diverged: false`. A press emits `Event` and the readout stays `40`. `StunUntil` with `predicted: Some(9.0)` and `snapshot: 3.0` follows that same clear-predicted rule and paints `3`.

A node built without `replica` reports one local row with empty predicted, snapshot, and pending fields.

- [ ] **Step 1: Test** the local row, the diverged pose mark, the health snapshot readout, and the empty second-frame damage.

- [ ] **Step 2: Run** `cargo test -p genos-ui --test replica` — expected: fail.

- [ ] **Step 3: Implement** `replica.rs` and the paint branch. Existing trees stay local, so the lighting snapshot stays valid.

- [ ] **Step 4: Run** `cargo test -p genos-ui` — expected: pass.

- [ ] **Step 5: Commit** `ui: show predicted and snapshot values on bound elements`

Update `ui.md` Limits: a replicated element shows the snapshot value, and the owned pose can also show the predicted value. The crate still does not open a socket.

### Task 47: Gallery binary

The `damage` knob and `Frame::damage` strokes **require #5.** The gallery and the state inspector do not.

**Files:**
- Create: `examples/ui/Cargo.toml`, `examples/ui/src/main.rs`, `examples/ui/src/pages.rs`
- Modify: `Cargo.toml` workspace members, `Makefile`, `docs/architecture.md`, `docs/systems/ui.md`, `docs/systems/debugging.md`

The binary is `genos-ui-gallery`, package `genos-ui-gallery`. It is the gallery of every UI element this plan adds, plus a world root. The agent rubric's folder for this system is `debug/ui`. This step keeps `examples/ui`, the same way the physics plan keeps `examples/physics-view`, until the shared `debug/scenes` library exists.

Scaffold, copied from the stress frame and kept local. Do not extract a shared runtime crate in this commit. `docs/architecture.md` says to add one when a second program needs the loop. The camera and the stress example already each own a loop. A third copy is the change this step makes. Extracting the loop would rewrite frame timing in those programs and is outside this plan.

The gallery does:

1. Parse `--proof`, `--size WxH`, `--frames N`, `--script PATH`, `--report DIR`, `--page NAME`.
2. Load `examples/camera/scene.rhai` with `genos_scene::load_path`. That is the shared scene.
3. Open `Window::open_proof` when `--proof` is set, otherwise `Window::open`.
4. Open `Renderer`, `InputSystem`, `FocusGate`, and `genos_debug::Tools`.
5. Implement `genos_debug::Host` for the loaded scene. Knobs: `theme` (`dark` or `light`), `locale` (`en` or `fr`), `motion` (`on` or `off`), `damage` (`off` or `on`), `diverge` (`on` or `off`).
6. Each frame: `tools.scene_dt`, `pump`, `begin_frame`, apply keys and the mouse, `poll_gamepads`, build the page tree, call `inspect`, and place `inspector_column` to the right of the page in one root. Call `frame` on that root with the input system, `dt`, the string table, and the text input. Copy `wheel_y` from the window. Call `set_text_input` while a text field has UI focus.
7. If `look_capture && !keys_captured`, feed `character_controller` so the camera still moves when the pointer misses the UI.
8. Draw paints as `ScreenRect` with alpha. Draw segments with `set_overlay_lines`. Upload image paints when the page has an image.
9. When `Renderer::report_overlay_damage` exists, pass `Frame::damage`. Until the renderer plan lands, still compute damage. The `damage` knob strokes those rectangles so a person can see them.
10. `tools.before_draw` and `tools.after_draw` bookend the draw, the same way `examples/stress/src/main.rs` does.
11. `--frames N` exits after N draws. `--script` runs a debug script through `genos_debug::script::spawn`.

Pages in `pages.rs`, selected by the tab element and by `--page`:

| Page | Elements |
| --- | --- |
| `text` | Labels, rich text runs, both locales |
| `buttons` | Button, toggle, checkbox, radio |
| `values` | Slider, dial, knob, spinner, both progress bars |
| `fields` | Single-line, multiline, selection |
| `collections` | Dropdown, list of 1000, tree, table, tabs |
| `layers` | Scroll, tooltip, context menu, modal |
| `media` | Color picker, image from `genos-load` of a 4×4 generated RGBA, the three icons |
| `layout` | Stack, wrap, grid, a `Place::stretch` child |
| `motion` | A button whose `motion` is 0.2 s, the reduced-motion knob |
| `chart` | Two series |
| `world` | A screen-root frame rate label and a world root on the first lamp |
| `inspector` | A local button, an owned `PoseX` slider, and a `Health` label |

`pages.rs` is pure: `fn page(name: &str, model: &Model) -> Node`. The window draws `inspector_column(&inspect(&page))` beside that page, so every page carries the state inspector. The test lives in `examples/ui/src/pages.rs` behind `#[cfg(test)]` and does not open a window. The binary does not bind a UDP socket and does not link `genos-net`.

`Model::demo()` with `diverge: true` builds the inspector page as: a local button; a `PoseX` slider on entity `{ index: 1, generation: 1 }` with `predicted: Some(8.0)` and `snapshot: 2.0`; a `Health` label on entity `{ index: 2, generation: 1 }` with `snapshot: 40.0` and `predicted: None`. `diverge: false` sets that pose `predicted` equal to `snapshot`.

- [ ] **Step 1: Test** `page("buttons", &Model::demo())` contains a `Kind::Button`, a `Kind::Toggle`, a `Kind::Checkbox`, and two `Kind::Radio` of the same group. `page("world", &Model::demo())` returns a root whose child has `Space` applied by the caller: the function returns the world node separately as `Model::world_label() -> Node` with a fixed size. `frame` on the buttons page with a press over the first button emits `Press`. `inspect(&page("inspector", &Model::demo()))` returns a local row, a diverged `PoseX` row (`predicted` 8, `snapshot` 2), and a `Health` row whose `predicted` is empty and whose `snapshot` is 40. `inspector_column` of those rows is a label list that includes the entity index `1` and the word `snapshot`. `Model::demo()` with `diverge: false` reports `diverged: false` on the pose row.

- [ ] **Step 2: Run** `cargo test -p genos-ui-gallery` — expected: the package is missing.

- [ ] **Step 3: Implement** the pages and the binary. Add the workspace member and a `ui` Makefile target: `cargo run -p genos-ui-gallery`. Document the binary in `docs/systems/debugging.md` next to the stress commands, including `--proof --frames 2`, `--page inspector`, and the `diverge` knob. Document the game use in `docs/systems/ui.md`: build a `Node`, call `frame`, draw `paints` and `segments`, honor `look_capture` and `keys_captured`. A replicated element copies `ReplicaView` from the caller.

- [ ] **Step 4: Run** `cargo test -p genos-ui-gallery` and `cargo build -p genos-ui-gallery` — expected: pass. Do not launch the window and do not run `make verify`.

- [ ] **Step 5: Commit** `examples: UI gallery with a state inspector`

---

## Gallery debug binary

The binary is the visual check. The per-commit proof stays the headless paint tests. A person runs:

```sh
cargo run -p genos-ui-gallery
cargo run -p genos-ui-gallery -- --proof --size 320x180 --frames 2
```

`--proof` uses `Window::open_proof` and the headless Weston note in `docs/systems/debugging.md`. The scene file is `examples/camera/scene.rhai`. The debug tools are `genos_debug::Tools` and `Host`, so a script can `knob("theme", "light")` and `shot("buttons")` the same way a stress script shots a view.

The `damage` knob draws the dirty rectangles from `Frame::damage`. **requires #5.** That is how a person checks the reuse contract from PR #5 against a live control. The profile graph stays on the camera and the stress example. The gallery's chart page is the general graph.

The state inspector is a column of `inspect` rows for the current page. Each row shows the element id, `local` or the entity index and generation, the field, the predicted value, the snapshot value, any pending request, and whether the row diverged. The snapshot column is the confirmed value. The `diverge` knob moves the inspector page's owned pose away from its snapshot so the muted mark is visible. `diverge` off copies the snapshot into `predicted`. The inspector page is also `--page inspector`. The gallery does not open a socket. `genos-net-view` remains the view of many player runtimes.

## Not in this plan

- The renderer frame-reuse sequence in PR #5's `docs/frame-reuse-plan.md`. **requires #5.**
- A cached UI layer texture. PR #5's `docs/frame-reuse.md` already defers it. **requires #5.**
- A world root that the scene occludes, or a UI quad drawn with the meshes.
- Kerning, bidi, Arabic, and Indic shaping.
- A Wayland clipboard.
- AT-SPI or any accessibility crate.
- Docking, multi-window viewports, and a markup language.
- A Rhai builder for the tree. Constructors stay in Rust until a later change registers them on a host.
- Rewriting `ProfileGraph` onto `Kind::Chart`.
- Editing `examples/camera` or `examples/stress` beyond the `Pointer` fields this plan adds.
- The snapshot codec, the UDP socket, `InputCommand` sampling, and `genos-session`. Those stay in the core-networking plan.
- `genos-net-view`. That binary is the multi-runtime desync view. The gallery inspector reads one UI tree.
- A dependency from `genos-ui` on `genos-sim` or `genos-net`.

## Coverage

| Request | Step |
| --- | --- |
| Labels | 4 |
| Rich text | 44 |
| Buttons | 9 |
| Toggles | 10 |
| Checkboxes | 11 |
| Radio buttons | 12 |
| Sliders | 13 |
| Dials and knobs | 14 |
| Progress, linear and radial | 15 |
| Text field, single-line, selection | 26 |
| Multiline | 27 |
| IME | 28, 29 |
| Dropdowns and selectors | 25, and radio at 12 |
| List | 21 |
| Tree | 22 |
| Tables | 23 |
| Tabs | 24 |
| Scroll | 19, 20 |
| Modals and popups | 33, 18 |
| Tooltips | 31 |
| Context menus | 32 |
| Color pickers | 38 |
| Number spinners | 16 |
| Images | 35, 36 |
| Icons | 37 |
| Stack | 5, existing `Direction` |
| Grid | 7 |
| Flex | 5, 6 |
| Anchors | existing `Place`, gallery page at 47 |
| Theming and styling | 3, 40 |
| Animation and transitions | 40 |
| Focus, keyboard, gamepad | 17 |
| Localisation | 41 |
| Accessibility | 42 |
| World root and screen root | existing `Space`, scale at 43, gallery at 47 |
| Charts | 39 |
| Dirty-region reuse | **requires #5.** `DamageTracker` and the marked damage assertions |
| UI bound to replicated state | 46 |
| Local and replicated elements | 46 |
| Predicted and confirmed (snapshot) values | 46 |
| State inspector | 46, gallery column at 47 |
| Gallery binary | 47 |
