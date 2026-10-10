//! Armarium core: the domain logic that the GUI and the MCP server share ([ADR-0004]).

pub mod db;
pub mod engine;
pub mod error;
pub mod format;
pub mod fsutil;
pub mod ingest;
pub mod model;
pub mod naming;
pub mod sql;
pub mod workspace;

pub use engine::{Engine, EngineConfig, EventSink, NoEvents};
pub use error::{CoreError, Result};
