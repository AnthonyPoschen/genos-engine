//! Draw order for the walls, solids, and lights stored on [`Scene`].
//!
//! The values live in the archetype columns. Each kind also keeps the handle
//! order a script or a removal produced. The renderer rebuilds `FixedPart`
//! ordinals from that order, so a wall that was index 2 is still index 2
//! until something removes a wall in front of it.

#![expect(
    clippy::panic,
    reason = "a draw-list index panics when the ordinal is out of range, the same contract as a slice"
)]
#![expect(
    clippy::expect_used,
    reason = "the floor row is the first spawn in an empty store"
)]

use genos_ecs::{SceneHandle, World};

use crate::types::{Floor, Light, Scene, Solid, Wall};
use crate::{
    DebugSettings, EventLoopSettings, FogSettings, FpsCounterSettings, HdrSettings,
    LightingSettings, MenuSettings, ReflectionSettings,
};

impl Scene {
    /// An empty room: one floor row, no walls, solids, or lights.
    ///
    /// # Panics
    ///
    /// The floor row is the first spawn. That spawn does not fail.
    #[must_use]
    pub fn empty(floor: Floor) -> Self {
        let mut objects = World::new();
        let floor_handle = objects.spawn_with(floor.clone()).expect("floor");
        Self {
            floor,
            objects,
            floor_handle,
            wall_order: Vec::new(),
            solid_order: Vec::new(),
            light_order: Vec::new(),
            ceiling: None,
            sky: None,
            lighting: LightingSettings::default(),
            debug: DebugSettings::default(),
            fps_counter: FpsCounterSettings::default(),
            menu: MenuSettings::default(),
            event_loop: EventLoopSettings::default(),
            reflections: ReflectionSettings::default(),
            hdr: HdrSettings::default(),
            fog: FogSettings::default(),
        }
    }

    /// Place `walls`, then `solids`, then `lights`, after the floor row.
    ///
    /// The vec order is the draw order. Handles follow that order.
    #[must_use]
    pub fn from_lists(
        floor: Floor,
        walls: Vec<Wall>,
        solids: Vec<Solid>,
        lights: Vec<Light>,
    ) -> Self {
        let mut scene = Self::empty(floor);
        scene.extend_walls(walls);
        scene.extend_solids(solids);
        scene.extend_lights(lights);
        scene
    }

    /// Copy of `self` with a new draw list. Settings and the floor stay.
    #[must_use]
    pub fn replace_parts(&self, walls: Vec<Wall>, solids: Vec<Solid>, lights: Vec<Light>) -> Self {
        let mut scene = Self::from_lists(self.floor.clone(), walls, solids, lights);
        scene.ceiling.clone_from(&self.ceiling);
        scene.sky.clone_from(&self.sky);
        scene.lighting.clone_from(&self.lighting);
        scene.debug.clone_from(&self.debug);
        scene.fps_counter = self.fps_counter;
        scene.menu.clone_from(&self.menu);
        scene.event_loop = self.event_loop;
        scene.reflections = self.reflections;
        scene.hdr.clone_from(&self.hdr);
        scene.fog.clone_from(&self.fog);
        scene
    }

    /// Write the floor field into its row when the bits differ.
    pub fn sync_floor(&mut self) {
        let stale = self
            .objects
            .get::<Floor>(self.floor_handle)
            .is_none_or(|row| !same_floor(row, &self.floor));
        if !stale {
            return;
        }
        let floor = self.floor.clone();
        let handle = self.floor_handle;
        let _ = self.objects.insert(handle, floor);
    }

    /// The store the draw lists name.
    #[must_use]
    pub const fn object_world(&self) -> &World {
        &self.objects
    }

    /// Handle of the floor row. It is index 0, generation 1, on a scene built
    /// by [`Self::empty`] or [`Self::from_lists`].
    #[must_use]
    pub const fn floor_handle(&self) -> SceneHandle {
        self.floor_handle
    }

    /// Walls in draw order.
    pub fn walls(&self) -> impl Iterator<Item = &Wall> {
        let objects = &self.objects;
        self.wall_order
            .iter()
            .filter_map(|handle| objects.get(*handle))
    }

    /// Solids in draw order.
    pub fn solids(&self) -> impl Iterator<Item = &Solid> {
        let objects = &self.objects;
        self.solid_order
            .iter()
            .filter_map(|handle| objects.get(*handle))
    }

    /// Lights in draw order.
    pub fn lights(&self) -> impl Iterator<Item = &Light> {
        let objects = &self.objects;
        self.light_order
            .iter()
            .filter_map(|handle| objects.get(*handle))
    }

