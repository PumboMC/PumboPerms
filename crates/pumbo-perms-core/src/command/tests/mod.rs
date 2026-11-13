//! The `/pp` command end to end, on an in-memory store.

use pumbo_common::lang::{Bundle, COMMON, Lang, check_bundle};
use pumbo_common::style;
use pumbo_common::text::{Color, Named};

use crate::LANG;
use crate::command::{self, Action, Env, SourceMode, Who, import_pending, import_source, split_args};
use crate::config::CoreCfg;
use crate::engine::Engine;
use crate::export;
use crate::perms::KnownName;
use crate::ui::{Click, Text};

mod basics;
mod completion;
mod import;
mod limits;
mod limits_props;
mod offline;
mod tracks;
mod views;

const STEVE: &str = "00000000-0000-0000-0000-00000000000a";
const ALEX: &str = "00000000-0000-0000-0000-00000000000b";

const PLATFORM: Bundle = Bundle { name: "test", files: &[("en", "prefix: \"&8[&#f28c28&lP&#ffd54f&lP&8] \"\n")] };

fn engine() -> Engine {
    let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
    let mut e = Engine::in_memory(CoreCfg::default(), lang);
    e.seen(STEVE, "Steve", true, 0).unwrap();
    e.seen(ALEX, "Alex", true, 0).unwrap();
    e
}

fn online(name: &str) -> Option<KnownName> {
    match name.to_lowercase().as_str() {
        "steve" => Some(KnownName { uuid: STEVE.into(), name: "Steve".into(), online: true }),
        _ => None,
    }
}

fn env(now: u64) -> Env<'static> {
    Env { now, online: &online, version: "0.1.0", version_rows: vec![("Platform".into(), "test".into())] }
}

/// Runs a command as the console, returns the plain reply.
fn console(e: &mut Engine, line: &str) -> String {
    run_as(e, &Who::console(), line, 1_000)
}

fn run_as(e: &mut Engine, who: &Who, line: &str, now: u64) -> String {
    let out = command::run(e, who, &split_args(line), &env(now));
    out.reply.plain()
}

fn steve() -> Who {
    Who::player("Steve", STEVE, 0, "world")
}

/// Every command a click in `t` runs or puts into the chat box, as `/pp` arguments.
fn clicks(t: &Text) -> Vec<Vec<String>> {
    t.lines
        .iter()
        .flat_map(|l| &l.segments)
        .filter_map(|s| match &s.click {
            Some(Click::Run(c) | Click::Suggest(c)) => c.strip_prefix("/pp ").map(split_args),
            _ => None,
        })
        .collect()
}
