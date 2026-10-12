# Untracked windows — which macOS windows should never be tiled, hidden, or workspace-managed

> **New convention note:** this is the first research-notes file in the repo (`docs/research/`
> is a new directory — the repo previously only had `adr/` and top-level docs). Ground rules
> used here, and proposed for future notes: every claim carries an inline citation (Apple
> docs / SDK headers / linked source files / live probe on this machine); anything not
> verified is marked **unverified**. This file records findings only — nothing is implemented.

*Date: 2026-10-10. Source snapshots: yabai `master`, Amethyst `development`, AeroSpace `main`, all fetched today. Apple SDK headers read from the local CommandLineTools SDK (`xcrun --show-sdk-path` → `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk`); the SDK is Apple's own primary source where the website omits numeric values. Live probes were read-only: no app was launched and nothing on screen was touched.*

---

## 1. Question & current behavior in PengWM

**Question.** Which macOS windows should be *untracked*: never tiled, never hidden when the
user switches workspace, never given tree membership — simply left where macOS keeps them,
rendering on top until the user closes them. Driving example: **System Settings**
(`com.apple.systempreferences`), which today gets tiled and the user considers wrong
("by default settings should already be untracked by default").

**What PengWM does today**

- Discovery classifies each window once, via `WindowClass::classify_window(role, subrole)`
  (`pengwm-daemon/src/adapter.rs:52-64` — [file](../../pengwm-daemon/src/adapter.rs)):
  `AXStandardWindow` → `Standard` (tiles); `AXDialog` / `AXSystemDialog` /
  `AXFloatingWindow` → popup classes; `AXSheet` and unknown subroles drop. The subrole is
  read in `ax_element::classify` (`pengwm-daemon/src/macos/ax_element.rs:385-389`).
- Routing applies the class: popup classes or windows of `restricted_apps` bundles become
  **workspace-bound popups**; everything else tiles
  (`pengwm-daemon/src/state/lifecycle.rs:43-55`). Popups hide with their workspace and come
  back on top on switch-back (`CONTEXT.md` → *Popup*; `docs/configuration.md` → *Popups*,
  lines 150-166). There is **no cross-workspace window class today** — a popup is bound to
  the workspace active on the monitor that contained it at creation
  (`lifecycle.rs:73-78`, `add_popup_to_workspace`) and hides through the same parking path
  as tree windows (`CONTEXT.md` → *Hiding*).
- `restricted_apps` defaults to `[]`
  (`pengwm-daemon/src/config/mod.rs:74-75`, default ctor at `:273`; documented in
  `docs/configuration.md` line 14). So System Settings has no default escape hatch: its
  main window is a level-0 `AXStandardWindow` (see §4) → classifies `Standard` → tiles.
- Some system overlays never reach PengWM at all: `running_app_pids` keeps only
  **Regular** activation-policy apps (`pengwm-daemon/src/macos/ns_workspace.rs:47-56`).
  Control Centre (`com.apple.controlcenter`, Accessory), Notification Centre
  (`com.apple.notificationcenterui`, Accessory), Quick Look
  (`com.apple.quicklook.QuickLookUIService`, Prohibited) and the open/save panel service
  (`com.apple.appkit.xpc.openAndSavePanelService`, Prohibited) are all non-Regular on this
  machine (live probe, §5), so they are invisible to the poll path already. Note the
  dead-pid sweep deliberately uses `running_pids_any_policy` because non-Regular apps own
  windows (`ns_workspace.rs:58-69`; `CONTEXT.md` → *DeadPidSweep*).

**Gap.** The structural signal PengWM has not consumed is the **CG window level** (§2) — the
one piece of macOS state that says "this window floats above the normal layer / over every
space" — plus a default-`restricted_apps`-style floating list for regular-policy system
apps like System Settings and Calculator.

---

## 2. The macOS mechanism: what "shows on top on every workspace" actually is

Two AppKit properties define it:

