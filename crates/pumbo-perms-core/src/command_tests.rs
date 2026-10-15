//! The `/pp` command end to end, on an in-memory store.

use pumbo_common::lang::{Bundle, COMMON, Lang, check_bundle};

use crate::LANG;
use crate::command::{self, Action, Env, SourceMode, Who, import_pending, import_source, split_args};
use crate::config::CoreCfg;
use crate::engine::Engine;
use crate::export;
use crate::perms::KnownName;
use crate::ui::{Click, Text};

const STEVE: &str = "00000000-0000-0000-0000-00000000000a";
const ALEX: &str = "00000000-0000-0000-0000-00000000000b";

const PLATFORM: Bundle = Bundle { name: "test", files: &[("en", "prefix: \"&#f28c28PumboPerms &8» \"\n")] };

fn engine() -> Engine {
    let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
    let mut e = Engine::in_memory(CoreCfg::default(), lang);
    e.seen(STEVE, "Steve", true).unwrap();
    e.seen(ALEX, "Alex", true).unwrap();
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
    assert!(console(&mut e, "user Steve parent add admin").contains(&format!("Steve ({STEVE}) now inherits admin.")));
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
    assert_eq!(e.expire(62_000), (true, Ok(())));
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
        command::complete(e, &con, &args, &[KnownName { uuid: STEVE.into(), name: "Steve".into(), online: true }], 0)
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

#[test]
fn imported_names_do_not_reach_click_commands() {
    let mut e = engine();
    let json = format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"default"}},{{"name":"vip"}},{{"name":"admin"}}],
          "users":[{{"uuid":"{ALEX}","name":"Steve parent add admin","data":{{"parents":[{{"group":"vip"}}]}}}}]}}"#
    );
    command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    let out = command::run(&mut e, &Who::console(), &split_args("group vip listmembers"), &env(1_000));
    let runs = clicks(&out.reply);
    assert!(!runs.is_empty(), "{}", out.reply.plain());
    assert!(runs.iter().all(|c| !c.join(" ").contains("parent add")), "{runs:?}");
}

#[test]
fn import_contexts_are_canonical() {
    let mut e = engine();
    let snap = export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip","data":{"permissions":[
            {"node":"fly","value":false,"context":{"Server":"Survival"}},
            {"node":"fly","value":true},
            {"node":"build","value":true,"context":{"planet":"mars"}}]}}]}"#,
    )
    .unwrap();
    let nodes = &snap.groups[0].data.permissions;
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert!(nodes.iter().any(|p| p.node == "fly" && !p.value && p.context.get("server") == Some("survival")));
    assert!(snap.fixed.iter().any(|f| f.contains("build") && f.contains("planet")), "{:?}", snap.fixed);
    let json = serde_json::to_string(&snap).unwrap();
    command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    console(&mut e, "user Steve parent add vip");
    let q = crate::context::Query::new("Survival", "", "");
    assert_eq!(e.perms.check(STEVE, "fly", &q, 0), Some(false), "the deny works on Survival");
    assert_eq!(e.perms.check(STEVE, "fly", &crate::context::Query::default(), 0), Some(true));
}

#[test]
fn import_drops_parent_without_group() {
    let snap = export::parse(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","data":{{"parents":[{{"group":"ghost"}}]}}}}],
          "users":[{{"uuid":"{STEVE}","name":"Steve","data":{{"parents":[{{"group":"vip"}},{{"group":"default"}},{{"group":"ghost"}}]}}}}]}}"#
    ))
    .unwrap();
    assert!(snap.groups[0].data.parents.is_empty());
    let parents: Vec<&str> = snap.users[0].data.parents.iter().map(|p| p.group.as_str()).collect();
    assert_eq!(parents, vec!["vip", "default"]);
    assert_eq!(snap.fixed.iter().filter(|f| f.contains("parent ghost")).count(), 2, "{:?}", snap.fixed);
}

