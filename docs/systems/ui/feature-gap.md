# UI feature gap

Status of the toolkit on `master` against the menu and HUD set. The living behavior is [ui.md](../ui.md). The design for the missing rows is [plan.md](plan.md). That plan's Gaps table is stale: tabs, settings-page scroll, and `PanelCheck` are already in the crate.

`works` means a game can use it through the public API today. `partial` means a narrower version exists. `missing` means there is no type and no paint path.

| Feature | Status | Evidence |
| --- | --- | --- |
| Layout, column and row | works | `Direction` is left-to-right or top-to-bottom (`crates/ui/src/layout.rs:24`). `Sizing` is fit, grow, fixed, or percent (`crates/ui/src/layout.rs:37`). Grow shares leftover space (`crates/ui/src/layout.rs:469`). |
| Layout, wrap and main-axis justify | missing | Flow places children on one line (`crates/ui/src/layout.rs:405`). `Node` has no `wrap` and no `Justify`. |
| Layout, grid | partial | On/off marks use a fixed three-column cell (`crates/ui/src/panel.rs:2093`). There is no `columns` field on `Node`. |
| Anchors | works | `Place::point` and `Place::stretch` (`crates/ui/src/layout.rs:114`). A placed child leaves the flow (`crates/ui/src/layout.rs:433`). |
| DPI scaling | missing | No scale field on `Node`, `State`, or `layout`. Sizes are window pixels. |
| Safe areas | missing | No inset for a handheld or a television overscan. The settings page uses a hard `8` px margin (`crates/ui/src/panel.rs:1603`). |
| Text shaping | missing | Labels are a 5×7 bitmap, uppercased (`crates/ui/src/text.rs:111`). The UI crate does not call `Font::glyph`. |
| Fonts | partial | The bitmap cell is 12×16 (`crates/ui/src/text.rs:7`). `genos-load` decodes a simple TrueType outline (`crates/load/src/ttf.rs:69`) and the fixture is `crates/load/fixtures/glyph.ttf`. The UI never loads it. Composite glyphs error (`crates/load/src/ttf.rs:77`). |
| Rich text | missing | `Node.text` is one `Option<String>` (`crates/ui/src/layout.rs:176`). |
| RTL | missing | Lines run left to right (`crates/ui/src/text.rs:40`). No bidi. |
| Wrapping | works | Whitespace wrap (`crates/ui/src/text.rs:67`). A word with no spaces stays one line (`docs/systems/ui.md`). |
| Ellipsis | missing | Glyphs past the width are dropped (`crates/ui/src/text.rs:42`). No ellipsis mark. |
| Localization | missing | Strings are stored on the node. No string table. |
| Buttons | works | `button_panel` (`crates/ui/src/panel.rs:624`) fires `Action::Press` (`crates/ui/src/lib.rs:66`). |
| Toggles and checkboxes | works | `PanelCheck` (`crates/ui/src/panel.rs:473`) fires `Action::Toggle` (`crates/ui/src/lib.rs:72`). Tests: `crates/ui/tests/checkbox.rs`. `Kind::Checkbox` is still absent. |
| Sliders | partial | `PanelSlider` is horizontal, numeric or named (`crates/ui/src/panel.rs:518`, `crates/ui/src/slider.rs:170`). No vertical slider and no `Kind::Slider`. |
| Dropdowns | partial | `PanelDropdown` opens one six-row list (`crates/ui/src/dropdown.rs:19`, `crates/ui/src/panel.rs:2323`). No `Kind::Dropdown`. |
| Radio | missing | No radio group and no `Event::Select` for one. |
| Text input and IME | missing | No caret, no selection, no commit string. `Pointer` has key edges for the panel (`crates/ui/src/panel.rs:64`) and no text. |
| Scroll views | partial | The settings page scrolls under a sticky title and tab bar (`crates/ui/src/panel.rs:1579`). Not a general scroll element. Tests: `crates/ui/tests/scroll.rs`. |
| Virtualized lists and tables | missing | No row window. Every laid-out child is a `Node`. |
| Tabs | works | `PanelTabs` (`crates/ui/src/panel.rs:448`) and `button_panel_blocks` (`crates/ui/src/panel.rs:697`). Tests: `crates/ui/tests/tabs.rs`. Not `Kind::Tabs`. |
| Modals and popups | partial | `FocusPolicy::Modal` is an input policy (`crates/scene/src/menu.rs:22`), not a dialog. The only overlay is the dropdown list (`crates/ui/src/panel.rs:2323`). |
| Tooltips | partial | A slider paints its own tip (`crates/ui/src/panel.rs:2500`) and sets `Frame.tooltip` (`crates/ui/src/panel.rs:302`). No delay and no shared layer. The tip is not a press target. |
| Context menus | missing | No right-button menu. |
| Drag and drop | partial | A draggable panel follows the pointer (`crates/ui/src/panel.rs:728`). No payload, no inventory grid, no drop target. |
| Progress bars | missing | The sun is a decorative disc (`crates/ui/src/panel.rs:2967`), not a value. |
| Images and 9-slice | missing | No image element. The overlay pipeline is untextured and unblended (`crates/render/src/gpu.rs:2615`). |
| Theming | partial | Omarchy `Palette` (`crates/ui/src/omarchy.rs:25`) plus per-node `Look` (`crates/ui/src/lib.rs:25`). No shared role names. |
| Animation and transitions | missing | No clock and no reduced-motion flag on `State`. |
| Focus and keyboard | partial | A press stores `State.focus` (`crates/ui/src/panel.rs:129`). Arrows walk the settings controls (`crates/ui/src/panel.rs:1072`). Tests: `crates/ui/tests/focus.rs`. |
| Gamepad navigation | missing | `GamepadDevice::down` reads the d-pad and the stick (`crates/input/src/device.rs:202`). There is no `pressed` edge for those codes and no `ui_actions` map. |
| Input rebinding UI | missing | `KeyBindings::rebind` exists (`crates/input/src/bindings.rs:137`). Nothing in `genos-ui` paints a rebind row. |
| Settings-menu building blocks | works | Folding sections, tabs, checks, sliders, and dropdowns (`crates/ui/src/panel.rs:697`). The lighting binary reads `debug/lighting/menu.txt`. |
| HUD, frame rate | works | `FpsOverlay` (`crates/ui/src/fps.rs:28`). |
| HUD, health, minimap, crosshair, damage numbers | missing | No bar, no map container, no crosshair node, no floating number. |
| Chat box | missing | No transcript element. |
| Notifications and toasts | missing | No toast layer. |
| World-space UI and nameplates | partial | `Space::World` projects a root (`crates/ui/src/layout.rs:210`). The sun follows the first lamp (`crates/ui/src/panel.rs:2967`). It does not scale with distance, the scene does not occlude it, and there is no nameplate. |
| Render to texture | missing | UI is rectangles in the overlay draw. A cached UI layer texture is deferred (`docs/systems/ui/plan.md`). |
| Accessibility, scaling and colorblind | missing | No UI scale and no palette variant for color vision. |
| Accessibility, screen reader | missing | No role, name, or value beside the paints. |
| Data binding | missing | No `ReplicaView`. The caller would have nowhere to hang a snapshot value. |
| Scripting hooks | missing | [ADR 0009](../../adr/0009-rhai-is-the-scripting-language.md) names Rhai. The plan does not register UI constructors on a host (`docs/systems/ui/plan.md`). |
| Debug inspector | partial | The profile graph has `inspect_region` (`crates/ui/src/profile.rs:330`). There is no UI-tree inspector and no `debug/ui` binary. |
| Perf, batching | partial | One overlay draw builds every solid rectangle (`crates/render/src/gpu.rs:1373`). |
| Perf, retained versus immediate | works | `Node` is retained and `layout` resolves it every frame (`crates/ui/src/layout.rs:238`). There is no second immediate-mode API. Dirty rectangles are not on `master`. `DamageTracker` is PR #5 and is not merged. |

