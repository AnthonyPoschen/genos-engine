#![cfg_attr(test, expect(clippy::expect_fun_call, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! Lighting settings: presets, the text form, and a scene file that overrides them.

use genos_scene::{
    load_str, AmbientMode, EngineSettings, LampPickMode, LightingPreset, LightingSettings,
    LIGHTING_BOUNCES_UNLIMITED,
};

fn room() -> &'static str {
    "floor(0.0, 0.0, 16.0, 16.0);\n\
     wall(2.0, 5.0, 8.0, 0.4, 2.6);\n\
     wall(6.0, 1.5, 0.4, 7.0, 2.6);\n\
     solid(\"square\", 0.0, 0.0, 1.5, 1.0, 0.0, 0.0);\n\
     solid(\"circle\", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0);\n\
     solid(\"square\", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0);\n\
     light(0.0, 4.0, 2.0, 1.0, 1.0, 1.0);\n"
}

#[test]
fn the_engine_settings_trait_reads_and_writes_lighting() {
    let settings = LightingSettings::preset(LightingPreset::Low);
    let text = EngineSettings::to_text(&settings);
    let parsed = <LightingSettings as EngineSettings>::parse(&text).expect("parse");
    assert_eq!(parsed, settings);
    let mut edited = parsed;
    EngineSettings::set_key(&mut edited, "probe_rays", "32").unwrap();
    assert_eq!(edited.probes.rays, 32);
    assert!(edited.direct.enabled);
    assert_eq!(edited.preset_label(), "custom");
    assert!(EngineSettings::to_text(&edited).starts_with("preset=custom\n"));
    let round =
        <LightingSettings as EngineSettings>::parse(&EngineSettings::to_text(&edited)).unwrap();
    assert_eq!(round.probes.rays, 32);
    assert_eq!(round.preset_label(), "custom");
}

#[test]
fn lamp_pick_round_trips_and_rejects_a_bad_mode() {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    assert_eq!(settings.lamp_pick.mode, LampPickMode::Hits);
    assert_eq!(settings.direct.max_lights, 0);
    settings.set_key("lamp_pick", "hits").unwrap();
    settings.set_key("lamp_pick_candidates", "8").unwrap();
    settings.set_key("lamp_pick_rays", "1").unwrap();
    settings.set_key("lamp_pick_radius", "0").unwrap();
    settings.set_key("lamp_pick_history", "20").unwrap();
    settings.set_key("lamp_pick_unbiased", "1").unwrap();
    settings.set_key("direct_lights", "3").unwrap();
    let parsed = LightingSettings::parse(&settings.to_text()).unwrap();
    assert_eq!(parsed, settings);
    assert_eq!(parsed.direct.max_lights, 3);
    assert_eq!(parsed.lamp_pick.rays, 1);
    assert!(settings.set_key("lamp_pick", "restir").is_err());
    settings.set_key("lamp_pick_samples", "2").unwrap();
    assert_eq!(settings.lamp_pick.rays, 2);
    assert_eq!(settings.direct.max_lights, 3);
    settings.set_key("lamp_pick_reuse", "on").unwrap();
    assert_eq!(settings.lamp_pick.mode, LampPickMode::Reuse);
    settings.set_key("lamp_pick_reuse", "off").unwrap();
    assert_eq!(settings.lamp_pick.mode, LampPickMode::Both);
    assert_eq!(settings.lamp_pick.rays, 2);
    settings
        .set_key("lamp_pick", "reuse (experimental)")
        .unwrap();
    assert_eq!(settings.lamp_pick.mode, LampPickMode::Reuse);
}

#[test]
fn gi_presets_sample_hits_and_reuse_is_not_a_preset() {
    for preset in [
        LightingPreset::High,
        LightingPreset::Gi1Bounce,
        LightingPreset::Low,
        LightingPreset::Medium,
    ] {
        assert_eq!(
            LightingSettings::preset(preset).lamp_pick.mode,
            LampPickMode::Hits,
            "{}",
            preset.name()
        );
    }
    for preset in [LightingPreset::Potato, LightingPreset::DayCycle] {
        assert_eq!(
            LightingSettings::preset(preset).lamp_pick.mode,
            LampPickMode::Off,
            "{}",
            preset.name()
        );
    }
    for preset in LightingPreset::all() {
        assert_ne!(
            LightingSettings::preset(*preset).lamp_pick.mode,
            LampPickMode::Reuse,
            "{}",
            preset.name()
        );
    }
}