#[test]
fn import_drops_duplicates_and_control_chars() {
    let mut e = engine();
    let long = "x".repeat(300);
    let json = format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","display_name":"VIP\n[Owner] Steve","data":{{
            "permissions":[{{"node":"fly","value":true}},{{"node":"fly","value":false}}],
            "parents":[{{"group":"default"}},{{"group":"default"}}],
            "meta":[{{"kind":"prefix","priority":10,"value":"&6[VIP]\n&cfake line"}},
                    {{"kind":"meta","key":"note","value":"{long}"}},
                    {{"kind":"meta","key":"a\nfake","value":"v"}},
                    {{"kind":"suffix","priority":1,"value":" &7*"}}]}}}}]}}"#
    );
    let snap = export::parse(&json).unwrap();
    let g = &snap.groups[0];
    assert_eq!((g.data.permissions.len(), g.data.permissions[0].value), (1, false), "the last one wins");
    assert_eq!(g.data.parents.len(), 1);
    assert_eq!(g.data.meta.len(), 1, "{:?}", g.data.meta);
    assert!(g.display_name.is_empty());
    assert_eq!(snap.fixed.len(), 6, "{:?}", snap.fixed);
    let out = command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    assert!(out.reply.plain().contains("6 entries of the file were fixed or left out"), "{}", out.reply.plain());
    // Commands refuse the same texts.
    assert!(console(&mut e, "group vip setdisplayname a\nb").contains("control characters"));
    assert!(console(&mut e, &format!("group vip meta set note {long}")).contains("too long"));
    assert!(console(&mut e, "group vip meta addprefix 5 \"a\u{7}\"").contains("control characters"));
    assert!(console(&mut e, "group vip meta set a\nFAKE v").contains("control characters"));
    assert!(console(&mut e, "group vip meta unset a\tb").contains("control characters"));
    assert!(console(&mut e, "group vip meta set \"a b\" v").contains("meta set <key>"));
}

#[test]
fn stored_values_render_as_plain_text() {
    let mut e = engine();
    let markup = r#"<click:run_command:'/pp user x parent add admin'>"[VIP]"#;
    console(&mut e, &format!("user Steve meta setprefix 10 \"{}\"", markup.replace('"', "\\\"")));
    let out = command::run(&mut e, &Who::console(), &split_args("user Steve info"), &env(0));
    assert!(out.reply.plain().contains(markup), "{}", out.reply.plain());
    for c in clicks(&out.reply) {
        assert!(!c.windows(2).any(|w| w == ["parent", "add"]), "{c:?}");
    }
    // The click that removes the prefix removes exactly that prefix.
    let remove = clicks(&out.reply).into_iter().find(|c| c.iter().any(|w| w == "removeprefix")).unwrap();
    assert!(command::run(&mut e, &Who::console(), &remove, &env(0)).reply.plain().contains("Removed 1"));
}

#[test]
fn nbsp_name_completes_as_one_word() {
    let mut e = engine();
    let u2 = "00000000-0000-0000-0000-0000000000c2";
    e.seen(u2, "Owner\u{a0}clear", false).unwrap();
    let online = [KnownName { uuid: u2.into(), name: "Owner\u{a0}clear".into(), online: false }];
    let got = command::complete(&mut e, &Who::console(), &["user".into(), String::new()], &online, 0);
    assert_eq!(got, vec![u2.to_string()]);
    assert_eq!(split_args(&format!("user {} ", got[0])).len(), 2);
    assert_eq!(split_args("user Owner\u{a0}clear info"), vec!["user", "Owner\u{a0}clear", "info"]);
}

#[test]
fn click_from_members_view_runs_only_info() {
    let mut e = engine();
    let (owner, attacker) = ("00000000-0000-0000-0000-0000000000c1", "00000000-0000-0000-0000-0000000000c2");
    e.seen(owner, "Owner", false).unwrap();
    e.seen(attacker, "Owner\u{a0}clear", false).unwrap();
    console(&mut e, "creategroup admin");
    console(&mut e, "creategroup vip");
    console(&mut e, "user Owner parent add admin");
    console(&mut e, &format!("user {attacker} parent add vip"));
    let out = command::run(&mut e, &Who::console(), &split_args("group vip listmembers"), &env(0));
    assert!(out.reply.plain().contains("Owner\\u{a0}clear"), "{}", out.reply.plain());
    let runs = clicks(&out.reply);
    assert!(!runs.is_empty());
    for c in runs {
        command::run(&mut e, &Who::console(), &c, &env(0));
    }
    assert!(e.perms.user(owner).unwrap().data.parents.iter().any(|p| p.group == "admin"));
}

