#include <CoreGraphics/CoreGraphics.h>
#include <stdio.h>

void list_displays(void){
    uint32_t displayCount = 0;
    CGGetActiveDisplayList(0, NULL, &displayCount);
    
    CGDirectDisplayID displays[displayCount];
    CGGetActiveDisplayList(displayCount, displays, &displayCount);
    
    for (uint32_t i = 0; i < displayCount; i++) {
        CGRect bounds = CGDisplayBounds(displays[i]);
        printf("Display %u: origin=(%.0f,%.0f) size=%.0fx%.0f\n",
               i,
               bounds.origin.x, bounds.origin.y,
               bounds.size.width, bounds.size.height
        );
    }
}
