#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreFoundation/CoreFoundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <dispatch/dispatch.h>
#include <math.h>
#include <pthread.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef struct {
    uint32_t display_id;
    double x;
    double y;
    double width;
    double height;
    uint32_t pixel_width;
    uint32_t pixel_height;
    uint32_t stride;
    uint8_t *rgba;
} FievelCapture;

typedef struct {
    double x;
    double y;
    double width;
    double height;
    const char *label;
    bool selected;
    uint32_t prefix_length;
    bool custom_colors;
    bool debug_dot;
    uint32_t border_color;
    uint32_t fill_color;
} FievelHint;

typedef struct FievelQueuedEvent {
    int kind;
    uint16_t keycode;
    int value;
    bool repeat;
    double dx;
    double dy;
    CGEventRef original;
    struct FievelQueuedEvent *next;
} FievelQueuedEvent;

typedef struct {
    int kind;
    uint16_t keycode;
    int value;
    bool repeat;
    double dx;
    double dy;
    CGEventRef original;
} FievelInputEvent;

static NSWindow *overlay_window;
static NSView *overlay_view;
static NSArray<NSDictionary *> *hint_items;
static bool hints_hidden;
static bool indicator_visible;
static bool indicator_locked;
static NSString *indicator_label;
static CFTimeInterval cursor_pulse_until;
static NSPoint cursor_pulse_position;
static CFMachPortRef event_tap;
static CFRunLoopSourceRef event_source;
static pthread_t event_thread;
static pthread_mutex_t event_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t event_ready = PTHREAD_COND_INITIALIZER;
static FievelQueuedEvent *event_head;
static FievelQueuedEvent *event_tail;
static bool event_thread_ready;
static bool event_thread_failed;
static uint64_t modifier_keys_held;
static pthread_mutex_t pointer_lock = PTHREAD_MUTEX_INITIALIZER;
static CGPoint last_pointer = {NAN, NAN};
static const int64_t event_marker = 0x66696576656c6d63LL;
static bool posted_keys[256];
static bool posted_buttons[2];
static bool exit_cleanup_registered;
static void fievel_release_posted_inputs(void);

@interface FievelOverlayView : NSView
@end

@implementation FievelOverlayView
- (BOOL)isFlipped {
    return NO;
}

