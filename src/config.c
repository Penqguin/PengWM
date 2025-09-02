/*
 * config.c - Configuration management implementation
 *
 * Handles loading, parsing, and managing configuration options
 * including keybinds and window rules.
 */

#include "config.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>

/* Global configuration */
Config g_config = {0};

/* Forward declarations for static functions */
static bool config_parse_file_handle(FILE *file);
static bool config_parse_line(const char *line, int line_number);
static bool config_parse_keybind_line(const char *value);
static bool config_parse_rule_line(const char *value);
static bool config_parse_setting_line(const char *key, const char *value);
static void config_parse_key_combination(const char *key_combo, int *keycode, KeyModifier *modifiers);

/* Default configuration values */
static const struct {
    const char *key_combo;
    KeyAction action;
} s_default_keybinds[] = {
    {"cmd+alt+h", ACTION_FOCUS_LEFT},
    {"cmd+alt+l", ACTION_FOCUS_RIGHT},
    {"cmd+alt+k", ACTION_FOCUS_UP},
    {"cmd+alt+j", ACTION_FOCUS_DOWN},
    {"cmd+alt+t", ACTION_TILE},
    {"cmd+alt+f", ACTION_FLOAT_TOGGLE},
    {"cmd+alt+return", ACTION_FULLSCREEN_TOGGLE},
    {"cmd+alt+q", ACTION_CLOSE_WINDOW},
    {"cmd+alt+shift+h", ACTION_SWAP_LEFT},
    {"cmd+alt+shift+l", ACTION_SWAP_RIGHT},
    {"cmd+alt+shift+k", ACTION_SWAP_UP},
    {"cmd+alt+shift+j", ACTION_SWAP_DOWN},
    {"cmd+alt+equal", ACTION_INCREASE_SIZE},
    {"cmd+alt+minus", ACTION_DECREASE_SIZE}
};

static const int s_default_keybind_count =
    sizeof(s_default_keybinds) / sizeof(s_default_keybinds[0]);

/*
 * config_get_home_path - Get user home directory path
 *
 * Returns: Home directory path, or NULL on error
 */
static const char* config_get_home_path(void)
{
    const char *home = getenv("HOME");
    return home ? home : "/tmp";
}

/*
 * config_create_directory - Create configuration directory if it doesn't exist
 *
 * dir_path: Directory path to create
 *
 * Returns: true on success or if directory already exists
 */
static bool config_create_directory(const char *dir_path)
{
    struct stat st;
    
    if (stat(dir_path, &st) == 0) {
        return S_ISDIR(st.st_mode);
    }
    
    return mkdir(dir_path, 0755) == 0;
}

/*
 * config_init - Initialize configuration system
 *
 * Returns: true on success, false on failure
 */
bool config_init(void)
{
    const char *home;
    char config_dir[CONFIG_MAX_PATH];
    
    /* Reset configuration to defaults */
    memset(&g_config, 0, sizeof(g_config));
    
    /* Set default values */
    g_config.auto_tile = true;
    g_config.focus_follows_mouse = false;
    g_config.gap_size = 10;
    g_config.split_ratio = 0.5f;
    
    /* Build config directory path */
    home = config_get_home_path();
    snprintf(config_dir, sizeof(config_dir), "%s/.pengwm", home);
    
    /* Create config directory */
    if (!config_create_directory(config_dir)) {
        printf("Warning: Failed to create config directory %s\n", config_dir);
    }
    
    /* Build config file path */
    snprintf(g_config.config_path, sizeof(g_config.config_path),
             "%s/config", config_dir);
    
    /* Load configuration */
    if (!config_load(g_config.config_path)) {
        printf("Using default configuration\n");
        config_load_defaults();
    }
    
    return true;
}

/*
 * config_load_defaults - Load default configuration values
 */
void config_load_defaults(void)
{
    int i;
    
    /* Load default keybinds */
    g_config.keybind_count = 0;
    for (i = 0; i < s_default_keybind_count && i < CONFIG_MAX_KEYBINDS; i++) {
        config_add_keybind(s_default_keybinds[i].key_combo,
                          s_default_keybinds[i].action);
    }
    
    /* No default app rules */
    g_config.rule_count = 0;
}

