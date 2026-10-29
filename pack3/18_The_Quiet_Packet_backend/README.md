# Backend — The Quiet Packet

This folder is organizer-only. Category: Network / Log Analysis; difficulty: Hard.

## Construction
The distributed participant artifact is in the sibling `18_The_Quiet_Packet_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Reassemble out-of-order packet fragments and verify checksums.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
