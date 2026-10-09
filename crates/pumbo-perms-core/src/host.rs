//! Handing decisions to a host that stores exact node values per player.
//!
//! Pumpkin keeps a map `node -> true/false` per player ("attachments") and
//! checks it before its own defaults; it understands wildcards only inside a
//! namespace (`minecraft:*`, `minecraft:command.*`) and checks them from the
//! broadest one, and it knows nothing about groups, contexts or time. So the
//! platform layer computes every node it can name ([`Catalog`]) for a player
//! and sets or unsets them on the host ([`plan`]). This needs no permission
//! event and no call back into the plugin while the host checks permissions.
//!
//! What it cannot do: a node nobody named (not built in, not in the data) is
//! decided by the host alone; for it only namespaced wildcards from the data
//! work, in the host's broad-first order.

use std::collections::{BTreeSet, HashMap};

use crate::engine::Engine;
use crate::node;
use crate::perms::Perms;

/// A change on the host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Op {
    Set(String, bool),
    Unset(String),
}

/// The host-form nodes worth setting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    builtin: BTreeSet<String>,
    nodes: BTreeSet<String>,
}

impl Catalog {
    /// `builtin`: nodes the host and other plugins check (host form).
    pub fn new<S: AsRef<str>>(builtin: impl IntoIterator<Item = S>) -> Self {
        let builtin: BTreeSet<String> = builtin.into_iter().map(|s| s.as_ref().to_string()).collect();
        Self { nodes: builtin.clone(), builtin }
    }

    /// Rebuilds the list: the built-in nodes, `known` (nodes set on the host
    /// before, the plugin's own nodes), every node in the data, every group as
    /// `group.<name>`, and `<namespace>:*` for every namespace in the list when
    /// the data has a wildcard the host cannot read (`*`, `pumbo.*`).
    pub fn refresh<S: AsRef<str>>(&mut self, perms: &Perms, known: impl IntoIterator<Item = S>) {
        let mut nodes = self.builtin.clone();
        nodes.extend(known.into_iter().map(|s| s.as_ref().to_string()));
        let mut global_wildcard = false;
        for n in perms.all_nodes() {
            let host = node::host_form(&n);
            if node::is_wildcard(&host) && !host.contains(':') {
                global_wildcard = true;
                continue;
            }
            nodes.insert(host);
        }
        for g in perms.groups() {
            nodes.insert(node::group_node(&g.name));
        }
        if global_wildcard {
            let namespaces: BTreeSet<String> =
                nodes.iter().filter_map(|n| n.split_once(':').map(|(ns, _)| format!("{ns}:*"))).collect();
            nodes.extend(namespaces);
        }
        self.nodes = nodes;
    }

    pub fn nodes(&self) -> &BTreeSet<String> {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// What one player should have on the host: node (host form) -> value.
/// Wildcards get the value of the data only; exact nodes also the operator
/// fallback, so that the host's own defaults keep deciding nodes nobody set.
pub fn wanted(
    e: &mut Engine,
    uuid: &str,
    world: &str,
    op_level: u8,
    now: u64,
    catalog: &Catalog,
) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    let q = e.query(world);
    for host in catalog.nodes() {
        let Ok(canonical) = node::normalize(host) else { continue };
        let value = if node::is_wildcard(&canonical) {
            e.perms.check(uuid, &canonical, &q, now)
        } else {
            e.decide(uuid, &canonical, world, op_level, now)
        };
        if let Some(v) = value {
            out.insert(host.clone(), v);
        }
    }
    out
}

/// Everything decided for a player on `server` (empty: global), for a host
/// that resolves wildcards itself with "most specific wins" (PumboProx): every
/// node and wildcard of the data with its value, Pumbo nodes also in the
/// Pumpkin spelling (`pumbobans:ban`, for the servers behind the proxy), and
/// `pumbo.perms.command`: whether `/pp` shows. Sorted.
pub fn decisions(e: &mut Engine, uuid: &str, server: &str, now: u64) -> Vec<(String, bool)> {
    let q = e.place(server, "");
    let mut out: Vec<(String, bool)> = Vec::new();
    for (n, v) in e.perms.effective(uuid, &q, now).nodes() {
        let host = node::host_form(n);
        if host != n {
            out.push((host, v));
        }
        out.push((n.to_string(), v));
    }
    let who = crate::command::Who { server: server.to_string(), ..crate::command::Who::player("", uuid, 0, "") };
    let visible = crate::command::allowed_any(e, &who, now);
    out.push((pumbo_common::command::permission(crate::command::PLUGIN, "command"), visible));
    out.sort();
    out
}

/// The host changes that turn `pushed` into `wanted`. `pushed = None` means
/// unknown (a fresh start): every catalog node not wanted is unset.
pub fn plan(wanted: &HashMap<String, bool>, pushed: Option<&HashMap<String, bool>>, catalog: &Catalog) -> Vec<Op> {
    let mut ops = Vec::new();
    for (node, v) in wanted {
        if pushed.and_then(|p| p.get(node)) != Some(v) {
            ops.push(Op::Set(node.clone(), *v));
        }
    }
    match pushed {
        Some(p) => {
            for node in p.keys() {
                if !wanted.contains_key(node) {
                    ops.push(Op::Unset(node.clone()));
                }
            }
        }
        None => {
            for node in catalog.nodes() {
                if !wanted.contains_key(node) {
                    ops.push(Op::Unset(node.clone()));
                }
            }
        }
    }
    ops.sort();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CoreCfg;
    use crate::context::Contexts;
    use crate::model::{ParentNode, PermNode};
    use pumbo_common::lang::{COMMON, Lang};

    const U: &str = "00000000-0000-0000-0000-000000000001";

    fn perm(node: &str, value: bool) -> PermNode {
        PermNode { node: node.into(), value, context: Contexts::global(), expiry: None }
    }

    fn engine() -> Engine {
        Engine::in_memory(CoreCfg::default(), Lang::load(&[COMMON], "en", None).0)
    }

    #[test]
    fn catalog_and_wanted_values() {
        let mut e = engine();
        e.perms.create_group("vip").unwrap();
        e.perms
            .edit_group("vip", |g| {
                g.data.set_permission(perm("minecraft:command.*", true));
                g.data.set_permission(perm("minecraft:command.stop", false));
                g.data.set_permission(perm("pumbo.bans.*", true));
                g.data.set_permission(perm("essentials.fly", true));
            })
            .unwrap();
        e.perms.edit_user(U, "Steve", |d| {
            d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None })
        });
        let mut c = Catalog::new(["minecraft:command.gamemode", "minecraft:command.stop", "pumpkin:command.tps"]);
        c.refresh(&e.perms, ["pumboperms:reload"]);
        let n = c.nodes();
        for want in
            ["minecraft:command.*", "pumbobans:*", "essentials.fly", "group.vip", "group.default", "pumboperms:reload"]
        {
            assert!(n.contains(want), "{want} missing from {n:?}");
        }
        assert!(!n.contains("minecraft:*"), "no global wildcard in the data");

