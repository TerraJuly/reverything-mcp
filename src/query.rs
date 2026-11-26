use crate::model::{FileCategory, SearchFilters};

#[derive(Debug, Clone, Copy)]
pub enum EntryKind {
    File,
    Folder,
}

impl EntryKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::File => "file:",
            Self::Folder => "folder:",
        }
    }
}

pub fn build_query(
    filters: &SearchFilters,
    extensions: &[String],
    category: Option<FileCategory>,
    kind: EntryKind,
) -> Result<String, String> {
    validate_text(&filters.query, "query", 4096)?;
    validate_native_expression(&filters.query)?;
    let mut terms = vec![kind.prefix().to_owned()];
    let mut has_filter = false;
    if !filters.query.trim().is_empty() {
        terms.push(format!("<{}>", filters.query.trim()));
        has_filter = true;
    }
    if let Some(pattern) = &filters.name_regex {
        validate_text(pattern, "name_regex", 4096)?;
        if pattern.trim().is_empty() {
            return Err("name_regex must not be empty".into());
        }
        // Everything quotes delimit a search token; PCRE's \x22 safely represents
        // a literal double quote without escaping into the query language.
        terms.push(format!(
            r#"nopath:regex:"{}""#,
            pattern.replace('"', r"\x22")
        ));
        has_filter = true;
    }
    if let Some(path) = &filters.path {
        let path = normalize_path(path)?;
        if filters.recursive {
            terms.push(format!(r#""{}\""#, path.trim_end_matches('\\')));
        } else {
            terms.push(format!(r#"parent:"{}""#, path));
        }
        has_filter = true;
    } else if !filters.recursive {
        return Err("recursive=false requires an absolute path".into());
    }
    if extensions.len() > 32 {
        return Err("extensions must contain at most 32 items".into());
    }
    for extension in extensions {
        if extension.is_empty()
            || extension.len() > 32
            || !extension
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err("extensions must be 1..32 ASCII letters/digits, underscores or hyphens, without dots or wildcards".into());
        }
    }
    if !extensions.is_empty() {
        terms.push(format!("ext:{}", extensions.join(";")));
        has_filter = true;
    }
    if let Some(category) = category {
        terms.push(category.query().to_owned());
        has_filter = true;
    }
    if !has_filter {
        return Err(
            "Provide query, name_regex, path, extensions, or category to narrow the search".into(),
        );
    }
    let query = terms
        .into_iter()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    validate_text(&query, "effective query", 16_384)?;
    Ok(query)
}

pub fn exact_path_query(path: &str) -> Result<(String, String), String> {
    let path = normalize_path(path)?;
    let pattern = escape_regex(&path);
    Ok((path, format!(r#"path:regex:"^{}$""#, pattern)))
}

pub fn normalize_path(path: &str) -> Result<String, String> {
    validate_text(path, "path", 4096)?;
    let path = path.replace('/', "\\");
    let bytes = path.as_bytes();
    let (prefix, rest, minimum_parts) = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
    {
        (format!("{}\\", &path[..2]), &path[3..], 0)
    } else if let Some(rest) = path.strip_prefix("\\\\") {
        ("\\\\".to_owned(), rest, 2)
    } else {
        return Err(
            "path must be an absolute drive path (C:\\...) or UNC share (\\\\server\\share\\...)"
                .into(),
        );
    };
    let parts: Vec<_> = rest.split('\\').filter(|part| !part.is_empty()).collect();
    if parts.len() < minimum_parts {
        return Err("UNC path must include server and share names".into());
    }
    for part in &parts {
        if *part == "."
            || *part == ".."
            || part
                .chars()
                .any(|ch| matches!(ch, ':' | '"' | '*' | '?' | '<' | '>' | '|'))
        {
            return Err("path must not contain query operators, wildcards, device prefixes, or dot segments".into());
        }
    }
    Ok(format!("{prefix}{}", parts.join("\\")))
}

fn validate_text(value: &str, field: &str, max_units: usize) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{field} must not contain control characters"));
    }
    if value.encode_utf16().count() > max_units {
        return Err(format!(
            "{field} must not exceed {max_units} UTF-16 code units"
        ));
    }
    Ok(())
}

fn validate_native_expression(query: &str) -> Result<(), String> {
    let mut quoted = false;
    let mut depth = 0_u32;
    let mut previous = '\0';
    for ch in query.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted {
            // Comparisons such as size:>1gb and dm:<today are not group brackets.
            if ch == '<' && previous != ':' {
                depth += 1;
            }
            if ch == '>' && previous != ':' {
                depth = depth.checked_sub(1).ok_or(
                    "query has an unmatched >; quote regex expressions containing angle brackets",
                )?;
            }
        }
        previous = ch;
    }
    if quoted || depth != 0 {
        return Err("query must have balanced double quotes and <...> groups; use name_regex for complex regex".into());
    }
    Ok(())
}

fn escape_regex(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '.' | '^' | '$' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn filters(value: serde_json::Value) -> SearchFilters {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn combines_structured_filters_without_global_regex_mode() {
        let value =
            filters(json!({"path":"E:\\Projects", "name_regex":"^test_.*", "query":"size:>1kb"}));
        let query = build_query(&value, &["rs".into()], None, EntryKind::File).unwrap();
        assert_eq!(
            query,
            r#"file: <size:>1kb> nopath:regex:"^test_.*" "E:\Projects\" ext:rs"#
        );
        let value = filters(json!({"path":"C:/Program Files/", "recursive":false}));
        assert_eq!(
            build_query(&value, &[], None, EntryKind::Folder).unwrap(),
            r#"folder: parent:"C:\Program Files""#
        );
    }

    #[test]
    fn constrains_raw_or_expression_and_quoted_regex() {
        let value = filters(json!({"query":"foo | bar", "name_regex":"a\" | folder:"}));
        let query = build_query(&value, &[], Some(FileCategory::Image), EntryKind::File).unwrap();
        assert_eq!(
            query,
            format!(
                r#"file: <foo | bar> nopath:regex:"a\x22 | folder:" {}"#,
                FileCategory::Image.query()
            )
        );
        for query in ["foo> | folder:", "\"unterminated", "<unclosed", "a\n-exit"] {
            assert!(
                build_query(&filters(json!({"query":query})), &[], None, EntryKind::File).is_err()
            );
        }
    }

    #[test]
    fn rejects_invalid_structured_fields_and_empty_search() {
        assert!(build_query(&filters(json!({})), &[], None, EntryKind::File).is_err());
        assert!(
            build_query(
                &filters(json!({"recursive":false, "query":"foo"})),
                &[],
                None,
                EntryKind::File
            )
            .is_err()
        );
        assert!(
            build_query(
                &filters(json!({"name_regex":""})),
                &[],
                None,
                EntryKind::File
            )
            .is_err()
        );
        for extension in [".rs", "rs;pdf", "*", "foo bar", "rs\" | folder:"] {
            assert!(
                build_query(
                    &filters(json!({})),
                    &[extension.into()],
                    None,
                    EntryKind::File
                )
                .is_err()
            );
        }
        assert!(
            build_query(
                &filters(json!({"query":"a".repeat(4097)})),
                &[],
                None,
                EntryKind::File
            )
            .is_err()
        );
    }

    #[test]
    fn validates_drive_unc_roots_and_exact_paths() {
        assert_eq!(normalize_path("C:\\").unwrap(), "C:\\");
        assert_eq!(
            normalize_path("\\\\server\\share\\").unwrap(),
            "\\\\server\\share"
        );
        for path in [
            "relative",
            "C:relative",
            "C:\\foo\\..\\bar",
            "C:\\foo*",
            "\\\\?\\C:\\foo",
            "\\\\server",
        ] {
            assert!(normalize_path(path).is_err(), "{path}");
        }
        assert_eq!(
            exact_path_query(r"C:\a [1].txt").unwrap().1,
            r#"path:regex:"^C:\\a \[1\]\.txt$""#
        );
    }
}
