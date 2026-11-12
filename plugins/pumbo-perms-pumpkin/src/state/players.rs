//! Online players: joins, leaves, worlds, refreshes, expiry and the host changes for each.

use std::collections::BTreeMap;

use pumbo_perms_core::command;
use pumbo_perms_core::host::{self, Op};

use super::{Online, Plan, RESEND_MS, Rt, passive};

impl Rt {
    /// Of the players whose command lists a change touched, those to send
    /// one now; the rest wait for [`Rt::due_resends`].
    pub fn resends(&mut self, touched: Vec<String>, now: u64) -> Vec<String> {
        let mut out = Vec::new();
        for uuid in touched {
            if self.resent.get(&uuid).is_some_and(|t| now.saturating_sub(*t) < RESEND_MS) {
                self.resend_waiting.insert(uuid);
            } else {
                self.resent.insert(uuid.clone(), now);
                out.push(uuid);
            }
        }
        out
    }

    /// The waiting players whose next command list is due.
    pub fn due_resends(&mut self, now: u64) -> Vec<String> {
        let due: Vec<String> = self
            .resend_waiting
            .iter()
            .filter(|u| self.resent.get(*u).is_none_or(|t| now.saturating_sub(*t) >= RESEND_MS))
            .cloned()
            .collect();
        for uuid in &due {
            self.resend_waiting.remove(uuid);
            self.resent.insert(uuid.clone(), now);
        }
        due
    }

    /// The host changes for one online player; `None` when there are none.
    /// Nothing before the first import pass (the bridge may still own them),
    /// and nothing while the proxy's PumboPerms rules ([`passive`]).
    pub fn plan_player(&mut self, uuid: &str, now: u64) -> Option<Plan> {
        if !self.ready || passive() {
            return None;
        }
        let o = self.online.get(uuid).cloned()?;
        let (wanted, hidden) =
            host::wanted_and_hidden(&mut self.engine, uuid, &o.world, o.op_level, now, &self.catalog);
        if self.hidden.get(uuid).map_or(&[][..], Vec::as_slice) != hidden.as_slice() {
            for (w, narrower) in &hidden {
                self.warn(format!("PumboPerms: not sent to Pumpkin for {}: {w} hides {narrower}", o.name));
            }
            if hidden.is_empty() {
                self.hidden.remove(uuid);
            } else {
                self.hidden.insert(uuid.to_string(), hidden);
            }
        }
        let ops = host::plan(&wanted, self.pushed.get(uuid), &self.catalog);
        (!ops.is_empty()).then(|| Plan { uuid: uuid.to_string(), ops, wanted })
    }

    /// Remembers what the players have on the host once the plans are
    /// applied; a player who left meanwhile is skipped (leaving took
    /// everything off).
    pub fn applied(&mut self, plans: Vec<Plan>) {
        for p in plans {
            if self.online.contains_key(&p.uuid) {
                self.pushed.insert(p.uuid, p.wanted);
            }
        }
    }

    /// A player joined (or was online at load): the host changes to apply.
    /// Pumpkin does not say whether Mojang checked the account, so the name
    /// counts as offline (an import never binds it here).
    pub fn join(&mut self, uuid: &str, online: Online, now: u64) -> Vec<Plan> {
        match self.engine.seen(uuid, &online.name, false) {
            Ok(Some(moved)) => self.warn(moved),
            Ok(None) => {}
            Err(e) => self.warn(format!("PumboPerms: cannot save the name of {}: {e}", online.name)),
        }
        for out in command::on_join(&mut self.engine, uuid, &online.name, false, now) {
            self.logs.push((true, out.reply.plain()));
        }
        self.online.insert(uuid.to_string(), online);
        let mut plans = self.due_expiry(now);
        if !plans.iter().any(|p| p.uuid == uuid) {
            plans.extend(self.plan_player(uuid, now));
        }
        plans
    }

