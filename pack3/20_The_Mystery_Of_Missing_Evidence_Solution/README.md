# Official solution — The Mystery of Missing Evidence

## Intended reasoning
The evidence supports two mutually exclusive locations and provides no independent discriminator. Do not guess. The intended answer is the meta-flag acknowledging that the evidence is insufficient.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{insufficient_evidence_is_an_answer}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
