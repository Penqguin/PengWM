/*
 * window_control.c - Window control implementation
 *
 * Implementation of window discovery, management, and control
 * using the macOS Accessibility API.
 */

#include "window_control.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

/* Global managed windows array */
ManagedWindow *g_managed_windows = NULL;
size_t g_managed_count = 0;

/* Static window ID counter */
static int s_next_window_id = 1000;

/*
 * get_manageable_windows - Discover all windows that should be managed
 *
 * Returns: Array of ManagedWindow structures
 * count: Output parameter for number of windows found
 */
ManagedWindow* get_manageable_windows(size_t *count)
{
    CFArrayRef window_list;
    CFIndex window_count;
    ManagedWindow *windows;
    size_t manageable_count = 0;
    CFIndex i;

    /* Get all on-screen windows */
    window_list = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID
    );
    
    if (!window_list) {
        *count = 0;
        return NULL;
    }

    window_count = CFArrayGetCount(window_list);
    windows = calloc(window_count, sizeof(ManagedWindow));
    
    if (!windows) {
        CFRelease(window_list);
        *count = 0;
        return NULL;
    }

    for (i = 0; i < window_count; i++) {
        CFDictionaryRef win_info = CFArrayGetValueAtIndex(window_list, i);
        int pid = 0;
        CGRect bounds = CGRectZero;
        char app_name[256] = {0};
        AXUIElementRef window_ref;
        CFNumberRef pid_ref;
        CFDictionaryRef bounds_dict;
        CFStringRef owner_name;

        /* Extract PID */
        pid_ref = CFDictionaryGetValue(win_info, kCGWindowOwnerPID);
        if (pid_ref) {
            CFNumberGetValue(pid_ref, kCFNumberIntType, &pid);
        }

        /* Extract window bounds */
        bounds_dict = CFDictionaryGetValue(win_info, kCGWindowBounds);
        if (bounds_dict) {
            CGRectMakeWithDictionaryRepresentation(bounds_dict, &bounds);
        }

        /* Extract app name */
        owner_name = CFDictionaryGetValue(win_info, kCGWindowOwnerName);
        if (owner_name) {
            CFStringGetCString(owner_name, app_name, sizeof(app_name),
                             kCFStringEncodingUTF8);
        }

        /* Skip windows that shouldn't be managed */
        if (!should_manage_window(app_name, bounds, pid)) {
            continue;
        }

        /* Get AXUIElement for this window */
        window_ref = get_ax_window_for_pid_and_bounds(pid, bounds);
        if (!window_ref) {
            continue;
        }

        /* Add to manageable windows array */
        windows[manageable_count].window_ref = window_ref;
        windows[manageable_count].window_id = generate_window_id();
        windows[manageable_count].pid = pid;
        windows[manageable_count].frame = bounds;
        strncpy(windows[manageable_count].app_name, app_name, 255);
        manageable_count++;
    }

    CFRelease(window_list);
    *count = manageable_count;
    return windows;
}

/*
 * should_manage_window - Determine if a window should be managed
 *
 * app_name: Name of the application owning the window
 * bounds: Window bounds rectangle
 * pid: Process ID of the owning application
 *
 * Returns: true if window should be managed, false otherwise
 */
bool should_manage_window(const char *app_name, CGRect bounds, int pid)
{
    (void)pid; /* Unused parameter */

    /* Skip windows that are too small */
    if (bounds.size.width < 100 || bounds.size.height < 100) {
        return false;
    }

    /* Skip system applications and utilities */
    if (strcmp(app_name, "WindowServer") == 0 ||
        strcmp(app_name, "Dock") == 0 ||
        strcmp(app_name, "Control Center") == 0 ||
        strcmp(app_name, "Notification Center") == 0 ||
        strcmp(app_name, "SystemUIServer") == 0) {
        return false;
    }

    return true;
}

/*
 * get_ax_window_for_pid_and_bounds - Get AXUIElement for window matching bounds
 *
 * pid: Process ID of the application
 * target_bounds: Target window bounds to match
 *
 * Returns: AXUIElementRef for the matching window, or NULL if not found
 */
