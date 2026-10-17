# Features

PumboPerms decides who may do what on your server. You put players into groups, give the groups permissions, prefixes and suffixes, and move players up and down rank ladders.

At the first start it takes over the permissions your server already has, so you do not start from zero. On Pumpkin it does that in its first tick, before it writes anything: the proxy's ranks for this server through PumboBridge, the plugins listed in `import.plugins`, or, when neither gives anything, the permissions other plugins set on the players online. On PumboProx it takes over the proxy's `permissions.yml`.

| Feature | What it does |
| --- | --- |
| **Groups** | Players get groups. A player without a group that applies everywhere is also in `default`, on every server and in every world. |
| **Inheritance** | A group can have parent groups and gets their permissions. |
| **Weights** | When two groups disagree, the heavier one wins. |
| **Wildcards** | `pumbo.bans.*`, `minecraft:command.*`, `*`. In PumboPerms the most specific node decides. Pumpkin reads wildcards from the broadest one, so see [How Pumpkin gets the permissions](how-it-works.md#how-pumpkin-gets-the-permissions). |
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
| **Placeholders** | `%pumboperms_prefix%`, `%pumboperms_rank%` and more for chat, tab and scoreboard plugins (from 0.1.2-beta). See [INTEGRATION.md](../INTEGRATION.md). |
| **Autocomplete** | Tab completes subcommands, players, groups, tracks and permission nodes, and offers players, groups and tracks only with the right to see them. On PumboProx it offers only the subcommands you may use; on Pumpkin everyone who may use `/pp` sees all subcommand words. See [Tab completion](commands.md#tab-completion). After a change each player's command list is updated. |
| **Look** | "Pumbo" in orange, "Perms" and the `/pp` command words in yellow. Info views show labels with their values, section headings, `true` in green and `false` in red, and a prefix in its own colours. The console gets the same text without colours. |

## Works with other Pumbo plugins

PumboPerms works on its own. When it finds other Pumbo plugins, it works with them:

| Plugin | Together |
| --- | --- |
| [PumboBridge](https://github.com/PumboMC/PumboBridge) | Carries the proxy's ranks to the servers. Without PumboPerms on the proxy, PumboPerms on a server imports the proxy's ranks for this server through the bridge, then writes the permissions on the server itself. With PumboPerms on the proxy, the proxy rules (see [Two builds](installation.md#two-builds)). |
| Other Pumbo plugins | Their permissions (`pumbo.bans.ban`, `pumbo.auth.stats`, ...) can be given in groups like any other node. Pumbo plugins can also ask PumboPerms directly, about offline players too. |

## For plugin developers

Your plugin can ask PumboPerms about permissions, ranks and prefixes, and use its placeholders, on Pumpkin and on PumboProx. See [INTEGRATION.md](../INTEGRATION.md).
