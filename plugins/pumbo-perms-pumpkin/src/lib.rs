//! PumboPerms for Pumpkin: permissions, groups and ranks.
//!
//! The rules live in `pumbo-perms-core`; this crate connects them to the
//! Pumpkin plugin API. It gives every online player the nodes the core decides
//! as Pumpkin permission attachments (see [`pumbo_perms_core::host`]), runs the
//! `/pumboperms` (`/pp`) command and, when enabled, answers Pumpkin's
//! permission check event directly.
//!
//! [`config`] and [`state`] are plain Rust and tested natively (the node
//! catalog is `pumbo_common::pumpkin`, shared with PumboBridge); the plugin
//! itself ([`plugin`]) only exists in the WebAssembly build.

// Panics trap the host and stdout is lost; these stay errors outside tests.
#![deny(
    clippy::string_slice,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::await_holding_lock,
    clippy::await_holding_refcell_ref,
    clippy::integer_division,
    clippy::modulo_arithmetic,
    clippy::unchecked_time_subtraction,
    clippy::get_unwrap,
    clippy::exit,
    clippy::large_stack_arrays,
    clippy::large_stack_frames,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::significant_drop_in_scrutinee
)]
#![cfg_attr(
    test,
    allow(
        clippy::string_slice,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::await_holding_lock,
        clippy::await_holding_refcell_ref,
        clippy::integer_division,
        clippy::modulo_arithmetic,
        clippy::unchecked_time_subtraction,
        clippy::get_unwrap,
        clippy::exit,
        clippy::large_stack_arrays,
        clippy::large_stack_frames,
        clippy::unwrap_in_result,
        clippy::panic_in_result_fn,
        clippy::significant_drop_in_scrutinee
    )
)]

pub mod config;
pub mod state;
pub mod suggest;

use pumbo_common::lang::Bundle;

/// Messages of the Pumpkin layer (prefix, files, mode).
pub const LANG: Bundle = Bundle {
    name: "pumboperms",
    files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))],
};

/// Every message bundle, in load order.
pub const BUNDLES: [Bundle; 3] = [pumbo_common::lang::COMMON, pumbo_perms_core::LANG, LANG];

/// Name of the plugin on Pumpkin; also the namespace of its permission nodes.
pub const PLUGIN_NAME: &str = "pumboperms";

/// Host node that decides whether a player sees `/pp` at all.
pub const COMMAND_NODE: &str = "pumboperms:command";

#[cfg(all(target_arch = "wasm32", feature = "mc263", feature = "mc262"))]
compile_error!("enable only one of the features `mc263` and `mc262`");
#[cfg(all(target_arch = "wasm32", not(any(feature = "mc263", feature = "mc262"))))]
compile_error!("enable one of the features `mc263` (Pumpkin 0.2.0) or `mc262` (Pumpkin 0.1.0-dev)");

#[cfg(all(target_arch = "wasm32", feature = "mc262", not(feature = "mc263")))]
extern crate api262 as papi;
#[cfg(all(target_arch = "wasm32", feature = "mc263", not(feature = "mc262")))]
extern crate api263 as papi;

#[cfg(target_arch = "wasm32")]
pub mod plugin;

#[cfg(test)]
mod tests {
    use pumbo_common::lang::{Lang, check_bundle};

    use super::*;

    #[test]
    fn messages_are_complete_in_every_language() {
        assert_eq!(check_bundle(&LANG), Vec::<String>::new());
        for code in ["en", "pl"] {
            let (l, w) = Lang::load(&BUNDLES, code, None);
            assert!(w.is_empty(), "{w:?}");
            assert!(l.get("prefix").contains("PumboPerms"));
        }
    }

    #[test]
    fn message_template_loads_without_warnings() {
        for code in ["en", "pl"] {
            let t = Lang::template(&BUNDLES, code);
            let (_, w) = Lang::load(&BUNDLES, code, Some(&t));
            assert!(w.is_empty(), "{w:?}");
        }
    }
}
