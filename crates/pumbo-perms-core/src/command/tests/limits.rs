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
    // Under membership no one joins a group they are not in yet.
    e.cfg.limits.require_group_membership = true;
    grant(&mut e, &[node, "pumbo.perms.user.parent.add", "pumbo.perms.user.parent.add.admin"]);
    assert!(as_mod(&mut e, "user Mod parent add admin").contains("Not allowed: you are not in admin."));
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
    // Every node but a few, and no group.
    grant(
        &mut e,
        &[
            "pumbo.perms.*",
            "pumbo.perms.user.clear.modify.others false",
            "pumbo.perms.group.setweight.modify.admin false",
            "pumbo.perms.track.edit.modify.ranks false",
            "pumbo.perms.user.permission.set.fly false",
            "pumbo.perms.user.permission.set.usecontext.server.lobby false",
        ],
    );
    let before = everything(&mut e);
    for line in [
        "user Alex clear",
        "group admin setweight 1",
        "track ranks clear",
        "user Steve permission set fly",
        "user Steve permission set kick server=lobby",
        "user Steve parent add vip",
        "user Alex parent remove vip",
        "group vip permission set kick",
        "promote Alex ranks",
    ] {
        assert!(as_mod(&mut e, line).contains("Not allowed"), "{line}");
    }
    assert_eq!(everything(&mut e), before);
}

#[test]
fn arguments_need_their_nodes() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &[
            "pumbo.perms.user.permission.set",
            "pumbo.perms.user.parent.add",
            "pumbo.perms.user.meta.set",
            "pumbo.perms.user.meta.unset",
            "pumbo.perms.user.promote",
            "pumbo.perms.user.demote",
            "pumbo.perms.modify.user.others",
            "pumbo.perms.usecontext.global",
        ],
    );
    // A node as it is stored, with its namespace.
    let set = "pumbo.perms.user.permission.set";
    let gamemode = "user Steve permission set minecraft:command.gamemode";
    assert_refused(&as_mod(&mut e, gamemode), &format!("{set}.minecraft:command.gamemode"));
    grant(&mut e, &[&format!("{set}.minecraft.command.gamemode")]);
    assert_refused(&as_mod(&mut e, gamemode), &format!("{set}.minecraft:command.gamemode"));
    grant(&mut e, &[&format!("{set}.minecraft:command.gamemode")]);
    assert_passed(&as_mod(&mut e, gamemode));
    assert_refused(
        &as_mod(&mut e, "user Steve permission set minecraft:command.kick"),
        &format!("{set}.minecraft:command.kick"),
    );
    grant(&mut e, &[&format!("{set}.minecraft:command.*")]);
    assert_passed(&as_mod(&mut e, "user Steve permission set minecraft:command.kick"));
    assert_refused(&as_mod(&mut e, "user Steve permission set pumbobans:ban"), &format!("{set}.pumbo.bans.ban"));
    // A group, a meta key and prefixes and suffixes, in lower case.
    assert_refused(&as_mod(&mut e, "user Steve parent add VIP"), "pumbo.perms.user.parent.add.vip");
    grant(
        &mut e,
        &["pumbo.perms.user.parent.add.vip", "pumbo.perms.user.meta.set.color", "pumbo.perms.user.meta.set.prefix"],
    );
    assert_passed(&as_mod(&mut e, "user Steve parent add VIP"));
    assert_refused(&as_mod(&mut e, "user Steve meta set Size 2"), "pumbo.perms.user.meta.set.size");
    assert_passed(&as_mod(&mut e, "user Steve meta set Color red"));
    assert_passed(&as_mod(&mut e, "user Steve meta setprefix 10 [V]"));
    assert_refused(&as_mod(&mut e, "user Steve meta addsuffix 10 [V]"), "pumbo.perms.user.meta.set.suffix");
    assert_refused(&as_mod(&mut e, "user Steve meta removeprefix 10"), "pumbo.perms.user.meta.unset.prefix");
    assert_refused(&as_mod(&mut e, "user Steve meta unset color"), "pumbo.perms.user.meta.unset.color");
    // Promote names the group the player goes up to; demote the one the player
    // leaves and the one below it, which the player gets.
    console(&mut e, "user Alex parent add helper");
    assert_refused(&as_mod(&mut e, "promote Alex ranks"), "pumbo.perms.user.promote.ranks.mod");
    grant(&mut e, &["pumbo.perms.user.promote.ranks.mod"]);
    assert!(as_mod(&mut e, "promote Alex ranks").contains("Promoted"));
    assert_refused(&as_mod(&mut e, "demote Alex ranks"), "pumbo.perms.user.demote.ranks.helper");
    grant(&mut e, &["pumbo.perms.user.demote.ranks.helper"]);
    assert_refused(&as_mod(&mut e, "demote Alex ranks"), "pumbo.perms.user.demote.ranks.mod");
    grant(&mut e, &["pumbo.perms.user.demote.ranks.mod"]);
    assert!(as_mod(&mut e, "demote Alex ranks").contains("Demoted"));
    // The wildcard of an action covers every argument; an explicit false refuses one.
    grant(&mut e, &["pumbo.perms.user.parent.add.*", "pumbo.perms.user.parent.add.admin false"]);
    assert_passed(&as_mod(&mut e, "user Steve parent add staff"));
    assert_refused(&as_mod(&mut e, "user Steve parent add admin"), "pumbo.perms.user.parent.add.admin");
}

