/* Cocoa window. The Rust `Window` API is unchanged.
 * `display` and `surface` are both the `CAMetalLayer`.
 * The renderer creates the VkSurface with VK_EXT_metal_surface.
 * Keys in `keys_down` are Linux evdev codes, the same index the Wayland seat writes.
 * This file is manual reference counting. Do not compile it with ARC. */

#import <Cocoa/Cocoa.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "genos_pump.h"
#include "touch_log.h"

struct GenosCocoa {
    NSWindow *window;
    NSView *view;
    CAMetalLayer *layer;
    id delegate;
    int width;
    int height;
    double scale;
    int closing;
    int focused;
    int last_x;
    int last_y;
    int have_pointer;
    int mouse_dx;
    int mouse_dy;
    int capture_click;
    int key_w;
    int key_a;
    int key_s;
    int key_d;
    int key_escape;
    int resized;
    int mouse_left;
    int mouse_right;
    int mouse_middle;
    struct GenosTouchLog touch;
    uint8_t keys_down[256];
    int pointer_locked;
    int clicked_while_focused;
    int fullscreen;
    double wheel_y;
    int cursor_hidden;
};

static void release_capture(struct GenosCocoa *win);
static int engage_capture(struct GenosCocoa *win);

static int evdev_from_mac(unsigned short code) {
    switch (code) {
    case 0: return 30;   /* A */
    case 1: return 31;   /* S */
    case 2: return 32;   /* D */
    case 3: return 33;   /* F */
    case 5: return 34;   /* G */
    case 4: return 35;   /* H */
    case 38: return 36;  /* J */
    case 40: return 37;  /* K */
    case 37: return 38;  /* L */
    case 6: return 44;   /* Z */
    case 7: return 45;   /* X */
    case 8: return 46;   /* C */
    case 9: return 47;   /* V */
    case 11: return 48;  /* B */
    case 45: return 49;  /* N */
    case 46: return 50;  /* M */
    case 12: return 16;  /* Q */
    case 13: return 17;  /* W */
    case 14: return 18;  /* E */
    case 15: return 19;  /* R */
    case 17: return 20;  /* T */
    case 16: return 21;  /* Y */
    case 32: return 22;  /* U */
    case 34: return 23;  /* I */
    case 31: return 24;  /* O */
    case 35: return 25;  /* P */
    case 18: return 2;   /* 1 */
    case 19: return 3;
    case 20: return 4;
    case 21: return 5;
    case 23: return 6;
    case 22: return 7;
    case 26: return 8;
    case 28: return 9;
    case 25: return 10;
    case 29: return 11;  /* 0 */
    case 53: return 1;   /* escape */
    case 33: return 26;  /* [ */
    case 30: return 27;  /* ] */
    case 50: return 41;  /* grave */
    case 51: return 14;  /* delete */
    case 48: return 15;  /* tab */
    case 36: return 28;  /* return */
    case 49: return 57;  /* space */
    case 56: return 42;  /* shift */
    case 60: return 54;  /* right shift */
    case 59: return 29;  /* control */
    case 62: return 97;  /* right control */
    case 58: return 56;  /* option */
    case 61: return 100; /* right option */
    case 126: return 103;
    case 125: return 108;
    case 123: return 105;
    case 124: return 106;
    case 122: return 59; /* F1 */
    case 120: return 60;
    case 99: return 61;
    case 118: return 62;
    case 96: return 63;
    case 97: return 64;
    case 98: return 65;
    case 100: return 66;
    case 101: return 67;
    case 109: return 68;
    case 103: return 87;
    case 111: return 88; /* F12 */
    default: return -1;
    }
}

extern int genos_modifier_down(unsigned key_code, unsigned long long flags);

static void set_evdev(struct GenosCocoa *win, int evdev, int down) {
    if (!win || evdev < 0 || evdev > 255) {
        return;
    }
    win->keys_down[evdev] = (uint8_t)(down ? 1 : 0);
    if (evdev == 17) win->key_w = down;
    if (evdev == 30) win->key_a = down;
    if (evdev == 31) win->key_s = down;
    if (evdev == 32) win->key_d = down;
    if (evdev == 1) {
        win->key_escape = down;
        if (down) {
            release_capture(win);
        }
    }
}

