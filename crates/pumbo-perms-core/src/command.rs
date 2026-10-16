//! The `/pp` command (`/pumboperms`): parsing, permission checks, changes,
//! replies.
//!
//! The platform passes the arguments, who sent them and a few things only it
//! knows (online players, the time); it gets back the reply, whether the data
//! changed (so it updates online players), the log entry (for notifications)
//! and actions that need files (reload, export, import).

use std::collections::HashSet;
use std::time::Duration;

use pumbo_common::command::permission;
use pumbo_common::id::is_java_name;
use pumbo_common::lang::Lang;
use pumbo_common::pumpkin::{COMMAND_NODES, PUMBO_NODES};
use pumbo_common::style;
use pumbo_common::text::{self, Args, Style};
use pumbo_common::time::{self, format_duration_coarse};

use crate::context::{self, ContextError, Contexts};
use crate::engine::Engine;
use crate::export::Snapshot;
use crate::merge::{self, Ledger};
use crate::model::{Data, MetaKind, ParentNode, PermNode, is_valid_meta_key, is_valid_text};
use crate::node::{self, NodeError};
use crate::perms::{Dirty, KnownName, PermsError, Source};
use crate::store::LogEntry;
use pumbo_common::help::{Entry, Help};

use crate::ui::{self, Click, Line, Text, row, value};

/// Plugin id: permissions are `pumbo.perms.<action>`.
pub const PLUGIN: &str = "perms";
/// The command shown in help and usage lines unless the platform registered
/// another one ([`Engine::root`]).
pub const ROOT: &str = "/pp";
/// Whether a player sees `/pp` at all: set for everyone with any `/pp` right.
pub const COMMAND_NODE: &str = "pumbo.perms.command";

/// Every action; the permission of each is `pumbo.perms.<action>`.
pub const ACTIONS: &[&str] = &[
    "version",
    "reload",
    "log",
    "log.notify",
    "export",
    "import",
    "editor",
    "user.info",
    "user.permission.info",
    "user.permission.set",
    "user.permission.unset",
    "user.permission.check",
    "user.permission.clear",
    "user.parent.info",
    "user.parent.add",
    "user.parent.remove",
    "user.parent.set",
    "user.meta.info",
    "user.meta.set",
    "user.meta.unset",
    "user.promote",
    "user.demote",
    "user.clear",
    "group.list",
    "group.info",
    "group.create",
    "group.delete",
    "group.rename",
    "group.setweight",
    "group.setdisplayname",
    "group.listmembers",
    "group.permission.info",
    "group.permission.set",
    "group.permission.unset",
    "group.permission.check",
    "group.permission.clear",
    "group.parent.info",
    "group.parent.add",
    "group.parent.remove",
    "group.parent.set",
    "group.meta.info",
    "group.meta.set",
    "group.meta.unset",
    "track.list",
    "track.info",
    "track.create",
    "track.delete",
    "track.edit",
];

/// First words after the root that `commands.disabled` can switch off, with
/// the other names each one answers to.
const SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("user", &["u"]),
    ("group", &["g"]),
    ("track", &["t"]),
    ("groups", &["listgroups"]),
    ("tracks", &["listtracks"]),
    ("creategroup", &[]),
    ("deletegroup", &[]),
    ("createtrack", &[]),
    ("deletetrack", &[]),
    ("promote", &[]),
    ("demote", &[]),
    ("log", &[]),
    ("export", &[]),
    ("import", &[]),
    ("editor", &[]),
    ("reload", &[]),
    ("version", &["info"]),
];

/// The main name of a first word (`u` -> `user`); `None` for anything else.
pub fn subcommand(word: &str) -> Option<&'static str> {
    let word = word.to_lowercase();
    SUBCOMMANDS.iter().find(|(name, other)| *name == word || other.contains(&word.as_str())).map(|(name, _)| *name)
}

fn switched_off(e: &Engine, word: &str) -> bool {
    subcommand(word).is_some_and(|main| e.cfg.disabled.iter().any(|d| d == main))
}

/// Log entries on one page.
const LOG_PAGE: usize = 10;

/// Who runs a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    pub name: String,
    /// `None` for the console, which may do everything.
    pub uuid: Option<String>,
    pub op_level: u8,
    pub world: String,
    /// The server the sender is on (PumboProx); empty: this server.
    pub server: String,
}

impl Who {
    pub fn console() -> Self {
        Self { name: "Console".into(), uuid: None, op_level: 4, world: String::new(), server: String::new() }
    }

    pub fn player(name: &str, uuid: &str, op_level: u8, world: &str) -> Self {
        Self { name: name.into(), uuid: Some(uuid.into()), op_level, world: world.into(), server: String::new() }
    }

    pub fn is_console(&self) -> bool {
        self.uuid.is_none()
    }
}

/// What the platform knows.
pub struct Env<'a> {
    pub now: u64,
    /// An online player by nickname.
    pub online: &'a dyn Fn(&str) -> Option<KnownName>,
    /// Plugin version for `/pp version`.
    pub version: &'a str,
    /// Extra `label: value` rows for `/pp version` (platform, mode...).
    pub version_rows: Vec<(String, String)>,
}

/// Work the platform does after the command: it needs files or the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Reload,
    /// Write [`crate::export::dump`] to `exports/<name>.json`.
    Export(String),
    /// Read `exports/<name>.json` and call [`import`].
    Import(String),
    /// Ask the source and call [`import_source`] (`/pp import run|preview|undo <source>`).
    Source(String, SourceMode),
}

/// What to do with another permission source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    Run,
    Preview,
    Undo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub reply: Text,
    /// The data changed: online players need their permissions updated.
    pub changed: bool,
    /// The saved log entry (for notifications).
    pub log: Option<LogEntry>,
    pub action: Option<Action>,
}

impl Outcome {
    fn reply(reply: Text) -> Self {
        Self { reply, ..Self::default() }
    }
}

type Res = Result<Outcome, Text>;

/// Splits the argument string into words at ASCII spaces, as Minecraft and
/// Pumpkin do; `"..."` keeps spaces (`\"` is a quote, `\\` a backslash).
pub fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut had_quotes = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => {
                cur.extend(chars.next());
            }
            '"' => {
                quoted = !quoted;
                had_quotes = true;
            }
            ' ' if !quoted => {
                if !cur.is_empty() || had_quotes {
                    out.push(std::mem::take(&mut cur));
                }
                had_quotes = false;
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() || had_quotes {
        out.push(cur);
    }
    out
}

/// Runs `/pp <args>`.
pub fn run(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Outcome {
    let mut out = match dispatch(e, who, args, env) {
        Ok(o) => {
            e.end();
            o
        }
        Err(reply) => {
            // A refused change leaves nothing behind in memory.
            e.rollback();
            Outcome::reply(reply)
        }
    };
    if !who.is_console() {
        // In chat a view sits between two bars and every line starts with the
        // prefix (`[PP] ...`).
        out.reply = ui::tagged(&e.lang, ui::framed(&e.lang, std::mem::take(&mut out.reply)));
    }
    out
}

/// Whether the sender may do `action`.
pub fn allowed(e: &mut Engine, who: &Who, action: &str, now: u64) -> bool {
    match &who.uuid {
        None => true,
        Some(uuid) => {
            let q = e.place(&who.server, &who.world);
            e.decide_in(uuid, &permission(PLUGIN, action), &q, who.op_level, now) == Some(true)
        }
    }
}

/// Whether the sender may use at least one subcommand (sees `/pp` at all).
pub fn allowed_any(e: &mut Engine, who: &Who, now: u64) -> bool {
    ACTIONS.iter().any(|a| allowed(e, who, a, now))
}

fn allowed_prefix(e: &mut Engine, who: &Who, prefix: &str, now: u64) -> bool {
    ACTIONS.iter().filter(|a| a.starts_with(prefix)).any(|a| allowed(e, who, a, now))
}

fn need(e: &mut Engine, who: &Who, action: &str, now: u64) -> Result<(), Text> {
    if allowed(e, who, action, now) { Ok(()) } else { Err(ui::error(&e.lang, "command-no-permission", &Args::new())) }
}

fn lower(s: Option<&String>) -> String {
    s.map(|s| s.to_lowercase()).unwrap_or_default()
}

fn dispatch(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let sub = lower(args.first());
    let rest = args.get(1..).unwrap_or(&[]);
    if switched_off(e, &sub) {
        return Err(ui::unknown(&e.lang, &sub, &format!("{} help", e.root)));
    }
    match sub.as_str() {
        "" | "help" | "?" => Ok(Outcome::reply(general_help(e, who, page_of(rest), now))),
        "version" | "info" => {
            need(e, who, "version", now)?;
            Ok(Outcome::reply(version(e, env)))
        }
        "reload" => {
            need(e, who, "reload", now)?;
            Ok(Outcome { action: Some(Action::Reload), ..Outcome::default() })
        }
        "log" => {
            need(e, who, "log", now)?;
            Ok(Outcome::reply(log_view(e, page_of(rest), now)))
        }
        "export" => {
            need(e, who, "export", now)?;
            let name = match rest.first() {
                Some(n) => n.to_lowercase(),
                None => crate::export::default_file_name(now),
            };
            if !crate::export::is_valid_file_name(&name) {
                return Err(ui::error(&e.lang, "bad-file-name", &Args::new().arg(value(&name))));
            }
            Ok(Outcome { action: Some(Action::Export(name)), ..Outcome::default() })
        }
        "import" => {
            need(e, who, "import", now)?;
            let Some(name) = rest.first().map(|n| n.to_lowercase()) else {
                return Err(ui::usage(&e.lang, &format!("{} import <file> | run|preview|undo <source>", e.root)));
            };
            let mode = match name.as_str() {
                "run" => Some(SourceMode::Run),
                "preview" => Some(SourceMode::Preview),
                "undo" => Some(SourceMode::Undo),
                _ => None,
            };
            if let Some(mode) = mode {
                let Some(source) = rest.get(1).map(|n| n.to_lowercase()) else {
                    return Err(ui::usage(&e.lang, &format!("{} import run|preview|undo <source>", e.root)));
                };
                if !crate::export::is_valid_file_name(&source) {
                    return Err(ui::error(&e.lang, "bad-file-name", &Args::new().arg(value(&source))));
                }
                if mode != SourceMode::Preview {
                    writable(e)?;
                }
                return Ok(Outcome { action: Some(Action::Source(source, mode)), ..Outcome::default() });
            }
            // A whole file replaces every group and player at once.
            if !who.is_console() {
                return Err(ui::error(&e.lang, "import-file-console", &Args::new()));
            }
            if !crate::export::is_valid_file_name(&name) {
                return Err(ui::error(&e.lang, "bad-file-name", &Args::new().arg(value(&name))));
            }
            writable(e)?;
            Ok(Outcome { action: Some(Action::Import(name)), ..Outcome::default() })
        }
        "editor" => {
            need(e, who, "editor", now)?;
            Ok(Outcome::reply(ui::info(&e.lang, "editor-later", &Args::new())))
        }
        "groups" | "listgroups" => {
            need(e, who, "group.list", now)?;
            Ok(Outcome::reply(groups_view(e, view_page(who, rest))))
        }
        "tracks" | "listtracks" => {
            need(e, who, "track.list", now)?;
            Ok(Outcome::reply(tracks_view(e, view_page(who, rest))))
        }
        "creategroup" | "deletegroup" | "createtrack" | "deletetrack" => {
            let Some(name) = rest.first() else {
                return Err(ui::usage(&e.lang, &format!("{} {sub} <name>", e.root)));
            };
            let verb = if sub.starts_with("create") { "create" } else { "delete" };
            let inner = [name.clone(), verb.to_string()];
            if sub.ends_with("group") { group_cmd(e, who, &inner, env) } else { track_cmd(e, who, &inner, env) }
        }
        "promote" | "demote" => {
            let (Some(player), Some(track)) = (rest.first(), rest.get(1)) else {
                return Err(ui::usage(&e.lang, &format!("{} {sub} <player> <track> [context...]", e.root)));
            };
            let mut inner = vec![player.clone(), sub.clone(), track.clone()];
            inner.extend(rest.iter().skip(2).cloned());
            user_cmd(e, who, &inner, env)
        }
        "user" | "u" => user_cmd(e, who, rest, env),
        "group" | "g" => group_cmd(e, who, rest, env),
        "track" | "t" => track_cmd(e, who, rest, env),
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} help", e.root))),
    }
}

