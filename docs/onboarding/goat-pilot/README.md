# GOAT onboarding reference pilot

Open **GOAT_Onboarding_72.html** in a browser to use all 72 questions offline. Search for words or a question ID and open its answer. Each of the 59 fully or partly source-backed answers links to a line at a fixed revision. The 13 local questions explain what to ask the responsible team. Across the bank, 35 are documented guidance, 22 need context as well as technical guidance, and 2 need release verification. The federation procedure is development guidance: its source says it has not yet run between two real goatnets or with a real counterparty, so the pilot displays that status for prepared cards and free-text federation results.

**Ask for help** is the common front door: any available GOAT onboarding or support team member can guide a newcomer or privately route the question. This does not give that teammate approval or certificate-issuance authority. The monitored destination that works before GOAT access is not yet confirmed, so the HTML keeps that known gap visible instead of publishing an unverified or personal contact. A CODEOWNER still needs to confirm the destination and have a newcomer verify it without GOAT access.

## WSL quick start

From the extracted folder, Python 3.8+ runs the bundled pilot without additional packages.

    python3 pilot.py ask Q23 --bundled
    python3 pilot.py ask Q70 --bundled
    python3 pilot.py ask "what can partners read with just a trust bundle" --bundled

The first two retrieve prepared answer cards. A new question retrieves ranked excerpts, using word matches weighted by rarity and headings. Read the surrounding source by following the line link. An exact question or ID is a lookup; success at lookup does not measure how an AI answers unfamiliar wording.

## Document refresh is manual

    python3 pilot.py ask "your question" --live --core-ref main --stuff-ref main

Nothing watches GitHub, and no schedule or default-branch event refreshes the pilot. The `--live` command above is the trigger: it fetches read-only public snapshots of both default branches, resolves them to exact commits, replaces the cache only after both scans succeed, and then searches the new index. A newly added in-scope Markdown document becomes searchable in that same successful command. A later run without `--live` may use the last saved index, labelled by its exact commits and with a warning to sync again; it does not check the remote.

For already-fetched local checkouts, the exact manual trigger is:

    python3 pilot.py sync --core /path/to/goatcore --core-ref origin/main --stuff /path/to/goatstuff --stuff-ref origin/main

For local checkouts this reads the available Git objects; fetch your remotes first if you want newer objects. The sync indexes Markdown README.md, docs/, and ops/gym/ within the two selected repositories. It replaces the complete cached index only after both scans succeed. Files outside that scope, local SOPs, PDFs, other repositories and deployed settings are outside the pilot. Cache location defaults to `$XDG_CACHE_HOME/goat-onboarding-pilot`, or `~/.cache/goat-onboarding-pilot` when `XDG_CACHE_HOME` is unset. No background watcher runs.

Finding a new document and updating a prepared answer are separate. Sync makes a new or changed document searchable, but it does not rewrite a reviewed card. A prepared technical answer is shown only when its cited source content is unchanged; otherwise the pilot withholds it and returns current excerpts for review.

The downloadable HTML does not read the cache and does not update automatically. To publish revised prepared answers, a maintainer must review the changed sources, update `sources.json`, the source metadata and answers in `bank.json`, and then run:

    python3 render.py

Commit the updated `GOAT_Onboarding_72.html` with those reviewed inputs. Automatic refresh is a follow-up, not current behavior. The pilot cannot decide that a local contact has authority or establish an installed version from a design decision. The snapshot was collected from public development docs on 20 November 2026. It does not establish the state of any deployed goatnet.

## Optional model-generated answer

If you already run an approved model locally with Ollama, substitute its installed name:

    python3 pilot.py ask Q70 --live --core-ref stable --model YOUR_INSTALLED_MODEL

The CLI sends the question and up to five retrieved excerpts to the local Ollama chat endpoint. It prints the model draft and source links for verification. Generation is optional; this package includes no model or credentials. The pilot does not verify each model sentence or guarantee the best source appears in the top five. Use public or fictional practice questions. Named authority and account questions require local confirmation.

## Acceptance checks

    python3 check.py
    python3 retrieval_check.py

The evidence captured here is in TEST_RESULTS.txt. **72/72 is the bundled-card execution check:** it verifies that every stored card is returned and its citation anchors resolve, not that an answer is accurate. **53/59 is the development retrieval result** on the 12 bundled documents; the six known misses remain Q24, Q26, Q27, Q28, Q30 and Q63. The deliberately written development paraphrases are not held-out learner questions, and answer accuracy with new users is still unmeasured. Controlled local Git tests exercise changed, missing and unchanged source behavior and cache preservation. A production pilot also needs real-user feedback and evaluation with the actual model and supported releases.