#[test]
fn clear_refuses_extra_arguments() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve parent add vip");
    console(&mut e, "user Steve meta set color gold");
    assert!(console(&mut e, "user Steve clear info").contains("Usage"));
    assert!(console(&mut e, "user Steve info extra").contains("Usage"));
    assert!(console(&mut e, "user Steve meta unset color extra").contains("Usage"));
    let data = &e.perms.user(STEVE).unwrap().data;
    assert_eq!((data.parents.len(), data.meta.len()), (1, 1));
}

#[test]
fn pending_import_binds_only_online() {
    let mut e = engine();
    let snap = export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
            "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
    )
    .unwrap();
    import_source(&mut e, &Who::console(), "file", Ok(("f".into(), snap)), SourceMode::Run, 1);
    let (offline, online) = ("00000000-0000-0000-0000-0000000000d1", "00000000-0000-0000-0000-0000000000d2");
    let out = import_pending(&mut e, "file", offline, "Mod1", false, 2).unwrap();
    assert!(!out.changed && out.reply.plain().contains("offline-mode"), "{}", out.reply.plain());
    assert!(e.perms.user(offline).is_none_or(|u| u.data.is_empty()));
    assert_eq!(e.ledger("file").unwrap().pending.len(), 1, "still waiting");
    let out = import_pending(&mut e, "file", online, "Mod1", true, 3).unwrap();
    assert!(out.changed);
    assert!(e.perms.user(online).unwrap().data.parents.iter().any(|p| p.group == "mod"));
    assert!(e.ledger("file").unwrap().pending.is_empty());
}

/// A source with `Mod1` in `mod` by nickname only.
fn mod1_source() -> export::Snapshot {
    export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
            "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
    )
    .unwrap()
}

fn in_mod(e: &Engine, uuid: &str) -> bool {
    e.perms.user(uuid).is_some_and(|u| u.data.parents.iter().any(|p| p.group == "mod"))
}

#[test]
fn reimport_keeps_first_resolved_uuid() {
    let mut e = engine();
    let (first, second) = ("00000000-0000-0000-0000-0000000000d1", "00000000-0000-0000-0000-0000000000d2");
    e.seen(first, "Mod1", true).unwrap();
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    assert!(out.reply.plain().contains(&format!("Mod1 -> {first}")), "{}", out.reply.plain());
    assert!(in_mod(&e, first));
    // The name goes to another account; the source is imported again.
    e.seen(second, "Mod1", true).unwrap();
    let mut changed = mod1_source();
    changed.groups[0].weight = 5;
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), changed)), SourceMode::Run, 2);
    assert!(out.reply.plain().contains(&format!("Mod1 -> {first}")), "{}", out.reply.plain());
    assert!(in_mod(&e, first) && !in_mod(&e, second));
    let (log, _) = e.store().unwrap().log_page(1, 1).unwrap();
    assert!(log[0].action.contains(&format!("Mod1 -> {first}")), "{:?}", log[0]);
}

#[test]
fn offline_name_does_not_resolve_import() {
    let mut e = engine();
    let offline = "00000000-0000-0000-0000-0000000000d1";
    e.seen(offline, "Mod1", false).unwrap();
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    assert!(out.reply.plain().contains("known only by name"), "{}", out.reply.plain());
    assert!(!in_mod(&e, offline));
    assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
}

#[test]
fn pending_on_join_same_on_both_adapters() {
    let player = "00000000-0000-0000-0000-0000000000d1";
    for names_checked in [true, false] {
        let mut e = engine();
        e.cfg.names_checked = names_checked;
        import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
        // Pumpkin always reports the account as offline; the proxy as Mojang says.
        let out = command::on_join(&mut e, player, "Mod1", false, 2);
        assert!(out.len() == 1 && !out[0].changed && !in_mod(&e, player), "{out:?}");
        assert!(out[0].reply.plain().contains("got nothing"), "{:?}", out[0].reply.plain());
        assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
    }
    let mut e = engine();
    import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    let out = command::on_join(&mut e, player, "Mod1", true, 2);
    assert!(out.len() == 1 && out[0].changed && in_mod(&e, player), "{out:?}");
    assert!(command::on_join(&mut e, player, "Mod1", true, 3).is_empty(), "nothing waits any more");
}

