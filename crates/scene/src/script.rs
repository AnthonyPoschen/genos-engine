use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use rhai::{Dynamic, Engine};

use genos_math::Vec3;

use crate::types::{Ceiling, Floor, Light, Scene, Shape, Solid, Wall, MAX_LAMPS, MAX_OCCLUDERS};

/// Load the shipped scene file through the Rhai host.
pub fn load_path(path: &Path) -> Result<Scene, String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    load_str(&text)
}

/// Run one script once and return the scene it placed.
///
/// Materials: `wall(x, z, width, depth, height, reflectance, color_mix)` and
/// `solid(shape, x, z, size, r, g, b, reflectance, color_mix)` take `reflectance` as a
/// plain albedo from 0 to 1, the share of arriving light the surface sends back (1 is
/// all of it); the engine divides by π where it needs radiance. A negative value is the
/// default, white paint at 0.8. `color_mix` is how much of the color tints the bounce
/// (negative: all of it). Overloads without them use both defaults.
/// `raised_wall(x, z, width, depth, height, base)` places a wall whose underside is
/// `base` metres above the floor.
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
        // `ceiling` is Rhai's own rounding function, so the roof is `roof`.
        engine.register_fn("roof", move |height: Dynamic| {
            builder.borrow_mut().ceiling = Some(Ceiling {
                height: num(&height),
                color: [1.0, 1.0, 1.0],
            });
        });
    }
    {
        let builder = builder.clone();
        engine.register_fn(
            "roof",
            move |height: Dynamic, r: Dynamic, g: Dynamic, b: Dynamic| {
                builder.borrow_mut().ceiling = Some(Ceiling {
                    height: num(&height),
                    color: [num(&r), num(&g), num(&b)],
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
                    base: 0.0,
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
                    base: 0.0,
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
        // A box from `base` to `base + height` above the floor: a lintel, a sill, a
        // beam or a roof slab.
        engine.register_fn(
            "raised_wall",
            move |x: Dynamic,
                  z: Dynamic,
                  width: Dynamic,
                  depth: Dynamic,
                  height: Dynamic,
                  base: Dynamic| {
                builder.borrow_mut().walls.push(Wall {
                    position: Vec3::new(num(&x), 0.0, num(&z)),
                    half_x: num(&width).abs() * 0.5,
                    half_z: num(&depth).abs() * 0.5,
                    height: num(&height),
                    base: num(&base).max(0.0),
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
                    yaw: 0.0,
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
                    yaw: 0.0,
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
    // The renderer has room for this many. Past it a lamp or an occluder would
    // silently drop out of the light.
    if built.lights.len() > MAX_LAMPS {
        return Err(format!(
            "script placed {} lights (lamps and suns); the renderer lights at most {MAX_LAMPS}",
            built.lights.len()
        ));
    }
    let occluders = built.walls.len() + built.solids.len();
    if occluders > MAX_OCCLUDERS {
        return Err(format!(
            "script placed {occluders} walls and solids ({} walls, {} solids); the renderer traces light against at most {MAX_OCCLUDERS}",
            built.walls.len(),
            built.solids.len()
        ));
    }
    if let Some(ceiling) = &built.ceiling {
        let top = built.walls.iter().map(|wall| wall.height).fold(0.0_f32, f32::max);
        // The baked light volumes cover the first 3 m above the floor.
        if ceiling.height <= 0.5 || ceiling.height > 3.0 {
            return Err("roof height must be above 0.5 m and at most 3 m".into());
        }
        if ceiling.height < top - 1.0e-3 {
            return Err("roof sits below a wall top".into());
        }
    }
    Ok(Scene {
        floor,
        walls: built.walls.clone(),
        solids: built.solids.clone(),
        lights: built.lights.clone(),
        ceiling: built.ceiling.clone(),
        sky: None,
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
    ceiling: Option<Ceiling>,
}
