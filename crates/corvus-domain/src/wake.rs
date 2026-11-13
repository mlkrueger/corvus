//! Waking idle agents through per-tool adapters ([system/06](../../../spec/system/06-delivery-helpers.md#agent-adapters),
//! D0021). Pure: the adapter model, its validation and argument rendering. Loading files and spawning processes
//! live in `corvus-core`.
//!
//! There is deliberately no placeholder for message text: the only text an adapter can pass to a tool is
//! [`WAKE_NOTE`].

use std::collections::BTreeMap;

use crate::error::{CorvusError, Result};
use crate::name::AgentName;

/// The fixed nudge given to a woken tool. Peer message text never reaches a tool's prompt.
pub const WAKE_NOTE: &str =
    "You have unread Corvus messages. Check your Corvus inbox (read_inbox), handle them, and ack each one.";

/// The closed set of placeholders an adapter command may use, each as a whole argument.
pub const PLACEHOLDERS: &[&str] = &["{agent}", "{conversation}", "{note}", "{count}", "{state_dir}"];

/// How a binding value (e.g. a conversation id) must look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingPattern {
    Uuid,
    /// A full-match pattern restricted to a safe subset: literal characters plus `[...]` classes with `+`/`*`/`?`
    /// or `{n,m}` is more than we need; instead patterns are a set of allowed characters and a length range.
    Charset {
        allowed: String,
        min: usize,
        max: usize,
    },
}

impl BindingPattern {
    /// `"uuid"`, or `"chars:<allowed>:<min>-<max>"` (e.g. `chars:abcdefghijklmnopqrstuvwxyz0123456789-:1-64`).
    pub fn parse(s: &str) -> Result<Self> {
        if s == "uuid" {
            return Ok(Self::Uuid);
        }
        let bad = || {
            CorvusError::Validation(format!(
                "binding pattern {s:?}: use \"uuid\" or \"chars:<allowed>:<min>-<max>\""
            ))
        };
        let rest = s.strip_prefix("chars:").ok_or_else(bad)?;
        let (allowed, range) = rest.rsplit_once(':').ok_or_else(bad)?;
        let (min, max) = range.split_once('-').ok_or_else(bad)?;
        let (min, max): (usize, usize) = (min.parse().map_err(|_| bad())?, max.parse().map_err(|_| bad())?);
        if allowed.is_empty() || min == 0 || max < min || max > 256 {
            return Err(bad());
        }
        Ok(Self::Charset {
            allowed: allowed.to_owned(),
            min,
            max,
        })
    }

    pub fn matches(&self, v: &str) -> bool {
        match self {
            Self::Uuid => v.len() == 36 && uuid::Uuid::parse_str(v).is_ok(),
            Self::Charset { allowed, min, max } => {
                let n = v.chars().count();
                n >= *min && n <= *max && v.chars().all(|c| allowed.contains(c))
            }
        }
    }
}

/// A validated adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub title: String,
    /// Binding fields this tool needs (today only `conversation`), with whether each is required.
    pub binding: BTreeMap<String, (bool, BindingPattern)>,
    pub wake: Vec<String>,
    pub timeout_seconds: u64,
    pub check: Option<Vec<String>>,
}

/// The raw shape of an adapter file, before validation (built by the TOML loader in core).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawAdapter {
    pub name: String,
    pub title: Option<String>,
    pub binding: BTreeMap<String, (bool, String)>,
    pub wake: Vec<String>,
    pub timeout_seconds: Option<u64>,
    pub check: Option<Vec<String>>,
}

/// Adapter names follow the agent-name grammar (they become file names).
pub fn validate_adapter_name(name: &str) -> Result<()> {
    AgentName::parse(name)
        .map(|_| ())
        .map_err(|_| CorvusError::Validation(format!("adapter name {name:?}: use 1-64 of A-Z a-z 0-9 _ . -")))
}

fn validate_command(what: &str, cmd: &[String], allow_placeholders: bool) -> Result<()> {
    let bad = |m: String| CorvusError::Validation(format!("{what}: {m}"));
    let Some(program) = cmd.first() else {
        return Err(bad("command must not be empty".into()));
    };
    if program.trim().is_empty() || program.contains('{') {
        return Err(bad("the program must be a literal name or path".into()));
    }
    for arg in cmd {
        if arg.contains('\0') {
            return Err(bad("arguments must not contain NUL".into()));
        }
        let has_brace = arg.contains('{') || arg.contains('}');
        if !has_brace {
            continue;
        }
        if !allow_placeholders {
            return Err(bad(format!("placeholders are not allowed here: {arg:?}")));
        }
        if !PLACEHOLDERS.contains(&arg.as_str()) {
            return Err(bad(format!(
                "{arg:?} is not a placeholder; placeholders are whole arguments from {}",
                PLACEHOLDERS.join(", ")
            )));
        }
    }
    Ok(())
}

