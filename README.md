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
  <a href="docs/features.md">Features</a> ·
  <a href="docs/installation.md">Installation</a> ·
  <a href="docs/configuration.md">Configuration</a> ·
  <a href="docs/commands.md">Commands</a> ·
  <a href="docs/how-it-works.md">How it works</a>
</p>

---

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

<p align="center">
  <a href="https://github.com/PumboMC/PumboProx"><img src="assets/pumboprox.webp" alt="PumboProx: everything you need to run a network on Pumpkin" width="100%"></a>
</p>

<p align="center"><b>Running more than one server?</b> <a href="https://github.com/PumboMC/PumboProx">PumboProx</a> is the proxy for Pumpkin networks, with plugins in WebAssembly.<br>PumboPerms runs on it too: ranks live in one place for the whole network and can differ from server to server.</p>

## License

PumboPerms (the core and both builds) is licensed under the [GNU General Public License v3.0](LICENSE). The shared library for Pumbo plugins (`pumbo-common`) is dual-licensed under MIT and Apache-2.0.

PumboPerms is not affiliated with Mojang, Microsoft or the Pumpkin project.

---

<p align="center">
  Part of <a href="https://github.com/PumboMC/PumboProx"><b>PumboProx</b></a>. Everything you need to run a network on Pumpkin.
</p>
