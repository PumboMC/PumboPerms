//! `permissions.yml` and other imports, exports and the file and export calls.

use super::*;

/// The proxy's permissions.yml comes in once at start: groups, contexts,
/// players by UUID now and by nickname at their first join; the same file
/// again does nothing, a changed one only gives a hint (`follow: false`).
#[test]
fn takes_over_permissions_yml() {
    let (p, _) = plugin(None);
    let alice = testing::add_player("Alice", Some("lobby"));
    let alice_uuid = uuid_of(&players::get(alice).unwrap());
    let data = format!(
        r#"{{"format":"pumboperms","version":1,
        "groups":[{{"name":"vip","data":{{"permissions":[{{"node":"minecraft:command.gamemode","value":true,"context":{{"server":"lobby"}}}}],
                     "parents":[{{"group":"default"}}]}}}},
                  {{"name":"default","data":{{"permissions":[{{"node":"pumbo.proxy.server","value":true}}]}}}}],
        "users":[{{"uuid":"{alice_uuid}","name":"Alice","data":{{"parents":[{{"group":"vip"}}]}}}}],
        "by-name":[{{"name":"Jeb_","data":{{"parents":[{{"group":"vip"}}]}}}}]}}"#
    );
    call(&p, METHOD_FILE, file(&data)).unwrap();
    assert!(logs().contains("permissions.yml: [PP] Imported file"), "{}", logs());
    assert!(logs().contains("Jeb_"), "pending player named in the log: {}", logs());
    let set = load(&p, alice);
    assert!(has(&set, "minecraft:command.gamemode", Context::Server("lobby".into()), true), "{set:?}");
    assert!(
        !set.iter().any(|e| e.node == "minecraft:command.gamemode" && e.context != Context::Server("lobby".into()))
    );
    assert!(has(&set, "pumbo.proxy.server", Context::Global, true));
    assert!(has(&set, "group.vip", Context::Global, true));
    let ledger = || p.with(|s| s.engine.ledger(SOURCE).unwrap()).unwrap();
    assert_eq!(ledger().pending[0].name, "Jeb_");

    // Jeb_ joins for the first time with an online-mode account: the entries
    // waiting for the name.
    let jeb = testing::add_player("jeb_", Some("survival"));
    testing::with(|h| h.players.get_mut(&jeb).unwrap().online_mode = true);
    let set = load(&p, jeb);
    assert!(has(&set, "group.vip", Context::Global, true), "{set:?}");
    assert!(logs().contains("joined for the first time"), "{}", logs());
    assert!(ledger().pending.is_empty());

    // The same file at the next start: nothing; a changed one: a hint only.
    let before = logs().len();
    call(&p, METHOD_FILE, file(&data)).unwrap();
    assert_eq!(logs().len(), before);
    call(&p, METHOD_FILE, file(&data.replace("pumbo.proxy.server", "pumbo.proxy.glist"))).unwrap();
    assert!(logs().contains("changed since it was imported"), "{}", logs());
    // By hand it goes in, and undo takes back exactly the import.
    console(&p, "import run file");
    assert!(logs().contains("Imported file"), "{}", logs());
    console(&p, "import undo file");
    let set = last_replace(alice).unwrap();
    assert!(!has(&set, "group.vip", Context::Global, true), "undo took Alice out of vip: {set:?}");
}

const EVERYTHING: &str = r#"{"format":"pumboperms","version":1,
    "groups":[{"name":"default","data":{"permissions":[{"node":"*","value":true}]}}]}"#;

#[test]
fn file_method_rejects_other_plugins() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    let got = call_as(&p, "evil-plugin", METHOD_FILE, file(EVERYTHING));
    assert!(matches!(got, Err(CallReject::Rejected(_))), "{got:?}");
    assert!(!has(&load(&p, steve), "*", Context::Global, true));
    call_as(&p, "proxy", METHOD_FILE, file(EVERYTHING)).unwrap();
    assert!(has(&load(&p, steve), "*", Context::Global, true), "the host may hand over permissions.yml");
}

