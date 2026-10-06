use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use rhai::{Dynamic, Engine};

use genos_math::Vec3;

use crate::types::{Floor, Light, Scene, Shape, Solid, Wall};

/// Load the shipped scene file through the Rhai host.
pub fn load_path(path: &Path) -> Result<Scene, String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    load_str(&text)
}

/// Run one script once and return the scene it placed.
pub fn load_str(source: &str) -> Result<Scene, String> {
    let builder = Rc::new(RefCell::new(Builder::default()));
    let mut engine = Engine::new();

    {
        let builder = builder.clone();
        engine.register_fn(
            "floor",
            move |x: Dynamic, z: Dynamic, width: Dynamic, depth: Dynamic| {
                builder.borrow_mut().floor = Some(Floor {
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    half_x: num(&width).abs() * 0.5,
                    half_z: num(&depth).abs() * 0.5,
                    color: [1.0, 1.0, 1.0],
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "wall",
            move |x: Dynamic, z: Dynamic, width: Dynamic, depth: Dynamic, height: Dynamic| {
                builder.borrow_mut().walls.push(Wall {
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    half_x: num(&width).abs() * 0.5,
                    half_z: num(&depth).abs() * 0.5,
                    height: num(&height),
                    color: [1.0, 1.0, 1.0],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "wall",
            move |x: Dynamic,
                  z: Dynamic,
                  width: Dynamic,
                  depth: Dynamic,
                  height: Dynamic,
                  reflectance: Dynamic,
                  color_mix: Dynamic| {
                builder.borrow_mut().walls.push(Wall {
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    half_x: num(&width).abs() * 0.5,
                    half_z: num(&depth).abs() * 0.5,
                    height: num(&height),
                    color: [1.0, 1.0, 1.0],
                    absorption: 0.0,
                    reflectance: num(&reflectance),
                    color_mix: num(&color_mix),
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "solid",
            move |shape: String,
                  x: Dynamic,
                  z: Dynamic,
                  size: Dynamic,
                  r: Dynamic,
                  g: Dynamic,
                  b: Dynamic| {
                let shape = match shape.as_str() {
                    "circle" => Shape::Circle,
                    _ => Shape::Square,
                };
                builder.borrow_mut().solids.push(Solid {
                    shape,
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    size: num(&size),
                    height: 1.2,
                    color: [num(&r), num(&g), num(&b)],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "solid",
            move |shape: String,
                  x: Dynamic,
                  z: Dynamic,
                  size: Dynamic,
                  r: Dynamic,
                  g: Dynamic,
                  b: Dynamic,
                  reflectance: Dynamic,
                  color_mix: Dynamic| {
                let shape = match shape.as_str() {
                    "circle" => Shape::Circle,
                    _ => Shape::Square,
                };
                builder.borrow_mut().solids.push(Solid {
                    shape,
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    size: num(&size),
                    height: 1.2,
                    color: [num(&r), num(&g), num(&b)],
                    absorption: 0.0,
                    reflectance: num(&reflectance),
                    color_mix: num(&color_mix),
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "light",
            move |x: Dynamic, y: Dynamic, z: Dynamic, r: Dynamic, g: Dynamic, b: Dynamic| {
                builder.borrow_mut().lights.push(Light {
                    position: Vec3::new(num(&x), num(&y), num(&z)),
                    color: [num(&r), num(&g), num(&b)],
                    direction: Vec3::ZERO,
                });
            },
        );
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "sun",
            move |x: Dynamic, y: Dynamic, z: Dynamic, r: Dynamic, g: Dynamic, b: Dynamic| {
                // xyz is the direction the rays travel. (0, -1, 1) is 45 degrees down toward +Z.
                builder.borrow_mut().lights.push(Light {
                    position: Vec3::new(0.0, 7.0, 0.0),
                    color: [num(&r), num(&g), num(&b)],
                    direction: Vec3::new(num(&x), num(&y), num(&z)),
                });
            },
        );
    }

    engine.run(source).map_err(|err| err.to_string())?;
    let built = builder.borrow();
    let floor = built.floor.clone().ok_or("script placed no floor")?;
    if built.walls.len() < 2 {
        return Err("script placed fewer than two walls".into());
    }
    if built.solids.len() < 3 {
        return Err("script placed fewer than three solids".into());
    }
    if built.lights.is_empty() {
        return Err("script placed no light".into());
    }
    Ok(Scene {
        floor,
        walls: built.walls.clone(),
        solids: built.solids.clone(),
        lights: built.lights.clone(),
    })
}

fn num(value: &Dynamic) -> f32 {
    if let Some(number) = value.as_float().ok() {
        return number as f32;
    }
    if let Some(number) = value.as_int().ok() {
        return number as f32;
    }
    0.0
}

#[derive(Default)]
struct Builder {
    floor: Option<Floor>,
    walls: Vec<Wall>,
    solids: Vec<Solid>,
    lights: Vec<Light>,
}
