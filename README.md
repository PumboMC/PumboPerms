<p align="center">
  <img src="assets/logo.png" alt="Pumbo logo" width="160">
</p>

<h1 align="center">PumboPerms</h1>

<p align="center">Ranks, groups and permissions for Pumpkin servers.</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License: GPL-3.0"></a>
  <img src="https://img.shields.io/badge/built%20with-Rust-orange?logo=rust" alt="Built with Rust">
  <img src="https://img.shields.io/badge/plugin-WebAssembly-654FF0?logo=webassembly&logoColor=white" alt="WebAssembly plugin">
  <a href="https://github.com/Pumpkin-MC/Pumpkin"><img src="https://img.shields.io/badge/Pumpkin-0.2.0%20%2826.3%29-F28C28" alt="Pumpkin 0.2.0 (26.3)"></a>
  <a href="https://github.com/PumboMC/PumboProx"><img src="https://img.shields.io/badge/PumboProx-supported-62B47A" alt="PumboProx: supported"></a>
  <img src="https://img.shields.io/badge/status-stable-brightgreen" alt="Status: stable">
</p>

<p align="center">
  <a href="#all-known-issues-and-roadmap">Roadmap</a> ·
  <a href="docs/features.md">Features</a> ·
  <a href="docs/installation.md">Installation</a> ·
  <a href="docs/configuration.md">Configuration</a> ·
  <a href="docs/commands.md">Commands</a> ·
  <a href="docs/how-it-works.md">How it works</a>
</p>

---


