// Closed-room check for the bounce: the shipped scene inside four outer walls.
// White surfaces only, one lamp just above the red box and below the wall tops.
// A white roof sits on the wall tops, so no light escapes.
floor(0.0, 1.4, 16.0, 18.72);
// Outer walls on the floor edges: x from -8 to 8, z from -7.96 to 10.76.
wall(0.0, -7.76, 16.0, 0.4, 2.6);
wall(0.0, 10.56, 16.0, 0.4, 2.6);
wall(-7.8, 1.4, 0.4, 18.72, 2.6);
wall(7.8, 1.4, 0.4, 18.72, 2.6);
wall(2.0, 5.0, 8.0, 0.4, 2.6);
wall(6.0, 1.5, 0.4, 7.0, 2.6);
wall(-1.5, 7.65, 0.4, 5.1, 2.6);
wall(1.5, 6.2, 0.4, 2.2, 2.6);
wall(3.8, 8.3, 4.6, 0.4, 2.6);
wall(2.3, 10.4, 7.6, 0.4, 2.6);
wall(5.9, 9.35, 0.4, 2.1, 2.6);
solid("square", 0.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.32, 1.0);
solid("circle", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0, 0.32, 1.0);
solid("square", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0, 0.32, 1.0);
roof(2.6);
light(0.0, 2.4, 0.0, 1.0, 1.0, 1.0);
