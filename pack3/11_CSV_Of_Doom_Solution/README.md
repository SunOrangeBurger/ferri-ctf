# Official solution — CSV of Doom

## Intended reasoning
Sort the CSV rows numerically by `position`, then concatenate the `fragment` values. The reconstructed text is the flag.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{sort_then_stitch}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
