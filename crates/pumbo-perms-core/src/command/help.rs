//! Help pages of `/pp` and the tables of its subcommands.

use std::collections::HashSet;

use pumbo_common::command::permission;
use pumbo_common::help::{Entry, Help};
use pumbo_common::style;
use pumbo_common::text::Args;

use crate::engine::Engine;
use crate::model::MetaKind;
use crate::ui::{self, Line, Text};

use super::{ACTIONS, Outcome, PLUGIN, Res, Who, allowed, allowed_prefix, switched_off};

pub(super) const GENERAL: &[(&str, &str, &str)] = &[
    ("user <player>", "user.", "desc-user"),
    ("group <name>", "group.", "desc-group"),
    ("track <name>", "track.", "desc-track"),
    ("groups [page]", "group.list", "desc-groups"),
    ("tracks [page]", "track.list", "desc-tracks"),
    ("creategroup <name>", "group.create", "desc-group-create"),
    ("deletegroup <name>", "group.delete", "desc-group-delete"),
    ("createtrack <name>", "track.create", "desc-track-create"),
    ("deletetrack <name>", "track.delete", "desc-track-delete"),
    ("promote <player> <track> [context...]", "user.promote", "desc-promote"),
    ("demote <player> <track> [context...]", "user.demote", "desc-demote"),
    ("log [page]", "log", "desc-log"),
    ("export [file]", "export", "desc-export"),
    ("import <file>", "import", "desc-import"),
    ("import run|preview|undo <source>", "import", "desc-import-source"),
    ("reload", "reload", "desc-reload"),
    ("version", "version", "desc-version"),
];

/// Subcommands of a user or group: (arguments, action without the kind, description).
pub(super) const HOLDER: &[(&str, &str, &str)] = &[
    ("info [page]", "info", "desc-info"),
    ("permission info [page]", "permission.info", "desc-perm-info"),
    ("permission set <node> [true|false] [context...]", "permission.set", "desc-perm-set"),
    ("permission settemp <node> [true|false] <duration> [context...]", "permission.set", "desc-perm-settemp"),
    ("permission unset <node> [context...]", "permission.unset", "desc-perm-unset"),
    ("permission unsettemp <node> [context...]", "permission.unset", "desc-perm-unsettemp"),
    ("permission check <node> [context...]", "permission.check", "desc-perm-check"),
    ("permission clear [context...]", "permission.clear", "desc-perm-clear"),
    ("parent info [page]", "parent.info", "desc-parent-info"),
    ("parent add <group> [context...]", "parent.add", "desc-parent-add"),
    ("parent addtemp <group> <duration> [context...]", "parent.add", "desc-parent-addtemp"),
    ("parent remove <group> [context...]", "parent.remove", "desc-parent-remove"),
    ("parent removetemp <group> [context...]", "parent.remove", "desc-parent-removetemp"),
    ("parent set <group> [context...]", "parent.set", "desc-parent-set"),
    ("meta info [page]", "meta.info", "desc-meta-info"),
    ("meta set <key> <value> [context...]", "meta.set", "desc-meta-set"),
    ("meta unset <key> [context...]", "meta.unset", "desc-meta-unset"),
    ("meta setprefix <priority> <text> [context...]", "meta.set", "desc-meta-setprefix"),
    ("meta addprefix <priority> <text> [context...]", "meta.set", "desc-meta-addprefix"),
    ("meta addtempprefix <priority> <text> <duration> [context...]", "meta.set", "desc-meta-addtempprefix"),
    ("meta removeprefix <priority> [text] [context...]", "meta.unset", "desc-meta-removeprefix"),
    ("meta removetempprefix <priority> [text] [context...]", "meta.unset", "desc-meta-removetempprefix"),
    ("meta setsuffix <priority> <text> [context...]", "meta.set", "desc-meta-setsuffix"),
    ("meta addsuffix <priority> <text> [context...]", "meta.set", "desc-meta-addsuffix"),
    ("meta addtempsuffix <priority> <text> <duration> [context...]", "meta.set", "desc-meta-addtempsuffix"),
    ("meta removesuffix <priority> [text] [context...]", "meta.unset", "desc-meta-removesuffix"),
    ("meta removetempsuffix <priority> [text] [context...]", "meta.unset", "desc-meta-removetempsuffix"),
];

