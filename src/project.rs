use std::path::{Path, PathBuf};

use crate::config::{AUTOSAVE_PATH, Config, META_PATH, Meta, PROJECT_EXT, ProjectFile};

use crate::app::App;

/// Deferred action waiting behind the "unsaved project" confirmation modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Quit,
    NewProject,
    OpenProject,
    OpenPath(PathBuf),
}

/// Newest-first recent list: dedups and caps at 8 entries.
fn push_recent(list: &mut Vec<String>, path: String) {
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(8);
}

impl App {
    pub fn push_recent_dbc(&mut self, path: String) {
        push_recent(&mut self.recent_dbc, path);
    }

    pub fn push_recent_log(&mut self, path: String) {
        push_recent(&mut self.recent_log, path);
    }

    /// Rebuilds the whole workspace back to factory defaults, keeping only
    /// the machine-local recents and the captured default layout.
    pub fn reset_to_defaults(&mut self) {
        self.stop();
        let recent_dbc = std::mem::take(&mut self.recent_dbc);
        let recent_log = std::mem::take(&mut self.recent_log);
        let recent_projects = std::mem::take(&mut self.recent_projects);
        let default_layout = std::mem::take(&mut self.default_layout);
        *self = App::new();
        self.recent_dbc = recent_dbc;
        self.recent_log = recent_log;
        self.recent_projects = recent_projects;
        self.default_layout = default_layout;
    }

    /// Display name of the current project: the file stem, or "Untitled".
    pub fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Project name with a `*` suffix when there are unsaved changes.
    pub fn display_name(&self) -> String {
        let name = self.project_name();
        if self.is_dirty() {
            format!("{name} *")
        } else {
            name
        }
    }

    pub fn push_recent_project(&mut self, path: String) {
        push_recent(&mut self.recent_projects, path);
    }

    /// Serializable snapshot of the workspace configuration, used to tell
    /// whether anything changed since the last load/save/reset.
    pub(crate) fn config_snapshot(&self) -> String {
        serde_json::to_string(&Config::from_app(self, None)).unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.config_snapshot() != self.baseline
    }

    /// Marks the current workspace as the clean baseline (after load/save).
    pub fn mark_clean(&mut self) {
        self.baseline = self.config_snapshot();
    }

