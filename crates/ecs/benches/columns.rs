//! Column walk over 4096 one-component objects.
//!
//! Timing is a local print. The check is the sum, which does not depend on the machine.

use std::process::ExitCode;
use std::time::Instant;

use genos_ecs::{push_u32, read_u32, Component, DecodeError, World};

#[derive(Clone, Copy)]
struct Marker(u32);

impl Component for Marker {
    fn component_name() -> &'static str {
        "marker"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.0);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let value = read_u32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(Self(value))
    }
}

fn main() -> ExitCode {
    let mut world = World::new();
    let spawn_started = Instant::now();
    for value in 0..4096u32 {
        let handle = match world.spawn() {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("spawn failed: {err}");
                return ExitCode::from(1);
            }
        };
        if let Err(err) = world.insert(handle, Marker(value)) {
            eprintln!("insert failed: {err}");
            return ExitCode::from(1);
        }
    }
    let spawn_us = spawn_started.elapsed().as_micros();
    let mut sum = 0u64;
    let walk_started = Instant::now();
    world.for_each::<Marker>(|_, marker| sum += u64::from(marker.0));
    let walk_ns = walk_started.elapsed().as_nanos();
    let expected = 4095u64 * 4096 / 2;
    println!("spawn_4096_us {spawn_us}");
    println!("walk_ns {walk_ns}");
    println!("sum {sum}");
    if sum != expected {
        eprintln!("sum {sum} != {expected}");
        return ExitCode::from(1);
    }
    let cursor = world.clock().saturating_sub(1);
    let Some(handle) = world.handles_with::<Marker>().first().copied() else {
        return ExitCode::from(1);
    };
    if world.insert(handle, Marker(0)).is_err() {
        return ExitCode::from(1);
    }
    let deltas = world.changes_since(cursor);
    println!("one_edit_deltas {}", deltas.len());
    if deltas.len() != 1 {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
