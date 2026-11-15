//! Commands, imports and questions about players whose records stay on disk.

use super::*;
use crate::model::{Data, Group, ParentNode, PermNode, User};
use crate::perms::Perms;

/// An engine that reads players' records when needed, with nobody online.
fn lazy() -> Engine {
    Engine::lazy(CoreCfg::default(), Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0)
}

fn player(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

fn parent(group: &str, expiry: Option<u64>) -> ParentNode {
    ParentNode { group: group.into(), context: crate::context::Contexts::global(), expiry }
}

fn node(node: &str, expiry: Option<u64>) -> PermNode {
    PermNode { node: node.into(), value: true, context: crate::context::Contexts::global(), expiry }
}

/// Player `i` in `vip`, under the name `Player<i>`.
fn in_vip(i: u32) -> User {
    User {
        uuid: player(i),
        name: format!("Player{i}"),
        data: Data { parents: vec![parent("vip", None)], ..Data::default() },
    }
}

/// Writes `users` and the groups `vip` and `mod` to the database as an earlier
/// start left them, and reads it again.
fn on_disk(e: &mut Engine, users: Vec<User>) {
    let groups = vec![Group::new("default"), Group::new("vip"), Group::new("mod")];
    let (p, mut d) = Perms::from_parts(groups, users, Vec::new());
    d.groups.extend(p.groups().map(|g| g.name.clone()));
    d.users.extend(p.users().map(|u| u.uuid.clone()));
    e.store().unwrap().commit(&p, &d, None, 0).unwrap();
    e.reload_data();
}

/// Reads so many other records that the ones read before go back to disk.
fn forget(e: &mut Engine) {
    for i in 0..100 {
        e.load_user(&player(100_000 + i), 1_000);
    }
}

fn ask(e: &mut Engine, request: &str) -> serde_json::Value {
    serde_json::from_slice(&crate::ipc::handle(e, request.as_bytes(), 1_000, "0")).unwrap()
}

#[test]
fn offline_answers_stay_current() {
    let mut e = lazy();
    // Not online: the platform does not know the name.
    let steve = player(1);
    on_disk(&mut e, vec![User { uuid: steve.clone(), name: "Herobrine".into(), data: Data::default() }]);
    e.seen(&steve, "Herobrine", true, 1_000).unwrap();
    e.gone(&steve);
    console(&mut e, "group vip meta setprefix 10 \"[V] \"");
    forget(&mut e);
    assert!(!e.perms.knows(&steve));
    assert!(console(&mut e, "user Herobrine parent add vip").contains("now inherits vip"));
    forget(&mut e);
    let check = format!(r#"{{"op":"check","uuid":"{steve}","node":"group.vip"}}"#);
    let info = format!(r#"{{"op":"info","uuid":"{steve}"}}"#);
    assert_eq!(ask(&mut e, &check)["value"], true);
    forget(&mut e);
    let got = ask(&mut e, &info);
    assert_eq!(
        (got["name"].as_str(), got["rank"].as_str(), got["prefix"].as_str()),
        (Some("Herobrine"), Some("vip"), Some("[V] "))
    );
    forget(&mut e);
    assert!(crate::host::decisions(&mut e, &steve, "", 1_000).contains(&("group.vip".to_string(), true)));
    forget(&mut e);
    let catalog = crate::host::Catalog::new(["group.vip"]);
    assert_eq!(crate::host::wanted(&mut e, &steve, "", 0, 1_000, &catalog).get("group.vip"), Some(&true));
    forget(&mut e);
    assert!(console(&mut e, "user Herobrine info").contains("vip"));
    forget(&mut e);
    console(&mut e, "user Herobrine parent remove vip");
    forget(&mut e);
    assert!(ask(&mut e, &check)["value"].is_null());
    forget(&mut e);
    assert_eq!(ask(&mut e, &info)["rank"], "default");
}

#[test]
fn temporary_entries_of_offline_players_do_not_decide() {
    let mut e = lazy();
    let mut alex = User { uuid: player(2), name: "Alex".into(), data: Data::default() };
    alex.data.parents.push(parent("vip", Some(500)));
    alex.data.permissions.push(node("fly", Some(500)));
    on_disk(&mut e, vec![alex]);
    let uuid = player(2);
    // The entries ended while the player was not online; the disk still has them.
    let info = ask(&mut e, &format!(r#"{{"op":"info","uuid":"{uuid}"}}"#));
    assert_eq!((info["name"].as_str(), info["rank"].as_str()), (Some("Alex"), Some("default")));
    forget(&mut e);
    assert_eq!(e.decide(&uuid, "group.vip", "", 0, 1_000), None);
    assert_eq!(e.decide(&uuid, "fly", "", 0, 1_000), None);
    assert_eq!(e.perms.member_count("vip", 1_000), 0);
    assert!(console(&mut e, "group vip info").contains("Members: 0"), "{}", console(&mut e, "group vip info"));
}

#[test]
fn expired_entries_of_a_loaded_record_do_not_count() {
    let mut e = lazy();
    let uuid = player(3);
    let mut notch = User { uuid: uuid.clone(), name: "Notch".into(), data: Data::default() };
    notch.data.parents.push(parent("vip", Some(500)));
    notch.data.permissions.push(node("keep", None));
    on_disk(&mut e, vec![notch]);
    let parents = console(&mut e, &format!("user {uuid} parent info"));
    assert!(!parents.contains("vip"), "{parents}");
    let removed = console(&mut e, &format!("user {uuid} parent remove vip"));
    assert!(removed.contains("does not inherit vip"), "{removed}");
    forget(&mut e);
    // An import gives the group although the record on disk had it until 500.
    let source = export::parse(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip"}}],
          "users":[{{"uuid":"{uuid}","name":"Notch","data":{{"parents":[{{"group":"vip"}}]}}}}]}}"#
    ))
    .unwrap();
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), source)), SourceMode::Run, 1_000);
    assert!(out.changed && out.reply.plain().contains("1 entries added"), "{}", out.reply.plain());
    let stored = e.store().unwrap().user(&uuid).unwrap().unwrap();
    assert_eq!(stored.data.parents, vec![parent("vip", None)]);
    assert_eq!(stored.data.permissions, vec![node("keep", None)]);
}