No code or parts was copied from [LuckPerms](https://github.com/LuckPerms/LuckPerms), but is strongly modeled on, that why we still want to thank [@lucko](https://github.com/lucko) for the solutions and standards it gave us.

## All known issues and Roadmap

1.0.0 makes PumboPerms ready for production on single servers and on networks. \
✅ FIXED - confirmed in game. Open a topic for the details. \
☑️ FIXED - not confirmed in game yet

Work on the next release happens on the [`dev`](https://github.com/PumboMC/PumboPerms/tree/dev) branch.

### 1.0.0 - ✅ 

<details>
<summary><b>Security</b></summary>

Security hardening of the proxy plugin, the Pumpkin plugin and PumboBridge. The details are in the [changelog](CHANGELOG.md). Please report problems privately, see [SECURITY.md](SECURITY.md).

</details>

<details open>
<summary><b>Permission rules: the same answer everywhere</b></summary>

- ✅ Wildcards and denials set on the proxy, such as `pumbo.bans.*`, apply on the servers exactly as on the proxy.
- ✅ `server=Lobby` and `server=lobby` mean the same server.
- ✅ The `default` group works the same on the proxy and on the servers.
- ✅ `world=` in `/pp` on the proxy is refused with a clear message, because the proxy does not see worlds, and `/pp ... permission check` shows the answer the server uses.
- ✅ PumboPerms on a server answers other plugins with the proxy's ranks instead of an old local copy.
- ✅ A cancelled world change keeps the permissions of the old world.
- ✅ Permissions that other plugins set for a session survive a rank update.
- ✅ `group.<name>` answers whether the player is in the group, and `permission set group.<name>` points you to `parent add`.
- ✅ Removing a temporary permission or group leaves the permanent one alone, and temporary ranks on a track expire as set.
- ✅ Staff who have `/pp` on the proxy have it on the servers too.

</details>

<details open>
<summary><b>Network sync</b></summary>

- ✅ A player's ranks reach the server as soon as they join, and again after a reconnect or a restart of the server or the proxy.
- ✅ Networks with thousands of ranked players stay in proxy mode.
- ✅ Servers refresh the proxy's ranks after changes to the database or the permission provider, and do not treat an old copy as current.
- ✅ Updates from the proxy carry a version and a confirmation, so a server cannot keep an older set.
- ✅ After a rank change, the affected players get their new command list at once.
- ✅ A player who leaves during an update keeps no removed rights, and server permissions end with the session.
- ✅ A new PumboPerms file placed next to the old one stops the proxy with a clear message instead of turning permissions off.
- ✅ Permission checks for offline players see changes at once.
- ✅ While PumboPerms reloads on the proxy, `/pp` answers that it is restarting instead of going to the server.

</details>

<details open>
<summary><b>Data and imports</b></summary>

- ✅ Running `/pp import run` again keeps the groups, tracks, weights and inheritance you changed with `/pp`, and removes only what the source dropped.
- ✅ Imports by player name match the right account.
- ✅ Imports reject invalid contexts and duplicate entries.
- ✅ Pending imports run on Pumpkin as on the proxy; there, players known only by name get their ranks with `/pp`.
- ✅ Import records live in the database and are written in one step.
- ✅ Long names with special characters no longer stop an import.
- ✅ `/pp export` keeps the old file when the database cannot be read.
- ✅ Two servers cannot open the same PumboPerms database at once.
- ✅ Database format versions with upgrade steps.

</details>

<details open>
<summary><b>Failures and recovery</b></summary>

- ✅ During a provider outage, a reload or a damaged database, online players keep their last ranks and `on-load-failure` decides for new ones (deny by default). Ranks removed with `/pp` stay removed.
- ✅ A damaged database shows up in the log with the reason.
- ✅ The known causes of internal errors in PumboPerms on Pumpkin are fixed, and PumboBridge notices when the plugin stops.
- ✅ Expired temporary permissions end on the servers even when a database write fails.
- ✅ A config file with a syntax error keeps the last working settings and shows the line with the error.
- ✅ Commands that change nothing no longer reload the database.
- ✅ A damaged player name record no longer blocks the database: PumboPerms skips it and warns once.

</details>

<details open>
<summary><b>Commands</b></summary>

- ✅ Tab completion for `/pp` subcommands, players, groups and nodes, on PumboProx and on Pumpkin.
- ✅ Tab completion shows only what the player may use.
- ✅ Rename `/pp` and `/pumboperms` in `config.yml`.
- ✅ Long `/pp` lists split into pages.
- ✅ `meta set` checks keys, and `meta unset` removes the key you name.
- ✅ `/pp reload` applies all settings.
- ✅ Polish messages without English fragments.
- ✅ A clearer `/pp` in chat, laid out: a `[PP]` tag on every line, the help grouped by topic, views between bars in the Pumbo colours, `true` in green, `false` in red and prefixes in their own colours.

</details>

<details open>
<summary><b>Speed</b></summary>

- ✅ A rank change recomputes only the players it affects.
- ✅ Servers stop reading the proxy's whole database every few seconds.
- ✅ The table of player names stays small and fast with many players.
- ✅ Lower cost per permission check and per expiry check.
- ✅ Lower memory use on the proxy for large networks.
- ✅ Lower memory use with hundreds of thousands of known players: names stay in the database, and memory keeps only the names of the players online.
- ✅ `/pp log` reads only the page you open, also with an unlimited log.

</details>

<details open>
<summary><b>Quality</b></summary>

- ✅ Cross tests in CI on PumboProx and on real Pumpkin servers: the proxy plugin on PumboProx, the Pumpkin plugin on its own, and both together through PumboBridge, on Pumpkin 0.2.0 (Minecraft 26.3).
- ✅ Tests for the Pumpkin plugin code.
- ✅ Performance tests that fail when PumboPerms gets slower.
- ✅ Property tests for the node, context and import parsers.
- ✅ Upgrade tests from older PumboPerms databases.

</details>

## Features

<details>
<summary><b>Ranks and permissions</b></summary>

- ✅ Groups with inheritance: a group gets the permissions of its parents
- ✅ Weights: when two groups disagree, the heavier one wins
- ✅ A `default` group for every player without a group, on every server and in every world
- ✅ Wildcards: `pumbo.bans.*`, `minecraft:command.*`, `*`; the most specific node decides
- ✅ Temporary permissions, groups, prefixes and suffixes (`1h30m`, `7d`) that end on their own, also while the player is online
- ✅ Contexts: `server=lobby`, `group=lobbies` or `world=world_nether` limit a permission, a group or a prefix
- ✅ Prefixes and suffixes with priorities, shown in their own colours
- ✅ Meta: your own key and value pairs on players and groups
- ✅ Tracks: rank ladders with `/pp promote` and `/pp demote`

</details>

<details>
<summary><b>Live changes</b></summary>

- ✅ No restart or reload needed: changes apply at once
- ✅ Players get their new command list right after a rank change

</details>

<details>
<summary><b>Commands</b></summary>

- ✅ Tab completion for subcommands, players, groups, tracks and permission nodes
- ✅ Tab offers players, groups and tracks only to staff who may see them
- ✅ Long lists split into pages with arrows to click
- ✅ Rename `/pp` or turn off subcommands in `config.yml`
- ✅ English and Polish messages

</details>

<details>
<summary><b>Data</b></summary>

- ✅ Takes over the permissions your server already has at the first start
- ✅ Imports ranks from PumboProx and from other plugins; `/pp import undo` takes an import back
- ✅ Running an import again keeps what you changed with `/pp`
- ✅ `/pp export` saves everything to one JSON file, and `/pp import` from the console restores it
- ✅ Every change in `/pp log`, with messages to staff about changes made by others
- ✅ Database format versions with upgrade steps

</details>

<details>
<summary><b>Networks (PumboProx)</b></summary>

- ✅ One set of ranks for every server, with differences per server where you need them
- ✅ Wildcards and denials set on the proxy apply on the servers the same way
- ✅ Ranks reach the server at every join and server switch, also after a reconnect or a restart
- ✅ Servers answer other plugins with the proxy's ranks, prefixes and suffixes
- ✅ Staff who have `/pp` on the proxy have it on the servers too
- ✅ Networks with thousands of ranked players stay in proxy mode

</details>

<details>
<summary><b>Safety and recovery</b></summary>

- ✅ A broken `config.yml` keeps the last working settings and shows the line with the error
- ✅ During a provider outage or with a damaged database, online players keep their ranks and new ones follow `on-load-failure` (deny by default)
- ✅ `/pp export` never overwrites a file and never replaces a backup with an empty one
- ✅ Two servers cannot open the same database at once
- ✅ Whole-file import only from the console; command blocks, signs and functions cannot run `/pp` as the console
- ✅ Closes a Pumpkin 0.2.0 hole: `/tp`, `/xp`, `/banip` and `/pardonip` need their permission

</details>

<details>
<summary><b>For plugin developers</b></summary>

- ✅ Placeholders: `%pumboperms_prefix%`, `%pumboperms_suffix%`, `%pumboperms_rank%`, `%pumboperms_group%`, `%pumboperms_groups%` and `%pumboperms_meta:<key>%`
- ✅ Other plugins can ask PumboPerms about permissions, ranks and prefixes, also for offline players

</details>

### What next? 1.1.0

- Staff limits, with two switches that are off by default: argument permissions, and a rule that staff must be in a group before they change it or give it to players. They decide which players, groups and tracks your staff may change and view.
- Rank, prefix, suffix and meta as placeholders through PumpkinPIE.
- A change copies only what it touches, so a change in a big group no longer doubles the memory PumboPerms uses.
- Players with their own groups, permissions or meta stay in the database while offline and load when they join or when a command changes them, so memory follows the players online.

## Performance

Measured on an Apple M4 Pro laptop with release builds of the plugin code; a VPS is often two or three times slower, and the plugin runs as WebAssembly, which adds to that. The details are in [How it works](docs/how-it-works.md#limits).

|                                             | PumboProx (network)                                                        | Pumpkin (one server)                                     |
| ------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------- |
| Memory                                      | about 45 MB with 300 players online on 10 servers and 3000 more in the database | about 13 MB with 100 players online and 8000 permissions |
| Memory for 400 000 known names              | +0.4 MiB, runs within a 64 MB plugin limit                                 | +0.4 MiB, runs within a 64 MB plugin limit               |
| One change among 20 000 players in a group  | heap peak +24.4 MiB                                                        | heap peak +24.4 MiB                                      |
| Change in a group every player has          | about 30 ms for 300 players on 10 servers                                  | about 10 ms for 100 players and 8000 permissions         |
| Change for one player                       | only that player is recomputed                                             | about 4 ms                                               |
| Import of 3000 players                      | `permissions.yml`: 28-44 ms, 14-18 ms again after a change                 | `/pp import <file>`: 27-31 ms, 69-74 ms again (it saves the current data first) |
| Join with 400 000 known names               | up to 0.135 ms (0.57 ms for the first after the start)                     | up to 0.135 ms (0.57 ms for the first after the start)   |
| Opening a database with 400 000 names       | 6 ms                                                                       | 15 ms, with loading the data                             |
| Database after a crash (kill -9)            | opens in 11-17 ms (3000 players)                                           | about 30 s: waits for the old lock to go stale           |


<p align="center"><b>Running more than one server?</b> <a href="https://github.com/PumboMC/PumboProx">PumboProx</a> is the proxy for Pumpkin networks, with plugins in WebAssembly.<br>PumboPerms runs on it too: ranks live in one place for the whole network and can differ from server to server.</p>

PumboProx and the other Pumbo plugins are still MVPs. We will bring every plugin to production state.


## License

PumboPerms (the core and both builds) is licensed under the [GNU General Public License v3.0](LICENSE). The shared library for Pumbo plugins (`pumbo-common`) is dual-licensed under MIT and Apache-2.0.

PumboPerms is not affiliated with Mojang, Microsoft or the Pumpkin project.

---

<p align="center">
  Part of <a href="https://github.com/PumboMC/PumboProx"><b>PumboProx</b></a>. Everything you need to run a network on Pumpkin.
</p>
