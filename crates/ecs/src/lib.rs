//! Archetype store for placed objects.
//!
//! A [scene handle](crate::SceneHandle) is an index plus a generation. Objects
//! that share a component set live in one table, and each column is contiguous.
//! A write stores a tick on that cell. [`World::changes_since`] returns the
//! bytes of the cells written after a tick, so one component can be replicated
//! without the rest of the object.
//!
//! The socket, the snapshot codec, and `NetId` belong to a later replication
//! ADR. This crate does not open a socket.
//!
//! - Replicated state: each changed component's name, tick, and bytes, addressed
//!   by a stable scene handle. The scene crate maps that handle to a `NetId`.
//! - Authority: the process that owns the [`World`]. A later server is the only
//!   writer of a replicated object. This crate does not check authority.
//! - Prediction / rollback: none. A caller can [`Clone`] the store and write a
//!   copy. The store does not rewind itself.
//! - Determinism: the same spawns and writes produce the same handles, the same
//!   ticks, and the same [`World::changes_since`] bytes. Query order follows
//!   table creation, then row. Deltas are sorted by handle index, then name.
//! - Headless server: no window, no Vulkan, no threads. The store is data.

mod bytes;
mod component;
mod handle;
mod world;

pub use bytes::{push_bytes, push_f32, push_u32, read_f32, read_u32, take};
pub use component::{Component, DecodeError};
pub use handle::SceneHandle;
pub use world::{ComponentDelta, Mut, Stats, StoreError, World};
