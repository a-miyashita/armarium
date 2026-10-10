//! A delimited text parser and writer, and dialect detection (05. File formats, section 1).
//!
//! Armarium uses its own parser instead of the `csv` crate, because it must know
//! which fields were quoted in the file. With this information, a save can quote
//! the same fields again and keep the file byte-for-byte the same.

use crate::model::{Dialect, EscapeStyle, LineEnding};

/// One parsed record. The fields are stored in one buffer.
#[derive(Default, Debug)]
pub struct Record {
    buf: String,
    ends: Vec<usize>,
    quoted: Vec<bool>,
}

impl Record {
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn field(&self, i: usize) -> &str {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.buf[start..self.ends[i]]
    }

    pub fn quoted(&self, i: usize) -> bool {
        self.quoted[i]
    }

    pub fn fields(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(move |i| self.field(i))
    }

    /// True if the record is one empty field that was not quoted (an empty line).
    pub fn is_blank_line(&self) -> bool {
        self.len() == 1 && self.ends[0] == 0 && !self.quoted[0]
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.ends.clear();
        self.quoted.clear();
    }
}

/// How a record ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    Line(LineEnding),
    Eof,
}

pub struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
    delim: u8,
    quote: Option<u8>,
    escape: EscapeStyle,
}

impl<'a> Parser<'a> {
    pub fn new(text: &'a str, delim: char, quote: Option<char>, escape: EscapeStyle) -> Parser<'a> {
        Parser {
            s: text.as_bytes(),
            text,
            pos: 0,
            delim: ascii(delim),
            quote: quote.map(ascii),
            escape,
        }
    }

    /// The byte position of the parser in the text.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Skips `n` lines without parsing quotes. Returns their text.
    pub fn skip_lines(&mut self, n: u32) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..n {
            if self.pos >= self.s.len() {
                break;
            }
            let start = self.pos;
            let mut end = self.s.len();
            let mut next = self.s.len();
            let mut i = self.pos;
            while i < self.s.len() {
                match self.s[i] {
                    b'\r' => {
                        end = i;
                        next = if self.s.get(i + 1) == Some(&b'\n') { i + 2 } else { i + 1 };
                        break;
                    }
                    b'\n' => {
                        end = i;
                        next = i + 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            out.push(self.text[start..end].to_string());
            self.pos = next;
        }
        out
    }

    /// Reads the next record. Returns `None` at the end of the text.
    pub fn next_record(&mut self, rec: &mut Record) -> Option<Terminator> {
        rec.clear();
        let s = self.s;
        let n = s.len();
        if self.pos >= n {
            return None;
        }
        loop {
            let mut quoted = false;
            if let Some(q) = self.quote {
                if self.pos < n && s[self.pos] == q {
                    quoted = true;
                    self.pos += 1;
                    self.read_quoted(rec, q);
                }
            }
            // The unquoted part (or text after a closing quote).
            let start = self.pos;
            while self.pos < n {
                let b = s[self.pos];
                if b == self.delim || b == b'\r' || b == b'\n' {
                    break;
                }
                self.pos += 1;
            }
            rec.buf.push_str(&self.text[start..self.pos]);
            rec.ends.push(rec.buf.len());
            rec.quoted.push(quoted);

            if self.pos >= n {
                return Some(Terminator::Eof);
            }
            match s[self.pos] {
                b'\r' => {
                    if self.pos + 1 < n && s[self.pos + 1] == b'\n' {
                        self.pos += 2;
                        return Some(Terminator::Line(LineEnding::Crlf));
                    }
                    self.pos += 1;
                    return Some(Terminator::Line(LineEnding::Cr));
                }
                b'\n' => {
                    self.pos += 1;
                    return Some(Terminator::Line(LineEnding::Lf));
                }
                _ => {
                    // Delimiter.
                    self.pos += 1;
                    if self.pos >= n {
                        rec.ends.push(rec.buf.len());
                        rec.quoted.push(false);
                        return Some(Terminator::Eof);
                    }
                }
            }
        }
    }

    fn read_quoted(&mut self, rec: &mut Record, q: u8) {
        let s = self.s;
        let n = s.len();
        let backslash = self.escape == EscapeStyle::Backslash;
        let mut start = self.pos;
        while self.pos < n {
            let b = s[self.pos];
            if backslash && b == b'\\' && self.pos + 1 < n && s[self.pos + 1] == q {
                rec.buf.push_str(&self.text[start..self.pos]);
                rec.buf.push(q as char);
                self.pos += 2;
                start = self.pos;
                continue;
            }
            if b == q {
                rec.buf.push_str(&self.text[start..self.pos]);
                if !backslash && self.pos + 1 < n && s[self.pos + 1] == q {
                    rec.buf.push(q as char);
                    self.pos += 2;
                    start = self.pos;
                    continue;
                }
                self.pos += 1;
                return;
            }
            self.pos += 1;
        }
        // An unterminated quote: keep the rest of the text.
        rec.buf.push_str(&self.text[start..n]);
    }
}

fn ascii(c: char) -> u8 {
    if c.is_ascii() {
        c as u8
    } else {
        b','
    }
}

/// Appends one field to `out`, with quotes if necessary (or if `force`).
pub fn write_field(out: &mut String, value: &str, dialect: &Dialect, force: bool) {
    let d = dialect.delimiter;
    let q = dialect.quote.unwrap_or('"');
    let needs = value.contains(d) || value.contains(q) || value.contains(['\r', '\n']);
    if !(force || needs) {
        out.push_str(value);
        return;
    }
    out.push(q);
    for c in value.chars() {
        if c == q {
            match dialect.escape {
                EscapeStyle::Doubled => out.push(q),
                EscapeStyle::Backslash => out.push('\\'),
            }
        }
        out.push(c);
    }
    out.push(q);
}

/// The result of dialect detection.
#[derive(Debug, Clone)]
pub struct Detected {
    pub delimiter: char,
    pub quote: Option<char>,
    pub escape: EscapeStyle,
    pub line_ending: LineEnding,
    pub has_header: bool,
    /// The sample has only one column for each candidate delimiter.
    pub uncertain: bool,
}

const DELIMITERS: [char; 4] = [',', '\t', ';', '|'];

/// Detects the dialect from the start of the text.
pub fn detect(
    text: &str,
    fixed_delimiter: Option<char>,
    fixed_quote: Option<Option<char>>,
    fixed_header: Option<bool>,
    skip_rows: u32,
) -> Detected {
    let mut cut = text.len().min(1 << 20);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let sample = &text[..cut];
    let partial = cut < text.len();

    let line_ending = first_line_ending(sample).unwrap_or(LineEnding::Crlf);
    let quotes: Vec<Option<char>> = match fixed_quote {
        Some(q) => vec![q],
        None => {
            if sample.contains('"') || !sample.contains('\'') {
                vec![Some('"')]
            } else {
                vec![Some('"'), Some('\'')]
            }
        }
    };
    let delims: Vec<char> = match fixed_delimiter {
        Some(d) => vec![d],
        None => DELIMITERS.to_vec(),
    };

    let mut best: Option<(f64, usize, char, Option<char>)> = None;
    for &q in &quotes {
        for &d in &delims {
            let (consistency, mode) = score(sample, d, q, skip_rows, partial);
            let better = match best {
                None => true,
                Some((bc, bm, _, _)) => {
                    let valid = mode > 1;
                    let bvalid = bm > 1;
                    (valid && !bvalid)
                        || (valid == bvalid && (consistency > bc + 1e-9 || ((consistency - bc).abs() < 1e-9 && mode > bm)))
                }
            };
            if better {
                best = Some((consistency, mode, d, q));
            }
        }
    }
    let (_, mode, delimiter, quote) = best.unwrap_or((0.0, 1, ',', Some('"')));
    let uncertain = fixed_delimiter.is_none() && mode <= 1;
    let delimiter = if uncertain { ',' } else { delimiter };

    let escape = match quote {
        Some(q) => {
            let doubled: String = [q, q].iter().collect();
            let backslash: String = ['\\', q].iter().collect();
            if sample.contains(&backslash) && !sample.contains(&doubled) {
                EscapeStyle::Backslash
            } else {
                EscapeStyle::Doubled
            }
        }
        None => EscapeStyle::Doubled,
    };

    let has_header = fixed_header.unwrap_or_else(|| detect_header(sample, delimiter, quote, escape, skip_rows, partial));
    Detected {
        delimiter,
        quote,
        escape,
        line_ending,
        has_header,
        uncertain,
    }
}

fn first_line_ending(s: &str) -> Option<LineEnding> {
    let b = s.as_bytes();
    let i = b.iter().position(|&c| c == b'\r' || c == b'\n')?;
    Some(if b[i] == b'\n' {
        LineEnding::Lf
    } else if b.get(i + 1) == Some(&b'\n') {
        LineEnding::Crlf
    } else {
        LineEnding::Cr
    })
}

/// Parses up to 1,000 records of the sample.
fn sample_records(
    sample: &str,
    d: char,
    q: Option<char>,
    esc: EscapeStyle,
    skip_rows: u32,
    partial: bool,
) -> Vec<Vec<String>> {
    let mut p = Parser::new(sample, d, q, esc);
    p.skip_lines(skip_rows);
    let mut rec = Record::default();
    let mut out = Vec::new();
    while let Some(t) = p.next_record(&mut rec) {
        if partial && t == Terminator::Eof {
            break; // The last record can be cut.
        }
        if rec.is_blank_line() {
            continue;
        }
        out.push(rec.fields().map(str::to_string).collect());
        if out.len() >= 1000 {
            break;
        }
    }
    out
}

/// Returns (fraction of records with the most common field count, that count).
fn score(sample: &str, d: char, q: Option<char>, skip_rows: u32, partial: bool) -> (f64, usize) {
    let recs = sample_records(sample, d, q, EscapeStyle::Doubled, skip_rows, partial);
    if recs.is_empty() {
        return (0.0, 1);
    }
    let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for r in &recs {
        *counts.entry(r.len()).or_default() += 1;
    }
    let (mode, freq) = counts
        .into_iter()
        .max_by_key(|&(len, freq)| (freq, len))
        .unwrap_or((1, 0));
    (freq as f64 / recs.len() as f64, mode)
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Kind {
    Empty,
    Number,
    Date,
    Text,
}

fn kind_of(v: &str) -> Kind {
    let t = v.trim();
    if t.is_empty() {
        Kind::Empty
    } else if t.parse::<f64>().is_ok() && t.chars().any(|c| c.is_ascii_digit()) {
        Kind::Number
    } else if crate::format::parse_date(t).is_some() || crate::format::parse_datetime(t).is_some() {
        Kind::Date
    } else {
        Kind::Text
    }
}

/// Row 1 is a header if it has no empty or duplicate values, and its types are
/// different from the types in the other rows.
fn detect_header(sample: &str, d: char, q: Option<char>, esc: EscapeStyle, skip_rows: u32, partial: bool) -> bool {
    let recs = sample_records(sample, d, q, esc, skip_rows, partial);
    let Some(first) = recs.first() else { return true };
    let mut seen = std::collections::HashSet::new();
    for v in first {
        if v.trim().is_empty() || !seen.insert(v.to_lowercase()) {
            return false;
        }
    }
    if recs.len() < 2 {
        return true;
    }
    let mut evidence = 0i32;
    for (j, h) in first.iter().enumerate() {
        let hk = kind_of(h);
        let mut typed = 0usize;
        let mut total = 0usize;
        for r in recs.iter().skip(1).take(200) {
            if let Some(v) = r.get(j) {
                let k = kind_of(v);
                if k == Kind::Empty {
                    continue;
                }
                total += 1;
                if k == hk {
                    typed += 1;
                }
            }
        }
        if total == 0 {
            continue;
        }
        let same = typed * 10 >= total * 9;
        if hk == Kind::Text && !same {
            evidence += 1;
        } else if hk != Kind::Text && same {
            evidence -= 1;
        }
    }
    evidence >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(text: &str, d: char, q: Option<char>, esc: EscapeStyle) -> Vec<(Vec<String>, Vec<bool>)> {
        let mut p = Parser::new(text, d, q, esc);
        let mut rec = Record::default();
        let mut out = Vec::new();
        while p.next_record(&mut rec).is_some() {
            out.push((rec.fields().map(str::to_string).collect(), (0..rec.len()).map(|i| rec.quoted(i)).collect()));
        }
        out
    }

    #[test]
    fn parses_quotes_and_line_endings() {
        let r = parse_all("a,\"b,\"\"c\"\"\"\r\n\"x\ny\",\r\n", ',', Some('"'), EscapeStyle::Doubled);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, vec!["a", "b,\"c\""]);
        assert_eq!(r[0].1, vec![false, true]);
        assert_eq!(r[1].0, vec!["x\ny", ""]);
        let r = parse_all("a\tb\nc", '\t', None, EscapeStyle::Doubled);
        assert_eq!(r, vec![(vec!["a".into(), "b".into()], vec![false, false]), (vec!["c".into()], vec![false])]);
        let r = parse_all("\"a\\\"b\",c\n", ',', Some('"'), EscapeStyle::Backslash);
        assert_eq!(r[0].0, vec!["a\"b", "c"]);
        let r = parse_all("a,b,\n", ',', Some('"'), EscapeStyle::Doubled);
        assert_eq!(r[0].0, vec!["a", "b", ""]);
    }

    #[test]
    fn detects_dialects() {
        let d = detect("id;name;amount\r\n1;a;10\r\n2;b;20\r\n", None, None, None, 0);
        assert_eq!(d.delimiter, ';');
        assert_eq!(d.line_ending, LineEnding::Crlf);
        assert!(d.has_header);

        let d = detect("1\t2\t3\n4\t5\t6\n", None, None, None, 0);
        assert_eq!(d.delimiter, '\t');
        assert!(!d.has_header);
        assert_eq!(d.line_ending, LineEnding::Lf);

        let d = detect("name,city\nTaro,\"Tokyo, Japan\"\nHanako,Osaka\n", None, None, None, 0);
        assert_eq!(d.delimiter, ',');
        assert!(d.has_header);

        let d = detect("a|b\n1|x,y\n2|z\n", None, None, None, 0);
        assert_eq!(d.delimiter, '|');

        let d = detect("just one column\nvalue\n", None, None, None, 0);
        assert!(d.uncertain);
    }

    #[test]
    fn writes_fields() {
        let d = Dialect::default();
        let mut s = String::new();
        write_field(&mut s, "a,b", &d, false);
        s.push('|');
        write_field(&mut s, "x\"y", &d, false);
        s.push('|');
        write_field(&mut s, "plain", &d, true);
        assert_eq!(s, "\"a,b\"|\"x\"\"y\"|\"plain\"");
    }
}
