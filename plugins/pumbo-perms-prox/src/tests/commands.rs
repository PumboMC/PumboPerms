//! `/pp` on the proxy, its aliases, config, messages and Tab completion.

use super::*;

#[test]
fn aliases_come_from_config() {
    let (p, dir) = plugin(Some("commands:\n  aliases: [perms, p]\n  disabled: [editor]\n"));
    assert_eq!(registered(), ["perms", "p"]);
    command(&p, "p", &["user"]);
    assert!(logs().contains("/perms user <player> info [page]"), "{}", logs());
    let umbrella: Vec<String> =
        testing::with(|h| h.commands.iter().filter(|c| c.umbrella).map(|c| c.name.clone()).collect());
    assert!(umbrella.contains(&"user".to_string()) && !umbrella.contains(&"editor".to_string()), "{umbrella:?}");
    command(&p, "perms", &["editor"]);
    assert!(logs().contains("Unknown subcommand editor. Use /perms help."), "{}", logs());
    // A reload keeps the registered short commands until the plugin restarts.
    std::fs::write(dir.join("config.yml"), "commands:\n  aliases: [x]\n").unwrap();
    block_on(p.on_reload()).unwrap();
    assert!(logs().contains("commands.aliases changes after"), "{}", logs());
    assert!(logs().contains("commands.disabled applies to /pumboperms <subcommand> after"), "{}", logs());
    testing::with(|h| h.logs.clear());
    command(&p, "perms", &["user"]);
    assert!(logs().contains("/perms user <player>"), "{}", logs());
}

#[test]
fn cleared_alias_list_registers_only_full_name() {
    let (p, _dir) = plugin(Some("commands:\n  aliases: []\n"));
    assert!(registered().is_empty(), "{:?}", registered());
    command(&p, "user", &[]);
    assert!(logs().contains("/pumboperms user <player> info [page]"), "{}", logs());
}

#[test]
fn taken_alias_is_skipped_with_warning() {
    let (p, _dir) = plugin_after(None, PermsStore::in_memory().unwrap(), || {
        Command::new("pp").usage("/pp").register().unwrap();
    });
    assert!(logs().contains("/pp not registered, /pumboperms works"), "{}", logs());
    command(&p, "user", &[]);
    assert!(logs().contains("/pumboperms user <player> info [page]"), "{}", logs());
}

#[test]
fn template_messages_and_manifest() {
    let (cfg, w) = common_config::load::<setup::Config>(setup::CONFIG_TEMPLATE);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg, setup::Config::default());
    assert_eq!(check_bundle(&setup::LANG), Vec::<String>::new());
    for code in ["en", "pl"] {
        let (l, w) = Lang::load(&setup::BUNDLES, code, None);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(l.get("prefix"), pumbo_common::style::tag_code("PumboPerms", pumbo_common::style::accent::PERMS));
    }
    // Every permission is declared with a description.
    let declared: Vec<&str> = MANIFEST.lines().filter_map(|l| l.trim().strip_prefix("- node: ")).collect();
    for a in ACTIONS.iter().copied().chain(["command", "debug"]) {
        assert!(declared.contains(&format!("pumbo.perms.{a}").as_str()), "{a} missing from the manifest");
    }
    assert_eq!(MANIFEST.matches("description:").count(), declared.len());
    // A bad value falls back with a warning; a broken file is refused on reload.
    let (cfg, w) = common_config::load::<setup::Config>("language: \"bad lang!\"\nimport:\n  folow: true\n");
    assert_eq!(cfg.language, "en");
    assert_eq!(w.len(), 2, "{w:?}");
    // Not YAML: the error names the line, /pp reload keeps the settings.
    let (p, dir) = plugin(Some("language: pl\n"));
    std::fs::write(dir.join("config.yml"), "language: [pl\nlog: {}\n").unwrap();
    let err = p.reload().unwrap_err();
    assert!(err.contains("line"), "{err}");
    assert_eq!(p.with(|s| s.cfg.language.clone()).unwrap(), "pl");
    let _ = std::fs::remove_file(dir.join("config.yml"));
}

/// A `config.yml` that is not valid YAML at start: the last good copy, not the
/// defaults (which would import permissions.yml again, for one).
#[test]
fn broken_config_at_start_keeps_last_good() {
    let (_, dir) = plugin(Some("language: pl\nimport:\n  enabled: false\n"));
    std::fs::write(dir.join("config.yml"), "language: [pl\n").unwrap();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Ok(PermsStore::in_memory().unwrap())).unwrap();
    assert_eq!(p.with(|s| (s.cfg.language.clone(), s.cfg.import.enabled)).unwrap(), ("pl".into(), false));
    assert!(testing::with(|h| h.logs.iter().any(|(_, l)| l.contains(common_config::LAST_GOOD))), "{}", logs());
    // Without the copy the plugin does not start.
    std::fs::remove_file(dir.join(common_config::LAST_GOOD)).unwrap();
    let p = PumboPerms {
        state: RefCell::new(None),
        config_dir: dir.to_string_lossy().into(),
        data_dir: dir.to_string_lossy().into(),
    };
    assert!(p.start(Ok(PermsStore::in_memory().unwrap())).unwrap_err().contains("not valid YAML"));
    let _ = std::fs::remove_file(dir.join("config.yml"));
}

