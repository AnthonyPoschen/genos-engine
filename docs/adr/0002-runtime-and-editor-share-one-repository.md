# The runtime and the editor share this repository

The runtime runs a game. The editor is the Linux tool that makes a game. Both live in this Cargo workspace. Runtime crates do not depend on the editor. The editor depends on runtime crates.

**Rejected:** A second repository for the editor.
