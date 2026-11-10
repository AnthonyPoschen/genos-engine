//! The hook a component implements so a change can travel as bytes.

use std::fmt;

/// Failure while reading component bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer ended before the value did.
    Truncated,
    /// Bytes remained after the value.
    Trailing,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("component bytes ended early"),
            Self::Trailing => formatter.write_str("component bytes had a tail"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// One replicable piece of an object.
///
/// `encode` / `decode` are the serialization hook. A change tick records that
/// this value was written. The delta is `component_name` plus those bytes, so
/// an outfit edit does not resend the rest of the object.
///
/// `component_name` is the wire name. It stays stable across builds. Two
/// components in one store must not share a name.
pub trait Component: Clone + Send + Sync + 'static {
    /// Stable name used in a delta and in [`crate::World::apply_delta`].
    #[must_use]
    fn component_name() -> &'static str;

    /// Append the canonical bytes.
    fn encode(&self, out: &mut Vec<u8>);

    /// Inverse of [`Self::encode`].
    ///
    /// # Errors
    ///
    /// [`DecodeError::Truncated`] or [`DecodeError::Trailing`] when `bytes` is
    /// not exactly one value.
    fn decode(bytes: &[u8]) -> Result<Self, DecodeError>;

    /// `encode` into a new buffer.
    #[must_use]
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode(&mut out);
        out
    }
}
