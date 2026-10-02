# Server-confirmed player game modes

Reference repository revision `da728f0ce4d7a5ae0be443b8abe03119858d923e`.
The current reconstruction's packet description, RVA `0x040a24c0` in
`current/1.26.50.26/src/__unmapped/04.cpp`, identifies `SetPlayerGameType` as
the client's request and `UpdatePlayerGameType` as the server's confirmation.
Current `UpdatePlayerGameType::getId`, RVA `0x03aaa170`, returns the same packet
ID as our generated `McpePacketName::UpdatePlayerGameTypePacket`; we use the enum,
not a copied numeric ID. The generated packet preserves a game type, signed actor
unique ID and unsigned player-input tick.

## Local identity and default mode

Named 26.30 `ClientNetworkHandler::handle(UpdatePlayerGameTypePacket)`, RVA
`0x03542aa0`, matches the packet target against player-list **unique IDs**. A
matching local player gets its mode changed and its UI publisher notified. A
matching remote player follows the remote-actor setter instead. A runtime ID, zero
or `-1` is not a wildcard target. The legacy `SetPlayerGameType` handler, RVA
`0x035429a0`, already has an implicit local target.

Named 26.30 `Player::getPlayerGameType`, RVA `0x0a1dde20`, resolves raw game type
`5` through the level's default game type. `Player::setPlayerGameType`, RVA
`0x0a1dd040`, retains that raw default binding while using the effective mode for
its mode-change work. Cinnabar reuses its existing player/default-mode reducer:
an explicit player mode is independent of later default changes, whereas a player
bound to the world default follows those changes. Unknown well-formed game types
remain counted, ignored data, not a disconnect.

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

Named 26.30 `ClientPlayerRewindListener::_onUpdatePlayerGameTypePacketReceived`,
RVA `0x02dac230`, applies a tick-zero packet immediately. With a nonzero tick and
an eligible replay timeline, it instead inserts `GameTypeReplay` at that tick and
suppresses the immediate setter; otherwise it applies immediately. Cinnabar now
decodes the tick, but applies accepted updates at receive FIFO commit rather than
replaying historical movement authority. That timing/replay branch remains a
separate incomplete parity gate; fixing the missing confirmation packet does not
close it.