#[test]
fn global_change_needs_usecontext_global() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &[
            "pumbo.perms.user.parent.add",
            "pumbo.perms.user.parent.add.vip",
            "pumbo.perms.user.permission.clear",
            "pumbo.perms.user.clear",
            "pumbo.perms.modify.user.others",
        ],
    );
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), "pumbo.perms.usecontext.global");
    assert_passed(&as_mod(&mut e, "user Steve parent add vip server=lobby"));
    assert_refused(&as_mod(&mut e, "user Steve permission clear"), "pumbo.perms.usecontext.global");
    assert_passed(&as_mod(&mut e, "user Steve permission clear server=lobby"));
    // Clearing a player takes every context at once: a change without one.
    assert_refused(&as_mod(&mut e, "user Steve clear"), "pumbo.perms.usecontext.global");
    // The node of the action decides first.
    grant(&mut e, &["pumbo.perms.user.clear.usecontext.global"]);
    assert_passed(&as_mod(&mut e, "user Steve clear"));
    grant(&mut e, &["pumbo.perms.usecontext.global", "pumbo.perms.user.parent.add.usecontext.global false"]);
    assert_passed(&as_mod(&mut e, "user Steve permission clear"));
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), "pumbo.perms.user.parent.add.usecontext.global");
}

#[test]
fn context_is_refused_only_when_set_false() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &["pumbo.perms.user.parent.add", "pumbo.perms.user.parent.add.vip", "pumbo.perms.modify.user.others"],
    );
    assert_passed(&as_mod(&mut e, "user Steve parent add vip server=lobby"));
    grant(&mut e, &["pumbo.perms.usecontext.server.survival false"]);
    let survival = "pumbo.perms.usecontext.server.survival";
    assert_refused(&as_mod(&mut e, "user Steve parent add vip server=survival"), survival);
    assert_refused(&as_mod(&mut e, "user Steve parent add vip server=survival world=nether"), survival);
    assert_passed(&as_mod(&mut e, "user Steve parent add vip world=nether"));
    // The node of the action decides before the general one.
    grant(&mut e, &["pumbo.perms.user.parent.add.usecontext.server.survival"]);
    assert_passed(&as_mod(&mut e, "user Steve parent add vip server=survival"));
    grant(&mut e, &["pumbo.perms.user.parent.add.usecontext.world.end false"]);
    assert_refused(
        &as_mod(&mut e, "user Steve parent add vip world=end"),
        "pumbo.perms.user.parent.add.usecontext.world.end",
    );
}

#[test]
fn action_wildcard_opens_its_targets_and_contexts() {
    let mut e = limited(true, false);
    console(&mut e, "user Mod parent add mod");
    grant(
        &mut e,
        &[
            "pumbo.perms.user.parent.add",
            "pumbo.perms.user.parent.add.*",
            "pumbo.perms.group.permission.set",
            "pumbo.perms.group.permission.set.*",
        ],
    );
    // Oneself, one's own group and changes without a context.
    assert_passed(&as_mod(&mut e, "user Mod parent add admin"));
    assert_passed(&as_mod(&mut e, "group mod permission set fly"));
    // An explicit false on a target closes it again.
    grant(
        &mut e,
        &["pumbo.perms.user.parent.add.modify.self false", "pumbo.perms.group.permission.set.modify.mod false"],
    );
    console(&mut e, "user Mod parent remove admin");
    assert_refused(&as_mod(&mut e, "user Mod parent add admin"), "pumbo.perms.user.parent.add.modify.self");
    assert_refused(&as_mod(&mut e, "group mod permission set kick"), "pumbo.perms.group.permission.set.modify.mod");
    assert_passed(&as_mod(&mut e, "user Steve parent add admin"));
    // Without the wildcard the same kind of change needs the context node.
    grant(
        &mut e,
        &[
            "pumbo.perms.user.permission.set",
            "pumbo.perms.user.permission.set.modify.others",
            "pumbo.perms.user.permission.set.fly",
        ],
    );
    assert_refused(&as_mod(&mut e, "user Steve permission set fly"), "pumbo.perms.usecontext.global");
}