/*
 * config_load - Load configuration from file
 *
 * config_path: Path to configuration file
 *
 * Returns: true on success, false on failure
 */
bool config_load(const char *config_path)
{
    FILE *file;
    
    if (!config_path) {
        return false;
    }
    
    file = fopen(config_path, "r");
    if (!file) {
        /* Try to create default config file */
        if (config_write_default(config_path)) {
            file = fopen(config_path, "r");
        }
        
        if (!file) {
            return false;
        }
    }
    
    bool success = config_parse_file_handle(file);
    fclose(file);
    
    return success;
}

/*
 * config_parse_file_handle - Parse configuration from open file handle
 *
 * file: Open file handle to parse
 *
 * Returns: true on success, false on failure
 */
static bool config_parse_file_handle(FILE *file)
{
    char line[512];
    int line_number = 0;
    
    /* Reset counters */
    g_config.keybind_count = 0;
    g_config.rule_count = 0;
    
    while (fgets(line, sizeof(line), file)) {
        line_number++;
        
        /* Remove trailing newline */
        line[strcspn(line, "\n")] = '\0';
        
        /* Skip empty lines and comments */
        if (line[0] == '\0' || line[0] == '#') {
            continue;
        }
        
        if (!config_parse_line(line, line_number)) {
            printf("Warning: Invalid config line %d: %s\n", line_number, line);
        }
    }
    
    return true;
}

/*
 * config_parse_line - Parse a single configuration line
 *
 * line: Configuration line to parse
 * line_number: Line number for error reporting
 *
 * Returns: true on success, false on parse error
 */
static bool config_parse_line(const char *line, int line_number)
{
    char key[128], value[128];
    
    (void)line_number;  /* Unused parameter */
    
    if (sscanf(line, "%127s %127[^\n]", key, value) != 2) {
        return false;
    }
    
    /* Parse different configuration types */
    switch (key[0]) {
        case 'k':  /* keybind */
            if (strcmp(key, "keybind") == 0) {
                return config_parse_keybind_line(value);
            }
            break;
        case 'r':  /* rule */
            if (strcmp(key, "rule") == 0) {
                return config_parse_rule_line(value);
            }
            break;
        case 's':  /* settings */
            return config_parse_setting_line(key, value);
        default:
            return config_parse_setting_line(key, value);
    }
    
    return false;
}

/*
 * config_parse_keybind_line - Parse a keybind configuration line
 *
 * value: Keybind specification string
 *
 * Returns: true on success, false on parse error
 */
static bool config_parse_keybind_line(const char *value)
{
    char key_combo[CONFIG_MAX_KEYBIND];
    char action_str[64];
    KeyAction action;
    
    if (sscanf(value, "%63s %63s", key_combo, action_str) != 2) {
        return false;
    }
    
    action = config_parse_action(action_str);
    if (action == ACTION_UNKNOWN) {
        return false;
    }
    
    return config_add_keybind(key_combo, action);
}

/*
 * config_parse_rule_line - Parse a window rule configuration line
 *
 * value: Rule specification string
 *
 * Returns: true on success, false on parse error
 */
static bool config_parse_rule_line(const char *value)
{
    char app_name[CONFIG_MAX_APP_NAME];
    char rule_str[32];
    WindowRule rule;
    
    if (sscanf(value, "%127s %31s", app_name, rule_str) != 2) {
        return false;
    }
    
    switch (rule_str[0]) {
        case 't':
            if (strcmp(rule_str, "tile") == 0) rule = RULE_TILE;
            else return false;
            break;
        case 'f':
            if (strcmp(rule_str, "float") == 0) rule = RULE_FLOAT;
            else return false;
            break;
        case 'i':
            if (strcmp(rule_str, "ignore") == 0) rule = RULE_IGNORE;
            else return false;
            break;
        default:
            return false;
    }
    
    return config_add_app_rule(app_name, rule);
}

/*
 * config_parse_setting_line - Parse a general setting line
 *
 * key: Setting name
 * value: Setting value
 *
 * Returns: true on success, false on parse error
 */