#[test]
fn pumpkin_import_reply_names_players_who_get_nothing() {
    let mut e = engine();
    e.cfg.names_checked = false;
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    let reply = out.reply.plain();
    assert!(reply.contains("get nothing here; give them their ranks with /pp: Mod1"), "{reply}");
    assert!(!reply.contains("premium"), "{reply}");
}

/// `ranks`: member, vip, mvp; `vip` temporary for Steve when `until` is set.
fn ranks(e: &mut Engine, first: &str, until: Option<&str>) {
    for g in [first, "vip", "mvp"] {
        console(e, &format!("creategroup {g}"));
    }
    console(e, "createtrack ranks");
    for g in [first, "vip", "mvp"] {
        console(e, &format!("track ranks append {g}"));
    }
    if let Some(d) = until {
        console(e, &format!("user Steve parent addtemp vip {d}"));
    }
}

#[test]
fn promote_keeps_expiry() {
    let mut e = engine();
    ranks(&mut e, "member", Some("30d"));
    let end = e.perms.user(STEVE).unwrap().data.parents[0].expiry.unwrap();
    let out = console(&mut e, "promote Steve ranks");
    assert!(out.contains("vip → mvp") && out.contains("expires in 30d"), "{out}");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert_eq!(parents.len(), 1, "{parents:?}");
    assert_eq!((parents[0].group.as_str(), parents[0].expiry), ("mvp", Some(end)));
}

#[test]
fn promote_skips_expired_rung() {
    let mut e = engine();
    ranks(&mut e, "member", Some("1h"));
    // An hour later the temporary vip is over: Steve starts from the bottom.
    let later = 1_000 + 3_600_000;
    let out = run_as(&mut e, &Who::console(), "promote Steve ranks", later);
    assert!(out.contains("is now member"), "{out}");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert!(parents.iter().any(|p| p.group == "member" && p.expiry.is_none()), "{parents:?}");
}

#[test]
fn promote_from_default_once() {
    let mut e = engine();
    ranks(&mut e, "default", None);
    let out = console(&mut e, "promote Steve ranks");
    assert!(out.contains("default → vip"), "{out}");
    assert!(e.perms.user(STEVE).unwrap().data.parents.iter().any(|p| p.group == "vip"));
}

#[test]
fn import_file_needs_console() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let out = command::run(&mut e, &op, &split_args("import backup1"), &env(0));
    assert!(out.action.is_none() && out.reply.plain().contains("Only the console"), "{}", out.reply.plain());
    let out = command::run(&mut e, &op, &split_args("import preview file"), &env(0));
    assert_eq!(out.action, Some(Action::Source("file".into(), SourceMode::Preview)));
    let out = command::run(&mut e, &Who::console(), &split_args("import backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Import("backup1".into())));
}

#[test]
fn user_replies_show_uuid() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    assert!(console(&mut e, "user steve parent add vip").contains(&format!("Steve ({STEVE})")));
    assert!(console(&mut e, "user Steve permission set fly").contains(&format!("Steve ({STEVE})")));
    let nameless = "00000000-0000-0000-0000-0000000000e1";
    let out = console(&mut e, &format!("user {nameless} parent add vip"));
    assert_eq!(out.matches(nameless).count(), 1, "{out}");
}

#[test]
fn group_node_cannot_be_set_as_permission() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    for line in [
        "user Steve permission set group.vip true",
        "user Steve permission settemp group.vip 1h",
        "user Steve permission unset group.vip",
        "group vip permission set group.admin",
    ] {
        let out = console(&mut e, line);
        assert!(out.contains("is a group, not a permission") && out.contains("parent add"), "{line}: {out}");
    }
    assert!(e.perms.user(STEVE).is_none_or(|u| u.data.permissions.is_empty()));
    let json = r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip","data":{"permissions":[{"node":"group.admin","value":true}]}}],"users":[],"tracks":[]}"#;
    let snap = export::parse(json).unwrap();
    assert!(snap.groups[0].data.permissions.is_empty());
    assert!(
        snap.fixed.iter().any(|f| f.contains("group.admin left out") && f.contains("parent add admin")),
        "{:?}",
        snap.fixed
    );
}

