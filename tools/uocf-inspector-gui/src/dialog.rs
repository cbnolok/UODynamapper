pub fn file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
}

pub fn pick_folder() -> Option<std::path::PathBuf> {
    file_dialog().pick_folder()
}

#[cfg(target_os = "linux")]
pub fn normalize_linux_portal_env_before_threads() {
    use std::env;

    let current = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !current.is_empty() {
        return;
    }

    let session = env::var("XDG_SESSION_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let desktop_session = env::var("DESKTOP_SESSION")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kde_full = env::var("KDE_FULL_SESSION").unwrap_or_default();

    let inferred = if kde_full.eq_ignore_ascii_case("true")
        || session.contains("kde")
        || session.contains("plasma")
        || desktop_session.contains("kde")
        || desktop_session.contains("plasma")
    {
        Some("KDE")
    } else if session.contains("gnome") || desktop_session.contains("gnome") {
        Some("GNOME")
    } else {
        None
    };

    if let Some(desktop) = inferred {
        unsafe { env::set_var("XDG_CURRENT_DESKTOP", desktop) };
    }
}

#[cfg(not(target_os = "linux"))]
pub fn normalize_linux_portal_env_before_threads() {}
