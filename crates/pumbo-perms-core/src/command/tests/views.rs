//! Views, help pages and how replies look in chat and in the console.

use super::*;

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
    let usage = command::run(&mut e, &Who::console(), &split_args("creategroup"), &env(0)).reply;
    assert_eq!(color_of(&usage, "/pp creategroup"), Some(style::accent::PERMS), "{:?}", usage.lines[0].segments);
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
fn player_help_is_grouped() {
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
fn user_group_and_track_without_a_name_list_their_commands() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let show = |e: &mut Engine, who: &Who, line: &str| command::run(e, who, &split_args(line), &env(0)).reply;
    let user = show(&mut e, &op, "user");
    let plain = user.plain();
    assert!(plain.contains("/pp user <player>"), "{plain}");
    for part in ["info [page]", "permission ...", "parent ...", "meta ...", "promote <track>"] {
        assert!(plain.contains(part), "{part}: {plain}");
    }
    // A click on a line types the command up to the player.
    for row in ["> info [page]", "> permission ..."] {
        let line = user.lines.iter().find(|l| l.plain().contains(row)).expect(row);
        assert!(line.segments.iter().any(|s| s.click == Some(Click::Suggest("/pp user ".into()))), "{line:?}");
    }
    let group = show(&mut e, &op, "group").plain();
    assert!(group.contains("/pp group <name>") && group.contains("setweight <weight>"), "{group}");
    let track = show(&mut e, &op, "track").plain();
    assert!(track.contains("/pp track <name>") && track.contains("append <group>"), "{track}");
    // The console gets the plain list, a player without the rights the refusal.
    assert!(console(&mut e, "user").contains("/pp user <player> info [page]"));
    let alex = Who::player("Alex", ALEX, 0, "world");
    assert!(!show(&mut e, &alex, "user").plain().contains("info [page]"));
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
