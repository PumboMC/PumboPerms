//! Replies in the shared Pumbo style ([`pumbo_common::style`],
//! [`pumbo_common::rich`], [`pumbo_common::help`]) and the pieces of the info
//! views (`/pp user <player> info`, `/pp group <name> info`).
//!
//! Views are lines of `&`-coded text parsed into [`Text`]; the colours below
//! are the codes of the shared palette.

pub use pumbo_common::rich::{Click, Line, Segment, Text};
pub use pumbo_common::style::{command, error, info, success, usage, value, warn};

use pumbo_common::lang::Lang;
use pumbo_common::style::{self, Tone};
use pumbo_common::text::Args;

/// `style::BRAND`.
pub const BRAND: &str = "&#f28c28";
/// `style::INFO`: labels, descriptions.
pub const INFO: &str = "&7";
/// `style::VALUE`: names, numbers, nodes.
pub const VALUE: &str = "&f";
/// `style::MUTED`: bullets, separators, contexts.
pub const MUTED: &str = "&8";
/// `style::WARN`: end times.
pub const WARN: &str = "&e";
/// `style::ERROR`: broken references.
pub const ERROR: &str = "&c";
/// `true`.
pub const YES: &str = "&a";
/// `false`.
pub const NO: &str = "&c";

/// An unknown subcommand; a click opens the help.
pub fn unknown(lang: &Lang, name: &str, help_command: &str) -> Text {
    style::unknown_subcommand(lang, name, help_command)
}

/// `true` / `false` in their colours, for a message argument.
pub fn boolean(v: bool) -> String {
    if v { format!("{YES}true&r") } else { format!("{NO}false&r") }
}

/// The first line of a view: the prefix and a title in the information tone.
pub fn header(lang: &Lang, key: &str, args: &Args) -> Text {
    style::message(lang, Tone::Info, key, args)
}

/// A labelled row of a view: `  Label: value` (`value` with `&` codes).
pub fn row(indent: usize, label: &str, value: &str) -> Line {
    let pad = "  ".repeat(indent);
    if label.is_empty() {
        return Line::parse(&format!("{pad}{value}"));
    }
    Line::parse(&format!("{pad}{INFO}{label}{MUTED}: {VALUE}{value}"))
}

/// Text in one colour, a tooltip for example.
pub fn hint(text: &str) -> Text {
    Text::parse(&format!("{INFO}{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumbo_common::lang::{Bundle, COMMON};

    const B: Bundle =
        Bundle { name: "t", files: &[("en", "prefix: \"&#f28c28PumboPerms &8» \"\nok: \"Group {0} created.\"\n")] };

    #[test]
    fn replies_and_rows() {
        let (l, _) = Lang::load(&[COMMON, B], "en", None);
        assert_eq!(success(&l, "ok", &Args::new().arg(value("vip"))).plain(), "PumboPerms » Group vip created.");
        assert_eq!(header(&l, "ok", &Args::new().arg("x")).plain(), "PumboPerms » Group x created.");
        assert_eq!(row(1, "Weight", "10").plain(), "  Weight: 10");
        assert_eq!(row(2, "", "&fx").plain(), "    x");
        assert!(boolean(true).contains("true") && boolean(false).contains("false"));
        assert!(unknown(&l, "foo", "/pp help").plain().contains("foo"));
    }
}
