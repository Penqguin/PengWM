/*
 * config.h - Configuration management header
 *
 * Handles keybinds, window rules, and other configuration options.
 */

#ifndef CONFIG_H
#define CONFIG_H

#include <stdbool.h>
#include <stdio.h>

/* Maximum lengths for configuration strings */
#define CONFIG_MAX_PATH 256
#define CONFIG_MAX_KEYBIND 64
#define CONFIG_MAX_APP_NAME 128
#define CONFIG_MAX_KEYBINDS 32
#define CONFIG_MAX_RULES 16

/* Key modifiers */
typedef enum {
    MOD_NONE = 0,
    MOD_SHIFT = (1 << 0),
    MOD_CTRL = (1 << 1),
    MOD_ALT = (1 << 2),
    MOD_CMD = (1 << 3)
} KeyModifier;

/* Keybind actions */
typedef enum {
    ACTION_FOCUS_LEFT = 0,
    ACTION_FOCUS_RIGHT,
    ACTION_FOCUS_UP,
    ACTION_FOCUS_DOWN,
    ACTION_TILE,
    ACTION_FLOAT_TOGGLE,
    ACTION_FULLSCREEN_TOGGLE,
    ACTION_CLOSE_WINDOW,
    ACTION_SWAP_LEFT,
    ACTION_SWAP_RIGHT,
    ACTION_SWAP_UP,
    ACTION_SWAP_DOWN,
    ACTION_INCREASE_SIZE,
    ACTION_DECREASE_SIZE,
    ACTION_UNKNOWN
} KeyAction;

/* Window management rules */
typedef enum {
    RULE_TILE = 0,
    RULE_FLOAT,
    RULE_IGNORE
} WindowRule;

/* Keybind structure */
typedef struct {
    char key_combo[CONFIG_MAX_KEYBIND];
    KeyModifier modifiers;
    int keycode;
    KeyAction action;
    bool enabled;
} Keybind;

/* Window rule structure */
typedef struct {
    char app_name[CONFIG_MAX_APP_NAME];
    WindowRule rule;
    bool enabled;
} AppRule;

/* Configuration structure */
typedef struct {
    /* Keybinds */
    Keybind keybinds[CONFIG_MAX_KEYBINDS];
    int keybind_count;
    
    /* Window rules */
    AppRule app_rules[CONFIG_MAX_RULES];
    int rule_count;
    
    /* General settings */
    bool auto_tile;
    bool focus_follows_mouse;
    int gap_size;
    float split_ratio;
    
    /* Paths */
    char config_path[CONFIG_MAX_PATH];
} Config;

/* Global configuration access */
extern Config g_config;

/* Configuration management functions */
bool config_init(void);
bool config_load(const char *config_path);
bool config_reload(void);
void config_cleanup(void);
void config_load_defaults(void);

/* Configuration file operations */
bool config_write_default(const char *filename);

/* Configuration access functions */
const char* config_get_path(void);
void config_print(void);

/* Keybind functions */
bool config_add_keybind(const char *key_combo, KeyAction action);
Keybind* config_find_keybind(int keycode, KeyModifier modifiers);
KeyAction config_parse_action(const char *action_str);
const char* config_action_to_string(KeyAction action);

/* Window rule functions */
bool config_add_app_rule(const char *app_name, WindowRule rule);
WindowRule config_get_window_rule(const char *app_name);
const char* config_rule_to_string(WindowRule rule);

/* Key parsing functions */
int config_parse_keycode(const char *key_str);
KeyModifier config_parse_modifiers(const char *mod_str);

#endif
