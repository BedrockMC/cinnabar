# Personal inventory close response

Reference: current mcsrc `1.26.50.26`, revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, plus the named 26.30 reference.


The matching manager operation is `onContainerScreenClose`: current
`__unmapped/02.cpp`, `0x028c85a0`, and named 26.30
`ItemStackNetManagerBase::onContainerScreenClose`, `0x0a0f8410`. It removes the
pending screen when the retained screen queue has more than its base entry.
Current manager base destructor `0x028c7400` assigns vtable `0x150133010`; the
matching PE's entry at `+0x38` is exactly `0x1428c85a0`. The response payload
is not a screen-identity correlation token.

Cinnabar formerly required a response's id/type to match its personal window.
An otherwise valid response with another type left the window closing; its
timeout then disabled further personal opens for that session. A response now
settles an already transport-admitted personal close independently of those
payload fields. Unsolicited responses during opening/open states or before
transport admission do not close anything. Typed server-initiated closes keep
their identity checks and cursor reconciliation.

Regression coverage runs production keyboard handling with actual `E`/Escape
messages, production inventory ingress, transport admission and three repeated
open/close cycles. It exercises inventory, generic and odd well-formed response
payloads without claiming those are an official BDS's response shapes. It also
checks that a late timeout cannot poison a settled close. The October 2 UTC
offline vanilla-BDS run performs three real E/Escape close/reopen cycles in one
focus interval after the offhand and drop/pickup transactions. The inspected
local `2026-10-02_01.01.27.png` macOS/Metal Retina scale-2 frame shows the
subsequent open and correct final inventory/cursor state. Tested executable
SHA-256: `b51eb853c4b7f04ba555c0dd0b76e5667addcbbae2ab40711483afce5f75e25f`.
This is a live functional witness, not full screen-lifecycle parity.
