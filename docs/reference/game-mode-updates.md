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

## Creative destruction is mode-driven (2026-10-04)

The current reconstruction's `GameMode::startDestroyBlock` (`028b4700`) and
`continueDestroyBlock` (`028b5160`, `__unmapped/02.cpp`) select the creative
destruction route through `Actor::isCreative` (`01c3ac00`, `__unmapped/01.cpp`).
That predicate reads only the game-type component: Creative, or world-default
resolving to Creative. It does not inspect the Instabuild ability.
`Player::getDestroyProgress` (`00202280`, `__unmapped/00.cpp`) passes **Flying**
into the destroy context. `PlayerDestroy::getDestroyProgress` and `getDestroySpeed`
(`02bbe540`, `02bbe860`, `__recovered/PlayerDestroy.cpp`) apply the speed, hardness,
harvest and movement penalties; neither selects instant destruction from Instabuild.
Current ability serialization (`001deef0`) independently distinguishes Flying
at offset `+0x6c` from Instabuild at `+0x84`.

Cinnabar previously let received Instabuild override `instant_break`, so survival
could take the creative mining route after a mode change despite the correct HUD.
The capability now follows the confirmed game mode only. Regressions cover
Survival with Instabuild enabled, Creative with it disabled, and a committed
Creative → Survival → Creative transition while retaining the same wire evidence.
Other ability grants and the passive evidence owner are unchanged. Native mode-layer
refresh (`0021b1d0`) is a separate incomplete gate; clearing all received layers
would incorrectly discard higher-priority server grants.

In the October 4 macOS/Metal scratch-BDS run, the console confirmed the local
player's Creative-to-Survival transition. The user tested the rebuilt client and
confirmed that breaking and dropped-item testing now work correctly. The
capability matrix and retained-evidence transition regressions also pass.

## Incomplete historical replay

