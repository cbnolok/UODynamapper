use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

use eframe::egui;
use crossbeam_channel::{Sender, Receiver, unbounded};
use crate::core::{PopulatorConfig, TaskProgress, UopDictionary, PopulatorTask};

enum Message {
    Log(String),
    Progress(f32, String),
    Finished(RunSummary),
}

struct RunSummary {
    found: HashMap<u64, String>,
    processed: usize,
    skipped: usize,
    errors: usize,
    stopped: bool,
}

pub struct UopPopulatorApp {
    uop_dir: Option<PathBuf>,
    dictionary_path: Option<PathBuf>,
    config_text: String,
    
    // Runtime state
    is_running: bool,
    stop_signal: Arc<AtomicBool>,
    log: Vec<String>,
    progress: f32,
    current_uop: String,
    
    // Communication
    tx: Sender<Message>,
    rx: Receiver<Message>,
    
    // Results
    dictionary: UopDictionary,
}

impl Default for UopPopulatorApp {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            uop_dir: None,
            dictionary_path: None,
            config_text: r#"[Texture.uop]
candidates = ["build/worldart/{:08}.dds"]
range = [0, 10000]

[LegacyTexture.uop]
candidates = ["build/tileartlegacy/{:08}.dds"]
range = [0, 10000]
"#.to_string(),
            is_running: false,
            stop_signal: Arc::new(AtomicBool::new(false)),
            log: Vec::new(),
            progress: 0.0,
            current_uop: String::new(),
            tx,
            rx,
            dictionary: UopDictionary::default(),
        }
    }
}

impl UopPopulatorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn add_log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
        if self.log.len() > 1000 {
            self.log.remove(0);
        }
    }

    fn start_cracking(&mut self, config: PopulatorConfig) {
        let uop_dir = self.uop_dir.clone().unwrap();
        let stop_signal = self.stop_signal.clone();
        let dictionary = Arc::new(self.dictionary.clone());
        let tx = self.tx.clone();

        std::thread::spawn(move || {
            let mut total_found = HashMap::new();
            let mut summary = RunSummary {
                found: HashMap::new(),
                processed: 0,
                skipped: 0,
                errors: 0,
                stopped: false,
            };
            let packages_count = config.packages.len();
            
            for (i, (uop_name, pkg_config)) in config.packages.into_iter().enumerate() {
                if stop_signal.load(Ordering::Relaxed) {
                    summary.stopped = true;
                    let _ = tx.send(Message::Log("Stop requested; ending after current package boundary.".to_string()));
                    break;
                }

                let uop_path = uop_dir.join(&uop_name);
                if !uop_path.exists() {
                    summary.skipped += 1;
                    let _ = tx.send(Message::Log(format!("Warning: {} not found; skipped.", uop_path.display())));
                    continue;
                }

                let progress = i as f32 / packages_count as f32;
                let _ = tx.send(Message::Progress(progress, uop_name.clone()));
                let _ = tx.send(Message::Log(format!("Processing {}...", uop_name)));

                let task = PopulatorTask {
                    uop_path,
                    config: pkg_config,
                    dictionary: dictionary.clone(),
                    stop_signal: stop_signal.clone(),
                };

                let task_tx = tx.clone();
                match task.run_with_progress(|event| match event {
                    TaskProgress::MissingHashes(count) => {
                        let _ = task_tx.send(Message::Log(format!("  Missing hashes: {}", count)));
                    }
                    TaskProgress::TryingTemplate(template) => {
                        let _ = task_tx.send(Message::Log(format!("  Trying template: {}", template)));
                    }
                    TaskProgress::TemplateMatches { template, found } => {
                        if found > 0 {
                            let _ = task_tx.send(Message::Log(format!("    Found {} matches with {}", found, template)));
                        }
                    }
                    TaskProgress::Stopped => {
                        let _ = task_tx.send(Message::Log("  Stop requested; package scan interrupted.".to_string()));
                    }
                }) {
                    Ok(found) => {
                        summary.processed += 1;
                        let _ = tx.send(Message::Log(format!("  Found {} new strings in {}", found.len(), uop_name)));
                        total_found.extend(found);
                    }
                    Err(e) => {
                        summary.errors += 1;
                        let _ = tx.send(Message::Log(format!("  Error processing {}: {}", uop_name, e)));
                    }
                }
            }

            summary.found = total_found;
            let _ = tx.send(Message::Progress(1.0, String::new()));
            let _ = tx.send(Message::Finished(summary));
        });
    }
}

