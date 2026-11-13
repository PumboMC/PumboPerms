//! PumboPerms for the PumboProx proxy: ranks and permissions for the whole
//! network, set in one place.
//!
//! The rules live in `pumbo-perms-core`; this crate connects them to the proxy:
//!
//! - it is the proxy's permission provider (`pumbo:permissions@1.0`): the
//!   proxy gets every decision of a player in the global context and in the
//!   context of each server, at join (`on-permission-load`) and again after
//!   every change (`permissions.replace`). The proxy answers checks from that
//!   copy and hands the decisions to the servers through PumboBridge, so a
//!   change shows everywhere at once without calling this plugin per check,
//! - `/pp` (also `/pumboperms <sub>`), from players
//!   and the console; contexts are `server=<name>` and `group=<server group>`,
//! - at the first start it takes over the proxy's `permissions.yml` (the proxy
//!   sends it through the service method `file`); players named there only by
//!   nickname get their entries at their first join,
//! - for others: `check-offline` (offline players), `export` (PumboBridge
//!   `perms-export`: only the flag that the proxy rules, so a PumboPerms on
//!   a server steps back) and `info` (rank, prefix, meta of a player on a
//!   server, which PumboBridge carries to the PumboPerms there).
//!
//! [`setup`] holds the parts that do not need the host.

// Panics trap the host and stdout is lost; these stay errors outside tests.
#![deny(
    clippy::string_slice,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::await_holding_lock,
    clippy::await_holding_refcell_ref,
    clippy::integer_division,
    clippy::modulo_arithmetic,
    clippy::unchecked_time_subtraction,
    clippy::get_unwrap,
    clippy::exit,
    clippy::large_stack_arrays,
    clippy::large_stack_frames,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::significant_drop_in_scrutinee
)]
#![cfg_attr(
    test,
    allow(
        clippy::string_slice,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::await_holding_lock,
        clippy::await_holding_refcell_ref,
        clippy::integer_division,
        clippy::modulo_arithmetic,
        clippy::unchecked_time_subtraction,
        clippy::get_unwrap,
        clippy::exit,
        clippy::large_stack_arrays,
        clippy::large_stack_frames,
        clippy::unwrap_in_result,
        clippy::panic_in_result_fn,
        clippy::significant_drop_in_scrutinee
    )
)]

pub mod setup;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use pumbo_common::id::Uuid;
use pumbo_common::rich::Text as Rich;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, COMMAND_NODE, Who};
use pumbo_perms_core::context::Query;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::export::Snapshot;
use pumbo_perms_core::store::PermsStore;
use pumbo_perms_core::ui;
use pumbo_sdk::{CallReject, Command, PermissionEntry, PlayerId, PlayerInfo, Text, log, players, scheduler, servers};

mod commands;
mod events;
mod push;

/// The source name of the proxy's `permissions.yml` (`/pp import ... file`).
pub const SOURCE: &str = "file";
/// Expired nodes and changed server lists are looked for this often.
const TICK_MS: u64 = 1000;
/// Players recomputed between two pauses of [`PumboPerms::push_all`].
const PORTION: usize = 25;
/// A broken database is logged and read again after this long.
const BROKEN_LOG_MS: u64 = 60_000;
/// The full name of the main command: `/pumbo<short-name>`.
const FULL_NAME: &str = "pumboperms";
/// `/pp` subcommands also under `/pumboperms` (`reload`, `version` and `debug`
/// there are the proxy's own), with aliases.
const UMBRELLA: &[(&str, &[&str])] = &[
    ("help", &[]),
    ("info", &[]),
    ("log", &[]),
    ("export", &[]),
    ("import", &[]),
    ("groups", &["listgroups"]),
    ("tracks", &["listtracks"]),
    ("creategroup", &[]),
    ("deletegroup", &[]),
    ("createtrack", &[]),
    ("deletetrack", &[]),
    ("promote", &[]),
    ("demote", &[]),
    ("user", &["u"]),
    ("group", &["g"]),
    ("track", &["t"]),
];

struct State {
    engine: Engine,
    cfg: setup::Config,
    /// The short commands registered at start (`commands.aliases`).
    aliases: Vec<String>,
    /// Players the proxy loaded from this plugin: UUID, the last set the proxy
    /// took and the info (`ipc::Info` as JSON) on the player's server then.
    loaded: HashMap<PlayerId, (String, Vec<PermissionEntry>, String)>,
    /// `permissions.yml` as the proxy sent it last (fingerprint, contents),
    /// or why it cannot be imported.
    file: Option<Result<(String, Snapshot), String>>,
    timer: u64,
    /// The proxy refused `replace`: another provider is configured.
    not_provider: bool,
    /// When the broken database was last logged.
    broken_logged: Option<u64>,
}

