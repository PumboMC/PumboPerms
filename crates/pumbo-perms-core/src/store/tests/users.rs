//! Players' records: one read alone, the counts at start, the guard that
//! keeps a player nobody read from being written, and group changes that
//! reach the records on disk.

use std::cell::Cell;

use super::*;
use crate::model::Data;

const A: &str = "00000000-0000-0000-0000-0000000000a1";
const B: &str = "00000000-0000-0000-0000-0000000000b2";

fn in_group(uuid: &str, group: &str, expiry: Option<u64>) -> User {
    let parent = ParentNode { group: group.into(), context: Contexts::global(), expiry };
    User { uuid: uuid.into(), name: "P".into(), data: Data { parents: vec![parent], ..Data::default() } }
}

fn fly() -> PermNode {
    PermNode { node: "fly".into(), value: true, context: Contexts::global(), expiry: None }
}

/// A store with `users` written as a change of the whole database would.
fn store_with(users: Vec<User>) -> PermsStore {
    let s = PermsStore::in_memory().unwrap();
    let groups = ["vip", "mod"].map(crate::model::Group::new).into();
    let (p, mut d) = Perms::from_parts(groups, users, Vec::new());
    d.groups.extend(p.groups().map(|g| g.name.clone()));
    d.users.extend(p.users().map(|u| u.uuid.clone()));
    s.commit(&p, &d, None, 100).unwrap();
    s
}

#[test]
fn one_player_is_read_alone() {
    let s = store_with(vec![in_group(A, "vip", None)]);
    assert_eq!(s.user(A).unwrap(), Some(in_group(A, "vip", None)));
    assert_eq!(s.user(B).unwrap(), None);
    s.store.write(|tx| tx.put(USERS, B, b"{broken")).unwrap();
    assert!(s.user(B).unwrap_err().0.contains("damaged record perms_users/"), "never an empty record");
}

#[test]
fn damaged_player_record_stops_the_load() {
    let s = store_with(vec![in_group(A, "vip", None)]);
    s.store.write(|tx| tx.put(USERS, B, b"{broken")).unwrap();
    assert!(s.load_index().is_err());
    assert!(s.load().is_err());
}

#[test]
fn index_counts_what_load_would_hold() {
    let mut users: Vec<User> = (0..40u32)
        .map(|i| {
            let uuid = format!("00000000-0000-4000-8000-{i:012}");
            let mut u =
                in_group(&uuid, if i % 3 == 0 { "mod" } else { "vip" }, (i % 4 == 0).then_some(u64::from(i) * 10));
            u.data.permissions.push(PermNode {
                node: format!("node.{}", i % 5),
                value: true,
                context: Contexts::global(),
                expiry: None,
            });
            u
        })
        .collect();
    users.push(in_group(A, "vip", Some(1_000)));
    let s = store_with(users);
    let (all, _) = s.load().unwrap();
    let (index, dirty) = s.load_index().unwrap();
    assert!(dirty.is_empty() && index.lazy() && index.users().next().is_none());
    for now in [0, 100, 395, 1_000] {
        for group in ["vip", "mod", "default"] {
            assert_eq!(index.member_count(group, now), all.member_count(group, now), "{group} at {now}");
        }
    }
    assert_eq!(index.all_nodes(), all.all_nodes());
    assert_eq!(index.groups().count(), all.groups().count());
}

#[test]
fn a_player_that_was_not_read_is_not_written() {
    let s = store_with(vec![in_group(A, "vip", None), in_group(B, "vip", None)]);
    let (mut p, _) = s.load_index().unwrap();
    // A change of A without reading A first: memory starts from nothing.
    let (_, d) = p.edit_user(A, "", |d| d.permissions.push(fly()));
    let saved = s.commit(&p, &d, None, 100);
    assert_eq!(s.user(A).unwrap(), Some(in_group(A, "vip", None)), "the record on disk stays");
    assert_eq!(saved.unwrap_err().0, format!("player {A} was not loaded before the change"));
    // Read first, the same change is saved.
    assert!(p.admit(B, s.user(B).unwrap()));
    let (_, d) = p.edit_user(B, "", |d| d.permissions.push(fly()));
    s.commit(&p, &d, None, 100).unwrap();
    assert_eq!(s.user(B).unwrap().unwrap().data.parents, in_group(B, "vip", None).data.parents);
}

thread_local! {
    static BREATHS: Cell<u32> = const { Cell::new(0) };
}

fn breath() {
    BREATHS.with(|b| b.set(b.get() + 1));
}