- (void)drawRect:(NSRect)dirtyRect {
    [super drawRect:dirtyRect];
    if (!hints_hidden) {
        for (NSDictionary *item in hint_items) {
            NSRect rect = [(NSValue *)item[@"rect"] rectValue];
            NSColor *fill = item[@"fill"];
            NSColor *border = item[@"border"];
            if ([item[@"debugDot"] boolValue]) {
                [fill setFill];
                NSRectFill(rect);
                continue;
            }
            NSBezierPath *path = [NSBezierPath bezierPathWithRoundedRect:rect xRadius:3 yRadius:3];
            [fill setFill];
            [path fill];
            [border setStroke];
            path.lineWidth = 1.5;
            [path stroke];

            NSString *label = item[@"label"];
            if (label.length == 0) {
                continue;
            }
            NSRect labelRect = NSMakeRect(NSMinX(rect), NSMinY(rect), MAX(18, label.length * 10 + 8), 20);
            [[item[@"readability"] colorWithAlphaComponent:0.9] setFill];
            [[NSBezierPath bezierPathWithRoundedRect:labelRect xRadius:2 yRadius:2] fill];
            NSDictionary *textAttributes = @{
                NSFontAttributeName: [NSFont boldSystemFontOfSize:13],
                NSForegroundColorAttributeName: item[@"text"],
            };
            uint32_t prefixLength = [item[@"prefixLength"] unsignedIntValue];
            prefixLength = MIN(prefixLength, (uint32_t)label.length);
            NSString *prefix = [label substringToIndex:prefixLength];
            NSString *suffix = [label substringFromIndex:prefixLength];
            NSDictionary *prefixAttributes = @{
                NSFontAttributeName: [NSFont boldSystemFontOfSize:13],
                NSForegroundColorAttributeName: item[@"highlight"],
            };
            NSPoint textPoint = NSMakePoint(NSMinX(labelRect) + 4, NSMinY(labelRect) + 2);
            [prefix drawAtPoint:textPoint withAttributes:prefixAttributes];
            textPoint.x += [prefix sizeWithAttributes:prefixAttributes].width;
            [suffix drawAtPoint:textPoint withAttributes:textAttributes];
        }
    }

    if (indicator_visible) {
        NSScreen *screen = NSScreen.mainScreen;
        NSRect screenFrame = screen.frame;
        NSString *label = indicator_label ?: (indicator_locked ? @"HOLD" : @"fievel");
        NSRect rect = NSMakeRect(
            NSMinX(screenFrame) - NSMinX(overlay_window.frame) + 12,
            NSMinY(screenFrame) - NSMinY(overlay_window.frame) + 12,
            MAX(72, label.length * 8 + 18),
            24
        );
        NSColor *fill = [NSColor colorWithCalibratedRed:0.10 green:0.10 blue:0.10 alpha:0.78];
        [fill setFill];
        [[NSBezierPath bezierPathWithRoundedRect:rect xRadius:4 yRadius:4] fill];
        NSDictionary *attributes = @{
            NSFontAttributeName: [NSFont boldSystemFontOfSize:11],
            NSForegroundColorAttributeName: NSColor.whiteColor,
        };
        [label
            drawAtPoint:NSMakePoint(NSMinX(rect) + 9, NSMinY(rect) + 6)
            withAttributes:attributes];
    }

    if (CFAbsoluteTimeGetCurrent() < cursor_pulse_until) {
        NSPoint window_point = [overlay_window convertPointFromScreen:cursor_pulse_position];
        NSPoint center = [overlay_view convertPoint:window_point fromView:nil];
        for (NSDictionary *ring in @[
            @{ @"radius": @52, @"width": @4, @"alpha": @0.72 },
            @{ @"radius": @35, @"width": @5, @"alpha": @0.92 },
            @{ @"radius": @18, @"width": @3, @"alpha": @0.82 },
        ]) {
            CGFloat radius = [ring[@"radius"] doubleValue];
            NSBezierPath *path = [NSBezierPath bezierPathWithOvalInRect:
                NSMakeRect(center.x - radius, center.y - radius, radius * 2, radius * 2)];
            path.lineWidth = [ring[@"width"] doubleValue];
            [[NSColor colorWithCalibratedRed:1.0
                green:0.12
                blue:0.12
                alpha:[ring[@"alpha"] doubleValue]] setStroke];
            [path stroke];
        }
    }
}
@end

static void fievel_ui_ensure(void) {
    if (overlay_window != nil) {
        return;
    }
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    NSRect frame = NSZeroRect;
    for (NSScreen *screen in NSScreen.screens) {
        frame = NSIsEmptyRect(frame) ? screen.frame : NSUnionRect(frame, screen.frame);
    }
    overlay_window = [[NSWindow alloc] initWithContentRect:frame
        styleMask:NSWindowStyleMaskBorderless
        backing:NSBackingStoreBuffered
        defer:NO];
    overlay_window.opaque = NO;
    overlay_window.backgroundColor = NSColor.clearColor;
    overlay_window.ignoresMouseEvents = YES;
    overlay_window.hasShadow = NO;
    overlay_window.level = NSStatusWindowLevel + 1;
    overlay_window.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorStationary |
        NSWindowCollectionBehaviorIgnoresCycle;
    overlay_view = [[FievelOverlayView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
    overlay_window.contentView = overlay_view;
    hint_items = @[];
    [overlay_window orderFrontRegardless];
    [NSApp activateIgnoringOtherApps:YES];
}

void fievel_ui_init(void) {
    @autoreleasepool {
        fievel_ui_ensure();
    }
}

void fievel_ui_set_indicator(bool visible, bool locked, const char *label) {
    @autoreleasepool {
        fievel_ui_ensure();
        indicator_visible = visible;
        indicator_locked = locked;
        indicator_label = label == NULL ? nil : [NSString stringWithUTF8String:label];
        [overlay_view setNeedsDisplay:YES];
    }
}

static NSColor *fievel_color(uint32_t rgba) {
    CGFloat r = (CGFloat)((rgba >> 24) & 0xff) / 255.0;
    CGFloat g = (CGFloat)((rgba >> 16) & 0xff) / 255.0;
    CGFloat b = (CGFloat)((rgba >> 8) & 0xff) / 255.0;
    CGFloat a = (CGFloat)(rgba & 0xff) / 255.0;
    return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
}

void fievel_ui_set_hints(
    const FievelHint *hints,
    size_t count,
    bool hidden,
    uint32_t border,
    uint32_t fill,
    uint32_t readability,
    uint32_t text,
    uint32_t highlight
) {
    @autoreleasepool {
        fievel_ui_ensure();
        NSMutableArray<NSDictionary *> *items = [NSMutableArray arrayWithCapacity:count];
        for (size_t i = 0; i < count; i++) {
            const FievelHint *hint = &hints[i];
            NSString *label = hint->label == NULL ? @"" : [NSString stringWithUTF8String:hint->label];
            NSRect rect = NSMakeRect(
                hint->x - NSMinX(overlay_window.frame),
                hint->y - NSMinY(overlay_window.frame),
                hint->width,
                hint->height
            );
            [items addObject:@{
                @"rect": [NSValue valueWithRect:rect],
                @"label": label ?: @"",
                @"selected": @(hint->selected),
                @"prefixLength": @(hint->prefix_length),
                @"debugDot": @(hint->debug_dot),
                @"border": fievel_color(hint->custom_colors ? hint->border_color : border),
                @"fill": fievel_color(hint->custom_colors ? hint->fill_color : fill),
                @"readability": fievel_color(readability),
                @"text": fievel_color(text),
                @"highlight": fievel_color(highlight),
            }];
        }
        hint_items = [items copy];
        hints_hidden = hidden;
        [overlay_view setNeedsDisplay:YES];
    }
}

void fievel_ui_clear_hints(void) {
    @autoreleasepool {
        fievel_ui_ensure();
        hint_items = @[];
        hints_hidden = false;
        [overlay_view setNeedsDisplay:YES];
    }
}

void fievel_ui_pump(void) {
    @autoreleasepool {
        fievel_ui_ensure();
        if (cursor_pulse_until > 0 && CFAbsoluteTimeGetCurrent() >= cursor_pulse_until) {
            cursor_pulse_until = 0;
            [overlay_view setNeedsDisplay:YES];
        }
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:0];
        NSEvent *event;
        while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
            untilDate:deadline
            inMode:NSDefaultRunLoopMode
            dequeue:YES]) != nil) {
            [NSApp sendEvent:event];
        }
        [NSApp updateWindows];
    }
}

