//! Tab completion.

use super::*;

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

#[test]
fn completion_skips_a_left_out_choice() {
    let mut e = engine();
    let con = Who::console();
    let got = tab(&mut e, &con, "user Steve permission settemp fly 1h ");
    assert!(got.contains(&"server=".to_string()), "{got:?}");
    assert!(tab(&mut e, &con, "user Steve permission settemp fly true 1h ").contains(&"server=".to_string()));
    assert!(tab(&mut e, &con, "user Steve permission settemp fly ").contains(&"true".to_string()));
}
