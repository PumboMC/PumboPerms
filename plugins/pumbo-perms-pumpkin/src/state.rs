//! The plugin state, reachable from every handler, and the decisions the
//! handlers make on it. Plain Rust: the event handlers in `plugin/` only read
//! the host, call these methods and apply the result, so the logic is tested
//! natively.
//!
//! A handler takes what it needs from the host first, decides inside [`with`]
//! and calls the host only after the state is released. That matters: a host
//! call can make Pumpkin check a permission, which (in check-event mode) comes
//! back into this plugin while the first handler still runs. Such a nested
//! call finds the state busy ([`with`] returns `None`) instead of waiting for
//! itself.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use pumbo_common::lang::Lang;
use pumbo_common::store::DbLock;
use pumbo_perms_core::command::{self, SourceMode, Who};
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::host::{self, Catalog, Op};
use serde::Deserialize;

use crate::config::Config;

/// What is known about an online player.
#[derive(Debug, Clone)]
pub struct Online {
    pub name: String,
    pub world: String,
    pub op_level: u8,
}

/// Host changes for one online player and what it has once they are applied
/// ([`Rt::applied`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub uuid: String,
    pub ops: Vec<Op>,
    pub wanted: HashMap<String, bool>,
}

pub struct Rt {
    pub dir: String,
    pub cfg: Config,
    pub engine: Engine,
    pub catalog: Catalog,
    pub online: HashMap<String, Online>,
    /// What is set on the host per online player. Pumpkin keeps attachments
    /// after a player leaves, so leaving takes them off and forgets them.
    pub pushed: HashMap<String, HashMap<String, bool>>,
    /// Whether the permission check event is answered (fixed at load).
    pub check_event: bool,
    /// `defaults.op-level` of the nodes registered in Pumpkin at load.
    pub registered_op_level: u8,
    /// When the periodic refresh last ran (ms).
    pub last_refresh: u64,
    /// Permission checks answered from memory (check-event mode).
    pub answered: u64,
    /// Log lines collected inside [`with`], written after it (true: a warning).
    pub logs: Vec<(bool, String)>,
    /// Error lines collected inside [`with`], written after it.
    pub errors: Vec<String>,
    /// The first import pass is done and the bridge released the
    /// attachments: from now on this plugin writes them (spec §5.3).
    pub ready: bool,
    /// A source answered "pending": ask again until this time (ms).
    pub waiting_until: Option<u64>,
    /// Wildcards last left out per player ([`host::hidden`]), logged once.
    pub hidden: HashMap<String, Vec<(String, String)>>,
    /// When the periodic import pass runs next (ms).
    pub next_pass: u64,
    /// Since when the first pass waits for a live export from PumboBridge,
    /// and whether that was logged.
    pub holding: Option<(u64, bool)>,
    /// `perms.redb.lock`: `None` while another instance holds the database
    /// (the engine is broken then). After the engine, so the database closes
    /// before the lock goes.
    pub lock: Option<DbLock>,
    /// When the lock was last refreshed, or the database last tried again (ms).
    pub lock_refreshed: u64,
    pub retried: u64,
    /// The nodes ever set on the host as the database lists them, read once.
    stored: Option<BTreeSet<String>>,
    /// When each online player last got its command list again (ms), and
    /// who waits for the next one ([`RESEND_MS`]).
    resent: HashMap<String, u64>,
    resend_waiting: BTreeSet<String>,
}

/// A player gets its command list again at most this often (20 ticks);
/// changes in between go out together.
pub const RESEND_MS: u64 = 1000;

/// The lock is refreshed at most this often; [`DbLock`] counts one not
/// refreshed for 30 s as stale.
pub const LOCK_REFRESH_MS: u64 = 10_000;
/// A database held by another instance is tried again this often.
pub const LOCK_RETRY_MS: u64 = 10_000;
/// A database that cannot be read is read again this often.
pub const READ_RETRY_MS: u64 = 60_000;

impl Rt {
    pub fn new(dir: String, cfg: Config, engine: Engine, check_event: bool) -> Self {
        Self::with_lock(dir, cfg, engine, check_event, None, 0)
    }

    /// [`Rt::new`] with the database lock taken at `now` (`None`: refused).
    pub fn with_lock(
        dir: String,
        cfg: Config,
        engine: Engine,
        check_event: bool,
        lock: Option<DbLock>,
        now: u64,
    ) -> Self {
        let mut rt = Self {
            dir,
            registered_op_level: cfg.defaults.op_level,
            last_refresh: 0,
            cfg,
            engine,
            catalog: Catalog::pumpkin(),
            online: HashMap::new(),
            pushed: HashMap::new(),
            check_event,
            answered: 0,
            logs: Vec::new(),
            ready: false,
            waiting_until: None,
            hidden: HashMap::new(),
            next_pass: 0,
            holding: None,
            lock,
            lock_refreshed: now,
            retried: now,
            errors: Vec::new(),
            stored: None,
            resent: HashMap::new(),
            resend_waiting: BTreeSet::new(),
        };
        rt.refresh_catalog();
        rt
    }

    /// Keeps the database lock fresh, at most every [`LOCK_REFRESH_MS`]. Every
    /// entry into the plugin calls this, so `/tick freeze` does not let the
    /// lock go stale while players use commands. A lock another instance took
    /// over stops all changes here.
    pub fn refresh_lock(&mut self, now: u64) {
        if now.saturating_sub(self.lock_refreshed) < LOCK_REFRESH_MS {
            return;
        }
        let Some(lock) = &self.lock else { return };
        self.lock_refreshed = now;
        if let Err(e) = lock.refresh(now) {
            self.lock = None;
            self.engine.close(e.to_string());
            self.error(format!("PumboPerms: {e}; changes are refused until it lets the database go"));
        }
    }

    /// From the periodic task: the database again after a failure. Without
    /// the lock (another instance had it) every [`LOCK_RETRY_MS`]: `open`
    /// opens it once the lock is free. A database that could not be read,
    /// every [`READ_RETRY_MS`]. Returns the host changes once it opened.
    pub fn retry_database(
        &mut self,
        now: u64,
        open: impl FnOnce(&Engine) -> Result<(DbLock, Engine), String>,
    ) -> Vec<Plan> {
        if self.engine.broken.is_none() {
            return Vec::new();
        }
        let wait = if self.lock.is_none() { LOCK_RETRY_MS } else { READ_RETRY_MS };
        if now.saturating_sub(self.retried) < wait {
            return Vec::new();
        }
        self.retried = now;
        if self.lock.is_some() && self.engine.store().is_some() {
            if self.engine.retry_open() {
                self.logs.push((false, "PumboPerms: the database can be read again; changes work again".into()));
                return self.plan_all(now);
            }
            let why = self.engine.broken.clone().unwrap_or_default();
            self.error(format!("PumboPerms: the database is not available, changes are refused: {why}"));
            return Vec::new();
        }
        // The file did not open under this lock: open it again, lock and all.
        self.lock = None;
        match open(&self.engine) {
            Ok((lock, engine)) => {
                self.lock = Some(lock);
                self.lock_refreshed = now;
                self.engine = engine;
                self.stored = None;
                if self.engine.broken.is_none() {
                    self.logs.push((false, "PumboPerms: the database is open now".into()));
                }
                self.refresh_catalog();
                self.plan_all(now)
            }
            Err(e) => {
                self.warn(format!("PumboPerms: {e}"));
                Vec::new()
            }
        }
    }

    pub fn error(&mut self, msg: String) {
        self.errors.push(msg);
    }

    pub fn lang(&self) -> &Lang {
        &self.engine.lang
    }

    pub fn warn(&mut self, msg: String) {
        self.logs.push((true, msg));
    }

    /// Whether to resend command lists after a change. Never in check-event
    /// mode: the resend (`set-permission-level`) makes Pumpkin check every
    /// command node from a pumping import, and a check handed back to this
    /// plugin before the host is ready for it hangs the server for good
    /// (Pumpkin 0.2.0 and 0.1.0-dev).
    pub fn resend(&self) -> bool {
        self.cfg.provider.resend_commands && !self.check_event
    }

