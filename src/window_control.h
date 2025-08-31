/*
 * window_control.h - Window control using macOS Accessibility API
 *
 * Handles discovery, management, and control of application windows
 * using the macOS Accessibility framework.
 */

#ifndef WINDOW_CONTROL_H
#define WINDOW_CONTROL_H

#include <ApplicationServices/ApplicationServices.h>
#include <CoreFoundation/CoreFoundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <stdbool.h>

/* Structure representing a managed window */
typedef struct {
    AXUIElementRef window_ref;
    int window_id;
    int pid;
    CGRect frame;
    char app_name[256];
} ManagedWindow;

/* Window discovery and management */
ManagedWindow* get_manageable_windows(size_t *count);
bool should_manage_window(const char *app_name, CGRect bounds, int pid);
AXUIElementRef get_ax_window_for_pid_and_bounds(int pid, CGRect target_bounds);

/* Window frame operations */
bool get_window_frame(AXUIElementRef window_ref, CGPoint *pos, CGSize *size);
bool set_window_frame(AXUIElementRef window_ref, CGRect new_frame);
bool frames_match(CGRect frame1, CGRect frame2, float tolerance);

/* Window focus and navigation */
ManagedWindow* get_currently_focused_window(void);
void focus_window(AXUIElementRef window_ref);

/* Window ID management */
int generate_window_id(void);

/* Managed window array operations */
void add_to_managed_windows(AXUIElementRef window_ref, int window_id, int pid, CGRect frame);
void remove_from_managed_windows(int window_id);

/* Event handling */
void setup_window_events(void);
void window_event_callback(AXObserverRef observer, AXUIElementRef element,
                          CFStringRef notification, void *refcon);
void handle_new_window(AXUIElementRef window_ref);
void handle_window_destroyed(AXUIElementRef window_ref);

/* Global managed windows access */
extern ManagedWindow *g_managed_windows;
extern size_t g_managed_count;

#endif /* WINDOW_CONTROL_H */
