//! `Corvus`: the async facade the tokio transports (HTTP, MCP, desktop IPC) use
//! ([system/01](../../../spec/system/01-architecture.md#database-access), D0014).
//!
//! Services are synchronous; each call runs on tokio's blocking pool so a slow disk never stalls the async
//! workers. Transports call exactly one service method per request through [`Corvus::run`].

use std::sync::Arc;

use corvus_domain::{CorvusError, Result};

use crate::api::{MessageView, WaitForInbox};
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::db::{Database, MigrationReport};
use crate::hermod::{Hermod, wait_for_inbox};
use crate::services::Services;

/// One per process. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Corvus {
    services: Services,
    hermod: Hermod,
}

impl Corvus {
    /// Opens (creating and migrating if needed) the configured database with the system clock.
    pub fn open(config: &Config) -> Result<(Self, MigrationReport)> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let (db, report) = Database::open(&config.database_path, clock.as_ref())?;
        let services = Services::from_config(Arc::new(db), clock, config);
        let hermod = Hermod::new(&config.database_path);
        Ok((Self { services, hermod }, report))
    }

    pub fn new(services: Services, hermod: Hermod) -> Self {
        Self { services, hermod }
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn hermod(&self) -> &Hermod {
        &self.hermod
    }

    /// Runs one synchronous service call on the blocking pool.
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Services) -> Result<T> + Send + 'static,
    {
        let s = self.services.clone();
        tokio::task::spawn_blocking(move || f(&s))
            .await
            .map_err(|e| CorvusError::Internal(format!("service call failed: {e}")))?
    }

    /// The blocking inbox wait (long poll), ≤ 120 s.
    pub async fn wait_for_inbox(&self, req: WaitForInbox) -> Result<Vec<MessageView>> {
        wait_for_inbox(&self.services, &self.hermod, &req).await
    }
}
