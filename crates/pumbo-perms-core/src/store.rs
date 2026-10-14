//! Saving the database through [`pumbo_common::store`].
//!
//! Tables (all values JSON unless noted):
//! - `perms_groups`: group name -> [`Group`]
//! - `perms_users`: UUID -> [`User`] (only users with data)
//! - `perms_names`: nickname key -> [`KnownName`] (everyone who joined)
//! - `perms_tracks`: track name -> [`Track`]
//! - `perms_log`: 20-digit sequence number -> [`LogEntry`]
//! - `perms_host_nodes`: nodes the platform ever handed to its host (value empty)
//! - `perms_imports`: source name -> [`Ledger`] (what an import added), from schema 2
//! - `perms_meta`: `schema`, `log_seq` (numbers)
//!
//! The whole database is read once at start; every change writes what it
//! touched in one transaction together with its log entry, an import also
//! with its ledger.
//!
//! Schema 1 (0.1.2-beta and older) kept the ledgers as files
//! (`imports/<source>.json`); [`PermsStore::migrate_ledgers`] moves them into
//! the table and raises the schema to 2. A build that knows only schema 1
//! refuses the file after that.

use std::collections::BTreeSet;

use pumbo_common::store::{Durability, ReadExt, Result, Store, StoreError, WriteExt, WriteOps};
use serde::{Deserialize, Serialize};

use crate::merge::Ledger;
use crate::model::{Group, Track, User};
use crate::perms::{Dirty, KnownName, Perms};

pub const GROUPS: &str = "perms_groups";
pub const USERS: &str = "perms_users";
pub const NAMES: &str = "perms_names";
pub const TRACKS: &str = "perms_tracks";
pub const LOG: &str = "perms_log";
pub const HOST_NODES: &str = "perms_host_nodes";
pub const IMPORTS: &str = "perms_imports";
pub const META: &str = "perms_meta";

/// Version of the stored data.
pub const SCHEMA: u64 = 2;

/// One change, as shown by `/pp log`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogEntry {
    pub time: u64,
    /// Who made the change (`Console` or a nickname).
    pub actor: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor_uuid: String,
    /// `user`, `group` or `track`.
    pub kind: String,
    /// Nickname, group or track name.
    pub target: String,
    /// What was done, as the command arguments after the target.
    pub action: String,
}

#[derive(Serialize, Deserialize)]
struct StoredName {
    uuid: String,
    name: String,
    /// Absent in schema 1: such a name counts as offline until the next join.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    online: bool,
}

/// The permission tables of one store.
#[derive(Debug)]
pub struct PermsStore {
    store: Store,
}

impl PermsStore {
    /// Opens the tables, writing the schema version on first use. A newer
    /// schema than this build knows is refused.
    pub fn new(store: Store) -> Result<Self> {
        let schema = store.read(|tx| tx.get_u64(META, "schema"))?;
        // Schema 1 stays until the platform calls `migrate_ledgers`.
        match schema {
            None => store.write(|tx| tx.put_u64(META, "schema", SCHEMA))?,
            Some(v) if v > SCHEMA => {
                return Err(StoreError::new(format!(
                    "database schema {v} is newer than this version understands ({SCHEMA})"
                )));
            }
            Some(_) => {}
        }
        Ok(Self { store })
    }

    pub fn in_memory() -> Result<Self> {
        Self::new(Store::in_memory())
    }

    /// Reads everything. A damaged record stops the load (fail closed) instead
    /// of silently losing someone's rank.
    pub fn load(&self) -> Result<(Perms, Dirty)> {
        let (groups, users, names, tracks) = self.store.read(|tx| {
            let mut groups: Vec<Group> = Vec::new();
            for (k, v) in tx.collect_prefix(GROUPS, "")? {
                groups.push(decode(GROUPS, &k, &v)?);
            }
            let mut users: Vec<User> = Vec::new();
            for (k, v) in tx.collect_prefix(USERS, "")? {
                users.push(decode(USERS, &k, &v)?);
            }
            let mut names: Vec<KnownName> = Vec::new();
            for (k, v) in tx.collect_prefix(NAMES, "")? {
                let n: StoredName = decode(NAMES, &k, &v)?;
                names.push(KnownName { uuid: n.uuid, name: n.name, online: n.online });
            }
            let mut tracks: Vec<Track> = Vec::new();
            for (k, v) in tx.collect_prefix(TRACKS, "")? {
                tracks.push(decode(TRACKS, &k, &v)?);
            }
            Ok((groups, users, names, tracks))
        })?;
        Ok(Perms::from_parts(groups, users, names, tracks))
    }

