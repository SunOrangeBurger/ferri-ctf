# Official solution — The Cipher That Filed a Complaint

## Intended reasoning
ROT13 turns the line into: The final token is rotating_is_not_a_strategy. Wrap that token in the required flag format.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{rotating_is_not_a_strategy}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
