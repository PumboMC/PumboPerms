//! Staff limits of `/pp`: which players, groups, tracks and arguments a sender may change.
//!
//! Both switches (`limits` in `config.yml`) are off by default; off, and for
//! the console, every check lets the command through. A node has a value
//! when [`Engine::decide_in`] gives one at the sender's place, with wildcards
//! and the operator rule, as for the right to the action itself
//! ([`super::allowed`]). The checks read the data before the change.

use pumbo_common::command::permission;
use pumbo_common::id::Uuid;
use pumbo_common::text::Args;

use crate::context::Contexts;
use crate::engine::Engine;
use crate::ui::{self, Text, value};

use super::target::Target;
use super::{PLUGIN, Who};

/// The fixed nodes of the staff limits, with their descriptions. Only
/// PumboPerms asks for them, so they are not in Pumpkin's registry.
pub const LIMIT_NODES: &[(&str, &str)] = &[
    ("pumbo.perms.modify.user.self", "Change your own data with /pp (staff limits)"),
    ("pumbo.perms.modify.user.others", "Change other players with /pp (staff limits)"),
    ("pumbo.perms.view.user.self", "See your own data with /pp (staff limits)"),
    ("pumbo.perms.view.user.others", "See other players with /pp (staff limits)"),
    ("pumbo.perms.usecontext.global", "Make /pp changes without a context (staff limits)"),
];

/// What a command changes or shows.
pub(super) enum Aim<'a> {
    Holder(&'a Target),
    Track(&'a str),
}

/// A change whose target passed [`target`]: the node of its action, for
/// [`change`].
pub(super) struct Asked(String);

/// What a change asks for beyond its target.
#[derive(Default)]
pub(super) struct Change<'a> {
    /// The node, group, meta key, `prefix` or `suffix`, or `<track>.<group>`
    /// of a track step.
    pub argument: Option<String>,
    /// `None`: the command has no contexts; `Some(global)`: a change without one.
    pub contexts: Option<&'a Contexts>,
}

/// Proof that the limits were checked; only this module makes one.
pub(super) struct Checked(());

/// The target of a change with the right to `action` (`user.parent.add`).
/// With `argument-permissions` on, a player needs `<action node>.modify.self`
/// for themselves and `.modify.others` for anyone else, a group
/// `.modify.<group>`, a track `.modify.<track>`; where that node has no value,
/// the general one: `pumbo.perms.modify.user.self|others`,
/// `pumbo.perms.modify.group.<group>`, `pumbo.perms.modify.track.<track>`.
pub(super) fn target(e: &mut Engine, who: &Who, action: &str, aim: Aim<'_>, now: u64) -> Result<Asked, Text> {
    aimed(e, who, action, &aim, "modify", now)?;
    Ok(Asked(permission(PLUGIN, action)))
}

/// The target of a view: [`target`] with `view` in place of `modify`.
pub(super) fn view(e: &mut Engine, who: &Who, action: &str, aim: Aim<'_>, now: u64) -> Result<(), Text> {
    aimed(e, who, action, &aim, "view", now)
}

/// The rest of a change, read before it is made; the refusal names every
/// node that is missing. With `argument-permissions` on, `<action
/// node>.<argument>` has to give `true`. A change without a context needs
/// `<action node>.usecontext.global`, or where that has no value
/// `pumbo.perms.usecontext.global`; a change with contexts is refused only
/// when, for one `key=value`, `<action node>.usecontext.<key>.<value>` or,
/// without a value, `pumbo.perms.usecontext.<key>.<value>` gives `false`.
pub(super) fn change(e: &mut Engine, who: &Who, asked: &Asked, c: &Change<'_>, now: u64) -> Result<Checked, Text> {
    if who.is_console() || !e.cfg.limits.argument_permissions {
        return Ok(Checked(()));
    }
    let own = &asked.0;
    let mut nodes = Vec::new();
    match c.contexts {
        Some(ctx) if ctx.is_global() => {
            let general = permission(PLUGIN, "usecontext.global");
            nodes.extend(missing(e, who, &format!("{own}.usecontext.global"), &general, now));
        }
        Some(ctx) => {
            for (key, val) in ctx.iter() {
                let mine = format!("{own}.usecontext.{key}.{val}");
                let general = permission(PLUGIN, &format!("usecontext.{key}.{val}"));
                let node = match decide(e, who, &mine, now) {
                    Some(v) => (!v).then_some(mine),
                    None => (decide(e, who, &general, now) == Some(false)).then_some(general),
                };
                nodes.extend(node);
            }
        }
        None => {}
    }
    if let Some(arg) = &c.argument {
        let node = format!("{own}.{arg}");
        if decide(e, who, &node, now) != Some(true) {
            nodes.push(node);
        }
    }
    if nodes.is_empty() { Ok(Checked(())) } else { Err(refusal(e, &nodes)) }
}

/// A new group or track: there is nothing to limit.
pub(super) fn unlimited() -> Checked {
    Checked(())
}

fn aimed(e: &mut Engine, who: &Who, action: &str, aim: &Aim<'_>, verb: &str, now: u64) -> Result<(), Text> {
    let Some(sender) = who.uuid.as_deref() else { return Ok(()) };
    if !e.cfg.limits.argument_permissions {
        return Ok(());
    }
    let (name, general) = match aim {
        Aim::Holder(Target::User { uuid, .. }) => {
            let whose = if same_player(sender, uuid) { "self" } else { "others" };
            (whose, format!("{verb}.user.{whose}"))
        }
        Aim::Holder(Target::Group { name }) => (name.as_str(), format!("{verb}.group.{name}")),
        Aim::Track(track) => (*track, format!("{verb}.track.{track}")),
    };
    let own = format!("{}.{verb}.{name}", permission(PLUGIN, action));
    match missing(e, who, &own, &permission(PLUGIN, &general), now) {
        Some(node) => Err(refusal(e, &[node])),
        None => Ok(()),
    }
}

/// `None` when `own` gives `true`, or has no value and `general` gives
/// `true`; otherwise the node that would let the command through.
fn missing(e: &mut Engine, who: &Who, own: &str, general: &str, now: u64) -> Option<String> {
    match decide(e, who, own, now) {
        Some(true) => None,
        Some(false) => Some(own.to_string()),
        None => (decide(e, who, general, now) != Some(true)).then(|| general.to_string()),
    }
}

/// The sender's value of `node` where the sender is.
fn decide(e: &mut Engine, who: &Who, node: &str, now: u64) -> Option<bool> {
    let uuid = who.uuid.as_deref()?;
    let q = e.place(&who.server, &who.world);
    e.decide_in(uuid, node, &q, who.op_level, now)
}

/// Whether two UUIDs are one player, in any spelling.
fn same_player(a: &str, b: &str) -> bool {
    match (Uuid::parse(a), Uuid::parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn refusal(e: &Engine, nodes: &[String]) -> Text {
    let nodes: Vec<String> = nodes.iter().map(value).collect();
    ui::error(&e.lang, "limit-node", &Args::new().arg(nodes.join(", ")))
}
