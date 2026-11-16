//! Property tests of the staff limits.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::Config;
use serde_json::{Value, json};

use super::*;
use crate::config::LimitsCfg;
use crate::context::Query;
use crate::ipc;

const MOD: &str = "00000000-0000-0000-0000-0000000000e1";
const PLAYERS: [(&str, &str); 3] = [("Steve", STEVE), ("Alex", ALEX), ("Mod", MOD)];
const GROUPS: [&str; 6] = ["staff", "mod", "helper", "vip", "admin", "default"];
const CONTEXTS: [&str; 4] = ["", "server=survival", "server=lobby world=nether", "world=nether"];
/// The groups the moderator may be in; the admin closes each to the moderator's own changes.
const OWN: [&str; 4] = ["staff", "mod", "helper", "default"];
/// The actions the moderator has, each with every argument, target and context
/// but the moderator and the moderator's groups.
const RIGHTS: [&str; 14] = [
    "user.parent.add",
    "user.parent.remove",
    "user.parent.set",
    "user.promote",
    "user.demote",
    "user.clear",
    "user.permission.set",
    "user.meta.set",
    "group.parent.add",
    "group.parent.remove",
    "group.parent.set",
    "group.permission.set",
    "group.meta.set",
    "track.edit",
];
/// Until when a temporary membership lasts.
const LATER: u64 = 1_000 + 3_600_000;

/// Groups a player starts in: (group, contexts, temporary).
type Parents = Vec<(&'static str, &'static str, bool)>;

fn contexts(c: &str) -> Value {
    let pairs = c.split_whitespace().filter_map(|kv| kv.split_once('='));
    Value::Object(pairs.map(|(k, v)| (k.to_string(), Value::from(v))).collect())
}

fn parents(list: &Parents) -> Vec<Value> {
    let parent = |&(group, ctx, temp): &(&str, &str, bool)| {
        let mut p = json!({ "group": group, "context": contexts(ctx) });
        if temp {
            p["expiry"] = LATER.into();
        }
        p
    };
    list.iter().map(parent).collect()
}

/// The data as one import: groups (`admin` with every PumboPerms node), the
/// track `ranks`, the moderator's nodes and everyone's groups.
fn setup(moderator: &Parents, mod_has_helper: bool, steve: &Parents, alex: &Parents) -> String {
    let node = |n: String, v: bool| json!({ "node": n, "value": v });
    let mut nodes = vec![node("pumbo.perms.modify.user.others".into(), true)];
    nodes.push(node("pumbo.perms.usecontext.global".into(), true));
    for a in RIGHTS {
        nodes.push(node(format!("pumbo.perms.{a}"), true));
        nodes.push(node(format!("pumbo.perms.{a}.*"), true));
        nodes.push(node(format!("pumbo.perms.{a}.modify.self"), false));
        nodes.extend(OWN.map(|g| node(format!("pumbo.perms.{a}.modify.{g}"), false)));
    }
    let mod_group = if mod_has_helper { json!({ "parents": [{ "group": "helper" }] }) } else { json!({}) };
    json!({
        "format": "pumboperms",
        "version": 1,
        "groups": [
            { "name": "staff" }, { "name": "mod", "data": mod_group }, { "name": "helper" }, { "name": "vip" },
            { "name": "admin", "weight": 100, "data": { "permissions": [{ "node": "pumbo.perms.*", "value": true }] } },
        ],
        "users": [
            { "uuid": MOD, "name": "Mod", "data": { "permissions": nodes, "parents": parents(moderator) } },
            { "uuid": STEVE, "name": "Steve", "data": { "parents": parents(steve) } },
            { "uuid": ALEX, "name": "Alex", "data": { "parents": parents(alex) } },
        ],
        "tracks": [{ "name": "ranks", "groups": ["helper", "mod", "admin"] }],
    })
    .to_string()
}

/// The places the oracle looks at.
fn places() -> Vec<Query> {
    let mut out = Vec::new();
    for server in ["", "lobby", "survival"] {
        for world in ["", "nether"] {
            out.push(Query::at(server, Vec::new(), world));
        }
    }
    out
}

fn groups_at(e: &mut Engine, uuid: &str, q: &Query) -> BTreeSet<String> {
    ipc::info(e, uuid, q, 1_000).map(|i| i.groups.into_iter().collect()).unwrap_or_default()
}

