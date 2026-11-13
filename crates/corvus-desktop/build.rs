//! The page may call only the commands listed in `src/commands.rs`; capabilities/default.json grants exactly those.
#[path = "src/commands.rs"]
mod commands;

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands::APP_COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
