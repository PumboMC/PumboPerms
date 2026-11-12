//! `/pp track`.

use pumbo_common::text::Args;

use crate::engine::Engine;
use crate::perms::PermsError;
use crate::ui::{self, Click, Line, row, value};

use super::edit::{perms_error, save};
use super::help::{track_help, unnamed_help};
use super::{Env, Outcome, Res, Who, allowed_prefix, lower, need, page_of, writable};

pub(super) fn track_cmd(e: &mut Engine, who: &Who, args: &[String], env: &Env) -> Res {
    let now = env.now;
    let Some(name) = args.first().map(|n| n.to_lowercase()) else {
        return unnamed_help(e, who, "track", now);
    };
    let sub = lower(args.get(1));
    let rest = args.get(2..).unwrap_or(&[]);
    let log = args.get(1..).map(|a| a.join(" ")).unwrap_or_default();
    let n = Args::new().arg(value(&name));
    match sub.as_str() {
        "" | "help" | "?" => {
            if !allowed_prefix(e, who, "track.", now) {
                return Err(ui::error(&e.lang, "command-no-permission", &Args::new()));
            }
            Ok(Outcome::reply(track_help(e, who, &name, page_of(rest), now)))
        }
        "info" | "i" => {
            need(e, who, "track.info", now)?;
            let Some(track) = e.perms.track(&name).cloned() else {
                return Err(perms_error(e, &PermsError::NoTrack(name)));
            };
            let mut out = ui::header(&e.lang, "title-track", &Args::new().arg(value(&name)));
            if track.groups.is_empty() {
                out.push(row(1, "", &format!("{}{}", ui::MUTED, e.lang.get("label-none"))));
            }
            for (i, g) in track.groups.iter().enumerate() {
                let shown = e.perms.display_name(g);
                let extra = if &shown == g { String::new() } else { format!(" {}({shown})", ui::MUTED) };
                out.push(
                    Line::parse(&format!("  {}{}. {}{g}{extra}", ui::MUTED, i + 1, ui::VALUE))
                        .on_click(Click::Suggest(format!("{} group {g} info", e.root))),
                );
            }
            Ok(Outcome::reply(out))
        }
        "create" => {
            need(e, who, "track.create", now)?;
            writable(e)?;
            let dirty = e.perms.create_track(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "track-created", &n);
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        "delete" => {
            need(e, who, "track.delete", now)?;
            writable(e)?;
            let dirty = e.perms.delete_track(&name).map_err(|err| perms_error(e, &err))?;
            let reply = ui::success(&e.lang, "track-deleted", &n);
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        "append" | "insert" | "remove" | "clear" => {
            need(e, who, "track.edit", now)?;
            writable(e)?;
            let group = rest.first().map(|g| g.to_lowercase());
            let (dirty, reply) = match (sub.as_str(), group) {
                ("clear", _) => (e.perms.track_clear(&name), ui::success(&e.lang, "track-cleared", &n)),
                ("append", Some(g)) => {
                    let d = e.perms.track_insert(&name, &g, None);
                    (d, ui::success(&e.lang, "track-added", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                ("insert", Some(g)) => {
                    let Some(pos) = rest.get(1).and_then(|p| p.parse::<usize>().ok()).filter(|p| *p >= 1) else {
                        return Err(ui::usage(&e.lang, &format!("{} track {name} insert <group> <position>", e.root)));
                    };
                    let d = e.perms.track_insert(&name, &g, Some(pos - 1));
                    (d, ui::success(&e.lang, "track-added", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                ("remove", Some(g)) => {
                    let d = e.perms.track_remove(&name, &g);
                    (d, ui::success(&e.lang, "track-removed", &Args::new().arg(value(&g)).arg(value(&name))))
                }
                (s, None) => return Err(ui::usage(&e.lang, &format!("{} track {name} {s} <group>", e.root))),
                _ => return Err(ui::unknown(&e.lang, &sub, &format!("{} help", e.root))),
            };
            let dirty = dirty.map_err(|err| perms_error(e, &err))?;
            save(e, who, dirty, ("track", &name, log), reply, now)
        }
        _ => Err(ui::unknown(&e.lang, &sub, &format!("{} track {name} help", e.root))),
    }
}
