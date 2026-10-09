//! The running plugin without its platform: the database, its store, the
//! config and the messages, and the operations the platform calls.

use std::collections::BTreeMap;

use pumbo_common::lang::Lang;
use pumbo_common::store::StoreError;

use crate::config::CoreCfg;
use crate::context::Query;
use crate::perms::{Decision, Dirty, Perms};
use crate::store::{LogEntry, PermsStore};

/// Permission nodes of Pumbo plugins start with this.
pub const PUMBO_NODES: &str = "pumbo.";

pub struct Engine {
    pub perms: Perms,
    store: Option<PermsStore>,
    pub cfg: CoreCfg,
    pub lang: Lang,
    /// Why the database is not available; changes are refused while it is set.
    pub broken: Option<String>,
    /// Server groups of each server the platform knows (PumboProx
    /// `server-group`); empty on a single server, where `context` says it.
    pub server_groups: BTreeMap<String, Vec<String>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("broken", &self.broken).finish_non_exhaustive()
    }
}

impl Engine {
    /// Loads the database. Without a usable store the engine still answers
    /// (nobody has anything, the platform defaults decide) but refuses changes.
    pub fn new(store: Result<PermsStore, StoreError>, cfg: CoreCfg, lang: Lang) -> Self {
        let mut engine =
            Self { perms: Perms::new(), store: None, cfg, lang, broken: None, server_groups: BTreeMap::new() };
        match store {
            Ok(store) => match store.load() {
                Ok((perms, dirty)) => {
                    engine.perms = perms;
                    if !dirty.is_empty()
                        && let Err(e) = store.commit(&engine.perms, &dirty, None, 0)
                    {
                        engine.broken = Some(e.to_string());
                    }
                    engine.store = Some(store);
                }
                Err(e) => engine.broken = Some(e.to_string()),
            },
            Err(e) => engine.broken = Some(e.to_string()),
        }
        engine
    }

    /// An engine on a fresh in-memory store (tests).
    pub fn in_memory(cfg: CoreCfg, lang: Lang) -> Self {
        Self::new(PermsStore::in_memory(), cfg, lang)
    }

    pub fn store(&self) -> Option<&PermsStore> {
        self.store.as_ref()
    }

    /// Where a player in `world` is, on this server.
    pub fn query(&self, world: &str) -> Query {
        self.place("", world)
    }

    /// Where a player on `server` (empty: this server) in `world` is, with
    /// the server's groups.
    pub fn place(&self, server: &str, world: &str) -> Query {
        let here = &self.cfg.context;
        let server = if server.is_empty() { here.server.as_str() } else { server };
        let groups = match self.server_groups.get(server) {
            Some(g) => g.clone(),
            None if server == here.server && !here.group.is_empty() => vec![here.group.clone()],
            None => Vec::new(),
        };
        Query::at(server, groups, world)
    }

    /// The decision for a player, with the operator fallback for Pumbo nodes:
    /// `Some(value)`, or `None` when the platform's own defaults should decide.
    pub fn decide(&mut self, uuid: &str, node: &str, world: &str, op_level: u8, now: u64) -> Option<bool> {
        let q = self.query(world);
        self.decide_in(uuid, node, &q, op_level, now)
    }

    /// [`Engine::decide`] at a given place.
    pub fn decide_in(&mut self, uuid: &str, node: &str, q: &Query, op_level: u8, now: u64) -> Option<bool> {
        if let Some(v) = self.perms.check(uuid, node, q, now) {
            return Some(v);
        }
        self.op_fallback(node, op_level)
    }

    /// Like [`Engine::decide`] with the deciding node, when the data decides.
    pub fn explain(&mut self, uuid: &str, node: &str, world: &str, now: u64) -> Option<Decision> {
        let q = self.query(world);
        self.perms.explain(uuid, node, &q, now)
    }

    /// `Some(true)` for a Pumbo node and an operator of a high enough level.
    pub fn op_fallback(&self, node: &str, op_level: u8) -> Option<bool> {
        let level = self.cfg.defaults.op_level;
        (level > 0 && op_level >= level && node.starts_with(PUMBO_NODES)).then_some(true)
    }

