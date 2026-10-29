# Official solution — The Confession That Didn't

## Intended reasoning
The bundle contains a manifest with the expected SHA-256 for the authentic note. Hash both notes and compare with the manifest. The matching note gives the payload; the other is a documented decoy.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{hashes_beat_hearsay}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