#[test]
fn dropped_wildcard_shows_in_permission_check() {
    let mut e = engine();
    console(&mut e, "creategroup mod");
    console(&mut e, "group mod permission set pumbo.bans.* true");
    console(&mut e, "group mod permission set pumbo.bans.exempt.* false");
    console(&mut e, "user Steve parent add mod");
    let line = "not sent to Pumpkin: pumbobans:* hides pumbobans:exempt.*";
    let out = console(&mut e, "user Steve permission check pumbo.bans.limit.helper");
    assert!(out.contains(line), "{out}");
    assert!(console(&mut e, "user Steve info").contains(line));
    let out = console(&mut e, "user Steve permission check minecraft:command.op");
    assert!(!out.contains("not sent"), "only wildcards over the node: {out}");
}

/// Re-imports, storage errors and subcommands that take only temporary entries.
mod storage_and_import {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pumbo_common::store::{FaultyBackend, Store};

    use super::*;
    use crate::store::PermsStore;

    struct Faults {
        reads: Arc<AtomicU32>,
        writes: Arc<AtomicU32>,
    }

    fn faulty_engine() -> (Engine, Faults) {
        let backend = FaultyBackend::default();
        let faults = Faults { reads: backend.fail_reads.clone(), writes: backend.fail_writes.clone() };
        let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
        let mut e = Engine::new(PermsStore::new(Store::new(backend)), CoreCfg::default(), lang);
        e.seen(STEVE, "Steve", true).unwrap();
        (e, faults)
    }

