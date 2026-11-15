//! The database in memory: undoing a change, and players kept on disk.

use super::*;
use crate::context::Contexts;
use crate::model::{ParentNode, PermNode};

mod offline;
mod undo;

const U: &str = "00000000-0000-0000-0000-000000000001";
const U2: &str = "00000000-0000-0000-0000-000000000002";
const U3: &str = "00000000-0000-0000-0000-000000000003";

fn parent(group: &str, expiry: Option<u64>) -> ParentNode {
    ParentNode { group: group.into(), context: Contexts::global(), expiry }
}

fn perm(node: &str, expiry: Option<u64>) -> PermNode {
    PermNode { node: node.into(), value: true, context: Contexts::global(), expiry }
}

/// Every group, user (also one without data) and track.
fn everything(p: &Perms) -> (Vec<Group>, Vec<User>, Vec<Track>) {
    let mut users: Vec<User> = p.users.values().cloned().collect();
    users.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    (p.groups.values().cloned().collect(), users, p.tracks.values().cloned().collect())
}
