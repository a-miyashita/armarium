//! Text forms of tool results (Markdown tables and CSV).

use armarium_core::model::CellValue;

fn cell_text(v: &CellValue) -> String {
    match v {
        CellValue::Null => String::new(),
        CellValue::Integer(i) => i.to_string(),
        CellValue::Real(f) => armarium_core::format::real_to_text(*f),
        CellValue::Text(s) => s.clone(),
    }
}

fn escape_md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// A Markdown table.
pub fn markdown_table(headers: &[String], rows: &[Vec<CellValue>]) -> String {
    let mut out = String::new();
    out.push('|');
    for h in headers {
        out.push(' ');
        out.push_str(&escape_md(h));
        out.push_str(" |");
    }
    out.push_str("\n|");
    for _ in headers {
        out.push_str("---|");
    }
    out.push('\n');
    for r in rows {
        out.push('|');
        for v in r {
            out.push(' ');
            out.push_str(&escape_md(&cell_text(v)));
            out.push_str(" |");
        }
        out.push('\n');
    }
    out
}

/// CSV text (comma, LF, quotes only where necessary).
pub fn csv(headers: &[String], rows: &[Vec<CellValue>]) -> String {
    fn field(out: &mut String, s: &str) {
        if s.contains([',', '"', '\n', '\r']) {
            out.push('"');
            out.push_str(&s.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(s);
        }
    }
    let mut out = String::new();
    for (i, h) in headers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        field(&mut out, h);
    }
    out.push('\n');
    for r in rows {
        for (i, v) in r.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            field(&mut out, &cell_text(v));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tables() {
        let h = vec!["a".to_string(), "b|c".to_string()];
        let rows = vec![vec![CellValue::Integer(1), CellValue::Text("x\ny".into())]];
        assert_eq!(
            markdown_table(&h, &rows),
            "| a | b\\|c |\n|---|---|\n| 1 | x y |\n"
        );
        assert_eq!(csv(&h, &rows), "a,b|c\n1,\"x\ny\"\n");
    }
}