    /// A source with helper, builder (weight 10) and mod, each with a node.
    fn ranks_source(extra: &str) -> export::Snapshot {
        export::parse(&format!(
            r#"{{"format":"pumboperms","version":1,"groups":[
                {{"name":"helper","data":{{"permissions":[{{"node":"a.helper","value":true}}]}}}},
                {{"name":"builder","weight":10,"data":{{"permissions":[{{"node":"a.builder","value":true}}]}}}},
                {{"name":"mod","data":{{"permissions":[{{"node":"a.mod","value":true}}{extra}]}}}}],
              "users":[{{"uuid":"{STEVE}","name":"Steve","data":{{"parents":[{{"group":"helper"}}]}}}}]}}"#
        ))
        .unwrap()
    }

    fn import(e: &mut Engine, fingerprint: &str, snap: export::Snapshot) {
        let out = import_source(e, &Who::console(), "file", Ok((fingerprint.into(), snap)), SourceMode::Run, 1_000);
        assert!(out.changed, "import did not run: {}", out.reply.plain());
    }

    #[test]
    fn reimport_keeps_tracks_inheritance_and_weight() {
        let mut e = engine();
        import(&mut e, "f1", ranks_source(""));
        console(&mut e, "createtrack ranks");
        for g in ["helper", "builder", "mod"] {
            console(&mut e, &format!("track ranks append {g}"));
        }
        console(&mut e, "creategroup trial");
        console(&mut e, "group trial parent add builder");
        console(&mut e, "group builder setweight 20");
        assert_eq!(e.perms.track("ranks").unwrap().groups.len(), 3);
        import(&mut e, "f2", ranks_source(r#",{"node":"a.new","value":true}"#));
        let track = e.perms.track("ranks").unwrap().groups.clone();
        let trial_inherits = e.perms.group("trial").unwrap().data.parents.iter().any(|p| p.group == "builder");
        let weight = e.perms.group("builder").unwrap().weight;
        assert_eq!((track, trial_inherits, weight), (vec!["helper".into(), "builder".into(), "mod".into()], true, 20));
    }

    #[test]
    fn reimport_keeps_admin_temporary_parent() {
        let mut e = engine();
        import(&mut e, "f1", ranks_source(""));
        console(&mut e, "user Steve parent addtemp helper 1d");
        assert_eq!(e.perms.user(STEVE).unwrap().data.parents.len(), 2);
        import(&mut e, "f2", ranks_source(r#",{"node":"a.new","value":true}"#));
        let parents = &e.perms.user(STEVE).unwrap().data.parents;
        assert!(parents.iter().any(|p| p.group == "helper" && p.expiry.is_some()), "{parents:?}");
        assert!(parents.iter().any(|p| p.group == "helper" && p.expiry.is_none()), "{parents:?}");
    }

    #[test]
    fn import_log_cut_on_char_boundary() {
        for n in [600, 601] {
            let mut e = engine();
            console(&mut e, "user Steve permission set fly false");
            let snap = export::parse(&format!(
                r#"{{"format":"pumboperms","version":1,"users":[{{"uuid":"{STEVE}","name":"{}",
                  "data":{{"permissions":[{{"node":"fly","value":true}}]}}}}]}}"#,
                "ż".repeat(n)
            ))
            .unwrap();
            import(&mut e, "f1", snap);
            let (log, _) = e.store().unwrap().log_page(1, 1).unwrap();
            assert!(log[0].action.len() <= 1000, "{n}: {} bytes", log[0].action.len());
        }
    }

    #[test]
    fn noop_command_on_read_error_does_not_break_engine() {
        let (mut e, faults) = faulty_engine();
        faults.reads.store(u32::MAX, Ordering::SeqCst);
        let out = console(&mut e, "user Steve permission unset typo.node");
        faults.reads.store(0, Ordering::SeqCst);
        assert!(out.contains("has no"), "{out}");
        assert_eq!(e.broken, None);
    }

    #[test]
    fn failed_commit_and_reload_restore_memory() {
        let (mut e, faults) = faulty_engine();
        console(&mut e, "creategroup vip");
        faults.reads.store(u32::MAX, Ordering::SeqCst);
        faults.writes.store(u32::MAX, Ordering::SeqCst);
        console(&mut e, "group vip permission set fly");
        assert!(e.perms.group("vip").unwrap().data.permissions.is_empty(), "memory shows what is not on disk");
    }

    #[test]
    fn parent_removetemp_keeps_permanent_parent() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        console(&mut e, "user Steve parent add vip");
        console(&mut e, "user Steve parent addtemp vip 7d");
        assert_eq!(e.perms.user(STEVE).unwrap().data.parents.len(), 2);
        console(&mut e, "user Steve parent removetemp vip");
        let parents = &e.perms.user(STEVE).unwrap().data.parents;
        assert!(parents.iter().any(|p| p.group == "vip" && p.expiry.is_none()), "{parents:?}");
    }

    #[test]
    fn meta_removetempprefix_keeps_permanent_prefix() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        console(&mut e, "group vip meta addprefix 10 \"&6[VIP] \"");
        console(&mut e, "group vip meta addtempprefix 10 \"&6[VIP] \" 7d");
        assert_eq!(e.perms.group("vip").unwrap().data.meta.len(), 2);
        console(&mut e, "group vip meta removetempprefix 10");
        let meta = &e.perms.group("vip").unwrap().data.meta;
        assert!(meta.iter().any(|m| m.value == "&6[VIP] " && m.expiry.is_none()), "{meta:?}");
    }

    #[test]
    fn meta_unknown_affix_subcommand_is_refused() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        let out = console(&mut e, "group vip meta addfooprefix 10 x");
        assert!(out.contains("Unknown subcommand"), "{out}");
        assert!(e.perms.group("vip").unwrap().data.meta.is_empty());
    }

    #[test]
    fn reimport_keeps_parent_removed_from_player_bound_at_join() {
        let jeb = "00000000-0000-0000-0000-0000000000e1";
        let source = |weight: i32| {
            let mut snap = export::parse(
                r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
                    "by-name":[{"name":"jeb_","data":{"parents":[{"group":"mod"}]}}]}"#,
            )
            .unwrap();
            snap.groups[0].weight = weight;
            snap
        };
        let mut e = engine();
        import(&mut e, "f1", source(0));
        e.seen(jeb, "Jeb_", true).unwrap();
        assert!(command::on_join(&mut e, jeb, "Jeb_", true, 2).iter().any(|o| o.changed));
        console(&mut e, "user Jeb_ parent remove mod");
        assert!(!in_mod(&e, jeb));
        let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), source(5))), SourceMode::Run, 3);
        assert!(!in_mod(&e, jeb), "{}", out.reply.plain());
        assert!(!out.reply.plain().contains("taken back"), "{}", out.reply.plain());
    }

    #[test]
    fn reimport_adds_no_parent_in_deleted_group() {
        let source = |members: &[(&str, &str)]| {
            let users: Vec<String> = members
                .iter()
                .map(|(uuid, name)| {
                    format!(r#"{{"uuid":"{uuid}","name":"{name}","data":{{"parents":[{{"group":"mod"}}]}}}}"#)
                })
                .collect();
            export::parse(&format!(
                r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"mod"}},{{"name":"vip"}}],"users":[{}]}}"#,
                users.join(",")
            ))
            .unwrap()
        };
        let mut e = engine();
        import(&mut e, "f1", source(&[(STEVE, "Steve")]));
        console(&mut e, "deletegroup mod");
        assert!(e.perms.group("mod").is_none());
        import(&mut e, "f2", source(&[(STEVE, "Steve"), (ALEX, "Alex")]));
        assert!(e.perms.group("mod").is_none() && !in_mod(&e, ALEX) && !in_mod(&e, STEVE));
    }

    #[test]
    fn damaged_ledger_reported_once_at_joins() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(crate::store::IMPORTS, "file", b"{broken")).unwrap();
        let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
        let mut e = Engine::new(PermsStore::new(store), CoreCfg::default(), lang);
        let first = command::on_join(&mut e, STEVE, "Steve", true, 1);
        assert!(first.len() == 1 && first[0].reply.plain().contains("damaged"), "{first:?}");
        assert!(command::on_join(&mut e, ALEX, "Alex", true, 2).is_empty());
    }
}

