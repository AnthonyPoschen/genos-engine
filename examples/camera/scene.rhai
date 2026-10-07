// White floor, a corner, three colored solids, and one bright white lamp.
// The floor also covers the L corridor on the +Z side of the long wall.
// Depth is a whole number of 16 cm cells so the floor grid stays even.
floor(0.0, 1.4, 16.0, 18.72);
wall(2.0, 5.0, 8.0, 0.4, 2.6);
wall(6.0, 1.5, 0.4, 7.0, 2.6);
// Near leg runs north from the long wall. The far leg turns east,
// far enough north that the opening view does not graze those walls.
wall(-1.5, 7.65, 0.4, 5.1, 2.6);
wall(1.5, 6.2, 0.4, 2.2, 2.6);
wall(3.8, 8.3, 4.6, 0.4, 2.6);
wall(2.3, 10.4, 7.6, 0.4, 2.6);
wall(5.9, 9.35, 0.4, 2.1, 2.6);
solid("square", 0.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.8, 1.0);
solid("circle", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0, 0.8, 1.0);
solid("square", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0, 0.8, 1.0);
// Rays travel down at 45 degrees toward +Z, into the hallway.
// sun(0.0, -1.0, 1.0, 1.0, 1.0, 1.0);
light(0.0, 7.0, 0.0, 1.0, 1.0, 1.0);
