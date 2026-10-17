//! Opening files and folders with the system's apps.

use std::path::Path;

/// Shows `path` in the file manager (selected on Windows; its folder elsewhere).
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(not(windows))]
    open(path.parent().unwrap_or(path));
}

/// Opens a file with its default app.
pub fn open(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Opens a web link in the default browser. Only http(s) and mailto links are opened.
pub fn open_url(url: &str) {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")) {
        return;
    }
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(url.trim()).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url.trim()).spawn();
}
