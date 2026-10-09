//! The data: groups, users, tracks and the nodes they hold.
//!
//! Users and groups are both "holders": each has permission nodes, parent
//! groups and meta (prefixes, suffixes, key-value pairs). Every node may be
//! limited by [`Contexts`] and may expire. One holder can have the same node
//! twice in the same contexts: once permanent and once temporary; while the
//! temporary one lasts, it wins (it is the newer exception, e.g. a one-hour ban
//! from flying on top of a permanent permission to fly).

use serde::{Deserialize, Serialize};

use crate::context::Contexts;

/// A permission node with its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermNode {
    pub node: String,
    pub value: bool,
    #[serde(default, skip_serializing_if = "Contexts::is_global")]
    pub context: Contexts,
    /// End time in Unix milliseconds; `None` is permanent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
}

/// Inheritance from a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentNode {
    pub group: String,
    #[serde(default, skip_serializing_if = "Contexts::is_global")]
    pub context: Contexts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetaKind {
    Prefix,
    Suffix,
    /// A key-value pair (`key` holds the key).
    Meta,
}

/// A prefix, a suffix or a key-value pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaNode {
    pub kind: MetaKind,
    /// Key of a [`MetaKind::Meta`] pair; empty for prefixes and suffixes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// Priority of a prefix or suffix: the highest one is shown.
    #[serde(default)]
    pub priority: i32,
    pub value: String,
    #[serde(default, skip_serializing_if = "Contexts::is_global")]
    pub context: Contexts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
}

/// Whether a node is still valid at `now`.
pub fn alive(expiry: Option<u64>, now: u64) -> bool {
    expiry.is_none_or(|e| e > now)
}

/// Everything one holder has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<ParentNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meta: Vec<MetaNode>,
}

