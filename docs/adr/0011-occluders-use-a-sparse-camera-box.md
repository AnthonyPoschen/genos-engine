# Occluders use a sparse camera box

A ray finds an occluder in a box centered on the camera. The cell stays 2 m. The half-extent of the box equals the camera far plane. The pack stores a cell only where a shape overlaps it. A ray jumps an empty 16 m brick. A miss inside a brick crosses the fine cells that repeat the shapes just tested. The floor plane and the roof plane stay single tests.

The walk reads its step cap from the scene block. A ray that leaves the box misses. A shape outside the box is absent. A change of the far plane resizes the box in the same frame. The cell size does not change.

**Rejected:** A dense array the size of the camera box. A signed-distance field. A coarser stand-in mesh. A cell that grows past 2 m. A 3D cell for each floor slab.
