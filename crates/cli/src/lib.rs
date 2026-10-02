//! The `search` binary: argument parsing, the serve loop, and the two MCP
//! transports. The engine itself lives in `search_core`; this crate only puts
//! it behind a command line and a listener.

pub mod args;
pub mod http;
pub mod mcp;
pub mod run;
pub mod stdio;

/// The product version, for `search version` and the MCP server info.
pub use search_core::VERSION;