impl Data {
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty() && self.parents.is_empty() && self.meta.is_empty()
    }

    /// Sets a permission, replacing the node with the same name, contexts and
    /// kind (permanent or temporary). Returns the replaced node.
    pub fn set_permission(&mut self, new: PermNode) -> Option<PermNode> {
        let pos = self
            .permissions
            .iter()
            .position(|p| p.node == new.node && p.context == new.context && p.expiry.is_some() == new.expiry.is_some());
        match pos {
            Some(i) => self.permissions.get_mut(i).map(|slot| std::mem::replace(slot, new)),
            None => {
                self.permissions.push(new);
                None
            }
        }
    }

    /// Removes a permission (`temporary` picks the permanent or the temporary one).
    pub fn unset_permission(&mut self, node: &str, context: &Contexts, temporary: bool) -> Option<PermNode> {
        let pos = self
            .permissions
            .iter()
            .position(|p| p.node == node && &p.context == context && p.expiry.is_some() == temporary)?;
        Some(self.permissions.remove(pos))
    }

    /// Removes all permissions, or only those in `context`. Returns how many.
    pub fn clear_permissions(&mut self, context: Option<&Contexts>) -> usize {
        let before = self.permissions.len();
        self.permissions.retain(|p| context.is_some_and(|c| &p.context != c));
        before - self.permissions.len()
    }

    /// Adds a parent group. Re-adding an existing temporary parent replaces its
    /// end time. False when exactly that parent is already there.
    pub fn add_parent(&mut self, new: ParentNode) -> bool {
        let pos = self.parents.iter().position(|p| {
            p.group == new.group && p.context == new.context && p.expiry.is_some() == new.expiry.is_some()
        });
        match pos {
            Some(i) => match self.parents.get_mut(i) {
                Some(slot) if *slot != new => {
                    *slot = new;
                    true
                }
                _ => false,
            },
            None => {
                self.parents.push(new);
                true
            }
        }
    }

    /// Removes a parent in exactly these contexts (permanent and temporary).
    pub fn remove_parent(&mut self, group: &str, context: &Contexts) -> bool {
        let before = self.parents.len();
        self.parents.retain(|p| !(p.group == group && &p.context == context));
        before != self.parents.len()
    }

    /// Makes `group` the only parent in these contexts.
    pub fn set_parent(&mut self, group: &str, context: &Contexts) {
        self.parents.retain(|p| &p.context != context);
        self.parents.push(ParentNode { group: group.to_string(), context: context.clone(), expiry: None });
    }

    /// Sets a key-value pair, replacing the same key in the same contexts.
    pub fn set_meta(&mut self, key: &str, value: &str, context: &Contexts, expiry: Option<u64>) {
        self.meta.retain(|m| {
            !(m.kind == MetaKind::Meta
                && m.key == key
                && &m.context == context
                && m.expiry.is_some() == expiry.is_some())
        });
        self.meta.push(MetaNode {
            kind: MetaKind::Meta,
            key: key.to_string(),
            priority: 0,
            value: value.to_string(),
            context: context.clone(),
            expiry,
        });
    }

    /// Removes a key (permanent and temporary) in these contexts.
    pub fn unset_meta(&mut self, key: &str, context: &Contexts) -> bool {
        let before = self.meta.len();
        self.meta.retain(|m| !(m.kind == MetaKind::Meta && m.key == key && &m.context == context));
        before != self.meta.len()
    }

    /// Adds a prefix or suffix. With `replace`, other ones of the same kind in
    /// these contexts go first (the "set" commands).
    pub fn add_affix(
        &mut self,
        kind: MetaKind,
        priority: i32,
        value: &str,
        context: &Contexts,
        expiry: Option<u64>,
        replace: bool,
    ) -> bool {
        let before = self.meta.clone();
        self.meta.retain(|m| {
            if m.kind != kind || &m.context != context {
                return true;
            }
            if replace {
                return false;
            }
            !(m.priority == priority && m.value == value && m.expiry.is_some() == expiry.is_some())
        });
        self.meta.push(MetaNode {
            kind,
            key: String::new(),
            priority,
            value: value.to_string(),
            context: context.clone(),
            expiry,
        });
        self.meta != before
    }

    /// Removes prefixes or suffixes with this priority (and this text, if given)
    /// in these contexts. Returns how many.
    pub fn remove_affix(&mut self, kind: MetaKind, priority: i32, value: Option<&str>, context: &Contexts) -> usize {
        let before = self.meta.len();
        self.meta.retain(|m| {
            !(m.kind == kind && m.priority == priority && &m.context == context && value.is_none_or(|v| m.value == v))
        });
        before - self.meta.len()
    }

    /// Drops expired nodes; returns how many.
    pub fn expire(&mut self, now: u64) -> usize {
        let before = self.permissions.len() + self.parents.len() + self.meta.len();
        self.permissions.retain(|p| alive(p.expiry, now));
        self.parents.retain(|p| alive(p.expiry, now));
        self.meta.retain(|m| alive(m.expiry, now));
        before - (self.permissions.len() + self.parents.len() + self.meta.len())
    }

    /// The earliest end time of a temporary node.
    pub fn next_expiry(&self) -> Option<u64> {
        let p = self.permissions.iter().filter_map(|p| p.expiry);
        let g = self.parents.iter().filter_map(|p| p.expiry);
        let m = self.meta.iter().filter_map(|m| m.expiry);
        p.chain(g).chain(m).min()
    }

    /// Removes every reference to a group (when it is deleted). Returns how many.
    pub fn remove_group(&mut self, group: &str) -> usize {
        let before = self.parents.len();
        self.parents.retain(|p| p.group != group);
        before - self.parents.len()
    }

    /// Renames references to a group.
    pub fn rename_group(&mut self, from: &str, to: &str) {
        for p in &mut self.parents {
            if p.group == from {
                p.group = to.to_string();
            }
        }
    }

    /// Whether any node uses the context key.
    pub fn uses_context(&self, key: &str) -> bool {
        self.permissions.iter().any(|p| p.context.has(key))
            || self.parents.iter().any(|p| p.context.has(key))
            || self.meta.iter().any(|m| m.context.has(key))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Shown instead of the name (`%rank%`); empty uses the name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    /// Higher weight wins conflicts with other groups.
    #[serde(default)]
    pub weight: i32,
    #[serde(default)]
    pub data: Data,
}

impl Group {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), display_name: String::new(), weight: 0, data: Data::default() }
    }

    pub fn shown_name(&self) -> &str {
        if self.display_name.is_empty() { &self.name } else { &self.display_name }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Hyphenated lowercase UUID (empty for players known only by name).
    #[serde(default)]
    pub uuid: String,
    /// Last known nickname.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub data: Data,
}