    #[must_use]
    pub fn wall_len(&self) -> usize {
        self.wall_order.len()
    }

    #[must_use]
    pub fn solid_len(&self) -> usize {
        self.solid_order.len()
    }

    #[must_use]
    pub fn light_len(&self) -> usize {
        self.light_order.len()
    }

    #[must_use]
    pub fn walls_cloned(&self) -> Vec<Wall> {
        self.walls().cloned().collect()
    }

    #[must_use]
    pub fn solids_cloned(&self) -> Vec<Solid> {
        self.solids().cloned().collect()
    }

    #[must_use]
    pub fn lights_cloned(&self) -> Vec<Light> {
        self.lights().cloned().collect()
    }

    /// # Panics
    ///
    /// `index` is past the last wall.
    #[must_use]
    pub fn wall(&self, index: usize) -> &Wall {
        match self.wall_at(index) {
            Some(wall) => wall,
            None => panic!("wall {index} is not in the draw list"),
        }
    }

    /// # Panics
    ///
    /// `index` is past the last solid.
    #[must_use]
    pub fn solid(&self, index: usize) -> &Solid {
        match self.solid_at(index) {
            Some(solid) => solid,
            None => panic!("solid {index} is not in the draw list"),
        }
    }

    /// # Panics
    ///
    /// `index` is past the last light.
    #[must_use]
    pub fn light(&self, index: usize) -> &Light {
        match self.light_at(index) {
            Some(light) => light,
            None => panic!("light {index} is not in the draw list"),
        }
    }

    #[must_use]
    pub fn wall_at(&self, index: usize) -> Option<&Wall> {
        let handle = *self.wall_order.get(index)?;
        self.objects.get(handle)
    }

    #[must_use]
    pub fn solid_at(&self, index: usize) -> Option<&Solid> {
        let handle = *self.solid_order.get(index)?;
        self.objects.get(handle)
    }

    #[must_use]
    pub fn light_at(&self, index: usize) -> Option<&Light> {
        let handle = *self.light_order.get(index)?;
        self.objects.get(handle)
    }

    /// # Panics
    ///
    /// `index` is past the last wall.
    pub fn wall_mut(&mut self, index: usize) -> &mut Wall {
        match self.wall_at_mut(index) {
            Some(wall) => wall,
            None => panic!("wall {index} is not in the draw list"),
        }
    }

    /// # Panics
    ///
    /// `index` is past the last solid.
    pub fn solid_mut(&mut self, index: usize) -> &mut Solid {
        match self.solid_at_mut(index) {
            Some(solid) => solid,
            None => panic!("solid {index} is not in the draw list"),
        }
    }

    /// # Panics
    ///
    /// `index` is past the last light.
    pub fn light_mut(&mut self, index: usize) -> &mut Light {
        match self.light_at_mut(index) {
            Some(light) => light,
            None => panic!("light {index} is not in the draw list"),
        }
    }

    #[must_use]
    pub fn wall_at_mut(&mut self, index: usize) -> Option<&mut Wall> {
        let handle = *self.wall_order.get(index)?;
        self.objects.borrow_mut(handle).ok()
    }

    #[must_use]
    pub fn solid_at_mut(&mut self, index: usize) -> Option<&mut Solid> {
        let handle = *self.solid_order.get(index)?;
        self.objects.borrow_mut(handle).ok()
    }

    #[must_use]
    pub fn light_at_mut(&mut self, index: usize) -> Option<&mut Light> {
        let handle = *self.light_order.get(index)?;
        self.objects.borrow_mut(handle).ok()
    }

    #[must_use]
    pub fn wall_handle(&self, index: usize) -> Option<SceneHandle> {
        self.wall_order.get(index).copied()
    }

    #[must_use]
    pub fn solid_handle(&self, index: usize) -> Option<SceneHandle> {
        self.solid_order.get(index).copied()
    }

    #[must_use]
    pub fn light_handle(&self, index: usize) -> Option<SceneHandle> {
        self.light_order.get(index).copied()
    }

    pub fn push_wall(&mut self, wall: Wall) {
        if let Ok(handle) = self.objects.spawn_with(wall) {
            self.wall_order.push(handle);
        }
    }

    pub fn push_solid(&mut self, solid: Solid) {
        if let Ok(handle) = self.objects.spawn_with(solid) {
            self.solid_order.push(handle);
        }
    }

