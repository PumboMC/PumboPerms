//! `/pp user` and `/pp group`: permissions, parents, meta and track steps.

use pumbo_common::style;
use pumbo_common::text::{self, Args, Style};

use crate::context;
use crate::engine::Engine;
use crate::model::{Data, MetaKind, ParentNode, PermNode};
use crate::node;
use crate::perms::{PermsError, Source};
use crate::ui::{self, Line, Text, value};

use super::edit::{
    coarse, context_hint, expiry, in_context, left, parse_bool, parse_duration, parse_int, parse_node, perms_error,
    save, split_any_context, split_context,
};
use super::help::{AFFIX_CHANGES, PARENT_CHANGES, holder_help, unnamed_help};
use super::target::{Target, checked_key, checked_text, find_user, is_meta_key};
use super::view::{context_args, group_view, members_view, meta_view, nodes_view, parents_view, user_view};
use super::{Env, Outcome, Res, Who, allowed_prefix, lower, need, page_of, view_page, writable};

pub(super) fn user_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let Some(name) = args.first() else {
        return unnamed_help(e, who, "user", env.now);
    };
    let known = find_user(e, env, name)?;
    let t = Target::User { uuid: known.uuid, name: known.name };
    holder_cmd(e, who, &t, args.get(1..).unwrap_or(&[]), env)
}

pub(super) fn group_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let Some(name) = args.first().map(|n| n.to_lowercase()) else {
        return unnamed_help(e, who, "group", now);
    };
    let sub = lower(args.get(1));
    let t = Target::Group { name: name.clone() };
    match sub.as_str() {
        "create" => {
            need(e, who, "group.create", now)?;
            writable(e)?;
            let dirty = e.perms.create_group(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-created", &Args::new().arg(value(&name)));
            return save(e, who, dirty, ("group", &name, "create".into()), reply, now);
        }
        "delete" => {
            need(e, who, "group.delete", now)?;
            writable(e)?;
            let dirty = e.perms.delete_group(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-deleted", &Args::new().arg(value(&name)));
            return save(e, who, dirty, ("group", &name, "delete".into()), reply, now);
        }
        _ => {}
    }
    if e.perms.group(&name).is_none() {
        return Err(perms_error(e, &PermsError::NoGroup(name)));
    }
    let rest = args.get(2..).unwrap_or(&[]);
    match sub.as_str() {
        "rename" => {
            need(e, who, "group.rename", now)?;
            writable(e)?;
            let Some(to) = rest.first().map(|n| n.to_lowercase()) else {
                return Err(ui::usage(&e.lang, &format!("{} rename <new-name>", t.command(&e.root))));
            };
            let dirty = e.perms.rename_group(&name, &to).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-renamed", &Args::new().arg(value(&name)).arg(value(&to)));
            save(e, who, dirty, ("group", &name, format!("rename {to}")), reply, now)
        }
        "setweight" => {
            need(e, who, "group.setweight", now)?;
            writable(e)?;
            let Some(raw) = rest.first() else {
                return Err(ui::usage(&e.lang, &format!("{} setweight <weight>", t.command(&e.root))));
            };
            let w = parse_int(e, raw)?;
            let ((), dirty) = e.perms.edit_group(&name, |g| g.weight = w).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "group-weight", &Args::new().arg(value(&name)).arg(value(w)));
            save(e, who, dirty, ("group", &name, format!("setweight {w}")), reply, now)
        }
        "setdisplayname" => {
            need(e, who, "group.setdisplayname", now)?;
            writable(e)?;
            if rest.is_empty() {
                return Err(ui::usage(&e.lang, &format!("{} setdisplayname <name|clear>", t.command(&e.root))));
            }
            let shown = rest.join(" ");
            checked_text(e, &shown)?;
            let clear = shown.eq_ignore_ascii_case("clear");
            let new = if clear { String::new() } else { shown.clone() };
            let ((), dirty) =
                e.perms.edit_group(&name, |g| g.display_name = new).map_err(|err| perms_error(e, &err))?;
            let reply = if clear {
                ui::success(&e.lang, "group-displayname-cleared", &Args::new().arg(value(&name)))
            } else {
                ui::success(&e.lang, "group-displayname", &Args::new().arg(value(&name)).arg(format!("{shown}&r")))
            };
            save(e, who, dirty, ("group", &name, format!("setdisplayname {shown}")), reply, now)
        }
        "listmembers" => {
            need(e, who, "group.listmembers", now)?;
            Ok(Outcome::reply(members_view(e, &name, page_of(rest), now)))
        }
        _ => holder_cmd(e, who, &t, args.get(1..).unwrap_or(&[]), env),
    }
}

