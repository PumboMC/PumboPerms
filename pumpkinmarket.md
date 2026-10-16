
Project is 100% open source GPL 3.0  - [PumboPerms](https://github.com/PumboMC/PumboPerms)

No code or parts was copied from [LuckPerms](https://github.com/LuckPerms/LuckPerms), but is strongly modeled on, that why we still want to thank [@lucko](https://github.com/lucko) for the solutions and standards it gave us.  

### Features?
all whats you are familiar with - rest on gh

## ***No restart or reload needed in any version***

  Permissions and groups are updated automaticaly and working insant without any restart or reload  
  
![/pp in game](assets/market/demo.gif)
  

## Performance

Measured on an Apple M4 Pro against pumpkin 0.2.0 


|                                            | Pumpkin (one server)                                                            |
| ------------------------------------------ | ------------------------------------------------------------------------------- |
| Memory                                     | about 13 MB with 100 players online and 8000 permissions                        |
| Memory for 400 000 known names             | +0.4 MiB, runs within a 64 MB plugin limit                                      |
| One change among 20 000 players in a group | heap peak +24.4 MiB                                                             |
| Change in a group every player has         | about 10 ms for 100 players and 8000 permissions                                |
| Change for one player                      | about 4 ms                                                                      |
| Import of 3000 players                     | `/pp import <file>`: 27-31 ms, 69-74 ms again (it saves the current data first) |
| Join with 400 000 known names              | up to 0.135 ms (0.57 ms for the first after the start)                          |
| Opening a database with 400 000 names      | 15 ms, with loading the data                                                    |
| Database after a crash (kill -9)           | about 30 s: waits for the old lock to go stale                                  |


Under load on the test network (4 players sending 100 commands a second each, 20 `/pp` changes a second, 3000 players in the data) nobody was disconnected.      


*PumoProx and rest of our plugins excet of PumboPerms is still (MVP)  
every plugin will be grinded to production state.    
