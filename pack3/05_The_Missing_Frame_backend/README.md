# Backend — The Missing Frame

This folder is organizer-only. Category: Image Forensics; difficulty: Medium.

## Construction
The distributed participant artifact is in the sibling `05_The_Missing_Frame_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Read pixel RGB values from a generated PNG; blue-channel low bits encode ASCII.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
