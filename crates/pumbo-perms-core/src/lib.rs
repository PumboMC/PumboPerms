//! PumboPerms core: groups, inheritance, permission decisions, meta and rank
//! tracks, without platform code.
//!
//! - [`node`]: node names, wildcards, the Pumpkin namespace spelling
//! - [`context`]: where a node applies (`server`, `group`, `world`)
//! - [`model`]: groups, users, tracks and their nodes
//! - [`perms`]: the database in memory and the decision rules
//! - [`store`]: saving it (redb or memory) with the change log
//! - [`engine`]: the database, store, config and messages together
//! - [`command`]: the `/pp` command
//! - [`export`]: JSON export and import
//! - [`ipc`]: questions from other plugins (JSON)
//! - [`host`]: setting decisions on a host that only stores exact nodes
//! - [`config`]: the `context`, `defaults` and `log` sections
//! - [`ui`]: what replies look like
//!
//! Time comes in as `now_ms` arguments. The platform layer asks [`perms::Perms`]
//! for decisions and runs the commands; it never decides anything itself.

pub mod command;
pub mod config;
pub mod context;
pub mod engine;
pub mod export;
pub mod host;
pub mod ipc;
pub mod merge;
pub mod model;
pub mod node;
pub mod perms;
pub mod store;
pub mod ui;

use pumbo_common::lang::Bundle;

/// Messages of the permission logic and the `/pp` command.
pub const LANG: Bundle =
    Bundle { name: "perms", files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))] };

#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod tests;
