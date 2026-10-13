//! Export and import of the whole database as JSON.
//!
//! ```json
//! { "format": "pumboperms", "version": 1, "exported": 1760000000000,
//!   "groups": [ { "name": "vip", "weight": 10, "data": { "permissions": [ ... ] } } ],
//!   "users": [ { "uuid": "...", "name": "Steve", "data": { "parents": [ ... ] } } ],
//!   "tracks": [ { "name": "ranks", "groups": ["default", "vip"] } ] }
//! ```
//!
//! An import checks every name and node first and replaces everything at once.
//! Entries no command could have written are fixed or left out, each with a
//! line in [`Snapshot::fixed`]: user names that are not Minecraft names,
//! contexts in another spelling, duplicates, parents without their group,
//! `group.<name>` set as a permission, texts that are too long or have
//! control characters and meta keys that are empty or have spaces.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::{Data, Group, MetaKind, Track, User, is_valid_meta_key, is_valid_text};
use crate::node;
use crate::perms::Perms;

pub const FORMAT: &str = "pumboperms";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub exported: u64,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub users: Vec<User>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Players a source knows only by nickname (`uuid` empty), such as
    /// PumboProx's `permissions.yml`: imported at their first join.
    #[serde(default, rename = "by-name", skip_serializing_if = "Vec::is_empty")]
    pub by_name: Vec<User>,
    /// Set by PumboPerms on PumboProx: the proxy's PumboPerms rules the
    /// network, a PumboPerms on a server behind it steps back.
    #[serde(default, rename = "proxy-rules", skip_serializing_if = "std::ops::Not::not")]
    pub proxy_rules: bool,
    /// What [`parse`] fixed or left out, one line each, for the import reply.
    #[serde(skip)]
    pub fixed: Vec<String>,
}

/// The whole database (users with data only, sorted).
pub fn snapshot(perms: &Perms, now: u64) -> Snapshot {
    let mut users: Vec<User> = perms.users().filter(|u| !u.data.is_empty()).cloned().collect();
    users.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    Snapshot {
        format: FORMAT.into(),
        version: VERSION,
        exported: now,
        groups: perms.groups().cloned().collect(),
        users,
        tracks: perms.tracks().cloned().collect(),
        by_name: Vec::new(),
        proxy_rules: false,
        fixed: Vec::new(),
    }
}

/// The whole database as pretty JSON.
pub fn dump(perms: &Perms, now: u64) -> String {
    serde_json::to_string_pretty(&snapshot(perms, now)).unwrap_or_default()
}

/// Reads and checks an export. Nodes are normalized; a bad name, UUID or node
/// is returned as an error, the entries listed in the module docs are fixed.
pub fn parse(text: &str) -> Result<Snapshot, String> {
    let snap: Snapshot = serde_json::from_str(text).map_err(|e| format!("not a PumboPerms export: {e}"))?;
    check(snap)
}

/// The checks of [`parse`] on an export already read (for one embedded in a
/// larger message).
pub fn check(mut snap: Snapshot) -> Result<Snapshot, String> {
    if snap.format != FORMAT {
        return Err(format!("unknown format '{}'", snap.format));
    }
    if snap.version > VERSION {
        return Err(format!("export version {} is newer than this version understands ({VERSION})", snap.version));
    }
    let mut groups: HashSet<String> = snap.groups.iter().map(|g| g.name.clone()).collect();
    groups.insert("default".into());
    let mut fixed = Vec::new();
    for g in &mut snap.groups {
        if !node::is_valid_name(&g.name) {
            return Err(format!("invalid group name '{}'", g.name));
        }
        let who = format!("group {}", g.name);
        if !is_valid_text(&g.display_name) {
            fixed.push(format!("{who}: display name left out ({TEXT_RULE})"));
            g.display_name.clear();
        }
        check_data(&mut g.data, &groups, &who, &mut fixed).map_err(|e| format!("{who}: {e}"))?;
    }
    for u in &mut snap.users {
        let uuid = pumbo_common::id::Uuid::parse(&u.uuid).ok_or_else(|| format!("invalid UUID '{}'", u.uuid))?;
        u.uuid = uuid.to_string();
        let who = format!("user {}", u.uuid);
        if !u.name.is_empty() && !pumbo_common::id::is_java_name(&u.name) {
            fixed.push(format!("{who}: name {:?} is not a Minecraft name, shown by UUID", u.name));
            u.name.clear();
        }
        check_data(&mut u.data, &groups, &who, &mut fixed).map_err(|e| format!("{who}: {e}"))?;
    }
    for u in &mut snap.by_name {
        if !pumbo_common::id::is_java_name(&u.name) {
            return Err(format!("invalid player name '{}'", u.name));
        }
        u.uuid.clear();
        let who = format!("player {}", u.name);
        check_data(&mut u.data, &groups, &who, &mut fixed).map_err(|e| format!("{who}: {e}"))?;
    }
    for t in &snap.tracks {
        if !node::is_valid_name(&t.name) {
            return Err(format!("invalid track name '{}'", t.name));
        }
        if let Some(g) = t.groups.iter().find(|g| !snap.groups.iter().any(|x| &x.name == *g)) {
            return Err(format!("track {} uses unknown group '{g}'", t.name));
        }
    }
    snap.fixed = fixed;
    Ok(snap)
}

const TEXT_RULE: &str = "over 256 bytes or with control characters";

