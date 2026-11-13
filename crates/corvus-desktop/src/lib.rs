//! The Corvus desktop app ([system/05](../../../spec/system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2)).
//!
//! The testable pieces live in modules here; `main.rs` only hands Tauri's generated context to [`run`].
//! The app never binds a socket: the page reaches the data router through the `api` IPC command.

mod app;
pub mod background;
pub mod commands;
pub mod info;
pub mod ipc;
pub mod nav;

pub use app::run;
