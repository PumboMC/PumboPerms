//! The `/pp` command (`/pumboperms`): parsing, permission checks, changes,
//! replies.
//!
//! The platform passes the arguments, who sent them and a few things only it
//! knows (online players, the time); it gets back the reply, whether the data
//! changed (so it updates online players), the log entry (for notifications)
//! and actions that need files (reload, export, import).

use pumbo_common::command::permission;
use pumbo_common::text::Args;

use crate::engine::Engine;
use crate::perms::KnownName;
use crate::store::LogEntry;
use crate::ui::{self, Text, value};

mod complete;
mod edit;
mod help;
mod holder;
mod import;
mod target;
mod track;
mod view;

pub use complete::{complete, own_nodes, tree_lines};
pub use import::{import, import_pending, import_source, on_join};
pub use view::{log_line, notification};

use help::general_help;
use holder::{group_cmd, user_cmd};
use track::track_cmd;
use view::{groups_view, log_view, tracks_view, version};

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