fn holder_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let kind = t.kind();
    let sub = lower(args.first());
    let rest = args.get(1..).unwrap_or(&[]);
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, &format!("{kind}."), now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            let base = t.command(&e.root);
            Ok(Outcome::reply(holder_help(e, who, &base, t.is_user(), "", page_of(rest), now)))
        }
        "info" | "i" => {
            need(e, who, &format!("{kind}.info"), now)?;
            let page = match rest {
                [] => view_page(who, rest),
                [p] if p.parse::<usize>().is_ok() => view_page(who, rest),
                _ => return Err(ui::usage(&e.lang, &format!("{} info [page]", t.command(&e.root)))),
            };
            Ok(Outcome::reply(if t.is_user() { user_view(e, t, page, now) } else { group_view(e, t, page, now) }))
        }
        "permission" | "perm" | "p" | "permissions" => permission_cmd(e, who, t, rest, now),
        "parent" | "parents" | "inherit" => parent_cmd(e, who, t, rest, now),
        "meta" => meta_cmd(e, who, t, rest, now),
        "promote" | "demote" if t.is_user() => step_cmd(e, who, t, &sub, rest, now),
        "clear" if t.is_user() => {
            need(e, who, "user.clear", now)?;
            if !rest.is_empty() {
                return Err(ui::usage(&e.lang, &format!("{} clear", t.command(&e.root))));
            }
            writable(e)?;
            let ((), dirty) = t.edit(e, |d| *d = Data::default()).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "user-cleared", &Args::new().arg(t.shown(e)));
            save(e, who, dirty, (kind, &t.label(), "clear".into()), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} help", t.command(&e.root)))),
    }
}

