//! The session database: connections, settings, and functions ([ADR-0003]).

pub mod auth;
pub mod changeset;

use crate::error::Result;
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OpenFlags};
use std::fs::File;
use std::path::{Path, PathBuf};

/// The session database file of this process, and its lock.
///
/// The file is deleted when the value is dropped. Files of processes that ended
/// without cleanup are deleted by `SessionFile::create` (their lock is free).
pub struct SessionFile {
    path: PathBuf,
    lock_path: PathBuf,
    _lock: File,
}

impl SessionFile {
    pub fn create(dir: &Path) -> Result<SessionFile> {
        std::fs::create_dir_all(dir)?;
        cleanup_stale(dir);
        let pid = std::process::id();
        let stamp = uuid::Uuid::now_v7().simple().to_string();
        let base = format!("session-{pid}-{}", &stamp[stamp.len() - 8..]);
        let lock_path = dir.join(format!("{base}.lock"));
        let lock = File::create(&lock_path)?;
        lock.lock()?;
        let path = dir.join(format!("{base}.db"));
        Ok(SessionFile {
            path,
            lock_path,
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SessionFile {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Deletes session files of processes that do not run any more.
fn cleanup_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(base) = name.strip_suffix(".lock") else {
            continue;
        };
        if !base.starts_with("session-") {
            continue;
        }
        let Ok(f) = File::options().write(true).open(e.path()) else {
            continue;
        };
        if f.try_lock().is_ok() {
            for suffix in [".db", ".db-wal", ".db-shm"] {
                let _ = std::fs::remove_file(dir.join(format!("{base}{suffix}")));
            }
            drop(f);
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Settings of the session database.
#[derive(Debug, Clone)]
pub struct DbSettings {
    /// The page cache size in KiB.
    pub cache_kib: i64,
}

impl Default for DbSettings {
    fn default() -> Self {
        DbSettings {
            cache_kib: 512 * 1024,
        }
    }
}

/// Opens the write connection.
pub fn open_write(path: &Path, settings: &DbSettings) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.pragma_update(None, "cache_size", -settings.cache_kib)?;
    conn.pragma_update(None, "temp_store", "FILE")?;
    // Never shrink the file automatically: rowid values must stay stable.
    conn.pragma_update(None, "auto_vacuum", "NONE")?;
    harden(&conn)?;
    register_functions(&conn)?;
    Ok(conn)
}

/// Opens a read connection (for page fetches).
pub fn open_read(path: &Path, settings: &DbSettings) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "cache_size", -(settings.cache_kib / 4).max(2048))?;
    harden(&conn)?;
    register_functions(&conn)?;
    Ok(conn)
}

/// Safe connection settings ([ADR-0009], section 2).
fn harden(conn: &Connection) -> Result<()> {
    use rusqlite::config::DbConfig;
    use rusqlite::limits::Limit;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)?;
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    Ok(())
}

/// Registers the Armarium functions (07. SQL, section 5).
pub fn register_functions(conn: &Connection) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("regexp", 2, flags, |ctx| {
        let re = ctx.get_or_create_aux(0, |vr| -> std::result::Result<regex::Regex, BoxError> {
            Ok(regex::Regex::new(vr.as_str()?)?)
        })?;
        let text = match ctx.get_raw(1) {
            rusqlite::types::ValueRef::Null => return Ok(None),
            v => value_text(v),
        };
        Ok(Some(re.is_match(&text)))
    })?;
    conn.create_scalar_function("regexp_replace", 3, flags, |ctx| {
        let re = ctx.get_or_create_aux(1, |vr| -> std::result::Result<regex::Regex, BoxError> {
            Ok(regex::Regex::new(vr.as_str()?)?)
        })?;
        let text = match ctx.get_raw(0) {
            rusqlite::types::ValueRef::Null => return Ok(None),
            v => value_text(v),
        };
        let repl: String = ctx.get::<Option<String>>(2)?.unwrap_or_default();
        Ok(Some(re.replace_all(&text, repl.as_str()).into_owned()))
    })?;
    conn.create_scalar_function("normalize_nfkc", 1, flags, |ctx| {
        use unicode_normalization::UnicodeNormalization;
        match ctx.get_raw(0) {
            rusqlite::types::ValueRef::Null => Ok(None),
            v => Ok(Some(value_text(v).nfkc().collect::<String>())),
        }
    })?;
    Ok(())
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

fn value_text(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => crate::format::real_to_text(f),
        ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
    }
}

/// Runs `f` in a transaction. Rolls back if `f` fails.
pub fn in_transaction<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(v) => {
            if let Err(e) = conn.execute_batch("COMMIT") {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.into());
            }
            Ok(v)
        }
        Err(e) => {
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests;
