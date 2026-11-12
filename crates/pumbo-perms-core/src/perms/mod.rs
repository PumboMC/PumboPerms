//! The permission database in memory and how a permission is decided.
//!
//! # Rules
//!
//! 1. **Holders.** A user inherits the groups of its parent nodes that apply
//!    where the player is (contexts) and have not expired, and their parents in
//!    turn. A user without an active parent that has no context is also in the
//!    default group, the same everywhere, so every host computes it alike. The
//!    order of importance is: the user itself, then the groups by weight
//!    (highest first), then by distance (direct parents before their parents),
//!    then by name.
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
//! Every inherited group answers its `group.<name>` node with `true`, before
//! any node set by hand, so `group.<name>` always tells the real membership.
//!
//! Prefix and suffix: the highest priority among all holders wins; on a tie the
//! more important holder, then the more specific context. Meta keys: the first
//! holder (in the order of rule 1) that has the key; on that holder the most
//! specific context.

use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::context::{Contexts, Query};
use crate::model::{Data, Group, Track, User};

mod change;
mod decide;

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
        // The wildcards of `node::parents`, most specific first, in one buffer.
        let base = node.strip_suffix(".*").or_else(|| node.strip_suffix(":*")).unwrap_or(node);
        let mut wildcard = String::with_capacity(base.len().saturating_add(1));
        for (i, c) in base.char_indices().rev() {
            if (c == '.' || c == ':')
                && let Some(head) = base.get(..=i)
            {
                wildcard.clear();
                wildcard.push_str(head);
                wildcard.push('*');
                if let Some(d) = self.exact.get(&wildcard) {
                    return Some(d);
                }
            }
        }
        self.exact.get("*")
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownName {
    pub uuid: String,
    pub name: String,
    /// The last join was with an account the host checked with Mojang: only
    /// such a name may stand for a UUID in an import.
    pub online: bool,
}

/// The result of a promotion or demotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub from: Option<String>,
    pub to: Option<String>,
    /// End time of the new rung (`None`: permanent).
    pub expiry: Option<u64>,
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

/// The groups, users and tracks of [`Perms`] ([`Perms::saved`]).
#[derive(Debug, Clone)]
pub struct Saved {
    groups: BTreeMap<String, Group>,
    users: HashMap<String, User>,
    tracks: BTreeMap<String, Track>,
}

/// The whole permission database, in memory.
#[derive(Debug, Clone)]
pub struct Perms {
    groups: BTreeMap<String, Group>,
    users: HashMap<String, User>,
    tracks: BTreeMap<String, Track>,
    default_group: String,
    generation: u64,
    memo: Memo,
}

/// What is computed from the data and kept until the data changes. A copy of
/// [`Perms`] starts without it.
#[derive(Debug, Default)]
struct Memo {
    /// Decisions per user and place.
    cache: HashMap<String, HashMap<Query, Effective>>,
    /// Entries in `cache`.
    cached: usize,
    /// [`Perms::all_nodes`].
    nodes: OnceCell<BTreeSet<String>>,
    /// [`Perms::next_expiry`].
    next_expiry: Cell<Option<Option<u64>>>,
    /// [`Perms::uses_context`] of `world`.
    worlds: Cell<Option<bool>>,
    /// What a user with nothing gets (never handed out: every lookup follows a computation).
    nothing: Effective,
}

