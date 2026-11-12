//! Tab completion and the command tree for platforms that register it.

use pumbo_common::command::permission;
use pumbo_common::pumpkin::{COMMAND_NODES, PUMBO_NODES};

use crate::context;
use crate::engine::Engine;
use crate::perms::KnownName;

use super::help::{GENERAL, GROUP_ONLY, HOLDER, TRACK, USER_ONLY, node_of, rights};
use super::target::user_arg;
use super::{ACTIONS, PLUGIN, Who, allowed, subcommand, switched_off};

/// Other names of words in the usage lines below the first.
const WORD_ALIASES: &[(&str, &[&str])] =
    &[("permission", &["perm", "p", "permissions"]), ("parent", &["parents", "inherit"]), ("info", &["i", "list"])];

/// Every usage line below the root with its action: the help tables.
pub(super) fn rows() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = GENERAL.iter().map(|(u, a, _)| (u.to_string(), a.to_string())).collect();
    for (kind, slot, extra) in [("user", "player", USER_ONLY), ("group", "name", GROUP_ONLY)] {
        rows.extend(
            HOLDER.iter().chain(extra).map(|(u, a, _)| (format!("{kind} <{slot}> {u}"), format!("{kind}.{a}"))),
        );
    }
    rows.extend(TRACK.iter().map(|(u, a, _)| (format!("track <name> {u}"), a.to_string())));
    rows
}

/// Every usage line below the root the sender may use: the help tables with
/// the help's own rights filter.
fn usage_lines(e: &mut Engine, who: &Who, now: u64) -> Vec<String> {
    let may = rights(e, who, now);
    let mut lines = vec!["help".to_string()];
    for (usage, action) in rows() {
        if may(&node_of(&action)) && !switched_off(e, usage.split(' ').next().unwrap_or_default()) {
            lines.push(usage);
        }
    }
    lines
}

/// Every command below the root, for a platform that registers the whole
/// command tree: the usage lines without the subcommands `commands.disabled`
/// switches off, and each level that lists its commands, alone, with
/// `help [page]` (the page arrows of a help run that) and with `[args...]`,
/// which takes any other word with the rest of the line, so other names
/// (`u`, `perm`, `?`), words in upper case and `editor` reach [`run`] as
/// they do on PumboProx, and Tab offers only the words of the help.
pub fn tree_lines(e: &Engine) -> Vec<String> {
    let mut levels = vec![String::new(), "track <name>".to_string()];
    for holder in ["user <player>", "group <name>"] {
        levels.push(holder.to_string());
        levels.extend(["permission", "parent", "meta"].map(|sub| format!("{holder} {sub}")));
    }
    let mut lines: Vec<String> = Vec::new();
    for level in levels {
        lines.push(format!("{level} help [page]").trim_start().to_string());
        lines.push(format!("{level} [args...]").trim_start().to_string());
        lines.push(level);
    }
    lines.extend(rows().into_iter().map(|(usage, _)| usage));
    lines.retain(|l| !l.is_empty() && !switched_off(e, l.split(' ').next().unwrap_or_default()));
    lines
}

/// Whether the typed word `w` (lower case) at position `i` is the literal `p`
/// (`a|b` for either).
fn word_matches(p: &str, w: &str, i: usize) -> bool {
    p.split('|').any(|alt| {
        alt == w
            || if i == 0 {
                subcommand(w) == Some(alt)
            } else {
                WORD_ALIASES.iter().any(|(name, other)| *name == alt && other.contains(&w))
            }
    })
}

/// Whether `p` is an optional choice (`[true|false]`) that `w` is not one of,
/// so the word belongs to the slot after it.
fn skips_choice(p: &str, w: &str) -> bool {
    p.strip_prefix('[')
        .and_then(|c| c.strip_suffix(']'))
        .is_some_and(|c| c.contains('|') && !c.split('|').any(|alt| alt.eq_ignore_ascii_case(w)))
}

/// The pattern words that may follow `typed` in a usage line (more than one
/// after an optional `[slot]`); empty when the line does not match.
fn next_words<'a>(pattern: &[&'a str], typed: &[String]) -> Vec<&'a str> {
    let mut i = 0;
    for w in typed {
        while pattern.get(i).is_some_and(|p| skips_choice(p, w)) {
            i += 1;
        }
        let Some(p) = pattern.get(i) else { return Vec::new() };
        if p.ends_with("...]") {
            return vec![p];
        }
        let slot = p.starts_with('<') || p.starts_with('[');
        if !slot && !word_matches(p, &w.to_lowercase(), i) {
            return Vec::new();
        }
        i += 1;
    }
    let mut out = Vec::new();
    for p in pattern.iter().skip(i) {
        out.push(*p);
        if !p.starts_with('[') {
            break;
        }
    }
    out
}

/// Completions for the last argument from the usage lines the sender may
/// use; online players by [`user_arg`]. Players, groups and tracks only with
/// the `info` right of their kind.
pub fn complete(e: &mut Engine, who: &Who, args: &[String], online: &[KnownName], now: u64) -> Vec<String> {
    let typed = args.last().map(|s| s.to_lowercase()).unwrap_or_default();
    let before = args.get(..args.len().saturating_sub(1)).unwrap_or(&[]);
    let lines = usage_lines(e, who, now);
    let [users, groups, tracks] = ["user", "group", "track"].map(|k| allowed(e, who, &format!("{k}.info"), now));
    let e = &*e;
    let values = |slot: &str, first: &str| -> Vec<String> {
        match (slot, first) {
            ("player", _) if users => online.iter().map(|k| user_arg(&k.uuid, &k.name)).collect(),
            ("group", _) | ("name", "group" | "deletegroup") if groups => {
                e.perms.groups().map(|g| g.name.clone()).collect()
            }
            ("track", _) | ("name", "track" | "deletetrack") if tracks => {
                e.perms.tracks().map(|t| t.name.clone()).collect()
            }
            ("node", _) => {
                let catalog = COMMAND_NODES.iter().chain(PUMBO_NODES).map(|n| n.to_string());
                e.perms.all_nodes().iter().cloned().chain(catalog).chain(own_nodes()).collect()
            }
            ("true|false", _) => vec!["true".into(), "false".into()],
            ("context...", _) => context::KEYS.iter().map(|k| format!("{k}=")).collect(),
            _ => Vec::new(),
        }
    };
    let mut options: Vec<String> = Vec::new();
    for line in &lines {
        let pattern: Vec<&str> = line.split(' ').collect();
        let first = pattern.first().copied().unwrap_or_default();
        for p in next_words(&pattern, before) {
            match p.strip_prefix('<').or_else(|| p.strip_prefix('[')) {
                Some(slot) => options.extend(values(slot.trim_end_matches(['>', ']']), first)),
                None => options.extend(p.split('|').map(String::from)),
            }
        }
    }
    options.sort();
    options.dedup();
    options.retain(|o| o.to_lowercase().starts_with(&typed));
    options.truncate(100);
    options
}

/// Every permission node of PumboPerms (`pumbo.perms.<action>`).
pub fn own_nodes() -> Vec<String> {
    ACTIONS.iter().map(|a| permission(PLUGIN, a)).collect()
}
