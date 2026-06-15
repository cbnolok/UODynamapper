pub fn file_dialog() -> rfd::FileDialog {
    gui_shared::file_dialog()
}

pub fn pick_folder() -> Option<std::path::PathBuf> {
    gui_shared::pick_folder()
}
