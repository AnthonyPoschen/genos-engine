//! Brute-force path references, and the record of a full-effort capture.

use std::path::PathBuf;

use genos_debug::render_reference;
use genos_lighting_ref::baseline;
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
    /// Owner GPU machine. Batches of 16 samples until the noise estimate is
    /// 0.05, the checkpoint's sample cap, or the time cap. The cap is 256,
    /// and 1024 for room-a, so stored noise stays under
    /// [`genos_lighting_ref::REFERENCE_NOISE_CEILING`]. The achieved noise is
    /// stored per checkpoint and the score uses it. The path trace is CPU work.
    pub fn owner() -> Self {
        Self {
            name: "owner",
            width: 320,
            height: 180,
            spp: 16,
            max_spp: 256,
            max_bounces: 8,
            seconds: 120.0,
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

    /// Owner sample cap for this checkpoint. Lavapipe-small stays one sample.
    /// A higher cap stretches the time budget by the same factor so the
    /// sample count is what stops the trace.
    pub fn for_checkpoint(self, scene: &str, checkpoint: &str) -> Self {
        if self.name != "owner" {
            return self;
        }
        let Some(desc) = genos_lighting_ref::checkpoints()
            .iter()
            .find(|desc| desc.scene == scene && desc.name == checkpoint)
        else {
            return self;
        };
        if desc.owner_spp <= self.max_spp {
            return self;
        }
        let scale = desc.owner_spp as f32 / self.max_spp.max(1) as f32;
        Self {
            max_spp: desc.owner_spp,
            seconds: self.seconds * scale,
            ..self
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
                    max_bounces: setup.max_bounces,
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
    write_base_metrics(&opts.reference_dir, &opts.commit, &records)?;
    Ok(records)
}

/// `metrics.json` beside the pictures, so `--base <commit>` finds a file
/// before any live capture. A later check that scores pictures replaces it
/// with those candidate metrics.
fn write_base_metrics(
    dir: &std::path::Path,
    commit: &str,
    records: &[ReferenceRecord],
) -> Result<(), String> {
    let mut owned = Vec::new();
    for record in records {
        let picture = store::read_reference(dir, &record.scene, &record.checkpoint)?;
        let metrics = baseline::reference_metrics(&picture, f64::from(record.noise))?;
        owned.push((record.scene.clone(), record.checkpoint.clone(), metrics));
    }
    let refs: Vec<(&str, &str, &genos_lighting_ref::FrameMetrics)> = owned
        .iter()
        .map(|(scene, checkpoint, metrics)| (scene.as_str(), checkpoint.as_str(), metrics))
        .collect();
    let path = dir.join("metrics.json");
    baseline::write_metrics(&path, commit, &refs)?;
    println!("base metrics {}", path.display());
    Ok(())
}