    /// Rebuilds the list of nodes set on the host and remembers new ones in the
    /// database (so that they can be unset after a restart of the plugin).
    /// Returns whether the list changed.
    pub fn refresh_catalog(&mut self) -> bool {
        let Some(store) = self.engine.store() else {
            return self.catalog.refresh(&self.engine.perms, own_host_nodes());
        };
        if self.stored.is_none() {
            match store.host_nodes() {
                Ok(nodes) => self.stored = Some(nodes),
                Err(e) => self.logs.push((true, format!("PumboPerms: cannot read the node list: {e}"))),
            }
        }
        // Unread list: empty for this refresh only, read again next time.
        let mut once = BTreeSet::new();
        let stored = self.stored.as_mut().unwrap_or(&mut once);
        let changed =
            self.catalog.refresh(&self.engine.perms, own_host_nodes().into_iter().chain(stored.iter().cloned()));
        let new: Vec<String> = self.catalog.nodes().iter().filter(|n| !stored.contains(*n)).cloned().collect();
        match store.add_host_nodes(&new) {
            Ok(()) => stored.extend(new),
            Err(e) => self.logs.push((true, format!("PumboPerms: cannot save the node list: {e}"))),
        }
        changed
    }

    /// After the first pass took them off the online players: forgets the
    /// nodes nobody has in the data any more.
    fn forget_old_nodes(&mut self) {
        let mut fresh = Catalog::pumpkin();
        fresh.refresh(&self.engine.perms, own_host_nodes());
        let (Some(store), Some(stored)) = (self.engine.store(), self.stored.as_mut()) else { return };
        let old: Vec<String> = stored.iter().filter(|n| !fresh.nodes().contains(*n)).cloned().collect();
        match store.remove_host_nodes(&old) {
            Ok(()) => {
                for n in &old {
                    stored.remove(n);
                }
                self.catalog = fresh;
            }
            Err(e) => self.logs.push((true, format!("PumboPerms: cannot save the node list: {e}"))),
        }
    }

    /// After a change of the data: the node list again and the host changes
    /// for the online players the change can reach (all of them when the
    /// list changed).
    pub fn after_change(&mut self, now: u64) -> Vec<Plan> {
        let everyone = self.refresh_catalog();
        let online: Vec<String> = self.online.keys().cloned().collect();
        let affected: Vec<&str> = self.engine.take_affected(online.iter().map(String::as_str));
        let uuids: Vec<String> =
            online.iter().filter(|u| everyone || affected.contains(&u.as_str())).cloned().collect();
        uuids.into_iter().filter_map(|u| self.plan_player(&u, now)).collect()
    }

    /// Of the players whose command lists a change touched, those to send
    /// one now; the rest wait for [`Rt::due_resends`].
    pub fn resends(&mut self, touched: Vec<String>, now: u64) -> Vec<String> {
        let mut out = Vec::new();
        for uuid in touched {
            if self.resent.get(&uuid).is_some_and(|t| now.saturating_sub(*t) < RESEND_MS) {
                self.resend_waiting.insert(uuid);
            } else {
                self.resent.insert(uuid.clone(), now);
                out.push(uuid);
            }
        }
        out
    }

    /// The waiting players whose next command list is due.
    pub fn due_resends(&mut self, now: u64) -> Vec<String> {
        let due: Vec<String> = self
            .resend_waiting
            .iter()
            .filter(|u| self.resent.get(*u).is_none_or(|t| now.saturating_sub(*t) >= RESEND_MS))
            .cloned()
            .collect();
        for uuid in &due {
            self.resend_waiting.remove(uuid);
            self.resent.insert(uuid.clone(), now);
        }
        due
    }

    /// The host changes for one online player; `None` when there are none.
    /// Nothing before the first import pass (the bridge may still own them),
    /// and nothing while the proxy's PumboPerms rules ([`passive`]).
    pub fn plan_player(&mut self, uuid: &str, now: u64) -> Option<Plan> {
        if !self.ready || passive() {
            return None;
        }
        let o = self.online.get(uuid).cloned()?;
        let (wanted, hidden) =
            host::wanted_and_hidden(&mut self.engine, uuid, &o.world, o.op_level, now, &self.catalog);
        if self.hidden.get(uuid).map_or(&[][..], Vec::as_slice) != hidden.as_slice() {
            for (w, narrower) in &hidden {
                self.warn(format!("PumboPerms: not sent to Pumpkin for {}: {w} hides {narrower}", o.name));
            }
            if hidden.is_empty() {
                self.hidden.remove(uuid);
            } else {
                self.hidden.insert(uuid.to_string(), hidden);
            }
        }
        let ops = host::plan(&wanted, self.pushed.get(uuid), &self.catalog);
        (!ops.is_empty()).then(|| Plan { uuid: uuid.to_string(), ops, wanted })
    }

    /// Remembers what the players have on the host once the plans are
    /// applied; a player who left meanwhile is skipped (leaving took
    /// everything off).
    pub fn applied(&mut self, plans: Vec<Plan>) {
        for p in plans {
            if self.online.contains_key(&p.uuid) {
                self.pushed.insert(p.uuid, p.wanted);
            }
        }
    }

    /// A player joined (or was online at load): the host changes to apply.
    /// Pumpkin does not say whether Mojang checked the account, so the name
    /// counts as offline (an import never binds it here).
    pub fn join(&mut self, uuid: &str, online: Online, now: u64) -> Vec<Plan> {
        match self.engine.seen(uuid, &online.name, false) {
            Ok(Some(moved)) => self.warn(moved),
            Ok(None) => {}
            Err(e) => self.warn(format!("PumboPerms: cannot save the name of {}: {e}", online.name)),
        }
        for out in command::on_join(&mut self.engine, uuid, &online.name, false, now) {
            self.logs.push((true, out.reply.plain()));
        }
        self.online.insert(uuid.to_string(), online);
        let mut plans = self.due_expiry(now);
        if !plans.iter().any(|p| p.uuid == uuid) {
            plans.extend(self.plan_player(uuid, now));
        }
        plans
    }

    /// A player left: everything this plugin set on them goes (Pumpkin keeps
    /// attachments until it stops) and is forgotten, so the next join sets
    /// the whole catalog again.
    pub fn leave(&mut self, uuid: &str, now: u64) -> (Vec<Op>, Vec<Plan>) {
        self.online.remove(uuid);
        self.hidden.remove(uuid);
        self.resent.remove(uuid);
        self.resend_waiting.remove(uuid);
        let mut ops: Vec<Op> =
            self.pushed.remove(uuid).into_iter().flat_map(|p| p.into_keys().map(Op::Unset)).collect();
        ops.sort();
        (ops, self.due_expiry(now))
    }

    /// A player changed worlds; a cancelled change keeps the old world.
    pub fn change_world(&mut self, uuid: &str, world: String, cancelled: bool, now: u64) -> Vec<Plan> {
        let mut plans = self.due_expiry(now);
        if cancelled || plans.iter().any(|p| p.uuid == uuid) {
            return plans;
        }
        let Some(o) = self.online.get_mut(uuid) else { return plans };
        o.world = world;
        if self.engine.perms.uses_context("world") {
            plans.extend(self.plan_player(uuid, now));
        }
        plans
    }

