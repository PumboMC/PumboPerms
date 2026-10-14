//! Taking over another permission source without losing anything
//! (PumboBridge spec §5.3): the proxy's table through PumboBridge, another
//! plugin that answers `{"op":"export-permissions"}`, or attachments other
//! plugins set.
//!
//! An import only adds: a group or user entry that is already there stays as
//! it is (a different value is a conflict, the existing one wins), weight and
//! prefixes only where the group has none. What one source added is kept in
//! its [`Ledger`] (the "imported from <source>" marker), so the next import
//! of that source takes back only the entries the source no longer has and
//! adds only new ones, and `undo` takes them all back; entries made in
//! PumboPerms are never touched. An imported entry an admin removed stays
//! in the ledger and does not come back with the next import.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::export::Snapshot;
use crate::model::{MetaNode, ParentNode, PermNode};
use crate::perms::{Dirty, Perms};

/// One added entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Entry {
    Group { group: String },
    GroupNode { group: String, node: PermNode },
    GroupParent { group: String, parent: ParentNode },
    GroupWeight { group: String, weight: i32 },
    GroupMeta { group: String, meta: MetaNode },
    UserNode { uuid: String, name: String, node: PermNode },
    UserParent { uuid: String, name: String, parent: ParentNode },
}

impl Entry {
    /// The same entry, whatever spelling of the player's name it carries (a
    /// join and the source may spell it differently).
    pub fn same(&self, other: &Entry) -> bool {
        match (self, other) {
            (Self::UserNode { uuid: a, node: x, .. }, Self::UserNode { uuid: b, node: y, .. }) => a == b && x == y,
            (Self::UserParent { uuid: a, parent: x, .. }, Self::UserParent { uuid: b, parent: y, .. }) => {
                a == b && x == y
            }
            _ => self == other,
        }
    }
}

/// What a source added last time (saved by the platform per source).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub fingerprint: String,
    /// Unix milliseconds of the import.
    pub at: u64,
    pub entries: Vec<Entry>,
    /// Players of the source known only by nickname: their entries are added
    /// when they join with an online-mode account ([`crate::command::import_pending`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<crate::model::User>,
    /// Nicknames of the source bound to a UUID (name key -> UUID): the first
    /// binding stays, a later owner of the name gets nothing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub entries: Vec<Entry>,
    /// Entries of the source that differ from existing ones (kept as they are).
    pub conflicts: Vec<String>,
}

fn same_node(a: &PermNode, b: &PermNode) -> bool {
    a.node == b.node && a.context == b.context && a.expiry.is_some() == b.expiry.is_some()
}

/// Every entry the source has, whatever the database holds.
pub fn candidates(snap: &Snapshot) -> Vec<Entry> {
    let mut out = Vec::new();
    for g in &snap.groups {
        let group = || g.name.clone();
        out.push(Entry::Group { group: group() });
        out.extend(g.data.permissions.iter().map(|n| Entry::GroupNode { group: group(), node: n.clone() }));
        out.extend(g.data.parents.iter().map(|p| Entry::GroupParent { group: group(), parent: p.clone() }));
        if g.weight != 0 {
            out.push(Entry::GroupWeight { group: group(), weight: g.weight });
        }
        out.extend(g.data.meta.iter().map(|m| Entry::GroupMeta { group: group(), meta: m.clone() }));
    }
    for u in &snap.users {
        let (uuid, name) = (|| u.uuid.clone(), || u.name.clone());
        out.extend(u.data.permissions.iter().map(|n| Entry::UserNode { uuid: uuid(), name: name(), node: n.clone() }));
        out.extend(u.data.parents.iter().map(|p| Entry::UserParent { uuid: uuid(), name: name(), parent: p.clone() }));
    }
    out
}

