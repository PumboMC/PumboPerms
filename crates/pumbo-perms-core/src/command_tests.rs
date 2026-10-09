//! The `/pp` command end to end, on an in-memory store.

use pumbo_common::lang::{Bundle, COMMON, Lang, check_bundle};

use crate::LANG;
use crate::command::{self, Action, Env, Who, split_args};
use crate::config::CoreCfg;
use crate::engine::Engine;
use crate::perms::KnownName;

const STEVE: &str = "00000000-0000-0000-0000-00000000000a";
const ALEX: &str = "00000000-0000-0000-0000-00000000000b";

const PLATFORM: Bundle = Bundle { name: "test", files: &[("en", "prefix: \"&#f28c28PumboPerms &8» \"\n")] };

fn engine() -> Engine {
    let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
    let mut e = Engine::in_memory(CoreCfg::default(), lang);
    e.seen(STEVE, "Steve").unwrap();
    e.seen(ALEX, "Alex").unwrap();
    e
}

fn online(name: &str) -> Option<KnownName> {
    match name.to_lowercase().as_str() {
        "steve" => Some(KnownName { uuid: STEVE.into(), name: "Steve".into() }),
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

#[test]
fn messages_are_complete_in_every_language() {
    assert_eq!(check_bundle(&LANG), Vec::<String>::new());
    let (pl, w) = Lang::load(&[COMMON, LANG], "pl", None);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(pl.get("help-section"), "Uprawnienia");
}

#[test]
fn every_message_key_used_exists() {
    let (l, _) = Lang::load(&[COMMON, LANG], "en", None);
    let src = include_str!("command.rs");
    let mut missing = Vec::new();
    for (i, _) in src.match_indices('"') {
        let rest = &src[i + 1..];
        let Some(end) = rest.find('"') else { continue };
        let word = &rest[..end];
        let looks_like_key = word.contains('-')
            && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            && [
                "perm-", "parent-", "meta-", "affix-", "group-", "track-", "bad-", "desc-", "label-", "title-",
                "hover-",
            ]
            .iter()
            .any(|p| word.starts_with(p));
        if looks_like_key && !l.has(word) {
            missing.push(word.to_string());
        }
    }
    missing.sort();
    missing.dedup();
    // keys built at run time: kind-prefix, desc-*-details are optional
    missing.retain(|k| k != "group-" && k != "track-");
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn quoting() {
    assert_eq!(split_args("a  \"b c\" d"), vec!["a", "b c", "d"]);
    assert_eq!(split_args("meta setprefix 10 \"&a[VIP] \""), vec!["meta", "setprefix", "10", "&a[VIP] "]);
    assert_eq!(split_args("x \"\" y"), vec!["x", "", "y"]);
    assert_eq!(split_args("\"say \\\"hi\\\"\""), vec!["say \"hi\""]);
    assert!(split_args("   ").is_empty());
}

#[test]
fn groups_permissions_and_inheritance_from_commands() {
    let mut e = engine();
    assert!(console(&mut e, "creategroup vip").contains("Group vip created."));
    assert!(console(&mut e, "group vip create").contains("already exists"));
    console(&mut e, "group admin create");
    assert!(
        console(&mut e, "group vip permission set minecraft:command.gamemode")
            .contains("Set minecraft:command.gamemode to true")
    );
    console(&mut e, "group admin parent add vip");
    console(&mut e, "group admin setweight 100");
    assert!(console(&mut e, "user Steve parent add admin").contains("Steve now inherits admin."));
    assert_eq!(e.decide(STEVE, "minecraft:command.gamemode", "world", 0, 1_000), Some(true));
    // a loop is refused
    assert!(console(&mut e, "group vip parent add admin").contains("loop"));
    // unknown groups and bad nodes
    assert!(console(&mut e, "user Steve parent add nope").contains("does not exist"));
    assert!(console(&mut e, "group vip permission set a..b").contains("empty part"));
    assert!(console(&mut e, "group vip permission set x maybe").contains("not true or false"));
    // check explains where the answer comes from
    let check = console(&mut e, "user Steve permission check minecraft:command.gamemode");
    assert!(check.contains("true") && check.contains("group vip"), "{check}");
    assert!(console(&mut e, "user Steve permission check other.node").contains("not set"));
    // the group check shows what a member gets
    assert!(console(&mut e, "group admin permission check minecraft:command.gamemode").contains("true"));
    // unset and clear
    assert!(console(&mut e, "group vip permission unset minecraft:command.gamemode").contains("Removed"));
    assert!(console(&mut e, "group vip permission unset minecraft:command.gamemode").contains("has no"));
    assert_eq!(e.decide(STEVE, "minecraft:command.gamemode", "world", 0, 1_000), None);
}

#[test]
fn data_survives_reloading_the_store() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "group vip permission set fly true world=nether");
    console(&mut e, "user Steve parent add vip");
    e.reload_data();
    assert_eq!(e.decide(STEVE, "fly", "nether", 0, 1_000), Some(true));
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 1_000), None);
}