#[test]
fn built_in_language_files_are_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lang");
    let written = Lang::write_templates(&setup::BUNDLES, &dir);
    assert!(written.is_empty(), "rewritten from the message bundles, commit them: {written:?}");
}

/// Change notices go only to players whose permission set has the notify node.
#[test]
fn notify_reaches_only_players_with_the_node() {
    let (p, _) = plugin(None);
    let staff = testing::add_player("Staff", Some("lobby"));
    let joe = testing::add_player("Joe", Some("lobby"));
    load(&p, staff);
    load(&p, joe);
    console(&p, "user Staff permission set pumbo.perms.log.notify");
    let (staff_before, joe_before) = (testing::messages(staff).len(), testing::messages(joe).len());
    console(&p, "creategroup vip");
    let got = testing::messages(staff);
    assert!(got.get(staff_before..).is_some_and(|new| new.iter().any(|m| m.contains("vip"))), "{got:?}");
    assert_eq!(testing::messages(joe).len(), joe_before);
}

#[test]
fn proxy_refuses_world_context() {
    let (p, _) = plugin(None);
    console(&p, "creategroup vip");
    let refused = |p: &PumboPerms, line: &str| {
        console(p, line);
        let out = testing::with(|h| h.logs.last().map(|(_, l)| l.clone())).unwrap_or_default();
        assert!(out.contains("does not see worlds"), "{line}: {out}");
    };
    refused(&p, "group vip permission set fly true world=nether");
    refused(&p, "group vip parent add default world=nether");
    refused(&p, "group vip meta set color red world=nether");
    refused(&p, "group vip permission check fly world=nether");
    assert!(p.with(|s| s.engine.perms.group("vip").unwrap().data.is_empty()).unwrap());
}

/// A group with 200 nodes: its views come in pages, and every message a
/// player gets stays under the host's text limit as valid JSON.
#[test]
fn long_views_fit_proxy_text_limit() {
    use pumbo_sdk::testing::Sent;

    let (p, _) = plugin(None);
    let admin = testing::add_player("Admin", Some("lobby"));
    load(&p, admin);
    console(&p, "user Admin permission set pumbo.perms.*");
    console(&p, "creategroup big");
    let long = "x".repeat(60);
    for i in 0..200 {
        console(&p, &format!("group big permission set some.node.{i:03}.{long} true server=lobby"));
    }
    let before = testing::sent_to(admin).len();
    for line in ["group big info", "group big permission info", "group big info 11", "group big permission info 99"] {
        block_on(p.run(Some(admin), split_args(line)));
    }
    let texts: Vec<String> = testing::sent_to(admin)
        .into_iter()
        .skip(before)
        .filter_map(|s| match s {
            Sent::Message(Text::Json(j)) => Some(j),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 4);
    for j in &texts {
        assert!(j.len() < MAX_TEXT, "{}", j.len());
        serde_json::from_str::<serde_json::Value>(j).unwrap();
    }
    assert!(texts[0].contains("1/11") && texts[1].contains("1/11") && texts[3].contains("11/11"), "{}", texts[0]);
    assert!(texts[3].contains("some.node.199"));

    // A reply too long for one message is cut between lines.
    let lines: Vec<String> = (0..2000).map(|i| format!("&aline {i} {long}")).collect();
    let parts = messages(&Rich::parse(&lines.join("\n")));
    assert!(parts.len() > 1);
    let mut seen = 0;
    for part in parts {
        let Text::Json(j) = part else { panic!("not JSON") };
        assert!(j.len() < MAX_TEXT, "{}", j.len());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        seen += v["extra"].as_array().unwrap().iter().filter(|x| x["text"] != "\n").count();
    }
    assert_eq!(seen, 2000, "every line once");
}

/// Tab on the proxy answers from the same table as on Pumpkin, for `/pp`
/// and for `/pumboperms <sub>`.
#[test]
fn suggest_follows_the_command_line() {
    let (p, _) = plugin(None);
    let admin = testing::add_player("Admin", Some("lobby"));
    let joe = testing::add_player("Joe", Some("lobby"));
    load(&p, admin);
    load(&p, joe);
    console(&p, "user Admin permission set pumbo.perms.*");
    console(&p, "creategroup vip");
    let s = |who: PlayerId, name: &str, args: &str| block_on(p.on_suggest(who, name.into(), args.into())).unwrap();
    assert!(s(admin, "pp", "").contains(&"user".to_string()));
    assert_eq!(s(admin, "pp", "user J"), ["Joe"]);
    assert_eq!(s(admin, "user", "Joe parent add v"), ["vip"]);
    assert!(
        s(admin, "pp", "user Joe permission set minecraft:command.gamem")
            .contains(&"minecraft:command.gamemode".into())
    );
    assert_eq!(s(joe, "pp", ""), ["help"]);
    assert!(s(joe, "pp", "user ").is_empty(), "no names without user.info");
}