#[test]
fn refusal_names_every_missing_node() {
    let mut e = limited(true, false);
    grant(
        &mut e,
        &["pumbo.perms.user.parent.add", "pumbo.perms.modify.user.others", "pumbo.perms.usecontext.server.lobby false"],
    );
    let both = "pumbo.perms.usecontext.global, pumbo.perms.user.parent.add.vip";
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), both);
    let both = "pumbo.perms.usecontext.server.lobby, pumbo.perms.user.parent.add.vip";
    assert_refused(&as_mod(&mut e, "user Steve parent add vip server=lobby"), both);
    // Missing groups are the second line of the same refusal.
    e.cfg.limits.require_group_membership = true;
    let out = as_mod(&mut e, "user Steve parent add vip server=lobby");
    assert!(refused(&out, both) && out.contains("Not allowed: you are not in vip in server=lobby."), "{out}");
    grant(
        &mut e,
        &["pumbo.perms.group.parent.add", "pumbo.perms.group.parent.add.*", "pumbo.perms.modify.group.admin"],
    );
    let out = as_mod(&mut e, "group admin parent add vip server=survival");
    assert!(out.contains("Not allowed: you are not in admin in server=survival, vip in server=survival."), "{out}");
    assert!(!out.contains("without"), "{out}");
}

#[test]
fn checks_come_before_the_edit() {
    let mut e = limited(true, false);
    console(&mut e, "user Steve parent add vip");
    console(&mut e, "user Alex parent add admin");
    grant(
        &mut e,
        &[
            "pumbo.perms.user.parent.add",
            "pumbo.perms.user.permission.unset",
            "pumbo.perms.user.promote",
            "pumbo.perms.group.parent.add",
        ],
    );
    // The target first, before anything about the data.
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), "pumbo.perms.modify.user.others");
    assert_refused(&as_mod(&mut e, "user Steve permission unset fly"), "pumbo.perms.modify.user.others");
    assert_refused(&as_mod(&mut e, "promote Alex ranks"), "pumbo.perms.modify.user.others");
    // Then the context and the argument, still before "already" and "has no".
    grant(&mut e, &["pumbo.perms.modify.user.others"]);
    let add = "pumbo.perms.usecontext.global, pumbo.perms.user.parent.add.vip";
    assert_refused(&as_mod(&mut e, "user Steve parent add vip"), add);
    let unset = "pumbo.perms.usecontext.global, pumbo.perms.user.permission.unset.fly";
    assert_refused(&as_mod(&mut e, "user Steve permission unset fly"), unset);
    // A step off the end of the track: after the target and the context, before the argument.
    assert_refused(&as_mod(&mut e, "promote Alex ranks"), "pumbo.perms.usecontext.global");
    grant(&mut e, &["pumbo.perms.usecontext.global"]);
    assert!(as_mod(&mut e, "promote Alex ranks").contains("already on the highest group"));
    // Membership too: after the track, before the edit.
    e.cfg.limits.require_group_membership = true;
    assert!(as_mod(&mut e, "promote Alex ranks").contains("already on the highest group"));
    assert!(as_mod(&mut e, "user Steve parent add vip").contains("Not allowed: you are not in vip."));
    e.cfg.limits.require_group_membership = false;
    // A loop of groups is an error of the edit, after the checks.
    console(&mut e, "group admin parent add helper");
    assert!(console(&mut e, "group helper parent add admin").contains("loop"));
    assert_refused(&as_mod(&mut e, "group helper parent add admin"), "pumbo.perms.modify.group.helper");
}

fn not_in(groups: &str) -> String {
    format!("Not allowed: you are not in {groups}.")
}

