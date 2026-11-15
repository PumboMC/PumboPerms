//! Players' records read when needed: visitors, online players, ended
//! entries and the guard against a change of a player nobody read.

use super::*;
use crate::model::{Data, Group, ParentNode, User};

fn engine() -> Engine {
    Engine::lazy(CoreCfg::default(), Lang::load(&[COMMON, crate::LANG], "en", None).0)
}

fn player(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

fn in_vip(uuid: &str, expiry: Option<u64>) -> User {
    let parent = ParentNode { group: "vip".into(), context: Contexts::global(), expiry };
    User { uuid: uuid.into(), name: "P".into(), data: Data { parents: vec![parent], ..Data::default() } }
}

/// Writes `users` (and the group `vip`) to the database as an earlier start
/// left them, and reads it again.
fn on_disk(e: &mut Engine, users: Vec<User>) {
    let (p, mut d) = Perms::from_parts(vec![Group::new("default"), Group::new("vip")], users, Vec::new());
    d.groups.extend(p.groups().map(|g| g.name.clone()));
    d.users.extend(p.users().map(|u| u.uuid.clone()));
    e.store().unwrap().commit(&p, &d, None, 0).unwrap();
    e.reload_data();
}

/// Records in memory.
fn held(e: &Engine) -> usize {
    e.perms.users().count()
}

#[test]
fn offline_player_loads_for_a_command_and_leaves_again() {
    let mut e = engine();
    on_disk(&mut e, (0..70).map(|i| in_vip(&player(i), None)).collect());
    for i in [0, 1] {
        e.seen(&player(1000 + i), "Online", true, 1).unwrap();
    }
    assert_eq!(held(&e), 0, "nobody with data is online");
    for i in 0..65 {
        assert_eq!(e.decide(&player(i), "group.vip", "", 0, 1), Some(true), "{i}");
        assert!(held(&e) <= VISITORS, "{i}: {} records", held(&e));
    }
    assert_eq!(held(&e), VISITORS);
    assert!(!e.perms.knows(&player(0)) && e.perms.knows(&player(1)) && e.perms.knows(&player(64)));
    // Read again when needed, and the oldest goes instead.
    assert_eq!(e.explain(&player(0), "group.vip", "", 1).map(|d| d.value), Some(true));
    assert!(e.perms.knows(&player(0)) && !e.perms.knows(&player(1)));
    assert_eq!(e.perms.member_count("vip", 1), 70);
}

#[test]
fn online_players_are_never_evicted() {
    let mut e = engine();
    on_disk(&mut e, (0..200).map(|i| in_vip(&player(i), None)).collect());
    e.seen(&player(0), "First", true, 1).unwrap();
    e.seen(&player(1), "Second", true, 1).unwrap();
    for i in 2..200 {
        e.load_user(&player(i), 1);
    }
    assert!(e.perms.user(&player(0)).is_some() && e.perms.user(&player(1)).is_some());
    assert_eq!(held(&e), VISITORS + 2);
    // Leaving makes a player one of the visitors, the newest.
    e.gone(&player(0));
    assert!(e.perms.knows(&player(0)) && held(&e) == VISITORS + 1);
    for i in 2..2 + u32::try_from(VISITORS).unwrap() {
        e.load_user(&player(i), 1);
    }
    assert!(!e.perms.knows(&player(0)) && e.perms.knows(&player(1)));
    // A visitor who joins stays.
    let visitor = player(150);
    e.load_user(&visitor, 1);
    e.seen(&visitor, "Visitor", true, 1).unwrap();
    for i in 2..150 {
        e.load_user(&player(i), 1);
    }
    assert!(e.perms.knows(&visitor) && e.perms.user(&visitor).is_some());
}

#[test]
fn reload_brings_back_the_online_players() {
    let mut e = engine();
    on_disk(&mut e, (0..3).map(|i| in_vip(&player(i), None)).collect());
    e.seen(&player(0), "Online", true, 1).unwrap();
    e.load_user(&player(1), 1);
    e.reload_data();
    assert!(e.perms.user(&player(0)).is_some(), "online: read again");
    assert!(!e.perms.knows(&player(1)), "a visitor stays on disk");
    assert_eq!(e.decide(&player(1), "group.vip", "", 0, 1), Some(true));
}

#[test]
fn expired_entries_of_a_loaded_record_are_dropped_and_saved() {
    let mut e = engine();
    let mut steve = in_vip(&player(0), Some(50));
    steve.data.parents.push(ParentNode { group: "default".into(), context: Contexts::global(), expiry: None });
    on_disk(&mut e, (0..100).map(|i| if i == 0 { steve.clone() } else { in_vip(&player(i), None) }).collect());
    e.load_user(&player(0), 100);
    let parents = |u: Option<User>| u.map(|u| u.data.parents.iter().map(|p| p.group.clone()).collect::<Vec<_>>());
    assert_eq!(parents(e.perms.user(&player(0)).cloned()), Some(vec!["default".to_string()]), "dropped in memory");
    assert_eq!(parents(e.store().unwrap().user(&player(0)).unwrap()).map(|p| p.len()), Some(2), "not saved yet");
    for i in 1..100 {
        e.load_user(&player(i), 100);
    }
    assert!(e.perms.knows(&player(0)), "it waits in memory until it is saved");
    assert_eq!(e.expire(100), (false, Ok(())), "nothing in memory ended now");
    assert_eq!(parents(e.store().unwrap().user(&player(0)).unwrap()), Some(vec!["default".to_string()]), "saved");
    assert_eq!(e.expire(101), (false, Ok(())), "nothing left to save");
    for i in 1..100 {
        e.load_user(&player(i), 100);
    }
    assert!(!e.perms.knows(&player(0)), "saved, it can go");
}

#[test]
fn expire_is_not_due_after_an_eviction() {
    let mut e = engine();
    on_disk(&mut e, (0..100).map(|i| in_vip(&player(i), (i == 0).then_some(1_000))).collect());
    e.load_user(&player(0), 1);
    assert_eq!(e.perms.next_expiry(), Some(1_000));
    for i in 1..100 {
        e.load_user(&player(i), 1);
    }
    assert!(!e.perms.knows(&player(0)));
    assert_eq!(e.perms.next_expiry(), None);
    assert_eq!(e.expire(2_000), (false, Ok(())));
    assert_eq!(e.decide(&player(0), "group.vip", "", 0, 2_000), None, "the rank ended while it was on disk");
}

#[test]
fn change_of_a_player_that_was_not_loaded_is_refused() {
    let mut e = engine();
    on_disk(&mut e, vec![in_vip(&player(0), None)]);
    let fly = PermNode { node: "fly".into(), value: true, context: Contexts::global(), expiry: None };
    // As a command changes a player, without reading the player first.
    e.begin();
    let (_, d) = e.perms.edit_user(&player(0), "P", |d| d.set_permission(fly));
    let refused = e.commit(&d, None);
    assert_eq!(e.store().unwrap().user(&player(0)).unwrap(), Some(in_vip(&player(0), None)), "the disk keeps it");
    assert_eq!(refused, Err(format!("player {} was not loaded before the change", player(0))));
    assert!(e.perms.user(&player(0)).is_none() && !e.perms.knows(&player(0)), "memory as before");
    assert_eq!(e.decide(&player(0), "group.vip", "", 0, 1), Some(true));
}

#[test]
fn rollback_lets_the_visitors_go() {
    let mut e = engine();
    on_disk(&mut e, (0..150).map(|i| in_vip(&player(i), None)).collect());
    for end in [false, true] {
        e.begin();
        for i in 0..150 {
            e.load_user(&player(i), 1);
        }
        assert_eq!(held(&e), 150, "a change keeps every record it read");
        if end {
            e.end()
        } else {
            e.rollback()
        }
        assert_eq!(held(&e), VISITORS, "end: {end}");
    }
}

#[test]
fn a_record_just_read_stays_for_whoever_read_it() {
    let mut e = engine();
    let mut users: Vec<User> = (0..64).map(|i| in_vip(&player(i), Some(50))).collect();
    users.extend((64..67).map(|i| in_vip(&player(i), None)));
    on_disk(&mut e, users);
    // 64 records lose an ended entry when read and wait to be saved, with no expire in between.
    for i in 0..64 {
        e.load_user(&player(i), 100);
    }
    for i in 64..67 {
        assert_eq!(e.decide(&player(i), "group.vip", "", 0, 100), Some(true), "{i}");
    }
    assert_eq!(e.expire(100), (false, Ok(())));
    assert!(held(&e) <= VISITORS + 1, "{} records", held(&e));
}

#[test]
fn reload_forgets_the_records_waiting_to_be_saved() {
    let mut e = engine();
    on_disk(&mut e, vec![in_vip(&player(0), Some(50))]);
    e.load_user(&player(0), 100);
    e.reload_data();
    assert_eq!(e.expire(100), (false, Ok(())), "nothing waits for a record that is not in memory");
}
