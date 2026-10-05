# Rhai is the scripting language

Games and mods need a scripting language. The language is Rhai. The book at `https://rhai.rs/book/` describes Rhai 1.26.0.

Rhai embeds in the Rust runtime. A script runs in a sandbox. The checked build stops a runaway script, a deep call stack, and an oversized value. A script does not panic the host. Mods keep that checked build.

A script calls functions that the runtime registers. A script does not own the window, the input, the renderer, the audio, or the network. The `rhai` crate is the one scripting crate allowed by [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md). The first script places the scene in [ADR 0010](0010-first-proof-is-the-lit-scripted-scene.md). The stable evaluator is the AST interpreter, and the experimental Grain VM stays out. The Rhai book places that interpreter about two to three times slower than Python 3 on typical work.

**Rejected:** Embedded Python, as Breaking Point did. Lua. A native mod binary. The Rhai `unchecked` feature for mods.