    /// Saves a change with its log entry. On failure the database is read back
    /// from the store, so memory never shows what is not on disk.
    pub fn commit(&mut self, dirty: &Dirty, log: Option<&LogEntry>) -> Result<(), String> {
        if let Some(why) = self.broken.clone() {
            self.reload_data();
            return Err(why);
        }
        let Some(store) = &self.store else {
            return Err("no database".into());
        };
        match store.commit(&self.perms, dirty, log, self.cfg.log.max_entries) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.reload_data();
                Err(e.to_string())
            }
        }
    }

    /// Reads the database again (after a failed write, or to throw away
    /// changes made in memory).
    pub fn reload_data(&mut self) {
        if let Some(store) = &self.store {
            match store.load() {
                Ok((perms, _)) => self.perms = perms,
                Err(e) => self.broken = Some(e.to_string()),
            }
        }
    }

    /// Records a joining player's name.
    pub fn seen(&mut self, uuid: &str, name: &str) -> Result<(), String> {
        if self.perms.seen(uuid, name)
            && let Some(store) = &self.store
            && let Some(known) = self.perms.find_user(uuid)
        {
            let known = crate::perms::KnownName { name: name.to_string(), ..known };
            store.save_name(&known).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Drops expired nodes and saves that. True when something expired.
    pub fn expire(&mut self, now: u64) -> Result<bool, String> {
        if self.perms.next_expiry().is_none_or(|t| t > now) {
            return Ok(false);
        }
        let dirty = self.perms.expire(now);
        if dirty.is_empty() {
            return Ok(false);
        }
        self.commit(&dirty, None)?;
        Ok(true)
    }

    /// Whether changes can be saved.
    pub fn writable(&self) -> bool {
        self.broken.is_none() && self.store.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Contexts;
    use crate::model::PermNode;
    use pumbo_common::lang::COMMON;

    const U: &str = "00000000-0000-0000-0000-000000000001";

    fn engine() -> Engine {
        Engine::in_memory(CoreCfg::default(), Lang::load(&[COMMON], "en", None).0)
    }

    #[test]
    fn op_fallback_only_for_pumbo_nodes() {
        let mut e = engine();
        assert_eq!(e.decide(U, "pumbo.bans.ban", "world", 3, 0), Some(true));
        assert_eq!(e.decide(U, "pumbo.bans.ban", "world", 2, 0), None);
        assert_eq!(e.decide(U, "minecraft:command.op", "world", 4, 0), None);
        let (_, d) = e.perms.edit_user(U, "Steve", |d| {
            d.set_permission(PermNode {
                node: "pumbo.bans.ban".into(),
                value: false,
                context: Contexts::global(),
                expiry: None,
            })
        });
        e.commit(&d, None).unwrap();
        assert_eq!(e.decide(U, "pumbo.bans.ban", "world", 4, 0), Some(false), "an explicit false beats the fallback");
        e.cfg.defaults.op_level = 0;
        assert_eq!(e.decide(U, "pumbo.bans.kick", "world", 4, 0), None);
    }

    #[test]
    fn broken_store_refuses_changes_but_answers() {
        let mut e =
            Engine::new(Err(StoreError::new("disk on fire")), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
        assert!(!e.writable());
        let (_, d) = e.perms.create_group("vip").map(|d| ((), d)).unwrap();
        assert_eq!(e.commit(&d, None), Err("disk on fire".into()));
        assert_eq!(e.decide(U, "pumbo.x.y", "", 3, 0), Some(true));
    }

    #[test]
    fn expiry_is_saved() {
        let mut e = engine();
        let (_, d) = e.perms.edit_user(U, "Steve", |d| {
            d.set_permission(PermNode { node: "a".into(), value: true, context: Contexts::global(), expiry: Some(100) })
        });
        e.commit(&d, None).unwrap();
        assert_eq!(e.expire(50), Ok(false));
        assert_eq!(e.expire(100), Ok(true));
        e.reload_data();
        assert!(e.perms.user(U).is_none());
    }
}
