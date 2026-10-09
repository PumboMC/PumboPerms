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
use pumbo_common::text::{self, Args};
use pumbo_common::time::{self, format_duration_coarse};

use crate::context::{self, Contexts};
use crate::engine::Engine;
use crate::export::Snapshot;
use crate::merge::{self, Ledger};
use crate::model::{Data, MetaKind, ParentNode, PermNode};
use crate::node::{self, NodeError};
use crate::perms::{Dirty, KnownName, PermsError, Source};
use crate::store::LogEntry;
use pumbo_common::help::{Entry, Help};

use crate::ui::{self, Click, Line, Text, row, value};

/// Plugin id: permissions are `pumbo.perms.<action>`.
pub const PLUGIN: &str = "perms";
/// The command as shown in help and usage lines.
pub const ROOT: &str = "/pp";

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

/// Splits the argument string into words; `"..."` keeps spaces (`\"` is a quote).
pub fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut had_quotes = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => {
                quoted = !quoted;
                had_quotes = true;
            }
            c if c.is_whitespace() && !quoted => {
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
    match dispatch(e, who, args, env) {
        Ok(o) => o,
        Err(reply) => Outcome::reply(reply),
    }
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
                return Err(ui::usage(&e.lang, "/pp import <file> | run|preview|undo <source>"));
            };
            let mode = match name.as_str() {
                "run" => Some(SourceMode::Run),
                "preview" => Some(SourceMode::Preview),
                "undo" => Some(SourceMode::Undo),
                _ => None,
            };
            if let Some(mode) = mode {
                let Some(source) = rest.get(1).map(|n| n.to_lowercase()) else {
                    return Err(ui::usage(&e.lang, "/pp import run|preview|undo <source>"));
                };
                if !crate::export::is_valid_file_name(&source) {
                    return Err(ui::error(&e.lang, "bad-file-name", &Args::new().arg(value(&source))));
                }
                if mode != SourceMode::Preview {
                    writable(e)?;
                }
                return Ok(Outcome { action: Some(Action::Source(source, mode)), ..Outcome::default() });
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
            Ok(Outcome::reply(groups_view(e)))
        }
        "tracks" | "listtracks" => {
            need(e, who, "track.list", now)?;
            Ok(Outcome::reply(tracks_view(e)))
        }
        "creategroup" | "deletegroup" | "createtrack" | "deletetrack" => {
            let Some(name) = rest.first() else {
                return Err(ui::usage(&e.lang, &format!("/pp {sub} <name>")));
            };
            let verb = if sub.starts_with("create") { "create" } else { "delete" };
            let inner = [name.clone(), verb.to_string()];
            if sub.ends_with("group") { group_cmd(e, who, &inner, env) } else { track_cmd(e, who, &inner, env) }
        }
        "promote" | "demote" => {
            let (Some(player), Some(track)) = (rest.first(), rest.get(1)) else {
                return Err(ui::usage(&e.lang, &format!("/pp {sub} <player> <track> [context...]")));
            };
            let mut inner = vec![player.clone(), sub.clone(), track.clone()];
            inner.extend(rest.iter().skip(2).cloned());
            user_cmd(e, who, &inner, env)
        }
        "user" | "u" => user_cmd(e, who, rest, env),
        "group" | "g" => group_cmd(e, who, rest, env),
        "track" | "t" => track_cmd(e, who, rest, env),
        _ => Err(ui::unknown(&e.lang, &sub, "/pp help")),
    }
}

fn page_of(rest: &[String]) -> usize {
    rest.first().and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1)
}

fn writable(e: &Engine) -> Result<(), Text> {
    if e.writable() { Ok(()) } else { Err(ui::error(&e.lang, "database-broken", &Args::new())) }
}

// ----- help -----

const GENERAL: &[(&str, &str, &str)] = &[
    ("/pp user <player>", "user.", "desc-user"),
    ("/pp group <name>", "group.", "desc-group"),
    ("/pp track <name>", "track.", "desc-track"),
    ("/pp groups", "group.list", "desc-groups"),
    ("/pp tracks", "track.list", "desc-tracks"),
    ("/pp creategroup <name>", "group.create", "desc-group-create"),
    ("/pp deletegroup <name>", "group.delete", "desc-group-delete"),
    ("/pp createtrack <name>", "track.create", "desc-track-create"),
    ("/pp deletetrack <name>", "track.delete", "desc-track-delete"),
    ("/pp promote <player> <track> [context...]", "user.promote", "desc-promote"),
    ("/pp demote <player> <track> [context...]", "user.demote", "desc-demote"),
    ("/pp log [page]", "log", "desc-log"),
    ("/pp export [file]", "export", "desc-export"),
    ("/pp import <file>", "import", "desc-import"),
    ("/pp import run|preview|undo <source>", "import", "desc-import-source"),
    ("/pp editor", "editor", "desc-editor"),
    ("/pp reload", "reload", "desc-reload"),
    ("/pp version", "version", "desc-version"),
];

