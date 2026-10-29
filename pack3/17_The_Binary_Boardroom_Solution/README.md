# Official solution — The Binary Boardroom

## Intended reasoning
Extract the least-significant blue-channel bits row-wise, group into bytes, then reverse the column permutation described in the meeting minutes. Decode the result as ASCII.

## Reproduction
From this directory, run:

```bash
python solve.py
```

The script reads the sibling participant artifact and checks the intended result. Expected output:

```text
nexus{pixels_have_a_chain_of_custody}
```

## Fairness note
The answer is supported by the supplied synthetic evidence. Decoys, if present, are rejected by explicit cross-checks rather than guesswork.
