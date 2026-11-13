//! Text field validators.

use crate::error::{CorvusError, Result};

/// Maximum body length, in Unicode scalar values.
pub const MAX_BODY_CHARS: usize = 20_000;

fn non_blank(s: &str, what: &str) -> Result<()> {
    if s.trim().is_empty() {
        return Err(CorvusError::validation(format!("{what} must not be empty")));
    }
    Ok(())
}

fn max_chars(s: &str, what: &str) -> Result<()> {
    if s.chars().count() > MAX_BODY_CHARS {
        return Err(CorvusError::validation(format!(
            "{what} must be at most {MAX_BODY_CHARS} characters"
        )));
    }
    Ok(())
}

pub fn validate_title(s: &str) -> Result<()> {
    non_blank(s, "title")
}

pub fn validate_task_body(s: &str) -> Result<()> {
    max_chars(s, "task body")
}

pub fn validate_message_body(s: &str) -> Result<()> {
    non_blank(s, "message body")?;
    max_chars(s, "message body")
}

pub fn validate_summary(s: &str) -> Result<()> {
    non_blank(s, "summary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn nonblank_accepted(pre in "[ \t\n]{0,3}", core in "[a-zA-Z0-9é日]{1,50}", post in "[ \t\n]{0,3}") {
            let s = format!("{pre}{core}{post}");
            prop_assert!(validate_title(&s).is_ok());
            prop_assert!(validate_summary(&s).is_ok());
            prop_assert!(validate_message_body(&s).is_ok());
            prop_assert!(validate_task_body(&s).is_ok());
        }

        #[test]
        fn whitespace_only_rejected(s in "[ \t\n\r\u{a0}\u{2003}]{0,30}") {
            prop_assert!(validate_title(&s).is_err());
            prop_assert!(validate_summary(&s).is_err());
            prop_assert!(validate_message_body(&s).is_err());
            // task body may be empty / blank
            prop_assert!(validate_task_body(&s).is_ok());
        }

        #[test]
        fn over_limit_rejected(c in prop::sample::select(vec!['a', 'é', '日', '😀'])) {
            let over: String = std::iter::repeat_n(c, MAX_BODY_CHARS + 1).collect();
            let at: String = std::iter::repeat_n(c, MAX_BODY_CHARS).collect();
            prop_assert!(validate_task_body(&over).is_err());
            prop_assert!(validate_message_body(&over).is_err());
            // multibyte chars count as one each
            prop_assert!(validate_task_body(&at).is_ok());
            prop_assert!(validate_message_body(&at).is_ok());
        }
    }
}
