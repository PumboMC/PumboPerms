//! The store: writes, the log, damaged records, schemas and the redb file.

mod names;

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
            g.data.set_permission(PermNode { node: "a".into(), value: true, context: Contexts::global(), expiry: None })
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
    let steve = KnownName { uuid: U.into(), name: "Steve".into(), online: true };
    assert_eq!(s.save_name(&steve, &mut Vec::new()).unwrap(), None, "a new name");

    let (mut q, dirty) = s.load().unwrap();
    assert!(dirty.is_empty());
    assert_eq!(q.check(U, "a", &Query::default(), 0), Some(true));
    assert_eq!(s.find_name("STEVE", &mut Vec::new()).unwrap(), Some(steve.clone()));
    assert_eq!(s.name_of(U, &mut Vec::new()).unwrap(), Some(steve.clone()));
    assert!(q.track("ranks").is_some());

    // users without data are removed; deleting a group removes its record
    let mut d = q.edit_user(U, "Steve", |u| u.parents.clear()).1;
    d.merge(q.delete_group("vip").unwrap());
    s.commit(&q, &d, None, 100).unwrap();
    let (r, _) = s.load().unwrap();
    assert!(r.user(U).is_none() && r.group("vip").is_none());
    assert_eq!(s.find_name("steve", &mut Vec::new()).unwrap(), Some(steve), "the name stays");
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
    s.remove_host_nodes(&["a".to_string()]).unwrap();
    assert_eq!(s.host_nodes().unwrap(), BTreeSet::from(["minecraft:command.op".to_string()]));

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

mod log;
