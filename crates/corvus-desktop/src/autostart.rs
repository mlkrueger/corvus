//! Launch at login ([system/05](../../../spec/system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2),
//! defaults row 68): the background leader should be running after a reboot without the owner doing anything.
//!
//! The app turns the login item on once, at its first launch from an installed bundle, and writes a marker so a
//! later choice — the Connect view's checkbox or macOS's own Login Items settings — is never overridden. The
//! autostart plugin runs here, in Rust: the page reaches it only through `get_launch_at_login` and
//! `set_launch_at_login` and holds no plugin permission.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

/// The argument the login item starts the app with: open into the menu bar, no window.
pub const AT_LOGIN_ARG: &str = "--autostart";

/// The launch agent's name, `~/Library/LaunchAgents/<name>.plist`. The bundle identifier, so it is
/// recognisable and cannot collide with another app.
pub const LAUNCH_AGENT_NAME: &str = "dev.corvus.desktop";

/// Written once the first-launch choice has been made; its presence means "leave the setting alone".
pub fn marker_path(state_dir: &Path) -> PathBuf {
    state_dir.join("desktop").join("autostart-set")
}

/// Whether this process was started by the login item.
pub fn launched_at_login<I: IntoIterator<Item = String>>(args: I) -> bool {
    args.into_iter().any(|a| a == AT_LOGIN_ARG)
}

/// Whether the executable is inside an installed app bundle (not `cargo run` or `tauri dev`): only then is a
/// login item worth making on its own.
pub fn is_installed_bundle(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    s.contains(".app/Contents/MacOS/") || std::env::var_os("APPIMAGE").is_some()
}

/// What to do at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtLaunch {
    /// First launch of an installed app: turn the login item on and write the marker.
    EnableAndMark,
    /// The login item is on: write it again so it points at this copy of the app (it may have moved).
    Refresh,
    /// Leave it as the owner set it.
    Nothing,
}

pub fn decide(marker_exists: bool, installed: bool, enabled: bool) -> AtLaunch {
    match (marker_exists, enabled) {
        (false, _) if installed => AtLaunch::EnableAndMark,
        (_, true) if installed => AtLaunch::Refresh,
        _ => AtLaunch::Nothing,
    }
}

fn write_marker(state_dir: &Path) {
    let path = marker_path(state_dir);
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, b"1\n"));
    if let Err(e) = result {
        tracing::warn!(error = %e, "cannot record the launch-at-login choice");
    }
}

/// Applies the first-launch default and refreshes an existing login item. Never fails the launch.
pub fn on_launch<R: Runtime>(app: &AppHandle<R>, state_dir: &Path) {
    let manager = app.autolaunch();
    let installed = std::env::current_exe().is_ok_and(|e| is_installed_bundle(&e));
    let enabled = manager.is_enabled().unwrap_or(false);
    match decide(marker_path(state_dir).exists(), installed, enabled) {
        AtLaunch::EnableAndMark => match manager.enable() {
            Ok(()) => write_marker(state_dir),
            Err(e) => tracing::warn!(error = %e, "cannot turn on launch at login"),
        },
        AtLaunch::Refresh => {
            if let Err(e) = manager.enable() {
                tracing::warn!(error = %e, "cannot refresh the login item");
            }
        }
        AtLaunch::Nothing => {}
    }
}

pub fn get<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// The owner's choice: applied, and recorded so the first-launch default never overrides it.
pub fn set<R: Runtime>(app: &AppHandle<R>, state_dir: &Path, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| e.to_string())?;
    write_marker(state_dir);
    get(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_of_an_installed_app_turns_it_on_once() {
        assert_eq!(decide(false, true, false), AtLaunch::EnableAndMark);
        assert_eq!(decide(false, true, true), AtLaunch::EnableAndMark);
        // The owner turned it off: it stays off.
        assert_eq!(decide(true, true, false), AtLaunch::Nothing);
        assert_eq!(decide(true, true, true), AtLaunch::Refresh);
    }

    #[test]
    fn a_development_build_is_never_turned_on_by_itself() {
        assert_eq!(decide(false, false, false), AtLaunch::Nothing);
        assert_eq!(decide(true, false, true), AtLaunch::Nothing);
    }

    #[test]
    fn the_login_argument_and_the_bundle_are_recognised() {
        assert!(launched_at_login(["corvus-desktop".into(), AT_LOGIN_ARG.into()]));
        assert!(!launched_at_login(["corvus-desktop".into()]));
        assert!(is_installed_bundle(Path::new(
            "/Applications/Corvus.app/Contents/MacOS/corvus-desktop"
        )));
        if std::env::var_os("APPIMAGE").is_none() {
            assert!(!is_installed_bundle(Path::new(
                "/repo/target/debug/corvus-desktop"
            )));
        }
    }

    #[test]
    fn the_marker_lives_in_the_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!marker_path(dir.path()).exists());
        write_marker(dir.path());
        assert!(marker_path(dir.path()).exists());
    }
}