void fievel_ui_locate_cursor(void) {
    @autoreleasepool {
        fievel_ui_ensure();
        cursor_pulse_position = NSEvent.mouseLocation;
        cursor_pulse_until = CFAbsoluteTimeGetCurrent() + 0.75;
        [overlay_window orderFrontRegardless];
        [overlay_view setNeedsDisplay:YES];
    }
}

bool fievel_request_input_permissions(void) {
    if (!CGPreflightListenEventAccess()) {
        CGRequestListenEventAccess();
    }
    if (!CGPreflightPostEventAccess()) {
        CGRequestPostEventAccess();
    }
    return CGPreflightListenEventAccess() && CGPreflightPostEventAccess();
}

bool fievel_request_screen_capture_permission(void) {
    if (!CGPreflightScreenCaptureAccess()) {
        CGRequestScreenCaptureAccess();
    }
    return CGPreflightScreenCaptureAccess();
}

bool fievel_capture_screens(FievelCapture **captures_out, size_t *count_out) {
    @autoreleasepool {
    if (captures_out == NULL || count_out == NULL) {
        return false;
    }
    uint32_t display_count = 0;
    if (CGGetActiveDisplayList(0, NULL, &display_count) != kCGErrorSuccess || display_count == 0) {
        return false;
    }
    CGDirectDisplayID *display_ids = calloc(display_count, sizeof(CGDirectDisplayID));
    FievelCapture *captures = calloc(display_count, sizeof(FievelCapture));
    if (display_ids == NULL || captures == NULL) {
        free(display_ids);
        free(captures);
        return false;
    }
    if (CGGetActiveDisplayList(display_count, display_ids, &display_count) != kCGErrorSuccess) {
        free(display_ids);
        free(captures);
        return false;
    }
    size_t captured = 0;
    __block SCShareableContent *shareable_content = nil;
    __block NSError *content_error = nil;
    dispatch_semaphore_t content_ready = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
        onScreenWindowsOnly:YES
        completionHandler:^(SCShareableContent *content, NSError *error) {
            shareable_content = content;
            content_error = error;
            dispatch_semaphore_signal(content_ready);
        }];
    if (dispatch_semaphore_wait(
            content_ready,
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(10 * NSEC_PER_SEC))) != 0 ||
        shareable_content == nil || content_error != nil) {
        free(display_ids);
        free(captures);
        return false;
    }
    NSMutableArray<SCWindow *> *own_windows = [NSMutableArray array];
    pid_t process_id = NSRunningApplication.currentApplication.processIdentifier;
    for (SCWindow *window in shareable_content.windows) {
        if (window.owningApplication.processID == process_id) {
            [own_windows addObject:window];
        }
    }
    for (uint32_t i = 0; i < display_count; i++) {
        SCDisplay *screen = nil;
        for (SCDisplay *candidate in shareable_content.displays) {
            if (candidate.displayID == display_ids[i]) {
                screen = candidate;
                break;
            }
        }
        if (screen == nil) {
            continue;
        }
        SCContentFilter *filter =
            [[SCContentFilter alloc] initWithDisplay:screen excludingWindows:own_windows];
        SCStreamConfiguration *configuration = [[SCStreamConfiguration alloc] init];
        configuration.width = CGDisplayPixelsWide(display_ids[i]);
        configuration.height = CGDisplayPixelsHigh(display_ids[i]);
        configuration.showsCursor = YES;
        __block CGImageRef image = NULL;
        __block NSError *capture_error = nil;
        dispatch_semaphore_t capture_ready = dispatch_semaphore_create(0);
        [SCScreenshotManager captureImageWithFilter:filter
            configuration:configuration
            completionHandler:^(CGImageRef result, NSError *error) {
                image = result == NULL ? NULL : CGImageRetain(result);
                capture_error = error;
                dispatch_semaphore_signal(capture_ready);
            }];
        if (dispatch_semaphore_wait(
                capture_ready,
                dispatch_time(DISPATCH_TIME_NOW, (int64_t)(10 * NSEC_PER_SEC))) != 0 ||
            image == NULL || capture_error != nil) {
            if (image != NULL) CGImageRelease(image);
            continue;
        }
        size_t width = CGImageGetWidth(image);
        size_t height = CGImageGetHeight(image);
        if (width == 0 || height == 0 || width > UINT32_MAX / 4 || height > SIZE_MAX / (width * 4)) {
            CGImageRelease(image);
            continue;
        }
        uint8_t *pixels = calloc(width * height, 4);
        CGColorSpaceRef color_space = CGColorSpaceCreateDeviceRGB();
        CGContextRef context = pixels == NULL || color_space == NULL ? NULL : CGBitmapContextCreate(
            pixels, width, height, 8, width * 4, color_space,
            kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big
        );
        if (pixels == NULL || color_space == NULL || context == NULL) {
            free(pixels);
            if (context != NULL) CGContextRelease(context);
            if (color_space != NULL) CGColorSpaceRelease(color_space);
            CGImageRelease(image);
            continue;
        }
        CGContextDrawImage(context, CGRectMake(0, 0, width, height), image);
        CGRect bounds = CGDisplayBounds(display_ids[i]);
        captures[captured++] = (FievelCapture) {
            .display_id = display_ids[i],
            .x = bounds.origin.x,
            .y = bounds.origin.y,
            .width = bounds.size.width,
            .height = bounds.size.height,
            .pixel_width = (uint32_t)width,
            .pixel_height = (uint32_t)height,
            .stride = (uint32_t)(width * 4),
            .rgba = pixels,
        };
        CGContextRelease(context);
        CGColorSpaceRelease(color_space);
        CGImageRelease(image);
    }
    free(display_ids);
    if (captured == 0) {
        free(captures);
        return false;
    }
    *captures_out = captures;
    *count_out = captured;
    return true;
    }
}