impl Adapter {
    /// Validates a raw adapter: name, binding patterns, closed placeholder set, whole-argument rule.
    pub fn validate(raw: RawAdapter) -> Result<Self> {
        validate_adapter_name(&raw.name)?;
        let mut binding = BTreeMap::new();
        for (field, (required, pattern)) in raw.binding {
            if field != "conversation" {
                return Err(CorvusError::Validation(format!(
                    "binding field {field:?}: only \"conversation\" is supported"
                )));
            }
            binding.insert(field, (required, BindingPattern::parse(&pattern)?));
        }
        validate_command("wake.command", &raw.wake, true)?;
        if raw.wake.iter().any(|a| a == "{conversation}") && !binding.contains_key("conversation") {
            return Err(CorvusError::Validation(
                "wake.command uses {conversation} but [binding] does not declare it".into(),
            ));
        }
        if let Some(c) = &raw.check {
            validate_command("check.command", c, false)?;
        }
        let timeout = raw.timeout_seconds.unwrap_or(300);
        if !(1..=3600).contains(&timeout) {
            return Err(CorvusError::Validation("wake.timeout_seconds: 1-3600".into()));
        }
        Ok(Self {
            title: raw.title.unwrap_or_else(|| raw.name.clone()),
            name: raw.name,
            binding,
            wake: raw.wake,
            timeout_seconds: timeout,
            check: raw.check,
        })
    }

    /// Checks binding values against the adapter's declared fields.
    pub fn validate_binding(&self, conversation: Option<&str>) -> Result<()> {
        match (self.binding.get("conversation"), conversation) {
            (Some((true, _)), None) => Err(CorvusError::Validation(format!(
                "adapter {} needs --conversation",
                self.name
            ))),
            (Some((_, p)), Some(v)) if !p.matches(v) => Err(CorvusError::Validation(format!(
                "conversation {v:?} does not match adapter {}'s pattern",
                self.name
            ))),
            (None, Some(_)) => Err(CorvusError::Validation(format!(
                "adapter {} takes no conversation",
                self.name
            ))),
            _ => Ok(()),
        }
    }

    /// The argument vector for one wake. Only whole-argument placeholders are replaced; `{note}` is always
    /// [`WAKE_NOTE`].
    pub fn render_wake(
        &self,
        agent: &AgentName,
        conversation: Option<&str>,
        count: usize,
        state_dir: &str,
    ) -> Vec<String> {
        self.wake
            .iter()
            .map(|a| match a.as_str() {
                "{agent}" => agent.to_string(),
                "{conversation}" => conversation.unwrap_or_default().to_owned(),
                "{note}" => WAKE_NOTE.to_owned(),
                "{count}" => count.to_string(),
                "{state_dir}" => state_dir.to_owned(),
                other => other.to_owned(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(wake: &[&str]) -> RawAdapter {
        RawAdapter {
            name: "codex".into(),
            title: None,
            binding: [("conversation".to_owned(), (true, "uuid".to_owned()))].into(),
            wake: wake.iter().map(|s| (*s).to_owned()).collect(),
            timeout_seconds: None,
            check: Some(vec!["codex".into(), "--version".into()]),
        }
    }

    #[test]
    fn valid_adapter_renders_only_the_fixed_note() {
        let a = Adapter::validate(raw(&["codex", "exec", "resume", "{conversation}", "{note}"])).unwrap();
        let agent = AgentName::parse("bob").unwrap();
        let conv = "01890000-0000-7000-8000-000000000000";
        let argv = a.render_wake(&agent, Some(conv), 3, "/s");
        assert_eq!(argv, vec!["codex", "exec", "resume", conv, WAKE_NOTE]);
        assert_eq!(a.timeout_seconds, 300);
    }

    #[test]
    fn unknown_or_embedded_placeholders_are_rejected() {
        for wake in [
            vec!["codex", "{message}"],
            vec!["codex", "--note={note}"],
            vec!["codex", "pre{agent}"],
            vec!["codex", "{body}"],
            vec!["{agent}", "x"],
            vec![],
            vec!["  "],
        ] {
            assert!(Adapter::validate(raw(&wake)).is_err(), "{wake:?}");
        }
        let mut r = raw(&["codex", "{note}"]);
        r.check = Some(vec!["codex".into(), "{agent}".into()]);
        assert!(Adapter::validate(r).is_err(), "no placeholders in check");
    }

    #[test]
    fn conversation_placeholder_needs_a_binding() {
        let mut r = raw(&["tool", "{conversation}"]);
        r.binding.clear();
        assert!(Adapter::validate(r).is_err());
    }

    #[test]
    fn binding_values_are_validated() {
        let a = Adapter::validate(raw(&["codex", "{conversation}", "{note}"])).unwrap();
        assert!(a.validate_binding(None).is_err());
        assert!(a.validate_binding(Some("not-a-uuid")).is_err());
        assert!(
            a.validate_binding(Some("01890000-0000-7000-8000-000000000000"))
                .is_ok()
        );
        let p = BindingPattern::parse("chars:abc-:1-5").unwrap();
        assert!(p.matches("a-b") && !p.matches("abcabc") && !p.matches("a b") && !p.matches(""));
        for bad in ["regex:.*", "chars::1-5", "chars:a:0-5", "chars:a:5-1", "chars:a"] {
            assert!(BindingPattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wake_note_contains_no_placeholder_and_mentions_the_inbox() {
        assert!(!WAKE_NOTE.contains('{'));
        assert!(WAKE_NOTE.contains("read_inbox"));
    }
}
