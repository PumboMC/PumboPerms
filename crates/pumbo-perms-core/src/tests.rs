//! Decision rules: inheritance, weights, wildcards, contexts, time and meta.

use crate::context::{Contexts, Query};
use crate::model::{MetaKind, ParentNode, PermNode};
use crate::perms::{Perms, PermsError, Source};

const U: &str = "00000000-0000-0000-0000-000000000001";
const U2: &str = "00000000-0000-0000-0000-000000000002";

fn ctx(s: &str) -> Contexts {
    let args: Vec<String> = s.split_whitespace().map(str::to_string).collect();
    Contexts::parse(&args).unwrap()
}

fn here() -> Query {
    Query::new("lobby", "lobbies", "world")
}

fn perm(node: &str, value: bool) -> PermNode {
    PermNode { node: node.into(), value, context: Contexts::global(), expiry: None }
}

fn perm_in(node: &str, value: bool, c: &str) -> PermNode {
    PermNode { node: node.into(), value, context: ctx(c), expiry: None }
}

fn perm_until(node: &str, value: bool, expiry: u64) -> PermNode {
    PermNode { node: node.into(), value, context: Contexts::global(), expiry: Some(expiry) }
}

fn parent(group: &str) -> ParentNode {
    ParentNode { group: group.into(), context: Contexts::global(), expiry: None }
}

fn group(p: &mut Perms, name: &str, weight: i32, perms: &[PermNode]) {
    if p.group(name).is_none() {
        p.create_group(name).unwrap();
    }
    p.edit_group(name, |g| {
        g.weight = weight;
        for n in perms {
            g.data.set_permission(n.clone());
        }
    })
    .unwrap();
}

fn user_in(p: &mut Perms, uuid: &str, groups: &[&str]) {
    p.edit_user(uuid, "Steve", |d| {
        for g in groups {
            d.add_parent(parent(g));
        }
    });
}

#[test]
fn nothing_set_is_undecided_and_everyone_is_in_default() {
    let mut p = Perms::new();
    assert_eq!(p.check(U, "a.b", &here(), 0), None);
    assert_eq!(p.check(U, "group.default", &here(), 0), Some(true));
    group(&mut p, "default", 0, &[perm("chat.use", true)]);
    assert_eq!(p.check(U, "chat.use", &here(), 0), Some(true));
    // a user with another group no longer gets default implicitly
    group(&mut p, "vip", 10, &[]);
    user_in(&mut p, U, &["vip"]);
    assert_eq!(p.check(U, "chat.use", &here(), 0), None);
    assert_eq!(p.check(U, "group.default", &here(), 0), None);
    // ...unless vip inherits it
    p.add_group_parent("vip", parent("default")).unwrap();
    assert_eq!(p.check(U, "chat.use", &here(), 0), Some(true));
    assert_eq!(p.effective(U, &here(), 0).groups, vec!["vip", "default"]);
}

#[test]
fn user_beats_groups_and_weight_decides_between_groups() {
    let mut p = Perms::new();
    group(&mut p, "low", 1, &[perm("fly", true), perm("build", false)]);
    group(&mut p, "high", 50, &[perm("fly", false)]);
    user_in(&mut p, U, &["low", "high"]);
    assert_eq!(p.check(U, "fly", &here(), 0), Some(false), "higher weight wins");
    assert_eq!(p.check(U, "build", &here(), 0), Some(false), "only one group sets it");
    p.edit_group("low", |g| g.weight = 100).unwrap();
    assert_eq!(p.check(U, "fly", &here(), 0), Some(true), "weights are read at check time");
    p.edit_user(U, "Steve", |d| d.set_permission(perm("fly", false)));
    let d = p.explain(U, "fly", &here(), 0).unwrap();
    assert_eq!((d.value, d.source), (false, Source::User), "the user's own node wins");
}

