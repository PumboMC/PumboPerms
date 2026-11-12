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
use std::collections::{BTreeSet, HashMap};

use pumbo_common::lang::Lang;
use pumbo_common::store::DbLock;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::host::{Catalog, Op};

use crate::config::Config;

mod import;
mod ipc;
mod players;

pub use import::{Answer, Bridge, BridgeMode, HOLD_MS, PASS_MS, Step, decide_import, export_answer, perms_mode};
pub use ipc::{SenderKind, ipc_reply, passive_reply, proxy_decision, who_for};

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
                for (uuid, o) in &self.online {
                    // As at a join; a name that is not saved is saved at the next one.
                    let _ = self.engine.seen(uuid, &o.name, false);
                }
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

#[cfg(test)]
mod tests;
