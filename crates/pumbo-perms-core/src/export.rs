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

use serde::{Deserialize, Serialize};

use crate::model::{Data, Group, Track, User};
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
    }
}

/// The whole database as pretty JSON.
pub fn dump(perms: &Perms, now: u64) -> String {
    serde_json::to_string_pretty(&snapshot(perms, now)).unwrap_or_default()
}

/// Reads and checks an export. Nodes are normalized; the first problem is
/// returned as an error.
pub fn parse(text: &str) -> Result<Snapshot, String> {
    let mut snap: Snapshot = serde_json::from_str(text).map_err(|e| format!("not a PumboPerms export: {e}"))?;
    if snap.format != FORMAT {
        return Err(format!("unknown format '{}'", snap.format));
    }
    if snap.version > VERSION {
        return Err(format!("export version {} is newer than this version understands ({VERSION})", snap.version));
    }
    for g in &mut snap.groups {
        if !node::is_valid_name(&g.name) {
            return Err(format!("invalid group name '{}'", g.name));
        }
        check_data(&mut g.data).map_err(|e| format!("group {}: {e}", g.name))?;
    }
    for u in &mut snap.users {
        let uuid = pumbo_common::id::Uuid::parse(&u.uuid).ok_or_else(|| format!("invalid UUID '{}'", u.uuid))?;
        u.uuid = uuid.to_string();
        check_data(&mut u.data).map_err(|e| format!("user {}: {e}", u.uuid))?;
    }
    for u in &mut snap.by_name {
        if !pumbo_common::id::is_java_name(&u.name) {
            return Err(format!("invalid player name '{}'", u.name));
        }
        u.uuid.clear();
        check_data(&mut u.data).map_err(|e| format!("player {}: {e}", u.name))?;
    }
    for t in &snap.tracks {
        if !node::is_valid_name(&t.name) {
            return Err(format!("invalid track name '{}'", t.name));
        }
        if let Some(g) = t.groups.iter().find(|g| !snap.groups.iter().any(|x| &x.name == *g)) {
            return Err(format!("track {} uses unknown group '{g}'", t.name));
        }
    }
    Ok(snap)
}

fn check_data(data: &mut Data) -> Result<(), String> {
    for p in &mut data.permissions {
        p.node = node::normalize(&p.node).map_err(|e| format!("'{}': {e}", p.node))?;
    }
    for p in &data.parents {
        if !node::is_valid_name(&p.group) {
            return Err(format!("invalid parent group '{}'", p.group));
        }
    }
    Ok(())
}

/// A file name for an export: `[a-z0-9_-]`, at most 64 characters.
pub fn is_valid_file_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// `pumboperms-2026-10-08-153000` (UTC) for an export without a name.
pub fn default_file_name(now_ms: u64) -> String {
    let secs = now_ms / 1000;
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!("pumboperms-{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Days since 1970-01-01 to a date (proleptic Gregorian).
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
