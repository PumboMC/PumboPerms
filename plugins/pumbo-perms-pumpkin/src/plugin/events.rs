//! Players joining, leaving and changing worlds, the periodic refresh, and
//! (in check-event mode) Pumpkin's permission checks.

use std::sync::atomic::{AtomicU64, Ordering};

use pumbo_common::clock::now_ms;
use pumbo_perms_core::command::{self, Who};

use super::api::{
    self, EventData, EventHandler, NamedColor, Player, PlayerChangeWorldEvent, PlayerCommandSendEvent, PlayerJoinEvent,
    PlayerLeaveEvent, PlayerPermissionCheckEvent, Server, TextComponent,
};
use super::push;
use super::state::{self, Online};

/// Permission checks that found the plugin busy (answered by Pumpkin alone).
static SKIPPED: AtomicU64 = AtomicU64::new(0);

pub fn skipped() -> u64 {
    SKIPPED.load(Ordering::Relaxed)
}

/// Starts tracking a player and sets its permissions.
pub fn track(player: &Player) {
    let uuid = api::uuid_string(&player.get_id());
    let online = Online {
        name: player.get_name(),
        world: player.get_world().get_name(),
        op_level: api::level_of(player.get_permission_level()),
    };
    let now = now_ms();
    let result = state::with(|rt| {
        if let Err(e) = rt.engine.seen(&uuid, &online.name) {
            rt.warn(format!("PumboPerms: cannot save the name of {}: {e}", online.name));
        }
        rt.online.insert(uuid.clone(), online);
        (rt.plan_player(&uuid, now), rt.resend())
    });
    state::flush_logs();
    if let Some((ops, resend)) = result {
        push::apply(player, &ops, resend);
    }
}

pub struct Join;

impl EventHandler<PlayerJoinEvent> for Join {
    fn handle(&self, _server: Server, e: EventData<PlayerJoinEvent>) -> EventData<PlayerJoinEvent> {
        track(&e.player);
        e
    }
}

pub struct Leave;

impl EventHandler<PlayerLeaveEvent> for Leave {
    fn handle(&self, _server: Server, e: EventData<PlayerLeaveEvent>) -> EventData<PlayerLeaveEvent> {
        let uuid = api::uuid_string(&e.player.get_id());
        state::with(|rt| rt.online.remove(&uuid));
        e
    }
}

pub struct ChangeWorld;

impl EventHandler<PlayerChangeWorldEvent> for ChangeWorld {
    fn handle(&self, _server: Server, e: EventData<PlayerChangeWorldEvent>) -> EventData<PlayerChangeWorldEvent> {
        let uuid = api::uuid_string(&e.player.get_id());
        let world = e.new_world.get_name();
        let now = now_ms();
        let result = state::with(|rt| {
            let o = rt.online.get_mut(&uuid)?;
            o.world = world;
            rt.engine.perms.uses_context("world").then(|| (rt.plan_player(&uuid, now), rt.resend()))
        });
        if let Some(Some((ops, resend))) = result {
            push::apply(&e.player, &ops, resend);
        }
        e
    }
}

/// Every `refresh-seconds`: expired nodes and changed operator levels.
pub fn tick(server: &Server) {
    let levels: Vec<(String, u8)> = server
        .get_all_players()
        .iter()
        .map(|p| (api::uuid_string(&p.get_id()), api::level_of(p.get_permission_level())))
        .collect();
    let now = now_ms();
    let result = state::with(|rt| {
        let mut changed = Vec::new();
        for (uuid, level) in &levels {
            if let Some(o) = rt.online.get_mut(uuid)
                && o.op_level != *level
            {
                o.op_level = *level;
                changed.push(uuid.clone());
            }
        }
        let expired = match rt.engine.expire(now) {
            Ok(e) => e,
            Err(e) => {
                rt.warn(format!("PumboPerms: cannot save expired permissions: {e}"));
                false
            }
        };
        let plans = if expired {
            rt.refresh_catalog();
            rt.plan_all(now)
        } else {
            changed.into_iter().map(|u| (u.clone(), rt.plan_player(&u, now))).filter(|(_, o)| !o.is_empty()).collect()
        };
        (plans, rt.resend())
    });
    state::flush_logs();
    if let Some((plans, resend)) = result {
        push::apply_all(server, plans, resend);
    }
}

/// Answers Pumpkin's permission check from memory. It makes exactly one host
/// call (the player's UUID) and never one that checks permissions, so it
/// cannot start another check; while the plugin is busy with something else
/// (a nested call) Pumpkin's own answer stays.
pub struct Check;

impl EventHandler<PlayerPermissionCheckEvent> for Check {
    fn handle(
        &self,
        _server: Server,
        mut e: EventData<PlayerPermissionCheckEvent>,
    ) -> EventData<PlayerPermissionCheckEvent> {
        if state::passive() {
            // The proxy's PumboPerms rules: PumboBridge's attachments decide.
            return e;
        }
        let uuid = api::uuid_string(&e.player.get_id());
        let now = now_ms();
        let node = e.permission.clone();
        let answer = state::with(|rt| {
            rt.answered += 1;
            let (name, world, op) =
                rt.online.get(&uuid).map(|o| (o.name.clone(), o.world.clone(), o.op_level)).unwrap_or_default();
            if node == crate::COMMAND_NODE {
                return Some(command::allowed_any(&mut rt.engine, &Who::player(&name, &uuid, op, &world), now));
            }
            let canonical = pumbo_perms_core::node::normalize(&node).ok()?;
            rt.engine.decide(&uuid, &canonical, &world, op, now)
        });
        match answer {
            Some(Some(v)) => e.permission_result = v,
            Some(None) => {}
            None => {
                SKIPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
        e
    }
}

/// `/tp`, `/xp`, `/banip`, `/pardonip` with the permission of their command,
/// which Pumpkin does not check (`pumbo_common::pumpkin::UNGUARDED_ALIASES`).
/// Not in check-event mode: `has-permission` would call back into this plugin.
pub struct AliasGuard;

impl EventHandler<PlayerCommandSendEvent> for AliasGuard {
    fn handle(&self, _server: Server, mut e: EventData<PlayerCommandSendEvent>) -> EventData<PlayerCommandSendEvent> {
        if !e.cancelled
            && let Some(node) = pumbo_common::pumpkin::unguarded_alias(&e.command)
            && !e.player.has_permission(node)
        {
            e.cancelled = true;
            let t = TextComponent::translate("command.unknown.command", Vec::new()).color_named(NamedColor::Red);
            e.player.send_system_message(t, false);
        }
        e
    }
}
