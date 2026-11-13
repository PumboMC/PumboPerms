//! Views of players, groups, tracks and the log, and staff notifications.

use std::time::Duration;

use pumbo_common::id::is_java_name;
use pumbo_common::lang::Lang;
use pumbo_common::style;
use pumbo_common::text::{self, Args};

use crate::context::Contexts;
use crate::engine::Engine;
use crate::model::{Data, MetaKind, PermNode};
use crate::store::LogEntry;
use crate::ui::{self, Click, Line, Text, row, value};

use super::edit::{coarse, left};
use super::holder::hidden_lines;
use super::target::{Target, safe_name};
use super::{Env, LOG_PAGE};

pub(super) fn version(e: &Engine, env: &Env) -> Text {
    let mut rows = env.version_rows.clone();
    let storage = match (&e.broken, e.store()) {
        (Some(why), _) => why.clone(),
        (None, Some(_)) => text::strip(&e.lang.format(
            "version-storage",
            &Args::new().arg(e.perms.groups().count()).arg(e.user_count()).arg(e.perms.tracks().count()),
        )),
        (None, None) => "-".into(),
    };
    rows.push((e.lang.get("label-storage"), storage));
    let or_dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };
    // On PumboProx the place is the player's server; the platform lists them.
    if e.server_groups.is_empty() {
        rows.push((
            e.lang.get("label-context"),
            format!("server={} group={}", or_dash(&e.cfg.context.server), or_dash(&e.cfg.context.group)),
        ));
    }
    style::version("PumboPerms", style::accent::PERMS, env.version, rows)
}

fn context_suffix(c: &Contexts) -> String {
    if c.is_global() { String::new() } else { format!(" {}{}", ui::INFO, text::escape(&c.to_string())) }
}

fn expiry_suffix(e: &Engine, x: Option<u64>, now: u64) -> String {
    match x {
        Some(x) => format!(" {}({})", ui::INFO, e.lang.format("expires-in", &Args::new().arg(left(now, x)))),
        None => String::new(),
    }
}

/// `--context` arguments for a command that removes a node again.
pub(super) fn context_args(c: &Contexts) -> String {
    c.iter().map(|(k, v)| format!(" {k}={v}")).collect()
}

fn removable(e: &Engine, line: Line, command: String) -> Line {
    line.on_click(Click::Suggest(command)).on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-remove"))))
}

fn nodes_lines(e: &Engine, t: &Target, data: &Data, now: u64) -> Vec<Line> {
    let mut perms: Vec<&PermNode> = data.permissions.iter().collect();
    perms.sort_by(|a, b| a.node.cmp(&b.node).then(a.context.cmp(&b.context)));
    perms
        .into_iter()
        .map(|p| {
            let unset = if p.expiry.is_some() { "unsettemp" } else { "unset" };
            let text = format!(
                "    {}• {}{} {}= {}{}{}",
                ui::MUTED,
                ui::VALUE,
                p.node,
                ui::MUTED,
                if p.value { format!("{}true", ui::YES) } else { format!("{}false", ui::NO) },
                context_suffix(&p.context),
                expiry_suffix(e, p.expiry, now)
            );
            removable(
                e,
                Line::parse(&text),
                format!("{} permission {unset} {}{}", t.command(&e.root), p.node, context_args(&p.context)),
            )
        })
        .collect()
}

/// A prefix, suffix or display name in its own colours. In chat the text as
/// typed (with its `&` codes) is the tooltip; the console, which has no
/// tooltips, gets it in grey quotes after the value.
fn styled(value: &str, chat: bool) -> Line {
    let raw = format!("{}\"{}\"", ui::INFO, text::escape(value));
    if chat {
        Line::parse(&format!("{value}&r")).on_hover(Text::parse(&raw))
    } else {
        Line::parse(&format!("{value}&r {raw}"))
    }
}

fn parent_lines(e: &Engine, t: &Target, data: &Data, now: u64, chat: bool) -> Vec<Line> {
    data.parents
        .iter()
        .map(|p| {
            let shown = e.perms.display_name(&p.group);
            let mut line = Line::parse(&format!("    {}• {}{}", ui::MUTED, ui::VALUE, p.group));
            if shown != p.group {
                // The display name in its colours; the console keeps the text as typed.
                let name = if chat { styled(&shown, true) } else { Line::parse(&text::escape(&shown)) };
                line = line
                    .append(Line::parse(&format!(" {}(", ui::MUTED)))
                    .append(name)
                    .append(Line::parse(&format!("{})", ui::MUTED)));
            }
            let missing = if e.perms.group(&p.group).is_none() { format!(" {}✖", ui::ERROR) } else { String::new() };
            let tail = format!("{missing}{}{}", context_suffix(&p.context), expiry_suffix(e, p.expiry, now));
            removable(
                e,
                line.append(Line::parse(&tail)),
                format!("{} parent remove {}{}", t.command(&e.root), p.group, context_args(&p.context)),
            )
        })
        .collect()
}