AXUIElementRef get_ax_window_for_pid_and_bounds(int pid, CGRect target_bounds)
{
    AXUIElementRef app_ref = AXUIElementCreateApplication(pid);
    CFArrayRef window_list = NULL;
    AXError err;
    CFIndex count, i;
    AXUIElementRef result = NULL;

    if (!app_ref) {
        return NULL;
    }

    /* Get all windows for this application */
    err = AXUIElementCopyAttributeValue(app_ref, kAXWindowsAttribute,
                                       (CFTypeRef*)&window_list);
    if (err != kAXErrorSuccess || !window_list) {
        CFRelease(app_ref);
        return NULL;
    }

    count = CFArrayGetCount(window_list);
    for (i = 0; i < count; i++) {
        AXUIElementRef window_ref = CFArrayGetValueAtIndex(window_list, i);
        CGPoint pos;
        CGSize size;

        if (get_window_frame(window_ref, &pos, &size)) {
            CGRect window_frame = CGRectMake(pos.x, pos.y, size.width, size.height);
            
            /* Check if this matches our target bounds */
            if (frames_match(window_frame, target_bounds, 10.0)) {
                CFRetain(window_ref);
                result = window_ref;
                break;
            }
        }
    }

    CFRelease(window_list);
    CFRelease(app_ref);
    return result;
}

/*
 * get_window_frame - Get position and size of a window
 *
 * window_ref: AXUIElement reference to the window
 * pos: Output parameter for window position
 * size: Output parameter for window size
 *
 * Returns: true on success, false on failure
 */
bool
get_window_frame(AXUIElementRef window_ref, CGPoint *pos, CGSize *size)
{
    CFTypeRef pos_value = NULL;
    CFTypeRef size_value = NULL;
    AXError err;
    bool success = false;

    /* Get position */
    err = AXUIElementCopyAttributeValue(window_ref, kAXPositionAttribute, &pos_value);
    if (err != kAXErrorSuccess) {
        goto cleanup;
    }

    if (!AXValueGetValue(pos_value, kAXValueCGPointType, pos)) {
        goto cleanup;
    }

    /* Get size */
    err = AXUIElementCopyAttributeValue(window_ref, kAXSizeAttribute, &size_value);
    if (err != kAXErrorSuccess) {
        goto cleanup;
    }

    if (!AXValueGetValue(size_value, kAXValueCGSizeType, size)) {
        goto cleanup;
    }

    success = true;

cleanup:
    if (pos_value) CFRelease(pos_value);
    if (size_value) CFRelease(size_value);
    return success;
}

/*
 * set_window_frame - Set position and size of a window
 *
 * window_ref: AXUIElement reference to the window
 * new_frame: New frame rectangle for the window
 *
 * Returns: true on success, false on failure
 */
bool
set_window_frame(AXUIElementRef window_ref, CGRect new_frame)
{
    CGPoint new_pos = new_frame.origin;
    CGSize new_size = new_frame.size;
    CFTypeRef pos_value = AXValueCreate(kAXValueCGPointType, &new_pos);
    CFTypeRef size_value = AXValueCreate(kAXValueCGSizeType, &new_size);
    AXError pos_err, size_err;
    bool success = false;

    if (!pos_value || !size_value) {
        goto cleanup;
    }

    /* Set position */
    pos_err = AXUIElementSetAttributeValue(window_ref, kAXPositionAttribute, pos_value);
    
    /* Set size */
    size_err = AXUIElementSetAttributeValue(window_ref, kAXSizeAttribute, size_value);

    success = (pos_err == kAXErrorSuccess) && (size_err == kAXErrorSuccess);

cleanup:
    if (pos_value) CFRelease(pos_value);
    if (size_value) CFRelease(size_value);
    return success;
}

/*
 * frames_match - Check if two frames match within tolerance
 *
 * frame1: First frame to compare
 * frame2: Second frame to compare
 * tolerance: Maximum allowed difference in pixels
 *
 * Returns: true if frames match within tolerance
 */
bool
frames_match(CGRect frame1, CGRect frame2, float tolerance)
{
    return (fabs(frame1.origin.x - frame2.origin.x) <= tolerance &&
            fabs(frame1.origin.y - frame2.origin.y) <= tolerance &&
            fabs(frame1.size.width - frame2.size.width) <= tolerance &&
            fabs(frame1.size.height - frame2.size.height) <= tolerance);
}

/*
 * get_currently_focused_window - Get the system's currently focused window
 *
 * Returns: Pointer to ManagedWindow if found, NULL otherwise
 */
ManagedWindow*
get_currently_focused_window(void)
{
    AXUIElementRef system_wide = AXUIElementCreateSystemWide();
    AXUIElementRef focused_app = NULL;
    AXUIElementRef focused_window = NULL;
    AXError err;
    size_t i;
    ManagedWindow *result = NULL;

    if (!system_wide) {
        return NULL;
    }

    /* Get focused application */
    err = AXUIElementCopyAttributeValue(system_wide,
                                       kAXFocusedApplicationAttribute,
                                       (CFTypeRef*)&focused_app);
    if (err != kAXErrorSuccess) {
        goto cleanup;
    }

    /* Get focused window of that application */
    err = AXUIElementCopyAttributeValue(focused_app,
                                       kAXFocusedWindowAttribute,
                                       (CFTypeRef*)&focused_window);
    if (err != kAXErrorSuccess) {
        goto cleanup;
    }

    /* Find this window in our managed windows */
    for (i = 0; i < g_managed_count; i++) {
        if (CFEqual(g_managed_windows[i].window_ref, focused_window)) {
            result = &g_managed_windows[i];
            break;
        }
    }

cleanup:
    if (focused_window) CFRelease(focused_window);
    if (focused_app) CFRelease(focused_app);
    CFRelease(system_wide);
    return result;
}

