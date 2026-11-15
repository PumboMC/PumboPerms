//! Property tests: parsers never panic on any input, an import taken back
//! leaves the database as it was, and so does any change undone; an engine
//! that reads players' records when needed answers like one with all of
//! them in memory.

use std::collections::BTreeSet;

use proptest::prelude::*;
use proptest::test_runner::Config;
use pumbo_common::lang::{COMMON, Lang};

use crate::command::{self, Env, SourceMode, Who};
use crate::config::CoreCfg;
use crate::context::Contexts;
use crate::engine::Engine;
use crate::export::{self, Snapshot};
use crate::merge::{self, Ledger};
use crate::model::{Data, Group, MetaKind, MetaNode, ParentNode, PermNode, User};
use crate::node;
use crate::perms::{KnownName, Perms};

fn cases() -> Config {
    Config { cases: 10_000, failure_persistence: None, ..Config::default() }
}

/// Free text, node-like text and arbitrary bytes.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-zA-Z0-9_.:*= -]{0,40}",
        proptest::collection::vec(any::<u8>(), 0..64).prop_map(|b| String::from_utf8_lossy(&b).into_owned()),
    ]
}

/// Keys of the export format, so random JSON reaches the deeper checks.
const KEYS: &[&str] = &[
    "name",
    "data",
    "permissions",
    "parents",
    "meta",
    "node",
    "value",
    "context",
    "expiry",
    "group",
    "uuid",
    "weight",
    "kind",
    "key",
    "priority",
    "groups",
    "users",
    "tracks",
    "by-name",
];

fn json() -> impl Strategy<Value = serde_json::Value> {
    use serde_json::Value;
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        text().prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            proptest::collection::btree_map(prop_oneof![text(), pick(KEYS),], inner, 0..8)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn pick(items: &'static [&'static str]) -> impl Strategy<Value = String> {
    proptest::sample::select(items).prop_map(String::from)
}

fn contexts() -> impl Strategy<Value = Contexts> {
    pick(&["", "server=lobby", "world=nether"])
        .prop_map(|c| if c.is_empty() { Contexts::global() } else { Contexts::parse(&[c]).unwrap_or_default() })
}

fn expiry() -> impl Strategy<Value = Option<u64>> {
    prop_oneof![Just(None), Just(Some(4_000_000_000_000))]
}

fn data() -> impl Strategy<Value = Data> {
    let nodes = pick(&["a", "a.b", "a.*", "*", "minecraft:command.give", "minecraft:*", "pumbo.bans.ban"]);
    let perm = (nodes, any::<bool>(), contexts(), expiry()).prop_map(|(node, value, context, expiry)| PermNode {
        node,
        value,
        context,
        expiry,
    });
    let parent = (pick(&["default", "vip", "mod"]), contexts(), expiry())
        .prop_map(|(group, context, expiry)| ParentNode { group, context, expiry });
    let meta = (
        proptest::sample::select(vec![MetaKind::Prefix, MetaKind::Suffix, MetaKind::Meta]),
        0..3i32,
        pick(&["[VIP] ", "x"]),
        contexts(),
        expiry(),
    )
        .prop_map(|(kind, priority, value, context, expiry)| MetaNode {
            key: if kind == MetaKind::Meta { "color".into() } else { String::new() },
            kind,
            priority,
            value,
            context,
            expiry,
        });
    (
        proptest::collection::vec(perm, 0..4),
        proptest::collection::vec(parent, 0..3),
        proptest::collection::vec(meta, 0..2),
    )
        .prop_map(|(permissions, parents, meta)| Data { permissions, parents, meta })
}

fn snapshot() -> impl Strategy<Value = Snapshot> {
    let group = (pick(&["default", "vip", "mod"]), 0..3i32, data()).prop_map(|(name, weight, data)| Group {
        name,
        display_name: String::new(),
        weight,
        data,
    });
    let user = (pick(&["00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"]), data())
        .prop_map(|(uuid, data)| User { name: format!("P{}", &uuid[uuid.len() - 1..]), uuid, data });
    (proptest::collection::vec(group, 0..4), proptest::collection::vec(user, 0..3)).prop_map(|(groups, users)| {
        Snapshot { format: export::FORMAT.into(), version: export::VERSION, groups, users, ..Snapshot::default() }
    })
}