#[test]
fn the_text_form_round_trips_every_preset() {
    for preset in LightingPreset::all() {
        let settings = LightingSettings::preset(*preset);
        let parsed = LightingSettings::parse(&settings.to_text()).expect(preset.name());
        assert_eq!(parsed, settings, "{}", preset.name());
    }
}

#[test]
fn potato_is_direct_shadows_and_flat_ambient() {
    let potato = LightingSettings::preset(LightingPreset::Potato);
    assert!(potato.direct.enabled);
    assert!(potato.shadows.enabled);
    assert!(!potato.gi.enabled);
    assert_eq!(potato.bounce.count, 0);
    assert!(!potato.probes.enabled);
    assert!(!potato.world_cache.enabled);
    assert!(!potato.denoiser.enabled);
    assert!(!potato.allocates_cache());
    assert_eq!(potato.ambient.mode, AmbientMode::Constant);
    assert!(!potato.gpu_buckets(1).compact);
    assert!(!potato.gpu_buckets(1).cache_rays);
    assert!(potato.gpu_buckets(1).direct);
    assert!(potato.gpu_buckets(1).shadows);
}

#[test]
fn high_launches_the_screen_wavelet_and_the_key_overrides_it() {
    let high = LightingSettings::preset(LightingPreset::High);
    assert!(high.denoiser.screen);
    assert!(high.to_text().contains("denoise_launch=screen"));
    assert_eq!(high.preset_label(), "high");
    for preset in LightingPreset::all() {
        if *preset == LightingPreset::High {
            continue;
        }
        let settings = LightingSettings::preset(*preset);
        assert!(
            !settings.denoiser.screen,
            "{} keeps the hash launch",
            preset.name()
        );
        assert!(
            settings.to_text().contains("denoise_launch=hash"),
            "{}",
            preset.name()
        );
    }
    // `--denoise-launch` and `GENOS_LIGHTING_DENOISE_LAUNCH` overlay this key
    // after the preset, the same path `set_key` uses.
    let mut over = high.clone();
    over.set_key("denoise_launch", "hash").unwrap();
    assert!(!over.denoiser.screen);
    assert!(over.to_text().contains("denoise_launch=hash"));
    assert_eq!(over.preset_label(), "custom");
    over.set_key("denoise_launch", "screen").unwrap();
    assert!(over.denoiser.screen);
    assert_eq!(over.preset_label(), "high");
    over.set_key("denoise_screen", "off").unwrap();
    assert!(!over.denoiser.screen);
    over.set_key("denoise_screen", "on").unwrap();
    assert!(over.denoiser.screen);
    let parsed = LightingSettings::parse(&over.to_text()).unwrap();
    assert!(parsed.denoiser.screen);
    over.set_key("full_effort", "on").unwrap();
    assert!(!over.denoiser.screen);
    over.set_key("full_effort", "off").unwrap();
    assert!(over.denoiser.screen);
    assert_eq!(over, LightingSettings::preset(LightingPreset::High));
}

#[test]
fn full_effort_spends_the_maximum_and_rewrites_every_pixel() {
    let mut settings = LightingSettings::preset(LightingPreset::Low);
    settings.set_key("full_effort", "on").unwrap();
    assert!(settings.is_full_effort());
    assert_eq!(settings.gi.budget_shift, 0);
    assert_eq!(settings.probe_rays(), 64);
    assert_eq!(settings.bounce.count, LIGHTING_BOUNCES_UNLIMITED);
    assert_eq!(settings.denoiser.iterations, 5);
    assert_eq!(settings.denoiser.dirty_fraction, 0.0);
    assert!(!settings.compose_reuse);
    assert_eq!(settings.direct.resolution_scale, 1);
    assert_eq!(settings.preset_label(), "custom");
    let text = settings.to_text();
    let parsed = LightingSettings::parse(&text).unwrap();
    assert!(parsed.is_full_effort());
    settings.set_key("full_effort", "off").unwrap();
    assert_eq!(settings, LightingSettings::preset(LightingPreset::High));
}

