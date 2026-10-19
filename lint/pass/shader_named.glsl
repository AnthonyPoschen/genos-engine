#version 450

const float QUARTER = 0.25;
const float BIAS = 0.1;

float shade(float x) {
    return x * QUARTER + BIAS;
}
