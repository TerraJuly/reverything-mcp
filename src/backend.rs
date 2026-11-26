use std::{path::PathBuf, time::Duration};

use serde::Deserialize;
use tokio::{
    process::Command,
    sync::{Mutex, Semaphore},
    time::timeout,
};

use crate::{
    model::{FileEntry, FileInfoResult, Pagination, SearchResult, SortField, StatusResult},
    process::run_limited,
    query::exact_path_query,
};

pub const ES_VERSION: &str = "1.1.1.40";
const EVERYTHING_UNAVAILABLE: &str = "未能连接到本机 Everything 搜索客户端。若尚未安装，请从 https://www.voidtools.com/downloads/ 下载并安装 Windows x64 版 Everything；安装后请启动 Everything 并等待索引加载。若已安装，请确认客户端正在运行，以及 --instance 配置正确。仅运行 Everything Service 不够。启动后可直接重试，无需重启 MCP。";

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error(
        "ES operation timed out. Check that Everything is running and its index is ready; narrow slow content: queries or increase --timeout-ms."
    )]
    Timeout,
    #[error("Could not run ES: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

pub struct EsBackend {
    path: PathBuf,
    instance: Option<String>,
    deadline: Duration,
    permits: Semaphore,
    verified_version: Mutex<Option<String>>,
}

