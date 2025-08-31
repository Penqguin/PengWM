/*
 * window_manager.c - Window manager implementation 
 */

#include "window_manager.h"
#include "window_control.h"
#include "bsp.h"
#include "config.h"
#include <stdio.h>
#include <string.h>

/* Global workspace management */
static BSPWorkspace *g_workspaces = NULL;
static size_t g_workspace_count = 0;
static bool g_wm_initialized = false;

/*
 * wm_init - Initialize the window manager
 *
 * Returns: true on success, false on failure
 */
bool
wm_init(void)
{
    size_t managed_count;

    if (g_wm_initialized) {
        return true;
    }

    /* Initialize workspaces for all displays */
    g_workspaces = init_workspaces(&g_workspace_count);
    if (!g_workspaces || g_workspace_count == 0) {
        printf("Failed to initialize workspaces\n");
        return false;
    }

    /* Discover and manage existing windows */
    g_managed_windows = get_manageable_windows(&managed_count);
    g_managed_count = managed_count;

    /* Insert windows into BSP trees */
    wm_organize_existing_windows();

    g_wm_initialized = true;
    printf("Window manager initialized with %zu workspaces and %zu windows\n",
           g_workspace_count, g_managed_count);

    return true;
}

/*
 * wm_organize_existing_windows - Organize existing windows into BSP trees
 */
void
wm_organize_existing_windows(void)
{
    size_t i;

    for (i = 0; i < g_managed_count; i++) {
        BSPWorkspace *workspace = wm_find_workspace_for_window(&g_managed_windows[i]);
        if (workspace) {
            bsp_insert(workspace->root, g_managed_windows[i].window_id);
        }
    }
}

/*
 * wm_find_workspace_for_window - Find appropriate workspace for a window
 *
 * window: Window to find workspace for
 *
 * Returns: Workspace containing the window, or primary workspace if none found
 */
BSPWorkspace*
wm_find_workspace_for_window(ManagedWindow *window)
{
    size_t i;
    CGPoint center;

    center = CGPointMake(
        window->frame.origin.x + window->frame.size.width / 2,
        window->frame.origin.y + window->frame.size.height / 2
    );

    for (i = 0; i < g_workspace_count; i++) {
        if (CGRectContainsPoint(g_workspaces[i].bounds, center)) {
            return &g_workspaces[i];
        }
    }

    /* Fallback to primary workspace */
    return g_workspace_count > 0 ? &g_workspaces[0] : NULL;
}

/*
 * wm_list_windows - List all managed windows
 */
void
wm_list_windows(void)
{
    size_t i;

    if (!g_wm_initialized) {
        printf("Window manager not initialized\n");
        return;
    }

    printf("Managed Windows (%zu total):\n", g_managed_count);
    printf("%-8s %-20s %-8s %s\n", "ID", "Application", "PID", "Frame");
    printf("-------- -------------------- -------- --------------------\n");

    for (i = 0; i < g_managed_count; i++) {
        ManagedWindow *win = &g_managed_windows[i];
        printf("%-8d %-20s %-8d (%.0f,%.0f %.0fx%.0f)\n",
               win->window_id,
               win->app_name,
               win->pid,
               win->frame.origin.x,
               win->frame.origin.y,
               win->frame.size.width,
               win->frame.size.height);
    }
}

/*
 * wm_tile - Apply BSP tiling to all workspaces
 */
void
wm_tile(void)
{
    size_t ws_idx;

    if (!g_wm_initialized && !wm_init()) {
        printf("Failed to initialize window manager\n");
        return;
    }

    printf("Applying BSP tiling to %zu workspace(s)...\n", g_workspace_count);

    for (ws_idx = 0; ws_idx < g_workspace_count; ws_idx++) {
        BSPWorkspace *workspace = &g_workspaces[ws_idx];
        printf("Tiling workspace %zu (display %u)...\n",
               ws_idx, workspace->display_id);
        
        /* Apply BSP layout to windows in this workspace */
        bsp_traverse(workspace->root, wm_apply_layout_to_leaf);
    }

    printf("Tiling complete\n");
}

