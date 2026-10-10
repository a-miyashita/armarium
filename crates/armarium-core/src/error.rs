//! Typed errors of the core library.
//!
//! Each error has a stable code (see `CoreError::code`). The adapters (Tauri
//! commands and MCP tools) send the code and the message to the client.

use serde::Serialize;
use ts_rs::TS;

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("the file is locked by another program: {0}")]
    FileLocked(String),
    #[error("cannot detect the encoding of the file: {0}")]
    EncodingUndetected(String),
    #[error("another program changed the file: {0}")]
    ExternalModification(String),
    #[error("the operation is not allowed in SQL: {0}")]
    SqlForbidden(String),
    #[error("{0}")]
    Sql(String),
    #[error("{0}")]
    TypeMismatch(String),
    #[error("cannot encode the value in {encoding}: {cells} cell(s), first at row {first_row}, column {first_column}")]
    Unencodable {
        encoding: String,
        cells: usize,
        first_row: u64,
        first_column: String,
    },
    #[error("the change is larger than the undo limit ({bytes} bytes); it was rolled back")]
    UndoLimitExceeded { bytes: u64 },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    InvalidArgument(String),
    #[error("{0}")]
    ReadOnly(String),
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),
    #[error("cannot read the file: {0}")]
    Parse(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

impl CoreError {
    /// The stable error code for the IPC boundary.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::FileNotFound(_) => "FILE_NOT_FOUND",
            CoreError::FileLocked(_) => "FILE_LOCKED",
            CoreError::EncodingUndetected(_) => "ENCODING_UNDETECTED",
            CoreError::ExternalModification(_) => "EXTERNAL_MODIFICATION",
            CoreError::SqlForbidden(_) => "SQL_FORBIDDEN",
            CoreError::Sql(_) => "SQL_ERROR",
            CoreError::TypeMismatch(_) => "TYPE_MISMATCH",
            CoreError::Unencodable { .. } => "UNENCODABLE",
            CoreError::UndoLimitExceeded { .. } => "UNDO_LIMIT_EXCEEDED",
            CoreError::NotFound(_) => "NOT_FOUND",
            CoreError::InvalidArgument(_) => "INVALID_ARGUMENT",
            CoreError::ReadOnly(_) => "READ_ONLY",
            CoreError::Cancelled => "CANCELLED",
            CoreError::UnsupportedFormat(_) => "UNSUPPORTED_FORMAT",
            CoreError::Parse(_) => "PARSE_ERROR",
            CoreError::Io(_) => "IO_ERROR",
            CoreError::Internal(_) => "INTERNAL",
        }
    }

    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }

    /// Converts an I/O error for a path to a typed error.
    pub fn from_io(err: std::io::Error, path: &std::path::Path) -> CoreError {
        let p = path.display().to_string();
        match err.kind() {
            std::io::ErrorKind::NotFound => CoreError::FileNotFound(p),
            std::io::ErrorKind::PermissionDenied => CoreError::FileLocked(p),
            _ => {
                // ERROR_SHARING_VIOLATION (32) and ERROR_LOCK_VIOLATION (33) on Windows.
                if matches!(err.raw_os_error(), Some(32) | Some(33)) {
                    CoreError::FileLocked(p)
                } else {
                    CoreError::Io(err)
                }
            }
        }
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(err: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(e, msg) = &err {
            let text = msg.clone().unwrap_or_else(|| e.to_string());
            match e.code {
                rusqlite::ErrorCode::AuthorizationForStatementDenied => {
                    return CoreError::SqlForbidden(text)
                }
                _ if text.contains("not authorized") => return CoreError::SqlForbidden(text),
                rusqlite::ErrorCode::OperationInterrupted => return CoreError::Cancelled,
                rusqlite::ErrorCode::ConstraintViolation
                    if text.contains("cannot store") =>
                {
                    return CoreError::TypeMismatch(text)
                }
                _ => return CoreError::Sql(text),
            }
        }
        CoreError::Sql(err.to_string())
    }
}

/// The JSON form of an error at the IPC boundary.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
}
