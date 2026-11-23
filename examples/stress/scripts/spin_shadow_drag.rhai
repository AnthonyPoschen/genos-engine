// A camera turn dragged the disc-shadow mean with the screen: a pixel that was
// off the previous frame read the same screen pixel's history, which on a
// floor seen from above is another point at the same depth. Turn a full
// circle looking down and require the direct picture back at the start pose
// to match the settled one.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun_speed", 0.0);
knob("lamps", "still");
knob("boxes", "still");
view("direct");
let x = 12.33958;
let y = 1.7;
let z = 26.17126;
let yaw = 1.79;
let pitch = -0.55;
camera(#{ position: [x, y, z], yaw: yaw, pitch: pitch });
wait_settled(#{ timeout_frames: 600 });
wait(30);
capture(#{ name: "before", path: "spin-before.png" });
let turn = [];
for i in 0..121 {
    turn.push([x, y, z, yaw + i.to_float() * 2.0 * PI() / 120.0, pitch]);
}
camera(#{ path: turn, frames: 120 });
wait(122);
capture(#{ name: "back", path: "spin-back.png" });
let d = diff(#{ a: "before", b: "back", out: "spin-diff.png", threshold: 10 });
log(`back at the start pose: mean_abs ${d.mean_abs}, over_share ${d.over_share}`);
check_lt(d.over_share, 0.05, "the direct picture after a full turn matches the settled one");