impl EsBackend {
    pub fn new(path: PathBuf, instance: Option<String>, deadline: Duration) -> Self {
        Self {
            path,
            instance,
            deadline,
            permits: Semaphore::new(2),
            verified_version: Mutex::new(None),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        // Explicitly override relevant es.ini defaults; never write/clear settings.
        command.args([
            "-no-argv",
            "-no-pause",
            "-no-highlight",
            "-no-header",
            "-no-footer",
        ]);
        command.args(["-instance", self.instance.as_deref().unwrap_or("")]);
        command
    }

    async fn execute(&self, command: &mut Command) -> Result<Vec<u8>, BackendError> {
        let output = run_limited(command).await?;
        if !output.status.success() {
            let message = match output.status.code() {
                Some(8) => EVERYTHING_UNAVAILABLE,
                Some(7) => {
                    "Everything IPC query failed. Check index readiness, Everything version, and the configured instance."
                }
                Some(4 | 6) => {
                    "ES rejected an argument. The bundled executable or its configuration may be incompatible."
                }
                _ => "ES failed to execute the query.",
            };
            let diagnostic = if output.stderr.is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            let detail: String = String::from_utf8_lossy(diagnostic)
                .trim()
                .chars()
                .take(1000)
                .collect();
            return Err(BackendError::Invalid(format!(
                "{message} Exit code: {:?}. {detail}",
                output.status.code()
            )));
        }
        Ok(output.stdout)
    }

    async fn ensure_es_version(&self) -> Result<String, BackendError> {
        let mut cached = self.verified_version.lock().await;
        if let Some(version) = &*cached {
            return Ok(version.clone());
        }
        let output = self.execute(self.command().arg("-version")).await?;
        let version = parse_version(&output)?;
        if version != ES_VERSION {
            return Err(BackendError::Invalid(format!(
                "Expected bundled ES {ES_VERSION}, found {version}. Use the executable supplied with this MCP release."
            )));
        }
        *cached = Some(version.clone());
        Ok(version)
    }

    async fn everything_version(&self) -> Result<String, BackendError> {
        let output = self
            .execute(self.command().arg("-get-everything-version"))
            .await?;
        let version = parse_version(&output)?;
        if !version.starts_with("1.4.") && !version.starts_with("1.5.") {
            return Err(BackendError::Invalid(format!(
                "Everything {version} is outside the supported 1.4/1.5 baseline; install a supported release."
            )));
        }
        Ok(version)
    }

    pub async fn status(&self) -> Result<StatusResult, BackendError> {
        timeout(self.deadline, async {
            let _permit = self
                .permits
                .acquire()
                .await
                .expect("semaphore is never closed");
            let es_version = self.ensure_es_version().await?;
            let everything_version = self.everything_version().await?;
            // -timeout makes ES wait for the database to load before querying.
            let page = Pagination {
                limit: 1,
                offset: 0,
                sort: SortField::Name,
                descending: false,
            };
            // An always-failing PCRE assertion cannot return any user filename.
            self.search_inner(r#"file: regex:"(?!)""#.into(), &page)
                .await?;
            Ok(StatusResult {
                es_version,
                everything_version,
                instance: self.instance.clone(),
                index_ready: true,
                backend: "es-x64",
            })
        })
        .await
        .map_err(|_| BackendError::Timeout)?
    }

    pub async fn search(
        &self,
        query: String,
        page: Pagination,
    ) -> Result<SearchResult, BackendError> {
        page.validate().map_err(BackendError::Invalid)?;
        timeout(self.deadline, async {
            let _permit = self
                .permits
                .acquire()
                .await
                .expect("semaphore is never closed");
            self.ensure_es_version().await?;
            self.everything_version().await?;
            self.search_inner(query, &page).await
        })
        .await
        .map_err(|_| BackendError::Timeout)?
    }

    pub async fn file_info(&self, path: &str) -> Result<FileInfoResult, BackendError> {
        let (path, query) = exact_path_query(path).map_err(BackendError::Invalid)?;
        let result = self
            .search(
                query,
                Pagination {
                    limit: 1,
                    offset: 0,
                    sort: SortField::Name,
                    descending: false,
                },
            )
            .await?;
        let entry = result.results.into_iter().next();
        Ok(FileInfoResult {
            path,
            found: entry.is_some(),
            entry,
        })
    }

    async fn search_inner(
        &self,
        query: String,
        page: &Pagination,
    ) -> Result<SearchResult, BackendError> {
        let mut command = self.command();
        command.args([
            "-json",
            "-columns",
            "full-path;size;date-modified;attributes",
            "-no-case",
            "-no-whole-word",
            "-no-match-path",
            "-no-diacritics",
            "-no-digit-grouping",
            "-no-size-leading-zero",
            "-size-format",
            "1",
            "-date-format",
            "3",
            "-no-utf8-bom",
            "-folder-append-path-separator",
            "-no-empty-search-help",
            "-no-hide-empty-search-results",
            // ES -count and viewport count are distinct. Reset persisted count,
            // then bound the IPC result window and fetch one extra for has_more.
            "-count",
            "18446744073709551615",
        ]);
        command.args(["-offset", &page.offset.to_string()]);
        command.args(["-max-results", &(page.limit + 1).to_string()]);
        let sort = format!(
            "{}-{}",
            page.sort.es_name(),
            if page.descending {
                "descending"
            } else {
                "ascending"
            }
        );
        command.args(["-sort", &sort]);
        command.args(["-timeout", &self.deadline.as_millis().to_string()]);
        append_literal_query(&mut command, &query)?;
        let bytes = self.execute(&mut command).await?;
        let mut results = parse_results(&bytes)?;
        if results.len() > page.limit as usize + 1 {
            return Err(BackendError::Invalid(
                "ES returned more rows than the requested result window".into(),
            ));
        }
        let has_more = results.len() > page.limit as usize;
        results.truncate(page.limit as usize);
        Ok(SearchResult {
            query,
            offset: page.offset,
            limit: page.limit,
            returned: results.len(),
            has_more,
            next_offset: has_more.then_some(page.offset + page.limit),
            results,
        })
    }
}

fn append_literal_query(command: &mut Command, query: &str) -> Result<(), BackendError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Verified in ES 1.1.1.40: -s* copies the remaining raw command line and
        // stops switch parsing. A normal .arg(query) would introduce Windows
        // quoting/escaping and change native Everything quotes/backslashes.
        // This is a direct CreateProcess call, NOT a shell invocation.
        command.arg("-s*");
        command.as_std_mut().raw_arg(query);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (command, query);
        Err(BackendError::Invalid(
            "The bundled ES backend requires Windows".into(),
        ))
    }
}

