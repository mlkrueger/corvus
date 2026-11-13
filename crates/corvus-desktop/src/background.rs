//! Pausable background work: the housekeeping loop (and, later, tracker polling) under the leader lock.
//! Pausing stops the loop and so drops leadership; resuming starts a fresh one.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use corvus_core::background::{LoopTimings, run_background};
use corvus_core::services::Services;
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::watch;

/// How long shutdown waits for a running pass before giving up on it.
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Running {
    stop: Option<watch::Sender<bool>>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub struct Background {
    services: Services,
    state_dir: PathBuf,
    inner: Mutex<Running>,
}

impl Background {
    /// Not started; call [`Background::set_paused`] with `false` to start.
    pub fn new(services: Services, state_dir: PathBuf) -> Self {
        Self {
            services,
            state_dir,
            inner: Mutex::new(Running {
                stop: None,
                handle: None,
            }),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.lock().stop.is_none()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `true` stops the loop (dropping leadership); `false` starts it if it is not running.
    pub fn set_paused(&self, paused: bool) {
        let mut inner = self.lock();
        if paused {
            if let Some(stop) = inner.stop.take() {
                let _ = stop.send(true);
            }
            return;
        }
        if inner.stop.is_some() {
            return;
        }
        let (tx, rx) = watch::channel(false);
        // A previous loop may still be finishing a pass; wait for it so two loops never overlap.
        let previous = inner.handle.take();
        let (services, dir) = (self.services.clone(), self.state_dir.clone());
        inner.handle = Some(async_runtime::spawn(async move {
            if let Some(prev) = previous {
                let _ = prev.await;
            }
            run_background(services, dir, "desktop", LoopTimings::default(), rx, |_| {}).await;
        }));
        inner.stop = Some(tx);
    }

    /// Stops the loop and waits (bounded) for it to finish. For app exit.
    pub async fn shutdown(&self) {
        let handle = {
            let mut inner = self.lock();
            if let Some(stop) = inner.stop.take() {
                let _ = stop.send(true);
            }
            inner.handle.take()
        };
        if let Some(handle) = handle
            && tokio::time::timeout(STOP_GRACE, handle).await.is_err()
        {
            tracing::warn!("background loop did not stop in time");
        }
    }
}