    /// Writes what `dirty` names, as it is now in `perms`, and the log entry,
    /// all in one transaction.
    pub fn commit(&self, perms: &Perms, dirty: &Dirty, log: Option<&LogEntry>, log_max: u64) -> Result<()> {
        self.store.write(|tx| {
            write_dirty(tx, perms, dirty)?;
            if let Some(entry) = log {
                append_log(tx, entry, log_max)?;
            }
            Ok(())
        })
    }

    /// Records a nickname (cheap, may be lost on a crash: it comes back at the next join).
    pub fn save_name(&self, name: &KnownName) -> Result<()> {
        self.store.write_with(Durability::Eventual, |tx| {
            tx.retain(NAMES, &mut |_, v| {
                serde_json::from_slice::<StoredName>(v).map_or(true, |n| n.uuid != name.uuid)
            })?;
            tx.put_json(
                NAMES,
                &pumbo_common::id::name_key(&name.name),
                &StoredName { uuid: name.uuid.clone(), name: name.name.clone(), online: name.online },
            )
        })
    }

    /// [`PermsStore::commit`] of an import together with the ledger of its
    /// source (an empty ledger removes it).
    pub fn commit_import(
        &self,
        perms: &Perms,
        dirty: &Dirty,
        log: Option<&LogEntry>,
        log_max: u64,
        (source, ledger): (&str, &Ledger),
    ) -> Result<()> {
        self.store.write(|tx| {
            write_dirty(tx, perms, dirty)?;
            if let Some(entry) = log {
                append_log(tx, entry, log_max)?;
            }
            if *ledger == Ledger::default() {
                tx.remove(IMPORTS, source)?;
            } else {
                tx.put_json(IMPORTS, source, ledger)?;
            }
            Ok(())
        })
    }

    /// What `source` added at its last import. A ledger that does not decode is
    /// an error, never an empty one: that would add everything again.
    pub fn ledger(&self, source: &str) -> Result<Ledger> {
        Ok(self.store.get_json(IMPORTS, source)?.unwrap_or_default())
    }

    /// Every source with a ledger; a damaged one comes as its error.
    pub fn ledgers(&self) -> Result<Vec<(String, Result<Ledger>)>> {
        let all = self.store.read(|tx| tx.collect_prefix(IMPORTS, ""))?;
        Ok(all
            .into_iter()
            .map(|(k, v)| {
                let l = decode(IMPORTS, &k, &v);
                (k, l)
            })
            .collect())
    }

    /// The schema of the file.
    pub fn schema(&self) -> Result<u64> {
        Ok(self.store.read(|tx| tx.get_u64(META, "schema"))?.unwrap_or(SCHEMA))
    }

