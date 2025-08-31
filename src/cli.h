/*
 * cli.h - Command line interface header
 */

#ifndef CLI_H
#define CLI_H

/*
 * cli_handle_command - Handle command line interface commands
 *
 * argc: Argument count
 * argv: Argument vector
 *
 * Returns: 0 on success, 1 on error, 2 for quit command
 */
int cli_handle_command(int argc, char **argv);

/*
 * cli_interactive_mode - Run interactive command mode
 *
 * Returns: 0 on normal exit
 */
int cli_interactive_mode(void);

#endif /* CLI_H */
