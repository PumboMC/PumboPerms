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

const STEVE: &str = "00000000-0000-0000-0000-00000000000a";
const ALEX: &str = "00000000-0000-0000-0000-00000000000b";

const PLATFORM: Bundle = Bundle { name: "test", files: &[("en", "prefix: \"&8[&#f28c28&lP&#ffd54f&lP&8] \"\n")] };

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
    assert!(sub.contains("/pp user Steve permission ...") && !sub.contains("permission set"), "{sub}");
    let perm_help = console(&mut e, "user Steve permission");
    assert!(perm_help.contains("settemp") && !perm_help.contains("meta setprefix"));
    // chat help lists every command at once, with clickable entries
    let op = Who::player("Steve", STEVE, 4, "world");
    let out = command::run(&mut e, &op, &split_args("help"), &env(0));
    let plain = out.reply.plain();
    assert!(!plain.contains("1/3") && plain.contains("user <player>") && plain.contains("reload"), "{plain}");
    assert!(
        out.reply.lines.iter().skip(1).take(8).all(|l| l
            .segments
            .iter()
            .skip(5)
            .all(|s| s.click.is_some() && s.hover.is_some()))
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
    assert_eq!(snap.fixed.iter().filter(|f| f.contains("parent \"ghost\"")).count(), 2, "{:?}", snap.fixed);
}

/// A deny group with a Polish name: preview, run, the ledger, a run of the
/// same file and undo all use its new name, and its member keeps the deny.
#[test]
fn deny_group_with_a_polish_name_keeps_denying() {
    const BAD_PLAYER: &str = "ŻółwiaGęśŚliczna_bardzo_długa_nazwa_z_ogonkami";
    let mut e = engine();
    let json = serde_json::json!({
        "format": "pumboperms", "version": 1,
        "groups": [
            {"name": "vip", "data": {"permissions": [{"node": "fly", "value": true}]}},
            {"name": "Zakaz Latania", "weight": 10, "data": {"permissions": [{"node": "fly", "value": false}]}},
        ],
        "users": [{"uuid": STEVE, "name": "Steve", "data": {"parents": [{"group": "vip"}, {"group": "Zakaz Latania"}]}}],
        "by-name": [{"name": BAD_PLAYER, "data": {"parents": [{"group": "vip"}]}}],
    });
    let snap = export::parse(&json.to_string()).unwrap();
    let source = |e: &mut Engine, fingerprint: &str, mode| {
        import_source(e, &Who::console(), "file", Ok((fingerprint.into(), snap.clone())), mode, 1)
    };
    let preview = source(&mut e, "f1", SourceMode::Preview).reply.plain();
    let run = source(&mut e, "f1", SourceMode::Run);
    for reply in [&preview, &run.reply.plain()] {
        assert!(reply.contains("2 entries of the file were fixed or left out"), "{reply}");
        assert!(reply.contains("\"Zakaz Latania\" imported as zakaz-latania"), "{reply}");
        assert!(reply.contains(&format!("{BAD_PLAYER:?}")), "{reply}");
    }
    assert!(preview.contains("Steve in zakaz-latania"), "{preview}");
    let fly = |e: &mut Engine| e.perms.check(STEVE, "fly", &crate::context::Query::default(), 0);
    assert_eq!(fly(&mut e), Some(false), "the deny holds");
    let ledger = e.ledger("file").unwrap();
    assert!(ledger.pending.is_empty(), "the bad player waits for no join");
    assert!(format!("{:?}", ledger.entries).contains("zakaz-latania"), "{:?}", ledger.entries);
    let again = source(&mut e, "f2", SourceMode::Run).reply.plain();
    assert!(again.contains("0 entries added") && !again.contains("no longer has"), "{again}");
    assert_eq!(e.ledger("file").unwrap().entries, ledger.entries, "the same file, the same entries");
    let undo = import_source(&mut e, &Who::console(), "file", Err(String::new()), SourceMode::Undo, 2);
    assert!(undo.changed, "{}", undo.reply.plain());
    assert!(e.perms.group("zakaz-latania").is_none() && e.perms.group("vip").is_none());
    assert_eq!(fly(&mut e), None);
}

