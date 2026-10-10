//! Changesets of the SQLite session extension, for undo and redo ([ADR-0006]).
//!
//! `rusqlite` does not expose `SQLITE_SESSION_OBJCONFIG_ROWID`, which is necessary
//! to record changes of tables without a PRIMARY KEY (the working tables use `rowid`).
//! Thus, this module calls the C API directly.

use crate::error::{CoreError, Result};
use rusqlite::ffi;
use rusqlite::Connection;
use std::ffi::{c_int, c_void, CString};

/// Records the changes of some tables while it exists.
pub struct Recorder {
    session: *mut ffi::sqlite3_session,
}

impl Recorder {
    /// Starts to record the changes of `tables` (in the `main` database).
    pub fn start(conn: &Connection, tables: &[&str]) -> Result<Recorder> {
        let db = unsafe { conn.handle() };
        let mut session: *mut ffi::sqlite3_session = std::ptr::null_mut();
        let main = CString::new("main").unwrap();
        let rc = unsafe { ffi::sqlite3session_create(db, main.as_ptr(), &mut session) };
        check(rc, "sqlite3session_create")?;
        let rec = Recorder { session };
        let mut on: c_int = 1;
        let rc = unsafe {
            ffi::sqlite3session_object_config(
                session,
                ffi::SQLITE_SESSION_OBJCONFIG_ROWID,
                &mut on as *mut c_int as *mut c_void,
            )
        };
        check(rc, "SQLITE_SESSION_OBJCONFIG_ROWID")?;
        for t in tables {
            let name = CString::new(*t).map_err(|e| CoreError::Internal(e.to_string()))?;
            let rc = unsafe { ffi::sqlite3session_attach(session, name.as_ptr()) };
            check(rc, "sqlite3session_attach")?;
        }
        Ok(rec)
    }

    /// The changeset of the changes so far.
    pub fn changeset(&self) -> Result<Vec<u8>> {
        let mut n: c_int = 0;
        let mut p: *mut c_void = std::ptr::null_mut();
        let rc = unsafe { ffi::sqlite3session_changeset(self.session, &mut n, &mut p) };
        check(rc, "sqlite3session_changeset")?;
        Ok(take_buffer(p, n))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe { ffi::sqlite3session_delete(self.session) };
    }
}

fn take_buffer(p: *mut c_void, n: c_int) -> Vec<u8> {
    if p.is_null() || n <= 0 {
        if !p.is_null() {
            unsafe { ffi::sqlite3_free(p) };
        }
        return Vec::new();
    }
    let v = unsafe { std::slice::from_raw_parts(p as *const u8, n as usize) }.to_vec();
    unsafe { ffi::sqlite3_free(p) };
    v
}

fn check(rc: c_int, what: &str) -> Result<()> {
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(CoreError::Internal(format!("{what} failed with code {rc}")))
    }
}

/// Inverts a changeset (an INSERT becomes a DELETE, and so on).
pub fn invert(changeset: &[u8]) -> Result<Vec<u8>> {
    if changeset.is_empty() {
        return Ok(Vec::new());
    }
    let mut n: c_int = 0;
    let mut p: *mut c_void = std::ptr::null_mut();
    let rc = unsafe {
        ffi::sqlite3changeset_invert(
            changeset.len() as c_int,
            changeset.as_ptr() as *mut c_void,
            &mut n,
            &mut p,
        )
    };
    check(rc, "sqlite3changeset_invert")?;
    Ok(take_buffer(p, n))
}

/// Conflict handling while Armarium applies a changeset for undo or redo.
///
/// The triggers on a working table also write the order table. When a changeset
/// contains the same change of the order table, the change conflicts with the
/// trigger result. The changeset value wins (REPLACE), and a row that the trigger
/// already deleted is skipped (OMIT).
unsafe extern "C" fn on_conflict(
    _ctx: *mut c_void,
    kind: c_int,
    _iter: *mut ffi::sqlite3_changeset_iter,
) -> c_int {
    match kind {
        ffi::SQLITE_CHANGESET_DATA | ffi::SQLITE_CHANGESET_CONFLICT => {
            ffi::SQLITE_CHANGESET_REPLACE
        }
        ffi::SQLITE_CHANGESET_NOTFOUND => ffi::SQLITE_CHANGESET_OMIT,
        _ => ffi::SQLITE_CHANGESET_ABORT,
    }
}

/// Applies a changeset to the `main` database.
pub fn apply(conn: &Connection, changeset: &[u8]) -> Result<()> {
    if changeset.is_empty() {
        return Ok(());
    }
    let db = unsafe { conn.handle() };
    let rc = unsafe {
        ffi::sqlite3changeset_apply(
            db,
            changeset.len() as c_int,
            changeset.as_ptr() as *mut c_void,
            None,
            Some(on_conflict),
            std::ptr::null_mut(),
        )
    };
    if rc != ffi::SQLITE_OK {
        let msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(db)) }
            .to_string_lossy()
            .into_owned();
        return Err(CoreError::Internal(format!(
            "cannot apply the changeset: {msg} (code {rc})"
        )));
    }
    Ok(())
}