    /// Schema 1 to 2: every `<dir>/<source>.json` goes into `perms_imports`
    /// as it is (an unreadable one stays unreadable there and blocks its
    /// source), all in one transaction with the new schema; then the files
    /// are renamed to `<source>.json.migrated`. Returns the moved sources,
    /// `None` at schema 2.
    pub fn migrate_ledgers(&self, dir: &std::path::Path) -> Result<Option<Vec<String>>> {
        if self.schema()? >= 2 {
            return Ok(None);
        }
        let mut files: Vec<(String, std::path::PathBuf, Vec<u8>)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(source) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json"))
                else {
                    continue;
                };
                if !crate::export::is_valid_file_name(source) {
                    continue;
                }
                let bytes = std::fs::read(&path)
                    .map_err(|e| StoreError::new(format!("cannot read {}: {e}", path.display())))?;
                files.push((source.to_string(), path, bytes));
            }
        }
        files.sort();
        self.store.write(|tx| {
            for (source, _, bytes) in &files {
                tx.put(IMPORTS, source, bytes)?;
            }
            tx.put_u64(META, "schema", SCHEMA)
        })?;
        for (_, path, _) in &files {
            let mut done = path.as_os_str().to_owned();
            done.push(".migrated");
            // The data is in the table; a file left behind is only read again by an old build.
            let _ = std::fs::rename(path, done);
        }
        Ok(Some(files.into_iter().map(|(s, _, _)| s).collect()))
    }

    /// Replaces every group, user and track (import), with a log entry.
    pub fn replace_all(&self, perms: &Perms, log: Option<&LogEntry>, log_max: u64) -> Result<()> {
        self.store.write(|tx| {
            for table in [GROUPS, USERS, TRACKS] {
                tx.retain(table, &mut |_, _| false)?;
            }
            let dirty = Dirty {
                groups: perms.groups().map(|g| g.name.clone()).collect(),
                users: perms.users().map(|u| u.uuid.clone()).collect(),
                tracks: perms.tracks().map(|t| t.name.clone()).collect(),
                ..Dirty::default()
            };
            write_dirty(tx, perms, &dirty)?;
            if let Some(entry) = log {
                append_log(tx, entry, log_max)?;
            }
            Ok(())
        })
    }

    /// Log entries, newest first: page `page` (1-based) of `per_page`, and the total.
    pub fn log_page(&self, page: usize, per_page: usize) -> Result<(Vec<LogEntry>, usize)> {
        let all = self.store.read(|tx| tx.collect_prefix(LOG, ""))?;
        let total = all.len();
        let skip = page.saturating_sub(1).saturating_mul(per_page);
        let entries = all
            .iter()
            .rev()
            .skip(skip)
            .take(per_page)
            .map(|(k, v)| decode::<LogEntry>(LOG, k, v))
            .collect::<Result<Vec<_>>>()?;
        Ok((entries, total))
    }

    /// Nodes the platform has ever set on its host.
    pub fn host_nodes(&self) -> Result<BTreeSet<String>> {
        let all = self.store.read(|tx| tx.collect_prefix(HOST_NODES, ""))?;
        Ok(all.into_iter().map(|(k, _)| k).collect())
    }

    pub fn add_host_nodes<'a>(&self, nodes: impl IntoIterator<Item = &'a String>) -> Result<()> {
        let nodes: Vec<&String> = nodes.into_iter().collect();
        if nodes.is_empty() {
            return Ok(());
        }
        self.store.write_with(Durability::Eventual, |tx| {
            for n in &nodes {
                tx.put(HOST_NODES, n, b"")?;
            }
            Ok(())
        })
    }
}

fn write_dirty(tx: &mut dyn WriteOps, perms: &Perms, dirty: &Dirty) -> Result<()> {
    for g in &dirty.deleted_groups {
        tx.remove(GROUPS, g)?;
    }
    for t in &dirty.deleted_tracks {
        tx.remove(TRACKS, t)?;
    }
    for name in &dirty.groups {
        match perms.group(name) {
            Some(g) => tx.put_json(GROUPS, name, g)?,
            None => {
                tx.remove(GROUPS, name)?;
            }
        }
    }
    for uuid in &dirty.users {
        match perms.user(uuid) {
            Some(u) if !u.data.is_empty() => tx.put_json(USERS, uuid, u)?,
            _ => {
                tx.remove(USERS, uuid)?;
            }
        }
    }
    for name in &dirty.tracks {
        match perms.track(name) {
            Some(t) => tx.put_json(TRACKS, name, t)?,
            None => {
                tx.remove(TRACKS, name)?;
            }
        }
    }
    Ok(())
}

#[expect(clippy::integer_division, reason = "batch size, rounded down")]
fn append_log(tx: &mut dyn WriteOps, entry: &LogEntry, log_max: u64) -> Result<()> {
    let seq = tx.get_u64(META, "log_seq")?.unwrap_or(0).saturating_add(1);
    tx.put_u64(META, "log_seq", seq)?;
    tx.put_json(LOG, &format!("{seq:020}"), entry)?;
    let len = tx.len(LOG)?;
    // trim in batches so that not every change rewrites the table
    if log_max > 0 && len > log_max.saturating_add(log_max / 10).max(log_max + 1) {
        let mut extra = len - log_max;
        tx.retain(LOG, &mut |_, _| {
            if extra > 0 {
                extra -= 1;
                false
            } else {
                true
            }
        })?;
    }
    Ok(())
}

