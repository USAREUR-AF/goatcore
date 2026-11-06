### goatbuilder — de (de)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Eine goat-Umgebung bauen und ausliefern: deklarieren, konvergieren, verifizieren – unter einem Deploy-Vertrag
help-flag-output = Ausgabeformat: Tabelle für Menschen; JSON oder YAML für Programme
help-flag-repo = Das Repository-Root. Standard: der Checkout, in dem dieses Binary läuft
help-flag-yes = Ein deklaratives Verb ohne Rückfrage bestätigen (für Automatisierung)
help-flag-frozen = Jeden Lauf verweigern, der den Checkout fast-forwarden müsste (striktes C3)
help-flag-mock = Gegen ein Fixture-Verzeichnis statt gegen Hosts laufen: keine Registry, kein Transport, deterministisch (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Sprache für Hilfe und Fehlererklärungen (ein Tag wie fr, uk, de); was eine Pipeline liest, bleibt Englisch. Standard: die OS-Locale
help-cmd-validate = Eine Konfiguration gegen das versionierte Schema und die Cross-Field-Regeln prüfen, offline
help-cmd-doctor = Prüfen, was ein Lauf braucht, bevor etwas geschnitten wird: Abhängigkeiten, Aktualität des Checkouts, Transport jeder Umgebung und ob das Playbook jeder Komponente auf einen eindeutigen Target-Mode auflöst
help-cmd-status = Deklariert gegen aufgezeichnet, pro Maschine: converged, pending oder unknown (C7, C8). Nur lesend. Exit 3, solange eine Maschine nicht converged ist
help-cmd-deploy = Den Pin, den main deklariert, auf eine Umgebung bringen und beweisen: Komponente und Gruppen der Umgebung aus den Pins auflösen, dazu das Play und seinen Target-Mode, jedes Secret, das das Play templatet, und den Transport; das Play über ansible-preflight ausführen; das Deploy-Ledger von jedem Host zurücklesen und erst als fertig werten, wenn die neueste Zeile den deklarierten Digest mit action=deploy nennt (C7). Auf einem Fixture (`--mock`) ersetzt der geskriptete Converger die Hosts
help-cmd-release = Einen Release Candidate von einem Tag auf main durch die Live-Validierung des dev-Goatnets bis zur preprod-Stufe bringen (ADR 1173); nach dem Soak mit `--from prod` graduieren, in fleet: einfalten, prod deployen und jede Stufe erneut verifizieren. Ersetzt ops/scripts/rc.sh Schritt für Schritt
help-cmd-rollback = Eine deployte Komponente auf ihren N-letzten Ref zurückrollen, nach Artefakttyp, mit dem Ledger auf dem Host als Source of Truth (ADR 1060). Ersetzt ops/scripts/rollback.sh: Exit 3 abgelehnt als unsicher (data, cert, forward-only, reversible:false), 4 geführt (config, acl)
help-cmd-fastlane = Ein Befehl von einem gemergten Commit zu jedem Goatnet (ADR 1127): die bewusste Ausnahme von der Ladder. Ersetzt ops/scripts/fastlane.sh Schritt für Schritt; lehnt einen ungepinnten Digest oder ein offenes Graduation-Set ab, und fährt die Deploys über dieses Binary
help-cmd-standup = Ein Standort von null bis verifiziert: die Phasen des Standups in der Reihenfolge der Doktrin, mit einem fail-closed Gate an jedem Ende, der Observability-Host als paralleler Zweig, die Rescue-Klassen und die Cross-Goatnet-Oberflächen zuletzt (ADR 1183). Ersetzt ops/scripts/goat-standup.sh Schritt für Schritt. Auf einem Fixture (`--mock`) wird jede Komponente jeder Klasse vom geskripteten Converger konvergiert, und die Zeit von deklariert → verifiziert ist das Maß des One-Hour-Goatnets
help-cmd-join-node = Eine weitere Maschine einer Klasse an einem Live-Standort, deklariert → konvergiert → verifiziert: die ausgeführte Sequenz aus docs/tutorials/02-first-federated-node.md als nummerierte Phasen mit ihren Gates (Erreichbarkeit, Key, Repo-Arbeit zuerst, Baseline, Kernel-WireGuard vor und nach dem Enrolment, die Kernel-ACL-Ebene als Enrolment-Gate, das Converge der Klasse, Abnahme)
help-cmd-federate = Ein souveränes Peering deklariert, konvergiert und verifiziert (ADR 1157): unsere Hälfte gemessen, die Pakete out of band ausgetauscht, eine Vereinbarung zusammengesetzt und verifiziert, die Leaves daraus geminted, die Regeln vom einen Writer gerendert, die Edge deployt, der von der Vereinbarung gewählte Transport geöffnet, Abnahme in allen vier Tiers. Ersetzt ops/scripts/peering-standup.sh Schritt für Schritt
help-cmd-pins = EIN Resolution-Pass mit ZWEI Renderings: die group_vars pins.yml jedes gelisteten Members und das IntentV1-Artefakt jedes gelisteten Goatnets, aus ops/pins/fleet-pins.yaml. Ersetzt ops/pins/generate-pins.py byte-genau. `--check` validiert, regeneriert und lehnt Drift ab, ebenso einen unsignierten oder manipulierten Intent und einen governed Pin, der außerhalb von pins.yml definiert ist (Exit 1)
help-cmd-promote = Release-Graduation über die Goatnet-Ladder führen und beobachten (ADR 1120, 1125): Status, Candidate öffnen, graduieren, ein Set graduieren, der Hotfix-Pfad, Review, Readiness, Abandon, Check. Ersetzt ops/release/graduate.py Schritt für Schritt. Nichts hier graduiert von selbst: eine Mutation läuft nur auf Aufruf, gibt aus, was sie getan hat, und verlangt ohne Terminal --yes (C10, ADR 1177)
help-cmd-explain = Einen Fehler deuten: einen Code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) oder beliebigen Fehlertext – den dieses Binarys oder die Worte eines Tools, die es durchgereicht hat (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Gibt aus, was passiert ist, warum, was zu tun ist und wo man mehr liest; `-` liest den Text von stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Ein Standort von „verifiziert" bis nichts — das Gegenstück zu standup, Aufzeichnungen bleiben (ein Rückbau hängt Teardown-Zeilen an, löscht nie etwas). Auf einem Fixture (`--mock`) wird der Rückbau in umgekehrter Phasenreihenfolge geprobt; auf echten Hosts wird die Reihenfolge ausgegeben und die Ausführung verweigert, bis jede Phase ihr Teardown-Play deklariert (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lektionen im vimtutor-Stil: Jede nennt den genauen Befehl, der gegen eine Wegwerfkopie eines Fixtures (oder schreibgeschützt gegen diesen Checkout) auszuführen ist, und prüft dann, was der Befehl tatsächlich getan hat — Exit-Code und Zeilen —, bevor die nächste freigeschaltet wird. `tutor` zeigt die aktuelle Lektion und markiert sie als erledigt, sobald deine Läufe sie erfüllen; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Versteckt (src/fun.rs)
help-cmd-argan = Versteckt (src/fun.rs)
help-cmd-bound = Versteckt (crates/goat-fun)
help-cmd-secrets = Die deklarierte Secrets-Haltung (ADR 1204): welcher Store gilt (1password, openbao, sops, git-crypt, env-file, env), was verfügbar ist, wie Fehlendes bereitgestellt wird, und die eigenen Prüfungen des Stores. `init --env <goatnet>` schreibt die Env-Datei für den Zero-SaaS-Schnellstart: einen starken Zufallswert für jede Variable, die die Plays des Goatnets templaten, 0600, per gitignore ausgeschlossen, ein vorhandener Wert wird nie rotiert. Gleiches Modul wie ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Liest für eine Komponente die neueste Deploy-Ledger-Zeile jedes Hosts und prüft, ob sie den Ref nennt, den main deklariert (C7). Nur lesend; Exit 3, solange ein Host ausstehend oder unbekannt ist. Dieselbe Prüfung wie `deploy --verify-only`
help-cmd-diff = Was sich gerade von main unterscheidet (C8): der deklarierte Ref von main gegen die neueste Ledger-Zeile jedes Hosts, für eine Komponente oder, ohne Angabe, für jede Komponente der Umgebung, in einem Durchlauf je Mitgliedsgruppe gelesen. Nur lesend; Exit 3 bei jeder Abweichung oder jedem nicht lesbaren Host
help-cmd-history = Das Deploy-Ledger als Zeitleiste, neueste zuerst, auf jedem Host der Umgebung (C8): jede Zeile ist ein eigener Eintrag mit der Maschine, der Zeit, der Aktion (deploy oder rollback) und der Version. Die vorherige Version des Eintrags und der Operator, der ihn ausgeführt hat, stehen in `-o json`. Nur lesend
help-cmd-plan = Schreibt in eine Plandatei zur Prüfung, was ein deklaratives Verb tun würde: die deklarierten Refs, Play und Ziel, das Ledger, den Commit von main, den Transport (deploy); den Artefakttyp und den Ziel-Ref (rollback). Berührt keinen Host-Zustand. Mit `apply` wie geprüft ausführen
help-cmd-apply = Führt genau einen geprüften Plan aus. Verweigert (Exit 1, mit Angabe, was sich bewegt hat), wenn sich seit dem Schreiben des Plans die Deklaration von main, der Pins-Commit, das Play oder das Rollback-Ziel verändert haben; ein geänderter Transport wird gemeldet, nicht verweigert
help-cmd-adopt = Deklariert, was eine Umgebung bereits ausführt: liest die Ledger jedes Hosts und schlägt für jede Komponente, deren laufender Ref von der Deklaration in main abweicht, eine Pins-Abweichung vor. Standardmäßig nur ein Bericht; mit --yes werden die Änderungen zur Prüfung in einen frischen Worktree von main geschrieben. Deployt nie

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Noch nicht: { $why }
tutor-lesson-done = Lektion { $n } erledigt: { $title }.
tutor-all-done-reset = Alle Lektionen sind erledigt. `goatbuilder tutor --reset` beginnt von vorn; die Tutorials gehen weiter: docs/tutorials/.
tutor-all-done = Alle Lektionen sind erledigt. Die Tutorials gehen weiter: docs/tutorials/.
tutor-workspace = Workspace: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Prüft, ob ein goatnet korrekt aufgebaut wurde, Schicht für Schicht: jede Abnahmeprüfung, die für den Standort gilt, einschließlich der Prüfungen, die ein Operator mit der goat-CLI ausführt. Die Abnahmephase des Standups als eigener Befehl. Nur lesend; Exit 3, solange eine Prüfung fehlschlägt oder nicht antworten kann
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Standort im Inselbetrieb: Trunk nicht abrufen; Checkout gegen den zwischengespeicherten Trunk prüfen und den Lauf als offline protokollieren (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Prüft, ob ein goatnet seinen deklarierten Endzustand erreicht hat: Jeder Dienst, den die eigenen Deklarationen vorsehen, ist deployt, läuft, ist gesund und erfüllt seine Aufgabe, die Zero-Trust-Kanten sind verdrahtet, und es läuft nichts, was nicht deklariert ist. Nur lesend; Exit 3, solange etwas nicht stimmt
help-cmd-survey = Wie konsistent die Software innerhalb jedes goatnets und über die goatnets der Ladder hinweg ist, gelesen in einem parallelen Durchlauf. Liest nur

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Wie konsistent die Software innerhalb jedes goatnets und über alle goatnets der Ladder hinweg ist, erfasst in einem parallelen Durchlauf. Nur Lesezugriffe.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Ersetzt diese Binärdatei durch das Release, das das zugehörige goatnet unter `get.<site>.<zone>/goatbuilder/` bereitstellt: Der Download wird vor jedem Eingriff gegen die zugehörige `.sha256` und die signierte `SHA256SUMS` geprüft, der Austausch ist atomar. Bereits aktuell: eine Zeile, Exit 0. `--check` liest nur (Exit 3, wenn veraltet). Die Quelle: `GOATBUILDER_UPDATE_URL`, sonst `--site` aus den group_vars, sonst die, die das letzte Update verwendet hat
tutor-prereq = Vorab: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Färbt das ok/FAIL-Zeichen und das Fazit der Tabelle (auto|always|never). `auto` färbt nur ein Terminal; NO_COLOR und CLICOLOR_FORCE haben Vorrang. JSON und YAML werden nie gefärbt
help-flag-verbose = Protokolliert, was ein Lauf tut — die externen Befehle, die er aufruft, und die Netzwerkphase von `update` — nach stderr, damit `-o json|yaml` ein sauberes Dokument bleibt. Oder setze RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunden zur Begrenzung der Verbindungs-/Erreichbarkeitsphase vor schnellem Abbruch — die einzige Phase, die bei einem nicht erreichbaren Host blockieren kann (`update`s Abruf der bereitgestellten Version). Begrenzt NICHT den Lauf eines Plays oder den Binär-Download (durch Stillstand begrenzt). Liest auch GOATBUILDER_TIMEOUT
help-cmd-completion = Gibt ein Shell-Completion-Skript nach stdout aus (`goatbuilder completion zsh >> ~/.zshrc`). Für dynamische Kandidaten, die ein statisches Skript nicht kennen kann — die Sites, Komponenten und Goatnets, die dieser Checkout deklariert — binde einen Hook an `goatbuilder _complete` (siehe den Kopf des erzeugten Skripts)
help-cmd-docs = Die Befehlsreferenz: `goatbuilder docs` verweist darauf; `--markdown` gibt das Ganze aus, aus dem lebenden Befehlsbaum durchlaufen, sodass es nie von `--help` abweichen kann
help-cmd-_complete = Versteckt: gibt dynamische Completion-Kandidaten für Shell-Hooks aus, einen pro Zeile (die Sites, Komponenten und Goatnets, die dieser Checkout deklariert). Nur Lesen, scheitert nie lautstark — ein Completion-Hook darf die Eingabeaufforderung nicht beschädigen

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Vor einem Rollout: die Bereitschafts- und Rücknahmeauswertung (ADR 1211): drei Fakten — State (stateless | stateful), Rollback (pin-revert | restore-required | none) und Scope (single | coupled) — pro Komponente deklariert und FAIL-CLOSED, wenn nicht deklariert. Dazu die Bereitschaftshälfte, die die anderen Verben belegen (Ref abrufbar, pins --check, Secrets-Gate, frische Quelle, deklarierter Endzustand), und eine Validierung des Rollback-Fakts (rollback --dry-run bei pin-revert, Aufbewahrungsfrist bei restore-required). `deploy` führt es zuerst aus; ehrlichkeitsbewertet, nie ein falsches Grün. Liest nur; Exit 3, wenn nicht sauber

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Einrichtung prüfen
tutor-l1-s1 = Prüfe die Konfigurationsdatei
tutor-l2-title = Zustand lesen: converged, pending, unknown
tutor-l2-text =
    Das Kommando goatbuilder `status` vergleicht für jede Maschine zwei Dinge: die Version, die main dafür deklariert, und die Version, die laut ihrem eigenen Ledger läuft. Es ändert nichts.

    Jede Maschine ist in einem von drei Zuständen:
    - converged (konvergiert): ihr Ledger nennt die deklarierte Version. Das ist erledigt.
    - pending (ausstehend): ihr Ledger nennt eine ältere Version, oder noch keine.
    - unknown (unbekannt): goatbuilder kann ihr Ledger nicht lesen.

    Nur converged zählt als erledigt. Wenn eine Maschine pending oder unknown ist, endet `status` mit Code 3.

    Ab dieser Lektion zeigt `--mock` goatbuilder auf eine Übungskopie in deinem Tutor-Ordner statt auf echte Maschinen. Nichts, was du dort tust, erreicht eine echte Maschine.

    Auf das `dev` der Übungskopie wurde noch nichts deployt. Erwarte also vier Maschinen als pending und eine als unknown. Converged Maschinen siehst du in Lektion 3.
tutor-l2-s1 = Lies den Zustand von dev
tutor-l3-title = Deploy: planen, ausführen, beweisen
tutor-l3-text =
    Deployen bringt die deklarierte Version auf jede Maschine und liest dann das Ledger jeder Maschine, um zu beweisen, dass sie angekommen ist. Ein Deploy ist erst fertig, wenn das Ledger jeder Maschine diese Version nennt. Ein Deploy, das ohne Fehler lief, reicht nicht.

    Du deployst `portal` auf das `preprod` der Übungskopie. `--dry-run` zeigt, was passieren würde, und ändert nichts. `--ack` hält fest, wer es getan hat; der Befehl trägt deinen Namen aus git ein. Nach dem Deploy sollte `status` beide Maschinen als `ok` und converged zeigen.
tutor-l3-s1 = Plane den Deploy
tutor-l3-s2 = Deploye portal auf preprod
tutor-l3-s3 = Prüfe, dass beide Maschinen converged sind
tutor-l4-title = Drift und Verlauf
tutor-l4-text =
    Zwei Befehle helfen dir, ein goatnet vor und nach einer Änderung zu verstehen. Beide lesen nur.
    - Das Kommando goatbuilder `diff` listet jede Maschine, deren Ledger von der deklarierten Version abweicht. So eine Abweichung heißt Drift.
    - Das Kommando goatbuilder `history` listet jede Version, die eine Komponente auf jeder Maschine hatte, neueste zuerst.

    Auf `dev` liegt jede Maschine zurück. Der Verlauf von `portal` auf `preprod` enthält jetzt deinen Deploy aus Lektion 3.
tutor-l4-s1 = Liste die Drift auf dev
tutor-l4-s2 = Lies den Verlauf von portal auf preprod
tutor-l5-title = Zurückrollen, und sehen, wie ein unsicherer Rollback verweigert wird
tutor-l5-text =
    Ein Rollback deklariert eine frühere Version und deployt sie wie jede andere Version. Die goatbuilder CLI liest dann das Ledger jeder Maschine, um zu beweisen, dass sie angekommen ist.

    Manche Änderungen lassen sich so nicht rückgängig machen. Das Deployen einer älteren Version macht Daten, die eine Komponente geschrieben hat, nicht ungeschehen, ebenso wenig ein Zertifikat oder eine Änderung, die nur vorwärts geht. Die goatbuilder CLI weigert sich bei diesen mit Exit-Code 3 und sagt, was statt dessen zu tun ist.

    Die `db-schema`-Komponente der Übungskopie enthält Daten, daher weigert sich goatbuilder, sie zurückzurollen. Diese Weigerung ist das erwartete Ergebnis.
tutor-l5-s1 = Plane den Rollback
tutor-l5-s2 = Rolle portal eine Version zurück
tutor-l5-s3 = Versuche, db-schema zurückzurollen, was goatbuilder verweigert
tutor-l6-title = Eine neue Version releasen: dev, dann preprod, dann prod
tutor-l6-s1 = Sieh, wo jede Komponente steht
tutor-l6-s2 = Führe den Release von dev als Probelauf aus
tutor-l6-s3 = Führe den prod-Schritt als Probelauf aus
tutor-l7-title = Fastlane, und wann du es nicht verwenden solltest
tutor-l7-text =
    Fastlane bringt eine Version auf einmal auf jedes goatnet. Es überspringt die Wartezeit auf preprod und die Release-Ledger.

    Nutze es nur für eine dringende Korrektur, die du durch Zurückgehen zur vorigen Version wieder rückgängig machen kannst. Nutze es nie für eine Datenbankänderung, eine Identitäts- oder Zugriffsänderung, oder irgendetwas, das du so nicht rückgängig machen kannst.

    Fastlane weigert sich bei einer Version, die CI nicht gebaut hat, und bei einer Komponente, die Teil eines noch laufenden Releases ist.

    Die allgemeine Form ist `goatbuilder fastlane <component> --ref <version> --reason <why>`. Führe das Beispiel unten aus. Es hat `--dry-run`, also ändert sich nichts. Es braucht `gh`.
tutor-l7-s1 = Führe ein Fastlane als Probelauf aus
tutor-l8-title = Pins: Drift finden und richtig beheben
tutor-l8-text =
    Eine Datei legt fest, welche Version jedes goatnet läuft: `ops/pins/fleet-pins.yaml`. Die goatbuilder CLI erzeugt die anderen Versionsdateien daraus: die `pins.yml`-Dateien und die signierten Intent-Dateien.

    Das Kommando goatbuilder `pins --check` vergleicht die erzeugten Dateien mit `fleet-pins.yaml` und weigert sich bei jeder Datei, die abweicht. Das Kommando goatbuilder `pins` erzeugt sie neu.

    Dein Tutor-Ordner hat eine Kopie des Repositorys, in der jemand eine `pins.yml`-Datei per Hand bearbeitet hat. Behebe eine Handbearbeitung nie mit einer weiteren Handbearbeitung. Ändere `fleet-pins.yaml`, und erzeuge dann neu.
tutor-l8-s1 = Prüfe dieses Checkout
tutor-l8-s2 = Prüfe die Kopie und finde die Handbearbeitung
tutor-l8-s3 = Erzeuge die Dateien der Kopie neu
tutor-l8-s4 = Prüfe die Kopie erneut
tutor-l9-title = Einen Fehler erklären
tutor-l9-s2 = Erkläre den Fehler über seinen Code
tutor-l9-s3 = Erkläre den Fehler über seinen Text
tutor-l10-title = Tutorial 1: den ersten Standort eines neuen goatnets bauen
tutor-l10-text =
    Diese Lektion baut ein ganzes goatnet in einer Übungskopie. Das Beispiel-goatnet ist `zagros`. Sein erster Standort, `alpha`, hat zwei Kernmaschinen, eine Monitoring-Maschine und eine Rettungsmaschine.

    Das Kommando goatbuilder `standup` richtet jede Maschine in fester Reihenfolge ein und prüft jedes Ledger. Seine letzte Zeile sagt, dass der Standort fertig ist. Auf echten Maschinen zeigt diese Zeile auch, wie lange es gedauert hat.

    Das vollständige Tutorial ist `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Baue den ersten Standort, alpha
tutor-l10-s2 = Prüfe das Ergebnis
tutor-l11-title = Tutorial 2: einen zweiten Standort hinzufügen
tutor-l11-text =
    Diese Lektion beginnt dort, wo Tutorial 1 endete. Sie fügt einen zweiten Standort hinzu, `beta`, in einer neuen Übungskopie, `zagros-2`.

    Ein zweiter Standort kopiert seine Benutzerkonten vom ersten Standort. Sein standup ändert dabei auch einmal den ersten Standort, damit das Kopieren in beide Richtungen funktioniert. Ein Schritt namens `alpha/core` macht das.

    Das vollständige Tutorial ist `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Baue den zweiten Standort, beta
tutor-l11-s2 = Prüfe beide Standorte
tutor-l12-s1 = Plane das Hinzufügen einer Maschine
tutor-l13-s1 = Plane eine Föderation
tutor-l14-title = goatbuilder aktuell halten
tutor-l14-s1 = Frag, ob ein neueres goatbuilder existiert
tutor-l15-title = Assay: prüfen, wie ein goatnet gebaut wurde
tutor-l15-text =
    Das Kommando goatbuilder `assay` prüft, dass ein goatnet korrekt gebaut wurde, eine Schicht nach der anderen, mit den Abnahmeprüfungen. Jede Prüfung bekommt eines von drei Ergebnissen:
    - PASS: die Prüfung ist bestanden.
    - FAIL: das goatnet hat ein Problem.
    - UNKNOWN: nichts konnte die Prüfung beantworten.

    Nur PASS zählt. Jedes FAIL oder UNKNOWN macht den Exit-Code 3.

    `--plan` listet die Prüfungen, die für ein goatnet gelten, nach Schicht, und berührt keine Maschine. Nutze es, um zu sehen, was ein echter Assay abdecken würde. Die allgemeine Form ist `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: sind die Versionen konsistent?
tutor-l16-text =
    Das Kommando goatbuilder `survey` prüft in einem Durchgang, ob dieselben Versionen dort laufen, wo sie sollen:
    - Innerhalb jedes goatnets: läuft jede Maschine die deklarierte Version, und stimmen Maschinen, die übereinstimmen sollten, tatsächlich überein?
    - Über dev, preprod und prod hinweg: eine Zeile pro Komponente, eine Spalte pro goatnet.

    Ein Unterschied, den der Release-Prozess erwartet, wird gemeldet, ist aber kein Fehler. Ein Beispiel ist eine neue Version, die auf dev ist, aber noch nicht auf prod. Ein Unterschied, den nichts deklariert, liest sich als `difference not declared`. Eine Maschine, die goatbuilder nicht lesen kann, ist unknown, niemals konsistent.

    Führe es auf der Übungskopie aus. Ihr `dev` liegt absichtlich zurück, erwarte also Befunde und Exit-Code 3.
tutor-l16-s1 = Führe survey auf der Übungskopie aus
tutor-l17-title = Endstate: läuft ein goatnet genau das, was es deklariert?
tutor-l17-text =
    Das Kommando goatbuilder `endstate` prüft zwei Dinge über ein goatnet:
    1. Jeder Dienst, den es deklariert, ist deployt, läuft, ist gesund und korrekt verbunden.
    2. Es läuft nichts, was es nicht deklariert. Ein nicht deklarierter Dienst heißt Orphan.

    Die goatbuilder CLI leitet aus den eigenen Einstellungen des Standorts ab, was ein Standort laufen lassen sollte, nicht aus einer getrennten Liste.

    Eine Prüfung, die nichts zu lesen hat, wird als UNOBSERVED markiert. Das ist eine ehrliche Antwort, kein Fehlschlag. Ein FAIL oder ein Orphan macht den Exit-Code 3.

    Die allgemeine Form ist `goatbuilder endstate <goatnet>`. Der Befehl unten prüft einen Beispiel-Standort aus gespeicherten Daten, berührt also keine Maschine.
tutor-l17-s1 = Prüfe den Beispiel-Standort
tutor-l18-title = Bonus: sei neugierig
tutor-l18-text =
    Diese Lektion ist optional. Führe den Befehl unten aus, und der Tutor ist fertig.

    Die goatbuilder CLI hat ein paar kleine versteckte Funktionen. Sie stehen nicht in der Hilfe oder in diesen Lektionen. Manche reagieren auf Wörter, die langjährige Unix-Nutzer oft in ein neues Programm tippen, nur um zu sehen, was passiert. Das meiste, was du versuchst, wird nichts tun.

    Diese Lektion sagt nicht, was sie sind. Sie zu finden ist der Sinn der Sache.

    Der Befehl unten listet jeden Fehler, den goatbuilder erklären kann. Das ist ein guter Ort, um mit dem Erkunden zu beginnen.
tutor-l18-s1 = Liste jeden Fehler, den goatbuilder erklären kann
tutor-heading = Lektion { $n } von { $total } · { $title }
tutor-setup = Führe das zuerst einmal in diesem Terminal aus:
tutor-not-recording = Dieses Terminal zeichnet deine Durchläufe nicht auf, daher kann der Tutor deine Schritte nicht abhaken. Führe das einmal aus, und führe dann die Schritte aus:
tutor-type = Führe diese Schritte in deinem Terminal aus ({ $met } von { $total } erledigt):
tutor-rerun = Führe nach jedem Schritt `goatbuilder tutor` erneut aus. Es prüft, was du gemacht hast, und hakt den Schritt ab.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Willkommen bei GOATBUILDER Tutor
tutor-bar-progress = Fortschritt
tutor-bar-done = { $done } von { $total } Lektionen erledigt
tutor-after-step = Schritt { $met } von { $total } erledigt. Weiter:
tutor-after-lesson = Lektion { $n } ist fertig. Führe `goatbuilder tutor` aus, um Lektion { $next } zu starten. Um sie erneut zu lesen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lektion { $n } ist fertig, und der Tutor auch. Führe `goatbuilder tutor` aus, um es als erledigt markiert zu sehen. Um sie erneut zu lesen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Du hast diese Lektion beendet, deshalb ist diese Seite nur zum Lesen. Führe `goatbuilder tutor` aus, um zurückzukehren, wo du warst.
tutor-other-lessons = Um eine Lektion erneut zu lesen: `goatbuilder tutor --lesson 1`. Um sie alle zu sehen: `goatbuilder tutor --list`.
tutor-l12-title = Eine Maschine zu einem laufenden Standort hinzufügen
tutor-l12-text =
    Das Kommando goatbuilder `join-node` fügt einem Standort, der schon läuft, eine weitere Maschine hinzu. Es gibt zuerst einen Plan aus, mit einer Zeile pro Phase. Jede Phase hat einen Typ:
    - tool: goatbuilder führt ein Skript aus.
    - ansible: goatbuilder führt ein Playbook aus.
    - gate: eine Prüfung, die vor der nächsten Phase bestehen muss.
    - manual: du machst das selbst, und der Lauf pausiert.

    Der Befehl unten hat `--dry-run`. Er liest dieses Checkout und ändert nichts. Unter der Tabelle liest du, was du von Hand tust.
tutor-l13-title = Mit dem goatnet einer anderen Organisation verbinden
tutor-l13-text =
    Das Kommando goatbuilder `federate` verbindet dein goatnet mit einem goatnet, das eine andere Organisation betreibt. Jede Seite baut ihre Hälfte einer Vereinbarung. Du tauschst die Hälften, und jede Seite fügt sie zur selben Vereinbarung zusammen.

    Bestätige den Schlüssel der anderen Seite auf einem anderen Weg, zum Beispiel einem Anruf, dem du vertraust. Ein Schlüssel, der innerhalb ihres Pakets ankommt, beweist nichts.

    Der Befehl unten hat `--dry-run`, also ändert er nichts. Ein echter Lauf stoppt bei Phase 4, weil du diese Phase von Hand machst.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Dieser Befehl ist nicht Teil von Lektion { $n }. Führe `goatbuilder tutor` aus, um zu sehen, wo du bist.
tutor-l1-s2 = Frag doctor nach einem goatnet
tutor-l1-text =
    Die goatbuilder CLI baut, deployt und releaset GOAT-Software. Bevor du es benutzt, prüfe deine Einrichtung mit zwei Befehlen:
    - Das Kommando goatbuilder `validate` prüft die Konfigurationsdatei, `ops/goatbuilder/goatbuilder.yaml`. Es liest nur dieses Checkout und braucht kein Netzwerk.
    - Das Kommando goatbuilder `doctor` prüft, was ein echter Deploy braucht: deine Tools, deine Logins, ob dieses Checkout zu main passt, und wie goatbuilder jedes goatnet erreicht. Ein goatnet ist eine GOAT-Umgebung.

    Das Kommando goatbuilder `doctor` zeigt, wie viele Prüfungen bestanden wurden, dann jedes Problem mit seiner Lösung. In dieser Lektion ist ein Problem eine Information, kein Fehler. Lies die Lösungen, musst sie aber jetzt noch nicht anwenden.

    Die meisten Lektionen brauchen keinen Login. Lektionen 2 bis 5, 10, 11 und 16 nutzen eine Übungskopie. Lektionen 8, 9, 12, 13, 15, 17 und 18 lesen nur dieses Checkout. Lektion 6 braucht `gh` und das Mesh, Lektion 7 braucht `gh`, und Lektion 14 braucht das Mesh. Keine Lektion braucht 1Password.
tutor-l14-text =
    Die goatbuilder CLI kann sich selbst aktualisieren, von der Kopie, die dein goatnet unter `get.<site>.<zone>/goatbuilder/` bereitstellt.

    Das Kommando goatbuilder `update --check` liest nur. Es vergleicht deine Version mit der bereitgestellten Version und ändert nichts. Exit-Code 0 heißt, du bist aktuell. Exit-Code 3 heißt, es gibt eine neuere Version.

    Eine echte Aktualisierung, ohne `--check`, prüft Checksumme und Signatur des Downloads, bevor sie irgendetwas ersetzt. Dann tauscht sie die Datei in einem Schritt aus.

    Die goatbuilder CLI muss wissen, welches goatnet es fragen soll: gib `--site` mit diesem goatnet an, oder setze `GOATBUILDER_UPDATE_URL`. Du musst im Mesh dieses goatnets sein. Wenn nicht, sagt `--check`, dass es den Server nicht erreichen kann, und nennt die Adresse, die es versucht hat. Das ist für diese Lektion das richtige Ergebnis.
tutor-l15-s1 = Liste die Prüfungen, die für ein goatnet gelten
tutor-l6-text =
    Ein Release bewegt eine neue Version in dieser Reihenfolge durch drei goatnets: dev, dann preprod, dann prod.

    1. Jemand taggt die Version auf main, und CI baut sie.
    2. dev: goatbuilder deployt sie und führt die Abnahmeprüfungen aus.
    3. preprod: goatbuilder deployt sie erneut, führt die Prüfungen erneut aus und hält die Ergebnisse fest.
    4. Die Version wartet dann für eine feste Zeit auf preprod. Nichts bewegt sich von selbst zu prod.
    5. prod: nach dieser Wartezeit führt eine Person den Release mit `--from prod` aus. Die goatbuilder CLI deployt zu prod und prüft alle drei goatnets.

    Das Kommando goatbuilder `promote status` zeigt, wo jede Komponente steht. Ein Kandidat ist eine Version, die getestet wird. Die allgemeine Form eines Releases ist `goatbuilder release <component> --tag <tag> --from dev`. Die Befehle unten nutzen eine Komponente dieses Checkouts und ein Beispiel-Tag. Jeder hat `--dry-run`, also wird nichts getaggt oder deployt.

    Die Probeläufe brauchen `gh` und das Mesh. Wenn du kein Admin des Repositorys bist, setze `GOAT_RC_ROLE_CHECK=0` vor jeden `release`-Befehl.
tutor-l9-s1 = Erzeuge einen echten Fehler mit einem goatnet, das es nicht gibt
tutor-l9-text =
    Wenn goatbuilder sich weigert, gibt es einen Code aus. Codes, die mit `GB-` beginnen, kommen von goatbuilder, `E_` von der goat-CLI, und `X-` von einem anderen Tool, das goatbuilder ausgeführt hat.

    Das Kommando goatbuilder `explain` nimmt einen Code, oder den Text einer Fehlerzeile, und sagt dir, was passiert ist, warum, wie du es behebst, und wo du mehr liest.

    Wenn du nach einem goatnet fragst, das dieses Checkout nicht deklariert, entsteht ein echter Fehler zum Erklären.