#[test]
fn weight_beats_depth_and_depth_breaks_ties() {
    let mut p = Perms::new();
    // admin (100) -> mod (50) -> helper (10); the user is in admin only
    group(&mut p, "helper", 10, &[perm("kick", false), perm("mute", true)]);
    group(&mut p, "mod", 50, &[perm("kick", true)]);
    group(&mut p, "admin", 100, &[]);
    p.add_group_parent("mod", parent("helper")).unwrap();
    p.add_group_parent("admin", parent("mod")).unwrap();
    user_in(&mut p, U, &["admin"]);
    assert_eq!(p.check(U, "kick", &here(), 0), Some(true));
    assert_eq!(p.check(U, "mute", &here(), 0), Some(true));
    assert_eq!(p.effective(U, &here(), 0).groups, vec!["admin", "mod", "helper"]);
    // a deeper group with more weight still wins
    p.edit_group("helper", |g| g.weight = 60).unwrap();
    assert_eq!(p.check(U, "kick", &here(), 0), Some(false));
    // equal weight: the closer group wins
    group(&mut p, "a", 0, &[perm("x", true)]);
    group(&mut p, "b", 0, &[perm("x", false)]);
    p.add_group_parent("a", parent("b")).unwrap();
    user_in(&mut p, U2, &["a"]);
    assert_eq!(p.check(U2, "x", &here(), 0), Some(true));
}

#[test]
fn equal_weight_equal_depth_is_decided_by_name() {
    let mut p = Perms::new();
    group(&mut p, "beta", 5, &[perm("x", false)]);
    group(&mut p, "alpha", 5, &[perm("x", true)]);
    user_in(&mut p, U, &["beta", "alpha"]);
    // stable and documented: alphabetical
    assert_eq!(p.check(U, "x", &here(), 0), Some(true));
}

#[test]
fn exact_beats_wildcards_from_any_holder() {
    let mut p = Perms::new();
    group(&mut p, "default", 0, &[perm("pumbo.bans.ban", false)]);
    group(&mut p, "admin", 100, &[perm("pumbo.*", true)]);
    p.add_group_parent("admin", parent("default")).unwrap();
    user_in(&mut p, U, &["admin"]);
    assert_eq!(p.check(U, "pumbo.bans.kick", &here(), 0), Some(true));
    assert_eq!(p.check(U, "pumbo.bans.ban", &here(), 0), Some(false), "the exact node is more specific");
    // longer wildcard beats shorter, regardless of weight
    group(&mut p, "default", 0, &[perm("pumbo.bans.*", false)]);
    assert_eq!(p.check(U, "pumbo.bans.kick", &here(), 0), Some(false));
    assert_eq!(p.check(U, "pumbo.auth.admin", &here(), 0), Some(true));
    // the exact node again wins over the longer wildcard
    p.edit_user(U, "Steve", |d| d.set_permission(perm("pumbo.bans.kick", true)));
    assert_eq!(p.check(U, "pumbo.bans.kick", &here(), 0), Some(true));
}

#[test]
fn star_and_namespaces() {
    let mut p = Perms::new();
    group(&mut p, "owner", 1000, &[perm("*", true), perm("minecraft:command.stop", false)]);
    user_in(&mut p, U, &["owner"]);
    assert_eq!(p.check(U, "anything.at.all", &here(), 0), Some(true));
    assert_eq!(p.check(U, "minecraft:command.op", &here(), 0), Some(true));
    assert_eq!(p.check(U, "minecraft:command.stop", &here(), 0), Some(false));
    group(&mut p, "owner", 1000, &[perm("minecraft:*", false)]);
    assert_eq!(p.check(U, "minecraft:command.op", &here(), 0), Some(false), "ns:* beats *");
    assert_eq!(p.check(U, "pumpkin:command.tps", &here(), 0), Some(true));
    let d = p.explain(U, "minecraft:command.give", &here(), 0).unwrap();
    assert_eq!(d.node, "minecraft:*");
}

#[test]
fn negated_wildcard_in_higher_group() {
    let mut p = Perms::new();
    group(&mut p, "default", 0, &[perm("essentials.home", true)]);
    group(&mut p, "jail", 500, &[perm("essentials.*", false)]);
    p.add_group_parent("jail", parent("default")).unwrap();
    user_in(&mut p, U, &["jail"]);
    // exact true from a low group beats a wildcard false from a high group
    assert_eq!(p.check(U, "essentials.home", &here(), 0), Some(true));
    assert_eq!(p.check(U, "essentials.tp", &here(), 0), Some(false));
    // to really take it away, deny it exactly in the higher group
    group(&mut p, "jail", 500, &[perm("essentials.home", false)]);
    assert_eq!(p.check(U, "essentials.home", &here(), 0), Some(false));
}

