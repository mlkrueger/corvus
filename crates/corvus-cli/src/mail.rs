//! `corvus mail …`: operator actions on mail, and the "not reachable" warning `corvus tell` prints
//! ([system/06](../../../spec/system/06-delivery-helpers.md), D0025).

use clap::Subcommand;
use corvus_core::{Services, api};
use corvus_domain::CorvusError;
use serde_json::Value;

#[derive(Subcommand, Debug)]
pub enum MailCommand {
    /// Move <from>'s unread direct mail to <to> (for a conversation that will not come back).
    Reassign { from: String, to: String },
}

pub fn run(services: &Services, command: MailCommand) -> Result<Value, CorvusError> {
    match command {
        MailCommand::Reassign { from, to } => {
            let result = services.muninn().reassign_mail(&api::ReassignMail { from, to })?;
            serde_json::to_value(result).map_err(|e| CorvusError::Internal(e.to_string()))
        }
    }
}

/// The warning for mail sent to an agent nothing can deliver to right now.
pub fn unreachable_warning(agent: &str) -> String {
    format!(
        "warning: {agent} is not reachable (no open session, waiter or watcher). The message was delivered and \
         waits until that conversation is resumed. To move its unread mail: corvus mail reassign {agent} <to>"
    )
}

/// After a direct message was sent: warn on stderr when nothing can deliver it. Never fails the command.
pub fn warn_if_unreachable(services: &Services, agent: &str) {
    if let Ok(status) = services.overview().agent_status(agent)
        && !status.reachable
    {
        eprintln!("{}", unreachable_warning(&status.name));
    }
}
