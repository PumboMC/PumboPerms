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

use crate::context::{self, Contexts, Query};
use crate::engine::Engine;
use crate::ui::{self, Text, value};

use super::edit::in_context;
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
    /// Groups the sender has to be a member of.
    pub groups: Vec<&'a str>,
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

/// The rest of a change, read before it is made: contexts, the argument and
/// the groups ([`Change`]); one refusal names every missing node and group.
pub(super) fn change(e: &mut Engine, who: &Who, asked: &Asked, c: &Change<'_>, now: u64) -> Result<Checked, Text> {
    let Some(sender) = who.uuid.as_deref() else { return Ok(Checked(())) };
    let nodes = if e.cfg.limits.argument_permissions { missing_nodes(e, who, asked, c, now) } else { Vec::new() };
    let groups = if e.cfg.limits.require_group_membership { missing_groups(e, sender, c, now) } else { Vec::new() };
    if nodes.is_empty() && groups.is_empty() {
        return Ok(Checked(()));
    }
    let mut out = Text::default();
    if !nodes.is_empty() {
        out.lines.extend(ui::error(&e.lang, "limit-node", &Args::new().arg(nodes.join(", "))).lines);
    }
    if !groups.is_empty() {
        out.lines.extend(ui::error(&e.lang, "limit-member", &Args::new().arg(groups.join(", "))).lines);
    }
    Err(out)
}

/// With `argument-permissions` on, `<action node>.<argument>` has to give
/// `true`. A change without a context needs `<action
/// node>.usecontext.global`, or where that has no value
/// `pumbo.perms.usecontext.global`; a change with contexts is refused only
/// when, for one `key=value`, `<action node>.usecontext.<key>.<value>` or,
/// without a value, `pumbo.perms.usecontext.<key>.<value>` gives `false`.
fn missing_nodes(e: &mut Engine, who: &Who, asked: &Asked, c: &Change<'_>, now: u64) -> Vec<String> {
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
    nodes.into_iter().map(value).collect()
}

/// With `require-group-membership` on, the sender inherits each group where
/// the change applies, directly or through other groups, `default` where the
/// engine gives it, a temporary membership while it lasts. Wildcards and the
/// operator rule do not count. The groups the sender is not in, each with
/// the contexts of the change.
fn missing_groups(e: &Engine, sender: &str, c: &Change<'_>, now: u64) -> Vec<String> {
    if c.groups.is_empty() {
        return Vec::new();
    }
    let ctx = c.contexts.cloned().unwrap_or_default();
    let inherited = e.perms.inheritance(sender, &place_of_change(e, &ctx), now);
    let mut out: Vec<String> = Vec::new();
    for g in &c.groups {
        let shown = format!("{}{}", value(g), in_context(e, &ctx));
        if !inherited.iter().any(|(name, _)| name == g) && !out.contains(&shown) {
            out.push(shown);
        }
    }
    out
}

/// Where a change with these contexts applies: a named server with its
/// groups, the `group=` of the change, the world; nothing named, nowhere in
/// particular (never this server, whose name a global change does not have).
fn place_of_change(e: &Engine, ctx: &Contexts) -> Query {
    let world = ctx.get(context::WORLD).unwrap_or("");
    let mut q = match ctx.get(context::SERVER) {
        Some(server) => e.place(server, world),
        None => Query::at("", Vec::new(), world),
    };
    if let Some(group) = ctx.get(context::GROUP)
        && !q.groups.iter().any(|g| g == group)
    {
        q.groups.push(group.to_string());
    }
    q
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
        Some(node) => Err(ui::error(&e.lang, "limit-node", &Args::new().arg(value(node)))),
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
