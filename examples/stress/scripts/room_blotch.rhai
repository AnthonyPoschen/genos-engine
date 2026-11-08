// ADR 0039 stage 2 cache-only blotch. Two poses, no path trace.
// The grade reads this capture. It does not call compare, so a missing
// reference cannot start a CPU trace or add settling frames.
// Sun freeze is the process flag --freeze-sun, not a line in this file.
//
// The settle job draws the settle present, then exactly two more presents,
// and saves the last of those. settled_frame is the settle present. frame
// is the saved present, and those pixels are that present. A status read
// and a later wait are not that pair: status returns while the next present
// is already open, so the next command is taken one present later.

time(#{ fixed_dt: 1.0 / 60.0 });
let eye = [0.664, 1.699, 9.629];
let pitch = -0.12;
view("full");

fn flatten(text) {
    let flat = text;
    flat.replace("\n", " || ");
    flat
}

fn shoot(name, yaw, eye, pitch) {
    let cam = camera(#{ position: eye, yaw: yaw, pitch: pitch });
    log(`pose ${name} x=${cam.position[0]} y=${cam.position[1]} z=${cam.position[2]} yaw=${cam.yaw} pitch=${cam.pitch}`);
    let settled = wait_settled(#{
        timeout_frames: 900,
        then_frames: 2,
        path: `${name}-live.png`,
        width: 320,
        height: 180
    });
    log(`settle pose=${name} settled=${settled.settled} frames=${settled.frames} still_frames=${settled.still_frames} wall_ms=${settled.wall_ms}`);
    require(settled.settled == true, `unsettled ${name}`);
    log(`schedule pose=${name} settled_frame=${settled.settled_frame} frame=${settled.frame}`);
    require(settled.frame == settled.settled_frame + 2, `capture-schedule ${name}`);
    log(`present pose=${name} frame=${settled.frame} captured=${settled.captured}`);
    require(settled.captured == true, `missing-capture ${name}`);
    let settings = get_lighting_settings();
    log(`resolved pose=${name} ${flatten(settings.text)}`);
    save_text(`resolved-${name}.txt`, settings.text);
    require(settings.text.contains("toggles=cells_only"), `wrong-flags toggles ${name}`);
    require(settings.text.contains("surface_cache.sh_cells=1"), `wrong-flags sh_cells ${name}`);
    require(settings.text.contains("cache_async=0"), `wrong-flags cache_async ${name}`);
    require(settings.text.contains("world_cell=0.25"), `wrong-flags world_cell ${name}`);
    require(settings.text.contains("world_cache=1"), `wrong-flags world_cache ${name}`);
    require(settings.text.contains("bounces=unlimited"), `wrong-flags bounces ${name}`);
    require(settings.text.contains("sc_settle=0"), `wrong-flags sc_settle ${name}`);
}

shoot("room-a", -0.679, eye, pitch);
shoot("room-b", -36.0 * 0.0174533, eye, pitch);
