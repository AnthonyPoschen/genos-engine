# 0019: Every feature is agent-operable

Status: **Proposed**, for Anthony's approval. Direction: [`docs/systems/agent-driven-editing.md`](../systems/agent-driven-editing.md).

## Decision

Every engine feature must be drivable and verifiable through text interfaces alone: data files, Rhai scripts, the CLI, MCP commands, compact metrics, and screenshots an agent can grade. A feature an agent cannot drive and check is not done.

Every new design doc has an **Agent control and verification** section. It names the commands or data fields that drive the feature, the metrics and captures that prove it works, and the harness check that fails when the wiring does nothing.

## Rejected

Editor GUI panels as the main way to author or tune content. GUIs are for players.

## Consequences

This extends [debugging](../systems/debugging.md) (one command set, a probe for every failure mode) from debugging to authoring. Existing design docs gain the section when they are next edited.
