### goatbuilder — el (el)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Build και ship ενός περιβάλλοντος goat: δήλωση, σύγκλιση, επαλήθευση — κάτω από ένα ενιαίο deploy contract
help-flag-output = Μορφή εξόδου: πίνακας για άνθρωπο· JSON ή YAML για πρόγραμμα
help-flag-repo = Η ρίζα του repository. Προεπιλογή: το checkout όπου τρέχει αυτό το binary
help-flag-yes = Επιβεβαίωση declarative verb χωρίς ερώτηση (για αυτοματισμό)
help-flag-frozen = Απόρριψη κάθε εκτέλεσης που θα απαιτούσε fast-forward του checkout (αυστηρό C3)
help-flag-mock = Εκτέλεση σε φάκελο fixture αντί για hosts: χωρίς registry, χωρίς transport, ντετερμινιστικά (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Γλώσσα για το help και τις εξηγήσεις σφαλμάτων (tag όπως fr, uk, de)· η έξοδος που διαβάζει ένα pipeline μένει στα αγγλικά. Προεπιλογή: το locale του OS
help-cmd-validate = Έλεγχος ενός config με βάση το versioned schema και τους κανόνες cross-field, offline
help-cmd-doctor = Έλεγχος όσων χρειάζεται μια εκτέλεση πριν γίνει οτιδήποτε: εξαρτήσεις, φρεσκάδα του checkout, transport κάθε περιβάλλοντος, και ότι το playbook κάθε component επιλύεται σε συγκεκριμένο target mode
help-cmd-status = Δηλωμένο έναντι καταγεγραμμένου, ανά μηχάνημα: converged, pending ή unknown (C7, C8). Μόνο ανάγνωση. Exit 3 όσο κάποιο μηχάνημα δεν είναι converged
help-cmd-deploy = Εφαρμογή του pin που δηλώνει το main σε ένα περιβάλλον και απόδειξη: επίλυση του component και των groups του περιβάλλοντος από τα pins, του play και του target mode του, κάθε secret που κάνει template το play, του transport· εκτέλεση του play μέσω ansible-preflight· ανάγνωση του deploy ledger από κάθε host και ολοκλήρωση μόνο όταν η νεότερη γραμμή δηλώνει το δηλωμένο digest με action=deploy (C7). Σε fixture (`--mock`) τη θέση των hosts παίρνει ο scripted converger
help-cmd-release = Μεταφορά ενός release candidate από ένα tag στο main, μέσω της live επικύρωσης του dev goatnet, ως το preprod rung (ADR 1173)· μετά το soak, με `--from prod`, graduate, ενσωμάτωσή του στο fleet:, deploy στο prod και επανεπαλήθευση κάθε rung. Αντικαθιστά το ops/scripts/rc.sh βήμα προς βήμα
help-cmd-rollback = Rollback ενός deployed component στο N-οστό πιο πρόσφατο ref του, ανά τύπο artifact, με το ledger του host ως πηγή αλήθειας (ADR 1060). Αντικαθιστά το ops/scripts/rollback.sh: exit 3 απόρριψη ως μη ασφαλές (data, cert, forward-only, reversible:false), 4 guided (config, acl)
help-cmd-fastlane = Μία εντολή από ένα merged commit σε κάθε goatnet (ADR 1127): η σκόπιμη εξαίρεση από το ladder. Αντικαθιστά το ops/scripts/fastlane.sh βήμα προς βήμα· απορρίπτει unpinned digest ή ανοιχτό graduation set, και τρέχει τα deploys μέσω αυτού του binary
help-cmd-standup = Ένα site από το μηδέν σε επαληθευμένο: οι φάσεις του standup με τη σειρά του doctrine και fail-closed gate σε κάθε άκρο, ο observability host ως παράλληλο σκέλος, οι κλάσεις rescue και οι επιφάνειες cross-goatnet τελευταίες (ADR 1183). Αντικαθιστά το ops/scripts/goat-standup.sh βήμα προς βήμα. Σε fixture (`--mock`) κάθε component κάθε κλάσης γίνεται converge από τον scripted converger και ο χρόνος declared → verified είναι το μέτρο του one-hour goatnet
help-cmd-join-node = Ένα ακόμη μηχάνημα μιας κλάσης σε live site, declared → converged → verified: η εκτελεσμένη ακολουθία του docs/tutorials/02-first-federated-node.md ως αριθμημένες φάσεις με τα gates της (reach, key, πρώτα η δουλειά στο repo, baseline, kernel WireGuard πριν και μετά το enrolment, το kernel ACL plane ως gate του enrolment, το converge της κλάσης, acceptance)
help-cmd-federate = Ένα sovereign peering δηλωμένο, converged και επαληθευμένο (ADR 1157): μετράται το δικό μας μισό, τα πακέτα ανταλλάσσονται out of band, συντάσσεται και επαληθεύεται μία συμφωνία, δημιουργούνται από αυτήν τα leaves, οι κανόνες αποδίδονται από τον έναν writer, γίνεται deploy το edge, ανοίγει το transport που επέλεξε η συμφωνία, acceptance και στα τέσσερα tiers. Αντικαθιστά το ops/scripts/peering-standup.sh βήμα προς βήμα
help-cmd-pins = ΜΙΑ επίλυση με ΔΥΟ αποδόσεις: το group_vars pins.yml κάθε listed member και το IntentV1 artifact κάθε listed goatnet, από το ops/pins/fleet-pins.yaml. Αντικαθιστά το ops/pins/generate-pins.py byte προς byte. Το `--check` επικυρώνει, αναγεννά και απορρίπτει drift, unsigned ή αλλοιωμένο intent, και governed pin ορισμένο εκτός pins.yml (exit 1)
help-cmd-promote = Καθοδήγηση και παρακολούθηση του release graduation στο ladder των goatnet (ADR 1120, 1125): status, άνοιγμα candidate, graduate, graduate ενός set, hotfix path, review, readiness, abandon, check. Αντικαθιστά το ops/release/graduate.py βήμα προς βήμα. Τίποτα εδώ δεν κάνει graduate μόνο του: μια μετάλλαξη τρέχει μόνο όταν κληθεί, τυπώνει τι έκανε και ζητά --yes όταν δεν υπάρχει terminal (C10, ADR 1177)
help-cmd-explain = Ερμηνεία ενός σφάλματος: κωδικός (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) ή οποιοδήποτε κείμενο σφάλματος — του ίδιου του binary ή λόγια εργαλείου που μεταφέρθηκαν (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Τυπώνει τι συνέβη, γιατί, τι να κάνεις και πού να διαβάσεις περισσότερα· το `-` διαβάζει το κείμενο από stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Ένα site από «verified» σε τίποτα — ο καθρέφτης του standup, τα αρχεία διατηρούνται (το burn-down προσθέτει γραμμές teardown, δεν σβήνει ποτέ). Σε fixture (`--mock`) κάνει πρόβα του burn-down με αντίστροφη σειρά φάσεων· σε πραγματικούς hosts τυπώνει τη σειρά και αρνείται να εκτελέσει μέχρι κάθε φάση να δηλώσει το δικό της play teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Μαθήματα, στιλ vimtutor: καθένα ονομάζει την ακριβή εντολή που θα τρέξεις σε ένα πρόχειρο αντίγραφο ενός fixture (ή, μόνο για ανάγνωση, σε αυτό το checkout) και μετά ελέγχει τι έκανε πραγματικά η εντολή — το exit και τις γραμμές της — πριν ξεκλειδώσει το επόμενο. Το `tutor` δείχνει το τρέχον μάθημα και το σημειώνει ως ολοκληρωμένο όταν οι εκτελέσεις σου το ικανοποιούν· `tutor <n>`, `--list`, `--reset`
help-cmd-make = Κρυφό (src/fun.rs)
help-cmd-argan = Κρυφό (src/fun.rs)
help-cmd-bound = Κρυφό (crates/goat-fun)
help-cmd-secrets = Η δηλωμένη στάση για τα secrets (ADR 1204): ποιο store ισχύει (1password, openbao, sops, git-crypt, env-file, env), τι είναι διαθέσιμο, πώς να παρέχεις ό,τι λείπει και οι δικοί του έλεγχοι του store. Το `init --env <goatnet>` γράφει το αρχείο env του quickstart χωρίς SaaS: μια ισχυρή τυχαία τιμή για κάθε μεταβλητή που τα plays του goatnet περνούν σε template, 0600, στο gitignore, χωρίς ποτέ να αλλάζει τιμή που υπάρχει ήδη. Ίδιο module με το ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Διαβάζει την πιο πρόσφατη γραμμή του deploy-ledger κάθε host για ένα component και ελέγχει αν δηλώνει το ref που δηλώνει το main (C7). Μόνο ανάγνωση· exit 3 όσο κάποιο host είναι σε αναμονή ή άγνωστο. Ο ίδιος έλεγχος που τρέχει το `deploy --verify-only`
help-cmd-diff = Τι διαφέρει από το main αυτή τη στιγμή (C8): το δηλωμένο ref του main απέναντι στην πιο πρόσφατη γραμμή ledger κάθε host, για ένα component ή, χωρίς, για κάθε component του περιβάλλοντος, με ανάγνωση σε ένα πέρασμα ανά ομάδα μελών. Μόνο ανάγνωση· exit 3 σε οποιαδήποτε απόκλιση ή host που δεν διαβάζεται
help-cmd-history = Το deploy ledger ως χρονολόγιο, από το νεότερο, σε κάθε host του περιβάλλοντος (C8): κάθε γραμμή είναι μία καταγραφή με το μηχάνημα, την ώρα, την ενέργεια (deploy ή rollback) και την έκδοση. Η προηγούμενη έκδοση της καταγραφής και ο χειριστής που την εκτέλεσε βρίσκονται στο `-o json`. Μόνο ανάγνωση
help-cmd-plan = Γράφει σε αρχείο plan, για έλεγχο, τι θα έκανε ένα δηλωτικό verb: τα δηλωμένα refs, το play και ο στόχος, το ledger, το commit του main, η μεταφορά (deploy)· ο τύπος του artifact και το ref-στόχος (rollback). Δεν αγγίζει την κατάσταση κανενός host. Τρέξτε το όπως ελέγχθηκε με `apply`
help-cmd-apply = Εκτελεί ακριβώς ένα ελεγμένο plan. Αρνείται (exit 1, με αναφορά του τι άλλαξε) όταν η δήλωση του main, το commit των pins, το play ή ο στόχος του rollback άλλαξαν από τότε που γράφτηκε το plan· αλλαγμένη μεταφορά αναφέρεται, δεν απορρίπτεται
help-cmd-adopt = Δηλώνει ό,τι τρέχει ήδη ένα περιβάλλον: διαβάζει τα ledgers κάθε host και προτείνει μια απόκλιση στα pins για κάθε component του οποίου το ref που τρέχει διαφέρει από τη δήλωση του main. Αναφορά από προεπιλογή· με --yes οι αλλαγές γράφονται σε νέο worktree του main για έλεγχο. Δεν κάνει ποτέ deploy

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Όχι ακόμα: { $why }
tutor-lesson-done = Μάθημα { $n } ολοκληρώθηκε: { $title }.
tutor-all-done-reset = Όλα τα μαθήματα ολοκληρώθηκαν. Το `goatbuilder tutor --reset` ξεκινά από την αρχή· τα tutorials συνεχίζουν πιο βαθιά: docs/tutorials/.
tutor-all-done = Όλα τα μαθήματα ολοκληρώθηκαν. Τα tutorials συνεχίζουν πιο βαθιά: docs/tutorials/.
tutor-workspace = Χώρος εργασίας: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Ελέγχει ότι ένα goatnet χτίστηκε σωστά, επίπεδο προς επίπεδο: κάθε έλεγχος αποδοχής που ισχύει για τον χώρο, μαζί με τους ελέγχους που εκτελεί ένας χειριστής με το goat CLI. Η φάση αποδοχής του standup ως ξεχωριστή εντολή. Μόνο ανάγνωση· έξοδος 3 όσο κάποιος έλεγχος αποτυγχάνει ή δεν μπορεί να απαντήσει
help-cmd-survey = Πόσο συνεπές είναι το λογισμικό μέσα σε κάθε goatnet και ανάμεσα στα goatnets της κλίμακας, με μία παράλληλη ανάγνωση. Μόνο ανάγνωση
help-flag-offline = Απομονωμένο site: μην κάνεις fetch του trunk· έλεγξε το checkout με βάση το αποθηκευμένο (cached) και κατάγραψε την εκτέλεση ως offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Έλεγχος ότι ένα goatnet βρίσκεται στην δηλωμένη τελική του κατάσταση: κάθε υπηρεσία που δηλώνει ότι τρέχει είναι αναπτυγμένη, σε λειτουργία, υγιής και επιτελεί τον ρόλο της, με τις zero-trust ακμές της συνδεδεμένες, και τίποτα δεν τρέχει χωρίς να δηλώνεται. Μόνο ανάγνωση· κωδικός εξόδου 3 όσο κάτι αποκλίνει
help-cmd-survey-long =
    Πόσο συνεπές είναι το λογισμικό μέσα σε κάθε goatnet και μεταξύ των goatnets της σκάλας, με ανάγνωση σε ένα παράλληλο πέρασμα. Μόνο ανάγνωση.
help-cmd-update = Αντικαθιστά αυτό το εκτελέσιμο με την έκδοση που διαθέτει το goatnet στο `get.<site>.<zone>/goatbuilder/`: η λήψη ελέγχεται έναντι του `.sha256` της και του υπογεγραμμένου `SHA256SUMS` πριν αγγιχτεί οτιδήποτε, και η αντικατάσταση είναι ατομική. Ήδη ενημερωμένο: μία γραμμή, έξοδος 0. Το `--check` μόνο διαβάζει (έξοδος 3 όταν υστερεί). Η πηγή: `GOATBUILDER_UPDATE_URL`, αλλιώς `--site` από τα group_vars του, αλλιώς αυτή της τελευταίας ενημέρωσης

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Πριν ξεκινήσεις: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Χρωματισμός των συμβόλων ok/FAIL και του verdict στον πίνακα (auto|always|never). Το `auto` χρωματίζει μόνο σε τερματικό· τα NO_COLOR και CLICOLOR_FORCE υπερισχύουν. Τα JSON και YAML δεν χρωματίζονται ποτέ
help-flag-verbose = Καταγραφή στο stderr όσων κάνει μια εκτέλεση — τις εξωτερικές εντολές που καλεί και τη φάση δικτύου του `update` — ώστε το `-o json|yaml` να μένει καθαρό έγγραφο. Ή ορίστε RUST_LOG=goatbuilder=debug
help-flag-timeout = Δευτερόλεπτα ορίου για τη φάση σύνδεσης/προσέγγισης πριν από άμεση αποτυχία — η μία φάση που μπορεί να μπλοκάρει σε μη προσβάσιμο host (η λήψη της διαθέσιμης έκδοσης από το `update`). ΔΕΝ περιορίζει την εκτέλεση ενός play ούτε τη λήψη του binary (όριο στασιμότητας). Διαβάζει και το GOATBUILDER_TIMEOUT
help-cmd-completion = Εκτύπωση script συμπλήρωσης κελύφους στο stdout (`goatbuilder completion zsh >> ~/.zshrc`). Για ζωντανές προτάσεις που ένα στατικό script δεν μπορεί να γνωρίζει — τα sites, τα components και τα goatnets που δηλώνει αυτό το checkout — συνδέστε ένα hook στο `goatbuilder _complete` (δείτε την κεφαλίδα του παραγόμενου script)
help-cmd-docs = Η αναφορά εντολών: το `goatbuilder docs` οδηγεί σε αυτήν· το `--markdown` εκπέμπει ολόκληρο το κείμενο, παραγμένο από το ζωντανό δέντρο εντολών ώστε να μην αποκλίνει ποτέ από το `--help`
help-cmd-_complete = Κρυφή: εκπέμπει ζωντανές προτάσεις συμπλήρωσης για hooks κελύφους, μία ανά γραμμή (τα sites, τα components και τα goatnets που δηλώνει αυτό το checkout). Μόνο διαβάζει, δεν αποτυγχάνει ποτέ θορυβωδώς — ένα hook συμπλήρωσης δεν πρέπει να χαλάει το prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Πριν από ένα rollout, η αναφορά ετοιμότητας και αναστρεψιμότητας (ADR 1211): τρία δεδομένα — State (stateless | stateful), Rollback (pin-revert | restore-required | none) και Scope (single | coupled) — δηλώνονται ανά στοιχείο και FAIL-CLOSED όταν λείπουν, με το σκέλος ετοιμότητας που αποδεικνύουν τα άλλα verbs (ref pullable, pins --check, η πύλη secrets, φρέσκια πηγή, η δηλωμένη τελική κατάσταση) και επικύρωση του δεδομένου rollback (rollback --dry-run για pin-revert, το παράθυρο διατήρησης για restore-required). Το `deploy` την εκτελεί πρώτη· βαθμολογείται με βάση την ειλικρίνεια, ποτέ ψευδές πράσινο. Μόνο ανάγνωση· έξοδος 3 όταν δεν είναι καθαρό

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Έλεγξε τη ρύθμισή σου
tutor-l1-s1 = Έλεγξε το αρχείο ρυθμίσεων
tutor-l2-title = Διάβασε την κατάσταση: converged, pending, unknown
tutor-l2-text =
    Το goatbuilder `status` συγκρίνει δύο πράγματα για κάθε μηχάνημα: την έκδοση που δηλώνει το main για αυτό, και την έκδοση που λέει η δική του καταγραφή ότι τρέχει. Δεν αλλάζει τίποτα.

    Κάθε μηχάνημα βρίσκεται σε μία από τρεις καταστάσεις:
    - converged: η καταγραφή του ονομάζει τη δηλωμένη έκδοση. Αυτό είναι έτοιμο.
    - pending: η καταγραφή του ονομάζει μια παλιότερη έκδοση, ή ακόμα τίποτα.
    - unknown: το goatbuilder δεν μπορεί να διαβάσει την καταγραφή του.

    Μόνο το converged μετράει ως έτοιμο. Αν κάποιο μηχάνημα είναι pending ή unknown, το `status` τερματίζει με κωδικό 3.

    Από αυτό το μάθημα και μετά, το `--mock` στρέφει το goatbuilder σε ένα αντίγραφο εξάσκησης στον φάκελο μαθημάτων σου αντί για πραγματικά μηχανήματα. Τίποτα από όσα κάνεις εκεί δεν φτάνει σε πραγματικό μηχάνημα.

    Τίποτα δεν έχει γίνει deploy ακόμα στο `dev` του αντιγράφου εξάσκησης. Οπότε περίμενε τέσσερα μηχανήματα pending και ένα unknown. Θα δεις μηχανήματα converged στο μάθημα 3.
tutor-l2-s1 = Διάβασε την κατάσταση του dev
tutor-l3-title = Deploy: σχεδίασέ το, τρέξε το, απόδειξέ το
tutor-l3-text =
    Το deploy βάζει τη δηλωμένη έκδοση σε κάθε μηχάνημα, και μετά διαβάζει την καταγραφή κάθε μηχανήματος για να αποδείξει ότι έφτασε. Ένα deploy είναι έτοιμο μόνο όταν η καταγραφή κάθε μηχανήματος ονομάζει αυτή την έκδοση. Ένα deploy που έτρεξε χωρίς σφάλματα δεν αρκεί.

    Θα κάνεις deploy του `portal` στο `preprod` του αντιγράφου εξάσκησης. Το `--dry-run` δείχνει τι θα συνέβαινε και δεν αλλάζει τίποτα. Το `--ack` καταγράφει ποιος το έκανε· η εντολή συμπληρώνει το όνομά σου από το git. Μετά το deploy, το `status` πρέπει να δείξει και τα δύο μηχανήματα ως `ok` και converged.
tutor-l3-s1 = Σχεδίασε το deploy
tutor-l3-s2 = Κάνε deploy το portal στο preprod
tutor-l3-s3 = Έλεγξε ότι και τα δύο μηχανήματα είναι converged
tutor-l4-title = Απόκλιση και ιστορικό
tutor-l4-text =
    Δύο εντολές σε βοηθάνε να καταλάβεις ένα goatnet πριν και μετά από μια αλλαγή. Οι δύο μόνο διαβάζουν.
    - Το goatbuilder `diff` απαριθμεί κάθε μηχάνημα του οποίου η καταγραφή διαφέρει από τη δηλωμένη έκδοση. Μια τέτοια διαφορά λέγεται απόκλιση.
    - Το goatbuilder `history` απαριθμεί κάθε έκδοση που είχε ένα component σε κάθε μηχάνημα, η πιο πρόσφατη πρώτη.

    Στο `dev`, κάθε μηχάνημα είναι πίσω. Το ιστορικό του `portal` στο `preprod` τώρα περιλαμβάνει το deploy σου από το μάθημα 3.
tutor-l4-s1 = Απαρίθμησε την απόκλιση στο dev
tutor-l4-s2 = Διάβασε το ιστορικό του portal στο preprod
tutor-l5-title = Κάνε rollback, και δες ένα μη ασφαλές rollback να απορρίπτεται
tutor-l5-text =
    Ένα rollback δηλώνει μια παλιότερη έκδοση και την κάνει deploy όπως κάθε άλλη έκδοση. Το goatbuilder CLI μετά διαβάζει την καταγραφή κάθε μηχανήματος για να αποδείξει ότι έφτασε.

    Μερικές αλλαγές δεν μπορούν να αναιρεθούν με αυτόν τον τρόπο. Το deploy μιας παλιότερης έκδοσης δεν μπορεί να αναιρέσει δεδομένα που έχει γράψει ένα component, ένα πιστοποιητικό, ή μια αλλαγή που κινείται μόνο προς τα εμπρός. Το goatbuilder CLI τα αρνείται με κωδικό εξόδου 3 και λέει τι να κάνεις αντί για αυτό.

    Το component `db-schema` του αντιγράφου εξάσκησης κρατάει δεδομένα, οπότε το goatbuilder αρνείται να το κάνει rollback. Η άρνηση είναι το αναμενόμενο αποτέλεσμα.
tutor-l5-s1 = Σχεδίασε το rollback
tutor-l5-s2 = Κάνε rollback το portal μία έκδοση πίσω
tutor-l5-s3 = Δοκίμασε να κάνεις rollback το db-schema, που το goatbuilder αρνείται
tutor-l6-title = Κάνε release μια νέα έκδοση: dev, μετά preprod, μετά prod
tutor-l6-s1 = Δες πού βρίσκεται κάθε component
tutor-l6-s2 = Κάνε dry-run το release από το dev
tutor-l6-s3 = Κάνε dry-run το βήμα του prod
tutor-l7-title = Fastlane, και πότε να μην το χρησιμοποιείς
tutor-l7-text =
    Το Fastlane βάζει μία έκδοση σε κάθε goatnet ταυτόχρονα. Προσπερνά την αναμονή στο preprod και τις καταγραφές του release.

    Χρησιμοποίησέ το μόνο για μια επείγουσα διόρθωση που μπορείς να αναιρέσεις γυρίζοντας στην προηγούμενη έκδοση. Μην το χρησιμοποιείς ποτέ για μια αλλαγή βάσης δεδομένων, μια αλλαγή ταυτότητας ή πρόσβασης, ή οτιδήποτε δεν μπορείς να αναιρέσεις με αυτόν τον τρόπο.

    Το Fastlane αρνείται μια έκδοση που το CI δεν έχει χτίσει, και ένα component που είναι μέρος ενός release που βρίσκεται ακόμα σε εξέλιξη.

    Η γενική μορφή είναι `goatbuilder fastlane <component> --ref <version> --reason <why>`. Τρέξε το παράδειγμα παρακάτω. Έχει `--dry-run`, οπότε τίποτα δεν αλλάζει. Χρειάζεται `gh`.
tutor-l7-s1 = Κάνε dry-run ένα fastlane
tutor-l8-title = Pins: βρες την απόκλιση και διόρθωσέ τη με τον σωστό τρόπο
tutor-l8-text =
    Ένα αρχείο ορίζει ποια έκδοση τρέχει κάθε goatnet: το `ops/pins/fleet-pins.yaml`. Το goatbuilder CLI παράγει τα άλλα αρχεία έκδοσης από αυτό: τα αρχεία `pins.yml` και τα υπογεγραμμένα αρχεία intent.

    Το goatbuilder `pins --check` συγκρίνει τα παραγμένα αρχεία με το `fleet-pins.yaml` και αρνείται κάθε αρχείο που διαφέρει. Το goatbuilder `pins` τα αναδημιουργεί.

    Ο φάκελος μαθημάτων σου έχει ένα αντίγραφο του repository στο οποίο κάποιος επεξεργάστηκε ένα αρχείο `pins.yml` με το χέρι. Μην διορθώνεις ποτέ μια επεξεργασία με το χέρι με άλλη επεξεργασία με το χέρι. Άλλαξε το `fleet-pins.yaml`, και μετά αναδημιούργησε.
tutor-l8-s1 = Έλεγξε αυτό το checkout
tutor-l8-s2 = Έλεγξε το αντίγραφο και βρες την επεξεργασία με το χέρι
tutor-l8-s3 = Αναδημιούργησε τα αρχεία του αντιγράφου
tutor-l8-s4 = Έλεγξε το αντίγραφο ξανά
tutor-l9-title = Εξήγησε ένα σφάλμα
tutor-l9-s2 = Εξήγησε το σφάλμα με τον κωδικό του
tutor-l9-s3 = Εξήγησε το σφάλμα με το κείμενό του
tutor-l10-title = Tutorial 1: χτίσε το πρώτο site ενός νέου goatnet
tutor-l10-text =
    Αυτό το μάθημα χτίζει ένα ολόκληρο goatnet σε ένα αντίγραφο εξάσκησης. Το goatnet παράδειγμα είναι το `zagros`. Το πρώτο του site, το `alpha`, έχει δύο βασικά μηχανήματα, ένα μηχάνημα παρακολούθησης και ένα μηχάνημα διάσωσης.

    Το goatbuilder `standup` στήνει κάθε μηχάνημα με σταθερή σειρά και ελέγχει κάθε καταγραφή. Η τελευταία του γραμμή λέει ότι το site είναι έτοιμο. Σε πραγματικά μηχανήματα, αυτή η γραμμή δείχνει και πόση ώρα χρειάστηκε.

    Το πλήρες tutorial είναι το `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Χτίσε το πρώτο site, alpha
tutor-l10-s2 = Έλεγξε το αποτέλεσμα
tutor-l11-title = Tutorial 2: πρόσθεσε ένα δεύτερο site
tutor-l11-text =
    Αυτό το μάθημα ξεκινά από εκεί που τελείωσε το tutorial 1. Προσθέτει ένα δεύτερο site, το `beta`, σε ένα νέο αντίγραφο εξάσκησης, το `zagros-2`.

    Ένα δεύτερο site αντιγράφει τους λογαριασμούς χρηστών από το πρώτο site. Το standup του αλλάζει επίσης το πρώτο site μία φορά, ώστε η αντιγραφή να λειτουργεί και προς τις δύο κατευθύνσεις. Ένα βήμα με όνομα `alpha/core` το κάνει αυτό.

    Το πλήρες tutorial είναι το `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Χτίσε το δεύτερο site, beta
tutor-l11-s2 = Έλεγξε και τα δύο site
tutor-l12-s1 = Σχεδίασε να προσθέσεις ένα μηχάνημα
tutor-l13-s1 = Σχεδίασε μια ομοσπονδία
tutor-l14-title = Κράτα το goatbuilder ενημερωμένο
tutor-l14-s1 = Ρώτησε αν υπάρχει νεότερο goatbuilder
tutor-l15-title = Assay: έλεγξε πώς χτίστηκε ένα goatnet
tutor-l15-text =
    Το goatbuilder `assay` ελέγχει ότι ένα goatnet χτίστηκε σωστά, ένα επίπεδο τη φορά, χρησιμοποιώντας τις δοκιμές αποδοχής. Κάθε δοκιμή παίρνει ένα από τρία αποτελέσματα:
    - PASS: η δοκιμή πέτυχε.
    - FAIL: το goatnet έχει πρόβλημα.
    - UNKNOWN: τίποτα δεν μπόρεσε να απαντήσει στη δοκιμή.

    Μόνο το PASS μετράει. Κάθε FAIL ή UNKNOWN κάνει τον κωδικό εξόδου 3.

    Το `--plan` απαριθμεί τις δοκιμές που ισχύουν για ένα goatnet, ανά επίπεδο, και δεν αγγίζει κανένα μηχάνημα. Χρησιμοποίησέ το για να δεις τι θα κάλυπτε ένα πραγματικό assay. Η γενική μορφή είναι `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: είναι οι εκδόσεις συνεπείς;
tutor-l16-text =
    Το goatbuilder `survey` ελέγχει σε ένα πέρασμα αν οι ίδιες εκδόσεις τρέχουν εκεί που πρέπει:
    - Μέσα σε κάθε goatnet: τρέχει κάθε μηχάνημα τη δηλωμένη έκδοση, και τα μηχανήματα που πρέπει να ταιριάζουν ταιριάζουν πράγματι;
    - Ανάμεσα σε dev, preprod και prod: μία γραμμή ανά component, μία στήλη ανά goatnet.

    Μια διαφορά που περιμένει η διαδικασία release αναφέρεται αλλά δεν είναι σφάλμα. Ένα παράδειγμα είναι μια νέα έκδοση που είναι στο dev αλλά όχι ακόμα στο prod. Μια διαφορά που δεν δηλώνει τίποτα διαβάζεται ως `difference not declared`. Ένα μηχάνημα που το goatbuilder δεν μπορεί να διαβάσει είναι unknown, ποτέ συνεπές.

    Τρέξε το στο αντίγραφο εξάσκησης. Το `dev` του είναι πίσω επίτηδες, οπότε περίμενε ευρήματα και κωδικό εξόδου 3.
tutor-l16-s1 = Κάνε survey το αντίγραφο εξάσκησης
tutor-l17-title = Endstate: τρέχει ένα goatnet ακριβώς αυτό που δηλώνει;
tutor-l17-text =
    Το goatbuilder `endstate` ελέγχει δύο πράγματα για ένα goatnet:
    1. Κάθε υπηρεσία που δηλώνει είναι deployed, τρέχει, είναι υγιής και σωστά συνδεδεμένη.
    2. Τίποτα δεν τρέχει που δεν δηλώνει. Μια υπηρεσία που δεν έχει δηλωθεί λέγεται ορφανή.

    Το goatbuilder CLI υπολογίζει τι πρέπει να τρέχει ένα site από τις δικές του ρυθμίσεις, όχι από έναν ξεχωριστό κατάλογο.

    Ένας έλεγχος που δεν έχει τίποτα να διαβάσει σημαδεύεται ως UNOBSERVED. Αυτή είναι μια ειλικρινής απάντηση, όχι αποτυχία. Ένα FAIL ή μια ορφανή υπηρεσία κάνει τον κωδικό εξόδου 3.

    Η γενική μορφή είναι `goatbuilder endstate <goatnet>`. Η εντολή παρακάτω ελέγχει ένα παράδειγμα site από αποθηκευμένα δεδομένα, οπότε δεν αγγίζει κανένα μηχάνημα.
tutor-l17-s1 = Έλεγξε το παράδειγμα site
tutor-l18-title = Μπόνους: να είσαι περίεργος
tutor-l18-text =
    Αυτό το μάθημα είναι προαιρετικό. Τρέξε την εντολή παρακάτω και τα μαθήματα τελειώνουν.

    Το goatbuilder CLI έχει μερικά μικρά κρυφά χαρακτηριστικά. Δεν είναι στη βοήθεια ή σε αυτά τα μαθήματα. Μερικά απαντούν σε λέξεις που οι παλιοί χρήστες του Unix συχνά πληκτρολογούν σε ένα νέο πρόγραμμα, απλώς για να δουν τι θα γίνει. Τα περισσότερα πράγματα που θα δοκιμάσεις δεν θα κάνουν τίποτα.

    Αυτό το μάθημα δεν λέει τι είναι. Το να τα βρεις είναι το νόημα.

    Η εντολή παρακάτω απαριθμεί κάθε σφάλμα που το goatbuilder μπορεί να εξηγήσει. Είναι ένα καλό σημείο για να ξεκινήσεις να εξερευνάς.
tutor-l18-s1 = Απαρίθμησε κάθε σφάλμα που το goatbuilder μπορεί να εξηγήσει
tutor-heading = Μάθημα { $n } από { $total } · { $title }
tutor-setup = Πρώτα, τρέξε αυτό μία φορά σε αυτό το τερματικό:
tutor-not-recording = Αυτό το τερματικό δεν καταγράφει τις εκτελέσεις σου, οπότε ο tutor δεν μπορεί να σημειώσει τα βήματά σου. Τρέξε αυτό μία φορά, και μετά τρέξε τα βήματα:
tutor-type = Κάνε αυτά τα βήματα στο τερματικό σου ({ $met } από { $total } έγιναν):
tutor-rerun = Μετά από κάθε βήμα, τρέξε ξανά το `goatbuilder tutor`. Ελέγχει τι έκανες και σημειώνει το βήμα.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Καλώς ήρθες στο GOATBUILDER Tutor
tutor-bar-progress = Πρόοδος
tutor-bar-done = { $done } από { $total } μαθήματα έγιναν
tutor-after-step = Βήμα { $met } από { $total } έγινε. Επόμενο:
tutor-after-lesson = Το μάθημα { $n } ολοκληρώθηκε. Τρέξε `goatbuilder tutor` για να ξεκινήσεις το μάθημα { $next }. Για να το διαβάσεις ξανά: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Το μάθημα { $n } ολοκληρώθηκε, και το ίδιο και ο tutor. Τρέξε `goatbuilder tutor` για να το δεις σημειωμένο ως έγινε. Για να το διαβάσεις ξανά: `goatbuilder tutor --lesson { $n }`.

tutor-review = Ολοκλήρωσες αυτό το μάθημα, οπότε αυτή η σελίδα είναι μόνο για ανάγνωση. Τρέξε `goatbuilder tutor` για να γυρίσεις εκεί που ήσουν.
tutor-other-lessons = Για να διαβάσεις ένα μάθημα ξανά: `goatbuilder tutor --lesson 1`. Για να τα δεις όλα: `goatbuilder tutor --list`.
tutor-l12-title = Προσθήκη ενός μηχανήματος σε ένα site που τρέχει
tutor-l12-text =
    Το goatbuilder `join-node` προσθέτει ένα ακόμα μηχάνημα σε ένα site που τρέχει ήδη. Τυπώνει πρώτα ένα σχέδιο, με μία γραμμή για κάθε φάση. Κάθε φάση έχει έναν τύπο:
    - tool: το goatbuilder τρέχει ένα script.
    - ansible: το goatbuilder τρέχει ένα playbook.
    - gate: έναν έλεγχο που πρέπει να περάσει πριν από την επόμενη φάση.
    - manual: το κάνεις μόνος σου, και η εκτέλεση παύει.

    Η εντολή από κάτω έχει `--dry-run`. Διαβάζει αυτό το checkout και δεν αλλάζει τίποτα. Κάτω από τον πίνακα, διάβασε τι κάνεις με το χέρι.
tutor-l13-title = Σύνδεση με το goatnet άλλου οργανισμού
tutor-l13-text =
    Το goatbuilder `federate` συνδέει το goatnet σου με ένα goatnet που τρέχει άλλος οργανισμός. Κάθε πλευρά φτιάχνει το δικό της μισό της συμφωνίας. Ανταλλάσσεις τα μισά, και κάθε πλευρά τα συνδυάζει στην ίδια συμφωνία.

    Επιβεβαίωσε το κλειδί της άλλης πλευράς με άλλο τρόπο, όπως ένα τηλεφώνημα που εμπιστεύεσαι. Ένα κλειδί που φτάνει μέσα στο πακέτο τους δεν αποδεικνύει τίποτα.

    Η εντολή από κάτω έχει `--dry-run`, οπότε δεν αλλάζει τίποτα. Μια πραγματική εκτέλεση σταματά στη φάση 4, επειδή αυτή τη φάση την κάνεις με το χέρι.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Αυτή η εντολή δεν είναι μέρος του μαθήματος { $n }. Τρέξε `goatbuilder tutor` για να δεις πού βρίσκεσαι.
tutor-l1-s2 = Ρώτησε το doctor για ένα goatnet
tutor-l1-text =
    Το goatbuilder CLI κάνει build, deploy και release του λογισμικού GOAT. Πριν το χρησιμοποιήσεις, έλεγξε τη ρύθμισή σου με δύο εντολές:
    - Το goatbuilder `validate` ελέγχει το αρχείο ρυθμίσεων, το `ops/goatbuilder/goatbuilder.yaml`. Διαβάζει μόνο αυτό το checkout και δεν χρειάζεται δίκτυο.
    - Το goatbuilder `doctor` ελέγχει τι χρειάζεται ένα πραγματικό deploy: τα εργαλεία σου, τις συνδέσεις σου, αν αυτό το checkout ταιριάζει με το main, και πώς το goatbuilder φτάνει σε κάθε goatnet. Ένα goatnet είναι ένα περιβάλλον GOAT.

    Το goatbuilder `doctor` τυπώνει πόσοι έλεγχοι πέρασαν, μετά κάθε πρόβλημα με τη διόρθωσή του. Σε αυτό το μάθημα ένα πρόβλημα είναι πληροφορία, όχι λάθος. Διάβασε τις διορθώσεις, αλλά δεν χρειάζεται να τις εφαρμόσεις τώρα.

    Τα περισσότερα μαθήματα δεν χρειάζονται σύνδεση. Τα μαθήματα 2 έως 5, 10, 11 και 16 χρησιμοποιούν ένα αντίγραφο εξάσκησης. Τα μαθήματα 8, 9, 12, 13, 15, 17 και 18 μόνο διαβάζουν αυτό το checkout. Το μάθημα 6 χρειάζεται `gh` και το δίκτυο, το μάθημα 7 χρειάζεται `gh`, και το μάθημα 14 χρειάζεται το δίκτυο. Κανένα μάθημα δεν χρειάζεται 1Password.
tutor-l14-text =
    Το goatbuilder CLI μπορεί να ενημερώσει μόνο του τον εαυτό του από το αντίγραφο που σερβίρει το goatnet σου στο `get.<site>.<zone>/goatbuilder/`.

    Το goatbuilder `update --check` μόνο διαβάζει. Συγκρίνει την έκδοσή σου με την έκδοση που σερβίρεται και δεν αλλάζει τίποτα. Κωδικός εξόδου 0 σημαίνει ότι είσαι ενημερωμένος. Κωδικός εξόδου 3 σημαίνει ότι υπάρχει νεότερη έκδοση.

    Μια πραγματική ενημέρωση, χωρίς `--check`, επαληθεύει το checksum και την υπογραφή της λήψης πριν αντικαταστήσει οτιδήποτε. Μετά αντικαθιστά το αρχείο σε ένα βήμα.

    Το goatbuilder CLI πρέπει να ξέρει ποιο goatnet να ρωτήσει: πέρασε `--site` με αυτό το goatnet, ή όρισε `GOATBUILDER_UPDATE_URL`. Πρέπει να είσαι στο δίκτυο αυτού του goatnet. Αν δεν είσαι, το `--check` λέει ότι δεν μπορεί να φτάσει τον server, και ονομάζει τη διεύθυνση που δοκίμασε. Αυτό είναι το σωστό αποτέλεσμα για αυτό το μάθημα.
tutor-l15-s1 = Απαρίθμησε τις δοκιμές που ισχύουν για ένα goatnet
tutor-l6-text =
    Ένα release μετακινεί μια νέα έκδοση μέσα από τρία goatnet με σειρά: dev, μετά preprod, μετά prod.

    1. Κάποιος κάνει tag την έκδοση στο main, και το CI την χτίζει.
    2. dev: το goatbuilder την κάνει deploy και τρέχει τις δοκιμές αποδοχής.
    3. preprod: το goatbuilder την κάνει deploy ξανά, τρέχει τις δοκιμές ξανά και καταγράφει τα αποτελέσματα.
    4. Μετά η έκδοση περιμένει στο preprod για συγκεκριμένο χρόνο. Τίποτα δεν περνάει στο prod μόνο του.
    5. prod: μετά αυτή την αναμονή, ένα άτομο τρέχει το release με `--from prod`. Το goatbuilder CLI κάνει deploy στο prod και ελέγχει όλα τα τρία goatnet.

    Το goatbuilder `promote status` δείχνει πού βρίσκεται κάθε component. Ένας υποψήφιος είναι μια έκδοση που δοκιμάζεται. Η γενική μορφή ενός release είναι `goatbuilder release <component> --tag <tag> --from dev`. Οι εντολές παρακάτω χρησιμοποιούν ένα component αυτού του checkout και ένα παράδειγμα tag. Κάθε μία έχει `--dry-run`, οπότε τίποτα δεν γίνεται tag ή deploy.

    Οι δοκιμαστικές εκτελέσεις χρειάζονται `gh` και το δίκτυο. Αν δεν είσαι διαχειριστής του repository, βάλε `GOAT_RC_ROLE_CHECK=0` πριν από κάθε εντολή `release`.
tutor-l9-s1 = Φτιάξε ένα πραγματικό σφάλμα με ένα goatnet που δεν υπάρχει
tutor-l9-text =
    Όταν το goatbuilder αρνείται, τυπώνει έναν κωδικό. Κωδικοί που ξεκινούν με `GB-` έρχονται από το goatbuilder, `E_` από το goat CLI, και `X-` από ένα άλλο εργαλείο που έτρεξε το goatbuilder.

    Το goatbuilder `explain` παίρνει έναν κωδικό, ή το κείμενο μιας γραμμής σφάλματος, και σου λέει τι συνέβη, γιατί, πώς να το διορθώσεις, και πού να διαβάσεις περισσότερα.

    Αν ζητήσεις ένα goatnet που αυτό το checkout δεν δηλώνει, φτιάχνεις ένα πραγματικό σφάλμα για εξήγηση.