/*
 * focus_window - Bring a window to front and focus it
 *
 * window_ref: AXUIElement reference to the window to focus
 */
void
focus_window(AXUIElementRef window_ref)
{
    /* Bring window to front */
    AXUIElementSetAttributeValue(window_ref, kAXMainAttribute, kCFBooleanTrue);
    AXUIElementPerformAction(window_ref, kAXRaiseAction);
}

/*
 * generate_window_id - Generate a unique window ID
 *
 * Returns: New unique window ID
 */
int
generate_window_id(void)
{
    return s_next_window_id++;
}

/*
 * add_to_managed_windows - Add a window to the managed windows array
 *
 * window_ref: AXUIElement reference to the window
 * window_id: Unique ID for the window
 * pid: Process ID of the owning application
 * frame: Current frame of the window
 */
void
add_to_managed_windows(AXUIElementRef window_ref, int window_id, int pid, CGRect frame)
{
    ManagedWindow *new_window;

    /* Resize array */
    g_managed_windows = realloc(g_managed_windows,
                               (g_managed_count + 1) * sizeof(ManagedWindow));
    
    if (!g_managed_windows) {
        fprintf(stderr, "Failed to allocate memory for managed windows\n");
        return;
    }

    new_window = &g_managed_windows[g_managed_count];
    new_window->window_ref = window_ref;
    CFRetain(window_ref);
    new_window->window_id = window_id;
    new_window->pid = pid;
    new_window->frame = frame;
    strcpy(new_window->app_name, "Unknown");

    g_managed_count++;
}

/*
 * remove_from_managed_windows - Remove a window from the managed windows array
 *
 * window_id: ID of the window to remove
 */
void
remove_from_managed_windows(int window_id)
{
    size_t i, j;

    for (i = 0; i < g_managed_count; i++) {
        if (g_managed_windows[i].window_id == window_id) {
            /* Release AX reference */
            CFRelease(g_managed_windows[i].window_ref);
            
            /* Shift array elements */
            for (j = i; j < g_managed_count - 1; j++) {
                g_managed_windows[j] = g_managed_windows[j + 1];
            }
            g_managed_count--;
            break;
        }
    }
}

/*
 * setup_window_events - Setup window event monitoring
 */
void
setup_window_events(void)
{
    AXObserverRef observer;
    AXError err = AXObserverCreate(getpid(), window_event_callback, &observer);
    
    if (err != kAXErrorSuccess) {
        printf("Failed to create AX observer\n");
        return;
    }
    
    /* Add to run loop */
    CFRunLoopAddSource(
        CFRunLoopGetCurrent(),
        AXObserverGetRunLoopSource(observer),
        kCFRunLoopDefaultMode
    );
}

/*
 * window_event_callback - Handle window events
 */
void
window_event_callback(AXObserverRef observer, AXUIElementRef element,
                     CFStringRef notification, void *refcon)
{
    (void)observer;  /* Unused parameter */
    (void)refcon;    /* Unused parameter */

    if (CFEqual(notification, kAXWindowCreatedNotification)) {
        handle_new_window(element);
    } else if (CFEqual(notification, kAXUIElementDestroyedNotification)) {
        handle_window_destroyed(element);
    }
}

/*
 * handle_new_window - Handle new window creation
 */
void
handle_new_window(AXUIElementRef window_ref)
{
    pid_t pid;
    AXError err = AXUIElementGetPid(window_ref, &pid);
    
    if (err != kAXErrorSuccess) {
        return;
    }

    /* This is a placeholder - full implementation would integrate with BSP tree */
    printf("New window detected from PID %d\n", (int)pid);
}

/*
 * handle_window_destroyed - Handle window destruction
 */
void
handle_window_destroyed(AXUIElementRef window_ref)
{
    size_t i;

    /* Find and remove from managed windows */
    for (i = 0; i < g_managed_count; i++) {
        if (CFEqual(g_managed_windows[i].window_ref, window_ref)) {
            remove_from_managed_windows(g_managed_windows[i].window_id);
            break;
        }
    }
}
