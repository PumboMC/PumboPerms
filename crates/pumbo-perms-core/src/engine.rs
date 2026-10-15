//! The running plugin without its platform: the database, its store, the
//! config and the messages, and the operations the platform calls.

use std::collections::{BTreeMap, BTreeSet};

use pumbo_common::lang::Lang;
use pumbo_common::store::StoreError;

use crate::config::CoreCfg;
use crate::context::Query;
use crate::merge::Ledger;
use crate::perms::{Decision, Dirty, Perms, Saved};
use crate::store::{LogEntry, PermsStore};

/// The operator fallback covers only these nodes: every other Pumbo plugin
/// registers its own operator levels on Pumpkin.
pub const OWN_NODES: &str = "pumbo.perms.";

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
    /// The command in help, usage lines and clicks: [`crate::command::ROOT`],
    /// or what the platform registered instead.
    pub root: String,
    /// The data before the change in progress ([`Engine::begin`]): a change
    /// that cannot be saved is undone from it, without reading the disk.
    before: Option<Saved>,
    /// The import ledgers as stored (a damaged one as its error), read once:
    /// every join looks at them.
    ledgers: Result<BTreeMap<String, Result<Ledger, String>>, String>,
    /// Sources whose damaged ledger a join already reported.
    pub(crate) reported: BTreeSet<String>,
    /// What changed since [`Engine::take_affected`] last looked; `None`:
    /// everything (the data was read again).
    changes: Option<Dirty>,
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
        let mut engine = Self {
            perms: Perms::new(),
            store: None,
            cfg,
            lang,
            broken: None,
            server_groups: BTreeMap::new(),
            root: crate::command::ROOT.to_string(),
            before: None,
            ledgers: Err("no database".into()),
            reported: BTreeSet::new(),
            changes: None,
        };
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
                Err(e) => {
                    // Kept for `retry_open`; changes are refused meanwhile.
                    engine.broken = Some(e.to_string());
                    engine.store = Some(store);
                }
            },
            Err(e) => engine.broken = Some(e.to_string()),
        }
        engine.load_ledgers();
        engine
    }

    /// Reads the import ledgers from the database into memory.
    fn load_ledgers(&mut self) {
        self.reported.clear();
        self.ledgers = match &self.store {
            Some(store) => store
                .ledgers()
                .map(|all| {
                    all.into_iter()
                        .map(|(s, l)| {
                            let l = l.map_err(|e| format!("the import ledger of {s} is damaged: {e}"));
                            (s, l)
                        })
                        .collect()
                })
                .map_err(|e| e.to_string()),
            None => Err(self.broken.clone().unwrap_or_else(|| "no database".into())),
        };
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

    /// Where a player on `server` (empty: this server; any letter case) in
    /// `world` is, with the server's groups.
    pub fn place(&self, server: &str, world: &str) -> Query {
        let here = &self.cfg.context;
        let server = if server.is_empty() { here.server.to_lowercase() } else { server.to_lowercase() };
        let groups = match self.server_groups.get(&server) {
            Some(g) => g.clone(),
            None if server == here.server && !here.group.is_empty() => vec![here.group.clone()],
            None => Vec::new(),
        };
        Query::at(&server, groups, world)
    }

    /// The decision for a player, with the operator fallback for PumboPerms nodes:
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

    /// `Some(true)` for a PumboPerms node and an operator of a high enough level.
    pub fn op_fallback(&self, node: &str, op_level: u8) -> Option<bool> {
        let level = self.cfg.defaults.op_level;
        (level > 0 && op_level >= level && node.starts_with(OWN_NODES)).then_some(true)
    }

    /// Remembers the data before a change, for [`Engine::rollback`].
    pub fn begin(&mut self) {
        self.before = Some(self.perms.saved());
    }

    /// Undoes the change since [`Engine::begin`] in memory (nothing when no
    /// change began).
    pub fn rollback(&mut self) {
        if let Some(saved) = self.before.take() {
            self.perms.restore(saved);
        }
    }

    /// Forgets the data kept by [`Engine::begin`].
    pub fn end(&mut self) {
        self.before = None;
    }

    /// Saves a change with its log entry. On failure the change is undone in
    /// memory ([`Engine::begin`]), so memory never shows what is not on disk.
    pub fn commit(&mut self, dirty: &Dirty, log: Option<&LogEntry>) -> Result<(), String> {
        self.changed(dirty);
        let max = self.cfg.log.max_entries;
        self.save(|store, perms| store.commit(perms, dirty, log, max))
    }

    /// [`Engine::commit`] of an import, with the ledger of its source in the
    /// same transaction.
    pub fn commit_import(
        &mut self,
        dirty: &Dirty,
        log: Option<&LogEntry>,
        source: &str,
        ledger: &Ledger,
    ) -> Result<(), String> {
        self.changed(dirty);
        let max = self.cfg.log.max_entries;
        self.save(|store, perms| store.commit_import(perms, dirty, log, max, (source, ledger)))?;
        if let Ok(all) = &mut self.ledgers {
            if *ledger == Ledger::default() {
                all.remove(source);
            } else {
                all.insert(source.to_string(), Ok(ledger.clone()));
            }
            self.reported.remove(source);
        }
        Ok(())
    }

    /// Records a change for [`Engine::take_affected`], also one that is not
    /// saved: undoing it in memory is a change too.
    fn changed(&mut self, dirty: &Dirty) {
        if let Some(all) = &mut self.changes {
            all.merge(dirty.clone());
        }
    }

    /// Everything may have changed (the data was replaced or read again).
    pub fn changed_everything(&mut self) {
        self.changes = None;
    }

    /// Of `uuids`, the users whose decisions may differ since the last call;
    /// starts counting again.
    pub fn take_affected<'a>(&mut self, uuids: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
        let Some(dirty) = self.changes.replace(Dirty::default()) else { return uuids.into_iter().collect() };
        let groups: Vec<&str> = dirty.groups.iter().chain(&dirty.deleted_groups).map(String::as_str).collect();
        let everyone = self.perms.reaches_everyone(&groups);
        uuids
            .into_iter()
            .filter(|u| everyone || dirty.users.contains(*u) || (!groups.is_empty() && self.perms.reaches(u, &groups)))
            .collect()
    }

    fn save(&mut self, write: impl FnOnce(&PermsStore, &Perms) -> Result<(), StoreError>) -> Result<(), String> {
        let result = match (&self.broken, &self.store) {
            (Some(why), _) => Err(why.clone()),
            (None, None) => Err("no database".into()),
            (None, Some(store)) => write(store, &self.perms).map_err(|e| e.to_string()),
        };
        if result.is_err() {
            self.rollback();
        }
        self.end();
        result
    }

    /// What `source` added at its last import; an error for a ledger that
    /// cannot be read (the import of that source waits for an admin).
    pub fn ledger(&self, source: &str) -> Result<Ledger, String> {
        match &self.ledgers {
            Ok(all) => all.get(source).cloned().unwrap_or_else(|| Ok(Ledger::default())),
            Err(why) => Err(why.clone()),
        }
    }

    /// Every source with a ledger (a damaged one as its error).
    pub fn ledgers(&self) -> Vec<(&str, Result<&Ledger, &str>)> {
        match &self.ledgers {
            Ok(all) => all.iter().map(|(s, l)| (s.as_str(), l.as_ref().map_err(String::as_str))).collect(),
            Err(_) if self.store.is_none() => Vec::new(),
            Err(why) => vec![("", Err(why.as_str()))],
        }
    }

    /// Moves the import ledgers of a schema 1 database from `dir` (the
    /// platform's `imports/` folder) into the database. Returns the line to
    /// log, if any. A failure leaves the database refusing changes: the
    /// ledgers would otherwise look empty and every import would add all
    /// again.
    pub fn migrate_ledgers(&mut self, dir: &std::path::Path) -> Option<String> {
        let migrated = self.store.as_ref()?.migrate_ledgers(dir);
        match migrated {
            Ok(None) => None,
            Ok(Some(moved)) => {
                self.load_ledgers();
                Some(format!(
                    "PumboPerms: database schema 1 -> {}: {} import ledgers moved into the database{}",
                    crate::store::SCHEMA,
                    moved.len(),
                    if moved.is_empty() { String::new() } else { format!(" ({})", moved.join(", ")) }
                ))
            }
            Err(e) => {
                let why = format!("cannot move the import ledgers into the database: {e}");
                self.broken = Some(why.clone());
                self.ledgers = Err(why.clone());
                Some(format!("PumboPerms: {why}"))
            }
        }
    }

    /// While the database is broken, tries to read it again: on success
    /// changes work again. Returns whether it recovered. The platform calls
    /// this now and then (not at every tick: a load reads everything).
    pub fn retry_open(&mut self) -> bool {
        let (Some(_), Some(store)) = (&self.broken, &self.store) else { return false };
        // Ledgers still in files: the next start moves them first.
        if store.schema().is_ok_and(|v| v < crate::store::SCHEMA) {
            return false;
        }
        match store.load() {
            Ok((perms, dirty)) => {
                if !dirty.is_empty()
                    && let Err(e) = store.commit(&perms, &dirty, None, 0)
                {
                    self.broken = Some(e.to_string());
                    return false;
                }
                self.perms = perms;
                self.broken = None;
                self.load_ledgers();
                self.changed_everything();
                true
            }
            Err(e) => {
                self.broken = Some(e.to_string());
                false
            }
        }
    }

    /// Lets go of the database file (another instance owns it now): nothing
    /// is written to it any more and changes are refused.
    pub fn close(&mut self, why: String) {
        self.store = None;
        self.ledgers = Err(why.clone());
        self.broken = Some(why);
    }

    /// Reads the database again.
    pub fn reload_data(&mut self) {
        if let Some(store) = &self.store {
            match store.load() {
                Ok((perms, _)) => {
                    self.perms = perms;
                    self.changed_everything();
                }
                Err(e) => self.broken = Some(e.to_string()),
            }
        }
    }

    /// Records a joining player's name (`online`: an account the host checked
    /// with Mojang). Returns the line to log when the name leaves a UUID that
    /// still has groups, permissions or meta.
    pub fn seen(&mut self, uuid: &str, name: &str, online: bool) -> Result<Option<String>, String> {
        let old = self.perms.name_key_of(uuid).map(str::to_string);
        let (changed, left) = self.perms.seen(uuid, name, online);
        if changed
            && self.broken.is_none()
            && let Some(store) = &self.store
            && let Some(known) = self.perms.find_user(name)
        {
            store.save_name(&known, old.as_deref()).map_err(|e| e.to_string())?;
        }
        Ok(left.map(|old| {
            format!(
                "PumboPerms: the name {name} moved from {old} to {uuid}; the groups, permissions and meta of {old} stay with {old}"
            )
        }))
    }

    /// Drops expired nodes and saves that. Returns whether something was due
    /// and the result of saving: a node past its time is gone from every
    /// answer either way (they skip expired nodes), so the platform sends the
    /// new decisions even when saving failed; the next call tries again.
    pub fn expire(&mut self, now: u64) -> (bool, Result<(), String>) {
        if self.perms.next_expiry().is_none_or(|t| t > now) {
            return (false, Ok(()));
        }
        self.begin();
        let dirty = self.perms.expire(now);
        if dirty.is_empty() {
            self.end();
            return (true, Ok(()));
        }
        (true, self.commit(&dirty, None))
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
    fn op_fallback_only_for_own_nodes() {
        let mut e = engine();
        assert_eq!(e.decide(U, "pumbo.perms.reload", "world", 3, 0), Some(true));
        assert_eq!(e.decide(U, "pumbo.perms.reload", "world", 2, 0), None);
        assert_eq!(e.decide(U, "minecraft:command.op", "world", 4, 0), None);
        let (_, d) = e.perms.edit_user(U, "Steve", |d| {
            d.set_permission(PermNode {
                node: "pumbo.perms.reload".into(),
                value: false,
                context: Contexts::global(),
                expiry: None,
            })
        });
        e.commit(&d, None).unwrap();
        assert_eq!(
            e.decide(U, "pumbo.perms.reload", "world", 4, 0),
            Some(false),
            "an explicit false beats the fallback"
        );
        e.cfg.defaults.op_level = 0;
        assert_eq!(e.decide(U, "pumbo.perms.log", "world", 4, 0), None);
    }

    #[test]
    fn op_fallback_does_not_override_plugin_defaults() {
        let mut e = engine();
        for node in ["pumbo.bans.ban", "pumbo.bans.limit.helper", "pumbo.bans.exempt.ban"] {
            assert_eq!(e.decide(U, node, "world", 4, 0), None, "{node}: Pumpkin's registry decides");
        }
    }

    #[test]
    fn broken_store_at_start_decides_nothing() {
        let mut e =
            Engine::new(Err(StoreError::new("disk on fire")), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
        assert!(!e.writable());
        let (_, d) = e.perms.create_group("vip").map(|d| ((), d)).unwrap();
        assert_eq!(e.commit(&d, None), Err("disk on fire".into()));
        assert_eq!(e.decide(U, "pumbo.bans.ban", "", 4, 0), None);
        assert_eq!(e.decide(U, "minecraft:command.op", "", 4, 0), None);
        assert_eq!(e.decide(U, "pumbo.perms.reload", "", 3, 0), Some(true), "operators can still repair it with /pp");
    }

    #[test]
    fn broken_clears_after_reads_recover() {
        let backend = pumbo_common::store::FaultyBackend::default();
        let reads = backend.fail_reads.clone();
        let store = PermsStore::new(pumbo_common::store::Store::new(backend)).unwrap();
        reads.store(u32::MAX, std::sync::atomic::Ordering::SeqCst);
        let mut e = Engine::new(Ok(store), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
        assert!(e.broken.is_some() && !e.retry_open(), "{:?}", e.broken);
        reads.store(0, std::sync::atomic::Ordering::SeqCst);
        assert!(e.retry_open());
        assert!(e.broken.is_none() && e.writable());
        assert!(!e.retry_open(), "nothing to recover");
    }

    #[test]
    fn seen_writes_nothing_while_broken() {
        let mut e = engine();
        e.broken = Some("taken over".into());
        e.seen(U, "Steve", true).unwrap();
        e.broken = None;
        e.reload_data();
        assert!(e.perms.find_user("Steve").is_none());
    }

    #[test]
    fn ledger_files_migrate_to_table() {
        use pumbo_common::store::{Store, WriteExt};
        // `{data}/imports/` on the proxy, `<data folder>/imports/` on Pumpkin.
        for layout in ["proxy-data", "pumpkin-plugins-data-pumboperms"] {
            let dir = std::env::temp_dir().join(format!("pumbo-perms-ledgers-{layout}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("imports")).unwrap();
            let ledger = Ledger { fingerprint: "f1".into(), at: 5, ..Ledger::default() };
            std::fs::write(dir.join("imports/file.json"), serde_json::to_string_pretty(&ledger).unwrap()).unwrap();
            let store = Store::open(dir.join("perms.redb")).unwrap();
            store.write(|tx| tx.put_u64(crate::store::META, "schema", 1)).unwrap();
            let mut e = Engine::new(PermsStore::new(store), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
            let line = e.migrate_ledgers(&dir.join("imports")).unwrap();
            assert!(line.contains("schema 1 -> 2") && line.contains("file"), "{line}");
            assert_eq!(e.ledger("file").unwrap(), ledger);
            assert!(dir.join("imports/file.json.migrated").exists());
            assert_eq!(e.migrate_ledgers(&dir.join("imports")), None);
            drop(e);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A join with 400 000 known names: a hash lookup and a point write.
    #[test]
    #[ignore = "release benchmark"]
    fn seen_is_constant_time() {
        use pumbo_common::store::{Store, WriteExt};
        // On redb, as the plugins run: the in-memory store copies itself at every write.
        let dir = std::env::temp_dir().join(format!("pumbo-perms-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("perms.redb")).unwrap();
        store
            .write(|tx| {
                for i in 0..400_000u32 {
                    let uuid = format!("00000000-0000-4000-8000-{i:012}");
                    tx.put_json(
                        crate::store::NAMES,
                        &format!("player{i}"),
                        &serde_json::json!({"uuid": uuid, "name": format!("Player{i}")}),
                    )?;
                }
                Ok(())
            })
            .unwrap();
        let mut e = Engine::new(PermsStore::new(store), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
        assert_eq!(e.perms.known_names().count(), 400_000);
        let mut worst = std::time::Duration::ZERO;
        for i in 0..200u32 {
            // New players, and known ones under a new name.
            let (uuid, name) = match i % 2 {
                0 => (format!("00000000-0000-4000-9000-{i:012}"), format!("Newcomer{i}")),
                _ => (format!("00000000-0000-4000-8000-{i:012}"), format!("Renamed{i}")),
            };
            let start = std::time::Instant::now();
            e.seen(&uuid, &name, true).unwrap();
            worst = worst.max(start.elapsed());
        }
        e.reload_data();
        assert!(e.perms.find_user("Player1").is_none() && e.perms.find_user("renamed1").is_some());
        assert_eq!(e.perms.known_names().count(), 400_100);
        println!("Engine::seen with 400 000 names: at most {worst:?}");
        drop(e);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(worst < std::time::Duration::from_millis(1), "{worst:?}");
    }

    #[test]
    fn expiry_is_saved() {
        let mut e = engine();
        let (_, d) = e.perms.edit_user(U, "Steve", |d| {
            d.set_permission(PermNode { node: "a".into(), value: true, context: Contexts::global(), expiry: Some(100) })
        });
        e.commit(&d, None).unwrap();
        assert_eq!(e.expire(50), (false, Ok(())));
        assert_eq!(e.expire(100), (true, Ok(())));
        e.reload_data();
        assert!(e.perms.user(U).is_none());
    }
}
