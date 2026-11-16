//! SQL, and only SQL ([system/01](../../../../spec/system/01-architecture.md#layering-enforced)). Store
//! functions take a [`WriteTx`](crate::db::WriteTx) (to write) or any [`Reader`](crate::db::Reader) (to read)
//! and never begin or commit transactions; services do.
//!
//! Every statement here is exercised by a test (there is no compile-time query checking).

pub mod agents;
pub mod artifacts;
pub mod claims;
pub mod conversations;
pub mod decisions;
pub mod digests;
pub mod events;
pub mod hints;
pub mod messages;
pub mod receipts;
pub mod sessions;
pub mod sources;
pub mod tasks;
pub mod threads;
