/*
 * window_manager.h - Window manager header (IMPROVED)
 */

#ifndef WINDOW_MANAGER_H
#define WINDOW_MANAGER_H

#include "bsp.h"
#include "window_control.h"
#include <stdbool.h>

/* Window manager initialization and cleanup */
bool wm_init(void);
void wm_cleanup(void);

/* Window listing and tiling */
void wm_list_windows(void);
void wm_tile(void);

/* Window focus and navigation */
void wm_focus(const char *direction);

/* Window management */
void wm_add_window(int pid);
void wm_close_window(int pid);

/* Internal helper functions */
void wm_organize_existing_windows(void);
BSPWorkspace* wm_find_workspace_for_window(ManagedWindow *window);
void wm_apply_layout_to_leaf(BSPNode *leaf);
ManagedWindow* wm_find_managed_window(int window_id);
BSPNode* wm_find_bsp_node_for_window(int window_id);
bool wm_is_window_managed(AXUIElementRef window_ref);
void wm_remove_from_bsp_tree(int window_id);

#endif
