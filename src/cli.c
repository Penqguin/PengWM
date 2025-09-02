/*
 * cli.c - Command line interface implementation
 */

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "cli.h"
#include "window.h"
#include "window_manager.h"
#include "config.h"

typedef enum {
    CMD_LIST = 0,
    CMD_TILE,
    CMD_FOCUS,
    CMD_ADD,
    CMD_REMOVE,
    CMD_HELP,
    CMD_CONFIG,
    CMD_STATUS,
    CMD_QUIT,
    CMD_UNKNOWN
} Command;

/*
 * parse_command - Parse command string into Command enum
 *
 * cmd: Command string to parse
 *
 * Returns: Command enum value
 */
static Command parse_command(const char *cmd)
{
    if (!cmd) {
        return CMD_UNKNOWN;
    }

    /* Use first character for fast lookup, then verify full string */
    switch (cmd[0]) {
        case 'l':
            if (strcmp(cmd, "list") == 0) return CMD_LIST;
            break;
        case 't':
            if (strcmp(cmd, "tile") == 0) return CMD_TILE;
            break;
        case 'f':
            if (strcmp(cmd, "focus") == 0) return CMD_FOCUS;
            break;
        case 'a':
            if (strcmp(cmd, "add") == 0) return CMD_ADD;
            break;
        case 'r':
            if (strcmp(cmd, "remove") == 0) return CMD_REMOVE;
            break;
        case 'h':
            if (strcmp(cmd, "help") == 0) return CMD_HELP;
            break;
        case 'c':
            if (strcmp(cmd, "config") == 0) return CMD_CONFIG;
            break;
        case 's':
            if (strcmp(cmd, "status") == 0) return CMD_STATUS;
            break;
        case 'q':
            if (strcmp(cmd, "quit") == 0) return CMD_QUIT;
            break;
        default:
            break;
    }
    
    return CMD_UNKNOWN;
}

/*
 * validate_direction - Validate direction argument
 *
 * direction: Direction string to validate
 *
 * Returns: true if direction is valid
 */
static bool validate_direction(const char *direction)
{
    if (!direction) {
        return false;
    }

    switch (direction[0]) {
        case 'l':
            return strcmp(direction, "left") == 0;
        case 'r':
            return strcmp(direction, "right") == 0;
        case 'u':
            return strcmp(direction, "up") == 0;
        case 'd':
            return strcmp(direction, "down") == 0;
        default:
            return false;
    }
}

/*
 * print_usage - Print usage information
 *
 * program_name: Name of the program
 */
static void print_usage(const char *program_name)
{
    printf("Usage: %s <command> [args]\n\n", program_name);
    printf("Commands:\n");
    printf("  list                     List all managed windows\n");
    printf("  tile                     Apply BSP tiling to all windows\n");
    printf("  focus <direction>        Focus window in direction (left/right/up/down)\n");
    printf("  add <pid>                Add windows from process ID\n");
    printf("  remove <pid>             Remove windows from process ID\n");
    printf("  config [reload]          Show or reload configuration\n");
    printf("  status                   Show window manager status\n");
    printf("  help                     Show this help message\n");
    printf("  quit                     Exit interactive mode\n");
}

/*
 * print_help - Print detailed help information
 */
static void print_help(void)
{
    printf("pengwm - Binary Space Partitioning Window Manager for macOS\n\n");
    
    printf("DESCRIPTION:\n");
    printf("  pengwm is a tiling window manager that automatically organizes\n");
    printf("  windows using binary space partitioning (BSP) algorithm.\n\n");
    
    printf("EXAMPLES:\n");
    printf("  pengwm list              # Show all managed windows\n");
    printf("  pengwm tile              # Apply tiling layout\n");
    printf("  pengwm focus left        # Focus window to the left\n");
    printf("  pengwm add 1234          # Add windows from process 1234\n");
    printf("  pengwm remove 1234       # Remove windows from process 1234\n\n");
    
    printf("PERMISSIONS:\n");
    printf("  pengwm requires Accessibility permissions to control windows.\n");
    printf("  Grant permission in: System Settings -> Privacy & Security -> Accessibility\n\n");
    
    printf("CONFIGURATION:\n");
    printf("  Configuration file: ~/.pengwm/config\n");
    printf("  Use 'pengwm config' to view current settings\n");
}

/*
 * handle_focus_command - Handle focus command with validation
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error
 */
static int handle_focus_command(int argc, char **argv)
{
    if (argc < 3) {
        printf("Error: focus command requires direction argument\n");
        printf("Usage: %s focus <left|right|up|down>\n", argv[0]);
        return 1;
    }

    if (!validate_direction(argv[2])) {
        printf("Error: invalid direction '%s'\n", argv[2]);
        printf("Valid directions: left, right, up, down\n");
        return 1;
    }

    wm_focus(argv[2]);
    return 0;
}

