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
//! - `/pp` (also `/pumboperms <sub>` and `/pumbo perms <sub>`), from players
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

use pumbo_common::clock::now_ms;
use pumbo_common::id::Uuid;
use pumbo_common::rich::Text as Rich;
use pumbo_common::store::Store;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, Action, Env, SourceMode, Who, split_args};
use pumbo_perms_core::context::Query;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::ipc;
use pumbo_perms_core::merge::Ledger;
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::store::PermsStore;
use pumbo_perms_core::ui;
use pumbo_sdk::contracts::{
    CheckOffline, CheckOfflineAnswer, HOST, METHOD_CHECK_OFFLINE, METHOD_EXPORT, METHOD_FILE, METHOD_INFO, PERMISSIONS,
    PERMISSIONS_HOST_ONLY, PermissionsExport, PermissionsFile, PermissionsInfo, PermissionsInfoRequest,
};
use pumbo_sdk::{
    CallReject, Command, CommandEvent, Context, PermissionEntry, PermissionSet, PlaceholderRequest, PlayerId,
    PlayerInfo, QueryContext, ServiceCall, Text, log, permissions, players, scheduler, servers,
};

/// The source name of the proxy's `permissions.yml` (`/pp import ... file`).
pub const SOURCE: &str = "file";
/// Expired nodes and changed server lists are looked for this often.
const TICK_MS: u64 = 1000;
/// A broken database is logged again after this long.
const BROKEN_LOG_MS: u64 = 60_000;
/// Whether a player sees `/pp` at all (set for everyone with any `/pp` right).
const COMMAND_NODE: &str = "pumbo.perms.command";
/// `/pp` subcommands also under `/pumboperms` and `/pumbo perms` (`reload`,
/// `version` and `debug` there are the proxy's own), with aliases.
const UMBRELLA: &[(&str, &[&str])] = &[
    ("help", &[]),
    ("info", &[]),
    ("log", &[]),
    ("export", &[]),
    ("import", &[]),
    ("editor", &[]),
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
    /// Players the proxy loaded from this plugin: UUID, the last set the proxy
    /// took and the info (`ipc::Info` as JSON) on the player's server then.
    loaded: HashMap<PlayerId, (String, Vec<PermissionEntry>, String)>,
    /// `permissions.yml` as the proxy sent it last (fingerprint, contents).
    file: Option<(String, Snapshot)>,
    /// What `permissions.yml` added (`imports/file.json`).
    ledger: Ledger,
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
    /// Data folder (`plugins/data/pumbo-perms/`): `perms.redb`, `imports/`, `exports/`.
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

fn json(t: &Rich) -> Text {
    Text::Json(pumbo_common::rich::json(t))
}

fn reply(to: Option<PlayerId>, t: &Rich) {
    if t.is_empty() {
        return;
    }
    match to {
        Some(id) => players::send_message(id, json(t)),
        None => log::info(&t.plain()),
    }
}

/// Server groups of every server of the proxy.
fn server_groups() -> BTreeMap<String, Vec<String>> {
    servers::all().into_iter().map(|s| (s.name.to_lowercase(), s.groups)).collect()
}

/// [`setup::provider_set`] as permission entries for the proxy.
fn entries(e: &mut Engine, uuid: &str, now: u64) -> Vec<PermissionEntry> {
    setup::provider_set(e, uuid, now)
        .into_iter()
        .map(|(server, node, value)| PermissionEntry {
            node,
            value,
            context: match server {
                Some(s) => Context::Server(s),
                None => Context::Global,
            },
        })
        .collect()
}

fn ledger_path(dir: &str) -> String {
    format!("{dir}/imports/{SOURCE}.json")
}

fn load_ledger(dir: &str) -> Ledger {
    std::fs::read_to_string(ledger_path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_ledger(dir: &str, ledger: &Ledger) {
    let path = ledger_path(dir);
    let result = if *ledger == Ledger::default() {
        std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
    } else {
        let _ = std::fs::create_dir_all(format!("{dir}/imports"));
        std::fs::write(&path, serde_json::to_string_pretty(ledger).unwrap_or_default())
    };
    if let Err(e) = result {
        log::warn(&format!("PumboPerms: cannot save {path}: {e}"));
    }
}

/// [`ipc::info`] on `server` (empty: global) as JSON, built as the host's
/// `info` call builds it: the server and its groups, no fallback context.
fn info_json(e: &mut Engine, uuid: &str, server: &str, now: u64) -> String {
    let groups = e.server_groups.get(&server.to_lowercase()).cloned().unwrap_or_default();
    let q = Query::at(server, groups, "");
    ipc::info(e, uuid, &q, now).and_then(|i| serde_json::to_string(&i).ok()).unwrap_or_default()
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
    /// `.await` inside `f`.
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        self.state.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f))
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
        if let Some(why) = &engine.broken {
            // Fail closed: the proxy refuses logins until the database works.
            log::error(&format!("PumboPerms: the database is not available, changes are refused: {why}"));
        }
        engine.server_groups = server_groups();
        let _ = std::fs::create_dir_all(format!("{}/exports", self.data_dir));
        let mut spec = vec![Command::new("pp").permission(COMMAND_NODE).usage("/pp help")];
        for (name, aliases) in UMBRELLA {
            let mut c = Command::new(name).permission(COMMAND_NODE).usage(&format!("/pp {name}")).umbrella();
            for a in *aliases {
                c = c.alias(a);
            }
            spec.push(c);
        }
        for c in spec {
            let name = c.0.name.clone();
            if let Err(e) = c.register() {
                log::warn(&format!("PumboPerms: /{name} not registered: {e}"));
            }
        }
        let summary = format!(
            "PumboPerms {} loaded: {} groups, {} users, {} tracks, {} servers",
            env!("CARGO_PKG_VERSION"),
            engine.perms.groups().count(),
            engine.perms.users().filter(|u| !u.data.is_empty()).count(),
            engine.perms.tracks().count(),
            engine.server_groups.len(),
        );
        let ledger = load_ledger(&self.data_dir);
        let timer = scheduler::every(TICK_MS);
        *self.state.borrow_mut() = Some(State {
            engine,
            cfg,
            loaded: HashMap::new(),
            file: None,
            ledger,
            timer,
            not_provider: false,
            broken_logged: None,
        });
        log::info(&summary);
        Ok(())
    }

    /// `/pp reload` and `/pumbo perms reload`: config and messages (the
    /// database stays). A file that is not valid YAML changes nothing.
    fn reload(&self) -> Result<Rich, String> {
        let (cfg, mut warnings) = setup::load_config(&self.config_dir);
        let (lang, w) = setup::load_lang(&self.config_dir, &cfg.language);
        warnings.extend(w);
        if let Some(w) = warnings.iter().find(|w| w.fatal) {
            log::warn(&format!("PumboPerms: reload refused, the current settings stay: {w}"));
            return Err(w.message.clone());
        }
        warnings.extend(pumbo_common::config::keep_last_good(&self.config_dir, &self.data_dir));
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

    /// Sends the proxy the new set of every loaded player whose set or info
    /// changed (a new set also makes the proxy ask for the info again).
    fn push_all(&self) {
        let groups = server_groups();
        let at: HashMap<PlayerId, String> =
            players::all().into_iter().map(|p| (p.id, p.context.server.unwrap_or_default())).collect();
        let now = now_ms();
        let changes = self
            .with(|s| {
                s.engine.server_groups = groups;
                let State { engine, loaded, .. } = s;
                let mut out = Vec::new();
                for (id, (uuid, last, last_info)) in loaded.iter() {
                    let set = entries(engine, uuid, now);
                    let info = info_json(engine, uuid, at.get(id).map_or("", String::as_str), now);
                    if *last != set || *last_info != info {
                        out.push((*id, set, info));
                    }
                }
                out
            })
            .unwrap_or_default();
        for (id, set, info) in changes {
            if pumbo_sdk::debug_enabled() {
                log::debug(&format!("PumboPerms: player {id}: {} entries", set.len()));
            }
            let sent = PermissionSet { entries: set };
            let result = permissions::replace(id, &sent);
            if result.is_ok() {
                // Kept only once the proxy has it: a refused set is sent again next time.
                self.with(|s| {
                    if let Some(l) = s.loaded.get_mut(&id) {
                        (l.1, l.2) = (sent.entries, info);
                    }
                });
            }
            if let Err(e) = result {
                let first = self.with(|s| !std::mem::replace(&mut s.not_provider, true)).unwrap_or(false);
                if first {
                    log::warn(&format!(
                        "PumboPerms: the proxy did not take the permissions ({e}); set permissions.provider to auto or pumbo-perms in pumboprox.yml"
                    ));
                }
            }
        }
    }

    fn version_rows(s: &State) -> Vec<(String, String)> {
        let l = &s.engine.lang;
        let mode = if s.not_provider { l.get("mode-not-provider") } else { l.get("mode-provider") };
        vec![
            (l.get("version-platform"), "PumboProx".to_string()),
            (l.get("version-mode"), mode),
            (l.get("version-online"), s.loaded.len().to_string()),
            (l.get("version-servers"), s.engine.server_groups.len().to_string()),
        ]
    }

    /// Runs `/pp <args>` for a player (`Some`) or the console.
    fn run(&self, player: Option<PlayerId>, args: Vec<String>) {
        let who = match player {
            None => Who::console(),
            Some(id) => {
                let Some(p) = players::get(id) else { return };
                let server = if p.in_virtual { String::new() } else { p.context.server.clone().unwrap_or_default() };
                Who { server, ..Who::player(&p.profile.name, &uuid_of(&p), 0, "") }
            }
        };
        let online = players::all();
        let now = now_ms();
        let dir = self.data_dir.clone();
        let mut reload = false;
        let result = self.with(|s| {
            let rows = Self::version_rows(s);
            let lookup = |name: &str| {
                online
                    .iter()
                    .find(|p| p.profile.name.eq_ignore_ascii_case(name))
                    .map(|p| KnownName { uuid: uuid_of(p), name: p.profile.name.clone() })
            };
            let env = Env { now, online: &lookup, version: env!("CARGO_PKG_VERSION"), version_rows: rows };
            let out = command::run(&mut s.engine, &who, &args, &env);
            let mut text = out.reply;
            let mut changed = out.changed;
            let mut log_entry = out.log;
            match out.action {
                Some(Action::Reload) => reload = true,
                Some(Action::Export(name)) => text = export_file(&s.engine, &dir, &name, now).unwrap_or_else(|e| e),
                Some(Action::Import(name)) => {
                    let o = import_file(&mut s.engine, &dir, &who, &name, now);
                    text = o.reply;
                    changed |= o.changed;
                    log_entry = o.log.or(log_entry);
                }
                Some(Action::Source(source, mode)) => {
                    let data = match (&s.file, source == SOURCE) {
                        (_, false) => {
                            Err(format!("unknown source {source}; on PumboProx it is {SOURCE} (permissions.yml)"))
                        }
                        (None, true) if mode != SourceMode::Undo => {
                            Err("the proxy has not sent permissions.yml yet".to_string())
                        }
                        (f, true) => f.clone().ok_or_else(String::new),
                    };
                    let (o, ledger) = command::import_source(&mut s.engine, &who, &source, data, &s.ledger, mode, now);
                    if let Some(l) = ledger {
                        save_ledger(&dir, &l);
                        s.ledger = l;
                    }
                    text = o.reply;
                    changed |= o.changed;
                }
                None => {}
            }
            let notice = log_entry.filter(|_| s.cfg.log.notify).map(|entry| {
                let line =
                    format!("PumboPerms: {} changed {} {}: {}", entry.actor, entry.kind, entry.target, entry.action);
                (command::notification(&s.engine, &entry), line)
            });
            (text, changed, notice)
        });
        let Some((mut text, changed, notice)) = result else {
            reply(player, &Rich::parse("&cPumboPerms is busy, try again."));
            return;
        };
        if reload {
            text = match self.reload() {
                Ok(t) => t,
                Err(e) => self
                    .with(|s| ui::error(&s.engine.lang, "command-reload-failed", &Args::new().arg(ui::value(&e))))
                    .unwrap_or_default(),
            };
        }
        reply(player, &text);
        if changed {
            self.push_all();
        }
        if let Some((message, line)) = notice {
            log::info(&line);
            for p in &online {
                if Some(p.id) != player && permissions::has(p.id, "pumbo.perms.log.notify", &QueryContext::Current) {
                    players::send_message(p.id, json(&message));
                }
            }
        }
    }

    /// `permissions.yml` from the proxy: imported at the first start (with
    /// `import.follow` again whenever it changes); the same file again does
    /// nothing.
    fn take_file(&self, f: PermissionsFile) {
        let snap = match export::parse(&f.data) {
            Ok(s) => s,
            Err(e) => {
                log::warn(&format!("PumboPerms: permissions.yml cannot be imported: {e}"));
                return;
            }
        };
        let now = now_ms();
        let dir = self.data_dir.clone();
        let line = self
            .with(|s| {
                s.file = Some((f.fingerprint.clone(), snap.clone()));
                if !s.cfg.import.enabled || s.ledger.fingerprint == f.fingerprint {
                    return None;
                }
                if s.ledger.at != 0 && !s.cfg.import.follow {
                    return Some((
                        "changed since it was imported; /pp import preview file shows what it would add, /pp import run file imports it".to_string(),
                        false,
                    ));
                }
                let (out, ledger) = command::import_source(
                    &mut s.engine,
                    &Who::console(),
                    SOURCE,
                    Ok((f.fingerprint, snap)),
                    &s.ledger,
                    SourceMode::Run,
                    now,
                );
                if let Some(l) = ledger {
                    save_ledger(&dir, &l);
                    s.ledger = l;
                }
                Some((out.reply.plain(), out.changed))
            })
            .flatten();
        if let Some((text, changed)) = line {
            for l in text.lines() {
                log::info(&format!("permissions.yml: {l}"));
            }
            if changed {
                self.push_all();
            }
        }
    }
}

fn export_file(e: &Engine, dir: &str, name: &str, now: u64) -> Result<Rich, Rich> {
    let folder = format!("{dir}/exports");
    let json = export::dump(&e.perms, now);
    let result = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(format!("{folder}/{name}.json"), json));
    let shown = format!("plugins/data/{}/exports/{name}.json", setup::PLUGIN_ID);
    match result {
        Ok(()) => Ok(ui::success(&e.lang, "exported", &Args::new().arg(ui::value(shown)))),
        Err(err) => Err(ui::error(&e.lang, "export-failed", &Args::new().arg(ui::value(err)))),
    }
}

