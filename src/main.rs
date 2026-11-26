mod backend;
mod config;
mod model;
mod process;
mod query;
mod server;

use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use crate::{backend::EsBackend, config::Config, server::EverythingServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    anyhow::ensure!(
        cfg!(all(target_os = "windows", target_arch = "x86_64")),
        "This release supports Windows x64 only; Everything must run on the same machine."
    );
    let timeout = config.timeout();
    let backend = EsBackend::new(config.resolve_es_path()?, config.instance, timeout);
    // Keep MCP available when Everything is stopped, so get_status can explain
    // the problem and a later search can recover without restarting the server.
    match backend.status().await {
        Ok(status) => tracing::info!(
            es_version = %status.es_version,
            everything_version = %status.everything_version,
            "Everything connection verified"
        ),
        Err(error) => {
            tracing::warn!(%error, "Everything 暂不可用；MCP 保持运行，请按提示安装或启动后重试")
        }
    }
    let service = EverythingServer::new(backend).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