    pub fn run_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Quit => self.quit = true,
            PendingAction::NewProject => self.new_project(),
            PendingAction::OpenProject => self.open_project_dialog(),
            PendingAction::OpenPath(p) => self.open_project_path(&p),
        }
    }

    /// Saved projects auto-save and proceed; untouched untitled workspaces
    /// have nothing to lose and proceed directly; only a modified untitled
    /// workspace is routed through the confirmation modal.
    pub fn guarded_action(&mut self, action: PendingAction) {
        if let Some(p) = self.project_path.clone() {
            self.save_project(Some(p));
            self.run_action(action);
        } else if self.is_dirty() {
            self.pending_action = Some(action);
        } else {
            self.run_action(action);
        }
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
            version: 1,
            layout: self.layout_cache.clone(),
            project: None,
            config: Config::from_app(self, base.as_deref()),
        };
        let written = serde_json::to_string_pretty(&proj)
            .map_err(|e| e.to_string())
            .and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                self.push_recent_project(path.to_string_lossy().to_string());
                self.project_path = Some(path.clone());
                self.baseline = self.config_snapshot();
                self.status = format!("project saved: {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("project save failed: {e}");
                false
            }
        }
    }

    /// Loads a .rxproj file, replacing the current workspace.
    pub fn open_project_path(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("project read failed: {e}");
                return;
            }
        };
        match serde_json::from_str::<ProjectFile>(&text) {
            Ok(proj) => {
                self.reset_to_defaults();
                let mut cfg = proj.config;
                cfg.resolve_paths(path.parent());
                cfg.apply(self);
                self.project_path = Some(path.to_path_buf());
                if !proj.layout.is_empty() {
                    self.pending_layout = Some(proj.layout);
                }
                self.baseline = self.config_snapshot();
                self.push_recent_project(path.to_string_lossy().to_string());
                self.status = format!("project loaded: {}", path.display());
            }
            Err(e) => self.status = format!("project ignored: {e}"),
        }
    }

    pub fn open_project_dialog(&mut self) {
        let pick = rfd::FileDialog::new()
            .add_filter("roxy-can project", &[PROJECT_EXT])
            .pick_file();
        if let Some(p) = pick {
            self.open_project_path(&p);
        }
    }

    /// Periodic crash cache; the real project file is never touched.
    pub fn write_autosave(&self) {
        let base = self
            .project_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.to_path_buf());
        let proj = ProjectFile {
            version: 1,
            layout: self.layout_cache.clone(),
            project: self
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            config: Config::from_app(self, base.as_deref()),
        };
        if let Ok(json) = serde_json::to_string_pretty(&proj) {
            let _ = std::fs::write(AUTOSAVE_PATH, json);
        }
    }

    /// Restores the crash cache left behind by an abnormal exit.
    pub fn load_autosave(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(AUTOSAVE_PATH) else {
            return false;
        };
        let Ok(proj) = serde_json::from_str::<ProjectFile>(&text) else {
            return false;
        };
        self.reset_to_defaults();
        let base = proj
            .project
            .as_deref()
            .map(Path::new)
            .and_then(|p| p.parent());
        let mut cfg = proj.config;
        cfg.resolve_paths(base);
        cfg.apply(self);
        self.project_path = proj.project.map(PathBuf::from);
        if !proj.layout.is_empty() {
            self.pending_layout = Some(proj.layout);
        }
        self.mark_clean();
        self.status = "restored autosave".to_string();
        true
    }

    /// Starts a fresh, completely empty untitled workspace: no DBCs, no
    /// observer windows, no generator entries, default layout.
    pub fn new_project(&mut self) {
        self.reset_to_defaults();
        for c in &mut self.channels {
            c.dbc_path.clear();
            c.dbc = None;
        }
        self.trace_windows.clear();
        self.msg_windows.clear();
        self.stats_windows.clear();
        self.graphics.clear();
        self.data_windows.clear();
        self.tx_list.clear();
        self.subs.clear();
        self.baseline = self.config_snapshot();
        if !self.default_layout.is_empty() {
            self.pending_layout = Some(self.default_layout.clone());
        }
        self.status = "new project".to_string();
    }

    /// Saved projects quit silently (auto-save on exit); untitled ones
    /// only go through the confirmation modal when actually modified.
    pub fn request_quit(&mut self) {
        self.guarded_action(PendingAction::Quit);
    }

    /// Writes the startup driver (last project + recent projects).
    pub fn write_meta(&self) {
        let meta = Meta {
            last_project: self
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            recent_projects: self.recent_projects.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&meta) {
            let _ = std::fs::write(META_PATH, json);
        }
    }

    /// Startup restore: an autosave left by a crash comes first, then the
    /// last project when one is known, then the legacy `roxy-can.json`
    /// import on a very first launch, else defaults.
    pub fn startup_workspace(&mut self) {
        if Path::new(AUTOSAVE_PATH).exists() {
            if self.load_autosave() {
                return;
            }
            let _ = std::fs::remove_file(AUTOSAVE_PATH);
        }
        if let Ok(text) = std::fs::read_to_string(META_PATH)
            && let Ok(meta) = serde_json::from_str::<Meta>(&text)
        {
            self.recent_projects = meta.recent_projects;
            if let Some(last) = meta.last_project {
                let path = PathBuf::from(last);
                if path.exists() {
                    self.open_project_path(&path);
                } else {
                    self.status = "last project missing, started empty".to_string();
                }
            }
            return;
        }
        self.load_config();
    }
}
