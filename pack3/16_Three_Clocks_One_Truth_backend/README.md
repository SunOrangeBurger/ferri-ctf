# Backend — Three Clocks, One Truth

This folder is organizer-only. Category: Multi-stage Forensics; difficulty: Hard.

## Construction
The distributed participant artifact is in the sibling `16_Three_Clocks_One_Truth_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Normalize timezone offsets, order records, then decode initials.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