pub(super) const USER_ONLY: &[(&str, &str, &str)] = &[
    ("promote <track> [context...]", "promote", "desc-user-promote"),
    ("demote <track> [context...]", "demote", "desc-user-demote"),
    ("clear", "clear", "desc-user-clear"),
];

pub(super) const GROUP_ONLY: &[(&str, &str, &str)] = &[
    ("create", "create", "desc-group-create"),
    ("delete", "delete", "desc-group-delete"),
    ("rename <new-name>", "rename", "desc-group-rename"),
    ("setweight <weight>", "setweight", "desc-group-setweight"),
    ("setdisplayname <name|clear>", "setdisplayname", "desc-group-setdisplayname"),
    ("listmembers [page]", "listmembers", "desc-group-listmembers"),
];

pub(super) const TRACK: &[(&str, &str, &str)] = &[
    ("info", "track.info", "desc-track-info"),
    ("create", "track.create", "desc-track-create"),
    ("delete", "track.delete", "desc-track-delete"),
    ("append <group>", "track.edit", "desc-track-append"),
    ("insert <group> <position>", "track.edit", "desc-track-insert"),
    ("remove <group>", "track.edit", "desc-track-remove"),
    ("clear", "track.edit", "desc-track-clear"),
];

/// `parent` subcommands that change something: (word, action, temporary).
/// `removetemp` takes only the temporary parent.
pub(super) const PARENT_CHANGES: &[(&str, &str, bool)] = &[
    ("add", "add", false),
    ("addtemp", "add", true),
    ("remove", "remove", false),
    ("removetemp", "remove", true),
    ("set", "set", false),
];

/// `meta` subcommands of prefixes and suffixes: (word, verb, kind, temporary).
pub(super) const AFFIX_CHANGES: &[(&str, &str, MetaKind, bool)] = &[
    ("setprefix", "set", MetaKind::Prefix, false),
    ("addprefix", "add", MetaKind::Prefix, false),
    ("addtempprefix", "add", MetaKind::Prefix, true),
    ("removeprefix", "remove", MetaKind::Prefix, false),
    ("removetempprefix", "remove", MetaKind::Prefix, true),
    ("setsuffix", "set", MetaKind::Suffix, false),
    ("addsuffix", "add", MetaKind::Suffix, false),
    ("addtempsuffix", "add", MetaKind::Suffix, true),
    ("removesuffix", "remove", MetaKind::Suffix, false),
    ("removetempsuffix", "remove", MetaKind::Suffix, true),
];

/// `/pp user <player> info` -> (`/pp user <player> info`, ``); arguments start
/// at the first `<` or `[`.
fn split_usage(usage: &str) -> (String, String) {
    let words: Vec<&str> = usage.split_whitespace().collect();
    let n = words.iter().position(|w| w.starts_with('<') || w.starts_with('[')).unwrap_or(words.len());
    (words.get(..n).unwrap_or(&[]).join(" "), words.get(n..).unwrap_or(&[]).join(" "))
}

/// The permission of a help entry: `x.*` for an action prefix `x.`.
pub(super) fn node_of(action: &str) -> String {
    if action.ends_with('.') { format!("pumbo.{PLUGIN}.{action}*") } else { permission(PLUGIN, action) }
}

pub(super) fn entry(e: &Engine, usage: &str, action: &str, summary: &str) -> Entry {
    let details_key = format!("{summary}-details");
    let details = if e.lang.has(&details_key) { e.lang.get(&details_key) } else { String::new() };
    let (command, args) = split_usage(usage);
    Entry::new(command, e.lang.get(summary)).args(args).details(details).permission(node_of(action))
}

/// The permission nodes the sender has among those of PumboPerms.
fn allowed_nodes(e: &mut Engine, who: &Who, now: u64) -> HashSet<String> {
    ACTIONS.iter().filter(|a| allowed(e, who, a, now)).map(|a| permission(PLUGIN, a)).collect()
}

/// Whether the sender may use an entry with this permission ([`node_of`]).
pub(super) fn rights(e: &mut Engine, who: &Who, now: u64) -> impl Fn(&str) -> bool + use<> {
    let nodes = allowed_nodes(e, who, now);
    move |n: &str| match n.strip_suffix('*') {
        Some(prefix) => nodes.iter().any(|a| a.starts_with(prefix)),
        None => nodes.contains(n),
    }
}

