// A dark corner and one lamp. No sun and no sky, so the corner is bounce.
// The lamp starts out in the room. bounce_corner.rhai walks it toward the
// corner and stops. Watch the corner, not the bulb.
floor(1.5, 1.5, 6.0, 6.0);
roof(2.8, 0.12, 0.12, 0.14);
// The two walls meet near the origin. Reflectance is low so the corner stays dim.
wall(1.5, 0.08, 3.0, 0.16, 2.6, 0.35, 1.0);
wall(0.08, 1.5, 0.16, 3.0, 2.6, 0.35, 1.0);
// The scene loader wants three solids. They sit behind the camera, small and
// dark, so they stay out of the corner.
solid("square", 4.2, 3.4, 0.25, 0.02, 0.02, 0.02, 0.02, 0.0);
solid("square", 3.4, 4.2, 0.25, 0.02, 0.02, 0.02, 0.02, 0.0);
solid("circle", 4.2, 4.2, 0.2, 0.02, 0.02, 0.02, 0.02, 0.0);
// Dim enough to stay under the tone shoulder. Color 8 sits near 0.78 everywhere,
// so the corner, the floor and the shadow read as one grey wall.
light(1.6, 1.55, 1.5, 0.2, 0.18, 0.14);
