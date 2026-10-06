//! Loopback MCP session for the live scene.
//!
//! The camera process binds `127.0.0.1` and writes a discovery record for its
//! process id. A client that did not start the process reads that record,
//! skips a record whose process is gone, and speaks MCP on the URL.

mod discover;
mod http;
mod json;
mod live;
mod rpc;

pub use discover::{discovery_dir, endpoint_at, live_endpoints, Endpoint};
pub use http::Server;
pub use json::{parse, Value};
pub use live::{Host, SCENE_URI};