fn show_help(e: &mut Engine, who: &Who, help: Help, page: usize, now: u64) -> Text {
    let check = rights(e, who, now);
    // Every command in one reply; the views keep their pages.
    let help = help
        .per_page(usize::MAX)
        .tag(style::prefix(&e.lang), Line::parse(&format!("{}> ", ui::MUTED)))
        .whole_descriptions()
        .framed();
    if who.is_console() { help.console(&e.lang, check) } else { help.chat(&e.lang, page, check) }
}

pub(super) fn general_help(e: &mut Engine, who: &Who, page: usize, now: u64) -> Text {
    let mut help = Help::new("PumboPerms", style::accent::PERMS, format!("{} help", e.root))
        .version(env!("CARGO_PKG_VERSION"))
        .root(e.root.clone())
        .section(e.lang.get("help-section"));
    for (u, a, d) in GENERAL.iter().filter(|(u, ..)| !switched_off(e, u.split(' ').next().unwrap_or_default())) {
        help.push(entry(e, &format!("{} {u}", e.root), a, d));
    }
    show_help(e, who, help, page, now)
}

/// The words of the player and group help that stand for their own list of
/// commands (`/pp user <player> permission`), with the
/// message that describes each.
const SUBGROUPS: &[(&str, &str)] =
    &[("permission", "desc-perm-group"), ("parent", "desc-parent-group"), ("meta", "desc-meta-group")];

/// `base` is the command of the player or group: `/pp user Steve`, or
/// `/pp user <player>` when no name was given.
pub(super) fn holder_help(
    e: &mut Engine,
    who: &Who,
    base: &str,
    user: bool,
    filter: &str,
    page: usize,
    now: u64,
) -> Text {
    let base = base.to_string();
    let kind = if user { "user" } else { "group" };
    let extra = if user { USER_ONLY } else { GROUP_ONLY };
    let help_command = if filter.is_empty() { format!("{base} help") } else { format!("{base} {filter} help") };
    // The player or group help lists `permission ...`, `parent ...` and
    // `meta ...`; each of those lists its own commands under its root. The
    // root names the player or group, so the header has no section, which
    // keeps it within the chat width.
    let root = if filter.is_empty() { base.clone() } else { format!("{base} {filter}") };
    let mut help = Help::new("PumboPerms", style::accent::PERMS, help_command).root(root);
    let mut grouped: Vec<&str> = Vec::new();
    for (u, a, d) in HOLDER.iter().chain(extra.iter()) {
        let word = u.split(' ').next().unwrap_or_default();
        match SUBGROUPS.iter().find(|(w, _)| *w == word) {
            Some((w, desc)) if filter.is_empty() => {
                if !grouped.contains(w) {
                    grouped.push(w);
                    let node = node_of(&format!("{kind}.{w}."));
                    // Under `/pp user <player>` a click types `/pp user `, as on the other lines.
                    let (command, args) = split_usage(&format!("{base} {w}"));
                    let args = if args.is_empty() { "...".to_string() } else { format!("{args} ...") };
                    help.push(Entry::new(command, e.lang.get(desc)).args(args).permission(node));
                }
            }
            _ if filter.is_empty() || word == filter => {
                help.push(entry(e, &format!("{base} {u}"), &format!("{kind}.{a}"), d));
            }
            _ => {}
        }
    }
    show_help(e, who, help, page, now)
}

/// `/pp user`, `/pp group` or `/pp track` without a name: its commands, with
/// `<player>` or `<name>` in place of the name.
pub(super) fn unnamed_help(e: &mut Engine, who: &Who, kind: &str, now: u64) -> Res {
    if !allowed_prefix(e, who, &format!("{kind}."), now) {
        return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
    }
    let help = match kind {
        "track" => track_help(e, who, "<name>", 1, now),
        _ => {
            let user = kind == "user";
            let base = format!("{} {kind} {}", e.root, if user { "<player>" } else { "<name>" });
            holder_help(e, who, &base, user, "", 1, now)
        }
    };
    Ok(Outcome::reply(help))
}

pub(super) fn track_help(e: &mut Engine, who: &Who, track: &str, page: usize, now: u64) -> Text {
    let base = format!("{} track {track}", e.root);
    let section = e.lang.format("title-track", &Args::new().arg(track));
    let mut help =
        Help::new("PumboPerms", style::accent::PERMS, format!("{base} help")).root(base.clone()).section(section);
    for (u, a, d) in TRACK {
        help.push(entry(e, &format!("{base} {u}"), a, d));
    }
    show_help(e, who, help, page, now)
}
