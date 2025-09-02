/*
 * bsp.c - Binary Space Partitioning implementation (FIXED VERSION)
 *
 */

#include "bsp.h"
#include "window_control.h"
#include <stdio.h>
#include <stdlib.h>

/*
 * init_workspaces - Initialize BSP workspaces for all displays
 *
 * out_count: Output parameter for number of workspaces created
 *
 * Returns: Array of BSPWorkspace structures
 */
BSPWorkspace* init_workspaces(size_t *out_count)
{
    uint32_t display_count = 0;
    CGDirectDisplayID *displays;
    BSPWorkspace *workspaces;
    uint32_t i;

    /* Get number of active displays */
    CGGetActiveDisplayList(0, NULL, &display_count);
    if (display_count == 0) {
        *out_count = 0;
        return NULL;
    }

    /* Allocate display array */
    displays = malloc(display_count * sizeof(CGDirectDisplayID));
    if (!displays) {
        *out_count = 0;
        return NULL;
    }

    /* Get display list */
    CGGetActiveDisplayList(display_count, displays, &display_count);

    /* Allocate workspace array */
    workspaces = calloc(display_count, sizeof(BSPWorkspace));
    if (!workspaces) {
        free(displays);
        *out_count = 0;
        return NULL;
    }

    /* Initialize each workspace */
    for (i = 0; i < display_count; i++) {
        workspaces[i].display_id = displays[i];
        workspaces[i].bounds = CGDisplayBounds(displays[i]);
        workspaces[i].root = bsp_create_root(workspaces[i].bounds);
        
        if (!workspaces[i].root) {
            /* Cleanup on failure */
            for (uint32_t j = 0; j < i; j++) {
                bsp_free(workspaces[j].root);
            }
            free(workspaces);
            free(displays);
            *out_count = 0;
            return NULL;
        }
    }

    free(displays);
    *out_count = display_count;
    return workspaces;
}

/*
 * bsp_create_root - Create root BSP node for a screen
 *
 * screen_rect: Rectangle representing the screen bounds
 *
 * Returns: Root BSP node
 */
BSPNode* bsp_create_root(CGRect screen_rect)
{
    BSPNode *root = calloc(1, sizeof(BSPNode));
    
    if (!root) {
        return NULL;
    }

    root->is_leaf = true;
    root->rect = screen_rect;
    root->window_id = -1;  /* No windows yet */
    root->split_vertical = false;
    root->first = NULL;
    root->second = NULL;
    root->parent = NULL;

    return root;
}

/*
 * bsp_split_leaf - Split a leaf node to accommodate a new window
 *
 * leaf: Leaf node to split
 * new_window_id: ID of the new window to place in split
 */
void bsp_split_leaf(BSPNode *leaf, int new_window_id)
{
    bool split_vertical;
    CGRect r1, r2;
    BSPNode *first, *second;

    if (!leaf || !leaf->is_leaf) {
        return;
    }
    
    /* Determine split orientation based on aspect ratio */
    split_vertical = (leaf->rect.size.width > leaf->rect.size.height);
    
    leaf->is_leaf = false;
    leaf->split_vertical = split_vertical;
    
    /* Split rectangle into two halves */
    r1 = leaf->rect;
    r2 = leaf->rect;
    
    if (split_vertical) {
        /* Vertical split - divide width */
        r1.size.width /= 2;
        r2.origin.x += r1.size.width;
        r2.size.width /= 2;
    } else {
        /* Horizontal split - divide height (FIXED) */
        r1.size.height /= 2;
        r2.origin.y += r1.size.height;
        r2.size.height /= 2;
    }
    
    /* Create first child (keeps old window) */
    first = calloc(1, sizeof(BSPNode));
    if (!first) {
        return;
    }
    
    first->is_leaf = true;
    first->rect = r1;
    first->window_id = leaf->window_id;
    first->parent = leaf;
    
    /* Create second child (gets new window) */
    second = calloc(1, sizeof(BSPNode));
    if (!second) {
        free(first);
        return;
    }
    
    second->is_leaf = true;
    second->rect = r2;
    second->window_id = new_window_id;
    second->parent = leaf;
    
    leaf->first = first;
    leaf->second = second;
    leaf->window_id = -1;  /* Parent no longer holds window */
}

