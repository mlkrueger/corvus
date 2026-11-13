//! The env file ([system/10](../../../spec/system/10-config-ops.md#env-file)): `KEY=VALUE` lines and `#`
//! comments, no interpolation. Variables already in the process environment win; the file only fills gaps.
//! The file must not be readable by group or others.

use std::collections::BTreeMap;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Why the env file could not be loaded.
#[derive(Debug)]
pub enum EnvFileError {
    /// Group- or world-readable (or writable) file.
    Permissions {
        path: PathBuf,
        mode: u32,
    },
    /// A line that is not `KEY=VALUE`.
    Syntax {
        path: PathBuf,
        line: usize,
    },
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl fmt::Display for EnvFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permissions { path, mode } => write!(
                f,
                "env file {} has mode {:04o}; it may hold secrets and must not be accessible by group or \
                 others (run: chmod 600 {})",
                path.display(),
                mode & 0o7777,
                path.display()
            ),
            Self::Syntax { path, line } => {
                write!(f, "env file {} line {line}: expected KEY=VALUE", path.display())
            }
            Self::Io { path, error } => write!(f, "env file {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for EnvFileError {}

/// Variables read from an env file. Values are never logged.
#[derive(Default, Clone)]
pub struct EnvFile {
    vars: BTreeMap<String, String>,
}

impl fmt::Debug for EnvFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvFile")
            .field("keys", &self.vars.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl EnvFile {
    /// Loads `path`. A missing file is not an error (it yields no variables).
    pub fn load(path: &Path) -> Result<Self, EnvFileError> {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(EnvFileError::Io {
                    path: path.to_owned(),
                    error,
                });
            }
        };
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(EnvFileError::Permissions {
                path: path.to_owned(),
                mode,
            });
        }
        let text = std::fs::read_to_string(path).map_err(|error| EnvFileError::Io {
            path: path.to_owned(),
            error,
        })?;
        Self::parse(&text).map_err(|line| EnvFileError::Syntax {
            path: path.to_owned(),
            line,
        })
    }

    /// Parses env-file text; on error returns the 1-based line number.
    pub fn parse(text: &str) -> Result<Self, usize> {
        let mut vars = BTreeMap::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line.split_once('=').ok_or(i + 1)?;
            let k = k.trim();
            let valid_key = !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit());
            if !valid_key {
                return Err(i + 1);
            }
            let v = v.trim();
            let v = strip_quotes(v);
            vars.insert(k.to_owned(), v.to_owned());
        }
        Ok(Self { vars })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

fn strip_quotes(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_lines_comments_and_quotes() {
        let f = EnvFile::parse("# c\n\nA=1\n B = two words \nC=\"q=x\"\nD='s'\nE=\n").unwrap();
        assert_eq!(f.get("A"), Some("1"));
        assert_eq!(f.get("B"), Some("two words"));
        assert_eq!(f.get("C"), Some("q=x"));
        assert_eq!(f.get("D"), Some("s"));
        assert_eq!(f.get("E"), Some(""));
    }

    #[test]
    fn no_interpolation() {
        let f = EnvFile::parse("A=$HOME\nB=$(whoami)").unwrap();
        assert_eq!(f.get("A"), Some("$HOME"));
        assert_eq!(f.get("B"), Some("$(whoami)"));
    }

    #[test]
    fn syntax_errors_name_the_line() {
        assert_eq!(EnvFile::parse("A=1\nnot a pair").unwrap_err(), 2);
        assert_eq!(EnvFile::parse("1A=x").unwrap_err(), 1);
        assert_eq!(EnvFile::parse("=x").unwrap_err(), 1);
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(EnvFile::load(&dir.path().join("nope")).unwrap().is_empty());
    }

    #[test]
    fn private_file_loads_and_readable_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("env");
        fs::write(&p, "CORVUS_LOG=debug\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(EnvFile::load(&p).unwrap().get("CORVUS_LOG"), Some("debug"));
        for mode in [0o640, 0o604, 0o644, 0o620] {
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            let err = EnvFile::load(&p).unwrap_err();
            assert!(matches!(err, EnvFileError::Permissions { .. }), "mode {mode:o}");
            assert!(err.to_string().contains("chmod 600"));
        }
    }

    #[test]
    fn debug_shows_keys_not_values() {
        let f = EnvFile::parse("CORVUS_API_TOKEN=supersecretsupersecretsupersecret00").unwrap();
        let d = format!("{f:?}");
        assert!(d.contains("CORVUS_API_TOKEN"));
        assert!(!d.contains("supersecret"));
    }
}
