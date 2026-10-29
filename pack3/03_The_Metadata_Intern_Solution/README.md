# Official solution — The Metadata Intern

## Intended reasoning
Inspect the HTML source, not just its rendered page. The archive note inside the HTML comment contains the flag payload.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{comments_are_not_confidential}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
