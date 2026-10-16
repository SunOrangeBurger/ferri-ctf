# Connecting hosted question websites

FerrisCTF remains the contest platform: players sign in, join teams, receive a
round's challenge pool, download participant files, and submit recovered flags.
An interactive question can also have a **Challenge website** URL. Eligible
players open that website in a new tab and return to FerrisCTF to submit its flag.

## Operator setup

1. Run the question's backend using that question's organizer instructions. Keep
   its backend, solutions, flags, access-code roster, and runtime database private.
2. Check its URL from a participant device. Use an address reachable by players,
   such as `http://192.168.1.10:9101/` on the venue LAN or a public HTTPS URL.
   `localhost` and `127.0.0.1` in a player's browser point to that player's own
   computer. An organizer's SSH tunnel is not a public participant URL.
3. Sign in with the existing Ferris/operator account and open **Manage Challenges**.
   Create a challenge with its title, instructions, category, difficulty, points,
   expected flag, and **Challenge website**. A participant ZIP is optional and may
   accompany the website. Upload only player-facing files.
4. For an existing challenge, use its **Challenge website** field in the bank to
   set or replace the URL. Save an empty field to remove it. This changes only
   the URL and records an audit entry.
5. Follow the usual event preparation and round allocation. The launch button is
   available only on an active challenge in the locked team's active-round pool.
   Adding a question does not retroactively add it to an already assigned pool.

The URL may be HTTP for the original college LAN deployment or HTTPS for public
hosting. Only full HTTP(S) URLs without embedded login credentials are accepted.
The website opens in a separate tab; its session is separate from FerrisCTF.
Provide any question-specific team code privately to the corresponding team.
FerrisCTF never appends its session, team credentials, or expected flag to the URL.
For public question services, prefer a different hostname from the contest
platform: [cookies are not isolated by port](https://www.rfc-editor.org/rfc/rfc6265#section-8.5).
Using only another port on the same hostname also sends that host's cookies to
the question service. A separate question hostname keeps FerrisCTF's host-only
session cookie off those requests.

## Match the revealed flag to the contest

FerrisCTF currently stores one expected flag per question. The question website
must reveal that same flag. For backends supporting `create-team --flag`, supply
the same question flag when provisioning each team's independently stored puzzle
state, and enter that flag in FerrisCTF. For backends with a supplied static flag,
use the matching organizer flag without altering its evidence files.

Backends that always generate different flags per team require a separate
integration; merely linking them does not make FerrisCTF accept those flags.
Read the backend's own organizer guide before provisioning real teams.

## Hosting and rollout

This field links to an already running service. Uploading a ZIP still stores a
downloadable attachment; it does not execute or extract a backend, start a
container, or configure a proxy. FerrisCTF makes no outbound request to the saved
URL. Network reachability and admission to the question service remain the
operator's responsibility, as in the original deployment specification.

## Server load

The launch control is a normal browser link. There is no iframe, preview,
prefetch, proxy, website health poll, or additional background worker. Players
connect directly to the question service only when they choose to open it.
The optional URL is read by the existing challenge-detail query; challenge-list
and scoreboard polling are unchanged. URL validation runs only on operator
create/save requests. An empty URL keeps the download-only path.

Each hosted question still consumes resources wherever its backend runs. This
integration adds no per-player process or container, and it does not establish
the capacity of question services sharing the same rented server. Use the
question backends' shared per-team state and measure their own load separately.

## Upgrade

Build and rehearse locally before updating a rented or shared host. Deployment of
the new binary automatically applies migration `0004_challenge_websites.sql`,
which adds a nullable field; existing challenges start with no website and keep
their download/submission behavior. Use the existing backup procedure for the
database, uploads, and server secret before a production upgrade. Creating this
PR does not deploy it or change a running server.
If rolling back the application, restore the corresponding pre-upgrade database
backup as well; do not remove migration-history records to force an old binary
to accept a newer database.

Validate the participant URL, team-specific state, access codes, and a full
solve-to-score flow. A FerrisCTF scoring soak test does not establish capacity of
the separate question backends, their databases, or their login limits.