/// A whole-file import keeps every rung of a track over a group with a
/// Polish name, so a promotion goes where it went before.
#[test]
fn import_keeps_every_rung_of_a_track() {
    let mut e = engine();
    let json = serde_json::json!({
        "format": "pumboperms", "version": 1,
        "groups": [{"name": "Początkujący"}, {"name": "vip"}],
        "tracks": [{"name": "ranks", "groups": ["default", "Początkujący", "vip"]}],
    });
    let out = command::import(&mut e, &Who::console(), "x", &json.to_string(), "before", 1);
    assert!(out.reply.plain().contains("\"Początkujący\" imported as poczatkujacy"), "{}", out.reply.plain());
    assert_eq!(e.perms.track("ranks").unwrap().groups, ["default", "poczatkujacy", "vip"]);
    console(&mut e, "promote Steve ranks");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert!(parents.len() == 1 && parents[0].group == "poczatkujacy", "{parents:?}");
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

fn tab(e: &mut Engine, who: &Who, line: &str) -> Vec<String> {
    let mut args = split_args(line);
    if line.is_empty() || line.ends_with(' ') {
        args.push(String::new());
    }
    let online = [KnownName { uuid: STEVE.into(), name: "Steve".into(), online: true }];
    command::complete(e, who, &args, &online, 0)
}

#[test]
fn node_completion_includes_catalog() {
    let mut e = engine();
    let con = Who::console();
    let got = tab(&mut e, &con, "user Steve permission set minecraft:command.gam");
    assert!(got.contains(&"minecraft:command.gamemode".to_string()), "{got:?}");
    let got = tab(&mut e, &con, "group default permission check pumbo.perms.user.i");
    assert_eq!(got, ["pumbo.perms.user.info"]);
    let pumbo = pumbo_common::pumpkin::PUMBO_NODES[0];
    assert!(tab(&mut e, &con, &format!("user Steve permission unset {pumbo}")).contains(&pumbo.to_string()));
    // A node goes where the node goes, contexts only where the line takes them.
    let at_node = tab(&mut e, &con, "user Steve permission set ");
    assert!(!at_node.iter().any(|o| o.ends_with('=')), "{at_node:?}");
    assert!(tab(&mut e, &con, "user Steve permission set fly ").contains(&"server=".to_string()));
    assert!(tab(&mut e, &con, "user Steve parent add default ").contains(&"server=".to_string()));
    assert!(tab(&mut e, &con, "user Steve meta set ").is_empty());
}

#[test]
fn completion_hides_forbidden_subcommands() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "createtrack ranks");
    console(&mut e, "group vip permission set secret.node");
    console(&mut e, "user Steve permission set pumbo.perms.user.info");
    let p = steve();
    assert_eq!(tab(&mut e, &p, ""), ["help", "user"]);
    assert_eq!(tab(&mut e, &p, "user "), ["Steve"]);
    assert_eq!(tab(&mut e, &p, "user Steve "), ["info"]);
    assert!(tab(&mut e, &p, "group ").is_empty());
    assert!(tab(&mut e, &p, "track ").is_empty());
    assert!(tab(&mut e, &p, "user Steve permission set ").is_empty());
    // Adding parents without group.info: the word, but no group names.
    console(&mut e, "user Steve permission set pumbo.perms.user.info false");
    console(&mut e, "user Steve permission set pumbo.perms.user.parent.add");
    assert!(tab(&mut e, &p, "user ").is_empty(), "names need user.info");
    assert_eq!(tab(&mut e, &p, "user Steve "), ["parent"]);
    assert_eq!(tab(&mut e, &p, "user Steve parent "), ["add", "addtemp"]);
    assert!(tab(&mut e, &p, "user Steve parent add ").is_empty());
    console(&mut e, "user Steve permission set pumbo.perms.group.info");
    assert_eq!(tab(&mut e, &p, "u Steve parents add v"), ["vip"]);
    // A switched-off subcommand is gone at every level.
    e.cfg.disabled = vec!["user".into()];
    assert_eq!(tab(&mut e, &p, ""), ["group", "help"]);
    assert!(tab(&mut e, &p, "user Steve ").is_empty());
}