/// One change of the database.
#[derive(Debug, Clone)]
enum Change {
    CreateGroup(String),
    DeleteGroup(String),
    RenameGroup(String, String),
    GroupNode(String, PermNode),
    GroupParent(String, ParentNode),
    UserNode(String, PermNode),
    UserParent(String, ParentNode),
    ClearUser(String),
    CreateTrack(String),
    DeleteTrack(String),
    TrackInsert(String, String),
    TrackRemove(String, String),
    TrackClear(String),
    Step(String, String, bool),
    Expire(u64),
}

const GROUPS: &[&str] = &["default", "vip", "mod", "new"];
const PLAYERS: &[&str] = &[
    "00000000-0000-0000-0000-000000000001",
    "00000000-0000-0000-0000-000000000002",
    "00000000-0000-0000-0000-000000000003",
];
const TRACKS: &[&str] = &["ranks", "other"];

/// Nothing, or an end within the first 200 ms.
fn ending() -> impl Strategy<Value = Option<u64>> {
    prop_oneof![Just(None), (1u64..200).prop_map(Some)]
}

fn change() -> impl Strategy<Value = Change> {
    let perm = || {
        (pick(&["a", "a.b", "*"]), any::<bool>(), contexts(), ending())
            .prop_map(|(node, value, context, expiry)| PermNode { node, value, context, expiry })
    };
    let parent = || {
        (pick(GROUPS), contexts(), ending()).prop_map(|(group, context, expiry)| ParentNode { group, context, expiry })
    };
    prop_oneof![
        pick(GROUPS).prop_map(Change::CreateGroup),
        pick(GROUPS).prop_map(Change::DeleteGroup),
        (pick(GROUPS), pick(GROUPS)).prop_map(|(a, b)| Change::RenameGroup(a, b)),
        (pick(GROUPS), perm()).prop_map(|(g, n)| Change::GroupNode(g, n)),
        (pick(GROUPS), parent()).prop_map(|(g, p)| Change::GroupParent(g, p)),
        (pick(PLAYERS), perm()).prop_map(|(u, n)| Change::UserNode(u, n)),
        (pick(PLAYERS), parent()).prop_map(|(u, p)| Change::UserParent(u, p)),
        pick(PLAYERS).prop_map(Change::ClearUser),
        pick(TRACKS).prop_map(Change::CreateTrack),
        pick(TRACKS).prop_map(Change::DeleteTrack),
        (pick(TRACKS), pick(GROUPS)).prop_map(|(t, g)| Change::TrackInsert(t, g)),
        (pick(TRACKS), pick(GROUPS)).prop_map(|(t, g)| Change::TrackRemove(t, g)),
        pick(TRACKS).prop_map(Change::TrackClear),
        (pick(PLAYERS), pick(TRACKS), any::<bool>()).prop_map(|(u, t, up)| Change::Step(u, t, up)),
        (0u64..250).prop_map(Change::Expire),
    ]
}

/// Makes one change; a refused one changes nothing.
fn make(p: &mut Perms, change: &Change) {
    match change {
        Change::CreateGroup(g) => drop(p.create_group(g)),
        Change::DeleteGroup(g) => drop(p.delete_group(g)),
        Change::RenameGroup(a, b) => drop(p.rename_group(a, b)),
        Change::GroupNode(g, n) => drop(p.edit_group(g, |g| g.data.set_permission(n.clone()))),
        Change::GroupParent(g, parent) => drop(p.add_group_parent(g, parent.clone())),
        Change::UserNode(u, n) => drop(p.edit_user(u, "P", |d| d.set_permission(n.clone()))),
        Change::UserParent(u, parent) => drop(p.edit_user(u, "P", |d| d.add_parent(parent.clone()))),
        Change::ClearUser(u) => drop(p.edit_user(u, "", |d| *d = Data::default())),
        Change::CreateTrack(t) => drop(p.create_track(t)),
        Change::DeleteTrack(t) => drop(p.delete_track(t)),
        Change::TrackInsert(t, g) => drop(p.track_insert(t, g, None)),
        Change::TrackRemove(t, g) => drop(p.track_remove(t, g)),
        Change::TrackClear(t) => drop(p.track_clear(t)),
        Change::Step(u, t, true) => drop(p.promote(u, "P", t, &Contexts::global(), 100)),
        Change::Step(u, t, false) => drop(p.demote(u, "P", t, &Contexts::global(), 100)),
        Change::Expire(now) => drop(p.expire(*now)),
    }
}

