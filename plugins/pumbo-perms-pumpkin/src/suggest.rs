//! Tab completion on Pumpkin: the command tree and where a suggestion goes in
//! the typed line.

/// Slots that may hold any character (`:` of a node, `=` of a context, `&` of
/// a prefix) take the rest of the line, and so does every `[...]` slot ending
/// in `...`: Pumpkin reads a single word only from `0-9 A-Z a-z _ - . +` or in
/// quotes. The words after one only show in the hint.
const FREE: &[&str] = &["node", "value", "text", "name|clear", "key"];

/// One word of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Word {
    /// Typed as it is.
    Fixed(String),
    /// An argument: its name, and whether it takes the rest of the line.
    Slot(String, bool),
}

/// A node of the command tree below the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub word: Word,
    /// A complete command ends here.
    pub executes: bool,
    pub children: Vec<Node>,
}

/// The tree of usage lines such as `user <player> parent add <group>
/// [context...]` ([`pumbo_perms_core::command::tree_lines`]): `a|b` are two
/// fixed words, `[...]` may be left out. A command ends where only optional
/// slots follow, and always at a slot that takes the rest of the line.
pub fn tree(lines: &[String]) -> Vec<Node> {
    let mut root = Vec::new();
    for line in lines {
        add(&mut root, &line.split(' ').collect::<Vec<_>>());
    }
    root
}

/// Adds `words` below a node; `true` when all of them may be left out.
fn add(children: &mut Vec<Node>, words: &[&str]) -> bool {
    let Some((first, rest)) = words.split_first() else { return true };
    let new = match first.strip_prefix('<').or_else(|| first.strip_prefix('[')) {
        Some(slot) => {
            let slot = slot.trim_end_matches(['>', ']']);
            let name = match slot {
                "true|false" => "value",
                other => other.trim_end_matches("..."),
            };
            vec![Word::Slot(name.to_string(), FREE.contains(&slot) || slot.ends_with("..."))]
        }
        None => first.split('|').map(|w| Word::Fixed(w.to_string())).collect(),
    };
    for word in new {
        let i = children.iter().position(|n| n.word == word).unwrap_or_else(|| {
            children.push(Node { word, executes: false, children: Vec::new() });
            children.len() - 1
        });
        if let Some(node) = children.get_mut(i) {
            let ends = add(&mut node.children, rest);
            node.executes |= ends || matches!(node.word, Word::Slot(_, true));
        }
    }
    first.starts_with('[') && add(children, rest)
}

/// Byte offset and length of the last word of `input`, the whole line the
/// client typed. Pumpkin 0.2 replaces only this word, so suggestions are bare
/// values. Words are split on plain spaces, like `split_args`.
pub fn last_word(input: &str) -> (usize, usize) {
    let start = input.rfind(' ').map_or(0, |i| i + 1);
    (start, input.len() - start)
}

#[cfg(test)]
mod tests {
    use pumbo_perms_core::config::CoreCfg;
    use pumbo_perms_core::engine::Engine;

    use super::*;

    fn pp_tree(disabled: &[&str]) -> Vec<Node> {
        let cfg = CoreCfg { disabled: disabled.iter().map(|d| d.to_string()).collect(), ..CoreCfg::default() };
        let lang = pumbo_common::lang::Lang::load(&crate::BUNDLES, "en", None).0;
        let e = Engine::new(Err(pumbo_common::store::StoreError::new("none")), cfg, lang);
        tree(&pumbo_perms_core::command::tree_lines(&e))
    }

