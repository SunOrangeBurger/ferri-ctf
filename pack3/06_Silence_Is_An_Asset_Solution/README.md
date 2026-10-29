# Official solution — Silence Is an Asset

## Intended reasoning
Use the provided standard-library script to measure tone/silence runs. The signal encodes Morse with short and long tone durations; translate the Morse groups to text.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{listen_between_beeps}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
