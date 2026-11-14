//! Property tests: parsers never panic on any input, an import taken back
//! leaves the database as it was, and so does any change undone.

use proptest::prelude::*;
use proptest::test_runner::Config;

use crate::context::Contexts;
use crate::export::{self, Snapshot};
use crate::merge::{self, Ledger};
use crate::model::{Data, Group, MetaKind, MetaNode, ParentNode, PermNode, User};
use crate::node;
use crate::perms::Perms;

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
        merge::undo(&mut perms, &Ledger { entries: added, ..Ledger::default() });
        prop_assert_eq!(export::snapshot(&perms, 0), before);
    }
}
