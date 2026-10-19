"""Scene and checkpoint catalog the verdict scripts score.

The ids match `genos-lighting-ref`. Sponza stays skipped. Each row names the
debug scene file a live `genos-lighting` launch can open. Reference pictures
are not stored.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class View:
    scene: str
    checkpoint: str
    lighting_scene: str
    lighting_checkpoint: str
    available: bool
    skip_reason: str = ""


VIEWS: tuple[View, ...] = (
    View("stress-still", "hall", "stress", "hall", True),
    View("stress-still", "room-a", "stress", "room-a", True),
    View("stress-moving", "hall-moving", "stress", "hall", True),
    View("stress-moving", "box-step-before", "stress", "hall", True),
    View("stress-moving", "box-step", "stress", "hall", True),
    View("bounce-corner", "corner", "bounce_corner", "corner", True),
    View("hell-100", "night-hall", "hell", "hall", True),
    View("direct-only", "hall", "stress", "hall", True),
    View("sponza", "atrium", "sponza", "atrium", False, "no Sponza asset in this repository"),
)


def available() -> list[View]:
    return [view for view in VIEWS if view.available]


def scenes() -> list[str]:
    ordered: list[str] = []
    for view in available():
        if view.scene not in ordered:
            ordered.append(view.scene)
    return ordered
