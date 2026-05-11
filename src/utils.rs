use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn find_home_file(rel_path: &str) -> Option<PathBuf> {
    let home = home_dir()?;
    let path = home.join(rel_path);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn read_home_file(rel_path: &str) -> Option<String> {
    let path = find_home_file(rel_path)?;
    std::fs::read_to_string(&path).ok()
}

pub fn progress_bar(percent: i32, width: usize) -> String {
    let filled = (width as f64 * (percent as f64 / 100.0)) as usize;
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}