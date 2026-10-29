# Backend — The Binary Boardroom

This folder is organizer-only. Category: Steganography; difficulty: Hard.

## Construction
The distributed participant artifact is in the sibling `17_The_Binary_Boardroom_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Extract low bits from generated image pixels and reverse a documented column permutation.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
