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
  <a href="https://github.com/Pumpkin-MC/Pumpkin"><img src="https://img.shields.io/badge/Pumpkin-0.1.0--dev%20%2826.2%29-F28C28" alt="Pumpkin 0.1.0-dev (26.2)"></a>
  <a href="https://github.com/PumboMC/PumboProx"><img src="https://img.shields.io/badge/PumboProx-supported-62B47A" alt="PumboProx: supported"></a>
  <img src="https://img.shields.io/badge/status-beta-yellow" alt="Status: beta">
</p>

<p align="center">
  <a href="#roadmap">Roadmap</a> ·
  <a href="#features">Features</a> ·
  <a href="#two-builds">Two builds</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#commands-and-permissions">Commands</a> ·
  <a href="#building">Building</a>
</p>

---

## Roadmap

PumboPerms is in beta. The next release makes it safe and predictable on networks. Open a topic for the details.

Work on the next release happens on the [`dev`](https://github.com/PumboMC/PumboPerms/tree/dev) branch. Items marked ✅ (Dev) are fixed there and ship with the next release.

<details open>
<summary><b>Security</b></summary>

Security hardening of the proxy plugin, the Pumpkin plugin and PumboBridge. Details come with the release. Please report problems privately, see [SECURITY.md](SECURITY.md).

</details>

<details open>
<summary><b>Permission rules: the same answer everywhere</b></summary>

- ✅ (Dev) Wildcards and denials set on the proxy, such as `pumbo.bans.*`, apply on the servers exactly as on the proxy. Until then, give exact nodes (`pumbo.bans.ban`) on a network.
- ✅ (Dev) `server=Lobby` and `server=lobby` mean the same server.
- ✅ (Dev) The `default` group works the same on the proxy and on the servers.
- ✅ (Dev) `world=` in `/pp` on the proxy is refused with a clear message, because the proxy does not see worlds, and `/pp ... permission check` shows the answer the server uses.
- ✅ (Dev) PumboPerms on a server answers other plugins with the proxy's ranks instead of an old local copy.
- ✅ (Dev) A cancelled world change keeps the permissions of the old world.
- ✅ (Dev) Permissions that other plugins set for a session survive a rank update.
- ✅ (Dev) `group.<name>` answers whether the player is in the group, and `permission set group.<name>` points you to `parent add`.
- Removing a temporary permission or group leaves the permanent one alone, and temporary ranks on a track expire as set.
- ✅ (Dev) Staff who have `/pp` on the proxy have it on the servers too.

</details>

<details open>
<summary><b>Network sync</b></summary>

- ✅ (Dev) A player's ranks reach the server as soon as they join, and again after a reconnect or a restart of the server or the proxy. Today a slow login can leave a player without the proxy's ranks until their permissions change.
- ✅ (Dev) Networks with thousands of ranked players stay in proxy mode. Today a large database makes the servers fall back to their local ranks.
- ✅ (Dev) Servers refresh the proxy's ranks after changes to the database or the permission provider, and do not treat an old copy as current.
- ✅ (Dev) Updates from the proxy carry a version and a confirmation, so a server cannot keep an older set.
- ✅ (Dev) After a rank change, the affected players get their new command list at once.
- ✅ (Dev) A player who leaves during an update keeps no removed rights, and server permissions end with the session.
- ✅ (Dev) A new PumboPerms file placed next to the old one stops the proxy with a clear message instead of turning permissions off.
- ✅ (Dev) Permission checks for offline players see changes at once.
- ✅ (Dev) While PumboPerms reloads on the proxy, `/pp` answers that it is restarting instead of going to the server.

</details>

<details open>
<summary><b>Data and imports</b></summary>

- Running `/pp import run` again keeps the groups, tracks, weights and inheritance you changed with `/pp`, and removes only what the source dropped.
- Imports by player name match the right account.
- ✅ (Dev) Imports reject invalid contexts and duplicate entries.
- Pending imports work on Pumpkin too.
- Import records live in the database and are written in one step.
- Long names with special characters no longer stop an import.
- `/pp export` keeps the old file when the database cannot be read.
- Two servers cannot open the same PumboPerms database at once.
- Database format versions with upgrade steps.

</details>

<details open>
<summary><b>Failures and recovery</b></summary>

- ✅ (Dev) During a provider outage, a reload or a damaged database, online players keep their last ranks and `on-load-failure` decides for new ones (deny by default). Ranks removed with `/pp` stay removed.
- ✅ (Dev) A damaged database shows up in the log with the reason.
- ✅ (Dev) The known causes of internal errors in PumboPerms on Pumpkin are fixed, and PumboBridge notices when the plugin stops.
- ✅ (Dev) Expired temporary permissions end on the servers even when a database write fails.
- ✅ (Dev) A config file with a syntax error keeps the last working settings and shows the line with the error.
- Commands that change nothing no longer reload the database.

</details>

<details open>
<summary><b>Commands</b></summary>

- Tab completion for `/pp` subcommands, players, groups and nodes, on PumboProx and on Pumpkin.
- Tab completion shows only what the player may use.
- Rename `/pp` and `/pumboperms` in `config.yml`.
- Long `/pp` lists split into pages.
- `meta set` checks keys, and `meta unset` removes the key you name.
- `/pp reload` applies all settings.
- Polish messages without English fragments.
- Clearer colours in the `/pp` help.

</details>

<details open>
<summary><b>Speed</b></summary>

- A rank change recomputes only the players it affects.
- Servers stop reading the proxy's whole database every few seconds.
- The table of player names stays small and fast with many players.
- Lower cost per permission check and per expiry check.
- Lower memory use on the proxy for large networks.

</details>

<details open>
<summary><b>Quality</b></summary>

- Cross tests in CI on PumboProx and on real Pumpkin servers: the proxy plugin on PumboProx, the Pumpkin plugin on its own, and both together through PumboBridge, on Pumpkin 0.2.0 (Minecraft 26.3) and 0.1.0-dev (Minecraft 26.2).
- ✅ (Dev) Tests for the Pumpkin plugin code.
- Performance tests that fail when PumboPerms gets slower.
- ✅ (Dev) Property tests for the node, context and import parsers.
- Upgrade tests from older PumboPerms databases.

</details>

### Later

- Several proxies sharing one database.
- Ranks that follow a PumboAuth account when it switches to premium.

## What it does

PumboPerms decides who may do what on your server. You put players into groups, give the groups permissions, prefixes and suffixes, and move players up and down rank ladders. At the first start it takes over the permissions your server already has, so you do not start from zero.

## Features

| | Feature | |
| --- | --- | --- |
| 👥 | **Groups** | Players get groups. A player without a group is in `default`. |
| 🧬 | **Inheritance** | A group can have parent groups and gets their permissions. |
| ⚖️ | **Weights** | When two groups disagree, the heavier one wins. |
| ✳️ | **Wildcards** | `pumbo.bans.*`, `minecraft:command.*`, `*`. The most specific node always wins. |
| ⏳ | **Temporary** | Permissions, groups, prefixes and suffixes for `1h30m` or `7d`. They end on their own, also while the player is online. |
| 🗺️ | **Contexts** | `server=lobby`, `group=lobbies` or `world=world_nether` limit a permission, a group or a prefix to where the player is. |
| 🏷️ | **Prefixes** | Text before the player's name, with priorities. |
| 🔖 | **Suffixes** | Text after the player's name, with priorities. |
| 🗃️ | **Meta** | Your own key and value pairs on players and groups. |
| 🪜 | **Tracks** | Ordered groups that form a rank ladder. |
| ⬆️ | **Promote** | `/pp promote <player> <track>` moves a player one step up. |
| ⬇️ | **Demote** | `/pp demote <player> <track>` moves a player one step down. |
| 📒 | **Log** | Every change is logged in `/pp log`. Staff can get a message about changes made by others. |
| 📤 | **Export** | `/pp export` writes everything into one JSON file. |
| 📥 | **Import** | `/pp import` reads such a file back. At start PumboPerms also takes over ranks from PumboProx and from other plugins. `/pp import undo` takes an import back. |
| 🧩 | **Placeholders** | `%pumboperms_prefix%`, `%pumboperms_rank%` and more for chat, tab and scoreboard plugins (from 0.1.2-beta). See [INTEGRATION.md](INTEGRATION.md). |
| ⌨️ | **Autocomplete** | Tab completes players, groups, tracks and permission nodes. After a change each player's command list is updated. On the proxy, tab completion for `/pp` comes later. |

## Two builds

| Build | File | Where it goes | Status |
| --- | --- | --- | --- |
| 🌐 **PumboProx** (whole network) | `PumboPerms-Proxy-<version>.wasm` | `plugins/` of the proxy | Beta. One set of ranks for the whole network: set them with `/pp` on the proxy and they apply on every server right away, through [PumboBridge](https://github.com/PumboMC/PumboBridge). Imports the proxy's `permissions.yml` at the first start. |
| 🎃 **Pumpkin** (one server) | `PumboPerms-Pumpkin-26.3-<version>.wasm` or `PumboPerms-Pumpkin-26.2-<version>.wasm` | `plugins/` of the server | Beta. Ranks and permissions for that server. |

When PumboPerms runs on the proxy, a PumboPerms on a server steps back and lets the proxy decide. Remove it from the proxy and the one on the server takes over again, with its own data untouched.

## Installation

> [!TIP]
> Download the files from [Releases](https://github.com/PumboMC/PumboPerms/releases/latest), or [build from source](#building).

1. Put the file that matches your Pumpkin version into the server's `plugins/` folder.
2. Start the server. The first start creates `plugins/data/pumboperms/config.yml` and the language files.
3. From the console (or as an operator), create a group and give it permissions:

   ```
   /pp creategroup vip
   /pp group vip parent add default
   /pp group vip setweight 10
   /pp group vip permission set minecraft:command.gamemode
   /pp group vip meta setprefix 10 "&6[VIP] "
   /pp user Steve parent add vip
   ```

The plugin only needs to read and write its own data folder. It has no network access.

## Configuration

The config file is written on the first start, with a short comment on every option. Reload it with `/pp reload`. Groups, players and tracks are not in this file: you manage them with `/pp`. The most used options:

| Option | Default | What it does |
| --- | --- | --- |
| `language` | `en` | Language of the messages: `en`, `pl` or any code with a `lang/<code>.yml` file. |
| `context.server` | empty | The name of this server, for nodes with `server=<name>`. |
| `context.group` | empty | The server group this server belongs to, for nodes with `group=<name>`. |
| `defaults.op-level` | `3` | Operators from this level get every `pumbo.*` permission that no group decides (0 turns it off). |
| `log.max-entries` | `10000` | Changes kept in the log (0: no limit). |
| `log.notify` | `true` | Tell staff with `pumbo.perms.log.notify` about changes. |
| `import.enabled` | `true` | Take over other permission sources at start. |
| `import.follow` | `false` | Import a source again whenever it changes (otherwise once). |
| `import.plugins` | `[]` | Other plugins to import from. |

## Commands and permissions

`/pumboperms`, or the short `/pp`, on PumboProx and on Pumpkin. `/pp help` lists the commands you may use, with clicks and tooltips.

| Command | What it does | Permission |
| --- | --- | --- |
| `/pp user <player> info` | Groups, permissions and meta of a player | `pumbo.perms.user.info` |
| `/pp user <player> permission set\|settemp\|unset <node>` | Change a player's permission | `pumbo.perms.user.permission.set`, `.unset` |
| `/pp user <player> permission check <node>` | How a permission is decided | `pumbo.perms.user.permission.check` |
| `/pp user <player> parent add\|addtemp\|remove\|set <group>` | Change a player's groups | `pumbo.perms.user.parent.add`, `.remove`, `.set` |
| `/pp user <player> meta setprefix\|setsuffix <priority> <text>` | Prefix or suffix of a player | `pumbo.perms.user.meta.set` |
| `/pp creategroup <name>` / `deletegroup <name>` | Create or delete a group | `pumbo.perms.group.create`, `.delete` |
| `/pp group <name> permission\|parent\|meta ...` | The same as for players | `pumbo.perms.group.permission.*`, ... |
| `/pp group <name> setweight <weight>` | Weight of a group | `pumbo.perms.group.setweight` |
| `/pp groups` | All groups | `pumbo.perms.group.list` |
| `/pp track <name> create\|append\|insert\|remove` | Build a rank ladder | `pumbo.perms.track.create`, `.edit` |
| `/pp promote <player> <track>` / `demote` | Move a player up or down a track | `pumbo.perms.user.promote`, `.demote` |
| `/pp log [page]` | Recent changes | `pumbo.perms.log` |
| `/pp export [file]` / `import <file>` | Save or load everything as JSON | `pumbo.perms.export`, `.import` |
| `/pp import preview\|run\|undo <source>` | Take over another source, or take it back | `pumbo.perms.import` |
| `/pp reload` / `version` | Reload the config, show the version | `pumbo.perms.reload`, `.version` |
| `/pp editor` | A web editor (planned) | `pumbo.perms.editor` |

Shorter names work too: `u`, `g` and `t` for `user`, `group` and `track`, `listgroups` and `listtracks` for `groups` and `tracks`. After a player or a group: `i` for `info`, `perm`, `p` or `permissions` for `permission`, `parents` or `inherit` for `parent`. For example `/pp u Steve p check minecraft:command.gamemode`.

Durations: `30s`, `10m`, `1h30m`, `7d`, `2w`, `1mo`, `1y`. Contexts go at the end of a command: `server=lobby`, `group=lobbies`, `world=world_nether`.

On Pumpkin the permissions are registered as `pumboperms:<name>` (the same node) and default to operators of level 3. The console may do everything.

## For plugin developers

Your plugin can ask PumboPerms about permissions, ranks and prefixes, and use its placeholders, on Pumpkin and on PumboProx. See [INTEGRATION.md](INTEGRATION.md).

## Works with other Pumbo plugins

PumboPerms works on its own. When it finds other Pumbo plugins, it works with them:

| Plugin | Together |
| --- | --- |
| 🌉 [PumboBridge](https://github.com/PumboMC/PumboBridge) | PumboPerms imports the proxy's ranks for this server through the bridge, then takes over writing permissions on the server. |
| 🧩 Other Pumbo plugins | Their permissions (`pumbo.bans.ban`, `pumbo.auth.stats`, ...) can be given in groups like any other node. Pumbo plugins can also ask PumboPerms directly, about offline players too. |

## Building

You need Rust stable with the WebAssembly target:

```sh
rustup target add wasm32-wasip2
```

Cargo fetches the shared Pumbo libraries (`pumbo-common`, `pumbo-sdk`) from the [PumboProx](https://github.com/PumboMC/PumboProx) repository on the first build.

Pumpkin build, both versions (`dist/PumboPerms-26.3.wasm` and `dist/PumboPerms-26.2.wasm`):

```sh
plugins/pumbo-perms-pumpkin/build.sh
```

PumboProx build (`plugins/pumbo-perms-prox/dist/pumbo-perms.wasm`):

```sh
plugins/pumbo-perms-prox/build.sh
```

Tests of the core and both builds:

```sh
cargo test
```
<p align="center">
  <a href="https://github.com/PumboMC/PumboProx"><img src="assets/pumboprox.webp" alt="PumboProx: everything you need to run a network on Pumpkin" width="100%"></a>
</p>

<p align="center"><b>Running more than one server?</b> <a href="https://github.com/PumboMC/PumboProx">PumboProx</a> is the proxy for Pumpkin networks, with plugins in WebAssembly.<br>PumboPerms runs on it too: ranks live in one place for the whole network and can differ from server to server.</p>

## License

PumboPerms (the core and the Pumpkin build) is licensed under the [GNU General Public License v3.0](LICENSE). The shared library for Pumbo plugins (`pumbo-common`) is dual-licensed under MIT and Apache-2.0.

PumboPerms is not affiliated with Mojang, Microsoft or the Pumpkin project.

---

<p align="center">
  Part of <a href="https://github.com/PumboMC/PumboProx"><b>PumboProx</b></a>. Everything you need to run a network on Pumpkin.
</p>
