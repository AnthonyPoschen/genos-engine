#!/usr/bin/env bash
set -u
dest="$HOME/.cursor/skills"; tmp="$(mktemp -d)"
if git clone --depth 1 -q https://github.com/AnthonyPoschen/agent-skills "$tmp/a"; then mkdir -p "$dest"; cp -R "$tmp/a/skills/." "$dest/"; echo "agent-skills synced at $(git -C "$tmp/a" rev-parse --short HEAD)"; else echo "agent-skills sync failed; continuing" >&2; fi
rm -rf "$tmp"; exit 0