static void clear_keys(struct GenosCocoa *win) {
    memset(win->keys_down, 0, sizeof(win->keys_down));
    win->key_w = 0;
    win->key_a = 0;
    win->key_s = 0;
    win->key_d = 0;
    win->key_escape = 0;
    win->mouse_left = 0;
    win->mouse_right = 0;
    win->mouse_middle = 0;
}

static void push_touch(struct GenosCocoa *win, uint64_t id, float x, float y, int phase) {
    genos_touch_log_push(&win->touch, id, x, y, phase);
}

static void cancel_touch(struct GenosCocoa *win) {
    genos_touch_log_cancel(&win->touch);
}

static double view_scale(struct GenosCocoa *win) {
    double scale = 1.0;
    if (win && win->window) {
        scale = win->window.backingScaleFactor;
    }
    if (scale < 1.0) {
        scale = 1.0;
    }
    win->scale = scale;
    return scale;
}

static void note_pixels(struct GenosCocoa *win) {
    NSRect bounds;
    double scale;
    int width;
    int height;
    if (!win || !win->view) {
        return;
    }
    scale = view_scale(win);
    bounds = win->view.bounds;
    width = (int)llround(bounds.size.width * scale);
    height = (int)llround(bounds.size.height * scale);
    if (width < 1) width = 1;
    if (height < 1) height = 1;
    if (width != win->width || height != win->height) {
        win->width = width;
        win->height = height;
        win->resized = 1;
    }
    if (win->layer) {
        win->layer.contentsScale = scale;
        win->layer.drawableSize = CGSizeMake(width, height);
    }
}

static void release_capture(struct GenosCocoa *win) {
    if (!win || !win->pointer_locked) {
        return;
    }
    CGAssociateMouseAndMouseCursorPosition(true);
    if (win->cursor_hidden) {
        CGDisplayShowCursor(kCGDirectMainDisplay);
        win->cursor_hidden = 0;
    }
    win->pointer_locked = 0;
}

static int engage_capture(struct GenosCocoa *win) {
    CGError err;
    if (!win) {
        return -1;
    }
    err = CGAssociateMouseAndMouseCursorPosition(false);
    if (err != kCGErrorSuccess) {
        fprintf(stderr, "genos-window: pointer capture failed %d\n", (int)err);
        return -1;
    }
    if (!win->cursor_hidden) {
        CGDisplayHideCursor(kCGDirectMainDisplay);
        win->cursor_hidden = 1;
    }
    win->pointer_locked = 1;
    return 0;
}

static void note_move(struct GenosCocoa *win, int x, int y) {
    if (!win->pointer_locked && win->have_pointer) {
        win->mouse_dx += x - win->last_x;
        win->mouse_dy += y - win->last_y;
    }
    win->last_x = x;
    win->last_y = y;
    win->have_pointer = 1;
}

static void note_touches(struct GenosCocoa *win, NSEvent *event) {
    NSSet<NSTouch *> *touches;
    NSRect bounds;
    double scale;
    if (!win || !event) {
        return;
    }
    touches = [event touchesMatchingPhase:NSTouchPhaseAny inView:nil];
    if (touches.count == 0) {
        return;
    }
    scale = view_scale(win);
    bounds = win->view ? win->view.bounds : NSMakeRect(0, 0, 1, 1);
    for (NSTouch *touch in touches) {
        NSPoint point = touch.normalizedPosition;
        float x = (float)(point.x * bounds.size.width * scale);
        float y = (float)((1.0 - point.y) * bounds.size.height * scale);
        uint64_t ident = (uint64_t)touch.identity.hash;
        int phase = GENOS_TOUCH_MOVED;
        switch (touch.phase) {
        case NSTouchPhaseBegan: phase = GENOS_TOUCH_BEGAN; break;
        case NSTouchPhaseMoved:
        case NSTouchPhaseStationary: phase = GENOS_TOUCH_MOVED; break;
        case NSTouchPhaseEnded: phase = GENOS_TOUCH_ENDED; break;
        case NSTouchPhaseCancelled: phase = GENOS_TOUCH_CANCELLED; break;
        default: break;
        }
        push_touch(win, ident, x, y, phase);
    }
}

