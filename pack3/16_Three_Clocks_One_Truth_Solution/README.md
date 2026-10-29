# Official solution — Three Clocks, One Truth

## Intended reasoning
Parse each ISO timestamp with its UTC offset, normalize to UTC, sort chronologically, then take the first letter of each `message` field. The letters spell the flag payload.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{normalize_before_order}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
