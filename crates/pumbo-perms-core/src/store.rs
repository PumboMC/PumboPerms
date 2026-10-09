//! Saving the database through [`pumbo_common::store`].
//!
//! Tables (all values JSON unless noted):
//! - `perms_groups`: group name -> [`Group`]
//! - `perms_users`: UUID -> [`User`] (only users with data)
//! - `perms_names`: nickname key -> [`KnownName`] (everyone who joined)
//! - `perms_tracks`: track name -> [`Track`]
//! - `perms_log`: 20-digit sequence number -> [`LogEntry`]
//! - `perms_host_nodes`: nodes the platform ever handed to its host (value empty)
//! - `perms_meta`: `schema`, `log_seq` (numbers)
//!
//! The whole database is read once at start; every change writes what it
//! touched in one transaction together with its log entry.

use std::collections::BTreeSet;

use pumbo_common::store::{Durability, ReadExt, Result, Store, StoreError, WriteExt, WriteOps};
use serde::{Deserialize, Serialize};

use crate::model::{Group, Track, User};
use crate::perms::{Dirty, KnownName, Perms};

pub const GROUPS: &str = "perms_groups";
pub const USERS: &str = "perms_users";
pub const NAMES: &str = "perms_names";
pub const TRACKS: &str = "perms_tracks";
pub const LOG: &str = "perms_log";
pub const HOST_NODES: &str = "perms_host_nodes";
pub const META: &str = "perms_meta";

/// Version of the stored data.
pub const SCHEMA: u64 = 1;

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
                names.push(KnownName { uuid: n.uuid, name: n.name });
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
                &StoredName { uuid: name.uuid.clone(), name: name.name.clone() },
            )
        })
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
        p.seen(U, "Steve");
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
    fn damaged_records_and_newer_schema_are_errors() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(GROUPS, "vip", b"{broken")).unwrap();
        let s = PermsStore::new(store).unwrap();
        assert!(s.load().is_err());

        let store = Store::in_memory();
        store.write(|tx| tx.put_u64(META, "schema", SCHEMA + 1)).unwrap();
        assert!(PermsStore::new(store).is_err());
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