fn meta_lines(e: &Engine, t: &Target, data: &Data, now: u64, chat: bool) -> Vec<Line> {
    let mut metas: Vec<_> = data.meta.iter().collect();
    metas.sort_by(|a, b| (a.kind as u8, -a.priority, &a.key).cmp(&(b.kind as u8, -b.priority, &b.key)));
    metas
        .into_iter()
        .map(|m| {
            let (label, remove) = match m.kind {
                MetaKind::Prefix => (
                    format!("{} {}", e.lang.get("kind-prefix"), m.priority),
                    format!("{} meta removeprefix {} {}", t.command(&e.root), m.priority, quoted(&m.value)),
                ),
                MetaKind::Suffix => (
                    format!("{} {}", e.lang.get("kind-suffix"), m.priority),
                    format!("{} meta removesuffix {} {}", t.command(&e.root), m.priority, quoted(&m.value)),
                ),
                MetaKind::Meta => (m.key.clone(), format!("{} meta unset {}", t.command(&e.root), quoted(&m.key))),
            };
            let head = format!("    {}• {}{} {}= ", ui::MUTED, ui::LABEL, text::escape(&label), ui::MUTED);
            let shown = match m.kind {
                MetaKind::Meta => Line::parse(&format!("{}{}", ui::VALUE, text::escape(&m.value))),
                _ => styled(&m.value, chat),
            };
            let tail = format!("{}{}", context_suffix(&m.context), expiry_suffix(e, m.expiry, now));
            let line = Line::parse(&head).append(shown).append(Line::parse(&tail));
            removable(e, line, format!("{remove}{}", context_args(&m.context)))
        })
        .collect()
}