#[test]
fn membership_is_needed_to_give_take_or_edit_a_group() {
    let mut e = limited(false, true);
    console(&mut e, "user Mod parent add staff");
    console(&mut e, "user Steve parent add vip");
    grant(&mut e, &["pumbo.perms.user.parent.*", "pumbo.perms.group.*"]);
    assert_passed(&as_mod(&mut e, "user Alex parent add staff"));
    assert!(as_mod(&mut e, "user Alex parent add vip").contains(&not_in("vip")));
    assert!(as_mod(&mut e, "user Steve parent remove vip").contains(&not_in("vip")));
    assert_passed(&as_mod(&mut e, "user Alex parent remove staff"));
    assert_passed(&as_mod(&mut e, "user Alex parent addtemp staff 1h"));
    for line in ["group vip permission set fly", "group vip meta set color red", "group vip meta setprefix 5 [V]"] {
        assert!(as_mod(&mut e, line).contains(&not_in("vip")), "{line}");
    }
    assert_passed(&as_mod(&mut e, "group staff permission set fly"));
    assert_passed(&as_mod(&mut e, "group staff meta setprefix 5 [S]"));
    // A group's parents: the group that changes and the one given or taken.
    assert!(as_mod(&mut e, "group staff parent add vip").contains(&not_in("vip")));
    assert!(as_mod(&mut e, "group vip parent add staff").contains(&not_in("vip")));
}

#[test]
fn membership_counts_inherited_groups() {
    let mut e = limited(false, true);
    console(&mut e, "group mod parent add helper");
    console(&mut e, "user Mod parent add mod");
    grant(&mut e, &["pumbo.perms.user.parent.add"]);
    assert_passed(&as_mod(&mut e, "user Alex parent add helper"));
    assert_passed(&as_mod(&mut e, "user Alex parent add mod"));
    assert!(as_mod(&mut e, "user Alex parent add admin").contains(&not_in("admin")));
    // `default` where the engine gives it: not next to a group without a context.
    assert!(as_mod(&mut e, "user Alex parent add default").contains(&not_in("default")));
    console(&mut e, "user Mod parent remove mod");
    console(&mut e, "user Mod parent add mod server=lobby");
    assert_passed(&as_mod(&mut e, "user Alex parent add default"));
}

#[test]
fn membership_follows_the_contexts_of_the_change() {
    let mut e = limited(false, true);
    console(&mut e, "user Mod parent add staff server=survival");
    grant(&mut e, &["pumbo.perms.user.parent.add", "pumbo.perms.group.permission.set"]);
    assert_passed(&as_mod(&mut e, "user Alex parent add staff server=survival"));
    assert_passed(&as_mod(&mut e, "user Alex parent add staff server=survival world=nether"));
    assert!(as_mod(&mut e, "user Alex parent add staff").contains(&not_in("staff")));
    assert!(as_mod(&mut e, "user Alex parent add staff server=lobby").contains(&not_in("staff in server=lobby")));
    assert_passed(&as_mod(&mut e, "group staff permission set fly server=survival"));
    assert!(as_mod(&mut e, "group staff permission set fly").contains(&not_in("staff")));
    // A change without a context is one everywhere, even on the server called survival.
    e.cfg.context.server = "survival".into();
    assert!(as_mod(&mut e, "user Alex parent add staff").contains(&not_in("staff")));
    // Membership in a server group covers its servers.
    console(&mut e, "user Mod parent add vip group=smp");
    e.server_groups.insert("survival".into(), vec!["smp".into()]);
    assert_passed(&as_mod(&mut e, "user Alex parent add vip server=survival"));
    assert_passed(&as_mod(&mut e, "user Alex parent add vip group=smp"));
    assert!(as_mod(&mut e, "user Alex parent add vip server=lobby").contains(&not_in("vip in server=lobby")));
    // A temporary membership counts while it lasts.
    console(&mut e, "user Mod parent addtemp admin 1h");
    assert_passed(&as_mod(&mut e, "user Alex parent add admin"));
    console(&mut e, "user Alex parent remove admin");
    let later = run_as(&mut e, &moderator(), "user Alex parent add admin", 1_000 + 3_600_001);
    assert!(later.contains(&not_in("admin")), "{later}");
}

#[test]
fn membership_is_checked_only_where_listed() {
    let mut e = limited(false, true);
    console(&mut e, "user Mod parent add staff");
    console(&mut e, "user Mod parent add mod");
    console(&mut e, "user Steve parent add admin");
    grant(&mut e, &["pumbo.perms.user.*"]);
    // `parent set` checks the group it sets, not the ones it takes away.
    assert_passed(&as_mod(&mut e, "user Steve parent set staff"));
    assert!(as_mod(&mut e, "user Steve parent set admin").contains(&not_in("admin")));
    // Clearing a player names no group.
    console(&mut e, "user Steve parent add admin");
    assert_passed(&as_mod(&mut e, "user Steve clear"));
    // Promote: the group the player goes up to; demote: the one the player
    // leaves and the one below it, which the player gets.
    console(&mut e, "user Alex parent add helper");
    assert_passed(&as_mod(&mut e, "promote Alex ranks"));
    assert!(as_mod(&mut e, "promote Alex ranks").contains(&not_in("admin")));
    assert!(as_mod(&mut e, "demote Alex ranks").contains(&not_in("helper")));
    console(&mut e, "user Mod parent add helper");
    assert_passed(&as_mod(&mut e, "demote Alex ranks"));
    console(&mut e, "user Alex parent set admin");
    assert!(as_mod(&mut e, "demote Alex ranks").contains(&not_in("admin")));
}