static bool config_parse_setting_line(const char *key, const char *value)
{
    if (strcmp(key, "auto_tile") == 0) {
        g_config.auto_tile = (strcmp(value, "true") == 0);
        return true;
    } else if (strcmp(key, "focus_follows_mouse") == 0) {
        g_config.focus_follows_mouse = (strcmp(value, "true") == 0);
        return true;
    } else if (strcmp(key, "gap_size") == 0) {
        g_config.gap_size = atoi(value);
        return g_config.gap_size >= 0;
    } else if (strcmp(key, "split_ratio") == 0) {
        g_config.split_ratio = atof(value);
        return g_config.split_ratio > 0.0f && g_config.split_ratio < 1.0f;
    }
    
    return false;
}

/*
 * config_add_keybind - Add a keybind to the configuration
 *
 * key_combo: Key combination string (e.g., "cmd+alt+h")
 * action: Action to perform
 *
 * Returns: true on success, false if no space available
 */
bool config_add_keybind(const char *key_combo, KeyAction action)
{
    Keybind *keybind;
    
    if (g_config.keybind_count >= CONFIG_MAX_KEYBINDS) {
        return false;
    }
    
    keybind = &g_config.keybinds[g_config.keybind_count];
    strncpy(keybind->key_combo, key_combo, CONFIG_MAX_KEYBIND - 1);
    keybind->key_combo[CONFIG_MAX_KEYBIND - 1] = '\0';
    keybind->action = action;
    keybind->enabled = true;
    
    /* Parse key combination */
    config_parse_key_combination(key_combo, &keybind->keycode, &keybind->modifiers);
    
    g_config.keybind_count++;
    return true;
}

/*
 * config_parse_key_combination - Parse key combination string
 *
 * key_combo: Key combination string
 * keycode: Output parameter for keycode
 * modifiers: Output parameter for modifiers
 */
static void config_parse_key_combination(const char *key_combo, int *keycode, KeyModifier *modifiers)
{
    char combo_copy[CONFIG_MAX_KEYBIND];
    char *token, *key_part = NULL;
    
    *modifiers = MOD_NONE;
    *keycode = 0;
    
    strncpy(combo_copy, key_combo, sizeof(combo_copy) - 1);
    combo_copy[sizeof(combo_copy) - 1] = '\0';
    
    /* Split by '+' and process each part */
    token = strtok(combo_copy, "+");
    while (token) {
        if (strcmp(token, "cmd") == 0) {
            *modifiers |= MOD_CMD;
        } else if (strcmp(token, "alt") == 0) {
            *modifiers |= MOD_ALT;
        } else if (strcmp(token, "ctrl") == 0) {
            *modifiers |= MOD_CTRL;
        } else if (strcmp(token, "shift") == 0) {
            *modifiers |= MOD_SHIFT;
        } else {
            key_part = token;  /* This should be the actual key */
        }
        token = strtok(NULL, "+");
    }
    
    if (key_part) {
        *keycode = config_parse_keycode(key_part);
    }
}

/*
 * config_parse_keycode - Parse key string to keycode
 *
 * key_str: Key string (e.g., "h", "return", "space")
 *
 * Returns: Keycode value, or 0 if unknown
 */
int config_parse_keycode(const char *key_str)
{
    if (!key_str) {
        return 0;
    }
    
    /* Single character keys */
    if (strlen(key_str) == 1) {
        return (int)key_str[0];
    }
    
    /* Special keys */
    if (strcmp(key_str, "return") == 0) return 13;
    if (strcmp(key_str, "space") == 0) return 32;
    if (strcmp(key_str, "tab") == 0) return 9;
    if (strcmp(key_str, "escape") == 0) return 27;
    if (strcmp(key_str, "equal") == 0) return '=';
    if (strcmp(key_str, "minus") == 0) return '-';
    
    return 0;
}

/*
 * config_parse_action - Parse action string to KeyAction enum
 *
 * action_str: Action string
 *
 * Returns: KeyAction enum value, or ACTION_UNKNOWN if not recognized
 */
