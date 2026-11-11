//! Saving a change and reading its arguments, contexts and times.

use std::time::Duration;

use pumbo_common::text::{self, Args};
use pumbo_common::time::{self, format_duration_coarse};

use crate::context::{self, ContextError, Contexts};
use crate::engine::Engine;
use crate::node::{self, NodeError};
use crate::perms::{Dirty, PermsError};
use crate::store::LogEntry;
use crate::ui::{self, Text, value};

use super::{Outcome, Res, Who};

/// Saves a change, writes the log entry, gives the success reply.
pub(super) fn save(
    e: &mut Engine,
    who: &Who,
    dirty: Dirty,
    (kind, target, action): (&str, &str, String),
    reply: Text,
    now: u64,
) -> Res {
    let log = LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: kind.to_string(),
        target: target.to_string(),
        action,
    };
    match e.commit(&dirty, Some(&log)) {
        Ok(()) => Ok(Outcome { reply, changed: true, log: Some(log), action: None }),
        Err(err) => Err(ui::error(&e.lang, "save-failed", &Args::new().arg(value(err)))),
    }
}

pub(super) fn perms_error(e: &Engine, err: &PermsError) -> Text {
    let l = &e.lang;
    let one = |key: &str, v: &str| ui::error(l, key, &Args::new().arg(value(v)));
    match err {
        PermsError::GroupExists(g) => one("group-exists", g),
        PermsError::NoGroup(g) => one("group-not-found", g),
        PermsError::TrackExists(t) => one("track-exists", t),
        PermsError::NoTrack(t) => one("track-not-found", t),
        PermsError::BadName(n) => one("bad-name", n),
        PermsError::DefaultGroup => ui::error(l, "default-group", &Args::new()),
        PermsError::Cycle(g) => one("parent-cycle", g),
        PermsError::Unchanged => ui::error(l, "unchanged", &Args::new()),
        PermsError::Ambiguous(g) => one("track-ambiguous", &g.join(", ")),
        PermsError::TrackEnd(g) => one("track-end", g),
        PermsError::TrackStart(g) => one("track-start", g),
        PermsError::NotOnTrack => ui::error(l, "not-on-track", &Args::new()),
        PermsError::EmptyTrack(t) => one("track-empty", t),
        PermsError::OnTrack(g) => one("track-on", g),
    }
}

pub(super) fn parse_node(e: &Engine, raw: &str) -> Result<String, Text> {
    node::normalize(raw).map_err(|err| {
        let key = match err {
            NodeError::Empty => "bad-node-empty",
            NodeError::TooLong => "bad-node-long",
            NodeError::BadChar(_) => "bad-node-char",
            NodeError::EmptySegment => "bad-node-segment",
            NodeError::TwoNamespaces => "bad-node-namespace",
            NodeError::BadWildcard => "bad-node-wildcard",
        };
        ui::error(&e.lang, key, &Args::new().arg(value(raw)))
    })
}

pub(super) fn parse_bool(e: &Engine, raw: &str) -> Result<bool, Text> {
    match raw.to_lowercase().as_str() {
        "true" | "yes" | "allow" => Ok(true),
        "false" | "no" | "deny" => Ok(false),
        _ => Err(ui::error(&e.lang, "bad-value", &Args::new().arg(value(raw)))),
    }
}

pub(super) fn parse_int(e: &Engine, raw: &str) -> Result<i32, Text> {
    raw.parse::<i32>().map_err(|_| ui::error(&e.lang, "bad-number", &Args::new().arg(value(raw))))
}

pub(super) fn parse_duration(e: &Engine, raw: &str) -> Result<Duration, Text> {
    match time::parse_duration(raw) {
        Ok(d) if d.as_secs() > 0 => Ok(d),
        _ => Err(ui::error(&e.lang, "command-invalid-duration", &Args::new().arg(value(raw)))),
    }
}

pub(super) fn expiry(now: u64, d: Duration) -> u64 {
    now.saturating_add(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Positional arguments and the trailing `key=value` contexts; `world=` only
/// where the platform sees worlds.
pub(super) fn split_context(e: &Engine, args: &[String]) -> Result<(Vec<String>, Contexts), Text> {
    let (pos, contexts) = split_any_context(e, args)?;
    if !e.cfg.worlds && contexts.get(context::WORLD).is_some() {
        return Err(ui::error(&e.lang, "no-worlds", &Args::new()));
    }
    Ok((pos, contexts))
}

/// [`split_context`] that also takes `world=` where nobody sees worlds.
pub(super) fn split_any_context(e: &Engine, args: &[String]) -> Result<(Vec<String>, Contexts), Text> {
    let mut end = args.len();
    while end > 0 && args.get(end - 1).is_some_and(|a| context::is_context_arg(a)) {
        end -= 1;
    }
    let (pos, ctx) = args.split_at(end);
    let contexts = Contexts::parse(ctx).map_err(|err| {
        let (key, arg) = match err {
            ContextError::UnknownKey(k) => ("bad-context-key", k),
            ContextError::BadValue(v) => ("bad-context-value", v),
            ContextError::Repeated(k) => ("bad-context-repeated", k),
        };
        ui::error(&e.lang, key, &Args::new().arg(value(arg)))
    })?;
    Ok((pos.to_vec(), contexts))
}

/// ` in server=lobby` or nothing.
pub(super) fn in_context(e: &Engine, c: &Contexts) -> String {
    if c.is_global() { String::new() } else { e.lang.format("in-context", &Args::new().arg(value(c))) }
}

/// [`in_context`] in grey, for views.
pub(super) fn context_hint(e: &Engine, c: &Contexts) -> String {
    if c.is_global() {
        return String::new();
    }
    format!("{}{}&r", ui::INFO, e.lang.format("in-context", &Args::new().arg(text::escape(&c.to_string()))))
}

pub(super) fn coarse(d: Duration) -> String {
    format_duration_coarse(d, 2)
}

pub(super) fn left(now: u64, expiry: u64) -> String {
    coarse(Duration::from_millis(expiry.saturating_sub(now)))
}