/// A value as one [`split_args`] word.
pub(super) fn quoted(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Rows on one page of a view.
const PAGE: usize = 20;

/// The title `out` with page `page` of `rows` and a page bar that runs
/// `command <page>`; page 0 is every row.
fn paged(lang: &Lang, mut out: Text, rows: Vec<Line>, page: usize, command: &str) -> Text {
    if page == 0 {
        out.lines.extend(rows);
        return out;
    }
    let pages = rows.len().div_ceil(PAGE).max(1);
    let page = page.clamp(1, pages);
    out.lines.extend(rows.into_iter().skip((page - 1) * PAGE).take(PAGE));
    if pages > 1 {
        out.push(nav(lang, page, pages, command));
    }
    out
}

/// The page bar `━━━ ◀ 2/5 ▶ ━━━` across the chat after the prefix, in the
/// colours of `[PP]`; the arrows run `command <page>` and are dark grey on the
/// first and last page.
fn nav(lang: &Lang, page: usize, pages: usize, command: &str) -> Line {
    let page = page.min(pages);
    let arrow = |symbol: &str, target: usize, active: bool| {
        if active {
            Line::parse(&format!("{}&l{symbol}", ui::BRAND)).on_click(Click::Run(format!("{command} {target}")))
        } else {
            Line::parse(&format!("{}{symbol}", ui::MUTED))
        }
    };
    let centre = arrow("◀", page.saturating_sub(1), page > 1)
        .append(Line::parse(&format!(" {}{page}/{pages} ", ui::VALUE)))
        .append(arrow("▶", page + 1, page < pages));
    let width = pumbo_common::rich::CHAT_WIDTH.saturating_sub(style::prefix(lang).width());
    style::bar(centre, width, style::BRAND, style::accent::PERMS)
}

pub(super) fn section(out: &mut Vec<Line>, e: &Engine, label_key: &str, count: usize, lines: Vec<Line>) {
    out.push(Line::parse(&format!("  {}{} ({count})", ui::ACCENT, e.lang.get(label_key))));
    if lines.is_empty() {
        out.push(Line::parse(&format!("    {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    out.extend(lines);
}

pub(super) fn user_view(e: &mut Engine, t: &Target, page: usize, now: u64) -> Text {
    let Target::User { uuid, .. } = t else { return Text::new() };
    let q = e.query("");
    let eff = e.perms.effective(uuid, &q, now).clone();
    let data = t.data(e).cloned().unwrap_or_default();
    let out = ui::header(&e.lang, "title-user", &Args::new().arg(value(t.label())));
    let mut rows = vec![row(1, &e.lang.get("label-uuid"), uuid).on_click(Click::Suggest(uuid.clone()))];
    let rank = e.perms.display_name(&eff.primary);
    let rank_text =
        if rank == eff.primary { text::escape(&rank) } else { format!("{}&r {}({})", rank, ui::MUTED, eff.primary) };
    rows.push(row(1, &e.lang.get("label-rank"), &rank_text));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let chat = page != 0;
    let affix = |label: &str, v: &Option<String>| match v {
        Some(p) => row(1, label, "").append(styled(p, chat)),
        None => row(1, label, &none),
    };
    rows.push(affix(&e.lang.get("label-prefix"), &eff.prefix));
    rows.push(affix(&e.lang.get("label-suffix"), &eff.suffix));
    let inherited: Vec<String> = eff.groups.iter().map(|g| text::escape(g)).collect();
    rows.push(row(
        1,
        &e.lang.get("label-inherits"),
        &if inherited.is_empty() { none.clone() } else { inherited.join(", ") },
    ));
    let hidden = crate::host::hidden(e, uuid, &q, now);
    hidden_lines(e, hidden.iter(), &mut rows);
    section(&mut rows, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now, chat));
    section(&mut rows, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    section(&mut rows, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now, chat));
    paged(&e.lang, out, rows, page, &format!("{} info", t.command(&e.root)))
}

pub(super) fn group_view(e: &mut Engine, t: &Target, page: usize, now: u64) -> Text {
    let Target::Group { name } = t else { return Text::new() };
    let Some(group) = e.perms.group(name).cloned() else { return Text::new() };
    let out = ui::header(&e.lang, "title-group", &Args::new().arg(value(name)));
    let none = format!("{}{}", ui::MUTED, e.lang.get("label-none"));
    let chat = page != 0;
    let shown_label = e.lang.get("label-display-name");
    let shown = if group.display_name.is_empty() {
        row(1, &shown_label, &none)
    } else if chat {
        row(1, &shown_label, "").append(styled(&group.display_name, true))
    } else {
        row(1, &shown_label, &format!("{}&r", group.display_name))
    };
    let mut rows = vec![shown, row(1, &e.lang.get("label-weight"), &group.weight.to_string())];
    let members = e.perms.members(name).len();
    rows.push(
        row(1, &e.lang.get("label-members"), &members.to_string())
            .on_click(Click::Run(format!("{} group {name} listmembers", e.root)))
            .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-members")))),
    );
    let tracks: Vec<String> = e.perms.tracks_of(name).iter().map(|t| t.name.clone()).collect();
    rows.push(row(1, &e.lang.get("label-tracks"), &if tracks.is_empty() { none } else { tracks.join(", ") }));
    section(&mut rows, e, "label-parents", group.data.parents.len(), parent_lines(e, t, &group.data, now, chat));
    section(&mut rows, e, "label-permissions", group.data.permissions.len(), nodes_lines(e, t, &group.data, now));
    section(&mut rows, e, "label-meta", group.data.meta.len(), meta_lines(e, t, &group.data, now, chat));
    paged(&e.lang, out, rows, page, &format!("{} info", t.command(&e.root)))
}

fn holder_header(e: &Engine, t: &Target) -> Text {
    let key = if t.is_user() { "title-user" } else { "title-group" };
    ui::header(&e.lang, key, &Args::new().arg(value(t.label())))
}

pub(super) fn nodes_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-permissions", data.permissions.len(), nodes_lines(e, t, &data, now));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} permission info", t.command(&e.root)))
}

pub(super) fn parents_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-parents", data.parents.len(), parent_lines(e, t, &data, now, page != 0));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} parent info", t.command(&e.root)))
}

pub(super) fn meta_view(e: &Engine, t: &Target, page: usize, now: u64) -> Text {
    let data = t.data(e).cloned().unwrap_or_default();
    let mut rows = Vec::new();
    section(&mut rows, e, "label-meta", data.meta.len(), meta_lines(e, t, &data, now, page != 0));
    paged(&e.lang, holder_header(e, t), rows, page, &format!("{} meta info", t.command(&e.root)))
}

pub(super) fn groups_view(e: &Engine, page: usize) -> Text {
    let mut groups: Vec<_> = e.perms.groups().collect();
    groups.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.name.cmp(&b.name)));
    let out = ui::header(&e.lang, "title-groups", &Args::new().arg(value(groups.len())));
    let rows = groups
        .into_iter()
        .map(|g| {
            let shown = if g.display_name.is_empty() {
                String::new()
            } else {
                format!(" {}({}&r{})", ui::MUTED, g.display_name, ui::MUTED)
            };
            Line::parse(&format!(
                "  {}• {}{}{shown} {}{} {}{}",
                ui::MUTED,
                ui::VALUE,
                g.name,
                ui::LABEL,
                e.lang.get("label-weight"),
                ui::VALUE,
                g.weight
            ))
            .on_click(Click::Run(format!("{} group {} info", e.root, g.name)))
            .on_hover(Text::parse(&format!("{}{}", ui::INFO, e.lang.get("hover-info"))))
        })
        .collect();
    paged(&e.lang, out, rows, page, &format!("{} groups", e.root))
}

