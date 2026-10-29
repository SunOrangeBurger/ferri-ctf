# Official solution — The Alt-Text Alibi

## Intended reasoning
View page source and inspect every image's alt attribute. One alt value is a meaningful underscore-separated payload; wrap it in the flag format.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{look_beyond_the_picture}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
