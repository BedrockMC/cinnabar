# Shield blocking query and owner equipment


## Rendering contract


The pinned Shield attachable's pre-animation scripts require both owner hands.
The main-hand blocking predicate is `query.blocking`, no Shield in the offhand,
and a Shield in the main hand. The offhand predicate is `query.blocking` and a
Shield in the offhand. The authored wield controller selects hand by typed
`context.item_slot` and transitions using the same blocking query. Pose
keyframes, rotations and transition timing remain in the pack; our runtime
must provide the owner equipment and metadata rather than duplicate poses.

Attachable input now retains both owner hand identifiers. Previously it
constructed an artificial query actor holding only the rendered item, which
lost offhand priority and the main-hand Bow predicate for an offhand Shield.

## Gameplay and authoritative state



Local player movement exclusion does not exclude authoritative metadata.
The regression publishes an actual `ActorEvent::Metadata` update, then
client-fed movement and predicted item-use updates, and verifies flag 72
survives. The attachable VM regression separately verifies that sneak alone
does not satisfy blocking and that a Shield in the other hand takes priority.

## Verification limitation

The pinned Dragonfly fixture's `server/item/shield.go` explicitly leaves
raising, absorbed damage and axe disable unimplemented. Its projectile
implementation also notes that Shield blocking is not implemented. That
fixture therefore cannot establish a native Shield gameplay or animation
gate, and injecting flag 72 into it would only manufacture the witness.
Use a vanilla BDS or a compatible server which supplies authoritative flag
72. No client-side fallback bypasses the native metadata query.

The offline vanilla-BDS test supplied actual flag 72 transitions when sneaking:
false/standing, false/sneaking, true/sneaking, true/standing, false/standing.
The owner subsequently confirmed the blocking animation works. The temporary
state trace was removed after that authority check. Application submission now
includes separate main/offhand rigs and texture bindings; fresh macOS/Metal
Retina captures show both shields and their inventory/offhand icons.
These are functional and rendered-frame checks, not a matched native gallery.
Patterned/glint layers and damage/cooldown parity remain incomplete.
