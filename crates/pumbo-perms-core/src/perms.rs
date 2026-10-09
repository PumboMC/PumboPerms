//! The permission database in memory and how a permission is decided.
//!
//! # Rules
//!
//! 1. **Holders.** A user inherits the groups of its parent nodes that apply
//!    where the player is (contexts) and have not expired, and their parents in
//!    turn. A user without any such parent is in the default group. The order of
//!    importance is: the user itself, then the groups by weight (highest first),
//!    then by distance (direct parents before their parents), then by name.
//! 2. **Specific beats general.** For a checked node, the exact node wins over
//!    any wildcard, and a longer wildcard over a shorter one
//!    (`pumbo.bans.ban` > `pumbo.bans.*` > `pumbo.*` > `*`), no matter which
//!    holder has them.
//! 3. **Among equally specific nodes** the more important holder wins (rule 1).
//! 4. **On one holder** the most specific context wins (server 4, server group 2,
//!    world 1, summed; global 0), then a temporary node over a permanent one,
//!    then `false` over `true`.
//! 5. Nothing set: undecided (`None`); the platform falls back to its defaults
//!    (operator levels on Pumpkin).
//!
//! Every inherited group also answers its `group.<name>` node with `true`.
//!
//! Prefix and suffix: the highest priority among all holders wins; on a tie the
//! more important holder, then the more specific context. Meta keys: the first
//! holder (in the order of rule 1) that has the key; on that holder the most
//! specific context.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;

use crate::context::{Contexts, Query};
use crate::model::{Data, Group, MetaKind, ParentNode, Track, User, alive};
use crate::node;

/// Default name of the group everyone without other groups is in.
pub const DEFAULT_GROUP: &str = "default";

/// Groups visited at most while following inheritance (cycles are cut anyway).
const MAX_GROUPS: usize = 512;

/// Cached decisions kept at most before the cache is emptied.
const MAX_CACHE: usize = 4096;

/// Who holds a node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    User,
    Group(String),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::User => f.write_str("user"),
            Source::Group(g) => write!(f, "group {g}"),
        }
    }
}

/// A decided node and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub value: bool,
    /// The node that decided (the checked node or a wildcard covering it).
    pub node: String,
    pub source: Source,
    pub context: Contexts,
    pub expiry: Option<u64>,
}

/// Everything decided for one user at one place, cached until the data changes
/// or a node in it expires.
#[derive(Debug, Clone, Default)]
pub struct Effective {
    generation: u64,
    /// The first end time of a node that was used (`u64::MAX` if none).
    valid_until: u64,
    exact: HashMap<String, Decision>,
    /// Inherited groups, most important first.
    pub groups: Vec<String>,
    /// The direct group with the highest weight (the rank).
    pub primary: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub meta: BTreeMap<String, String>,
}

impl Effective {
    /// The decision for a node: exact first, then the wildcards above it.
    pub fn explain(&self, node: &str) -> Option<&Decision> {
        if let Some(d) = self.exact.get(node) {
            return Some(d);
        }
        node::parents(node).iter().find_map(|w| self.exact.get(w))
    }

    pub fn check(&self, node: &str) -> Option<bool> {
        self.explain(node).map(|d| d.value)
    }

