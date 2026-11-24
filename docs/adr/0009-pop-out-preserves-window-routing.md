# Pop-out preserves a window's routing verdict

Pop-out is a reversible user choice for a live tileable window, not a change
to its [first routing verdict](0003-window-identity.md). Keep the pop-out
state separate and session-local. Reclassifying the window as `Untracked`
would blur the distinction between a user-toggled window and one that should
never tile under the app or window rules.

A popped-out window shares [untracked stacking](0002-untracked-windows.md)
but has no workspace membership. When switching workspaces on the same display,
retain keyboard focus only if an untracked or popped-out window already had it.
Raising an overlay must not let it take keyboard focus from another window.

Turning pop-out off admits the window to the workspace visible on its display.
If full, use [traveling overflow](0008-current-workspace-overflow.md), not a
workspace-bound wait: the command asks to resume tiling, not to choose a new
home. Tiling establishes the home when a slot opens.