pub struct PumboPerms {
    state: RefCell<Option<State>>,
    /// Read-only config folder (`plugins/pumbo-perms/`).
    pub config_dir: String,
    /// Data folder (`plugins/data/pumbo-perms/`): `perms.redb`, `exports/`
    /// (and `imports/` of a database before schema 2).
    pub data_dir: String,
}

impl Default for PumboPerms {
    fn default() -> Self {
        PumboPerms { state: RefCell::new(None), config_dir: "/config".into(), data_dir: "/data".into() }
    }
}

fn uuid_of(p: &PlayerInfo) -> String {
    Uuid::from_high_low(p.profile.id.high, p.profile.id.low).to_string()
}

/// A player as `/pp` sees the sender: on their server, no operator level.
fn who_of(p: &PlayerInfo) -> Who {
    let server = if p.in_virtual { String::new() } else { p.context.server.clone().unwrap_or_default() };
    Who { server, ..Who::player(&p.profile.name, &uuid_of(p), 0, "") }
}

fn json(t: &Rich) -> Text {
    Text::Json(pumbo_common::rich::json(t))
}

/// The host cuts a text longer than this (`MAX_TEXT` in pumbo-host).
const MAX_TEXT: usize = 32 * 1024;

/// `t` in messages whose JSON stays under [`MAX_TEXT`], cut between lines.
fn messages(t: &Rich) -> Vec<Text> {
    let mut out = Vec::new();
    let (mut part, mut size) = (Rich::new(), 0);
    for line in &t.lines {
        // One line alone costs a little more than inside a message, so the sum is an upper bound.
        let n = pumbo_common::rich::json(&Rich::from(line.clone())).len();
        if size + n > MAX_TEXT && !part.is_empty() {
            out.push(json(&std::mem::take(&mut part)));
            size = 0;
        }
        part.push(line.clone());
        size += n;
    }
    if !part.is_empty() {
        out.push(json(&part));
    }
    out
}

fn reply(to: Option<PlayerId>, t: &Rich) {
    if t.is_empty() {
        return;
    }
    match to {
        Some(id) => messages(t).into_iter().for_each(|m| players::send_message(id, m)),
        None => log::info(&t.plain()),
    }
}

/// Server groups of every server of the proxy.
fn server_groups() -> BTreeMap<String, Vec<String>> {
    servers::all()
        .into_iter()
        .map(|s| (s.name.to_lowercase(), s.groups.iter().map(|g| g.to_lowercase()).collect()))
        .collect()
}

/// A place from a contract context string (`global`, `server=x`, `group=x`).
fn query_of(e: &Engine, context: &str) -> Query {
    match context.split_once('=') {
        Some(("server", s)) => e.place(s, ""),
        Some(("group", g)) => Query::at("", vec![g.to_string()], ""),
        _ => Query::default(),
    }
}

fn cbor<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, CallReject> {
    let mut out = Vec::new();
    pumbo_sdk::ciborium::into_writer(v, &mut out).map_err(|e| CallReject::Rejected(e.to_string()))?;
    Ok(out)
}

fn uncbor<T: serde::de::DeserializeOwned>(b: &[u8]) -> Result<T, CallReject> {
    pumbo_sdk::ciborium::from_reader(b).map_err(|e| CallReject::Rejected(format!("bad request: {e}")))
}

