// Held hall with the sun running, so compose takes the sun band.
// An unresolved miss must not leave the picture black after the cache fills.
// The counted pixels are examples/stress/tests/shadow_grain.rs.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "run");
knob("boxes", "still");
knob("lamps", "still");
knob("dynamic", 0);
jump_checkpoint(#{ name: "hall" });
wait(24);
let pic = luminance(#{ region: [0.0, 0.0, 1.0, 1.0], mode: "full" });
require(pic.luminance > 0.02, "held hall mean stayed dark");
require(pic.min > 0.01, "a held pixel stayed black");
require(pic.black_share < 0.05, "held hall black share");
