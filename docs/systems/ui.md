# UI

The UI owns layout, pointer hits, and the lighting panel. A menu and an in-game panel use the same tree.

## Goal

A game places UI on the viewport or on a world point. Focus, hover, and press change the look of an element. A player can drag a panel. On Omarchy, the lighting panel uses the current theme colors. A theme change updates those colors without a process restart. A camera frame shows the current frame rate and a line graph of stage time.

## Intent

Children in the flow use Clay sizing. A child with a place stays out of that flow.

Each axis is fit, grow, fixed, or percent of the parent. Grow shares leftover space. Open siblings finish the same size. A sibling that is already larger keeps that size.

A screen root stays on its viewport point when the camera moves. A world root follows a world point through the camera. Both roots use the same size rules.

The lighting frame draws a sun on the first lamp. The sun uses world space. The panel stays in screen space. A press on the sun does not move the lamp. That press allows camera look capture.

The panel moves that lamp with three sliders. Each slider has notches. Neighbor notches are 1 meter apart. A drag snaps to a notch and sets one axis. The Y slider includes a position below the floor and the height 7. Dim and bright scale the lamp color.

The same panel shows the picture modes `off` and `FXAA`. A press selects that mode for the next picture. That press does not start camera look capture.

The same panel shows `Run` or `Stop` for the red box path. `Run` starts the loop. `Stop` holds the box at its current point on that path. The next `Run` continues from that point. That press does not start camera look capture.

Hover, focus, and press each select a look. A drag moves a draggable panel by the pointer delta. The children move with the panel. A press on the panel does not start camera look capture.

Each panel has a wheel policy and a key policy. The game sets both, and it can replace either on a later frame. Modal owns that input while the panel is shown. Hover receives it only while the pointer is over the panel. Click-to-focus takes it on a click and releases it on a click outside, Escape, or grave. Passive never takes it. Input the panel does not take stays false on `Frame.consumed`, and the game keeps that event. Window focus, the display-server state, is a separate word. This policy is which panel inside the window receives the event.

`lighting_frame` reads `current/theme/colors.toml` when the process is on Omarchy. The `current` directory is `$XDG_STATE_HOME/omarchy/current` when `XDG_STATE_HOME` is set. Otherwise the directory is `~/.local/state/omarchy/current`. The process is on Omarchy only when that directory exists. Detection does not read `/etc/os-release`.

A canonical color key wins over its legacy alias. These pairs follow that rule:

- `background` wins over `bg`.
- `foreground` wins over `fg`.
- `dark_background` wins over `dark_bg`.
- `darker_background` wins over `darker_bg`.
- `lighter_background` wins over `lighter_bg`.
- `dark_foreground` wins over `dark_fg`.
- `light_foreground` wins over `light_fg`.
- `bright_foreground` wins over `bright_fg`.

A `#RRGGBB` value becomes the `[f32; 3]` color the panel paints. The mode is light when `colors.toml` sets `mode = "light"`, or when the theme directory contains `light.mode`. Otherwise the mode is dark.

The idle look uses the theme background and the theme foreground. The hover border uses the theme accent. The pressed background uses the theme accent. The same colors are on the panel and on each lamp control.

A missing `colors.toml` during a theme swap does not lock the panel. The panel keeps the previous colors for that `current` directory. The next read that succeeds applies the new colors. When the `current` directory is absent, the panel keeps the built-in looks.

The profiler has three modes. The `make run` command uses basic mode. `--profile off` draws no profiler. `--profile detailed` records GPU time and writes `genos-camera.profile`. The `--proof` check uses detailed mode when `--profile` is absent.

Each completed frame stores an ordering time and a CPU time for each stage. Stage labels are `window pump`, `input`, `ui`, `camera/scene update`, `simulation step`, and `gpu draw/present`. Basic mode stores GPU time 0. Detailed mode stores the Vulkan timestamp for the draw. A stage with no GPU work stores GPU time 0.

The graph draws one line for each CPU series. Detailed mode also draws one line for each GPU series. The on-screen graph keeps frames from the last 10 seconds. Age is the gap from the newest frame. A frame older than 10 seconds is dropped. A frame at 10 seconds stays.

The plot draws one box for each 250 ms interval. A CPU stage box is the average time in that interval. The DRAW box keeps the highest sample in that interval. The GPU box keeps the highest sample in that interval.

