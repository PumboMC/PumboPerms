//! The running plugin without its platform: the database, its store, the
//! config and the messages, and the operations the platform calls.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use pumbo_common::lang::Lang;
use pumbo_common::store::StoreError;

use crate::config::CoreCfg;
use crate::context::Query;
use crate::merge::Ledger;
use crate::perms::{Decision, Dirty, KnownName, Perms};
use crate::store::{LogEntry, PermsStore};

/// The operator fallback covers only these nodes: every other Pumbo plugin
/// registers its own operator levels on Pumpkin.
pub const OWN_NODES: &str = "pumbo.perms.";

/// Records of players who are not online kept in memory at most, besides
/// those a change in progress needs ([`Engine::load_user`]).
pub const VISITORS: usize = 64;

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
    /// The import ledgers as stored (a damaged one as its error), read once:
    /// every join looks at them.
    ledgers: Result<BTreeMap<String, Result<Ledger, String>>, String>,
    /// Sources whose damaged ledger a join already reported.
    pub(crate) reported: BTreeSet<String>,
    /// What changed since [`Engine::take_affected`] last looked; `None`:
    /// everything (the data was read again).
    changes: Option<Dirty>,
    /// The players online now, by UUID ([`Engine::seen`] until
    /// [`Engine::gone`]); everyone else's name is looked up in the database.
    connected: HashMap<String, KnownName>,
    /// Damaged name records already reported, and the lines for
    /// [`Engine::take_warnings`].
    damaged_names: RefCell<(BTreeSet<String>, Vec<String>)>,
    /// Players' records stay in the database until a player is needed
    /// ([`Engine::load_user`]); otherwise all of them are read at start.
    on_demand: bool,
    /// The records in memory of players who are not online, oldest first.
    visitors: VecDeque<String>,
    /// Players whose record lost ended entries when it was read: memory has
    /// them without, the database still with them until [`Engine::expire`]
    /// saves them, so they stay in memory until then.
    swept: BTreeSet<String>,
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
        Self::open(store, cfg, lang, false)
    }

    /// [`Engine::new`], with the players' records read when needed
    /// (`on_demand`) or all at start.
    pub(crate) fn open(store: Result<PermsStore, StoreError>, cfg: CoreCfg, lang: Lang, on_demand: bool) -> Self {
        let mut engine = Self {
            perms: Perms::new(),
            store: None,
            cfg,
            lang,
            broken: None,
            server_groups: BTreeMap::new(),
            root: crate::command::ROOT.to_string(),
            ledgers: Err("no database".into()),
            reported: BTreeSet::new(),
            changes: None,
            connected: HashMap::new(),
            damaged_names: RefCell::default(),
            on_demand,
            visitors: VecDeque::new(),
            swept: BTreeSet::new(),
        };
        match store {
            Ok(store) => match engine.read(&store) {
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

    /// An engine on a fresh in-memory store with every record in memory
    /// (tests).
    pub fn in_memory(cfg: CoreCfg, lang: Lang) -> Self {
        Self::open(PermsStore::in_memory(), cfg, lang, false)
    }

    /// The groups and tracks, and the players' records or only their counts.
    fn read(&self, store: &PermsStore) -> pumbo_common::store::Result<(Perms, Dirty)> {
        if self.on_demand { store.load_index() } else { store.load() }
    }

    /// Brings a player's record into memory before anything reads or changes
    /// it, without its ended entries: memory drops them at once and
    /// [`Engine::expire`] saves that. A record that cannot be read leaves the
    /// database refusing changes and the player without data. Outside a
    /// change, the oldest records of players who are not online go back.
    pub fn load_user(&mut self, uuid: &str, now: u64) {
        if self.perms.knows(uuid) {
            return;
        }
        let record = match self.store.as_ref().map(|s| s.user(uuid)) {
            Some(Ok(record)) => record,
            Some(Err(e)) => {
                self.broken = Some(e.to_string());
                None
            }
            None => None,
        };
        if !self.perms.admit(uuid, record) {
            return;
        }
        if self.perms.user(uuid).is_some_and(|u| u.data.next_expiry().is_some_and(|t| t <= now)) {
            self.perms.edit_user(uuid, "", |d| d.expire(now));
            self.swept.insert(uuid.to_string());
        }
        if !self.connected.contains_key(uuid) {
            self.visitors.push_back(uuid.to_string());
        }
        self.trim();
    }

    /// Lets the oldest records of players who are not online go while more
    /// than [`VISITORS`] are in memory; nothing during a change. A record
    /// whose ended entries are not saved yet stays and waits at the end.
    fn trim(&mut self) {
        if self.perms.changing() {
            return;
        }
        // The newest stays: whoever read it is about to use it.
        let mut turns = self.visitors.len().saturating_sub(1);
        while self.visitors.len() > VISITORS && turns > 0 {
            turns = turns.saturating_sub(1);
            let Some(uuid) = self.visitors.pop_front() else { break };
            if self.connected.contains_key(&uuid) {
                continue;
            }
            if self.swept.contains(&uuid) {
                self.visitors.push_back(uuid);
                continue;
            }
            self.perms.evict(&uuid);
        }
    }

    /// After the data was read again: the online players' records come back
    /// (their ended entries go with the next [`Engine::expire`]).
    fn reread(&mut self) {
        self.visitors.clear();
        self.swept.clear();
        let online: Vec<String> = self.connected.keys().cloned().collect();
        for uuid in online {
            self.load_user(&uuid, 0);
        }
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
        self.load_user(uuid, now);
        if let Some(v) = self.perms.check(uuid, node, q, now) {
            return Some(v);
        }
        self.op_fallback(node, op_level)
    }

    /// Like [`Engine::decide`] with the deciding node, when the data decides.
    pub fn explain(&mut self, uuid: &str, node: &str, world: &str, now: u64) -> Option<Decision> {
        self.load_user(uuid, now);
        let q = self.query(world);
        self.perms.explain(uuid, node, &q, now)
    }

    /// `Some(true)` for a PumboPerms node and an operator of a high enough level.
    pub fn op_fallback(&self, node: &str, op_level: u8) -> Option<bool> {
        let level = self.cfg.defaults.op_level;
        (level > 0 && op_level >= level && node.starts_with(OWN_NODES)).then_some(true)
    }

    /// Starts a change: the records it touches are kept as they were, for
    /// [`Engine::rollback`].
    pub fn begin(&mut self) {
        self.perms.begin();
    }

    /// Undoes the change since [`Engine::begin`] in memory, without reading
    /// the disk (nothing when no change began).
    pub fn rollback(&mut self) {
        self.perms.rollback();
        self.trim();
    }

    /// Ends the change: what it did stays.
    pub fn end(&mut self) {
        self.perms.end();
        self.trim();
    }

    /// Saves a change with its log entry. On failure the change is undone in
    /// memory ([`Engine::begin`]), so memory never shows what is not on disk.
    pub fn commit(&mut self, dirty: &Dirty, log: Option<&LogEntry>) -> Result<(), String> {
        self.changed(dirty);
        let max = self.cfg.log.max_entries;
        self.save(|store, perms| store.commit(perms, dirty, log, max))?;
        self.saved(dirty);
        Ok(())
    }

    /// The records a saved change wrote no longer wait in [`Engine::swept`].
    fn saved(&mut self, dirty: &Dirty) {
        for uuid in &dirty.users {
            self.swept.remove(uuid);
        }
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
        self.saved(dirty);
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
        match self.read(store) {
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
                self.reread();
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
            match self.read(store) {
                Ok((perms, _)) => {
                    self.perms = perms;
                    self.changed_everything();
                    self.reread();
                }
                Err(e) => self.broken = Some(e.to_string()),
            }
        }
    }

    /// Records a joining player's name (`online`: an account the host checked
    /// with Mojang) at `now`: kept in memory while the player is online,
    /// written to the database for later. Returns the line to log when the
    /// name leaves a UUID that still has groups, permissions or meta.
    pub fn seen(&mut self, uuid: &str, name: &str, online: bool, now: u64) -> Result<Option<String>, String> {
        let known = KnownName { uuid: uuid.to_string(), name: name.to_string(), online };
        self.connected.insert(uuid.to_string(), known.clone());
        self.visitors.retain(|v| v != uuid);
        self.load_user(uuid, now);
        self.perms.set_user_name(uuid, name);
        let Some(store) = self.store.as_ref().filter(|_| self.broken.is_none()) else { return Ok(None) };
        let mut damaged = Vec::new();
        let previous = store.save_name(&known, &mut damaged);
        self.note_damaged(damaged);
        let left = previous.map_err(|e| e.to_string())?.filter(|old| old != uuid && self.has_data(old));
        Ok(left.map(|old| {
            format!(
                "PumboPerms: the name {name} moved from {old} to {uuid}; the groups, permissions and meta of {old} stay with {old}"
            )
        }))
    }

    /// Whether a player has groups, permissions or meta of their own, in
    /// memory or in the database (a read error counts as none).
    fn has_data(&self, uuid: &str) -> bool {
        if self.perms.knows(uuid) {
            return self.perms.user(uuid).is_some_and(|u| !u.data.is_empty());
        }
        let stored = self.store.as_ref().map(|s| s.user(uuid));
        stored.is_some_and(|r| r.is_ok_and(|u| u.is_some_and(|u| !u.data.is_empty())))
    }

    /// A player left (the platform calls it once no session of the UUID
    /// is left): their name comes from the database from now on, and their
    /// record waits among the others of players who are not online.
    pub fn gone(&mut self, uuid: &str) {
        if self.connected.remove(uuid).is_some() && self.perms.lazy() && self.perms.knows(uuid) {
            self.visitors.push_back(uuid.to_string());
        }
        self.trim();
    }

    /// Players with their own groups, permissions or meta: the records in
    /// the database, or in memory without one.
    pub fn user_count(&self) -> u64 {
        match self.store.as_ref().map(PermsStore::user_count) {
            Some(Ok(n)) => n,
            _ => u64::try_from(self.perms.users().filter(|u| !u.data.is_empty()).count()).unwrap_or(u64::MAX),
        }
    }

    /// One page (1-based) of the players with a live parent `group` at
    /// `now`, by name and UUID, and how many there are: the records in
    /// memory and those in the database, read in one pass.
    pub fn members_page(
        &self,
        group: &str,
        page: usize,
        per_page: usize,
        now: u64,
    ) -> Result<(Vec<(String, String)>, usize), String> {
        let seed: Vec<(String, String)> =
            self.perms.members_at(group, now).into_iter().map(|u| (u.name.clone(), u.uuid.clone())).collect();
        match self.store.as_ref().filter(|_| self.perms.lazy()) {
            Some(store) => store
                .members_page(group, (page, per_page), now, &|uuid| self.perms.knows(uuid), seed)
                .map_err(|e| e.to_string()),
            None => {
                let total = seed.len();
                let skipped = page.saturating_sub(1).saturating_mul(per_page);
                Ok((seed.into_iter().skip(skipped).take(per_page).collect(), total))
            }
        }
    }

    /// The player online under `uuid`, as [`Engine::seen`] recorded them.
    pub fn connected(&self, uuid: &str) -> Option<&KnownName> {
        self.connected.get(uuid)
    }

    /// A user by nickname or by UUID. A nickname gives the account that
    /// joined last under it (the platform looks among its online players
    /// first); a record's own name never decides it. A UUID always gives a
    /// user: with the name it is online under, the name of its record, the
    /// name it joined with last, or else the UUID as the name.
    pub fn find_user(&self, name_or_uuid: &str) -> Option<KnownName> {
        let Some(uuid) = pumbo_common::id::Uuid::parse(name_or_uuid).map(|u| u.to_string()) else {
            return self.stored(|store, damaged| store.find_name(name_or_uuid, damaged));
        };
        if let Some(known) = self.connected.get(&uuid) {
            return Some(known.clone());
        }
        let recorded = if self.perms.knows(&uuid) {
            self.perms.user(&uuid).map(|u| u.name.clone())
        } else {
            self.store.as_ref().and_then(|s| s.user(&uuid).ok().flatten()).map(|u| u.name)
        };
        let recorded = recorded.filter(|n| !n.is_empty());
        let known = match recorded {
            Some(name) => Some(KnownName { uuid: uuid.clone(), name, online: false }),
            None => self.stored(|store, damaged| store.name_of(&uuid, damaged)),
        };
        Some(known.unwrap_or_else(|| KnownName { name: uuid.clone(), uuid, online: false }))
    }

    /// A name lookup in the database; a read error counts as not found.
    fn stored(
        &self,
        lookup: impl FnOnce(&PermsStore, &mut Vec<String>) -> Result<Option<KnownName>, StoreError>,
    ) -> Option<KnownName> {
        let store = self.store.as_ref()?;
        let mut damaged = Vec::new();
        let found = lookup(store, &mut damaged);
        self.note_damaged(damaged);
        found.ok().flatten()
    }

    /// One warning per damaged name record, for [`Engine::take_warnings`].
    fn note_damaged(&self, damaged: Vec<String>) {
        if damaged.is_empty() {
            return;
        }
        let mut notes = self.damaged_names.borrow_mut();
        let (reported, lines) = &mut *notes;
        for record in damaged {
            if reported.insert(record.clone()) {
                lines.push(format!(
                    "PumboPerms: the name record {record} is damaged; it counts as missing until the player joins again"
                ));
            }
        }
    }

    /// Lines the platform logs as warnings (damaged name records), each once.
    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut self.damaged_names.borrow_mut().1)
    }

    /// Drops expired nodes and saves that, with the records whose ended
    /// entries went when they were read. Returns whether something in memory
    /// was due and the result of saving: a node past its time is gone from
    /// every answer either way (they skip expired nodes), so the platform
    /// sends the new decisions even when saving failed; the next call tries
    /// again.
    pub fn expire(&mut self, now: u64) -> (bool, Result<(), String>) {
        let due = self.perms.next_expiry().is_some_and(|t| t <= now);
        if !due && self.swept.is_empty() {
            return (false, Ok(()));
        }
        self.begin();
        let mut dirty = if due { self.perms.expire(now) } else { Dirty::default() };
        dirty.users.extend(self.swept.iter().cloned());
        if dirty.is_empty() {
            self.end();
            return (due, Ok(()));
        }
        (due, self.commit(&dirty, None))
    }

    /// Whether changes can be saved.
    pub fn writable(&self) -> bool {
        self.broken.is_none() && self.store.is_some()
    }
}

#[cfg(test)]
mod tests;
