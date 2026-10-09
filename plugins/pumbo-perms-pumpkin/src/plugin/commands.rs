//! `/pumboperms` and its alias `/pp`.

use pumbo_common::clock::now_ms;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, Action, Env, Who, split_args};
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::ui::{self, Text, value};

use super::api::{
    self, Arg, CommandError, CommandHandler, CommandSender, CommandSuggestion, CommandSuggestionHandler,
    CommandSuggestions, ConsumedArgs, Server, SuggestionRequest,
};
use super::state::{self, Rt};
use super::{push, settings};

/// Runs the command; `with_args` when the greedy argument is there.
pub struct Pp {
    pub with_args: bool,
}

impl CommandHandler for Pp {
    fn handle(&self, sender: CommandSender, server: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        let raw = if self.with_args {
            match args.get_value("args") {
                Arg::Simple(s) | Arg::Msg(s) => s,
                _ => String::new(),
            }
        } else {
            String::new()
        };
        run(&sender, &server, &raw);
        Ok(1)
    }
}

/// Who sent a command, read from the host before the state is taken.
pub fn who_of(sender: &CommandSender) -> Who {
    match sender.as_player() {
        Some(p) => Who::player(
            &p.get_name(),
            &api::uuid_string(&p.get_id()),
            api::level_of(p.get_permission_level()),
            &p.get_world().get_name(),
        ),
        None => Who::console(),
    }
}

fn run(sender: &CommandSender, server: &Server, raw: &str) {
    let who = who_of(sender);
    let args = split_args(raw);
    let now = now_ms();
    let result = state::with(|rt| {
        let rows = version_rows(rt);
        let out = {
            let Rt { engine, online, .. } = &mut *rt;
            let lookup = |name: &str| {
                online
                    .iter()
                    .find(|(_, o)| o.name.eq_ignore_ascii_case(name))
                    .map(|(uuid, o)| KnownName { uuid: uuid.clone(), name: o.name.clone() })
            };
            let env = Env { now, online: &lookup, version: env!("CARGO_PKG_VERSION"), version_rows: rows };
            command::run(engine, &who, &args, &env)
        };
        let mut reply = out.reply;
        let mut changed = out.changed;
        let mut log = out.log;
        match out.action {
            Some(Action::Reload) => {
                reply = settings::reload(rt);
                changed = true;
            }
            Some(Action::Export(name)) => reply = export(rt, &name, now).unwrap_or_else(|e| e),
            Some(Action::Import(name)) => {
                let o = import(rt, &who, &name, now);
                reply = o.reply;
                changed |= o.changed;
                log = o.log.or(log);
            }
            Some(Action::Source(source, mode)) => {
                let o = super::import::command(rt, server, &who, &source, mode, now);
                reply = o.reply;
                changed |= o.changed;
            }
            None => {}
        }
        let plans = if changed {
            rt.refresh_catalog();
            rt.plan_all(now)
        } else {
            Vec::new()
        };
        let mut notify = Vec::new();
        if let Some(entry) = &log
            && rt.cfg.log.notify
        {
            let text = command::notification(&rt.engine, entry);
            let staff: Vec<(String, String, u8, String)> =
                rt.online.iter().map(|(u, o)| (u.clone(), o.name.clone(), o.op_level, o.world.clone())).collect();
            for (uuid, name, op, world) in staff {
                if Some(&uuid) == who.uuid.as_ref() {
                    continue;
                }
                if command::allowed(&mut rt.engine, &Who::player(&name, &uuid, op, &world), "log.notify", now) {
                    notify.push((uuid, text.clone()));
                }
            }
            rt.logs.push((
                false,
                format!("PumboPerms: {} changed {} {}: {}", entry.actor, entry.kind, entry.target, entry.action),
            ));
        }
        (reply, plans, notify, rt.resend())
    });
    state::flush_logs();
    let Some((mut reply, plans, notify, resend)) = result else {
        api::reply(sender, &Text::parse("&cPumboPerms is busy, try again."));
        return;
    };
    if state::passive()
        && let Some(mut note) = state::with(|rt| ui::warn(rt.lang(), "proxy-rules", &Args::new()))
    {
        note.extend(reply);
        reply = note;
    }
    api::reply(sender, &reply);
    push::apply_all(server, plans, resend);
    for (uuid, text) in notify {
        if let Some(p) = api::parse_uuid(&uuid).and_then(|id| server.get_player_by_uuid(id)) {
            api::tell(&p, &text);
        }
    }
}