## Priority that follows the plan

[plan.md](plan.md) is the design. Damage assertions stay skipped until PR #5 is on `master`. The build order for menus and HUDs is:

1. Outline face from `genos-load`'s TrueType loader. Shaping is left to right on `Glyph.advance`. No kerning, no bidi, no HarfBuzz (Task 45).
2. `frame` and `Kind` for label, button, toggle, checkbox, radio, slider, and progress (Tasks 2 through 4, 9 through 13, and 15).
3. Clip, one popup layer, a modal, and a tooltip (Tasks 8, 18, 31, and 33).
4. Text field, caret, in-process clipboard, commit, and preedit (Tasks 26 through 28 and 30).
5. Virtualized list and table (Tasks 21 and 23).
6. Image element and 9-slice (Task 35, plus a nine-quad image the plan does not name).
7. Keyboard and gamepad focus through `ui_actions` (Task 17).
8. String table (Task 41).

Still out of this pass, and still in the plan or outside it: dial, spinner, tree, rich text, Wayland wheel and text-input, overlay alpha in the renderer, color picker, chart, motion, replicated binding, the gallery binary, DPI, safe areas, ellipsis, RTL, inventory drag, chat, toasts, nameplates, colorblind palettes, a screen-reader socket, a rebind screen, and render-to-texture.
