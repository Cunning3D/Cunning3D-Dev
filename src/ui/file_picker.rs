use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy::tasks::{IoTaskPool, Task};
use futures_lite::future;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilePickerMode {
    OpenProject,
    SaveProject,
    SaveProjectAs,
}

impl Default for FilePickerMode {
    fn default() -> Self {
        Self::OpenProject
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub struct OpenFilePickerEvent {
    pub mode: FilePickerMode,
}

#[derive(Message, Clone, Debug)]
pub struct FilePickerChosenEvent {
    pub mode: FilePickerMode,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
struct EntryInfo {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Resource, Default)]
pub struct FilePickerState {
    open: bool,
    mode: FilePickerMode,
    current_dir: PathBuf,
    filter: String,
    file_name: String,
    selected: Option<PathBuf>,
    entries: Vec<EntryInfo>,
    loading: bool,
    error: Option<String>,
    recent: Vec<PathBuf>,
    list_task: Option<Task<Result<Vec<EntryInfo>, String>>>,
}

impl FilePickerState {
    fn title(&self) -> &'static str {
        match self.mode {
            FilePickerMode::OpenProject => "Open Project",
            FilePickerMode::SaveProject => "Save Project",
            FilePickerMode::SaveProjectAs => "Save As",
        }
    }
    fn confirm_label(&self) -> &'static str {
        match self.mode {
            FilePickerMode::OpenProject => "Open",
            FilePickerMode::SaveProject | FilePickerMode::SaveProjectAs => "Save",
        }
    }
}

fn list_dir(path: &Path, filter: &str) -> Result<Vec<EntryInfo>, String> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(path).map_err(|e| format!("read_dir failed: {e}"))?;
    for it in rd {
        let it = it.map_err(|e| format!("read_dir entry failed: {e}"))?;
        let p = it.path();
        let name = it
            .file_name()
            .to_string_lossy()
            .to_string();
        let md = it.metadata().ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        if !filter.is_empty() && !name.to_lowercase().contains(&filter.to_lowercase()) {
            continue;
        }
        // Only show .c3d files in Open mode (directories always visible)
        if !is_dir {
            if p.extension().and_then(|s| s.to_str()).unwrap_or("") != "c3d" {
                continue;
            }
        }
        out.push(EntryInfo {
            name,
            path: p,
            is_dir,
        });
    }
    // dirs first, then name
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

fn spawn_list_task(dir: PathBuf, filter: String) -> Task<Result<Vec<EntryInfo>, String>> {
    IoTaskPool::get().spawn(async move { list_dir(&dir, &filter) })
}

