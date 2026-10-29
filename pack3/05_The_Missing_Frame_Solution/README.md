# Official solution — The Missing Frame

## Intended reasoning
Use the provided Python script or inspect RGB pixels row-wise. Read the least-significant bit of each blue channel, group bits into bytes, and decode ASCII until the closing brace.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{pixels_keep_secrets}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
