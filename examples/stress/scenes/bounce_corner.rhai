// Closed corner for a lamp that moves, then stops.
// The point lamp sits in the -X/-Z corner. --lamp-stop shifts it along +X
// and then holds it, so catch-up can be counted. The sun in this file does
// not move. Place the camera inside the room:
//   --eye 0.8 1.6 0.8 -0.785 0.1
floor(0.0, 0.0, 6.0, 6.0);
roof(3.0, 0.9, 0.9, 0.9);
wall(0.0, -3.0, 6.2, 0.2, 3.0);
wall(-3.0, 0.0, 0.2, 6.2, 3.0);
wall(0.0, 3.0, 6.2, 0.2, 3.0);
wall(3.0, 0.0, 0.2, 6.2, 3.0);
solid("square", 0.4, 0.2, 0.4, 0.8, 0.2, 0.15);
solid("square", -0.2, 1.2, 0.35, 0.15, 0.2, 0.8);
solid("circle", 1.4, -0.4, 0.3, 0.2, 0.7, 0.2);
light(-1.2, 1.6, -1.2, 4.0, 3.6, 3.2);
sun(0.15, -0.9, 0.3, 0.6, 0.58, 0.5);