#[test]
fn day_cycle_and_one_bounce_sit_between_potato_and_low() {
    let names: Vec<_> = LightingPreset::all()
        .iter()
        .map(|preset| preset.name())
        .collect();
    assert_eq!(
        names,
        ["potato", "day-cycle", "gi-1bounce", "low", "medium", "high"]
    );
    let day = LightingSettings::preset(LightingPreset::DayCycle);
    assert!(!day.gi.enabled);
    assert_eq!(day.bounce.count, 0);
    assert!(day.shadows.enabled);
    assert!(day.sky);
    assert_eq!(day.ambient.mode, AmbientMode::Curve);
    assert!(!day.allocates_cache());
    let one = LightingSettings::preset(LightingPreset::Gi1Bounce);
    assert!(one.gi.enabled);
    assert_eq!(one.bounce.count, 1);
    assert!(!one.world_cache.enabled);
    assert!(!one.denoiser.enabled);
    assert!(!one.prefill);
    assert_eq!(one.probe_rays(), 64);
    assert!(one.probes_this_frame(1));
    assert!(!one.cache_rays_this_frame(1));
    let high = LightingSettings::preset(LightingPreset::High);
    assert_eq!(high.bounce.count, LIGHTING_BOUNCES_UNLIMITED);
    assert_eq!(high.probe_rays(), 64);
    assert_eq!(high.denoiser.iterations, 5);
    assert!(high.world_cache.enabled);
    assert!(high.prefill);
}

#[test]
fn high_is_the_tuned_path() {
    let high = LightingSettings::default();
    assert_eq!(high.preset, LightingPreset::High);
    assert!(high.gi.enabled);
    assert_eq!(high.bounce.count, LIGHTING_BOUNCES_UNLIMITED);
    assert_eq!(high.probe_rays(), 64);
    assert_eq!(high.denoiser.iterations, 5);
    assert!(high.denoiser.screen);
    assert!(!high.denoiser.adaptive);
    assert!(high.denoiser.plain());
    let medium = LightingSettings::preset(LightingPreset::Medium);
    assert_eq!(medium.denoiser.iterations, 3);
    assert!(!medium.denoiser.screen);
    assert!(!medium.denoiser.adaptive);
    assert!(medium.denoiser.plain());
    let low = LightingSettings::preset(LightingPreset::Low);
    assert_eq!(low.denoiser.iterations, 1);
    assert!(!low.denoiser.adaptive);
    assert!(low.denoiser.plain());
    assert!((high.denoiser.variance_threshold - 0.02).abs() < 1.0e-6);
    assert_eq!(high.world_mode(), 3);
    assert!(high.allocates_cache());
    let buckets = high.gpu_buckets(1);
    assert!(buckets.direct && buckets.shadows && buckets.place && buckets.probe_trace);
    assert!(buckets.cache && buckets.cache_rays && buckets.atrous && buckets.prefill);
}

#[test]
fn a_partial_line_overlays_a_preset() {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.set_key("bounces", "1").unwrap();
    settings.set_key("denoise_iters", "3").unwrap();
    settings.set_key("denoise_adaptive", "on").unwrap();
    assert_eq!(settings.bounce.count, 1);
    assert_eq!(settings.denoiser.iterations, 3);
    assert!(settings.denoiser.adaptive);
    let text = settings.to_text();
    assert!(text.contains("denoise_adaptive=1"));
    let parsed = LightingSettings::parse(&text).unwrap();
    assert!(parsed.denoiser.adaptive);
    assert!(settings.probes_this_frame(1));
    assert!(!settings.cache_rays_this_frame(1));
    assert!(!settings.atrous_this_frame(1));
    let mut two = settings.clone();
    two.set_key("bounces", "2").unwrap();
    assert!(two.cache_rays_this_frame(1));
    assert_eq!(two.world_mode(), 3);
    let mut off = settings;
    off.set_key("bounces", "0").unwrap();
    assert!(!off.probes_this_frame(1));
    assert!(!off.cache_rays_this_frame(1));
    assert!(!off.atrous_this_frame(1));
    assert!(!off.allocates_cache());
    let mut bad = LightingSettings::preset(LightingPreset::High);
    let err = bad.set_key("bounces", "4").unwrap_err();
    assert!(err.contains("0, 1, 2"), "{err}");
}

#[test]
fn a_scene_file_overrides_the_lighting_defaults() {
    let source = format!(
        "{}\nlighting_preset(\"potato\");\nlighting(\"direct_lights=3; ambient_intensity=0.4\");\n",
        room()
    );
    let scene = load_str(&source).expect("scene");
    assert_eq!(scene.lighting.preset, LightingPreset::Potato);
    assert!(!scene.lighting.gi.enabled);
    assert!(!scene.lighting.allocates_cache());
    assert_eq!(scene.lighting.direct.max_lights, 3);
    assert!((scene.lighting.ambient.intensity - 0.4).abs() < 1.0e-6);
    let err = load_str(&format!("{}\nlighting(\"nope=1\");\n", room())).unwrap_err();
    assert!(err.contains("unknown lighting setting"), "{err}");
    let err = load_str(&format!("{}\nlighting_preset(\"ultra\");\n", room())).unwrap_err();
    assert!(err.contains("unknown lighting preset"), "{err}");
}

