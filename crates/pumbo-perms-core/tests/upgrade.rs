//! A database written by the released PumboPerms 0.1.2-beta opens in this
//! build. `fixtures/perms-0.1.2-beta.redb` comes from
//! `PumboPerms-Pumpkin-26.3-0.1.2-beta.wasm` (SHA-256 eb38421a…392a) on
//! Pumpkin 0.2.0, filled by step J8 of `tools/perms-live.sh`: groups `helper`
//! and `mod` (weight 20, `pumbo.bans.*`, parent `helper`, prefix `&c[Mod]`),
//! track `staff`, and `PpMod7x` in `mod`, in `helper` for 30 days and with
//! meta `color`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use pumbo_common::lang::{COMMON, Lang};
use pumbo_common::store::Store;
use pumbo_perms_core::config::CoreCfg;
use pumbo_perms_core::context::Query;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::model::MetaKind;
use pumbo_perms_core::store::{PermsStore, SCHEMA};

/// `PpMod7x` on a lone Pumpkin (offline UUID from SHA-256).
const MODERATOR: &str = "fef4de34-7728-2a0f-199b-48e23053c1bd";

#[test]
fn opens_database_from_0_1_2_beta() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-upgrade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("perms.redb");
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/perms-0.1.2-beta.redb"), &path).unwrap();

    let store = PermsStore::new(Store::open(&path).unwrap()).unwrap();
    assert_eq!(store.schema().unwrap(), 1);
    let mut e = Engine::new(Ok(store), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
    assert_eq!(e.broken, None);
    let line = e.migrate_ledgers(&dir.join("imports")).unwrap();
    assert!(line.contains("schema 1 -> 2"), "{line}");
    assert_eq!(e.store().unwrap().schema().unwrap(), SCHEMA);

    let p = &mut e.perms;
    let helper = p.group("helper").expect("helper");
    assert!(helper.data.is_empty());
    let m = p.group("mod").expect("mod");
    assert_eq!(m.weight, 20);
    assert!(m.data.permissions.iter().any(|n| n.node == "pumbo.bans.*" && n.value));
    assert!(m.data.parents.iter().any(|x| x.group == "helper"));
    assert!(m.data.meta.iter().any(|x| x.kind == MetaKind::Prefix && x.priority == 50 && x.value == "&c[Mod]"));
    assert_eq!(p.track("staff").expect("staff").groups, vec!["helper".to_string(), "mod".to_string()]);

    let user = p.user(MODERATOR).expect("PpMod7x");
    assert_eq!(user.name, "PpMod7x");
    let parents: Vec<(&str, bool)> = user.data.parents.iter().map(|x| (x.group.as_str(), x.expiry.is_some())).collect();
    assert!(parents.contains(&("mod", false)) && parents.contains(&("helper", true)), "{parents:?}");
    assert!(user.data.meta.iter().any(|x| x.key == "color" && x.value == "red"));
    assert_eq!(p.find_user("ppmod7x").map(|k| k.uuid), Some(MODERATOR.to_string()));
    let here = Query::default();
    assert_eq!(p.check(MODERATOR, "pumbo.bans.ban", &here, 0), Some(true));

    let (log, total) = e.store().unwrap().log_page(1, 20).unwrap();
    assert_eq!(total, 12, "{log:?}");
    assert_eq!(log[0].action, "meta set color red");
    assert_eq!(log[11].action, "create");

    // Changes and import ledgers work on the migrated file, and it opens again.
    let (_, d) = e.perms.create_group("vip").map(|d| ((), d)).unwrap();
    e.commit(&d, None).unwrap();
    assert_eq!(e.ledger("file").unwrap(), Default::default());
    drop(e);
    let again = PermsStore::new(Store::open(&path).unwrap()).unwrap();
    assert_eq!(again.schema().unwrap(), SCHEMA);
    assert!(again.load().unwrap().0.group("vip").is_some());
    drop(again);
    let _ = std::fs::remove_dir_all(&dir);
}
