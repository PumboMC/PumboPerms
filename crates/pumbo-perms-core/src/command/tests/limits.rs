//! Staff limits of `/pp`.

use super::*;
use crate::config::LimitsCfg;

const MOD: &str = "00000000-0000-0000-0000-0000000000e1";

/// Groups `staff`, `mod`, `helper`, `vip` and `admin`, the track `ranks`
/// (`helper`, `mod`, `admin`), the moderator `Mod` online and the switches
/// as given.
fn limited(argument_permissions: bool, require_group_membership: bool) -> Engine {
    let mut e = engine();
    e.seen(MOD, "Mod", true, 0).unwrap();
    for g in ["staff", "mod", "helper", "vip", "admin"] {
        console(&mut e, &format!("creategroup {g}"));
    }
    for line in ["createtrack ranks", "track ranks append helper", "track ranks append mod", "track ranks append admin"]
    {
        console(&mut e, line);
    }
    e.cfg.limits = LimitsCfg { argument_permissions, require_group_membership };
    e
}

fn moderator() -> Who {
    Who::player("Mod", MOD, 0, "world")
}

/// The reply to the moderator.
fn as_mod(e: &mut Engine, line: &str) -> String {
    run_as(e, &moderator(), line, 1_000)
}

/// Sets nodes on the moderator from the console (`node` or `node false`).
fn grant(e: &mut Engine, nodes: &[&str]) {
    for n in nodes {
        console(e, &format!("user Mod permission set {n}"));
    }
}

fn refused(reply: &str, nodes: &str) -> bool {
    reply.contains(&format!("Not allowed without {nodes}."))
}

#[track_caller]
fn assert_refused(reply: &str, nodes: &str) {
    assert!(refused(reply, nodes), "expected a refusal naming {nodes}: {reply}");
}

#[track_caller]
fn assert_passed(reply: &str) {
    assert!(!reply.contains("Not allowed"), "{reply}");
}

#[test]
fn limits_are_off_by_default() {
    assert_eq!(CoreCfg::default().limits, LimitsCfg::default());
    assert!(!LimitsCfg::default().argument_permissions && !LimitsCfg::default().require_group_membership);
    let mut e = limited(false, false);
    grant(&mut e, &["pumbo.perms.user.parent.add"]);
    assert!(as_mod(&mut e, "user Mod parent add admin").contains("now inherits admin"));
}

#[test]
fn modify_needs_the_self_or_others_node() {
    let mut e = limited(true, false);
    grant(&mut e, &["pumbo.perms.user.parent.add", "pumbo.perms.user.parent.add.vip", "pumbo.perms.usecontext.global"]);
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), "pumbo.perms.modify.user.others");
    assert_refused(&as_mod(&mut e, "user Mod parent add vip"), "pumbo.perms.modify.user.self");
    grant(&mut e, &["pumbo.perms.modify.user.others"]);
    assert!(as_mod(&mut e, "user Steve parent add vip").contains("now inherits vip"));
    assert_refused(&as_mod(&mut e, "user Mod parent add vip"), "pumbo.perms.modify.user.self");
    grant(&mut e, &["pumbo.perms.modify.user.self"]);
    assert!(as_mod(&mut e, "user Mod parent add vip").contains("now inherits vip"));
}

#[test]
fn action_node_decides_before_the_general_one() {
    let mut e = limited(true, false);
    grant(&mut e, &["pumbo.perms.user.parent.add", "pumbo.perms.user.parent.add.vip", "pumbo.perms.usecontext.global"]);
    // The node of the action alone is enough.
    grant(&mut e, &["pumbo.perms.user.parent.add.modify.others"]);
    assert_passed(&as_mod(&mut e, "user Steve parent add vip"));
    // Set to false it refuses although the general node is true, and the refusal names it.
    grant(&mut e, &["pumbo.perms.modify.user.others", "pumbo.perms.user.parent.add.modify.others false"]);
    assert_refused(&as_mod(&mut e, "user Alex parent add vip"), "pumbo.perms.user.parent.add.modify.others");
    // An action without a node of its own follows the general one.
    grant(&mut e, &["pumbo.perms.user.parent.remove", "pumbo.perms.user.parent.remove.vip"]);
    assert_passed(&as_mod(&mut e, "user Steve parent remove vip"));
}

