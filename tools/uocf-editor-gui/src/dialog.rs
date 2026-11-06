pub fn file_dialog() -> rfd::FileDialog {
    uot_gui::file_dialog()
}

pub fn pick_folder() -> Option<std::path::PathBuf> {
    uot_gui::pick_folder()
}

pub fn message_dialog() -> rfd::MessageDialog {
    rfd::MessageDialog::new()
}