    /// Whether the periodic refresh is due: `provider.refresh-seconds` of the
    /// config in force, so `/pp reload` changes the period at once.
    pub fn refresh_due(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.last_refresh) < u64::from(self.cfg.provider.refresh_seconds) * 1000 {
            return false;
        }
        self.last_refresh = now;
        true
    }

    /// Operator levels and worlds of online players as the server reports
    /// them (`(uuid, level, world)`), and expired nodes: the changes.
    pub fn refresh(&mut self, players: &[(String, u8, String)], now: u64) -> Vec<Plan> {
        let mut changed = Vec::new();
        let world_context = self.engine.perms.uses_context("world");
        for (uuid, level, world) in players {
            if let Some(o) = self.online.get_mut(uuid)
                && (o.op_level != *level || o.world != *world)
            {
                if o.op_level != *level || world_context {
                    changed.push(uuid.clone());
                }
                (o.op_level, o.world) = (*level, world.clone());
            }
        }
        let mut plans = self.due_expiry(now);
        for uuid in changed {
            if !plans.iter().any(|p| p.uuid == uuid) {
                plans.extend(self.plan_player(&uuid, now));
            }
        }
        plans
    }

    /// Nodes past their time: removed and the changes planned for everyone.
    /// Every entry into the plugin calls this, so a stopped periodic task
    /// (`/tick freeze`) does not keep a node alive. A failed save is only
    /// logged: answers skip expired nodes anyway, and the next call saves.
    pub fn due_expiry(&mut self, now: u64) -> Vec<Plan> {
        self.refresh_lock(now);
        let (due, saved) = self.engine.expire(now);
        if let Err(e) = saved {
            self.warn(format!("PumboPerms: cannot save the removal of expired permissions, trying again: {e}"));
        }
        if !due {
            return Vec::new();
        }
        self.after_change(now)
    }

    /// Whether an online player may run an alias Pumpkin leaves unchecked
    /// (`pumbo_common::pumpkin::UNGUARDED_ALIASES`), from memory: in check-event
    /// mode `has-permission` would call back into this plugin. Without a
    /// decision Pumpkin's default for the node counts (its operator level).
    pub fn alias_allowed(&mut self, uuid: &str, node: &str, now: u64) -> bool {
        let Some(o) = self.online.get(uuid).cloned() else { return false };
        let canonical = pumbo_perms_core::node::normalize(node).unwrap_or_default();
        self.engine
            .decide(uuid, &canonical, &o.world, o.op_level, now)
            .unwrap_or(o.op_level >= pumbo_common::pumpkin::alias_op_level(node))
    }

    /// The proxy's PumboPerms took over: everything set so far, per online
    /// player, for PumboBridge to unset.
    pub fn hand_over(&mut self) -> BTreeMap<String, Vec<String>> {
        self.pushed.drain().map(|(uuid, nodes)| (uuid, nodes.into_keys().collect())).collect()
    }

    /// One import pass over the sources that answered (and the attachments of
    /// other plugins, when they are the fallback). Returns whether it was the
    /// first pass, the host changes and whether to resend command lists.
    pub fn import_pass(
        &mut self,
        answers: Vec<(String, Answer)>,
        fallback: Option<(String, Snapshot)>,
        now: u64,
    ) -> (bool, Vec<Plan>, bool) {
        if self.lock.is_none() && self.engine.broken.is_some() {
            // Another instance holds the database: no import and no first pass
            // (it would release PumboBridge's attachments for an empty engine).
            return (false, Vec::new(), self.resend());
        }
        let follow = self.cfg.import.follow;
        let pending = answers.iter().any(|(_, a)| matches!(a, Answer::Pending));
        let mut changed = false;
        let who = Who::console();
        let mut runs: Vec<(String, String, Snapshot)> = Vec::new();
        for (source, answer) in answers {
            match answer {
                Answer::Data(f, s) => runs.push((source, f, s)),
                Answer::Bad(e) => self.warn(format!("PumboPerms: cannot read permissions of {source}: {e}")),
                Answer::Pending | Answer::Absent => {}
            }
        }
        if let Some((f, s)) = fallback {
            runs.push(("attachments".into(), f, s));
        }
        for (source, fingerprint, snap) in runs {
            let ledger = match self.engine.ledger(&source) {
                Ok(l) => l,
                Err(why) => {
                    // An empty ledger would import everything again.
                    self.error(format!("PumboPerms: {source} is not imported automatically: {why}"));
                    continue;
                }
            };
            // Once per source unless `follow` (a ledger means it was imported).
            if ledger.at != 0 && !follow {
                continue;
            }
            let out =
                command::import_source(&mut self.engine, &who, &source, Ok((fingerprint, snap)), SourceMode::Run, now);
            if out.changed {
                changed = true;
                self.logs.push((false, out.reply.plain()));
            }
        }
        if pending && !self.ready {
            self.waiting_until = Some(now + WAIT_MS);
        } else if !pending {
            self.waiting_until = None;
        }
        let first = !self.ready;
        self.ready = true;
        let plans = if first {
            self.refresh_catalog();
            let plans = self.plan_all(now);
            if !passive() {
                self.forget_old_nodes();
            }
            plans
        } else if changed {
            self.after_change(now)
        } else {
            Vec::new()
        };
        (first, plans, self.resend())
    }

    /// Host changes for every online player.
    pub fn plan_all(&mut self, now: u64) -> Vec<Plan> {
        let uuids: Vec<String> = self.online.keys().cloned().collect();
        uuids.into_iter().filter_map(|u| self.plan_player(&u, now)).collect()
    }

    /// Before the periodic pass asks PumboBridge: whether it is time.
    pub fn pass_due(&self, now: u64) -> bool {
        now >= self.next_pass
    }

    /// After PumboBridge answered: whether the pass stops here. Before the
    /// first pass, a bridge that connects to a proxy but has no live export
    /// yet holds everything: no import, no attachments, no release of the
    /// bridge, so Pumpkin's defaults apply instead of an old local database.
    /// The bridge is asked again at every run for [`HOLD_MS`], then every
    /// [`PASS_MS`] with one warning.
    pub fn hold(&mut self, bridge: &Bridge, now: u64) -> bool {
        let waiting = !self.ready && matches!(bridge, Bridge::Mode(m) if m.connected && !m.live);
        if !waiting {
            self.holding = None;
            self.next_pass = now.saturating_add(PASS_MS);
            return false;
        }
        let (since, warned) = *self.holding.get_or_insert((now, false));
        if now.saturating_sub(since) < HOLD_MS {
            self.next_pass = now;
        } else {
            if !warned {
                self.warn("PumboPerms: PumboBridge has no permission table from the proxy yet; waiting for it (Pumpkin's defaults apply until then)".into());
                self.holding = Some((since, true));
            }
            self.next_pass = now.saturating_add(PASS_MS);
        }
        true
    }
}

/// `pumboperms:*` nodes of every `/pp` action, in host form.
fn own_host_nodes() -> Vec<String> {
    pumbo_perms_core::command::own_nodes().iter().map(|n| pumbo_perms_core::node::host_form(n)).collect()
}

thread_local! {
    static RT: RefCell<Option<Rt>> = const { RefCell::new(None) };
    /// The proxy's PumboPerms rules the network (PumboPerms spec §16): this
    /// plugin writes no attachments and PumboBridge does. Outside the state, so
    /// that `hello` from the bridge never finds it busy.
    static PASSIVE: Cell<bool> = const { Cell::new(false) };
}

pub fn passive() -> bool {
    PASSIVE.with(Cell::get)
}

/// Sets the mode; true when it changed.
pub fn set_passive(on: bool) -> bool {
    PASSIVE.with(|p| p.replace(on)) != on
}

pub fn install(rt: Rt) {
    RT.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            *g = Some(rt);
        }
    });
}

/// Drops the state (closes the database).
pub fn uninstall() {
    RT.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            *g = None;
        }
    });
}

/// Runs `f` on the state; `None` when it is not loaded or busy (a nested call).
pub fn with<R>(f: impl FnOnce(&mut Rt) -> R) -> Option<R> {
    RT.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// Who sent a command, as Pumpkin tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderKind {
    Console,
    Rcon,
    Player,
    CommandBlock,
    /// Signs, functions and other senders without an entity.
    Dummy,
}

/// The sender of `/pp`: `player` is (name, UUID, operator level, world).
/// Command blocks, signs and other senders get `None` and no rights at all:
/// anyone who can place one would otherwise act as the console.
pub fn who_for(kind: SenderKind, player: Option<(&str, &str, u8, &str)>) -> Option<Who> {
    match (kind, player) {
        (SenderKind::Console, _) => Some(Who::console()),
        (SenderKind::Rcon, _) => Some(Who { name: "RCON".into(), ..Who::console() }),
        (SenderKind::Player, Some((name, uuid, op_level, world))) => Some(Who::player(name, uuid, op_level, world)),
        _ => None,
    }
}

/// Answers another plugin's message (`pumbo_perms_core::ipc`) from this
/// database; `engine` is `None` when the state is busy.
pub fn ipc_reply(message: &[u8], engine: Option<&mut Engine>, now: u64) -> Result<Vec<u8>, String> {
    let engine = engine.ok_or_else(|| "PumboPerms is busy".to_string())?;
    Ok(pumbo_perms_core::ipc::handle(engine, message, now, env!("CARGO_PKG_VERSION")))
}

