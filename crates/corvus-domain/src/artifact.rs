//! Artifact references. At least as strict as the `artifacts` SQL CHECKs.

use crate::enums::ArtifactKind;
use crate::error::{CorvusError, Result};

pub const MAX_ARTIFACT_PATH_CHARS: usize = 1024;
pub const MIN_COMMIT_LEN: usize = 7;
pub const MAX_COMMIT_LEN: usize = 40;

/// Validate a repository-relative artifact path.
pub fn validate_artifact_path(path: &str) -> Result<()> {
    let err = |m: &str| Err(CorvusError::validation(format!("artifact path {m}")));
    if path.is_empty() {
        return err("must not be empty");
    }
    if path.chars().count() > MAX_ARTIFACT_PATH_CHARS {
        return err("is too long (max 1024 characters)");
    }
    if path.starts_with('/') {
        return err("must be relative");
    }
    if path.starts_with('~') {
        return err("must not start with '~'");
    }
    if path.contains('\0') {
        return err("must not contain NUL");
    }
    if path.split('/').any(|c| c == "..") {
        return err("must not contain '..' components");
    }
    Ok(())
}

/// Validate a commit hash: 7-40 lowercase hex characters.
pub fn validate_commit_hash(h: &str) -> Result<()> {
    if !(MIN_COMMIT_LEN..=MAX_COMMIT_LEN).contains(&h.len())
        || !h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(CorvusError::validation(
            "commit hash must be 7-40 lowercase hex characters",
        ));
    }
    Ok(())
}

/// A validated reference to something produced by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ArtifactRef {
    pub kind: ArtifactKind,
    pub path: String,
    pub commit: Option<String>,
}

impl ArtifactRef {
    pub fn new(kind: ArtifactKind, path: impl Into<String>, commit: Option<String>) -> Result<Self> {
        let path = path.into();
        validate_artifact_path(&path)?;
        if let Some(c) = &commit {
            validate_commit_hash(c)?;
        }
        Ok(Self { kind, path, commit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn valid_paths_accepted(p in "[A-Za-z0-9_.-][A-Za-z0-9_. /-]{0,200}") {
            prop_assume!(!p.starts_with('~') && !p.split('/').any(|c| c == ".."));
            prop_assert!(validate_artifact_path(&p).is_ok());
        }

        #[test]
        fn dotdot_anywhere_rejected(a in "[a-z]{1,8}", b in "[a-z]{1,8}") {
            for p in ["..".to_string(), format!("../{a}"), format!("{a}/../{b}"), format!("{a}/.."), format!("{a}/{b}/../{a}")] {
                prop_assert!(validate_artifact_path(&p).is_err(), "{}", p);
            }
        }

        #[test]
        fn dotdot_lookalikes_accepted(a in "[a-z]{1,8}") {
            for p in [format!("..{a}"), format!("{a}.."), format!("{a}/...{a}"), format!("{a}/..{a}/x"), "...".to_string()] {
                prop_assert!(validate_artifact_path(&p).is_ok(), "{}", p);
            }
        }

        #[test]
        fn absolute_rejected(p in "/[a-z/]{0,30}") {
            prop_assert!(validate_artifact_path(&p).is_err());
        }

        #[test]
        fn tilde_rejected(p in "~[a-z/]{0,30}") {
            prop_assert!(validate_artifact_path(&p).is_err());
        }

        #[test]
        fn nul_rejected(a in "[a-z]{0,10}", b in "[a-z]{0,10}") {
            let p = format!("x{a}\0{b}");
            prop_assert!(validate_artifact_path(&p).is_err());
        }

        #[test]
        fn too_long_rejected(n in 1025usize..1200) {
            prop_assert!(validate_artifact_path(&"a".repeat(n)).is_err());
        }

        #[test]
        fn valid_commits_accepted(h in "[0-9a-f]{7,40}") {
            prop_assert!(validate_commit_hash(&h).is_ok());
            prop_assert!(ArtifactRef::new(ArtifactKind::Commit, "src/lib.rs", Some(h)).is_ok());
        }

        #[test]
        fn bad_commits_rejected(
            short in "[0-9a-f]{0,6}",
            long in "[0-9a-f]{41,60}",
            upper in "[0-9a-f]{3}[A-F][0-9a-f]{3}",
            nonhex in "[0-9a-f]{3}[g-z\\- ][0-9a-f]{3}",
        ) {
            for h in [short, long, upper, nonhex] {
                prop_assert!(validate_commit_hash(&h).is_err(), "{}", h);
            }
        }
    }

    #[test]
    fn multibyte_path_counts_chars() {
        assert!(validate_artifact_path(&"é".repeat(1024)).is_ok());
        assert!(validate_artifact_path(&"é".repeat(1025)).is_err());
    }

    #[test]
    fn empty_rejected() {
        assert!(validate_artifact_path("").is_err());
        assert!(validate_commit_hash("").is_err());
    }

    #[test]
    fn artifact_ref_new() {
        let a = ArtifactRef::new(ArtifactKind::File, "a/b.rs", None).unwrap();
        assert_eq!(a.path, "a/b.rs");
        assert!(ArtifactRef::new(ArtifactKind::File, "../b", None).is_err());
        assert!(ArtifactRef::new(ArtifactKind::File, "b", Some("XYZ".into())).is_err());
    }
}
