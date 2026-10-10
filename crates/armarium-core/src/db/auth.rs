//! The SQL sandbox: an authorizer with an allow list ([ADR-0009]).

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::Connection;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Functions that user SQL can call (lowercase).
const ALLOWED_FUNCTIONS: &[&str] = &[
    // Core scalar functions
    "abs", "char", "coalesce", "concat", "concat_ws", "format", "glob", "hex", "if", "ifnull",
    "iif", "instr", "length", "like", "likelihood", "likely", "lower", "ltrim", "max", "min",
    "nullif", "octet_length", "printf", "quote", "random", "randomblob", "replace", "round",
    "rtrim", "sign", "soundex", "substr", "substring", "trim", "typeof", "unhex", "unicode",
    "unistr", "unistr_quote", "unlikely", "upper", "zeroblob", "changes", "total_changes",
    "last_insert_rowid", "sqlite_version",
    // Aggregate functions
    "avg", "count", "group_concat", "string_agg", "sum", "total", "median", "percentile",
    "percentile_cont", "percentile_disc",
    // Window functions
    "row_number", "rank", "dense_rank", "percent_rank", "cume_dist", "ntile", "lag", "lead",
    "first_value", "last_value", "nth_value",
    // Date and time functions
    "date", "time", "datetime", "julianday", "unixepoch", "strftime", "timediff",
    "current_date", "current_time", "current_timestamp",
    // Math functions
    "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil", "ceiling", "cos", "cosh",
    "degrees", "exp", "floor", "ln", "log", "log10", "log2", "mod", "pi", "pow", "power",
    "radians", "sin", "sinh", "sqrt", "tan", "tanh", "trunc",
    // JSON functions
    "json", "jsonb", "json_array", "jsonb_array", "json_array_length", "json_error_position",
    "json_extract", "jsonb_extract", "json_insert", "jsonb_insert", "json_object", "jsonb_object",
    "json_patch", "jsonb_patch", "json_pretty", "json_remove", "jsonb_remove", "json_replace",
    "jsonb_replace", "json_set", "jsonb_set", "json_type", "json_valid", "json_quote",
    "json_group_array", "jsonb_group_array", "json_group_object", "jsonb_group_object", "->",
    "->>",
    // Armarium functions
    "regexp", "regexp_replace", "normalize_nfkc",
];

/// Table-valued functions that user SQL can read.
const ALLOWED_TABLE_FUNCTIONS: &[&str] = &["json_each", "json_tree", "jsonb_each", "jsonb_tree"];

const ROWID_NAMES: &[&str] = &["rowid", "oid", "_rowid_"];

#[derive(Default)]
struct State {
    enabled: bool,
    /// Working tables that user SQL can read and write (folded names).
    working: HashSet<String>,
    /// Forbid all writes (`query` tool and `--read-only`).
    read_only: bool,
    /// Tables that the statement writes (as SQLite reports them).
    writes: Vec<String>,
    /// The first operation that the authorizer refused.
    denied: Option<String>,
}

/// The result of one checked statement.
#[derive(Debug, Default, Clone)]
pub struct AuthReport {
    pub writes: Vec<String>,
    pub denied: Option<String>,
}

/// Controls the authorizer of one connection.
#[derive(Clone)]
pub struct Sandbox {
    state: Arc<Mutex<State>>,
}

fn is_internal(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("__arm_")
}

impl Sandbox {
    /// Installs the authorizer. It allows everything until `begin` is called.
    pub fn install(conn: &Connection) -> rusqlite::Result<Sandbox> {
        let state = Arc::new(Mutex::new(State::default()));
        let s2 = state.clone();
        conn.authorizer(Some(move |ctx: AuthContext<'_>| -> Authorization {
            let mut st = s2.lock().unwrap_or_else(|e| e.into_inner());
            if !st.enabled {
                return Authorization::Allow;
            }
            match decide(&st, &ctx) {
                Ok(write) => {
                    if let Some(t) = write {
                        if !st.writes.iter().any(|w| w.eq_ignore_ascii_case(&t)) {
                            st.writes.push(t);
                        }
                    }
                    Authorization::Allow
                }
                Err(reason) => {
                    if st.denied.is_none() {
                        st.denied = Some(reason);
                    }
                    Authorization::Deny
                }
            }
        }))?;
        Ok(Sandbox { state })
    }

    /// Enables the checks for user SQL.
    pub fn begin(&self, working: impl IntoIterator<Item = String>, read_only: bool) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.enabled = true;
        st.working = working.into_iter().map(|n| n.to_ascii_lowercase()).collect();
        st.read_only = read_only;
        st.writes.clear();
        st.denied = None;
    }

    /// Clears the report of the last statement. The checks stay enabled.
    pub fn reset_report(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.writes.clear();
        st.denied = None;
    }

    pub fn report(&self) -> AuthReport {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        AuthReport {
            writes: st.writes.clone(),
            denied: st.denied.clone(),
        }
    }

    /// Disables the checks (for the internal SQL of the application).
    pub fn end(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.enabled = false;
    }
}