    /// Every node set for this user (exact names and wildcards), with values.
    pub fn nodes(&self) -> impl Iterator<Item = (&str, bool)> {
        self.exact.iter().map(|(k, d)| (k.as_str(), d.value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermsError {
    GroupExists(String),
    NoGroup(String),
    TrackExists(String),
    NoTrack(String),
    BadName(String),
    /// The default group cannot be deleted or renamed.
    DefaultGroup,
    /// A group cannot inherit itself, directly or through others.
    Cycle(String),
    /// Already exactly so; nothing changed.
    Unchanged,
    /// Promote/demote: the user is on more than one group of the track.
    Ambiguous(Vec<String>),
    /// Promote: already on the last group.
    TrackEnd(String),
    /// Demote: already on the first group.
    TrackStart(String),
    /// Demote: not on the track.
    NotOnTrack,
    EmptyTrack(String),
    /// The group is already on the track.
    OnTrack(String),
}

impl fmt::Display for PermsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermsError::GroupExists(g) => write!(f, "group {g} already exists"),
            PermsError::NoGroup(g) => write!(f, "group {g} does not exist"),
            PermsError::TrackExists(t) => write!(f, "track {t} already exists"),
            PermsError::NoTrack(t) => write!(f, "track {t} does not exist"),
            PermsError::BadName(n) => write!(f, "invalid name {n}"),
            PermsError::DefaultGroup => f.write_str("the default group cannot be removed"),
            PermsError::Cycle(g) => write!(f, "{g} already inherits this group"),
            PermsError::Unchanged => f.write_str("nothing changed"),
            PermsError::Ambiguous(g) => write!(f, "the user is on several groups of the track: {}", g.join(", ")),
            PermsError::TrackEnd(g) => write!(f, "already on the last group ({g})"),
            PermsError::TrackStart(g) => write!(f, "already on the first group ({g})"),
            PermsError::NotOnTrack => f.write_str("not on the track"),
            PermsError::EmptyTrack(t) => write!(f, "track {t} has no groups"),
            PermsError::OnTrack(g) => write!(f, "group {g} is already on the track"),
        }
    }
}

impl std::error::Error for PermsError {}

/// A user by name, as known from earlier joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownName {
    pub uuid: String,
    pub name: String,
}

/// The result of a promotion or demotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// What a change touched, so that exactly that is saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dirty {
    pub groups: BTreeSet<String>,
    pub users: BTreeSet<String>,
    pub tracks: BTreeSet<String>,
    pub deleted_groups: BTreeSet<String>,
    pub deleted_tracks: BTreeSet<String>,
}

impl Dirty {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            && self.users.is_empty()
            && self.tracks.is_empty()
            && self.deleted_groups.is_empty()
            && self.deleted_tracks.is_empty()
    }

    pub fn merge(&mut self, other: Dirty) {
        self.groups.extend(other.groups);
        self.users.extend(other.users);
        self.tracks.extend(other.tracks);
        self.deleted_groups.extend(other.deleted_groups);
        self.deleted_tracks.extend(other.deleted_tracks);
    }
}

/// The whole permission database, in memory.
#[derive(Debug, Clone)]
pub struct Perms {
    groups: BTreeMap<String, Group>,
    users: HashMap<String, User>,
    names: HashMap<String, KnownName>,
    tracks: BTreeMap<String, Track>,
    default_group: String,
    generation: u64,
    cache: HashMap<(String, Query), Effective>,
}

impl Default for Perms {
    fn default() -> Self {
        Self::new()
    }
}

impl Perms {
    /// An empty database with the default group.
    pub fn new() -> Self {
        let mut groups = BTreeMap::new();
        groups.insert(DEFAULT_GROUP.to_string(), Group::new(DEFAULT_GROUP));
        Self {
            groups,
            users: HashMap::new(),
            names: HashMap::new(),
            tracks: BTreeMap::new(),
            default_group: DEFAULT_GROUP.to_string(),
            generation: 1,
            cache: HashMap::new(),
        }
    }

    /// Builds the database from stored records. The default group is created
    /// when missing (returned as dirty so it gets saved).
    pub fn from_parts(
        groups: Vec<Group>,
        users: Vec<User>,
        names: Vec<KnownName>,
        tracks: Vec<Track>,
    ) -> (Self, Dirty) {
        let mut p = Self::new();
        p.groups.clear();
        for g in groups {
            p.groups.insert(g.name.clone(), g);
        }
        for u in users {
            p.users.insert(u.uuid.clone(), u);
        }
        for n in names {
            p.names.insert(pumbo_common::id::name_key(&n.name), n);
        }
        for t in tracks {
            p.tracks.insert(t.name.clone(), t);
        }
        let mut dirty = Dirty::default();
        if !p.groups.contains_key(DEFAULT_GROUP) {
            p.groups.insert(DEFAULT_GROUP.to_string(), Group::new(DEFAULT_GROUP));
            dirty.groups.insert(DEFAULT_GROUP.to_string());
        }
        (p, dirty)
    }