impl Clone for Memo {
    fn clone(&self) -> Self {
        Self::default()
    }
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
            tracks: BTreeMap::new(),
            default_group: DEFAULT_GROUP.to_string(),
            generation: 1,
            memo: Memo::default(),
        }
    }

    /// Builds the database from stored records. The default group is created
    /// when missing (returned as dirty so it gets saved).
    pub fn from_parts(groups: Vec<Group>, users: Vec<User>, tracks: Vec<Track>) -> (Self, Dirty) {
        let mut p = Self::new();
        p.groups.clear();
        for g in groups {
            p.groups.insert(g.name.clone(), g);
        }
        for u in users {
            p.users.insert(u.uuid.clone(), u);
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

    /// Groups, users and tracks as they are now, to [`Perms::restore`] after a
    /// change that could not be saved.
    pub fn saved(&self) -> Saved {
        Saved { groups: self.groups.clone(), users: self.users.clone(), tracks: self.tracks.clone() }
    }

    pub fn restore(&mut self, saved: Saved) {
        self.groups = saved.groups;
        self.users = saved.users;
        self.tracks = saved.tracks;
        self.generation = self.generation.wrapping_add(1);
        self.memo = Memo::default();
    }

    /// Counts the change and forgets what was computed from the data it
    /// touched: the decisions of its users and of everyone who can inherit
    /// one of its groups.
    fn touch(&mut self, dirty: &Dirty) {
        self.generation = self.generation.wrapping_add(1);
        let groups: Vec<&str> = dirty.groups.iter().chain(&dirty.deleted_groups).map(String::as_str).collect();
        if groups.is_empty() && dirty.users.is_empty() {
            return;
        }
        (self.memo.nodes, self.memo.next_expiry, self.memo.worlds) =
            (OnceCell::new(), Cell::new(None), Cell::new(None));
        if self.reaches_everyone(&groups) {
            (self.memo.cache, self.memo.cached) = (HashMap::new(), 0);
            return;
        }
        let gone: Vec<String> = self
            .memo
            .cache
            .keys()
            .filter(|u| dirty.users.contains(*u) || (!groups.is_empty() && self.reaches(u, &groups)))
            .cloned()
            .collect();
        for u in gone {
            if let Some(places) = self.memo.cache.remove(&u) {
                self.memo.cached = self.memo.cached.saturating_sub(places.len());
            }
        }
    }

    /// Every `server=` and `group=` value in what the user has or can inherit:
    /// on a server named nowhere there, directly or by a server group, the
    /// decisions are the global ones.
    pub fn places(&self, uuid: &str) -> BTreeSet<(&str, &str)> {
        let mut out = BTreeSet::new();
        fn note<'a>(out: &mut BTreeSet<(&'a str, &'a str)>, data: &'a Data) {
            let contexts = data.permissions.iter().map(|p| &p.context);
            let contexts = contexts.chain(data.parents.iter().map(|p| &p.context));
            for c in contexts.chain(data.meta.iter().map(|m| &m.context)) {
                out.extend(c.iter().filter(|(k, _)| *k != crate::context::WORLD));
            }
        }
        let mut stack: Vec<&str> = vec![&self.default_group];
        if let Some(user) = self.users.get(uuid) {
            note(&mut out, &user.data);
            stack.extend(user.data.parents.iter().map(|p| p.group.as_str()));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(g) = stack.pop() {
            if seen.len() >= MAX_GROUPS || !seen.insert(g) {
                continue;
            }
            if let Some(group) = self.groups.get(g) {
                note(&mut out, &group.data);
                stack.extend(group.data.parents.iter().map(|p| p.group.as_str()));
            }
        }
        out
    }

    /// Whether the user can inherit one of `groups` anywhere, at any time
    /// (also through a parent that names a group not created yet).
    pub fn reaches(&self, uuid: &str, groups: &[&str]) -> bool {
        let Some(user) = self.users.get(uuid) else { return false };
        self.inherits(user.data.parents.iter().map(|p| p.group.as_str()).collect(), groups)
    }

    /// Whether one of `groups` is the default group or one it can inherit:
    /// a change there can reach every user.
    pub fn reaches_everyone(&self, groups: &[&str]) -> bool {
        !groups.is_empty() && self.inherits(vec![&self.default_group], groups)
    }

    fn inherits<'a>(&'a self, mut stack: Vec<&'a str>, groups: &[&str]) -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(g) = stack.pop() {
            if groups.contains(&g) {
                return true;
            }
            if seen.len() >= MAX_GROUPS || !seen.insert(g) {
                continue;
            }
            if let Some(group) = self.groups.get(g) {
                stack.extend(group.data.parents.iter().map(|p| p.group.as_str()));
            }
        }
        false
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

    /// Shows a user's new nickname (`listmembers`, `info`); the record is
    /// saved with its next change. A user without a record gets none.
    pub fn set_user_name(&mut self, uuid: &str, name: &str) {
        if let Some(u) = self.users.get_mut(uuid)
            && u.name != name
        {
            u.name = name.to_string();
        }
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

    /// The earliest end time of any temporary node, kept until the data changes.
    pub fn next_expiry(&self) -> Option<u64> {
        if let Some(kept) = self.memo.next_expiry.get() {
            return kept;
        }
        let g = self.groups.values().filter_map(|g| g.data.next_expiry());
        let u = self.users.values().filter_map(|u| u.data.next_expiry());
        let next = g.chain(u).min();
        self.memo.next_expiry.set(Some(next));
        next
    }

    /// Whether any node uses the context key (`world`): if none does, a player
    /// changing worlds changes nothing.
    pub fn uses_context(&self, key: &str) -> bool {
        let scan = || {
            self.groups.values().any(|g| g.data.uses_context(key))
                || self.users.values().any(|u| u.data.uses_context(key))
        };
        if key != crate::context::WORLD {
            return scan();
        }
        let worlds = self.memo.worlds.get().unwrap_or_else(scan);
        self.memo.worlds.set(Some(worlds));
        worlds
    }

    /// Every permission node written anywhere (for platforms that have to set
    /// nodes one by one), kept until the data changes.
    pub fn all_nodes(&self) -> &BTreeSet<String> {
        self.memo.nodes.get_or_init(|| {
            let g = self.groups.values().flat_map(|g| g.data.permissions.iter());
            let u = self.users.values().flat_map(|u| u.data.permissions.iter());
            g.chain(u).map(|p| p.node.clone()).collect()
        })
    }
}
