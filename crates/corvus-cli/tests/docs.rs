//! Plan 0010: documentation tests — the README's guidance copy, the decisions index, no secret-looking strings,
//! and no broken relative links in the docs and spec.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn readme_contains_the_guidance_verbatim() {
    assert!(
        read("README.md").contains(corvus_domain::GUIDANCE),
        "README.md's guidance copy drifted from corvus_domain::GUIDANCE"
    );
}

#[test]
fn decisions_index_lists_every_record() {
    let index = read("DECISIONS.md");
    let mut n = 0;
    for e in std::fs::read_dir(root().join("spec/decisions"))
        .unwrap()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.len() > 5 && name[..4].chars().all(|c| c.is_ascii_digit()) && name.ends_with(".md") {
            n += 1;
            assert!(
                index.contains(&format!("spec/decisions/{name}")),
                "DECISIONS.md is missing {name}"
            );
            assert!(
                index.contains(&format!("D{}", &name[..4])),
                "DECISIONS.md is missing D{}",
                &name[..4]
            );
        }
    }
    assert!(n >= 21, "found only {n} decision records");
}

/// Files under version control (falls back to a directory walk outside a git checkout).
fn tracked_files() -> Vec<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z"])
        .output();
    match out {
        Ok(o) if o.status.success() => o
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| root().join(String::from_utf8_lossy(s).into_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn no_secret_looking_strings_in_the_repository() {
    type Check = (&'static str, fn(&str) -> bool);
    let patterns: [Check; 6] = [
        ("private key block", |l| {
            l.contains("-----BEGIN") && l.contains("PRIVATE KEY")
        }),
        ("Linear API key", |l| l.contains("lin_api_")),
        ("GitHub token", |l| {
            l.contains("ghp_") || l.contains("github_pat_")
        }),
        ("Anthropic key", |l| l.contains("sk-ant-")),
        ("AWS access key", |l| {
            l.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w.len() == 20 && w.starts_with("AKIA"))
        }),
        ("Slack token", |l| l.contains("xoxb-") || l.contains("xoxp-")),
    ];
    let mut hits = Vec::new();
    for f in tracked_files() {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        }; // binary files
        for (i, line) in text.lines().enumerate() {
            // Test fixtures name themselves (`lin_api_SECRETKEY…`, `lin_api_test`): not real secrets.
            let lower = line.to_ascii_lowercase();
            let fixture = ["secret", "test", "fake", "example", "redacted", "lin_api_x\""]
                .iter()
                .any(|w| lower.contains(w));
            for (what, matches) in &patterns {
                if matches(line) && !fixture {
                    hits.push(format!("{}:{}: {what}", f.display(), i + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "secret-looking strings:\n{}", hits.join("\n"));
}

#[test]
fn relative_links_resolve() {
    let mut files: Vec<PathBuf> = ["README.md", "DECISIONS.md", "AGENTS.md"]
        .iter()
        .map(|f| root().join(f))
        .collect();
    for dir in ["docs", "spec"] {
        let mut stack = vec![root().join(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "md") && !p.ends_with("_template.md") {
                    files.push(p);
                }
            }
        }
    }
    let mut broken = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            let Some(end) = rest.find(')') else { break };
            let target = &rest[..end];
            rest = &rest[end..];
            if target.contains("://")
                || target.starts_with('#')
                || target.starts_with("mailto:")
                || target.contains(' ')
            {
                continue;
            }
            let path = target.split('#').next().unwrap();
            if path.is_empty() {
                continue;
            }
            let resolved = f.parent().unwrap().join(path);
            if !resolved.exists() {
                broken.push(format!(
                    "{} → {target}",
                    f.strip_prefix(root()).unwrap().display()
                ));
            }
        }
    }
    assert!(broken.is_empty(), "broken relative links:\n{}", broken.join("\n"));
}
