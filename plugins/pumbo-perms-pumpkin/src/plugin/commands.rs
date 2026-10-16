//! `/pumboperms` and its short names such as `/pp`.

use std::path::Path;

use pumbo_common::clock::now_ms;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, Action, Env, Who, split_args};
use pumbo_perms_core::files;
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::ui::{self, Text, value};

use super::api::{
    self, Arg, ArgumentType, CommandError, CommandHandler, CommandNode, CommandSender, CommandSenderType,
    CommandSuggestion, CommandSuggestionHandler, CommandSuggestions, ConsumedArgs, Server, StringType,
    SuggestionRequest,
};
use super::state::{self, Rt, SenderKind};
use super::{push, settings};
use crate::suggest::{Node, Word};

/// Runs a complete command: the words of its path in the tree, with the
/// arguments Pumpkin read.
pub struct Pp {
    pub path: Vec<Word>,
}

impl CommandHandler for Pp {
    fn handle(&self, sender: CommandSender, server: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        let mut words = Vec::new();
        for word in &self.path {
            match word {
                Word::Fixed(w) => words.push(w.clone()),
                Word::Slot(name, rest) => {
                    let value = match args.get_value(name) {
                        Arg::Simple(s) | Arg::Msg(s) => s,
                        _ => String::new(),
                    };
                    if *rest {
                        words.extend(split_args(&value));
                    } else {
                        words.push(value);
                    }
                }
            }
        }
        run(&sender, &server, &words);
        Ok(1)
    }
}

/// The Pumpkin node of `n` with everything below it; `path` leads to `n`.
pub fn node(n: &Node, path: &mut Vec<Word>) -> CommandNode {
    path.push(n.word.clone());
    let mut built = match &n.word {
        Word::Fixed(w) => CommandNode::literal(w),
        Word::Slot(name, rest) => {
            let kind = if *rest { StringType::Greedy } else { StringType::Quotable };
            CommandNode::argument(name, &ArgumentType::String(kind)).suggest(Suggest)
        }
    };
    for child in &n.children {
        built = built.then(node(child, path));
    }
    // Every node runs, also before a command is complete: PumboPerms answers
    // with the usage or the help, as on PumboProx and in LuckPerms, where
    // Pumpkin would only say the command is unknown or incomplete.
    built = built.execute(Pp { path: path.clone() });
    path.pop();
    built
}

/// Who sent a command, read from the host before the state is taken; `None`
/// for command blocks, signs and other senders ([`state::who_for`]).
pub fn who_of(sender: &CommandSender) -> Option<Who> {
    let (kind, player) = match sender.get_command_sender_type() {
        CommandSenderType::Console => (SenderKind::Console, None),
        CommandSenderType::Rcon => (SenderKind::Rcon, None),
        CommandSenderType::Player(p) => (
            SenderKind::Player,
            Some((
                p.get_name(),
                api::uuid_string(&p.get_id()),
                api::level_of(p.get_permission_level()),
                p.get_world().get_name(),
            )),
        ),
        CommandSenderType::CommandBlock(_) => (SenderKind::CommandBlock, None),
        CommandSenderType::Dummy => (SenderKind::Dummy, None),
    };
    state::who_for(kind, player.as_ref().map(|(n, u, l, w)| (n.as_str(), u.as_str(), *l, w.as_str())))
}

fn run(sender: &CommandSender, server: &Server, args: &[String]) {
    let Some(who) = who_of(sender) else {
        let refused = state::with(|rt| ui::error(rt.lang(), "sender-refused", &Args::new()));
        api::reply(
            sender,
            &refused.unwrap_or_else(|| Text::parse("&cPumboPerms commands run only from players and the console.")),
        );
        return;
    };
    let now = now_ms();
    let result = state::with(|rt| {
        let mut plans = rt.due_expiry(now);
        // The sender's operator level as Pumpkin has it now.
        if let Some(uuid) = &who.uuid
            && let Some(o) = rt.online.get_mut(uuid)
            && o.op_level != who.op_level
        {
            o.op_level = who.op_level;
            if !plans.iter().any(|p| &p.uuid == uuid) {
                plans.extend(rt.plan_player(uuid, now));
            }
        }
        let rows = version_rows(rt);
        let out = {
            let Rt { engine, online, .. } = &mut *rt;
            let lookup = |name: &str| {
                online.iter().find(|(_, o)| o.name.eq_ignore_ascii_case(name)).map(|(uuid, o)| KnownName {
                    uuid: uuid.clone(),
                    name: o.name.clone(),
                    online: false,
                })
            };
            let env = Env { now, online: &lookup, version: env!("CARGO_PKG_VERSION"), version_rows: rows };
            command::run(engine, &who, args, &env)
        };
        let mut reply = out.reply;
        let mut changed = out.changed;
        let mut log = out.log;
        let mut reloaded = false;
        match out.action {
            Some(Action::Reload) => {
                reply = settings::reload(rt);
                reloaded = true;
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
        if reloaded {
            rt.refresh_catalog();
            plans = rt.plan_all(now);
        } else if changed {
            plans = rt.after_change(now);
        }
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
    push::apply_plans(server, None, plans, resend);
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

fn export(rt: &Rt, name: &str, now: u64) -> Result<Text, Text> {
    let path = files::export_to(&rt.engine, Path::new(&rt.dir), name, now)?;
    Ok(ui::success(rt.lang(), "exported", &Args::new().arg(value(path))))
}

fn import(rt: &mut Rt, who: &Who, name: &str, now: u64) -> command::Outcome {
    files::import_file(&mut rt.engine, Path::new(&rt.dir), who, name, now)
}

/// Tab completion of every argument, from the whole line after the command
/// name (whichever name was typed).
pub struct Suggest;

impl CommandSuggestionHandler for Suggest {
    fn suggest(&self, sender: CommandSender, server: Server, request: SuggestionRequest) -> CommandSuggestions {
        let typed = request.input.split_once(' ').map_or("", |(_, rest)| rest);
        let mut args = split_args(typed);
        if typed.is_empty() || typed.ends_with(' ') {
            args.push(String::new());
        }
        let Some(who) = who_of(&sender) else {
            return CommandSuggestions { start: request.start, length: 0, values: Vec::new() };
        };
        let online: Vec<KnownName> = server
            .get_all_players()
            .iter()
            .map(|p| KnownName { uuid: api::uuid_string(&p.get_id()), name: p.get_name(), online: false })
            .collect();
        let now = now_ms();
        let options =
            state::with(|rt| command::complete(&mut rt.engine, &who, &args, &online, now)).unwrap_or_default();
        // Pumpkin 0.2 replaces only the last word of the line.
        let (start, length) = crate::suggest::last_word(&request.input);
        CommandSuggestions {
            start: u32::try_from(start).unwrap_or(0),
            length: u32::try_from(length).unwrap_or(0),
            values: options.into_iter().map(|value| CommandSuggestion { value, tooltip: None }).collect(),
        }
    }
}