/// What a user gets at a place, comparable.
fn decided(p: &mut crate::perms::Perms, uuid: &str, q: &crate::context::Query, now: u64) -> String {
    let eff = p.effective(uuid, q, now);
    let mut nodes: Vec<(&str, bool)> = eff.nodes().collect();
    nodes.sort();
    format!("{nodes:?} {:?} {} {:?} {:?} {:?}", eff.groups, eff.primary, eff.prefix, eff.suffix, eff.meta)
}

/// Decisions are kept per user and dropped only for the users a change can
/// reach: after every kind of change the kept answer equals a fresh one.
#[test]
fn cache_follows_group_change() {
    use crate::context::Query;
    let mut e = engine();
    const NEW: &str = "00000000-0000-0000-0000-00000000000c";
    const SCOPED: &str = "00000000-0000-0000-0000-00000000000d";
    e.seen(NEW, "Newbie", true).unwrap();
    e.seen(SCOPED, "Scoped", true).unwrap();
    let places = [Query::default(), Query::new("lobby", "hubs", "world"), Query::new("survival", "", "nether")];
    let users = [STEVE, ALEX, NEW, SCOPED];
    let steps = [
        "creategroup vip",
        "creategroup mod",
        "group vip permission set a.b",
        "user Steve parent add vip",
        "user Alex parent add mod",
        "user Scoped parent add vip server=lobby",
        "group mod parent add vip",
        "group vip permission set a.* false server=lobby",
        "group default permission set d.x",
        "user Newbie parent add later",
        "creategroup later",
        "group later permission set l.x",
        "group mod parent add later",
        "group vip meta setprefix 10 [V]",
        "group mod meta addprefix 20 [M]",
        "group vip meta set color gold",
        "group mod setweight 5",
        "group vip setweight 10",
        "user Steve permission set fly true world=nether",
        "group vip permission settemp t.x true 1m",
        "user Alex parent addtemp vip 1m",
        "createtrack ranks",
        "track ranks append vip",
        "track ranks append mod",
        "user Newbie promote ranks",
        "group vip rename vip2",
        "group mod parent remove later",
        "group default permission unset d.x",
        "group later permission set group.vip2 false",
        "deletegroup later",
        "user Steve parent remove vip2",
        "group vip2 permission clear",
        "deletegroup mod",
        "user Alex clear",
        "creategroup base",
        "group default parent add base",
        "group base permission set b.x",
        "group base permission settemp b.t true 1m",
    ];
    for (i, step) in steps.iter().enumerate() {
        let now = 1_000 + i as u64;
        for u in users {
            for q in &places {
                e.perms.effective(u, q, now);
            }
        }
        let reply = console(&mut e, step);
        // A copy starts without kept decisions.
        let mut fresh = e.perms.clone();
        for u in users {
            for q in &places {
                assert_eq!(
                    decided(&mut e.perms, u, q, now),
                    decided(&mut fresh, u, q, now),
                    "after `{step}` ({reply}): {u} at {q:?}"
                );
            }
        }
    }
    // A temporary node ends without a change: the kept answer ends with it.
    assert_eq!(e.perms.check(ALEX, "t.x", &places[0], 70_000), None);
    assert_eq!(e.perms.check(SCOPED, "b.t", &places[0], 2_000), Some(true));
    assert_eq!(e.perms.check(SCOPED, "b.t", &places[0], 70_000), None);
}