impl PumboPerms {
    /// Runs `f` with the state; `None` when it is missing or borrowed. Never
    /// `.await` inside `f`. Logs the warnings of the engine afterwards.
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let mut state = self.state.try_borrow_mut().ok()?;
        let s = state.as_mut()?;
        let out = f(s);
        s.engine.take_warnings().iter().for_each(|line| log::warn(line));
        Some(out)
    }

    /// Loads config, messages and the database, registers the commands. A
    /// broken `config.yml` starts from the last good copy, or not at all.
    pub fn start(&self, store: Result<PermsStore, pumbo_common::store::StoreError>) -> Result<(), String> {
        let (cfg, mut warnings) = setup::load_config_at_start(&self.config_dir, &self.data_dir).map_err(|e| {
            log::error(&format!("PumboPerms: config: {e}"));
            e
        })?;
        let (lang, w) = setup::load_lang(&self.config_dir, &cfg.language);
        warnings.extend(w);
        for w in &warnings {
            if w.fatal {
                log::error(&format!("PumboPerms: config: {w}"));
            } else {
                log::warn(&format!("PumboPerms: config: {w}"));
            }
        }
        let mut engine = Engine::new(store, cfg.core(), lang);
        if let Some(line) = engine.migrate_ledgers(std::path::Path::new(&format!("{}/imports", self.data_dir))) {
            log::info(&line);
        }
        if let Some(why) = &engine.broken {
            // Fail closed: the proxy refuses logins until the database works.
            log::error(&format!("PumboPerms: the database is not available, changes are refused: {why}"));
        }
        engine.server_groups = server_groups();
        let _ = std::fs::create_dir_all(format!("{}/exports", self.data_dir));
        // A short command another plugin holds is refused by the proxy.
        let (wanted, _) = cfg.commands.aliases_to_register(FULL_NAME, |_| None);
        let mut aliases = Vec::new();
        for alias in wanted {
            match Command::new(&alias).permission(COMMAND_NODE).usage(&format!("/{alias} help")).register() {
                Ok(()) => aliases.push(alias),
                Err(e) => log::warn(&format!("PumboPerms: /{alias} not registered, /{FULL_NAME} works: {e}")),
            }
        }
        engine.root = format!("/{}", aliases.first().map_or(FULL_NAME, String::as_str));
        let off =
            |name: &str| command::subcommand(name).is_some_and(|main| engine.cfg.disabled.iter().any(|d| d == main));
        for (name, other) in UMBRELLA.iter().filter(|(name, _)| !off(name)) {
            let mut c =
                Command::new(name).permission(COMMAND_NODE).usage(&format!("{} {name}", engine.root)).umbrella();
            for a in *other {
                c = c.alias(a);
            }
            if let Err(e) = c.register() {
                log::warn(&format!("PumboPerms: /{FULL_NAME} {name} not registered: {e}"));
            }
        }
        let summary = format!(
            "PumboPerms {} loaded: {} groups, {} users, {} tracks, {} servers",
            env!("CARGO_PKG_VERSION"),
            engine.perms.groups().count(),
            engine.user_count(),
            engine.perms.tracks().count(),
            engine.server_groups.len(),
        );
        let timer = scheduler::every(TICK_MS);
        *self.state.borrow_mut() = Some(State {
            engine,
            cfg,
            aliases,
            loaded: HashMap::new(),
            file: None,
            timer,
            not_provider: false,
            broken_logged: None,
        });
        log::info(&summary);
        Ok(())
    }

    /// `/pp reload` and `/pumboperms reload`: config and messages (the
    /// database stays). A file that is not valid YAML changes nothing.
    fn reload(&self) -> Result<Rich, String> {
        let (mut cfg, mut warnings) = setup::load_config(&self.config_dir);
        let (lang, w) = setup::load_lang(&self.config_dir, &cfg.language);
        warnings.extend(w);
        if let Some(w) = warnings.iter().find(|w| w.fatal) {
            log::warn(&format!("PumboPerms: reload refused, the current settings stay: {w}"));
            return Err(w.message.clone());
        }
        warnings.extend(pumbo_common::config::keep_last_good(&self.config_dir, &self.data_dir));
        if let Some(Some(registered)) =
            self.with(|s| (cfg.commands.aliases != s.cfg.commands.aliases).then(|| s.cfg.commands.aliases.clone()))
        {
            warnings.push(pumbo_common::config::Warning::new(
                Some("commands.aliases"),
                "commands.aliases changes after /prox plugins reload pumbo-perms",
            ));
            cfg.commands.aliases = registered;
        }
        if self.with(|s| cfg.commands.disabled != s.cfg.commands.disabled) == Some(true) {
            warnings.push(pumbo_common::config::Warning::new(
                Some("commands.disabled"),
                "commands.disabled applies to /pumboperms <subcommand> after /prox plugins reload pumbo-perms",
            ));
        }
        for w in &warnings {
            log::warn(&format!("PumboPerms: config: {w}"));
        }
        let n = warnings.len();
        self.with(|s| {
            s.engine.cfg = cfg.core();
            s.engine.lang = lang;
            s.cfg = cfg;
            if n == 0 {
                ui::success(&s.engine.lang, "command-reloaded", &Args::new())
            } else {
                ui::warn(&s.engine.lang, "reloaded-warnings", &Args::new().arg(ui::value(n)))
            }
        })
        .ok_or_else(|| "PumboPerms is not loaded".to_string())
    }
}

pumbo_sdk::plugin!(PumboPerms, suggest);
pumbo_sdk::embed!(
    manifest = "pumbo-perms.yml",
    config = "assets/config.yml",
    lang = ["assets/lang/en.yml", "assets/lang/pl.yml"],
);

#[cfg(test)]
mod tests;
