//! Test helpers shared with the platform crates (feature `testing`).
//!
//! `pumpkin_has` models how Pumpkin 0.2.0 reads the attributes a player gets:
//! `PermissionManager::has_permission`, `pumpkin-util/src/permission.rs:330-392`.
//! PumboProx keeps a copy with the same case table
//! (`pumbo-host/tests/pumpkin_model`); a new Pumpkin release changes both.

use std::collections::HashMap;

/// Pumpkin's answer from the attributes alone; `None`: its registry decides.
pub fn pumpkin_has(attachments: &HashMap<String, bool>, node: &str) -> Option<bool> {
    if let Some(v) = attachments.get(node) {
        return Some(*v);
    }
    let parts: Vec<&str> = node.split(':').collect();
    let [ns, key] = parts.as_slice() else {
        return None;
    };
    if let Some(v) = attachments.get(&format!("{ns}:*")) {
        return Some(*v);
    }
    let keys: Vec<&str> = key.split('.').collect();
    let mut current = format!("{ns}:");
    for (i, part) in keys.iter().enumerate() {
        current.push_str(part);
        if let Some(v) = attachments.get(&current) {
            return Some(*v);
        }
        if i + 1 < keys.len() {
            if let Some(v) = attachments.get(&format!("{current}.*")) {
                return Some(*v);
            }
            current.push('.');
        }
    }
    // Children of registered permissions are left out: PumboPerms registers none.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pumpkin_has_follows_pumpkin_0_2_0() {
        let set = |entries: &[(&str, bool)]| -> HashMap<String, bool> {
            entries.iter().map(|(n, v)| (n.to_string(), *v)).collect()
        };
        // (case, attributes, node, Pumpkin's answer)
        type Case<'a> = (&'a str, &'a [(&'a str, bool)], &'a str, Option<bool>);
        let cases: &[Case] = &[
            ("exact entry", &[("ns:a.b", true)], "ns:a.b", Some(true)),
            ("exact entry before wildcards", &[("ns:*", true), ("ns:a.b", false)], "ns:a.b", Some(false)),
            ("namespace wildcard before a narrower one", &[("ns:*", true), ("ns:a.*", false)], "ns:a.b", Some(true)),
            ("broadest prefix first", &[("ns:a.*", true), ("ns:a.b.*", false)], "ns:a.b.c", Some(true)),
            ("exact parent node answers for its children", &[("ns:a", true)], "ns:a.b", Some(true)),
            ("parent node before the parent wildcard", &[("ns:a", false), ("ns:a.*", true)], "ns:a.b", Some(false)),
            ("wildcard does not match its base", &[("ns:a.*", true)], "ns:a", None),
            ("no attribute", &[("ns:x", true)], "ns:a.b", None),
            ("other namespace", &[("other:*", true)], "ns:a", None),
            ("no namespace: exact only", &[("a", true)], "a.b", None),
            ("two colons: exact only", &[("ns:*", true)], "ns:a:b", None),
        ];
        for (name, entries, node, want) in cases {
            assert_eq!(pumpkin_has(&set(entries), node), *want, "{name}");
        }
    }
}
