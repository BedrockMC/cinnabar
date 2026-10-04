# Barrier selection overlays

Barrier blocks keep their collision and remain available to the interaction ray.
Selection publication suppresses both the filled highlight and outline outside
Creative mode. Creative retains the existing overlay. The decision uses the
canonical block identifier in the current network ID space and the local player’s
effective game mode; it never guesses from texture visibility or cube geometry.

Tests cover sequential and hashed IDs, Survival, Adventure, Creative and unknown
modes, plus unchanged stone selection and barrier collision/picking.
Version-matched native comparison remains incomplete: the selection eligibility
path was inspected, but the remaining service-backed comparison was unavailable.
