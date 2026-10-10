//! The workspace: registered sources, SQL names, and `workspace.json` (04. Workspace).

use crate::error::{CoreError, Result};
use crate::fsutil;
use crate::model::{LogicalType, ReadOptions, SourceId, SourceKind};
use crate::naming;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const WORKSPACE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
    /// Unknown fields. Armarium keeps them when it writes the file.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for WorkspaceFile {
    fn default() -> Self {
        WorkspaceFile {
            version: WORKSPACE_VERSION,
            sources: Vec::new(),
            extra: Map::new(),
        }
    }
}

fn default_version() -> u32 {
    WORKSPACE_VERSION
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

fn is_empty_object(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.is_empty()) || v.is_null()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceEntry {
    pub id: SourceId,
    pub path: String,
    pub kind: SourceKind,
    /// The SQL name of a CSV or TSV source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql_name: Option<String>,
    #[serde(default)]
    pub added_at: String,
    /// Read settings. Armarium keeps the raw JSON so that unknown fields stay.
    #[serde(default = "empty_object")]
    pub options: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<SheetEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SheetEntry {
    pub name: String,
    pub sql_name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default = "empty_object", skip_serializing_if = "is_empty_object")]
    pub options: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A sheet of a workbook, as the reader gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetListing {
    pub name: String,
    pub hidden: bool,
}

impl SourceEntry {
    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }

    pub fn file_name(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone())
    }

    /// True if the source cannot be written in its own format.
    pub fn is_read_only_format(&self) -> bool {
        let ext = extension(Path::new(&self.path));
        matches!(ext.as_str(), "xls" | "xlsb" | "ods")
    }

    /// The read settings for a table (a sheet overrides the source settings).
    pub fn read_options(&self, sheet: Option<&str>) -> ReadOptions {
        let mut merged = match &self.options {
            Value::Object(m) => m.clone(),
            _ => Map::new(),
        };
        if let Some(name) = sheet {
            if let Some(s) = self.sheets.iter().find(|s| s.name == name) {
                if let Value::Object(m) = &s.options {
                    for (k, v) in m {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        // `headerRow` (1-based) is a short form of `skipRows` and `hasHeader`.
        if let Some(row) = merged.get("headerRow").and_then(|v| v.as_u64()) {
            if row >= 1 {
                merged.insert("skipRows".into(), Value::from(row - 1));
                merged.insert("hasHeader".into(), Value::Bool(true));
            }
        }
        // Remove `null` values so that the defaults apply ("detect automatically").
        merged.retain(|_, v| !v.is_null());
        serde_json::from_value(Value::Object(merged)).unwrap_or_default()
    }
}

/// The kind of a file, from its extension.
pub fn kind_for_path(path: &Path) -> Result<SourceKind> {
    match extension(path).as_str() {
        "csv" | "tsv" | "tab" | "txt" => Ok(SourceKind::Csv),
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => Ok(SourceKind::Excel),
        other => Err(CoreError::UnsupportedFormat(if other.is_empty() {
            path.display().to_string()
        } else {
            format!(".{other}")
        })),
    }
}

pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The workspace in memory, with its file.
pub struct Workspace {
    path: PathBuf,
    file: WorkspaceFile,
    stamp: Option<(u64, std::time::SystemTime)>,
    changed: HashSet<SourceId>,
    removed: HashSet<SourceId>,
}

impl Workspace {
    /// Opens the workspace file. If the file does not exist, the workspace is empty.
    pub fn open(path: impl Into<PathBuf>) -> Result<Workspace> {
        let path = path.into();
        let mut ws = Workspace {
            path,
            file: WorkspaceFile::default(),
            stamp: None,
            changed: HashSet::new(),
            removed: HashSet::new(),
        };
        if let Some((file, stamp)) = ws.read_disk()? {
            ws.file = file;
            ws.stamp = Some(stamp);
        }
        Ok(ws)
    }

    /// The default location of `workspace.json`.
    pub fn default_path() -> PathBuf {
        fsutil::config_dir().join("workspace.json")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sources(&self) -> &[SourceEntry] {
        &self.file.sources
    }

    pub fn source(&self, id: &str) -> Result<&SourceEntry> {
        self.file
            .sources
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound(format!("source {id}")))
    }

    fn source_mut(&mut self, id: &str) -> Result<&mut SourceEntry> {
        self.changed.insert(id.to_string());
        self.file
            .sources
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound(format!("source {id}")))
    }

    /// Finds a source by its path.
    pub fn find_by_path(&self, path: &str) -> Option<&SourceEntry> {
        self.file
            .sources
            .iter()
            .find(|s| fsutil::same_path(&s.path, path))
    }

    /// All table SQL names in the workspace (folded).
    fn sql_names(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        for s in &self.file.sources {
            if let Some(n) = &s.sql_name {
                set.insert(naming::fold(n));
            }
            for sh in &s.sheets {
                set.insert(naming::fold(&sh.sql_name));
            }
        }
        set
    }

    fn unique_name(&self, base: &str, extra: &HashSet<String>) -> String {
        let used = self.sql_names();
        naming::make_unique(base, |c| {
            let f = naming::fold(c);
            used.contains(&f) || extra.contains(&f)
        })
    }

    /// Registers a file. Returns the ID of the source and `true` if the source is new.
    ///
    /// `sheets` is the list of sheets for an Excel workbook.
    pub fn add_source(
        &mut self,
        path: &Path,
        sheets: Option<Vec<SheetListing>>,
    ) -> Result<(SourceId, bool)> {
        let abs = fsutil::absolute(path)?;
        let path_str = abs.to_string_lossy().into_owned();
        if let Some(existing) = self.find_by_path(&path_str) {
            return Ok((existing.id.clone(), false));
        }
        if !abs.is_file() {
            return Err(CoreError::FileNotFound(path_str));
        }
        let kind = kind_for_path(&abs)?;
        let stem = abs
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = uuid::Uuid::now_v7().to_string();
        let mut entry = SourceEntry {
            id: id.clone(),
            path: path_str,
            kind,
            sql_name: None,
            added_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            options: empty_object(),
            sheets: Vec::new(),
            extra: Map::new(),
        };
        match kind {
            SourceKind::Csv => {
                entry.sql_name = Some(self.unique_name(&naming::normalize(&stem), &HashSet::new()));
            }
            SourceKind::Excel => {
                let mut taken = HashSet::new();
                for sh in sheets.unwrap_or_default() {
                    let name = self
                        .unique_name(&naming::normalize(&format!("{stem}_{}", sh.name)), &taken);
                    taken.insert(naming::fold(&name));
                    entry.sheets.push(SheetEntry {
                        name: sh.name,
                        sql_name: name,
                        hidden: sh.hidden,
                        options: empty_object(),
                        extra: Map::new(),
                    });
                }
            }
        }
        self.changed.insert(id.clone());
        self.file.sources.push(entry);
        Ok((id, true))
    }

    /// Unregisters a source. The file does not change.
    pub fn remove_source(&mut self, id: &str) -> Result<SourceEntry> {
        let pos = self
            .file
            .sources
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound(format!("source {id}")))?;
        self.changed.remove(id);
        self.removed.insert(id.to_string());
        Ok(self.file.sources.remove(pos))
    }

    /// Changes the SQL name of a table. Returns the name that Armarium uses.
    pub fn rename_table(
        &mut self,
        id: &str,
        sheet: Option<&str>,
        new_name: &str,
    ) -> Result<String> {
        let name = naming::validate_table_name(new_name)?;
        let current = self.table_sql_name(id, sheet)?;
        let folded = naming::fold(&name);
        if folded != naming::fold(&current) && self.sql_names().contains(&folded) {
            return Err(CoreError::InvalidArgument(format!(
                "the SQL name '{name}' is already used"
            )));
        }
        let src = self.source_mut(id)?;
        match sheet {
            None => src.sql_name = Some(name.clone()),
            Some(sh) => {
                let e = src
                    .sheets
                    .iter_mut()
                    .find(|s| s.name == sh)
                    .ok_or_else(|| CoreError::NotFound(format!("sheet {sh}")))?;
                e.sql_name = name.clone();
            }
        }
        Ok(name)
    }

    pub fn table_sql_name(&self, id: &str, sheet: Option<&str>) -> Result<String> {
        let src = self.source(id)?;
        match sheet {
            None => src
                .sql_name
                .clone()
                .ok_or_else(|| CoreError::NotFound(format!("table of source {id}"))),
            Some(sh) => src
                .sheets
                .iter()
                .find(|s| s.name == sh)
                .map(|s| s.sql_name.clone())
                .ok_or_else(|| CoreError::NotFound(format!("sheet {sh}"))),
        }
    }

    /// Replaces the read settings of a source or a sheet.
    pub fn set_options(
        &mut self,
        id: &str,
        sheet: Option<&str>,
        options: &ReadOptions,
    ) -> Result<()> {
        let new_value =
            serde_json::to_value(options).map_err(|e| CoreError::Internal(e.to_string()))?;
        let src = self.source_mut(id)?;
        let target = match sheet {
            None => &mut src.options,
            Some(sh) => {
                &mut src
                    .sheets
                    .iter_mut()
                    .find(|s| s.name == sh)
                    .ok_or_else(|| CoreError::NotFound(format!("sheet {sh}")))?
                    .options
            }
        };
        // Keep unknown fields; replace the known fields.
        let mut obj = match target.take() {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        for key in [
            "encoding",
            "delimiter",
            "quote",
            "hasHeader",
            "skipRows",
            "typeInference",
            "columnTypes",
            "headerRow",
        ] {
            obj.remove(key);
        }
        if let Value::Object(m) = new_value {
            obj.extend(m);
        }
        *target = Value::Object(obj);
        Ok(())
    }

    /// Sets the type override of one column (by display name).
    pub fn set_column_type(
        &mut self,
        id: &str,
        sheet: Option<&str>,
        column: &str,
        ty: Option<LogicalType>,
    ) -> Result<()> {
        let mut opts = self.source(id)?.read_options(sheet);
        match ty {
            Some(t) => {
                opts.column_types.insert(column.to_string(), t);
            }
            None => {
                opts.column_types.remove(column);
            }
        }
        self.set_options(id, sheet, &opts)
    }

    /// Connects a source to a different path. The SQL names and the settings do not change.
    pub fn relocate(&mut self, id: &str, new_path: &Path) -> Result<()> {
        let abs = fsutil::absolute(new_path)?;
        let s = abs.to_string_lossy().into_owned();
        if let Some(other) = self.find_by_path(&s) {
            if other.id != id {
                return Err(CoreError::InvalidArgument(format!(
                    "the file is already registered: {s}"
                )));
            }
        }
        self.source_mut(id)?.path = s;
        Ok(())
    }

    /// Updates the list of sheets of a workbook. Keeps the entries of sheets that still exist.
    /// Returns true if the list changed.
    pub fn sync_sheets(&mut self, id: &str, sheets: &[SheetListing]) -> Result<bool> {
        let src = self.source(id)?.clone();
        let stem = Path::new(&src.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut new_list = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        for sh in sheets {
            if let Some(old) = src.sheets.iter().find(|s| s.name == sh.name) {
                let mut e = old.clone();
                e.hidden = sh.hidden;
                taken.insert(naming::fold(&e.sql_name));
                new_list.push(e);
            }
        }
        // Other sources' names must not collide with the new names.
        let mut others: HashSet<String> = HashSet::new();
        for s in self.file.sources.iter().filter(|s| s.id != id) {
            if let Some(n) = &s.sql_name {
                others.insert(naming::fold(n));
            }
            for sh in &s.sheets {
                others.insert(naming::fold(&sh.sql_name));
            }
        }
        for (i, sh) in sheets.iter().enumerate() {
            if src.sheets.iter().any(|s| s.name == sh.name) {
                continue;
            }
            let base = naming::normalize(&format!("{stem}_{}", sh.name));
            let name = naming::make_unique(&base, |c| {
                let f = naming::fold(c);
                taken.contains(&f) || others.contains(&f)
            });
            taken.insert(naming::fold(&name));
            let entry = SheetEntry {
                name: sh.name.clone(),
                sql_name: name,
                hidden: sh.hidden,
                options: empty_object(),
                extra: Map::new(),
            };
            new_list.insert(i.min(new_list.len()), entry);
        }
        // Keep the order of the workbook.
        new_list.sort_by_key(|e| {
            sheets
                .iter()
                .position(|s| s.name == e.name)
                .unwrap_or(usize::MAX)
        });
        if new_list == src.sheets {
            return Ok(false);
        }
        self.source_mut(id)?.sheets = new_list;
        Ok(true)
    }

    fn read_disk(&self) -> Result<Option<(WorkspaceFile, (u64, std::time::SystemTime))>> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CoreError::from_io(e, &self.path)),
        };
        let text =
            std::fs::read_to_string(&self.path).map_err(|e| CoreError::from_io(e, &self.path))?;
        let file: WorkspaceFile = if text.trim().is_empty() {
            WorkspaceFile::default()
        } else {
            serde_json::from_str(&text)
                .map_err(|e| CoreError::Parse(format!("{}: {e}", self.path.display())))?
        };
        if file.version > WORKSPACE_VERSION {
            return Err(CoreError::Parse(format!(
                "{}: workspace version {} is newer than this application supports",
                self.path.display(),
                file.version
            )));
        }
        let stamp = (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH));
        Ok(Some((file, stamp)))
    }

    /// Reads the file again if another process changed it. Returns true if it changed.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => Some((m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH))),
            Err(_) => None,
        };
        if meta == self.stamp {
            return Ok(false);
        }
        match self.read_disk()? {
            Some((file, stamp)) => {
                let changed = file != self.file;
                self.file = file;
                self.stamp = Some(stamp);
                Ok(changed)
            }
            None => {
                self.stamp = None;
                Ok(false)
            }
        }
    }

    /// Writes the changes. Reads the latest file first and merges for each source
    /// ([ADR-0012]). The last write wins for the same source.
    pub fn save(&mut self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let lock_path = PathBuf::from(format!("{}.lock", self.path.display()));
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| CoreError::from_io(e, &lock_path))?;
        lock.lock().map_err(|e| CoreError::from_io(e, &lock_path))?;

        let mut merged = match self.read_disk()? {
            Some((disk, _)) => disk,
            None => WorkspaceFile::default(),
        };
        merged.version = WORKSPACE_VERSION;
        merged.sources.retain(|s| !self.removed.contains(&s.id));
        for ours in self
            .file
            .sources
            .iter()
            .filter(|s| self.changed.contains(&s.id))
        {
            match merged.sources.iter_mut().find(|s| s.id == ours.id) {
                Some(slot) => *slot = ours.clone(),
                None => merged.sources.push(ours.clone()),
            }
        }
        let json = serde_json::to_string_pretty(&merged)
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        fsutil::atomic_write(&self.path, |w| {
            use std::io::Write;
            w.write_all(json.as_bytes())?;
            w.write_all(b"\n")?;
            Ok(())
        })?;
        let meta = std::fs::metadata(&self.path)?;
        self.stamp = Some((meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
        self.file = merged;
        self.changed.clear();
        self.removed.clear();
        drop(lock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        p
    }

    #[test]
    fn registers_and_names_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::open(dir.path().join("workspace.json")).unwrap();
        let a = touch(dir.path(), "売上 2026.csv");
        let (id, new) = ws.add_source(&a, None).unwrap();
        assert!(new);
        assert_eq!(
            ws.source(&id).unwrap().sql_name.as_deref(),
            Some("売上_2026")
        );
        let (id2, new2) = ws.add_source(&a, None).unwrap();
        assert_eq!(id, id2);
        assert!(!new2);

        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let b = touch(&sub, "売上-2026.tsv");
        let (idb, _) = ws.add_source(&b, None).unwrap();
        assert_eq!(
            ws.source(&idb).unwrap().sql_name.as_deref(),
            Some("売上_2026_2")
        );

        let x = touch(dir.path(), "budget.xlsx");
        let (idx, _) = ws
            .add_source(
                &x,
                Some(vec![
                    SheetListing {
                        name: "2026".into(),
                        hidden: false,
                    },
                    SheetListing {
                        name: "Sheet 1".into(),
                        hidden: true,
                    },
                ]),
            )
            .unwrap();
        let src = ws.source(&idx).unwrap();
        assert_eq!(src.sheets[0].sql_name, "budget_2026");
        assert_eq!(src.sheets[1].sql_name, "budget_Sheet_1");
        assert!(src.sheets[1].hidden);

        assert!(ws.rename_table(&idb, None, "budget_2026").is_err());
        assert!(ws.rename_table(&idb, None, "__arm_x").is_err());
        ws.rename_table(&idb, None, "sales_tsv").unwrap();
        ws.save().unwrap();

        let ws2 = Workspace::open(dir.path().join("workspace.json")).unwrap();
        assert_eq!(ws2.sources().len(), 3);
        assert_eq!(
            ws2.source(&idb).unwrap().sql_name.as_deref(),
            Some("sales_tsv")
        );
    }

    #[test]
    fn merges_changes_of_two_processes_and_keeps_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let wp = dir.path().join("workspace.json");
        std::fs::write(&wp, r#"{"version":1,"future":{"x":1},"sources":[]}"#).unwrap();
        let mut a = Workspace::open(&wp).unwrap();
        let mut b = Workspace::open(&wp).unwrap();
        let fa = touch(dir.path(), "a.csv");
        let fb = touch(dir.path(), "b.csv");
        a.add_source(&fa, None).unwrap();
        a.save().unwrap();
        b.add_source(&fb, None).unwrap();
        b.save().unwrap();
        assert!(a.reload_if_changed().unwrap());
        assert_eq!(a.sources().len(), 2);
        let text = std::fs::read_to_string(&wp).unwrap();
        assert!(text.contains("\"future\""));
    }

    #[test]
    fn reads_options_with_header_row() {
        let entry: SourceEntry = serde_json::from_value(serde_json::json!({
            "id": "x", "path": "C:\\a.xlsx", "kind": "excel",
            "options": { "hasHeader": true, "encoding": null, "unknown": 5 },
            "sheets": [{ "name": "2026", "sqlName": "a_2026", "options": { "headerRow": 3 } }]
        }))
        .unwrap();
        let o = entry.read_options(Some("2026"));
        assert_eq!(o.skip_rows, 2);
        assert_eq!(o.has_header, Some(true));
        assert!(o.type_inference);
        let back = serde_json::to_value(&entry).unwrap();
        assert_eq!(back["options"]["unknown"], 5);
    }
}
