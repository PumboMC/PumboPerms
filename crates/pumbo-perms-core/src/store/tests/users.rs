//! Players' records: one read alone, the counts at start and the guard that
//! keeps a player nobody read from being written.

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
    s.load_index().unwrap();
    assert_eq!(BREATHS.with(Cell::get), 2, "once per 1024 records");
}
