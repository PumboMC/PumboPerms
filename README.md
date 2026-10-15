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
  <a href="#updating">Updating</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#commands-and-permissions">Commands</a> ·
  <a href="#security">Security</a> ·
  <a href="#limits">Limits</a> ·
  <a href="#building">Building</a>
</p>

---

<p align="center">
  <a href="https://github.com/PumboMC/PumboProx"><img src="assets/pumboprox.webp" alt="PumboProx: everything you need to run a network on Pumpkin" width="100%"></a>
</p>

<p align="center"><b>Running more than one server?</b> <a href="https://github.com/PumboMC/PumboProx">PumboProx</a> is the proxy for Pumpkin networks, with plugins in WebAssembly.<br>PumboPerms runs on it too: ranks live in one place for the whole network and can differ from server to server.</p>

> [!NOTE]
> PumboPerms is in **beta** (0.1.1-beta). Try it on a test server before you put players on it.

## Roadmap

The next release, 1.0.0, makes PumboPerms ready for production on single servers and on networks. ✅ FIXED - confirmed in game, ☑️ FIXED - not confirmed in game yet. Open a topic for the details.

Work on the next release happens on the [`dev`](https://github.com/PumboMC/PumboPerms/tree/dev) branch. Items marked ✅ or ☑️ are fixed there and ship with the next release.

<details open>
<summary><b>Security</b></summary>

Security hardening of the proxy plugin, the Pumpkin plugin and PumboBridge. Details come with the release. Please report problems privately, see [SECURITY.md](SECURITY.md).

</details>

<details open>
<summary><b>Permission rules: the same answer everywhere</b></summary>

- ✅ Wildcards and denials set on the proxy, such as `pumbo.bans.*`, apply on the servers exactly as on the proxy.
- ✅ `server=Lobby` and `server=lobby` mean the same server.
- ✅ The `default` group works the same on the proxy and on the servers.
- ✅ `world=` in `/pp` on the proxy is refused with a clear message, because the proxy does not see worlds, and `/pp ... permission check` shows the answer the server uses.
- ☑️ PumboPerms on a server answers other plugins with the proxy's ranks instead of an old local copy.
- ☑️ A cancelled world change keeps the permissions of the old world.
- ☑️ Permissions that other plugins set for a session survive a rank update.
- ✅ `group.<name>` answers whether the player is in the group, and `permission set group.<name>` points you to `parent add`.
- ✅ Removing a temporary permission or group leaves the permanent one alone, and temporary ranks on a track expire as set.
- ☑️ Staff who have `/pp` on the proxy have it on the servers too.

</details>

<details open>
<summary><b>Network sync</b></summary>

- ☑️ A player's ranks reach the server as soon as they join, and again after a reconnect or a restart of the server or the proxy.
- ☑️ Networks with thousands of ranked players stay in proxy mode.
- ☑️ Servers refresh the proxy's ranks after changes to the database or the permission provider, and do not treat an old copy as current.
- ☑️ Updates from the proxy carry a version and a confirmation, so a server cannot keep an older set.
- ✅ After a rank change, the affected players get their new command list at once.
- ☑️ A player who leaves during an update keeps no removed rights, and server permissions end with the session.
- ☑️ A new PumboPerms file placed next to the old one stops the proxy with a clear message instead of turning permissions off.
- ✅ Permission checks for offline players see changes at once.
- ✅ While PumboPerms reloads on the proxy, `/pp` answers that it is restarting instead of going to the server.

</details>

<details open>
<summary><b>Data and imports</b></summary>

- ✅ Running `/pp import run` again keeps the groups, tracks, weights and inheritance you changed with `/pp`, and removes only what the source dropped.
- ✅ Imports by player name match the right account.
- ☑️ Imports reject invalid contexts and duplicate entries.
- ☑️ Pending imports run on Pumpkin as on the proxy; there, players known only by name get their ranks with `/pp`.
- ✅ Import records live in the database and are written in one step.
- ☑️ Long names with special characters no longer stop an import.
- ☑️ `/pp export` keeps the old file when the database cannot be read.
- ☑️ Two servers cannot open the same PumboPerms database at once.
- ✅ Database format versions with upgrade steps.

</details>

<details open>
<summary><b>Failures and recovery</b></summary>

- ☑️ During a provider outage, a reload or a damaged database, online players keep their last ranks and `on-load-failure` decides for new ones (deny by default). Ranks removed with `/pp` stay removed.
- ☑️ A damaged database shows up in the log with the reason.
- ☑️ The known causes of internal errors in PumboPerms on Pumpkin are fixed, and PumboBridge notices when the plugin stops.
- ☑️ Expired temporary permissions end on the servers even when a database write fails.
- ✅ A config file with a syntax error keeps the last working settings and shows the line with the error.
- ☑️ Commands that change nothing no longer reload the database.

</details>

<details open>
<summary><b>Commands</b></summary>

- ✅ Tab completion for `/pp` subcommands, players, groups and nodes, on PumboProx and on Pumpkin.
- ☑️ Tab completion shows only what the player may use.
- ☑️ Rename `/pp` and `/pumboperms` in `config.yml`.
- ☑️ Long `/pp` lists split into pages.
- ☑️ `meta set` checks keys, and `meta unset` removes the key you name.
- ☑️ `/pp reload` applies all settings.
- ☑️ Polish messages without English fragments.
- ✅ A clearer `/pp` in chat, laid out like LuckPerms: a `[PP]` tag on every line, the help grouped by topic, views between bars in the Pumbo colours, `true` in green, `false` in red and prefixes in their own colours.

</details>

<details open>
<summary><b>Speed</b></summary>

- ☑️ A rank change recomputes only the players it affects.
- ☑️ Servers stop reading the proxy's whole database every few seconds.
- ☑️ The table of player names stays small and fast with many players.
- ☑️ Lower cost per permission check and per expiry check.
- ☑️ Lower memory use on the proxy for large networks.

</details>

<details open>
<summary><b>Quality</b></summary>

- ☑️ Cross tests in CI on PumboProx and on real Pumpkin servers: the proxy plugin on PumboProx, the Pumpkin plugin on its own, and both together through PumboBridge, on Pumpkin 0.2.0 (Minecraft 26.3) and 0.1.0-dev (Minecraft 26.2).
- ☑️ Tests for the Pumpkin plugin code.
- ☑️ Performance tests that fail when PumboPerms gets slower.
- ☑️ Property tests for the node, context and import parsers.
- ☑️ Upgrade tests from older PumboPerms databases.

</details>

### Later

- Several proxies sharing one database.
- Ranks that follow a PumboAuth account when it switches to premium.

## What it does

PumboPerms decides who may do what on your server. You put players into groups, give the groups permissions, prefixes and suffixes, and move players up and down rank ladders.

At the first start it takes over the permissions your server already has, so you do not start from zero. On Pumpkin it does that in its first tick, before it writes anything: the proxy's ranks for this server through PumboBridge, the plugins listed in `import.plugins`, or, when neither gives anything, the permissions other plugins set on the players online. On PumboProx it takes over the proxy's `permissions.yml`.

## Features

| Feature | What it does |
| --- | --- |
| **Groups** | Players get groups. A player without a group that applies everywhere is also in `default`, on every server and in every world. |
| **Inheritance** | A group can have parent groups and gets their permissions. |
| **Weights** | When two groups disagree, the heavier one wins. |
| **Wildcards** | `pumbo.bans.*`, `minecraft:command.*`, `*`. In PumboPerms the most specific node decides. Pumpkin reads wildcards from the broadest one, so see [How Pumpkin gets the permissions](#how-pumpkin-gets-the-permissions). |
| **Temporary** | Permissions, groups, prefixes and suffixes for `1h30m` or `7d`. They end on their own, also while the player is online. `settemp` on a permission that already has a running end time in the same context is refused: run `unsettemp` first, then `settemp` with the new time. |
| **Contexts** | `server=lobby`, `group=lobbies` or `world=world_nether` limit a permission, a group or a prefix to where the player is. |
| **Prefixes** | Text before the player's name, with priorities. |
| **Suffixes** | Text after the player's name, with priorities. |
| **Meta** | Your own key and value pairs on players and groups. |
| **Tracks** | Ordered groups that form a rank ladder. |
| **Promote** | `/pp promote <player> <track>` moves a player one step up. |
| **Demote** | `/pp demote <player> <track>` moves a player one step down. |
| **Log** | Every change is logged in `/pp log`. Staff can get a message about changes made by others. |
| **Export** | `/pp export` writes everything into one JSON file. |
| **Import** | `/pp import` reads such a file back. At start PumboPerms also takes over ranks from PumboProx and from other plugins. `/pp import undo` takes an import back. |
| **Placeholders** | `%pumboperms_prefix%`, `%pumboperms_rank%` and more for chat, tab and scoreboard plugins (from 0.1.2-beta). See [INTEGRATION.md](INTEGRATION.md). |
| **Autocomplete** | Tab completes subcommands, players, groups, tracks and permission nodes, and offers players, groups and tracks only with the right to see them. On PumboProx it offers only the subcommands you may use; on Pumpkin everyone who may use `/pp` sees all subcommand words. See [Tab completion](#tab-completion). After a change each player's command list is updated. |
| **Look** | "Pumbo" in orange, "Perms" and the `/pp` command words in yellow. Info views show labels with their values, section headings, `true` in green and `false` in red, and a prefix in its own colours. The console gets the same text without colours. |

## Two builds

| Build | File in the release | Keep it in `plugins/` as | Status |
| --- | --- | --- | --- |
| **PumboProx** (whole network) | `PumboPerms-Proxy-<version>.wasm` | `pumbo-perms.wasm`, on the proxy | Beta. One set of ranks for the whole network: set them with `/pp` on the proxy and they apply on every server right away, through [PumboBridge](https://github.com/PumboMC/PumboBridge). Imports the proxy's `permissions.yml` at the first start. |
| **Pumpkin 0.2.0** (one server, Minecraft 26.3) | `PumboPerms-Pumpkin-26.3-<version>.wasm` | `PumboPerms-26.3.wasm`, on the server | Beta. Ranks and permissions for that server. |
| **Pumpkin 0.1.0-dev** (one server, Minecraft 26.2) | `PumboPerms-Pumpkin-26.2-<version>.wasm` | `PumboPerms-26.2.wasm`, on the server | Beta. Ranks and permissions for that server. |

Pumpkin checks the plugin API strictly: a file only loads on the Pumpkin version it was built for.

When PumboPerms runs on the proxy, the proxy rules. A PumboPerms on a server behind it steps back within 10 seconds: it hands the permissions it set to PumboBridge, which writes the proxy's decisions from then on, and its `/pp` answers that the proxy manages the ranks. Its own data stays untouched. The export PumboBridge receives holds only the flag that the proxy rules, whatever the size of your database and whether PumboPerms on the proxy runs right now, so a server never takes over because the proxy's PumboPerms is restarting. Remove PumboPerms from the proxy and the one on the server takes over again.

**Keeps the ranks while it is down.** When PumboPerms on the proxy restarts, is reloaded or unloaded, or cannot read its database, players online keep their permissions and the proxy refuses new logins with "Permissions are temporarily unavailable" (`on-load-failure: deny`), instead of falling back to `permissions.yml`, which still holds the ranks from before the takeover. Fix it from the proxy console with `/prox plugins reload pumbo-perms`, or set `permissions.provider: file`.

Update PumboBridge together with PumboPerms: an older PumboBridge does not list the Pumbo permissions.

## Installation

> [!TIP]
> Download the files from [Releases](https://github.com/PumboMC/PumboPerms/releases/latest), or [build from source](#building).

1. Put the file that matches your Pumpkin version into the server's `plugins/` folder, under the name from [Two builds](#two-builds).
2. Start the server. The first start creates `plugins/data/pumboperms/config.yml` and `lang/en.yml` (the file of the language set in the config).
3. From the console (or as an operator), create a group and give it permissions:

   ```
   /pp creategroup vip
   /pp group vip parent add default
   /pp group vip setweight 10
   /pp group vip permission set minecraft:command.gamemode
   /pp group vip meta setprefix 10 "&6[VIP] "
   /pp user Steve parent add vip
   ```

On PumboProx, put `pumbo-perms.wasm` into the proxy's `plugins/` folder and start the proxy. It writes `plugins/pumbo-perms/config.yml` and imports the groups and players of `permissions.yml`; the log line `permissions.yml: PumboPerms » Imported file: ...` says what it took over.

The plugin only needs to read and write its own data folder. It has no network access.

### Recommended Pumpkin settings

For a server with players on it, set this in `pumpkin.toml`:

```toml
[plugins]
hot_reload = false

[plugins.overrides.pumboperms]
allowed_permissions = ["fs.read.data", "fs.write.data"]
max_memory_mb = 64
```

- `allowed_permissions` approves the only two sandbox permissions PumboPerms asks for. Write them with dots: the colon form from Pumpkin's examples never matches. Without this line every new build (a new file hash) asks in the console, hot reload does not load the plugin, and a "no" in the console stays for that file.
- With `hot_reload` on, Pumpkin reloads plugins while players play: it changes the order of event handlers, can refuse permissions, and registers commands under load.
- `max_memory_mb = 64` leaves PumboPerms room (see [Limits](#limits)).
- Let only the user that runs the server write to `plugins/` and `plugins/cache/`: Pumpkin loads compiled plugin code from the cache.

## Updating

Save the new file in `plugins/` under the same name as the old one: `pumbo-perms.wasm` on PumboProx, `PumboPerms-26.3.wasm` or `PumboPerms-26.2.wasm` on Pumpkin. Keep the name, so the update overwrites the old file. On PumboProx two files with the id `pumbo-perms` stop the proxy at start with both names and "delete the older one". Your data folder stays.

Coming from 0.1.2-beta, run `/pp export` first. The first start of the new version moves the import ledgers into `perms.redb` and raises its schema; 0.1.2-beta then refuses the file as newer than it understands. To go back, put the old file in place and import your export with `/pp import <file>` on a fresh `perms.redb`.

Update PumboBridge on your servers at the same time.

## Configuration

The config file is written on the first start, with a short comment on every option: `plugins/data/pumboperms/config.yml` on Pumpkin, `plugins/pumbo-perms/config.yml` on PumboProx. Reload it with `/pp reload`. Groups, players and tracks are not in this file: you manage them with `/pp`.

| Option | Default | Where | What it does |
| --- | --- | --- | --- |
| `language` | `en` | both | Language of the messages: `en`, `pl` or any code with a `lang/<code>.yml` file. |
| `commands.aliases` | `[pp]` | both | Short names of `/pumboperms`. Change the list or clear it; `/pumboperms` always works. A change applies after a server restart on Pumpkin and after `/prox plugins reload pumbo-perms` on PumboProx. |
| `commands.disabled` | `[]` | both | `/pp` subcommands to turn off (`editor`, `export`, `user`...). They leave the help and Tab completion, and typed they get "Unknown subcommand". After `/pp reload` they are refused at once; Tab on Pumpkin follows after a server restart, and `/pumboperms <subcommand>` on PumboProx after `/prox plugins reload pumbo-perms`. |
| `context.server` | empty | Pumpkin | The name of this server, for nodes with `server=<name>`. |
| `context.group` | empty | Pumpkin | The server group this server belongs to, for nodes with `group=<name>`. |
| `defaults.op-level` | `3` | Pumpkin | Operators from this level may use every `/pp` subcommand that no group decides (0 turns it off). Other Pumbo plugins and Pumpkin's commands keep their own operator levels. Pumpkin's own default for the `pumboperms:` nodes changes only after a server restart. |
| `log.max-entries` | `10000` | both | Changes kept in the log (0: no limit). |
| `log.notify` | `true` | both | Tell staff with `pumbo.perms.log.notify` about changes. |
| `provider.check-event` | `false` | Pumpkin | Also answer each of Pumpkin's permission checks. For testing only: on Pumpkin 0.2.0 and 0.1.0-dev it can hang the server. Needs a restart. |
| `provider.resend-commands` | `true` | Pumpkin | Send players their command list again after a change (not with `check-event: true`). |
| `provider.refresh-seconds` | `5` | Pumpkin | How often PumboPerms checks expired nodes and the operator levels and worlds of online players. Commands, joins and calls from other plugins remove expired nodes at once. `/pp reload` changes it. |
| `import.enabled` | `true` | both | Take over other permission sources at start (on PumboProx: `permissions.yml`). |
| `import.follow` | `false` | both | Import a source again whenever it changes (otherwise once). |
| `import.plugins` | `[]` | Pumpkin | Other plugins to import from. |

On Pumpkin, PumboPerms leaves out a short name that Pumpkin or another Pumbo plugin uses for its own command, with a warning in the log. Pumpkin does not tell plugins which commands third-party plugins registered, so do not use their names. On PumboProx, the proxy refuses a name another plugin holds, and the log says so. `/pumboperms` works in both cases.

Each time `config.yml` loads, PumboPerms keeps a copy as `config.last-good.yml` in its data folder. If the file is not valid YAML at start, PumboPerms starts from that copy and logs an error; without a copy it does not start. `/pp reload` with a broken file keeps the current settings.

After an update, PumboPerms on Pumpkin, and PumboProx for the files of its plugins, give a `config.yml` or `lang/<code>.yml` that nobody edited the new built-in version: they keep a copy of what they wrote in `.generated/` and compare the file with it. A `config.yml` whose built-in values changed stays as it is, and the log names those options, so the values in force never change on their own. A file you edited stays as it is, and the log names the options or texts it lacks and the ones that still have the previous built-in text; remove those keys or delete the file to get the new ones. A deleted file comes back. On Pumpkin, a file an earlier release wrote counts as untouched without a copy too, as long as nobody edited it; any other file without a copy stays, and PumboPerms compares it with this version from then on.

If your edited `lang/<code>.yml` has `target-user: "{0}"`, replies about a player do not show the UUID: delete that line, or write `"{0} &8({1})&r"`.

## Commands and permissions

`/pumboperms`, or the short `/pp`, on PumboProx and on Pumpkin, from players and the console, on Pumpkin also from RCON. `/pp help` lists the commands you may use, with clicks and tooltips. Long lists (`info`, `permission info`, `parent info`, `meta info`, `groups`, `tracks`) show 20 rows per page with arrows to click; the page number goes last (`/pp group admin info 2`), and the console gets the whole list. `/pp log` shows 10 changes per page and `listmembers` 15 players. On PumboProx a reply too long for one chat message comes in several.

| Command | What it does | Permission (`pumbo.perms.`...) |
| --- | --- | --- |
| `/pp user <player> info` | Groups, permissions and meta of a player | `user.info` |
| `/pp user <player> permission info` | The player's permissions | `user.permission.info` |
| `/pp user <player> permission set\|settemp <node> [true\|false] [duration]` | Give or deny a permission | `user.permission.set` |
| `/pp user <player> permission unset\|unsettemp <node>` | Take a permission away | `user.permission.unset` |
| `/pp user <player> permission check <node>` | How a permission is decided | `user.permission.check` |
| `/pp user <player> permission clear` | Remove all of the player's permissions | `user.permission.clear` |
| `/pp user <player> parent info` | The player's groups | `user.parent.info` |
| `/pp user <player> parent add\|addtemp <group> [duration]` | Add a group | `user.parent.add` |
| `/pp user <player> parent remove\|removetemp <group>` | Remove a group | `user.parent.remove` |
| `/pp user <player> parent set <group>` | Make a group the only one | `user.parent.set` |
| `/pp user <player> meta info` | Prefixes, suffixes and meta | `user.meta.info` |
| `/pp user <player> meta set <key> <value>` / `unset <key>` | Meta values | `user.meta.set`, `user.meta.unset` |
| `/pp user <player> meta setprefix\|addprefix\|addtempprefix <priority> <text> [duration]` | Prefixes (the same with `suffix`) | `user.meta.set` |
| `/pp user <player> meta removeprefix\|removetempprefix <priority> [text]` | Remove a prefix (the same with `suffix`) | `user.meta.unset` |
| `/pp user <player> clear` | Remove everything of a player | `user.clear` |
| `/pp creategroup <name>` / `deletegroup <name>` | Create or delete a group | `group.create`, `group.delete` |
| `/pp group <name> info` | Parents, permissions and meta of a group | `group.info` |
| `/pp group <name> permission\|parent\|meta ...` | The same as for players | `group.permission.*`, `group.parent.*`, `group.meta.*` |
| `/pp group <name> rename <new name>` | Rename a group | `group.rename` |
| `/pp group <name> setweight <weight>` | Weight of a group | `group.setweight` |
| `/pp group <name> setdisplayname <name\|clear>` | Display name (the rank) | `group.setdisplayname` |
| `/pp group <name> listmembers [page]` | Players in the group | `group.listmembers` |
| `/pp groups` | All groups | `group.list` |
| `/pp createtrack <name>` / `deletetrack <name>` | Create or delete a track | `track.create`, `track.delete` |
| `/pp track <name> info` | The groups of a track | `track.info` |
| `/pp track <name> append\|insert\|remove\|clear` | Build a rank ladder | `track.edit` |
| `/pp tracks` | All tracks | `track.list` |
| `/pp promote <player> <track>` / `demote` | Move a player up or down a track | `user.promote`, `user.demote` |
| `/pp log [page]` | Recent changes | `log` |
| `/pp export [file]` / `import <file>` | Save or load everything as JSON | `export`, `import` |
| `/pp import preview\|run\|undo <source>` | Take over another source, or take it back | `import` |
| `/pp reload` / `version` | Reload the config, show the version | `reload`, `version` |

On PumboProx `/pumboperms debug` (`pumbo.perms.debug`) turns on or off a log line for every permission set PumboPerms sends to the proxy, and the import source is `file`, the proxy's `permissions.yml`.

Shorter names work too: `u`, `g` and `t` for `user`, `group` and `track`, `listgroups` and `listtracks` for `groups` and `tracks`, `info` for `version` and `?` for `help`. After a player or a group: `i` for `info`, `perm`, `p` or `permissions` for `permission`, `parents` or `inherit` for `parent`. After `permission`, `parent` or `meta`: `list` for `info`. For example `/pp u Steve p check minecraft:command.gamemode`.

Durations: `30s`, `10m`, `1h30m`, `7d`, `2w`, `1mo`, `1y`. Contexts go at the end of a command: `server=lobby`, `group=lobbies`, `world=world_nether`. Text with spaces goes in quotes: `"&6[VIP] "`; inside quotes `\"` is a quote and `\\` a backslash. A new meta key may have 1-64 characters `a-z`, `0-9`, `_`, `.` and `-`.

Server and server-group names in contexts ignore letter case: `server=survival` applies on the server `Survival` from `pumboprox.yml`. PumboProx does not start with two servers whose names differ only in letter case. A server in `pumboprox.yml` wins over a server of `managed-servers` whose name differs only in letter case; the proxy leaves the second one out and logs a warning.

The proxy does not see worlds. On PumboProx `/pp` refuses `world=`, `permission check` with `world=` adds that such entries apply nowhere, and an import keeps them and says how many there are.

`group.<name>` always answers whether the player is in that group. `/pp ... permission set group.<name>` is refused: use `parent add <name>`. An import leaves such permissions out and lists them in its reply. Behind PumboProx, PumboBridge also sets `group.<name>` on the server, so a Pumpkin plugin that checks it gets the same answer.

A player name that cannot be a Minecraft name (an offline-mode name with special spaces or `§`) shows with its UUID, and clicks and Tab completion use the UUID.

On Pumpkin the permissions are registered as `pumboperms:<name>` (the same node) and default to operators of level 3. The console may do everything. On Pumpkin, `/tp`, `/xp`, `/banip` and `/pardonip` need the permission of their command also after `/execute ... run`. Tab completion still shows these four aliases to everyone, because Pumpkin builds that list; PumboPerms refuses only running them.

### Tab completion

On both platforms Tab completes players, groups and tracks only for those with the `info` right of their kind (`pumbo.perms.user.info`, `group.info`, `track.info`). Where a permission node goes, it lists the nodes in your data, Pumpkin's commands, the nodes of the Pumbo plugins and those of PumboPerms; context keys (`server=`) come only where the command takes a context.

On PumboProx, PumboPerms answers Tab itself through the proxy's plugin suggestions: after `/pp` it offers the subcommands you may use, and on every level after that only the words you may use.

On Pumpkin, `/pp`, `/pumboperms` and every name in `commands.aliases` are separate commands with the whole tree of subcommands, so Tab works right after the space (Pumpkin 0.2.0 suggests nothing there for an alias). Above the chat box the client names the next argument (`<player>`, `<node> [<value>|<context>]`), and after a complete command it shows nothing more. Pumpkin 0.2.0 and 0.1.0-dev give plugins no way to hide part of a command, so everyone who may use `/pp` sees all subcommand words. Pumpkin reads the line along that tree before PumboPerms sees it, which makes a few things work differently than on PumboProx:

- Pumpkin matches the words of the help exactly, in lower case. Any other word in their place goes to PumboPerms with the rest of the line and works as on the proxy: the shorter names above, words in upper case (`/pp USER Steve INFO`) and `editor`, which answers that the editor is not there yet. Tab offers the words of the help, and after a shorter name the words that follow it.
- Where an argument can take the word, Pumpkin gives it to the argument: after `import`, `RUN` goes to `<file>`, so `/pp import RUN proxy` fails.
- Pumpkin reads a one-word argument (`<player>`, `<group>`, `<duration>`, `[page]`...) only from `0-9 A-Z a-z _ - . +`; any other character needs quotes, `"..."` or `'...'`, and the quote has to be closed. Nodes, contexts, meta keys and values, prefixes, suffixes and display names take the rest of the line, so they need no quotes, and `"..."` keeps spaces there as on the proxy.
- Where the tree has no room for another word, Pumpkin answers with its own error instead of the PumboPerms usage line: `/pp user` without a player, or a word after a complete command (`/pp group vip create now`).
- `permission settemp`, `meta addtempprefix` and `meta addtempsuffix` show the duration in the hint as `[<duration>]`, because the node or text before it takes the rest of the line. The duration is still required; without one PumboPerms answers with the usage line.

### Export and import

`/pp export` writes `exports/<file>.json` in the data folder. It never overwrites a file and refuses to run while the database cannot be read. The export holds groups, players and tracks, without the log, the known nicknames or the import ledgers. `/pp import <file>` saves the current data first and then replaces everything.

An import fixes or leaves out entries no command could write, and its reply counts them and lists up to 20. A group or track with an invalid name comes in under a valid form of it: lower case, Polish letters without their marks, `-` for other characters, at most 36 characters, and `-2`, `-3`... when the file has that name already (`group "zła grupa!" imported as zla-grupa`). Inheritance, ranks and tracks follow the new name. The import leaves out a group with nothing usable in its name, together with every player and group that inherits it and every track it is on, so nobody keeps a rank without the denies of its groups, and it leaves out whole a track with a group the file does not have. It also leaves out players and UUIDs that are not valid, permissions that are not permissions, entries with an unknown context key, `group.<name>` set as a permission, parents without their group, prefixes, suffixes, display names and meta values over 256 bytes or with control characters, and meta keys that are empty or have spaces. It shows user names that are not Minecraft names by UUID and fixes contexts in another spelling and duplicates. A line break in a name, node or context shows as `\n` in the reply and the log. PumboPerms refuses only a file that is not a PumboPerms export.

Importing a source again keeps the tracks, inheritance, weights and ranks you set since: it takes back only what the source no longer has and adds what is new, and an imported entry you removed does not come back. `/pp import undo <source>` keeps a group that has members, is on a track or is inherited by another group. A ledger that cannot be read stops the automatic import of its source with an error in the log.

On PumboProx, players written by nickname in `permissions.yml` go to the account that has the name and joined with an online-mode (premium) account, or get their entries when such an account joins. An offline-mode account with that name gets nothing, also when it joined before the import: the import reply and the log name the player, and you give the rank by hand. The first account a nickname went to keeps it at every later import.

On PumboProx, the next start with the same `permissions.yml` imports nothing. When the file changed, the log says so; `/pp import preview file` shows what would change and `/pp import run file` imports it (with `import.follow: true` PumboPerms does it on its own). `/prox reload` hands a changed file to the running PumboPerms at once. If that fails, the log says "permissions.yml not handed to the permission provider" and the next `/prox reload` sends the file again. When PumboPerms cannot read the file it got, the log says why, and `/pp import preview|run file` answers "Cannot import file: permissions.yml has errors" with the same reason.

On Pumpkin, players a source names only by nickname get nothing, because Pumpkin does not tell whether Mojang checked an account. The import reply lists them; give them their ranks with `/pp`. From the source `proxy` only the copy PumboBridge got from the proxy since it last connected counts; an older one from disk is not imported. While the bridge has a proxy key and no such copy yet, PumboPerms waits before its first pass and Pumpkin's own defaults apply; after 10 seconds it says so in the log and keeps waiting, also when the proxy cannot be reached. Behind a proxy with PumboPerms, `/pp import run proxy` answers that there is nothing to import.

## Security

Give write rights in `/pp` only to admins you trust with everything. Any right that changes permissions is enough to become a full admin, for example:

- `pumbo.perms.user.promote` on a track that leads to the admin group: promote yourself;
- `pumbo.perms.user.permission.set`: `/pp user <you> permission set *`;
- `pumbo.perms.group.parent.add` or `user.parent.add`: put yourself into the admin group;
- `pumbo.perms.import`: `/pp import run <source>` brings back an old permission source;
- on PumboProx, `*` also grants `pumbo.proxy.bridge.key`, the PumboBridge key of the network.

`/pp import <file>` replaces the whole database, so only the console may run it (on Pumpkin also RCON, which counts as the console). `/pp` does not run from command blocks, signs or functions on Pumpkin: anyone who can place one would act as the console. Use the console or RCON for automation. `/pp log` records every change with the admin who made it.

On PumboProx, players written by nickname in `permissions.yml` get their ranks only when they join with an online-mode (premium) account. Anyone can use a nickname on an offline-mode account, so such an account gets nothing; the log names it and you give the rank by hand.

Report security problems privately, see [SECURITY.md](SECURITY.md).

## How Pumpkin gets the permissions

Pumpkin keeps a list of permission values per player and reads wildcards from the broadest one. It also reads a permission set exactly as a wildcard for everything below it. PumboPerms on Pumpkin and PumboBridge behind PumboProx send each player a list that Pumpkin reads the way PumboPerms decides:

- Every permission of the Pumbo plugins gets its own value (`pumbobans:ban`, `pumbofilter:bypass` and so on), even when your data only has a wildcard such as `pumbo.*`.
- A wildcard with a narrower wildcard of another value below it is not sent. With `pumbo.bans.*` true and `pumbo.bans.exempt.*` false only `pumbobans:exempt.*` goes to Pumpkin. The server log names such a wildcard, and `/pp user <name> info` and `permission check` show "not sent to Pumpkin: pumbobans:* hides pumbobans:exempt.*". Below such a wildcard, a permission of another plugin that your data does not name falls back to Pumpkin's own default, so an operator may get it.
- A permission set exactly also wins over a narrower wildcard below it. With `essentials:fly` true and `essentials:fly.*` false, a permission below it that no group or user names gets true from Pumpkin. The server log and `/pp` show "not sent to Pumpkin: essentials:fly hides essentials:fly.*"; give such a permission its own value.
- A permission that Pumpkin, the Pumbo plugins or your data name, without its own decision, below a permission set to true, gets false. A moderator with `pumbo.bans.notify` does not get `pumbobans:notify.silent` from Pumpkin's operator level: give it on its own. With `myplugin:home` for `vip` and `myplugin:home.others` only for `admin`, a vip player gets false for `myplugin:home.others`, on a single server and behind PumboProx alike.

When a player leaves, PumboPerms takes off everything it set (Pumpkin would keep it until the server stops), and when they join again it checks the whole list, so a rank taken away meanwhile is gone. After a change that affects commands, the player gets the command list again, at most once a second; changes in that second go out together.

PumboPerms looks for expired temporary permissions every `provider.refresh-seconds` and each time it runs: a player joins, leaves, changes worlds or sends any command, or another plugin asks PumboPerms. `/tick freeze` stops the periodic check, so during a freeze an expired permission goes at the next of those moments.

One copy of PumboPerms owns `perms.redb` and keeps `perms.redb.lock` fresh every 10 seconds. A second copy on the same folder logs "the database is open in another instance", refuses changes and tries again every 10 seconds. `/tick freeze` stops the periodic refresh: a freeze longer than 30 seconds in which nobody uses a command or joins, together with loading a second copy in that time, lets both copies open the database. A server that crashed or was killed leaves its lock behind, so after a quick restart PumboPerms waits up to 30 seconds: until then it imports nothing, sets no permissions and refuses changes, then opens the database and sets the permissions of the players online.

If PumboPerms crashes (Pumpkin shows the plugin as failed), the permissions it last set stay on online players until the server restarts, temporary ones included. Restart the server; `/plugin unload` does not take them off.

## Limits

Measured on an Apple M4 Pro laptop with release builds of the plugin code; a VPS is often two or three times slower, and the plugin runs as WebAssembly, which adds to that.

On PumboProx (the proxy stops a plugin call after 200 ms of continuous work, `plugins.budget-ms`): a change in a group every player inherits, with 300 players online on 10 servers, 20 groups of 50 permissions and 3000 more players in the database, takes about 30 ms for all 300 new sets, and a change that reaches one player recomputes that player alone. Importing a `permissions.yml` with 3000 players takes 28-44 ms the first time and 14-18 ms again after a change; reading the file, the import and each group of 25 new sets are separate pieces of work for the budget. The import itself is one database transaction, 13-27 ms of that. In a test on PumboProx (built for debugging) with the built `pumbo-perms.wasm`, PumboPerms finished importing a `permissions.yml` with 3000 players in 20 groups of 50 permissions about 120 ms after the proxy started, and a change in `default` reached 200 players online in about 35 ms. No piece of work ran over the budget, also with `plugins.memory-mb: 64`. A first join with 400 000 nicknames in the database takes under 1 ms. Keep the default `plugins.memory-mb: 256` in `pumboprox.yml`.

PumboProx itself, with 1040 entries per player (20 groups of 50 nodes, entries on 10 servers), answers a permission check in under 7 µs, also when no entry matches. The sets it sends to servers with PumboBridge take 43 ms for 300 players after a change that touches all of them. Reading a player's entries from a `permissions.yml` with 3000 players takes 5 µs at join.

On Pumpkin, PumboPerms handles one call at a time, and Pumpkin waits for it at every player command and join, so while PumboPerms works on a change, commands and joins wait too. A change in a group every player inherits, with 100 players online and 8000 permissions named in the data, takes about 10 ms. A player gets the command list again at most once a second. Set `max_memory_mb = 64` in `[plugins.overrides.pumboperms]`.

## For plugin developers

Your plugin can ask PumboPerms about permissions, ranks and prefixes, and use its placeholders, on Pumpkin and on PumboProx. See [INTEGRATION.md](INTEGRATION.md).

## Works with other Pumbo plugins

PumboPerms works on its own. When it finds other Pumbo plugins, it works with them:

| Plugin | Together |
| --- | --- |
| [PumboBridge](https://github.com/PumboMC/PumboBridge) | Carries the proxy's ranks to the servers. Without PumboPerms on the proxy, PumboPerms on a server imports the proxy's ranks for this server through the bridge, then writes the permissions on the server itself. With PumboPerms on the proxy, the proxy rules (see [Two builds](#two-builds)). |
| Other Pumbo plugins | Their permissions (`pumbo.bans.ban`, `pumbo.auth.stats`, ...) can be given in groups like any other node. Pumbo plugins can also ask PumboPerms directly, about offline players too. |

## Building

You need Rust stable with the WebAssembly target:

```sh
rustup target add wasm32-wasip2
```

Cargo fetches the shared Pumbo libraries (`pumbo-common`, `pumbo-sdk`, `pumbo-contracts`) from the [PumboProx](https://github.com/PumboMC/PumboProx) repository on the first build.

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

## License

PumboPerms (the core and both builds) is licensed under the [GNU General Public License v3.0](LICENSE). The shared library for Pumbo plugins (`pumbo-common`) is dual-licensed under MIT and Apache-2.0.

PumboPerms is not affiliated with Mojang, Microsoft or the Pumpkin project.

---

<p align="center">
  Part of <a href="https://github.com/PumboMC/PumboProx"><b>PumboProx</b></a>. Everything you need to run a network on Pumpkin.
</p>
