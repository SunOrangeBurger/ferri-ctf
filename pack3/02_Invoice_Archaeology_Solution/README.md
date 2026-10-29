# Official solution — Invoice Archaeology

## Intended reasoning
Read the quantity values from top to bottom: 18, 5, 3, 5, 9. Convert A1Z26 (18=R, 5=E, 3=C, 5=E, 9=I) and use the invoice note in the README to take the first five letters of the hidden phrase from the companion artifact. The companion artifact spells RECEIPTS TELL THE TRUTH; the flag uses its normalized phrase.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{receipts_tell_the_truth}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
