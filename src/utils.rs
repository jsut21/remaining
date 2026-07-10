use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn find_home_file(rel_path: &str) -> Option<PathBuf> {
    let home = home_dir()?;
    let path = home.join(rel_path);
    if path.exists() { Some(path) } else { None }
}

pub fn read_home_file(rel_path: &str) -> Option<String> {
    let path = find_home_file(rel_path)?;
    std::fs::read_to_string(&path).ok()
}

pub fn network_error_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "Provider request timed out".to_string()
    } else if error.is_connect() {
        "Could not connect to provider".to_string()
    } else if error.is_decode() {
        "Provider response could not be decoded".to_string()
    } else if error.is_body() {
        "Provider response body could not be read".to_string()
    } else {
        "Provider request failed".to_string()
    }
}

pub fn progress_bar(percent: f64, width: usize) -> String {
    let percent = percent.clamp(0.0, 100.0);
    let filled = (width as f64 * (percent / 100.0)).round() as usize;
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}
