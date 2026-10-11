//! `armarium-mcp`: the Armarium MCP server (stdio) ([ADR-0013], 08. MCP server).
//!
//! stdout is only for the MCP protocol. Logs go to stderr.

mod server;
mod text;

use anyhow::Context;
use armarium_core::{Engine, EngineConfig, NoEvents};
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "armarium-mcp", version, about = "Armarium MCP server (stdio)")]
struct Cli {
    /// Path of workspace.json.
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// Disable the tools that write (execute, save_table, discard_changes,
    /// register_source, unregister_source, export_result).
    #[arg(long)]
    read_only: bool,
    /// A directory from which register_source can register files and to which
    /// export_result can write. You can give this option more than one time.
    #[arg(long = "allow-dir")]
    allow_dirs: Vec<PathBuf>,
    /// Query timeout in seconds.
    #[arg(long, default_value_t = 60)]
    query_timeout: u64,
    /// Maximum number of rows in one response.
    #[arg(long, default_value_t = 500)]
    max_rows: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ARMARIUM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let cli = Cli::parse();

    let mut cfg = EngineConfig::default();
    if let Some(ws) = &cli.workspace {
        cfg.workspace_path = ws.clone();
    }
    let mut allow_dirs = Vec::new();
    for d in &cli.allow_dirs {
        let abs =
            std::fs::canonicalize(d).with_context(|| format!("--allow-dir {}", d.display()))?;
        allow_dirs.push(abs);
    }
    let engine = Engine::new(cfg, Arc::new(NoEvents)).context("cannot start the engine")?;
    let options = server::Options {
        read_only: cli.read_only,
        allow_dirs,
        timeout: Duration::from_secs(cli.query_timeout),
        max_rows: cli.max_rows.max(1),
    };
    let service = server::Armarium::new(Arc::new(engine), options)
        .serve(rmcp::transport::stdio())
        .await
        .context("cannot start the MCP service")?;
    service.waiting().await?;
    Ok(())
}
