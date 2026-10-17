//! Names of joined players: in memory while they are online, in the
//! database for everyone else.

use super::*;
use crate::command::{self, Env, SourceMode, Who, import_source, split_args};

const A: &str = "00000000-0000-0000-0000-0000000000a1";
const B: &str = "00000000-0000-0000-0000-0000000000b2";
const C: &str = "00000000-0000-0000-0000-0000000000c3";

fn engine() -> Engine {
    Engine::in_memory(CoreCfg::default(), Lang::load(&[COMMON, crate::LANG], "en", None).0)
}

/// The platform has nobody online: every name comes from the engine.
fn nobody(_: &str) -> Option<KnownName> {
    None
}

fn console(e: &mut Engine, line: &str) -> String {
    let env = Env { now: 1_000, online: &nobody, version: "0", version_rows: Vec::new() };
    command::run(e, &Who::console(), &split_args(line), &env).reply.plain()
}

fn ask(e: &mut Engine, request: &str) -> serde_json::Value {
    serde_json::from_slice(&crate::ipc::handle(e, request.as_bytes(), 2_000, "0")).unwrap()
}

fn in_group(e: &Engine, uuid: &str, group: &str) -> bool {
    e.perms.user(uuid).is_some_and(|u| u.data.parents.iter().any(|p| p.group == group))
}

fn uuid(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

#[test]
fn offline_answers_stay_current() {
    let mut e = engine();
    e.seen(A, "Steve", true).unwrap();
    e.gone(A);
    console(&mut e, "creategroup vip");
    console(&mut e, "group vip meta setprefix 10 \"[V] \"");
    assert!(console(&mut e, "user Steve parent add vip").contains("now inherits vip"));
    let check = format!(r#"{{"op":"check","uuid":"{A}","node":"group.vip"}}"#);
    let info = format!(r#"{{"op":"info","uuid":"{A}"}}"#);
    assert_eq!(ask(&mut e, &check)["value"], true);
    let got = ask(&mut e, &info);
    assert_eq!(
        (got["name"].as_str(), got["rank"].as_str(), got["prefix"].as_str()),
        (Some("Steve"), Some("vip"), Some("[V] "))
    );
    assert!(console(&mut e, "user Steve info").contains("vip"));
    console(&mut e, "user Steve parent remove vip");
    assert!(ask(&mut e, &check)["value"].is_null());
    assert_eq!(ask(&mut e, &info)["rank"], "default");
    assert!(!console(&mut e, "user Steve info").contains("vip"));
}

#[test]
fn stale_record_name_does_not_shadow_the_new_owner() {
    let mut e = engine();
    e.seen(A, "Steve", false).unwrap();
    console(&mut e, "creategroup vip");
    console(&mut e, "creategroup admin");
    console(&mut e, "user Steve parent add vip");
    e.gone(A);
    e.seen(B, "Steve", false).unwrap();
    e.gone(B);
    assert_eq!(e.perms.user(A).unwrap().name, "Steve", "the record of the old owner keeps the name");
    assert!(console(&mut e, "user Steve parent add admin").contains(B));
    assert!(in_group(&e, B, "admin") && !in_group(&e, A, "admin"));
    assert!(in_group(&e, A, "vip"));
}

#[test]
fn import_binds_only_names_seen_with_online_accounts() {
    let mut e = engine();
    // An online account that left, an offline one and an online one still here.
    e.seen(A, "Mod1", true).unwrap();
    e.gone(A);
    e.seen(B, "Mod2", false).unwrap();
    e.seen(C, "Mod3", true).unwrap();
    let source = || {
        crate::export::parse(
            r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],"by-name":[
                {"name":"Mod1","data":{"parents":[{"group":"mod"}]}},
                {"name":"Mod2","data":{"parents":[{"group":"mod"}]}},
                {"name":"Mod3","data":{"parents":[{"group":"mod"}]}},
                {"name":"Mod4","data":{"parents":[{"group":"mod"}]}}]}"#,
        )
        .unwrap()
    };
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), source())), SourceMode::Run, 1);
    let reply = out.reply.plain();
    assert!(reply.contains(&format!("Mod1 -> {A}")) && reply.contains(&format!("Mod3 -> {C}")), "{reply}");
    assert!(in_group(&e, A, "mod") && in_group(&e, C, "mod") && !in_group(&e, B, "mod"));
    let pending = |e: &Engine| e.ledger("file").unwrap().pending.into_iter().map(|u| u.name).collect::<Vec<_>>();
    assert_eq!(pending(&e), ["Mod2", "Mod4"]);
    // The offline account only in the database: still not bound.
    e.gone(B);
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), source())), SourceMode::Run, 2);
    assert!(!out.reply.plain().contains(B), "{}", out.reply.plain());
    assert_eq!(pending(&e), ["Mod2", "Mod4"]);
}