/*
 * wm_apply_layout_to_leaf - Apply BSP layout to a leaf node
 *
 * leaf: BSP leaf node containing window layout information
 */
void
wm_apply_layout_to_leaf(BSPNode *leaf)
{
    ManagedWindow *window;

    if (!leaf || !leaf->is_leaf || leaf->window_id == -1) {
        return;
    }

    /* Find the managed window with this ID */
    window = wm_find_managed_window(leaf->window_id);
    if (!window) {
        printf("Warning: No managed window found for ID %d\n", leaf->window_id);
        return;
    }

    /* Apply the BSP-computed rectangle to the actual window */
    if (!set_window_frame(window->window_ref, leaf->rect)) {
        printf("Failed to set frame for window %d (%s)\n",
               leaf->window_id, window->app_name);
    } else {
        /* Update our cached frame */
        window->frame = leaf->rect;
    }
}

/*
 * wm_find_managed_window - Find managed window by ID
 *
 * window_id: ID of window to find
 *
 * Returns: Managed window structure, or NULL if not found
 */
ManagedWindow*
wm_find_managed_window(int window_id)
{
    size_t i;

    for (i = 0; i < g_managed_count; i++) {
        if (g_managed_windows[i].window_id == window_id) {
            return &g_managed_windows[i];
        }
    }
    return NULL;
}

/*
 * wm_focus - Focus window in specified direction
 *
 * direction: Direction to focus (left, right, up, down)
 */
void
wm_focus(const char *direction)
{
    ManagedWindow *focused_window;
    BSPNode *focused_node, *target_node;
    ManagedWindow *target_window;

    if (!g_wm_initialized) {
        printf("Window manager not initialized\n");
        return;
    }

    if (!direction) {
        printf("No direction specified\n");
        return;
    }

    /* Get currently focused window */
    focused_window = get_currently_focused_window();
    if (!focused_window) {
        printf("No focused window found\n");
        return;
    }

    /* Find its BSP node */
    focused_node = wm_find_bsp_node_for_window(focused_window->window_id);
    if (!focused_node) {
        printf("Focused window not found in BSP tree\n");
        return;
    }

    /* Find neighbor based on direction */
    target_node = bsp_find_neighbor(focused_node, direction);
    if (!target_node || target_node->window_id == -1) {
        printf("No window found in direction '%s'\n", direction);
        return;
    }

    target_window = wm_find_managed_window(target_node->window_id);
    if (target_window) {
        focus_window(target_window->window_ref);
        printf("Focused window %d (%s)\n",
               target_window->window_id, target_window->app_name);
    }
}

/*
 * wm_find_bsp_node_for_window - Find BSP node containing a window
 *
 * window_id: ID of window to find
 *
 * Returns: BSP node containing the window, or NULL if not found
 */
BSPNode*
wm_find_bsp_node_for_window(int window_id)
{
    size_t ws_idx;
    BSPNode *node;

    for (ws_idx = 0; ws_idx < g_workspace_count; ws_idx++) {
        node = bsp_find_node_for_window(g_workspaces[ws_idx].root, window_id);
        if (node) {
            return node;
        }
    }
    return NULL;
}

/*
 * wm_add_window - Add a window to management by PID
 *
 * pid: Process ID of application whose windows to add
 */
