//! Replies in the shared Pumbo style ([`pumbo_common::style`],
//! [`pumbo_common::rich`], [`pumbo_common::help`]) and the pieces of the info
//! views (`/pp user <player> info`, `/pp group <name> info`).
//!
//! Views are lines of `&`-coded text parsed into [`Text`]; the colours below
//! are the codes of the shared palette. A view has a title in the PumboPerms
//! yellow, `Label: value` rows (labels light orange, values white), section
//! headings in yellow with items under a dimmed `•`, contexts and end times in
//! grey, and `none` in dark grey.

pub use pumbo_common::rich::{Click, Line, Text};
pub use pumbo_common::style::{error, info, success, value, warn};

use pumbo_common::lang::Lang;
use pumbo_common::style;
use pumbo_common::text::Args;

/// `style::BRAND`: page arrows.
pub const BRAND: &str = "&#f28c28";
/// `style::accent::PERMS`: section headings (titles get it from [`header`]).
pub const ACCENT: &str = "&#ffd54f";
/// `style::LABEL`: field labels, meta keys, the target of a log entry.
pub const LABEL: &str = "&#ffc27a";
/// `style::INFO`: contexts, end times, the text of a prefix in quotes, tooltips.
pub const INFO: &str = "&7";
/// `style::VALUE`: names, numbers, nodes.
pub const VALUE: &str = "&f";
/// `style::MUTED`: bullets, separators, `none`.
pub const MUTED: &str = "&8";
/// `style::WARN`: warnings inside views.
pub const WARN: &str = "&e";
/// `style::ERROR`: broken references.
pub const ERROR: &str = "&c";
/// `true`.
pub const YES: &str = "&a";
/// `false`.
pub const NO: &str = "&c";

/// Chat text with the PumboPerms prefix in front of every line (`[PP] ...`);
/// lines that already start with it stay as they are. An
/// empty prefix leaves the text alone.
pub fn tagged(lang: &Lang, text: Text) -> Text {
    let tag = style::prefix(lang);
    let plain = tag.plain();
    if plain.is_empty() {
        return text;
    }
    let lines =
        text.lines.into_iter().map(|l| if l.plain().starts_with(&plain) { l } else { tag.clone().append(l) }).collect();
    Text { lines }
}

/// A view in chat between two bars in the colours of `[PP]`
/// ([`style::bar`]), its title in the middle of the top one. A reply of one
/// line, or text that already has its bars (the help, a page bar at the end),
/// keeps them as they are. A title too wide for the bar stays a line of its
/// own under a plain bar.
pub fn framed(lang: &Lang, mut text: Text) -> Text {
    let ruled = |l: &Line| l.segments.iter().any(|s| s.style.strikethrough);
    if text.lines.len() < 2 || text.lines.first().is_some_and(ruled) {
        return text;
    }
    let tag = style::prefix(lang);
    let width = pumbo_common::rich::CHAT_WIDTH.saturating_sub(tag.width());
    let bar = |centre: Line| style::bar(centre, width, style::BRAND, style::accent::PERMS);
    let first = text.lines.remove(0);
    let mut title = Line::new();
    let skip = if !tag.plain().is_empty() && first.plain().starts_with(&tag.plain()) { tag.segments.len() } else { 0 };
    for segment in first.segments.iter().skip(skip) {
        title.push(segment.clone());
    }
    let mut lines = Vec::new();
    if title.width() * 4 <= width * 3 {
        lines.push(bar(title));
    } else {
        lines.push(bar(Line::new()));
        lines.push(first);
    }
    lines.extend(text.lines);
    if !lines.last().is_some_and(ruled) {
        lines.push(bar(Line::new()));
    }
    Text { lines }
}

/// A usage line in the PumboPerms colour.
pub fn usage(lang: &Lang, line: &str) -> Text {
    style::usage(lang, style::accent::PERMS, line)
}

/// An unknown subcommand; a click opens the help.
pub fn unknown(lang: &Lang, name: &str, help_command: &str) -> Text {
    style::unknown_subcommand(lang, style::accent::PERMS, name, help_command)
}

/// `true` / `false` in their colours, for a message argument.
pub fn boolean(v: bool) -> String {
    if v { format!("{YES}true&r") } else { format!("{NO}false&r") }
}

/// The first line of a view: the prefix and a title in the PumboPerms yellow.
pub fn header(lang: &Lang, key: &str, args: &Args) -> Text {
    style::title(lang, style::accent::PERMS, key, args)
}

/// A labelled row of a view: `  Label: value` (`value` with `&` codes).
pub fn row(indent: usize, label: &str, value: &str) -> Line {
    let pad = "  ".repeat(indent);
    if label.is_empty() {
        return Line::parse(&format!("{pad}{value}"));
    }
    Line::parse(&format!("{pad}{LABEL}{label}: {VALUE}{value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumbo_common::lang::{Bundle, COMMON};
    use pumbo_common::style::code;

    const B: Bundle = Bundle {
        name: "t",
        files: &[("en", "prefix: \"&8[&#f28c28&lP&#ffd54f&lP&8] \"\nok: \"Group {0} created.\"\n")],
    };

    #[test]
    fn replies_and_rows() {
        let (l, _) = Lang::load(&[COMMON, B], "en", None);
        assert_eq!(success(&l, "ok", &Args::new().arg(value("vip"))).plain(), "[PP] Group vip created.");
        assert_eq!(header(&l, "ok", &Args::new().arg("x")).plain(), "[PP] Group x created.");
        assert_eq!(row(1, "Weight", "10").plain(), "  Weight: 10");
        assert_eq!(row(2, "", "&fx").plain(), "    x");
        assert!(boolean(true).contains("true") && boolean(false).contains("false"));
        assert!(unknown(&l, "foo", "/pp help").plain().contains("foo"));
    }

    #[test]
    fn codes_follow_the_shared_palette() {
        let pairs = [
            (BRAND, style::BRAND),
            (ACCENT, style::accent::PERMS),
            (LABEL, style::LABEL),
            (INFO, style::INFO),
            (VALUE, style::VALUE),
            (MUTED, style::MUTED),
            (WARN, style::WARN),
            (ERROR, style::ERROR),
            (YES, style::SUCCESS),
            (NO, style::ERROR),
        ];
        for (ours, shared) in pairs {
            assert_eq!(ours, code(shared));
        }
        let parts: Vec<_> =
            row(1, "Weight", "10").segments.into_iter().map(|s| (s.text, s.style.color)).collect::<Vec<_>>();
        assert_eq!(
            parts,
            vec![("  ".into(), None), ("Weight: ".into(), Some(style::LABEL)), ("10".into(), Some(style::VALUE))]
        );
    }
}