static void handle_event(struct GenosCocoa *win, NSEvent *event) {
    int evdev;
    int down;
    NSPoint point;
    double scale;
    if (!win || !event || event.window != win->window) {
        return;
    }
    switch (event.type) {
    case NSEventTypeKeyDown:
        set_evdev(win, evdev_from_mac(event.keyCode), 1);
        break;
    case NSEventTypeKeyUp:
        set_evdev(win, evdev_from_mac(event.keyCode), 0);
        break;
    case NSEventTypeFlagsChanged:
        evdev = evdev_from_mac(event.keyCode);
        down = genos_modifier_down((unsigned)event.keyCode, (unsigned long long)event.modifierFlags);
        if (down >= 0) {
            set_evdev(win, evdev, down);
        }
        break;
    case NSEventTypeLeftMouseDown:
        win->mouse_left = 1;
        win->capture_click = 1;
        win->clicked_while_focused = 1;
        break;
    case NSEventTypeLeftMouseUp:
        win->mouse_left = 0;
        break;
    case NSEventTypeRightMouseDown:
        win->mouse_right = 1;
        break;
    case NSEventTypeRightMouseUp:
        win->mouse_right = 0;
        break;
    case NSEventTypeOtherMouseDown:
        if (event.buttonNumber == 2) {
            win->mouse_middle = 1;
        }
        break;
    case NSEventTypeOtherMouseUp:
        if (event.buttonNumber == 2) {
            win->mouse_middle = 0;
        }
        break;
    case NSEventTypeMouseMoved:
    case NSEventTypeLeftMouseDragged:
    case NSEventTypeRightMouseDragged:
    case NSEventTypeOtherMouseDragged:
        if (win->pointer_locked) {
            scale = view_scale(win);
            /* Cocoa deltaY is positive upward. The pump's Y is positive downward. */
            win->mouse_dx += (int)llround(event.deltaX * scale);
            win->mouse_dy += (int)llround(-event.deltaY * scale);
        } else {
            point = event.locationInWindow;
            scale = view_scale(win);
            note_move(
                win,
                (int)llround(point.x * scale),
                (int)llround((win->view.bounds.size.height - point.y) * scale)
            );
        }
        break;
    case NSEventTypeGesture:
    case NSEventTypeMagnify:
    case NSEventTypeSwipe:
    case NSEventTypeRotate:
    case NSEventTypeBeginGesture:
    case NSEventTypeEndGesture:
    case NSEventTypeSmartMagnify:
    case NSEventTypePressure:
    case NSEventTypeDirectTouch:
        note_touches(win, event);
        break;
    case NSEventTypeScrollWheel:
        if (event.hasPreciseScrollingDeltas) {
            win->wheel_y += event.scrollingDeltaY / 10.0;
        } else {
            win->wheel_y += event.scrollingDeltaY;
        }
        break;
    default:
        break;
    }
}

@interface GenosDelegate : NSObject <NSWindowDelegate>
{
    struct GenosCocoa *win;
}
- (id)initWithWindow:(struct GenosCocoa *)window;
@end

