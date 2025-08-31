#include "window.h"
#include <CoreGraphics/CoreGraphics.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>

void window_list(void) {
    CFArrayRef windowList = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID
    );

    CFIndex count = CFArrayGetCount(windowList);
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef winInfo = CFArrayGetValueAtIndex(windowList, i);

        CFStringRef ownerName = CFDictionaryGetValue(winInfo, kCGWindowOwnerName);
        int pid = 0;
        CFNumberRef pidRef = CFDictionaryGetValue(winInfo, kCGWindowOwnerPID);
        if (pidRef) CFNumberGetValue(pidRef, kCFNumberIntType, &pid);

        if (ownerName) {
            char nameBuf[256];
            CFStringGetCString(ownerName, nameBuf, sizeof(nameBuf), kCFStringEncodingUTF8);
            printf("PID: %d, App: %s\n", pid, nameBuf);
        }
    }

    CFRelease(windowList);
}