The readout still shows the latest frame. While the history is shorter than 10 seconds, the plot shows only those intervals. A full window starts 10 seconds before the newest frame. The picture is built at most 5 times a second. Pause and reset build on that frame.

While the graph is live, the readout shows the latest frame rate in frames per second. The readout shows each stage CPU time in milliseconds. Detailed mode also shows the draw GPU time. The on-screen names are `WINDOW`, `INPUT`, `UI`, `SCENE`, `SIM`, `DRAW`, and `GPU`.

P pauses the camera. The pause stops camera motion and the simulation step. The graph holds. New frames do not enter that graph. The picture keeps drawing the held graph. Those points stay in use until the interval changes.

A drag on the held graph chooses a time interval. The readout then shows the peak DRAW frame in that interval. Detailed mode also shows the peak GPU frame. R resets the graph. The graph follows new frames for the last 10 seconds.

Detailed mode appends each completed live frame to `genos-camera.profile`. That write does not sync the file to disk. A reader can recover the ordering time and each stage CPU and GPU time. Basic mode does not write that file. Basic mode does not query GPU timestamps.

## Code

The crate is `crates/ui`, package `genos-ui`.

- `layout` is in `src/layout.rs`.
- Text measure is in `src/text.rs`. The small bitmap for tick names is there too.
- Named and numeric slider values are in `src/slider.rs`.
- `lighting_frame`, `apply_lamp`, and `apply_frame_action` are in `src/panel.rs`.
- Omarchy theme colors are in `src/omarchy.rs`.
- `profile_overlay`, `remember_frame`, `ProfileGraph`, and `ProfileStream` are in `src/profile.rs`.
- `FpsOverlay` is in `src/fps.rs`. It draws one frame-rate line. Off, `note` returns without accumulating time or building glyphs. On, the glyphs are rebuilt at `rate_hz` (default 4, one 250 ms average) and the frames in between reuse those rectangles.
- `FocusPolicy` is in `genos-scene` (`crates/scene/src/menu.rs`). `InputPolicy`, `FocusManager`, and `Consumed` are in `src/focus.rs`. `MenuSettings.wheel` and `MenuSettings.keys` store the debug menu's two policies. `State::set_input_policy` writes the manager. `PANEL_ID` is the settings panel.

- `button_panel` lays out a draggable panel of labelled button rows (`PanelRow`, `PanelButton`). A press fires `Action::Press(id)` with the game's id. `selected` draws a button in its pressed look. Ids from `u32::MAX - 4095` up belong to the panel. A row may end in a `PanelSlider`. A press anywhere on the track sets the value under the pointer and a held press follows it, firing `Action::Slide { id, value }` each frame the value changes. With `steps` empty, the value snaps to `step` (`0` is continuous). Named `steps` (`SliderStep`, in `src/slider.rs`) space the names evenly on the track and report each step's own value, so the values need not be evenly spaced. Small dim names sit at the ticks. The current name is brighter. When those names would overlap, only the selected name is drawn. While the pointer is on the slider, `Frame.tooltip` is the name and the exact value, and the same text is painted above the thumb. `Pointer.wheel_y` positive steps toward the maximum. `Pointer.key_dec` and `Pointer.key_inc` are press edges: they step the slider under the pointer, or the slider that has UI focus. The stress example builds its numeric sliders from `genos-debug` knobs. The lighting debug binary groups those rows into folding sections (`button_panel_sections`, `PanelSection`). On/off marks in a section sit in a three-column grid. Each cell is the same width and the label wraps inside the cell. A press on a section title fires `Action::Press` with that id and folds the rows under it. `State` remembers the fold, so the next frame keeps it. `State::fold_section` sets a fold before the first frame. The lighting debug binary uses a dropdown for scene, checkpoint, view, shadow filter, and ambient, a slider for each ordered lighting grade, and a checkbox for each on/off switch. The frame-rate plate is a checkbox on the FPS row, and its update rate is the numeric slider on that row. A row may hold a `PanelDropdown`. The closed box shows the selected choice. A press opens a list over the rest of the panel. The list shows six rows and scrolls. The row under the pointer is highlighted. Arrow keys move that highlight, Enter picks it, and Escape or a press outside closes the list without a choice. The wheel scrolls the list and does not step a slider while it is open. A row may also hold `PanelCheck` marks. A press fires `Action::Toggle` with the next on state. The mark is a filled square inside the box. The caller passes `on` on the next frame.

## Game use