    /// A player left: everything this plugin set on them goes (Pumpkin keeps
    /// attachments until it stops) and is forgotten, so the next join sets
    /// the whole catalog again.
    pub fn leave(&mut self, uuid: &str, now: u64) -> (Vec<Op>, Vec<Plan>) {
        self.online.remove(uuid);
        self.engine.gone(uuid);
        self.hidden.remove(uuid);
        self.resent.remove(uuid);
        self.resend_waiting.remove(uuid);
        let mut ops: Vec<Op> =
            self.pushed.remove(uuid).into_iter().flat_map(|p| p.into_keys().map(Op::Unset)).collect();
        ops.sort();
        (ops, self.due_expiry(now))
    }

    /// A player changed worlds; a cancelled change keeps the old world.
    pub fn change_world(&mut self, uuid: &str, world: String, cancelled: bool, now: u64) -> Vec<Plan> {
        let mut plans = self.due_expiry(now);
        if cancelled || plans.iter().any(|p| p.uuid == uuid) {
            return plans;
        }
        let Some(o) = self.online.get_mut(uuid) else { return plans };
        o.world = world;
        if self.engine.perms.uses_context("world") {
            plans.extend(self.plan_player(uuid, now));
        }
        plans
    }

    /// Whether the periodic refresh is due: `provider.refresh-seconds` of the
    /// config in force, so `/pp reload` changes the period at once.
    pub fn refresh_due(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.last_refresh) < u64::from(self.cfg.provider.refresh_seconds) * 1000 {
            return false;
        }
        self.last_refresh = now;
        true
    }

    /// Operator levels and worlds of online players as the server reports
    /// them (`(uuid, level, world)`), and expired nodes: the changes.
    pub fn refresh(&mut self, players: &[(String, u8, String)], now: u64) -> Vec<Plan> {
        let mut changed = Vec::new();
        let world_context = self.engine.perms.uses_context("world");
        for (uuid, level, world) in players {
            if let Some(o) = self.online.get_mut(uuid)
                && (o.op_level != *level || o.world != *world)
            {
                if o.op_level != *level || world_context {
                    changed.push(uuid.clone());
                }
                (o.op_level, o.world) = (*level, world.clone());
            }
        }
        let mut plans = self.due_expiry(now);
        for uuid in changed {
            if !plans.iter().any(|p| p.uuid == uuid) {
                plans.extend(self.plan_player(&uuid, now));
            }
        }
        plans
    }

    /// Nodes past their time: removed and the changes planned for everyone.
    /// Every entry into the plugin calls this, so a stopped periodic task
    /// (`/tick freeze`) does not keep a node alive. A failed save is only
    /// logged: answers skip expired nodes anyway, and the next call saves.
    pub fn due_expiry(&mut self, now: u64) -> Vec<Plan> {
        self.refresh_lock(now);
        let (due, saved) = self.engine.expire(now);
        if let Err(e) = saved {
            self.warn(format!("PumboPerms: cannot save the removal of expired permissions, trying again: {e}"));
        }
        if !due {
            return Vec::new();
        }
        self.after_change(now)
    }

    /// Whether an online player may run an alias Pumpkin leaves unchecked
    /// (`pumbo_common::pumpkin::UNGUARDED_ALIASES`), from memory: in check-event
    /// mode `has-permission` would call back into this plugin. Without a
    /// decision Pumpkin's default for the node counts (its operator level).
    pub fn alias_allowed(&mut self, uuid: &str, node: &str, now: u64) -> bool {
        let Some(o) = self.online.get(uuid).cloned() else { return false };
        let canonical = pumbo_perms_core::node::normalize(node).unwrap_or_default();
        self.engine
            .decide(uuid, &canonical, &o.world, o.op_level, now)
            .unwrap_or(o.op_level >= pumbo_common::pumpkin::alias_op_level(node))
    }

    /// The proxy's PumboPerms took over: everything set so far, per online
    /// player, for PumboBridge to unset.
    pub fn hand_over(&mut self) -> BTreeMap<String, Vec<String>> {
        self.pushed.drain().map(|(uuid, nodes)| (uuid, nodes.into_keys().collect())).collect()
    }

    /// Host changes for every online player.
    pub fn plan_all(&mut self, now: u64) -> Vec<Plan> {
        let uuids: Vec<String> = self.online.keys().cloned().collect();
        uuids.into_iter().filter_map(|u| self.plan_player(&u, now)).collect()
    }
}
