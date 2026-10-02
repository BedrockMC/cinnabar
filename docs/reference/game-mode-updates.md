# Server-confirmed player game modes


## Local identity and default mode



## Cinnabar receive path

The raw world-packet admission list and normalized event decoder now accept
`UpdatePlayerGameType`. Its targeted UI event retains the unique ID and tick until
the ordered world stream admits it against the bootstrap local unique ID. Only
then is it converted to the existing local game-mode event. The committed UI path
updates the retained HUD and existing movement/inventory capability inputs. A
targeted event injected directly into the UI is ignored because that layer cannot
establish local-player identity.

Regressions exercise generated packet encoding through raw ingress, all supported
mode/default mappings, signed identity and full-width tick preservation, malformed
truncation, foreign/sentinel targets, FIFO fencing, and committed HUD/input authority.
The October 1 offline vanilla-BDS run changes survival to creative and back
without reconnecting. macOS/Metal Retina captures `2026-10-01_23.37.28.png`
and `23.48.14.png` show the creative catalog and survival crafting inventory;
the intervening HUD captures show hearts/hunger returning in survival. The
server console confirms each mode command. This closes the missing-packet
functional regression, not historical replay or full native visual parity.

## Incomplete historical replay

