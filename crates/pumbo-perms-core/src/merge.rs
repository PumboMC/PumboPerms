//! Taking over another permission source without losing anything
//! (PumboBridge spec §5.3): the proxy's table through PumboBridge, another
//! plugin that answers `{"op":"export-permissions"}`, or attachments other
//! plugins set.
//!
//! An import only adds: a group or user entry that is already there stays as
//! it is (a different value is a conflict, the existing one wins), weight and
//! prefixes only where the group has none. What one source added is kept in
//! its [`Ledger`] (the "imported from <source>" marker), so the next import
//! of that source replaces exactly those entries, and `undo` takes them back;
//! entries made in PumboPerms are never touched.

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

/// What a source added last time (saved by the platform per source).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub fingerprint: String,
    /// Unix milliseconds of the import.
    pub at: u64,
    pub entries: Vec<Entry>,
    /// Players of the source known only by nickname: their entries are added
    /// at their first join ([`crate::command::import_pending`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<crate::model::User>,
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

/// What importing `snap` adds to `perms`.
pub fn plan(perms: &Perms, snap: &Snapshot) -> Plan {
    let mut p = Plan::default();
    let known = |g: &str| perms.group(g).is_some() || snap.groups.iter().any(|s| s.name == g);
    for g in &snap.groups {
        let existing = perms.group(&g.name);
        if existing.is_none() {
            p.entries.push(Entry::Group { group: g.name.clone() });
        }
        let have = existing.map(|x| &x.data);
        for n in &g.data.permissions {
            match have.and_then(|d| d.permissions.iter().find(|x| same_node(x, n))) {
                Some(x) if x.value == n.value => {}
                Some(x) => p.conflicts.push(format!(
                    "group {}: {} is {} here and {} in the source (kept)",
                    g.name, n.node, x.value, n.value
                )),
                None => p.entries.push(Entry::GroupNode { group: g.name.clone(), node: n.clone() }),
            }
        }
        for par in &g.data.parents {
            let there =
                have.is_some_and(|d| d.parents.iter().any(|x| x.group == par.group && x.context == par.context));
            if !there && known(&par.group) {
                p.entries.push(Entry::GroupParent { group: g.name.clone(), parent: par.clone() });
            }
        }
        if g.weight != 0 && existing.is_none_or(|x| x.weight == 0) {
            p.entries.push(Entry::GroupWeight { group: g.name.clone(), weight: g.weight });
        }
        for m in &g.data.meta {
            let there = have
                .is_some_and(|d| d.meta.iter().any(|x| x.kind == m.kind && x.key == m.key && x.context == m.context));
            if !there {
                p.entries.push(Entry::GroupMeta { group: g.name.clone(), meta: m.clone() });
            }
        }
    }
    for u in &snap.users {
        let have = perms.user(&u.uuid).map(|x| &x.data);
        for n in &u.data.permissions {
            match have.and_then(|d| d.permissions.iter().find(|x| same_node(x, n))) {
                Some(x) if x.value == n.value => {}
                Some(x) => p.conflicts.push(format!(
                    "user {}: {} is {} here and {} in the source (kept)",
                    u.name, n.node, x.value, n.value
                )),
                None => p.entries.push(Entry::UserNode { uuid: u.uuid.clone(), name: u.name.clone(), node: n.clone() }),
            }
        }
        for par in &u.data.parents {
            let there =
                have.is_some_and(|d| d.parents.iter().any(|x| x.group == par.group && x.context == par.context));
            if !there && known(&par.group) {
                p.entries.push(Entry::UserParent { uuid: u.uuid.clone(), name: u.name.clone(), parent: par.clone() });
            }
        }
    }
    p
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
    for e in ledger.entries.iter().rev() {
        let removed = match e {
            Entry::Group { group } => {
                let empty = perms.group(group).is_some_and(|g| g.data.is_empty()) && perms.members(group).is_empty();
                empty && perms.delete_group(group).map(|d| dirty.merge(d)).is_ok()
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
                .edit_group(group, |g| g.data.remove_parent(&parent.group, &parent.context))
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
                    let (r, d) = perms.edit_user(uuid, name, |data| data.remove_parent(&parent.group, &parent.context));
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

    #[test]
    fn import_command_runs_once_per_fingerprint_and_undoes() {
        use crate::command::{SourceMode, Who, import_source};
        use pumbo_common::lang::{COMMON, Lang};
        let mut e = crate::engine::Engine::in_memory(
            crate::config::CoreCfg::default(),
            Lang::load(&[COMMON, crate::LANG], "en", None).0,
        );
        let who = Who::console();
        let (out, ledger) =
            import_source(&mut e, &who, "proxy", Ok(("f1".into(), snap())), &Ledger::default(), SourceMode::Run, 1);
        assert!(out.reply.plain().contains("Imported proxy"), "{}", out.reply.plain());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.fingerprint, "f1");
        let (out, again) = import_source(&mut e, &who, "proxy", Ok(("f1".into(), snap())), &ledger, SourceMode::Run, 2);
        assert!(again.is_none() && out.reply.plain().contains("has not changed"));
        let (out, _) = import_source(&mut e, &who, "proxy", Ok(("f2".into(), snap())), &ledger, SourceMode::Preview, 3);
        assert!(out.reply.plain().contains("would add"), "{}", out.reply.plain());
        let (out, cleared) = import_source(&mut e, &who, "proxy", Err("unused".into()), &ledger, SourceMode::Undo, 4);
        assert!(out.reply.plain().contains("Removed"), "{}", out.reply.plain());
        assert_eq!(cleared, Some(Ledger::default()));
        assert!(e.perms.group("builder").is_none(), "the imported group was empty again");
    }

    #[test]
    fn players_by_name_wait_for_their_first_join() {
        use crate::command::{SourceMode, Who, import_pending, import_source};
        use pumbo_common::lang::{COMMON, Lang};
        let mut e = crate::engine::Engine::in_memory(
            crate::config::CoreCfg::default(),
            Lang::load(&[COMMON, crate::LANG], "en", None).0,
        );
        let known = "00000000-0000-0000-0000-000000000002";
        let jeb = "00000000-0000-0000-0000-000000000003";
        e.seen(known, "Known").unwrap();
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
        let (out, ledger) =
            import_source(&mut e, &who, "file", Ok(("f1".into(), s)), &Ledger::default(), SourceMode::Run, 1);
        assert!(out.reply.plain().contains("first join: Jeb_"), "{}", out.reply.plain());
        let mut ledger = ledger.unwrap();
        assert_eq!(ledger.pending.len(), 1);
        assert_eq!(e.perms.user(known).unwrap().data.parents[0].group, "builder", "a known name is a UUID now");
        assert!(import_pending(&mut e, "file", &mut ledger, known, "Known", 2).is_none());
        let out = import_pending(&mut e, "file", &mut ledger, jeb, "JEB_", 3).unwrap();
        assert!(out.changed && out.reply.plain().contains("joined"), "{}", out.reply.plain());
        assert_eq!(e.perms.user(jeb).unwrap().data.parents[0].group, "builder");
        assert!(
            ledger.pending.is_empty()
                && ledger.entries.iter().any(|x| matches!(x, Entry::UserParent { uuid, .. } if uuid == jeb))
        );
        let (_, cleared) = import_source(&mut e, &who, "file", Err(String::new()), &ledger, SourceMode::Undo, 4);
        assert_eq!(cleared, Some(Ledger::default()));
        assert!(e.perms.user(jeb).is_none_or(|u| u.data.parents.is_empty()), "undo takes it back");
    }
}
