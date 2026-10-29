#!/usr/bin/env python3
"""Import pack4 participant ZIPs and solution flags; keep CaughtRedHanded inactive.

Run from the repository root: python3 scripts/import_pack4.py
CaughtRedHanded needs per-team case support before it can be activated.
"""
from import_pack3 import main

if __name__ == '__main__':
    main(pack="pack4", expected_count=30)
