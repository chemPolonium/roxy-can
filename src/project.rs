use std::path::{Path, PathBuf};

use crate::config::{Config, PROJECT_EXT, ProjectFile};

use crate::app::App;

/// Newest-first recent list: dedups and caps at 8 entries.
fn push_recent(list: &mut Vec<String>, path: String) {
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(8);
}

/// Whether a record draft is still one of the tool's own defaults
/// (empty, the bare "record" name, or a full path ending in a
/// Record/record folder): only those may be silently retargeted on
/// project open/save.
fn app_record_draft_is_default(draft: &str) -> bool {
    let draft = draft.trim();
    if draft.is_empty() {
        return true;
    }
    if draft.eq_ignore_ascii_case("record") {
        return true;
    }
    let norm = draft.replace('\\', "/").to_ascii_lowercase();
    norm.ends_with("/record/record")
}

impl App {
    pub fn push_recent_dbc(&mut self, path: String) {
        push_recent(&mut self.recent_dbc, path);
    }

    pub fn push_recent_log(&mut self, path: String) {
        push_recent(&mut self.recent_log, path);
    }

    /// Rebuilds the whole workspace back to factory defaults, keeping only
    /// the machine-local recents.
    pub fn reset_to_defaults(&mut self) {
        self.stop();
        let recent_dbc = std::mem::take(&mut self.recent_dbc);
        let recent_log = std::mem::take(&mut self.recent_log);
        let recent_projects = std::mem::take(&mut self.recent_projects);
        // Revisions must keep increasing across a reset, or the browser would
        // not notice that the new project's workspace arrived.
        let workspace_rev = self.workspace_rev;
        // Rebuild on the same drive the workspace was on: the web host's
        // reset replaces the core thread (the old one exits with its dropped
        // sender), tests rebuild their hand-cranked core.
        let threaded = matches!(self.drive, crate::app::CoreDrive::Threaded);
        *self = if threaded {
            App::new()
        } else {
            App::headless()
        };
        self.recent_dbc = recent_dbc;
        self.recent_log = recent_log;
        self.recent_projects = recent_projects;
        self.workspace_rev = workspace_rev;
    }

    /// Display name of the current project: the file stem, or "Untitled".
    pub fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub fn push_recent_project(&mut self, path: String) {
        push_recent(&mut self.recent_projects, path);
    }

    /// Serializable snapshot of the workspace configuration, used to tell
    /// whether anything changed since the last load/save/reset.
    pub(crate) fn config_snapshot(&self) -> String {
        // The web workspace is saved but never marks a project dirty: moving
        // a panel or a cursor is not a semantic change to the bench setup.
        let mut cfg = Config::from_app(self, None);
        cfg.web = None;
        serde_json::to_string(&cfg).unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.config_snapshot() != self.baseline
    }

    /// Marks the current workspace as the clean baseline (after load/save).
    pub fn mark_clean(&mut self) {
        self.baseline = self.config_snapshot();
    }

    /// Saves the workspace as a .rxproj file. `None` opens a picker.
    /// Returns true when a file was written.
    pub fn save_project(&mut self, path: Option<PathBuf>) -> bool {
        let path = match path {
            Some(p) => p,
            None => {
                let mut dlg = rfd::FileDialog::new().add_filter("roxy-can project", &[PROJECT_EXT]);
                if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(dir);
                }
                dlg = dlg.set_file_name(format!("{}.rxproj", self.project_name()));
                match dlg.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let base = path.parent().map(|d| d.to_path_buf());
        let proj = ProjectFile {
            config: Config::from_app(self, base.as_deref()),
        };
        let written = serde_json::to_string_pretty(&proj)
            .map_err(|e| e.to_string())
            .and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                self.push_recent_project(path.to_string_lossy().to_string());
                self.project_path = Some(path.clone());
                self.retarget_record_draft();
                self.baseline = self.config_snapshot();
                self.status = format!("project saved: {}", path.display());
                true
            }
            Err(e) => {
                self.fail(format!("project save failed: {e}"));
                false
            }
        }
    }

    /// Loads a .rxproj file, replacing the current workspace.
    pub fn open_project_path(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.fail(format!("project read failed: {e}"));
                return;
            }
        };
        match serde_json::from_str::<ProjectFile>(&text) {
            Ok(proj) => {
                self.reset_to_defaults();
                let mut cfg = proj.config;
                cfg.resolve_paths(path.parent());
                let problems = cfg.apply(self);
                self.workspace_rev = self.workspace_rev.wrapping_add(1);
                self.project_path = Some(path.to_path_buf());
                self.retarget_record_draft();
                self.baseline = self.config_snapshot();
                self.push_recent_project(path.to_string_lossy().to_string());
                self.status = format!("project loaded: {}", path.display());
                // After the "loaded" line: a half-restored project has to say
                // so on the bar the user is looking at, and keep the detail
                // where it can still be read.
                self.report_load_problems(problems);
            }
            Err(e) => self.fail(format!("project ignored: {e}")),
        }
    }

    /// Resets the record draft to its default file name. The recording
    /// lands in the project's Record/ folder (resolved at arm time);
    /// only the name shows in the toolbar input, and a draft the user
    /// typed themselves is left alone.
    pub fn retarget_record_draft(&mut self) {
        if app_record_draft_is_default(&self.record_path_buf) {
            self.record_path_buf = "record".to_string();
        }
    }

    /// Starts a fresh, completely empty untitled workspace: no DBCs, no
    /// observer windows, no generator entries.
    #[cfg(test)]
    pub fn new_project(&mut self) {
        self.reset_to_defaults();
        self.workspace_rev = self.workspace_rev.wrapping_add(1);
        // The bus half of "empty" is a command; wait for it to land so
        // the baseline below describes the emptied workspace.
        self.send(crate::bus::BusCommand::ClearDatabases);
        self.settle();
        self.trace_windows.clear();
        self.msg_windows.clear();
        self.stats_windows.clear();
        self.graphics.clear();
        self.data_windows.clear();
        // The sixth kind the bootstrap opens. Missing it left a State Tracker
        // window standing in every "new project".
        self.state_trackers.clear();
        self.baseline = self.config_snapshot();
        self.status = "new project".to_string();
    }
}