/// Subcommands of a user or group: (arguments, action without the kind, description).
const HOLDER: &[(&str, &str, &str)] = &[
    ("info", "info", "desc-info"),
    ("permission info", "permission.info", "desc-perm-info"),
    ("permission set <node> [true|false] [context...]", "permission.set", "desc-perm-set"),
    ("permission settemp <node> [true|false] <duration> [context...]", "permission.set", "desc-perm-settemp"),
    ("permission unset <node> [context...]", "permission.unset", "desc-perm-unset"),
    ("permission unsettemp <node> [context...]", "permission.unset", "desc-perm-unsettemp"),
    ("permission check <node> [context...]", "permission.check", "desc-perm-check"),
    ("permission clear [context...]", "permission.clear", "desc-perm-clear"),
    ("parent info", "parent.info", "desc-parent-info"),
    ("parent add <group> [context...]", "parent.add", "desc-parent-add"),
    ("parent addtemp <group> <duration> [context...]", "parent.add", "desc-parent-addtemp"),
    ("parent remove <group> [context...]", "parent.remove", "desc-parent-remove"),
    ("parent set <group> [context...]", "parent.set", "desc-parent-set"),
    ("meta info", "meta.info", "desc-meta-info"),
    ("meta set <key> <value> [context...]", "meta.set", "desc-meta-set"),
    ("meta unset <key> [context...]", "meta.unset", "desc-meta-unset"),
    ("meta setprefix <priority> <text> [context...]", "meta.set", "desc-meta-setprefix"),
    ("meta addprefix <priority> <text> [context...]", "meta.set", "desc-meta-addprefix"),
    ("meta addtempprefix <priority> <text> <duration> [context...]", "meta.set", "desc-meta-addtempprefix"),
    ("meta removeprefix <priority> [text] [context...]", "meta.unset", "desc-meta-removeprefix"),
    ("meta setsuffix <priority> <text> [context...]", "meta.set", "desc-meta-setsuffix"),
    ("meta addsuffix <priority> <text> [context...]", "meta.set", "desc-meta-addsuffix"),
    ("meta addtempsuffix <priority> <text> <duration> [context...]", "meta.set", "desc-meta-addtempsuffix"),
    ("meta removesuffix <priority> [text] [context...]", "meta.unset", "desc-meta-removesuffix"),
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
    ("listmembers", "listmembers", "desc-group-listmembers"),
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

/// `/pp user <player> info` -> (`/pp user <player> info`, ``); arguments start
/// at the first `<` or `[`.
fn split_usage(usage: &str) -> (String, String) {
    let words: Vec<&str> = usage.split_whitespace().collect();
    let n = words.iter().position(|w| w.starts_with('<') || w.starts_with('[')).unwrap_or(words.len());
    (words.get(..n).unwrap_or(&[]).join(" "), words.get(n..).unwrap_or(&[]).join(" "))
}

fn entry(e: &Engine, usage: &str, action: &str, summary: &str) -> Entry {
    let details_key = format!("{summary}-details");
    let details = if e.lang.has(&details_key) { e.lang.get(&details_key) } else { String::new() };
    let permission =
        if action.ends_with('.') { format!("pumbo.{PLUGIN}.{action}*") } else { permission(PLUGIN, action) };
    let (command, args) = split_usage(usage);
    Entry::new(command, e.lang.get(summary)).args(args).details(details).permission(permission)
}

/// The permission nodes the sender has among those of PumboPerms.
fn allowed_nodes(e: &mut Engine, who: &Who, now: u64) -> HashSet<String> {
    ACTIONS.iter().filter(|a| allowed(e, who, a, now)).map(|a| permission(PLUGIN, a)).collect()
}

fn show_help(e: &mut Engine, who: &Who, help: Help, page: usize, now: u64) -> Text {
    let nodes = allowed_nodes(e, who, now);
    let check = |n: &str| match n.strip_suffix('*') {
        Some(prefix) => nodes.iter().any(|a| a.starts_with(prefix)),
        None => nodes.contains(n),
    };
    if who.is_console() { help.console(&e.lang, check) } else { help.chat(&e.lang, page, check) }
}

fn general_help(e: &mut Engine, who: &Who, page: usize, now: u64) -> Text {
    let mut help = Help::new("PumboPerms", format!("{ROOT} help"))
        .version(env!("CARGO_PKG_VERSION"))
        .root(ROOT)
        .section(e.lang.get("help-section"));
    for (u, a, d) in GENERAL {
        help.push(entry(e, u, a, d));
    }
    show_help(e, who, help, page, now)
}

fn holder_help(e: &mut Engine, who: &Who, t: &Target, filter: &str, page: usize, now: u64) -> Text {
    let base = t.command();
    let kind = t.kind();
    let extra = if t.is_user() { USER_ONLY } else { GROUP_ONLY };
    let section_key = if t.is_user() { "title-user" } else { "title-group" };
    let section = e.lang.format(section_key, &Args::new().arg(t.label()));
    let help_command = if filter.is_empty() { format!("{base} help") } else { format!("{base} {filter} help") };
    let mut help = Help::new("PumboPerms", help_command).root(base.clone()).section(section);
    for (u, a, d) in HOLDER.iter().chain(extra.iter()).filter(|(u, _, _)| filter.is_empty() || u.starts_with(filter)) {
        help.push(entry(e, &format!("{base} {u}"), &format!("{kind}.{a}"), d));
    }
    show_help(e, who, help, page, now)
}

fn track_help(e: &mut Engine, who: &Who, track: &str, page: usize, now: u64) -> Text {
    let base = format!("{ROOT} track {track}");
    let section = e.lang.format("title-track", &Args::new().arg(track));
    let mut help = Help::new("PumboPerms", format!("{base} help")).root(base.clone()).section(section);
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

    fn label(&self) -> &str {
        match self {
            Target::User { name, .. } => name,
            Target::Group { name } => name,
        }
    }

    fn command(&self) -> String {
        format!("{ROOT} {} {}", self.kind(), self.label())
    }

    /// The target inside a message: `Steve` or `group vip`.
    fn shown(&self, e: &Engine) -> String {
        let key = if self.is_user() { "target-user" } else { "target-group" };
        e.lang.format(key, &Args::new().arg(value(self.label())))
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

fn find_user(e: &Engine, env: &Env, name: &str) -> Result<KnownName, Text> {
    if let Some(k) = (env.online)(name) {
        return Ok(k);
    }
    e.perms.find_user(name).ok_or_else(|| ui::error(&e.lang, "command-player-not-found", &Args::new().arg(value(name))))
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

/// Positional arguments and the trailing `key=value` contexts.
fn split_context(e: &Engine, args: &[String]) -> Result<(Vec<String>, Contexts), Text> {
    let mut end = args.len();
    while end > 0 && args.get(end - 1).is_some_and(|a| context::is_context_arg(a)) {
        end -= 1;
    }
    let (pos, ctx) = args.split_at(end);
    let contexts =
        Contexts::parse(ctx).map_err(|err| ui::error(&e.lang, "bad-context", &Args::new().arg(value(err))))?;
    Ok((pos.to_vec(), contexts))
}

/// ` in server=lobby` or nothing.
fn in_context(e: &Engine, c: &Contexts) -> String {
    if c.is_global() { String::new() } else { e.lang.format("in-context", &Args::new().arg(value(c))) }
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
        return Err(ui::usage(&e.lang, "/pp user <player> <subcommand>"));
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
        return Err(ui::usage(&e.lang, "/pp group <name> <subcommand>"));
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
                return Err(ui::usage(&e.lang, &format!("{} rename <new-name>", t.command())));
            };
            let dirty = e.perms.rename_group(&name, &to).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-renamed", &Args::new().arg(value(&name)).arg(value(&to)));
            save(e, who, dirty, ("group", &name, format!("rename {to}")), reply, now)
        }
        "setweight" => {
            need(e, who, "group.setweight", now)?;
            writable(e)?;
            let Some(raw) = rest.first() else {
                return Err(ui::usage(&e.lang, &format!("{} setweight <weight>", t.command())));
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
                return Err(ui::usage(&e.lang, &format!("{} setdisplayname <name|clear>", t.command())));
            }
            let shown = rest.join(" ");
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
            Ok(Outcome::reply(if t.is_user() { user_view(e, t, now) } else { group_view(e, t, now) }))
        }
        "permission" | "perm" | "p" | "permissions" => permission_cmd(e, who, t, rest, now),
        "parent" | "parents" | "inherit" => parent_cmd(e, who, t, rest, now),
        "meta" => meta_cmd(e, who, t, rest, now),
        "promote" | "demote" if t.is_user() => step_cmd(e, who, t, &sub, rest, now),
        "clear" if t.is_user() => {
            need(e, who, "user.clear", now)?;
            writable(e)?;
            let ((), dirty) = t.edit(e, |d| *d = Data::default()).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "user-cleared", &Args::new().arg(t.shown(e)));
            save(e, who, dirty, (kind, t.label(), "clear".into()), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} help", t.command()))),
    }
}

fn permission_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("permission {}", args.join(" "));
    let base = format!("{} permission", t.command());
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, &format!("{kind}.permission."), now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            Ok(Outcome::reply(holder_help(e, who, t, "permission", page_of(&pos), now)))
        }
        "info" | "list" => {
            need(e, who, &format!("{kind}.permission.info"), now)?;
            Ok(Outcome::reply(nodes_view(e, t, now)))
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
            save(e, who, dirty, (kind, t.label(), action_log), reply, now)
        }
        "unset" | "unsettemp" => {
            need(e, who, &format!("{kind}.permission.unset"), now)?;
            writable(e)?;
            let Some(raw) = pos.first() else {
                return Err(ui::usage(&e.lang, &format!("{base} {sub} <node> [context...]")));
            };
            let node = parse_node(e, raw)?;
            let temp = sub == "unsettemp";
            let (removed, dirty) =
                t.edit(e, |d| d.unset_permission(&node, &ctx, temp)).map_err(|err| perms_error(e, &err))?;
            let a =
                Args::new().with("node", value(&node)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
            if removed.is_none() {
                e.reload_data();
                return Err(ui::error(&e.lang, "perm-unset-none", &a));
            }
            let reply = ui::success(&e.lang, "perm-unset", &a);
            save(e, who, dirty, (kind, t.label(), action_log), reply, now)
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
            save(e, who, dirty, (kind, t.label(), action_log), reply, now)
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
            let a = Args::new().with("node", value(&node)).with("target", t.shown(e));
            let reply = match decision {
                Some(d) => {
                    let mut text = ui::info(&e.lang, "perm-check", &a.clone().with("value", ui::boolean(d.value)));
                    let source = match &d.source {
                        Source::User => e.lang.format("target-user", &Args::new().arg(value(t.label()))),
                        Source::Group(g) => e.lang.format("target-group", &Args::new().arg(value(g))),
                    };
                    let mut how = e.lang.format(
                        "perm-check-source",
                        &Args::new()
                            .with("source", source)
                            .with("node", value(&d.node))
                            .with("context", in_context(e, &d.context)),
                    );
                    if let Some(x) = d.expiry {
                        how.push_str(&e.lang.format("expires-in", &Args::new().arg(value(left(now, x)))));
                    }
                    text.push(Line::parse(&format!("  {}{how}", ui::INFO)));
                    text
                }
                None => ui::info(&e.lang, "perm-check-undecided", &a),
            };
            Ok(Outcome::reply(reply))
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    }
}