fn permission_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let rest = args.get(1..).unwrap_or(&[]);
    let (pos, ctx) = if sub == "check" { split_any_context(e, rest)? } else { split_context(e, rest)? };
    let action_log = format!("permission {}", args.join(" "));
    let base = format!("{} permission", t.command(&e.root));
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, &format!("{kind}.permission."), now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            let base = t.command(&e.root);
            Ok(Outcome::reply(holder_help(e, who, &base, t.is_user(), "permission", page_of(&pos), now)))
        }
        "info" | "list" => {
            need(e, who, &format!("{kind}.permission.info"), now)?;
            Ok(Outcome::reply(nodes_view(e, t, view_page(who, &pos), now)))
        }
        "set" | "settemp" => {
            need(e, who, &format!("{kind}.permission.set"), now)?;
            writable(e)?;
            let temp = sub == "settemp";
            let usage = if temp {
                format!("{base} settemp <node> [true|false] <duration> [context...]")
            } else {
                format!("{base} set <node> [true|false] [context...]")
            };
            let Some(raw) = pos.first() else { return Err(ui::usage(&e.lang, &usage)) };
            let node = parse_node(e, raw)?;
            not_group_node(e, t, &node)?;
            let (value_arg, dur_arg) = match (temp, pos.len()) {
                (false, 1) => (None, None),
                (false, 2) => (pos.get(1), None),
                (true, 2) => (None, pos.get(1)),
                (true, 3) => (pos.get(1), pos.get(2)),
                _ => return Err(ui::usage(&e.lang, &usage)),
            };
            let v = match value_arg {
                Some(raw) => parse_bool(e, raw)?,
                None => true,
            };
            let dur = match dur_arg {
                Some(raw) => Some(parse_duration(e, raw)?),
                None => None,
            };
            // Changing an end time could cut a punishment short without a word.
            let timed = |p: &PermNode| p.node == node && p.context == ctx && p.expiry.is_some_and(|x| x > now);
            if temp && t.data(e).is_some_and(|d| d.permissions.iter().any(timed)) {
                let undo = format!("{base} unsettemp {node}{}", context_args(&ctx));
                return Err(ui::error(&e.lang, "perm-settemp-exists", &Args::new().arg(value(&node)).arg(value(undo))));
            }
            let new =
                PermNode { node: node.clone(), value: v, context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            let (_, dirty) = t.edit(e, |d| d.set_permission(new)).map_err(|err| perms_error(e, &err))?;
            let mut a = Args::new()
                .with("node", value(&node))
                .with("value", ui::boolean(v))
                .with("target", t.shown(e))
                .with("context", in_context(e, &ctx));
            let key = match dur {
                Some(d) => {
                    a.set("duration", value(coarse(d)));
                    "perm-settemp"
                }
                None => "perm-set",
            };
            let reply = ui::success(&e.lang, key, &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "unset" | "unsettemp" => {
            need(e, who, &format!("{kind}.permission.unset"), now)?;
            writable(e)?;
            let Some(raw) = pos.first() else {
                return Err(ui::usage(&e.lang, &format!("{base} {sub} <node> [context...]")));
            };
            let node = parse_node(e, raw)?;
            not_group_node(e, t, &node)?;
            let temp = sub == "unsettemp";
            let (removed, dirty) =
                t.edit(e, |d| d.unset_permission(&node, &ctx, temp)).map_err(|err| perms_error(e, &err))?;
            let a =
                Args::new().with("node", value(&node)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
            if removed.is_none() {
                e.rollback();
                return Err(ui::error(&e.lang, "perm-unset-none", &a));
            }
            let reply = ui::success(&e.lang, "perm-unset", &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "clear" => {
            need(e, who, &format!("{kind}.permission.clear"), now)?;
            writable(e)?;
            let only =
                if ctx.is_global() && !args.iter().any(|a| context::is_context_arg(a)) { None } else { Some(&ctx) };
            let (count, dirty) = t.edit(e, |d| d.clear_permissions(only)).map_err(|err| perms_error(e, &err))?;
            let a =
                Args::new().with("count", value(count)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
            let reply = ui::success(&e.lang, "perm-clear", &a);
            save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
        }
        "check" => {
            need(e, who, &format!("{kind}.permission.check"), now)?;
            let Some(raw) = pos.first() else {
                return Err(ui::usage(&e.lang, &format!("{base} check <node> [context...]")));
            };
            let node = parse_node(e, raw)?;
            let mut q = e.place(ctx.get(context::SERVER).unwrap_or(""), ctx.get(context::WORLD).unwrap_or(""));
            if let Some(g) = ctx.get(context::GROUP) {
                q.groups = vec![g.to_string()];
            }
            let decision = match t {
                Target::User { uuid, .. } => e.perms.explain(uuid, &node, &q, now),
                Target::Group { name } => e.perms.group_effective(name, &q, now).explain(&node).cloned(),
            };
            // A view: the player by name as in its info title (the UUID is in
            // `info`), so the lines fit the chat; a group as `group vip`.
            let target = if t.is_user() { value(t.label()) } else { t.shown(e) };
            let a = Args::new().with("node", value(&node)).with("target", target.clone());
            let mut reply = match decision {
                Some(d) => {
                    let mut text = ui::header(&e.lang, "perm-check", &a.clone().with("value", ui::boolean(d.value)));
                    let source = match &d.source {
                        Source::User => target.clone(),
                        Source::Group(g) => e.lang.format("target-group", &Args::new().arg(value(g))),
                    };
                    let mut how = e.lang.format(
                        "perm-check-source",
                        &Args::new()
                            .with("source", source)
                            .with("node", value(&d.node))
                            .with("context", context_hint(e, &d.context)),
                    );
                    if let Some(x) = d.expiry {
                        let ends = e.lang.format("expires-in", &Args::new().arg(left(now, x)));
                        how.push_str(&format!("{}{ends}&r", ui::INFO));
                    }
                    text.push(Line::parse_with(&format!("  {how}"), Style::colored(style::LABEL)));
                    text
                }
                None => ui::header(&e.lang, "perm-check-undecided", &a),
            };
            if let Target::User { uuid, .. } = t {
                let host = node::host_form(&node);
                let hidden = crate::host::hidden(e, uuid, &q, now);
                hidden_lines(e, hidden.iter().filter(|(w, _)| node::covers(w, &host)), &mut reply.lines);
            }
            if !e.cfg.worlds && ctx.get(context::WORLD).is_some() {
                reply.push(ui::row(1, "", &format!("{}! {}", ui::WARN, e.lang.get("perm-check-no-worlds"))));
            }
            Ok(Outcome::reply(reply))
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    }
}

/// `group.<name>` tells membership: it is given with `parent add`.
fn not_group_node(e: &Engine, t: &Target, node: &str) -> Result<(), Text> {
    let Some(group) = node.strip_prefix(node::GROUP_PREFIX) else { return Ok(()) };
    let a =
        Args::new().with("node", value(node)).with("command", value(t.command(&e.root))).with("group", value(group));
    Err(ui::error(&e.lang, "perm-group-node", &a))
}

/// "not sent to Pumpkin: <wildcard> hides <narrower>" ([`crate::host::hidden`]).
pub(super) fn hidden_lines<'a>(e: &Engine, hidden: impl Iterator<Item = &'a (String, String)>, out: &mut Vec<Line>) {
    for (w, narrower) in hidden {
        let line = e.lang.format("perm-hidden", &Args::new().arg(value(w)).arg(value(narrower)));
        out.push(Line::parse_with(&format!("  ! {line}"), Style::colored(style::WARN)));
    }
}

fn parent_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("parent {}", args.join(" "));
    let base = format!("{} parent", t.command(&e.root));
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.parent."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        let base = t.command(&e.root);
        return Ok(Outcome::reply(holder_help(e, who, &base, t.is_user(), "parent", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.parent.info"), now)?;
        return Ok(Outcome::reply(parents_view(e, t, view_page(who, &pos), now)));
    }
    let Some(&(_, action, temp)) = PARENT_CHANGES.iter().find(|(w, _, _)| *w == sub) else {
        return Err(ui::unknown(&e.lang, &sub, &format!("{base} help")));
    };
    need(e, who, &format!("{kind}.parent.{action}"), now)?;
    writable(e)?;
    let adds_temp = temp && action == "add";
    let usage = if adds_temp {
        format!("{base} addtemp <group> <duration> [context...]")
    } else {
        format!("{base} {sub} <group> [context...]")
    };
    let Some(group) = pos.first().map(|g| g.to_lowercase()) else { return Err(ui::usage(&e.lang, &usage)) };
    let dur = if adds_temp {
        match pos.get(1) {
            Some(raw) => Some(parse_duration(e, raw)?),
            None => return Err(ui::usage(&e.lang, &usage)),
        }
    } else {
        None
    };
    if action != "remove" && e.perms.group(&group).is_none() {
        return Err(perms_error(e, &PermsError::NoGroup(group)));
    }
    let a = Args::new().with("group", value(&group)).with("target", t.shown(e)).with("context", in_context(e, &ctx));
    let (key, dirty) = match (action, t) {
        ("add", Target::Group { name }) => {
            let p = ParentNode { group: group.clone(), context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            match e.perms.add_group_parent(name, p) {
                Ok(d) => (if adds_temp { "parent-addtemp" } else { "parent-add" }, d),
                Err(PermsError::Unchanged) => return Err(ui::error(&e.lang, "parent-already", &a)),
                Err(err) => return Err(perms_error(e, &err)),
            }
        }
        ("add", Target::User { .. }) => {
            let p = ParentNode { group: group.clone(), context: ctx.clone(), expiry: dur.map(|d| expiry(now, d)) };
            let (added, d) = t.edit(e, |data| data.add_parent(p)).map_err(|err| perms_error(e, &err))?;
            if !added {
                e.rollback();
                return Err(ui::error(&e.lang, "parent-already", &a));
            }
            (if adds_temp { "parent-addtemp" } else { "parent-add" }, d)
        }
        ("remove", _) => {
            let (removed, d) =
                t.edit(e, |data| data.remove_parent(&group, &ctx, temp)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.rollback();
                return Err(ui::error(&e.lang, "parent-remove-none", &a));
            }
            ("parent-remove", d)
        }
        _ => {
            if let Target::Group { name } = t
                && e.perms.would_cycle(name, &group)
            {
                return Err(perms_error(e, &PermsError::Cycle(group)));
            }
            let ((), d) = t.edit(e, |data| data.set_parent(&group, &ctx)).map_err(|err| perms_error(e, &err))?;
            ("parent-set", d)
        }
    };
    let a = match dur {
        Some(d) => a.with("duration", value(coarse(d))),
        None => a,
    };
    let reply = ui::success(&e.lang, key, &a);
    save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
}

fn meta_cmd(e: &mut Engine, who: &Who, t: &Target, args: &[String], now: u64) -> Res {
    let kind = t.kind();
    let sub = lower(args.first());
    let (pos, ctx) = split_context(e, args.get(1..).unwrap_or(&[]))?;
    let action_log = format!("meta {}", args.join(" "));
    let base = format!("{} meta", t.command(&e.root));
    if matches!(sub.as_str(), "" | "help" | "?") {
        if !allowed_prefix(e, who, &format!("{kind}.meta."), now) {
            return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
        }
        let base = t.command(&e.root);
        return Ok(Outcome::reply(holder_help(e, who, &base, t.is_user(), "meta", page_of(&pos), now)));
    }
    if matches!(sub.as_str(), "info" | "list") {
        need(e, who, &format!("{kind}.meta.info"), now)?;
        return Ok(Outcome::reply(meta_view(e, t, view_page(who, &pos), now)));
    }
    let affix = AFFIX_CHANGES.iter().find(|(w, ..)| *w == sub).map(|&(_, verb, kind, temp)| (verb, kind, temp));
    if affix.is_none() && sub != "set" && sub != "unset" {
        return Err(ui::unknown(&e.lang, &sub, &format!("{base} help")));
    }
    let removing = sub == "unset" || affix.is_some_and(|(verb, ..)| verb == "remove");
    need(e, who, &format!("{kind}.meta.{}", if removing { "unset" } else { "set" }), now)?;
    writable(e)?;
    let mut a = Args::new().with("target", t.shown(e)).with("context", in_context(e, &ctx));
    let (key, dirty) = match (sub.as_str(), affix) {
        ("set", _) => {
            let (Some(k), Some(v)) = (pos.first(), pos.get(1)) else {
                return Err(ui::usage(&e.lang, &format!("{base} set <key> <value> [context...]")));
            };
            let k = k.to_lowercase();
            checked_key(e, &k, &format!("{base} set <key> <value> [context...]"))?;
            if !is_meta_key(&k) {
                return Err(ui::error(&e.lang, "bad-meta-key", &Args::new().arg(value(text::escape(&k)))));
            }
            let v = pos.get(1..).map(|p| p.join(" ")).unwrap_or_else(|| v.clone());
            checked_text(e, &v)?;
            a = a.with("key", value(&k)).with("value", value(&v));
            let ((), d) = t.edit(e, |data| data.set_meta(&k, &v, &ctx, None)).map_err(|err| perms_error(e, &err))?;
            ("meta-set", d)
        }
        ("unset", _) => {
            let (Some(k), 1) = (pos.first().map(|k| k.to_lowercase()), pos.len()) else {
                return Err(ui::usage(&e.lang, &format!("{base} unset <key> [context...]")));
            };
            checked_key(e, &k, &format!("{base} unset <key> [context...]"))?;
            a = a.with("key", value(&k));
            let (removed, d) = t.edit(e, |data| data.unset_meta(&k, &ctx)).map_err(|err| perms_error(e, &err))?;
            if !removed {
                e.rollback();
                return Err(ui::error(&e.lang, "meta-unset-none", &a));
            }
            ("meta-unset", d)
        }
        (s, Some((verb, mk, temp))) => {
            let which = if mk == MetaKind::Prefix { "prefix" } else { "suffix" };
            a = a.with("kind", e.lang.get(&format!("kind-{which}")));
            if verb == "remove" {
                let Some(raw) = pos.first() else {
                    return Err(ui::usage(&e.lang, &format!("{base} {s} <priority> [text] [context...]")));
                };
                let prio = parse_int(e, raw)?;
                let text_arg = pos.get(1).cloned();
                a = a.with("priority", value(prio));
                let (count, d) = t
                    .edit(e, |data| data.remove_affix(mk, prio, text_arg.as_deref(), &ctx, temp))
                    .map_err(|err| perms_error(e, &err))?;
                if count == 0 {
                    e.rollback();
                    return Err(ui::error(&e.lang, "affix-remove-none", &a));
                }
                a = a.with("count", value(count));
                ("affix-remove", d)
            } else {
                let replace = verb == "set";
                let usage = if temp {
                    format!("{base} {s} <priority> <text> <duration> [context...]")
                } else {
                    format!("{base} {s} <priority> <text> [context...]")
                };
                let (Some(raw), Some(txt)) = (pos.first(), pos.get(1)) else { return Err(ui::usage(&e.lang, &usage)) };
                let prio = parse_int(e, raw)?;
                checked_text(e, txt)?;
                let dur = if temp {
                    match pos.get(2) {
                        Some(d) => Some(parse_duration(e, d)?),
                        None => return Err(ui::usage(&e.lang, &usage)),
                    }
                } else if pos.len() > 2 {
                    return Err(ui::usage(&e.lang, &usage));
                } else {
                    None
                };
                a = a.with("priority", value(prio)).with("text", format!("{}&r", txt));
                let exp = dur.map(|d| expiry(now, d));
                let (changed, d) = t
                    .edit(e, |data| data.add_affix(mk, prio, txt, &ctx, exp, replace))
                    .map_err(|err| perms_error(e, &err))?;
                if !changed {
                    e.rollback();
                    return Err(ui::error(&e.lang, "unchanged", &Args::new()));
                }
                if let Some(d) = dur {
                    a = a.with("duration", value(coarse(d)));
                }
                (
                    if replace {
                        "affix-set"
                    } else if temp {
                        "affix-addtemp"
                    } else {
                        "affix-add"
                    },
                    d,
                )
            }
        }
        _ => return Err(ui::unknown(&e.lang, &sub, &format!("{base} help"))),
    };
    let reply = ui::success(&e.lang, key, &a);
    save(e, who, dirty, (kind, &t.label(), action_log), reply, now)
}

fn step_cmd(e: &mut Engine, who: &Who, t: &Target, sub: &str, args: &[String], now: u64) -> Res {
    need(e, who, &format!("user.{sub}"), now)?;
    writable(e)?;
    let (pos, ctx) = split_context(e, args)?;
    let Some(track) = pos.first().map(|t| t.to_lowercase()) else {
        return Err(ui::usage(&e.lang, &format!("{} {sub} <track> [context...]", t.command(&e.root))));
    };
    let Target::User { uuid, name } = t else {
        return Err(ui::unknown(&e.lang, sub, &format!("{} help", e.root)));
    };
    let up = sub == "promote";
    let a = Args::new().with("target", t.shown(e)).with("track", value(&track)).with("context", in_context(e, &ctx));
    let result =
        if up { e.perms.promote(uuid, name, &track, &ctx, now) } else { e.perms.demote(uuid, name, &track, &ctx, now) };
    let (step, dirty) = match result {
        Ok(r) => r,
        Err(PermsError::TrackEnd(g)) => return Err(ui::error(&e.lang, "track-end", &a.with("group", value(g)))),
        Err(PermsError::TrackStart(g)) => return Err(ui::error(&e.lang, "track-start", &a.with("group", value(g)))),
        Err(PermsError::NotOnTrack) => return Err(ui::error(&e.lang, "not-on-track", &a)),
        Err(PermsError::Ambiguous(g)) => {
            return Err(ui::error(&e.lang, "track-ambiguous", &a.with("groups", value(g.join(", ")))));
        }
        Err(err) => return Err(perms_error(e, &err)),
    };
    let to = step.to.clone().unwrap_or_default();
    let until = match step.expiry {
        Some(x) => format!(" ({})", e.lang.format("expires-in", &Args::new().arg(value(left(now, x))))),
        None => String::new(),
    };
    let a = a.with("to", value(e.perms.display_name(&to))).with("until", until);
    let reply = match &step.from {
        None => ui::success(&e.lang, "promoted-first", &a),
        Some(from) => {
            let key = if up { "promoted" } else { "demoted" };
            ui::success(&e.lang, key, &a.with("from", value(e.perms.display_name(from))))
        }
    };
    save(e, who, dirty, ("user", &t.label(), format!("{sub} {}", args.join(" "))), reply, now)
}