#[test]
fn contexts_filter_and_rank_on_one_holder() {
    let mut p = Perms::new();
    p.edit_user(U, "Steve", |d| {
        d.set_permission(perm("fly", true));
        d.set_permission(perm_in("fly", false, "world=nether"));
        d.set_permission(perm_in("fly", true, "server=lobby world=nether"));
        d.set_permission(perm_in("pvp", false, "group=lobbies"));
        d.set_permission(perm_in("pvp", true, "server=lobby"));
        d.set_permission(perm_in("build", true, "server=survival"));
    });
    let nether_lobby = Query::new("lobby", "lobbies", "nether");
    let nether_survival = Query::new("survival", "", "nether");
    assert_eq!(p.check(U, "fly", &here(), 0), Some(true));
    assert_eq!(p.check(U, "fly", &nether_survival, 0), Some(false), "world beats global");
    assert_eq!(p.check(U, "fly", &nether_lobby, 0), Some(true), "server+world beats world");
    assert_eq!(p.check(U, "pvp", &here(), 0), Some(true), "server beats server group");
    assert_eq!(p.check(U, "pvp", &Query::new("other", "lobbies", "world"), 0), Some(false));
    assert_eq!(p.check(U, "build", &here(), 0), None, "other server's node does not apply");
    assert_eq!(p.check(U, "build", &nether_survival, 0), Some(true));
    assert_eq!(p.check(U, "build", &Query::default(), 0), None);
}

#[test]
fn contexts_do_not_beat_holder_order() {
    let mut p = Perms::new();
    group(&mut p, "default", 0, &[perm_in("fly", false, "server=lobby world=world")]);
    p.edit_user(U, "Steve", |d| d.set_permission(perm("fly", true)));
    assert_eq!(p.check(U, "fly", &here(), 0), Some(true));
}

#[test]
fn parents_with_contexts() {
    let mut p = Perms::new();
    group(&mut p, "builder", 10, &[perm("worldedit.*", true)]);
    p.edit_user(U, "Steve", |d| {
        d.add_parent(ParentNode { group: "builder".into(), context: ctx("server=creative"), expiry: None });
    });
    assert_eq!(p.check(U, "worldedit.wand", &here(), 0), None);
    assert_eq!(p.check(U, "group.default", &here(), 0), Some(true), "no parent applies here: default");
    let creative = Query::new("creative", "", "world");
    assert_eq!(p.check(U, "worldedit.wand", &creative, 0), Some(true));
    assert_eq!(p.check(U, "group.default", &creative, 0), None);
}

#[test]
fn temporary_nodes_expire_and_win_while_they_last() {
    let mut p = Perms::new();
    p.edit_user(U, "Steve", |d| {
        d.set_permission(perm("fly", true));
        d.set_permission(perm_until("fly", false, 1_000));
        d.set_permission(perm_until("vip.kit", true, 2_000));
    });
    assert_eq!(p.check(U, "fly", &here(), 0), Some(false), "temporary wins over permanent");
    assert_eq!(p.check(U, "vip.kit", &here(), 999), Some(true));
    // the cache notices the end time without any change to the data
    assert_eq!(p.check(U, "fly", &here(), 1_000), Some(true));
    assert_eq!(p.check(U, "vip.kit", &here(), 1_999), Some(true));
    assert_eq!(p.check(U, "vip.kit", &here(), 2_000), None);
    assert_eq!(p.next_expiry(), Some(1_000));
    let dirty = p.expire(1_500);
    assert!(dirty.users.contains(U));
    assert_eq!(p.next_expiry(), Some(2_000));
}