#[test]
fn import_of_many_offline_players_keeps_their_data() {
    let mut e = lazy();
    let users: Vec<User> = (0..200)
        .map(|i| User {
            uuid: player(i),
            name: format!("Player{i}"),
            data: Data { permissions: vec![node(&format!("plots.plot.{i}"), None)], ..Data::default() },
        })
        .collect();
    on_disk(&mut e, users);
    let source = |group: &str| {
        let users: Vec<String> = (0..200)
            .map(|i| {
                format!(r#"{{"uuid":"{}","name":"Player{i}","data":{{"parents":[{{"group":"{group}"}}]}}}}"#, player(i))
            })
            .collect();
        export::parse(&format!(
            r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"{group}"}}],"users":[{}]}}"#,
            users.join(",")
        ))
        .unwrap()
    };
    let on_disk_now = |e: &Engine, i: u32| e.store().unwrap().user(&player(i)).unwrap().unwrap().data;
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), source("mod"))), SourceMode::Run, 1_000);
    assert!(out.changed, "{}", out.reply.plain());
    for i in 0..200 {
        let data = on_disk_now(&e, i);
        assert_eq!(data.permissions, vec![node(&format!("plots.plot.{i}"), None)], "{i} kept its node");
        assert_eq!(data.parents, vec![parent("mod", None)], "{i} got the group");
    }
    assert_eq!(e.perms.member_count("mod", 1_000), 200);
    assert!(e.perms.users().count() <= crate::engine::VISITORS, "the records went back after the import");
    // A preview of another source changes nothing.
    let out =
        import_source(&mut e, &Who::console(), "other", Ok(("f2".into(), source("vip"))), SourceMode::Preview, 1_000);
    assert!(out.reply.plain().contains("would add 200 entries"), "{}", out.reply.plain());
    assert_eq!(e.perms.member_count("vip", 1_000), 0);
    assert_eq!(on_disk_now(&e, 7).parents, vec![parent("mod", None)]);
    // Undo takes the group back and leaves the node.
    let out = import_source(&mut e, &Who::console(), "file", Err(String::new()), SourceMode::Undo, 1_000);
    assert!(out.changed, "{}", out.reply.plain());
    for i in 0..200 {
        let data = on_disk_now(&e, i);
        assert!(data.parents.is_empty() && data.permissions.len() == 1, "{i}: {data:?}");
    }
    assert_eq!(e.perms.member_count("mod", 1_000), 0);
}