#[test]
fn full_passes_call_the_host_hook() {
    let users = (0..3000u32).map(|i| in_group(&format!("00000000-0000-4000-8000-{i:012}"), "vip", None)).collect();
    let mut s = store_with(users);
    s.load_index().unwrap();
    assert_eq!(BREATHS.with(Cell::get), 0, "nothing set");
    s.set_breathe(breath);
    assert!(s.breathe().is_some());
    let (mut p, _) = s.load_index().unwrap();
    assert_eq!(BREATHS.with(Cell::get), 2, "once per 1024 records");
    BREATHS.with(|b| b.set(0));
    let d = p.rename_group("vip", "gold").unwrap();
    s.commit(&p, &d, None, 100).unwrap();
    assert_eq!(BREATHS.with(Cell::get), 2, "the group change goes over every record too");
    BREATHS.with(|b| b.set(0));
    crate::export::write(&mut Vec::new(), &p, &s, 0).unwrap();
    assert_eq!(BREATHS.with(Cell::get), 2, "and an export");
}

/// The groups of every record on disk, with its name and nodes.
fn on_disk(s: &PermsStore) -> Vec<(String, String, Vec<String>, usize)> {
    let (all, _) = s.load().unwrap();
    let mut out: Vec<_> = all
        .users()
        .map(|u| {
            let groups = u.data.parents.iter().map(|p| p.group.clone()).collect();
            (u.uuid.clone(), u.name.clone(), groups, u.data.permissions.len())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn group_rename_and_delete_reach_records_on_disk() {
    let mut users: Vec<User> = (0..40u32)
        .map(|i| {
            let mut u = in_group(&format!("00000000-0000-4000-8000-{i:012}"), "vip", (i % 3 == 0).then_some(100));
            if i % 2 == 0 {
                u.data.parents.push(ParentNode { group: "mod".into(), context: Contexts::global(), expiry: None });
            }
            u
        })
        .collect();
    // Named "vip" without the group: its bytes hold "vip" too.
    let mut named = in_group(B, "mod", None);
    named.name = "vip".into();
    users.push(named);
    let s = store_with(users);
    let before = on_disk(&s);
    let (mut p, _) = s.load_index().unwrap();
    // One record in memory, the rest on disk.
    let first = "00000000-0000-4000-8000-000000000000";
    assert!(p.admit(first, s.user(first).unwrap()));
    let d = p.rename_group("vip", "gold").unwrap();
    assert_eq!((d.users.len(), d.regroup.clone()), (1, vec![("vip".to_string(), Some("gold".to_string()))]));
    s.commit(&p, &d, None, 100).unwrap();
    let renamed: Vec<_> = before
        .iter()
        .map(|(u, n, g, k)| (u.clone(), n.clone(), g.iter().map(|g| g.replace("vip", "gold")).collect(), *k))
        .collect();
    assert_eq!(on_disk(&s), renamed);
    let (fresh, _) = s.load_index().unwrap();
    for now in [0, 99, 100] {
        for group in ["vip", "gold", "mod"] {
            assert_eq!(p.member_count(group, now), fresh.member_count(group, now), "{group} at {now}");
        }
    }
    assert_eq!(p.member_count("gold", 0), 40);
    let d = p.delete_group("gold").unwrap();
    s.commit(&p, &d, None, 100).unwrap();
    let deleted: Vec<_> = renamed
        .iter()
        .filter(|(_, _, g, _)| g.iter().any(|g| g != "gold"))
        .map(|(u, n, g, k)| (u.clone(), n.clone(), g.iter().filter(|g| *g != "gold").cloned().collect(), *k))
        .collect();
    assert_eq!(on_disk(&s), deleted);
    assert_eq!(p.member_count("gold", 0), 0);
    assert_eq!(p.member_count("mod", 0), s.load().unwrap().0.member_count("mod", 0));
}

#[test]
fn a_record_left_empty_is_removed() {
    let mut kept = in_group(B, "vip", None);
    kept.data.permissions.push(fly());
    let s = store_with(vec![in_group(A, "vip", None), kept]);
    let (mut p, _) = s.load_index().unwrap();
    let d = p.delete_group("vip").unwrap();
    s.commit(&p, &d, None, 100).unwrap();
    assert_eq!(s.user(A).unwrap(), None, "nothing left");
    assert_eq!(s.user(B).unwrap().map(|u| (u.data.parents.len(), u.data.permissions)), Some((0, vec![fly()])));
}

#[test]
fn load_after_a_group_change_in_the_same_change_is_refused() {
    let s = store_with(vec![in_group(A, "vip", None), in_group(B, "vip", None)]);
    let (mut p, _) = s.load_index().unwrap();
    p.begin();
    assert!(p.admit(A, s.user(A).unwrap()), "before the group change");
    let mut d = p.delete_group("vip").unwrap();
    assert!(!p.admit(B, s.user(B).unwrap()) && !p.knows(B), "after it");
    d.merge(p.edit_user(B, "", |d| d.permissions.push(fly())).1);
    assert!(s.commit(&p, &d, None, 100).is_err());
    p.rollback();
    assert_eq!(
        (s.user(A).unwrap(), s.user(B).unwrap()),
        (Some(in_group(A, "vip", None)), Some(in_group(B, "vip", None)))
    );
    assert_eq!(p.member_count("vip", 0), 2);
    assert!(p.admit(B, s.user(B).unwrap()), "the change is over");
}