#[test]
fn the_ambient_curve_interpolates_and_scales_the_sky_from_noon() {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.ambient.mode = AmbientMode::Curve;
    settings.ambient.night_color = [0.0, 0.0, 1.0];
    settings.ambient.night_intensity = 0.0;
    settings.ambient.dawn_color = [1.0, 0.0, 0.0];
    settings.ambient.dawn_intensity = 1.0;
    settings.ambient.noon_color = [1.0, 1.0, 1.0];
    settings.ambient.noon_intensity = 1.0;
    settings.ambient.dusk_color = [0.0, 1.0, 0.0];
    settings.ambient.dusk_intensity = 1.0;

    let at = |hours: f32| {
        let mut sample = settings.clone();
        sample.time_of_day = hours;
        sample.sanitize();
        let (color, _, intensity, _) = sample.ambient_at(0.2);
        (color, intensity, sample.sky_ambient_scale())
    };

    let (night, night_i, _) = at(0.0);
    assert_eq!(night, [0.0, 0.0, 1.0]);
    assert_eq!(night_i, 0.0);
    let (wrap, wrap_i, _) = at(24.0);
    assert_eq!(wrap, night);
    assert_eq!(wrap_i, night_i);

    let (dawn, dawn_i, _) = at(6.0);
    assert_eq!(dawn, [1.0, 0.0, 0.0]);
    assert_eq!(dawn_i, 1.0);
    let (noon, noon_i, noon_sky) = at(12.0);
    assert_eq!(noon, [1.0, 1.0, 1.0]);
    assert_eq!(noon_i, 1.0);
    assert_eq!(noon_sky, [1.0, 1.0, 1.0]);
    let (dusk, dusk_i, _) = at(18.0);
    assert_eq!(dusk, [0.0, 1.0, 0.0]);
    assert_eq!(dusk_i, 1.0);

    let (mid, mid_i, _) = at(3.0);
    assert!((mid[0] - 0.5).abs() < 1.0e-5, "{mid:?}");
    assert!((mid[2] - 0.5).abs() < 1.0e-5, "{mid:?}");
    assert!((mid_i - 0.5).abs() < 1.0e-5, "{mid_i}");
    let (morning, _, _) = at(9.0);
    assert!(
        (morning[0] - 1.0).abs() < 1.0e-5 && (morning[1] - 0.5).abs() < 1.0e-5,
        "{morning:?}"
    );
    let (afternoon, _, _) = at(15.0);
    assert!(
        (afternoon[0] - 0.5).abs() < 1.0e-5 && (afternoon[1] - 1.0).abs() < 1.0e-5,
        "{afternoon:?}"
    );
    let (evening, evening_i, evening_sky) = at(21.0);
    assert!(
        (evening[1] - 0.5).abs() < 1.0e-5 && (evening[2] - 0.5).abs() < 1.0e-5,
        "{evening:?}"
    );
    assert!((evening_i - 0.5).abs() < 1.0e-5);
    assert!(
        evening_sky[1] < 0.6 && evening_sky[2] < 0.6,
        "{evening_sky:?}"
    );

    settings.ambient.mode = AmbientMode::Day;
    settings.time_of_day = 21.0;
    assert_eq!(settings.sky_ambient_scale(), [1.0, 1.0, 1.0]);

    let mut clock = LightingSettings::preset(LightingPreset::High);
    clock.time_of_day = 18.0;
    assert_eq!(clock.preset_label(), "high");

    settings.set_key("ambient", "curve").unwrap();
    settings.set_key("time_of_day", "18").unwrap();
    assert_eq!(settings.ambient.mode, AmbientMode::Curve);
    assert_eq!(settings.time_of_day, 18.0);
    assert_eq!(settings.preset_label(), "custom");
    let text = settings.to_text();
    let parsed = LightingSettings::parse(&text).unwrap();
    assert_eq!(parsed.time_of_day, 18.0);
    assert_eq!(parsed.ambient.mode, AmbientMode::Curve);
    assert_eq!(parsed.curve_sample(18.0).0, [0.0, 1.0, 0.0]);

    let scene = load_str(&format!(
        "{}\nlighting(\"ambient=curve; time_of_day=6\");\ntime_of_day(9);\n",
        room()
    ))
    .expect("scene");
    assert_eq!(scene.lighting.ambient.mode, AmbientMode::Curve);
    assert!((scene.lighting.time_of_day - 9.0).abs() < 1.0e-5);
}
