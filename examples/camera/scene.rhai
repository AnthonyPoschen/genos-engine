// White floor, a corner, three colored solids, and one bright white lamp.
floor(0.0, 0.0, 16.0, 16.0);
wall(2.0, 5.0, 8.0, 0.4, 2.6);
wall(6.0, 1.5, 0.4, 7.0, 2.6);
solid("square", 0.0, 0.0, 1.5, 1.0, 0.0, 0.0);
solid("circle", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0);
solid("square", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0);
light(0.0, 7.0, 0.0, 1.0, 1.0, 1.0);