#[test]
fn rejoin_under_a_new_name_updates_views() {
    let mut e = engine();
    e.seen(A, "Steve", false).unwrap();
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve parent add vip");
    e.gone(A);
    e.seen(A, "Alex", false).unwrap();
    let info = format!(r#"{{"op":"info","uuid":"{A}"}}"#);
    for online in [true, false] {
        let members = console(&mut e, "group vip listmembers");
        assert!(members.contains("Alex") && !members.contains("Steve"), "{members}");
        assert_eq!(ask(&mut e, &info)["name"], "Alex");
        if online {
            e.gone(A);
        }
    }
}

#[test]
fn gone_keeps_the_other_account_of_the_same_name() {
    let mut e = engine();
    e.seen(A, "Steve", false).unwrap();
    e.seen(B, "Steve", false).unwrap();
    e.gone(A);
    assert_eq!(e.connected(B).map(|k| k.name.as_str()), Some("Steve"));
    assert_eq!(e.find_user("steve").map(|k| k.uuid), Some(B.to_string()));
    assert_eq!(e.find_user(B).map(|k| k.name), Some("Steve".to_string()));
    assert_eq!(e.find_user(A).map(|k| k.name), Some(A.to_string()), "the name went to the other account");
    e.gone(B);
    assert_eq!(e.find_user("steve").map(|k| k.uuid), Some(B.to_string()));
}

#[test]
fn seen_keeps_only_online_names_in_memory() {
    let mut e = engine();
    for i in 0..300 {
        e.seen(&uuid(i), &format!("Player{i}"), false).unwrap();
        if i != 7 {
            e.gone(&uuid(i));
        }
    }
    assert_eq!(e.connected.len(), 1);
    assert_eq!(e.connected(&uuid(7)).map(|k| k.name.as_str()), Some("Player7"));
    assert_eq!(e.find_user("player250").map(|k| k.uuid), Some(uuid(250)));
    assert_eq!(e.find_user(&uuid(250)).map(|k| k.name), Some("Player250".to_string()));
    assert!(e.perms.users().next().is_none(), "joins make no user records");
}

#[test]
fn seen_warns_when_name_leaves_uuid_with_data() {
    let mut e = engine();
    assert_eq!(e.seen(A, "Steve", true), Ok(None));
    // Without data nothing is left behind.
    assert_eq!(e.seen(B, "Steve", false), Ok(None));
    assert_eq!(e.seen(A, "Steve", true), Ok(None));
    let (_, d) = e.perms.edit_user(A, "Steve", |d| {
        d.set_permission(PermNode { node: "fly".into(), value: true, context: Contexts::global(), expiry: None })
    });
    e.commit(&d, None).unwrap();
    let line = e.seen(B, "Steve", false).unwrap().unwrap();
    assert!(line.contains(&format!("moved from {A} to {B}")), "{line}");
    assert_eq!(e.find_user("steve").map(|k| (k.uuid, k.online)), Some((B.to_string(), false)));
    // The same account again: no warning.
    assert_eq!(e.seen(B, "Steve", false), Ok(None));
}

/// The name's previous account comes from the database: also after a
/// restart, when nobody of the two is online.
#[test]
fn name_moving_to_another_account_warns() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-name-moves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let open = || {
        let store = PermsStore::new(pumbo_common::store::Store::open(dir.join("perms.redb")).unwrap());
        Engine::new(store, CoreCfg::default(), Lang::load(&[COMMON, crate::LANG], "en", None).0)
    };
    let mut e = open();
    e.seen(A, "Steve", true).unwrap();
    console(&mut e, "user Steve permission set fly");
    drop(e);
    let mut e = open();
    let line = e.seen(B, "Steve", false).unwrap().unwrap();
    assert!(line.contains(&format!("the name Steve moved from {A} to {B}")), "{line}");
    assert_eq!(e.seen(B, "Steve", false), Ok(None), "the same account again");
    assert_eq!(e.seen(C, "Steve", false), Ok(None), "{B} has no data");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renamed_player_old_name_does_not_resolve() {
    let mut e = engine();
    e.seen(A, "Steve", false).unwrap();
    e.gone(A);
    e.seen(A, "Steve2", false).unwrap();
    e.gone(A);
    assert!(e.find_user("steve").is_none());
    assert!(console(&mut e, "user Steve info").contains("not found"));
    assert_eq!(e.find_user("steve2").map(|k| k.uuid), Some(A.to_string()));
    assert_eq!(e.find_user(A).map(|k| k.name), Some("Steve2".to_string()));
}
