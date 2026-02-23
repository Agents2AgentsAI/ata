use std::fs;
use std::path::Path;

fn main() {
    for dir_path in ["src/skills/assets/samples", "src/skills/assets/research"] {
        let dir = Path::new(dir_path);
        if dir.exists() {
            println!("cargo:rerun-if-changed={}", dir.display());
            visit_dir(dir);
        }
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
