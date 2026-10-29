# Official solution — Bitwise Breakroom

## Intended reasoning
Split the log on spaces, parse each group as base-2, and convert the resulting byte values to ASCII. The result is the flag.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{bits_before_biscuits}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
