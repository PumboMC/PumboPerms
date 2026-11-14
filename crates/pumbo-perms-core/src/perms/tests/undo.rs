//! A change undone from the records it kept.

use super::*;
use crate::context::Query;

/// `vip`, `mod` inheriting `vip` and `admin` inheriting `mod` and `vip` for a
/// while; track `ranks`; three players with groups, nodes and an entry that
/// ended at 50.
fn database() -> Perms {
    let mut p = Perms::new();
    for g in ["vip", "mod", "admin"] {
        p.create_group(g).unwrap();
    }
    p.edit_group("vip", |g| g.data.set_permission(perm("vip.chat", None))).unwrap();
    p.add_group_parent("mod", parent("vip", None)).unwrap();
    p.add_group_parent("admin", parent("mod", None)).unwrap();
    p.edit_group("admin", |g| g.data.add_parent(parent("vip", Some(50)))).unwrap();
    p.create_track("ranks").unwrap();
    for g in ["vip", "mod", "admin"] {
        p.track_insert("ranks", g, None).unwrap();
    }
    p.edit_user(U, "Steve", |d| {
        d.add_parent(parent("vip", None));
        d.set_permission(perm("fly", Some(50)));
    });
    p.edit_user(U2, "Alex", |d| d.add_parent(parent("mod", None)));
    p.edit_user(U3, "Notch", |d| {
        d.add_parent(parent("vip", Some(40)));
        d.add_parent(parent("admin", None));
    });
    p
}

const NEW: &str = "00000000-0000-0000-0000-000000000009";

/// A change made to the database of a test.
type Change = fn(&mut Perms);

#[test]
fn rollback_restores_every_kind_of_change() {
    let mut p = database();
    let before = everything(&p);
    let here = Query::default();
    let answer = |p: &mut Perms| (p.check(U, "vip.chat", &here, 0), p.effective(U2, &here, 0).groups.clone());
    let first = answer(&mut p);
    let changes: Vec<(&str, Change)> = vec![
        ("create_group", |p| drop(p.create_group("new").unwrap())),
        ("edit_group", |p| drop(p.edit_group("vip", |g| g.data.permissions.clear()).unwrap())),
        ("add_group_parent", |p| drop(p.add_group_parent("vip", parent("default", None)).unwrap())),
        ("delete_group", |p| drop(p.delete_group("vip").unwrap())),
        ("rename_group", |p| drop(p.rename_group("vip", "gold").unwrap())),
        ("edit_user", |p| drop(p.edit_user(U, "Steve", |d| d.parents.clear()))),
        ("edit_user of a new player", |p| drop(p.edit_user(NEW, "New", |d| d.add_parent(parent("vip", None))))),
        ("promote", |p| drop(p.promote(U2, "Alex", "ranks", &Contexts::global(), 0).unwrap())),
        ("demote", |p| drop(p.demote(U2, "Alex", "ranks", &Contexts::global(), 0).unwrap())),
        ("create_track", |p| drop(p.create_track("other").unwrap())),
        ("delete_track", |p| drop(p.delete_track("ranks").unwrap())),
        ("track_insert", |p| drop(p.track_insert("ranks", "default", Some(0)).unwrap())),
        ("track_remove", |p| drop(p.track_remove("ranks", "mod").unwrap())),
        ("track_clear", |p| drop(p.track_clear("ranks").unwrap())),
        ("expire", |p| drop(p.expire(100))),
    ];
    for (name, change) in changes {
        // The answers are cached before the change.
        assert_eq!(answer(&mut p), first, "{name}");
        p.begin();
        change(&mut p);
        assert_ne!(everything(&p), before, "{name} changed nothing");
        p.rollback();
        assert!(!p.changing(), "{name}");
        assert_eq!(everything(&p), before, "{name}");
        assert_eq!(answer(&mut p), first, "{name}: what was computed from the change is gone");
    }
}

#[test]
fn rollback_restores_bulk_changes() {
    let mut p = database();
    let before = everything(&p);
    for (name, change) in [
        ("delete", (|p| drop(p.delete_group("vip").unwrap())) as Change),
        ("rename", |p| drop(p.rename_group("vip", "gold").unwrap())),
        ("expire", |p| assert_eq!(p.expire(60).users.len(), 2)),
        ("all of them", |p| {
            p.rename_group("vip", "gold").unwrap();
            p.edit_user(U, "Steve", |d| d.add_parent(parent("gold", None)));
            p.delete_group("gold").unwrap();
            p.rename_group("mod", "vip").unwrap();
            p.expire(60);
        }),
    ] {
        p.begin();
        change(&mut p);
        assert_ne!(everything(&p), before, "{name}");
        p.rollback();
        assert_eq!(everything(&p), before, "{name}");
    }
    // A record touched twice keeps its first image; `end` keeps the change.
    p.begin();
    p.edit_user(U, "Steve", |d| d.parents.clear());
    p.begin();
    p.edit_user(U, "Steve", |d| d.add_parent(parent("mod", None)));
    p.rollback();
    assert_eq!(everything(&p), before);
    p.begin();
    p.delete_group("vip").unwrap();
    p.end();
    p.rollback();
    assert!(p.group("vip").is_none() && !p.changing(), "nothing to undo after the end");
}