fn import_file(e: &mut Engine, dir: &str, who: &Who, name: &str, now: u64) -> command::Outcome {
    let Ok(json) = std::fs::read_to_string(format!("{dir}/exports/{name}.json")) else {
        let reply = ui::error(&e.lang, "import-missing", &Args::new().arg(ui::value(format!("exports/{name}.json"))));
        return command::Outcome { reply, ..command::Outcome::default() };
    };
    let backup = format!("before-import-{}", export::default_file_name(now));
    if let Err(reply) = export_file(e, dir, &backup, now) {
        return command::Outcome { reply, ..command::Outcome::default() };
    }
    command::import(e, who, name, &json, &format!("exports/{backup}.json"), now)
}

impl pumbo_sdk::Plugin for PumboPerms {
    async fn init(&self) -> Result<(), String> {
        pumbo_common::panic::install(log::error);
        let store = Store::open(format!("{}/perms.redb", self.data_dir)).and_then(PermsStore::new);
        self.start(store)
    }

    async fn on_reload(&self) -> Result<(), String> {
        self.reload().map(|_| ())
    }

    async fn on_command(&self, e: CommandEvent) {
        // `/pp <args>`, or a subcommand of `/pumboperms` and `/pumbo perms`.
        let mut words = if e.name == "pp" { Vec::new() } else { vec![e.name] };
        words.extend(e.args);
        self.run(e.player, split_args(&words.join(" ")));
    }

