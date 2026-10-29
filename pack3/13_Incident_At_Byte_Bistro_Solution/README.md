# Official solution — Incident at Byte Bistro

## Intended reasoning
Compare the chat and access log timestamps. The workstation clock is exactly 90 seconds fast; correct it, then match the only login to the message that mentions the deploy. The recovered note contains the flag payload.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{the_clock_was_wrong}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