        let w = wanted(&mut e, U, "world", 0, 0, &c);
        assert_eq!(w.get("minecraft:command.gamemode"), Some(&true));
        assert_eq!(w.get("minecraft:command.stop"), Some(&false));
        assert_eq!(w.get("minecraft:command.*"), Some(&true));
        assert_eq!(w.get("pumbobans:*"), Some(&true));
        assert_eq!(w.get("group.vip"), Some(&true));
        assert_eq!(w.get("pumpkin:command.tps"), None, "undecided: the host default stays");
        assert_eq!(w.get("pumboperms:reload"), None);
        // operators get Pumbo nodes nobody decided, but not wildcards
        let w = wanted(&mut e, U, "world", 3, 0, &c);
        assert_eq!(w.get("pumboperms:reload"), Some(&true));
    }

    #[test]
    fn global_wildcards_become_namespace_wildcards() {
        let mut e = engine();
        e.perms.edit_user(U, "Steve", |d| {
            d.set_permission(perm("*", true));
            d.set_permission(perm("minecraft:command.op", false));
        });
        let mut c = Catalog::new(["minecraft:command.op", "minecraft:command.give", "pumpkin:command.tps"]);
        c.refresh(&e.perms, Vec::<String>::new());
        assert!(c.nodes().contains("minecraft:*") && c.nodes().contains("pumpkin:*"));
        assert!(!c.nodes().contains("*"));
        let w = wanted(&mut e, U, "", 0, 0, &c);
        assert_eq!(w.get("minecraft:*"), Some(&true));
        assert_eq!(w.get("minecraft:command.give"), Some(&true));
        assert_eq!(w.get("minecraft:command.op"), Some(&false));
    }

    #[test]
    fn decisions_per_server_for_the_proxy() {
        let mut e = engine();
        e.server_groups.insert("lobby".into(), vec!["hubs".into()]);
        e.perms.create_group("vip").unwrap();
        e.perms
            .edit_group("vip", |g| {
                g.data.set_permission(perm("pumbo.bans.*", true));
                g.data.set_permission(PermNode {
                    node: "minecraft:command.gamemode".into(),
                    value: true,
                    context: Contexts::parse(&["group=hubs".to_string()]).unwrap(),
                    expiry: None,
                });
                g.data.set_permission(perm("pumbo.perms.user.info", true));
            })
            .unwrap();
        e.perms.edit_user(U, "Steve", |d| {
            d.add_parent(ParentNode {
                group: "vip".into(),
                context: Contexts::parse(&["server=lobby".to_string()]).unwrap(),
                expiry: None,
            })
        });
        let lobby = decisions(&mut e, U, "lobby", 0);
        for want in [
            ("pumbo.bans.*", true),
            ("pumbobans:*", true),
            ("minecraft:command.gamemode", true),
            ("group.vip", true),
            ("pumbo.perms.command", true),
        ] {
            assert!(lobby.contains(&(want.0.to_string(), want.1)), "{want:?} not in {lobby:?}");
        }
        let survival = decisions(&mut e, U, "survival", 0);
        assert!(!survival.iter().any(|(n, _)| n.starts_with("minecraft:") || n == "group.vip"), "{survival:?}");
        assert!(survival.contains(&("pumbo.perms.command".to_string(), false)));
        assert_eq!(decisions(&mut e, U, "", 0), survival, "global = a server without own entries");
    }

    #[test]
    fn plans_only_differences() {
        let c = Catalog::new(["a", "b", "c"]);
        let wanted: HashMap<String, bool> = [("a".to_string(), true), ("b".to_string(), false)].into();
        // unknown host state: set what is wanted, unset the rest of the catalog
        assert_eq!(
            plan(&wanted, None, &c),
            vec![Op::Set("a".into(), true), Op::Set("b".into(), false), Op::Unset("c".into())]
        );
        let pushed: HashMap<String, bool> = [("a".to_string(), true), ("d".to_string(), true)].into();
        assert_eq!(plan(&wanted, Some(&pushed), &c), vec![Op::Set("b".into(), false), Op::Unset("d".into())]);
        assert!(plan(&wanted, Some(&wanted), &c).is_empty());
    }
}
