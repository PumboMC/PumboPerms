//! Registering the `pumboperms` placeholders with PumpkinPIE, and answering
//! its requests ([`crate::pie`]).

use std::cell::RefCell;

use crate::pie::{self, Next, PieState, Request};

use super::{api, import, state};

thread_local! {
    /// Outside the state, so registering never finds it busy.
    static PIE: RefCell<PieState> = RefCell::new(PieState::default());
}

/// From the periodic task, before anything else: a ping when one is due and
/// the registration after an answer in protocol 0. At most two messages, and
/// none from loading, unloading or an answer to PumpkinPIE.
pub fn tick(now: u64) {
    if !PIE.with(|p| p.borrow().due(now)) {
        return;
    }
    let pong = pie::heard(import::send(pie::PROVIDER, &pie::ping()).as_deref());
    if step(now, pong) == Next::Register {
        let reply = pie::heard(import::send(pie::PROVIDER, &pie::register()).as_deref());
        step(now, reply);
    }
}

fn step(now: u64, heard: pie::Heard) -> Next {
    let (next, line) = PIE.with(|p| p.borrow_mut().step(now, heard));
    match line {
        Some((true, line)) => api::warn(&line),
        Some((false, line)) => api::info(&line),
        None => {}
    }
    next
}

/// The answer to PumpkinPIE's `on_request`, from memory. While the proxy
/// rules, the values come from PumboBridge, which is the one other plugin
/// this asks; never PumpkinPIE, whose handler is waiting for this answer.
pub fn answer(req: &Request, now: u64) -> Vec<u8> {
    if state::passive() {
        return state::Rt::pie_passive_reply(req, super::player_info);
    }
    state::Rt::pie_reply(req, now)
}
