// ADR 0039 stage 2 cache-only blotch. Two poses, no path trace.
// The grade reads this capture. It does not call compare, so a missing
// reference cannot start a CPU trace or add settling frames.
// Sun freeze is the process flag --freeze-sun, not a line in this file.
time(#{ fixed_dt: 1.0 / 60.0 });
let eye = [0.664, 1.699, 9.629];
let pitch = -0.12;
view("full");

let cam = camera(#{ position: eye, yaw: -0.679, pitch: pitch });
log(`pose room-a x=${cam.position[0]} y=${cam.position[1]} z=${cam.position[2]} yaw=${cam.yaw} pitch=${cam.pitch}`);
let settled = wait_settled(#{ timeout_frames: 900 });
log(`settle pose=room-a settled=${settled.settled} frames=${settled.frames} still_frames=${settled.still_frames} wall_ms=${settled.wall_ms}`);
require(settled.settled == true, "unsettled room-a");
let settings = get_lighting_settings();
log(`resolved pose=room-a ${settings.text.replace("\n", " || ")}`);
require(settings.text.contains("toggles=cells_only"), "wrong-flags toggles room-a");
require(settings.text.contains("surface_cache.sh_cells=1"), "wrong-flags sh_cells room-a");
require(settings.text.contains("cache_async=0"), "wrong-flags cache_async room-a");
require(settings.text.contains("world_cell=0.25"), "wrong-flags world_cell room-a");
require(settings.text.contains("world_cache=1"), "wrong-flags world_cache room-a");
require(settings.text.contains("bounces=unlimited"), "wrong-flags bounces room-a");
let before = status();
log(`schedule pose=room-a before=${before.frame}`);
wait(2);
let after = status();
log(`schedule pose=room-a after=${after.frame}`);
require(after.frame == before.frame + 2, "capture-schedule room-a");
capture(#{ path: "room-a-live.png", width: 320, height: 180 });
let shot = status();
log(`present pose=room-a frame=${shot.frame} picture_settled=${shot.picture_settled} stable=${shot.stable} still_frames=${shot.still_frames}`);

cam = camera(#{ position: eye, yaw: -36.0 * 0.0174533, pitch: pitch });
log(`pose room-b x=${cam.position[0]} y=${cam.position[1]} z=${cam.position[2]} yaw=${cam.yaw} pitch=${cam.pitch}`);
settled = wait_settled(#{ timeout_frames: 900 });
log(`settle pose=room-b settled=${settled.settled} frames=${settled.frames} still_frames=${settled.still_frames} wall_ms=${settled.wall_ms}`);
require(settled.settled == true, "unsettled room-b");
settings = get_lighting_settings();
log(`resolved pose=room-b ${settings.text.replace("\n", " || ")}`);
require(settings.text.contains("toggles=cells_only"), "wrong-flags toggles room-b");
require(settings.text.contains("surface_cache.sh_cells=1"), "wrong-flags sh_cells room-b");
require(settings.text.contains("cache_async=0"), "wrong-flags cache_async room-b");
require(settings.text.contains("world_cell=0.25"), "wrong-flags world_cell room-b");
require(settings.text.contains("world_cache=1"), "wrong-flags world_cache room-b");
require(settings.text.contains("bounces=unlimited"), "wrong-flags bounces room-b");
before = status();
log(`schedule pose=room-b before=${before.frame}`);
wait(2);
after = status();
log(`schedule pose=room-b after=${after.frame}`);
require(after.frame == before.frame + 2, "capture-schedule room-b");
capture(#{ path: "room-b-live.png", width: 320, height: 180 });
shot = status();
log(`present pose=room-b frame=${shot.frame} picture_settled=${shot.picture_settled} stable=${shot.stable} still_frames=${shot.still_frames}`);
