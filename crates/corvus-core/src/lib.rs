//! Corvus core: configuration, clock, the one database opener and migrator, and (from plan 0002) the
//! services that own every transaction.

pub mod api;
pub mod clock;
pub mod config;
pub mod db;
pub mod envfile;
pub mod hermod;
pub mod ids;
pub mod logging;
pub mod services;
pub mod store;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use clock::{Clock, ManualClock, Millis, SystemClock};
pub use config::Config;
pub use db::{Database, ReadOnlyDatabase};
pub use services::{Services, Settings};
