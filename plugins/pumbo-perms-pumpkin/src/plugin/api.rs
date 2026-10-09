//! The parts of the Pumpkin plugin API this plugin uses, the same for Pumpkin
//! 0.2.0 (MC 26.3) and 0.1.0-dev (MC 26.2), and the conversion of replies into
//! text components.

pub use crate::papi::command::{Arg, ArgumentType, Command, CommandNode, CommandSender, ConsumedArgs, StringType};
pub use crate::papi::command::{CommandError, CommandSuggestion, CommandSuggestions, SuggestionRequest};
pub use crate::papi::commands::{CommandHandler, CommandSuggestionHandler};
pub use crate::papi::common::{NamedColor, RgbColor};
pub use crate::papi::events::{
    EventData, EventHandler, EventPriority, PlayerChangeWorldEvent, PlayerCommandSendEvent, PlayerJoinEvent,
    PlayerLeaveEvent, PlayerPermissionCheckEvent,
};
pub use crate::papi::logging::{LogLevel, log};
pub use crate::papi::permission::{Permission, PermissionDefault, PermissionLevel};
pub use crate::papi::scheduler::SchedulerExt;
pub use crate::papi::text::TextComponent;
pub use crate::papi::uuid::Uuid;
pub use crate::papi::{Context, Player, Plugin, PluginMetadata, Server, permissions};

use pumbo_common::rich::{Click, Segment, Text};
use pumbo_common::text::{Color, Named};

#[cfg(feature = "mc263")]
pub const API_LABEL: &str = "Pumpkin 0.2.0 (Minecraft 26.3)";
#[cfg(feature = "mc262")]
pub const API_LABEL: &str = "Pumpkin 0.1.0-dev (Minecraft 26.2)";

pub fn info(msg: &str) {
    log(LogLevel::Info, msg);
}

pub fn warn(msg: &str) {
    log(LogLevel::Warn, msg);
}

pub fn error(msg: &str) {
    log(LogLevel::Error, msg);
}

pub fn level_of(level: PermissionLevel) -> u8 {
    match level {
        PermissionLevel::Zero => 0,
        PermissionLevel::One => 1,
        PermissionLevel::Two => 2,
        PermissionLevel::Three => 3,
        PermissionLevel::Four => 4,
    }
}

pub fn level_from(level: u8) -> PermissionLevel {
    match level {
        0 => PermissionLevel::Zero,
        1 => PermissionLevel::One,
        2 => PermissionLevel::Two,
        3 => PermissionLevel::Three,
        _ => PermissionLevel::Four,
    }
}

/// `xxxxxxxx-xxxx-...` in lowercase.
pub fn uuid_string(id: &Uuid) -> String {
    id.to_string()
}

pub fn parse_uuid(s: &str) -> Option<Uuid> {
    let id = pumbo_common::id::Uuid::parse(s)?;
    let (high, low) = id.high_low();
    Some(Uuid { high, low })
}

fn named(n: Named) -> NamedColor {
    match n {
        Named::Black => NamedColor::Black,
        Named::DarkBlue => NamedColor::DarkBlue,
        Named::DarkGreen => NamedColor::DarkGreen,
        Named::DarkAqua => NamedColor::DarkAqua,
        Named::DarkRed => NamedColor::DarkRed,
        Named::DarkPurple => NamedColor::DarkPurple,
        Named::Gold => NamedColor::Gold,
        Named::Gray => NamedColor::Gray,
        Named::DarkGray => NamedColor::DarkGray,
        Named::Blue => NamedColor::Blue,
        Named::Green => NamedColor::Green,
        Named::Aqua => NamedColor::Aqua,
        Named::Red => NamedColor::Red,
        Named::LightPurple => NamedColor::LightPurple,
        Named::Yellow => NamedColor::Yellow,
        Named::White => NamedColor::White,
    }
}

/// One segment as a component. Text component methods take the component and
/// give it back (chaining). Segments are siblings under an unstyled root, so
/// only what is on is set.
fn segment(seg: &Segment) -> TextComponent {
    let mut c = TextComponent::text(&seg.text);
    match seg.style.color {
        Some(Color::Named(n)) => c = c.color_named(named(n)),
        Some(Color::Rgb(rgb)) => {
            let [_, r, g, b] = rgb.to_be_bytes();
            c = c.color_rgb(RgbColor { r, g, b });
        }
        None => {}
    }
    let st = seg.style;
    if st.bold {
        c = c.bold(true);
    }
    if st.italic {
        c = c.italic(true);
    }
    if st.underlined {
        c = c.underlined(true);
    }
    if st.strikethrough {
        c = c.strikethrough(true);
    }
    if st.obfuscated {
        c = c.obfuscated(true);
    }
    match &seg.click {
        Some(Click::Suggest(s)) => c = c.click_suggest_command(s),
        Some(Click::Run(s)) => c = c.click_run_command(s),
        Some(Click::Copy(s)) => c = c.click_copy_to_clipboard(s),
        Some(Click::Url(s)) => c = c.click_open_url(s),
        None => {}
    }
    if let Some(h) = &seg.hover {
        c = c.hover_show_text(component(h));
    }
    c
}

/// A reply as one chat message: lines joined by line breaks.
pub fn component(t: &Text) -> TextComponent {
    let mut root = TextComponent::text("");
    for (i, line) in t.lines.iter().enumerate() {
        if i > 0 {
            root = root.add_child(TextComponent::text("\n"));
        }
        for seg in &line.segments {
            root = root.add_child(segment(seg));
        }
    }
    root
}

/// Sends a reply to a command sender: components for players, plain text for
/// the console.
pub fn reply(sender: &CommandSender, t: &Text) {
    if t.is_empty() {
        return;
    }
    if sender.is_player() {
        sender.send_message(component(t));
    } else {
        sender.send_message(TextComponent::text(&t.plain()));
    }
}

pub fn tell(player: &Player, t: &Text) {
    if !t.is_empty() {
        player.send_system_message(component(t), false);
    }
}
