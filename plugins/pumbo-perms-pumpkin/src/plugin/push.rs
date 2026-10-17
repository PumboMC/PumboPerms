//! Setting the decided permissions on players (Pumpkin attachments).

use pumbo_common::clock::now_ms;
use pumbo_perms_core::host::Op;

use super::api::{self, Player, Server};
use super::state::{self, Plan};

/// Applies the changes to one player; returns whether one can change its
/// command list. Pumpkin's command requirements always use namespaced nodes
/// (`minecraft:command.give`, `<plugin>:<node>`), so changes to other nodes
/// never need a new list.
pub fn apply(player: &Player, ops: &[Op]) -> bool {
    let mut commands = false;
    for op in ops {
        match op {
            Op::Set(node, value) => {
                commands |= node.contains(':');
                player.set_permission(node, *value);
            }
            Op::Unset(node) => {
                commands |= node.contains(':');
                player.unset_permission(node);
            }
        }
    }
    commands
}

/// Sends the command list again (setting the operator level to the same
/// value makes Pumpkin send it), so Tab completion matches the permissions.
pub fn resend(player: &Player) {
    player.set_permission_level(player.get_permission_level());
}

/// Applies plans to the players still online (`me`: the player of the
/// event, which the server may not list), then records them in the state.
/// With `resend`, a player whose command list changed gets it again, at most
/// once a second (`state::RESEND_MS`).
pub fn apply_plans(server: &Server, me: Option<&Player>, plans: Vec<Plan>, resend: bool) {
    if plans.is_empty() {
        return;
    }
    let mut done = Vec::new();
    let mut touched = Vec::new();
    for plan in plans {
        let mut found = None;
        on_player(server, me, &plan.uuid, |player| found = Some(apply(player, &plan.ops)));
        let Some(commands) = found else { continue };
        if commands && resend {
            touched.push(plan.uuid.clone());
        }
        done.push(plan);
    }
    let now = now_ms();
    let due = state::with(|rt| {
        rt.applied(done);
        rt.resends(touched, now)
    });
    resend_to(server, me, due.unwrap_or_default());
}

/// Command lists that waited for their turn.
pub fn resend_due(server: &Server) {
    let now = now_ms();
    let due = state::with(|rt| rt.due_resends(now)).unwrap_or_default();
    resend_to(server, None, due);
}

fn resend_to(server: &Server, me: Option<&Player>, uuids: Vec<String>) {
    for uuid in uuids {
        on_player(server, me, &uuid, resend);
    }
}

/// Runs `f` on the online player `uuid`.
fn on_player(server: &Server, me: Option<&Player>, uuid: &str, f: impl FnOnce(&Player)) {
    if let Some(p) = me.filter(|p| api::uuid_string(&p.get_id()) == uuid) {
        f(p);
    } else if let Some(p) = api::parse_uuid(uuid).and_then(|id| server.get_player_by_uuid(id)) {
        f(&p);
    }
}
