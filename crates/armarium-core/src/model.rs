//! Shared types of the core library.
//!
//! The types in this module cross the IPC boundary. `ts-rs` generates the
//! TypeScript definitions from them (`cargo test` writes them to `ui/src/bindings`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The ID of a source. A UUID v7 in text form. It is stored in `workspace.json`.
pub type SourceId = String;

/// The ID of a table. It is valid only in one process (one session database).
pub type TableId = u32;

/// The ID of a grid view (sort and filter) or a query result.
pub type ViewId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum SourceKind {
    Csv,
    Excel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum LogicalType {
    Integer,
    Real,
    Text,
    Date,
    #[serde(rename = "datetime")]
    DateTime,
    Boolean,
}

impl LogicalType {
    /// The column type of the STRICT table.
    pub fn sqlite_type(self) -> &'static str {
        match self {
            LogicalType::Integer | LogicalType::Boolean => "INTEGER",
            LogicalType::Real => "REAL",
            LogicalType::Text | LogicalType::Date | LogicalType::DateTime => "TEXT",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LogicalType::Integer => "integer",
            LogicalType::Real => "real",
            LogicalType::Text => "text",
            LogicalType::Date => "date",
            LogicalType::DateTime => "datetime",
            LogicalType::Boolean => "boolean",
        }
    }

    pub fn parse(s: &str) -> Option<LogicalType> {
        Some(match s.to_ascii_lowercase().as_str() {
            "integer" | "int" => LogicalType::Integer,
            "real" | "float" | "double" => LogicalType::Real,
            "text" | "string" => LogicalType::Text,
            "date" => LogicalType::Date,
            "datetime" => LogicalType::DateTime,
            "boolean" | "bool" => LogicalType::Boolean,
            _ => return None,
        })
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, LogicalType::Integer | LogicalType::Real)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TypeSource {
    Inferred,
    UserOverride,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnSchema {
    /// The header text in the file. Armarium writes it when it saves the file.
    pub display_name: String,
    /// The SQL identifier (unique in the table, not empty).
    pub sql_name: String,
    pub logical_type: LogicalType,
    pub type_source: TypeSource,
    /// All data fields of the column were quoted in the file. Armarium quotes them again on save.
    #[serde(skip)]
    #[ts(skip)]
    pub always_quoted: bool,
}

/// A cell value. JSON form: `null`, a number, a string, or a boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum CellValue {
    Null,
    Integer(#[ts(type = "number")] i64),
    Real(f64),
    Text(String),
}

impl CellValue {
    pub fn from_ref(v: rusqlite::types::ValueRef<'_>) -> CellValue {
        use rusqlite::types::ValueRef;
        match v {
            ValueRef::Null => CellValue::Null,
            ValueRef::Integer(i) => CellValue::Integer(i),
            ValueRef::Real(f) => CellValue::Real(f),
            ValueRef::Text(t) => CellValue::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => CellValue::Text(format!("<blob {} bytes>", b.len())),
        }
    }

    pub fn to_sql_value(&self) -> rusqlite::types::Value {
        use rusqlite::types::Value;
        match self {
            CellValue::Null => Value::Null,
            CellValue::Integer(i) => Value::Integer(*i),
            CellValue::Real(f) => Value::Real(*f),
            CellValue::Text(s) => Value::Text(s.clone()),
        }
    }

    /// The text form of the value. It is the same text as `CAST(v AS TEXT)` in SQLite.
    pub fn to_text(&self) -> Option<String> {
        match self {
            CellValue::Null => None,
            CellValue::Integer(i) => Some(i.to_string()),
            CellValue::Real(f) => Some(crate::format::real_to_text(*f)),
            CellValue::Text(s) => Some(s.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum LineEnding {
    Crlf,
    Lf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Crlf => "\r\n",
            LineEnding::Lf => "\n",
            LineEnding::Cr => "\r",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EscapeStyle {
    /// `""` in a quoted field.
    Doubled,
    /// `\"` in a quoted field.
    Backslash,
}

/// The format of a delimited text file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Dialect {
    /// The WHATWG name of the encoding, for example `UTF-8` or `Shift_JIS`.
    pub encoding: String,
    pub bom: bool,
    pub delimiter: char,
    /// `None` means that fields are never quoted.
    pub quote: Option<char>,
    pub escape: EscapeStyle,
    pub line_ending: LineEnding,
    pub has_header: bool,
    pub final_line_ending: bool,
    /// Rows at the start of the file before the header. Armarium keeps their text.
    pub skipped_lines: Vec<String>,
    /// The header fields were all quoted in the file.
    pub header_quoted: bool,
}

impl Default for Dialect {
    fn default() -> Self {
        Dialect {
            encoding: "UTF-8".into(),
            bom: false,
            delimiter: ',',
            quote: Some('"'),
            escape: EscapeStyle::Doubled,
            line_ending: LineEnding::Crlf,
            has_header: true,
            final_line_ending: true,
            skipped_lines: Vec::new(),
            header_quoted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "state")]
#[ts(export)]
pub enum LoadState {
    Unloaded,
    Loaded { dirty: bool },
    Error { code: String, message: String },
}

/// Information about one table for the tree and for `list_sources`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableInfo {
    pub id: TableId,
    pub source_id: SourceId,
    pub sheet: Option<String>,
    pub sql_name: String,
    pub hidden: bool,
    pub load: LoadState,
    /// Another program changed the file after Armarium read it.
    pub externally_modified: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceInfo {
    pub id: SourceId,
    pub path: String,
    pub file_name: String,
    pub kind: SourceKind,
    pub exists: bool,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    #[ts(type = "number | null")]
    pub modified_ms: Option<i64>,
    pub tables: Vec<TableInfo>,
}

/// The schema and the size of a loaded table.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableMeta {
    pub id: TableId,
    pub sql_name: String,
    pub columns: Vec<ColumnSchema>,
    #[ts(type = "number")]
    pub row_count: u64,
    pub dirty: bool,
    pub read_only: bool,
    /// Why the original file cannot be overwritten (for example, an Excel workbook in M4).
    pub save_as_only: bool,
    pub dialect: Option<Dialect>,
    pub warnings: Vec<String>,
    #[ts(type = "number")]
    pub version: u64,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// A page of rows in column-oriented form ([ADR-0014]).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowPage {
    #[ts(type = "number")]
    pub offset: u64,
    #[ts(type = "number")]
    pub total: u64,
    /// The `rowid` of each row. Empty for query results.
    #[ts(type = "Array<number>")]
    pub rids: Vec<i64>,
    /// The original row number (1-based, the order in the file) of each row.
    #[ts(type = "Array<number>")]
    pub row_numbers: Vec<u64>,
    /// `data[c][r]`: the text of column `c` in row `r`. `null` is NULL.
    pub data: Vec<Vec<Option<String>>>,
    /// Cells that have a formula or an Excel error: (row index in this page, column index, note).
    pub notes: Vec<CellNote>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellNote {
    pub row: u32,
    pub col: u32,
    pub kind: CellNoteKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CellNoteKind {
    Formula,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SortKey {
    pub column: String,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FilterOp {
    Contains,
    Equals,
    NotEquals,
    Greater,
    Less,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilterCond {
    /// The SQL name of the column. `None` means "any column" (only for `contains`).
    pub column: Option<String>,
    pub op: FilterOp,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewSpec {
    pub sort: Vec<SortKey>,
    pub filters: Vec<FilterCond>,
}

/// One grid edit operation (06. Editing, section 2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "op")]
#[ts(export)]
pub enum EditOp {
    /// Set cell values. Each value is the text that the user entered (`null` or "" is NULL).
    SetCells {
        cells: Vec<CellEdit>,
    },
    /// Insert rows after a row (or before it, with `above`). `afterRid: null` inserts at the start.
    #[serde(rename_all = "camelCase")]
    InsertRows {
        #[ts(type = "number | null")]
        after_rid: Option<i64>,
        count: u32,
        /// Insert the rows before `afterRid` instead of after it.
        #[serde(default)]
        #[ts(optional)]
        above: Option<bool>,
    },
    DeleteRows {
        #[ts(type = "Array<number>")]
        rids: Vec<i64>,
    },
    /// Paste TSV text. The anchor is the top-left cell. New rows are added at the end if necessary.
    #[serde(rename_all = "camelCase")]
    Paste {
        #[ts(type = "number")]
        anchor_rid: i64,
        anchor_column: String,
        tsv: String,
        /// The rids of the rows below the anchor, in display order (the frontend sends them).
        #[ts(type = "Array<number>")]
        rids: Vec<i64>,
    },
    #[serde(rename_all = "camelCase")]
    InsertColumn {
        name: String,
        logical_type: LogicalType,
        /// The position in the display order. `null` adds the column at the end.
        position: Option<u32>,
    },
    DeleteColumn {
        column: String,
    },
    RenameColumn {
        column: String,
        to: String,
    },
    #[serde(rename_all = "camelCase")]
    ChangeColumnType {
        column: String,
        logical_type: LogicalType,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellEdit {
    #[ts(type = "number")]
    pub rid: i64,
    /// The SQL name of the column.
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditResult {
    pub table: TableMeta,
    /// The rids of new rows (for `InsertRows` and `Paste`).
    #[ts(type = "Array<number>")]
    pub inserted_rids: Vec<i64>,
}

/// The result of one SQL statement.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(export)]
pub enum StatementResult {
    #[serde(rename_all = "camelCase")]
    Rows {
        sql: String,
        result_id: ViewId,
        columns: Vec<ResultColumn>,
        #[ts(type = "number")]
        row_count: u64,
        elapsed_ms: u32,
    },
    #[serde(rename_all = "camelCase")]
    Changed {
        sql: String,
        table_id: TableId,
        sql_name: String,
        #[ts(type = "number")]
        changed_rows: u64,
        /// The rows of a `RETURNING` clause, if any.
        result_id: Option<ViewId>,
        columns: Vec<ResultColumn>,
        #[ts(type = "number")]
        row_count: u64,
        elapsed_ms: u32,
    },
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResultColumn {
    pub name: String,
    pub logical_type: LogicalType,
}

/// A change notification from the core to the adapters.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum CoreEvent {
    #[serde(rename_all = "camelCase")]
    TableChanged {
        table_id: TableId,
        #[ts(type = "number")]
        version: u64,
        dirty: bool,
        #[ts(type = "number")]
        row_count: u64,
    },
    WorkspaceChanged,
    #[serde(rename_all = "camelCase")]
    Progress {
        task: String,
        #[ts(type = "number")]
        done: u64,
        #[ts(type = "number | null")]
        total: Option<u64>,
    },
}

/// Options that override the detection when Armarium reads a source.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReadOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delimiter: Option<char>,
    /// `Some('\0')` means "no quote character".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<char>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_header: Option<bool>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skip_rows: u32,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub type_inference: bool,
    /// Type overrides for each column (by display name).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub column_types: std::collections::BTreeMap<String, LogicalType>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn is_true(v: &bool) -> bool {
    *v
}
fn default_true() -> bool {
    true
}

/// The format of an exported file.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "format")]
#[ts(export)]
pub enum ExportFormat {
    #[serde(rename_all = "camelCase")]
    Csv {
        encoding: String,
        bom: bool,
        delimiter: char,
        line_ending: LineEnding,
        quote_all: bool,
        header: bool,
    },
    #[serde(rename_all = "camelCase")]
    Xlsx { sheet_name: Option<String> },
}

impl ExportFormat {
    /// The default format for a file name (from its extension).
    pub fn for_path(path: &std::path::Path) -> ExportFormat {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "xlsx" => ExportFormat::Xlsx { sheet_name: None },
            "tsv" | "tab" => ExportFormat::Csv {
                encoding: "UTF-8".into(),
                bom: false,
                delimiter: '\t',
                line_ending: LineEnding::Crlf,
                quote_all: false,
                header: true,
            },
            _ => ExportFormat::Csv {
                encoding: "UTF-8".into(),
                bom: true,
                delimiter: ',',
                line_ending: LineEnding::Crlf,
                quote_all: false,
                header: true,
            },
        }
    }
}

/// Differences between a working copy and the file (`diff_table`).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableDiff {
    #[ts(type = "number")]
    pub inserted_rows: u64,
    #[ts(type = "number")]
    pub deleted_rows: u64,
    #[ts(type = "number")]
    pub changed_rows: u64,
    pub columns_changed: bool,
    pub sample: Vec<CellChange>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellChange {
    #[ts(type = "number")]
    pub row_number: u64,
    pub column: String,
    pub before: Option<String>,
    pub after: Option<String>,
}