/// What importing `snap` adds to `perms`.
pub fn plan(perms: &Perms, snap: &Snapshot) -> Plan {
    let mut p = Plan::default();
    let known = |g: &str| perms.group(g).is_some() || snap.groups.iter().any(|s| s.name == g);
    let conflict = |kind: &str, who: &str, have: &PermNode, new: &PermNode| {
        format!("{kind} {who}: {} is {} here and {} in the source (kept)", new.node, have.value, new.value)
    };
    for entry in candidates(snap) {
        let add = match &entry {
            Entry::Group { group } => perms.group(group).is_none(),
            Entry::GroupNode { group, node } => {
                match perms.group(group).and_then(|g| g.data.permissions.iter().find(|x| same_node(x, node))) {
                    Some(x) if x.value == node.value => false,
                    Some(x) => {
                        p.conflicts.push(conflict("group", group, x, node));
                        false
                    }
                    None => true,
                }
            }
            Entry::GroupParent { group, parent } => {
                let there = perms.group(group).is_some_and(|g| {
                    g.data.parents.iter().any(|x| x.group == parent.group && x.context == parent.context)
                });
                !there && known(&parent.group)
            }
            Entry::GroupWeight { group, .. } => perms.group(group).is_none_or(|x| x.weight == 0),
            Entry::GroupMeta { group, meta } => !perms.group(group).is_some_and(|g| {
                g.data.meta.iter().any(|x| x.kind == meta.kind && x.key == meta.key && x.context == meta.context)
            }),
            Entry::UserNode { uuid, name, node } => {
                match perms.user(uuid).and_then(|u| u.data.permissions.iter().find(|x| same_node(x, node))) {
                    Some(x) if x.value == node.value => false,
                    Some(x) => {
                        p.conflicts.push(conflict("user", name, x, node));
                        false
                    }
                    None => true,
                }
            }
            Entry::UserParent { uuid, parent, .. } => {
                let there = perms.user(uuid).is_some_and(|u| {
                    u.data.parents.iter().any(|x| x.group == parent.group && x.context == parent.context)
                });
                !there && known(&parent.group)
            }
        };
        if add {
            p.entries.push(entry);
        }
    }
    p
}

/// Leaves out of `plan` what `recorded` (the last ledger of the source)
/// already holds, so an imported entry an admin removed does not come back,
/// and then parents in groups that neither exist nor come with the plan.
pub fn skip_recorded(plan: &mut Plan, perms: &Perms, recorded: &[Entry]) {
    plan.entries.retain(|x| !recorded.iter().any(|r| r.same(x)));
    let created: Vec<String> = plan
        .entries
        .iter()
        .filter_map(|x| match x {
            Entry::Group { group } => Some(group.clone()),
            _ => None,
        })
        .collect();
    plan.entries.retain(|x| match x {
        Entry::GroupParent { parent, .. } | Entry::UserParent { parent, .. } => {
            perms.group(&parent.group).is_some() || created.contains(&parent.group)
        }
        _ => true,
    });
}

/// Adds the planned entries; returns what changed and the entries really
/// added (a parent that would make a cycle is skipped).
pub fn apply(perms: &mut Perms, plan: &Plan) -> (Dirty, Vec<Entry>) {
    let mut dirty = Dirty::default();
    let mut done = Vec::new();
    for e in &plan.entries {
        let ok = match e {
            Entry::Group { group } => perms.create_group(group).map(|d| dirty.merge(d)).is_ok(),
            Entry::GroupNode { group, node } => {
                perms.edit_group(group, |g| g.data.set_permission(node.clone())).map(|(_, d)| dirty.merge(d)).is_ok()
            }
            Entry::GroupParent { group, parent } => {
                !perms.would_cycle(group, &parent.group)
                    && perms
                        .edit_group(group, |g| g.data.add_parent(parent.clone()))
                        .map(|(_, d)| dirty.merge(d))
                        .is_ok()
            }
            Entry::GroupWeight { group, weight } => {
                perms.edit_group(group, |g| g.weight = *weight).map(|(_, d)| dirty.merge(d)).is_ok()
            }
            Entry::GroupMeta { group, meta } => {
                perms.edit_group(group, |g| g.data.meta.push(meta.clone())).map(|(_, d)| dirty.merge(d)).is_ok()
            }
            Entry::UserNode { uuid, name, node } => {
                dirty.merge(perms.edit_user(uuid, name, |d| d.set_permission(node.clone())).1);
                true
            }
            Entry::UserParent { uuid, name, parent } => {
                dirty.merge(perms.edit_user(uuid, name, |d| d.add_parent(parent.clone())).1);
                true
            }
        };
        if ok {
            done.push(e.clone());
        }
    }
    (dirty, done)
}

