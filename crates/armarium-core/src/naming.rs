//! SQL names of tables and columns ([ADR-0010]).

use crate::error::{CoreError, Result};

/// Prefixes that SQL names must not have (internal use).
pub const RESERVED_PREFIXES: [&str; 2] = ["__arm_", "sqlite_"];

/// Column names that would hide the `rowid` of the working table.
const RESERVED_COLUMNS: [&str; 3] = ["rowid", "oid", "_rowid_"];

/// Makes an SQL name from a file name or a sheet name.
///
/// Replaces each character that is not a Unicode letter, a digit, or `_` with `_`,
/// replaces a sequence of `_` with one `_`, and removes `_` at the start and the end.
pub fn normalize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_us = false;
    for ch in raw.chars() {
        let ok = ch.is_alphanumeric() || ch == '_';
        let c = if ok { ch } else { '_' };
        if c == '_' {
            if !last_us {
                out.push('_');
            }
            last_us = true;
        } else {
            out.push(c);
            last_us = false;
        }
    }
    let trimmed = out.trim_matches('_');
    let mut name = if trimmed.is_empty() {
        "t".to_string()
    } else {
        trimmed.to_string()
    };
    if is_reserved(&name) {
        name = format!("t_{name}");
    }
    name
}

/// True if the name starts with an internal prefix (not case-sensitive).
pub fn is_reserved(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    RESERVED_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// SQL identifiers are not case-sensitive for ASCII letters.
pub fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Adds `_2`, `_3`, and so on to `base` until `taken` returns false.
pub fn make_unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}_{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Checks a table SQL name that the user entered.
pub fn validate_table_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidArgument("the SQL name is empty".into()));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(CoreError::InvalidArgument(
            "the SQL name contains a control character".into(),
        ));
    }
    if is_reserved(name) {
        return Err(CoreError::InvalidArgument(format!(
            "SQL names that start with {} are reserved",
            RESERVED_PREFIXES.join(" or ")
        )));
    }
    Ok(name.to_string())
}

/// Makes the SQL names of the columns from the header values.
///
/// The header value is used as it is. An empty value becomes `column{n}`.
/// A duplicate (not case-sensitive) gets `_2`, `_3`, and so on.
pub fn column_names(headers: &[String]) -> Vec<String> {
    let mut used: Vec<String> = Vec::with_capacity(headers.len());
    let mut out = Vec::with_capacity(headers.len());
    for (i, h) in headers.iter().enumerate() {
        let base = if h.trim().is_empty() {
            format!("column{}", i + 1)
        } else if RESERVED_COLUMNS.contains(&fold(h).as_str()) {
            format!("{h}_2")
        } else {
            h.clone()
        };
        let name = make_unique(&base, |c| {
            let f = fold(c);
            used.iter().any(|u| *u == f) || RESERVED_COLUMNS.contains(&f.as_str())
        });
        used.push(fold(&name));
        out.push(name);
    }
    out
}

/// Makes one new column SQL name that is unique among `existing`.
pub fn column_name(display: &str, position: usize, existing: &[&str]) -> String {
    let base = if display.trim().is_empty() {
        format!("column{}", position + 1)
    } else {
        display.to_string()
    };
    make_unique(&base, |c| {
        let f = fold(c);
        existing.iter().any(|e| fold(e) == f) || RESERVED_COLUMNS.contains(&f.as_str())
    })
}

/// Quotes an identifier for SQL.
pub fn quote_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push('"');
    for c in name.chars() {
        if c == '"' {
            s.push('"');
        }
        s.push(c);
    }
    s.push('"');
    s
}

/// True if the name can be written in SQL without quotes.
pub fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_') && !crate::sql::is_keyword(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_file_names() {
        assert_eq!(normalize("sales"), "sales");
        assert_eq!(normalize("売上 2026"), "売上_2026");
        assert_eq!(normalize("a -- b"), "a_b");
        assert_eq!(normalize("__x__"), "x");
        assert_eq!(normalize("!!!"), "t");
        assert_eq!(normalize("sqlite_data"), "t_sqlite_data");
        assert_eq!(normalize("budget_2026"), "budget_2026");
    }

    #[test]
    fn makes_unique_names() {
        let taken = ["sales", "sales_2"];
        let n = make_unique("sales", |c| taken.contains(&c));
        assert_eq!(n, "sales_3");
    }

    #[test]
    fn makes_column_names() {
        let h: Vec<String> = ["id", "", "Name", "name", "rowid"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            column_names(&h),
            vec!["id", "column2", "Name", "name_2", "rowid_2"]
        );
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert!(is_plain_identifier("sales"));
        assert!(!is_plain_identifier("2026"));
        assert!(!is_plain_identifier("select"));
        assert!(is_plain_identifier("売上"));
    }
}