#[test]
fn self_check_matches_any_uuid_spelling() {
    let mut e = limited(true, false);
    grant(&mut e, &["pumbo.perms.user.info", "pumbo.perms.view.user.self"]);
    // A platform that names the moderator in capitals and without dashes.
    let shouting = |name: &str| {
        (name == "Mod").then(|| KnownName {
            uuid: MOD.replace('-', "").to_uppercase(),
            name: "Mod".into(),
            online: true,
        })
    };
    let env = Env { now: 1_000, online: &shouting, version: "0.1.0", version_rows: Vec::new() };
    let out = command::run(&mut e, &moderator(), &split_args("user Mod info"), &env);
    assert_passed(&out.reply.plain());
    assert!(out.reply.plain().contains("Mod"), "{}", out.reply.plain());
    // The same view of someone else is refused.
    assert_refused(&as_mod(&mut e, "user Steve info"), "pumbo.perms.view.user.others");
}

#[test]
fn group_and_track_writes_need_modify_nodes() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &[
            "pumbo.perms.group.setweight",
            "pumbo.perms.group.setdisplayname",
            "pumbo.perms.group.rename",
            "pumbo.perms.group.delete",
            "pumbo.perms.group.create",
            "pumbo.perms.track.edit",
            "pumbo.perms.track.delete",
            "pumbo.perms.track.create",
        ],
    );
    let group = "pumbo.perms.modify.group.vip";
    assert_refused(&as_mod(&mut e, "group vip setweight 5"), group);
    assert_refused(&as_mod(&mut e, "group vip setdisplayname VIP"), group);
    assert_refused(&as_mod(&mut e, "group vip rename vip2"), group);
    assert_refused(&as_mod(&mut e, "group vip delete"), group);
    assert_refused(&as_mod(&mut e, "deletegroup vip"), group);
    assert_refused(&as_mod(&mut e, "track ranks append vip"), "pumbo.perms.modify.track.ranks");
    assert_refused(&as_mod(&mut e, "track ranks clear"), "pumbo.perms.modify.track.ranks");
    assert_refused(&as_mod(&mut e, "deletetrack ranks"), "pumbo.perms.modify.track.ranks");
    // Creating has no target.
    assert_passed(&as_mod(&mut e, "creategroup fresh"));
    assert_passed(&as_mod(&mut e, "createtrack other"));
    grant(&mut e, &[group, "pumbo.perms.modify.track.ranks"]);
    assert!(as_mod(&mut e, "group vip setweight 5").contains("Weight of group vip set to 5."));
    assert_passed(&as_mod(&mut e, "track ranks append vip"));
    // A rename needs only the old name; the new one is another group.
    assert!(as_mod(&mut e, "group vip rename vip2").contains("vip2"));
    assert_refused(&as_mod(&mut e, "group vip2 setweight 6"), "pumbo.perms.modify.group.vip2");
    assert_refused(&as_mod(&mut e, "deletegroup vip2"), "pumbo.perms.modify.group.vip2");
    assert_passed(&as_mod(&mut e, "deletetrack ranks"));
}

#[test]
fn views_need_view_nodes() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &[
            "pumbo.perms.user.info",
            "pumbo.perms.user.permission.info",
            "pumbo.perms.user.permission.check",
            "pumbo.perms.user.parent.info",
            "pumbo.perms.user.meta.info",
            "pumbo.perms.group.info",
            "pumbo.perms.group.listmembers",
            "pumbo.perms.group.permission.info",
            "pumbo.perms.group.list",
            "pumbo.perms.track.info",
            "pumbo.perms.track.list",
            "pumbo.perms.log",
            "pumbo.perms.version",
        ],
    );
    for line in [
        "user Steve info",
        "user Steve permission info",
        "user Steve permission check fly",
        "user Steve parent info",
        "user Steve meta info",
    ] {
        assert_refused(&as_mod(&mut e, line), "pumbo.perms.view.user.others");
    }
    for line in ["group vip info", "group vip listmembers", "group vip permission info"] {
        assert_refused(&as_mod(&mut e, line), "pumbo.perms.view.group.vip");
    }
    assert_refused(&as_mod(&mut e, "track ranks info"), "pumbo.perms.view.track.ranks");
    // Lists, the log, the help and the version have no target.
    for line in ["groups", "tracks", "log", "help", "version", "user Steve help"] {
        assert_passed(&as_mod(&mut e, line));
    }
    grant(&mut e, &["pumbo.perms.view.user.others", "pumbo.perms.view.group.vip", "pumbo.perms.view.track.ranks"]);
    assert!(as_mod(&mut e, "user Steve info").contains("Steve"));
    assert!(as_mod(&mut e, "group vip info").contains("vip"));
    assert!(as_mod(&mut e, "track ranks info").contains("helper"));
}

