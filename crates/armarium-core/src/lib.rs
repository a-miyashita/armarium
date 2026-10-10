//! Armarium core: the domain logic that the GUI and the MCP server share ([ADR-0004]).

pub mod db;
pub mod error;
pub mod format;
pub mod fsutil;
pub mod model;
pub mod naming;
pub mod sql;
pub mod workspace;

pub use error::{CoreError, Result};