void
wm_add_window(int pid)
{
    AXUIElementRef app_ref;
    CFArrayRef window_list = NULL;
    AXError err;
    CFIndex count, i;
    int windows_added = 0;

    if (!g_wm_initialized && !wm_init()) {
        printf("Failed to initialize window manager\n");
        return;
    }

    /* Create app reference from PID */
    app_ref = AXUIElementCreateApplication(pid);
    if (!app_ref) {
        printf("Failed to create app reference for PID %d\n", pid);
        return;
    }

    /* Get all windows for this application */
    err = AXUIElementCopyAttributeValue(app_ref, kAXWindowsAttribute,
                                       (CFTypeRef*)&window_list);
    if (err != kAXErrorSuccess || !window_list) {
        printf("Failed to get windows for PID %d\n", pid);
        CFRelease(app_ref);
        return;
    }

    count = CFArrayGetCount(window_list);
    for (i = 0; i < count; i++) {
        AXUIElementRef window_ref = CFArrayGetValueAtIndex(window_list, i);
        CGPoint pos;
        CGSize size;

        if (get_window_frame(window_ref, &pos, &size)) {
            CGRect frame = CGRectMake(pos.x, pos.y, size.width, size.height);
            BSPWorkspace *workspace;
            int window_id;

            /* Skip if already managed */
            if (wm_is_window_managed(window_ref)) {
                continue;
            }

            /* Find appropriate workspace */
            for (size_t ws_idx = 0; ws_idx < g_workspace_count; ws_idx++) {
                if (CGRectIntersectsRect(frame, g_workspaces[ws_idx].bounds)) {
                    workspace = &g_workspaces[ws_idx];
                    window_id = generate_window_id();
                    
                    /* Insert into BSP tree */
                    bsp_insert(workspace->root, window_id);
                    
                    /* Add to managed windows array */
                    add_to_managed_windows(window_ref, window_id, pid, frame);
                    
                    windows_added++;
                    break;
                }
            }
        }
    }

    CFRelease(window_list);
    CFRelease(app_ref);

    if (windows_added > 0) {
        printf("Added %d window(s) from PID %d\n", windows_added, pid);
        /* Re-tile after adding */
        wm_tile();
    } else {
        printf("No new windows found for PID %d\n", pid);
    }
}

/*
 * wm_is_window_managed - Check if a window is already managed
 *
 * window_ref: AXUIElement reference to check
 *
 * Returns: true if window is already managed
 */
bool
wm_is_window_managed(AXUIElementRef window_ref)
{
    size_t i;

    for (i = 0; i < g_managed_count; i++) {
        if (CFEqual(g_managed_windows[i].window_ref, window_ref)) {
            return true;
        }
    }
    return false;
}

/*
 * wm_close_window - Remove a window from management by PID
 *
 * pid: Process ID of application whose windows to remove
 */
void
wm_close_window(int pid)
{
    size_t i;
    int windows_removed = 0;

    if (!g_wm_initialized) {
        printf("Window manager not initialized\n");
        return;
    }

    /* Find and remove all windows with matching PID */
    for (i = 0; i < g_managed_count; i++) {
        if (g_managed_windows[i].pid == pid) {
            int window_id = g_managed_windows[i].window_id;
            
            /* Remove from BSP tree */
            wm_remove_from_bsp_tree(window_id);
            
            /* Remove from managed windows */
            remove_from_managed_windows(window_id);
            
            windows_removed++;
            i--; /* Array was shifted, check same index again */
        }
    }

    if (windows_removed > 0) {
        printf("Removed %d window(s) from PID %d\n", windows_removed, pid);
        /* Re-tile remaining windows */
        wm_tile();
    } else {
        printf("No windows found for PID %d\n", pid);
    }
}

/*
 * wm_remove_from_bsp_tree - Remove a window from all BSP trees
 *
 * window_id: ID of window to remove
 */
void
wm_remove_from_bsp_tree(int window_id)
{
    size_t ws_idx;

    for (ws_idx = 0; ws_idx < g_workspace_count; ws_idx++) {
        if (bsp_remove_window(g_workspaces[ws_idx].root, window_id)) {
            break; /* Window found and removed */
        }
    }
}

/*
 * wm_cleanup - Clean up window manager resources
 */
void
wm_cleanup(void)
{
    size_t i;

    if (!g_wm_initialized) {
        return;
    }

    /* Free BSP trees */
    for (i = 0; i < g_workspace_count; i++) {
        bsp_free(g_workspaces[i].root);
    }
    free(g_workspaces);
    g_workspaces = NULL;
    g_workspace_count = 0;

    /* Release managed window references */
    for (i = 0; i < g_managed_count; i++) {
        CFRelease(g_managed_windows[i].window_ref);
    }
    free(g_managed_windows);
    g_managed_windows = NULL;
    g_managed_count = 0;

    g_wm_initialized = false;
    printf("Window manager cleaned up\n");
}
