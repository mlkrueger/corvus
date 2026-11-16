//! Records the git commit for `corvus --version`. `CORVUS_GIT_COMMIT` overrides it (release builds in a container
//! without git); otherwise it falls back to "unknown" outside a git checkout.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=CORVUS_GIT_COMMIT");
    if let Some(c) = std::env::var("CORVUS_GIT_COMMIT")
        .ok()
        .filter(|c| !c.trim().is_empty())
    {
        println!("cargo:rustc-env=CORVUS_GIT_COMMIT={}", c.trim());
        return;
    }
    let commit = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=CORVUS_GIT_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
}