/// [`ipc_reply`] while the proxy's PumboPerms rules: the same answers from
/// what the proxy decided. `check` reads the set the proxy sent ([`proxy_decision`]:
/// `None` when no set arrived for the player, `Some(None)` for a node outside
/// it), `info` and `fill` use what PumboBridge carries from the proxy
/// (`info`: the proxy's `ipc::Info` as JSON). Without them: `proxy-rules`.
pub fn passive_reply(
    message: &[u8],
    has: impl FnOnce(&str, &str) -> Option<Option<bool>>,
    info: impl FnOnce(&str) -> Option<String>,
) -> Vec<u8> {
    use pumbo_perms_core::ipc::{self, Request};
    let refused = serde_json::json!({ "ok": false, "error": "proxy-rules", "passive": true });
    let from_proxy = |uuid: &str| -> Option<ipc::Info> {
        let uuid = pumbo_common::id::Uuid::parse(uuid)?.to_string();
        serde_json::from_str(&info(&uuid)?).ok()
    };
    let reply = match ipc::parse(message) {
        Err(reply) => reply,
        Ok(Request::Hello) => {
            let mut hello = ipc::hello(env!("CARGO_PKG_VERSION"));
            if let Some(o) = hello.as_object_mut() {
                o.insert("passive".into(), true.into());
            }
            hello
        }
        Ok(Request::Check { uuid, node, .. }) => {
            let host = pumbo_perms_core::node::normalize(&node).map(|n| pumbo_perms_core::node::host_form(&n));
            match (pumbo_common::id::Uuid::parse(&uuid), host) {
                (Some(u), Ok(host)) => match has(&u.to_string(), &host) {
                    Some(value) => serde_json::json!({ "ok": true, "value": value }),
                    None => refused,
                },
                (None, _) => serde_json::json!({ "ok": false, "error": "bad uuid" }),
                (_, Err(e)) => serde_json::json!({ "ok": false, "error": e.to_string() }),
            }
        }
        Ok(Request::Info { uuid, .. }) => match from_proxy(&uuid).and_then(|i| serde_json::to_value(i).ok()) {
            Some(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("ok".into(), true.into());
                }
                v
            }
            None => refused,
        },
        Ok(Request::Fill { uuid, text, .. }) => match from_proxy(&uuid) {
            Some(i) => serde_json::json!({ "ok": true, "text": ipc::fill(&i, &text) }),
            None => refused,
        },
    };
    reply.to_string().into_bytes()
}

/// The proxy's decision for `node` (Pumpkin's spelling) from PumboBridge's
/// `player-info` answer. Not `has_permission`: a registry default is no
/// decision, and it would fire the permission check event from an ipc call.
pub fn proxy_decision(player_info: &[u8], node: &str) -> Option<Option<bool>> {
    let v: serde_json::Value = serde_json::from_slice(player_info).ok()?;
    Some(v.get("nodes")?.as_object()?.get(node).and_then(serde_json::Value::as_bool))
}

/// How long a "pending" source is asked again after the start.
const WAIT_MS: u64 = 120_000;
/// The periodic import pass runs this often.
pub const PASS_MS: u64 = 10_000;
/// How long the first pass waits for PumboBridge's live export, asking at
/// every run, before it warns and asks every [`PASS_MS`].
pub const HOLD_MS: u64 = 10_000;

/// One source's answer to `{"op":"export-permissions"}`.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Data(String, Snapshot),
    Pending,
    /// No such plugin (or it does not export).
    Absent,
    Bad(String),
}

/// Reads an answer to `{"op":"export-permissions"}` in one pass; also the
/// `proxy-rules` flag, which a newer export version than this one
/// understands still carries.
pub fn export_answer(reply: Option<&[u8]>) -> (bool, Answer) {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Reply {
        ok: bool,
        pending: bool,
        fingerprint: String,
        data: Option<Snapshot>,
    }
    let Some(bytes) = reply else { return (false, Answer::Absent) };
    match serde_json::from_slice::<Reply>(bytes) {
        Err(e) => (false, Answer::Bad(export::not_an_export(&e))),
        Ok(r) if !r.ok => (false, if r.pending { Answer::Pending } else { Answer::Absent }),
        Ok(Reply { data: None, .. }) => (false, Answer::Bad("no data".into())),
        Ok(Reply { fingerprint, data: Some(snap), .. }) => {
            let flag = snap.proxy_rules;
            (flag, export::check(snap).map_or_else(Answer::Bad, |s| Answer::Data(fingerprint, s)))
        }
    }
}

/// PumboBridge's answer to `{"op":"perms-mode"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BridgeMode {
    pub proxy_rules: bool,
    pub fingerprint: String,
    /// `bridge` or `local` (a local PumboPerms writes the attachments).
    pub mode: String,
    /// The export came from the proxy in the bridge's current session.
    pub live: bool,
    /// The bridge connects to a proxy.
    pub connected: bool,
}

/// PumboBridge as the periodic pass sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Bridge {
    Absent,
    /// A PumboBridge without `perms-mode`: the flag and the answer of
    /// `export-permissions`, as before.
    Old(bool, Answer),
    Mode(BridgeMode),
}

/// Reads the answer to `{"op":"perms-mode"}`; `None` when it is not one (an
/// older PumboBridge answers "unknown op").
pub fn perms_mode(reply: &[u8]) -> Option<BridgeMode> {
    let v: serde_json::Value = serde_json::from_slice(reply).ok()?;
    if v.get("ok")?.as_bool() != Some(true) {
        return None;
    }
    serde_json::from_value(v).ok()
}

impl Bridge {
    /// Whether the proxy's PumboPerms rules, when the answer can tell: only
    /// an export from the bridge's current session counts, never one from
    /// disk or from before a reconnect.
    pub fn proxy_rules(&self) -> Option<bool> {
        match self {
            Bridge::Mode(m) if m.live => Some(m.proxy_rules),
            Bridge::Old(flag, Answer::Data(..)) => Some(*flag),
            _ => None,
        }
    }

    /// The answer of the `proxy` source without asking for the export, or
    /// `None` when it has to be asked: a live export whose fingerprint is not
    /// the one imported last (`ledger_fingerprint`).
    pub fn proxy_source(&self, ledger_fingerprint: &str) -> Option<Answer> {
        match self {
            Bridge::Absent => Some(Answer::Absent),
            Bridge::Old(_, a) => Some(a.clone()),
            Bridge::Mode(m) if !m.live => Some(if m.connected { Answer::Pending } else { Answer::Absent }),
            Bridge::Mode(m) if m.fingerprint == ledger_fingerprint => Some(Answer::Absent),
            Bridge::Mode(_) => None,
        }
    }
}

/// What the periodic import pass does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing this time.
    Wait,
    /// The proxy's table did not come in time: stop waiting and say so.
    GiveUp,
    /// Ask the sources and import what they send.
    Ask,
}

/// The decision before asking: only the first pass, a source still pending
/// after the start, or every pass with `import.follow`; nothing while the
/// proxy's PumboPerms rules.
pub fn decide_import(passive: bool, ready: bool, follow: bool, waiting_until: Option<u64>, now: u64) -> Step {
    if passive {
        return Step::Wait;
    }
    if ready && !follow {
        return match waiting_until {
            Some(t) if now < t => Step::Ask,
            Some(_) => Step::GiveUp,
            None => Step::Wait,
        };
    }
    Step::Ask
}

#[cfg(test)]
mod tests {
    use pumbo_common::lang::Lang;
    use pumbo_perms_core::engine::Engine;

    use super::*;

    const U: &str = "00000000-0000-0000-0000-000000000001";

    fn rt() -> Rt {
        let cfg = Config::default();
        let engine = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
        let mut rt = Rt::new(String::new(), cfg, engine, false);
        rt.engine.perms.create_group("vip").unwrap();
        rt
    }

    fn steve() -> Online {
        Online { name: "Steve".into(), world: "world".into(), op_level: 0 }
    }

    /// The changes for `uuid` in `plans`, applied as the plugin does.
    fn ops(rt: &mut Rt, plans: Vec<Plan>, uuid: &str) -> Vec<Op> {
        let mine = plans.iter().filter(|p| p.uuid == uuid).flat_map(|p| p.ops.clone()).collect();
        rt.applied(plans);
        mine
    }

    fn join(rt: &mut Rt, uuid: &str, now: u64) -> Vec<Op> {
        let plans = rt.join(uuid, steve(), now);
        ops(rt, plans, uuid)
    }

