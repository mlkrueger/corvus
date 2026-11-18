//! The complete list of commands the page may call. Shared by `build.rs` (the app ACL manifest), the invoke
//! handler and the capability test, so they cannot drift apart.

pub const APP_COMMANDS: &[&str] = &[
    "api",
    "app_info",
    "install_cli",
    "set_background_paused",
    "check_update",
    "install_update",
];
