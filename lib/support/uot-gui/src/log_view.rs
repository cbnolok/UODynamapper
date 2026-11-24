use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, TextStyle, Ui};

/// Offer a UTF-8 text export of the supplied log lines.
pub fn export_log_button<'a>(
    ui: &mut Ui,
    tool_name: &str,
    lines: impl IntoIterator<Item = &'a str>,
) {
    let error_id = ui.make_persistent_id(("log_export_error", tool_name));
    let mut lines = lines.into_iter().peekable();
    if ui.add_enabled(lines.peek().is_some(), egui::Button::new("Export log…")).clicked() {
        let result = save_log(tool_name, lines);
        ui.data_mut(|data| {
            data.remove::<String>(error_id);
            if let Err(error) = result {
                data.insert_temp(error_id, format!("Could not export log: {error}"));
            }
        });
    }
    if let Some(error) = ui.data(|data| data.get_temp::<String>(error_id)) {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}

fn save_log<'a>(tool_name: &str, lines: impl Iterator<Item = &'a str>) -> io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    let file_name = format!("{tool_name}-{timestamp}.txt");
    let Some(path) = crate::file_dialog()
        .add_filter("Text file", &["txt"])
        .set_file_name(file_name)
        .save_file()
    else {
        return Ok(());
    };

    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&path, text)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))
}

/// Show read-only log text with selection across messages, copying, and scrolling.
pub fn show_log_text<'a>(
    ui: &mut Ui,
    id_salt: &str,
    lines: impl IntoIterator<Item = (&'a str, Option<Color32>)>,
    font: TextStyle,
) {
    let font = font.resolve(ui.style());
    let default_color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    for (line, color) in lines {
        let format = egui::TextFormat::simple(font.clone(), color.unwrap_or(default_color));
        if !job.sections.is_empty() {
            job.append("\n", 0.0, format.clone());
        }
        job.append(line, 0.0, format);
    }
    let mut text = job.text.as_str();

    egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let mut layouter = |ui: &Ui, _: &dyn egui::TextBuffer, wrap_width| {
                let mut job = job.clone();
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            let response = ui.add(egui::TextEdit::multiline(&mut text)
                .id_salt(id_salt)
                .frame(egui::Frame::NONE)
                .font(font)
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .layouter(&mut layouter));

            response.context_menu(|ui| {
                if ui.add_enabled(!text.is_empty(), egui::Button::new("Copy all")).clicked() {
                    ui.ctx().copy_text(text.to_owned());
                    ui.close();
                }
            });
        });
}