/// The export of the database and every user record, also one without data.
fn all_of(p: &Perms) -> (Snapshot, Vec<User>) {
    let mut users: Vec<User> = p.users().cloned().collect();
    users.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    (export::snapshot(p, 0), users)
}

proptest! {
    #![proptest_config(cases())]

    #[test]
    fn any_change_can_be_undone(base in snapshot(), changes in proptest::collection::vec(change(), 1..12)) {
        let mut perms = Perms::new();
        let setup = merge::plan(&perms, &base);
        merge::apply(&mut perms, &setup);
        for (track, groups) in [("ranks", ["default", "vip"]), ("other", ["mod", "vip"])] {
            perms.create_track(track).ok();
            for g in groups {
                perms.track_insert(track, g, None).ok();
            }
        }
        let before = all_of(&perms);
        perms.begin();
        for c in &changes {
            make(&mut perms, c);
        }
        perms.rollback();
        prop_assert_eq!(all_of(&perms), before);
    }

    #[test]
    fn node_parse_never_panics(input in text()) {
        if let Err(e) = node::normalize(&input) {
            prop_assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn context_parse_never_panics(args in proptest::collection::vec(text(), 0..4)) {
        if let Err(e) = Contexts::parse(&args) {
            prop_assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn export_parse_never_panics(raw in text(), value in json()) {
        let wrapped = serde_json::json!({
            "format": export::FORMAT, "version": export::VERSION,
            "groups": [value.clone()], "users": [value.clone()], "tracks": [value.clone()], "by-name": [value.clone()],
        });
        for input in [raw, value.to_string(), wrapped.to_string()] {
            match export::parse(&input) {
                Ok(snap) => prop_assert!(snap.fixed.iter().all(|f| !f.contains(['\n', '\r'])), "{:?}", snap.fixed),
                Err(e) => prop_assert!(!e.is_empty() && !e.contains(['\n', '\r']), "{e}"),
            }
        }
    }

    #[test]
    fn plan_apply_undo_restores_state(base in snapshot(), source in snapshot()) {
        let mut perms = Perms::new();
        let setup = merge::plan(&perms, &base);
        merge::apply(&mut perms, &setup);
        let before = export::snapshot(&perms, 0);
        let plan = merge::plan(&perms, &source);
        let (_, added) = merge::apply(&mut perms, &plan);
        merge::undo(&mut perms, &Ledger { entries: added, ..Ledger::default() }, 0);
        prop_assert_eq!(export::snapshot(&perms, 0), before);
    }
}

/// Players of [`lazy_and_eager_agree`]: more than the records a lazy engine
/// keeps of players who are not online, so records go and come back.
const PEOPLE: usize = 130;

fn person(i: usize) -> (String, String) {
    (format!("00000000-0000-4000-8000-{i:012}"), format!("P{i}"))
}

/// One step of [`lazy_and_eager_agree`].
#[derive(Debug, Clone)]
enum Step {
    /// `/pp <line>` from the console.
    Pp(String),
    Join(usize),
    Leave(usize),
    /// Time passes (ms).
    Wait(u64),
    /// Another plugin asks about a player.
    Ask(usize),
    /// The source `file` gives 110 players `group` (and every `every`th one a node).
    Import(SourceMode, String, usize),
    Export,
}

fn step() -> impl Strategy<Value = Step> {
    let who = || (0..PEOPLE).prop_map(|i| person(i).0);
    let group = || pick(&["default", "vip", "mod", "gold"]);
    let node = || pick(&["fly", "build", "kit.*", "pumbo.perms.user.info"]);
    let user = prop_oneof![
        (node(), any::<bool>()).prop_map(|(n, v)| format!("permission set {n} {v}")),
        (node(), pick(&["1s", "2s"])).prop_map(|(n, d)| format!("permission settemp {n} true {d}")),
        node().prop_map(|n| format!("permission unset {n}")),
        group().prop_map(|g| format!("parent add {g}")),
        (group(), pick(&["1s", "3s"])).prop_map(|(g, d)| format!("parent addtemp {g} {d}")),
        group().prop_map(|g| format!("parent remove {g}")),
        group().prop_map(|g| format!("parent set {g}")),
        Just("meta set color red".to_string()),
        Just("meta setprefix 5 \"[x] \"".to_string()),
        Just("clear".to_string()),
        Just("promote ranks".to_string()),
        Just("demote ranks".to_string()),
        Just("info".to_string()),
        Just("parent info".to_string()),
        node().prop_map(|n| format!("permission check {n}")),
    ];
    let group_cmd = prop_oneof![
        Just("create".to_string()),
        Just("delete".to_string()),
        group().prop_map(|g| format!("rename {g}")),
        node().prop_map(|n| format!("permission set {n}")),
        group().prop_map(|g| format!("parent add {g}")),
        Just("info".to_string()),
        (1u32..4).prop_map(|p| format!("listmembers {p}")),
    ];
    let track = prop_oneof![
        Just("createtrack ranks".to_string()),
        Just("deletetrack ranks".to_string()),
        group().prop_map(|g| format!("track ranks append {g}")),
        group().prop_map(|g| format!("track ranks remove {g}")),
    ];
    let mode = proptest::sample::select(vec![SourceMode::Run, SourceMode::Preview, SourceMode::Undo]);
    prop_oneof![
        6 => (who(), user).prop_map(|(u, c)| Step::Pp(format!("user {u} {c}"))),
        3 => (group(), group_cmd).prop_map(|(g, c)| Step::Pp(format!("group {g} {c}"))),
        1 => track.prop_map(Step::Pp),
        2 => (0..PEOPLE).prop_map(Step::Join),
        2 => (0..PEOPLE).prop_map(Step::Leave),
        2 => (0u64..1_500).prop_map(Step::Wait),
        2 => (0..PEOPLE).prop_map(Step::Ask),
        1 => (mode, pick(&["vip", "mod", "gold"]), 1usize..4).prop_map(|(m, g, n)| Step::Import(m, g, n)),
        1 => Just(Step::Export),
    ]
}

/// What a step answers: replies, outcomes and the answers to another plugin.
fn answer(e: &mut Engine, step: &Step, online: &BTreeSet<usize>, now: u64, turn: usize) -> Vec<String> {
    let names: Vec<KnownName> =
        online.iter().map(|i| KnownName { uuid: person(*i).0, name: person(*i).1, online: true }).collect();
    let lookup = |name: &str| names.iter().find(|k| k.name.eq_ignore_ascii_case(name)).cloned();
    match step {
        Step::Pp(line) => {
            let env = Env { now, online: &lookup, version: "0", version_rows: Vec::new() };
            vec![format!("{:?}", command::run(e, &Who::console(), &command::split_args(line), &env))]
        }
        Step::Join(i) => {
            let (uuid, name) = person(*i);
            let mut out = vec![format!("{:?}", e.seen(&uuid, &name, true, now))];
            out.extend(command::on_join(e, &uuid, &name, true, now).iter().map(|o| format!("{o:?}")));
            out
        }
        Step::Leave(i) => {
            e.gone(&person(*i).0);
            Vec::new()
        }
        Step::Wait(_) => Vec::new(),
        Step::Ask(i) => {
            let uuid = person(*i).0;
            let mut out = Vec::new();
            for request in [
                format!(r#"{{"op":"info","uuid":"{uuid}"}}"#),
                format!(r#"{{"op":"check","uuid":"{uuid}","node":"fly"}}"#),
                format!(r#"{{"op":"check","uuid":"{uuid}","node":"kit.a"}}"#),
            ] {
                out.push(String::from_utf8_lossy(&crate::ipc::handle(e, request.as_bytes(), now, "0")).into_owned());
            }
            out.push(format!("{:?}", crate::host::decisions(e, &uuid, "", now)));
            out
        }
        Step::Import(mode, group, every) => {
            let parent = |g: &str| ParentNode { group: g.to_string(), context: Contexts::global(), expiry: None };
            let users = (0..110)
                .map(|i| {
                    let (uuid, name) = person(i);
                    let mut data = Data { parents: vec![parent(group)], ..Data::default() };
                    if i % every == 0 {
                        data.permissions.push(PermNode {
                            node: format!("kit.{group}"),
                            value: true,
                            context: Contexts::global(),
                            expiry: None,
                        });
                    }
                    User { uuid, name, data }
                })
                .collect();
            let snap = Snapshot {
                format: export::FORMAT.into(),
                version: export::VERSION,
                groups: vec![Group::new(group)],
                users,
                ..Snapshot::default()
            };
            let source = Ok((format!("f{turn}"), snap));
            vec![format!("{:?}", command::import_source(e, &Who::console(), "file", source, *mode, now))]
        }
        Step::Export => vec![exported(e, now)],
    }
}

/// The export of the database, as `/pp export` writes it.
fn exported(e: &Engine, now: u64) -> String {
    let mut out = Vec::new();
    let written = e.store().map(|s| export::write(&mut out, &e.perms, s, now));
    format!("{written:?} {}", String::from_utf8_lossy(&out))
}

/// Both engines with the same start: every player joined once (the names are
/// in the database), and 80 of them have a group from an earlier import.
fn engines() -> [Engine; 2] {
    let lang = || Lang::load(&[COMMON, crate::LANG], "en", None).0;
    let mut both = [Engine::in_memory(CoreCfg::default(), lang()), Engine::lazy(CoreCfg::default(), lang())];
    for e in &mut both {
        for i in 0..PEOPLE {
            let (uuid, name) = person(i);
            e.seen(&uuid, &name, true, 0).ok();
            e.gone(&uuid);
        }
        let earlier = Snapshot {
            format: export::FORMAT.into(),
            version: export::VERSION,
            groups: vec![Group::new("mod"), Group::new("vip")],
            users: (50..PEOPLE)
                .map(|i| {
                    let parent = ParentNode { group: "mod".into(), context: Contexts::global(), expiry: None };
                    User {
                        uuid: person(i).0,
                        name: person(i).1,
                        data: Data { parents: vec![parent], ..Data::default() },
                    }
                })
                .collect(),
            ..Snapshot::default()
        };
        command::import_source(e, &Who::console(), "earlier", Ok(("e1".into(), earlier)), SourceMode::Run, 0);
    }
    both
}

proptest! {
    #![proptest_config(Config { cases: 64, failure_persistence: None, ..Config::default() })]

    #[test]
    fn lazy_and_eager_agree(steps in proptest::collection::vec(step(), 1..40)) {
        let [mut eager, mut lazy] = engines();
        let (mut now, mut online) = (1_000u64, BTreeSet::new());
        for (turn, step) in steps.iter().enumerate() {
            if let Step::Wait(ms) = step {
                now += ms;
            }
            // As from the platform, before every step.
            let _ = (eager.expire(now), lazy.expire(now));
            let want = answer(&mut eager, step, &online, now, turn);
            let got = answer(&mut lazy, step, &online, now, turn);
            prop_assert_eq!(&got, &want, "step {}: {:?}", turn, step);
            match step {
                Step::Join(i) => drop(online.insert(*i)),
                Step::Leave(i) => drop(online.remove(i)),
                _ => {}
            }
        }
        prop_assert_eq!(exported(&lazy, now), exported(&eager, now));
    }
}