Build a `Node` tree. Call `layout` with `Space::Screen` or `Space::World`.

The camera frame calls `lighting_frame` each frame. The call passes the first lamp. That call loads the Omarchy theme when the `current` directory exists. A game does not pass a palette.

The frame calls `apply_frame_action` for each action. A picture selection returns the mode. The frame sets that mode on the renderer before the draw. A lamp action stays on the lamp path. `ToggleBoxRun` stays with the caller. The caller holds or resumes the red box path. The frame draws every paint in `Frame::paints` with `Renderer::draw_with_overlay`. The sun is one of those paints. The frame starts look capture only when `look_capture` is true.

Basic mode and detailed mode call `profile_overlay` and draw those paints. Detailed mode draws with `Renderer::draw_profiled`. That mode calls `ProfileStream::append` for each live sample. Basic mode draws with `Renderer::draw_with_overlay`.

`genos-lighting` keeps an `FpsOverlay` and calls `note` once a frame with the effective `FpsCounterSettings`. It draws `paints` in the overlay pass. A false `enabled` makes that call return without further work.

`button_panel_blocks` lays out `PanelBlock` values. A block is a fold (`PanelSection`), a tab strip (`PanelTabs`), or rows with no fold header (`PanelBlock::Rows`). A tab's blocks can be folds. A fold's `blocks` can be a tab strip. A fold can also sit outside every tab. A press on a tab shows that tab on the next frame and fires `Action::Press` with the tab id. The tab bar stays on screen while the page under it scrolls. `Pointer.tab_next` and `Pointer.tab_prev` are press edges that do the same for the strip. A header under the pointer cycles that strip. Otherwise the first strip cycles. An open dropdown keeps the keys. `State::select_tab` remembers the tab before the first frame, the same way `fold_section` remembers a fold. `State::panel_hovered` is true when the pointer was over the panel on the last frame, so a caller can reserve Tab for that case. Each tab is one line tall and as wide as its label. The tabs sit on one row. When that row is wider than the panel, it clips and the wheel over the row scrolls it sideways. Switching pages does not move the row. The selected tab uses the pressed look. With an Omarchy palette, that pressed look is the theme accent and an idle tab is the theme background. A press on a fold title still folds only that fold.

A game calls `State::set_input_policy` with the panel id. `PANEL_ID` is the settings panel. `wheel` and `keys` are separate `FocusPolicy` values: `Modal`, `Hover`, `ClickToFocus`, or `Passive`. `State::focused_panel` and `State::set_focused_panel` query and arm click-to-focus. `State::focused_control` is the control arrows move inside that panel. `Frame.consumed` reports `pointer`, `wheel`, `keys`, and `blur`. A false field falls through to the game. A panel with no policy keeps the older path and leaves `consumed` false. Click-to-focus arms on a click over the panel. A click outside does not set `consumed.pointer`, and `look_capture` stays true, so the game receives that press. Escape or grave (`Pointer.key_blur`) releases it and sets `consumed.blur`. Escape closes an open dropdown first. Modal sets `consumed.pointer` on a click outside and keeps `look_capture` false. It also sets `consumed.keys` on any key edge, including grave, which sets `consumed.blur` so a menu chord can leave the panel up. Hover delivers keys only while the pointer is over the panel. Passive ignores the pointer and the keys. A press then sets `look_capture` and fires no action. While keys are allowed, arrows walk checks, sliders, dropdowns, and press targets from top to bottom, then left to right. A focused slider steps instead. Enter activates the focused control. `Pointer.tab_next` and `Pointer.tab_prev` cycle a tab strip only while keys are allowed. The wheel policy gates the page, a slider, and an open dropdown. `Consumed.wheel` is true when one of those moved. A modal wheel policy also sets it for any notch, even when nothing moved.