/// Everything the console shows of the moderator and of the groups the moderator inherits anywhere.
fn moderator_data(e: &mut Engine) -> Vec<String> {
    let mut groups = BTreeSet::new();
    for q in places() {
        groups.extend(groups_at(e, MOD, &q));
    }
    let mut views: Vec<String> = ["info", "permission info", "parent info", "meta info"]
        .iter()
        .map(|v| console(e, &format!("user Mod {v}")))
        .collect();
    for g in groups {
        for v in ["info", "permission info", "parent info", "meta info"] {
            // Members are the players' data, tracks their own.
            let view = console(e, &format!("group {g} {v}"));
            let own = |l: &&str| !["Members:", "Tracks:"].iter().any(|p| l.trim_start().starts_with(p));
            views.push(view.lines().filter(own).collect::<Vec<_>>().join("\n"));
        }
    }
    views
}

fn parents_strategy() -> impl Strategy<Value = Parents> {
    let group = select(&["staff", "mod", "helper", "vip", "admin"][..]);
    proptest::collection::vec((group, select(CONTEXTS.to_vec()), any::<bool>()), 0..3)
}

fn own_strategy() -> impl Strategy<Value = Parents> {
    proptest::collection::vec((select(OWN[..3].to_vec()), select(CONTEXTS.to_vec()), any::<bool>()), 0..3)
}

/// A command the moderator tries.
fn step() -> impl Strategy<Value = String> {
    let player = || select(&["Steve", "Alex", "Mod"][..]);
    let group = || select(GROUPS.to_vec());
    let ctx = || select(CONTEXTS.to_vec()).prop_map(|c| if c.is_empty() { String::new() } else { format!(" {c}") });
    prop_oneof![
        (player(), select(&["add", "addtemp", "remove", "removetemp", "set"][..]), group(), ctx()).prop_map(
            |(p, verb, g, c)| {
                let dur = if verb == "addtemp" { " 1h" } else { "" };
                format!("user {p} parent {verb} {g}{dur}{c}")
            }
        ),
        player().prop_map(|p| format!("user {p} clear")),
        (select(&["promote", "demote"][..]), player(), ctx()).prop_map(|(v, p, c)| format!("{v} {p} ranks{c}")),
        (group(), select(&["add", "set", "remove"][..]), group(), ctx())
            .prop_map(|(g, v, h, c)| format!("group {g} parent {v} {h}{c}")),
        (select(&["append", "remove"][..]), group()).prop_map(|(v, g)| format!("track ranks {v} {g}")),
        (group(), 1..4usize).prop_map(|(g, i)| format!("track ranks insert {g} {i}")),
        (group(), ctx()).prop_map(|(g, c)| format!("group {g} permission set pumbo.perms.*{c}")),
        (group(), ctx()).prop_map(|(g, c)| format!("group {g} meta set weight 100{c}")),
        (player(), select(&["group.admin", "pumbo.perms.*", "*"][..]))
            .prop_map(|(p, n)| format!("user {p} permission set {n}")),
    ]
}

proptest! {
    #![proptest_config(Config { cases: 1000, failure_persistence: None, ..Config::default() })]

    /// With both switches on, whatever the moderator tries: the moderator and
    /// the moderator's groups stay as they were, and every group another
    /// player gains somewhere is one the moderator has there.
    #[test]
    fn limited_staff_never_hands_out_a_foreign_group(
        moderator in own_strategy(),
        mod_has_helper in any::<bool>(),
        steve in parents_strategy(),
        alex in parents_strategy(),
        steps in proptest::collection::vec(step(), 1..16),
    ) {
        let mut e = engine();
        e.seen(MOD, "Mod", true, 0).unwrap();
        let json = setup(&moderator, mod_has_helper, &steve, &alex);
        let out = command::import(&mut e, &Who::console(), "setup", &json, "none", 0);
        prop_assert!(out.changed, "{}", out.reply.plain());
        e.cfg.limits = LimitsCfg { argument_permissions: true, require_group_membership: true };
        let who = Who::player("Mod", MOD, 0, "world");
        let before_mod = moderator_data(&mut e);
        let mut before: BTreeMap<(&str, usize), BTreeSet<String>> = BTreeMap::new();
        for (name, uuid) in &PLAYERS[..2] {
            for (i, q) in places().iter().enumerate() {
                before.insert((*name, i), groups_at(&mut e, uuid, q));
            }
        }
        for line in &steps {
            run_as(&mut e, &who, line, 1_000);
        }
        prop_assert_eq!(moderator_data(&mut e), before_mod, "the moderator or their groups changed: {:?}", steps);
        for (name, uuid) in &PLAYERS[..2] {
            for (i, q) in places().iter().enumerate() {
                let mine = groups_at(&mut e, MOD, q);
                for g in groups_at(&mut e, uuid, q).difference(&before[&(*name, i)]) {
                    prop_assert!(g == "default" || mine.contains(g), "{} gained {} at {:?} after {:?}", name, g, q, steps);
                }
            }
        }
    }
}
