use eframe::egui::{self, Color32, TextStyle, Ui};

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
