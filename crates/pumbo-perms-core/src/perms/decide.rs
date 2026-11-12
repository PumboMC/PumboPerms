//! Decisions: what a user or group gets at a place, and why.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::context::{Contexts, Query};
use crate::model::{Data, Group, MetaKind, ParentNode, alive};
use crate::node;

use super::{Decision, Effective, MAX_CACHE, MAX_GROUPS, Perms, Source};

impl Perms {
    /// The cached decisions for a user at a place.
    pub fn effective(&mut self, uuid: &str, query: &Query, now: u64) -> &Effective {
        let fresh = self.memo.cache.get(uuid).and_then(|m| m.get(query)).is_some_and(|e| now < e.valid_until);
        if !fresh {
            if self.memo.cached >= MAX_CACHE {
                (self.memo.cache, self.memo.cached) = (HashMap::new(), 0);
            }
            let e = self.compute(uuid, query, now);
            if self.memo.cache.entry(uuid.to_string()).or_default().insert(query.clone(), e).is_none() {
                self.memo.cached = self.memo.cached.saturating_add(1);
            }
        }
        self.cached(uuid, query)
    }

    /// [`Perms::effective`] and [`Perms::all_nodes`] at once.
    pub fn effective_and_nodes(&mut self, uuid: &str, query: &Query, now: u64) -> (&Effective, &BTreeSet<String>) {
        self.effective(uuid, query, now);
        (self.cached(uuid, query), self.all_nodes())
    }

    fn cached(&self, uuid: &str, query: &Query) -> &Effective {
        self.memo.cache.get(uuid).and_then(|m| m.get(query)).unwrap_or(&self.memo.nothing)
    }

    /// Decides a node for a user: `Some(value)` or undecided.
    pub fn check(&mut self, uuid: &str, node: &str, query: &Query, now: u64) -> Option<bool> {
        self.effective(uuid, query, now).check(node)
    }

    /// Like [`Perms::check`], with where the answer came from.
    pub fn explain(&mut self, uuid: &str, node: &str, query: &Query, now: u64) -> Option<Decision> {
        self.effective(uuid, query, now).explain(node).cloned()
    }

    /// Groups the user inherits at `query`, most important first, with their
    /// distance (1 = direct parent).
    pub fn inheritance(&self, uuid: &str, query: &Query, now: u64) -> Vec<(String, usize)> {
        let empty = Data::default();
        let data = self.users.get(uuid).map(|u| &u.data).unwrap_or(&empty);
        self.inheritance_of(data, query, now)
    }

    fn inheritance_of(&self, data: &Data, query: &Query, now: u64) -> Vec<(String, usize)> {
        let active = |p: &&ParentNode| self.groups.contains_key(&p.group);
        let mut direct: Vec<&str> =
            applicable_parents(data, query, now).filter(active).map(|p| p.group.as_str()).collect();
        let global_parent = applicable_parents(data, query, now).filter(active).any(|p| p.context.is_global());
        if !global_parent && self.groups.contains_key(&self.default_group) {
            direct.push(&self.default_group);
        }
        self.closure(&direct, query, now)
    }