pub(super) fn tracks_view(e: &Engine, page: usize) -> Text {
    let tracks: Vec<_> = e.perms.tracks().collect();
    let out = ui::header(&e.lang, "title-tracks", &Args::new().arg(value(tracks.len())));
    let mut rows = Vec::new();
    if tracks.is_empty() {
        rows.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for t in tracks {
        let ladder = if t.groups.is_empty() {
            "-".to_string()
        } else {
            t.groups.join(&format!(" {}→ {}", ui::MUTED, ui::VALUE))
        };
        rows.push(
            Line::parse(&format!("  {}• {}{} {}: {}{ladder}", ui::MUTED, ui::VALUE, t.name, ui::MUTED, ui::VALUE))
                .on_click(Click::Run(format!("{} track {} info", e.root, t.name))),
        );
    }
    paged(&e.lang, out, rows, page, &format!("{} tracks", e.root))
}

pub(super) fn members_view(e: &Engine, group: &str, page: usize, _now: u64) -> Text {
    const PER: usize = 15;
    let members = e.perms.members(group);
    let children: Vec<String> = e.perms.child_groups(group).iter().map(|g| g.name.clone()).collect();
    let mut out = ui::header(&e.lang, "title-members", &Args::new().arg(value(group)).arg(value(members.len())));
    if !children.is_empty() {
        out.push(row(1, &e.lang.get("label-child-groups"), &children.join(", ")));
    }
    let pages = members.len().div_ceil(PER).max(1);
    let page = page.clamp(1, pages);
    if members.is_empty() {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
    }
    for u in members.iter().skip((page - 1) * PER).take(PER) {
        let t = Target::User { uuid: u.uuid.clone(), name: u.name.clone() };
        let uuid = if u.name.is_empty() || is_java_name(&u.name) {
            String::new()
        } else {
            format!(" {}({})", ui::MUTED, u.uuid)
        };
        out.push(
            Line::parse(&format!("  {}• {}{}{uuid}", ui::MUTED, ui::VALUE, text::escape(&t.label())))
                .on_click(Click::Run(format!("{} info", t.command(&e.root)))),
        );
    }
    if pages > 1 {
        out.push(nav(&e.lang, page, pages, &format!("{} group {group} listmembers", e.root)));
    }
    out
}

pub(super) fn log_view(e: &Engine, page: usize, now: u64) -> Text {
    let mut out = ui::header(&e.lang, "title-log", &Args::new());
    let Some(store) = e.store() else {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("label-none"))));
        return out;
    };
    let (entries, total) = match store.log_page(page, LOG_PAGE) {
        Ok(r) => r,
        Err(err) => return ui::error(&e.lang, "save-failed", &Args::new().arg(value(err))),
    };
    if entries.is_empty() {
        out.push(Line::parse(&format!("  {}{}", ui::MUTED, e.lang.get("log-empty"))));
    }
    for entry in &entries {
        out.push(log_line(e, entry, now));
    }
    let pages = total.div_ceil(LOG_PAGE).max(1);
    if pages > 1 {
        out.push(nav(&e.lang, page, pages, &format!("{} log", e.root)));
    }
    out
}

/// One log entry: `5m ago  Console » group vip  permission set fly`.
pub fn log_line(e: &Engine, entry: &LogEntry, now: u64) -> Line {
    let ago =
        e.lang.format("time-ago", &Args::new().arg(coarse(Duration::from_millis(now.saturating_sub(entry.time)))));
    Line::parse(&format!(
        "  {}{ago} {}{} {}» {}{} {} {}{}",
        ui::INFO,
        ui::VALUE,
        text::escape(&safe_name(&entry.actor)),
        ui::MUTED,
        ui::LABEL,
        text::escape(&entry.kind),
        text::escape(&entry.target),
        ui::VALUE,
        text::escape(&entry.action)
    ))
}

/// The notification for staff about a change.
pub fn notification(e: &Engine, entry: &LogEntry) -> Text {
    let a = Args::new()
        .arg(value(safe_name(&entry.actor)))
        .arg(value(format!("{} {}", entry.kind, entry.target)))
        .arg(value(&entry.action));
    ui::info(&e.lang, "notify", &a)
}