KeyAction config_parse_action(const char *action_str)
{
    if (!action_str) return ACTION_UNKNOWN;
    
    switch (action_str[0]) {
        case 'f':
            if (strcmp(action_str, "focus_left") == 0) return ACTION_FOCUS_LEFT;
            if (strcmp(action_str, "focus_right") == 0) return ACTION_FOCUS_RIGHT;
            if (strcmp(action_str, "focus_up") == 0) return ACTION_FOCUS_UP;
            if (strcmp(action_str, "focus_down") == 0) return ACTION_FOCUS_DOWN;
            if (strcmp(action_str, "float_toggle") == 0) return ACTION_FLOAT_TOGGLE;
            if (strcmp(action_str, "fullscreen_toggle") == 0) return ACTION_FULLSCREEN_TOGGLE;
            break;
        case 't':
            if (strcmp(action_str, "tile") == 0) return ACTION_TILE;
            break;
        case 'c':
            if (strcmp(action_str, "close_window") == 0) return ACTION_CLOSE_WINDOW;
            break;
        case 's':
            if (strcmp(action_str, "swap_left") == 0) return ACTION_SWAP_LEFT;
            if (strcmp(action_str, "swap_right") == 0) return ACTION_SWAP_RIGHT;
            if (strcmp(action_str, "swap_up") == 0) return ACTION_SWAP_UP;
            if (strcmp(action_str, "swap_down") == 0) return ACTION_SWAP_DOWN;
            break;
        case 'i':
            if (strcmp(action_str, "increase_size") == 0) return ACTION_INCREASE_SIZE;
            break;
        case 'd':
            if (strcmp(action_str, "decrease_size") == 0) return ACTION_DECREASE_SIZE;
            break;
    }
    
    return ACTION_UNKNOWN;
}

/*
 * config_write_default - Write default configuration to file
 *
 * filename: Path to write configuration file
 *
 * Returns: true on success, false on failure
 */
bool config_write_default(const char *filename)
{
    FILE *file;
    int i;
    
    file = fopen(filename, "w");
    if (!file) {
        return false;
    }
    
    fprintf(file, "# pengwm configuration file\n");
    fprintf(file, "# This file was auto-generated\n\n");
    
    fprintf(file, "# General settings\n");
    fprintf(file, "auto_tile true\n");
    fprintf(file, "focus_follows_mouse false\n");
    fprintf(file, "gap_size 10\n");
    fprintf(file, "split_ratio 0.5\n\n");
    
    fprintf(file, "# Keybinds\n");
    for (i = 0; i < s_default_keybind_count; i++) {
        fprintf(file, "keybind %s %s\n",
                s_default_keybinds[i].key_combo,
                config_action_to_string(s_default_keybinds[i].action));
    }
    
    fprintf(file, "\n# Window rules (examples)\n");
    fprintf(file, "# rule Terminal tile\n");
    fprintf(file, "# rule Calculator float\n");
    fprintf(file, "# rule \"System Preferences\" ignore\n");
    
    fclose(file);
    return true;
}

/*
 * config_action_to_string - Convert KeyAction to string
 *
 * action: KeyAction to convert
 *
 * Returns: String representation of action
 */
const char* config_action_to_string(KeyAction action)
{
    switch (action) {
        case ACTION_FOCUS_LEFT: return "focus_left";
        case ACTION_FOCUS_RIGHT: return "focus_right";
        case ACTION_FOCUS_UP: return "focus_up";
        case ACTION_FOCUS_DOWN: return "focus_down";
        case ACTION_TILE: return "tile";
        case ACTION_FLOAT_TOGGLE: return "float_toggle";
        case ACTION_FULLSCREEN_TOGGLE: return "fullscreen_toggle";
        case ACTION_CLOSE_WINDOW: return "close_window";
        case ACTION_SWAP_LEFT: return "swap_left";
        case ACTION_SWAP_RIGHT: return "swap_right";
        case ACTION_SWAP_UP: return "swap_up";
        case ACTION_SWAP_DOWN: return "swap_down";
        case ACTION_INCREASE_SIZE: return "increase_size";
        case ACTION_DECREASE_SIZE: return "decrease_size";
        case ACTION_UNKNOWN:
        default:
            return "unknown";
    }
}

/*
 * config_add_app_rule - Add an application rule to the configuration
 *
 * app_name: Application name to match
 * rule: Rule to apply
 *
 * Returns: true on success, false if no space available
 */