/*
 * handle_add_command - Handle add command with validation
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error
 */
static int handle_add_command(int argc, char **argv)
{
    int pid;
    char *endptr;

    if (argc < 3) {
        printf("Error: add command requires PID argument\n");
        printf("Usage: %s add <pid>\n", argv[0]);
        return 1;
    }

    pid = (int)strtol(argv[2], &endptr, 10);
    if (*endptr != '\0' || pid <= 0) {
        printf("Error: invalid PID '%s' (must be positive integer)\n", argv[2]);
        return 1;
    }

    wm_add_window(pid);
    return 0;
}

/*
 * handle_remove_command - Handle remove command with validation
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error
 */
static int handle_remove_command(int argc, char **argv)
{
    int pid;
    char *endptr;

    if (argc < 3) {
        printf("Error: remove command requires PID argument\n");
        printf("Usage: %s remove <pid>\n", argv[0]);
        return 1;
    }

    pid = (int)strtol(argv[2], &endptr, 10);
    if (*endptr != '\0' || pid <= 0) {
        printf("Error: invalid PID '%s' (must be positive integer)\n", argv[2]);
        return 1;
    }

    wm_close_window(pid);
    return 0;
}

/*
 * handle_config_command - Handle config command
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error
 */
static int handle_config_command(int argc, char **argv)
{
    if (argc >= 3 && strcmp(argv[2], "reload") == 0) {
        if (config_reload()) {
            printf("Configuration reloaded successfully\n");
        } else {
            printf("Failed to reload configuration\n");
            return 1;
        }
    } else {
        config_print();
    }
    return 0;
}

/*
 * handle_status_command - Handle status command
 *
 * Returns: 0 always
 */
static int handle_status_command(void)
{
    extern size_t g_managed_count;
    extern BSPWorkspace *g_workspaces;
    extern size_t g_workspace_count;
    
    printf("pengwm Status:\n");
    printf("  Workspaces: %zu\n", g_workspace_count);
    printf("  Managed Windows: %zu\n", g_managed_count);
    printf("  Configuration: %s\n", config_get_path());
    printf("  BSP Algorithm: Active\n");
    
    return 0;
}

/*
 * cli_handle_command - Handle command line interface commands
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error, 2 for quit command
 */
int cli_handle_command(int argc, char **argv)
{
    Command cmd;
    
    if (argc < 2) {
        print_usage(argv[0]);
        return 1;
    }

    cmd = parse_command(argv[1]);

    switch (cmd) {
        case CMD_LIST:
            wm_list_windows();
            break;

        case CMD_TILE:
            printf("Applying BSP tiling...\n");
            wm_tile();
            break;

        case CMD_FOCUS:
            return handle_focus_command(argc, argv);

        case CMD_ADD:
            return handle_add_command(argc, argv);

        case CMD_REMOVE:
            return handle_remove_command(argc, argv);

        case CMD_CONFIG:
            return handle_config_command(argc, argv);

        case CMD_STATUS:
            return handle_status_command();

        case CMD_HELP:
            print_help();
            break;

        case CMD_QUIT:
            printf("Exiting pengwm...\n");
            return 2;  /* Special return code for quit */

        case CMD_UNKNOWN:
        default:
            printf("Error: unknown command '%s'\n", argv[1]);
            printf("Use '%s help' for available commands\n", argv[0]);
            return 1;
    }

    return 0;
}

/*
 * cli_interactive_mode - Run interactive command mode
 *
 * Returns: 0 on normal exit
 */
int cli_interactive_mode(void)
{
    char input[256];
    char *argv[16];
    int argc;
    int result;
    
    printf("pengwm interactive mode - type 'help' for commands, 'quit' to exit\n");
    
    while (1) {
        printf("pengwm> ");
        fflush(stdout);
        
        if (!fgets(input, sizeof(input), stdin)) {
            break;  /* EOF or error */
        }
        
        /* Remove trailing newline */
        input[strcspn(input, "\n")] = '\0';
        
        /* Skip empty lines */
        if (strlen(input) == 0) {
            continue;
        }
        
        /* Simple argument parsing */
        argc = 0;
        argv[argc++] = "pengwm";  /* Program name */
        
        char *token = strtok(input, " \t");
        while (token && argc < 15) {
            argv[argc++] = token;
            token = strtok(NULL, " \t");
        }
        argv[argc] = NULL;
        
        if (argc > 1) {
            result = cli_handle_command(argc, argv);
            if (result == 2) {
                break;  /* Quit command */
            }
        }
    }
    
    return 0;
}