#[test]
fn rights_come_from_the_data_before_the_change() {
    let mut e = limited(true, false);
    let node = "pumbo.perms.modify.user.self";
    grant(
        &mut e,
        &[
            "pumbo.perms.user.permission.set",
            "pumbo.perms.user.permission.set.pumbo.perms.modify.user.self",
            "pumbo.perms.usecontext.global",
        ],
    );
    assert_refused(&as_mod(&mut e, &format!("user Mod permission set {node}")), node);
    assert_eq!(e.decide(MOD, node, "world", 0, 1_000), None);
}

/// A source that makes Steve an admin.
fn admin_source() -> export::Snapshot {
    export::parse(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"admin"}}],
            "users":[{{"uuid":"{STEVE}","name":"Steve","data":{{"parents":[{{"group":"admin"}}]}}}}]}}"#
    ))
    .unwrap()
}

#[test]
fn console_and_import_stay_unlimited() {
    let mut e = limited(true, true);
    for line in [
        "user Steve parent add admin",
        "group vip permission set fly",
        "group admin parent add vip server=lobby",
        "track ranks insert vip 1",
        "promote Alex ranks",
        "user Steve meta setprefix 10 [A]",
        "group vip setweight 7",
        "user Alex clear",
    ] {
        assert_passed(&console(&mut e, line));
    }
    assert!(console(&mut e, "user Steve parent info").contains("admin"));
    // Whoever may import changes anyone, as the source says.
    grant(&mut e, &["pumbo.perms.import"]);
    let out = command::run(&mut e, &moderator(), &split_args("import run file"), &env(1_000));
    assert_eq!(out.action, Some(Action::Source("file".into(), SourceMode::Run)), "{}", out.reply.plain());
    console(&mut e, "user Steve parent remove admin");
    let out = import_source(&mut e, &moderator(), "file", Ok(("f1".into(), admin_source())), SourceMode::Run, 1_000);
    assert!(out.changed, "{}", out.reply.plain());
    assert!(console(&mut e, "user Steve parent info").contains("admin"));
}

/// Every view the console has of the data, the log and the import ledgers.
fn everything(e: &mut Engine) -> Vec<String> {
    let mut out: Vec<String> = ["user Steve info", "user Alex info", "user Mod info", "log"]
        .into_iter()
        .chain(["staff", "mod", "helper", "vip", "admin"].map(|g| format!("group {g} info")).iter().map(String::as_str))
        .map(|l| console(e, l))
        .collect();
    out.push(console(e, "track ranks info"));
    out.push(format!("{:?}", e.ledgers()));
    out
}

#[test]
fn refusal_leaves_data_ledger_and_log_unchanged() {
    let mut e = limited(true, true);
    import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), admin_source())), SourceMode::Run, 1_000);
    console(&mut e, "user Alex parent add vip");
    grant(
        &mut e,
        &[
            "pumbo.perms.user.parent.add",
            "pumbo.perms.user.parent.add.*",
            "pumbo.perms.user.parent.add.modify.others false",
            "pumbo.perms.user.clear",
            "pumbo.perms.group.setweight",
            "pumbo.perms.track.edit",
            "pumbo.perms.usecontext.global",
        ],
    );
    let before = everything(&mut e);
    for line in ["user Steve parent add vip", "user Alex clear", "group admin setweight 1", "track ranks clear"] {
        assert!(as_mod(&mut e, line).contains("Not allowed"), "{line}");
    }
    assert_eq!(everything(&mut e), before);
}