void fievel_release_captures(FievelCapture *captures, size_t count) {
    if (captures == NULL) {
        return;
    }
    for (size_t i = 0; i < count; i++) {
        free(captures[i].rgba);
    }
    free(captures);
}

static bool fievel_queue_event(FievelQueuedEvent *queued) {
    FievelQueuedEvent *item = malloc(sizeof(FievelQueuedEvent));
    if (item == NULL) {
        return false;
    }
    *item = *queued;
    item->next = NULL;
    pthread_mutex_lock(&event_lock);
    if (event_tail == NULL) {
        event_head = item;
    } else {
        event_tail->next = item;
    }
    event_tail = item;
    pthread_cond_signal(&event_ready);
    pthread_mutex_unlock(&event_lock);
    return true;
}

static int fievel_modifier_index(uint16_t keycode) {
    switch (keycode) {
        case 56: return 0;
        case 60: return 1;
        case 59: return 2;
        case 62: return 3;
        case 58: return 4;
        case 61: return 5;
        case 55: return 6;
        case 54: return 7;
        case 57: return 8;
        default: return -1;
    }
}

static CGEventRef fievel_event_callback(
    CGEventTapProxy proxy,
    CGEventType type,
    CGEventRef event,
    void *user_info
) {
    (void)proxy;
    (void)user_info;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        if (event_tap != NULL) {
            CGEventTapEnable(event_tap, true);
        }
        return event;
    }
    if (CGEventGetIntegerValueField(event, kCGEventSourceUserData) == event_marker) {
        return event;
    }
    if (type == kCGEventKeyDown || type == kCGEventKeyUp || type == kCGEventFlagsChanged) {
        uint16_t keycode = (uint16_t)CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        int value = type == kCGEventKeyUp ? 0 : 1;
        if (type == kCGEventFlagsChanged) {
            int index = fievel_modifier_index(keycode);
            if (index < 0) {
                return event;
            }
            uint64_t bit = UINT64_C(1) << index;
            value = (modifier_keys_held & bit) == 0 ? 1 : 0;
            modifier_keys_held ^= bit;
        }
        FievelQueuedEvent queued = {
            .kind = 1,
            .keycode = keycode,
            .value = value,
            .repeat = type == kCGEventKeyDown &&
                CGEventGetIntegerValueField(event, kCGKeyboardEventAutorepeat) != 0,
            .original = (CGEventRef)CFRetain(event),
        };
        if (fievel_queue_event(&queued)) {
            return NULL;
        }
        CFRelease(queued.original);
        return event;
    }
    if (type == kCGEventMouseMoved || type == kCGEventLeftMouseDragged ||
        type == kCGEventRightMouseDragged || type == kCGEventOtherMouseDragged) {
        CGPoint location = CGEventGetLocation(event);
        pthread_mutex_lock(&pointer_lock);
        double dx = isnan(last_pointer.x) ? 0 : location.x - last_pointer.x;
        double dy = isnan(last_pointer.y) ? 0 : location.y - last_pointer.y;
        last_pointer = location;
        pthread_mutex_unlock(&pointer_lock);
        FievelQueuedEvent queued = { .kind = 2, .dx = dx, .dy = dy };
        fievel_queue_event(&queued);
    }
    return event;
}

