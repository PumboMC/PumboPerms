//! Players whose records stay on disk: what memory counts of them, and their
//! records coming into memory and leaving it.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Bound;

use crate::model::{Group, Track, User, alive};

use super::{Dirty, Perms};

/// Members of one group among the records that are not in memory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Counted {
    /// Players with a parent of the group that never ends.
    lasting: u32,
    /// Players whose last parent of the group ends at the key (ms).
    ending: BTreeMap<u64, u32>,
}

impl Counted {
    /// The players still in the group at `now`.
    fn alive(&self, now: u64) -> usize {
        let ending = self.ending.range((Bound::Excluded(now), Bound::Unbounded)).map(|(_, n)| count(*n));
        ending.fold(count(self.lasting), usize::saturating_add)
    }
}

fn count(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// What memory knows about the players whose records stay on disk.
#[derive(Debug, Clone, Default)]
pub struct Offline {
    /// Players whose record is in memory, or known to have none.
    loaded: HashSet<String>,
    members: BTreeMap<String, Counted>,
    /// Every permission node of a record on disk; it only grows until the next load.
    nodes: BTreeSet<String>,
}

impl Offline {
    /// Counts a record that stays on disk.
    pub fn add(&mut self, user: &User) {
        for (group, end) in memberships(user) {
            let counted = self.members.entry(group).or_default();
            match end {
                None => counted.lasting = counted.lasting.saturating_add(1),
                Some(t) => {
                    let n = counted.ending.entry(t).or_default();
                    *n = n.saturating_add(1);
                }
            }
        }
        self.nodes.extend(user.data.permissions.iter().map(|p| p.node.clone()));
    }

    /// Takes back what [`Offline::add`] counted for `user`.
    fn remove(&mut self, user: &User) {
        for (group, end) in memberships(user) {
            let Some(counted) = self.members.get_mut(&group) else { continue };
            match end {
                None => counted.lasting = counted.lasting.saturating_sub(1),
                Some(t) => {
                    if let Some(n) = counted.ending.get_mut(&t) {
                        *n = n.saturating_sub(1);
                        if *n == 0 {
                            counted.ending.remove(&t);
                        }
                    }
                }
            }
            if counted.lasting == 0 && counted.ending.is_empty() {
                self.members.remove(&group);
            }
        }
    }
}

/// What a record adds to the member counts: every group among its parents
/// once (in any context), without an end when one of those parents has none,
/// otherwise with the latest end. It depends on the record alone, so a record
/// leaving memory adds back exactly what it took coming in.
fn memberships(user: &User) -> BTreeMap<String, Option<u64>> {
    let mut out: BTreeMap<String, Option<u64>> = BTreeMap::new();
    for p in &user.data.parents {
        out.entry(p.group.clone())
            .and_modify(|end| *end = end.zip(p.expiry).map(|(a, b)| a.max(b)))
            .or_insert(p.expiry);
    }
    out
}

impl Perms {
    /// The groups and tracks of the database; its players stay on disk,
    /// counted in `offline`. The default group is created when missing.
    pub fn from_disk(groups: Vec<Group>, tracks: Vec<Track>, offline: Offline) -> (Self, Dirty) {
        let (mut p, dirty) = Self::from_parts(groups, Vec::new(), tracks);
        p.offline = Some(offline);
        (p, dirty)
    }

    /// Whether records stay on disk until a player is needed.
    pub fn lazy(&self) -> bool {
        self.offline.is_some()
    }

    /// Whether memory has the record of `uuid`, or knows there is none.
    pub fn knows(&self, uuid: &str) -> bool {
        self.offline.as_ref().is_none_or(|o| o.loaded.contains(uuid))
    }

    /// Brings the record of `uuid` from disk into memory (`None`: it has
    /// none). Refused for a player memory has a record of without having read
    /// it: a change made that record, and saving it stays refused.
    pub fn admit(&mut self, uuid: &str, user: Option<User>) -> bool {
        let Some(offline) = &mut self.offline else { return true };
        if offline.loaded.contains(uuid) {
            return true;
        }
        if self.users.contains_key(uuid) {
            return false;
        }
        if let Some(u) = &user {
            offline.remove(u);
        }
        offline.loaded.insert(uuid.to_string());
        if let Some(u) = user {
            self.users.insert(uuid.to_string(), u);
        }
        self.forget(uuid);
        true
    }

    /// Lets the record of `uuid` go back to disk, where it is the same; not
    /// while a change is in progress.
    pub fn evict(&mut self, uuid: &str) -> bool {
        if self.changing() {
            return false;
        }
        let Some(offline) = &mut self.offline else { return false };
        if !offline.loaded.remove(uuid) {
            return false;
        }
        if let Some(u) = self.users.remove(uuid) {
            offline.add(&u);
        }
        self.forget(uuid);
        true
    }

    /// Forgets what was computed for `uuid` and from every record: a record
    /// came or went.
    fn forget(&mut self, uuid: &str) {
        if let Some(places) = self.memo.cache.remove(uuid) {
            self.memo.cached = self.memo.cached.saturating_sub(places.len());
        }
        self.memo.next_expiry.set(None);
        self.memo.worlds.set(None);
    }

    /// The players in memory with a live parent `group` (in any context),
    /// by name and UUID.
    pub fn members_at(&self, group: &str, now: u64) -> Vec<&User> {
        let mut out: Vec<&User> = self
            .users
            .values()
            .filter(|u| u.data.parents.iter().any(|p| p.group == group && alive(p.expiry, now)))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.uuid.cmp(&b.uuid)));
        out
    }

    /// How many players have a live parent `group` at `now`: those in memory
    /// and, from the counts, those on disk.
    pub fn member_count(&self, group: &str, now: u64) -> usize {
        let on_disk = self.offline.as_ref().and_then(|o| o.members.get(group)).map_or(0, |c| c.alive(now));
        self.members_at(group, now).len().saturating_add(on_disk)
    }

    /// The nodes of the records on disk.
    pub(super) fn offline_nodes(&self) -> Option<&BTreeSet<String>> {
        self.offline.as_ref().map(|o| &o.nodes)
    }
}