@implementation GenosDelegate
- (id)initWithWindow:(struct GenosCocoa *)window {
    self = [super init];
    if (self) {
        win = window;
    }
    return self;
}
- (BOOL)windowShouldClose:(id)sender {
    (void)sender;
    /* Keep the window. The pump reports `closing` and the frame decides. */
    if (win) {
        win->closing = 1;
    }
    return NO;
}
- (void)windowDidResize:(NSNotification *)note {
    (void)note;
    note_pixels(win);
}
- (void)windowDidBecomeKey:(NSNotification *)note {
    (void)note;
    if (win) {
        win->focused = 1;
    }
}
- (void)windowDidResignKey:(NSNotification *)note {
    (void)note;
    if (win) {
        win->focused = 0;
        clear_keys(win);
        cancel_touch(win);
        release_capture(win);
    }
}
- (void)windowDidEnterFullScreen:(NSNotification *)note {
    (void)note;
    if (win) {
        win->fullscreen = 1;
        note_pixels(win);
    }
}
- (void)windowDidExitFullScreen:(NSNotification *)note {
    (void)note;
    if (win) {
        win->fullscreen = 0;
        note_pixels(win);
    }
}
@end

static void drain(struct GenosCocoa *win) {
    NSEvent *event;
    if (!win || ![NSThread isMainThread]) {
        return;
    }
    while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                       untilDate:[NSDate distantPast]
                                          inMode:NSDefaultRunLoopMode
                                         dequeue:YES])) {
        handle_event(win, event);
        [NSApp sendEvent:event];
    }
    [NSApp updateWindows];
}

static void copy_out(struct GenosCocoa *win, struct GenosPump *out) {
    memset(out, 0, sizeof(*out));
    out->width = win->width > 0 ? win->width : 1;
    out->height = win->height > 0 ? win->height : 1;
    out->closing = win->closing;
    out->focused = win->focused;
    out->mouse_dx = win->mouse_dx;
    out->mouse_dy = win->mouse_dy;
    out->capture_click = win->capture_click;
    out->key_w = win->key_w;
    out->key_a = win->key_a;
    out->key_s = win->key_s;
    out->key_d = win->key_d;
    out->key_escape = win->key_escape;
    out->resized = win->resized;
    out->mouse_left = win->mouse_left;
    memcpy(out->keys_down, win->keys_down, sizeof(out->keys_down));
    out->pointer_locked = win->pointer_locked;
    out->clicked_while_focused = win->clicked_while_focused;
    out->fullscreen = win->fullscreen;
    out->pointer_x = win->last_x;
    out->pointer_y = win->last_y;
    out->display = (void *)win->layer;
    out->surface = (void *)win->layer;
    out->wheel_y = win->wheel_y;
    out->minimized = 0;
    win->wheel_y = 0;
    win->mouse_dx = 0;
    win->mouse_dy = 0;
    win->capture_click = 0;
    win->clicked_while_focused = 0;
    win->resized = 0;
}

int genos_window_backend_value = GENOS_BACKEND_METAL;

int genos_window_backend(void) {
    return genos_window_backend_value;
}

