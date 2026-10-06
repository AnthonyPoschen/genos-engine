# UI

The UI owns layout, pointer hits, and the lighting panel. A menu and an in-game panel use the same tree.

## Goal

A game places UI on the viewport or on a world point. Focus, hover, and press change the look of an element. A player can drag a panel.

## Intent

Children in the flow use Clay sizing. A child with a place stays out of that flow.

Each axis is fit, grow, fixed, or percent of the parent. Grow shares leftover space. Open siblings finish the same size. A sibling that is already larger keeps that size.

A screen root stays on its viewport point when the camera moves. A world root follows a world point through the camera. Both roots use the same size rules.

Hover, focus, and press each select a look. A drag moves a draggable panel by the pointer delta. The children move with the panel. A press on the panel does not start camera look capture.

## Code

The crate is `crates/ui`, package `genos-ui`.

- `layout` is in `src/layout.rs`.
- Text measure is in `src/text.rs`.
- `lighting_frame` and `apply_lamp` are in `src/panel.rs`.

## Game use

Build a `Node` tree. Call `layout` with `Space::Screen` or `Space::World`.

The camera frame calls `lighting_frame` each frame. The frame calls `apply_lamp` for each action. The frame draws `Frame::paints` with `Renderer::draw_with_overlay`. The frame starts look capture only when `look_capture` is true.

## Limits

There is no scroll, no image, and no font file. Text uses a built-in bitmap. The lighting panel is screen space. The panel controls one lamp.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps the window, the input, and the renderer out of this crate.
