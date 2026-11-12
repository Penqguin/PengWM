# Current-workspace placement with traveling overflow

Status: Accepted and implemented, 2026-11-11.

Fresh tileable windows open on the current workspace by default. When full,
the workspace hosts them as overlays that travel on explicit workspace selections
until a slot becomes available. Tiling establishes a home. Explicit placement
and returning settled windows retain their chosen home when full and wait there
as overlays. This makes capacity visible where the user is working and preserves
the destination of an explicit placement.

Existing app associations become opt-in through a placement setting. Slot
allocation prioritizes returning settled windows, explicitly placed windows,
then traveling overflow, using opening order within each group. Ordinary monitor
focus changes leave traveling overflow on its current monitor.

Restart recovery extends ADR-0005 only for surviving windows whose identity can
be verified. Fresh or recreated windows use fresh-window routing. The existing
bundle/title projection cannot establish survival and remains insufficient as
recovery evidence. Recovery compares boot-session UUID, process start time,
PID, WindowId, and a unique readable AXIdentifier. Ambiguous or unavailable
evidence uses fresh routing. ADR-0006's topology membership protection
and ADR-0007's per-display action gate continue to govern their respective paths.

This introduces explicit pending placement state and ordered admission across
the lifecycle, replacing capacity spill to another global workspace. The cost
is coordination across membership, focus, parking, switching, and persistence.
The agreed behavior and implementation sequence are in
[the window placement spec](../specs/0003-window-placement.md).
