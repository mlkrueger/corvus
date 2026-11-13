//! Corvus core: configuration, clock, the one database opener and migrator, and (from plan 0002) the
//! services that own every transaction.

pub mod clock;
pub mod config;
pub mod db;
pub mod envfile;
pub mod ids;
pub mod logging;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use clock::{Clock, ManualClock, Millis, SystemClock};
pub use config::Config;
pub use db::{Database, ReadOnlyDatabase};
