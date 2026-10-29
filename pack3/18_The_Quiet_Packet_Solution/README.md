# Official solution — The Quiet Packet

## Intended reasoning
Sort fragments by sequence number, concatenate their payloads, and verify the SHA-256 digest against the supplied capture manifest. Only then decode the assembled message.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{reassemble_then_verify}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
