//! A scene handle names one placed object across a despawn and a reload.

/// Index plus generation for one placed object.
///
/// The index is reused. The generation changes when the slot is freed, so a
/// handle saved before the despawn does not address the next object in that
/// slot. Generation `0` is never issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneHandle {
    index: u32,
    generation: u32,
}

impl SceneHandle {
    /// A handle with this index and generation.
    ///
    /// The store never returns generation `0`. A caller that builds a handle
    /// for a save file uses the generation the file recorded.
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Slot in the handle table.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Generation that must match the live slot.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}