static void *fievel_event_thread_main(void *unused) {
    (void)unused;
    @autoreleasepool {
        CFRunLoopSourceRef source = event_source;
        if (source == NULL) {
            pthread_mutex_lock(&event_lock);
            event_thread_failed = true;
            event_thread_ready = true;
            pthread_cond_broadcast(&event_ready);
            pthread_mutex_unlock(&event_lock);
            return NULL;
        }
        CFRunLoopRef loop = CFRunLoopGetCurrent();
        CFRetain(loop);
        CFRunLoopAddSource(loop, source, kCFRunLoopCommonModes);
        pthread_mutex_lock(&event_lock);
        event_thread_ready = true;
        pthread_cond_broadcast(&event_ready);
        pthread_mutex_unlock(&event_lock);
        CFRunLoopRun();
        CFRelease(loop);
    }
    return NULL;
}

bool fievel_input_start(void) {
    if (!fievel_request_input_permissions()) {
        return false;
    }
    if (!exit_cleanup_registered) {
        atexit(fievel_release_posted_inputs);
        exit_cleanup_registered = true;
    }
    if (event_tap != NULL) {
        return true;
    }
    CGEventMask mask =
        CGEventMaskBit(kCGEventKeyDown) |
        CGEventMaskBit(kCGEventKeyUp) |
        CGEventMaskBit(kCGEventFlagsChanged) |
        CGEventMaskBit(kCGEventMouseMoved) |
        CGEventMaskBit(kCGEventLeftMouseDragged) |
        CGEventMaskBit(kCGEventRightMouseDragged) |
        CGEventMaskBit(kCGEventOtherMouseDragged);
    event_tap = CGEventTapCreate(
        kCGSessionEventTap,
        kCGHeadInsertEventTap,
        kCGEventTapOptionDefault,
        mask,
        fievel_event_callback,
        NULL
    );
    if (event_tap == NULL) {
        return false;
    }
    event_source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, event_tap, 0);
    if (event_source == NULL) {
        CFRelease(event_tap);
        event_tap = NULL;
        return false;
    }
    if (pthread_create(&event_thread, NULL, fievel_event_thread_main, NULL) != 0) {
        CFRelease(event_source);
        CFRelease(event_tap);
        event_source = NULL;
        event_tap = NULL;
        return false;
    }
    pthread_mutex_lock(&event_lock);
    while (!event_thread_ready) {
        pthread_cond_wait(&event_ready, &event_lock);
    }
    bool started = !event_thread_failed;
    pthread_mutex_unlock(&event_lock);
    if (!started) {
        return false;
    }
    CGEventTapEnable(event_tap, true);
    return true;
}

