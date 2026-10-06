//! Game audio. The mix is stereo PCM. The operating system only receives that mix.
//!
//! Linux submits through ALSA. Windows submits through WASAPI. macOS submits
//! through CoreAudio. No third-party crate owns the device.
//!
//! Doppler, balance, distance, and wall transmission are numbers the mix applies.
//! Many positional paths share one GPU pass on the renderer device. That pass
//! reads the resident scene and does not present a swapchain image.

mod device;
mod mix;
mod space;

pub use device::submit;
pub use genos_math::Vec3;
pub use mix::{mix, Barrier, Listener, Source};
pub use space::SPEED_OF_SOUND;
