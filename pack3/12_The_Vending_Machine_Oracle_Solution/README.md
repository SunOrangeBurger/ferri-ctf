# Official solution — The Vending Machine Oracle

## Intended reasoning
Test each drawer as the location. Exactly one statement must be true, and the flag must be in the drawer whose own statement is true. The only consistent drawer is D; the inventory slip names its payload: only_one_drawer_fits.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{only_one_drawer_fits}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