#[test]
fn listmembers_shows_players_on_disk() {
    let mut e = lazy();
    let mut users: Vec<User> = (0..40).map(|i| User { name: format!("Disk{i:02}"), ..in_vip(i) }).collect();
    // Gone was in vip until 500: at 1000 it is neither listed nor counted.
    let ended = Data { parents: vec![parent("vip", Some(500))], ..Data::default() };
    users.push(User { name: "Gone".into(), data: ended, ..in_vip(40) });
    on_disk(&mut e, users);
    // Two online players in vip, one of them read from the database at the join.
    e.seen(&player(0), "Disk00", true, 1_000).unwrap();
    console(&mut e, "user Steve parent add vip");
    let first = console(&mut e, "group vip listmembers");
    assert!(first.contains("Members of vip: 41"), "{first}");
    let shown: Vec<&str> = first.lines().filter_map(|l| l.trim().strip_prefix("• ")).collect();
    let mut want: Vec<String> = (0..40).map(|i| format!("Disk{i:02}")).collect();
    want.push("Steve".into());
    assert_eq!(shown, want[..15], "{first}");
    let last = console(&mut e, "group vip listmembers 3");
    let shown: Vec<&str> = last.lines().filter_map(|l| l.trim().strip_prefix("• ")).collect();
    assert_eq!(shown, want[30..], "{last}");
    assert!(console(&mut e, "group vip listmembers 9").contains("Steve"), "past the end: the last page");
}

#[test]
fn listmembers_shows_the_recorded_name_for_players_on_disk() {
    let mut e = lazy();
    on_disk(&mut e, vec![User { name: "OldName".into(), ..in_vip(5) }]);
    e.seen(&player(5), "NewName", true, 1_000).unwrap();
    assert!(console(&mut e, "group vip listmembers").contains("NewName"), "online: the name of the join");
    e.gone(&player(5));
    forget(&mut e);
    let list = console(&mut e, "group vip listmembers");
    assert!(list.contains("OldName") && !list.contains("NewName"), "{list}");
}

#[test]
fn group_info_counts_players_on_disk() {
    let mut e = lazy();
    let mut users: Vec<User> = (0..30).map(in_vip).collect();
    users.push(User { data: Data { parents: vec![parent("vip", Some(500))], ..Data::default() }, ..in_vip(30) });
    users.push(User { data: Data { parents: vec![parent("vip", Some(5_000))], ..Data::default() }, ..in_vip(31) });
    on_disk(&mut e, users);
    console(&mut e, "user Steve parent add vip");
    let info = console(&mut e, "group vip info");
    assert!(info.contains("Members: 32"), "30 on disk, one until 5000 and Steve: {info}");
    assert_eq!(e.perms.users().count(), 1, "counted without reading them");
}

#[test]
fn import_undo_keeps_a_group_with_members_on_disk() {
    let mut e = lazy();
    on_disk(
        &mut e,
        vec![User { data: Data { permissions: vec![node("keep", None)], ..Data::default() }, ..in_vip(7) }],
    );
    let source = export::parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"builder"}]}"#).unwrap();
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), source)), SourceMode::Run, 1_000);
    assert!(out.changed, "{}", out.reply.plain());
    // An admin puts a player who is not online into the imported group.
    console(&mut e, &format!("user {} parent add builder", player(7)));
    forget(&mut e);
    assert!(!e.perms.knows(&player(7)));
    let out = import_source(&mut e, &Who::console(), "file", Err(String::new()), SourceMode::Undo, 1_000);
    assert!(out.reply.plain().contains("Removed 0"), "{}", out.reply.plain());
    assert!(e.perms.group("builder").is_some(), "the group still has a member");
    let stored = e.store().unwrap().user(&player(7)).unwrap().unwrap().data;
    assert_eq!((stored.parents, stored.permissions), (vec![parent("builder", None)], vec![node("keep", None)]));
}