fn page_of(rest: &[String]) -> usize {
    rest.first().and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1)
}

/// The page of a view to show: every row (0) for the console, which cannot
/// click the page arrows.
fn view_page(who: &Who, rest: &[String]) -> usize {
    if who.is_console() { 0 } else { page_of(rest) }
}

/// Refuses a change while the database is broken; otherwise remembers the
/// data, so a change that cannot be saved is undone without reading the disk.
fn writable(e: &mut Engine) -> Result<(), Text> {
    if !e.writable() {
        return Err(ui::error(&e.lang, "database-broken", &Args::new()));
    }
    e.begin();
    Ok(())
}

// ----- help -----

const GENERAL: &[(&str, &str, &str)] = &[
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
const HOLDER: &[(&str, &str, &str)] = &[
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

const USER_ONLY: &[(&str, &str, &str)] = &[
    ("promote <track> [context...]", "promote", "desc-user-promote"),
    ("demote <track> [context...]", "demote", "desc-user-demote"),
    ("clear", "clear", "desc-user-clear"),
];

const GROUP_ONLY: &[(&str, &str, &str)] = &[
    ("create", "create", "desc-group-create"),
    ("delete", "delete", "desc-group-delete"),
    ("rename <new-name>", "rename", "desc-group-rename"),
    ("setweight <weight>", "setweight", "desc-group-setweight"),
    ("setdisplayname <name|clear>", "setdisplayname", "desc-group-setdisplayname"),
    ("listmembers [page]", "listmembers", "desc-group-listmembers"),
];

const TRACK: &[(&str, &str, &str)] = &[
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
const PARENT_CHANGES: &[(&str, &str, bool)] = &[
    ("add", "add", false),
    ("addtemp", "add", true),
    ("remove", "remove", false),
    ("removetemp", "remove", true),
    ("set", "set", false),
];

/// `meta` subcommands of prefixes and suffixes: (word, verb, kind, temporary).
const AFFIX_CHANGES: &[(&str, &str, MetaKind, bool)] = &[
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
fn node_of(action: &str) -> String {
    if action.ends_with('.') { format!("pumbo.{PLUGIN}.{action}*") } else { permission(PLUGIN, action) }
}

fn entry(e: &Engine, usage: &str, action: &str, summary: &str) -> Entry {
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
fn rights(e: &mut Engine, who: &Who, now: u64) -> impl Fn(&str) -> bool + use<> {
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

fn general_help(e: &mut Engine, who: &Who, page: usize, now: u64) -> Text {
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

fn holder_help(e: &mut Engine, who: &Who, t: &Target, filter: &str, page: usize, now: u64) -> Text {
    let base = t.command(&e.root);
    let kind = t.kind();
    let extra = if t.is_user() { USER_ONLY } else { GROUP_ONLY };
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
                    help.push(Entry::new(format!("{base} {w}"), e.lang.get(desc)).args("...").permission(node));
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

fn track_help(e: &mut Engine, who: &Who, track: &str, page: usize, now: u64) -> Text {
    let base = format!("{} track {track}", e.root);
    let section = e.lang.format("title-track", &Args::new().arg(track));
    let mut help =
        Help::new("PumboPerms", style::accent::PERMS, format!("{base} help")).root(base.clone()).section(section);
    for (u, a, d) in TRACK {
        help.push(entry(e, &format!("{base} {u}"), a, d));
    }
    show_help(e, who, help, page, now)
}

// ----- targets -----

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    User { uuid: String, name: String },
    Group { name: String },
}

impl Target {
    fn is_user(&self) -> bool {
        matches!(self, Target::User { .. })
    }

    fn kind(&self) -> &'static str {
        if self.is_user() { "user" } else { "group" }
    }

    /// The name as views show it ([`safe_name`]; a user without one: the UUID).
    fn label(&self) -> String {
        match self {
            Target::User { uuid, name } if name.is_empty() => uuid.clone(),
            Target::User { name, .. } => safe_name(name),
            Target::Group { name } => name.clone(),
        }
    }

    fn command(&self, root: &str) -> String {
        match self {
            Target::User { uuid, name } => format!("{root} user {}", user_arg(uuid, name)),
            Target::Group { name } => format!("{root} group {name}"),
        }
    }

    /// The target inside a message: `Steve (<uuid>)` or `group vip`.
    fn shown(&self, e: &Engine) -> String {
        match self {
            Target::User { uuid, name } if name.is_empty() || name == uuid => value(uuid),
            Target::User { uuid, .. } => e.lang.format("target-user", &Args::new().arg(value(self.label())).arg(uuid)),
            Target::Group { name } => e.lang.format("target-group", &Args::new().arg(value(name))),
        }
    }

    fn edit<R>(&self, e: &mut Engine, f: impl FnOnce(&mut Data) -> R) -> Result<(R, Dirty), PermsError> {
        match self {
            Target::User { uuid, name } => Ok(e.perms.edit_user(uuid, name, f)),
            Target::Group { name } => e.perms.edit_group(name, |g| f(&mut g.data)),
        }
    }

    fn data<'a>(&self, e: &'a Engine) -> Option<&'a Data> {
        match self {
            Target::User { uuid, .. } => e.perms.user(uuid).map(|u| &u.data),
            Target::Group { name } => e.perms.group(name).map(|g| &g.data),
        }
    }
}

/// How a command names a player: the nickname when it can be a Java name,
/// otherwise the UUID (other names can hold spaces that split the command).
fn user_arg(uuid: &str, name: &str) -> String {
    if is_java_name(name) { name.to_string() } else { uuid.to_string() }
}

/// A nickname as views show it: a name that cannot be a Java name with every
/// character outside printable ASCII escaped (`§`, spaces that look like one).
fn safe_name(name: &str) -> String {
    if is_java_name(name) {
        return name.to_string();
    }
    name.chars().map(|c| if c.is_ascii_graphic() { c.to_string() } else { c.escape_unicode().to_string() }).collect()
}

/// A prefix, suffix, display name or meta value from a command.
fn checked_text(e: &Engine, raw: &str) -> Result<(), Text> {
    if is_valid_text(raw) { Ok(()) } else { Err(ui::error(&e.lang, "bad-text", &Args::new())) }
}

/// A new meta key: 1-64 characters a-z, 0-9, `_`, `.` and `-`.
fn is_meta_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'))
}

/// A meta key that can be stored; `usage` is the reply for an empty key or one with spaces.
fn checked_key(e: &Engine, key: &str, usage: &str) -> Result<(), Text> {
    checked_text(e, key)?;
    if is_valid_meta_key(key) { Ok(()) } else { Err(ui::usage(&e.lang, usage)) }
}

fn find_user(e: &Engine, env: &Env, name: &str) -> Result<KnownName, Text> {
    if let Some(k) = (env.online)(name) {
        return Ok(k);
    }
    e.find_user(name).ok_or_else(|| ui::error(&e.lang, "command-player-not-found", &Args::new().arg(value(name))))
}

// ----- changes -----

/// Saves a change, writes the log entry, gives the success reply.
fn save(
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

fn perms_error(e: &Engine, err: &PermsError) -> Text {
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

fn parse_node(e: &Engine, raw: &str) -> Result<String, Text> {
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

fn parse_bool(e: &Engine, raw: &str) -> Result<bool, Text> {
    match raw.to_lowercase().as_str() {
        "true" | "yes" | "allow" => Ok(true),
        "false" | "no" | "deny" => Ok(false),
        _ => Err(ui::error(&e.lang, "bad-value", &Args::new().arg(value(raw)))),
    }
}

fn parse_int(e: &Engine, raw: &str) -> Result<i32, Text> {
    raw.parse::<i32>().map_err(|_| ui::error(&e.lang, "bad-number", &Args::new().arg(value(raw))))
}

fn parse_duration(e: &Engine, raw: &str) -> Result<Duration, Text> {
    match time::parse_duration(raw) {
        Ok(d) if d.as_secs() > 0 => Ok(d),
        _ => Err(ui::error(&e.lang, "command-invalid-duration", &Args::new().arg(value(raw)))),
    }
}

fn expiry(now: u64, d: Duration) -> u64 {
    now.saturating_add(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Positional arguments and the trailing `key=value` contexts; `world=` only
/// where the platform sees worlds.
fn split_context(e: &Engine, args: &[String]) -> Result<(Vec<String>, Contexts), Text> {
    let (pos, contexts) = split_any_context(e, args)?;
    if !e.cfg.worlds && contexts.get(context::WORLD).is_some() {
        return Err(ui::error(&e.lang, "no-worlds", &Args::new()));
    }
    Ok((pos, contexts))
}

/// [`split_context`] that also takes `world=` where nobody sees worlds.
fn split_any_context(e: &Engine, args: &[String]) -> Result<(Vec<String>, Contexts), Text> {
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
fn in_context(e: &Engine, c: &Contexts) -> String {
    if c.is_global() { String::new() } else { e.lang.format("in-context", &Args::new().arg(value(c))) }
}

/// [`in_context`] in grey, for views.
fn context_hint(e: &Engine, c: &Contexts) -> String {
    if c.is_global() {
        return String::new();
    }
    format!("{}{}&r", ui::INFO, e.lang.format("in-context", &Args::new().arg(text::escape(&c.to_string()))))
}

fn coarse(d: Duration) -> String {
    format_duration_coarse(d, 2)
}

fn left(now: u64, expiry: u64) -> String {
    coarse(Duration::from_millis(expiry.saturating_sub(now)))
}

// ----- user and group -----

fn user_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let Some(name) = args.first() else {
        if !allowed_prefix(e, who, "user.", env.now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Err(ui::usage(&e.lang, &format!("{} user <player> <subcommand>", e.root)));
    };
    let known = find_user(e, env, name)?;
    let t = Target::User { uuid: known.uuid, name: known.name };
    holder_cmd(e, who, &t, args.get(1..).unwrap_or(&[]), env)
}

fn group_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let Some(name) = args.first().map(|n| n.to_lowercase()) else {
        if !allowed_prefix(e, who, "group.", now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Err(ui::usage(&e.lang, &format!("{} group <name> <subcommand>", e.root)));
    };
    let sub = lower(args.get(1));
    let t = Target::Group { name: name.clone() };
    match sub.as_str() {
        "create" => {
            need(e, who, "group.create", now)?;
            writable(e)?;
            let dirty = e.perms.create_group(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-created", &Args::new().arg(value(&name)));
            return save(e, who, dirty, ("group", &name, "create".into()), reply, now);
        }
        "delete" => {
            need(e, who, "group.delete", now)?;
            writable(e)?;
            let dirty = e.perms.delete_group(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-deleted", &Args::new().arg(value(&name)));
            return save(e, who, dirty, ("group", &name, "delete".into()), reply, now);
        }
        _ => {}
    }
    if e.perms.group(&name).is_none() {
        return Err(perms_error(e, &PermsError::NoGroup(name)));
    }
    let rest = args.get(2..).unwrap_or(&[]);
    match sub.as_str() {
        "rename" => {
            need(e, who, "group.rename", now)?;
            writable(e)?;
            let Some(to) = rest.first().map(|n| n.to_lowercase()) else {
                return Err(ui::usage(&e.lang, &format!("{} rename <new-name>", t.command(&e.root))));
            };
            let dirty = e.perms.rename_group(&name, &to).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-renamed", &Args::new().arg(value(&name)).arg(value(&to)));
            save(e, who, dirty, ("group", &name, format!("rename {to}")), reply, now)
        }
        "setweight" => {
            need(e, who, "group.setweight", now)?;
            writable(e)?;
            let Some(raw) = rest.first() else {
                return Err(ui::usage(&e.lang, &format!("{} setweight <weight>", t.command(&e.root))));
            };
            let w = parse_int(e, raw)?;
            let ((), dirty) = e.perms.edit_group(&name, |g| g.weight = w).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-weight", &Args::new().arg(value(&name)).arg(value(w)));
            save(e, who, dirty, ("group", &name, format!("setweight {w}")), reply, now)
        }
        "setdisplayname" => {
            need(e, who, "group.setdisplayname", now)?;
            writable(e)?;
            if rest.is_empty() {
                return Err(ui::usage(&e.lang, &format!("{} setdisplayname <name|clear>", t.command(&e.root))));
            }
            let shown = rest.join(" ");
            checked_text(e, &shown)?;
            let clear = shown.eq_ignore_ascii_case("clear");
            let new = if clear { String::new() } else { shown.clone() };
            let ((), dirty) =
                e.perms.edit_group(&name, |g| g.display_name = new).map_err(|err| perms_error(e, &err))?;
            let reply = if clear {
                ui::success(&e.lang, "group-displayname-cleared", &Args::new().arg(value(&name)))
            } else {
                ui::success(&e.lang, "group-displayname", &Args::new().arg(value(&name)).arg(format!("{shown}&r")))
            };
            save(e, who, dirty, ("group", &name, format!("setdisplayname {shown}")), reply, now)
        }
        "listmembers" => {
            need(e, who, "group.listmembers", now)?;
            Ok(Outcome::reply(members_view(e, &name, page_of(rest))))
        }
        _ => holder_cmd(e, who, &t, args.get(1..).unwrap_or(&[]), env),
    }
}

fn holder_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let kind = t.kind();
    let sub = lower(args.first());
    let rest = args.get(1..).unwrap_or(&[]);
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, &format!("{kind}."), now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            Ok(Outcome::reply(holder_help(e, who, t, "", page_of(rest), now)))
        }
        "info" | "i" => {
            need(e, who, &format!("{kind}.info"), now)?;
            let page = match rest {
                [] => view_page(who, rest),
                [p] if p.parse::<usize>().is_ok() => view_page(who, rest),
                _ => return Err(ui::usage(&e.lang, &format!("{} info [page]", t.command(&e.root)))),
            };
            Ok(Outcome::reply(if t.is_user() { user_view(e, t, page, now) } else { group_view(e, t, page, now) }))
        }
        "permission" | "perm" | "p" | "permissions" => permission_cmd(e, who, t, rest, now),
        "parent" | "parents" | "inherit" => parent_cmd(e, who, t, rest, now),
        "meta" => meta_cmd(e, who, t, rest, now),
        "promote" | "demote" if t.is_user() => step_cmd(e, who, t, &sub, rest, now),
        "clear" if t.is_user() => {
            need(e, who, "user.clear", now)?;
            if !rest.is_empty() {
                return Err(ui::usage(&e.lang, &format!("{} clear", t.command(&e.root))));
            }
            writable(e)?;
            let ((), dirty) = t.edit(e, |d| *d = Data::default()).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "user-cleared", &Args::new().arg(t.shown(e)));
            save(e, who, dirty, (kind, &t.label(), "clear".into()), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} help", t.command(&e.root)))),
    }
}

fn permission_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let rest = args.get(1..).unwrap_or(&[]);
    let (pos, ctx) = if sub == "check" { split_any_context(e, rest)? } else { split_context(e, rest)? };
    let action_log = format!("permission {}", args.join(" "));
    let base = format!("{} permission", t.command(&e.root));
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, &format!("{kind}.permission."), now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            Ok(Outcome::reply(holder_help(e, who, t, "permission", page_of(&pos), now)))
        }
        "info" | "list" => {
            need(e, who, &format!("{kind}.permission.info"), now)?;
            Ok(Outcome::reply(nodes_view(e, t, view_page(who, &pos), now)))
        }
        "set" | "settemp" => {
            need(e, who, &format!("{kind}.permission.set"), now)?;
            writable(e)?;
            let temp = sub == "settemp";
            let usage = if temp {
                format!("{base} settemp <node> [true|false] <duration> [context...]")
            } else {
                format!("{base} set <node> [true|false] [context...]")
            };
            let Some(raw) = pos.first() else { return Err(ui::usage(&e.lang, &usage)) };
            let node = parse_node(e, raw)?;
            not_group_node(e, t, &node)?;
            let (value_arg, dur_arg) = match (temp, pos.len()) {
                (false, 1) => (None, None),
                (false, 2) => (pos.get(1), None),
                (true, 2) => (None, pos.get(1)),
                (true, 3) => (pos.get(1), pos.get(2)),
                _ => return Err(ui::usage(&e.lang, &usage)),
            };
            let v = match value_arg {
                Some(raw) => parse_bool(e, raw)?,
                None => true,
            };
            let dur = match dur_arg {
                Some(raw) => Some(parse_duration(e, raw)?),
                None => None,
            };
            // Changing an end time could cut a punishment short without a word.
            let timed = |p: &PermNode| p.node == node && p.context == ctx && p.expiry.is_some_and(|x| x > now);
            if temp && t.data(e).is_some_and(|d| d.permissions.iter().any(timed)) {
                let undo = format!("{base} unsettemp {node}{}", context_args(&ctx));
                return Err(ui::error(&e.lang, "perm-settemp-exists", &Args::new().arg(value(&node)).arg(value(undo))));
            }
            let new =
                PermNode { node: node.clone(), value: v, context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            let (_, dirty) = t.edit(e, |d| d.set_permission(new)).map_err(|err| perms_error(e, &err))?;
            let mut a = Args::new()
                .with("node", value(&node))
                .with("value", ui::boolean(v))
                .with("target", t.shown(e))
                .with("context", in_context(e, &ctx));
            let key = match dur {
                Some(d) => {
                    a.set("duration", value(coarse(d)));
                    "perm-settemp"
                }
                None => "perm-set",
            };
            let reply = ui::success(&e.lang, key, &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "unset" | "unsettemp" => {
            need(e, who, &format!("{kind}.permission.unset"), now)?;
            writable(e)?;
            let Some(raw) = pos.first() else {
                return Err(ui::usage(&e.lang, &format!("{base} {sub} <node> [context...]")));
            };
            let node = parse_node(e, raw)?;
            not_group_node(e, t, &node)?;
            let temp = sub == "unsettemp";
            let (removed, dirty) =
                t.edit(e, |d| d.unset_permission(&node, &ctx, temp)).map_err(|err| perms_error(e, &err))?;
            let a =
                Args::new().with("node", value(&node)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
            if removed.is_none() {
                e.rollback();
                return Err(ui::error(&e.lang, "perm-unset-none", &a));
            }
            let reply = ui::success(&e.lang, "perm-unset", &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "clear" => {
            need(e, who, &format!("{kind}.permission.clear"), now)?;
            writable(e)?;
            let only =
                if ctx.is_global() && !args.iter().any(|a| context::is_context_arg(a)) { None } else { Some(&ctx) };
            let (count, dirty) = t.edit(e, |d| d.clear_permissions(only)).map_err(|err| perms_error(e, &err))?;
            let a =
                Args::new().with("count", value(count)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
            let reply = ui::success(&e.lang, "perm-clear", &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "check" => {
            need(e, who, &format!("{kind}.permission.check"), now)?;
            let Some(raw) = pos.first() else {
                return Err(ui::usage(&e.lang, &format!("{base} check <node> [context...]")));
            };
            let node = parse_node(e, raw)?;
            let mut q = e.place(ctx.get(context::SERVER).unwrap_or(""), ctx.get(context::WORLD).unwrap_or(""));
            if let Some(g) = ctx.get(context::GROUP) {
                q.groups = vec![g.to_string()];
            }
            let decision = match t {
                Target::User { uuid, .. } => e.perms.explain(uuid, &node, &q, now),
                Target::Group { name } => e.perms.group_effective(name, &q, now).explain(&node).cloned(),
            };
            // A view: the player by name as in its info title (the UUID is in
            // `info`), so the lines fit the chat; a group as `group vip`.
            let target = if t.is_user() { value(t.label()) } else { t.shown(e) };
            let a = Args::new().with("node", value(&node)).with("target", target.clone());
            let mut reply = match decision {
                Some(d) => {
                    let mut text = ui::header(&e.lang, "perm-check", &a.clone().with("value", ui::boolean(d.value)));
                    let source = match &d.source {
                        Source::User => target.clone(),
                        Source::Group(g) => e.lang.format("target-group", &Args::new().arg(value(g))),
                    };
                    let mut how = e.lang.format(
                        "perm-check-source",
                        &Args::new()
                            .with("source", source)
                            .with("node", value(&d.node))
                            .with("context", context_hint(e, &d.context)),
                    );
                    if let Some(x) = d.expiry {
                        let ends = e.lang.format("expires-in", &Args::new().arg(left(now, x)));
                        how.push_str(&format!("{}{ends}&r", ui::INFO));
                    }
                    text.push(Line::parse_with(&format!("  {how}"), Style::colored(style::LABEL)));
                    text
                }
                None => ui::header(&e.lang, "perm-check-undecided", &a),
            };
            if let Target::User { uuid, .. } = t {
                let host = node::host_form(&node);
                let hidden = crate::host::hidden(e, uuid, &q, now);
                hidden_lines(e, hidden.iter().filter(|(w, _)| node::covers(w, &host)), &mut reply.lines);
            }
            if !e.cfg.worlds && ctx.get(context::WORLD).is_some() {
                reply.push(ui::row(1, "", &format!("{}! {}", ui::WARN, e.lang.get("perm-check-no-worlds"))));
            }
            Ok(Outcome::reply(reply))
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    }
}

/// `group.<name>` tells membership: it is given with `parent add`.
fn not_group_node(e: &Engine, t: &Target, node: &str) -> Result<(), Text> {
    let Some(group) = node.strip_prefix(node::GROUP_PREFIX) else { return Ok(()) };
    let a =
        Args::new().with("node", value(node)).with("command", value(t.command(&e.root))).with("group", value(group));
    Err(ui::error(&e.lang, "perm-group-node", &a))
}

/// "not sent to Pumpkin: <wildcard> hides <narrower>" ([`crate::host::hidden`]).
fn hidden_lines<'a>(e: &Engine, hidden: impl Iterator<Item = &'a (String, String)>, out: &mut Vec<Line>) {
    for (w, narrower) in hidden {
        let line = e.lang.format("perm-hidden", &Args::new().arg(value(w)).arg(value(narrower)));
        out.push(Line::parse_with(&format!("  ! {line}"), Style::colored(style::WARN)));
    }
}

fn parent_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("parent {}", args.join(" "));
    let base = format!("{} parent", t.command(&e.root));
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.parent."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Ok(Outcome::reply(holder_help(e, who, t, "parent", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.parent.info"), now)?;
        return Ok(Outcome::reply(parents_view(e, t, view_page(who, &pos), now)));
    }
    let Some(&(_, action, temp)) = PARENT_CHANGES.iter().find(|(w, _, _)| *w == sub) else {
        return Err(ui::unknown(&e.lang, &sub, &format!("{base} help")));
    };
    need(e, who, &format!("{kind}.parent.{action}"), now)?;
    writable(e)?;
    let adds_temp = temp && action == "add";
    let usage = if adds_temp {
        format!("{base} addtemp <group> <duration> [context...]")
    } else {
        format!("{base} {sub} <group> [context...]")
    };
    let Some(group) = pos.first().map(|g| g.to_lowercase()) else { return Err(ui::usage(&e.lang, &usage)) };
    let dur = if adds_temp {
        match pos.get(1) {
            Some(raw) => Some(parse_duration(e, raw)?),
            None => return Err(ui::usage(&e.lang, &usage)),
        }
    } else {
        None
    };
    if action != "remove" && e.perms.group(&group).is_none() {
        return Err(perms_error(e, &PermsError::NoGroup(group)));
    }
    let a = Args::new().with("group", value(&group)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
    let (key, dirty) = match (action, t) {
        ("add", Target::Group { name }) => {
            let p = ParentNode { group: group.clone(), context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            match e.perms.add_group_parent(name, p) {
                Ok(d) => (if adds_temp { "parent-addtemp" } else { "parent-add" }, d),
                Err(PermsError::Unchanged) => return Err(ui::error(&e.lang, "parent-already", &a)),
                Err(err) => return Err(perms_error(e, &err)),
            }
        }
        ("add", Target::User { .. }) => {
            let p = ParentNode { group: group.clone(), context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            let (added, d) = t.edit(e, |data| data.add_parent(p)).map_err(|err| perms_error(e, &err))?;
            if !added {
                e.rollback();
                return Err(ui::error(&e.lang, "parent-already", &a));
            }
            (if adds_temp { "parent-addtemp" } else { "parent-add" }, d)
        }
        ("remove", _) => {
            let (removed, d) =
                t.edit(e, |data| data.remove_parent(&group, &ctx, temp)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.rollback();
                return Err(ui::error(&e.lang, "parent-remove-none", &a));
            }
            ("parent-remove", d)
        }
        _ => {
            if let Target::Group { name } = t
                && e.perms.would_cycle(name, &group)
            {
                return Err(perms_error(e, &PermsError::Cycle(group)));
            }
            let ((), d) = t.edit(e, |data| data.set_parent(&group, &ctx)).map_err(|err| perms_error(e, &err))?;
            ("parent-set", d)
        }
    };
    let a = match dur {
        Some(d) => a.with("duration", value(coarse(d))),
        None => a,
    };
    let reply = ui::success(&e.lang, key, &a);
    save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
}

fn meta_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("meta {}", args.join(" "));
    let base = format!("{} meta", t.command(&e.root));
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.meta."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Ok(Outcome::reply(holder_help(e, who, t, "meta", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.meta.info"), now)?;
        return Ok(Outcome::reply(meta_view(e, t, view_page(who, &pos), now)));
    }
    let affix = AFFIX_CHANGES.iter().find(|(w, ..)| *w == sub).map(|&(_, verb, kind, temp)| (verb, kind, temp));
    if affix.is_none() && sub != "set" && sub != "unset" {
        return Err(ui::unknown(&e.lang, &sub, &format!("{base} help")));
    }
    let removing = sub == "unset" || affix.is_some_and(|(verb, ..)| verb == "remove");
    need(e, who, &format!("{kind}.meta.{}", if removing { "unset" } else { "set" }), now)?;
    writable(e)?;
    let mut a = Args::new().with("target", t.shown(e)).with("context", in_context(e, &ctx));
    let (key, dirty) = match (sub.as_str(), affix) {
        ("set", _) => {
            let (Some(k), Some(v)) = (pos.first(), pos.get(1)) else {
                return Err(ui::usage(&e.lang, &format!("{base} set <key> <value> [context...]")));
            };
            let k = k.to_lowercase();
            checked_key(e, &k, &format!("{base} set <key> <value> [context...]"))?;
            if !is_meta_key(&k) {
                return Err(ui::error(&e.lang, "bad-meta-key", &Args::new().arg(value(text::escape(&k)))));
            }
            let v = pos.get(1..).map(|p| p.join(" ")).unwrap_or_else(|| v.clone());
            checked_text(e, &v)?;
            a = a.with("key", value(&k)).with("value", value(&v));
            let ((), d) = t.edit(e, |data| data.set_meta(&k, &v, &ctx, None)).map_err(|err| perms_error(e, &err))?;
            ("meta-set", d)
        }
        ("unset", _) => {
            let (Some(k), 1) = (pos.first().map(|k| k.to_lowercase()), pos.len()) else {
                return Err(ui::usage(&e.lang, &format!("{base} unset <key> [context...]")));
            };
            checked_key(e, &k, &format!("{base} unset <key> [context...]"))?;
            a = a.with("key", value(&k));
            let (removed, d) = t.edit(e, |data| data.unset_meta(&k, &ctx)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.rollback();
                return Err(ui::error(&e.lang, "meta-unset-none", &a));
            }
            ("meta-unset", d)
        }
        (s, Some((verb, mk, temp))) => {
            let which = if mk == MetaKind::Prefix { "prefix" } else { "suffix" };
            a = a.with("kind", e.lang.get(&format!("kind-{which}")));
            if verb == "remove" {
                let Some(raw) = pos.first() else {
                    return Err(ui::usage(&e.lang, &format!("{base} {s} <priority> [text] [context...]")));
                };
                let prio = parse_int(e, raw)?;
                let text_arg = pos.get(1).cloned();
                a = a.with("priority", value(prio));
                let (count, d) = t
                    .edit(e, |data| data.remove_affix(mk, prio, text_arg.as_deref(), &ctx, temp))
                    .map_err(|err| perms_error(e, &err))?;
                if count == 0 {
                    e.rollback();
                    return Err(ui::error(&e.lang, "affix-remove-none", &a));
                }
                a = a.with("count", value(count));
                ("affix-remove", d)
            } else {
                let replace = verb == "set";
                let usage = if temp {
                    format!("{base} {s} <priority> <text> <duration> [context...]")
                } else {
                    format!("{base} {s} <priority> <text> [context...]")
                };
                let (Some(raw), Some(txt)) = (pos.first(), pos.get(1)) else { return Err(ui::usage(&e.lang, &usage)) };
                let prio = parse_int(e, raw)?;
                checked_text(e, txt)?;
                let dur = if temp {
                    match pos.get(2) {
                        Some(d) => Some(parse_duration(e, d)?),
                        None => return Err(ui::usage(&e.lang, &usage)),
                    }
                } else if pos.len() > 2 {
                    return Err(ui::usage(&e.lang, &usage));
                } else {
                    None
                };
                a = a.with("priority", value(prio)).with("text", format!("{}&r", txt));
                let exp = dur.map(|d| expiry(now, d));
                let (changed, d) = t
                    .edit(e, |data| data.add_affix(mk, prio, txt, &ctx, exp, replace))
                    .map_err(|err| perms_error(e, &err))?;
                if !changed {
                    e.rollback();
                    return Err(ui::error(&e.lang, "unchanged", &Args::new()));
                }
                if let Some(d) = dur {
                    a = a.with("duration", value(coarse(d)));
                }
                (
                    if replace {
                        "affix-set"
                    } else if temp {
                        "affix-addtemp"
                    } else {
                        "affix-add"
                    },
                    d,
                )
            }
        }
        _ => return Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    };
    let reply = ui::success(&e.lang, key, &a);
    save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
}

fn step_cmd(e: &mut Engine, who: &Who, t: &Target, sub: &str, args: &[String], now: u64) -> Res {
    need(e, who, &format!("user.{sub}"), now)?;
    writable(e)?;
    let (pos, ctx) = split_context(e, args)?;
    let Some(track) = pos.first().map(|t| t.to_lowercase()) else {
        return Err(ui::usage(&e.lang, &format!("{} {sub} <track> [context...]", t.command(&e.root))));
    };
    let Target::User { uuid, name } = t else {
        return Err(ui::unknown(&e.lang, sub, &format!("{} help", e.root)));
    };
    let up = sub == "promote";
    let a = Args::new().with("target", t.shown(e)).with("track", value(&track)).with("context", in_context(e, &ctx));
    let result =
        if up { e.perms.promote(uuid, name, &track, &ctx, now) } else { e.perms.demote(uuid, name, &track, &ctx, now) };
    let (step, dirty) = match result {
        Ok(r) => r,
        Err(PermsError::TrackEnd(g)) => return Err(ui::error(&e.lang, "track-end", &a.with("group", value(g)))),
        Err(PermsError::TrackStart(g)) => return Err(ui::error(&e.lang, "track-start", &a.with("group", value(g)))),
        Err(PermsError::NotOnTrack) => return Err(ui::error(&e.lang, "not-on-track", &a)),
        Err(PermsError::Ambiguous(g)) => {
            return Err(ui::error(&e.lang, "track-ambiguous", &a.with("groups", value(g.join(", ")))));
        }
        Err(err) => return Err(perms_error(e, &err)),
    };
    let to = step.to.clone().unwrap_or_default();
    let until = match step.expiry {
        Some(x) => format!(" ({})", e.lang.format("expires-in", &Args::new().arg(value(left(now, x))))),
        None => String::new(),
    };
    let a = a.with("to", value(e.perms.display_name(&to))).with("until", until);
    let reply = match &step.from {
        None => ui::success(&e.lang, "promoted-first", &a),
        Some(from) => {
            let key = if up { "promoted" } else { "demoted" };
            ui::success(&e.lang, key, &a.with("from", value(e.perms.display_name(from))))
        }
    };
    save(e, who, dirty, ("user", &t.label(), format!("{sub} {}", args.join(" "))), reply, now)
}

// ----- tracks -----

fn track_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let Some(name) = args.first().map(|n| n.to_lowercase()) else {
        if !allowed_prefix(e, who, "track.", now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Err(ui::usage(&e.lang, &format!("{} track <name> <subcommand>", e.root)));
    };
    let sub = lower(args.get(1));
    let rest = args.get(2..).unwrap_or(&[]);
    let log = args.get(1..).map(|a| a.join(" ")).unwrap_or_default();
    let n = Args::new().arg(value(&name));
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, "track.", now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            Ok(Outcome::reply(track_help(e, who, &name, page_of(rest), now)))
        }
        "info" | "i" => {
            need(e, who, "track.info", now)?;
            let Some(track) = e.perms.track(&name).cloned() else {
                return Err(perms_error(e, &PermsError::NoTrack(name)));
            };
            let mut out = ui::header(&e.lang, "title-track", &Args::new().arg(value(&name)));
            if track.groups.is_empty() {
                out.push(row(1, "", &format!("{}{}", ui::MUTED, e.lang.get("label-none"))));
            }
            for (i, g) in track.groups.iter().enumerate() {
                let shown = e.perms.display_name(g);
                let extra = if &shown == g { String::new() } else { format!(" {}({shown})", ui::MUTED) };
                out.push(
                    Line::parse(&format!("  {}{}. {}{g}{extra}", ui::MUTED, i + 1, ui::VALUE))
                        .on_click(Click::Suggest(format!("{} group {g} info", e.root))),
                );
            }
            Ok(Outcome::reply(out))
        }
        "create" => {
            need(e, who, "track.create", now)?;
            writable(e)?;
            let dirty = e.perms.create_track(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "track-created", &n);
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        "delete" => {
            need(e, who, "track.delete", now)?;
            writable(e)?;
            let dirty = e.perms.delete_track(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "track-deleted", &n);
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        "append" | "insert" | "remove" | "clear" => {
            need(e, who, "track.edit", now)?;
            writable(e)?;
            let group = rest.first().map(|g| g.to_lowercase());
            let (dirty, reply) = match (sub.as_str(), group) {
                ("clear", _) => (e.perms.track_clear(&name), ui::success(&e.lang, "track-cleared", &n)),
                ("append", Some(g)) => {
                    let d = e.perms.track_insert(&name, &g, None);
                    (d, ui::success(&e.lang, "track-added", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                ("insert", Some(g)) => {
                    let Some(pos) = rest.get(1).and_then(|p| p.parse::<usize>().ok()).filter(|p| *p >= 1) else {
                        return Err(ui::usage(&e.lang, &format!("{} track {name} insert <group> <position>", e.root)));
                    };
                    let d = e.perms.track_insert(&name, &g, Some(pos - 1));
                    (d, ui::success(&e.lang, "track-added", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                ("remove", Some(g)) => {
                    let d = e.perms.track_remove(&name, &g);
                    (d, ui::success(&e.lang, "track-removed", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                (s, None) => return Err(ui::usage(&e.lang, &format!("{} track {name} {s} <group>", e.root))),
                _ => return Err(ui::unknown(&e.lang, &sub, &format!("{} help", e.root))),
            };
            let dirty = dirty.map_err(|err| perms_error(e, &err))?;
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} track {name} help", e.root))),
    }
}

/// Runs an import after the platform read the file: replaces everything and
/// saves it (the platform exports a backup first).
pub fn import(e: &mut Engine, who: &Who, file: &str, json: &str, backup: &str, now: u64) -> Outcome {
    let mut snap = match crate::export::parse(json) {
        Ok(s) => s,
        Err(err) => return Outcome::reply(ui::error(&e.lang, "import-failed", &Args::new().arg(value(err)))),
    };
    let counts = (snap.groups.len(), snap.users.len(), snap.tracks.len());
    let fixed = std::mem::take(&mut snap.fixed);
    let worlds = world_warning(e, &snap);
    let before = e.perms.clone();
    e.perms.replace_all(snap.groups, snap.users, snap.tracks);
    let log = LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: "import".into(),
        target: file.to_string(),
        action: format!("import {file}"),
    };
    let saved = match e.store() {
        Some(store) if e.broken.is_none() => {
            store.replace_all(&e.perms, Some(&log), e.cfg.log.max_entries).map_err(|x| x.to_string())
        }
        _ => Err(e.broken.clone().unwrap_or_else(|| "no database".into())),
    };
    if let Err(err) = saved {
        e.perms = before;
        return Outcome::reply(ui::error(&e.lang, "import-failed", &Args::new().arg(value(err))));
    }
    e.changed_everything();
    let a =
        Args::new().arg(value(counts.0)).arg(value(counts.1)).arg(value(counts.2)).arg(value(file)).arg(value(backup));
    let mut reply = ui::success(&e.lang, "imported", &a);
    fixed_lines(e, &fixed, &mut reply);
    reply.extend(worlds);
    Outcome { reply, changed: true, log: Some(log), action: None }
}

/// `/pp import run|preview|undo <source>` after the platform asked the source:
/// `data` is its fingerprint and export. What the source added last time comes
/// from its ledger in the database, and the new ledger is saved together with
/// the import: entries the source dropped go, new ones are added, and the rest
/// stays as it is now (also an imported entry an admin removed).
pub fn import_source(
    e: &mut Engine,
    who: &Who,
    source: &str,
    data: Result<(String, Snapshot), String>,
    mode: SourceMode,
    now: u64,
) -> Outcome {
    let fail = |e: &Engine, why: String| {
        Outcome::reply(ui::error(&e.lang, "import-source-failed", &Args::new().arg(value(source)).arg(value(why))))
    };
    let log = |action: String| LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: "import".into(),
        target: source.to_string(),
        action: cut(action, 1000),
    };
    let ledger = match e.ledger(source) {
        Ok(l) => l,
        Err(why) => return fail(e, why),
    };
    if mode == SourceMode::Undo {
        let before = e.perms.clone();
        let (dirty, n) = merge::undo(&mut e.perms, &ledger);
        let entry = log(format!("import undo {source}: {n} entries"));
        if let Err(why) = e.commit_import(&dirty, Some(&entry), source, &Ledger::default()) {
            e.perms = before;
            return fail(e, why);
        }
        let a = Args::new().arg(value(source)).arg(value(n));
        return Outcome {
            reply: ui::success(&e.lang, "import-source-undone", &a),
            changed: n > 0,
            ..Outcome::default()
        };
    }
    let (fingerprint, mut snap) = match data {
        Ok(d) => d,
        Err(why) => return fail(e, why),
    };
    if mode == SourceMode::Run && ledger.fingerprint == fingerprint {
        return Outcome::reply(ui::info(&e.lang, "import-source-same", &Args::new().arg(value(source))));
    }
    let worlds = world_warning(e, &snap);
    // Players by nickname: the UUID this source bound the name to before, or
    // one that joined with an online-mode account; the rest at such a join.
    let mut pending = Vec::new();
    let mut resolved = std::collections::BTreeMap::new();
    for u in std::mem::take(&mut snap.by_name) {
        let key = pumbo_common::id::name_key(&u.name);
        let bound = ledger.resolved.get(&key).cloned();
        let uuid = bound.or_else(|| e.find_user(&u.name).filter(|k| k.online).map(|k| k.uuid));
        match uuid {
            Some(uuid) => {
                resolved.insert(key, uuid.clone());
                snap.users.push(crate::model::User { uuid, name: u.name, data: u.data });
            }
            None => pending.push(u),
        }
    }
    let bound: Vec<String> = snap
        .users
        .iter()
        .filter(|u| resolved.get(&pumbo_common::id::name_key(&u.name)) == Some(&u.uuid))
        .map(|u| format!("{} -> {}", u.name, u.uuid))
        .collect();
    let notes = |e: &Engine, reply: &mut Text| {
        if !bound.is_empty() {
            let a = Args::new().arg(value(bound.len())).arg(value(bound.join(", ")));
            reply.extend(ui::info(&e.lang, "import-source-resolved", &a));
        }
        if !pending.is_empty() {
            let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
            let a = Args::new().arg(value(pending.len())).arg(value(names.join(", ")));
            let key = if e.cfg.names_checked { "import-source-pending" } else { "import-source-name-only" };
            reply.extend(ui::info(&e.lang, key, &a));
        }
    };
    // What the source no longer has goes; what it still has stays as it is.
    let all: HashSet<String> = merge::candidates(&snap).iter().map(merge::Entry::key).collect();
    let (kept, gone): (Vec<merge::Entry>, Vec<merge::Entry>) =
        ledger.entries.iter().cloned().partition(|x| all.contains(&x.key()));
    let before = e.perms.clone();
    let (mut dirty, removed) = merge::undo(&mut e.perms, &Ledger { entries: gone.clone(), ..Ledger::default() });
    let mut plan = merge::plan(&e.perms, &snap);
    merge::skip_recorded(&mut plan, &e.perms, &ledger.entries);
    if mode == SourceMode::Preview {
        e.perms = before;
        let a = Args::new().arg(value(source)).arg(value(plan.entries.len())).arg(value(plan.conflicts.len()));
        let mut reply = ui::header(&e.lang, "import-source-preview", &a);
        fixed_lines(e, &snap.fixed, &mut reply);
        reply.extend(worlds);
        for entry in plan.entries.iter().take(15) {
            reply.push(ui::row(1, "", &format!("{}+ {}", ui::YES, describe(entry))));
        }
        for entry in gone.iter().take(15) {
            reply.push(ui::row(1, "", &format!("{}- {}", ui::NO, describe(entry))));
        }
        for c in plan.conflicts.iter().take(10) {
            reply.push(ui::row(1, "", &format!("{}! {c}", ui::WARN)));
        }
        notes(e, &mut reply);
        return Outcome::reply(reply);
    }
    let (more, added) = merge::apply(&mut e.perms, &plan);
    dirty.merge(more);
    let mut action = format!("import {source}: {} entries", added.len());
    if removed > 0 {
        action.push_str(&format!(", {removed} taken back"));
    }
    if !plan.conflicts.is_empty() {
        action.push_str(&format!(", kept on conflict: {}", plan.conflicts.join("; ")));
    }
    if !bound.is_empty() {
        action.push_str(&format!(", by name: {}", bound.join(", ")));
    }
    if !pending.is_empty() {
        let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
        action.push_str(&format!(", waiting for the first join: {}", names.join(", ")));
    }
    let mut entries = kept;
    entries.extend(added.iter().cloned());
    let new = Ledger { fingerprint, at: now, entries, pending: pending.clone(), resolved };
    if let Err(why) = e.commit_import(&dirty, Some(&log(action)), source, &new) {
        e.perms = before;
        return fail(e, why);
    }
    let a = Args::new().arg(value(source)).arg(value(added.len())).arg(value(plan.conflicts.len()));
    let mut reply = ui::success(&e.lang, "import-source-done", &a);
    if removed > 0 {
        reply.extend(ui::info(&e.lang, "import-source-removed", &Args::new().arg(value(removed))));
    }
    fixed_lines(e, &snap.fixed, &mut reply);
    reply.extend(worlds);
    notes(e, &mut reply);
    Outcome { reply, changed: true, ..Outcome::default() }
}

/// A player joined: what every source has for its nickname
/// ([`import_pending`]); one outcome per source that has something.
pub fn on_join(e: &mut Engine, uuid: &str, name: &str, online: bool, now: u64) -> Vec<Outcome> {
    let mut waiting = Vec::new();
    let mut damaged = Vec::new();
    for (source, ledger) in e.ledgers() {
        match ledger {
            Ok(l) if l.pending.is_empty() => {}
            Ok(_) => waiting.push(source.to_string()),
            Err(why) => damaged.push((source.to_string(), why.to_string())),
        }
    }
    // A damaged ledger once, not at every join.
    damaged.retain(|(source, _)| e.reported.insert(source.clone()));
    let mut out: Vec<Outcome> = damaged
        .into_iter()
        .map(|(source, why)| {
            let a = Args::new().arg(value(&source)).arg(value(why));
            Outcome::reply(ui::error(&e.lang, "import-source-failed", &a))
        })
        .collect();
    for source in waiting {
        out.extend(import_pending(e, &source, uuid, name, online, now));
    }
    out
}

/// A player joined: what `source` has for its nickname (`ledger.pending`) is
/// added now, only adding like every import. Only for an account the host
/// checked with Mojang (`online`): anyone can take a nickname in offline mode,
/// so an offline account gets nothing and the entries keep waiting (the reply
/// says so, `changed` stays false). `None` when the source has nothing for this
/// name.
pub fn import_pending(e: &mut Engine, source: &str, uuid: &str, name: &str, online: bool, now: u64) -> Option<Outcome> {
    let mut ledger = e.ledger(source).ok()?;
    let key = pumbo_common::id::name_key(name);
    let pos = ledger.pending.iter().position(|u| pumbo_common::id::name_key(&u.name) == key)?;
    if !online {
        let a = Args::new().arg(value(source)).arg(value(name));
        let key = if e.cfg.names_checked { "import-source-offline" } else { "import-source-joined-name-only" };
        return Some(Outcome::reply(ui::warn(&e.lang, key, &a)));
    }
    let waiting = ledger.pending.remove(pos);
    let user = crate::model::User { uuid: uuid.to_string(), name: name.to_string(), data: waiting.data };
    let plan = merge::plan(&e.perms, &Snapshot { users: vec![user], ..Snapshot::default() });
    let before = e.perms.clone();
    let (dirty, added) = merge::apply(&mut e.perms, &plan);
    let log = LogEntry {
        time: now,
        actor: "PumboPerms".into(),
        actor_uuid: String::new(),
        kind: "import".into(),
        target: source.to_string(),
        action: format!("import {source}: {name} -> {uuid} joined, {} entries", added.len()),
    };
    let a = Args::new().arg(value(source)).arg(value(format!("{name} -> {uuid}"))).arg(value(added.len()));
    ledger.entries.extend(added);
    ledger.resolved.insert(key, uuid.to_string());
    if let Err(why) = e.commit_import(&dirty, Some(&log), source, &ledger) {
        e.perms = before;
        let a = Args::new().arg(value(source)).arg(value(why));
        return Some(Outcome::reply(ui::error(&e.lang, "import-source-failed", &a)));
    }
    Some(Outcome {
        reply: ui::success(&e.lang, "import-source-joined", &a),
        changed: true,
        log: Some(log),
        action: None,
    })
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn cut(mut text: String, max: usize) -> String {
    text.truncate(text.floor_char_boundary(max));
    text
}

/// Where nobody sees worlds, one warning with the number of `world=` entries
/// the import keeps.
fn world_warning(e: &Engine, snap: &Snapshot) -> Text {
    if e.cfg.worlds {
        return Text::new();
    }
    let data = snap.groups.iter().map(|g| &g.data).chain(snap.users.iter().chain(&snap.by_name).map(|u| &u.data));
    let count: usize = data
        .map(|d| {
            let contexts = d.permissions.iter().map(|p| &p.context);
            let contexts =
                contexts.chain(d.parents.iter().map(|p| &p.context)).chain(d.meta.iter().map(|m| &m.context));
            contexts.filter(|c| c.get(context::WORLD).is_some()).count()
        })
        .sum();
    if count == 0 {
        return Text::new();
    }
    ui::warn(&e.lang, "import-worlds", &Args::new().arg(value(count)))
}

/// What the import fixed or left out ([`crate::export::Snapshot::fixed`]).
fn fixed_lines(e: &Engine, fixed: &[String], reply: &mut Text) {
    if fixed.is_empty() {
        return;
    }
    reply.extend(ui::warn(&e.lang, "import-fixed", &Args::new().arg(value(fixed.len()))));
    for f in fixed.iter().take(20) {
        reply.push(ui::row(1, "", &format!("{}! {}", ui::WARN, text::escape(f))));
    }
}

fn describe(e: &merge::Entry) -> String {
    use merge::Entry as E;
    match e {
        E::Group { group } => format!("group {group}"),
        E::GroupNode { group, node } => format!("{group}: {} = {}", node.node, node.value),
        E::GroupParent { group, parent } => format!("{group} inherits {}", parent.group),
        E::GroupWeight { group, weight } => format!("{group}: weight {weight}"),
        E::GroupMeta { group, meta } => format!("{group}: {:?} {}", meta.kind, meta.value),
        E::UserNode { name, node, .. } => format!("{name}: {} = {}", node.node, node.value),
        E::UserParent { name, parent, .. } => format!("{name} in {}", parent.group),
    }
}

// ----- views -----

fn version(e: &Engine, env: &Env) -> Text {
    let mut rows = env.version_rows.clone();
    let storage = match (&e.broken, e.store()) {
        (Some(why), _) => why.clone(),
        (None, Some(_)) => text::strip(
            &e.lang.format(
                "version-storage",
                &Args::new()
                    .arg(e.perms.groups().count())
                    .arg(e.perms.users().filter(|u| !u.data.is_empty()).count())
                    .arg(e.perms.tracks().count()),
            ),
        ),
        (None, None) => "-".into(),
    };
    rows.push((e.lang.get("label-storage"), storage));
    let or_dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };
    // On PumboProx the place is the player's server; the platform lists them.
    if e.server_groups.is_empty() {
        rows.push((
            e.lang.get("label-context"),
            format!("server={} group={}", or_dash(&e.cfg.context.server), or_dash(&e.cfg.context.group)),
        ));
    }
    style::version("PumboPerms", style::accent::PERMS, env.version, rows)
}

fn context_suffix(c: &Contexts) -> String {
    if c.is_global() { String::new() } else { format!(" {}{}", ui::INFO, text::escape(&c.to_string())) }
}

fn expiry_suffix(e: &Engine, x: Option<u64>, now: u64) -> String {
    match x {
        Some(x) => format!(" {}({})", ui::INFO, e.lang.format("expires-in", &Args::new().arg(left(now, x)))),
        None => String::new(),
    }
}

/// `--context` arguments for a command that removes a node again.
fn context_args(c: &Contexts) -> String {
    c.iter().map(|(k, v)| format!(" {k}={v}")).collect()
}

fn removable(e: &Engine, line: Line, command: String) -> Line {
    line.on_click(Click::Suggest(command)).on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-remove"))))
}

fn nodes_lines(e: &Engine, t: &Target, data: &Data, now: u64) -> Vec<Line> {
    let mut perms: Vec<&PermNode> = data.permissions.iter().collect();
    perms.sort_by(|a, b| a.node.cmp(&b.node).then(a.context.cmp(&b.context)));
    perms
        .into_iter()
        .map(|p| {
            let unset = if p.expiry.is_some() { "unsettemp" } else { "unset" };
            let text = format!(
                "    {}• {}{} {}= {}{}{}",
                ui::MUTED,
                ui::VALUE,
                p.node,
                ui::MUTED,
                if p.value { format!("{}true", ui::YES) } else { format!("{}false", ui::NO) },
                context_suffix(&p.context),
                expiry_suffix(e, p.expiry, now)
            );
            removable(
                e,
                Line::parse(&text),
                format!("{} permission {unset} {}{}", t.command(&e.root), p.node, context_args(&p.context)),
            )
        })
        .collect()
}

/// A prefix, suffix or display name in its own colours. In chat the text as
/// typed (with its `&` codes) is the tooltip; the console, which has no
/// tooltips, gets it in grey quotes after the value.
fn styled(value: &str, chat: bool) -> Line {
    let raw = format!("{}\"{}\"", ui::INFO, text::escape(value));
    if chat {
        Line::parse(&format!("{value}&r")).on_hover(Text::parse(&raw))
    } else {
        Line::parse(&format!("{value}&r {raw}"))
    }
}

fn parent_lines(e: &Engine, t: &Target, data: &Data, now: u64, chat: bool) -> Vec<Line> {
    data.parents
        .iter()
        .map(|p| {
            let shown = e.perms.display_name(&p.group);
            let mut line = Line::parse(&format!("    {}• {}{}", ui::MUTED, ui::VALUE, p.group));
            if shown != p.group {
                // The display name in its colours; the console keeps the text as typed.
                let name = if chat { styled(&shown, true) } else { Line::parse(&text::escape(&shown)) };
                line = line
                    .append(Line::parse(&format!(" {}(", ui::MUTED)))
                    .append(name)
                    .append(Line::parse(&format!("{})", ui::MUTED)));
            }
            let missing = if e.perms.group(&p.group).is_none() { format!(" {}✖", ui::ERROR) } else { String::new() };
            let tail = format!("{missing}{}{}", context_suffix(&p.context), expiry_suffix(e, p.expiry, now));
            removable(
                e,
                line.append(Line::parse(&tail)),
                format!("{} parent remove {}{}", t.command(&e.root), p.group, context_args(&p.context)),
            )
        })
        .collect()
}

fn meta_lines(e: &Engine, t: &Target, data: &Data, now: u64, chat: bool) -> Vec<Line> {
    let mut metas: Vec<_> = data.meta.iter().collect();
    metas.sort_by(|a, b| (a.kind as u8, -a.priority, &a.key).cmp(&(b.kind as u8, -b.priority, &b.key)));
    metas
        .into_iter()
        .map(|m| {
            let (label, remove) = match m.kind {
                MetaKind::Prefix => (
                    format!("{} {}", e.lang.get("kind-prefix"), m.priority),
                    format!("{} meta removeprefix {} {}", t.command(&e.root), m.priority, quoted(&m.value)),
                ),
                MetaKind::Suffix => (
                    format!("{} {}", e.lang.get("kind-suffix"), m.priority),
                    format!("{} meta removesuffix {} {}", t.command(&e.root), m.priority, quoted(&m.value)),
                ),
                MetaKind::Meta => (m.key.clone(), format!("{} meta unset {}", t.command(&e.root), quoted(&m.key))),
            };
            let head = format!("    {}• {}{} {}= ", ui::MUTED, ui::LABEL, text::escape(&label), ui::MUTED);
            let shown = match m.kind {
                MetaKind::Meta => Line::parse(&format!("{}{}", ui::VALUE, text::escape(&m.value))),
                _ => styled(&m.value, chat),
            };
            let tail = format!("{}{}", context_suffix(&m.context), expiry_suffix(e, m.expiry, now));
            let line = Line::parse(&head).append(shown).append(Line::parse(&tail));
            removable(e, line, format!("{remove}{}", context_args(&m.context)))
        })
        .collect()
}

/// A value as one [`split_args`] word.
fn quoted(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Rows on one page of a view.
const PAGE: usize = 20;

/// The title `out` with page `page` of `rows` and a page bar that runs
/// `command <page>`; page 0 is every row.
fn paged(lang: &Lang, mut out: Text, rows: Vec<Line>, page: usize, command: &str) -> Text {
    if page == 0 {
        out.lines.extend(rows);
        return out;
    }
    let pages = rows.len().div_ceil(PAGE).max(1);
    let page = page.clamp(1, pages);
    out.lines.extend(rows.into_iter().skip((page - 1) * PAGE).take(PAGE));
    if pages > 1 {
        out.push(nav(lang, page, pages, command));
    }
    out
}

/// The page bar `━━━ ◀ 2/5 ▶ ━━━` across the chat after the prefix, in the
/// colours of `[PP]`; the arrows run `command <page>` and are dark grey on the
/// first and last page.
fn nav(lang: &Lang, page: usize, pages: usize, command: &str) -> Line {
    let page = page.min(pages);
    let arrow = |symbol: &str, target: usize, active: bool| {
        if active {
            Line::parse(&format!("{}&l{symbol}", ui::BRAND)).on_click(Click::Run(format!("{command} {target}")))
        } else {
            Line::parse(&format!("{}{symbol}", ui::MUTED))
        }
    };
    let centre = arrow("◀", page.saturating_sub(1), page > 1)
        .append(Line::parse(&format!(" {}{page}/{pages} ", ui::VALUE)))
        .append(arrow("▶", page + 1, page < pages));
    let width = pumbo_common::rich::CHAT_WIDTH.saturating_sub(style::prefix(lang).width());
    style::bar(centre, width, style::BRAND, style::accent::PERMS)
}

fn section(out: &mut Vec<Line>, e: &Engine, label_key: &str, count: usize, lines: Vec<Line>) {
    out.push(Line::parse(&format!("  {}{} ({count})", ui::ACCENT, e.lang.get(label_key))));
    if lines.is_empty() {
        out.push(Line::parse(&format!("    {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    out.extend(lines);
}

fn user_view(e: &mut Engine, t: &Target, page: usize, now: u64) -> Text {
    let Target::User { uuid, .. } = t else { return Text::new() };
    let q = e.query("");
    let eff = e.perms.effective(uuid, &q, now).clone();
    let data = t.data(e).cloned().unwrap_or_default();
    let out = ui::header(&e.lang, "title-user", &Args::new().arg(value(t.label())));
    let mut rows = vec![row(1, &e.lang.get("label-uuid"), uuid).on_click(Click::Suggest(uuid.clone()))];
    let rank = e.perms.display_name(&eff.primary);
    let rank_text =
        if rank == eff.primary { text::escape(&rank) } else { format!("{}&r {}({})", rank, ui::MUTED, eff.primary) };
    rows.push(row(1, &e.lang.get("label-rank"), &rank_text));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let chat = page != 0;
    let affix = |label: &str, v: &Option<String>| match v {
        Some(p) => row(1, label, "").append(styled(p, chat)),
        None => row(1, label, &none),
    };
    rows.push(affix(&e.lang.get("label-prefix"), &eff.prefix));
    rows.push(affix(&e.lang.get("label-suffix"), &eff.suffix));
    let inherited: Vec<String> = eff.groups.iter().map(|g| text::escape(g)).collect();
    rows.push(row(
        1,
        &e.lang.get("label-inherits"),
        &if inherited.is_empty() { none.clone() } else { inherited.join(", ") },
    ));
    let hidden = crate::host::hidden(e, uuid, &q, now);
    hidden_lines(e, hidden.iter(), &mut rows);
    section(&mut rows, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now, chat));
    section(&mut rows, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    section(&mut rows, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now, chat));
    paged(&e.lang, out, rows, page, &format!("{} info", t.command(&e.root)))
}

fn group_view(e: &mut Engine, t: &Target, page: usize, now: u64) -> Text {
    let Target::Group { name } = t else { return Text::new() };
    let Some(group) = e.perms.group(name).cloned() else { return Text::new() };
    let out = ui::header(&e.lang, "title-group", &Args::new().arg(value(name)));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let chat = page != 0;
    let shown_label = e.lang.get("label-display-name");
    let shown = if group.display_name.is_empty() {
        row(1, &shown_label, &none)
    } else if chat {
        row(1, &shown_label, "").append(styled(&group.display_name, true))
    } else {
        row(1, &shown_label, &format!("{}&r", group.display_name))
    };
    let mut rows = vec![shown, row(1, &e.lang.get("label-weight"), &group.weight.to_string())];
    let members = e.perms.members(name).len();
    rows.push(
        row(1, &e.lang.get("label-members"), &members.to_string())
            .on_click(Click::Run(format!("{} group {name} listmembers", e.root)))
            .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-members")))),
    );
    let tracks: Vec<String> = e.perms.tracks_of(name).iter().map(|t| t.name.clone()).collect();
    rows.push(row(1, &e.lang.get("label-tracks"), &if tracks.is_empty() { none } else { tracks.join(", ") }));
    section(&mut rows, e, "label-parents", group.data.parents.len(), parent_lines(e, t, &group.data, now, chat));
    section(&mut rows, e, "label-permissions", group.data.permissions.len(), nodes_lines(e, t, &group.data, now));
    section(&mut rows, e, "label-meta", group.data.meta.len(), meta_lines(e, t, &group.data, now, chat));
    paged(&e.lang, out, rows, page, &format!("{} info", t.command(&e.root)))
}

fn holder_header(e: &Engine, t: &Target) -> Text {
    let key = if t.is_user() { "title-user" } else { "title-group" };
    ui::header(&e.lang, key, &Args::new().arg(value(t.label())))
}

fn nodes_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} permission info", t.command(&e.root)))
}

fn parents_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now, page != 0));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} parent info", t.command(&e.root)))
}

fn meta_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now, page != 0));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} meta info", t.command(&e.root)))
}

fn groups_view(e: &Engine, page: usize) -> Text {
    let mut groups: Vec<_> = e.perms.groups().collect();
    groups.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.name.cmp(&b.name)));
    let out = ui::header(&e.lang, "title-groups", &Args::new().arg(value(groups.len())));
    let rows = groups
        .into_iter()
        .map(|g| {
            let shown = if g.display_name.is_empty() {
                String::new()
            } else {
                format!(" {}({}&r{})", ui::MUTED, g.display_name, ui::MUTED)
            };
            Line::parse(&format!(
                "  {}• {}{}{shown} {}{} {}{}",
                ui::MUTED,
                ui::VALUE,
                g.name,
                ui::LABEL,
                e.lang.get("label-weight"),
                ui::VALUE,
                g.weight
            ))
            .on_click(Click::Run(format!("{} group {} info", e.root, g.name)))
            .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-info"))))
        })
        .collect();
    paged(&e.lang, out, rows, page, &format!("{} groups", e.root))
}

fn tracks_view(e: &Engine, page: usize) -> Text {
    let tracks: Vec<_> = e.perms.tracks().collect();
    let out = ui::header(&e.lang, "title-tracks", &Args::new().arg(value(tracks.len())));
    let mut rows = Vec::new();
    if tracks.is_empty() {
        rows.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for t in tracks {
        let ladder = if t.groups.is_empty() {
            "-".to_string()
        } else {
            t.groups.join(&format!(" {}→ {}", ui::MUTED, ui::VALUE))
        };
        rows.push(
            Line::parse(&format!("  {}• {}{} {}: {}{ladder}", ui::MUTED, ui::VALUE, t.name, ui::MUTED, ui::VALUE))
                .on_click(Click::Run(format!("{} track {} info", e.root, t.name))),
        );
    }
    paged(&e.lang, out, rows, page, &format!("{} tracks", e.root))
}

fn members_view(e: &Engine, group: &str, page: usize) -> Text {
    const PER: usize = 15;
    let members = e.perms.members(group);
    let children: Vec<String> = e.perms.child_groups(group).iter().map(|g| g.name.clone()).collect();
    let mut out = ui::header(&e.lang, "title-members", &Args::new().arg(value(group)).arg(value(members.len())));
    if !children.is_empty() {
        out.push(row(1, &e.lang.get("label-child-groups"), &children.join(", ")));
    }
    let pages = members.len().div_ceil(PER).max(1);
    let page = page.clamp(1, pages);
    if members.is_empty() {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for u in members.iter().skip((page - 1) * PER).take(PER) {
        let t = Target::User { uuid: u.uuid.clone(), name: u.name.clone() };
        let uuid = if u.name.is_empty() || is_java_name(&u.name) {
            String::new()
        } else {
            format!(" {}({})", ui::MUTED, u.uuid)
        };
        out.push(
            Line::parse(&format!("  {}• {}{}{uuid}", ui::MUTED, ui::VALUE, text::escape(&t.label())))
                .on_click(Click::Run(format!("{} info", t.command(&e.root)))),
        );
    }
    if pages > 1 {
        out.push(nav(&e.lang, page, pages, &format!("{} group {group} listmembers", e.root)));
    }
    out
}

fn log_view(e: &Engine, page: usize, now: u64) -> Text {
    let mut out = ui::header(&e.lang, "title-log", &Args::new());
    let Some(store) = e.store() else {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
        return out;
    };
    let (entries, total) = match store.log_page(page, LOG_PAGE) {
        Ok(r) => r,
        Err(err) => return ui::error(&e.lang, "save-failed", &Args::new().arg(value(err))),
    };
    if entries.is_empty() {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("log-empty"))));
    }
    for entry in &entries {
        out.push(log_line(e, entry, now));
    }
    let pages = total.div_ceil(LOG_PAGE).max(1);
    if pages > 1 {
        out.push(nav(&e.lang, page, pages, &format!("{} log", e.root)));
    }
    out
}

/// One log entry: `5m ago  Console » group vip  permission set fly`.
pub fn log_line(e: &Engine, entry: &LogEntry, now: u64) -> Line {
    let ago =
        e.lang.format("time-ago", &Args::new().arg(coarse(Duration::from_millis(now.saturating_sub(entry.time)))));
    Line::parse(&format!(
        "  {}{ago} {}{} {}» {}{} {} {}{}",
        ui::INFO,
        ui::VALUE,
        text::escape(&safe_name(&entry.actor)),
        ui::MUTED,
        ui::LABEL,
        text::escape(&entry.kind),
        text::escape(&entry.target),
        ui::VALUE,
        text::escape(&entry.action)
    ))
}

/// The notification for staff about a change.
pub fn notification(e: &Engine, entry: &LogEntry) -> Text {
    let a = Args::new()
        .arg(value(safe_name(&entry.actor)))
        .arg(value(format!("{} {}", entry.kind, entry.target)))
        .arg(value(&entry.action));
    ui::info(&e.lang, "notify", &a)
}

// ----- Tab completion -----

/// Other names of words in the usage lines below the first.
const WORD_ALIASES: &[(&str, &[&str])] =
    &[("permission", &["perm", "p", "permissions"]), ("parent", &["parents", "inherit"]), ("info", &["i", "list"])];

/// Every usage line below the root with its action: the help tables.
fn rows() -> Vec<(String, String)> {
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
