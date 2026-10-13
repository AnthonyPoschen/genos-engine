//! Brute-force path references, and the record of a full-effort capture.

use std::path::PathBuf;

use genos_debug::render_reference;
use genos_lighting_ref::store::{self, Manifest, ReferenceRecord, SkippedScene};
use genos_lighting_ref::Picture;

use super::scenes::{self, shot};

#[derive(Clone, Copy)]
pub struct TraceProfile {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub spp: u32,
    pub max_spp: u32,
    pub max_bounces: u32,
    pub seconds: f32,
    pub noise_target: f32,
}

impl TraceProfile {
    /// Owner GPU machine. The path trace is CPU work with a time cap.
    pub fn owner() -> Self {
        Self {
            name: "owner",
            width: 320,
            height: 180,
            spp: 8,
            max_spp: 64,
            max_bounces: 8,
            seconds: 20.0,
            noise_target: 0.05,
        }
    }

    /// Cloud quick check. One sample per pixel, short paths, a small picture.
    pub fn lavapipe_small() -> Self {
        Self {
            name: "lavapipe-small",
            width: 32,
            height: 18,
            spp: 1,
            max_spp: 1,
            max_bounces: 2,
            seconds: 3.0,
            noise_target: 1.0,
        }
    }
}

pub struct GenerateOptions {
    pub reference_dir: PathBuf,
    pub commit: String,
    pub profile: TraceProfile,
    pub only_scene: Option<String>,
    pub only_checkpoint: Option<String>,
}

pub fn generate(opts: &GenerateOptions) -> Result<Vec<ReferenceRecord>, String> {
    let mut records = Vec::new();
    let mut skipped = Vec::new();
    for scene in genos_lighting_ref::scenes() {
        if opts.only_scene.as_ref().is_some_and(|id| id != scene.id) {
            continue;
        }
        if !scene.available {
            skipped.push(SkippedScene {
                scene: scene.id.to_string(),
                reason: scene.skip_reason.to_string(),
            });
            continue;
        }
        for cp in genos_lighting_ref::checkpoints()
            .iter()
            .filter(|cp| cp.scene == scene.id)
        {
            if opts
                .only_checkpoint
                .as_ref()
                .is_some_and(|name| name != cp.name)
            {
                continue;
            }
            let Some(shot) = shot(cp.scene, cp.name) else {
                return Err(format!("no camera for {} {}", cp.scene, cp.name));
            };
            let setup = scenes::reference_setup(&shot, &opts.profile);
            let traced = render_reference(&setup);
            let linear = traced
                .linear
                .iter()
                .flat_map(|rgb| *rgb)
                .collect::<Vec<_>>();
            let picture = Picture::new(traced.image.width, traced.image.height, linear)?;
            let record = store::write_reference(
                &opts.reference_dir,
                ReferenceRecord {
                    scene: cp.scene.to_string(),
                    checkpoint: cp.name.to_string(),
                    commit: opts.commit.clone(),
                    method: "path".to_string(),
                    width: picture.width,
                    height: picture.height,
                    spp: traced.spp,
                    noise: traced.noise,
                    seconds: traced.seconds,
                    max_bounces: opts.profile.max_bounces,
                    png_bytes: 0,
                    linear_bytes: 0,
                    note: cp.note.to_string(),
                },
                &picture,
            )?;
            println!(
                "{} {}  {}x{}  {} spp  noise {:.4}  {:.2} s  png {}  linear {}",
                record.scene,
                record.checkpoint,
                record.width,
                record.height,
                record.spp,
                record.noise,
                record.seconds,
                store::format_bytes(record.png_bytes),
                store::format_bytes(record.linear_bytes)
            );
            records.push(record);
        }
    }
    store::write_manifest(
        &opts.reference_dir,
        &Manifest {
            commit: opts.commit.clone(),
            profile: opts.profile.name.to_string(),
            records: records.clone(),
            skipped,
        },
    )?;
    Ok(records)
}
