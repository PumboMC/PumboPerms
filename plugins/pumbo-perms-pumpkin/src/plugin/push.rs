//! Setting the decided permissions on players (Pumpkin attachments).

use pumbo_perms_core::host::Op;

use super::api::{self, Player, Server};
use super::state::{self, Plan};

/// Applies the changes to one player. With `resend`, the player gets its
/// command list again when a change can affect it (setting the operator level
/// to the same value makes Pumpkin send it), so Tab completion matches the new
/// permissions. Pumpkin's command requirements always use namespaced nodes
/// (`minecraft:command.give`, `<plugin>:<node>`), so changes to other nodes
/// never need it.
pub fn apply(player: &Player, ops: &[Op], resend: bool) {
    if ops.is_empty() {
        return;
    }
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
    if resend && commands {
        player.set_permission_level(player.get_permission_level());
    }
}

/// Applies plans to the players still online (`me`: the player of the
/// event, which the server may not list), then records them in the state.
pub fn apply_plans(server: &Server, me: Option<&Player>, plans: Vec<Plan>, resend: bool) {
    if plans.is_empty() {
        return;
    }
    let mut done = Vec::new();
    for plan in plans {
        let mine = me.filter(|p| api::uuid_string(&p.get_id()) == plan.uuid);
        let found = if mine.is_none() {
            api::parse_uuid(&plan.uuid).and_then(|id| server.get_player_by_uuid(id))
        } else {
            None
        };
        let Some(player) = mine.or(found.as_ref()) else { continue };
        apply(player, &plan.ops, resend);
        done.push(plan);
    }
    state::with(|rt| rt.applied(done));
}