    /// Counts the change; cached decisions are rebuilt.
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cache.clear();
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn default_group(&self) -> &str {
        &self.default_group
    }

    // ----- lookups -----

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.get(name)
    }

    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.groups.values()
    }

    pub fn user(&self, uuid: &str) -> Option<&User> {
        self.users.get(uuid)
    }

    pub fn users(&self) -> impl Iterator<Item = &User> {
        self.users.values()
    }

    pub fn track(&self, name: &str) -> Option<&Track> {
        self.tracks.get(name)
    }

    pub fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.values()
    }

    pub fn known_names(&self) -> impl Iterator<Item = &KnownName> {
        self.names.values()
    }

    /// A user by nickname (from earlier joins) or by UUID.
    pub fn find_user(&self, name_or_uuid: &str) -> Option<KnownName> {
        if let Some(uuid) = pumbo_common::id::Uuid::parse(name_or_uuid) {
            let uuid = uuid.to_string();
            let name = self
                .users
                .get(&uuid)
                .map(|u| u.name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| self.names.values().find(|n| n.uuid == uuid).map(|n| n.name.clone()))
                .unwrap_or_else(|| uuid.clone());
            return Some(KnownName { uuid, name });
        }
        self.names.get(&pumbo_common::id::name_key(name_or_uuid)).cloned()
    }

    /// Records a nickname seen at join. Returns true when the stored name index
    /// changed (and should be saved).
    pub fn seen(&mut self, uuid: &str, name: &str) -> bool {
        let key = pumbo_common::id::name_key(name);
        let entry = KnownName { uuid: uuid.to_string(), name: name.to_string() };
        let changed = self.names.get(&key) != Some(&entry);
        if changed {
            // a name now belongs to another account: drop the old mapping of this uuid
            self.names.retain(|_, n| n.uuid != uuid);
            self.names.insert(key, entry);
        }
        if let Some(u) = self.users.get_mut(uuid)
            && u.name != name
        {
            u.name = name.to_string();
            return true;
        }
        changed
    }

    /// Users that have `group` as a direct parent (in any context).
    pub fn members(&self, group: &str) -> Vec<&User> {
        let mut out: Vec<&User> =
            self.users.values().filter(|u| u.data.parents.iter().any(|p| p.group == group)).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.uuid.cmp(&b.uuid)));
        out
    }

    /// Groups that have `group` as a direct parent.
    pub fn child_groups(&self, group: &str) -> Vec<&Group> {
        self.groups.values().filter(|g| g.data.parents.iter().any(|p| p.group == group)).collect()
    }

    /// Tracks a group is on.
    pub fn tracks_of(&self, group: &str) -> Vec<&Track> {
        self.tracks.values().filter(|t| t.groups.iter().any(|g| g == group)).collect()
    }

    /// The earliest end time of any temporary node.
    pub fn next_expiry(&self) -> Option<u64> {
        let g = self.groups.values().filter_map(|g| g.data.next_expiry());
        let u = self.users.values().filter_map(|u| u.data.next_expiry());
        g.chain(u).min()
    }

    /// Whether any node uses the context key (`world`): if none does, a player
    /// changing worlds changes nothing.
    pub fn uses_context(&self, key: &str) -> bool {
        self.groups.values().any(|g| g.data.uses_context(key)) || self.users.values().any(|u| u.data.uses_context(key))
    }

    /// Every permission node written anywhere (for platforms that have to set
    /// nodes one by one).
    pub fn all_nodes(&self) -> BTreeSet<String> {
        let g = self.groups.values().flat_map(|g| g.data.permissions.iter());
        let u = self.users.values().flat_map(|u| u.data.permissions.iter());
        g.chain(u).map(|p| p.node.clone()).collect()
    }

    // ----- deciding -----

    /// The cached decisions for a user at a place.
    pub fn effective(&mut self, uuid: &str, query: &Query, now: u64) -> &Effective {
        let key = (uuid.to_string(), query.clone());
        let generation = self.generation;
        let stale = self.cache.get(&key).is_none_or(|e| e.generation != generation || now >= e.valid_until);
        if stale {
            if self.cache.len() >= MAX_CACHE {
                self.cache.clear();
            }
            let e = self.compute(uuid, query, now);
            self.cache.insert(key.clone(), e);
        }
        self.cache.entry(key).or_default()
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
        let mut direct: Vec<&str> = applicable_parents(data, query, now)
            .filter(|p| self.groups.contains_key(&p.group))
            .map(|p| p.group.as_str())
            .collect();
        if direct.is_empty() && self.groups.contains_key(&self.default_group) {
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

        // permissions: holders in order, on each holder the best node first
        let mut exact: HashMap<String, Decision> = HashMap::new();
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
        for (g, _) in &groups {
            let n = node::group_node(g);
            exact.entry(n.clone()).or_insert_with(|| Decision {
                value: true,
                node: n,
                source: Source::Group(g.clone()),
                context: Contexts::global(),
                expiry: None,
            });
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
            generation: self.generation,
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

    // ----- changes -----

    pub fn create_group(&mut self, name: &str) -> Result<Dirty, PermsError> {
        if !node::is_valid_name(name) {
            return Err(PermsError::BadName(name.to_string()));
        }
        if self.groups.contains_key(name) {
            return Err(PermsError::GroupExists(name.to_string()));
        }
        self.groups.insert(name.to_string(), Group::new(name));
        self.touch();
        Ok(Dirty { groups: [name.to_string()].into(), ..Dirty::default() })
    }

    /// Deletes a group and every reference to it (parents, tracks).
    pub fn delete_group(&mut self, name: &str) -> Result<Dirty, PermsError> {
        if name == self.default_group {
            return Err(PermsError::DefaultGroup);
        }
        if self.groups.remove(name).is_none() {
            return Err(PermsError::NoGroup(name.to_string()));
        }
        let mut dirty = Dirty { deleted_groups: [name.to_string()].into(), ..Dirty::default() };
        for g in self.groups.values_mut() {
            if g.data.remove_group(name) > 0 {
                dirty.groups.insert(g.name.clone());
            }
        }
        for u in self.users.values_mut() {
            if u.data.remove_group(name) > 0 {
                dirty.users.insert(u.uuid.clone());
            }
        }
        for t in self.tracks.values_mut() {
            let before = t.groups.len();
            t.groups.retain(|g| g != name);
            if t.groups.len() != before {
                dirty.tracks.insert(t.name.clone());
            }
        }
        self.touch();
        Ok(dirty)
    }

    /// Changes a group with `f` (after checking it exists).
    pub fn edit_group<R>(&mut self, name: &str, f: impl FnOnce(&mut Group) -> R) -> Result<(R, Dirty), PermsError> {
        let group = self.groups.get_mut(name).ok_or_else(|| PermsError::NoGroup(name.to_string()))?;
        let r = f(group);
        self.touch();
        Ok((r, Dirty { groups: [name.to_string()].into(), ..Dirty::default() }))
    }

    /// Changes a user's data with `f`, creating the record when needed.
    pub fn edit_user<R>(&mut self, uuid: &str, name: &str, f: impl FnOnce(&mut Data) -> R) -> (R, Dirty) {
        let user = self.users.entry(uuid.to_string()).or_insert_with(|| User {
            uuid: uuid.to_string(),
            name: name.to_string(),
            data: Data::default(),
        });
        if !name.is_empty() && user.name != name {
            user.name = name.to_string();
        }
        let r = f(&mut user.data);
        self.touch();
        (r, Dirty { users: [uuid.to_string()].into(), ..Dirty::default() })
    }

    /// Whether making `parent` a parent of `group` would close a loop.
    pub fn would_cycle(&self, group: &str, parent: &str) -> bool {
        if group == parent {
            return true;
        }
        // does `parent` already inherit `group` (in any context)?
        let mut seen = HashSet::new();
        let mut stack = vec![parent];
        while let Some(g) = stack.pop() {
            if g == group {
                return true;
            }
            if !seen.insert(g) || seen.len() > MAX_GROUPS {
                continue;
            }
            if let Some(gr) = self.groups.get(g) {
                stack.extend(gr.data.parents.iter().map(|p| p.group.as_str()));
            }
        }
        false
    }

    /// Adds a parent to a group, refusing loops and unknown groups.
    pub fn add_group_parent(&mut self, group: &str, parent: ParentNode) -> Result<Dirty, PermsError> {
        if !self.groups.contains_key(&parent.group) {
            return Err(PermsError::NoGroup(parent.group));
        }
        if self.would_cycle(group, &parent.group) {
            return Err(PermsError::Cycle(parent.group));
        }
        let (added, dirty) = self.edit_group(group, |g| g.data.add_parent(parent))?;
        if added { Ok(dirty) } else { Err(PermsError::Unchanged) }
    }

    pub fn rename_group(&mut self, from: &str, to: &str) -> Result<Dirty, PermsError> {
        if from == self.default_group {
            return Err(PermsError::DefaultGroup);
        }
        if !node::is_valid_name(to) {
            return Err(PermsError::BadName(to.to_string()));
        }
        if self.groups.contains_key(to) {
            return Err(PermsError::GroupExists(to.to_string()));
        }
        let mut group = self.groups.remove(from).ok_or_else(|| PermsError::NoGroup(from.to_string()))?;
        group.name = to.to_string();
        self.groups.insert(to.to_string(), group);
        let mut dirty =
            Dirty { groups: [to.to_string()].into(), deleted_groups: [from.to_string()].into(), ..Dirty::default() };
        for g in self.groups.values_mut() {
            if g.data.parents.iter().any(|p| p.group == from) {
                g.data.rename_group(from, to);
                dirty.groups.insert(g.name.clone());
            }
        }
        for u in self.users.values_mut() {
            if u.data.parents.iter().any(|p| p.group == from) {
                u.data.rename_group(from, to);
                dirty.users.insert(u.uuid.clone());
            }
        }
        for t in self.tracks.values_mut() {
            if t.groups.iter().any(|g| g == from) {
                for g in &mut t.groups {
                    if g == from {
                        *g = to.to_string();
                    }
                }
                dirty.tracks.insert(t.name.clone());
            }
        }
        self.touch();
        Ok(dirty)
    }

    pub fn create_track(&mut self, name: &str) -> Result<Dirty, PermsError> {
        if !node::is_valid_name(name) {
            return Err(PermsError::BadName(name.to_string()));
        }
        if self.tracks.contains_key(name) {
            return Err(PermsError::TrackExists(name.to_string()));
        }
        self.tracks.insert(name.to_string(), Track { name: name.to_string(), groups: Vec::new() });
        self.touch();
        Ok(Dirty { tracks: [name.to_string()].into(), ..Dirty::default() })
    }

    pub fn delete_track(&mut self, name: &str) -> Result<Dirty, PermsError> {
        self.tracks.remove(name).ok_or_else(|| PermsError::NoTrack(name.to_string()))?;
        self.touch();
        Ok(Dirty { deleted_tracks: [name.to_string()].into(), ..Dirty::default() })
    }

    /// Puts a group on a track at `position` (0-based; past the end appends).
    pub fn track_insert(&mut self, track: &str, group: &str, position: Option<usize>) -> Result<Dirty, PermsError> {
        if !self.groups.contains_key(group) {
            return Err(PermsError::NoGroup(group.to_string()));
        }
        let t = self.tracks.get_mut(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        if t.groups.iter().any(|g| g == group) {
            return Err(PermsError::OnTrack(group.to_string()));
        }
        let at = position.unwrap_or(t.groups.len()).min(t.groups.len());
        t.groups.insert(at, group.to_string());
        self.touch();
        Ok(Dirty { tracks: [track.to_string()].into(), ..Dirty::default() })
    }

    pub fn track_remove(&mut self, track: &str, group: &str) -> Result<Dirty, PermsError> {
        let t = self.tracks.get_mut(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        let before = t.groups.len();
        t.groups.retain(|g| g != group);
        if t.groups.len() == before {
            return Err(PermsError::NoGroup(group.to_string()));
        }
        self.touch();
        Ok(Dirty { tracks: [track.to_string()].into(), ..Dirty::default() })
    }

    pub fn track_clear(&mut self, track: &str) -> Result<Dirty, PermsError> {
        let t = self.tracks.get_mut(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        t.groups.clear();
        self.touch();
        Ok(Dirty { tracks: [track.to_string()].into(), ..Dirty::default() })
    }

    /// Moves a user one group up a track (in exactly these contexts). A user
    /// not on the track gets its first group.
    pub fn promote(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
    ) -> Result<(Step, Dirty), PermsError> {
        self.step(uuid, name, track, context, true)
    }

    /// Moves a user one group down a track. The first group is the bottom: a
    /// user there stays (use `parent remove` to take it off the track).
    pub fn demote(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
    ) -> Result<(Step, Dirty), PermsError> {
        self.step(uuid, name, track, context, false)
    }

    fn step(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
        up: bool,
    ) -> Result<(Step, Dirty), PermsError> {
        let t = self.tracks.get(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        let ladder = t.groups.clone();
        let first = ladder.first().cloned().ok_or_else(|| PermsError::EmptyTrack(track.to_string()))?;
        let empty = Data::default();
        let data = self.users.get(uuid).map(|u| &u.data).unwrap_or(&empty);
        let mut on: Vec<&String> =
            ladder.iter().filter(|g| data.parents.iter().any(|p| &p.group == *g && &p.context == context)).collect();
        on.dedup();
        let (from, to) = match on.as_slice() {
            [] if up => (None, first),
            [] => return Err(PermsError::NotOnTrack),
            [current] => {
                let i = ladder.iter().position(|g| g == *current).unwrap_or(0);
                let next = if up { i.checked_add(1) } else { i.checked_sub(1) };
                match next.and_then(|n| ladder.get(n)) {
                    Some(n) => (Some((*current).clone()), n.clone()),
                    None if up => return Err(PermsError::TrackEnd((*current).clone())),
                    None => return Err(PermsError::TrackStart((*current).clone())),
                }
            }
            many => return Err(PermsError::Ambiguous(many.iter().map(|g| (*g).clone()).collect())),
        };
        if !self.groups.contains_key(&to) {
            return Err(PermsError::NoGroup(to));
        }
        let from_c = from.clone();
        let to_c = to.clone();
        let ((), dirty) = self.edit_user(uuid, name, |d| {
            if let Some(f) = &from_c {
                d.remove_parent(f, context);
            }
            d.add_parent(ParentNode { group: to_c, context: context.clone(), expiry: None });
        });
        Ok((Step { from, to: Some(to) }, dirty))
    }

    /// Removes expired nodes everywhere. Returns what changed.
    pub fn expire(&mut self, now: u64) -> Dirty {
        let mut dirty = Dirty::default();
        for g in self.groups.values_mut() {
            if g.data.expire(now) > 0 {
                dirty.groups.insert(g.name.clone());
            }
        }
        for u in self.users.values_mut() {
            if u.data.expire(now) > 0 {
                dirty.users.insert(u.uuid.clone());
            }
        }
        if !dirty.is_empty() {
            self.touch();
        }
        dirty
    }

    /// Replaces everything (import). Missing default group is added.
    pub fn replace_all(&mut self, groups: Vec<Group>, users: Vec<User>, tracks: Vec<Track>) {
        let names: Vec<KnownName> = self.names.values().cloned().collect();
        let (mut fresh, _) = Self::from_parts(groups, users, names, tracks);
        fresh.generation = self.generation.wrapping_add(1);
        *self = fresh;
    }
}

fn applicable_parents<'a>(data: &'a Data, query: &'a Query, now: u64) -> impl Iterator<Item = &'a ParentNode> + 'a {
    data.parents.iter().filter(move |p| alive(p.expiry, now) && p.context.applies(query))
}
