//! The engine: the operator fallback, a broken store, ledgers and expiry.

mod names;

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
    assert_eq!(e.decide(U, "pumbo.perms.reload", "world", 4, 0), Some(false), "an explicit false beats the fallback");
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
    e.seen(U, "Steve", true, 0).unwrap();
    assert_eq!(e.find_user(U).map(|k| k.name), Some("Steve".into()), "known while online");
    e.broken = None;
    e.reload_data();
    assert!(e.find_user("Steve").is_none(), "not in the database");
    e.gone(U);
    assert_eq!(e.find_user(U).map(|k| k.name), Some(U.into()));
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

/// A join with 400 000 known names: a few point reads and a point write.
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
    assert_eq!(e.find_user("player399999").map(|k| k.uuid), Some("00000000-0000-4000-8000-000000399999".into()));
    let mut worst = std::time::Duration::ZERO;
    for i in 0..200u32 {
        // New players, and known ones under a new name.
        let (uuid, name) = match i % 2 {
            0 => (format!("00000000-0000-4000-9000-{i:012}"), format!("Newcomer{i}")),
            _ => (format!("00000000-0000-4000-8000-{i:012}"), format!("Renamed{i}")),
        };
        let start = std::time::Instant::now();
        e.seen(&uuid, &name, true, 0).unwrap();
        worst = worst.max(start.elapsed());
    }
    e.reload_data();
    assert!(e.find_user("Player1").is_none() && e.find_user("renamed1").is_some());
    assert!(e.find_user("Player0").is_some() && e.find_user("newcomer0").is_some());
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