    pub fn push_light(&mut self, light: Light) {
        if let Ok(handle) = self.objects.spawn_with(light) {
            self.light_order.push(handle);
        }
    }

    pub fn extend_walls(&mut self, walls: impl IntoIterator<Item = Wall>) {
        for wall in walls {
            self.push_wall(wall);
        }
    }

    pub fn extend_solids(&mut self, solids: impl IntoIterator<Item = Solid>) {
        for solid in solids {
            self.push_solid(solid);
        }
    }

    pub fn extend_lights(&mut self, lights: impl IntoIterator<Item = Light>) {
        for light in lights {
            self.push_light(light);
        }
    }

    /// Replace the wall draw list. Handles of the previous walls are freed.
    pub fn set_walls(&mut self, walls: Vec<Wall>) {
        self.clear_walls();
        self.extend_walls(walls);
    }

    /// Replace the solid draw list. Handles of the previous solids are freed.
    pub fn set_solids(&mut self, solids: Vec<Solid>) {
        self.clear_solids();
        self.extend_solids(solids);
    }

    /// Replace the light draw list. Handles of the previous lights are freed.
    pub fn set_lights(&mut self, lights: Vec<Light>) {
        self.clear_lights();
        self.extend_lights(lights);
    }

    pub fn clear_walls(&mut self) {
        for handle in self.wall_order.drain(..) {
            let _ = self.objects.despawn(handle);
        }
    }

    pub fn clear_solids(&mut self) {
        for handle in self.solid_order.drain(..) {
            let _ = self.objects.despawn(handle);
        }
    }

    pub fn clear_lights(&mut self) {
        for handle in self.light_order.drain(..) {
            let _ = self.objects.despawn(handle);
        }
    }

    /// # Panics
    ///
    /// `index` is past the last wall, or the row is missing.
    pub fn remove_wall(&mut self, index: usize) -> Wall {
        remove_at::<Wall>(&mut self.objects, &mut self.wall_order, index)
    }

    /// # Panics
    ///
    /// `index` is past the last solid, or the row is missing.
    pub fn remove_solid(&mut self, index: usize) -> Solid {
        remove_at::<Solid>(&mut self.objects, &mut self.solid_order, index)
    }

    /// # Panics
    ///
    /// `index` is past the last light, or the row is missing.
    pub fn remove_light(&mut self, index: usize) -> Light {
        remove_at::<Light>(&mut self.objects, &mut self.light_order, index)
    }
}

fn remove_at<T: genos_ecs::Component>(
    objects: &mut World,
    order: &mut Vec<SceneHandle>,
    index: usize,
) -> T {
    let handle = order[index];
    let value = objects.get::<T>(handle).cloned();
    let _ = objects.despawn(handle);
    order.remove(index);
    match value {
        Some(value) => value,
        None => panic!("draw list row is not live"),
    }
}

fn same_floor(row: &Floor, field: &Floor) -> bool {
    row.position.x.to_bits() == field.position.x.to_bits()
        && row.position.y.to_bits() == field.position.y.to_bits()
        && row.position.z.to_bits() == field.position.z.to_bits()
        && row.half_x.to_bits() == field.half_x.to_bits()
        && row.half_z.to_bits() == field.half_z.to_bits()
        && row.color[0].to_bits() == field.color[0].to_bits()
        && row.color[1].to_bits() == field.color[1].to_bits()
        && row.color[2].to_bits() == field.color[2].to_bits()
}

impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.floor == other.floor
            && self.ceiling == other.ceiling
            && self.sky == other.sky
            && self.lighting == other.lighting
            && self.debug == other.debug
            && self.fps_counter == other.fps_counter
            && self.menu == other.menu
            && self.event_loop == other.event_loop
            && self.reflections == other.reflections
            && self.hdr == other.hdr
            && self.fog == other.fog
            && self.walls().eq(other.walls())
            && self.solids().eq(other.solids())
            && self.lights().eq(other.lights())
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for Scene {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Scene")
            .field("floor", &self.floor)
            .field("walls", &self.walls_cloned())
            .field("solids", &self.solids_cloned())
            .field("lights", &self.lights_cloned())
            .field("ceiling", &self.ceiling)
            .field("sky", &self.sky)
            .field("lighting", &self.lighting)
            .field("debug", &self.debug)
            .field("fps_counter", &self.fps_counter)
            .field("menu", &self.menu)
            .field("event_loop", &self.event_loop)
            .field("reflections", &self.reflections)
            .field("hdr", &self.hdr)
            .field("fog", &self.fog)
            .finish()
    }
}