pub fn file_picker_ui_system(
    mut egui_contexts: EguiContexts,
    mut st: ResMut<FilePickerState>,
    mut open_rx: MessageReader<OpenFilePickerEvent>,
    mut chosen_tx: MessageWriter<FilePickerChosenEvent>,
) {
    // Handle open requests
    for ev in open_rx.read() {
        st.open = true;
        st.mode = ev.mode;
        st.filter.clear();
        st.error = None;
        st.selected = None;
        st.loading = true;
        st.current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        st.file_name = "project.c3d".to_string();
        st.list_task = Some(spawn_list_task(st.current_dir.clone(), st.filter.clone()));
    }

    // Poll directory listing task
    if let Some(t) = &mut st.list_task {
        if let Some(res) = future::block_on(future::poll_once(t)) {
            st.list_task = None;
            match res {
                Ok(v) => {
                    st.entries = v;
                    st.loading = false;
                }
                Err(e) => {
                    st.entries.clear();
                    st.loading = false;
                    st.error = Some(e);
                }
            }
        }
    }

    if !st.open {
        return;
    }

    let Some(ctx) = egui_contexts.try_ctx_mut() else {
        return;
    };
    // While open, keep UI responsive without relying on external invalidators.
    ctx.request_repaint();

    let mut should_close = false;
    let mut confirmed: Option<PathBuf> = None;
    egui::Window::new(st.title())
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(720.0, 520.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Directory");
                ui.monospace(st.current_dir.display().to_string());
                if ui.button("Parent").clicked() {
                    if let Some(p) = st.current_dir.parent() {
                        st.current_dir = p.to_path_buf();
                        st.loading = true;
                        st.list_task =
                            Some(spawn_list_task(st.current_dir.clone(), st.filter.clone()));
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Search");
                let changed = ui.text_edit_singleline(&mut st.filter).changed();
                if changed {
                    st.loading = true;
                    st.list_task = Some(spawn_list_task(
                        st.current_dir.clone(),
                        st.filter.clone(),
                    ));
                }
                if ui.button("Refresh").clicked() {
                    st.loading = true;
                    st.list_task =
                        Some(spawn_list_task(st.current_dir.clone(), st.filter.clone()));
                }
            });

            if let Some(err) = st.error.as_ref() {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            if st.loading {
                ui.label("Reading directory...");
            }

            ui.separator();

            ui.columns(2, |cols| {
                // Left: recents
                cols[0].vertical(|ui| {
                    ui.strong("Recent");
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_source("file_picker_recent")
                        .max_height(260.0)
                        .show(ui, |ui| {
                            if st.recent.is_empty() {
                                ui.weak("None");
                            }
                            let recent: Vec<PathBuf> = st.recent.iter().take(20).cloned().collect();
                            for p in recent {
                                let txt = p.display().to_string();
                                if ui.selectable_label(false, txt).clicked() {
                                    if p.is_dir() {
                                        st.current_dir = p;
                                        st.loading = true;
                                        st.list_task = Some(spawn_list_task(
                                            st.current_dir.clone(),
                                            st.filter.clone(),
                                        ));
                                    } else {
                                        st.selected = Some(p);
                                    }
                                }
                            }
                        });
                });

                // Right: directory listing
                cols[1].vertical(|ui| {
                    ui.strong("Files");
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_source("file_picker_files")
                        .show(ui, |ui| {
                        let entries: Vec<(String, PathBuf, bool)> = st
                            .entries
                            .iter()
                            .map(|e| (e.name.clone(), e.path.clone(), e.is_dir))
                            .collect();
                        for (name, path, is_dir) in entries {
                            let sel = st.selected.as_ref() == Some(&path);
                            let label = if is_dir {
                                format!("📁 {}", name)
                            } else {
                                name.clone()
                            };
                            if ui.selectable_label(sel, label).clicked() {
                                if is_dir {
                                    st.current_dir = path;
                                    st.loading = true;
                                    st.list_task = Some(spawn_list_task(
                                        st.current_dir.clone(),
                                        st.filter.clone(),
                                    ));
                                    st.selected = None;
                                } else {
                                    st.selected = Some(path);
                                    st.file_name = name;
                                }
                            }
                        }
                    });
                });
            });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Filename");
                ui.text_edit_singleline(&mut st.file_name);
            });

            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    should_close = true;
                }
                ui.add_space(12.0);
                let ok_enabled = match st.mode {
                    FilePickerMode::OpenProject => st.selected.is_some(),
                    FilePickerMode::SaveProject | FilePickerMode::SaveProjectAs => {
                        !st.file_name.trim().is_empty()
                    }
                };
                if ui
                    .add_enabled(ok_enabled, egui::Button::new(st.confirm_label()))
                    .clicked()
                {
                    let path = match st.mode {
                        FilePickerMode::OpenProject => st.selected.clone().unwrap(),
                        FilePickerMode::SaveProject | FilePickerMode::SaveProjectAs => {
                            st.current_dir.join(st.file_name.trim())
                        }
                    };
                    confirmed = Some(path);
                    should_close = true;
                }
            });
        });

    if let Some(p) = confirmed {
        // Update recents (in-memory). Persistence is handled by settings/jobs later.
        st.recent.retain(|x| x != &p);
        st.recent.insert(0, p.clone());
        chosen_tx.write(FilePickerChosenEvent { mode: st.mode, path: p });
    }

    if should_close {
        st.open = false;
        st.list_task = None;
    }
}

