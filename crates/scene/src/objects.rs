//! Placed objects are rows in the scene store.
//!
//! [`crate::Scene`] owns the walls, solids, and lights. [`publish`] copies
//! that geometry into a new store, floor then walls then solids then lights,
//! so two publishes of the same lists return the same handles. A menu or
//! lighting setting is not a row.
//!
//! `FixedPart` ordinals are positions in that handle order. The renderer
//! rebuilds them from the lists. The ordinal is not a second copy of the row.

use genos_ecs::{
    push_f32, push_u32, read_f32, read_u32, Component, DecodeError, SceneHandle, StoreError, World,
};
use genos_math::Vec3;

use crate::types::{Floor, Light, Scene, Shape, Solid, Wall};

impl Component for Floor {
    fn component_name() -> &'static str {
        "floor"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_vec3(out, self.position);
        push_f32(out, self.half_x);
        push_f32(out, self.half_z);
        push_color(out, self.color);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let position = read_vec3(&mut input)?;
        let half_x = read_f32(&mut input)?;
        let half_z = read_f32(&mut input)?;
        let color = read_color(&mut input)?;
        finish(
            input,
            Self {
                position,
                half_x,
                half_z,
                color,
            },
        )
    }
}

impl Component for Wall {
    fn component_name() -> &'static str {
        "wall"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_vec3(out, self.position);
        push_f32(out, self.half_x);
        push_f32(out, self.half_z);
        push_f32(out, self.height);
        push_f32(out, self.base);
        push_color(out, self.color);
        push_f32(out, self.absorption);
        push_f32(out, self.reflectance);
        push_f32(out, self.color_mix);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let position = read_vec3(&mut input)?;
        let half_x = read_f32(&mut input)?;
        let half_z = read_f32(&mut input)?;
        let height = read_f32(&mut input)?;
        let base = read_f32(&mut input)?;
        let color = read_color(&mut input)?;
        let absorption = read_f32(&mut input)?;
        let reflectance = read_f32(&mut input)?;
        let color_mix = read_f32(&mut input)?;
        finish(
            input,
            Self {
                position,
                half_x,
                half_z,
                height,
                base,
                color,
                absorption,
                reflectance,
                color_mix,
            },
        )
    }
}

impl Component for Solid {
    fn component_name() -> &'static str {
        "solid"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, shape_code(self.shape));
        push_vec3(out, self.position);
        push_f32(out, self.size);
        push_f32(out, self.height);
        push_f32(out, self.yaw);
        push_color(out, self.color);
        push_f32(out, self.absorption);
        push_f32(out, self.reflectance);
        push_f32(out, self.color_mix);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let shape = read_shape(&mut input)?;
        let position = read_vec3(&mut input)?;
        let size = read_f32(&mut input)?;
        let height = read_f32(&mut input)?;
        let yaw = read_f32(&mut input)?;
        let color = read_color(&mut input)?;
        let absorption = read_f32(&mut input)?;
        let reflectance = read_f32(&mut input)?;
        let color_mix = read_f32(&mut input)?;
        finish(
            input,
            Self {
                shape,
                position,
                size,
                height,
                yaw,
                color,
                absorption,
                reflectance,
                color_mix,
            },
        )
    }
}

impl Component for Light {
    fn component_name() -> &'static str {
        "light"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_vec3(out, self.position);
        push_color(out, self.color);
        push_vec3(out, self.direction);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let position = read_vec3(&mut input)?;
        let color = read_color(&mut input)?;
        let direction = read_vec3(&mut input)?;
        finish(
            input,
            Self {
                position,
                color,
                direction,
            },
        )
    }
}

/// Copy the draw lists into a new store.
///
/// Handles follow spawn order: floor, walls, solids, lights. The same lists
/// produce the same handles.
///
/// # Errors
///
/// The store cannot take another object.
pub fn publish(scene: &Scene) -> Result<World, StoreError> {
    let mut world = World::new();
    world.spawn_with(scene.floor.clone())?;
    for wall in scene.walls() {
        world.spawn_with(wall.clone())?;
    }
    for solid in scene.solids() {
        world.spawn_with(solid.clone())?;
    }
    for light in scene.lights() {
        world.spawn_with(light.clone())?;
    }
    Ok(world)
}

/// A scene and a store published from its draw lists.
pub struct Published {
    /// The scene the store was copied from.
    pub scene: Scene,
    /// Rows for the floor, walls, solids, and lights on [`Self::scene`].
    pub world: World,
}