/*
 * bsp_insert - Insert a window into the BSP tree
 *
 * node: Node to insert into
 * window_id: ID of window to insert
 */
void bsp_insert(BSPNode *node, int window_id)
{
    if (!node) {
        return;
    }
    
    if (node->is_leaf) {
        switch (node->window_id) {
            case -1:
                /* Empty leaf - assign window */
                node->window_id = window_id;
                break;
            default:
                /* Already occupied - split */
                bsp_split_leaf(node, window_id);
                break;
        }
    } else {
        /* Insert into first child by default */
        /* TODO: Improve insertion policy */
        bsp_insert(node->first, window_id);
    }
}

/*
 * bsp_remove_window - Remove a window from the BSP tree
 *
 * node: Node to search for window
 * window_id: ID of window to remove
 *
 * Returns: true if window was found and removed
 */
bool bsp_remove_window(BSPNode *node, int window_id)
{
    if (!node) {
        return false;
    }
    
    if (node->is_leaf) {
        if (node->window_id == window_id) {
            node->window_id = -1;  /* Mark as empty */
            return true;
        }
        return false;
    }
    
    /* Check children */
    if (bsp_remove_window(node->first, window_id) ||
        bsp_remove_window(node->second, window_id)) {
        /* After removal, try to collapse empty branches */
        bsp_collapse_empty_branches(node);
        return true;
    }
    
    return false;
}

/*
 * bsp_collapse_empty_branches - Collapse branches with only empty leaves
 *
 * node: Node to check for collapsing
 */
void bsp_collapse_empty_branches(BSPNode *node)
{
    bool first_empty, second_empty;
    BSPNode *replacement;
    
    if (!node || node->is_leaf) {
        return;
    }
    
    first_empty = (node->first->is_leaf && node->first->window_id == -1);
    second_empty = (node->second->is_leaf && node->second->window_id == -1);
    
    if (first_empty && !second_empty) {
        /* Replace this node with second child */
        replacement = node->second;
        node->is_leaf = replacement->is_leaf;
        node->window_id = replacement->window_id;
        node->split_vertical = replacement->split_vertical;
        
        free(node->first);
        node->first = replacement->first;
        node->second = replacement->second;
        
        if (node->first) node->first->parent = node;
        if (node->second) node->second->parent = node;
        
        free(replacement);
    } else if (second_empty && !first_empty) {
        /* Replace this node with first child */
        replacement = node->first;
        node->is_leaf = replacement->is_leaf;
        node->window_id = replacement->window_id;
        node->split_vertical = replacement->split_vertical;
        
        free(node->second);
        node->first = replacement->first;
        node->second = replacement->second;
        
        if (node->first) node->first->parent = node;
        if (node->second) node->second->parent = node;
        
        free(replacement);
    }
}

/*
 * bsp_find_node_for_window - Find BSP node containing a window
 *
 * node: Root node to search from
 * window_id: ID of window to find
 *
 * Returns: BSP node containing the window, or NULL if not found
 */
BSPNode* bsp_find_node_for_window(BSPNode *node, int window_id)
{
    BSPNode *found;

    if (!node) {
        return NULL;
    }
    
    if (node->is_leaf) {
        return (node->window_id == window_id) ? node : NULL;
    }
    
    found = bsp_find_node_for_window(node->first, window_id);
    if (found) {
        return found;
    }
    
    return bsp_find_node_for_window(node->second, window_id);
}

/*
 * bsp_find_neighbor - Find neighboring node in specified direction
 *
 * node: Starting node
 * direction: Direction to search (left, right, up, down)
 *
 * Returns: Neighboring node, or NULL if none found
 */
BSPNode* bsp_find_neighbor(BSPNode *node, const char *direction)
{
    if (!node || !direction) {
        return NULL;
    }

    switch (direction[0]) {
        case 'l':  /* left */
            return bsp_find_left_neighbor(node);
        case 'r':  /* right */
            return bsp_find_right_neighbor(node);
        case 'u':  /* up */
            return bsp_find_up_neighbor(node);
        case 'd':  /* down */
            return bsp_find_down_neighbor(node);
        default:
            return NULL;
    }
}

