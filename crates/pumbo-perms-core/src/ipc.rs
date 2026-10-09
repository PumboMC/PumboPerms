//! Questions other plugins ask PumboPerms, as JSON messages.
//!
//! On Pumpkin they come through the plugin message channel (`ipc`), on
//! PumboProx through the service registry. Requests:
//!
//! - `{"op":"hello"}` -> `{"ok":true,"plugin":"PumboPerms","version":"0.1.0","protocol":1}`
//! - `{"op":"check","uuid":"…","node":"pumbo.bans.ban","world":"world","op-level":0}`
//!   -> `{"ok":true,"value":true}` (`null`: nobody decides, use your default)
//! - `{"op":"info","uuid":"…","world":"world"}` -> rank, prefix, suffix, groups
//!   and meta (what `%rank%`, `%prefix%` and `%suffix%` show)
//! - `{"op":"fill","uuid":"…","world":"world","text":"%pumboperms_prefix%Steve"}`
//!   -> `{"ok":true,"text":"[VIP] Steve"}`: the placeholders of [`placeholder`]
//!
//! Errors: `{"ok":false,"error":"…"}`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::context::Query;
use crate::engine::Engine;
use crate::node;

/// Version of the message format.
pub const PROTOCOL: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    #[serde(alias = "ping")]
    Hello,
    Check {
        uuid: String,
        node: String,
        #[serde(default)]
        world: String,
        #[serde(default, rename = "op-level")]
        op_level: u8,
    },
    Info {
        uuid: String,
        #[serde(default)]
        world: String,
    },
    Fill {
        uuid: String,
        #[serde(default)]
        world: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    pub name: String,
    /// The primary group.
    pub rank: String,
    /// Its display name (`%rank%`).
    pub rank_display: String,
    pub prefix: String,
    pub suffix: String,
    /// Every inherited group, most important first.
    pub groups: Vec<String>,
    pub meta: BTreeMap<String, String>,
}

/// What PumboPerms shows for player `uuid` at `q`; `None` for a bad UUID.
pub fn info(e: &mut Engine, uuid: &str, q: &Query, now: u64) -> Option<Info> {
    let uuid = uuid_of(uuid)?;
    let eff = e.perms.effective(&uuid, q, now).clone();
    Some(Info {
        name: e.perms.find_user(&uuid).map(|k| k.name).unwrap_or_default(),
        rank_display: e.perms.display_name(&eff.primary),
        rank: eff.primary,
        prefix: eff.prefix.unwrap_or_default(),
        suffix: eff.suffix.unwrap_or_default(),
        groups: eff.groups,
        meta: eff.meta,
    })
}

/// Placeholder `key` of the `pumboperms` namespace: `prefix`, `suffix`, `rank`
/// (display name of the primary group), `group` (its name), `groups` and
/// `meta` with the meta key as `arg`. `None` for an unknown key.
pub fn placeholder(info: &Info, key: &str, arg: Option<&str>) -> Option<String> {
    Some(match key {
        "prefix" => info.prefix.clone(),
        "suffix" => info.suffix.clone(),
        "rank" => info.rank_display.clone(),
        "group" => info.rank.clone(),
        "groups" => info.groups.join(", "),
        "meta" => info.meta.get(arg?)?.clone(),
        _ => return None,
    })
}