#[test]
fn temporary_parent_expires() {
    let mut p = Perms::new();
    group(&mut p, "vip", 10, &[perm("vip.chat", true)]);
    p.edit_user(U, "Steve", |d| {
        d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: Some(500) });
    });
    assert_eq!(p.check(U, "vip.chat", &here(), 100), Some(true));
    assert_eq!(p.effective(U, &here(), 100).primary, "vip");
    assert_eq!(p.check(U, "vip.chat", &here(), 500), None);
    assert_eq!(p.effective(U, &here(), 500).primary, "default");
}

#[test]
fn one_holder_has_no_true_ties() {
    let mut p = Perms::new();
    p.edit_user(U, "Steve", |d| {
        d.set_permission(perm_in("x", true, "world=world"));
        d.set_permission(perm_in("x", false, "server=lobby"));
        d.set_permission(perm_in("y", true, "world=world"));
        d.set_permission(perm_in("y", false, "world=world"));
    });
    // server (4) beats world (1)
    assert_eq!(p.check(U, "x", &here(), 0), Some(false));
    // y: same node, same contexts, same kind -> the second replaced the first;
    // context scores are powers of two, so different contexts never tie
    assert_eq!(p.check(U, "y", &here(), 0), Some(false));
}

#[test]
fn cycles_are_refused_and_survive_bad_data() {
    let mut p = Perms::new();
    group(&mut p, "a", 0, &[]);
    group(&mut p, "b", 0, &[]);
    p.add_group_parent("a", parent("b")).unwrap();
    assert_eq!(p.add_group_parent("b", parent("a")), Err(PermsError::Cycle("a".into())));
    assert_eq!(p.add_group_parent("a", parent("a")), Err(PermsError::Cycle("a".into())));
    assert_eq!(p.add_group_parent("a", parent("nope")), Err(PermsError::NoGroup("nope".into())));
    assert_eq!(p.add_group_parent("a", parent("b")), Err(PermsError::Unchanged));
    // a loop that came in from a file still resolves
    p.edit_group("b", |g| g.data.add_parent(parent("a"))).unwrap();
    group(&mut p, "a", 0, &[perm("x", true)]);
    user_in(&mut p, U, &["b"]);
    assert_eq!(p.check(U, "x", &here(), 0), Some(true));
}

#[test]
fn meta_prefix_suffix_and_rank() {
    let mut p = Perms::new();
    group(&mut p, "vip", 10, &[]);
    group(&mut p, "admin", 100, &[]);
    p.edit_group("vip", |g| {
        g.display_name = "VIP".into();
        g.data.add_affix(MetaKind::Prefix, 10, "&a[VIP] ", &Contexts::global(), None, false);
        g.data.add_affix(MetaKind::Suffix, 5, " &7*", &Contexts::global(), None, false);
        g.data.set_meta("color", "green", &Contexts::global(), None);
    })
    .unwrap();
    p.edit_group("admin", |g| {
        g.data.add_affix(MetaKind::Prefix, 100, "&c[Admin] ", &Contexts::global(), None, false);
        g.data.set_meta("color", "red", &Contexts::global(), None);
    })
    .unwrap();
    user_in(&mut p, U, &["vip", "admin"]);
    let e = p.effective(U, &here(), 0).clone();
    assert_eq!(e.prefix.as_deref(), Some("&c[Admin] "));
    assert_eq!(e.suffix.as_deref(), Some(" &7*"));
    assert_eq!(e.meta.get("color").map(String::as_str), Some("red"), "admin is the more important holder");
    assert_eq!(e.primary, "admin");
    // the user's own prefix with a lower priority still loses to the group's
    p.edit_user(U, "Steve", |d| {
        d.add_affix(MetaKind::Prefix, 50, "&e[Me] ", &Contexts::global(), None, false);
        d.set_meta("color", "blue", &Contexts::global(), None);
    });
    let e = p.effective(U, &here(), 0).clone();
    assert_eq!(e.prefix.as_deref(), Some("&c[Admin] "));
    assert_eq!(e.meta.get("color").map(String::as_str), Some("blue"));
    // equal priority: the more important holder (the user) wins
    p.edit_user(U, "Steve", |d| d.add_affix(MetaKind::Prefix, 100, "&e[Me] ", &Contexts::global(), None, true));
    assert_eq!(p.effective(U, &here(), 0).prefix.as_deref(), Some("&e[Me] "));
    // a prefix only for another world does not apply
    p.edit_user(U, "Steve", |d| d.add_affix(MetaKind::Prefix, 999, "&5[End] ", &ctx("world=the_end"), None, false));
    assert_eq!(p.effective(U, &here(), 0).prefix.as_deref(), Some("&e[Me] "));
    assert_eq!(p.effective(U, &Query::new("lobby", "", "the_end"), 0).prefix.as_deref(), Some("&5[End] "));
    assert_eq!(p.display_name("vip"), "VIP");
    assert_eq!(p.display_name("admin"), "admin");
}

