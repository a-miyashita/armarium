//! Conversion between text and typed values.

use crate::error::{CoreError, Result};
use crate::model::LogicalType;
use rusqlite::types::Value;

/// A text form of a REAL value for display outside SQLite.
///
/// Persist does not use this function. It uses `CAST(x AS TEXT)` in SQLite,
/// because type inference also uses it ([ADR-0007]).
pub fn real_to_text(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

/// Converts the text that the user entered to a value of the logical type.
///
/// An empty input is NULL. The conversion accepts some common forms that are not
/// the standard form of the type (for example, `1,234` and `2026/01/02`).
pub fn parse_input(text: Option<&str>, ty: LogicalType) -> Result<Value> {
    let Some(raw) = text else {
        return Ok(Value::Null);
    };
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    let mismatch = || {
        CoreError::TypeMismatch(format!(
            "'{}' is not a valid {} value",
            truncate(raw, 40),
            ty.as_str()
        ))
    };
    match ty {
        LogicalType::Text => Ok(Value::Text(raw.to_string())),
        LogicalType::Integer => {
            let s = strip_number(raw);
            s.parse::<i64>().map(Value::Integer).map_err(|_| mismatch())
        }
        LogicalType::Real => {
            let s = strip_number(raw);
            let only_numeric = s
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'));
            match s.parse::<f64>() {
                Ok(f) if f.is_finite() && only_numeric => Ok(Value::Real(f)),
                _ => Err(mismatch()),
            }
        }
        LogicalType::Date => parse_date(raw.trim())
            .map(|(y, m, d)| Value::Text(format!("{y:04}-{m:02}-{d:02}")))
            .ok_or_else(mismatch),
        LogicalType::DateTime => parse_datetime(raw.trim())
            .map(Value::Text)
            .ok_or_else(mismatch),
        LogicalType::Boolean => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "y" | "on" => Ok(Value::Integer(1)),
            "0" | "false" | "no" | "n" | "off" => Ok(Value::Integer(0)),
            _ => Err(mismatch()),
        },
    }
}

/// Converts a value that is already in SQLite to the logical type.
pub fn convert_value(v: &Value, ty: LogicalType) -> Result<Value> {
    match v {
        Value::Null => Ok(Value::Null),
        Value::Integer(i) => match ty {
            LogicalType::Integer => Ok(Value::Integer(*i)),
            LogicalType::Real => Ok(Value::Real(*i as f64)),
            LogicalType::Boolean if *i == 0 || *i == 1 => Ok(Value::Integer(*i)),
            _ => parse_input(Some(&i.to_string()), ty),
        },
        Value::Real(f) => match ty {
            LogicalType::Real => Ok(Value::Real(*f)),
            LogicalType::Integer if f.fract() == 0.0 && f.abs() < 9.2e18 => {
                Ok(Value::Integer(*f as i64))
            }
            LogicalType::Text => Ok(Value::Text(real_to_text(*f))),
            _ => parse_input(Some(&real_to_text(*f)), ty),
        },
        Value::Text(s) => parse_input(Some(s), ty),
        Value::Blob(_) => Err(CoreError::TypeMismatch("a BLOB value cannot be converted".into())),
    }
}

fn strip_number(raw: &str) -> String {
    let t = raw.trim();
    // Remove thousands separators only if they are in the correct places.
    if t.contains(',') {
        let (int_part, _) = t.split_once('.').unwrap_or((t, ""));
        let digits = int_part.trim_start_matches(['+', '-']);
        let groups: Vec<&str> = digits.split(',').collect();
        let valid = groups.len() > 1
            && !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3);
        if valid {
            return t.replace(',', "");
        }
    }
    t.to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Parses `YYYY-MM-DD`, `YYYY/MM/DD`, `YYYY-M-D`, or `YYYY/M/D`.
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let sep = if s.contains('/') { '/' } else { '-' };
    let mut it = s.split(sep);
    let y = it.next()?;
    let m = it.next()?;
    let d = it.next()?;
    if it.next().is_some() || y.len() != 4 || m.is_empty() || m.len() > 2 || d.is_empty() || d.len() > 2 {
        return None;
    }
    if !(y.bytes().chain(m.bytes()).chain(d.bytes()).all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (y, m, d): (i32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    Some((y, m, d))
}

/// Parses a date and time. The result is `YYYY-MM-DD HH:MM:SS[.fff]`.
pub fn parse_datetime(s: &str) -> Option<String> {
    let (date, time) = s.split_once([' ', 'T'])?;
    let (y, mo, d) = parse_date(date)?;
    let time = time.trim();
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (time, None),
    };
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let mi: u32 = parts[1].parse().ok()?;
    let se: u32 = if parts.len() == 3 { parts[2].parse().ok()? } else { 0 };
    if h > 23 || mi > 59 || se > 59 || parts.iter().any(|p| p.is_empty() || p.len() > 2) {
        return None;
    }
    let mut out = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{se:02}");
    if let Some(f) = frac {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push('.');
        out.push_str(f);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inputs() {
        assert_eq!(parse_input(Some(""), LogicalType::Integer).unwrap(), Value::Null);
        assert_eq!(parse_input(Some("1,234"), LogicalType::Integer).unwrap(), Value::Integer(1234));
        assert_eq!(parse_input(Some(" 12 "), LogicalType::Integer).unwrap(), Value::Integer(12));
        assert!(parse_input(Some("12,34"), LogicalType::Integer).is_err());
        assert!(parse_input(Some("abc"), LogicalType::Integer).is_err());
        assert_eq!(parse_input(Some("1.5"), LogicalType::Real).unwrap(), Value::Real(1.5));
        assert_eq!(parse_input(Some("1e3"), LogicalType::Real).unwrap(), Value::Real(1000.0));
        assert!(parse_input(Some("inf"), LogicalType::Real).is_err());
        assert!(parse_input(Some("NaN"), LogicalType::Real).is_err());
        assert_eq!(
            parse_input(Some("2026/1/2"), LogicalType::Date).unwrap(),
            Value::Text("2026-01-02".into())
        );
        assert!(parse_input(Some("2026-02-30"), LogicalType::Date).is_err());
        assert_eq!(
            parse_input(Some("2026-01-02T03:04"), LogicalType::DateTime).unwrap(),
            Value::Text("2026-01-02 03:04:00".into())
        );
        assert_eq!(parse_input(Some("TRUE"), LogicalType::Boolean).unwrap(), Value::Integer(1));
    }
}
