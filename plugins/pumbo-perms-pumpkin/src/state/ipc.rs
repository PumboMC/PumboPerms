//! Senders of `/pp` and the answers to other plugins' messages.

use pumbo_perms_core::command::Who;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::ipc;

use super::{Rt, with};
use crate::pie;

/// Who sent a command, as Pumpkin tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderKind {
    Console,
    Rcon,
    Player,
    CommandBlock,
    /// Signs, functions and other senders without an entity.
    Dummy,
}

/// The sender of `/pp`: `player` is (name, UUID, operator level, world).
/// Command blocks, signs and other senders get `None` and no rights at all:
/// anyone who can place one would otherwise act as the console.
pub fn who_for(kind: SenderKind, player: Option<(&str, &str, u8, &str)>) -> Option<Who> {
    match (kind, player) {
        (SenderKind::Console, _) => Some(Who::console()),
        (SenderKind::Rcon, _) => Some(Who { name: "RCON".into(), ..Who::console() }),
        (SenderKind::Player, Some((name, uuid, op_level, world))) => Some(Who::player(name, uuid, op_level, world)),
        _ => None,
    }
}

/// Answers another plugin's message (`pumbo_perms_core::ipc`) from this
/// database; `engine` is `None` when the state is busy.
pub fn ipc_reply(message: &[u8], engine: Option<&mut Engine>, now: u64) -> Result<Vec<u8>, String> {
    let engine = engine.ok_or_else(|| "PumboPerms is busy".to_string())?;
    Ok(pumbo_perms_core::ipc::handle(engine, message, now, env!("CARGO_PKG_VERSION")))
}

/// [`ipc_reply`] while the proxy's PumboPerms rules: the same answers from
/// what the proxy decided. `check` reads the set the proxy sent ([`proxy_decision`]:
/// `None` when no set arrived for the player, `Some(None)` for a node outside
/// it), `info` and `fill` use what PumboBridge carries from the proxy
/// (`info`: the proxy's `ipc::Info` as JSON). Without them: `proxy-rules`.
pub fn passive_reply(
    message: &[u8],
    has: impl FnOnce(&str, &str) -> Option<Option<bool>>,
    info: impl FnOnce(&str) -> Option<String>,
) -> Vec<u8> {
    use pumbo_perms_core::ipc::{self, Request};
    let refused = serde_json::json!({ "ok": false, "error": "proxy-rules", "passive": true });
    let from_proxy = |uuid: &str| -> Option<ipc::Info> {
        let uuid = pumbo_common::id::Uuid::parse(uuid)?.to_string();
        serde_json::from_str(&info(&uuid)?).ok()
    };
    let reply = match ipc::parse(message) {
        Err(reply) => reply,
        Ok(Request::Hello) => {
            let mut hello = ipc::hello(env!("CARGO_PKG_VERSION"));
            if let Some(o) = hello.as_object_mut() {
                o.insert("passive".into(), true.into());
            }
            hello
        }
        Ok(Request::Check { uuid, node, .. }) => {
            let host = pumbo_perms_core::node::normalize(&node).map(|n| pumbo_perms_core::node::host_form(&n));
            match (pumbo_common::id::Uuid::parse(&uuid), host) {
                (Some(u), Ok(host)) => match has(&u.to_string(), &host) {
                    Some(value) => serde_json::json!({ "ok": true, "value": value }),
                    None => refused,
                },
                (None, _) => serde_json::json!({ "ok": false, "error": "bad uuid" }),
                (_, Err(e)) => serde_json::json!({ "ok": false, "error": e.to_string() }),
            }
        }
        Ok(Request::Info { uuid, .. }) => match from_proxy(&uuid).and_then(|i| serde_json::to_value(i).ok()) {
            Some(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("ok".into(), true.into());
                }
                v
            }
            None => refused,
        },
        Ok(Request::Fill { uuid, text, .. }) => match from_proxy(&uuid) {
            Some(i) => serde_json::json!({ "ok": true, "text": ipc::fill(&i, &text) }),
            None => refused,
        },
    };
    reply.to_string().into_bytes()
}

/// The proxy's decision for `node` (Pumpkin's spelling) from PumboBridge's
/// `player-info` answer. Not `has_permission`: a registry default is no
/// decision, and it would fire the permission check event from an ipc call.
pub fn proxy_decision(player_info: &[u8], node: &str) -> Option<Option<bool>> {
    let v: serde_json::Value = serde_json::from_slice(player_info).ok()?;
    Some(v.get("nodes")?.as_object()?.get(node).and_then(serde_json::Value::as_bool))
}

impl Rt {
    /// The UUID and world of an online player by nickname: as written, then
    /// in any letter case.
    pub fn online_named(&self, name: &str) -> Option<(String, String)> {
        // A linear scan of the online players; a name map if PumpkinPIE asks thousands of times a second.
        let exact = self.online.iter().find(|(_, o)| o.name == name);
        let found = exact.or_else(|| self.online.iter().find(|(_, o)| o.name.eq_ignore_ascii_case(name)));
        found.map(|(uuid, o)| (uuid.clone(), o.world.clone()))
    }

    /// A placeholder PumpkinPIE asks for, from this database: the value for
    /// the online player in their world, as `fill` gives it; `None` declines.
    pub fn pie_value(&mut self, req: &pie::Request, now: u64) -> Option<String> {
        let (name, argument) = req.wanted()?;
        let (uuid, world) = self.online_named(req.viewer.as_deref()?)?;
        let q = self.engine.query(&world);
        let info = ipc::info(&mut self.engine, &uuid, &q, now)?;
        ipc::placeholder(&info, name, argument)
    }

    /// The reply to PumpkinPIE's `on_request` from the state. A busy state
    /// (a nested call) or none declines: the placeholder stays in the text.
    pub fn pie_reply(req: &pie::Request, now: u64) -> Vec<u8> {
        pie::answer(with(|rt| rt.pie_value(req, now)).flatten())
    }

    /// [`Rt::pie_reply`] while the proxy rules: the player from the state,
    /// which is released before `info` asks for what PumboBridge carries from
    /// the proxy (the proxy's `ipc::Info` as JSON).
    pub fn pie_passive_reply(req: &pie::Request, info: impl FnOnce(&str) -> Option<String>) -> Vec<u8> {
        let value = req.wanted().and_then(|(name, argument)| {
            let viewer = req.viewer.as_deref()?;
            let (uuid, _) = with(|rt| rt.online_named(viewer)).flatten()?;
            let info: ipc::Info = serde_json::from_str(&info(&uuid)?).ok()?;
            ipc::placeholder(&info, name, argument)
        });
        pie::answer(value)
    }
}