/// Checks one holder's data: an error for a bad node or group name, fixes for
/// the rest (see the module docs).
fn check_data(data: &mut Data, groups: &HashSet<String>, who: &str, fixed: &mut Vec<String>) -> Result<(), String> {
    let mut clean = Data::default();
    for mut p in std::mem::take(&mut data.permissions) {
        p.node = node::normalize(&p.node).map_err(|e| format!("'{}': {e}", p.node))?;
        if let Some(group) = p.node.strip_prefix(node::GROUP_PREFIX) {
            fixed.push(format!("{who}: {} left out, membership is given with parent add {group}", p.node));
            continue;
        }
        match p.context.canonical() {
            Ok(c) => p.context = c,
            Err(e) => {
                fixed.push(format!("{who}: {} left out, {e}", p.node));
                continue;
            }
        }
        let node = p.node.clone();
        if clean.set_permission(p).is_some() {
            fixed.push(format!("{who}: {node} given twice, the last one kept"));
        }
    }
    for mut p in std::mem::take(&mut data.parents) {
        if !node::is_valid_name(&p.group) {
            return Err(format!("invalid parent group '{}'", p.group));
        }
        if !groups.contains(&p.group) {
            fixed.push(format!("{who}: parent {} left out, the import has no such group", p.group));
            continue;
        }
        match p.context.canonical() {
            Ok(c) => p.context = c,
            Err(e) => {
                fixed.push(format!("{who}: parent {} left out, {e}", p.group));
                continue;
            }
        }
        let group = p.group.clone();
        if !clean.add_parent(p) {
            fixed.push(format!("{who}: parent {group} given twice"));
        }
    }
    for mut m in std::mem::take(&mut data.meta) {
        let what = match m.kind {
            MetaKind::Prefix => "a prefix".to_string(),
            MetaKind::Suffix => "a suffix".to_string(),
            MetaKind::Meta => format!("meta {:?}", m.key),
        };
        if !is_valid_text(&m.value) {
            fixed.push(format!("{who}: {what} left out ({TEXT_RULE})"));
            continue;
        }
        if m.kind == MetaKind::Meta && !is_valid_meta_key(&m.key) {
            fixed.push(format!("{who}: {what} left out (key empty, with spaces, {TEXT_RULE})"));
            continue;
        }
        match m.context.canonical() {
            Ok(c) => m.context = c,
            Err(e) => {
                fixed.push(format!("{who}: {what} left out, {e}"));
                continue;
            }
        }
        clean.meta.push(m);
    }
    *data = clean;
    Ok(())
}

/// A file name for an export: `[a-z0-9_-]`, at most 64 characters.
pub fn is_valid_file_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// `pumboperms-2026-10-08-153000` (UTC) for an export without a name.
#[expect(clippy::integer_division, reason = "whole units of time")]
pub fn default_file_name(now_ms: u64) -> String {
    let secs = now_ms / 1000;
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!("pumboperms-{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Days since 1970-01-01 to a date (proleptic Gregorian).
#[expect(clippy::integer_division, reason = "the calendar algorithm divides in whole units")]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, u32::try_from(m).unwrap_or(1), u32::try_from(d).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Contexts, Query};
    use crate::model::{ParentNode, PermNode};

    const U: &str = "00000000-0000-0000-0000-000000000001";

    #[test]
    fn round_trip() {
        let mut p = Perms::new();
        p.create_group("vip").unwrap();
        p.edit_group("vip", |g| {
            g.weight = 10;
            g.data.set_permission(PermNode {
                node: "fly".into(),
                value: true,
                context: Contexts::global(),
                expiry: None,
            });
        })
        .unwrap();
        p.edit_user(U, "Steve", |d| {
            d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None })
        });
        p.create_track("ranks").unwrap();
        p.track_insert("ranks", "vip", None).unwrap();
        let json = dump(&p, 5);
        let snap = parse(&json).unwrap();
        let mut q = Perms::new();
        q.replace_all(snap.groups, snap.users, snap.tracks);
        assert_eq!(q.check(U, "fly", &Query::default(), 0), Some(true));
        assert_eq!(q.track("ranks").unwrap().groups, vec!["vip"]);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("nope").is_err());
        assert!(parse(r#"{"format":"luckperms","version":1}"#).is_err());
        assert!(parse(r#"{"format":"pumboperms","version":99}"#).is_err());
        assert!(parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"Bad Name"}]}"#).is_err());
        assert!(parse(r#"{"format":"pumboperms","version":1,"users":[{"uuid":"x"}]}"#).is_err());
        assert!(
            parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"a","data":{"permissions":[{"node":"a b","value":true}]}}]}"#)
                .is_err()
        );
        assert!(parse(r#"{"format":"pumboperms","version":1,"tracks":[{"name":"t","groups":["x"]}]}"#).is_err());
        let ok = parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"a","data":{"permissions":[{"node":"PumboBans:Ban","value":true}]}}]}"#)
            .unwrap();
        assert_eq!(ok.groups[0].data.permissions[0].node, "pumbo.bans.ban");
    }

    #[test]
    fn file_names() {
        assert!(is_valid_file_name("backup-1_a"));
        assert!(!is_valid_file_name("../etc") && !is_valid_file_name("A") && !is_valid_file_name(""));
        assert_eq!(default_file_name(0), "pumboperms-1970-01-01-000000");
        assert_eq!(default_file_name(1_759_937_400_000), "pumboperms-2025-10-08-153000");
        assert_eq!(default_file_name(951_782_400_000), "pumboperms-2000-02-29-000000");
    }
}