fn parent_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("parent {}", args.join(" "));
    let base = format!("{} parent", t.command());
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.parent."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Ok(Outcome::reply(holder_help(e, who, t, "parent", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.parent.info"), now)?;
        return Ok(Outcome::reply(parents_view(e, t, now)));
    }
    let action = match sub.as_str() {
        "add" | "addtemp" => "add",
        "remove" | "removetemp" => "remove",
        "set" => "set",
        _ => return Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    };
    need(e, who, &format!("{kind}.parent.{action}"), now)?;
    writable(e)?;
    let temp = sub == "addtemp";
    let usage = if temp {
        format!("{base} addtemp <group> <duration> [context...]")
    } else {
        format!("{base} {sub} <group> [context...]")
    };
    let Some(group) = pos.first().map(|g| g.to_lowercase()) else { return Err(ui::usage(&e.lang, &usage)) };
    let dur = if temp {
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
                Ok(d) => (if temp { "parent-addtemp" } else { "parent-add" }, d),
                Err(PermsError::Unchanged) => return Err(ui::error(&e.lang, "parent-already", &a)),
                Err(err) => return Err(perms_error(e, &err)),
            }
        }
        ("add", Target::User { .. }) => {
            let p = ParentNode { group: group.clone(), context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            let (added, d) = t.edit(e, |data| data.add_parent(p)).map_err(|err| perms_error(e, &err))?;
            if !added {
                e.reload_data();
                return Err(ui::error(&e.lang, "parent-already", &a));
            }
            (if temp { "parent-addtemp" } else { "parent-add" }, d)
        }
        ("remove", _) => {
            let (removed, d) =
                t.edit(e, |data| data.remove_parent(&group, &ctx)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.reload_data();
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
    save(e, who, dirty, (kind, t.label(), action_log), reply, now)
}

fn meta_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("meta {}", args.join(" "));
    let base = format!("{} meta", t.command());
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.meta."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Ok(Outcome::reply(holder_help(e, who, t, "meta", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.meta.info"), now)?;
        return Ok(Outcome::reply(meta_view(e, t, now)));
    }
    let affix = |s: &str| {
        if s.ends_with("prefix") {
            Some(MetaKind::Prefix)
        } else if s.ends_with("suffix") {
            Some(MetaKind::Suffix)
        } else {
            None
        }
    };
    let removing = sub == "unset" || sub.starts_with("remove");
    need(e, who, &format!("{kind}.meta.{}", if removing { "unset" } else { "set" }), now)?;
    writable(e)?;
    let mut a = Args::new().with("target", t.shown(e)).with("context", in_context(e, &ctx));
    let (key, dirty) = match (sub.as_str(), affix(&sub)) {
        ("set", _) => {
            let (Some(k), Some(v)) = (pos.first(), pos.get(1)) else {
                return Err(ui::usage(&e.lang, &format!("{base} set <key> <value> [context...]")));
            };
            let k = k.to_lowercase();
            let v = pos.get(1..).map(|p| p.join(" ")).unwrap_or_else(|| v.clone());
            a = a.with("key", value(&k)).with("value", value(&v));
            let ((), d) = t.edit(e, |data| data.set_meta(&k, &v, &ctx, None)).map_err(|err| perms_error(e, &err))?;
            ("meta-set", d)
        }
        ("unset", _) => {
            let Some(k) = pos.first().map(|k| k.to_lowercase()) else {
                return Err(ui::usage(&e.lang, &format!("{base} unset <key> [context...]")));
            };
            a = a.with("key", value(&k));
            let (removed, d) = t.edit(e, |data| data.unset_meta(&k, &ctx)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.reload_data();
                return Err(ui::error(&e.lang, "meta-unset-none", &a));
            }
            ("meta-unset", d)
        }
        (s, Some(mk)) => {
            let which = if mk == MetaKind::Prefix { "prefix" } else { "suffix" };
            a = a.with("kind", e.lang.get(&format!("kind-{which}")));
            if s.starts_with("remove") {
                let Some(raw) = pos.first() else {
                    return Err(ui::usage(&e.lang, &format!("{base} {s} <priority> [text] [context...]")));
                };
                let prio = parse_int(e, raw)?;
                let text_arg = pos.get(1).cloned();
                a = a.with("priority", value(prio));
                let (count, d) = t
                    .edit(e, |data| data.remove_affix(mk, prio, text_arg.as_deref(), &ctx))
                    .map_err(|err| perms_error(e, &err))?;
                if count == 0 {
                    e.reload_data();
                    return Err(ui::error(&e.lang, "affix-remove-none", &a));
                }
                a = a.with("count", value(count));
                ("affix-remove", d)
            } else {
                let temp = s.starts_with("addtemp");
                let replace = s.starts_with("set");
                if !(temp || replace || s.starts_with("add")) {
                    return Err(ui::unknown(&e.lang, s, &format!("{base} help")));
                }
                let usage = if temp {
                    format!("{base} {s} <priority> <text> <duration> [context...]")
                } else {
                    format!("{base} {s} <priority> <text> [context...]")
                };
                let (Some(raw), Some(txt)) = (pos.first(), pos.get(1)) else { return Err(ui::usage(&e.lang, &usage)) };
                let prio = parse_int(e, raw)?;
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
                    e.reload_data();
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
    save(e, who, dirty, (kind, t.label(), action_log), reply, now)
}

fn step_cmd(e: &mut Engine, who: &Who, t: &Target, sub: &str, args: &[String], now: u64) -> Res {
    need(e, who, &format!("user.{sub}"), now)?;
    writable(e)?;
    let (pos, ctx) = split_context(e, args)?;
    let Some(track) = pos.first().map(|t| t.to_lowercase()) else {
        return Err(ui::usage(&e.lang, &format!("{} {sub} <track> [context...]", t.command())));
    };
    let Target::User { uuid, name } = t else {
        return Err(ui::unknown(&e.lang, sub, "/pp help"));
    };
    let up = sub == "promote";
    let a = Args::new().with("target", t.shown(e)).with("track", value(&track)).with("context", in_context(e, &ctx));
    let result = if up { e.perms.promote(uuid, name, &track, &ctx) } else { e.perms.demote(uuid, name, &track, &ctx) };
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
    let a = a.with("to", value(e.perms.display_name(&to)));
    let reply = match &step.from {
        None => ui::success(&e.lang, "promoted-first", &a),
        Some(from) => {
            let key = if up { "promoted" } else { "demoted" };
            ui::success(&e.lang, key, &a.with("from", value(e.perms.display_name(from))))
        }
    };
    save(e, who, dirty, ("user", t.label(), format!("{sub} {}", args.join(" "))), reply, now)
}

// ----- tracks -----

fn track_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let Some(name) = args.first().map(|n| n.to_lowercase()) else {
        if !allowed_prefix(e, who, "track.", now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        return Err(ui::usage(&e.lang, "/pp track <name> <subcommand>"));
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
                        .on_click(Click::Suggest(format!("{ROOT} group {g} info"))),
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
                        return Err(ui::usage(&e.lang, &format!("{ROOT} track {name} insert <group> <position>")));
                    };
                    let d = e.perms.track_insert(&name, &g, Some(pos - 1));
                    (d, ui::success(&e.lang, "track-added", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                ("remove", Some(g)) => {
                    let d = e.perms.track_remove(&name, &g);
                    (d, ui::success(&e.lang, "track-removed", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                (s, None) => return Err(ui::usage(&e.lang, &format!("{ROOT} track {name} {s} <group>"))),
                _ => return Err(ui::unknown(&e.lang, &sub, "/pp help")),
            };
            let dirty = dirty.map_err(|err| perms_error(e, &err))?;
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{ROOT} track {name} help"))),
    }
}

/// Runs an import after the platform read the file: replaces everything and
/// saves it (the platform exports a backup first).
pub fn import(e: &mut Engine, who: &Who, file: &str, json: &str, backup: &str, now: u64) -> Outcome {
    let snap = match crate::export::parse(json) {
        Ok(s) => s,
        Err(err) => return Outcome::reply(ui::error(&e.lang, "import-failed", &Args::new().arg(value(err)))),
    };
    let counts = (snap.groups.len(), snap.users.len(), snap.tracks.len());
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
    let a =
        Args::new().arg(value(counts.0)).arg(value(counts.1)).arg(value(counts.2)).arg(value(file)).arg(value(backup));
    Outcome { reply: ui::success(&e.lang, "imported", &a), changed: true, log: Some(log), action: None }
}

/// `/pp import run|preview|undo <source>` after the platform asked the source:
/// `data` is its fingerprint and export, `ledger` what it added last time.
/// Returns the reply and the ledger to save (`Some` with no entries: delete).
pub fn import_source(
    e: &mut Engine,
    who: &Who,
    source: &str,
    data: Result<(String, Snapshot), String>,
    ledger: &Ledger,
    mode: SourceMode,
    now: u64,
) -> (Outcome, Option<Ledger>) {
    let fail = |e: &Engine, why: String| {
        Outcome::reply(ui::error(&e.lang, "import-source-failed", &Args::new().arg(value(source)).arg(value(why))))
    };
    let log = |action: String| LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: "import".into(),
        target: source.to_string(),
        action,
    };
    if mode == SourceMode::Undo {
        let before = e.perms.clone();
        let (dirty, n) = merge::undo(&mut e.perms, ledger);
        if let Err(why) = e.commit(&dirty, Some(&log(format!("import undo {source}: {n} entries")))) {
            e.perms = before;
            return (fail(e, why), None);
        }
        let a = Args::new().arg(value(source)).arg(value(n));
        let out =
            Outcome { reply: ui::success(&e.lang, "import-source-undone", &a), changed: n > 0, ..Outcome::default() };
        return (out, Some(Ledger::default()));
    }
    let (fingerprint, mut snap) = match data {
        Ok(d) => d,
        Err(why) => return (fail(e, why), None),
    };
    if mode == SourceMode::Run && ledger.fingerprint == fingerprint {
        return (Outcome::reply(ui::info(&e.lang, "import-source-same", &Args::new().arg(value(source)))), None);
    }
    // Players by nickname: those who joined before by their UUID now, the
    // rest at their first join.
    let mut pending = Vec::new();
    for u in std::mem::take(&mut snap.by_name) {
        match e.perms.find_user(&u.name) {
            Some(k) => snap.users.push(crate::model::User { uuid: k.uuid, name: k.name, data: u.data }),
            None => pending.push(u),
        }
    }
    let waiting = |e: &Engine, reply: &mut Text| {
        if !pending.is_empty() {
            let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
            let a = Args::new().arg(value(pending.len())).arg(value(names.join(", ")));
            reply.extend(ui::info(&e.lang, "import-source-pending", &a));
        }
    };
    // The previous import of this source goes first: the new one replaces it.
    let before = e.perms.clone();
    let (mut dirty, _) = merge::undo(&mut e.perms, ledger);
    let plan = merge::plan(&e.perms, &snap);
    if mode == SourceMode::Preview {
        e.perms = before;
        let a = Args::new().arg(value(source)).arg(value(plan.entries.len())).arg(value(plan.conflicts.len()));
        let mut reply = ui::header(&e.lang, "import-source-preview", &a);
        for entry in plan.entries.iter().take(15) {
            reply.push(ui::row(1, "", &format!("{}+ {}", ui::YES, describe(entry))));
        }
        for c in plan.conflicts.iter().take(10) {
            reply.push(ui::row(1, "", &format!("{}! {c}", ui::WARN)));
        }
        waiting(e, &mut reply);
        return (Outcome::reply(reply), None);
    }
    let (more, added) = merge::apply(&mut e.perms, &plan);
    dirty.merge(more);
    let mut action = format!("import {source}: {} entries", added.len());
    if !plan.conflicts.is_empty() {
        action.push_str(&format!(", kept on conflict: {}", plan.conflicts.join("; ")));
    }
    if !pending.is_empty() {
        let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
        action.push_str(&format!(", waiting for the first join: {}", names.join(", ")));
    }
    action.truncate(1000);
    if let Err(why) = e.commit(&dirty, Some(&log(action))) {
        e.perms = before;
        return (fail(e, why), None);
    }
    let a = Args::new().arg(value(source)).arg(value(added.len())).arg(value(plan.conflicts.len()));
    let mut reply = ui::success(&e.lang, "import-source-done", &a);
    waiting(e, &mut reply);
    let out = Outcome { reply, changed: true, ..Outcome::default() };
    (out, Some(Ledger { fingerprint, at: now, entries: added, pending }))
}

/// A player joined: what a source has for its nickname (`ledger.pending`) is
/// added now, only adding like every import. `None` when the source has
/// nothing for this name; the ledger then needs no saving.
pub fn import_pending(
    e: &mut Engine,
    source: &str,
    ledger: &mut Ledger,
    uuid: &str,
    name: &str,
    now: u64,
) -> Option<Outcome> {
    let key = pumbo_common::id::name_key(name);
    let pos = ledger.pending.iter().position(|u| pumbo_common::id::name_key(&u.name) == key)?;
    let waiting = ledger.pending.remove(pos);
    let user = crate::model::User { uuid: uuid.to_string(), name: name.to_string(), data: waiting.data.clone() };
    let plan = merge::plan(&e.perms, &Snapshot { users: vec![user], ..Snapshot::default() });
    let before = e.perms.clone();
    let (dirty, added) = merge::apply(&mut e.perms, &plan);
    let log = LogEntry {
        time: now,
        actor: "PumboPerms".into(),
        actor_uuid: String::new(),
        kind: "import".into(),
        target: source.to_string(),
        action: format!("import {source}: {name} joined, {} entries", added.len()),
    };
    let a = Args::new().arg(value(source)).arg(value(name)).arg(value(added.len()));
    if let Err(why) = e.commit(&dirty, Some(&log)) {
        e.perms = before;
        ledger.pending.insert(pos, waiting);
        let a = Args::new().arg(value(source)).arg(value(why));
        return Some(Outcome::reply(ui::error(&e.lang, "import-source-failed", &a)));
    }
    ledger.entries.extend(added);
    Some(Outcome {
        reply: ui::success(&e.lang, "import-source-joined", &a),
        changed: true,
        log: Some(log),
        action: None,
    })
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
    pumbo_common::style::version("PumboPerms", env.version, rows)
}

fn context_suffix(c: &Contexts) -> String {
    if c.is_global() { String::new() } else { format!(" {}{}", ui::MUTED, text::escape(&c.to_string())) }
}

fn expiry_suffix(e: &Engine, x: Option<u64>, now: u64) -> String {
    match x {
        Some(x) => format!(" {}({})", ui::WARN, e.lang.format("expires-in", &Args::new().arg(left(now, x)))),
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
                format!("{} permission {unset} {}{}", t.command(), p.node, context_args(&p.context)),
            )
        })
        .collect()
}

fn parent_lines(e: &Engine, t: &Target, data: &Data, now: u64) -> Vec<Line> {
    data.parents
        .iter()
        .map(|p| {
            let shown = e.perms.display_name(&p.group);
            let extra =
                if shown == p.group { String::new() } else { format!(" {}({})", ui::MUTED, text::escape(&shown)) };
            let missing = if e.perms.group(&p.group).is_none() { format!(" {}✖", ui::ERROR) } else { String::new() };
            let text = format!(
                "    {}• {}{}{extra}{missing}{}{}",
                ui::MUTED,
                ui::VALUE,
                p.group,
                context_suffix(&p.context),
                expiry_suffix(e, p.expiry, now)
            );
            removable(
                e,
                Line::parse(&text),
                format!("{} parent remove {}{}", t.command(), p.group, context_args(&p.context)),
            )
        })
        .collect()
}

fn meta_lines(e: &Engine, t: &Target, data: &Data, now: u64) -> Vec<Line> {
    let mut metas: Vec<_> = data.meta.iter().collect();
    metas.sort_by(|a, b| (a.kind as u8, -a.priority, &a.key).cmp(&(b.kind as u8, -b.priority, &b.key)));
    metas
        .into_iter()
        .map(|m| {
            let (label, remove) = match m.kind {
                MetaKind::Prefix => (
                    format!("{} {}", e.lang.get("kind-prefix"), m.priority),
                    format!("{} meta removeprefix {} \"{}\"", t.command(), m.priority, m.value),
                ),
                MetaKind::Suffix => (
                    format!("{} {}", e.lang.get("kind-suffix"), m.priority),
                    format!("{} meta removesuffix {} \"{}\"", t.command(), m.priority, m.value),
                ),
                MetaKind::Meta => (m.key.clone(), format!("{} meta unset {}", t.command(), m.key)),
            };
            let shown = match m.kind {
                MetaKind::Meta => text::escape(&m.value),
                _ => format!("{}&r {}\"{}\"", m.value, ui::MUTED, text::escape(&m.value)),
            };
            let text = format!(
                "    {}• {}{} {}= {}{}{}{}",
                ui::MUTED,
                ui::INFO,
                text::escape(&label),
                ui::MUTED,
                ui::VALUE,
                shown,
                context_suffix(&m.context),
                expiry_suffix(e, m.expiry, now)
            );
            removable(e, Line::parse(&text), format!("{remove}{}", context_args(&m.context)))
        })
        .collect()
}

fn section(out: &mut Text, e: &Engine, label_key: &str, count: usize, lines: Vec<Line>) {
    out.push(Line::parse(&format!("  {}{} {}({count})", ui::BRAND, e.lang.get(label_key), ui::MUTED)));
    if lines.is_empty() {
        out.push(Line::parse(&format!("    {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for l in lines {
        out.push(l);
    }
}

fn user_view(e: &mut Engine, t: &Target, now: u64) -> Text {
    let Target::User { uuid, name } = t else { return Text::new() };
    let q = e.query("");
    let eff = e.perms.effective(uuid, &q, now).clone();
    let data = t.data(e).cloned().unwrap_or_default();
    let mut out = ui::header(&e.lang, "title-user", &Args::new().arg(value(name)));
    out.push(row(1, &e.lang.get("label-uuid"), uuid).on_click(Click::Suggest(uuid.clone())));
    let rank = e.perms.display_name(&eff.primary);
    let rank_text =
        if rank == eff.primary { text::escape(&rank) } else { format!("{}&r {}({})", rank, ui::MUTED, eff.primary) };
    out.push(row(1, &e.lang.get("label-rank"), &rank_text));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let affix = |v: &Option<String>| {
        v.as_ref().map(|p| format!("{p}&r {}\"{}\"", ui::MUTED, text::escape(p))).unwrap_or_else(|| none.clone())
    };
    out.push(row(1, &e.lang.get("label-prefix"), &affix(&eff.prefix)));
    out.push(row(1, &e.lang.get("label-suffix"), &affix(&eff.suffix)));
    let inherited: Vec<String> = eff.groups.iter().map(|g| text::escape(g)).collect();
    out.push(row(
        1,
        &e.lang.get("label-inherits"),
        &if inherited.is_empty() { none.clone() } else { inherited.join(", ") },
    ));
    section(&mut out, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now));
    section(&mut out, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    section(&mut out, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now));
    out
}

fn group_view(e: &mut Engine, t: &Target, now: u64) -> Text {
    let Target::Group { name } = t else { return Text::new() };
    let Some(group) = e.perms.group(name).cloned() else { return Text::new() };
    let mut out = ui::header(&e.lang, "title-group", &Args::new().arg(value(name)));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let shown = if group.display_name.is_empty() { none.clone() } else { format!("{}&r", group.display_name) };
    out.push(row(1, &e.lang.get("label-display-name"), &shown));
    out.push(row(1, &e.lang.get("label-weight"), &group.weight.to_string()));
    let members = e.perms.members(name).len();
    out.push(
        row(1, &e.lang.get("label-members"), &members.to_string())
            .on_click(Click::Run(format!("{ROOT} group {name} listmembers")))
            .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-members")))),
    );
    let tracks: Vec<String> = e.perms.tracks_of(name).iter().map(|t| t.name.clone()).collect();
    out.push(row(1, &e.lang.get("label-tracks"), &if tracks.is_empty() { none } else { tracks.join(", ") }));
    section(&mut out, e, "label-parents", group.data.parents.len(), parent_lines(e, t, &group.data, now));
    section(&mut out, e, "label-permissions", group.data.permissions.len(), nodes_lines(e, t, &group.data, now));
    section(&mut out, e, "label-meta", group.data.meta.len(), meta_lines(e, t, &group.data, now));
    out
}

fn holder_header(e: &Engine, t: &Target) -> Text {
    let key = if t.is_user() { "title-user" } else { "title-group" };
    ui::header(&e.lang, key, &Args::new().arg(value(t.label())))
}

fn nodes_view(e: &Engine, t: &Target, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut out = holder_header(e, t);
    section(&mut out, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    out
}

fn parents_view(e: &Engine, t: &Target, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut out = holder_header(e, t);
    section(&mut out, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now));
    out
}

fn meta_view(e: &Engine, t: &Target, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut out = holder_header(e, t);
    section(&mut out, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now));
    out
}

fn groups_view(e: &Engine) -> Text {
    let mut groups: Vec<_> = e.perms.groups().collect();
    groups.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.name.cmp(&b.name)));
    let mut out = ui::header(&e.lang, "title-groups", &Args::new().arg(value(groups.len())));
    for g in groups {
        let shown = if g.display_name.is_empty() {
            String::new()
        } else {
            format!(" {}({}&r{})", ui::MUTED, g.display_name, ui::MUTED)
        };
        let line = Line::parse(&format!(
            "  {}• {}{}{shown} {}{} {}",
            ui::MUTED,
            ui::VALUE,
            g.name,
            ui::INFO,
            e.lang.get("label-weight"),
            g.weight
        ))
        .on_click(Click::Run(format!("{ROOT} group {} info", g.name)))
        .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-info"))));
        out.push(line);
    }
    out
}

fn tracks_view(e: &Engine) -> Text {
    let tracks: Vec<_> = e.perms.tracks().collect();
    let mut out = ui::header(&e.lang, "title-tracks", &Args::new().arg(value(tracks.len())));
    if tracks.is_empty() {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for t in tracks {
        let ladder = if t.groups.is_empty() {
            "-".to_string()
        } else {
            t.groups.join(&format!(" {}→ {}", ui::MUTED, ui::VALUE))
        };
        out.push(
            Line::parse(&format!("  {}• {}{} {}: {}{ladder}", ui::MUTED, ui::VALUE, t.name, ui::MUTED, ui::VALUE))
                .on_click(Click::Run(format!("{ROOT} track {} info", t.name))),
        );
    }
    out
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
        let name = if u.name.is_empty() { u.uuid.clone() } else { u.name.clone() };
        out.push(
            Line::parse(&format!("  {}• {}{}", ui::MUTED, ui::VALUE, text::escape(&name)))
                .on_click(Click::Run(format!("{ROOT} user {name} info"))),
        );
    }
    if pages > 1 {
        out.push(
            Line::parse(&format!("  {}{page}/{pages}", ui::MUTED)).append(
                Line::parse(&format!(" {}▶", ui::BRAND))
                    .on_click(Click::Run(format!("{ROOT} group {group} listmembers {}", page % pages + 1))),
            ),
        );
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
        let mut nav = Line::parse("  ");
        if page > 1 {
            nav = nav.append(
                Line::parse(&format!("{}◀ ", ui::BRAND)).on_click(Click::Run(format!("{ROOT} log {}", page - 1))),
            );
        }
        nav = nav.append(Line::parse(&format!("{}{}/{pages}", ui::INFO, page.min(pages))));
        if page < pages {
            nav = nav.append(
                Line::parse(&format!(" {}▶", ui::BRAND)).on_click(Click::Run(format!("{ROOT} log {}", page + 1))),
            );
        }
        out.push(nav);
    }
    out
}

/// One log entry: `5m ago  Console » group vip  permission set fly`.
pub fn log_line(e: &Engine, entry: &LogEntry, now: u64) -> Line {
    let ago =
        e.lang.format("time-ago", &Args::new().arg(coarse(Duration::from_millis(now.saturating_sub(entry.time)))));
    Line::parse(&format!(
        "  {}{ago} {}{} {}» {}{} {} {}{}",
        ui::MUTED,
        ui::VALUE,
        text::escape(&entry.actor),
        ui::MUTED,
        ui::INFO,
        text::escape(&entry.kind),
        text::escape(&entry.target),
        ui::VALUE,
        text::escape(&entry.action)
    ))
}

/// The notification for staff about a change.
pub fn notification(e: &Engine, entry: &LogEntry) -> Text {
    let a = Args::new()
        .arg(value(&entry.actor))
        .arg(value(format!("{} {}", entry.kind, entry.target)))
        .arg(value(&entry.action));
    ui::info(&e.lang, "notify", &a)
}

// ----- Tab completion -----

/// Completions for the last argument.
pub fn complete(e: &mut Engine, who: &Who, args: &[String], online: &[String], now: u64) -> Vec<String> {
    let typed = args.last().map(|s| s.to_lowercase()).unwrap_or_default();
    let n = args.len();
    let first = lower(args.first());
    let mut options: Vec<String> = Vec::new();
    match n {
        0 | 1 => {
            for (word, action) in [
                ("user", "user."),
                ("group", "group."),
                ("track", "track."),
                ("groups", "group.list"),
                ("tracks", "track.list"),
                ("creategroup", "group.create"),
                ("deletegroup", "group.delete"),
                ("createtrack", "track.create"),
                ("deletetrack", "track.delete"),
                ("promote", "user.promote"),
                ("demote", "user.demote"),
                ("log", "log"),
                ("export", "export"),
                ("import", "import"),
                ("editor", "editor"),
                ("reload", "reload"),
                ("version", "version"),
                ("help", ""),
            ] {
                let ok = action.is_empty()
                    || if action.ends_with('.') {
                        allowed_prefix(e, who, action, now)
                    } else {
                        allowed(e, who, action, now)
                    };
                if ok {
                    options.push(word.to_string());
                }
            }
        }
        2 => match first.as_str() {
            "user" | "u" | "promote" | "demote" => options.extend(online.iter().cloned()),
            "group" | "g" | "deletegroup" => options.extend(e_groups(e)),
            "track" | "t" | "deletetrack" => options.extend(e.perms.tracks().map(|t| t.name.clone())),
            _ => {}
        },
        3 => {
            match first.as_str() {
                "user" | "u" => options
                    .extend(["info", "permission", "parent", "meta", "promote", "demote", "clear"].map(String::from)),
                "group" | "g" => options.extend(
                    [
                        "info",
                        "permission",
                        "parent",
                        "meta",
                        "create",
                        "delete",
                        "rename",
                        "setweight",
                        "setdisplayname",
                        "listmembers",
                    ]
                    .map(String::from),
                ),
                "track" | "t" => options
                    .extend(["info", "create", "delete", "append", "insert", "remove", "clear"].map(String::from)),
                "promote" | "demote" => options.extend(e.perms.tracks().map(|t| t.name.clone())),
                _ => {}
            }
        }
        _ => {
            let third = lower(args.get(2));
            let fourth = lower(args.get(3));
            match (first.as_str(), third.as_str()) {
                ("user" | "u" | "group" | "g", "permission" | "perm" | "p") if n == 4 => {
                    options.extend(["info", "set", "settemp", "unset", "unsettemp", "check", "clear"].map(String::from))
                }
                ("user" | "u" | "group" | "g", "parent" | "parents" | "inherit") if n == 4 => {
                    options.extend(["info", "add", "addtemp", "remove", "set"].map(String::from))
                }
                ("user" | "u" | "group" | "g", "parent" | "parents" | "inherit") if n == 5 => {
                    options.extend(e_groups(e))
                }
                ("user" | "u" | "group" | "g", "meta") if n == 4 => options.extend(
                    [
                        "info",
                        "set",
                        "unset",
                        "setprefix",
                        "addprefix",
                        "addtempprefix",
                        "removeprefix",
                        "setsuffix",
                        "addsuffix",
                        "addtempsuffix",
                        "removesuffix",
                    ]
                    .map(String::from),
                ),
                ("user" | "u", "promote" | "demote") if n == 4 => {
                    options.extend(e.perms.tracks().map(|t| t.name.clone()))
                }
                ("track" | "t", "append" | "insert" | "remove") if n == 4 => options.extend(e_groups(e)),
                ("user" | "u" | "group" | "g", "permission" | "perm" | "p")
                    if n == 5 && fourth != "clear" && fourth != "info" =>
                {
                    options.extend(e.perms.all_nodes())
                }
                ("user" | "u" | "group" | "g", "permission" | "perm" | "p") if n == 6 && fourth.starts_with("set") => {
                    options.extend(["true", "false"].map(String::from))
                }
                _ => {}
            }
            if n >= 5 && typed.contains('=') {
                // contexts are completed by key only
            } else if n >= 5 && options.is_empty() {
                options.extend(context::KEYS.iter().map(|k| format!("{k}=")));
            }
        }
    }
    options.sort();
    options.dedup();
    options.retain(|o| o.to_lowercase().starts_with(&typed));
    options.truncate(100);
    options
}

fn e_groups(e: &Engine) -> Vec<String> {
    e.perms.groups().map(|g| g.name.clone()).collect()
}

/// Every permission node of PumboPerms (`pumbo.perms.<action>`).
pub fn own_nodes() -> Vec<String> {
    ACTIONS.iter().map(|a| permission(PLUGIN, a)).collect()
}
