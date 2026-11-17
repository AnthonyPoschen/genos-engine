#![expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

//! Game audio. The mix is stereo PCM. The operating system only receives that mix.
//!
//! Linux submits through ALSA. Windows submits through WASAPI. macOS submits
//! through CoreAudio. No third-party crate owns the device.
//!
//! Doppler, balance, distance, and wall transmission are numbers the mix applies.
//! Many positional paths share one GPU pass on the renderer device. That pass
//! reads the resident scene and does not present a swapchain image.

mod device;
mod disk;
mod engine;
mod mix;
mod path;
mod poly;
mod ring;
mod space;

#[cfg(target_os = "linux")]
pub use device::AlsaDevice;
#[cfg(target_os = "macos")]
pub use device::CoreAudioDevice;
#[cfg(target_os = "windows")]
pub use device::WasapiDevice;
pub use engine::{start_disk, start_with, AudioStats, Stream};
pub use genos_math::Vec3;
pub use mix::{allocating_mix_entries, mix, mix_into, Barrier, Listener, Source};
pub use path::{calling_thread, DevicePath, FakeCapture, FakeDevice, NullDevice, OpenedDevice};
pub use ring::Command;
pub use space::SPEED_OF_SOUND;
