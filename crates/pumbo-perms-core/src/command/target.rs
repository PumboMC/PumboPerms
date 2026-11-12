//! The player or group a command acts on, and checks of its texts and keys.

use pumbo_common::id::is_java_name;
use pumbo_common::text::Args;

use crate::engine::Engine;
use crate::model::{Data, is_valid_meta_key, is_valid_text};
use crate::perms::{Dirty, KnownName, PermsError};
use crate::ui::{self, Text, value};

use super::Env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    User { uuid: String, name: String },
    Group { name: String },
}

impl Target {
    pub(super) fn is_user(&self) -> bool {
        matches!(self, Target::User { .. })
    }

    pub(super) fn kind(&self) -> &'static str {
        if self.is_user() { "user" } else { "group" }
    }

    /// The name as views show it ([`safe_name`]; a user without one: the UUID).
    pub(super) fn label(&self) -> String {
        match self {
            Target::User { uuid, name } if name.is_empty() => uuid.clone(),
            Target::User { name, .. } => safe_name(name),
            Target::Group { name } => name.clone(),
        }
    }

    pub(super) fn command(&self, root: &str) -> String {
        match self {
            Target::User { uuid, name } => format!("{root} user {}", user_arg(uuid, name)),
            Target::Group { name } => format!("{root} group {name}"),
        }
    }

    /// The target inside a message: `Steve (<uuid>)` or `group vip`.
    pub(super) fn shown(&self, e: &Engine) -> String {
        match self {
            Target::User { uuid, name } if name.is_empty() || name == uuid => value(uuid),
            Target::User { uuid, .. } => e.lang.format("target-user", &Args::new().arg(value(self.label())).arg(uuid)),
            Target::Group { name } => e.lang.format("target-group", &Args::new().arg(value(name))),
        }
    }

    pub(super) fn edit<R>(&self, e: &mut Engine, f: impl FnOnce(&mut Data) -> R) -> Result<(R, Dirty), PermsError> {
        match self {
            Target::User { uuid, name } => Ok(e.perms.edit_user(uuid, name, f)),
            Target::Group { name } => e.perms.edit_group(name, |g| f(&mut g.data)),
        }
    }

    pub(super) fn data<'a>(&self, e: &'a Engine) -> Option<&'a Data> {
        match self {
            Target::User { uuid, .. } => e.perms.user(uuid).map(|u| &u.data),
            Target::Group { name } => e.perms.group(name).map(|g| &g.data),
        }
    }
}

/// How a command names a player: the nickname when it can be a Java name,
/// otherwise the UUID (other names can hold spaces that split the command).
pub(super) fn user_arg(uuid: &str, name: &str) -> String {
    if is_java_name(name) { name.to_string() } else { uuid.to_string() }
}

/// A nickname as views show it: a name that cannot be a Java name with every
/// character outside printable ASCII escaped (`§`, spaces that look like one).
pub(super) fn safe_name(name: &str) -> String {
    if is_java_name(name) {
        return name.to_string();
    }
    name.chars().map(|c| if c.is_ascii_graphic() { c.to_string() } else { c.escape_unicode().to_string() }).collect()
}

/// A prefix, suffix, display name or meta value from a command.
pub(super) fn checked_text(e: &Engine, raw: &str) -> Result<(), Text> {
    if is_valid_text(raw) { Ok(()) } else { Err(ui::error(&e.lang, "bad-text", &Args::new())) }
}

/// A new meta key: 1-64 characters a-z, 0-9, `_`, `.` and `-`.
pub(super) fn is_meta_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'))
}

/// A meta key that can be stored; `usage` is the reply for an empty key or one with spaces.
pub(super) fn checked_key(e: &Engine, key: &str, usage: &str) -> Result<(), Text> {
    checked_text(e, key)?;
    if is_valid_meta_key(key) { Ok(()) } else { Err(ui::usage(&e.lang, usage)) }
}

pub(super) fn find_user(e: &Engine, env: &Env, name: &str) -> Result<KnownName, Text> {
    if let Some(k) = (env.online)(name) {
        return Ok(k);
    }
    e.find_user(name).ok_or_else(|| ui::error(&e.lang, "command-player-not-found", &Args::new().arg(value(name))))
}
