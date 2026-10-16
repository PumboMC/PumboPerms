# Configuration

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
| `provider.check-event` | `false` | Pumpkin | Also answer each of Pumpkin's permission checks. For testing only: on Pumpkin 0.2.0 it can hang the server. Needs a restart. |
| `provider.resend-commands` | `true` | Pumpkin | Send players their command list again after a change (not with `check-event: true`). |
| `provider.refresh-seconds` | `5` | Pumpkin | How often PumboPerms checks expired nodes and the operator levels and worlds of online players. Commands, joins and calls from other plugins remove expired nodes at once. `/pp reload` changes it. |
| `import.enabled` | `true` | both | Take over other permission sources at start (on PumboProx: `permissions.yml`). |
| `import.follow` | `false` | both | Import a source again whenever it changes (otherwise once). |
| `import.plugins` | `[]` | Pumpkin | Other plugins to import from. |

On Pumpkin, PumboPerms leaves out a short name that Pumpkin or another Pumbo plugin uses for its own command, with a warning in the log. Pumpkin does not tell plugins which commands third-party plugins registered, so do not use their names. On PumboProx, the proxy refuses a name another plugin holds, and the log says so. `/pumboperms` works in both cases.

Each time `config.yml` loads, PumboPerms keeps a copy as `config.last-good.yml` in its data folder. If the file is not valid YAML at start, PumboPerms starts from that copy and logs an error; without a copy it does not start. `/pp reload` with a broken file keeps the current settings.

After an update, PumboPerms on Pumpkin, and PumboProx for the files of its plugins, give a `config.yml` or `lang/<code>.yml` that nobody edited the new built-in version: they keep a copy of what they wrote in `.generated/` and compare the file with it. A `config.yml` whose built-in values changed stays as it is, and the log names those options, so the values in force never change on their own. A file you edited stays as it is, and the log names the options or texts it lacks and the ones that still have the previous built-in text; remove those keys or delete the file to get the new ones. A deleted file comes back. On Pumpkin, a file an earlier release wrote counts as untouched without a copy too, as long as nobody edited it; any other file without a copy stays, and PumboPerms compares it with this version from then on.

If your edited `lang/<code>.yml` has `target-user: "{0}"`, replies about a player do not show the UUID: delete that line, or write `"{0} &8({1})&r"`.
