# Backend — Invoice Archaeology

This folder is organizer-only. Category: File Forensics; difficulty: Easy.

## Construction
The distributed participant artifact is in the sibling `02_Invoice_Archaeology_Participants/` directory. The canonical flag is stored in the sibling solution folder. This challenge uses synthetic data only.

## Intended method
Read a deliberately odd sequence of invoice line-item quantities.

## Regeneration and validation
Run `python build.py` from this directory. It rebuilds a copy of the participant artifact(s) in `rebuilt/` using the deterministic source description in `challenge.json`. Compare the rebuilt artifact with the distributed one where applicable.
