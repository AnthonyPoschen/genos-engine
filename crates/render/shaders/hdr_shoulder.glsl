// Legacy shoulder. The same curve as mesh::tone. Compose and the display
// pass both include this, so a change to the curve cannot hide in one copy.
const float SHOULDER_START = 0.64;
const float SHOULDER_GAIN = 0.14;
const float SHOULDER_SOFT = 1.1;

float tone_channel(float x) {
    if (x <= SHOULDER_START) {
        return max(x, 0.0);
    }
    float extra = x - SHOULDER_START;
    return SHOULDER_START + SHOULDER_GAIN * (extra / (extra + SHOULDER_SOFT));
}