#[test]
fn meta_key_is_validated_and_quoted() {
    let mut e = engine();
    for bad in ["a:b", "x/y", &"k".repeat(65)] {
        let out = console(&mut e, &format!("user Steve meta set {bad} v"));
        assert!(out.contains("not a valid meta key"), "{bad}: {out}");
    }
    assert!(console(&mut e, "user Steve meta set Rank.Color-2_x gold").contains("Set rank.color-2_x"));
    assert!(console(&mut e, "user Steve meta unset rank.color-2_x extra").contains("Usage"));
    // The removal a click puts into the chat box takes exactly that entry.
    let text = r#"&c[A "q" \x\] "#;
    console(&mut e, &format!("user Steve meta addprefix 5 {}", quote(text)));
    console(&mut e, "user Steve meta addprefix 5 other");
    // A key an import brought, outside the rule for new keys.
    let odd = crate::model::MetaNode {
        kind: crate::model::MetaKind::Meta,
        key: "odd\"key".into(),
        priority: 0,
        value: "1".into(),
        context: crate::context::Contexts::global(),
        expiry: None,
    };
    e.perms.edit_user(STEVE, "Steve", |d| d.meta.push(odd));
    let out = command::run(&mut e, &Who::console(), &split_args("user Steve meta info"), &env(0));
    let mut runs = clicks(&out.reply);
    runs.dedup();
    for c in runs {
        if c.join(" ").contains("removeprefix 5 &c") || c.join(" ").contains("unset odd") {
            let out = command::run(&mut e, &Who::console(), &c, &env(0)).reply.plain();
            assert!(out.contains("Removed"), "{c:?}: {out}");
        }
    }
    let left: Vec<String> = e.perms.user(STEVE).unwrap().data.meta.iter().map(|m| m.value.clone()).collect();
    assert_eq!(left, ["gold", "other"], "only the clicked entries went");
}

/// [`split_args`] backwards, as a test writes a quoted word.
fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

#[test]
fn settemp_refuses_existing_expiry() {
    let mut e = engine();
    console(&mut e, "user Steve permission settemp fly true 10m");
    let out = console(&mut e, "user Steve permission settemp fly true 1h");
    assert!(
        out.contains("already has an end time") && out.contains("/pp user Steve permission unsettemp fly"),
        "{out}"
    );
    let p = e.perms.user(STEVE).unwrap().data.permissions.clone();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].expiry, Some(1_000 + 600_000), "the old end time stays");
    // Another context is another entry; after unsettemp the new time goes in.
    assert!(console(&mut e, "user Steve permission settemp fly true 1h server=lobby").contains("for 1h"));
    console(&mut e, "user Steve permission unsettemp fly");
    assert!(console(&mut e, "user Steve permission settemp fly true 1h").contains("for 1h"));
}

#[test]
fn editor_hidden_from_help() {
    let mut e = engine();
    assert!(!console(&mut e, "help").contains("editor"));
    let op = Who::player("Steve", STEVE, 4, "world");
    for page in 1..=3 {
        assert!(!run_as(&mut e, &op, &format!("help {page}"), 0).contains("editor"));
    }
    assert!(!tab(&mut e, &Who::console(), "").contains(&"editor".to_string()));
    assert!(tab(&mut e, &Who::console(), "ed").is_empty());
    assert!(console(&mut e, "editor").contains("not available yet"));
}

/// `group=a` and `group=b` score the same, so on a server in both groups the
/// order of [`crate::perms`] decides: false wins the tie.
#[test]
fn group_contexts_tie_resolves_to_false() {
    let mut e = engine();
    console(&mut e, "user Steve permission set fly true group=a");
    console(&mut e, "user Steve permission set fly false group=b");
    console(&mut e, "user Steve permission set walk false group=a");
    console(&mut e, "user Steve permission set walk true group=b");
    let q = crate::context::Query::at("", vec!["a".into(), "b".into()], "");
    assert_eq!(e.perms.check(STEVE, "fly", &q, 0), Some(false));
    assert_eq!(e.perms.check(STEVE, "walk", &q, 0), Some(false));
}