struct GenosCocoa *genos_window_open_on(
    int width,
    int height,
    const char *app_id,
    const char *title,
    const char *display_name
) {
    struct GenosCocoa *win;
    NSRect content;
    NSWindowStyleMask style;
    NSString *title_text;
    NSString *ident;
    double scale;
    (void)display_name;
    if (![NSThread isMainThread]) {
        fprintf(stderr, "genos-window: Cocoa open is not on the main thread\n");
        return NULL;
    }
    if (width < 1) width = 1;
    if (height < 1) height = 1;
    @autoreleasepool {
        win = (struct GenosCocoa *)calloc(1, sizeof(*win));
        if (!win) {
            return NULL;
        }
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        [NSApp finishLaunching];
        scale = NSScreen.mainScreen.backingScaleFactor;
        if (scale < 1.0) {
            scale = 1.0;
        }
        win->scale = scale;
        content = NSMakeRect(0, 0, (CGFloat)width / (CGFloat)scale, (CGFloat)height / (CGFloat)scale);
        style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable
            | NSWindowStyleMaskResizable;
        win->window = [[NSWindow alloc] initWithContentRect:content
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
        if (!win->window) {
            fprintf(stderr, "genos-window: CreateWindow failed\n");
            free(win);
            return NULL;
        }
        win->window.releasedWhenClosed = NO;
        win->window.collectionBehavior = NSWindowCollectionBehaviorFullScreenPrimary;
        win->window.acceptsMouseMovedEvents = YES;
        title_text = title ? [NSString stringWithUTF8String:title] : nil;
        if (!title_text) {
            title_text = @"Genos";
        }
        win->window.title = title_text;
        ident = app_id ? [NSString stringWithUTF8String:app_id] : nil;
        if (ident) {
            win->window.identifier = ident;
        }
        win->view = [[NSView alloc] initWithFrame:content];
        win->view.wantsLayer = YES;
        if ([win->view respondsToSelector:@selector(setAllowedTouchTypes:)]) {
            win->view.allowedTouchTypes = NSTouchTypeMaskDirect | NSTouchTypeMaskIndirect;
        }
        win->layer = [CAMetalLayer layer];
        [win->layer retain];
        win->layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
        win->layer.framebufferOnly = YES;
        win->view.layer = win->layer;
        win->window.contentView = win->view;
        [win->view release];
        win->delegate = [[GenosDelegate alloc] initWithWindow:win];
        win->window.delegate = win->delegate;
        note_pixels(win);
        [win->window makeKeyAndOrderFront:nil];
        if ([NSApp respondsToSelector:@selector(activate)]) {
            [NSApp performSelector:@selector(activate)];
        }
        win->focused = win->window.keyWindow ? 1 : 0;
        fprintf(
            stderr,
            "genos-window: backend=metal scale=%.2f client=%dx%d\n",
            win->scale,
            win->width,
            win->height
        );
        return win;
    }
}

struct GenosCocoa *genos_window_open(int width, int height, const char *app_id, const char *title) {
    return genos_window_open_on(width, height, app_id, title, NULL);
}

void genos_window_pump(struct GenosCocoa *win, struct GenosPump *out) {
    if (!win || !out) {
        return;
    }
    @autoreleasepool {
        drain(win);
        note_pixels(win);
        copy_out(win, out);
    }
}

int genos_window_pump_size(void) {
    return (int)sizeof(struct GenosPump);
}

int genos_window_extra_size(void) {
    return (int)sizeof(struct GenosExtra);
}

int genos_window_copy_extra(struct GenosCocoa *win, struct GenosExtra *out) {
    if (!out) {
        return -1;
    }
    memset(out, 0, sizeof(*out));
    if (!win) {
        return -1;
    }
    out->mouse_right = win->mouse_right;
    out->mouse_middle = win->mouse_middle;
    out->touch_count = genos_touch_log_copy(&win->touch, out->touch);
    return 0;
}

void genos_window_dispatch(struct GenosCocoa *win) {
    (void)win;
}

int genos_window_set_title(struct GenosCocoa *win, const char *title) {
    NSString *text;
    if (!win || !win->window) {
        return -1;
    }
    text = title ? [NSString stringWithUTF8String:title] : nil;
    if (!text) {
        return -1;
    }
    win->window.title = text;
    return 0;
}

int genos_window_set_fullscreen(struct GenosCocoa *win, int fullscreen) {
    BOOL is_full;
    if (!win || !win->window) {
        return -1;
    }
    is_full = (win->window.styleMask & NSWindowStyleMaskFullScreen) != 0;
    if ((fullscreen != 0) != (is_full != 0)) {
        [win->window toggleFullScreen:nil];
    }
    win->fullscreen = fullscreen ? 1 : 0;
    return 0;
}

int genos_window_set_pointer_capture(struct GenosCocoa *win, int capture) {
    if (!win) {
        return -1;
    }
    if (!capture) {
        release_capture(win);
        return 0;
    }
    return engage_capture(win);
}

void genos_window_destroy(struct GenosCocoa *win) {
    if (!win) {
        return;
    }
    release_capture(win);
    if (win->window) {
        win->window.delegate = nil;
        [win->window orderOut:nil];
    }
    if (win->delegate) {
        [win->delegate release];
    }
    if (win->layer) {
        [win->layer release];
    }
    if (win->window) {
        [win->window release];
    }
    free(win);
}