#[derive(Deserialize)]
struct EsRow {
    filename: String,
    size: Option<u64>,
    date_modified: Option<String>,
    attributes: Option<u32>,
}

fn parse_results(bytes: &[u8]) -> Result<Vec<FileEntry>, BackendError> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    // Verified ES 1.1.1.40 emits no JSON at all on a successful zero-row query.
    // execute() has already checked exit status before reaching this parser.
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let rows: Vec<EsRow> = serde_json::from_slice(bytes)
        .map_err(|error| BackendError::Invalid(format!("Invalid ES JSON output: {error}")))?;
    rows.into_iter()
        .map(|row| {
            if row.filename.is_empty() || row.filename.chars().any(char::is_control) {
                return Err(BackendError::Invalid(
                    "ES returned an invalid full path".into(),
                ));
            }
            let is_directory = row
                .attributes
                .is_some_and(|attributes| attributes & 0x10 != 0)
                || row.filename.ends_with(['\\', '/']);
            Ok(FileEntry {
                full_path: row.filename,
                is_directory,
                size: row.size,
                date_modified: row.date_modified,
                attributes: row.attributes,
            })
        })
        .collect()
}

fn parse_version(bytes: &[u8]) -> Result<String, BackendError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| BackendError::Invalid("ES version output is not UTF-8".into()))?;
    let version = text.trim_start_matches('\u{feff}').trim();
    let components: Vec<_> = version.split('.').collect();
    if components.len() != 4
        || components
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(BackendError::Invalid(
            "Unexpected version response from ES".into(),
        ));
    }
    Ok(version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_everything_hint_explains_install_start_and_retry() {
        assert!(EVERYTHING_UNAVAILABLE.contains("https://www.voidtools.com/downloads/"));
        assert!(EVERYTHING_UNAVAILABLE.contains("安装"));
        assert!(EVERYTHING_UNAVAILABLE.contains("启动"));
        assert!(EVERYTHING_UNAVAILABLE.contains("无需重启 MCP"));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version(b"1.1.1.40\r\n").unwrap(), ES_VERSION);
        for bad in ["", "not-a-version", "1.4", "1.4.a.3", "1.4.1.1032\nhelp"] {
            assert!(parse_version(bad.as_bytes()).is_err());
        }
    }

    #[test]
    fn parses_verified_es_json_and_optional_properties() {
        let rows = parse_results(br#"[{"filename":"C:\\test.rs","size":12,"date_modified":"2026-01-01T00:00:00Z","attributes":32},{"filename":"C:\\folder\\","size":null,"date_modified":null,"attributes":null}]"#).unwrap();
        assert_eq!(rows[0].full_path, r"C:\test.rs");
        assert!(!rows[0].is_directory);
        assert_eq!(rows[0].size, Some(12));
        assert!(rows[1].is_directory);
        assert!(parse_results(b"[]").unwrap().is_empty());
        assert!(parse_results(b"").unwrap().is_empty());
        assert!(parse_results(b" \r\n").unwrap().is_empty());
        for bad in [
            b"{}".as_slice(),
            b"[{\"size\":12}]",
            b"[{\"filename\":\"\"}]",
        ] {
            assert!(parse_results(bad).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn query_is_a_literal_raw_tail_not_an_es_switch() {
        let mut command = Command::new("es.exe");
        append_literal_query(&mut command, r#"file: <-exit | "C:\Program Files\">"#).unwrap();
        // get_args exposes both ordinary and raw arguments; ES parses only the
        // first one and treats the second as the verbatim search tail.
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args, ["-s*", r#"file: <-exit | "C:\Program Files\">"#]);
    }
}
