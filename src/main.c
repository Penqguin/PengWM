/*
 * main.c - Main entry point for pengwm
 */

#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <unistd.h>
#include "window_manager.h"
#include "config.h"
#include "cli.h"

/* Global flag for clean shutdown */
static volatile sig_atomic_t g_shutdown_requested = 0;

/*
 * signal_handler - Handle shutdown signals gracefully
 *
 * signum: Signal number received
 */
static void signal_handler(int signum)
{
    (void)signum;  /* Unused parameter */
    g_shutdown_requested = 1;
}

/*
 * setup_signal_handlers - Install signal handlers for clean shutdown
 */
static void setup_signal_handlers(void)
{
    signal(SIGINT, signal_handler);   /* Ctrl+C */
    signal(SIGTERM, signal_handler);  /* Termination request */
    signal(SIGHUP, signal_handler);   /* Hangup (terminal closed) */
}

/*
 * print_version - Print version information
 */
static void print_version(void)
{
    printf("pengwm 1.0.0 - Binary Space Partitioning Window Manager for macOS\n");
    printf("Built with Accessibility API support\n");
}

/*
 * print_startup_banner - Print startup information
 */
static void print_startup_banner(void)
{
    printf("pengwm - BSP Window Manager starting...\n");
    printf("Initializing window management...\n");
}

/*
 * check_accessibility_permissions - Check if app has Accessibility permissions
 *
 * Returns: true if permissions are granted, false otherwise
 */
// static bool check_accessibility_permissions(void)
// {
//     /* Create a test AX reference to check permissions */
//     AXUIElementRef system_wide = AXUIElementCreateSystemWide();
//     AXUIElementRef focused_app = NULL;
    
//     if (!system_wide) {
//         return false;
//     }
    
//     AXError err = AXUIElementCopyAttributeValue(system_wide,
//                                                kAXFocusedApplicationAttribute,
//                                                (CFTypeRef*)&focused_app);
    
//     bool has_permissions = (err == kAXErrorSuccess);
    
//     if (focused_app) {
//         CFRelease(focused_app);
//     }
//     CFRelease(system_wide);
    
//     return has_permissions;
// }

/*
 * print_permission_help - Print help for granting Accessibility permissions
 */
// static void
// print_permission_help(void)
// {
//     printf("\nAccessibility Permission Required:\n");
//     printf("pengwm needs Accessibility permission to control windows.\n\n");
//     printf("To grant permission:\n");
//     printf("1. Open System Settings (or System Preferences)\n");
//     printf("2. Go to Privacy & Security -> Accessibility\n");
//     printf("3. Click the '+' button and add pengwm\n");
//     printf("4. Make sure the checkbox next to pengwm is enabled\n");
//     printf("5. Restart pengwm\n\n");
//     printf("Note: You may need to restart pengwm after granting permission.\n");
// }

/*
 * interactive_mode - Run pengwm in interactive mode
 *
 * Returns: Exit code
 */
static int interactive_mode(void)
{
    printf("Starting pengwm interactive mode...\n");
    printf("Type 'help' for commands, 'quit' to exit\n");
    
    return cli_interactive_mode();
}

/*
 * daemon_mode - Run pengwm in daemon mode (background service)
 *
 * Returns: Exit code
 */
static int daemon_mode(void)
{
    printf("Starting pengwm in daemon mode...\n");
    printf("Press Ctrl+C to exit\n");
    
    /* Main daemon loop */
    while (!g_shutdown_requested) {
        /* In a real implementation, this would:
         * - Monitor for new windows
         * - Handle window events
         * - Process hotkeys
         * - Update tiling layouts
         */
        sleep(1);  /* Simple polling for now */
    }
    
    printf("\nShutdown requested, cleaning up...\n");
    return 0;
}

/*
 * main - Main entry point
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: Exit code (0 for success, non-zero for error)
 */
int main(int argc, char **argv)
{
    bool daemon_flag = false;
    bool version_flag = false;
    bool interactive_flag = false;
    int exit_code = 0;
    int i;

    /* Parse command line flags */
    for (i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-d") == 0 || strcmp(argv[i], "--daemon") == 0) {
            daemon_flag = true;
        } else if (strcmp(argv[i], "-v") == 0 || strcmp(argv[i], "--version") == 0) {
            version_flag = true;
        } else if (strcmp(argv[i], "-i") == 0 || strcmp(argv[i], "--interactive") == 0) {
            interactive_flag = true;
        } else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            printf("Usage: %s [options] [command]\n\n", argv[0]);
            printf("Options:\n");
            printf("  -d, --daemon        Run in daemon mode\n");
            printf("  -i, --interactive   Run in interactive mode\n");
            printf("  -v, --version       Show version information\n");
            printf("  -h, --help          Show this help\n\n");
            printf("Commands:\n");
            printf("  list                List managed windows\n");
            printf("  tile                Apply BSP tiling\n");
            printf("  focus <dir>         Focus window in direction\n");
            printf("  add <pid>           Add windows from PID\n");
            printf("  remove <pid>        Remove windows from PID\n");
            printf("  config              Show configuration\n");
            printf("  help                Show detailed help\n");
            return 0;
        } else {
            /* Not a flag, break to handle as command */
            break;
        }
    }

    /* Handle version flag */
    if (version_flag) {
        print_version();
        return 0;
    }

    /* Check for Accessibility permissions */
    // if (!check_accessibility_permissions()) {
    //     printf("Error: pengwm does not have Accessibility permissions.\n");
    //     print_permission_help();
    //     return 1;
    // }

    /* Initialize configuration system */
    if (!config_init()) {
        printf("Error: Failed to initialize configuration system\n");
        return 1;
    }

    /* Initialize window manager */
    if (!wm_init()) {
        printf("Error: Failed to initialize window manager\n");
        config_cleanup();
        return 1;
    }

    print_startup_banner();

    /* Setup signal handlers for clean shutdown */
    setup_signal_handlers();

    /* Determine operation mode */
    if (daemon_flag) {
        exit_code = daemon_mode();
    } else if (interactive_flag) {
        exit_code = interactive_mode();
    } else if (argc > i) {
        /* Command mode - handle single command */
        exit_code = cli_handle_command(argc - i + 1, &argv[i - 1]);
    } else {
        /* No arguments - show usage */
        printf("pengwm initialized successfully.\n");
        printf("Use '%s help' for available commands\n", argv[0]);
        printf("Use '%s -i' for interactive mode\n", argv[0]);
        printf("Use '%s -d' for daemon mode\n", argv[0]);
    }

    /* Cleanup */
    printf("Cleaning up...\n");
    wm_cleanup();
    config_cleanup();

    return exit_code;
}
