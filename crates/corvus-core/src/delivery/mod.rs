//! Delivery helpers ([system/06](../../../../spec/system/06-delivery-helpers.md)): the inbox hook, the wake
//! supervisor and agent adapters. None of them acks: only the agent acknowledges a message.

pub mod adapters;
pub mod bindings;
pub mod claude_setup;
pub mod hook;
pub mod reach;
pub mod supervise;
pub mod waiter;
pub mod watch;
