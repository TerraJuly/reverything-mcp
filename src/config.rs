use std::{path::PathBuf, time::Duration};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Search local Windows files via Everything over MCP stdio"
)]
pub struct Config {
    /// Path to ES 1.1.1.40 x64. Defaults to the bundled executable, never PATH.
    #[arg(long, env = "REVERYTHING_ES_PATH")]
    pub es_path: Option<PathBuf>,

    /// Everything named instance (for example, 1.5a for older alpha installs).
    #[arg(long, env = "REVERYTHING_INSTANCE", value_parser = parse_instance)]
    pub instance: Option<String>,

    /// Whole-operation timeout, including concurrency queueing and ES startup.
    #[arg(long, env = "REVERYTHING_TIMEOUT_MS", default_value_t = 10_000,
        value_parser = clap::value_parser!(u64).range(100..=120_000))]
    pub timeout_ms: u64,
}

impl Config {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    pub fn resolve_es_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.es_path {
            return Ok(path.canonicalize()?);
        }
        let executable = std::env::current_exe()?;
        let beside_executable = executable
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot locate the MCP executable directory"))?
            .join("bin/es.exe");
        if beside_executable.is_file() {
            return Ok(beside_executable);
        }
        // Development only: release bundles must contain their own bin/es.exe.
        if cfg!(debug_assertions) {
            let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/es.exe");
            if development.is_file() {
                return Ok(development);
            }
        }
        anyhow::bail!(
            "Bundled bin/es.exe is missing. Extract the entire release ZIP, or set --es-path / REVERYTHING_ES_PATH."
        )
    }
}

fn parse_instance(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.encode_utf16().count() > 128
        || value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '"' | '\\' | '/'))
    {
        return Err(
            "instance must be 1..128 UTF-16 units without quotes, slashes, or control characters"
                .into(),
        );
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timeout_and_overrides() {
        let config = Config::try_parse_from(["reverything-mcp"]).unwrap();
        assert_eq!(config.timeout(), Duration::from_secs(10));
        let config = Config::try_parse_from([
            "reverything-mcp",
            "--timeout-ms",
            "500",
            "--instance",
            "1.5a",
        ])
        .unwrap();
        assert_eq!(config.instance.as_deref(), Some("1.5a"));
        assert_eq!(config.timeout(), Duration::from_millis(500));
    }

    #[test]
    fn instance_names_cannot_escape_es_argument_quoting() {
        assert!(parse_instance("My Instance 1.5a").is_ok());
        for name in ["", "name\" -exit", "name\\", "name\n"] {
            assert!(parse_instance(name).is_err());
        }
    }

    #[test]
    fn timeout_is_bounded() {
        for value in ["0", "99", "120001"] {
            assert!(Config::try_parse_from(["reverything-mcp", "--timeout-ms", value]).is_err());
        }
    }
}
