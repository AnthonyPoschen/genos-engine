#!/usr/bin/env python3
"""Command line for the noise-floor verdict. The logic lives in noise_floor.py."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from noise_floor import main

if __name__ == "__main__":
    main()
