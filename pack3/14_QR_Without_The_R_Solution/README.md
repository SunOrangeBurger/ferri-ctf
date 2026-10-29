# Official solution — QR Without the R

## Intended reasoning
Read the 8-bit groups in each row from left to right, then join rows from top to bottom and decode the resulting bytes. The README explains the row separators.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{rows_are_read_left_to_right}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