impl eframe::App for UopPopulatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle messages
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Message::Log(m) => self.add_log(m),
                Message::Progress(p, name) => {
                    self.progress = p;
                    self.current_uop = name;
                }
                Message::Finished(summary) => {
                    self.is_running = false;
                    let status = if summary.stopped { "Stopped" } else { "Finished" };
                    self.add_log(format!(
                        "{}. Processed {}, skipped {}, errors {}, found {} total new strings.",
                        status,
                        summary.processed,
                        summary.skipped,
                        summary.errors,
                        summary.found.len(),
                    ));
                    for (hash, name) in summary.found {
                        self.dictionary.set(hash, name);
                    }
                    if let Some(path) = &self.dictionary_path {
                        if let Err(e) = self.dictionary.save(path) {
                            self.add_log(format!("Error saving dictionary: {}", e));
                        } else {
                            self.add_log("Dictionary saved successfully.");
                        }
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("UOP Dictionary Populator");
                ui.add_space(8.0);
            });

            ui.group(|ui| {
                ui.label("Paths");
                ui.horizontal(|ui| {
                    ui.label("UOP Directory:");
                    let text = self.uop_dir.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "Select Directory...".to_string());
                    if ui.button(text).clicked() {
                        if let Some(path) = crate::dialog::file_dialog().pick_folder() {
                            self.uop_dir = Some(path);
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Dictionary File:");
                    let text = self.dictionary_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "Select/Create File...".to_string());
                    if ui.button(text).clicked() {
                        if let Some(path) = crate::dialog::file_dialog().add_filter("DIC Dictionary", &["dic"]).save_file() {
                            self.dictionary_path = Some(path);
                            if self.dictionary_path.as_ref().unwrap().exists() {
                                match UopDictionary::load(self.dictionary_path.as_ref().unwrap()) {
                                    Ok(dict) => {
                                        self.dictionary = dict;
                                        self.add_log(format!(
                                            "Loaded dictionary with {} entries ({} named).",
                                            self.dictionary.len(),
                                            self.dictionary.named_len(),
                                        ));
                                    }
                                    Err(e) => self.add_log(format!("Failed to load dictionary: {}", e)),
                                }
                            }
                        }
                    }
                });
            });

            ui.add_space(8.0);

            ui.group(|ui| {
                ui.label("Configuration (TOML)");
                let editor_height = ui.available_height() * 0.4;
                egui::ScrollArea::vertical().max_height(editor_height).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.config_text)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY));
                });
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.is_running, |ui| {
                    if ui.button("Start Cracking").clicked() {
                        if self.uop_dir.is_none() {
                            self.add_log("Error: No UOP directory selected.");
                        } else if self.dictionary_path.is_none() {
                            self.add_log("Error: No dictionary output file selected.");
                        } else {
                            match toml::from_str::<PopulatorConfig>(&self.config_text) {
                                Ok(config) => {
                                    if let Err(e) = config.validate() {
                                        self.add_log(format!("Config Error: {}", e));
                                    } else {
                                        self.is_running = true;
                                        self.stop_signal.store(false, Ordering::Relaxed);
                                        self.progress = 0.0;
                                        self.current_uop.clear();
                                        self.add_log("Starting dictionary population.");
                                        self.start_cracking(config);
                                    }
                                }
                                Err(e) => self.add_log(format!("Config Error: {}", e)),
                            }
                        }
                    }
                });
                
                ui.add_enabled_ui(self.is_running, |ui| {
                    if ui.button("Stop").clicked() {
                        self.stop_signal.store(true, Ordering::Relaxed);
                        self.add_log("Stop requested; waiting for the active template or package to observe it.");
                    }
                });
            });

            if self.is_running {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(format!("Processing: {}", self.current_uop));
                    ui.add(egui::ProgressBar::new(self.progress).show_percentage().animate(true));
                });
            }

            ui.add_space(8.0);

            ui.group(|ui| {
                ui.label("Log");
                let log_height = ui.available_height();
                egui::ScrollArea::vertical().max_height(log_height).stick_to_bottom(true).show(ui, |ui| {
                    for line in &self.log {
                        ui.label(line);
                    }
                });
            });
        });

        if self.is_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