    fn give(rt: &mut Rt, group: &str, node: &str, expiry: Option<u64>) {
        if rt.engine.perms.group(group).is_none() {
            rt.engine.perms.create_group(group).unwrap();
        }
        let (_, d) = rt
            .engine
            .perms
            .edit_group(group, |g| {
                g.data.set_permission(pumbo_perms_core::model::PermNode {
                    node: node.into(),
                    value: true,
                    context: pumbo_perms_core::context::Contexts::global(),
                    expiry: None,
                })
            })
            .unwrap();
        rt.engine.commit(&d, None).unwrap();
        let parent = pumbo_perms_core::model::ParentNode {
            group: group.into(),
            context: pumbo_perms_core::context::Contexts::global(),
            expiry,
        };
        let (_, d) = rt.engine.perms.edit_user(U, "Steve", |d| d.add_parent(parent));
        rt.engine.commit(&d, None).unwrap();
        rt.refresh_catalog();
    }

    fn give_vip(rt: &mut Rt) {
        let parent = pumbo_perms_core::model::ParentNode {
            group: "vip".into(),
            context: pumbo_perms_core::context::Contexts::global(),
            expiry: None,
        };
        rt.engine.perms.edit_user(U, "Steve", |d| d.add_parent(parent));
        rt.refresh_catalog();
    }

    #[test]
    fn passive_or_not_ready_plans_nothing() {
        let mut rt = rt();
        assert!(join(&mut rt, U, 0).is_empty(), "before the first import pass");
        rt.ready = true;
        set_passive(true);
        give_vip(&mut rt);
        assert!(rt.plan_all(0).is_empty(), "the proxy's PumboPerms rules");
        assert!(rt.change_world(U, "nether".into(), false, 0).is_empty());
        assert_eq!(decide_import(true, false, true, None, 0), Step::Wait);
        set_passive(false);
        assert!(!rt.plan_all(0).is_empty());
    }

    #[test]
    fn takeover_unsets_whole_catalog() {
        let mut rt = rt();
        rt.ready = true;
        join(&mut rt, U, 0);
        let handed = rt.hand_over();
        assert!(handed.contains_key(U) && rt.pushed.is_empty());
        // The proxy's PumboPerms is gone: the next pass is a first one again.
        rt.ready = false;
        let (first, plans, _) = rt.import_pass(Vec::new(), None, 0);
        assert!(first);
        let ops = ops(&mut rt, plans, U);
        let wanted = rt.pushed.get(U).unwrap();
        for node in rt.catalog.nodes().iter().filter(|n| !wanted.contains_key(*n)) {
            assert!(ops.contains(&Op::Unset(node.clone())), "{node} not unset");
        }
    }

    #[test]
    fn plans_only_differences() {
        let mut rt = rt();
        rt.ready = true;
        give_vip(&mut rt);
        let first = join(&mut rt, U, 0);
        assert!(first.contains(&Op::Set("group.vip".into(), true)), "{first:?}");
        assert!(rt.plan_all(0).is_empty(), "nothing changed");
    }

    #[test]
    fn leave_unsets_and_forgets_pushed() {
        let mut rt = rt();
        rt.ready = true;
        give(&mut rt, "admin", "minecraft:command.op", None);
        assert!(join(&mut rt, U, 0).contains(&Op::Set("minecraft:command.op".into(), true)));
        // The change comes while the player is leaving: its plan finds nobody to apply to.
        let (_, d) = rt
            .engine
            .perms
            .edit_user(U, "Steve", |d| d.remove_parent("admin", &pumbo_perms_core::context::Contexts::global(), false));
        rt.engine.commit(&d, None).unwrap();
        let late = rt.plan_all(0);
        let (gone, _) = rt.leave(U, 0);
        assert!(gone.contains(&Op::Unset("minecraft:command.op".into())), "{gone:?}");
        rt.applied(late);
        assert!(!rt.pushed.contains_key(U), "{:?}", rt.pushed);
        // Back again: the whole catalog is checked, the node goes even if the unset at leave failed.
        let back = join(&mut rt, U, 0);
        assert!(back.contains(&Op::Unset("minecraft:command.op".into())), "{back:?}");
    }

    #[test]
    fn unload_clears_offline_attachments() {
        let mut rt = rt();
        rt.ready = true;
        give(&mut rt, "admin", "minecraft:command.op", None);
        join(&mut rt, U, 0);
        rt.leave(U, 0);
        // Demoted while offline, then the plugin is unloaded and loaded again.
        let (_, d) = rt
            .engine
            .perms
            .edit_user(U, "Steve", |d| d.remove_parent("admin", &pumbo_perms_core::context::Contexts::global(), false));
        rt.engine.commit(&d, None).unwrap();
        let Rt { dir, cfg, engine, .. } = rt;
        let mut rt = Rt::new(dir, cfg, engine, false);
        rt.ready = true;
        let back = join(&mut rt, U, 0);
        assert!(back.contains(&Op::Unset("minecraft:command.op".into())), "{back:?}");
        assert!(!back.contains(&Op::Set("minecraft:command.op".into(), true)));
    }

    #[test]
    fn load_reconciles_online_attachments() {
        let earlier = rt();
        // A node an earlier run set, gone from the data since.
        earlier.engine.store().unwrap().add_host_nodes(vec![&"essentials:fly".to_string()]).unwrap();
        let mut rt = Rt::new(String::new(), earlier.cfg.clone(), earlier.engine, false);
        join(&mut rt, U, 0);
        let (first, plans, _) = rt.import_pass(Vec::new(), None, 0);
        assert!(first);
        let ops = ops(&mut rt, plans, U);
        assert!(ops.contains(&Op::Unset("essentials:fly".into())), "{ops:?}");
        // Taken off everyone online: forgotten, so the list does not only grow.
        assert!(!rt.catalog.nodes().contains("essentials:fly"));
        assert!(!rt.engine.store().unwrap().host_nodes().unwrap().contains("essentials:fly"));
    }

    #[test]
    fn cancelled_change_world_keeps_world() {
        let mut rt = rt();
        rt.ready = true;
        let node = pumbo_perms_core::model::PermNode {
            node: "minecraft:command.fly".into(),
            value: true,
            context: pumbo_perms_core::context::Contexts::parse(&["world=nether".into()]).unwrap(),
            expiry: None,
        };
        rt.engine.perms.edit_user(U, "Steve", |d| d.set_permission(node));
        rt.refresh_catalog();
        join(&mut rt, U, 0);
        assert!(rt.change_world(U, "nether".into(), true, 0).is_empty());
        assert_eq!(rt.online[U].world, "world");
        let plans = rt.change_world(U, "nether".into(), false, 0);
        assert_eq!(rt.online[U].world, "nether");
        assert!(ops(&mut rt, plans, U).contains(&Op::Set("minecraft:command.fly".into(), true)));
        // A world change the event did not report shows at the next refresh.
        let plans = rt.refresh(&[(U.to_string(), 0, "world".into())], 0);
        assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.fly".into())));
    }

    #[test]
    fn expiry_checked_on_entry() {
        let mut rt = rt();
        rt.ready = true;
        give(&mut rt, "vip", "minecraft:command.gamemode", Some(10_000));
        assert!(join(&mut rt, U, 0).contains(&Op::Set("minecraft:command.gamemode".into(), true)));
        assert!(rt.due_expiry(9_999).is_empty());
        // Another player joins after the time: the expired node goes for everyone.
        let other = "00000000-0000-0000-0000-000000000002";
        let plans = rt.join(other, steve(), 12_000);
        assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.gamemode".into())));
        assert!(rt.engine.perms.next_expiry().is_none(), "saved");
    }

