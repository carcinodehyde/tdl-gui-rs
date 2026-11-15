// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod runner;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui;

use runner::{Event, Job, LineKind};

const APP_NAME: &str = "tdl-gui-rs";
const MAX_LOG_LINES: usize = 1000;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([760.0, 640.0])
            .with_min_inner_size([480.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

struct App {
    urls_text: String,
    target_dir: String,
    /// Running download, if any.
    job: Option<Job>,
    events: Option<Receiver<Event>>,
    /// Number of URLs handed to tdl for the current/last run.
    total: usize,
    /// Per-file progress (0.0..=1.0), keyed by tdl's tracker label.
    files: HashMap<String, f32>,
    failed: usize,
    progress: f32,
    status: String,
    log: Vec<String>,
    /// Whether a run has been started at least once (keeps the bar visible).
    has_run: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            urls_text: String::new(),
            target_dir: default_download_dir(),
            job: None,
            events: None,
            total: 0,
            files: HashMap::new(),
            failed: 0,
            progress: 0.0,
            status: String::new(),
            log: Vec::new(),
            has_run: false,
        }
    }
}

fn default_download_dir() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// One URL per line; blank lines and `#` comments are ignored, duplicates dropped.
fn parse_urls(text: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !urls.iter().any(|u| u == line) {
            urls.push(line.to_owned());
        }
    }
    urls
}

impl App {
    fn downloading(&self) -> bool {
        self.job.is_some()
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > MAX_LOG_LINES {
            let excess = self.log.len() - MAX_LOG_LINES;
            self.log.drain(..excess);
        }
    }

    fn import_txt(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Text files", &["txt"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                self.urls_text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
                self.status = format!("Imported {}", path.display());
            }
            Err(e) => self.status = format!("Could not read {}: {e}", path.display()),
        }
    }

    fn start_download(&mut self, ctx: &egui::Context) {
        let urls = parse_urls(&self.urls_text);
        let dir = self.target_dir.trim().to_owned();
        if urls.is_empty() || dir.is_empty() {
            return;
        }

        self.total = urls.len();
        self.files.clear();
        self.failed = 0;
        self.progress = 0.0;
        self.log.clear();
        self.has_run = true;

        match runner::spawn(&urls, &dir, ctx.clone()) {
            Ok((job, rx)) => {
                self.status = format!("Starting download of {} URL(s)…", self.total);
                self.job = Some(job);
                self.events = Some(rx);
            }
            Err(e) => {
                self.status = format!("Failed to start tdl: {e}");
                self.push_log(self.status.clone());
            }
        }
    }

    fn poll_events(&mut self) {
        let Some(rx) = self.events.take() else { return };
        let mut finished = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Line(line) => self.handle_line(line),
                Event::Exited { success, code } => {
                    finished = true;
                    self.on_exit(success, code);
                }
            }
        }
        if !finished {
            self.events = Some(rx);
        }
    }

    fn handle_line(&mut self, line: String) {
        match runner::classify(&line) {
            LineKind::Progress { key, fraction } => {
                self.files.insert(key, fraction);
            }
            LineKind::Done { key } => {
                if self.files.insert(key, 1.0) != Some(1.0) {
                    self.push_log(line);
                }
            }
            LineKind::Failed { key } => {
                if self.files.insert(key, 1.0) != Some(1.0) {
                    self.failed += 1;
                    self.push_log(line);
                }
            }
            LineKind::Other => {
                self.push_log(line);
                return;
            }
        }

        let total = self.total.max(self.files.len()).max(1);
        let sum: f32 = self.files.values().sum();
        self.progress = (sum / total as f32).clamp(0.0, 1.0);
        let done = self.files.values().filter(|f| **f >= 1.0).count();
        self.status = format!("Downloading… {done}/{total} file(s) finished");
    }

    fn on_exit(&mut self, success: bool, code: Option<i32>) {
        let cancelled = self.job.take().is_some_and(|job| job.cancelled());
        if cancelled {
            self.status = "Download cancelled.".to_owned();
        } else if success && self.failed == 0 {
            self.progress = 1.0;
            self.status = "Download complete.".to_owned();
        } else if success {
            self.status = format!("Finished with {} failed file(s) — see log.", self.failed);
        } else {
            let code = code.map_or_else(|| "unknown".to_owned(), |c| c.to_string());
            self.status = format!("tdl exited with an error (code {code}) — see log.");
        }
        self.push_log(self.status.clone());
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        let downloading = self.downloading();

        egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
            ui.add_space(6.0);

            // Target folder selector.
            ui.horizontal(|ui| {
                ui.label("Download folder:");
                ui.add_enabled_ui(!downloading, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Browse…").clicked() {
                            let mut dialog = rfd::FileDialog::new();
                            if PathBuf::from(self.target_dir.trim()).is_dir() {
                                dialog = dialog.set_directory(self.target_dir.trim());
                            }
                            if let Some(dir) = dialog.pick_folder() {
                                self.target_dir = dir.display().to_string();
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.target_dir)
                                .hint_text("Choose where files are saved")
                                .desired_width(f32::INFINITY),
                        );
                    });
                });
            });

            ui.add_space(6.0);

            // Download / cancel.
            let url_count = parse_urls(&self.urls_text).len();
            let can_start = !downloading && url_count > 0 && !self.target_dir.trim().is_empty();
            ui.horizontal(|ui| {
                let label = if downloading {
                    "Downloading…".to_owned()
                } else if url_count > 0 {
                    format!("Download ({url_count})")
                } else {
                    "Download".to_owned()
                };
                let button = egui::Button::new(egui::RichText::new(label).size(16.0))
                    .min_size(egui::vec2(160.0, 32.0));
                if ui.add_enabled(can_start, button).clicked() {
                    self.start_download(ctx);
                }
                if downloading {
                    let cancel = egui::Button::new("Cancel").min_size(egui::vec2(80.0, 32.0));
                    if ui.add(cancel).clicked() {
                        if let Some(job) = &self.job {
                            job.cancel();
                            self.status = "Cancelling…".to_owned();
                        }
                    }
                }
            });

            if self.has_run {
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(self.progress)
                        .show_percentage()
                        .animate(downloading),
                );
            }

            if !self.status.is_empty() {
                ui.add_space(4.0);
                ui.label(&self.status);
            }

            if !self.log.is_empty() {
                egui::CollapsingHeader::new("tdl output")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("log")
                            .max_height(140.0)
                            .auto_shrink([false, true])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in &self.log {
                                    ui.label(egui::RichText::new(line).monospace().size(11.0));
                                }
                            });
                    });
            }
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Telegram message links (one per line):");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!downloading, egui::Button::new("Import TXT…"))
                        .clicked()
                    {
                        self.import_txt();
                    }
                    let can_clear = !downloading && !self.urls_text.is_empty();
                    if ui
                        .add_enabled(can_clear, egui::Button::new("Clear"))
                        .clicked()
                    {
                        self.urls_text.clear();
                    }
                });
            });
            ui.add_space(4.0);

            let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
            let rows = ((ui.available_height() / row_height) as usize)
                .saturating_sub(1)
                .max(4);
            egui::ScrollArea::vertical()
                .id_salt("urls")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_enabled(
                        !downloading,
                        egui::TextEdit::multiline(&mut self.urls_text)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("https://t.me/channel/123\nhttps://t.me/c/1234567890/456")
                            .desired_width(f32::INFINITY)
                            .desired_rows(rows),
                    );
                });
        });
    }
}
