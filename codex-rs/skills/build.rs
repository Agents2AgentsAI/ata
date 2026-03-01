use std::fs;
use std::path::Path;

fn main() {
    for dir_name in ["src/assets/samples", "src/assets/research", "src/assets/workspace"] {
        let dir = Path::new(dir_name);
        if !dir.exists() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", dir.display());
        visit_dir(dir);
    }
}

fn visit_dir(dir: &Path) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            visit_dir(&path);
        }
    }
}