    /// Groups reachable from `start` (distance 1), each once, sorted by
    /// importance.
    fn closure(&self, start: &[&str], query: &Query, now: u64) -> Vec<(String, usize)> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out: Vec<(&Group, usize)> = Vec::new();
        let mut queue: VecDeque<(&str, usize)> = start.iter().map(|g| (*g, 1)).collect();
        while let Some((name, depth)) = queue.pop_front() {
            if out.len() >= MAX_GROUPS || !seen.insert(name) {
                continue;
            }
            let Some(group) = self.groups.get(name) else { continue };
            out.push((group, depth));
            for p in applicable_parents(&group.data, query, now) {
                queue.push_back((&p.group, depth + 1));
            }
        }
        out.sort_by(|(a, da), (b, db)| b.weight.cmp(&a.weight).then(da.cmp(db)).then(a.name.cmp(&b.name)));
        out.into_iter().map(|(g, d)| (g.name.clone(), d)).collect()
    }

    fn compute(&self, uuid: &str, query: &Query, now: u64) -> Effective {
        let empty = Data::default();
        let user_data = self.users.get(uuid).map(|u| &u.data).unwrap_or(&empty);
        self.compute_with(user_data, query, now)
    }

    /// What a member of only this group would get (`/pp group <g> permission check`).
    pub fn group_effective(&self, group: &str, query: &Query, now: u64) -> Effective {
        let data = Data {
            parents: vec![ParentNode { group: group.to_string(), context: Contexts::global(), expiry: None }],
            ..Data::default()
        };
        self.compute_with(&data, query, now)
    }

    fn compute_with(&self, user_data: &Data, query: &Query, now: u64) -> Effective {
        let groups = self.inheritance_of(user_data, query, now);

        let mut holders: Vec<(Source, &Data)> = vec![(Source::User, user_data)];
        for (g, _) in &groups {
            if let Some(group) = self.groups.get(g) {
                holders.push((Source::Group(g.clone()), &group.data));
            }
        }

        let mut valid_until = u64::MAX;
        let mut note = |expiry: Option<u64>| {
            if let Some(t) = expiry {
                valid_until = valid_until.min(t);
            }
        };
        // the parents that were followed end too
        for (_, data) in &holders {
            for p in data.parents.iter().filter(|p| alive(p.expiry, now) && p.context.applies(query)) {
                note(p.expiry);
            }
        }

        let mut exact: HashMap<String, Decision> = HashMap::new();
        // membership first: a `group.<name>` set by hand never hides it
        for (g, _) in &groups {
            let n = node::group_node(g);
            exact.insert(
                n.clone(),
                Decision {
                    value: true,
                    node: n,
                    source: Source::Group(g.clone()),
                    context: Contexts::global(),
                    expiry: None,
                },
            );
        }
        // permissions: holders in order, on each holder the best node first
        for (source, data) in &holders {
            let mut nodes: Vec<_> =
                data.permissions.iter().filter(|p| alive(p.expiry, now) && p.context.applies(query)).collect();
            nodes.sort_by(|a, b| {
                b.context
                    .score()
                    .cmp(&a.context.score())
                    .then(b.expiry.is_some().cmp(&a.expiry.is_some()))
                    .then(a.value.cmp(&b.value))
            });
            for p in nodes {
                note(p.expiry);
                exact.entry(p.node.clone()).or_insert_with(|| Decision {
                    value: p.value,
                    node: p.node.clone(),
                    source: source.clone(),
                    context: p.context.clone(),
                    expiry: p.expiry,
                });
            }
        }

        // prefix and suffix: highest priority, then holder order, then context
        let mut affixes: [Option<String>; 2] = [None, None];
        for (slot, kind) in affixes.iter_mut().zip([MetaKind::Prefix, MetaKind::Suffix]) {
            let mut best: Option<(i32, std::cmp::Reverse<usize>, u8, bool, &str)> = None;
            for (i, (_, data)) in holders.iter().enumerate() {
                for m in data.meta.iter().filter(|m| m.kind == kind && alive(m.expiry, now) && m.context.applies(query))
                {
                    note(m.expiry);
                    let cand =
                        (m.priority, std::cmp::Reverse(i), m.context.score(), m.expiry.is_some(), m.value.as_str());
                    if best.as_ref().is_none_or(|b| (cand.0, cand.1, cand.2, cand.3) > (b.0, b.1, b.2, b.3)) {
                        best = Some(cand);
                    }
                }
            }
            *slot = best.map(|b| b.4.to_string());
        }
        let [prefix, suffix] = affixes;
        // meta keys: the first holder that has the key
        let mut meta = BTreeMap::new();
        for (_, data) in &holders {
            let mut metas: Vec<_> = data
                .meta
                .iter()
                .filter(|m| m.kind == MetaKind::Meta && alive(m.expiry, now) && m.context.applies(query))
                .collect();
            metas.sort_by(|a, b| {
                b.context.score().cmp(&a.context.score()).then(b.expiry.is_some().cmp(&a.expiry.is_some()))
            });
            for m in metas {
                note(m.expiry);
                meta.entry(m.key.clone()).or_insert_with(|| m.value.clone());
            }
        }

        let primary = self.primary_of(user_data, query, now);
        Effective {
            valid_until,
            exact,
            groups: groups.into_iter().map(|(g, _)| g).collect(),
            primary,
            prefix,
            suffix,
            meta,
        }
    }

    /// The direct group with the highest weight (then the first by name); the
    /// default group when there is none.
    fn primary_of(&self, data: &Data, query: &Query, now: u64) -> String {
        applicable_parents(data, query, now)
            .filter_map(|p| self.groups.get(&p.group))
            .max_by(|a, b| a.weight.cmp(&b.weight).then(b.name.cmp(&a.name)))
            .map(|g| g.name.clone())
            .unwrap_or_else(|| self.default_group.clone())
    }

    /// Display name of a group (the name when it has none).
    pub fn display_name(&self, group: &str) -> String {
        self.groups.get(group).map(|g| g.shown_name().to_string()).unwrap_or_else(|| group.to_string())
    }
}

fn applicable_parents<'a>(data: &'a Data, query: &'a Query, now: u64) -> impl Iterator<Item = &'a ParentNode> + 'a {
    data.parents.iter().filter(move |p| alive(p.expiry, now) && p.context.applies(query))
}
