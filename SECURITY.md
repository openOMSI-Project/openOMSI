# Security

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through
GitHub instead: **[Report a vulnerability](https://github.com/openOMSI-org/openOMSI/security/advisories/new)**
(the *Security* tab of the repository). Only the maintainers see it.

Say what is affected, how to reproduce it, and what an attacker could do with it. A small
example (a file, a request, a log) helps a lot. The maintainers are volunteers: we aim to
answer within a week, keep you posted while it is fixed, and credit you in the advisory if
you want.

## Supported versions

Only the latest release. The game updates itself, so fixes are not backported to older
builds.

## What counts

Examples of what we want to hear about:

* the **multiplayer and dedicated server**: a client that can crash or take over a server
  or another player's game, get past the admin password or the vehicle list, or read files
  from the host;
* **content files**: a map, vehicle, script or archive that makes the game read or write
  files outside the game's folders, or run code, just by being loaded;
* the **launcher's updates and downloads**: anything that could make it install something
  that is not an openOMSI release;
* the crash reporter or the logs sending or showing more than they say (passwords, tokens,
  personal paths beyond the user name).

Not a vulnerability: a plugin (Lua, or an OMSI plugin's DLL) doing harm - plugins run with the game's rights
by design, so install only plugins you trust - and a content file that only makes the game
crash, which is a normal bug and goes into an [issue](https://github.com/openOMSI-org/openOMSI/issues/new/choose).
