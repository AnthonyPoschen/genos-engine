/* GameController pads. The Rust side writes them with the public setters.
 * A controller without an extended gamepad does not take a slot.
 * Stick Y is already positive upward. */

#import <GameController/GameController.h>

#include <stdint.h>
#include <string.h>

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

static void fill(struct GenosPadSample *out, GCController *controller) {
    GCExtendedGamepad *pad;
    memset(out, 0, sizeof(*out));
    pad = controller.extendedGamepad;
    if (!pad) {
        return;
    }
    out->connected = 1;
    copy_name(out->name, controller.vendorName);
    if (out->name[0] == 0) {
        copy_name(out->name, controller.productCategory);
    }
    if (out->name[0] == 0) {
        strncpy(out->name, "Gamepad", 127);
    }
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
    NSArray<GCController *> *pads;
    int used = 0;
    if (!out || cap <= 0) {
        return -1;
    }
    memset(out, 0, (size_t)cap * sizeof(*out));
    pads = [GCController controllers];
    for (GCController *controller in pads) {
        if (used >= cap) {
            break;
        }
        fill(&out[used], controller);
        if (out[used].connected) {
            used += 1;
        }
    }
    return used;
}