/// Returns `Ok(Some(table))` for an allowed write, `Ok(None)` for another allowed
/// operation, and `Err(reason)` for a refused operation.
fn decide(st: &State, ctx: &AuthContext<'_>) -> Result<Option<String>, String> {
    let internal_trigger = ctx.accessor.map(is_internal).unwrap_or(false);
    let in_main = matches!(ctx.database_name, None | Some("main"));
    let working = |t: &str| in_main && st.working.contains(&t.to_ascii_lowercase());
    match ctx.action {
        AuthAction::Select | AuthAction::Recursive => Ok(None),
        AuthAction::Read { table_name, .. } => {
            if working(table_name)
                || (internal_trigger && is_internal(table_name))
                || ALLOWED_TABLE_FUNCTIONS.contains(&table_name.to_ascii_lowercase().as_str())
            {
                Ok(None)
            } else {
                Err(format!("reading table {table_name}"))
            }
        }
        AuthAction::Insert { table_name } | AuthAction::Delete { table_name } => {
            write_target(st, table_name, internal_trigger, working(table_name))
        }
        AuthAction::Update { table_name, column_name } => {
            if ROWID_NAMES.contains(&column_name.to_ascii_lowercase().as_str()) && !internal_trigger {
                return Err(format!("updating the {column_name} of {table_name}"));
            }
            write_target(st, table_name, internal_trigger, working(table_name))
        }
        AuthAction::Function { function_name } => {
            if ALLOWED_FUNCTIONS.contains(&function_name.to_ascii_lowercase().as_str()) {
                Ok(None)
            } else {
                Err(format!("function {function_name}()"))
            }
        }
        AuthAction::Pragma { pragma_name, .. } => Err(format!("PRAGMA {pragma_name}")),
        AuthAction::Attach { .. } => Err("ATTACH".into()),
        AuthAction::Detach { .. } => Err("DETACH".into()),
        AuthAction::Transaction { .. } => Err("transaction control".into()),
        AuthAction::Savepoint { .. } => Err("SAVEPOINT".into()),
        AuthAction::AlterTable { table_name, .. } => Err(format!("ALTER TABLE {table_name}")),
        AuthAction::Analyze { .. } => Err("ANALYZE".into()),
        AuthAction::Reindex { .. } => Err("REINDEX".into()),
        AuthAction::CreateTable { .. }
        | AuthAction::CreateTempTable { .. }
        | AuthAction::CreateIndex { .. }
        | AuthAction::CreateTempIndex { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::CreateTempView { .. }
        | AuthAction::CreateTrigger { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateVtable { .. } => Err("CREATE".into()),
        AuthAction::DropTable { .. }
        | AuthAction::DropTempTable { .. }
        | AuthAction::DropIndex { .. }
        | AuthAction::DropTempIndex { .. }
        | AuthAction::DropView { .. }
        | AuthAction::DropTempView { .. }
        | AuthAction::DropTrigger { .. }
        | AuthAction::DropTempTrigger { .. }
        | AuthAction::DropVtable { .. } => Err("DROP".into()),
        AuthAction::Unknown { code, .. } => {
            // SQLITE_COPY (0) is obsolete. VACUUM is reported as an unknown code in some builds.
            Err(format!("operation code {code}"))
        }
        _ => Err("operation".into()),
    }
}

fn write_target(
    st: &State,
    table: &str,
    internal_trigger: bool,
    is_working: bool,
) -> Result<Option<String>, String> {
    if internal_trigger && is_internal(table) {
        return Ok(None);
    }
    if st.read_only {
        return Err(format!("writing table {table} (read-only)"));
    }
    if is_working {
        Ok(Some(table.to_string()))
    } else {
        Err(format!("writing table {table}"))
    }
}
