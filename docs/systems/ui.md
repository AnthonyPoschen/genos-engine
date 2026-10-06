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

The same panel shows the picture modes `off`, `FXAA`, and `SSAA`. A press selects that mode for the next picture. That press does not start camera look capture.

Hover, focus, and press each select a look. A drag moves a draggable panel by the pointer delta. The children move with the panel. A press on the panel does not start camera look capture.

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

P pauses the camera. The pause stops camera motion and the simulation step. The graph holds. New frames do not enter that graph. The picture keeps drawing the held graph. Those boxes stay in use until the interval changes.

A drag on the held graph chooses a time interval. The readout then shows the peak DRAW frame in that interval. Detailed mode also shows the peak GPU frame. R resets the graph. The graph follows new frames for the last 10 seconds.

Detailed mode appends each completed live frame to `genos-camera.profile`. That write does not sync the file to disk. A reader can recover the ordering time and each stage CPU and GPU time. Basic mode does not write that file. Basic mode does not query GPU timestamps.

## Code

The crate is `crates/ui`, package `genos-ui`.

- `layout` is in `src/layout.rs`.
- Text measure is in `src/text.rs`.
- `lighting_frame`, `apply_lamp`, and `apply_frame_action` are in `src/panel.rs`.
- Omarchy theme colors are in `src/omarchy.rs`.
- `profile_overlay`, `remember_frame`, `ProfileGraph`, and `ProfileStream` are in `src/profile.rs`.

## Game use

Build a `Node` tree. Call `layout` with `Space::Screen` or `Space::World`.

The camera frame calls `lighting_frame` each frame. The call passes the first lamp. That call loads the Omarchy theme when the `current` directory exists. A game does not pass a palette.

The frame calls `apply_frame_action` for each action. A picture selection returns the mode. The frame sets that mode on the renderer before the draw. A lamp action stays on the lamp path. The frame draws every paint in `Frame::paints` with `Renderer::draw_with_overlay`. The sun is one of those paints. The frame starts look capture only when `look_capture` is true.

Basic mode and detailed mode call `profile_overlay` and draw those paints. Detailed mode draws with `Renderer::draw_profiled`. That mode calls `ProfileStream::append` for each live sample. Basic mode draws with `Renderer::draw_with_overlay`.

P pauses the graph. A drag inspects one interval. L follows live frames again.

## Limits

There is no scroll, no image, and no font file. Text uses a built-in bitmap. The lighting panel is screen space. The panel controls one lamp. The picture labels are `off`, `FXAA`, and `SSAA`.

A sun marker follows the first lamp in world space. The marker is not a handle. Three notched sliders move that lamp. Each notch is 1 meter from the next. Y notches go below the floor.

The theme colors do not change the lamp, the radiance field, or a non-UI surface. The reader does not apply ANSI colors or gradient keys.

The graph has no pan, no zoom, and no free scroll. Pause holds the current window. A drag on the held graph chooses one interval. This crate does not read Vulkan timestamps. The profile does not time each Rust function. Timed work stays the frame stages.

The graph draws one box per 250 ms average. It does not draw one box per frame.

## Decisions

- [ADR 0006](../adr/0006-editor-is-linux-first.md) fits the editor to Omarchy first.
- The lighting panel uses that theme before an editor crate exists.
- The sun is a world-space indicator. It has no axis handles.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this reader on the Rust standard library.
- The reader does not take a TOML crate or a file watcher.
