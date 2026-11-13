//! Changes to groups, users and tracks, and expiry.

use std::collections::HashSet;

use crate::context::Contexts;
use crate::model::{Data, Group, ParentNode, Track, User, alive};
use crate::node;

use super::{Dirty, MAX_GROUPS, Perms, PermsError, Step};

impl Perms {
    pub fn create_group(&mut self, name: &str) -> Result<Dirty, PermsError> {
        if !node::is_valid_name(name) {
            return Err(PermsError::BadName(name.to_string()));
        }
        if self.groups.contains_key(name) {
            return Err(PermsError::GroupExists(name.to_string()));
        }
        self.groups.insert(name.to_string(), Group::new(name));
        let dirty = Dirty { groups: [name.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
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
        self.touch(&dirty);
        Ok(dirty)
    }

    /// Changes a group with `f` (after checking it exists).
    pub fn edit_group<R>(&mut self, name: &str, f: impl FnOnce(&mut Group) -> R) -> Result<(R, Dirty), PermsError> {
        let group = self.groups.get_mut(name).ok_or_else(|| PermsError::NoGroup(name.to_string()))?;
        let r = f(group);
        let dirty = Dirty { groups: [name.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok((r, dirty))
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
        let dirty = Dirty { users: [uuid.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        (r, dirty)
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
        self.touch(&dirty);
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
        let dirty = Dirty { tracks: [name.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
    }

    pub fn delete_track(&mut self, name: &str) -> Result<Dirty, PermsError> {
        self.tracks.remove(name).ok_or_else(|| PermsError::NoTrack(name.to_string()))?;
        let dirty = Dirty { deleted_tracks: [name.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
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
        let dirty = Dirty { tracks: [track.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
    }

    pub fn track_remove(&mut self, track: &str, group: &str) -> Result<Dirty, PermsError> {
        let t = self.tracks.get_mut(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        let before = t.groups.len();
        t.groups.retain(|g| g != group);
        if t.groups.len() == before {
            return Err(PermsError::NoGroup(group.to_string()));
        }
        let dirty = Dirty { tracks: [track.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
    }

    pub fn track_clear(&mut self, track: &str) -> Result<Dirty, PermsError> {
        let t = self.tracks.get_mut(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        t.groups.clear();
        let dirty = Dirty { tracks: [track.to_string()].into(), ..Dirty::default() };
        self.touch(&dirty);
        Ok(dirty)
    }

    /// Moves a user one group up a track (in exactly these contexts). A user
    /// not on the track gets its first group, or the second one when the
    /// first is the default group the user is already in.
    pub fn promote(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
        now: u64,
    ) -> Result<(Step, Dirty), PermsError> {
        self.step(uuid, name, track, context, true, now)
    }

    /// Moves a user one group down a track. The first group is the bottom: a
    /// user there stays (use `parent remove` to take it off the track).
    pub fn demote(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
        now: u64,
    ) -> Result<(Step, Dirty), PermsError> {
        self.step(uuid, name, track, context, false, now)
    }

    /// Makes the step [`Perms::step_plan`] works out.
    fn step(
        &mut self,
        uuid: &str,
        name: &str,
        track: &str,
        context: &Contexts,
        up: bool,
        now: u64,
    ) -> Result<(Step, Dirty), PermsError> {
        let plan = self.step_plan(uuid, track, context, up, now)?;
        let from_c = plan.from.clone();
        let to_c = plan.to.clone();
        let expiry = plan.expiry;
        let ((), dirty) = self.edit_user(uuid, name, |d| {
            if let Some(f) = &from_c {
                d.remove_parent(f, context, false);
            }
            if let Some(group) = to_c {
                d.add_parent(ParentNode { group, context: context.clone(), expiry });
            }
        });
        Ok((plan, dirty))
    }

    /// One rung along a track among the live parents, worked out without
    /// changing anything ([`Perms::promote`] and [`Perms::demote`] make it);
    /// the new rung keeps the end time of the old one (a 30-day `vip` gives a
    /// 30-day `mvp`).
    pub fn step_plan(
        &self,
        uuid: &str,
        track: &str,
        context: &Contexts,
        up: bool,
        now: u64,
    ) -> Result<Step, PermsError> {
        let t = self.tracks.get(track).ok_or_else(|| PermsError::NoTrack(track.to_string()))?;
        let ladder = t.groups.clone();
        let first = ladder.first().cloned().ok_or_else(|| PermsError::EmptyTrack(track.to_string()))?;
        let empty = Data::default();
        let data = self.users.get(uuid).map(|u| &u.data).unwrap_or(&empty);
        let ends = |g: &str| -> Vec<Option<u64>> {
            let live = data.parents.iter().filter(|p| p.group == g && &p.context == context && alive(p.expiry, now));
            live.map(|p| p.expiry).collect()
        };
        let mut on: Vec<&String> = ladder.iter().filter(|g| !ends(g).is_empty()).collect();
        on.dedup();
        // A permanent parent wins over a temporary one; of two temporary ones the later end.
        let expiry = on.first().and_then(|g| {
            let ends = ends(g);
            if ends.contains(&None) { None } else { ends.into_iter().flatten().max() }
        });
        // The default group counts as the first rung for a user without an
        // active parent that has no context (the rule of `default`).
        let in_default =
            first == self.default_group && !data.parents.iter().any(|p| alive(p.expiry, now) && p.context.is_global());
        let (from, to) = match on.as_slice() {
            [] if up && in_default => match ladder.get(1) {
                Some(next) => (Some(first), next.clone()),
                None => return Err(PermsError::TrackEnd(first)),
            },
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
        Ok(Step { from, to: Some(to), expiry })
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
            self.touch(&dirty);
        }
        dirty
    }

    /// Replaces everything (import). Missing default group is added.
    pub fn replace_all(&mut self, groups: Vec<Group>, users: Vec<User>, tracks: Vec<Track>) {
        let (mut fresh, _) = Self::from_parts(groups, users, tracks);
        fresh.generation = self.generation.wrapping_add(1);
        *self = fresh;
    }
}
