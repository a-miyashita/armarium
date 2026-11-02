//! Scratch sources: results that Armarium keeps in files of its own (04. Workspace, section 6).

use super::Inner;
use crate::error::{CoreError, Result};
use crate::model::*;
use crate::workspace::is_scratch_file_name;
use std::path::Path;
use std::time::Instant;

/// The format of a scratch file: UTF-8 without BOM, comma, LF, with a header row.
fn scratch_format() -> ExportFormat {
    ExportFormat::Csv {
        encoding: "UTF-8".into(),
        bom: false,
        delimiter: ',',
        line_ending: LineEnding::Lf,
        quote_all: false,
        header: true,
    }
}

impl Inner {
    /// Writes a result to a new scratch file and registers it.
    pub fn save_result_as_scratch(&mut self, result: ViewId) -> Result<ScratchInfo> {
        let path = self.ws.allocate_scratch(&self.cfg.scratch_root)?;
        if let Err(e) = self.export_result(result, &path, &scratch_format()) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        let registered = (|| -> Result<SourceId> {
            let (id, _) = self.ws.add_source(&path, None)?;
            self.ws.set_scratch(&id, true)?;
            self.ws.save()?;
            Ok(id)
        })();
        let source_id = match registered {
            Ok(id) => id,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(e);
            }
        };
        self.sync_tables();
        self.emit(CoreEvent::WorkspaceChanged);
        let table = self
            .tables
            .values()
            .find(|t| t.source_id == source_id)
            .ok_or_else(|| CoreError::Internal("cannot find the scratch table".into()))?;
        Ok(ScratchInfo {
            source_id,
            table_id: table.id,
            sql_name: table.sql_name.clone(),
        })
    }

    /// Deletes a scratch file. An error is not important: the cleanup at the next start retries.
    pub fn delete_scratch_file(&self, path: &Path) {
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("cannot delete {}: {e}", path.display());
            }
        }
    }

    /// Saves dirty scratch tables. With `force`, Armarium saves all of them now.
    /// Otherwise it saves the tables that had no change for `scratch_idle`.
    pub fn autosave_scratch(&mut self, force: bool) -> Vec<TableId> {
        let now = Instant::now();
        let idle = self.cfg.scratch_idle;
        let candidates: Vec<(TableId, u64)> = self
            .tables
            .values()
            .filter(|t| {
                self.ws
                    .source(&t.source_id)
                    .map(|s| s.scratch)
                    .unwrap_or(false)
            })
            .filter_map(|t| t.data.as_ref().map(|d| (t.id, d.state, d.dirty())))
            .filter_map(|(id, state, dirty)| dirty.then_some((id, state)))
            .collect();
        // Forget the tables that are clean now.
        let dirty_ids: Vec<TableId> = candidates.iter().map(|c| c.0).collect();
        self.scratch_seen.retain(|id, _| dirty_ids.contains(id));
        self.scratch_failed.retain(|id, _| dirty_ids.contains(id));

        let mut saved = Vec::new();
        for (id, state) in candidates {
            // A failed table waits for a new change, unless the caller forces the save.
            if !force && self.scratch_failed.get(&id) == Some(&state) {
                continue;
            }
            if !force && !idle.is_zero() {
                match self.scratch_seen.get(&id) {
                    Some((seen, since)) if *seen == state => {
                        if now.duration_since(*since) < idle {
                            continue;
                        }
                    }
                    _ => {
                        self.scratch_seen.insert(id, (state, now));
                        continue;
                    }
                }
            }
            match self.save_table(id, false) {
                Ok(_) => {
                    self.scratch_seen.remove(&id);
                    self.scratch_failed.remove(&id);
                    saved.push(id);
                }
                Err(e) => {
                    tracing::warn!("automatic save of a scratch table failed: {e}");
                    let first = self.scratch_failed.insert(id, state).is_none();
                    if first {
                        self.emit(CoreEvent::WorkspaceChanged);
                    }
                }
            }
        }
        saved
    }

    /// Deletes files in the scratch directory that no source uses. Returns the number of files.
    pub fn cleanup_scratch(&mut self) -> usize {
        let Some(dir) = self.ws.scratch_dir(&self.cfg.scratch_root) else {
            return 0;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return 0;
        };
        let mut n = 0;
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            // An unfinished write (see `atomic_write`) or an unregistered scratch file.
            let temp = name.starts_with(".scratch_") && name.ends_with(".tmp");
            let orphan = is_scratch_file_name(&name)
                && self.ws.find_by_path(&path.to_string_lossy()).is_none();
            if (temp || orphan) && path.is_file() && std::fs::remove_file(&path).is_ok() {
                n += 1;
            }
        }
        n
    }
}
