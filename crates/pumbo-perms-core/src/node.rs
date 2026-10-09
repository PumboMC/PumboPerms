//! Permission node names.
//!
//! A node is a lowercase name made of segments separated by `.`, optionally in
//! a namespace (`minecraft:command.gamemode`). A segment of `*` as the last one
//! makes it a wildcard: `pumbo.bans.*` covers every node below `pumbo.bans`,
//! `minecraft:*` every node in the `minecraft` namespace, `*` everything.
//!
//! Pumbo plugins name their nodes `pumbo.<plugin>.<action>`. On Pumpkin a plugin
//! can only register nodes in its own namespace, so the same node is written
//! `pumbo<plugin>:<action>` there (`pumbobans:ban`). Both spellings are one node:
//! [`normalize`] turns the namespaced one into the dotted one, [`host_form`]
//! goes back.

use std::fmt;

/// Longest accepted node.
pub const MAX_LEN: usize = 200;

/// Prefix of the nodes that stand for inherited groups (`group.admin`).
pub const GROUP_PREFIX: &str = "group.";

const PUMBO: &str = "pumbo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeError {
    Empty,
    TooLong,
    /// A character outside `a-z 0-9 _ - . : *`.
    BadChar(char),
    /// An empty segment (`a..b`, `.a`, `a.`) or namespace (`:a`).
    EmptySegment,
    /// More than one `:`.
    TwoNamespaces,
    /// `*` somewhere else than as the whole last segment.
    BadWildcard,
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeError::Empty => f.write_str("empty permission"),
            NodeError::TooLong => write!(f, "permission longer than {MAX_LEN} characters"),
            NodeError::BadChar(c) => write!(f, "character '{c}' is not allowed in a permission"),
            NodeError::EmptySegment => f.write_str("permission has an empty part"),
            NodeError::TwoNamespaces => f.write_str("permission has more than one ':'"),
            NodeError::BadWildcard => f.write_str("'*' may only be the whole last part (a.b.*)"),
        }
    }
}

impl std::error::Error for NodeError {}

/// Checks a node typed by an admin or asked by a plugin and returns its
/// canonical form (lowercase, Pumbo namespaces as dotted names).
pub fn normalize(input: &str) -> Result<String, NodeError> {
    let node = input.trim().to_lowercase();
    if node.is_empty() {
        return Err(NodeError::Empty);
    }
    if node.len() > MAX_LEN {
        return Err(NodeError::TooLong);
    }
    if let Some(c) = node
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ':' | '*')))
    {
        return Err(NodeError::BadChar(c));
    }
    if node == "*" {
        return Ok(node);
    }
    let (namespace, path) = match node.split_once(':') {
        Some((ns, path)) => {
            if path.contains(':') {
                return Err(NodeError::TwoNamespaces);
            }
            if ns.is_empty() || path.is_empty() {
                return Err(NodeError::EmptySegment);
            }
            if ns.contains('*') {
                return Err(NodeError::BadWildcard);
            }
            (Some(ns), path)
        }
        None => (None, node.as_str()),
    };
    let segments: Vec<&str> = path.split('.').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(NodeError::EmptySegment);
    }
    let last = segments.len() - 1;
    for (i, s) in segments.iter().enumerate() {
        if s.contains('*') && (*s != "*" || i != last) {
            return Err(NodeError::BadWildcard);
        }
    }
    if let Some(ns) = namespace
        && let Some(id) = ns.strip_prefix(PUMBO)
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Ok(format!("{PUMBO}.{id}.{path}"));
    }
    Ok(node)
}

/// The spelling of a node for a host that only knows namespaced nodes
/// (`pumbo.bans.ban` -> `pumbobans:ban`). Other nodes stay as they are.
pub fn host_form(node: &str) -> String {
    if let Some(rest) = node.strip_prefix("pumbo.")
        && let Some((id, action)) = rest.split_once('.')
        && !id.is_empty()
        && id != "*"
    {
        return format!("{PUMBO}{id}:{action}");
    }
    node.to_string()
}

/// Whether a node ends in a wildcard (`a.*`, `ns:*`, `*`).
pub fn is_wildcard(node: &str) -> bool {
    node.ends_with('*')
}

/// The wildcards that cover `node`, most specific first:
/// `ns:a.b.c` -> `ns:a.b.*`, `ns:a.*`, `ns:*`, `*`. A wildcard is not its own
/// parent: `a.b.*` -> `a.*`, `*`.
pub fn parents(node: &str) -> Vec<String> {
    if node == "*" {
        return Vec::new();
    }
    // A wildcard asked as a node starts from what it covers: `a.b.*` from `a.b`.
    let base = node.strip_suffix(".*").or_else(|| node.strip_suffix(":*")).unwrap_or(node);
    let mut out = Vec::new();
    for (i, c) in base.char_indices().rev() {
        if (c == '.' || c == ':')
            && let Some(head) = base.get(..=i)
        {
            out.push(format!("{head}*"));
        }
    }
    out.push("*".to_string());
    out
}

