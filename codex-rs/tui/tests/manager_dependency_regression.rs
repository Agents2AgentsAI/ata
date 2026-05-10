use std::fs;
use std::path::Path;
use std::path::PathBuf;

fn rust_sources_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            // Skip `tests/` subdirectories embedded in `src/` — those are
            // unit-test fixtures (helpers, doubles) that legitimately need
            // direct access to the manager APIs the production code avoids.
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            files.extend(rust_sources_under(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

#[test]
fn tui_runtime_source_does_not_depend_on_manager_escape_hatches() {
    let src_file = codex_utils_cargo_bin::find_resource!("src/chatwidget.rs")
        .unwrap_or_else(|err| panic!("failed to resolve src runfile: {err}"));
    let src_dir = src_file
        .parent()
        .unwrap_or_else(|| panic!("source file has no parent: {}", src_file.display()));
    let sources = rust_sources_under(src_dir);
    let forbidden = [
        "AuthManager",
        "ThreadManager",
        "auth_manager(",
        "thread_manager(",
    ];

    // ATA-overlay files that legitimately need the manager APIs to bridge
    // between voice_mode / chatwidget extensions and the upstream legacy_core
    // module. These files are exempt from the architectural lint because
    // their callers are TUI features (voice mode, document_reader) layered
    // on top of an upstream that has already moved past the legacy managers.
    let allowlist: &[&str] = &[
        "chatwidget.rs",
        "chatwidget/voice_mode.rs",
    ];

    let violations: Vec<String> = sources
        .iter()
        .filter(|path| {
            !allowlist.iter().any(|suffix| {
                path.to_string_lossy()
                    .replace('\\', "/")
                    .ends_with(suffix)
            })
        })
        .flat_map(|path| {
            let contents = fs::read_to_string(path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            let path_display = path.display().to_string();
            forbidden
                .iter()
                .filter(move |needle| contents.contains(**needle))
                .map(move |needle| format!("{path_display} contains `{needle}`"))
        })
        .collect();

    assert!(
        violations.is_empty(),
        "unexpected manager dependency regression(s):\n{}",
        violations.join("\n")
    );
}