/// Replaces `%pumboperms_<key>%` and `%pumboperms_meta:<key>%` in `text`;
/// unknown ones stay as they are.
pub fn fill(info: &Info, text: &str) -> String {
    const START: &str = "%pumboperms_";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(START) {
        out.push_str(&rest[..at]);
        let token = &rest[at + START.len()..];
        let Some(end) = token.find('%') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let (key, arg) = token[..end].split_once(':').map_or((&token[..end], None), |(k, a)| (k, Some(a)));
        match placeholder(info, key, arg) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[at..at + START.len() + end + 1]),
        }
        rest = &token[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Answers one request; `version` is the plugin version.
pub fn handle(e: &mut Engine, request: &[u8], now: u64, version: &str) -> Vec<u8> {
    let reply = match serde_json::from_slice::<Request>(request) {
        Err(err) => error(&format!("bad request: {err}")),
        Ok(Request::Hello) => {
            serde_json::json!({ "ok": true, "plugin": "PumboPerms", "version": version, "protocol": PROTOCOL })
        }
        Ok(Request::Check { uuid, node, world, op_level }) => match (uuid_of(&uuid), node::normalize(&node)) {
            (Some(uuid), Ok(node)) => {
                serde_json::json!({ "ok": true, "value": e.decide(&uuid, &node, &world, op_level, now) })
            }
            (None, _) => error("bad uuid"),
            (_, Err(err)) => error(&err.to_string()),
        },
        Ok(Request::Fill { uuid, world, text }) => {
            let q = e.query(&world);
            match info(e, &uuid, &q, now) {
                Some(info) => serde_json::json!({ "ok": true, "text": fill(&info, &text) }),
                None => error("bad uuid"),
            }
        }
        Ok(Request::Info { uuid, world }) => {
            let q = e.query(&world);
            match info(e, &uuid, &q, now) {
                Some(info) => match serde_json::to_value(&info) {
                    Ok(mut v) => {
                        if let Some(o) = v.as_object_mut() {
                            o.insert("ok".into(), serde_json::Value::Bool(true));
                        }
                        v
                    }
                    Err(err) => error(&err.to_string()),
                },
                None => error("bad uuid"),
            }
        }
    };
    serde_json::to_vec(&reply).unwrap_or_default()
}

fn uuid_of(s: &str) -> Option<String> {
    pumbo_common::id::Uuid::parse(s).map(|u| u.to_string())
}

fn error(msg: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": msg })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CoreCfg;
    use pumbo_common::lang::{COMMON, Lang};

    const U: &str = "00000000-0000-0000-0000-000000000001";

    fn ask(e: &mut Engine, json: &str) -> serde_json::Value {
        serde_json::from_slice(&handle(e, json.as_bytes(), 0, "9.9.9")).unwrap()
    }

    #[test]
    fn answers() {
        let mut e = Engine::in_memory(CoreCfg::default(), Lang::load(&[COMMON], "en", None).0);
        e.perms.create_group("vip").unwrap();
        e.perms
            .edit_group("vip", |g| {
                g.display_name = "VIP".into();
                g.data.add_affix(
                    crate::model::MetaKind::Prefix,
                    1,
                    "[V] ",
                    &crate::context::Contexts::global(),
                    None,
                    false,
                );
            })
            .unwrap();
        e.perms.edit_user(U, "Steve", |d| {
            d.add_parent(crate::model::ParentNode {
                group: "vip".into(),
                context: crate::context::Contexts::global(),
                expiry: None,
            });
        });
        assert_eq!(ask(&mut e, r#"{"op":"hello"}"#)["version"], "9.9.9");
        assert_eq!(ask(&mut e, r#"{"op":"ping"}"#)["protocol"], 1);
        let r = ask(&mut e, &format!(r#"{{"op":"check","uuid":"{U}","node":"group.vip"}}"#));
        assert_eq!(r["value"], true);
        let r = ask(&mut e, &format!(r#"{{"op":"check","uuid":"{U}","node":"pumbobans:ban","op-level":3}}"#));
        assert_eq!(r["value"], true, "operator fallback for Pumbo nodes");
        let r = ask(&mut e, &format!(r#"{{"op":"check","uuid":"{U}","node":"other.node"}}"#));
        assert!(r["value"].is_null());
        let r = ask(&mut e, &format!(r#"{{"op":"info","uuid":"{U}"}}"#));
        assert_eq!(
            (r["rank"].as_str(), r["rank_display"].as_str(), r["prefix"].as_str()),
            (Some("vip"), Some("VIP"), Some("[V] "))
        );
        assert_eq!(r["ok"], true);
        let t = "%pumboperms_prefix%Steve (%pumboperms_rank%, %pumboperms_group%) %pumboperms_nope% %other% 100%";
        let r = ask(&mut e, &format!(r#"{{"op":"fill","uuid":"{U}","text":"{t}"}}"#));
        assert_eq!(r["text"], "[V] Steve (VIP, vip) %pumboperms_nope% %other% 100%");
        let r =
            ask(&mut e, &format!(r#"{{"op":"fill","uuid":"{U}","text":"%pumboperms_meta:none% %pumboperms_prefix"}}"#));
        assert_eq!(r["text"], "%pumboperms_meta:none% %pumboperms_prefix");
        assert_eq!(ask(&mut e, "nonsense")["ok"], false);
        assert_eq!(ask(&mut e, r#"{"op":"check","uuid":"x","node":"a"}"#)["ok"], false);
        assert_eq!(ask(&mut e, &format!(r#"{{"op":"check","uuid":"{U}","node":"a b"}}"#))["ok"], false);
    }
}