bool config_add_app_rule(const char *app_name, WindowRule rule)
{
    AppRule *app_rule;
    
    if (g_config.rule_count >= CONFIG_MAX_RULES) {
        return false;
    }
    
    app_rule = &g_config.app_rules[g_config.rule_count];
    strncpy(app_rule->app_name, app_name, CONFIG_MAX_APP_NAME - 1);
    app_rule->app_name[CONFIG_MAX_APP_NAME - 1] = '\0';
    app_rule->rule = rule;
    app_rule->enabled = true;
    
    g_config.rule_count++;
    return true;
}

/*
 * config_get_window_rule - Get window rule for an application
 *
 * app_name: Application name to look up
 *
 * Returns: WindowRule for the application, or RULE_TILE as default
 */
WindowRule config_get_window_rule(const char *app_name)
{
    int i;
    
    if (!app_name) {
        return RULE_TILE;
    }
    
    for (i = 0; i < g_config.rule_count; i++) {
        if (g_config.app_rules[i].enabled &&
            strcmp(g_config.app_rules[i].app_name, app_name) == 0) {
            return g_config.app_rules[i].rule;
        }
    }
    
    return RULE_TILE;  /* Default rule */
}

/*
 * config_rule_to_string - Convert WindowRule to string
 *
 * rule: WindowRule to convert
 *
 * Returns: String representation of rule
 */
const char* config_rule_to_string(WindowRule rule)
{
    switch (rule) {
        case RULE_TILE: return "tile";
        case RULE_FLOAT: return "float";
        case RULE_IGNORE: return "ignore";
        default: return "unknown";
    }
}

/*
 * config_find_keybind - Find keybind by keycode and modifiers
 *
 * keycode: Key code to match
 * modifiers: Key modifiers to match
 *
 * Returns: Matching keybind, or NULL if not found
 */
Keybind* config_find_keybind(int keycode, KeyModifier modifiers)
{
    int i;
    
    for (i = 0; i < g_config.keybind_count; i++) {
        if (g_config.keybinds[i].enabled &&
            g_config.keybinds[i].keycode == keycode &&
            g_config.keybinds[i].modifiers == modifiers) {
            return &g_config.keybinds[i];
        }
    }
    
    return NULL;
}

/*
 * config_reload - Reload configuration from file
 *
 * Returns: true on success, false on failure
 */
bool config_reload(void)
{
    return config_load(g_config.config_path);
}

/*
 * config_get_path - Get current configuration file path
 *
 * Returns: Configuration file path
 */
const char* config_get_path(void)
{
    return g_config.config_path;
}

/*
 * config_print - Print current configuration to stdout
 */
void config_print(void)
{
    int i;
    
    printf("pengwm Configuration:\n");
    printf("  Config file: %s\n", g_config.config_path);
    printf("\nGeneral Settings:\n");
    printf("  auto_tile: %s\n", g_config.auto_tile ? "true" : "false");
    printf("  focus_follows_mouse: %s\n", g_config.focus_follows_mouse ? "true" : "false");
    printf("  gap_size: %d\n", g_config.gap_size);
    printf("  split_ratio: %.2f\n", g_config.split_ratio);
    
    printf("\nKeybinds (%d):\n", g_config.keybind_count);
    for (i = 0; i < g_config.keybind_count; i++) {
        Keybind *kb = &g_config.keybinds[i];
        printf("  %-20s -> %s %s\n",
               kb->key_combo,
               config_action_to_string(kb->action),
               kb->enabled ? "" : "(disabled)");
    }
    
    printf("\nApplication Rules (%d):\n", g_config.rule_count);
    if (g_config.rule_count == 0) {
        printf("  (none configured)\n");
    } else {
        for (i = 0; i < g_config.rule_count; i++) {
            AppRule *rule = &g_config.app_rules[i];
            printf("  %-20s -> %s %s\n",
                   rule->app_name,
                   config_rule_to_string(rule->rule),
                   rule->enabled ? "" : "(disabled)");
        }
    }
}

/*
 * config_cleanup - Clean up configuration resources
 */
void config_cleanup(void)
{
    /* Reset configuration structure */
    memset(&g_config, 0, sizeof(g_config));
}
