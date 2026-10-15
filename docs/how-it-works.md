# How it works

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
