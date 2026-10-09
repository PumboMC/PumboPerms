//! Contexts: where a node applies.
//!
//! A node without contexts is global. `server=<name>` limits it to one server,
//! `group=<name>` to a group of servers (on PumboProx), `world=<name>` to one
//! world. A node applies when every context it has matches where the player is.
//! When the same node is set more than once on one holder, the most specific
//! context wins: server (4) beats server group (2) beats world (1) beats global
//! (0), and the scores add up (`server` + `world` = 5).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SERVER: &str = "server";
pub const GROUP: &str = "group";
pub const WORLD: &str = "world";

/// Context keys in display order.
pub const KEYS: [&str; 3] = [SERVER, GROUP, WORLD];

/// Longest context value.
pub const MAX_VALUE_LEN: usize = 64;

/// Where the player is when a permission is checked. Empty fields are unknown:
/// nodes limited to that key do not apply. A server may be in several server
/// groups (PumboProx `server-group`); `group=` nodes of each of them apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub server: String,
    pub groups: Vec<String>,
    pub world: String,
}

impl Query {
    pub fn new(server: &str, group: &str, world: &str) -> Self {
        let groups = if group.is_empty() { Vec::new() } else { vec![group.to_lowercase()] };
        Self::at(server, groups, world)
    }

    /// A place with every server group of the server.
    pub fn at(server: &str, groups: Vec<String>, world: &str) -> Self {
        Self {
            server: server.to_lowercase(),
            groups: groups.into_iter().map(|g| g.to_lowercase()).collect(),
            world: world.to_lowercase(),
        }
    }

    fn matches(&self, key: &str, value: &str) -> bool {
        match key {
            SERVER => self.server == value,
            GROUP => self.groups.iter().any(|g| g == value),
            WORLD => self.world == value,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    UnknownKey(String),
    BadValue(String),
    /// The same key twice in one command.
    Repeated(String),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::UnknownKey(k) => write!(f, "unknown context '{k}' (use server=, group= or world=)"),
            ContextError::BadValue(v) => write!(f, "invalid context value '{v}'"),
            ContextError::Repeated(k) => write!(f, "context '{k}' given twice"),
        }
    }
}

impl std::error::Error for ContextError {}

/// The contexts of one node. Empty means global.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Contexts(BTreeMap<String, String>);

impl Contexts {
    pub fn global() -> Self {
        Self::default()
    }

    pub fn is_global(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Adds `key=value` after checking both.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), ContextError> {
        let key = canonical_key(key).ok_or_else(|| ContextError::UnknownKey(key.to_string()))?;
        let value = value.trim().to_lowercase();
        if value.is_empty()
            || value.len() > MAX_VALUE_LEN
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b':'))
        {
            return Err(ContextError::BadValue(value));
        }
        if self.0.contains_key(key) {
            return Err(ContextError::Repeated(key.to_string()));
        }
        self.0.insert(key.to_string(), value);
        Ok(())
    }

    /// Builds contexts from the trailing `key=value` arguments of a command.
    pub fn parse(args: &[String]) -> Result<Self, ContextError> {
        let mut out = Self::default();
        for a in args {
            let (k, v) = a.split_once('=').ok_or_else(|| ContextError::UnknownKey(a.clone()))?;
            out.insert(k, v)?;
        }
        Ok(out)
    }

    /// Whether a node with these contexts applies at `query`.
    pub fn applies(&self, query: &Query) -> bool {
        self.0.iter().all(|(k, v)| query.matches(k, v))
    }

    /// How specific the contexts are: server 4, server group 2, world 1, summed.
    pub fn score(&self) -> u8 {
        self.0
            .keys()
            .map(|k| match k.as_str() {
                SERVER => 4,
                GROUP => 2,
                WORLD => 1,
                _ => 0,
            })
            .sum()
    }

    /// Whether these contexts use `key`.
    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}

impl fmt::Display for Contexts {
    /// `server=lobby world=nether`, or `global`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("global");
        }
        let mut first = true;
        for key in KEYS {
            if let Some(v) = self.0.get(key) {
                if !first {
                    f.write_str(" ")?;
                }
                write!(f, "{key}={v}")?;
                first = false;
            }
        }
        Ok(())
    }
}

/// Whether an argument looks like a context (`key=value`).
pub fn is_context_arg(arg: &str) -> bool {
    arg.split_once('=').is_some_and(|(k, _)| canonical_key(k).is_some())
}

fn canonical_key(key: &str) -> Option<&'static str> {
    match key.trim().to_lowercase().as_str() {
        SERVER => Some(SERVER),
        GROUP | "server-group" | "servergroup" => Some(GROUP),
        WORLD => Some(WORLD),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_and_displays() {
        let c = Contexts::parse(&args("World=Nether server=lobby")).unwrap();
        assert_eq!(c.to_string(), "server=lobby world=nether");
        assert_eq!(c.get("world"), Some("nether"));
        assert_eq!(Contexts::parse(&args("server-group=lobbies")).unwrap().get("group"), Some("lobbies"));
        assert_eq!(Contexts::global().to_string(), "global");
        assert!(Contexts::parse(&[]).unwrap().is_global());
        assert_eq!(Contexts::parse(&args("dimension=end")), Err(ContextError::UnknownKey("dimension".into())));
        assert_eq!(Contexts::parse(&args("world=a world=b")), Err(ContextError::Repeated("world".into())));
        assert_eq!(Contexts::parse(&args("world=")), Err(ContextError::BadValue(String::new())));
        assert_eq!(Contexts::parse(&args("world=a/b")), Err(ContextError::BadValue("a/b".into())));
        assert!(Contexts::parse(&args("nonsense")).is_err());
        assert!(is_context_arg("server=x") && !is_context_arg("x") && !is_context_arg("foo=x"));
    }

    #[test]
    fn applies_and_scores() {
        let q = Query::new("lobby", "lobbies", "World");
        assert!(Contexts::global().applies(&q));
        assert!(Contexts::parse(&args("server=lobby world=world")).unwrap().applies(&q));
        assert!(!Contexts::parse(&args("server=survival")).unwrap().applies(&q));
        assert!(Contexts::parse(&args("group=lobbies")).unwrap().applies(&q));
        let two = Query::at("arena", vec!["Arenas".into(), "events".into()], "");
        assert!(Contexts::parse(&args("group=events")).unwrap().applies(&two));
        assert!(Contexts::parse(&args("group=arenas server=arena")).unwrap().applies(&two));
        assert!(!Contexts::parse(&args("group=lobbies")).unwrap().applies(&two));
        // unknown parts of the query never match a context
        assert!(!Contexts::parse(&args("server=lobby")).unwrap().applies(&Query::default()));
        assert_eq!(Contexts::global().score(), 0);
        assert_eq!(Contexts::parse(&args("world=a")).unwrap().score(), 1);
        assert_eq!(Contexts::parse(&args("group=a")).unwrap().score(), 2);
        assert_eq!(Contexts::parse(&args("server=a world=b")).unwrap().score(), 5);
        assert!(
            Contexts::parse(&args("server=a")).unwrap().score()
                > Contexts::parse(&args("group=a world=b")).unwrap().score()
        );
    }
}
