use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 200;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchFilters {
    /// Optional native Everything expression. Use ext:, size:, dm:, or content:.
    /// Grouping/quotes must be balanced when combined with structured filters.
    #[serde(default)]
    pub query: String,
    /// Optional PCRE filename regex, not a whole-query regex mode.
    pub name_regex: Option<String>,
    /// Absolute Windows directory (drive path or UNC share).
    pub path: Option<String>,
    /// Include descendants of path. Defaults to true. Requires path if false.
    #[serde(default = "default_recursive")]
    pub recursive: bool,
}

fn default_recursive() -> bool {
    true
}
fn default_limit() -> u32 {
    DEFAULT_LIMIT
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    #[serde(flatten)]
    pub filters: SearchFilters,
    /// File extensions without dots/wildcards, e.g. ["rs", "toml"]. OR within the list.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// A built-in Everything file category. AND with other filters.
    pub category: Option<FileCategory>,
    #[serde(flatten)]
    pub page: Pagination,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FolderSearchParams {
    #[serde(flatten)]
    pub filters: SearchFilters,
    #[serde(flatten)]
    pub page: Pagination,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pagination {
    /// Maximum returned results, 1..=200. Defaults to 20.
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 200))]
    pub limit: u32,
    /// Zero-based result offset. Use next_offset from the previous response.
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub sort: SortField,
    #[serde(default)]
    pub descending: bool,
}

impl Pagination {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(format!("limit must be between 1 and {MAX_LIMIT}"));
        }
        if self.offset.checked_add(self.limit + 1).is_none() {
            return Err("offset is too large for pagination".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileCategory {
    Document,
    Image,
    Audio,
    Video,
    Archive,
    Executable,
}

impl FileCategory {
    pub fn query(self) -> &'static str {
        match self {
            // Deterministic extension sets: user-editable Everything macros can
            // be absent or renamed, even in an otherwise compatible release.
            Self::Document => "ext:pdf;doc;docx;txt;rtf;odt;md;rst;xls;xlsx;ods;ppt;pptx;odp;csv",
            Self::Image => "ext:jpg;jpeg;png;gif;bmp;tif;tiff;webp;svg;ico;heic;avif",
            Self::Audio => "ext:mp3;wav;flac;aac;m4a;ogg;opus;wma;aiff",
            Self::Video => "ext:mp4;mkv;avi;mov;webm;wmv;flv;m4v;mpg;mpeg;ts",
            Self::Archive => "ext:zip;7z;rar;tar;gz;bz2;xz;cab;iso",
            Self::Executable => "ext:exe;msi;com;bat;cmd;scr",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Name,
    Path,
    Size,
    Extension,
    DateModified,
}

impl SortField {
    pub fn es_name(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Path => "path",
            Self::Size => "size",
            Self::Extension => "extension",
            Self::DateModified => "date-modified",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInfoParams {
    /// Absolute Windows file or directory path. No wildcards or query expressions.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FileEntry {
    pub full_path: String,
    pub is_directory: bool,
    /// Size in bytes, or null if unavailable in the index.
    pub size: Option<u64>,
    /// UTC ISO-8601 value, or null if unavailable in the index.
    pub date_modified: Option<String>,
    /// Windows file-attribute bits, or null if unavailable.
    pub attributes: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileInfoResult {
    pub path: String,
    pub found: bool,
    /// null means not found in the index, not proof that the path does not exist.
    pub entry: Option<FileEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResult {
    /// Effective native Everything expression after structured filters are applied.
    pub query: String,
    pub offset: u32,
    pub limit: u32,
    pub returned: usize,
    pub has_more: bool,
    pub next_offset: Option<u32>,
    pub results: Vec<FileEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResult {
    pub es_version: String,
    pub everything_version: String,
    pub instance: Option<String>,
    pub index_ready: bool,
    pub backend: &'static str,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SyntaxResult {
    pub guide: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_and_native_syntax() {
        let value: SearchParams = serde_json::from_value(json!({"query": "ext:rs"})).unwrap();
        assert_eq!(value.page.limit, 20);
        assert_eq!(value.page.offset, 0);
        assert!(value.filters.recursive);
        assert_eq!(value.page.sort.es_name(), "name");
    }

    #[test]
    fn pagination_is_bounded() {
        for limit in [0, 201, u32::MAX] {
            let value: SearchParams =
                serde_json::from_value(json!({"query": "ext:rs", "limit": limit})).unwrap();
            assert!(value.page.validate().is_err());
        }
        let value: SearchParams =
            serde_json::from_value(json!({"query": "ext:rs", "offset": u32::MAX})).unwrap();
        assert!(value.page.validate().is_err());
    }

    #[test]
    fn rejects_unknown_options_and_file_filters_on_folders() {
        assert!(
            serde_json::from_value::<SearchParams>(json!({"query":"ext:rs", "command":"-exit"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<SearchParams>(json!({"query":"ext:rs", "sort":"-exit"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<FolderSearchParams>(json!({"extensions":["rs"]})).is_err()
        );
    }
}
