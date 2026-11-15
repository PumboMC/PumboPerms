//! Players kept on disk: the member counts and records coming and going.

use super::*;
use crate::context::Query;

const A: &str = "00000000-0000-0000-0000-0000000000a1";
const B: &str = "00000000-0000-0000-0000-0000000000b2";
const C: &str = "00000000-0000-0000-0000-0000000000c3";
const D: &str = "00000000-0000-0000-0000-0000000000d4";
const E: &str = "00000000-0000-0000-0000-0000000000e5";
const NOBODY: &str = "00000000-0000-0000-0000-000000000099";

fn user(uuid: &str, parents: Vec<ParentNode>) -> User {
    User { uuid: uuid.into(), name: uuid[uuid.len() - 2..].into(), data: Data { parents, ..Data::default() } }
}

/// A in vip; B in vip until 100; C in vip until 10 and in mod; D in vip on
/// one server and until 200 everywhere; E in no group, with a node.
fn records() -> Vec<User> {
    let on_lobby =
        ParentNode { context: Contexts::parse(&["server=lobby".to_string()]).unwrap(), ..parent("vip", None) };
    let mut e = user(E, Vec::new());
    e.data.permissions.push(perm("e.node", None));
    vec![
        user(A, vec![parent("vip", None)]),
        user(B, vec![parent("vip", Some(100))]),
        user(C, vec![parent("vip", Some(10)), parent("mod", None)]),
        user(D, vec![on_lobby, parent("vip", Some(200))]),
        e,
    ]
}

fn groups() -> Vec<Group> {
    ["default", "vip", "mod"].map(Group::new).into()
}

/// The records on disk, counted.
fn on_disk(records: &[User]) -> Perms {
    let mut offline = Offline::default();
    for u in records {
        offline.add(u);
    }
    Perms::from_disk(groups(), Vec::new(), offline).0
}

/// The member counts of every group at some times.
fn counts(p: &Perms) -> Vec<usize> {
    let mut out = Vec::new();
    for now in [0, 9, 10, 50, 99, 100, 150, 199, 200, 300] {
        out.extend(["default", "vip", "mod", "none"].map(|g| p.member_count(g, now)));
    }
    out
}

#[test]
fn admit_and_evict_keep_the_member_counts() {
    let all = records();
    let everyone = Perms::from_parts(groups(), all.clone(), Vec::new()).0;
    let mut p = on_disk(&all);
    let want = counts(&everyone);
    assert_eq!(counts(&p), want);
    assert_eq!((p.member_count("vip", 50), p.member_count("vip", 150), p.member_count("mod", 300)), (3, 2, 1));
    assert!(!p.knows(A) && p.lazy() && !everyone.lazy() && everyone.knows(A));
    for u in &all {
        assert!(p.admit(&u.uuid, Some(u.clone())));
        assert!(p.knows(&u.uuid));
        assert_eq!(counts(&p), want, "{} came in", u.uuid);
    }
    assert!(p.admit(NOBODY, None) && p.knows(NOBODY) && p.user(NOBODY).is_none());
    assert!(p.admit(A, Some(user(A, Vec::new()))), "a record in memory stays as it is");
    assert_eq!((counts(&p), p.user(A)), (want.clone(), all.first()));
    for u in &all {
        assert!(p.evict(&u.uuid));
        assert!(!p.knows(&u.uuid) && p.user(&u.uuid).is_none());
        assert_eq!(counts(&p), want, "{} went out", u.uuid);
    }
    assert!(p.evict(NOBODY) && !p.evict(NOBODY));
    assert_eq!(counts(&p), want);
    // The nodes of the records on disk are known all the time.
    assert!(p.all_nodes().contains("e.node"));
}

#[test]
fn member_count_follows_the_time() {
    let p = on_disk(&[user(A, vec![parent("vip", Some(100))]), user(B, vec![parent("vip", Some(100))])]);
    assert_eq!((p.member_count("vip", 0), p.member_count("vip", 99)), (2, 2));
    assert_eq!((p.member_count("vip", 100), p.member_count("vip", 1000)), (0, 0), "the rank ended while offline");
}

#[test]
fn member_count_is_exact_during_a_change() {
    let all = records();
    let mut p = on_disk(&all);
    let want = counts(&p);
    p.begin();
    assert!(p.admit(A, all.first().cloned()));
    p.edit_user(A, "", |d| d.parents.clear());
    assert_eq!(p.member_count("vip", 50), 2);
    p.edit_user(A, "", |d| d.add_parent(parent("mod", None)));
    assert_eq!((p.member_count("vip", 50), p.member_count("mod", 50)), (2, 2));
    assert!(p.admit(B, all.get(1).cloned()));
    assert_eq!(p.member_count("vip", 50), 2, "loading during the change counts the same");
    p.rollback();
    assert_eq!(counts(&p), want);
    assert!(p.knows(A) && p.knows(B), "loaded records stay after the change");
}

#[test]
fn evict_waits_for_the_change_to_end() {
    let all = records();
    let mut p = on_disk(&all);
    p.admit(A, all.first().cloned());
    p.begin();
    assert!(!p.evict(A) && p.knows(A) && p.user(A).is_some());
    p.end();
    assert!(p.evict(A) && !p.knows(A));
    assert!(!p.evict(A), "already on disk");
}

#[test]
fn evicted_record_keeps_its_nodes_in_the_list() {
    let all = records();
    let mut p = on_disk(&all);
    assert!(p.admit(A, all.first().cloned()));
    p.begin();
    p.edit_user(A, "", |d| d.permissions.push(perm("a.fresh", None)));
    p.end();
    assert!(p.evict(A));
    assert!(p.all_nodes().contains("a.fresh"), "a node of a player back on disk stays in the list");
}

#[test]
fn evicted_record_leaves_no_expiry_behind() {
    let b = user(B, vec![parent("vip", Some(100))]);
    let mut p = on_disk(std::slice::from_ref(&b));
    assert_eq!(p.next_expiry(), None);
    p.admit(B, Some(b));
    assert_eq!(p.next_expiry(), Some(100));
    assert!(p.check(B, "group.vip", &Query::default(), 50).is_some());
    p.evict(B);
    assert_eq!(p.next_expiry(), None, "the end of a record on disk is not due here");
    assert!(p.memo.cache.is_empty());
}
