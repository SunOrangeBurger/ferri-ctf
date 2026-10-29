# Official solution — The Automation That Automated Itself

## Intended reasoning
Use the known-plaintext pair in the report to infer the letter mapping, then apply that mapping to the encoded second line. The mapping is consistent and yields the payload.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{automation_needs_observation}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
