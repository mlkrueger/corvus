#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::Path;

use corvus_desktop::commands::APP_COMMANDS;
use serde_json::Value;

const EXPECTED: [&str; 6] = [
    "api",
    "app_info",
    "install_cli",
    "set_background_paused",
    "check_update",
    "install_update",
];

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn the_build_time_allow_list_is_exactly_the_six_commands() {
    let got: BTreeSet<&str> = APP_COMMANDS.iter().copied().collect();
    assert_eq!(got, BTreeSet::from(EXPECTED));
    assert_eq!(APP_COMMANDS.len(), EXPECTED.len(), "no duplicates");
    // The build script uses that same list.
    let build = read("build.rs");
    assert!(build.contains("commands::APP_COMMANDS") && build.contains("app_manifest"));
}

#[test]
fn every_capability_grants_only_those_commands_and_no_plugin_permission() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut granted = BTreeSet::new();
    let mut files = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        files += 1;
        let cap: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cap["windows"], serde_json::json!(["main"]), "{path:?}");
        assert!(
            cap.get("webviews").is_none() && cap.get("remote").is_none(),
            "{path:?}"
        );
        for p in cap["permissions"].as_array().unwrap() {
            // Plain strings only: no scoped objects, no `plugin:` or `core:` permissions.
            let p = p
                .as_str()
                .unwrap_or_else(|| panic!("scoped permission in {path:?}: {p}"));
            granted.insert(p.to_owned());
        }
    }
    assert!(files >= 1);
    let want: BTreeSet<String> = EXPECTED
        .iter()
        .map(|c| format!("allow-{}", c.replace('_', "-")))
        .collect();
    assert_eq!(granted, want);
}

#[test]
fn only_the_single_instance_and_updater_plugins_are_used() {
    let manifest = read("Cargo.toml");
    for banned in [
        "tauri-plugin-shell",
        "tauri-plugin-fs",
        "tauri-plugin-http",
        "tauri-plugin-opener",
    ] {
        assert!(!manifest.contains(banned), "{banned}");
    }
    assert!(manifest.contains("tauri-plugin-single-instance"));
    // The updater runs in Rust behind `check_update` / `install_update`; the page gets no permission for it
    // (checked above: every capability grants app commands only).
    assert!(manifest.contains("tauri-plugin-updater"));
    assert_eq!(
        manifest.matches("tauri-plugin-").count(),
        2,
        "only these two plugins"
    );
    // The generated ACL manifest (written by the build) lists the app commands only.
    let acl: Value = serde_json::from_str(&read("gen/schemas/acl-manifests.json")).unwrap();
    let keys: BTreeSet<&str> = acl.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(
        keys.iter().all(|k| *k == "__app-acl__"
            || *k == "core:default"
            || !k.contains("plugin")
            || k.contains("single-instance")),
        "{keys:?}"
    );
}

#[test]
fn the_app_version_and_updater_settings_are_what_a_release_needs() {
    let conf: Value = serde_json::from_str(&read("tauri.conf.json")).unwrap();
    // The updater compares this version with the release's; it must be the workspace version.
    assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
    let updater = &conf["plugins"]["updater"];
    assert!(updater["pubkey"].as_str().is_some_and(|k| k.len() > 40));
    let endpoints = updater["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1);
    let url = endpoints[0].as_str().unwrap();
    assert!(url.starts_with("https://github.com/") && url.ends_with("/releases/latest/download/latest.json"));
    assert!(updater.get("dangerousInsecureTransportProtocol").is_none());
    assert!(
        conf["bundle"].get("createUpdaterArtifacts").is_none(),
        "the Makefile signs the archive itself"
    );
}