#[test]
fn export_method_rejects_other_plugins() {
    let (p, _) = plugin(None);
    let got = call_as(&p, "evil-plugin", METHOD_EXPORT, cbor(&contracts_request()).unwrap());
    assert!(matches!(got, Err(CallReject::Rejected(_))), "{got:?}");
    call_as(&p, "proxy", METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
}

#[test]
fn pending_import_skips_offline_accounts() {
    let (p, _) = plugin(None);
    let data = r#"{"format":"pumboperms","version":1,
        "groups":[{"name":"mod","data":{"permissions":[{"node":"pumbo.bans.ban","value":true}]}}],
        "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#;
    call_as(&p, "proxy", METHOD_FILE, file(data)).unwrap();
    let offline = testing::add_player("Mod1", Some("lobby"));
    assert!(!has(&load(&p, offline), "group.mod", Context::Global, true), "an offline account took the rank");
    let online = testing::add_player("Mod1", Some("lobby"));
    testing::with(|h| h.players.get_mut(&online).unwrap().online_mode = true);
    assert!(has(&load(&p, online), "group.mod", Context::Global, true));
}

/// A bad group name in permissions.yml comes in under a valid one and a bad
/// player name is left out; the rest is imported and the log names both whole.
#[test]
fn bad_names_in_permissions_yml_are_fixed_or_left_out() {
    const BAD_GROUP: &str = "zła grupa!";
    const BAD_PLAYER: &str = "ŻółwiaGęśŚliczna_bardzo_długa_nazwa_z_ogonkami";
    let (p, _) = plugin(None);
    let alice = testing::add_player("Alice", Some("lobby"));
    let data = serde_json::json!({
        "format": "pumboperms", "version": 1,
        "groups": [{"name": BAD_GROUP, "data": {"permissions": [{"node": "fly", "value": true}]}},
                   {"name": "vip", "data": {"permissions": [{"node": "minecraft:command.gamemode", "value": true}]}}],
        "users": [{"uuid": uuid_of(&players::get(alice).unwrap()), "name": "Alice",
                   "data": {"parents": [{"group": "vip"}, {"group": BAD_GROUP}]}}],
        "by-name": [{"name": BAD_PLAYER, "data": {"parents": [{"group": "vip"}]}}],
    });
    call(&p, METHOD_FILE, file(&data.to_string())).unwrap();
    let log = logs();
    assert!(
        !log.contains("cannot be imported") && log.contains("2 entries of the file were fixed or left out"),
        "{log}"
    );
    assert!(
        log.contains(&format!("{BAD_GROUP:?} imported as zla-grupa"))
            && log.contains(&format!("{BAD_PLAYER:?} left out")),
        "{log}"
    );
    let alice = load(&p, alice);
    assert!(has(&alice, "group.vip", Context::Global, true) && has(&alice, "group.zla-grupa", Context::Global, true));
    assert!(has(&alice, "fly", Context::Global, true));
}

/// A file the plugin cannot read: preview and run give the reason the log gives.
#[test]
fn rejected_permissions_yml_is_named_in_import_replies() {
    let (p, _) = plugin(None);
    call(&p, METHOD_FILE, file(r#"{"format":"pumboperms","version":99}"#)).unwrap();
    let why = "export version 99 is newer than this version understands (1)";
    assert!(logs().contains(&format!("permissions.yml cannot be imported: {why}")), "{}", logs());
    for mode in ["preview", "run"] {
        testing::with(|h| h.logs.clear());
        console(&p, &format!("import {mode} file"));
        assert!(
            logs().contains(&format!("Cannot import file: permissions.yml has errors: {why}")),
            "{mode}: {}",
            logs()
        );
    }
}

#[test]
fn export_answer_fits_the_host_payload_limit() {
    let (p, _) = plugin(None);
    console(&p, "creategroup vip");
    p.with(|s| {
        let mut dirty = pumbo_perms_core::perms::Dirty::default();
        for i in 0..3000u64 {
            let uuid = Uuid::from_high_low(1, i).to_string();
            let parent = pumbo_perms_core::model::ParentNode {
                group: "vip".into(),
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            let (_, d) = s.engine.perms.edit_user(&uuid, &format!("Player{i}"), |d| d.add_parent(parent));
            dirty.merge(d);
        }
        s.engine.commit(&dirty, None).unwrap();
    })
    .unwrap();
    let out = call_as(&p, "proxy", METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
    // Only the flag, whatever the database holds (the host takes up to 256 KiB).
    assert!(out.len() < 1024, "{} bytes", out.len());
}

#[test]
fn export_on_broken_database_keeps_existing_file() {
    testing::reset();
    let dir = std::env::temp_dir().join(format!("pumbo-perms-prox-broken-export-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("exports")).unwrap();
    let path = dir.join("exports/nightly.json");
    std::fs::write(&path, "precious").unwrap();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Err(pumbo_common::store::StoreError::new("disk on fire"))).unwrap();
    console(&p, "export nightly");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "precious");
    console(&p, "export fresh");
    assert!(!dir.join("exports/fresh.json").exists(), "no empty export of a broken database");
}

/// An offline account that joined before the import does not get the rank of
/// its nickname: the name table counts only accounts Mojang checked.
#[test]
fn offline_join_then_import_run_gets_nothing() {
    let (p, _) = plugin(Some("import:\n  enabled: false\n"));
    let offline = testing::add_player("Mod1", Some("lobby"));
    load(&p, offline);
    let data = r#"{"format":"pumboperms","version":1,
        "groups":[{"name":"mod","data":{"permissions":[{"node":"pumbo.bans.ban","value":true}]}}],
        "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#;
    call(&p, METHOD_FILE, file(data)).unwrap();
    console(&p, "import run file");
    assert!(logs().contains("players are known only by name") && logs().contains("Mod1"), "{}", logs());
    assert!(!has(&load(&p, offline), "group.mod", Context::Global, true), "an offline account took the rank");
    assert_eq!(p.with(|s| s.engine.ledger(SOURCE).unwrap().pending.len()).unwrap(), 1);
}

/// The ledgers of a database from before schema 2 move into it at start.
#[test]
fn ledger_file_moves_into_the_database() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-prox-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("imports")).unwrap();
    std::fs::write(dir.join("imports/file.json"), r#"{"fingerprint":"f1","at":7,"entries":[]}"#).unwrap();
    let store = pumbo_common::store::Store::open(dir.join("perms.redb")).unwrap();
    store.write(|tx| pumbo_common::store::WriteExt::put_u64(tx, pumbo_perms_core::store::META, "schema", 1)).unwrap();
    testing::reset();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(PermsStore::new(store)).unwrap();
    assert!(logs().contains("schema 1 -> 2"), "{}", logs());
    assert_eq!(p.with(|s| s.engine.ledger(SOURCE).unwrap().fingerprint).unwrap(), "f1");
    assert!(dir.join("imports/file.json.migrated").exists());
}