#[test]
fn demote_gives_only_groups_the_staff_may_give() {
    // A track that puts admin below helper, made by staff in helper who may edit tracks.
    let tracks = ["pumbo.perms.track.create", "pumbo.perms.track.edit", "pumbo.perms.user.parent.add"];
    let chain = ["createtrack x", "track x append admin", "track x append helper", "user Steve parent add helper"];
    for (argument_permissions, extra) in [
        (false, &[][..]),
        (
            true,
            &[
                "pumbo.perms.modify.user.others",
                "pumbo.perms.usecontext.global",
                "pumbo.perms.user.parent.add.helper",
                "pumbo.perms.modify.track.x",
                "pumbo.perms.user.demote.x.helper",
            ][..],
        ),
    ] {
        let mut e = limited(argument_permissions, true);
        console(&mut e, "user Mod parent add helper");
        grant(&mut e, &tracks);
        grant(&mut e, &["pumbo.perms.user.demote"]);
        grant(&mut e, extra);
        assert!(as_mod(&mut e, "user Steve parent add admin").contains("Not allowed"));
        for line in chain {
            assert_passed(&as_mod(&mut e, line));
        }
        let out = as_mod(&mut e, "demote Steve x");
        assert!(out.contains("Not allowed") && !out.contains("Demoted"), "{argument_permissions}: {out}");
        let groups = console(&mut e, "user Steve info");
        assert!(!groups.contains("admin"), "{argument_permissions}: {groups}");
    }
}

#[test]
fn weight_name_rename_and_delete_skip_membership() {
    let mut e = limited(false, true);
    grant(&mut e, &["pumbo.perms.group.*", "pumbo.perms.track.*"]);
    for line in [
        "group vip setweight 5",
        "group vip setdisplayname VIP",
        "group vip rename vip2",
        "deletegroup vip2",
        "track ranks append staff",
        "track ranks remove staff",
        "deletetrack ranks",
        "creategroup fresh",
        "createtrack fresh",
    ] {
        assert_passed(&as_mod(&mut e, line));
    }
}

#[test]
fn membership_does_not_open_editing_the_own_group() {
    let mut e = limited(true, true);
    console(&mut e, "user Mod parent add staff");
    grant(
        &mut e,
        &["pumbo.perms.group.permission.set", "pumbo.perms.group.permission.set.fly", "pumbo.perms.usecontext.global"],
    );
    assert_refused(&as_mod(&mut e, "group staff permission set fly"), "pumbo.perms.modify.group.staff");
    grant(&mut e, &["pumbo.perms.modify.group.staff"]);
    assert_passed(&as_mod(&mut e, "group staff permission set fly"));
}

#[test]
fn wildcards_and_operators_are_not_membership() {
    let mut e = limited(false, true);
    grant(&mut e, &["*"]);
    assert!(as_mod(&mut e, "user Steve parent add admin").contains(&not_in("admin")));
    // An operator gets every PumboPerms node, not a group.
    let op = Who::player("Op", "00000000-0000-0000-0000-0000000000e2", 4, "world");
    assert!(run_as(&mut e, &op, "user Steve parent add admin", 1_000).contains(&not_in("admin")));
    // With argument permissions alone the operator passes.
    e.cfg.limits = LimitsCfg { argument_permissions: true, require_group_membership: false };
    assert!(run_as(&mut e, &op, "user Steve parent add admin", 1_000).contains("now inherits admin"));
}

#[test]
fn new_group_needs_the_console_under_membership() {
    let mut e = limited(false, true);
    grant(&mut e, &["*"]);
    assert_passed(&as_mod(&mut e, "creategroup fresh"));
    assert!(as_mod(&mut e, "user Steve parent add fresh").contains(&not_in("fresh")));
    assert!(as_mod(&mut e, "group fresh permission set fly").contains(&not_in("fresh")));
    assert!(as_mod(&mut e, "group admin parent add fresh").contains(&not_in("admin, fresh")));
    assert!(console(&mut e, "user Steve parent add fresh").contains("now inherits fresh"));
    assert!(console(&mut e, "group fresh permission set fly").contains("Set fly"));
}
