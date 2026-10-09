//! The plugin state, reachable from every handler.
//!
//! A handler takes what it needs from the host first, decides inside [`with`]
//! and calls the host only after the state is released. That matters: a host
//! call can make Pumpkin check a permission, which (in check-event mode) comes
//! back into this plugin while the first handler still runs. Such a nested
//! call finds the state busy ([`with`] returns `None`) instead of waiting for
//! itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use pumbo_common::lang::Lang;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::host::{self, Catalog, Op};

use crate::config::Config;

/// What is known about an online player.
#[derive(Debug, Clone)]
pub struct Online {
    pub name: String,
    pub world: String,
    pub op_level: u8,
}

pub struct Rt {
    pub dir: String,
    pub cfg: Config,
    pub engine: Engine,
    pub catalog: Catalog,
    pub online: HashMap<String, Online>,
    /// What was set on the host per player (kept after leaving: Pumpkin keeps
    /// the attachments until it stops).
    pub pushed: HashMap<String, HashMap<String, bool>>,
    /// Whether the permission check event is answered (fixed at load).
    pub check_event: bool,
    /// Permission checks answered from memory (check-event mode).
    pub answered: u64,
    /// Log lines collected inside [`with`], written after it.
    pub logs: Vec<(bool, String)>,
    /// The first import pass is done and the bridge released the
    /// attachments: from now on this plugin writes them (spec §5.3).
    pub ready: bool,
    /// A source answered "pending": ask again until this time (ms).
    pub waiting_until: Option<u64>,
}

impl Rt {
    pub fn new(dir: String, cfg: Config, engine: Engine, check_event: bool) -> Self {
        let mut rt = Self {
            dir,
            cfg,
            engine,
            catalog: Catalog::new(pumbo_common::pumpkin::COMMAND_NODES),
            online: HashMap::new(),
            pushed: HashMap::new(),
            check_event,
            answered: 0,
            logs: Vec::new(),
            ready: false,
            waiting_until: None,
        };
        rt.refresh_catalog();
        rt
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
    pub fn refresh_catalog(&mut self) {
        let mut known: Vec<String> =
            pumbo_perms_core::command::own_nodes().iter().map(|n| pumbo_perms_core::node::host_form(n)).collect();
        if let Some(store) = self.engine.store() {
            match store.host_nodes() {
                Ok(nodes) => known.extend(nodes),
                Err(e) => self.logs.push((true, format!("PumboPerms: cannot read the node list: {e}"))),
            }
        }
        self.catalog.refresh(&self.engine.perms, known);
        if let Some(store) = self.engine.store() {
            let stored = store.host_nodes().unwrap_or_default();
            let new: Vec<&String> = self.catalog.nodes().iter().filter(|n| !stored.contains(*n)).collect();
            if let Err(e) = store.add_host_nodes(new) {
                self.logs.push((true, format!("PumboPerms: cannot save the node list: {e}")));
            }
        }
    }

    /// The host changes for one online player, remembered as done. Nothing
    /// before the first import pass (the bridge may still own them), and
    /// nothing while the proxy's PumboPerms rules ([`passive`]).
    pub fn plan_player(&mut self, uuid: &str, now: u64) -> Vec<Op> {
        if !self.ready || passive() {
            return Vec::new();
        }
        let Some(o) = self.online.get(uuid).cloned() else { return Vec::new() };
        let mut wanted = host::wanted(&mut self.engine, uuid, &o.world, o.op_level, now, &self.catalog);
        let visible = pumbo_perms_core::command::allowed_any(
            &mut self.engine,
            &pumbo_perms_core::command::Who::player(&o.name, uuid, o.op_level, &o.world),
            now,
        );
        wanted.insert(crate::COMMAND_NODE.to_string(), visible);
        let ops = host::plan(&wanted, self.pushed.get(uuid), &self.catalog);
        self.pushed.insert(uuid.to_string(), wanted);
        ops
    }

    /// Host changes for every online player.
    pub fn plan_all(&mut self, now: u64) -> Vec<(String, Vec<Op>)> {
        let uuids: Vec<String> = self.online.keys().cloned().collect();
        uuids.into_iter().map(|u| (u.clone(), self.plan_player(&u, now))).filter(|(_, ops)| !ops.is_empty()).collect()
    }
}

thread_local! {
    static RT: RefCell<Option<Rt>> = const { RefCell::new(None) };
}

/// The proxy's PumboPerms rules the network (PumboPerms spec §16): this
/// plugin writes no attachments and PumboBridge does. Outside the state, so
/// that `hello` from the bridge never finds it busy.
static PASSIVE: AtomicBool = AtomicBool::new(false);

pub fn passive() -> bool {
    PASSIVE.load(Ordering::Relaxed)
}

/// Sets the mode; true when it changed.
pub fn set_passive(on: bool) -> bool {
    PASSIVE.swap(on, Ordering::Relaxed) != on
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

/// Writes the log lines collected inside [`with`].
pub fn flush_logs() {
    let lines = with(|rt| std::mem::take(&mut rt.logs)).unwrap_or_default();
    for (warn, line) in lines {
        if warn {
            super::api::warn(&line);
        } else {
            super::api::info(&line);
        }
    }
}