fn decode<T: serde::de::DeserializeOwned>(table: &str, key: &str, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| StoreError::new(format!("damaged record {table}/{key}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Contexts, Query};
    use crate::model::{ParentNode, PermNode};

    const U: &str = "00000000-0000-0000-0000-000000000001";

    fn entry(n: u64) -> LogEntry {
        LogEntry {
            time: n,
            actor: "Console".into(),
            kind: "group".into(),
            target: "vip".into(),
            action: format!("#{n}"),
            ..LogEntry::default()
        }
    }

    #[test]
    fn round_trip_and_partial_writes() {
        let s = PermsStore::in_memory().unwrap();
        let (mut p, dirty) = s.load().unwrap();
        // a fresh database creates the default group and wants it saved
        assert!(dirty.groups.contains("default"));
        s.commit(&p, &dirty, None, 100).unwrap();

        let mut d = p.create_group("vip").unwrap();
        d.merge(
            p.edit_group("vip", |g| {
                g.data.set_permission(PermNode {
                    node: "a".into(),
                    value: true,
                    context: Contexts::global(),
                    expiry: None,
                })
            })
            .unwrap()
            .1,
        );
        d.merge(
            p.edit_user(U, "Steve", |u| {
                u.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None })
            })
            .1,
        );
        d.merge(p.create_track("ranks").unwrap());
        s.commit(&p, &d, Some(&entry(1)), 100).unwrap();
        p.seen(U, "Steve", true);
        s.save_name(&p.find_user("steve").unwrap()).unwrap();

        let (mut q, dirty) = s.load().unwrap();
        assert!(dirty.is_empty());
        assert_eq!(q.check(U, "a", &Query::default(), 0), Some(true));
        assert_eq!(q.find_user("Steve").unwrap().uuid, U);
        assert!(q.track("ranks").is_some());

        // users without data are removed; deleting a group removes its record
        let mut d = q.edit_user(U, "Steve", |u| u.parents.clear()).1;
        d.merge(q.delete_group("vip").unwrap());
        s.commit(&q, &d, None, 100).unwrap();
        let (r, _) = s.load().unwrap();
        assert!(r.user(U).is_none() && r.group("vip").is_none());
        assert_eq!(r.find_user("steve").unwrap().uuid, U, "the name index stays");
    }

    #[test]
    fn log_pages_newest_first_and_is_trimmed() {
        let s = PermsStore::in_memory().unwrap();
        let p = Perms::new();
        for n in 1..=30 {
            s.commit(&p, &Dirty::default(), Some(&entry(n)), 10).unwrap();
        }
        let (page, total) = s.log_page(1, 5).unwrap();
        assert!(total <= 11, "{total}");
        assert_eq!(page.first().unwrap().time, 30);
        assert_eq!(page.len(), 5);
        let (page2, _) = s.log_page(2, 5).unwrap();
        assert_eq!(page2.first().unwrap().time, 25);
        let (none, _) = s.log_page(99, 5).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn damaged_records_are_errors() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(GROUPS, "vip", b"{broken")).unwrap();
        let s = PermsStore::new(store).unwrap();
        assert!(s.load().is_err());
    }

    #[test]
    fn host_nodes_and_import() {
        let s = PermsStore::in_memory().unwrap();
        s.add_host_nodes(&["minecraft:command.op".to_string(), "a".to_string()]).unwrap();
        s.add_host_nodes(&["a".to_string()]).unwrap();
        assert_eq!(s.host_nodes().unwrap().len(), 2);

        let mut p = Perms::new();
        p.create_group("old").unwrap();
        s.replace_all(&p, None, 10).unwrap();
        let mut fresh = Perms::new();
        fresh.create_group("new").unwrap();
        s.replace_all(&fresh, Some(&entry(1)), 10).unwrap();
        let (r, _) = s.load().unwrap();
        assert!(r.group("old").is_none() && r.group("new").is_some() && r.group("default").is_some());
        assert_eq!(s.log_page(1, 10).unwrap().1, 1);
    }

    fn ledger(fingerprint: &str) -> Ledger {
        Ledger { fingerprint: fingerprint.into(), at: 1, ..Ledger::default() }
    }

    #[test]
    fn ledger_commits_with_import() {
        let s = PermsStore::in_memory().unwrap();
        let (mut p, _) = s.load().unwrap();
        let d = p.create_group("vip").unwrap();
        s.commit_import(&p, &d, Some(&entry(1)), 100, ("file", &ledger("f1"))).unwrap();
        assert_eq!(s.ledger("file").unwrap().fingerprint, "f1");
        assert!(s.load().unwrap().0.group("vip").is_some());
        assert_eq!(s.ledger("other").unwrap(), Ledger::default());
        // A failed import changes neither.
        let backend = pumbo_common::store::FaultyBackend::default();
        let writes = backend.fail_writes.clone();
        let s = PermsStore::new(Store::new(backend)).unwrap();
        writes.store(1, std::sync::atomic::Ordering::SeqCst);
        assert!(s.commit_import(&p, &d, None, 100, ("file", &ledger("f2"))).is_err());
        assert_eq!(s.ledger("file").unwrap(), Ledger::default());
        assert!(s.load().unwrap().0.group("vip").is_none());
        // An empty ledger removes the source.
        let s = PermsStore::in_memory().unwrap();
        s.commit_import(&p, &d, None, 100, ("file", &ledger("f1"))).unwrap();
        s.commit_import(&p, &Dirty::default(), None, 100, ("file", &Ledger::default())).unwrap();
        assert!(s.ledgers().unwrap().is_empty());
    }

    #[test]
    fn unreadable_ledger_blocks_auto_import() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(IMPORTS, "file", b"{broken")).unwrap();
        let s = PermsStore::new(store).unwrap();
        assert!(s.ledger("file").is_err(), "never an empty ledger, which would import everything again");
        let all = s.ledgers().unwrap();
        assert!(all.len() == 1 && all[0].1.is_err());
        let mut e = crate::engine::Engine::new(
            Ok(s),
            crate::config::CoreCfg::default(),
            pumbo_common::lang::Lang::load(&[pumbo_common::lang::COMMON, crate::LANG], "en", None).0,
        );
        let snap = crate::export::parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip"}]}"#).unwrap();
        let out = crate::command::import_source(
            &mut e,
            &crate::command::Who::console(),
            "file",
            Ok(("f".into(), snap)),
            crate::command::SourceMode::Run,
            1,
        );
        assert!(out.reply.plain().contains("ledger of file is damaged") && !out.changed, "{}", out.reply.plain());
        assert!(e.perms.group("vip").is_none());
    }

    #[test]
    fn newer_schema_is_refused() {
        let store = Store::in_memory();
        store.write(|tx| tx.put_u64(META, "schema", SCHEMA + 1)).unwrap();
        let err = PermsStore::new(store).unwrap_err();
        assert!(err.0.contains("newer than this version understands"), "{err}");
    }

    #[test]
    fn schema_1_migrates_to_2() {
        let dir = std::env::temp_dir().join(format!("pumbo-perms-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file.json"), serde_json::to_string(&ledger("f1")).unwrap()).unwrap();
        std::fs::write(dir.join("proxy.json"), "{broken").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a ledger").unwrap();
        let store = Store::in_memory();
        store.write(|tx| tx.put_u64(META, "schema", 1)).unwrap();
        let s = PermsStore::new(store).unwrap();
        assert_eq!(s.schema().unwrap(), 1);
        assert_eq!(s.migrate_ledgers(&dir).unwrap(), Some(vec!["file".to_string(), "proxy".to_string()]));
        assert_eq!(s.schema().unwrap(), 2);
        assert_eq!(s.ledger("file").unwrap().fingerprint, "f1");
        assert!(s.ledger("proxy").is_err(), "an unreadable file stays unreadable");
        assert!(dir.join("file.json.migrated").exists() && !dir.join("file.json").exists());
        assert!(dir.join("notes.txt").exists());
        // Once is enough.
        assert_eq!(s.migrate_ledgers(&dir).unwrap(), None);
        // A new database starts at schema 2 with nothing to move.
        assert_eq!(PermsStore::in_memory().unwrap().schema().unwrap(), SCHEMA);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redb_file_survives_reopening() {
        let path = std::env::temp_dir().join(format!("pumbo-perms-store-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let s = PermsStore::new(Store::open(&path).unwrap()).unwrap();
            let (mut p, mut d) = s.load().unwrap();
            d.merge(p.create_group("vip").unwrap());
            s.commit(&p, &d, Some(&entry(1)), 100).unwrap();
        }
        let s = PermsStore::new(Store::open(&path).unwrap()).unwrap();
        let (p, _) = s.load().unwrap();
        assert!(p.group("vip").is_some());
        drop(s);
        let _ = std::fs::remove_file(&path);
    }
}