#[test]
fn views_have_pages_for_players_and_all_rows_for_the_console() {
    let mut e = engine();
    console(&mut e, "user Alex permission set pumbo.perms.*");
    for i in 0..45 {
        console(&mut e, &format!("creategroup g{i:02}"));
        console(&mut e, &format!("createtrack t{i:02}"));
        console(&mut e, &format!("user Steve parent add g{i:02}"));
        console(&mut e, &format!("user Steve meta set k{i:02} v"));
    }
    let alex = Who::player("Alex", ALEX, 0, "world");
    for (view, letter) in
        [("groups", 'g'), ("tracks", 't'), ("user Steve parent info", 'g'), ("user Steve meta info", 'k')]
    {
        let show =
            |e: &mut Engine, page: &str| command::run(e, &alex, &split_args(&format!("{view} {page}")), &env(0)).reply;
        let first = show(&mut e, "1");
        assert!(first.plain().contains("1/3") && first.plain().contains(&format!("{letter}05")), "{view}");
        let runs: Vec<String> = first
            .lines
            .iter()
            .flat_map(|l| &l.segments)
            .filter_map(|s| match &s.click {
                Some(Click::Run(c)) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert!(runs.contains(&format!("/pp {view} 2")), "{view}: {runs:?}");
        let last = show(&mut e, "3").plain();
        assert!(last.contains("3/3") && last.contains(&format!("{letter}44")), "{view}: {last}");
        assert!(!last.contains(&format!("{letter}37")), "{view}: {last}");
        assert_eq!(show(&mut e, "99").plain(), last, "{view}");
        let all = console(&mut e, view);
        assert!(all.contains(&format!("{letter}00")) && all.contains(&format!("{letter}44")), "{view}");
        assert!(!all.contains("1/3"), "{view}: {all}");
    }
}

#[test]
fn completion_skips_a_left_out_choice() {
    let mut e = engine();
    let con = Who::console();
    let got = tab(&mut e, &con, "user Steve permission settemp fly 1h ");
    assert!(got.contains(&"server=".to_string()), "{got:?}");
    assert!(tab(&mut e, &con, "user Steve permission settemp fly true 1h ").contains(&"server=".to_string()));
    assert!(tab(&mut e, &con, "user Steve permission settemp fly ").contains(&"true".to_string()));
}

/// The colour of the first segment whose text, without the spaces around it, is `text`.
fn color_of(t: &Text, text: &str) -> Option<Color> {
    t.lines.iter().flat_map(|l| &l.segments).find(|s| s.text.trim() == text).and_then(|s| s.style.color)
}

/// The colour of the first segment that contains `part`.
fn color_with(t: &Text, part: &str) -> Option<Color> {
    t.lines.iter().flat_map(|l| &l.segments).find(|s| s.text.contains(part)).and_then(|s| s.style.color)
}

#[test]
fn info_views_in_colour() {
    let mut e = engine();
    for line in [
        "creategroup vip",
        "group vip setdisplayname &6VIP",
        "group vip setweight 10",
        "group vip meta setprefix 10 \"&6[VIP] \"",
        "group vip permission settemp fly true 1h server=lobby",
        "group vip permission set minecraft:command.gamemode false",
        "user Steve parent add vip",
        "user Steve permission set essentials:fly true",
        "user Steve permission set essentials:fly.* false",
    ] {
        console(&mut e, line);
    }
    let run = |e: &mut Engine, line: &str| command::run(e, &Who::console(), &split_args(line), &env(1_000)).reply;

    let group = run(&mut e, "group vip info");
    assert_eq!(color_of(&group, "Group"), Some(style::accent::PERMS), "title");
    assert_eq!(color_of(&group, "vip"), Some(style::VALUE), "name in the title");
    assert_eq!(color_of(&group, "Weight:"), Some(style::LABEL), "label");
    assert_eq!(color_of(&group, "10"), Some(style::VALUE), "value");
    assert_eq!(color_of(&group, "VIP"), Some(Color::Named(Named::Gold)), "display name in its own colour");
    assert_eq!(color_of(&group, "Permissions (2)"), Some(style::accent::PERMS), "heading");
    assert_eq!(color_of(&group, "•"), Some(style::MUTED), "marker");
    assert_eq!(color_of(&group, "true"), Some(style::SUCCESS));
    assert_eq!(color_of(&group, "false"), Some(style::ERROR));
    assert_eq!(color_with(&group, "server=lobby"), Some(style::INFO), "context");
    assert_eq!(color_with(&group, "expires in"), Some(style::INFO), "end time");
    assert_eq!(color_of(&group, "none"), Some(style::MUTED));
    assert_eq!(color_of(&group, "prefix 10"), Some(style::LABEL), "meta key");

    let user = run(&mut e, "user Steve info");
    assert_eq!(color_of(&user, "Prefix:"), Some(style::LABEL));
    // the prefix in its own colours, then its text in grey quotes
    assert_eq!(color_of(&user, "[VIP]"), Some(Color::Named(Named::Gold)));
    assert_eq!(color_of(&user, "\"&6[VIP] \""), Some(style::INFO));
    assert!(user.plain().contains("Prefix: [VIP]  \"&6[VIP] \""), "{}", user.plain());
    // a warning keeps its colour after a highlighted value
    assert!(
        user.plain().contains("  ! not sent to Pumpkin: essentials:fly hides essentials:fly.*"),
        "{}",
        user.plain()
    );
    assert_eq!(color_with(&user, "hides"), Some(style::WARN));

    let check = run(&mut e, "group vip permission check fly server=lobby");
    assert_eq!(check.plain().lines().count(), 2, "{}", check.plain());
    assert_eq!(color_of(&check, "for group"), Some(style::accent::PERMS), "title of the check");
    assert_eq!(color_of(&check, "true"), Some(style::SUCCESS));
    let how = Text::from(check.lines[1].clone());
    assert_eq!(how.lines[0].segments[0].style.color, Some(style::LABEL), "{:?}", how.lines[0].segments);
    assert_eq!(color_of(&how, "fly"), Some(style::VALUE));
    assert_eq!(color_with(&how, "server=lobby"), Some(style::INFO));
    assert_eq!(color_with(&how, "expires in"), Some(style::INFO));
    let undecided = run(&mut e, "group vip permission check nothing.here");
    assert_eq!(color_with(&undecided, "not set"), Some(style::accent::PERMS));

    for (view, title) in [
        ("groups", "Groups:"),
        ("tracks", "Tracks:"),
        ("group vip listmembers", "Members of"),
        ("log", "Recent changes"),
    ] {
        assert_eq!(color_of(&run(&mut e, view), title), Some(style::accent::PERMS), "{view}");
    }
    assert_eq!(color_of(&run(&mut e, "groups"), "Weight"), Some(style::LABEL));
    let log = run(&mut e, "log");
    assert_eq!(color_with(&log, "group vip"), Some(style::LABEL), "{:?}", log.lines[1].segments);
    assert_eq!(color_with(&log, " ago"), Some(style::INFO));
}

#[test]
fn help_and_usage_take_the_perms_yellow() {
    let mut e = engine();
    console(&mut e, "user Alex permission set pumbo.perms.*");
    let alex = Who::player("Alex", ALEX, 0, "world");
    let help = command::run(&mut e, &alex, &split_args("help"), &env(0)).reply;
    // The name sits in the top bar after the `[PP] ` tag; command lines have `> `.
    let header = &help.lines[0];
    let at = header.segments.iter().position(|s| s.text == "Pumbo").expect("name in the top bar");
    assert_eq!(header.segments[at].style.color, Some(style::BRAND));
    assert_eq!(
        (header.segments[at + 1].text.as_str(), header.segments[at + 1].style.color),
        ("Perms", Some(style::accent::PERMS))
    );
    assert_eq!(color_of(&help, "/pp"), Some(style::accent::PERMS), "root command");
    let user = &help.lines[1].segments[5..];
    assert_eq!((user[0].text.as_str(), user[0].style.color), ("user ", Some(style::accent::PERMS)));
    assert_eq!(user[1].style.color, Some(style::ARGUMENT));
    assert_eq!(color_of(&help, "[page]"), Some(style::OPTIONAL));
    let usage = command::run(&mut e, &Who::console(), &split_args("user"), &env(0)).reply;
    assert_eq!(color_of(&usage, "/pp user"), Some(style::accent::PERMS), "{:?}", usage.lines[0].segments);
    let unknown = command::run(&mut e, &Who::console(), &split_args("foo"), &env(0)).reply;
    assert_eq!(color_of(&unknown, "/pp help"), Some(style::accent::PERMS), "{:?}", unknown.lines[0].segments);
}

#[test]
fn chat_lines_start_with_the_tag() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "group vip permission set essentials:home");
    console(&mut e, "user Steve parent add vip");
    let op = Who::player("Steve", STEVE, 4, "world");
    let tag = [
        ("[", Some(style::MUTED)),
        ("P", Some(style::BRAND)),
        ("P", Some(style::accent::PERMS)),
        ("] ", Some(style::MUTED)),
    ];
    for line in [
        "help",
        "user Steve",
        "group vip info",
        "user Steve info",
        "groups",
        "log",
        "creategroup vip",
        "nonsense",
        "user",
    ] {
        let reply = command::run(&mut e, &op, &split_args(line), &env(0)).reply;
        assert!(!reply.lines.is_empty(), "/pp {line}");
        for l in &reply.lines {
            let got: Vec<_> = l.segments.iter().take(4).map(|s| (s.text.as_str(), s.style.color)).collect();
            assert_eq!(got, tag, "/pp {line}: {}", l.plain());
            let bold: Vec<bool> = l.segments.iter().take(4).map(|s| s.style.bold).collect();
            assert_eq!(bold, [false, true, true, false], "/pp {line}: bold letters, plain brackets");
        }
    }
    // Command lines in the help: `[PP] > user <player> ...`; the header has no `> `.
    let help = command::run(&mut e, &op, &split_args("help"), &env(0)).reply;
    assert_ne!(help.lines[0].segments[4].text, "> ");
    for l in &help.lines[1..6] {
        assert_eq!(
            (l.segments[4].text.as_str(), l.segments[4].style.color),
            ("> ", Some(style::MUTED)),
            "{}",
            l.plain()
        );
    }
    for l in &help.lines {
        assert!(l.width() <= pumbo_common::rich::CHAT_WIDTH, "{} px: {}", l.width(), l.plain());
    }
    // The console gets the prefix once, at the start, as before.
    let view = console(&mut e, "group vip info");
    let lines: Vec<&str> = view.lines().collect();
    assert!(lines[0].starts_with("[PP] ") && lines.len() > 3, "{view}");
    assert!(lines[1..].iter().all(|l| !l.contains("[PP]")), "{view}");
    assert!(!console(&mut e, "help").contains("[PP]"));
}

