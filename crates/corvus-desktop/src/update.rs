//! Checking for and installing a newer version of the app
//! ([system/05](../../../spec/system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2), D0027).
//!
//! The check asks GitHub Releases for `latest.json`; nothing is downloaded until the owner clicks install. The
//! updater plugin runs here, in Rust: the page reaches it only through the `check_update` and `install_update`
//! commands and holds no plugin permission.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

/// A check answered from memory when the last one is younger than this, unless forced.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

pub const DISABLED: &str = "update checks are turned off (CORVUS_UPDATE_CHECK=off)";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateStatus {
    /// `false` when `CORVUS_UPDATE_CHECK=off`: no check is ever made.
    pub enabled: bool,
    pub current: String,
    /// The newer version, when there is one.
    pub available: Option<String>,
    /// Why the last check failed, if it did.
    pub error: Option<String>,
}

impl UpdateStatus {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            current: env!("CARGO_PKG_VERSION").to_owned(),
            available: None,
            error: None,
        }
    }
}

/// The app's update state: the switch, the last answer and whether an install is running.
#[derive(Debug)]
pub struct Updates {
    enabled: bool,
    last: Mutex<Option<(Instant, UpdateStatus)>>,
    installing: AtomicBool,
}

impl Updates {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last: Mutex::new(None),
            installing: AtomicBool::new(false),
        }
    }

    /// The answer that needs no request: always when disabled; otherwise the last one while it is fresh and
    /// the caller did not force a new check.
    pub fn remembered(&self, now: Instant, force: bool) -> Option<UpdateStatus> {
        if !self.enabled {
            return Some(UpdateStatus::new(false));
        }
        if force {
            return None;
        }
        let last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        last.as_ref()
            .filter(|(at, _)| now.saturating_duration_since(*at) < CHECK_INTERVAL)
            .map(|(_, s)| s.clone())
    }

    pub fn remember(&self, now: Instant, status: &UpdateStatus) {
        *self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((now, status.clone()));
    }

    /// Claims the one install slot; `false` if an install is already running.
    fn begin_install(&self) -> bool {
        !self.installing.swap(true, Ordering::SeqCst)
    }

    fn end_install(&self) {
        self.installing.store(false, Ordering::SeqCst);
    }
}

async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, String> {
    let updater = app
        .updater()
        .map_err(|e| format!("cannot start the update check: {e}"))?;
    updater
        .check()
        .await
        .map_err(|e| format!("cannot check for updates: {e}"))
}

/// Whether a newer version exists. Never fails: a failed check is logged and reported in `error`.
pub async fn check<R: Runtime>(app: &AppHandle<R>, updates: &Updates, force: bool) -> UpdateStatus {
    if let Some(status) = updates.remembered(Instant::now(), force) {
        return status;
    }
    let mut status = UpdateStatus::new(true);
    match find(app).await {
        Ok(update) => status.available = update.map(|u| u.version),
        Err(e) => {
            tracing::warn!(error = %e, "update check failed");
            status.error = Some(e);
        }
    }
    updates.remember(Instant::now(), &status);
    status
}

/// Downloads and verifies the newer version, runs `before_replace` (stop background work, checkpoint), then
/// replaces the app. On success the caller restarts the app. `Err` after `before_replace` ran is reported as
/// `Failed::AfterStop` so the caller can start background work again.
pub async fn install<R, F, Fut>(
    app: &AppHandle<R>,
    updates: &Updates,
    before_replace: F,
) -> Result<(), Failed>
where
    R: Runtime,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if !updates.enabled {
        return Err(Failed::Before(DISABLED.to_owned()));
    }
    if !updates.begin_install() {
        return Err(Failed::Before("an update is already being installed".to_owned()));
    }
    let result = async {
        let update = find(app)
            .await
            .map_err(Failed::Before)?
            .ok_or_else(|| Failed::Before("no newer version is available".to_owned()))?;
        // `download` verifies the signature against the updater public key before returning the bytes.
        let bytes = update
            .download(|_, _| {}, || {})
            .await
            .map_err(|e| Failed::Before(format!("cannot download the update: {e}")))?;
        before_replace().await;
        update
            .install(bytes)
            .map_err(|e| Failed::AfterStop(format!("cannot install the update: {e}")))
    }
    .await;
    updates.end_install();
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// Nothing was changed.
    Before(String),
    /// Background work was already stopped.
    AfterStop(String),
}

impl Failed {
    pub fn message(&self) -> &str {
        match self {
            Self::Before(m) | Self::AfterStop(m) => m,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(version: &str) -> UpdateStatus {
        UpdateStatus {
            available: Some(version.to_owned()),
            ..UpdateStatus::new(true)
        }
    }

    #[test]
    fn disabled_always_answers_without_a_check() {
        let u = Updates::new(false);
        let now = Instant::now();
        for force in [false, true] {
            let s = u.remembered(now, force).expect("answered from memory");
            assert!(!s.enabled);
            assert_eq!(s.available, None);
        }
    }

    #[test]
    fn a_fresh_answer_is_reused_until_the_interval_passes_or_a_check_is_forced() {
        let u = Updates::new(true);
        let t0 = Instant::now();
        assert_eq!(u.remembered(t0, false), None, "nothing known yet");
        u.remember(t0, &found("9.9.9"));
        let just_inside = t0 + CHECK_INTERVAL - Duration::from_secs(1);
        assert_eq!(u.remembered(just_inside, false), Some(found("9.9.9")));
        assert_eq!(u.remembered(just_inside, true), None, "forced");
        assert_eq!(u.remembered(t0 + CHECK_INTERVAL, false), None, "stale");
    }

    #[test]
    fn a_failed_check_is_remembered_too() {
        let u = Updates::new(true);
        let t0 = Instant::now();
        let failed = UpdateStatus {
            error: Some("offline".into()),
            ..UpdateStatus::new(true)
        };
        u.remember(t0, &failed);
        assert_eq!(u.remembered(t0 + Duration::from_secs(60), false), Some(failed));
    }

    #[test]
    fn only_one_install_runs_at_a_time() {
        let u = Updates::new(true);
        assert!(u.begin_install());
        assert!(!u.begin_install());
        u.end_install();
        assert!(u.begin_install());
    }

    #[test]
    fn the_status_reports_this_build_as_current() {
        assert_eq!(UpdateStatus::new(true).current, env!("CARGO_PKG_VERSION"));
    }
}
