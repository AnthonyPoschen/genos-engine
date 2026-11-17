/* GameController pads. The Rust side writes them with the public setters.
 * A controller keeps one slot until it disconnects. The array order does not
 * move the others. Stick Y is already positive upward.
 * The name is copied once, on connect. A later poll does not build a string. */

#import <GameController/GameController.h>

#include <pthread.h>
#include <stdint.h>
#include <string.h>

enum { GENOS_PAD_SLOTS = 16 };

struct GenosPadSample {
    int connected;
    char name[128];
    float lx;
    float ly;
    float rx;
    float ry;
    float lt;
    float rt;
    /* south, east, west, north, d-pad, shoulders, select, start, home, stick clicks. */
    int buttons[15];
};

struct GenosPadSlot {
    GCController *controller;
    char name[128];
    int named;
};

static struct GenosPadSlot slots[GENOS_PAD_SLOTS];
static pthread_mutex_t slots_lock = PTHREAD_MUTEX_INITIALIZER;
static id connect_obs;
static id disconnect_obs;
static int started;

static void copy_name(char *dst, NSString *text) {
    const char *utf8;
    if (!text || text.length == 0) {
        return;
    }
    utf8 = text.UTF8String;
    if (!utf8) {
        return;
    }
    strncpy(dst, utf8, 127);
    dst[127] = 0;
}

static int index_of(GCController *controller) {
    int slot;
    for (slot = 0; slot < GENOS_PAD_SLOTS; slot++) {
        if (slots[slot].controller == controller) {
            return slot;
        }
    }
    return -1;
}

static void assign_slot(GCController *controller) {
    int slot;
    pthread_mutex_lock(&slots_lock);
    if (!controller || !controller.extendedGamepad || index_of(controller) >= 0) {
        pthread_mutex_unlock(&slots_lock);
        return;
    }
    for (slot = 0; slot < GENOS_PAD_SLOTS; slot++) {
        if (slots[slot].controller) {
            continue;
        }
        slots[slot].controller = controller;
        [controller retain];
        slots[slot].named = 0;
        slots[slot].name[0] = 0;
        pthread_mutex_unlock(&slots_lock);
        return;
    }
    pthread_mutex_unlock(&slots_lock);
}

static void clear_slot(GCController *controller) {
    int slot;
    pthread_mutex_lock(&slots_lock);
    slot = index_of(controller);
    if (slot < 0) {
        pthread_mutex_unlock(&slots_lock);
        return;
    }
    [slots[slot].controller release];
    memset(&slots[slot], 0, sizeof(slots[slot]));
    pthread_mutex_unlock(&slots_lock);
}

static void note_connection(NSNotification *note, int connected) {
    GCController *controller = note.object;
    if (![controller isKindOfClass:[GCController class]]) {
        return;
    }
    if (connected) {
        assign_slot(controller);
    } else {
        clear_slot(controller);
    }
}

static void ensure_started(void) {
    NSArray<GCController *> *pads;
    if (started) {
        return;
    }
    started = 1;
    connect_obs = [[NSNotificationCenter defaultCenter]
        addObserverForName:GCControllerDidConnectNotification
                    object:nil
                     queue:[NSOperationQueue mainQueue]
                usingBlock:^(NSNotification *note) {
                    note_connection(note, 1);
                }];
    [connect_obs retain];
    disconnect_obs = [[NSNotificationCenter defaultCenter]
        addObserverForName:GCControllerDidDisconnectNotification
                    object:nil
                     queue:[NSOperationQueue mainQueue]
                usingBlock:^(NSNotification *note) {
                    note_connection(note, 0);
                }];
    [disconnect_obs retain];
    pads = [GCController controllers];
    for (GCController *controller in pads) {
        assign_slot(controller);
    }
}

static void cache_name(struct GenosPadSlot *slot) {
    if (slot->named) {
        return;
    }
    copy_name(slot->name, slot->controller.vendorName);
    if (slot->name[0] == 0) {
        copy_name(slot->name, slot->controller.productCategory);
    }
    if (slot->name[0] == 0) {
        strncpy(slot->name, "Gamepad", 127);
    }
    slot->named = 1;
}

static void fill(struct GenosPadSample *out, struct GenosPadSlot *slot) {
    GCExtendedGamepad *pad;
    memset(out, 0, sizeof(*out));
    pad = slot->controller.extendedGamepad;
    if (!pad) {
        return;
    }
    out->connected = 1;
    cache_name(slot);
    memcpy(out->name, slot->name, sizeof(out->name));
    out->lx = pad.leftThumbstick.xAxis.value;
    out->ly = pad.leftThumbstick.yAxis.value;
    out->rx = pad.rightThumbstick.xAxis.value;
    out->ry = pad.rightThumbstick.yAxis.value;
    out->lt = pad.leftTrigger.value;
    out->rt = pad.rightTrigger.value;
    out->buttons[0] = pad.buttonA.pressed ? 1 : 0;
    out->buttons[1] = pad.buttonB.pressed ? 1 : 0;
    out->buttons[2] = pad.buttonX.pressed ? 1 : 0;
    out->buttons[3] = pad.buttonY.pressed ? 1 : 0;
    out->buttons[4] = pad.dpad.up.pressed ? 1 : 0;
    out->buttons[5] = pad.dpad.down.pressed ? 1 : 0;
    out->buttons[6] = pad.dpad.left.pressed ? 1 : 0;
    out->buttons[7] = pad.dpad.right.pressed ? 1 : 0;
    out->buttons[8] = pad.leftShoulder.pressed ? 1 : 0;
    out->buttons[9] = pad.rightShoulder.pressed ? 1 : 0;
    out->buttons[13] = pad.leftThumbstickButton.pressed ? 1 : 0;
    out->buttons[14] = pad.rightThumbstickButton.pressed ? 1 : 0;
    if (pad.buttonOptions) {
        out->buttons[10] = pad.buttonOptions.pressed ? 1 : 0;
    }
    if (pad.buttonMenu) {
        out->buttons[11] = pad.buttonMenu.pressed ? 1 : 0;
    }
    if ([pad respondsToSelector:@selector(buttonHome)] && pad.buttonHome) {
        out->buttons[12] = pad.buttonHome.pressed ? 1 : 0;
    }
}

int genos_gamecontroller_poll(struct GenosPadSample *out, int cap) {
    int slot;
    int limit;
    if (!out || cap <= 0) {
        return -1;
    }
    @autoreleasepool {
        ensure_started();
        memset(out, 0, (size_t)cap * sizeof(*out));
        limit = cap < GENOS_PAD_SLOTS ? cap : GENOS_PAD_SLOTS;
        pthread_mutex_lock(&slots_lock);
        for (slot = 0; slot < limit; slot++) {
            if (!slots[slot].controller) {
                continue;
            }
            fill(&out[slot], &slots[slot]);
        }
        pthread_mutex_unlock(&slots_lock);
    }
    return cap;
}