#[test]
fn player_help_is_grouped_like_luckperms() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let show = |e: &mut Engine, who: &Who, line: &str| command::run(e, who, &split_args(line), &env(0)).reply;
    let user = show(&mut e, &op, "user Steve");
    let plain = user.plain();
    for part in ["info [page]", "permission ...", "parent ...", "meta ...", "promote <track>"] {
        assert!(plain.contains(part), "{part}: {plain}");
    }
    assert!(!plain.contains("permission set") && !plain.contains("meta setprefix"), "{plain}");
    // `/pp user Steve permission` lists its commands without the word again.
    let perm = show(&mut e, &op, "user Steve permission");
    let rows: Vec<String> = perm.lines.iter().skip(1).map(|l| l.plain()).collect();
    assert!(rows.iter().any(|l| l.starts_with("[PP] > set <node>")), "{rows:?}");
    assert!(rows.iter().all(|l| !l.contains("> permission ")), "{rows:?}");
    // A description is there whole or not at all, and no line is cut.
    let meta = show(&mut e, &op, "group default meta");
    let general = show(&mut e, &op, "help");
    for t in [&user, &perm, &meta, &general] {
        for l in &t.lines {
            assert!(l.width() <= pumbo_common::rich::CHAT_WIDTH, "{} px: {}", l.width(), l.plain());
            assert!(!l.plain().ends_with("..."), "cut: {}", l.plain());
        }
    }
    // Only the groups of commands the player may use.
    console(&mut e, "user Alex permission set pumbo.perms.user.permission.info");
    let alex = Who::player("Alex", ALEX, 0, "world");
    let limited = show(&mut e, &alex, "user Alex").plain();
    assert!(limited.contains("permission ...") && !limited.contains("meta ...") && !limited.contains("parent ..."));
}