#[test]
fn primary_group_is_the_heaviest_direct_parent() {
    let mut p = Perms::new();
    group(&mut p, "vip", 10, &[]);
    group(&mut p, "staff", 50, &[]);
    group(&mut p, "owner", 1000, &[]);
    p.add_group_parent("staff", parent("owner")).unwrap();
    user_in(&mut p, U, &["vip", "staff"]);
    // owner is only inherited, so it is not the rank
    assert_eq!(p.effective(U, &here(), 0).primary, "staff");
    assert_eq!(p.effective(U2, &here(), 0).primary, "default");
}

#[test]
fn tracks_promote_and_demote() {
    let mut p = Perms::new();
    for (g, w) in [("member", 1), ("vip", 10), ("mvp", 20)] {
        group(&mut p, g, w, &[]);
    }
    p.create_track("ranks").unwrap();
    for g in ["member", "vip", "mvp"] {
        p.track_insert("ranks", g, None).unwrap();
    }
    assert_eq!(p.track_insert("ranks", "vip", None), Err(PermsError::OnTrack("vip".into())));
    let g = Contexts::global();
    let (s, _) = p.promote(U, "Steve", "ranks", &g).unwrap();
    assert_eq!((s.from, s.to.as_deref()), (None, Some("member")));
    let (s, _) = p.promote(U, "Steve", "ranks", &g).unwrap();
    assert_eq!((s.from.as_deref(), s.to.as_deref()), (Some("member"), Some("vip")));
    assert_eq!(p.effective(U, &here(), 0).primary, "vip");
    p.promote(U, "Steve", "ranks", &g).unwrap();
    assert_eq!(p.promote(U, "Steve", "ranks", &g).map(|_| ()), Err(PermsError::TrackEnd("mvp".into())));
    let (s, _) = p.demote(U, "Steve", "ranks", &g).unwrap();
    assert_eq!(s.to.as_deref(), Some("vip"));
    p.demote(U, "Steve", "ranks", &g).unwrap();
    assert_eq!(p.demote(U, "Steve", "ranks", &g).map(|_| ()), Err(PermsError::TrackStart("member".into())));
    assert_eq!(p.demote(U2, "Alex", "ranks", &g).map(|_| ()), Err(PermsError::NotOnTrack));
    // on two groups of a track at once: refuse to guess
    p.edit_user(U, "Steve", |d| d.add_parent(parent("mvp")));
    assert!(matches!(p.promote(U, "Steve", "ranks", &g), Err(PermsError::Ambiguous(_))));
    // tracks per context are separate ladders
    let lobby = ctx("server=lobby");
    let (s, _) = p.promote(U2, "Alex", "ranks", &lobby).unwrap();
    assert_eq!(s.to.as_deref(), Some("member"));
    assert_eq!(p.effective(U2, &here(), 0).primary, "member");
    assert_eq!(p.effective(U2, &Query::new("survival", "", "world"), 0).primary, "default");
    assert_eq!(p.promote(U, "S", "nope", &g).map(|_| ()), Err(PermsError::NoTrack("nope".into())));
    p.create_track("empty").unwrap();
    assert_eq!(p.promote(U, "S", "empty", &g).map(|_| ()), Err(PermsError::EmptyTrack("empty".into())));
}