/// Takes back what a source added, where it is still as imported; returns
/// what changed and how many entries went.
pub fn undo(perms: &mut Perms, ledger: &Ledger) -> (Dirty, usize) {
    let mut dirty = Dirty::default();
    let mut n = 0;
    // Groups last: an imported group may still be the parent of another one.
    let (groups, rest): (Vec<&Entry>, Vec<&Entry>) =
        ledger.entries.iter().rev().partition(|e| matches!(e, Entry::Group { .. }));
    for e in rest.into_iter().chain(groups) {
        let removed = match e {
            Entry::Group { group } => {
                // Only a group nothing refers to and nobody changed since.
                let untouched =
                    perms.group(group).is_some_and(|g| g.data.is_empty() && g.weight == 0 && g.display_name.is_empty());
                let unused = perms.members(group).is_empty()
                    && perms.child_groups(group).is_empty()
                    && perms.tracks_of(group).is_empty();
                untouched && unused && perms.delete_group(group).map(|d| dirty.merge(d)).is_ok()
            }
            Entry::GroupNode { group, node } => perms
                .edit_group(group, |g| {
                    let same = g.data.permissions.iter().any(|x| same_node(x, node) && x.value == node.value);
                    same && g.data.unset_permission(&node.node, &node.context, node.expiry.is_some()).is_some()
                })
                .map(|(r, d)| {
                    dirty.merge(d);
                    r
                })
                .unwrap_or(false),
            Entry::GroupParent { group, parent } => perms
                .edit_group(group, |g| g.data.remove_parent_exact(parent))
                .map(|(r, d)| {
                    dirty.merge(d);
                    r
                })
                .unwrap_or(false),
            Entry::GroupWeight { group, weight } => perms
                .edit_group(group, |g| {
                    let same = g.weight == *weight;
                    if same {
                        g.weight = 0;
                    }
                    same
                })
                .map(|(r, d)| {
                    dirty.merge(d);
                    r
                })
                .unwrap_or(false),
            Entry::GroupMeta { group, meta } => perms
                .edit_group(group, |g| {
                    let before = g.data.meta.len();
                    g.data.meta.retain(|m| m != meta);
                    before != g.data.meta.len()
                })
                .map(|(r, d)| {
                    dirty.merge(d);
                    r
                })
                .unwrap_or(false),
            Entry::UserNode { uuid, name, node } => {
                if perms.user(uuid).is_none() {
                    false
                } else {
                    let (r, d) = perms.edit_user(uuid, name, |data| {
                        let same = data.permissions.iter().any(|x| same_node(x, node) && x.value == node.value);
                        same && data.unset_permission(&node.node, &node.context, node.expiry.is_some()).is_some()
                    });
                    dirty.merge(d);
                    r
                }
            }
            Entry::UserParent { uuid, name, parent } => {
                if perms.user(uuid).is_none() {
                    false
                } else {
                    let (r, d) = perms.edit_user(uuid, name, |data| data.remove_parent_exact(parent));
                    dirty.merge(d);
                    r
                }
            }
        };
        n += usize::from(removed);
    }
    (dirty, n)
}