    /// The node at `path` (fixed words, `<name>` for a slot).
    fn at<'a>(nodes: &'a [Node], path: &str) -> &'a Node {
        let mut found: Option<&Node> = None;
        let mut level = nodes;
        for step in path.split(' ') {
            let slot = step.strip_prefix('<').and_then(|s| s.strip_suffix('>'));
            let node = level
                .iter()
                .find(|n| match (&n.word, slot) {
                    (Word::Slot(name, _), Some(want)) => name == want,
                    (Word::Fixed(word), None) => word == step,
                    _ => false,
                })
                .unwrap_or_else(|| panic!("no {step} in {path}"));
            level = &node.children;
            found = Some(node);
        }
        found.unwrap()
    }

    fn names(node: &Node) -> Vec<String> {
        node.children
            .iter()
            .map(|n| match &n.word {
                Word::Fixed(w) => w.clone(),
                Word::Slot(name, _) => format!("<{name}>"),
            })
            .collect()
    }

    #[test]
    fn subcommands_are_fixed_words_and_complete_commands_end_the_branch() {
        let t = pp_tree(&[]);
        let root: Vec<&Word> = t.iter().map(|n| &n.word).collect();
        for w in ["user", "group", "track", "groups", "tracks", "creategroup", "promote", "log", "import", "help"] {
            assert!(root.contains(&&Word::Fixed(w.into())), "{w}");
        }
        // Right after the root only the words of the help and one slot for any other word.
        let slots: Vec<&Word> = root.iter().copied().filter(|w| matches!(w, Word::Slot(..))).collect();
        assert_eq!(slots, [&Word::Slot("args".into(), true)]);
        for other in ["u", "info", "listgroups", "editor", "?"] {
            assert!(!root.contains(&&Word::Fixed(other.into())), "{other}");
        }
        // `/pp track x create ` offers nothing more, and no greedy argument is left to hint at.
        let create = at(&t, "track <name> create");
        assert!(create.executes && create.children.is_empty());
        let user = at(&t, "user <player>");
        assert!(user.executes, "lists the player's commands");
        for w in ["info", "permission", "parent", "meta", "promote", "demote", "clear", "help"] {
            assert!(names(user).contains(&w.to_string()), "{w}: {:?}", names(user));
        }
        let perm = at(&t, "group <name> permission");
        assert_eq!(names(perm), ["help", "<args>", "info", "set", "settemp", "unset", "unsettemp", "check", "clear"]);
        assert!(!at(&t, "user").executes && !at(&t, "user <player> permission set").executes);
        assert!(at(&t, "user <player> info").executes && at(&t, "user <player> info <page>").executes);
        assert_eq!(names(at(&t, "import")), ["<file>", "run", "preview", "undo"]);
        assert!(at(&t, "import undo <source>").executes);
    }

    #[test]
    fn free_text_takes_the_rest_of_the_line_and_hints_what_follows() {
        let t = pp_tree(&[]);
        let node = at(&t, "user <player> permission set <node>");
        assert_eq!(node.word, Word::Slot("node".into(), true));
        assert!(node.executes);
        assert_eq!(names(node), ["<value>", "<context>"]);
        // settemp needs the duration, but the node takes it along, so it runs there too.
        let temp = at(&t, "user <player> permission settemp <node>");
        assert!(temp.executes);
        assert_eq!(names(temp), ["<value>", "<duration>"]);
        assert!(at(&t, "user <player> permission settemp <node> <duration>").executes);
        assert!(!at(&t, "user <player> permission settemp <node> <value>").executes);
        let prefix = at(&t, "group <name> meta setprefix <priority>");
        assert_eq!(prefix.word, Word::Slot("priority".into(), false));
        assert_eq!(at(&t, "group <name> meta setprefix <priority> <text>").word, Word::Slot("text".into(), true));
        assert_eq!(at(&t, "user <player> parent add <group>").word, Word::Slot("group".into(), false));
        assert_eq!(at(&t, "user <player> parent add <group> <context>").word, Word::Slot("context".into(), true));
        assert_eq!(at(&t, "group <name> setdisplayname <name|clear>").word, Word::Slot("name|clear".into(), true));
        // Meta keys may have any character too.
        let key = at(&t, "user <player> meta set <key>");
        assert!(key.word == Word::Slot("key".into(), true) && key.executes);
        assert_eq!(names(key), ["<value>"]);
        assert_eq!(at(&t, "group <name> meta unset <key>").word, Word::Slot("key".into(), true));
    }

    /// Every level of fixed words also takes any other word with the rest of
    /// the line (`u`, `perm`, `list`, `?`, `editor`, words in upper case),
    /// which the core runs as on the proxy; a level with an argument has none.
    #[test]
    fn every_level_of_words_takes_any_other_word() {
        fn catch_alls(nodes: &[Node]) -> usize {
            let here = nodes.iter().filter(|n| n.word == Word::Slot("args".into(), true)).count();
            here + nodes.iter().map(|n| catch_alls(&n.children)).sum::<usize>()
        }
        let t = pp_tree(&[]);
        let mut levels = vec!["track <name>".to_string()];
        for holder in ["user <player>", "group <name>"] {
            levels.push(holder.into());
            levels.extend(["permission", "parent", "meta"].map(|w| format!("{holder} {w}")));
        }
        for level in &levels {
            let args = at(&t, &format!("{level} <args>"));
            assert!(args.executes && args.children.is_empty(), "{level}");
        }
        assert_eq!(catch_alls(&t), levels.len() + 1, "and one after the root");
        for level in ["user", "import", "user <player> permission set"] {
            assert!(!names(at(&t, level)).contains(&"<args>".to_string()), "{level}");
        }
    }

    #[test]
    fn argument_names_are_unique_on_every_path() {
        fn walk(nodes: &[Node], seen: &mut Vec<String>) {
            for n in nodes {
                let pushed = match &n.word {
                    Word::Slot(name, _) => {
                        assert!(!seen.contains(name), "{name} twice on {seen:?}");
                        seen.push(name.clone());
                        true
                    }
                    Word::Fixed(_) => false,
                };
                walk(&n.children, seen);
                if pushed {
                    seen.pop();
                }
            }
        }
        walk(&pp_tree(&[]), &mut Vec::new());
    }

    #[test]
    fn switched_off_subcommands_are_left_out() {
        let t = pp_tree(&["user", "editor"]);
        assert!(!t.iter().any(|n| n.word == Word::Fixed("user".into())));
        assert!(t.iter().any(|n| n.word == Word::Fixed("group".into())));
        // Typed anyway, it reaches the core, which refuses it as on the proxy.
        assert!(t.iter().any(|n| n.word == Word::Slot("args".into(), true)));
    }

    #[test]
    fn suggestion_replaces_only_the_last_word() {
        assert_eq!(last_word("/pp user "), (9, 0));
        assert_eq!(last_word("/pp user Qh"), (9, 2));
        assert_eq!(last_word("/pp user Qhash parent add "), (26, 0));
        assert_eq!(last_word("/pp  group  vi"), (12, 2));
    }
}
