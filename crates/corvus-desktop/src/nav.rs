//! The navigation guard: the webview may show only the app's own bundled assets. Every other navigation and
//! every new-window request is cancelled; `https:` targets are handed to the system browser from Rust.

use std::process::{Command, Stdio};

use tauri::Url;

/// Hosts that serve the bundled assets (`tauri://localhost` on macOS/Linux, `http(s)://tauri.localhost` on Windows).
fn bundled_origin(url: &Url) -> bool {
    matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
    )
}

/// The Vite dev server, debug builds only.
fn dev_server(url: &Url) -> bool {
    cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(5173)
        && url.username().is_empty()
        && url.password().is_none()
}

/// True only for the app's own origins. Everything else (external, loopback, `file:`, `data:`, `javascript:`) is false.
pub fn navigation_allowed(url: &Url) -> bool {
    (bundled_origin(url) && url.username().is_empty() && url.password().is_none()) || dev_server(url)
}

/// The program and argument vector that opens `url` in the system browser; `None` unless it is `https:`.
pub fn browser_command(url: &Url) -> Option<(&'static str, Vec<String>)> {
    if url.scheme() != "https" || url.host_str().is_none() {
        return None;
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return None;
    };
    Some((program, vec![url.as_str().to_owned()]))
}

/// Hands a rejected target to the system browser if it is `https:`; anything else is dropped (and logged).
/// Never goes through a shell.
pub fn hand_to_browser(url: &Url) {
    let Some((program, args)) = browser_command(url) else {
        tracing::warn!(scheme = url.scheme(), "blocked navigation to a non-https target");
        return;
    };
    tracing::info!(
        host = url.host_str().unwrap_or(""),
        "opening external link in the system browser"
    );
    match Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Reap the child so it does not linger as a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, program, "could not open the system browser"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn allows_bundled_origins() {
        for s in [
            "tauri://localhost",
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/assets/a.js",
        ] {
            assert!(navigation_allowed(&u(s)), "{s}");
        }
    }

    #[test]
    fn rejects_everything_else() {
        for s in [
            "https://example.com",
            "https://tauri.localhost.evil.example/",
            "http://tauri.localhost:8080/",
            "tauri://evil/",
            "tauri://user@localhost/",
            "http://127.0.0.1:8765/",
            "http://localhost:8765/",
            "http://localhost/",
            "file:///etc/passwd",
            "data:text/html,<b>x</b>",
            "javascript:alert(1)",
            "about:blank",
            "blob:tauri://localhost/x",
        ] {
            assert!(!navigation_allowed(&u(s)), "{s}");
        }
    }

    #[test]
    fn dev_server_only_in_debug() {
        assert_eq!(
            navigation_allowed(&u("http://localhost:5173/")),
            cfg!(debug_assertions)
        );
        assert!(!navigation_allowed(&u("http://localhost:5174/")));
        assert!(!navigation_allowed(&u("https://localhost:5173/")));
    }

    #[test]
    fn only_https_reaches_the_browser() {
        let cmd = browser_command(&u("https://example.com/a?b=c"));
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            let (_, args) = cmd.expect("https is opened");
            assert_eq!(args, vec!["https://example.com/a?b=c".to_owned()]);
        }
        for s in [
            "http://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:,x",
            "tauri://x/",
        ] {
            assert!(browser_command(&u(s)).is_none(), "{s}");
        }
    }
}
