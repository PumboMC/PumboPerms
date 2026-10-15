//! Handing decisions to a host that stores exact node values per player.
//!
//! Pumpkin keeps a map `node -> true/false` per player ("attachments") and
//! checks it before its own defaults; it understands wildcards only inside a
//! namespace (`minecraft:*`, `minecraft:command.*`) and checks them from the
//! broadest one, and it reads an exact node set to a value as a wildcard for
//! the nodes below it. It knows nothing about groups, contexts or time. So the
//! platform layer computes every node it can name ([`Catalog`]) for a player
//! and sets or unsets them on the host ([`plan`]). This needs no permission
//! event and no call back into the plugin while the host checks permissions.
//!
//! What it cannot do: a node nobody named (not built in, not in the data) is
//! decided by the host alone; for it only namespaced wildcards from the data
//! work, and a wildcard with a narrower one of another value under it is not
//! sent at all ([`hidden`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Bound;

use pumbo_common::pumpkin::{COMMAND_NODES, PUMBO_NODES};
use pumbo_contracts::permissions::drop_shadowed;

use crate::context::Query;
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
    /// `nodes` without wildcards.
    exact: BTreeSet<String>,
    /// `nodes` in the spelling of the data -> host form, normalized once per refresh.
    canonical: BTreeMap<String, String>,
}

impl Catalog {
    /// `builtin`: nodes the host and other plugins check (host form).
    pub fn new<S: AsRef<str>>(builtin: impl IntoIterator<Item = S>) -> Self {
        let builtin: BTreeSet<String> = builtin.into_iter().map(|s| s.as_ref().to_string()).collect();
        let exact = exact_of(&builtin);
        let canonical = canonical_of(&builtin);
        Self { nodes: builtin.clone(), builtin, exact, canonical }
    }

    /// The nodes Pumpkin and the Pumbo plugins check.
    pub fn pumpkin() -> Self {
        Self::new(COMMAND_NODES.iter().chain(PUMBO_NODES))
    }