#[test]
fn deleting_and_renaming_groups_cleans_references() {
    let mut p = Perms::new();
    group(&mut p, "vip", 10, &[perm("x", true)]);
    group(&mut p, "mvp", 20, &[]);
    p.add_group_parent("mvp", parent("vip")).unwrap();
    user_in(&mut p, U, &["vip"]);
    p.create_track("ranks").unwrap();
    p.track_insert("ranks", "vip", None).unwrap();
    let dirty = p.rename_group("vip", "gold").unwrap();
    assert!(dirty.deleted_groups.contains("vip") && dirty.groups.contains("gold") && dirty.groups.contains("mvp"));
    assert!(dirty.users.contains(U) && dirty.tracks.contains("ranks"));
    assert_eq!(p.check(U, "x", &here(), 0), Some(true));
    assert_eq!(p.check(U, "group.gold", &here(), 0), Some(true));
    let dirty = p.delete_group("gold").unwrap();
    assert!(dirty.users.contains(U) && dirty.groups.contains("mvp") && dirty.tracks.contains("ranks"));
    assert_eq!(p.check(U, "x", &here(), 0), None);
    assert_eq!(p.check(U, "group.default", &here(), 0), Some(true));
    assert!(p.track("ranks").unwrap().groups.is_empty());
    assert_eq!(p.delete_group("default"), Err(PermsError::DefaultGroup));
    assert_eq!(p.rename_group("default", "x"), Err(PermsError::DefaultGroup));
    assert_eq!(p.create_group("Bad Name"), Err(PermsError::BadName("Bad Name".into())));
    assert_eq!(p.create_group("mvp"), Err(PermsError::GroupExists("mvp".into())));
}

#[test]
fn names_and_lookup() {
    let mut p = Perms::new();
    assert!(p.seen(U, "Steve"));
    assert!(!p.seen(U, "Steve"));
    assert_eq!(p.find_user("steve").unwrap().uuid, U);
    assert_eq!(p.find_user(U).unwrap().name, "Steve");
    // renamed account: the old name no longer points to it
    assert!(p.seen(U, "Steve2"));
    assert!(p.find_user("steve").is_none());
    assert_eq!(p.find_user("STEVE2").unwrap().name, "Steve2");
    assert!(p.find_user("nobody").is_none());
    // a UUID is accepted even when never seen
    assert_eq!(p.find_user("00000000000000000000000000000009").unwrap().uuid, "00000000-0000-0000-0000-000000000009");
}

#[test]
fn cache_follows_changes() {
    let mut p = Perms::new();
    group(&mut p, "vip", 10, &[perm("a", true)]);
    user_in(&mut p, U, &["vip"]);
    assert_eq!(p.check(U, "a", &here(), 0), Some(true));
    let g0 = p.generation();
    p.edit_group("vip", |g| g.data.set_permission(perm("a", false))).unwrap();
    assert!(p.generation() > g0);
    assert_eq!(p.check(U, "a", &here(), 0), Some(false));
    // different places are cached separately
    p.edit_group("vip", |g| g.data.set_permission(perm_in("a", true, "world=nether"))).unwrap();
    assert_eq!(p.check(U, "a", &Query::new("lobby", "", "nether"), 0), Some(true));
    assert_eq!(p.check(U, "a", &here(), 0), Some(false));
}

#[test]
fn many_users_and_groups_stay_fast() {
    let mut p = Perms::new();
    for i in 0..50 {
        let name = format!("g{i}");
        let perms: Vec<PermNode> = (0..40).map(|j| perm(&format!("node.{i}.{j}"), j % 3 != 0)).collect();
        group(&mut p, &name, i, &perms);
        if i > 0 {
            p.add_group_parent(&name, parent(&format!("g{}", i - 1))).unwrap();
        }
    }
    user_in(&mut p, U, &["g49"]);
    let start = std::time::Instant::now();
    let mut yes = 0;
    for k in 0..20_000 {
        let n = format!("node.{}.{}", k % 50, k % 40);
        if p.check(U, &n, &here(), 0) == Some(true) {
            yes += 1;
        }
    }
    assert!(yes > 0);
    // generous bound: a check is a hash lookup once the user is cached
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
}