/// SHA-256-free fingerprint for sources that send none (attachments): the
/// text itself is short, so FNV-1a over it is enough to see a change.
pub fn fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Contexts;
    use crate::model::{Data, Group, User};

    fn node(n: &str, v: bool) -> PermNode {
        PermNode { node: n.into(), value: v, context: Contexts::global(), expiry: None }
    }

    fn snap() -> Snapshot {
        Snapshot {
            format: "pumboperms".into(),
            version: 1,
            exported: 0,
            groups: vec![
                Group {
                    name: "builder".into(),
                    display_name: String::new(),
                    weight: 10,
                    data: Data {
                        permissions: vec![node("minecraft:command.gamemode", true)],
                        parents: vec![ParentNode {
                            group: "default".into(),
                            context: Contexts::global(),
                            expiry: None,
                        }],
                        meta: vec![],
                    },
                },
                Group {
                    name: "default".into(),
                    display_name: String::new(),
                    weight: 0,
                    data: Data { permissions: vec![node("minecraft:command.help", true)], ..Data::default() },
                },
            ],
            users: vec![User {
                uuid: "10920508-d5d8-3eed-93d2-92f193afe7d7".into(),
                name: "Alice".into(),
                data: Data {
                    parents: vec![ParentNode { group: "builder".into(), context: Contexts::global(), expiry: None }],
                    ..Data::default()
                },
            }],
            tracks: vec![],
            by_name: vec![],
            proxy_rules: false,
            fixed: Vec::new(),
        }
    }

    #[test]
    fn imports_add_only_and_undo_takes_back_only_theirs() {
        let mut perms = Perms::new();
        // An entry made in PumboPerms that the source contradicts.
        perms.edit_group("default", |g| g.data.set_permission(node("minecraft:command.help", false))).unwrap();
        let before = perms.clone();
        let p = plan(&perms, &snap());
        assert_eq!(p.conflicts.len(), 1, "{:?}", p.conflicts);
        let (_, added) = apply(&mut perms, &p);
        assert!(added.contains(&Entry::Group { group: "builder".into() }));
        assert_eq!(perms.group("builder").unwrap().weight, 10);
        assert!(!perms.group("default").unwrap().data.permissions[0].value, "existing value wins");
        assert_eq!(perms.user("10920508-d5d8-3eed-93d2-92f193afe7d7").unwrap().data.parents[0].group, "builder");
        // The same source again adds nothing.
        assert!(plan(&perms, &snap()).entries.is_empty());
        // An admin adds to the imported group; undo keeps that and the group.
        perms.edit_group("builder", |g| g.data.set_permission(node("own.node", true))).unwrap();
        let ledger = Ledger { fingerprint: "f".into(), at: 0, entries: added, ..Ledger::default() };
        let (_, n) = undo(&mut perms, &ledger);
        assert!(n >= 4, "{n}");
        let builder = perms.group("builder").unwrap();
        assert_eq!(builder.data.permissions, vec![node("own.node", true)]);
        assert_eq!(builder.weight, 0);
        assert!(perms.user("10920508-d5d8-3eed-93d2-92f193afe7d7").is_none_or(|u| u.data.parents.is_empty()));
        assert_eq!(perms.group("default").unwrap().data, before.group("default").unwrap().data);
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }

    fn engine() -> crate::engine::Engine {
        use pumbo_common::lang::{COMMON, Lang};
        crate::engine::Engine::in_memory(
            crate::config::CoreCfg::default(),
            Lang::load(&[COMMON, crate::LANG], "en", None).0,
        )
    }

    #[test]
    fn import_command_runs_once_per_fingerprint_and_undoes() {
        use crate::command::{SourceMode, Who, import_source};
        let mut e = engine();
        let who = Who::console();
        let out = import_source(&mut e, &who, "proxy", Ok(("f1".into(), snap())), SourceMode::Run, 1);
        assert!(out.reply.plain().contains("Imported proxy"), "{}", out.reply.plain());
        assert_eq!(e.ledger("proxy").unwrap().fingerprint, "f1");
        let out = import_source(&mut e, &who, "proxy", Ok(("f1".into(), snap())), SourceMode::Run, 2);
        assert!(!out.changed && out.reply.plain().contains("has not changed"));
        let mut more = snap();
        more.groups[1].data.permissions.push(node("minecraft:command.list", true));
        let out = import_source(&mut e, &who, "proxy", Ok(("f2".into(), more)), SourceMode::Preview, 3);
        assert!(out.reply.plain().contains("would add 1 entries"), "{}", out.reply.plain());
        let out = import_source(&mut e, &who, "proxy", Err("unused".into()), SourceMode::Undo, 4);
        assert!(out.reply.plain().contains("Removed"), "{}", out.reply.plain());
        assert_eq!(e.ledger("proxy").unwrap(), Ledger::default());
        assert!(e.perms.group("builder").is_none(), "the imported group was empty again");
    }

    #[test]
    fn players_by_name_wait_for_an_online_join() {
        use crate::command::{SourceMode, Who, import_pending, import_source};
        let mut e = engine();
        let known = "00000000-0000-0000-0000-000000000002";
        let jeb = "00000000-0000-0000-0000-000000000003";
        e.seen(known, "Known", true).unwrap();
        let in_builder = Data {
            parents: vec![ParentNode { group: "builder".into(), context: Contexts::global(), expiry: None }],
            ..Data::default()
        };
        let mut s = snap();
        s.by_name = vec![
            User { uuid: String::new(), name: "Jeb_".into(), data: in_builder.clone() },
            User { uuid: String::new(), name: "known".into(), data: in_builder },
        ];
        let who = Who::console();
        let out = import_source(&mut e, &who, "file", Ok(("f1".into(), s)), SourceMode::Run, 1);
        let reply = out.reply.plain();
        assert!(reply.contains("premium) account: Jeb_"), "{reply}");
        assert!(reply.contains(&format!("known -> {known}")), "{reply}");
        assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
        assert_eq!(e.perms.user(known).unwrap().data.parents[0].group, "builder", "a known name is a UUID now");
        assert!(import_pending(&mut e, "file", known, "Known", true, 2).is_none());
        let out = import_pending(&mut e, "file", jeb, "JEB_", true, 3).unwrap();
        assert!(out.changed && out.reply.plain().contains("joined"), "{}", out.reply.plain());
        assert!(out.reply.plain().contains(&format!("JEB_ -> {jeb}")), "{}", out.reply.plain());
        assert_eq!(e.perms.user(jeb).unwrap().data.parents[0].group, "builder");
        let ledger = e.ledger("file").unwrap();
        assert!(
            ledger.pending.is_empty()
                && ledger.entries.iter().any(|x| matches!(x, Entry::UserParent { uuid, .. } if uuid == jeb))
        );
        import_source(&mut e, &who, "file", Err(String::new()), SourceMode::Undo, 4);
        assert_eq!(e.ledger("file").unwrap(), Ledger::default());
        assert!(e.perms.user(jeb).is_none_or(|u| u.data.parents.is_empty()), "undo takes it back");
    }

    #[test]
    fn undo_keeps_group_with_references() {
        let mut perms = Perms::new();
        let planned = plan(&perms, &snap());
        let (_, added) = apply(&mut perms, &planned);
        let ledger = Ledger { entries: added, ..Ledger::default() };
        // builder is on a track and another group inherits it.
        perms.create_track("ranks").unwrap();
        perms.track_insert("ranks", "builder", None).unwrap();
        perms.create_group("trial").unwrap();
        perms
            .edit_group("trial", |g| {
                g.data.add_parent(ParentNode { group: "builder".into(), context: Contexts::global(), expiry: None })
            })
            .unwrap();
        undo(&mut perms, &ledger);
        assert!(perms.group("builder").is_some(), "a group with references stays");
        assert_eq!(perms.track("ranks").unwrap().groups, vec!["builder".to_string()]);
        assert!(perms.group("trial").unwrap().data.parents.iter().any(|p| p.group == "builder"));
        // A display name given here keeps it as well.
        let mut perms = Perms::new();
        let planned = plan(&perms, &snap());
        let (_, added) = apply(&mut perms, &planned);
        perms.edit_group("builder", |g| g.display_name = "Builder".into()).unwrap();
        undo(&mut perms, &Ledger { entries: added, ..Ledger::default() });
        assert!(perms.group("builder").is_some());
    }

    #[test]
    fn rerun_does_not_restore_removed_entry() {
        use crate::command::{SourceMode, Who, import_source};
        let mut e = engine();
        let who = Who::console();
        import_source(&mut e, &who, "file", Ok(("f1".into(), snap())), SourceMode::Run, 1);
        // An admin takes Alice out of builder; the source changes elsewhere.
        let alice = "10920508-d5d8-3eed-93d2-92f193afe7d7";
        let (_, d) = e.perms.edit_user(alice, "Alice", |u| u.remove_parent("builder", &Contexts::global(), false));
        e.commit(&d, None).unwrap();
        let mut more = snap();
        more.groups[1].data.permissions.push(node("minecraft:command.list", true));
        let out = import_source(&mut e, &who, "file", Ok(("f2".into(), more)), SourceMode::Run, 2);
        assert!(out.reply.plain().contains("1 entries added"), "{}", out.reply.plain());
        assert!(e.perms.user(alice).is_none_or(|u| u.data.parents.is_empty()), "the removed entry stays removed");
        // An entry the source no longer has goes.
        let out = import_source(&mut e, &who, "file", Ok(("f3".into(), snap())), SourceMode::Run, 3);
        assert!(out.reply.plain().contains("1 entries the source no longer has"), "{}", out.reply.plain());
        let default = &e.perms.group("default").unwrap().data.permissions;
        assert!(!default.iter().any(|n| n.node == "minecraft:command.list"), "{default:?}");
    }
}
