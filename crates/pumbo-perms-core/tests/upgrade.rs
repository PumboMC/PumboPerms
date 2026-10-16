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
    let here = Query::default();
    assert_eq!(p.check(MODERATOR, "pumbo.bans.ban", &here, 0), Some(true));
    assert_eq!(e.find_user("ppmod7x").map(|k| k.uuid), Some(MODERATOR.to_string()));

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

const STEVE: &str = "11111111-1111-4111-8111-111111111111";
const ALEX: &str = "22222222-2222-4222-8222-222222222222";
const NOTCH: &str = "33333333-3333-4333-8333-333333333333";
const RENAMED: &str = "44444444-4444-4444-8444-444444444444";
const FIRST: &str = "55555555-5555-4555-8555-555555555555";
const SECOND: &str = "66666666-6666-4666-8666-666666666666";

/// A database written by PumboPerms 1.0.0 (schema 2, names only in
/// `perms_names`) opens in this build. `fixtures/perms-1.0.0.redb` was filled
/// through `Engine::seen` and `/pp` as the console: joins of `Steve` (online
/// account), `Alex` (offline), `Notch` (online, no data), `OldName` and
/// `Shared` (offline); groups `vip` (weight 10, prefix `&6[VIP] `) and
/// `admin` (weight 100, `*`), track `ranks`; `Steve` in `vip` with meta
/// `color`, `Alex` with `fly`, `OldName` in `admin`, `Shared` in `vip`; then
/// `OldName` joined as `NewName`, another account took `Shared`, and an
/// import of `file` left `Pending1` waiting by name.
#[test]
fn opens_database_from_1_0_0() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-upgrade-1-0-0-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("perms.redb");
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/perms-1.0.0.redb"), &path).unwrap();

    let store = PermsStore::new(Store::open(&path).unwrap()).unwrap();
    assert_eq!(store.schema().unwrap(), SCHEMA);
    let mut e = Engine::new(Ok(store), CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
    assert_eq!(e.broken, None);
    assert_eq!(e.migrate_ledgers(&dir.join("imports")), None, "nothing to migrate");

    let p = &mut e.perms;
    let vip = p.group("vip").expect("vip");
    assert_eq!(vip.weight, 10);
    assert!(vip.data.meta.iter().any(|x| x.kind == MetaKind::Prefix && x.priority == 10 && x.value == "&6[VIP] "));
    assert!(p.group("admin").expect("admin").data.permissions.iter().any(|n| n.node == "*" && n.value));
    assert_eq!(p.track("ranks").expect("ranks").groups, vec!["vip".to_string(), "admin".to_string()]);
    let here = Query::default();
    assert_eq!(p.check(STEVE, "group.vip", &here, 0), Some(true));
    assert!(p.user(STEVE).unwrap().data.meta.iter().any(|x| x.key == "color" && x.value == "gold"));
    assert_eq!(p.check(ALEX, "fly", &here, 0), Some(true));
    assert_eq!(p.check(RENAMED, "anything", &here, 0), Some(true));
    assert_eq!(p.user(RENAMED).unwrap().name, "OldName", "the record keeps the name of its last change");
    assert_eq!(p.check(FIRST, "group.vip", &here, 0), Some(true));

    let by_name = |e: &Engine, name: &str| e.find_user(name).map(|k| (k.uuid, k.online));
    assert_eq!(by_name(&e, "steve"), Some((STEVE.to_string(), true)));
    assert_eq!(by_name(&e, "ALEX"), Some((ALEX.to_string(), false)));
    assert_eq!(by_name(&e, "Notch"), Some((NOTCH.to_string(), true)));
    assert_eq!(by_name(&e, "newname"), Some((RENAMED.to_string(), true)));
    assert_eq!(by_name(&e, "oldname"), None);
    assert_eq!(by_name(&e, "shared"), Some((SECOND.to_string(), true)));
    // By UUID: the name of the record; without one the UUID until the next join.
    let by_uuid = |e: &Engine, uuid: &str| e.find_user(uuid).map(|k| k.name);
    assert_eq!(by_uuid(&e, STEVE), Some("Steve".to_string()));
    assert_eq!(by_uuid(&e, NOTCH), Some(NOTCH.to_string()));
    // Joins write the name of each account; the 1.0.0 record of an old name stops counting.
    e.seen(NOTCH, "Notch2", true).unwrap();
    e.gone(NOTCH);
    assert_eq!(by_name(&e, "notch"), None);
    assert_eq!(by_name(&e, "notch2"), Some((NOTCH.to_string(), true)));
    assert_eq!(by_uuid(&e, NOTCH), Some("Notch2".to_string()));
    let moved = e.seen(FIRST, "Shared", false).unwrap();
    assert_eq!(moved, None, "{SECOND} has no data");
    assert_eq!(by_name(&e, "shared"), Some((FIRST.to_string(), false)));

    assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
    let (log, total) = e.store().unwrap().log_page(1, 20).unwrap();
    assert_eq!(total, 15, "{log:?}");
    assert_eq!(log[1].action, "parent add vip");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}