    /// Rebuilds the list: the built-in nodes, `known` (nodes set on the host
    /// before, the plugin's own nodes), every node in the data, every group as
    /// `group.<name>`, and `<namespace>:*` for every namespace in the list and
    /// of the Pumbo plugins when the data has a wildcard the host cannot read
    /// (`*`, `pumbo.*`). Returns whether the list changed.
    pub fn refresh<S: AsRef<str>>(&mut self, perms: &Perms, known: impl IntoIterator<Item = S>) -> bool {
        let mut nodes = self.builtin.clone();
        nodes.extend(known.into_iter().map(|s| s.as_ref().to_string()));
        let mut global_wildcard = false;
        for n in perms.all_nodes() {
            let host = node::host_form(n);
            if is_global_wildcard(&host) {
                global_wildcard = true;
                continue;
            }
            nodes.insert(host);
        }
        for g in perms.groups() {
            nodes.insert(node::group_node(&g.name));
        }
        if global_wildcard {
            let namespaces = namespace_wildcards(nodes.iter().map(String::as_str).chain(PUMBO_NODES.iter().copied()));
            nodes.extend(namespaces);
        }
        if nodes == self.nodes {
            return false;
        }
        self.exact = exact_of(&nodes);
        self.canonical = canonical_of(&nodes);
        self.nodes = nodes;
        true
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

fn canonical_of(nodes: &BTreeSet<String>) -> BTreeMap<String, String> {
    nodes.iter().filter_map(|h| node::normalize(h).ok().map(|c| (c, h.clone()))).collect()
}

fn exact_of(nodes: &BTreeSet<String>) -> BTreeSet<String> {
    nodes.iter().filter(|n| !node::is_wildcard(n)).cloned().collect()
}

/// A wildcard in host form that no namespace holds (`*`, `pumbo.*`).
fn is_global_wildcard(host: &str) -> bool {
    node::is_wildcard(host) && !host.contains(':')
}

/// `<namespace>:*` for every namespaced node.
fn namespace_wildcards<'a>(nodes: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    nodes.into_iter().filter_map(|n| n.split_once(':').map(|(ns, _)| format!("{ns}:*"))).collect()
}

/// `pumboperms:command`: whether `/pp` shows, as Pumpkin spells it.
pub fn command_node() -> String {
    node::host_form(crate::command::COMMAND_NODE)
}

/// What one player should have on the host: node (host form) -> value, from
/// the data alone, read the way Pumpkin reads it ([`drop_shadowed`]), and
/// `pumboperms:command` (the operator fallback of `/pp` counts there).
pub fn wanted(
    e: &mut Engine,
    uuid: &str,
    world: &str,
    op_level: u8,
    now: u64,
    catalog: &Catalog,
) -> HashMap<String, bool> {
    wanted_and_hidden(e, uuid, world, op_level, now, catalog).0
}

/// [`wanted`] and the wildcards left out of it ([`hidden`]).
pub fn wanted_and_hidden(
    e: &mut Engine,
    uuid: &str,
    world: &str,
    op_level: u8,
    now: u64,
    catalog: &Catalog,
) -> (HashMap<String, bool>, Vec<(String, String)>) {
    let q = e.query(world);
    let mut set = BTreeMap::new();
    let effective = e.perms.effective(uuid, &q, now);
    // Only a node of the player's own data, or one under its wildcards, can be decided.
    for (n, _) in effective.nodes() {
        let from: Vec<(&String, &String)> = match n.strip_suffix('*') {
            Some("") => catalog.canonical.iter().collect(),
            Some(base) => {
                let below = catalog.canonical.range::<str, _>((Bound::Included(base), Bound::Unbounded));
                below.take_while(|(c, _)| c.starts_with(base)).collect()
            }
            None => catalog.canonical.get_key_value(n).into_iter().collect(),
        };
        for (canonical, host) in from {
            if let Some(v) = effective.check(canonical) {
                set.insert(host.clone(), v);
            }
        }
    }
    // Pumpkin reads an exact `ns:a` set to true as a wildcard: the catalog nodes under one.
    let mut under_true = BTreeSet::new();
    for (parent, _) in set.iter().filter(|(n, v)| **v && !node::is_wildcard(n) && n.matches(':').count() == 1) {
        let Ok(parent) = node::normalize(parent) else { continue };
        let below = catalog.canonical.range::<str, _>((Bound::Excluded(parent.as_str()), Bound::Unbounded));
        let below = below.take_while(|(c, _)| c.starts_with(parent.as_str()));
        let below =
            below.filter(|(c, h)| c.get(parent.len()..).is_some_and(|r| r.starts_with('.')) && !node::is_wildcard(h));
        under_true.extend(below.map(|(_, h)| h.clone()));
    }
    let hidden = shadowed(&mut set, &under_true);
    let who = crate::command::Who::player("", uuid, op_level, world);
    set.insert(command_node(), crate::command::allowed_any(e, &who, now));
    (set.into_iter().collect(), hidden)
}

/// Removes the wildcards Pumpkin would read before a narrower wildcard with
/// another value ([`drop_shadowed`]); returns each with that narrower one, and
/// each exact `ns:a` with a narrower wildcard of another value under it, which
/// Pumpkin also reads first.
fn shadowed(set: &mut BTreeMap<String, bool>, catalog: &BTreeSet<String>) -> Vec<(String, String)> {
    let wildcards: Vec<(String, bool)> =
        set.iter().filter(|(n, _)| node::is_wildcard(n)).map(|(n, v)| (n.clone(), *v)).collect();
    let one_colon = |n: &str| n.matches(':').count() == 1;
    let parents: Vec<(String, String)> = set
        .iter()
        .filter(|(p, _)| !node::is_wildcard(p) && one_colon(p))
        .flat_map(|(p, v)| {
            let under = format!("{p}.");
            wildcards
                .iter()
                .filter(move |(w, wv)| wv != v && w.ends_with(".*") && w.starts_with(&under) && one_colon(w))
                .map(move |(w, _)| (p.clone(), w.clone()))
        })
        .collect();
    let mut out: Vec<(String, String)> = drop_shadowed(set, catalog)
        .into_iter()
        .filter_map(|w| {
            let value = wildcards.iter().find(|(n, _)| *n == w)?.1;
            let base = w.strip_suffix('*')?;
            let narrower =
                wildcards.iter().find(|(n, v)| *v != value && *n != w && n.ends_with(".*") && n.starts_with(base))?;
            Some((w, narrower.0.clone()))
        })
        .collect();
    out.extend(parents);
    out
}

/// Nodes of a player at `q` that Pumpkin reads before a narrower wildcard with
/// another value ([`shadowed`]): (node, narrower), in host form. Such a
/// wildcard is not sent; under either, a node outside the catalogs gets
/// Pumpkin's answer, not PumboPerms'.
pub fn hidden(e: &mut Engine, uuid: &str, q: &Query, now: u64) -> Vec<(String, String)> {
    let effective = e.perms.effective(uuid, q, now);
    // Only a wildcard hides anything.
    if !effective.nodes().any(|(n, _)| node::is_wildcard(n)) {
        return Vec::new();
    }
    let hosts: Vec<String> = effective.nodes().map(|(n, _)| node::host_form(n)).collect();
    let mut nodes: BTreeSet<String> = hosts.iter().filter(|h| h.contains(':')).cloned().collect();
    if hosts.iter().any(|h| is_global_wildcard(h)) {
        let all = COMMAND_NODES.iter().chain(PUMBO_NODES).copied().chain(hosts.iter().map(String::as_str));
        nodes.extend(namespace_wildcards(all));
    }
    let mut set = BTreeMap::new();
    for w in nodes {
        if let Ok(canonical) = node::normalize(&w)
            && let Some(v) = e.perms.check(uuid, &canonical, q, now)
        {
            set.insert(w, v);
        }
    }
    shadowed(&mut set, &BTreeSet::new())
}

/// Everything decided for a player on `server` (empty: global), for a host
/// that resolves wildcards itself with "most specific wins" (PumboProx): every
/// node and wildcard of the data with its value, Pumbo nodes also in the
/// Pumpkin spelling (`pumbobans:ban`, for the servers behind the proxy), with
/// what Pumpkin's catalog holds for `*` and `pumbo.*` (`<ns>:*` of every Pumbo
/// plugin and namespace named, the exact nodes of the data, the groups), the
/// exact nodes of the data under a [`hidden`] wildcard, `false` for an exact
/// node of the data without a decision under an exact parent set to true (as
/// Pumpkin's own catalog gets it, [`drop_shadowed`]), and `pumbo.perms.command`
/// (also as `pumboperms:command`): whether `/pp` shows. Sorted.
pub fn decisions(e: &mut Engine, uuid: &str, server: &str, now: u64) -> Vec<(String, bool)> {
    let q = e.place(server, "");
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut global = false;
    // The bridge catalog lacks most data nodes, so set the false Pumpkin's own catalog would get:
    // nodes of the data without a decision under an exact `ns:a` set to true (a wildcard above
    // such a parent would decide the child too).
    let mut children: BTreeSet<String> = BTreeSet::new();
    let (effective, data) = e.perms.effective_and_nodes(uuid, &q, now);
    for (n, v) in effective.nodes() {
        let host = node::host_form(n);
        global |= is_global_wildcard(&host);
        if v && !node::is_wildcard(n) && host.matches(':').count() == 1 {
            let below = data.range::<str, _>((Bound::Excluded(n), Bound::Unbounded));
            for child in
                below.take_while(|c| c.starts_with(n)).filter(|c| c.get(n.len()..).is_some_and(|r| r.starts_with('.')))
            {
                let child = node::host_form(child);
                let undecided = node::normalize(&child).is_ok_and(|c| effective.check(&c).is_none());
                if undecided && !node::is_wildcard(&child) && child.matches(':').count() == 1 {
                    children.insert(child);
                }
            }
        }
        if host != n {
            out.push((n.to_string(), v));
        }
        out.push((host, v));
    }
    out.extend(children.into_iter().map(|c| (c, false)));
    let mut extra: BTreeSet<String> = BTreeSet::new();
    if global {
        // What Pumpkin's catalog names for a global wildcard: data nodes, groups, `<ns>:*`.
        let data: Vec<String> = e.perms.all_nodes().iter().map(|n| node::host_form(n)).collect();
        let named = COMMAND_NODES.iter().chain(PUMBO_NODES).copied().chain(data.iter().map(String::as_str));
        extra.extend(namespace_wildcards(named));
        extra.extend(data.iter().filter(|h| !node::is_wildcard(h) && h.contains(':')).cloned());
        extra.extend(e.perms.groups().map(|g| node::group_node(&g.name)));
    }
    for (w, narrower) in hidden(e, uuid, &q, now) {
        let under = e.perms.all_nodes().iter().map(|n| node::host_form(n));
        extra.extend(under.filter(|h| {
            !node::is_wildcard(h) && h.contains(':') && (node::covers(&w, h) || node::covers(&narrower, h))
        }));
    }
    if !extra.is_empty() {
        let mut named: HashSet<String> = out.iter().map(|(n, _)| n.clone()).collect();
        for host in extra {
            if !named.contains(&host)
                && let Ok(canonical) = node::normalize(&host)
                && let Some(v) = e.perms.check(uuid, &canonical, &q, now)
            {
                named.insert(host.clone());
                out.push((host, v));
            }
        }
    }
    // As `command::allowed_any` for a player without an operator level.
    let effective = e.perms.effective(uuid, &q, now);
    let own = |n: &str| n == "*" || n == "pumbo.*" || n.starts_with(crate::engine::OWN_NODES);
    let visible =
        effective.nodes().any(|(n, _)| own(n)) && action_nodes().iter().any(|a| effective.check(a) == Some(true));
    out.push((crate::command::COMMAND_NODE.to_string(), visible));
    out.push((command_node(), visible));
    out.sort();
    out
}

/// `pumbo.perms.<action>` of every `/pp` action.
fn action_nodes() -> &'static [String] {
    static NODES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        crate::command::ACTIONS.iter().map(|a| pumbo_common::command::permission(crate::command::PLUGIN, a)).collect()
    });
    &NODES
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
    use crate::testing::pumpkin_has;
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
        // operators keep Pumpkin's registry for nodes nobody decided; only /pp shows
        let w = wanted(&mut e, U, "world", 3, 0, &c);
        assert_eq!(w.get("pumboperms:reload"), None);
        assert_eq!(w.get("pumboperms:command"), Some(&true));
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

    fn member_of(e: &mut Engine, group: &str, nodes: &[(&str, bool)]) {
        e.perms.create_group(group).unwrap();
        e.perms
            .edit_group(group, |g| {
                for (n, v) in nodes {
                    g.data.set_permission(perm(n, *v));
                }
            })
            .unwrap();
        e.perms.edit_user(U, "Bob", |d| {
            d.add_parent(ParentNode { group: group.into(), context: Contexts::global(), expiry: None })
        });
    }

    #[test]
    fn narrow_wildcard_deny_wins_on_pumpkin() {
        let mut e = engine();
        member_of(&mut e, "mod", &[("pumbo.bans.*", true), ("pumbo.bans.exempt.*", false)]);
        let mut c = Catalog::new(pumbo_common::pumpkin::COMMAND_NODES.iter().copied());
        c.refresh(&e.perms, Vec::<String>::new());
        let w = wanted(&mut e, U, "world", 0, 0, &c);
        let core = e.perms.check(U, "pumbo.bans.exempt.ban", &e.query("world"), 0);
        assert_eq!(core, Some(false));
        assert_eq!(pumpkin_has(&w, "pumbobans:exempt.ban"), core, "{w:?}");
    }

    #[test]
    fn pumbo_wildcard_reaches_pumbo_namespaces_on_pumpkin() {
        let mut e = engine();
        member_of(&mut e, "admin", &[("pumbo.*", true)]);
        let mut c = Catalog::new(pumbo_common::pumpkin::COMMAND_NODES.iter().copied());
        c.refresh(&e.perms, Vec::<String>::new());
        let w = wanted(&mut e, U, "world", 0, 0, &c);
        assert_eq!(e.perms.check(U, "pumbo.bans.ban", &e.query("world"), 0), Some(true));
        assert_eq!(pumpkin_has(&w, "pumbobans:ban"), Some(true), "{w:?}");
    }

    #[test]
    fn op_fallback_does_not_override_plugin_defaults() {
        let mut e = engine();
        e.perms.create_group("helper").unwrap();
        e.perms
            .edit_group("helper", |g| {
                g.data.set_permission(perm("pumbo.bans.limit.helper", true));
                g.data.set_permission(perm("pumbo.bans.exempt.ban", true));
            })
            .unwrap();
        let mut c = Catalog::new(Vec::<String>::new());
        c.refresh(&e.perms, Vec::<String>::new());
        assert!(c.nodes().contains("pumbobans:limit.helper"));
        // U is an operator of level 3 outside helper.
        let w = wanted(&mut e, U, "world", 3, 0, &c);
        assert_eq!(w.get("pumbobans:limit.helper"), None, "{w:?}");
        assert_eq!(w.get("pumbobans:exempt.ban"), None, "{w:?}");
    }

    #[test]
    fn parent_node_does_not_grant_catalog_children() {
        let mut e = engine();
        member_of(&mut e, "mod", &[("pumbo.bans.notify", true), ("minecraft:command.random", true)]);
        let mut c = Catalog::pumpkin();
        c.refresh(&e.perms, Vec::<String>::new());
        let w = wanted(&mut e, U, "world", 0, 0, &c);
        for (parent, child) in [
            ("pumbobans:notify", "pumbobans:notify.silent"),
            ("minecraft:command.random", "minecraft:command.random.reset"),
        ] {
            assert_eq!(pumpkin_has(&w, parent), Some(true));
            assert_eq!(w.get(child), Some(&false), "{w:?}");
            assert_eq!(pumpkin_has(&w, child), Some(false));
        }
    }

    #[test]
    fn pumbo_nodes_cover_plugin_nodes() {
        let own =
            crate::command::own_nodes().into_iter().chain([pumbo_common::command::permission("perms", "command")]);
        let hosts: Vec<String> = own.map(|n| node::host_form(&n)).collect();
        for host in &hosts {
            assert!(PUMBO_NODES.contains(&host.as_str()), "{host} missing from PUMBO_NODES");
        }
        for n in PUMBO_NODES.iter().filter(|n| n.starts_with("pumboperms:")) {
            assert!(hosts.iter().any(|h| h == n), "{n} in PUMBO_NODES is no node of the plugin");
        }
    }

    #[test]
    fn decisions_expand_pumbo_wildcard() {
        let mut e = engine();
        member_of(&mut e, "admin", &[("pumbo.*", true), ("pumbo.bans.*", false)]);
        let d = decisions(&mut e, U, "", 0);
        for want in [("pumboperms:*", true), ("pumbofilter:*", true), ("pumboauth:*", true), ("pumbobans:*", false)] {
            assert!(d.contains(&(want.0.to_string(), want.1)), "{want:?} not in {d:?}");
        }
        assert_eq!(d.iter().filter(|(n, _)| n == "pumbobans:*").count(), 1);
        assert!(!d.iter().any(|(n, _)| n == "minecraft:*"), "only the Pumbo namespaces");
    }

    #[test]
    fn decisions_carry_command_node_in_host_form() {
        let mut e = engine();
        member_of(&mut e, "helper", &[("pumbo.perms.user.info", true)]);
        let d = decisions(&mut e, U, "", 0);
        assert!(d.contains(&("pumbo.perms.command".to_string(), true)), "{d:?}");
        assert!(d.contains(&("pumboperms:command".to_string(), true)), "{d:?}");
        let other = decisions(&mut e, "00000000-0000-0000-0000-000000000002", "", 0);
        assert!(other.contains(&("pumboperms:command".to_string(), false)), "{other:?}");
    }

    #[test]
    fn hidden_wildcards_reach_the_proxy_as_exact_nodes() {
        let mut e = engine();
        member_of(&mut e, "mod", &[("pumbo.bans.*", true), ("pumbo.bans.exempt.*", false)]);
        e.perms.create_group("helper").unwrap();
        e.perms.edit_group("helper", |g| g.data.set_permission(perm("pumbo.bans.limit.helper", true))).unwrap();
        let q = e.query("");
        assert_eq!(hidden(&mut e, U, &q, 0), vec![("pumbobans:*".to_string(), "pumbobans:exempt.*".to_string())]);
        let d = decisions(&mut e, U, "", 0);
        assert!(d.contains(&("pumbobans:limit.helper".to_string(), true)), "{d:?}");
    }

    #[test]
    fn exact_parent_over_narrower_wildcard_is_reported() {
        let mut e = engine();
        member_of(&mut e, "vip", &[("essentials:fly", true), ("essentials:fly.*", false)]);
        e.perms.create_group("helper").unwrap();
        e.perms.edit_group("helper", |g| g.data.set_permission(perm("essentials:fly.speed", true))).unwrap();
        let q = e.query("");
        assert_eq!(hidden(&mut e, U, &q, 0), vec![("essentials:fly".to_string(), "essentials:fly.*".to_string())]);
        // Pumpkin reads `essentials:fly` first, so a known child goes with its own value.
        let d = decisions(&mut e, U, "", 0);
        assert!(d.contains(&("essentials:fly.speed".to_string(), false)), "{d:?}");
        let mut c = Catalog::new(Vec::<String>::new());
        c.refresh(&e.perms, Vec::<String>::new());
        let (w, hidden) = wanted_and_hidden(&mut e, U, "world", 0, 0, &c);
        assert_eq!(hidden, vec![("essentials:fly".to_string(), "essentials:fly.*".to_string())]);
        assert_eq!(pumpkin_has(&w, "essentials:fly.speed"), Some(false), "{w:?}");
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
