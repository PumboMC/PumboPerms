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
pub fn track(server: &Server, player: &Player) {
    let uuid = api::uuid_string(&player.get_id());
    let online = Online {
        name: player.get_name(),
        world: player.get_world().get_name(),
        op_level: api::level_of(player.get_permission_level()),
    };
    let now = now_ms();
    let result = state::with(|rt| (rt.join(&uuid, online, now), rt.resend()));
    state::flush_logs();
    if let Some((plans, resend)) = result {
        push::apply_plans(server, Some(player), plans, resend);
    }
}

pub struct Join;

impl EventHandler<PlayerJoinEvent> for Join {
    fn handle(&self, server: Server, e: EventData<PlayerJoinEvent>) -> EventData<PlayerJoinEvent> {
        track(&server, &e.player);
        e
    }
}

pub struct Leave;

impl EventHandler<PlayerLeaveEvent> for Leave {
    fn handle(&self, server: Server, e: EventData<PlayerLeaveEvent>) -> EventData<PlayerLeaveEvent> {
        let uuid = api::uuid_string(&e.player.get_id());
        let now = now_ms();
        let result = state::with(|rt| (rt.leave(&uuid, now), rt.resend()));
        state::flush_logs();
        if let Some(((ops, plans), resend)) = result {
            // No command list for a player who is leaving.
            push::apply(&e.player, &ops);
            push::apply_plans(&server, None, plans, resend);
        }
        e
    }
}

pub struct ChangeWorld;

impl EventHandler<PlayerChangeWorldEvent> for ChangeWorld {
    fn handle(&self, server: Server, e: EventData<PlayerChangeWorldEvent>) -> EventData<PlayerChangeWorldEvent> {
        let uuid = api::uuid_string(&e.player.get_id());
        let world = e.new_world.get_name();
        let now = now_ms();
        let cancelled = e.cancelled;
        let result = state::with(|rt| (rt.change_world(&uuid, world, cancelled, now), rt.resend()));
        state::flush_logs();
        if let Some((plans, resend)) = result {
            push::apply_plans(&server, Some(&e.player), plans, resend);
        }
        e
    }
}

/// Every `refresh-seconds`: expired nodes, changed operator levels and worlds.
pub fn tick(server: &Server) {
    if !state::with(|rt| rt.refresh_due(now_ms())).unwrap_or(false) {
        return;
    }
    let players: Vec<(String, u8, String)> = server
        .get_all_players()
        .iter()
        .map(|p| (api::uuid_string(&p.get_id()), api::level_of(p.get_permission_level()), p.get_world().get_name()))
        .collect();
    let now = now_ms();
    let result = state::with(|rt| (rt.refresh(&players, now), rt.resend()));
    state::flush_logs();
    if let Some((plans, resend)) = result {
        push::apply_plans(server, None, plans, resend);
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
            if node == pumbo_perms_core::host::command_node() {
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

/// `/tp`, `/xp`, `/banip`, `/pardonip` (also after `execute ... run`) with the
/// permission of their command, which Pumpkin does not check
/// (`pumbo_common::pumpkin::UNGUARDED_ALIASES`). In check-event mode the answer
/// comes from memory: `has-permission` would call back into this plugin.
pub struct AliasGuard {
    pub check_event: bool,
}

impl EventHandler<PlayerCommandSendEvent> for AliasGuard {
    fn handle(&self, server: Server, mut e: EventData<PlayerCommandSendEvent>) -> EventData<PlayerCommandSendEvent> {
        // Before Pumpkin checks the command: an expired node is gone even
        // while the periodic task stands (`/tick freeze`). The command list
        // follows at the next change, not from inside this event.
        let now = now_ms();
        if let Some(plans) = state::with(|rt| rt.due_expiry(now)) {
            state::flush_logs();
            push::apply_plans(&server, None, plans, false);
        }
        if !e.cancelled
            && let Some(node) = pumbo_common::pumpkin::unguarded_alias(&e.command)
            && !self.allowed(&e.player, node)
        {
            e.cancelled = true;
            let t = TextComponent::translate("command.unknown.command", Vec::new()).color_named(NamedColor::Red);
            e.player.send_system_message(t, false);
        }
        e
    }
}

impl AliasGuard {
    fn allowed(&self, player: &Player, node: &str) -> bool {
        // Passive: the Check handler stays out, PumboBridge's attachments decide.
        if !self.check_event || state::passive() {
            return player.has_permission(node);
        }
        let uuid = api::uuid_string(&player.get_id());
        let now = now_ms();
        // Busy (a nested call): refuse, the player can try again.
        state::with(|rt| rt.alias_allowed(&uuid, node, now)).unwrap_or(false)
    }
}
