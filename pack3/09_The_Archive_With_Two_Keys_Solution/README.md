# Official solution — The Archive With Two Keys

## Intended reasoning
The JSON contains ciphertext hex and two key fragments. Join the fragments in the order indicated by the timestamps in the notes, then XOR the ciphertext bytes with the repeating key and decode UTF-8.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{two_notes_one_key}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
