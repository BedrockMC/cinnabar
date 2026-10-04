# Flight control across spatial corrections


## Identified native behavior


The existing protocol carries those wire start/stop edges from the simulated
mode. Its `PosDelta` uses end-of-tick motion, matching the native send path;
that lane does not require a flight-specific change.

## Fixed mismatch

An in-session snap called the same hard reanchor as a new session, which reset
the locomotion tracker to walking. A player who started flight locally before
receiving the server's flying ability acknowledgement therefore lost flight
after a spatial correction, and the next movement tick emitted `StopFlying`.

The snap now retains the locomotion tracker alongside its already-retained jump
input. It preserves both local flight state and the previous server flying
state, so a later authoritative flight clear is still detected. Ordinary
session and dimension reanchors retain their separate reset behavior.

The focused regression covers a locally initiated flight with no server
acknowledgement and a flight already acknowledged by the server. It applies
the snap through the transactional controller/ticker reconciliation, then
admits the next tick through the actual outbound ticker. That tick continues
flying without another start or a stop edge and retains the held vertical
control. A later authoritative flight clear still ends acknowledged flight
and produces its stop edge. The ticker's existing spatial correction retains
its previous held input; the snap's replay seed does not affect that path.

## Identified flight travel


- Idle horizontal input is maximum absolute processed axis below the native
  float `0.01`. Creative idle flight multiplies existing vertical motion by
  `0.375` only while neither vertical control is held.
- Held keyboard jump adds `0.15` times vertical fly speed; held keyboard sneak
  adds `-0.22` times that speed. Holding both clears vertical motion.
- Creative idle horizontal friction overrides the normal modifier with
  `0.375`; other idle flight uses `0.75`. Those modifiers affect horizontal drag.





The former flight path damped vertical motion after movement with the same
`0.91`-based horizontal retention. Creative idle damping also happened after
movement. The dedicated flight helpers apply creative idle damping before
collision resolution and retain the separate vertical drag afterward. Their
focused regressions freeze both movement and retained velocity for held
controls, hover, custom abilities and submerged flight.

## Vertical input lanes



Cinnabar sent held jump and sneak, plus raw ascend/descend during flight, but
omitted processed `WantUp`/`WantDown`. The server therefore lacked the flight
vertical controls that the client simulated. The encoder now sends those
processed controls with held jump and sneak. The regression reconstructs the
native server control mask from the outbound flags for up, down, both and
released controls. The protocol regression verifies their named wire rows.

This correction does not establish full flight parity. The wall-time
double-tap detector remains an approximation of the native tick countdown;
rapid-toggle acknowledgement ordering still needs independently identified
reconciliation behavior. Modified movement drag attributes and live server
acceptance remain separate evidence gates.