/// Whether `pattern` (a node or a wildcard) covers `node`.
pub fn covers(pattern: &str, node: &str) -> bool {
    if pattern == node || pattern == "*" {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(head) if head.ends_with('.') || head.ends_with(':') => node.starts_with(head),
        _ => false,
    }
}

/// Node of an inherited group: `group.<name>`.
pub fn group_node(group: &str) -> String {
    format!("{GROUP_PREFIX}{group}")
}

/// Whether a group or track name is acceptable: 1 to 36 characters from
/// `a-z 0-9 _ -`.
pub fn is_valid_name(name: &str) -> bool {
    (1..=36).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_checks() {
        assert_eq!(normalize(" Pumbo.Bans.Ban "), Ok("pumbo.bans.ban".into()));
        assert_eq!(normalize("minecraft:command.gamemode"), Ok("minecraft:command.gamemode".into()));
        assert_eq!(normalize("*"), Ok("*".into()));
        assert_eq!(normalize("a.b.*"), Ok("a.b.*".into()));
        assert_eq!(normalize("minecraft:*"), Ok("minecraft:*".into()));
        assert_eq!(normalize(""), Err(NodeError::Empty));
        assert_eq!(normalize("a b"), Err(NodeError::BadChar(' ')));
        assert_eq!(normalize("a..b"), Err(NodeError::EmptySegment));
        assert_eq!(normalize("a."), Err(NodeError::EmptySegment));
        assert_eq!(normalize(":a"), Err(NodeError::EmptySegment));
        assert_eq!(normalize("a:b:c"), Err(NodeError::TwoNamespaces));
        assert_eq!(normalize("a*"), Err(NodeError::BadWildcard));
        assert_eq!(normalize("*.a"), Err(NodeError::BadWildcard));
        assert_eq!(normalize("a.*.b"), Err(NodeError::BadWildcard));
        assert_eq!(normalize("*:a"), Err(NodeError::BadWildcard));
        assert_eq!(normalize(&"a".repeat(MAX_LEN + 1)), Err(NodeError::TooLong));
    }

    #[test]
    fn pumbo_namespaces_are_dotted_nodes() {
        assert_eq!(normalize("pumbobans:ban"), Ok("pumbo.bans.ban".into()));
        assert_eq!(normalize("PumboPerms:user.info"), Ok("pumbo.perms.user.info".into()));
        assert_eq!(normalize("pumbobans:*"), Ok("pumbo.bans.*".into()));
        // a bare `pumbo` namespace has no plugin id
        assert_eq!(normalize("pumbo:x"), Ok("pumbo:x".into()));
        assert_eq!(host_form("pumbo.bans.ban"), "pumbobans:ban");
        assert_eq!(host_form("pumbo.perms.user.info"), "pumboperms:user.info");
        assert_eq!(host_form("pumbo.bans.*"), "pumbobans:*");
        assert_eq!(host_form("pumbo.*"), "pumbo.*");
        assert_eq!(host_form("pumbo.bans"), "pumbo.bans");
        assert_eq!(host_form("minecraft:command.op"), "minecraft:command.op");
        assert_eq!(host_form("essentials.fly"), "essentials.fly");
        for n in ["pumbo.bans.ban", "pumbo.perms.user.info", "pumbo.bans.*"] {
            assert_eq!(normalize(&host_form(n)).as_deref(), Ok(n));
        }
    }

    #[test]
    fn wildcard_parents_most_specific_first() {
        assert_eq!(parents("ns:a.b.c"), vec!["ns:a.b.*", "ns:a.*", "ns:*", "*"]);
        assert_eq!(parents("a.b.c"), vec!["a.b.*", "a.*", "*"]);
        assert_eq!(parents("a"), vec!["*"]);
        assert_eq!(parents("a.b.*"), vec!["a.*", "*"]);
        assert_eq!(parents("ns:*"), vec!["*"]);
        assert_eq!(parents("ns:a.*"), vec!["ns:*", "*"]);
        assert!(parents("*").is_empty());
    }

    #[test]
    fn coverage() {
        assert!(covers("*", "anything.at.all"));
        assert!(covers("pumbo.bans.*", "pumbo.bans.ban"));
        assert!(covers("pumbo.bans.*", "pumbo.bans.exempt.ban"));
        assert!(!covers("pumbo.bans.*", "pumbo.bansx.ban"));
        assert!(!covers("pumbo.bans.*", "pumbo.bans"));
        assert!(covers("minecraft:*", "minecraft:command.op"));
        assert!(covers("a.b", "a.b"));
        assert!(!covers("a.b", "a.b.c"));
        assert!(is_wildcard("a.*") && !is_wildcard("a.b"));
        assert_eq!(group_node("vip"), "group.vip");
        assert!(is_valid_name("vip-2_x"));
        assert!(!is_valid_name("VIP") && !is_valid_name("") && !is_valid_name(&"a".repeat(37)));
    }
}
