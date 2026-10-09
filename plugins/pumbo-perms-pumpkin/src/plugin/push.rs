//! Setting the decided permissions on players (Pumpkin attachments).

use pumbo_perms_core::host::Op;

use super::api::{self, Player, Server};

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

/// Applies changes to players by UUID (those that left meanwhile are skipped).
pub fn apply_all(server: &Server, plans: Vec<(String, Vec<Op>)>, resend: bool) {
    for (uuid, ops) in plans {
        if let Some(id) = api::parse_uuid(&uuid)
            && let Some(player) = server.get_player_by_uuid(id)
        {
            apply(&player, &ops, resend);
        }
    }
}
