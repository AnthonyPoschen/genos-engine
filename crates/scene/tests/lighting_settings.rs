//! Lighting settings: presets, the text form, and a scene file that overrides them.

use genos_scene::{
    load_str, AmbientMode, EngineSettings, LightingPreset, LightingSettings,
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
fn high_is_the_tuned_path() {
    let high = LightingSettings::default();
    assert_eq!(high.preset, LightingPreset::High);
    assert!(high.gi.enabled);
    assert_eq!(high.bounce.count, LIGHTING_BOUNCES_UNLIMITED);
    assert_eq!(high.probe_rays(), 64);
    assert_eq!(high.denoiser.iterations, 5);
    assert_eq!(
        high.denoiser.resolution,
        genos_scene::DenoiseResolution::Full
    );
    assert!(!high.denoiser.adaptive);
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
    settings.set_key("denoise_res", "half").unwrap();
    assert_eq!(settings.bounce.count, 1);
    assert_eq!(settings.denoiser.iterations, 3);
    assert_eq!(
        settings.denoiser.resolution,
        genos_scene::DenoiseResolution::Half
    );
    let text = settings.to_text();
    assert!(text.contains("denoise_res=half"));
    let parsed = LightingSettings::parse(&text).unwrap();
    assert_eq!(
        parsed.denoiser.resolution,
        genos_scene::DenoiseResolution::Half
    );
    assert!(settings.cache_rays_this_frame(1));
    let mut off = settings;
    off.set_key("bounces", "0").unwrap();
    assert!(!off.cache_rays_this_frame(1));
    assert!(!off.atrous_this_frame(1));
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