#[test]
fn help_and_pages_are_framed() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let show = |e: &mut Engine, line: &str| command::run(e, &op, &split_args(line), &env(0)).reply;
    let ruled = |l: &pumbo_common::rich::Line| {
        let rules: Vec<_> = l.segments.iter().filter(|s| s.style.strikethrough).map(|s| s.style.color).collect();
        rules.first() == Some(&Some(style::BRAND)) && rules.last() == Some(&Some(style::accent::PERMS))
    };
    for line in ["help", "user Steve", "user Steve permission"] {
        let help = show(&mut e, line);
        let (top, bottom) = (&help.lines[0], help.lines.last().unwrap());
        assert!(ruled(top) && ruled(bottom), "/pp {line}: {} / {}", top.plain(), bottom.plain());
        assert!(top.plain().contains("PumboPerms"), "{}", top.plain());
        // The hint is the tooltip of the name, not a line of its own.
        assert!(!help.plain().contains("Hover a command"), "{}", help.plain());
        let name = top.segments.iter().find(|s| s.text == "Pumbo").unwrap();
        assert!(name.hover.as_ref().is_some_and(|h| h.plain().contains("Hover a command")));
        for l in &help.lines {
            assert!(l.width() <= pumbo_common::rich::CHAT_WIDTH, "{} px: {}", l.width(), l.plain());
        }
    }
    // A paged view ends with a bar whose arrows turn the page.
    console(&mut e, "creategroup big");
    for n in 0..45 {
        console(&mut e, &format!("group big permission set big.node{n:02}"));
    }
    let view = show(&mut e, "group big permission info 2");
    let bar = view.lines.last().unwrap();
    assert!(ruled(bar) && bar.plain().contains("2/3"), "{}", bar.plain());
    let runs: Vec<String> = bar
        .segments
        .iter()
        .filter_map(|s| match &s.click {
            Some(Click::Run(c)) => Some(c.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(runs, ["/pp group big permission info 1", "/pp group big permission info 3"]);
    assert!(bar.width() <= pumbo_common::rich::CHAT_WIDTH);
}

#[test]
fn views_sit_between_bars_in_chat() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let show = |e: &mut Engine, line: &str| command::run(e, &op, &split_args(line), &env(0)).reply;
    let ruled = |l: &pumbo_common::rich::Line| l.segments.iter().any(|s| s.style.strikethrough);
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve permission set fly");
    for line in ["user Steve info", "group vip info", "groups", "log", "user Steve permission check fly", "version"] {
        let view = show(&mut e, line);
        let (top, bottom) = (&view.lines[0], view.lines.last().unwrap());
        assert!(ruled(top) && ruled(bottom), "/pp {line}: {}", view.plain());
        assert_eq!(view.lines.iter().filter(|l| ruled(l)).count(), 2, "/pp {line}: two bars");
        for l in &view.lines {
            assert!(l.width() <= pumbo_common::rich::CHAT_WIDTH, "{} px: {}", l.width(), l.plain());
        }
    }
    let top = show(&mut e, "user Steve info").lines[0].plain();
    assert!(top.contains(" Player Steve "), "the title sits in the top bar: {top}");
    // A page bar at the end is the bottom bar.
    for n in 0..45 {
        console(&mut e, &format!("group vip permission set vip.node{n:02}"));
    }
    let paged = show(&mut e, "group vip permission info");
    assert!(paged.lines.last().unwrap().plain().contains("1/3"));
    assert_eq!(paged.lines.iter().filter(|l| ruled(l)).count(), 2);
    // One line stays one line; the console keeps its plain text.
    assert_eq!(show(&mut e, "creategroup other").lines.len(), 1);
    assert!(!console(&mut e, "user Steve info").lines().next().unwrap().contains("  "));
}

#[test]
fn chat_shows_codes_of_prefixes_in_a_tooltip() {
    let mut e = engine();
    for c in [
        "creategroup vip",
        "group vip meta setprefix 10 \"&6[VIP] \"",
        "group vip setdisplayname \"&6VIP\"",
        "user Steve parent add vip",
    ] {
        console(&mut e, c);
    }
    let op = Who::player("Steve", STEVE, 4, "world");
    let show = |e: &mut Engine, line: &str| command::run(e, &op, &split_args(line), &env(0)).reply;
    let tooltip = |t: &Text, part: &str| {
        t.lines
            .iter()
            .flat_map(|l| &l.segments)
            .find(|s| s.text.contains(part))
            .and_then(|s| s.hover.as_ref())
            .map(|h| h.plain())
    };
    for line in ["user Steve info", "group vip info", "user Steve parent info", "group vip meta info"] {
        let plain = show(&mut e, line).plain();
        assert!(!plain.contains("&6"), "/pp {line}: the code is in the tooltip, not the line: {plain}");
    }
    let user = show(&mut e, "user Steve info");
    assert_eq!(tooltip(&user, "[VIP]").as_deref(), Some("\"&6[VIP] \""));
    let group = show(&mut e, "group vip info");
    assert_eq!(tooltip(&group, "VIP").as_deref(), Some("\"&6VIP\""), "shown as");
    let parents = show(&mut e, "user Steve parent info");
    assert_eq!(tooltip(&parents, "VIP").as_deref(), Some("\"&6VIP\""), "display name of a parent");
    // The console has no tooltips: it keeps the text as typed.
    assert!(console(&mut e, "user Steve info").contains("[VIP]  \"&6[VIP] \""));
    assert!(console(&mut e, "user Steve parent info").contains("vip (&6VIP)"));
}