impl Published {
    /// Publish `scene` and keep that store beside it.
    ///
    /// # Errors
    ///
    /// [`publish`] failed.
    pub fn new(scene: Scene) -> Result<Self, StoreError> {
        let world = publish(&scene)?;
        Ok(Self { scene, world })
    }

    /// Set menu visibility.
    ///
    /// The menu is a setting, not a placed row, so this does not write
    /// [`Self::world`].
    ///
    /// # Errors
    ///
    /// The menu key rejected the value.
    pub fn set_menu_visible(&mut self, visible: bool) -> Result<(), String> {
        self.scene
            .menu
            .set_key("visible", if visible { "shown" } else { "hidden" })
    }
}

/// Write a new position on the store row.
///
/// This is the published copy. The scene the copy was made from stays as it
/// was. An unchanged position does not write, so it emits no delta. A stale
/// handle increments [`genos_ecs::Stats::stale_rejects`] and writes nothing.
///
/// # Errors
///
/// [`StoreError::Stale`] when `handle` is not live. [`StoreError::Missing`]
/// when the row is not a floor, wall, solid, or light.
pub fn move_placed(
    world: &mut World,
    handle: SceneHandle,
    x: f32,
    y: f32,
    z: f32,
) -> Result<(), StoreError> {
    if try_move::<Floor>(world, handle, x, y, z)?
        || try_move::<Wall>(world, handle, x, y, z)?
        || try_move::<Solid>(world, handle, x, y, z)?
        || try_move::<Light>(world, handle, x, y, z)?
    {
        Ok(())
    } else {
        Err(StoreError::Missing)
    }
}

fn try_move<T>(
    world: &mut World,
    handle: SceneHandle,
    x: f32,
    y: f32,
    z: f32,
) -> Result<bool, StoreError>
where
    T: Component + Positioned,
{
    match world.get_mut::<T>(handle) {
        Err(StoreError::Missing) => Ok(false),
        Err(err) => Err(err),
        Ok(mut value) => {
            let next = Vec3::new(x, y, z);
            if same_vec(value.position(), next) {
                return Ok(true);
            }
            value.set_position(next);
            Ok(true)
        }
    }
}

trait Positioned {
    fn position(&self) -> Vec3;
    fn set_position(&mut self, position: Vec3);
}

impl Positioned for Floor {
    fn position(&self) -> Vec3 {
        self.position
    }
    fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }
}

impl Positioned for Wall {
    fn position(&self) -> Vec3 {
        self.position
    }
    fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }
}

impl Positioned for Solid {
    fn position(&self) -> Vec3 {
        self.position
    }
    fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }
}

impl Positioned for Light {
    fn position(&self) -> Vec3 {
        self.position
    }
    fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }
}

fn same_vec(left: Vec3, right: Vec3) -> bool {
    left.x.to_bits() == right.x.to_bits()
        && left.y.to_bits() == right.y.to_bits()
        && left.z.to_bits() == right.z.to_bits()
}

fn push_vec3(out: &mut Vec<u8>, value: Vec3) {
    push_f32(out, value.x);
    push_f32(out, value.y);
    push_f32(out, value.z);
}

fn push_color(out: &mut Vec<u8>, color: [f32; 3]) {
    push_f32(out, color[0]);
    push_f32(out, color[1]);
    push_f32(out, color[2]);
}

fn read_vec3(input: &mut &[u8]) -> Result<Vec3, DecodeError> {
    Ok(Vec3::new(
        read_f32(input)?,
        read_f32(input)?,
        read_f32(input)?,
    ))
}

fn read_color(input: &mut &[u8]) -> Result<[f32; 3], DecodeError> {
    Ok([read_f32(input)?, read_f32(input)?, read_f32(input)?])
}

fn shape_code(shape: Shape) -> u32 {
    match shape {
        Shape::Square => 1,
        Shape::Circle => 2,
    }
}

fn read_shape(input: &mut &[u8]) -> Result<Shape, DecodeError> {
    match read_u32(input)? {
        1 => Ok(Shape::Square),
        2 => Ok(Shape::Circle),
        _ => Err(DecodeError::Truncated),
    }
}

fn finish<T>(input: &[u8], value: T) -> Result<T, DecodeError> {
    if input.is_empty() {
        Ok(value)
    } else {
        Err(DecodeError::Trailing)
    }
}