    /// `%pumboperms_prefix%`, `_suffix`, `_rank`, `_group`, `_groups` and
    /// `%pumboperms_meta:<key>%` of the player on their current server.
    async fn on_placeholder(&self, reqs: Vec<PlaceholderRequest>) -> Vec<Option<Text>> {
        let now = now_ms();
        let mut state = self.state.borrow_mut();
        let Some(s) = state.as_mut() else {
            return vec![None; reqs.len()];
        };
        reqs.iter()
            .map(|r| {
                let uuid = s.loaded.get(&r.player?)?.0.clone();
                let q = s.engine.place(r.context.server.as_deref().unwrap_or(""), "");
                let info = ipc::info(&mut s.engine, &uuid, &q, now)?;
                ipc::placeholder(&info, &r.key, r.arg.as_deref()).map(Text::Legacy)
            })
            .collect()
    }

    async fn on_permission_load(&self, p: PlayerInfo) -> Result<PermissionSet, String> {
        let groups = server_groups();
        let (uuid, name) = (uuid_of(&p), p.profile.name.clone());
        let now = now_ms();
        let dir = self.data_dir.clone();
        let (set, joined) = self
            .with(|s| {
                if let Some(why) = &s.engine.broken {
                    return Err(format!("the database is not available: {why}"));
                }
                s.not_provider = false;
                s.engine.server_groups = groups;
                if let Err(e) = s.engine.seen(&uuid, &name) {
                    log::warn(&format!("PumboPerms: cannot save the name of {name}: {e}"));
                }
                let joined =
                    command::import_pending(&mut s.engine, SOURCE, &mut s.ledger, &uuid, &name, p.online_mode, now);
                if joined.as_ref().is_some_and(|o| o.changed) {
                    save_ledger(&dir, &s.ledger);
                }
                let set = entries(&mut s.engine, &uuid, now);
                let info = info_json(&mut s.engine, &uuid, p.context.server.as_deref().unwrap_or(""), now);
                s.loaded.insert(p.id, (uuid.clone(), set.clone(), info));
                Ok((set, joined.map(|o| o.reply.plain())))
            })
            .ok_or_else(|| "PumboPerms is not loaded".to_string())??;
        for l in joined.iter().flat_map(|t| t.lines()) {
            log::info(l);
        }
        Ok(PermissionSet { entries: set })
    }

