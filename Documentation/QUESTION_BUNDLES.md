# Prepare the 15 supplied questions

The optional standard-library packager converts the organizer's structured
`FINAL_QUESTIONS.zip` into ordered Q01–Q15 participant uploads and a separate,
private set of form fields, flags, backends, solutions, and hosting instructions.
It does not execute the source archive, install dependencies, access the network,
upload challenges, start services, or change an existing event.

Run with Python 3.11 or later from this checkout:

```sh
python3 scripts/package_questions.py /path/to/FINAL_QUESTIONS.zip \
  --output /path/to/private-delivery/CTF-Questions
```

Choose a private output location outside the Git checkout. The packager refuses
to replace an existing delivery, preserving generated flags, secrets, and any
subsequent event state. Do not commit the input archive or generated delivery.
The archive is supplied privately by the organizer; it is not included here.

The output consists of:

- `CTF-Participant-Uploads.zip`: a collection of 15 individually numbered ZIPs.
  Extract it first and upload the matching inner ZIP for each challenge.
- `CTF-Organizer-Private.zip`: the entire private delivery, including participant
  uploads, copyable descriptions, per-question JSON, original solutions and
  backends, local helper, setup guide, source hashes, and checksums.
- `CTF-Questions/START_HERE.md`: the entry point in the extracted delivery.

Only the files in the participant directories' reviewed inventories can enter a
participant ZIP. Virtual environments, runtime databases, SQLite sidecars,
caches, and macOS metadata are excluded from the cleaned sources. README changes
replace local development addresses with the organizer-supplied event URL. Q14's
brief is adapted to its existing offline evidence route; its evidence is copied
unchanged. The other 14 questions still require hosted backends.

Points and waves start unset. Difficulties need explicit operator review before
creation. The existing 60/30/10 pool allocation does not automatically give every
team all 15 questions, especially when the source briefs mostly say Hard.
Numbering preserves the source order, not an unverified difficulty progression.
FerrisCTF currently sorts player lists by difficulty and then title.

For Q01–Q10 and Q15, each delivery generates a new private fixed question flag;
the bundled provisioning helper supplies it to each team's independent puzzle.
Q11–Q14 keep their original static flags and evidence. A consistent expected flag
is necessary for FerrisCTF's current one-flag-per-challenge verifier.

The per-question `challenge.json` files are field references, not an automatic
import format. Use the existing operator form and upload the matching ZIP;
`expected_flag` maps to the form's Flag field. The helper can check missing
settings, provision a supplied team roster into local question databases, and
run one service on loopback in the foreground. It is never called by FerrisCTF
and adds no work to player requests.

The private setup guide explains team codes, separate rehearsal copies, original
dependency pins versus the shared rehearsal environment, participant URL setup,
and round/pool limitations. Wave values remain planning metadata: this pack does
not enforce waves or prevent access through a previously known backend URL.
Publishing or deploying those services remains a separate operator action.

Packager checks, using synthetic files with no event answers:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts \
  -p 'test_package_questions.py' -v
```

For the actual private delivery, rehearse the original per-question test suites
and the full upload/download/solve flow in disposable copies. Packaging does not
establish production capacity or representative puzzle difficulty.