#[test]
fn players_need_permissions_and_see_only_their_commands() {
    let mut e = engine();
    let p = steve();
    // nothing allowed: the help is empty and commands are refused
    assert!(run_as(&mut e, &p, "help", 0).contains("no commands you can use"));
    assert!(run_as(&mut e, &p, "creategroup x", 0).contains("permission"));
    assert!(run_as(&mut e, &p, "user Steve info", 0).contains("permission"));
    // operators of level 3 may do everything by default
    let op = Who::player("Steve", STEVE, 3, "world");
    assert!(run_as(&mut e, &op, "creategroup x", 0).contains("created"));
    // grant only user info
    console(&mut e, "user Steve permission set pumbo.perms.user.info");
    let help = run_as(&mut e, &p, "help", 0);
    assert!(help.contains("user <player>") && !help.contains("creategroup"), "{help}");
    assert!(run_as(&mut e, &p, "user Steve info", 0).contains("Rank"));
    assert!(run_as(&mut e, &p, "user Steve permission set x", 0).contains("permission"));
    // a wildcard over the plugin
    console(&mut e, "user Steve permission set pumbo.perms.*");
    assert!(run_as(&mut e, &p, "creategroup y", 0).contains("created"));
    // the Pumpkin namespace spelling is the same node
    console(&mut e, "user Steve permission set pumboperms:group.create false");
    assert!(run_as(&mut e, &p, "creategroup z", 0).contains("permission"));
    assert!(command::allowed_any(&mut e, &p, 0));
    assert!(!command::allowed_any(&mut e, &Who::player("Alex", ALEX, 0, "world"), 0));
}

#[test]
fn temporary_permissions_and_parents() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    let out = console(&mut e, "user Steve permission settemp fly true 10s");
    assert!(out.contains("for 10s"), "{out}");
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 5_000), Some(true));
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 11_000), None);
    assert!(console(&mut e, "user Steve permission settemp fly").contains("Usage"));
    assert!(console(&mut e, "user Steve permission settemp fly true nope").contains("Invalid time"));
    console(&mut e, "user Steve parent addtemp vip 1m");
    assert_eq!(e.decide(STEVE, "group.vip", "world", 0, 50_000), Some(true));
    assert_eq!(e.decide(STEVE, "group.vip", "world", 0, 62_000), None);
    assert_eq!(e.expire(62_000), Ok(true));
    assert!(e.perms.user(STEVE).is_none_or(|u| u.data.is_empty()));
}

#[test]
fn meta_prefix_and_views() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "group vip setdisplayname &6VIP");
    assert!(console(&mut e, "group vip meta setprefix 10 \"&6[VIP] \"").contains("Set the prefix"));
    console(&mut e, "group vip meta set color gold");
    console(&mut e, "user Steve parent set vip");
    let info = console(&mut e, "user Steve info");
    for want in ["Steve", "Rank: VIP (vip)", "[VIP]", "Parent groups (1)", "vip"] {
        assert!(info.contains(want), "{want} not in {info}");
    }
    let eff = e.perms.effective(STEVE, &e.query("world"), 1_000).clone();
    assert_eq!(eff.prefix.as_deref(), Some("&6[VIP] "));
    assert_eq!(eff.meta.get("color").map(String::as_str), Some("gold"));
    assert!(console(&mut e, "group vip meta removeprefix 10").contains("Removed 1"));
    assert!(console(&mut e, "group vip meta removeprefix 10").contains("has no prefix"));
    assert!(console(&mut e, "group vip meta unset color").contains("Removed"));
    let ginfo = console(&mut e, "group vip info");
    assert!(ginfo.contains("Members: 1") && ginfo.contains("Weight: 0"), "{ginfo}");
    assert!(console(&mut e, "group vip listmembers").contains("Steve"));
    assert!(console(&mut e, "groups").contains("vip"));
}

#[test]
fn tracks_promote_demote() {
    let mut e = engine();
    for g in ["member", "vip", "mvp"] {
        console(&mut e, &format!("creategroup {g}"));
    }
    console(&mut e, "createtrack ranks");
    for g in ["member", "vip", "mvp"] {
        assert!(console(&mut e, &format!("track ranks append {g}")).contains("Added"));
    }
    assert!(console(&mut e, "track ranks info").contains("1. member"));
    assert!(console(&mut e, "promote Steve ranks").contains("first group"));
    assert!(console(&mut e, "promote Steve ranks").contains("member → vip"));
    assert!(console(&mut e, "user Steve promote ranks").contains("vip → mvp"));
    assert!(console(&mut e, "promote Steve ranks").contains("highest"));
    assert!(console(&mut e, "demote Steve ranks").contains("mvp → vip"));
    assert!(console(&mut e, "demote Alex ranks").contains("not on track"));
    assert!(console(&mut e, "tracks").contains("member → vip → mvp"));
    assert!(console(&mut e, "track ranks insert admin 1").contains("does not exist"));
    console(&mut e, "creategroup admin");
    console(&mut e, "track ranks insert admin 1");
    assert!(console(&mut e, "track ranks info").contains("1. admin"));
}

