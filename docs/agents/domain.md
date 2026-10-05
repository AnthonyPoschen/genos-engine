# Domain docs

Read these files before you add a term or a decision.

- Read `CONTEXT.md` at the repository root.
- Read `docs/adr/` for a decision in the area you will change.
- Before you add a crate, a window, or a renderer, read `docs/architecture.md`.

This repository has one context. There is no `CONTEXT-MAP.md`.

## File structure

```text
/
├── AGENTS.md
├── CONTEXT.md
├── README.md
└── docs/
    ├── adr/
    ├── agents/
    │   └── domain.md
    ├── architecture.md
    ├── lighting.md
    ├── goals/
    │   └── mvp-scene.md
    └── references/
        └── breaking-point.md
```

## Vocabulary

Use the terms in `CONTEXT.md`. When a definition lists `_Avoid_`, use the defined term.

## ADR conflicts

When a change conflicts with an ADR, name that ADR. Do not override an ADR in silence.