/*
 * bsp_find_left_neighbor - Find left neighbor of a node
 */
BSPNode* bsp_find_left_neighbor(BSPNode *node)
{
    BSPNode *current = node;
    
    while (current->parent) {
        BSPNode *parent = current->parent;
        if (parent->split_vertical && parent->second == current) {
            /* We're the right child of a vertical split */
            return bsp_find_rightmost_leaf(parent->first);
        }
        current = parent;
    }
    return NULL;
}

/*
 * bsp_find_right_neighbor - Find right neighbor of a node
 */
BSPNode* bsp_find_right_neighbor(BSPNode *node)
{
    BSPNode *current = node;
    
    while (current->parent) {
        BSPNode *parent = current->parent;
        if (parent->split_vertical && parent->first == current) {
            /* We're the left child of a vertical split */
            return bsp_find_leftmost_leaf(parent->second);
        }
        current = parent;
    }
    return NULL;
}

/*
 * bsp_find_up_neighbor - Find up neighbor of a node
 */
BSPNode* bsp_find_up_neighbor(BSPNode *node)
{
    BSPNode *current = node;
    
    while (current->parent) {
        BSPNode *parent = current->parent;
        if (!parent->split_vertical && parent->second == current) {
            /* We're the bottom child of a horizontal split */
            return bsp_find_bottommost_leaf(parent->first);
        }
        current = parent;
    }
    return NULL;
}

/*
 * bsp_find_down_neighbor - Find down neighbor of a node
 */
BSPNode* bsp_find_down_neighbor(BSPNode *node)
{
    BSPNode *current = node;
    
    while (current->parent) {
        BSPNode *parent = current->parent;
        if (!parent->split_vertical && parent->first == current) {
            /* We're the top child of a horizontal split */
            return bsp_find_topmost_leaf(parent->second);
        }
        current = parent;
    }
    return NULL;
}

/*
 * bsp_find_leftmost_leaf - Find leftmost leaf in subtree
 */
BSPNode* bsp_find_leftmost_leaf(BSPNode *node)
{
    while (node && !node->is_leaf) {
        node = node->first;
    }
    return node;
}

/*
 * bsp_find_rightmost_leaf - Find rightmost leaf in subtree
 */
BSPNode* bsp_find_rightmost_leaf(BSPNode *node)
{
    while (node && !node->is_leaf) {
        node = node->second;
    }
    return node;
}

/*
 * bsp_find_topmost_leaf - Find topmost leaf in subtree
 */
BSPNode* bsp_find_topmost_leaf(BSPNode *node)
{
    while (node && !node->is_leaf) {
        node = node->first;   /* For horizontal splits, first is top */
    }
    return node;
}

/*
 * bsp_find_bottommost_leaf - Find bottommost leaf in subtree
 */
BSPNode* bsp_find_bottommost_leaf(BSPNode *node)
{
    while (node && !node->is_leaf) {
        node = node->second;  /* For horizontal splits, second is bottom */
    }
    return node;
}

/*
 * bsp_free - Recursively free a BSP tree
 *
 * node: Root of the tree or subtree to free
 */
void bsp_free(BSPNode *node) 
{
    if (node == NULL) {
        return;
    }

    /* Recursively free children first (post-order traversal) */
    bsp_free(node->first);
    bsp_free(node->second);

    /* Free the node itself */
    free(node);
}

/*
 * bsp_traverse - Traverse the BSP tree and apply a callback to leaves
 *
 * node: Node to start traversal from
 * callback: Function to call for each leaf node with a window
 */
void bsp_traverse(BSPNode *node, void (*callback)(BSPNode*)) 
{
    if (node == NULL || callback == NULL) {
        return;
    }

    if (node->is_leaf) {
        /* If it's a leaf node, call the callback function.
         * The callback is responsible for checking if there is a window.
         */
        callback(node);
    } else {
        /* If it's an internal node, traverse its children */
        bsp_traverse(node->first, callback);
        bsp_traverse(node->second, callback);
    }
}
