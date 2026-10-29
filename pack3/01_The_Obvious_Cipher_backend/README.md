# Backend — The Cipher That Filed a Complaint

This folder is organizer-only. Category: Cryptography; difficulty: Easy.

## Construction
The distributed participant artifact is in the sibling `01_The_Obvious_Cipher_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
ROT13 followed by reading the decoded sentence's final token.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