    async fn on_disconnect(&self, p: PlayerId) {
        self.with(|s| s.loaded.remove(&p));
    }

    async fn on_timer(&self, timer: u64) {
        let groups = server_groups();
        let now = now_ms();
        let push = self
            .with(|s| {
                if s.timer != timer {
                    return false;
                }
                if let Some(why) = &s.engine.broken
                    && s.broken_logged.is_none_or(|t| now >= t.saturating_add(BROKEN_LOG_MS))
                {
                    s.broken_logged = Some(now);
                    log::error(&format!(
                        "PumboPerms: the database is not available ({why}); logins wait for it and changes are refused"
                    ));
                }
                let (due, saved) = s.engine.expire(now);
                if let Err(e) = saved {
                    log::warn(&format!("PumboPerms: cannot save the removal of expired entries, trying again: {e}"));
                }
                due || s.engine.server_groups != groups
            })
            .unwrap_or(false);
        if push {
            self.push_all();
        }
    }

    async fn on_service_call(&self, c: ServiceCall) -> Result<Vec<u8>, CallReject> {
        if c.service != PERMISSIONS.name {
            return Err(CallReject::UnknownMethod);
        }
        // Another plugin could replace every rank or read the whole database.
        if PERMISSIONS_HOST_ONLY.contains(&c.method.as_str()) && c.caller != HOST {
            return Err(CallReject::Rejected("host only".into()));
        }
        match c.method.as_str() {
            METHOD_CHECK_OFFLINE => {
                let req: CheckOffline = uncbor(&c.payload)?;
                let uuid = Uuid::parse(&req.uuid).ok_or_else(|| CallReject::Rejected("bad uuid".into()))?.to_string();
                let node =
                    pumbo_perms_core::node::normalize(&req.node).map_err(|e| CallReject::Rejected(e.to_string()))?;
                let now = now_ms();
                let value = self
                    .with(|s| {
                        let q = query_of(&s.engine, &req.context);
                        s.engine.perms.check(&uuid, &node, &q, now)
                    })
                    .ok_or_else(|| CallReject::Rejected("PumboPerms is busy".into()))?;
                cbor(&CheckOfflineAnswer { value })
            }
            METHOD_FILE => {
                self.take_file(uncbor(&c.payload)?);
                cbor(&true)
            }
            METHOD_EXPORT => {
                // Only the flag: servers behind the proxy step back and get
                // their sets through the bridge. The whole database would not
                // fit the host's payload limit, and nothing there reads it.
                let snap = Snapshot {
                    format: export::FORMAT.into(),
                    version: export::VERSION,
                    proxy_rules: true,
                    ..Snapshot::default()
                };
                cbor(&PermissionsExport { data: serde_json::to_string(&snap).unwrap_or_default() })
            }
            METHOD_INFO => {
                let req: PermissionsInfoRequest = uncbor(&c.payload)?;
                let uuid = Uuid::parse(&req.uuid).ok_or_else(|| CallReject::Rejected("bad uuid".into()))?.to_string();
                let now = now_ms();
                let data = self
                    .with(|s| {
                        let groups = req.groups.iter().map(|g| g.to_lowercase()).collect();
                        let q = Query::at(&req.server.to_lowercase(), groups, "");
                        ipc::info(&mut s.engine, &uuid, &q, now).and_then(|i| serde_json::to_string(&i).ok())
                    })
                    .ok_or_else(|| CallReject::Rejected("PumboPerms is busy".into()))?;
                cbor(&PermissionsInfo { data })
            }
            _ => Err(CallReject::UnknownMethod),
        }
    }
}

pumbo_sdk::plugin!(PumboPerms);
pumbo_sdk::embed!(
    manifest = "pumbo-perms.yml",
    config = "assets/config.yml",
    lang = ["assets/lang/en.yml", "assets/lang/pl.yml"],
);

#[cfg(test)]
mod tests;