`genos-lighting` draws the settings menu with `button_panel_blocks`. The panel's width is at most one third of the viewport. Every label wraps on spaces inside that width. A word with no spaces stays on one line, and glyphs that do not fit are left out, so the panel stays within that width. The groups come from `debug/lighting/menu.txt`. A line is `tab | section key | title | open or closed | setting`. A tab of `-` sits outside the tab strip. A section key of `-` puts the rows on the tab with no fold. The shipped pages are Scene, Pipeline, Lamps, GI, Denoise, Shadows, Perf, and Info. The Lamps page keeps an open fold of that name. GI keeps an open Probes fold and a closed World cache fold. The other pages are short, so they have no fold. Edit that file, or point `GENOS_MENU_LAYOUT` at another file of the same shape, and restart. Control labels use the lighting glossary. Config keys stay the same. The title is the scene name. A press on a title changes the fold, and the binary writes the folded names to `<out>/menu/collapsed.txt`. A press on a tab writes the page name to `<out>/menu/tab.txt`. The next launch reads both files. `]` and `[` change the page while the menu has the keys. Tab and shift-Tab do that too. The default key policy is click-to-focus, so a click on the menu is required first. The first grave releases that focus and leaves the menu up. The next grave hides it. F1 hides it either way. The title bar has a hide button. While the menu is hidden, a menu tab in the top-left corner shows it again. Shown or hidden, the open page, the folded section keys, and the wheel and key policies are `MenuSettings` on the scene. `--menu hidden` hides the panel. `get_menu_settings` and `set_menu_settings` read and write the same fields. Perf sets the policies from Wheel focus and Key focus. F toggles the frame-rate plate. F3 is the same key. The menu does not print those keys.

P pauses the graph. A drag inspects one interval. L follows live frames again.

## Limits

The settings panel scrolls when its page is taller than the window. The title and the tab bar stay put. A tab row wider than the panel scrolls sideways when the wheel is over that row. The page still scrolls when the wheel is over the page. The wheel moves the page when the wheel policy allows it, and not while the pointer is over a slider. A panel with no policy still scrolls while the pointer is over it. Hover is that rule. Click-to-focus scrolls after the panel has focus, including after the pointer leaves. Modal scrolls while the panel is shown. Passive does not. An open dropdown keeps the wheel when that policy allows it. There is no image and no font file. Text uses a built-in bitmap. Every printable ASCII character has its own glyph, including the apostrophe, the comma, and the semicolon. A character outside that set is a box. Tick names use the same bitmap at one pixel. The lighting panel is screen space. The panel controls one lamp. The picture labels are `off` and `FXAA`.

A slider draws its own tip while the pointer is on it. There is no general popup, and the tip is not a press target. The plan's tooltip layer is still absent. One dropdown list can be open. It has no scrollbar. It is not a general layer.

A sun marker follows the first lamp in world space. The marker is not a handle. Three notched sliders move that lamp. Each notch is 1 meter from the next. Y notches go below the floor.

The theme colors do not change the lamp, the radiance field, or a non-UI surface. The reader does not apply ANSI colors or gradient keys.

The graph has no pan, no zoom, and no free scroll. Pause holds the current window. A drag on the held graph chooses one interval. This crate does not read Vulkan timestamps. The profile does not time each Rust function. Timed work stays the frame stages.

The graph draws one point per 250 ms average, joined by 2 px lines (`ProfileView.segments`, drawn as thin quads in the overlay draw). It does not draw one point per frame.

`FpsOverlay` is the small frame-rate plate. `genos-lighting` draws it at the bottom right when `Scene.fps_counter` is on. The plate shows frames per second and the average frame time in milliseconds. The number changes at `rate_hz`. The default is 4 Hz. A disabled overlay does not keep a window and does not build text.

The expansion into a full set of UI elements is planned in [plan.md](ui/plan.md). This record stays the behavior that is in the crate today. The crate does not open a socket. Dirty rectangles are not in this tree. PR #5 adds `DamageTracker`, and the plan marks every step that reads it with **requires #5**.

The plan's networking contract, stated here for the rubric:

- Replicated state: an element is local, or the caller copies one component field (`Pose`, `Health`, or `StunUntil`) from an entity. Gesture, focus, and theme stay off the snapshot.
- Authority: the authority runtime writes replicated components. The element emits an event. The caller builds the command or the notice.
- Prediction / rollback: the owned pose may show a predicted value beside the snapshot value. `Health` and `StunUntil` show the snapshot value. This crate does not roll back a simulation.
- Determinism: the same tree, state, and frame arguments produce the same paints and the same inspector rows.
- Headless server: the authority runtime draws no UI. Gallery tests do not bind a UDP socket.

## Decisions

- [ADR 0006](../adr/0006-editor-is-linux-first.md) fits the editor to Omarchy first.
- The lighting panel uses that theme before an editor crate exists.
- The sun is a world-space indicator. It has no axis handles.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this reader on the Rust standard library.
- The reader does not take a TOML crate or a file watcher.