bool fievel_input_next(FievelInputEvent *out, uint32_t wait_millis) {
    if (out == NULL) {
        return false;
    }
    pthread_mutex_lock(&event_lock);
    if (event_head == NULL && wait_millis > 0) {
        struct timespec deadline;
        clock_gettime(CLOCK_REALTIME, &deadline);
        deadline.tv_sec += wait_millis / 1000;
        deadline.tv_nsec += (long)(wait_millis % 1000) * 1000000L;
        if (deadline.tv_nsec >= 1000000000L) {
            deadline.tv_sec += 1;
            deadline.tv_nsec -= 1000000000L;
        }
        while (event_head == NULL) {
            int result = pthread_cond_timedwait(&event_ready, &event_lock, &deadline);
            if (result == ETIMEDOUT) {
                break;
            }
        }
    }
    FievelQueuedEvent *item = event_head;
    if (item != NULL) {
        event_head = item->next;
        if (event_head == NULL) {
            event_tail = NULL;
        }
    }
    pthread_mutex_unlock(&event_lock);
    if (item == NULL) {
        return false;
    }
    *out = (FievelInputEvent) {
        .kind = item->kind,
        .keycode = item->keycode,
        .value = item->value,
        .repeat = item->repeat,
        .dx = item->dx,
        .dy = item->dy,
        .original = item->original,
    };
    free(item);
    return true;
}

void fievel_input_release(FievelInputEvent *event) {
    if (event != NULL && event->original != NULL) {
        CFRelease(event->original);
        event->original = NULL;
    }
}

void fievel_input_repost(FievelInputEvent *event) {
    if (event != NULL && event->original != NULL) {
        CGEventPost(kCGHIDEventTap, event->original);
        CFRelease(event->original);
        event->original = NULL;
    }
}

static CGEventSourceRef fievel_event_source(void) {
    return CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
}

static void fievel_release_posted_inputs(void) {
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return;
    }
    for (uint16_t keycode = 0; keycode < 256; keycode++) {
        if (!posted_keys[keycode]) {
            continue;
        }
        CGEventRef event = CGEventCreateKeyboardEvent(source, keycode, false);
        if (event != NULL) {
            CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
            CGEventPost(kCGHIDEventTap, event);
            CFRelease(event);
        }
    }
    CGPoint point = CGPointZero;
    CGEventRef current = CGEventCreate(source);
    if (current != NULL) {
        point = CGEventGetLocation(current);
        CFRelease(current);
    }
    for (int button = 0; button < 2; button++) {
        if (!posted_buttons[button]) {
            continue;
        }
        CGMouseButton mouse_button = button == 0 ? kCGMouseButtonLeft : kCGMouseButtonRight;
        CGEventType type = button == 0 ? kCGEventLeftMouseUp : kCGEventRightMouseUp;
        CGEventRef event = CGEventCreateMouseEvent(source, type, point, mouse_button);
        if (event != NULL) {
            CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
            CGEventPost(kCGHIDEventTap, event);
            CFRelease(event);
        }
    }
    CFRelease(source);
}

bool fievel_send_key(uint16_t keycode, int value) {
    if (keycode >= 256) {
        return false;
    }
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return false;
    }
    bool down = value != 0;
    if (value != 2) {
        posted_keys[keycode] = down;
    }
    CGEventRef event = CGEventCreateKeyboardEvent(source, keycode, down);
    if (event != NULL) {
        CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
        CGEventSetIntegerValueField(event, kCGKeyboardEventAutorepeat, value == 2 ? 1 : 0);
        CGEventPost(kCGHIDEventTap, event);
        CFRelease(event);
    }
    CFRelease(source);
    return event != NULL;
}