- **`NSWindow.Level`** — "The stacking of levels takes precedence over the stacking of
  windows within each level" ([Apple: NSWindow.Level](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct)).
  The constants are `#define` aliases of CoreGraphics values
  (`AppKit.framework/Headers/NSWindow.h:192-201`, local SDK), whose numeric values are
  **not** shown in the web docs but are in Apple's CoreGraphics header
  (`CoreGraphics.framework/Headers/CGWindowLevel.h:67-79`, local SDK):

  | Constant | Value | Meaning (header comment/Apple doc) |
  |---|---|---|
  | `kCGNormalWindowLevel` (`normal`) | **0** | "The default level for `NSWindow` objects" — [NSWindow.Level.normal](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct/normal) |
  | `kCGFloatingWindowLevel` (`floating`) | **3** | "Useful for floating palettes" — [NSWindow.Level.floating](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct/floating) |
  | `kCGModalPanelWindowLevel` (`modalPanel`) | **8** | "The level for a modal panel" |
  | `kCGMainMenuWindowLevel` (`mainMenu`) | **24** | "Reserved for the application's main menu" |
  | `kCGStatusWindowLevel` (`statusBar`) | **25** | "The level for a status window" |
  | `kCGPopUpMenuWindowLevel` (`popUpMenu`) | **101** | "The level for a pop-up menu" |
  | `kCGScreenSaverWindowLevel` (`screenSaver`) | **1000** | "The level for a screen saver" |
  | `kCGDesktopWindowLevel` | `kCGMinimumWindowLevel + 20` (int32-min territory, far **below** 0) | desktop/decal windows |

  Live cross-check on this machine right now: the only non-0 layers on screen were
  `layer=24` (Window Server "Menubar"), `layer=2147483630` (Window Server) and Notification
  Centre's backdrop at a deep negative layer — i.e. everything Apple keeps permanently
  above/below the normal layer carries a non-zero level (probe script +
  output recorded during this research).

- **`NSWindow.CollectionBehavior`** — controls Spaces membership:
  [`canJoinAllSpaces`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallspaces)
  "The window can appear in all spaces",
  [`moveToActiveSpace`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/movetoactivespace)
  "When the window becomes active, move it to the active space instead of switching spaces",
  [`transient`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/transient)
  "The window floats in Spaces and hides in Mission Control" — and Apple's own `NSWindow.h`
  comments encode the same coupling for window levels:
  "Default behavior if `windowLevel != NSNormalWindowLevel`" (per-CollectionBehavior
  constant comments, lines 106-113 of the local SDK header), i.e. a window floats in
  Spaces by default whenever its level is not normal.

**"Untracked overlay" = non-zero level (+ canJoinAllSpaces).** A window at level > 0 is
stacked above every level-0 window app-wide, and system overlays also set
canJoinAllSpaces so they appear regardless of which workspace is active.

