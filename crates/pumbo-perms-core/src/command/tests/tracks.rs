//! Tracks: promote and demote.

use super::*;

#[test]
fn tracks_promote_demote() {
    let mut e = engine();
    for g in ["member", "vip", "mvp"] {
        console(&mut e, &format!("creategroup {g}"));
    }
    console(&mut e, "createtrack ranks");
    for g in ["member", "vip", "mvp"] {
        assert!(console(&mut e, &format!("track ranks append {g}")).contains("Added"));
    }
    assert!(console(&mut e, "track ranks info").contains("1. member"));
    assert!(console(&mut e, "promote Steve ranks").contains("first group"));
    assert!(console(&mut e, "promote Steve ranks").contains("member → vip"));
    assert!(console(&mut e, "user Steve promote ranks").contains("vip → mvp"));
    assert!(console(&mut e, "promote Steve ranks").contains("highest"));
    assert!(console(&mut e, "demote Steve ranks").contains("mvp → vip"));
    assert!(console(&mut e, "demote Alex ranks").contains("not on track"));
    assert!(console(&mut e, "tracks").contains("member → vip → mvp"));
    assert!(console(&mut e, "track ranks insert admin 1").contains("does not exist"));
    console(&mut e, "creategroup admin");
    console(&mut e, "track ranks insert admin 1");
    assert!(console(&mut e, "track ranks info").contains("1. admin"));
}

/// `ranks`: member, vip, mvp; `vip` temporary for Steve when `until` is set.
fn ranks(e: &mut Engine, first: &str, until: Option<&str>) {
    for g in [first, "vip", "mvp"] {
        console(e, &format!("creategroup {g}"));
    }
    console(e, "createtrack ranks");
    for g in [first, "vip", "mvp"] {
        console(e, &format!("track ranks append {g}"));
    }
    if let Some(d) = until {
        console(e, &format!("user Steve parent addtemp vip {d}"));
    }
}

#[test]
fn promote_keeps_expiry() {
    let mut e = engine();
    ranks(&mut e, "member", Some("30d"));
    let end = e.perms.user(STEVE).unwrap().data.parents[0].expiry.unwrap();
    let out = console(&mut e, "promote Steve ranks");
    assert!(out.contains("vip → mvp") && out.contains("expires in 30d"), "{out}");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert_eq!(parents.len(), 1, "{parents:?}");
    assert_eq!((parents[0].group.as_str(), parents[0].expiry), ("mvp", Some(end)));
}

#[test]
fn promote_skips_expired_rung() {
    let mut e = engine();
    ranks(&mut e, "member", Some("1h"));
    // An hour later the temporary vip is over: Steve starts from the bottom.
    let later = 1_000 + 3_600_000;
    let out = run_as(&mut e, &Who::console(), "promote Steve ranks", later);
    assert!(out.contains("is now member"), "{out}");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert!(parents.iter().any(|p| p.group == "member" && p.expiry.is_none()), "{parents:?}");
}

#[test]
fn promote_from_default_once() {
    let mut e = engine();
    ranks(&mut e, "default", None);
    let out = console(&mut e, "promote Steve ranks");
    assert!(out.contains("default → vip"), "{out}");
    assert!(e.perms.user(STEVE).unwrap().data.parents.iter().any(|p| p.group == "vip"));
}
