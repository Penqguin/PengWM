/*
 * bsp.h - Binary Space Partitioning header
 */

#ifndef BSP_H
#define BSP_H

#include <stdbool.h>
#include <CoreGraphics/CoreGraphics.h>

typedef struct BSPNode {
    bool is_leaf;
    int window_id;
    CGRect rect;
    
    bool split_vertical;
    
    struct BSPNode *first;
    struct BSPNode *second;
    struct BSPNode *parent;
} BSPNode;

typedef struct {
    CGDirectDisplayID display_id;
    CGRect bounds;
    BSPNode *root;
} BSPWorkspace;

/* Workspace management */
BSPWorkspace* init_workspaces(size_t *out_count);

/* BSP tree operations */
BSPNode* bsp_create_root(CGRect screen_rect);
void bsp_insert(BSPNode *node, int window_id);
bool bsp_remove_window(BSPNode *node, int window_id);
void bsp_traverse(BSPNode *node, void (*callback)(BSPNode*));
void bsp_free(BSPNode *node);
void bsp_split_leaf(BSPNode *leaf, int new_window_id);

/* Node search and navigation */
BSPNode* bsp_find_node_for_window(BSPNode *node, int window_id);
BSPNode* bsp_find_neighbor(BSPNode *node, const char *direction);

/* Direction-specific neighbor finding */
BSPNode* bsp_find_left_neighbor(BSPNode *node);
BSPNode* bsp_find_right_neighbor(BSPNode *node);
BSPNode* bsp_find_up_neighbor(BSPNode *node);
BSPNode* bsp_find_down_neighbor(BSPNode *node);

/* Tree traversal helpers */
BSPNode* bsp_find_leftmost_leaf(BSPNode *node);
BSPNode* bsp_find_rightmost_leaf(BSPNode *node);
BSPNode* bsp_find_topmost_leaf(BSPNode *node);
BSPNode* bsp_find_bottommost_leaf(BSPNode *node);

/* Tree maintenance */
void bsp_collapse_empty_branches(BSPNode *node);

#endif 
