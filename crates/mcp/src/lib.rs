//! Loopback MCP session for the live scene.
//!
//! The camera process binds `127.0.0.1` and writes a discovery record for its
//! process id. A client that did not start the process reads that record,
//! skips a record whose process is gone, and speaks MCP on the URL.

pub mod command;
mod control;
mod discover;
mod http;
pub mod json;
mod lighting;
mod live;
mod rpc;

pub use control::{
    finish_shot, post_settings, publish_settings, settings_text, shot_bytes, shot_path,
    take_settings, take_shot_request, wait_shot, LightingSettings,
};
pub use discover::{discovery_dir, endpoint_at, live_endpoints, Endpoint};
pub use http::Server;
pub use json::{encode, parse, Value};
pub use lighting::publish as publish_lighting;
pub use live::{Host, SCENE_URI};
pub use rpc::command_timeout as rpc_timeout;

/// The loopback port the picture serves. `GENOS_MCP_PORT` overrides it.
pub const PORT: u16 = 8765;

/// Port for [`Server::start_on`]. The environment wins when it is a number.
pub fn listen_port() -> u16 {
    std::env::var("GENOS_MCP_PORT")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(PORT)
}