#[test]
fn errors_and_help_are_friendly() {
    let mut e = engine();
    assert!(console(&mut e, "nonsense").contains("Unknown subcommand nonsense"));
    assert!(console(&mut e, "user Nobody info").contains("was not found"));
    assert!(console(&mut e, "group nope info").contains("does not exist"));
    assert!(console(&mut e, "user Steve permission set").contains("Usage"));
    assert!(console(&mut e, "deletegroup default").contains("cannot be deleted"));
    assert!(console(&mut e, "user Steve permission set a b c d").contains("Usage"));
    assert!(console(&mut e, "editor").contains("not available yet"));
    // console help: no colours, every command
    let help = console(&mut e, "help");
    assert!(!help.contains('&') && help.contains("/pp reload") && help.contains("/pp user <player>"));
    let sub = console(&mut e, "user Steve");
    assert!(sub.contains("/pp user Steve permission set <node>"), "{sub}");
    let perm_help = console(&mut e, "user Steve permission");
    assert!(perm_help.contains("settemp") && !perm_help.contains("meta setprefix"));
    // chat help has pages and clickable entries
    let op = Who::player("Steve", STEVE, 4, "world");
    let out = command::run(&mut e, &op, &split_args("help"), &env(0));
    assert!(out.reply.plain().contains("1/3"));
    assert!(
        out.reply
            .lines
            .iter()
            .skip(1)
            .take(8)
            .all(|l| l.segments.iter().all(|s| s.click.is_some() && s.hover.is_some()))
    );
}

#[test]
fn log_and_actions() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    let out = command::run(&mut e, &Who::console(), &split_args("group vip permission set fly"), &env(5_000));
    let entry = out.log.clone().unwrap();
    assert_eq!(
        (entry.kind.as_str(), entry.target.as_str(), entry.action.as_str()),
        ("group", "vip", "permission set fly")
    );
    assert!(out.changed);
    let log = run_as(&mut e, &Who::console(), "log", 65_000);
    assert!(log.contains("1m ago Console » group vip permission set fly"), "{log}");
    assert!(command::notification(&e, &entry).plain().contains("Console changed group vip: permission set fly"));
    let out = command::run(&mut e, &Who::console(), &split_args("reload"), &env(0));
    assert_eq!(out.action, Some(Action::Reload));
    let out = command::run(&mut e, &Who::console(), &split_args("export backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Export("backup1".into())));
    let out = command::run(&mut e, &Who::console(), &split_args("export ../x"), &env(0));
    assert!(out.action.is_none() && out.reply.plain().contains("not a valid file name"));
    let out = command::run(&mut e, &Who::console(), &split_args("import backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Import("backup1".into())));
}

#[test]
fn import_replaces_everything() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve parent add vip");
    let dump = crate::export::dump(&e.perms, 0);
    console(&mut e, "deletegroup vip");
    console(&mut e, "creategroup other");
    let out = command::import(&mut e, &Who::console(), "b", &dump, "before", 9);
    assert!(out.reply.plain().contains("Imported 2 groups, 1 users"), "{}", out.reply.plain());
    assert!(e.perms.group("vip").is_some() && e.perms.group("other").is_none());
    e.reload_data();
    assert_eq!(e.decide(STEVE, "group.vip", "", 0, 10), Some(true));
    let bad = command::import(&mut e, &Who::console(), "b", "{}", "before", 9);
    assert!(bad.reply.plain().contains("Import failed"));
    assert!(e.perms.group("vip").is_some(), "a failed import changes nothing");
}

#[test]
fn completion() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    let con = Who::console();
    let c = |e: &mut Engine, line: &str| {
        let mut args = split_args(line);
        if line.ends_with(' ') {
            args.push(String::new());
        }
        command::complete(e, &con, &args, &["Steve".to_string()], 0)
    };
    assert!(c(&mut e, "us").contains(&"user".to_string()));
    assert_eq!(c(&mut e, "user S"), vec!["Steve"]);
    assert!(c(&mut e, "user Steve ").contains(&"permission".to_string()));
    assert!(c(&mut e, "user Steve parent add ").contains(&"vip".to_string()));
    assert!(c(&mut e, "group ").contains(&"default".to_string()));
    assert!(c(&mut e, "user Steve permission set fly ").contains(&"true".to_string()));
    // a player without permissions gets no subcommands
    assert!(command::complete(&mut e, &Who::player("Alex", ALEX, 0, ""), &[String::new()], &[], 0) == vec!["help"]);
}

#[test]
fn broken_database_refuses_changes() {
    let lang = Lang::load(&[COMMON, LANG], "en", None).0;
    let mut e = Engine::new(Err(pumbo_common::store::StoreError::new("broken")), CoreCfg::default(), lang);
    assert!(console(&mut e, "creategroup vip").contains("database is not available"));
    assert!(console(&mut e, "version").contains("broken"));
}
