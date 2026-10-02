# Native sparse inventory prediction

Reference: local `mcsrc-1.26.50`, reconstruction revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current Windows client
`1.26.50.26`, matching PE SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.
Our implementation is independently written from these contracts.

## Identified native functions

| Current RVA | Function | Contract |
| --- | --- | --- |
| `08942880` | `SparseContainer::getItem` | Return the absolute sparse item when the cell is predicted; otherwise return its backing item. |
| `08942ce0` | `SparseContainer::setItem` | Save the new sparse item and invoke the set listener. |
| `089457b0` | `SparseContainerSetListenerClient::postSetItem` | Stamp every changed item with the current typed request id, including an emptied item, and register its container with the request. |
| `089442a0` | `SparseContainerClient::_networkUpdateItem` | Update the backing container without rebasing or subtracting an active prediction. |
| `08934960` | `ItemStackRequestActionHandler::_validateRequestSlot` | Resolve odd-negative request references through request-id, container-runtime-id, and requested-slot assignments. A request id is not a globally unique item identity. |
| `028d10d0` | `ItemStackNetManagerClient::handleItemStackResponse` | Find the issued request across retained screens; skip unknown ids. Process each answer immediately. |
| `089446a0` | `SparseContainerClient::tryPushSlotPrediction` | Requested slot locates the sparse item; actual slot receives the correction. Validate amount/net-id pairing. A later owner selects the historic path. A missing sparse cell is skipped. |
| `08944f90` | `SparseContainerClient::_pushHistoricPredictionItem` | Correct backing using the request's historic item without removing the newer active prediction. |
| `08943e40` | `SparseContainerClient::clearAllPredictions` | Remove remaining active cells whose stamp is the answered request, not older or later owners. |
| `028cfc20` | `ItemStackNetManagerClient::_clearPredictiveContainerRequest` | Remove the answered historic snapshot and clear that request's active sparse cells. |
| `028c85a0` | `ItemStackNetManagerBase::onContainerScreenClose` | Retire the oldest retained screen after close acknowledgement. Its late replies no longer own a retained screen. |

The slot field formerly called `hotbar_slot` in our normalized response is the
wire `requested_slot`; it is not a second hotbar address. Both addresses use the
same canonical container projection, including offhand wire slot 1/content slot
0 and the hotbar/inventory aliases. The compatibility field name remains for now.

## Bug and implementation

The previous ledger reapplied relative operations to authoritative backing data
when folding and accepting requests. A drop from 64 to 63 could consequently
display 62 after the authoritative count arrived. A source emptied by an
authoritative update made a successful Take appear stale; recovery then poisoned
cursor authority and refused every later cursor gesture. Predicted items also
retained old positive server ids and split halves were refused until an answer.

The ledger now keeps absolute per-cell snapshots with active request ownership,
including empty cells. New actions name prior sparse owners; occupied predictions
carry that owner's negative request id. Replacing an owner preserves its historic
snapshot but does not resurrect it when the newer owner is answered. Accepted
corrections update backing data from the requested snapshot, never by replaying an
operation against a pushed count. A nonempty reply to an emptied prediction uses
the retained pre-empty item, matching native zeroed-out-item handling. Count/id
pair errors are counted skips. Well-framed negative response ids are retained for
consumer validation rather than causing a protocol disconnect. Damage correction
zero is authoritative too; it is not mistaken for an absent repair.

The offhand regression uses the observed vanilla BDS shapes: Take offhand name
34/slot 1/id 49 to cursor name 59/slot 0; acceptance empties offhand with id -1
and gives cursor id 49/count 1. An offhand-empty authority push before that reply
does not lock the cursor. Placement works both immediately after acceptance and
as a dependent request while Take is still unanswered, followed by a further
unrelated gesture.

## Verification and remaining gates

Focused source-contract tests cover drop push ordering, request-id/slot chaining,
split halves, sparse empty ownership, historic replies, missing active cells,
requested-to-actual slot remapping, backing replacement, invalid count/id pairs,
and the offhand sequence. Existing inventory tests were migrated from the old
proxy delta contracts to the identified native contracts.

Live acceptance on 2026-10-02 UTC used the canonical macOS/Metal build at
Retina scale 2, offline loopback vanilla BDS (the server version pinned in
`assets/bedrock-target.json`). Client executable SHA-256:
`b51eb853c4b7f04ba555c0dd0b76e5667addcbbae2ab40711483afce5f75e25f`.
At 00:58:46–49, shield offhand Take, cursor Place into inventory 13, Take back,
Place into offhand, grass Take, and Place into inventory 9 all received Accepted
answers (-7, -9, -11, -13, -15, -17). The offhand-empty content push preceded
the first answer, reproducing the reported poison sequence. The inspected
`2026-10-02_00.58.49.png` frame showed shield restored to offhand, grass 6 in
inventory 9, and an empty cursor. A further picked-up diamond Take/Place also
passed (-23, -25); `2026-10-02_01.01.27.png` verified the final stacks after
three close/reopen cycles. Screenshots remain local, outside git.

Real drop/pickup also passed through the separate
[normal transaction receive path](inventory-normal-transactions.md).
Existing bounded transport-pressure,
timeout/refresh recovery, registry-change recovery, and multi-screen UI admission
policies remain Cinnabar safety policies, not a claim that every native inventory
manager lifecycle and arbitrary container has been reproduced.