fn version_rows(rt: &Rt) -> Vec<(String, String)> {
    let l = rt.lang();
    let mode = if state::passive() {
        l.get("mode-proxy")
    } else if rt.check_event {
        l.get("mode-event")
    } else {
        l.get("mode-attachments")
    };
    let mut rows = vec![(l.get("version-platform"), api::API_LABEL.to_string()), (l.get("version-mode"), mode)];
    rows.push((l.get("version-online"), rt.online.len().to_string()));
    if rt.check_event {
        rows.push(("Checks".into(), format!("{} answered, {} skipped (busy)", rt.answered, super::events::skipped())));
    }
    rows
}

fn export(rt: &mut Rt, name: &str, now: u64) -> Result<Text, Text> {
    let dir = format!("{}/exports", rt.dir);
    let path = format!("{dir}/{name}.json");
    let json = pumbo_perms_core::export::dump(&rt.engine.perms, now);
    let result = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, json));
    match result {
        Ok(()) => Ok(ui::success(rt.lang(), "exported", &Args::new().arg(value(format!("exports/{name}.json"))))),
        Err(e) => Err(ui::error(rt.lang(), "export-failed", &Args::new().arg(value(e)))),
    }
}

fn import(rt: &mut Rt, who: &Who, name: &str, now: u64) -> command::Outcome {
    let path = format!("{}/exports/{name}.json", rt.dir);
    let json = match std::fs::read_to_string(&path) {
        Ok(j) => j,
        Err(_) => {
            let reply = ui::error(rt.lang(), "import-missing", &Args::new().arg(value(format!("exports/{name}.json"))));
            return command::Outcome { reply, ..command::Outcome::default() };
        }
    };
    let backup = format!("before-import-{}", pumbo_perms_core::export::default_file_name(now));
    if let Err(reply) = export(rt, &backup, now) {
        return command::Outcome { reply, ..command::Outcome::default() };
    }
    command::import(&mut rt.engine, who, name, &json, &format!("exports/{backup}.json"), now)
}

/// Tab completion of the greedy argument.
pub struct Suggest;

impl CommandSuggestionHandler for Suggest {
    fn suggest(&self, sender: CommandSender, server: Server, request: SuggestionRequest) -> CommandSuggestions {
        let typed = request.remaining.clone();
        let mut args = split_args(&typed);
        if typed.is_empty() || typed.ends_with(' ') {
            args.push(String::new());
        }
        let who = who_of(&sender);
        let online: Vec<String> = server.get_all_players().iter().map(|p| p.get_name()).collect();
        let now = now_ms();
        let options =
            state::with(|rt| command::complete(&mut rt.engine, &who, &args, &online, now)).unwrap_or_default();
        // Pumpkin replaces the whole argument, so every value repeats what comes before the last word.
        let head_len = if typed.ends_with(' ') { typed.len() } else { typed.rfind(' ').map(|i| i + 1).unwrap_or(0) };
        let head = typed.get(..head_len).unwrap_or("");
        CommandSuggestions {
            start: request.start,
            length: u32::try_from(typed.len()).unwrap_or(u32::MAX),
            values: options
                .into_iter()
                .map(|o| CommandSuggestion { value: format!("{head}{o}"), tooltip: None })
                .collect(),
        }
    }
}