    #[test]
    fn expiry_due_plans_when_commit_fails() {
        let backend = pumbo_common::store::FaultyBackend::default();
        let writes = backend.fail_writes.clone();
        let store = pumbo_perms_core::store::PermsStore::new(pumbo_common::store::Store::new(backend)).unwrap();
        let cfg = Config::default();
        let engine = Engine::new(Ok(store), cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
        let mut rt = Rt::new(String::new(), cfg, engine, false);
        rt.ready = true;
        give(&mut rt, "vip", "minecraft:command.gamemode", Some(10_000));
        join(&mut rt, U, 0);
        writes.store(u32::MAX, std::sync::atomic::Ordering::SeqCst);
        let plans = rt.due_expiry(12_000);
        assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.gamemode".into())));
        assert!(rt.logs.iter().any(|(_, l)| l.contains("trying again")), "{:?}", rt.logs);
        writes.store(0, std::sync::atomic::Ordering::SeqCst);
        assert!(rt.due_expiry(13_000).is_empty(), "nothing new to send");
        assert!(rt.engine.perms.next_expiry().is_none(), "saved at the next call");
    }

    #[test]
    fn entry_refreshes_lock_while_ticks_frozen() {
        let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-lock-{}.redb", std::process::id()));
        let lock = DbLock::acquire(&path, 0).unwrap();
        let cfg = Config::default();
        let engine = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
        let mut rt = Rt::with_lock(String::new(), cfg, engine, false, Some(lock), 0);
        // No periodic task runs; a player's command 25 s later is an entry.
        assert!(rt.due_expiry(25_000).is_empty());
        assert!(DbLock::acquire(&path, 54_999).is_err(), "a second copy is refused while players use the plugin");
        // Another instance took the stale lock over: this one stops writing.
        let other = DbLock::acquire(&path, 90_000).unwrap();
        rt.due_expiry(100_000);
        assert!(rt.lock.is_none() && rt.engine.broken.is_some() && !rt.engine.writable());
        assert!(rt.errors.iter().any(|l| l.contains("taken over")), "{:?}", rt.errors);
        // Nothing reaches the file the other instance has open, a join neither.
        assert!(rt.engine.store().is_none());
        rt.join(U, steve(), 100_001);
        assert!(rt.engine.store().is_none());
        drop(other);
    }

    #[test]
    fn database_held_elsewhere_waits_then_plans_for_online_players() {
        let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-late-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
        // Steve in `group`, as the database on disk says.
        let opened = |group: &str, now: u64| -> Result<(DbLock, Engine), String> {
            let lock = DbLock::acquire(&path, now).map_err(|e| e.to_string())?;
            let cfg = Config::default();
            let mut e = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
            let mut d = e.perms.create_group(group).unwrap();
            let parent = pumbo_perms_core::model::ParentNode {
                group: group.into(),
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            d.merge(e.perms.edit_user(U, "Steve", |d| d.add_parent(parent)).1);
            e.commit(&d, None).unwrap();
            Ok((lock, e))
        };
        // A killed server left its lock; the restart comes 1 s later.
        std::mem::forget(DbLock::acquire(&path, 0).unwrap());
        let cfg = Config::default();
        let lang = Lang::load(&crate::BUNDLES, "en", None).0;
        let engine = Engine::new(Err(pumbo_common::store::StoreError::new("held")), cfg.core(), lang);
        let mut rt = Rt::with_lock(String::new(), cfg, engine, false, None, 1_000);
        assert!(join(&mut rt, U, 1_000).is_empty());
        let (first, plans, _) = rt.import_pass(Vec::new(), None, 1_000);
        assert!(!first && plans.is_empty() && !rt.ready, "no first pass on an empty engine");
        assert!(rt.retry_database(11_000, |_| opened("vip", 11_000)).is_empty());
        assert!(!rt.import_pass(Vec::new(), None, 11_000).0 && !rt.ready && rt.lock.is_none());
        // The lock went stale: the database opens and the first pass sets it all.
        assert!(rt.retry_database(31_000, |_| opened("vip", 31_000)).is_empty(), "not ready yet");
        assert!(rt.lock.is_some() && rt.engine.broken.is_none());
        let (first, plans, _) = rt.import_pass(Vec::new(), None, 31_000);
        assert!(first && ops(&mut rt, plans, U).contains(&Op::Set("group.vip".into(), true)));
        // Another instance takes it over and changes Steve; when it lets go, the
        // database opens here again and the players get what it holds now.
        let other = DbLock::acquire(&path, 61_000).unwrap();
        rt.refresh_lock(61_000 + 1);
        assert!(rt.lock.is_none() && rt.engine.store().is_none());
        drop(other);
        let now = 61_000 + LOCK_RETRY_MS + 1;
        let plans = rt.retry_database(now, |_| opened("mvp", now));
        assert!(ops(&mut rt, plans, U).contains(&Op::Set("group.mvp".into(), true)));
    }

    #[test]
    fn failed_open_under_lock_is_tried_again() {
        let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-reopen-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
        let cfg = Config::default();
        let lang = Lang::load(&crate::BUNDLES, "en", None).0;
        let engine = Engine::new(Err(pumbo_common::store::StoreError::new("cannot open")), cfg.core(), lang);
        let lock = DbLock::acquire(&path, 0).unwrap();
        let mut rt = Rt::with_lock(String::new(), cfg, engine, false, Some(lock), 0);
        let reopen = |now: u64| -> Result<(DbLock, Engine), String> {
            let lock = DbLock::acquire(&path, now).map_err(|e| e.to_string())?;
            Ok((lock, Engine::in_memory(Config::default().core(), Lang::load(&crate::BUNDLES, "en", None).0)))
        };
        rt.retry_database(READ_RETRY_MS, |_| reopen(READ_RETRY_MS));
        assert!(rt.engine.writable() && rt.lock.is_some(), "{:?}", rt.engine.broken);
    }

    #[test]
    fn pumpkin_import_reply_is_truthful() {
        let mut rt = rt();
        let snap = pumbo_perms_core::export::parse(
            r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
                "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
        )
        .unwrap();
        rt.import_pass(vec![("proxy".into(), Answer::Data("f1".into(), snap))], None, 0);
        let said = rt.logs.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
        assert!(
            said.contains("known only by name and get nothing here; give them their ranks with /pp: Mod1"),
            "{said}"
        );
        assert!(!said.contains("premium"), "{said}");
        // Mod1 joins: nothing, and the log says why.
        rt.logs.clear();
        let plans = rt.join(U, Online { name: "Mod1".into(), ..steve() }, 1);
        rt.applied(plans);
        assert!(rt.engine.perms.user(U).is_none_or(|u| u.data.parents.is_empty()));
        assert!(rt.logs.iter().any(|(warn, l)| *warn && l.contains("Mod1 joined and got nothing")), "{:?}", rt.logs);
    }

    #[test]
    fn non_player_senders_get_no_console_rights() {
        assert_eq!(who_for(SenderKind::CommandBlock, None), None);
        assert_eq!(who_for(SenderKind::Dummy, None), None);
        assert_eq!(who_for(SenderKind::Player, None), None);
        assert!(who_for(SenderKind::Console, None).is_some_and(|w| w.is_console()));
        assert!(who_for(SenderKind::Rcon, None).is_some_and(|w| w.is_console()));
        let p = who_for(SenderKind::Player, Some(("Steve", U, 2, "world"))).unwrap();
        assert_eq!((p.uuid.as_deref(), p.op_level), (Some(U), 2));
        assert!(!p.is_console());
    }

    #[test]
    fn alias_guard_decides_from_memory() {
        let mut rt = rt();
        let _ = rt.join(U, Online { op_level: 2, ..steve() }, 0);
        assert!(rt.alias_allowed(U, "minecraft:command.teleport", 0), "Pumpkin's Op(2)");
        assert!(!rt.alias_allowed(U, "minecraft:command.banip", 0), "banip is Op(3)");
        rt.engine.perms.edit_user(U, "Steve", |d| {
            d.set_permission(pumbo_perms_core::model::PermNode {
                node: "minecraft:command.teleport".into(),
                value: false,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            })
        });
        assert!(!rt.alias_allowed(U, "minecraft:command.teleport", 0), "the data decides first");
        assert!(!rt.alias_allowed("00000000-0000-0000-0000-000000000009", "minecraft:command.teleport", 0));
    }

    #[test]
    fn passive_ipc_refuses_reads() {
        let mut rt = rt();
        give_vip(&mut rt);
        let check = format!(r#"{{"op":"check","uuid":"{U}","node":"group.vip"}}"#);
        let info = format!(r#"{{"op":"info","uuid":"{U}"}}"#);
        let fill = format!(r#"{{"op":"fill","uuid":"{U}","text":"%pumboperms_group%"}}"#);
        let local: serde_json::Value =
            serde_json::from_slice(&ipc_reply(check.as_bytes(), Some(&mut rt.engine), 0).unwrap()).unwrap();
        assert_eq!(local["value"], true);
        // Nothing from the proxy for this player (offline, or not arrived yet).
        for msg in [&check, &info, &fill] {
            let v: serde_json::Value =
                serde_json::from_slice(&passive_reply(msg.as_bytes(), |_, _| None, |_| None)).unwrap();
            assert_eq!(
                (v["ok"].clone(), v["error"].clone(), v["passive"].clone()),
                (false.into(), "proxy-rules".into(), true.into()),
                "{msg}"
            );
        }
        let hello: serde_json::Value =
            serde_json::from_slice(&passive_reply(br#"{"op":"hello"}"#, |_, _| None, |_| None)).unwrap();
        assert_eq!((hello["ok"].clone(), hello["passive"].clone()), (true.into(), true.into()));
        assert_eq!(hello["ops"], serde_json::json!(pumbo_perms_core::ipc::OPS));
        let other: serde_json::Value =
            serde_json::from_slice(&passive_reply(br#"{"op":"nope"}"#, |_, _| None, |_| None)).unwrap();
        assert_eq!(other["error"], "unsupported op");
        assert!(ipc_reply(check.as_bytes(), None, 0).is_err());
    }

    #[test]
    fn passive_ipc_answers_from_proxy_data() {
        let info = pumbo_perms_core::ipc::Info {
            name: "Steve".into(),
            rank: "vip".into(),
            rank_display: "VIP".into(),
            prefix: "[VIP] ".into(),
            suffix: String::new(),
            groups: vec!["vip".into(), "default".into()],
            meta: Default::default(),
        };
        let data = serde_json::to_string(&info).unwrap();
        let bridge = serde_json::json!({
            "ok": true, "data": data, "nodes": {"pumbobans:ban": true, "minecraft:command.kick": false},
        })
        .to_string();
        let ask = |msg: String| -> serde_json::Value {
            let reply = passive_reply(
                msg.as_bytes(),
                |uuid, node| {
                    assert_eq!(uuid, U);
                    proxy_decision(bridge.as_bytes(), node)
                },
                |uuid| (uuid == U).then(|| data.clone()),
            );
            serde_json::from_slice(&reply).unwrap()
        };
        let v = ask(format!(r#"{{"op":"check","uuid":"{U}","node":"pumbo.bans.ban"}}"#));
        assert_eq!((v["ok"].clone(), v["value"].clone()), (true.into(), true.into()), "read in Pumpkin's spelling");
        assert!(ask(format!(r#"{{"op":"check","uuid":"{U}","node":"essentials.fly"}}"#))["value"].is_null());
        assert_eq!(ask(format!(r#"{{"op":"check","uuid":"{U}","node":"minecraft:command.kick"}}"#))["value"], false);
        // Registered in Pumpkin with an operator default, but not in the set: no decision.
        let v = ask(format!(r#"{{"op":"check","uuid":"{U}","node":"minecraft:command.gamemode"}}"#));
        assert_eq!((v["ok"].clone(), v["value"].clone()), (true.into(), serde_json::Value::Null));
        assert_eq!(proxy_decision(br#"{"ok":true,"data":null,"nodes":null}"#, "a:b"), None, "no set yet");
        assert_eq!(proxy_decision(br#"{"ok":true,"data":null}"#, "a:b"), None, "an older bridge");
        let v = ask(format!(r#"{{"op":"info","uuid":"{}"}}"#, U.replace('-', "")));
        assert_eq!(
            (v["ok"].clone(), v["rank_display"].clone(), v["prefix"].clone()),
            (true.into(), "VIP".into(), "[VIP] ".into())
        );
        let v = ask(format!(r#"{{"op":"fill","uuid":"{U}","text":"%pumboperms_prefix%Steve"}}"#));
        assert_eq!(v["text"], "[VIP] Steve");
        let other = "00000000-0000-0000-0000-000000000002";
        assert_eq!(ask(format!(r#"{{"op":"info","uuid":"{other}"}}"#))["error"], "proxy-rules");
    }

    fn mode(proxy_rules: bool, fingerprint: &str, live: bool, connected: bool) -> Bridge {
        Bridge::Mode(BridgeMode {
            proxy_rules,
            fingerprint: fingerprint.into(),
            mode: "bridge".into(),
            live,
            connected,
        })
    }

    #[test]
    fn passive_flag_survives_newer_export_version() {
        let reply = br#"{"ok":true,"fingerprint":"f","data":{"format":"pumboperms","version":99,"proxy-rules":true}}"#;
        let (flag, answer) = export_answer(Some(reply));
        assert!(flag, "the flag is read even when the export is too new to import");
        assert!(matches!(answer, Answer::Bad(ref e) if e.contains("newer")), "{answer:?}");
        let m = perms_mode(br#"{"ok":true,"proxy-rules":true,"fingerprint":"f","mode":"bridge","live":true,"connected":true,"saved":1,"pending":false}"#).unwrap();
        assert_eq!(Bridge::Mode(m).proxy_rules(), Some(true));
    }

    #[test]
    fn perms_mode_falls_back_to_export_on_old_bridge() {
        assert_eq!(perms_mode(br#"{"ok":false,"error":"unknown op perms-mode"}"#), None);
        let reply = br#"{"ok":true,"fingerprint":"f","data":{"format":"pumboperms","version":1,"proxy-rules":true}}"#;
        let (flag, answer) = export_answer(Some(reply));
        let old = Bridge::Old(flag, answer);
        assert_eq!(old.proxy_rules(), Some(true), "an older bridge decides the mode as before");
        assert!(matches!(old.proxy_source(""), Some(Answer::Data(..))));
        assert_eq!(export_answer(Some(br#"{"ok":false,"pending":true}"#)).1, Answer::Pending);
        assert_eq!(export_answer(None).1, Answer::Absent);
    }

    #[test]
    fn tick_skips_export_when_fingerprint_same() {
        let live = mode(false, "abc", true, true);
        assert_eq!(live.proxy_source("abc"), Some(Answer::Absent), "imported already: no export asked");
        assert_eq!(live.proxy_source("old"), None, "a new table: ask for it");
        assert_eq!(Bridge::Absent.proxy_source("abc"), Some(Answer::Absent));
    }

    #[test]
    fn stale_export_is_not_imported() {
        // From disk, the proxy not connected: neither the mode nor the data.
        let stale = mode(true, "abc", false, false);
        assert_eq!(stale.proxy_rules(), None);
        assert_eq!(stale.proxy_source(""), Some(Answer::Absent));
        // Connected, before the proxy sent this session's export: wait.
        assert_eq!(mode(true, "abc", false, true).proxy_source(""), Some(Answer::Pending));
        let mut rt = rt();
        assert!(!rt.hold(&stale, 0), "a server without a proxy goes on as a single server");
    }

    #[test]
    fn connected_bridge_waits_for_live_answer() {
        let mut rt = rt();
        join(&mut rt, U, 0);
        let waiting = mode(true, "abc", false, true);
        assert!(rt.pass_due(0) && rt.hold(&waiting, 0));
        assert!(rt.pass_due(1_000), "asked again at the next run");
        assert!(rt.hold(&waiting, 9_000) && !rt.ready);
        assert!(rt.logs.is_empty());
        assert!(rt.hold(&waiting, HOLD_MS));
        assert!(rt.logs.iter().any(|(warn, l)| *warn && l.contains("waiting")), "{:?}", rt.logs);
        assert!(!rt.pass_due(HOLD_MS + 1_000), "then every pass only");
        rt.hold(&waiting, HOLD_MS + PASS_MS);
        assert_eq!(rt.logs.len(), 1, "warned once");
        assert!(rt.plan_all(HOLD_MS).is_empty(), "no attachments while holding");
        let live = mode(true, "abc", true, true);
        assert!(!rt.hold(&live, HOLD_MS + PASS_MS + 1));
        assert_eq!(rt.holding, None);
    }

    #[test]
    fn dropped_wildcard_is_logged_once_per_change() {
        let mut rt = rt();
        rt.ready = true;
        rt.engine
            .perms
            .edit_group("vip", |g| {
                for (node, value) in [("pumbo.bans.*", true), ("pumbo.bans.exempt.*", false)] {
                    g.data.set_permission(pumbo_perms_core::model::PermNode {
                        node: node.into(),
                        value,
                        context: pumbo_perms_core::context::Contexts::global(),
                        expiry: None,
                    });
                }
            })
            .unwrap();
        give_vip(&mut rt);
        let ops = join(&mut rt, U, 0);
        assert!(!ops.iter().any(|op| matches!(op, Op::Set(n, _) if n == "pumbobans:*")), "{ops:?}");
        assert!(ops.contains(&Op::Set("pumbobans:ban".into(), true)), "the catalog keeps exact Pumbo nodes");
        let logged =
            |rt: &Rt| rt.logs.iter().filter(|(_, l)| l.contains("pumbobans:* hides pumbobans:exempt.*")).count();
        assert_eq!(logged(&rt), 1, "{:?}", rt.logs);
        rt.plan_all(0);
        assert_eq!(logged(&rt), 1);
        rt.leave(U, 0);
        assert!(rt.hidden.is_empty(), "{:?}", rt.hidden);
        let _ = rt.join("00000000-0000-0000-0000-000000000002", steve(), 0);
        assert!(rt.hidden.is_empty(), "a player with nothing hidden keeps no entry");
    }

    /// 100 players online and 8000 nodes in the catalog (20 groups of 50,
    /// 700 players offline with 10 nodes of their own each).
    fn at_scale() -> (Rt, Vec<String>) {
        use pumbo_perms_core::context::Contexts;
        use pumbo_perms_core::model::{ParentNode, PermNode};
        let mut rt = rt();
        rt.ready = true;
        let perm = |node: String, value: bool| PermNode { node, value, context: Contexts::global(), expiry: None };
        let parent = |g: usize| ParentNode { group: format!("g{}", g % 20), context: Contexts::global(), expiry: None };
        for g in 0..20 {
            rt.engine.perms.create_group(&format!("g{g}")).unwrap();
            rt.engine
                .perms
                .edit_group(&format!("g{g}"), |gr| {
                    gr.data.add_parent(ParentNode {
                        group: "default".into(),
                        context: Contexts::global(),
                        expiry: None,
                    });
                    for n in 0..50 {
                        let node = match n % 3 {
                            0 => format!("minecraft:command.c{g}n{n}"),
                            1 => format!("pumbo.p{g}.a{n}"),
                            _ => format!("plugin{g}:node{n}"),
                        };
                        gr.data.set_permission(perm(node, n % 7 != 0));
                    }
                })
                .unwrap();
        }
        for k in 0..700 {
            rt.engine.perms.edit_user(&format!("00000000-0000-4000-8000-{k:012}"), "", |d| {
                d.add_parent(parent(k));
                for n in 0..10 {
                    d.set_permission(perm(format!("own{k}:node{n}"), true));
                }
            });
        }
        rt.refresh_catalog();
        assert!(rt.catalog.len() > 8000, "{}", rt.catalog.len());
        let online: Vec<String> = (0..100).map(|k| format!("00000000-0000-4000-9000-{k:012}")).collect();
        for (k, uuid) in online.iter().enumerate() {
            rt.engine.perms.edit_user(uuid, &format!("P{k}"), |d| d.add_parent(parent(k)));
            let plans = rt.join(uuid, Online { name: format!("P{k}"), world: "world".into(), op_level: 0 }, 0);
            rt.applied(plans);
        }
        (rt, online)
    }

    /// An edit with 100 players online and 8000 catalog nodes: the plans come
    /// from each player's own nodes, not from the whole catalog.
    #[test]
    #[ignore = "release benchmark"]
    fn plan_after_edit_scale() {
        let rss = || {
            let out =
                std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
            out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()).unwrap_or(0)
        };
        let before = rss();
        let (mut rt, online) = at_scale();
        let mut worst = std::time::Duration::ZERO;
        for round in 0..10u64 {
            let node = pumbo_perms_core::model::PermNode {
                node: "minecraft:command.c0n1".into(),
                value: round % 2 == 0,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            let (_, d) = rt.engine.perms.edit_group("default", |g| g.data.set_permission(node)).unwrap();
            rt.engine.commit(&d, None).unwrap();
            let start = std::time::Instant::now();
            let plans = rt.after_change(round);
            let took = start.elapsed();
            worst = worst.max(took);
            assert_eq!(plans.len(), online.len(), "a change in default reaches everyone");
            rt.applied(plans);
            println!("plan after an edit in default, 100 players, {} catalog nodes: {took:?}", rt.catalog.len());
        }
        // One player's change plans one player.
        let (_, d) = rt.engine.perms.edit_user(&online[3], "P3", |d| {
            d.set_permission(pumbo_perms_core::model::PermNode {
                node: "plugin1:node2".into(),
                value: false,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            })
        });
        rt.engine.commit(&d, None).unwrap();
        let start = std::time::Instant::now();
        let plans = rt.after_change(20);
        println!("plan after an edit of one player: {:?}", start.elapsed());
        assert_eq!(plans.iter().map(|p| p.uuid.as_str()).collect::<Vec<_>>(), [online[3].as_str()]);
        // Native, so 8-byte pointers: the plugin in WASM needs less. Run alone.
        println!("memory: the process grew by {} MB", rss().saturating_sub(before) / 1024);
        assert!(worst < std::time::Duration::from_millis(20), "{worst:?}");
    }

    #[test]
    fn edit_plans_only_players_it_reaches() {
        let mut rt = rt();
        rt.ready = true;
        give(&mut rt, "vip", "minecraft:command.fly", None);
        const OTHER: &str = "00000000-0000-0000-0000-000000000002";
        let plans = rt.join(OTHER, steve(), 0);
        rt.applied(plans);
        join(&mut rt, U, 0);
        rt.engine.take_affected([U, OTHER]);
        let (_, d) = rt
            .engine
            .perms
            .edit_group("vip", |g| {
                g.data.set_permission(pumbo_perms_core::model::PermNode {
                    node: "minecraft:command.fly".into(),
                    value: false,
                    context: pumbo_perms_core::context::Contexts::global(),
                    expiry: None,
                })
            })
            .unwrap();
        rt.engine.commit(&d, None).unwrap();
        let plans = rt.after_change(0);
        assert_eq!(plans.iter().map(|p| p.uuid.as_str()).collect::<Vec<_>>(), [U], "only vip members");
        assert_eq!(ops(&mut rt, plans, U), [Op::Set("minecraft:command.fly".into(), false)]);
    }

    #[test]
    fn command_lists_resent_at_most_once_a_second() {
        let mut rt = rt();
        const OTHER: &str = "00000000-0000-0000-0000-000000000002";
        assert_eq!(rt.resends(vec![U.into(), OTHER.into()], 1_000), [U, OTHER]);
        // Three more changes within the second: one list, when the second is over.
        for at in [1_100, 1_400, 1_900] {
            assert!(rt.resends(vec![U.into()], at).is_empty());
            assert!(rt.due_resends(at).is_empty());
        }
        assert_eq!(rt.due_resends(2_000), [U]);
        assert!(rt.due_resends(2_500).is_empty());
        assert_eq!(rt.resends(vec![OTHER.into()], 2_500), [OTHER]);
        // Leaving drops a waiting one.
        assert!(rt.resends(vec![OTHER.into()], 2_600).is_empty());
        rt.leave(OTHER, 2_600);
        assert!(rt.due_resends(9_000).is_empty());
    }

    #[test]
    fn reload_reads_refresh_seconds() {
        let mut rt = rt();
        assert_eq!(rt.cfg.provider.refresh_seconds, 5);
        assert!(rt.refresh_due(100_000));
        assert!(!rt.refresh_due(104_999));
        assert!(rt.refresh_due(105_000));
        // `/pp reload` replaces the config; the same task follows the new period.
        let (cfg, _) = pumbo_common::config::load::<Config>("provider:\n  refresh-seconds: 1\n");
        rt.cfg = cfg;
        assert!(!rt.refresh_due(105_999));
        assert!(rt.refresh_due(106_000));
        assert!(rt.refresh_due(107_000));
    }

    #[test]
    fn import_asks_until_ready_then_only_while_waiting() {
        assert_eq!(decide_import(false, false, false, None, 0), Step::Ask);
        assert_eq!(decide_import(false, true, false, None, 0), Step::Wait);
        assert_eq!(decide_import(false, true, false, Some(10), 5), Step::Ask);
        assert_eq!(decide_import(false, true, false, Some(10), 10), Step::GiveUp);
        assert_eq!(decide_import(false, true, true, None, 0), Step::Ask);
    }
}
