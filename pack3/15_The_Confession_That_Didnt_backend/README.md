# Backend — The Confession That Didn't

This folder is organizer-only. Category: Document Forensics; difficulty: Medium.

## Construction
The distributed participant artifact is in the sibling `15_The_Confession_That_Didnt_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Reject a decoy by validating SHA-256 against manifest.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