bool fievel_move_pointer(double dx, double dy) {
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return false;
    }
    CGEventRef current = CGEventCreate(source);
    CGPoint point = current == NULL ? CGPointZero : CGEventGetLocation(current);
    if (current != NULL) CFRelease(current);
    point.x += dx;
    point.y += dy;
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) == kCGErrorSuccess && count > 0) {
        CGDirectDisplayID *ids = calloc(count, sizeof(CGDirectDisplayID));
        if (ids != NULL && CGGetActiveDisplayList(count, ids, &count) == kCGErrorSuccess) {
            double min_x = INFINITY, min_y = INFINITY, max_x = -INFINITY, max_y = -INFINITY;
            for (uint32_t i = 0; i < count; i++) {
                CGRect bounds = CGDisplayBounds(ids[i]);
                min_x = fmin(min_x, CGRectGetMinX(bounds));
                min_y = fmin(min_y, CGRectGetMinY(bounds));
                max_x = fmax(max_x, CGRectGetMaxX(bounds));
                max_y = fmax(max_y, CGRectGetMaxY(bounds));
            }
            point.x = fmin(fmax(point.x, min_x), max_x - 1);
            point.y = fmin(fmax(point.y, min_y), max_y - 1);
        }
        free(ids);
    }
    CGEventRef event = CGEventCreateMouseEvent(source, kCGEventMouseMoved, point, kCGMouseButtonLeft);
    if (event != NULL) {
        CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, event);
        CFRelease(event);
    }
    pthread_mutex_lock(&pointer_lock);
    last_pointer = point;
    pthread_mutex_unlock(&pointer_lock);
    CFRelease(source);
    return event != NULL;
}

bool fievel_mouse_button(bool right, bool down) {
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return false;
    }
    CGEventRef current = CGEventCreate(source);
    CGPoint point = current == NULL ? CGPointZero : CGEventGetLocation(current);
    if (current != NULL) CFRelease(current);
    CGMouseButton button = right ? kCGMouseButtonRight : kCGMouseButtonLeft;
    posted_buttons[right ? 1 : 0] = down;
    CGEventType type = right
        ? (down ? kCGEventRightMouseDown : kCGEventRightMouseUp)
        : (down ? kCGEventLeftMouseDown : kCGEventLeftMouseUp);
    CGEventRef event = CGEventCreateMouseEvent(source, type, point, button);
    if (event != NULL) {
        CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, event);
        CFRelease(event);
    }
    CFRelease(source);
    return event != NULL;
}

bool fievel_scroll(int32_t vertical, int32_t horizontal) {
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return false;
    }
    CGEventRef event = CGEventCreateScrollWheelEvent(
        source, kCGScrollEventUnitLine, 2, vertical, horizontal
    );
    if (event != NULL) {
        CGEventSetIntegerValueField(event, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, event);
        CFRelease(event);
    }
    CFRelease(source);
    return event != NULL;
}

double fievel_main_display_height(void) {
    return CGDisplayBounds(CGMainDisplayID()).size.height;
}

bool fievel_click_at(double x, double y, bool right) {
    CGEventSourceRef source = fievel_event_source();
    if (source == NULL) {
        return false;
    }
    CGPoint point = CGPointMake(x, y);
    CGMouseButton button = right ? kCGMouseButtonRight : kCGMouseButtonLeft;
    CGEventType move_type = kCGEventMouseMoved;
    CGEventRef move = CGEventCreateMouseEvent(source, move_type, point, kCGMouseButtonLeft);
    if (move != NULL) {
        CGEventSetIntegerValueField(move, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, move);
        CFRelease(move);
    }
    CGEventType down_type = right ? kCGEventRightMouseDown : kCGEventLeftMouseDown;
    CGEventType up_type = right ? kCGEventRightMouseUp : kCGEventLeftMouseUp;
    CGEventRef down = CGEventCreateMouseEvent(source, down_type, point, button);
    CGEventRef up = CGEventCreateMouseEvent(source, up_type, point, button);
    if (down != NULL) {
        CGEventSetIntegerValueField(down, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, down);
        CFRelease(down);
    }
    if (up != NULL) {
        CGEventSetIntegerValueField(up, kCGEventSourceUserData, event_marker);
        CGEventPost(kCGHIDEventTap, up);
        CFRelease(up);
    }
    pthread_mutex_lock(&pointer_lock);
    last_pointer = point;
    pthread_mutex_unlock(&pointer_lock);
    CFRelease(source);
    return move != NULL && down != NULL && up != NULL;
}