/// An ordered ladder of groups for promote and demote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(s: &str) -> Contexts {
        let args: Vec<String> = s.split_whitespace().map(str::to_string).collect();
        Contexts::parse(&args).unwrap()
    }

    fn perm(node: &str, value: bool, c: &str, expiry: Option<u64>) -> PermNode {
        PermNode { node: node.into(), value, context: ctx(c), expiry }
    }

    #[test]
    fn permanent_and_temporary_live_side_by_side() {
        let mut d = Data::default();
        assert_eq!(d.set_permission(perm("fly", true, "", None)), None);
        assert_eq!(d.set_permission(perm("fly", false, "", Some(10))), None);
        assert_eq!(d.permissions.len(), 2);
        // same kind replaces
        assert_eq!(d.set_permission(perm("fly", false, "", None)).map(|p| p.value), Some(true));
        assert_eq!(d.permissions.len(), 2);
        // other contexts are separate nodes
        d.set_permission(perm("fly", true, "world=nether", None));
        assert_eq!(d.permissions.len(), 3);
        assert!(d.unset_permission("fly", &ctx(""), true).is_some());
        assert!(d.unset_permission("fly", &ctx(""), true).is_none());
        assert_eq!(d.clear_permissions(Some(&ctx("world=nether"))), 1);
        assert_eq!(d.clear_permissions(None), 1);
        assert!(d.is_empty());
    }

    #[test]
    fn parents_and_meta() {
        let mut d = Data::default();
        let vip = |expiry| ParentNode { group: "vip".into(), context: Contexts::global(), expiry };
        assert!(d.add_parent(vip(None)));
        assert!(!d.add_parent(vip(None)));
        assert!(d.add_parent(vip(Some(5))));
        assert!(d.add_parent(vip(Some(9))), "a new end time replaces the old one");
        assert_eq!(d.parents.len(), 2);
        d.set_parent("admin", &Contexts::global());
        assert_eq!(d.parents.iter().map(|p| p.group.as_str()).collect::<Vec<_>>(), vec!["admin"]);
        assert!(d.remove_parent("admin", &Contexts::global()));
        assert!(!d.remove_parent("admin", &Contexts::global()));

        d.set_meta("color", "red", &Contexts::global(), None);
        d.set_meta("color", "blue", &Contexts::global(), None);
        assert_eq!(d.meta.len(), 1);
        assert!(d.unset_meta("color", &Contexts::global()));
        assert!(d.add_affix(MetaKind::Prefix, 10, "[A]", &Contexts::global(), None, false));
        assert!(d.add_affix(MetaKind::Prefix, 20, "[B]", &Contexts::global(), None, false));
        assert!(!d.add_affix(MetaKind::Prefix, 20, "[B]", &Contexts::global(), None, false));
        assert_eq!(d.meta.len(), 2);
        assert!(d.add_affix(MetaKind::Prefix, 5, "[C]", &Contexts::global(), None, true));
        assert_eq!(d.meta.len(), 1);
        assert_eq!(d.remove_affix(MetaKind::Prefix, 5, Some("[X]"), &Contexts::global()), 0);
        assert_eq!(d.remove_affix(MetaKind::Prefix, 5, None, &Contexts::global()), 1);
    }

    #[test]
    fn expiry() {
        let mut d = Data::default();
        d.set_permission(perm("a", true, "", Some(100)));
        d.set_permission(perm("b", true, "", None));
        d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: Some(50) });
        d.add_affix(MetaKind::Suffix, 1, "x", &Contexts::global(), Some(70), false);
        assert_eq!(d.next_expiry(), Some(50));
        assert_eq!(d.expire(60), 1);
        assert_eq!(d.next_expiry(), Some(70));
        assert_eq!(d.expire(100), 2);
        assert_eq!(d.next_expiry(), None);
        assert!(alive(None, u64::MAX) && alive(Some(10), 9) && !alive(Some(10), 10));
    }

    #[test]
    fn json_keeps_old_records_readable() {
        let g: Group = serde_json::from_str(r#"{"name":"vip"}"#).unwrap();
        assert_eq!(g, Group::new("vip"));
        let json = serde_json::to_string(&Group::new("vip")).unwrap();
        assert_eq!(json, r#"{"name":"vip","weight":0,"data":{}}"#);
        let p: PermNode = serde_json::from_str(r#"{"node":"a","value":true,"context":{"world":"x"}}"#).unwrap();
        assert_eq!(p.context.get("world"), Some("x"));
    }
}