**How PengWM can detect it.** PengWM already extracts the CG window id from its cached AX
element via `ax_window_id_from_element` → private `_AXUIElementGetWindow`
(`pengwm-daemon/src/macos/ax_element.rs:116-132`). One
`CGWindowListCopyWindowInfo(.excludeDesktopElements | .optionOnScreenOnly)` pass returns per
window: `kCGWindowNumber` (id), `kCGWindowLayer` (level), `kCGWindowOwnerPID`,
`kCGWindowBounds`, optionally `kCGWindowName`/`kCGWindowOwnerName`
([Apple: CGWindowListCopyWindowInfo keys](https://developer.apple.com/documentation/coregraphics/cgwindowinfo);
enumerated with comments in Apple's `CoreGraphics.framework/Headers/CGWindow.h:44-127`,
local SDK). Match the AX-extracted window id against that dictionary → read `kCGWindowLayer`;
`level != 0` ⇒ treat as untracked overlay. Caveats from the header itself: `kCGWindowName`
is "If present" (optional — and on modern macOS requires
[screen-recording entitlement](https://developer.apple.com/documentation/coregraphics/cgwindowisoptionsonscreenonly)
to expose other apps' window names — **unverified**), while id/layer/ownerPID are always
present for on-screen windows.

**This is exactly how AeroSpace does it**: its `getWindowLevel(for:)` calls
`CGWindowListCopyWindowInfo`, maps `kCGWindowNumber → kCGWindowLayer`, classifies
level `0` as `.normalWindow`, `3` as `.alwaysOnTopWindow`, anything else `.unknown(level)`
([AeroSpace `windowLevelCache.swift`](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/windowLevelCache.swift)),
and its window filter drops windows whose level is not `.normalWindow` — for the apps
where the level check is already rolled out (§3).
And yabai does the equivalent consistently: it tiles only windows where
`window_is_standard(window) && window_level_is_standard(window)` (`window_level_is_standard`
being `level == CGWindowLevelForKey(kCGNormalWindowLevelKey)`:
[yabai `window.c:1079-1083`](https://github.com/koekeishiya/yabai/blob/master/src/window.c#L1080:L1083),
`g_layer_normal_window_level = CGWindowLevelForKey(LAYER_NORMAL)` with
`#define LAYER_NORMAL kCGNormalWindowLevelKey` —
[yabai `yabai.c:145`](https://github.com/koekeishiya/yabai/blob/master/src/yabai.c#L145),
[yabai `misc/macros.h:44`](https://github.com/koekeishiya/yabai/blob/master/src/misc/macros.h#L44)),
and **also refuses to manage sticky windows** (windows present on more than one
space), reading stickiness via `SLSCopySpacesForWindows` — see §3.

---

## 3. What other tiling WMs float/unmanage by default

| WM | Gate | What it excludes from tiling by default | Source |
|---|---|---|---|
| **yabai** | `window_manager_should_manage_window` | (1) **sticky** windows — `window_is_sticky` reports a window whose `SLSCopySpacesForWindows` set spans > 1 space (i.e. canJoinAllSpaces); (2) non-standard subrole; (3) **non-normal window level**; (4) non-movable windows. Everything else tiles. | [window_manager.c:272-280](https://github.com/koekeishiya/yabai/blob/master/src/window_manager.c#L272:L280) (`return (window_is_standard(window) && window_level_is_standard(window) && window_can_move(window)) || ...`), guarded by `if (window_is_sticky(window->id)) return false;`; [`window_is_sticky`](https://github.com/koekeishiya/yabai/blob/master/src/window.c#L843:L856); [Real-window+subrole set](https://github.com/koekeishiya/yabai/blob/master/src/window.c#L1049:L1063) (`window_is_real` accepts AXStandardWindow/AXFloatingWindow/AXDialog but only AXStandardWindow is *managed*). |
| **Amethyst** | `shouldBeManaged()` | Manages only **movable** windows whose subrole is exactly `AXStandardWindow` — dialogs/system dialogs/sheets/popups are simply never managed (macOS leaves them floating on their own). No dialog or level machinery beyond that. Floating *apps* are config-driven (`floating` bundle list, `floating-is-blacklist`); with an empty list, whether an unlisted app floats is decided by the **blacklist flag default = true**, meaning unlisted apps do **not** float — i.e. shipped defaults float **nothing** by bundle id. Optional `float-small-windows` heuristic. System Settings tiles under shipped defaults. | [Window.swift `shouldBeManaged`, lines 276-289](https://github.com/ianyh/Amethyst/blob/development/Amethyst/Model/Window.swift) (`guard let subrole ... subrole == kAXStandardWindowSubrole`); [UserConfiguration.swift:729-733](https://github.com/ianyh/Amethyst/blob/development/Amethyst/Preferences/UserConfiguration.swift) (`guard storage.object(forKey: .floatingBundleIdentifiersIsBlacklist) != nil else { return true }`); [default.amethyst:211-212](https://github.com/ianyh/Amethyst/blob/development/Amethyst/default.amethyst) ships `"floating": []`, `"floating-is-blacklist": true`. |
| **AeroSpace** | `isWindowHeuristic` + `isDialogHeuristic` + ship-widening level filter | (a) **Dialog heuristic:** windows whose subrole ≠ `AXStandardWindow` float by default; also "windows without a fullscreen button (…System Settings… Calculator…, excluding terminal apps) are generally considered dialogs" — the docs' own warning-level heuristic includes System Settings, Calculator, About-This-Mac, IntelliJ dialogs, Finder copy dialog. "Windows that are recognized as dialogs are floated by default." (b) **Window-level filter (partially rolled out):** `windowLevel != .normalWindow` → not a window at all ("popup"), currently enabled for Slack/Chrome/Brave/Firefox/iTerm2/Outlook/etc. — i.e. per-bundle rollout because a level-based answer needs per-app evidence. (c) dialog heuristic special-cases 1Password (`id == ._1password && windowLevel != .normalWindow`), iPhone Simulator, Photo Booth, Ghostty quick terminal. | [AxUiElementWindowType.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/model/AxUiElementWindowType.swift) (`if get(...) != kAXStandardWindowSubrole { return true }`, Photo Booth / 1Password examples / Firefox-PiP tweaks); [windowLevelCache.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/windowLevelCache.swift); [docs/guide.adoc:788-800](https://github.com/nikitabobko/AeroSpace/blob/main/docs/guide.adoc) ("Windows that are recognized as dialogs are floated by default… windows without a fullscreen button… System Settings…"). |
| **Hammerspoon** (community convention, not a tiling WM) | `hs.window.filter.allowedWindowRoles` = `{AXStandardWindow, AXDialog, AXSystemDialog}` | Determines which AX subroles count as "windows" at all for its scripting helpers (community snippets then float specific apps at a layer above). | [windowfilter.lua → allowedWindowRoles](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/window/window_filter.lua) (`windowfilter.allowedWindowRoles = {['AXStandardWindow']=true,['AXDialog']=true,['AXSystemDialog']=true}`). |

Not reviewed for defaults: Rectangle / Rectangle Pro's default float-app list — their
pinned/floating feature is Pro-only and not documented in the OSS repo (Rectangle OSS is
drag-to-resize only); no primary source found. **unverified**

Summary of the convergent conventions:

- **AX subrole** is the primary structural signal everywhere (yabai, Amethyst, AeroSpace,
  Hammerspoon).
- **CG window level** is the authoritative signal for *overlays* (yabai: never manages
  level ≠ 0 windows; AeroSpace: classifies level ≠ 0 windows as popups — rolled out
  bundle-by-bundle to avoid false positives).
- Apple's own apps: the *overlay* ones (Spotlight, Control Centre, Notification Centre,
  HUDs) are non-Regular-policy processes on this machine (§5) and therefore invisible to
  PengWM's poll already; the *Regular-policy utilities* whose windows keep being tiled
  (System Settings, Calculator…) are exposed exactly like any standard app and need an
  explicit default-float list.

---

## 4. System-app audit

Bundle ids verified against Apple's own id list where available
([AeroSpace goodies "List of Apple application IDs"](https://github.com/nikitabobko/AeroSpace/blob/main/docs/goodies.adoc#popular-apps-ids),
lines 255-303 — Apple app bundle ids: System Settings `com.apple.systempreferences`,
Calculator `com.apple.calculator`, etc.). Activation policies below are from a **live
probe** on this machine (macOS, `NSWorkspace.runningApplications` +
`CGWindowListCopyWindowInfo`), which measured both directions: what appears as Regular, and
which on-screen windows carry a non-zero layer.

### 4a. Regular-policy system apps (PengWM *sees* these; they tile today)

| App | Bundle id | What happens on screen | Evidence |
|---|---|---|---|
| **System Settings** | `com.apple.systempreferences` | Present in AeroSpace's official doc example: its `on-window-detected` sample routes it to `layout floating` ([guide.adoc:540-551](https://github.com/nikitabobko/AeroSpace/blob/main/docs/guide.adoc)); yabai would unmanage it only if the user adds a rule — under shipped gates it is `window_is_standard && level==0 && movable` so yabai **tiles** it too (inference from yabai gates in §3 — its own README's System-Settings references are about Mission Control settings, not tiling; no official "float Settings" rule found in yabai's shipped examples). Main window: `AXStandardWindow`, level 0 (structural inference from yabai/AeroSpace gates; the user reports it tiles in PengWM). | AeroSpace guide link above; [AeroSpace goodies id list](https://github.com/nikitabobko/AeroSpace/blob/main/docs/goodies.adoc#popular-apps-ids). |
| Calculator | `com.apple.calculator` | AeroSpace dialog heuristic: "windows without a fullscreen button … Calculator" → floats by default there. Under PengWM it would classify `Standard` and tile. | [AxUiElementWindowType.swift comment block](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/model/AxUiElementWindowType.swift); goodies list. |
| **Activity Monitor** | `com.apple.ActivityMonitor` | Regular (live probe: found running as Regular, pid 26253). Normally tiled in most WMs; not a float-by-default candidate. | Live probe. |
| Photo Booth | `com.apple.PhotoBooth` | AeroSpace special-cases it (`if id == .photoBooth { return true }` — float). | [AxUiElementWindowType.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/model/AxUiElementWindowType.swift). |
| Notes / TextEdit / Preview etc. | `com.apple.Notes`, `com.apple.TextEdit`, `com.apple.Preview` | Standard document windows — correctly tiled; not in any WM's float-by-default list. | goodies list; no float treatment in AeroSpace/yabai default docs. |

### 4b. Non-Regular-policy system processes (already invisible to PengWM's poll)

Live probe on this machine, policy tags as labeled by `NSRunningApplication.activationPolicy`:

| Surface | Owner process (bundle id, if any) | Policy (live) | Notes |
|---|---|---|---|
| **Spotlight** panel | `com.apple.Spotlight` | **unverified** — Spotlight was not open during the probe; not observed. It is known to be a separate system process (not the host app's Regular window tree) — but not confirmed here. Its activation policy is a follow-up probe. | yabai treats Spotlight's popup as an unmanaged overlay implicitly (non-standard window tree), consistent with AeroSpace not managing it (its AXWindow tree never exposes Spotlight results). Neither docs page found an explicit statement; treat as needing a live probe before shipping a rule. |
| **Control Centre** panel | `com.apple.controlcenter` | **Accessory** (live: pid 1044, "Control Centre") | Non-Regular, invisible to PengWM poll already. |
| **Notification Centre** | `com.apple.notificationcenterui` | **Accessory** (live: pid 1108) | Its on-screen backdrop window carried a deeply negative `kCGWindowLayer` in the live `CGWindowListCopyWindowInfo` dump — i.e. a decal-level window, not a normal-window overlay. |
| **Quick Look** panel | `com.apple.quicklook.QuickLookUIService` | **Prohibited** (live: pids 1261, 62595, 76522) | Never a target of PengWM's poll. AeroSpace's comment explicitly says Finder preview ("hit space") subroles as "Quick Look" but its window tree stays service-owned. |
| **Open/Save panels, security panels** | `com.apple.appkit.xpc.openAndSavePanelService` | **Prohibited** (live) | The AX tree of the *host* Regular app still exposes the sheet to the window manager (the sheet is a child window of the calling app, subrole `AXSheet` → dropped by PengWM today; AeroSpace describes native file pickers as `kAXDialogSubrole` — consistent with apps that expose the panel directly, e.g. IntelliJ). [AeroSpace comment](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/model/AxUiElementWindowType.swift) ("macOS native file picker (…') (kAXDialogSubrole)"). |
| **Volume/brightness HUDs** | owner process unconfirmed (system overlay; not `com.apple.AudioCaptureUI`). | **unverified** — no HUD was open during the probe; community evidence in the yabai/AeroSpace ecosystems treats these as separate overlay processes. | Needs a live probe to confirm. |
| **CameraView / green-pill camera indicator** | system window owned by… | **unverified** — not observed live; owner process unknown. | Needs a live probe. |
| **Keychain password prompts** (Keychain access unlock, PDF password fields) | `SecurityAgent` / `Keychain-Circle-Notification` (Accessory, live: pid 1077). | Accessory for the circle-notification; the actual password panel owner status **unverified**. | Needs a live probe. |
| **Widgets / wallpaper / MenuBar agent / LoginWindow / CameraController** etc. | `com.apple.wallpaper.agent` (Accessory), `com.apple.MenuBarAgent` (Accessory), `com.apple.loginwindow` (Accessory), widget extensions (Prohibited) | live probe (multiple pids) | All invisible to PengWM's Regular-only poll. |
| **Dock** | `com.apple.dock` | Accessory (live: pid 1255) | Invisible to poll. |
| **pengwm-menubar** | spawned by PengWM; already `excluded_pids` (menubar pid filter in `event_loop.rs:86-93`). | Accessory (live) | Already handled; cite `CONTEXT.md` → *"Shared Daemon↔Menubar Contract"*. |
| Stage Manager, window cycling | Window Manager daemon `com.apple.WindowManager` | Accessory (live: pid 916) | Owns visual companion windows; never in the app-tree poll. |

Takeaway: **the truly omnipresent Apple overlays (Spotlight, Control Centre, Notification
Centre, HUDs, Quick Look) are not Regular apps, so PengWM effectively never sees them**.
The only system apps that *do* fall through the AX window tree in a Regular-policy way —
and are in users' faces daily — are **System Settings**, **Calculator**,
**About This Mac** and similar "Settings-like" utility windows, whose surfaced windows are
level-0 standard windows. AeroSpace solves those with its fullscreen-button heuristic plus
hardcoded bundle ids. About This Mac's owning process is **unverified** here (it opens
without a Dock app appearing; likely a system-process presentation — needs a probe).

---

## 5. Recommended default untracked list for PengWM

Tiers, from structural (least maintenance) to per-app (most maintenance):

### Tier A — detect structurally (AX subrole + CG window level)

1. **Keep** the existing subrole gate: `AXDialog`, `AXSystemDialog`, `AXFloatingWindow` →
   popup (today), and `AXSheet` → drop [unchanged, `adapter.rs:52-64`].
2. **Add: CG window level ≠ 0 ⇒ untracked overlay** (never tiled, never hidden, never touched) for
   any window whose AX-extracted id maps, in `CGWindowListCopyWindowInfo`, to a layer other
   than `kCGNormalWindowLevel` (0):
   - Mechanism per §2 — matches yabai ("level_is_standard" gate in
     [window_manager.c:272-280](https://github.com/koekeishiya/yabai/blob/master/src/window_manager.c#L272:L280))
     and AeroSpace ([windowLevelCache.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/windowLevelCache.swift)).
   - Also consider the canJoinAllSpaces auxiliary signal: a window present in ≥ 2 spaces is
     yabai's separate "sticky" gate
     ([window_is_sticky (SLSCopySpacesForWindows)](https://github.com/koekeishiya/yabai/blob/master/src/window.c#L843:L856));
     for PengWM it is sufficient that level ≠ 0 covers the practical overlays.
3. **`AXUnknown` subrole windows** (Chromium cmd-F bars, menu tooltips, Xcode build
   bubbles) — today these are `Unknown` → dropped → left wherever macOS puts them, which
   is already the "float, don't manage" outcome. Keep. (AeroSpace comment enumerates the
   same set; see its file comments.)

Why structural-detection first: it survives app updates, needs no bundle-id table, and
mirrors macOS's own signaling (a window *declaring itself* an overlay is untracked).

### Tier B — apps floated (untracked) by default, by bundle id

These are Regular-policy apps that present level-0 standard windows but are utility
surfaces; PengWM cannot see their overlay-ness structurally:

| Bundle id | Rationale |
|---|---|
| `com.apple.systempreferences` (**System Settings**) | Explicitly requested by the user. Trade-off noted: **yabai ships no default rule and *does* tile Settings** under its shipped gates (see §4a), as would Amethyst; AeroSpace floats it only via the fullscreen-button heuristic (mistaking it for a dialog) or via its own doc example. PengWM's list is explicit, unlike AeroSpace's incidental heuristic. |
| `com.apple.calculator` (Calculator) | AeroSpace's heuristic explicitly names Calculator (a deliberately small-purpose window); floating while keeping the calculator reachable matches its single-window usage pattern. |
| `com.apple.PhotoBooth` | AeroSpace special-cases it at source level. |
| Optional, to be confirmed by probe: `com.apple.clock` (Clock/World Clock app), `com.apple.weather` | Charging/timer HUD-style app windows. **unverified** in a live probe here. In default-pengwm terms they are borderline; leave out until confirmed. |

Other defaults to include if/when relevant:

- About This Mac panel (`com.apple.systempreferences` too, same process — covered by the
  Settings entry).
- Third-party overlay apps *not* covered here (they're user-specific): use
  `restricted_apps` today (existing config, `docs/configuration.md`). If the new
  untracked config key lands, deprecate/react appropriately so users don't tune two keys.

### What "untracked" means in PengWM terms

Not tiled (never joins a tree), not counted against `max_tiles`, **not closed**, and
**not hidden** on workspace switch: it renders on top of whatever workspace is active on
its monitor until the user closes it — structurally like the existing popup path
([*Popup*, CONTEXT.md](../../CONTEXT.md)) but **cross-workspace**: it must survive
workspace switches and, on each switch, keep stacking above the newly visible tree. That
cross-workspace membership is the one behavior class today's popup path explicitly does
*not* have ("Placed at creation and re-placed on wake; afterwards the user may drag it …
Hides with its workspace through the normal parking path" — CONTEXT.md → *Popup*),
whereas yabai achieves cross-space presence without managing the window at all — which is
the recommended shape here: **untracked windows are not merely "popups on every
workspace"; they are windows PengWM doesn't touch** (no writes, no raise, no hide), the
same treatment as today's `Sheet`/`Unknown` drop, minus the visual-parking trick.

Options thereby reduce to: (i) new `Untracked` WindowClass routed to a never-parked
overlay set (no hide writes on workspace switch; still listed in the menubar UI), or
(ii) simply reclassify to the dropped path (never cached), leaving macOS to own the
window. Option (ii) is the yabai-equivalent and requires zero state — but then features
like counting unmatched windows toward `max_tiles` or surfacing them in the menubar's
popup state become impossible.

---

## 6. Open questions / decision points for the user

1. **Not-tiled vs not-touched.** Should untracked (Tier-B) windows keep working like
   popups (centered, drag, raise-on-switch, persisted in bar state) but across all
   workspaces — or simply be left alone like `Unknown`/`Sheet` drops (PengWM never writes
   to them)? yabai/AeroSpace effectively pick "left alone"; PengWM's popup machinery would
   need a genuinely new membership class for cross-workspace popups.
2. **Is the exact structure (windows vs apps) the right seam?** Tier A is a
   `WindowClass::Overlay` + CG-level query; Tier B is a config list. Should they be:
   hard-coded defaults for Tier A + config-driven `[windows] float_apps = [...]` for
   Tier B, in the style of AeroSpace's `layout floating` rules? (config shape could mirror `restricted_apps`, as noted in
   `docs/configuration.md:14` — same matching style).
3. **Does System Settings belong on Tier B?** Under yabai it tiles (no shipped rule), and
   AeroSpace only defaults-floats it incidentally, via its fixed fullscreen-button
   heuristic. One counter-argument for keeping
   Settings *tiled*: users may want to alt-tab-side to it in some sessions; the cross-
   workspace-on-top semantic (Tier-popup-like) for Settings means it can cover windows in
   uncontrolled way. Worth asking the user whether Settings should be
   untracked **on top** or merely **untiled** (draggable, normal stacking).
4. **Calculator / Photo Booth / Clock: in or out of Tier B defaults?** All are defensible
   either way; evidence depends on user preference, not on a hard primary source.
5. **Spotlight, Control Centre, Notification Centre, volume/brightness HUDs, camera
   pill, Quick Look, password prompts**: today none of them reach PengWM at all (probe:
   accessory/prohibited policies, or service-owned windows). Ship **no rule** for these
   now; add a tiny probe harness (e.g. a one-off `debug-windows` style CLI that dumps
   `CGWindowListCopyWindowInfo` + AX subrole for whatever's on screen) before ever adding
   rules that would depend on them. Otherwise no structural change is needed for
   PengWM's default behavior for these surfaces.
6. **Level ≠ 0 ⇒ untracked: any regressions?** Firefox PiP and Chromium popups are
   already covered by AXFloatingWindow today; but some apps create level-0 non-standard
   subroles for their inspectors (OK under current drop rule) and others (Slack's "calls",
   TeamsPiP) float at level 3 while surfacing `AXStandardWindow` subrole (yabai-level). A
   level-3 `AXStandardWindow` today would be tiled by PengWM (subrole wins); under the new
   rule it becomes untracked — bigger trust in the level. Test on the user's Chrome/Slack
   before default-on. Also confirm how a window-level cache behaves across app lifecycle
   events (AeroSpace's cache is built from one on-screen snapshot and must be invalidated
   on config change/reload; PengWM's element cache already holds per-window state so
   adding the level there fits).

### Follow-ups (probing, not implementation)

- [ ] Live-probe Spotlight on open; determine owning pid/bundle and its window layer.
- [ ] Live-probe one HUD (volume/brightness), the camera green-pill, and a Keychain unlock
      prompt: owner process and layer.
- [ ] Live-probe System Settings' main window: subrole + CG layer — confirm level 0 /
      `AXStandardWindow` on current macOS.

---

## Appendix: evidence inventory

| # | Claim | Primary source |
|---|---|---|
| 1 | Window level table numeric values (0/3/8/24/25/101/1000) | Apple `CGWindowLevel.h:67-79` (local SDK header, path via `xcrun --show-sdk-path`); constants named on [developer.apple.com NSWindow.Level](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct) |
| 2 | "Levels take precedence over within-level stacking" | [Apple NSWindow.Level](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct) |
| 3 | `canJoinAllSpaces` / `moveToActiveSpace` / `transient` semantics | [Apple NSWindow.CollectionBehavior](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct) |
| 4 | Level ≠ normal ⟹ default float-in-spaces behavior | Apple `NSWindow.h` CollectionBehavior comment block (local SDK, lines ~104-113): "Default behavior if `windowLevel != NSNormalWindowLevel`" |
| 5 | Subrole constant values (`AXStandardWindow`, `AXDialog`, `AXSystemDialog`, `AXFloatingWindow`) | Apple `AXRoleConstants.h:411-417` (HIServices framework headers, local SDK) |
| 6 | `kCGWindowLayer`/`kCGWindowNumber`/`kCGWindowOwnerPID`/`kCGWindowName` keys and their value types | Apple `CGWindow.h:44-127` (local SDK); pointer at [CGWindowInfo keys doc](https://developer.apple.com/documentation/coregraphics/cgwindowinfo) |
| 7 | PengWM classification, popup routing, `restricted_apps` default `[]` | `pengwm-daemon/src/adapter.rs`, `pengwm-daemon/src/state/lifecycle.rs`, `pengwm-daemon/src/config/mod.rs`, `docs/configuration.md` |
| 8 | yabai only manages standard-subrole, level-0, movable, non-sticky windows | [yabai window_manager.c:272-280](https://github.com/koekeishiya/yabai/blob/master/src/window_manager.c#L272:L280), [window.c](https://github.com/koekeishiya/yabai/blob/master/src/window.c), [yabai.c:145-147](https://github.com/koekeishiya/yabai/blob/master/src/yabai.c#L145:L147), [misc/macros.h:44](https://github.com/koekeishiya/yabai/blob/master/src/misc/macros.h#L44) |
| 9 | Amethyst manages only movable `AXStandardWindow`; ships empty float list with blacklist-default-true | [Amethyst Window.swift](https://github.com/ianyh/Amethyst/blob/development/Amethyst/Model/Window.swift), [UserConfiguration.swift](https://github.com/ianyh/Amethyst/blob/development/Amethyst/Preferences/UserConfiguration.swift), [default.amethyst](https://github.com/ianyh/Amethyst/blob/development/Amethyst/default.amethyst) |
| 10 | AeroSpace: dialog heuristic floats dialogs by default incl. System Settings/Calculator (no-fullscreen-button heuristic); level query via CGWindowListCopyWindowInfo; Settings example in official docs | [AxUiElementWindowType.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/model/AxUiElementWindowType.swift), [windowLevelCache.swift](https://github.com/nikitabobko/AeroSpace/blob/main/Sources/AppBundle/windowLevelCache.swift), [guide.adoc:540-551, 788-800](https://github.com/nikitabobko/AeroSpace/blob/main/docs/guide.adoc) |
| 11 | Apple bundle-id table (System Settings, Calculator, etc.) | [AeroSpace goodies](https://github.com/nikitabobko/AeroSpace/blob/main/docs/goodies.adoc#popular-apps-ids) (a curated but complete Apple first-party list) |
| 12 | Hammerspoon default allowed roles `{AXStandardWindow, AXDialog, AXSystemDialog}` | [Hammerspoon windowfilter.lua](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/window/window_filter.lua) (community convention marker) |
| 13 | Live activation-policy facts (Control Centre/Notification Centre Accessory; QuickLook/openSave panels Prohibited; widgets, wallpaper, WindowManager processes not Regular) | Live probe executed during this research on this machine (`NSWorkspace.runningApplications` + `CGWindowListCopyWindowInfo`), recorded 2026-10-10; script kept in the session's temp dir, not in the repo |
