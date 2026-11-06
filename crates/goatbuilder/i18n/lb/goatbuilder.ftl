### goatbuilder — lb (lb)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Een goat-Environnement bauen an ausrollen: deklaréieren, konvergéieren, verifizéieren — ënner engem Deploy-Kontrakt
help-flag-output = Ausgabeformat: eng Tabell fir e Mënsch; JSON oder YAML fir e Programm
help-flag-repo = D'Wuerzelverzeechnis vum Repository. Standard: de Checkout, an deem dëse Binary leeft
help-flag-yes = E deklarative Verb ouni Nofro confirméieren (fir Automatisatioun)
help-flag-frozen = All Lafen refuséieren, deen de Checkout fast-forwarde misst (strikt C3)
help-flag-mock = Géint e Fixture-Verzeechnis amplaz géint Hosten lafen: keng Registry, keen Transport, deterministesch (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Sprooch fir Hëllef an Feelererklärungen (en Tag wéi fr, uk, de); Ausgab, déi eng Pipeline liest, bleift Englesch. Standard: d'OS-Locale
help-cmd-validate = Eng Config géint dat versionéiert Schema an d'Cross-Field-Reegele préiwen, offline
help-cmd-doctor = Préift, wat e Lafen brauch, ier eppes geschnidde gëtt: Dependenzen, Aktualitéit vum Checkout, den Transport vun all Environnement, an dass d'Playbook vun all Komponent op e kloere Target-Modus opléist
help-cmd-status = Deklaréiert géint opgeholl, pro Maschinn: converged, pending oder unknown (C7, C8). Liest nëmmen. Exit 3, soulaang eng Maschinn net converged ass
help-cmd-deploy = De Pin, deen main deklaréiert, op ee Environnement leeën an beweisen: d'Komponent an d'Gruppe vum Environnement aus de Pins opléisen, d'Play an hiren Target-Modus, all Secret, dee vun der Play getemplated gëtt, den Transport; d'Play duerch ansible-preflight lafen; de Deploy-Ledger vun all Host zeréckliesen a just als fäerdeg uerteelen, wann déi neisten Zeil den deklaréierten Digest mat action=deploy nennt (C7). Op engem Fixture (`--mock`) steet de gescripte Converger fir d'Hosten an
help-cmd-release = E Release-Kandidat vun engem Tag op main duerch d'Live-Validatioun vum Dev-Goatnet bis op d'Preprod-Sprousse féieren (ADR 1173); no der Soak mat `--from prod` graduéieren, a fleet: androen, Prod deployen an all Sprousse nei verifizéieren. Ersetzt ops/scripts/rc.sh Schrëtt fir Schrëtt
help-cmd-rollback = Eng deployed Komponent op hir N-t rezentste Ref zeréckrollen, no Artefakt-Typ, mam On-Host-Ledger als Quell vun der Wourecht (ADR 1060). Ersetzt ops/scripts/rollback.sh: Exit 3 refuséiert als onsécher (data, cert, forward-only, reversible:false), 4 guidéiert (config, acl)
help-cmd-fastlane = Een Kommando vun engem gemergte Commit bis op all Goatnet (ADR 1127): déi bewosst Ausnam vun der Leeder. Ersetzt ops/scripts/fastlane.sh Schrëtt fir Schrëtt; refuséiert en net gepinnten Digest oder en oppent Graduatiounsset, a léisst d'Deploys duerch dëse Binary lafen
help-cmd-standup = E Site vun null bis verifizéiert: d'Phase vum Standup an der Reihenfolleg vun der Doktrin, mat engem fail-closed Gate un all Enn, den Observabilitéits-Host als paralleleg Branche, d'Rescue-Klassen an d'Cross-Goatnet-Surfacen zulescht (ADR 1183). Ersetzt ops/scripts/goat-standup.sh Schrëtt fir Schrëtt. Op engem Fixture (`--mock`) gëtt all Komponent vun all Klass vum gescripte Converger konvergéiert an d'Zäit deklaréiert → verifizéiert ass d'Mooss vum Eng-Stonn-Goatnet
help-cmd-join-node = Eng weider Maschinn vun enger Klass op engem Live-Site, deklaréiert → konvergéiert → verifizéiert: déi ausgefouert Sequenz aus docs/tutorials/02-first-federated-node.md als nummeréiert Phasen mat hire Gates (Reach, Schlëssel, Repo-Aarbecht fir d'éischt, Baseline, Kernel-WireGuard virun an no der Enrolment, d'Kernel-ACL-Ebene als Enrolment-Gate, de Converge vun der Klass, Akzeptanz)
help-cmd-federate = Eng souverän Peering deklaréiert, konvergéiert a verifizéiert (ADR 1157): eis Hallschent gemooss, d'Pakéi ausser Bandbreed ausgetosch, een Accord komponéiert a verifizéiert, d'Leaves doraus geprägt, d'Reegelen vum eenzege Writer gerendert, den Edge deployed, den Transport opgemaach, deen den Accord gewielt huet, Akzeptanz an alle véier Tiers. Ersetzt ops/scripts/peering-standup.sh Schrëtt fir Schrëtt
help-cmd-pins = EEN Opléisungsduerchgang mat ZWEE Renderingen: d'pins.yml an der group_vars vun all opgelëschtem Member an den IntentV1-Artefakt vun all opgelëschtem Goatnet, aus ops/pins/fleet-pins.yaml. Ersetzt ops/pins/generate-pins.py Byte fir Byte. `--check` validéiert, regeneréiert a refuséiert Drift, en onennerschriwwenen oder manipuléierten Intent, an e governe Pin, deen ausserhalb vun pins.yml definéiert ass (Exit 1)
help-cmd-promote = D'Release-Graduatioun iwwer d'Goatnet-Leeder guidéieren an observéieren (ADR 1120, 1125): Status, e Kandidat opmaachen, graduéieren, e Set graduéieren, den Hotfix-Wee, Review, Readiness, abandonéieren, check. Ersetzt ops/release/graduate.py Schrëtt fir Schrëtt. Hei graduéiert näischt vun alleng: eng Mutatioun leeft just, wann se opgeruff gëtt, dréckt aus, wat se gemaach huet, a freet ouni Terminal no --yes (C10, ADR 1177)
help-cmd-explain = E Feeler interpretéieren: e Code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) oder iergendeen Feelertext — dee vun dësem Binary selwer oder d'Wierder vun engem Tool, déi en transportéiert huet (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Dréckt aus, wat geschitt ass, firwat, wat ze maachen ass a wou méi ze liesen ass; `-` liest den Text vun stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = E Site vu verifizéiert op näischt — dem standup säi Spigelbild, Records bleiwen (e Burn-down hänkt Teardown-Rowen un, läscht ni eppes). Op engem Fixture (`--mock`) probéiert et de Burn-down an ëmgekéierter Phasereihenfolleg; op richtegen Hosten dréckt et d'Reihenfolleg aus a refuséiert d'Ausféierung, bis all Phas hiert Teardown-Play deklaréiert (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lektiounen am vimtutor-Stil: jiddereng nennt de genauie Befehl, deen op enger Wegwerf-Kopie vun engem Fixture (oder, nëmme liesend, op dësem Checkout) lafe soll, a kontrolléiert dann, wat dee Befehl wierklech gemaach huet — säin Exit a seng Rowen — ier déi nächst opgeet. `tutor` weist d'Lektioun, op där s de bass, a markéiert se als fäerdeg, wann deng Runs se erfëllen; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Verstoppt (src/fun.rs)
help-cmd-argan = Verstoppt (src/fun.rs)
help-cmd-bound = Verstoppt (crates/goat-fun)
help-cmd-secrets = Déi deklaréiert Secrets-Haltung (ADR 1204): wéi ee Store a Kraaft ass (1password, openbao, sops, git-crypt, env-file, env), wat verfügbar ass, wéi een dat Feelend liwwert, an dem Store seng eege Checken. `init --env <goatnet>` schreift dem Zero-SaaS-Quickstart seng Env-Datei: e staarke Zoufallswert fir all Variabel, déi dem Goatnet seng Plays templatéieren, 0600, gitignored, a rotéiert ni e Wert, deen schonn do ass. Datselwecht Modul wéi ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Liest fir eng Komponent déi nei'st Deploy-Ledger-Zeil vun all Host a seet, ob si de Ref nennt, deen main deklaréiert (C7). Liest just; Exit 3, soulaang e Host pending oder unknown ass. Dee selwechte Check wéi `deploy --verify-only` mécht
help-cmd-diff = Wat anescht ass wéi main am Moment (C8): dem main säin deklaréierte Ref géint déi nei'st Ledger-Zeil vun all Host, fir eng Komponent oder, ouni eng, fir all Komponent vun der Ëmgéigend, an engem Duerchgang pro Memberguppe gelies. Liest just; Exit 3 bei all Drift oder onliesbarem Host
help-cmd-history = Den Deploy-Ledger als Zäitleescht, nei'st als éischt, op all Host vun der Ëmgéigend (C8): all Zeil ass hire eegene Record mat der Maschinn, der Zäit, der Aktioun (deploy oder rollback) an der Versioun. D'virdrugen Versioun vum Record an den Operateur, dee en ausgefouert huet, stinn am `-o json`. Liest just
help-cmd-plan = Schreift a eng Plang-Datei fir d'Review, wat e deklarative Verb géif maachen: déi deklaréiert Refs, d'Play an d'Ziel, den Ledger, dem main säi Commit, den Transport (deploy); den Artefakt-Typ an de Ziel-Ref (rollback). Rëhrt keen Host-Zoustand un. Féier en nom Review mat `apply` aus
help-cmd-apply = Féiert genau e gepréiften Plang aus. Verweigert (Exit 1, mat Nennung vu wat sech beweegt huet), wann dem main seng Deklaratioun, de Pins-Commit, d'Play oder d'Rollback-Ziel zënter dem Schreiwe vum Plang geännert goufen; en geännerten Transport gëtt gemellt, net verweigert
help-cmd-adopt = Deklaréiert, wat eng Ëmgéigend scho lafe léisst: liest d'Ledgere vun all Host a proposéiert eng Pins-Ofwäichung fir all Komponent, där hire lafende Ref vun der main-Deklaratioun ofwäicht. Standardméisseg e Rapport; mat --yes ginn d'Ännerunge fir d'Review an en neie Worktree vu main geschriwwen. Deployt ni

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-prereq = Ier s du ufänks: { $prereq }
tutor-not-yet = Nach net: { $why }
tutor-lesson-done = Lektioun { $n } gemaach: { $title }.
tutor-all-done-reset = All Lektioune si gemaach. `goatbuilder tutor --reset` fänkt vun vir un; d'Tutorials ginn weider: docs/tutorials/.
tutor-all-done = All Lektioune si gemaach. D'Tutorials ginn weider: docs/tutorials/.
tutor-workspace = Workspace: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Iwwerpréift Schicht fir Schicht, ob e goatnet richteg opgebaut gouf: all Ofnahmpréifung, déi fir de Site gëllt, och d'Préifungen, déi en Operateur mat der goat CLI leeft. D'Ofnahmphas vum Standup als eegene Befeel. Liest nëmmen; Exit 3, soulaang eng Préifung feelschléit oder net äntwere kann
help-cmd-survey = Wéi konsistent d'Software an all goatnet an tëscht de goatnets vun der Leeder ass, an engem parallele Laf gelies. Liest nëmmen
help-flag-offline = Ofgeschnidde Site: den Trunk net eroflueden; de Checkout géint dee gecachten Trunk bewäerten an de Lauf als offline festhalen (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Kontrolléier, ob e Goatnet an sengem deklaréierten Endzoustand ass: all Service, deen seng eege Deklaratioune soen, datt e leeft, ass deployéiert, leeft, ass gesond an mécht seng Aarbecht, seng Zero-Trust-Edgen sinn verdrot, a näischt leeft, wat et net deklaréiert. Liest nëmmen; Exit 3, soulaang eppes net stëmmt
help-cmd-survey-long =
    Wéi konsistent d'Software an all goatnet an tëscht de goatnets vun der Leeder ass, an engem parallele Laf gelies. Liest nëmmen. Bannent engem goatnet: de ref, deen main deklaréiert, géint déi neist Zeil am ledger vun all Host an d'Containeren, déi en aus deem Image leeft (e lafenden; e gestoppten gëtt nëmmen op engem waarme Standby beuerteelt, wou näischt leeft), Hosten vun engem goatnet, déi net iwwereneestëmmen, de goat CLI op all Host géint deen, deen säi goatnet ubitt, an dat, wat goat-get ubitt, géint main. Tëscht goatnets: eng Zeil pro Komponent, eng Kolonn pro goatnet, mat der Bewäertung vun der Leeder fir all Ënnerscheed, sou wéi promote status se gëtt. Exit 3 bei all Drift oder alles, wat net gelies gouf; en Ënnerscheed, deen d'Leeder deklaréiert, gëtt gemellt, awer net als Feeler gezielt.
help-cmd-update = Ersetzt dës Binärdatei duerch de Release, deen d'goatnet ënner `get.<site>.<zone>/goatbuilder/` ubitt: den Download gëtt géint säin `.sha256` an déi ënnerschriwwen `SHA256SUMS` gepréift, ier iergendeppes ugepaakt gëtt, an den Tausch ass atomar. Schonn aktuell: eng Zeil, Exit 0. `--check` liest nëmmen (Exit 3, wann hannendrun). D'Quell: `GOATBUILDER_UPDATE_URL`, soss `--site` aus senge group_vars, soss déi vum leschten Update

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Tabell-ok/FAIL-Glyph an Uerteel faarweg maachen (auto|always|never). `auto` faarwt nëmmen en Terminal; NO_COLOR a CLICOLOR_FORCE hunn Virrang. JSON an YAML ginn ni gefaarft
help-flag-verbose = Protokolléieren, wat e Lauf mécht — déi extern Kommandoen, déi en opruff, an déi Netzwierk-Phas vun `update` — op stderr, sou dass `-o json|yaml` en sauberen Dokument bleift. Oder RUST_LOG=goatbuilder=debug setzen
help-flag-timeout = Sekonne fir d'Connect-/Reach-Phas ze begrenzen, ier se séier feelt — déi eng Phas, déi op engem onerreechbaren Host blockéiere kann (`update` seng Ofro vun der ugebuedener Versioun). Begrenzt NET dem Play säi Lauf oder den Binary-Download (stall-begrenzt). Liest och GOATBUILDER_TIMEOUT
help-cmd-completion = E Shell-Completion-Skript op stdout ausginn (`goatbuilder completion zsh >> ~/.zshrc`). Fir live Kandidaten, déi e statescht Skript net kann wëssen — d'Siten, Komponenten a Goatnets, déi dëse Checkout deklaréiert — en Hook un `goatbuilder _complete` hänken (kuck de Header vum generéierte Skript)
help-cmd-docs = D'Kommando-Referenz: `goatbuilder docs` weist drop hin; `--markdown` gëtt dat Ganzt aus, duerch de liewege Kommando-Bam gaange, sou dass et ni vun `--help` ofwäiche kann
help-cmd-_complete = Verstoppt: live Completion-Kandidate fir Shell-Hooks ausginn, een pro Zeil (d'Siten, Komponenten a Goatnets, déi dëse Checkout deklaréiert). Liest nëmmen, feelt ni haart — en Completion-Hook däerf de Prompt net futti maachen

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Virun engem Rollout: d'Readiness- a Reversibilitéits-Ausgab (ADR 1211): dräi Fakten — State (stateless | stateful), Rollback (pin-revert | restore-required | none) a Scope (single | coupled) — pro Komponent deklaréiert a FAIL-CLOSED, wann net deklaréiert, mat där Readiness-Hälschent, déi déi aner Verben beweisen (Ref pullbar, pins --check, de Secrets-Gate, frësch Source, den deklaréierten Endzoustand) an enger Validatioun vum Rollback-Fakt (rollback --dry-run fir pin-revert, d'Retentiounsfënster fir restore-required). `deploy` féiert et als Éischt aus; ehrlechkeetsbewäert,ni e falsche Gréngen. Liest nëmmen; Exit 3, wann net propper

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kontrolléier däi Setup
tutor-l1-s1 = Kontrolléier d'Config-Datei
tutor-l2-title = Liest den Zoustand: converged, pending, unknown
tutor-l2-text =
    De Kommando goatbuilder `status` vergläicht zwou Saachen pro Maschinn: d'Versioun, déi main fir si deklaréiert, an d'Versioun, déi hire eegene Record seet, datt si leeft. Hien ännert näischt.

    All Maschinn ass an engem vun dräi Zoustänn:
    - converged (konvergéiert): hire Record nennt déi deklaréiert Versioun. Dat ass gemaach.
    - pending (am waarden): hire Record nennt eng méi al Versioun, oder nach näischt.
    - unknown (onbekannt): goatbuilder kann hire Record net liesen.

    Just converged zielt als gemaach. Wann eng Maschinn pending oder unknown ass, gëtt `status` mam Code 3 eraus.

    Vun dëser Lektioun u weist `--mock` goatbuilder op eng Üebungskopie an dengem Tutor-Dossier, amplaz op richteg Maschinnen. Näischt, wat s du do méchs, erreecht eng richteg Maschinn.

    Op der Üebungskopie hirem `dev` gouf nach näischt deployt. Also waart véier pending Maschinnen an eng unknown. Converged Maschinnen gesäis du an der Lektioun 3.
tutor-l2-s1 = Liest den Zoustand vun dev
tutor-l3-title = Deploy: plangen, ausféieren, beweisen
tutor-l3-text =
    E Deploy leet déi deklaréiert Versioun op all Maschinn, liest dann de Record vun all Maschinn, fir ze beweisen, datt si ukomm ass. E Deploy ass just gemaach, wann de Record vun all Maschinn déi Versioun nennt. E Deploy, deen ouni Feeler gelaf ass, ass net genuch.

    Du wäerts `portal` op der Üebungskopie hirem `preprod` deployen. `--dry-run` weist, wat géif geschéien, an ännert näischt. `--ack` hält fest, wien et gemaach huet; d'Kommando fëllt däin Numm aus git aus. No dem Deploy soll `status` béid Maschinnen als `ok` a converged weisen.
tutor-l3-s1 = Plang den Deploy
tutor-l3-s2 = Deploy portal op preprod
tutor-l3-s3 = Kontrolléier, datt béid Maschinnen converged sinn
tutor-l4-title = Ofwäichung a Geschicht
tutor-l4-text =
    Zwee Kommandoen hëllefe dir, e Goatnet virun an no enger Ännerung ze verstoen. Béid liesen nëmmen.
    - De Kommando goatbuilder `diff` lëscht all Maschinn op, där hire Record anescht ass wéi déi deklaréiert Versioun. En Ënnerscheed heescht Ofwäichung.
    - De Kommando goatbuilder `history` lëscht all Versioun op, déi eng Komponent op all Maschinn hat, déi neisten als éischt.

    Op `dev` ass all Maschinn hannendrun. D'Geschicht vu `portal` op `preprod` enthält elo däin Deploy aus der Lektioun 3.
tutor-l4-s1 = Lëscht d'Ofwäichung op dev op
tutor-l4-s2 = Liest d'Geschicht vu portal op preprod
tutor-l5-title = Zeréckrolle, a gesinn, wéi en onsécheren Rollback refuséiert gëtt
tutor-l5-text =
    E Rollback deklaréiert eng fréiera Versioun a deployt si wéi all aner Versioun. De goatbuilder CLI liest dann de Record vun all Maschinn, fir ze beweisen, datt si ukomm ass.

    E puer Ännerungen kann een esou net réckgängeg maachen. En eeler Versioun deployen kann Date, déi eng Komponent geschriwwen huet, en Zertifikat, oder eng Ännerung, déi nëmmen no vir geet, net réckgängeg maachen. De goatbuilder CLI refuséiert déi mam Code 3 a seet, wat een amplaz maache soll.

    D'Komponent `db-schema` vun der Üebungskopie hält Date, dofir refuséiert goatbuilder, si zeréckzerollen. D'Refus ass dat erwaart Resultat.
tutor-l5-s1 = Plang den Rollback
tutor-l5-s2 = Roll portal eng Versioun zeréck
tutor-l5-s3 = Probéier db-schema zeréckzerollen, wat goatbuilder refuséiert
tutor-l6-title = Releasen eng nei Versioun: dev, dann preprod, dann prod
tutor-l6-s1 = Gesinn, wou all Komponent ass
tutor-l6-s2 = Féier e Testlaf vum Release vun dev aus
tutor-l6-s3 = Féier e Testlaf vum prod Schrëtt aus
tutor-l7-title = Fastlane, a wéini net benotzen
tutor-l7-text =
    Fastlane leet eng Versioun op eemol op all Goatnet. Hien iwwersprengt d'Waardezäit op preprod an d'Release-Records.

    Benotz en just fir eng drénglech Fixatioun, déi s du réckgängeg maache kanns, andeems s du op déi vireg Versioun zeréckgeess. Benotz en ni fir eng Datebank-Ännerung, eng Identitéits- oder Zougangsännerung, oder eppes, wat s du esou net réckgängeg maache kanns.

    Fastlane refuséiert eng Versioun, déi d'CI net gebaut huet, an eng Komponent, déi Deel vun engem Release ass, deen nach am Gaang ass.

    D'allgemeng Form ass `goatbuilder fastlane <component> --ref <version> --reason <why>`. Féier d'Beispill hei drënner aus. Et huet `--dry-run`, dofir ännert sech näischt. Et brauch `gh`.
tutor-l7-s1 = Féier e Testlaf vun engem fastlane aus
tutor-l8-title = Pins: fann d'Ofwäichung a fix si richteg
tutor-l8-text =
    Eng Datei leet fest, wéi eng Versioun all Goatnet leeft: `ops/pins/fleet-pins.yaml`. De goatbuilder CLI generéiert déi aner Versiounsdateien doraus: d'`pins.yml` Dateien an d'ënnerschriwwen Intent-Dateien.

    De Kommando goatbuilder `pins --check` vergläicht déi generéiert Dateien mat `fleet-pins.yaml` a refuséiert all Datei, déi anescht ass. De Kommando goatbuilder `pins` regeneréiert si.

    Däi Tutor-Dossier huet eng Kopie vum Repository, an där een `pins.yml` Datei mat Hand geännert huet. Fix ni eng Ännerung mat Hand mat enger anerer Ännerung mat Hand. Änner `fleet-pins.yaml`, a regeneréier dann.
tutor-l8-s1 = Kontrolléier dëse Checkout
tutor-l8-s2 = Kontrolléier d'Kopie a fann d'Ännerung mat Hand
tutor-l8-s3 = Regeneréier d'Dateien vun der Kopie
tutor-l8-s4 = Kontrolléier d'Kopie nach eng Kéier
tutor-l9-title = Erklär e Feeler
tutor-l9-s2 = Erklär de Feeler mat sengem Code
tutor-l9-s3 = Erklär en mat sengem Text
tutor-l10-title = Tutorial 1: bau de éischten Site vun engem neie Goatnet
tutor-l10-text =
    Dës Lektioun baut e ganzen Goatnet an enger Üebungskopie. Den Beispill-Goatnet ass `zagros`. Säin éischten Site, `alpha`, huet zwou Käre-Maschinnen, eng Iwwerwaachungsmaschinn an eng Rettungsmaschinn.

    De Kommando goatbuilder `standup` setzt all Maschinn an enger fixer Reihenfolleg op a kontrolléiert all Record. Seng lescht Zeil seet, datt de Site gemaach ass. Op richtege Maschinnen weist déi Zeil och, wéi laang et gedauert huet.

    Den ganzen Tutorial ass `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Bau den éischten Site, alpha
tutor-l10-s2 = Kontrolléier d'Resultat
tutor-l11-title = Tutorial 2: setz en zweeten Site bäi
tutor-l11-text =
    Dës Lektioun fänkt un, wou den Tutorial 1 opgehal huet. Si setzt en zweeten Site, `beta`, an enger neier Üebungskopie, `zagros-2`, bäi.

    En zweeten Site kopéiert seng Benotzerkonten vum éischten Site. Säin Standup ännert och eemol den éischten Site, fir datt d'Kopéieren an béide Richtungen funktionéiert. En Schrëtt mam Numm `alpha/core` mécht dat.

    Den ganzen Tutorial ass `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Bau den zweeten Site, beta
tutor-l11-s2 = Kontrolléier béid Sites
tutor-l12-s1 = Plang, eng Maschinn bäizesetzen
tutor-l13-s1 = Plang eng Verbindung mat engem aneren Goatnet
tutor-l14-title = Hal goatbuilder aktuell
tutor-l14-s1 = Froe, ob et e méi neie goatbuilder gëtt
tutor-l15-title = Assay: kontrolléier, wéi e Goatnet gebaut gouf
tutor-l15-text =
    De Kommando goatbuilder `assay` kontrolléiert, datt e Goatnet richteg gebaut gouf, Schicht fir Schicht, mat den Akzeptanztester. All Test kritt ee vun dräi Resultater:
    - PASS: den Test huet bestanen.
    - FAIL: de Goatnet huet e Problem.
    - UNKNOWN: näischt konnt dem Test äntweren.

    Just PASS zielt. All FAIL oder UNKNOWN mécht den Exit-Code 3.

    `--plan` lëscht d'Tester op, déi fir e Goatnet gëllen, pro Schicht, a rëhrt keng Maschinn un. Benotz en, fir ze gesinn, wat en richtegen Assay géif ofdecken. D'allgemeng Form ass `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: sinn d'Versiounen konsequent?
tutor-l16-text =
    De Kommando goatbuilder `survey` kontrolléiert an engem Duerchgang, ob déiselwecht Versiounen do lafen, wou se sollen:
    - Bannent all Goatnet: leeft all Maschinn déi deklaréiert Versioun, a stëmmen Maschinnen, déi iwwereneestëmme sollen, wierklech iwwerteneen?
    - Iwwer dev, preprod a prod: eng Zeil pro Komponent, eng Kolonn pro Goatnet.

    En Ënnerscheed, deen de Release-Prozess erwaart, gëtt gemellt, ass awer kee Feeler. E Beispill ass eng nei Versioun, déi op dev ass, mee nach net op prod. En Ënnerscheed, deen näischt deklaréiert, liest sech als `difference not declared`. Eng Maschinn, déi goatbuilder net liesen kann, ass unknown, ni konsequent.

    Féier en op der Üebungskopie aus. Hire `dev` ass mat Absicht hannendrun, also waart Fonnt a Code 3.
tutor-l16-s1 = Féier survey op der Üebungskopie aus
tutor-l17-title = Endstate: leeft e Goatnet genau dat, wat hien deklaréiert?
tutor-l17-text =
    De Kommando goatbuilder `endstate` kontrolléiert zwou Saachen iwwer e Goatnet:
    1. All Service, deen hien deklaréiert, ass deployt, leeft, ass gesond a richteg verbonnen.
    2. Näischt leeft, wat hien net deklaréiert. En net deklaréierten Service heescht Weesekand.

    De goatbuilder CLI fënnt eraus, wat e Site soll lafen, aus dem Site senge eegenen Astellungen, net aus enger separater Lëscht.

    E Check, deen näischt ze liese huet, gëtt als UNOBSERVED markéiert. Dat ass eng éierlech Äntwert, keen Feeler. E FAIL oder e Weesekand mécht den Exit-Code 3.

    D'allgemeng Form ass `goatbuilder endstate <goatnet>`. De Kommando hei drënner kontrolléiert en Beispill-Site aus gespäicherten Donnéeën, dofir rëhrt en keng Maschinn un.
tutor-l17-s1 = Kontrolléier den Beispill-Site
tutor-l18-title = Bonus: sief virwëtzeg
tutor-l18-text =
    Dës Lektioun ass optional. Féier de Kommando hei drënner aus, an den Tutor ass fäerdeg.

    De goatbuilder CLI huet e puer kleng verstoppte Feature. Si sinn net an der Hëllef oder an dëse Lektiounen. E puer äntweren op Wierder, déi laangjäreg Unix-Benotzer oft an e neit Programm tippen, just fir ze gesinn, wat geschitt. Déi meeschte Saachen, déi s du probéiers, maachen näischt.

    Dës Lektioun seet net, wat si sinn. Si ze fannen ass de Punkt.

    De Kommando hei drënner lëscht all Feeler op, deen goatbuilder erklären kann. Dat ass eng gutt Plaz, fir unzefänken ze entdecken.
tutor-l18-s1 = Lëscht all Feeler op, deen goatbuilder erklären kann
tutor-heading = Lektioun { $n } vun { $total } · { $title }
tutor-setup = Als éischt, féier dëst eng Kéier an dësem Terminal aus:
tutor-not-recording = Dëse Terminal hält deng Lääf net op, dofir kann den Tutor deng Schrëtt net markéieren. Féier dëst eng Kéier aus, féier dann d'Schrëtt aus:
tutor-type = Féier dës Schrëtt an dengem Terminal aus ({ $met } vun { $total } gemaach):
tutor-rerun = No all Schrëtt féier `goatbuilder tutor` nach eng Kéier aus. Hien kontrolléiert, wat s du gemaach hues, a markéiert de Schrëtt.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Wëllkomm bei GOATBUILDER Tutor
tutor-bar-progress = Fortschrëtt
tutor-bar-done = { $done } vun { $total } Lektiounen gemaach
tutor-after-step = Schrëtt { $met } vun { $total } gemaach. Als nächst:
tutor-after-lesson = Lektioun { $n } ass gemaach. Féier `goatbuilder tutor` aus fir d'Lektioun { $next } unzefänken. Fir si nach eng Kéier ze liesen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lektioun { $n } ass gemaach, an den Tutor och. Féier `goatbuilder tutor` aus fir et als gemaach markéiert ze gesinn. Fir si nach eng Kéier ze liesen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Du hues dës Lektioun fäerdeg gemaach, dofir ass dës Säit just zum Liesen. Féier `goatbuilder tutor` aus fir zréck do higoen, wou s du waars.
tutor-other-lessons = Fir eng Lektioun nach eng Kéier ze liesen: `goatbuilder tutor --lesson 1`. Fir si all ze gesinn: `goatbuilder tutor --list`.
tutor-l12-title = Setz eng Maschinn bäi engem lafenden Site derbäi
tutor-l12-text =
    De Kommando goatbuilder `join-node` setzt eng weider Maschinn bäi engem Site derbäi, deen schonn leeft. Et weist als éischt e Plang mat enger Reih pro Phase. Jiddwer Phase huet en Typ:
    - tool: goatbuilder féiert e Script aus.
    - ansible: goatbuilder féiert e Playbook aus.
    - gate: eng Kontroll, déi virun der nächster Phase klappe muss.
    - manual: du méchs dat selwer, an de Lauf pauséiert.

    De Kommando hei drënner huet `--dry-run`. Hie liest dëse Checkout an ännert näischt. Ënnert der Tabell liest du, wat du vun Hand méchs.
tutor-l13-title = Verbann dech mam Goatnet vun enger anerer Organisatioun
tutor-l13-text =
    De Kommando goatbuilder `federate` verbënnt däin Goatnet mat engem Goatnet, deen eng aner Organisatioun leeft. All Säit baut hir Hallschent vun engem Accord. Du tauschs d'Hallschenten aus, an all Säit setzt se zu deemselwechten Accord zesummen.

    Confirméier den Schlëssel vun der anerer Säit op eng separat Aart, wéi zum Beispill en Uruff, deem s du trauss. En Schlëssel, deen bannent hirem Paket ukënnt, beweist näischt.

    De Kommando hei drënner huet `--dry-run`, dofir ännert hien näischt. E richtegen Lauf hält bei Phase 4 op, well s du dës Phase vun Hand méchs.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Dëse Kommando ass net Deel vun der Lektioun { $n }. Féier `goatbuilder tutor` aus fir ze gesinn, wou s du bass.
tutor-l1-s2 = Fro den doctor no engem Goatnet
tutor-l1-text =
    De goatbuilder CLI baut, deployt a verëffentlecht GOAT-Software. Ier s du en benotz, kontrolléier däi Setup mat zwee Kommandoen:
    - De Kommando goatbuilder `validate` kontrolléiert d'Config-Datei, `ops/goatbuilder/goatbuilder.yaml`. E liest just dëse Checkout a brauch keen Netzwierk.
    - De Kommando goatbuilder `doctor` kontrolléiert, wat e richtegen Deploy brauch: deng Tools, deng Umeldungen, ob dëse Checkout mat main iwwereneestëmmt, a wéi goatbuilder all Goatnet erreecht. E Goatnet ass eng GOAT-Ëmwelt.

    De Kommando goatbuilder `doctor` dréckt aus, wéi vill Kontrollen duerchgaange sinn, an duerno all Problem mat senger Léisung. An dëser Lektioun ass e Problem eng Informatioun, kee Feeler. Lies d'Léisungen, mee du muss se elo net uwenden.

    Déi meescht Lektioune brauche keng Umeldung. D'Lektiounen 2 bis 5, 10, 11 a 16 benotzen eng Üebungskopie. D'Lektiounen 8, 9, 12, 13, 15, 17 an 18 liesen just dëse Checkout. Lektioun 6 brauch `gh` an de Mesh, Lektioun 7 brauch `gh`, a Lektioun 14 brauch de Mesh. Keng Lektioun brauch 1Password.
tutor-l14-text =
    De goatbuilder CLI kann sech selwer aktualiséieren aus der Kopie, déi däi Goatnet op `get.<site>.<zone>/goatbuilder/` ubitt.

    De Kommando goatbuilder `update --check` liest just. E vergläicht deng Versioun mat der ugebueder Versioun an ännert näischt. Exit-Code 0 heescht, datt s du um aktuelle Stand bass. Exit-Code 3 heescht, datt et eng méi nei Versioun gëtt.

    En richtegen Update, ouni `--check`, iwwerpréift d'Checksumm an d'Ënnerschrëft vum Download, ier en iergendeppes ersetzt. Duerno tauscht en d'Datei an engem Schrëtt aus.

    De goatbuilder CLI muss wëssen, wéi ee Goatnet e soll froen: gëff `--site` mat deem Goatnet un, oder setz `GOATBUILDER_UPDATE_URL`. Du muss um Mesh vun deem Goatnet sinn. Wann net, seet `--check`, datt en de Server net erreeche kann, a nennt d'Adress, déi e probéiert huet. Dat ass e korrekt Resultat fir dës Lektioun.
tutor-l15-s1 = Lëscht d'Tester op, déi fir een Goatnet gëllen
tutor-l6-text =
    E Release bréngt eng nei Versioun der Rei no duerch dräi Goatnets: dev, dann preprod, dann prod.

    1. Iergendeen taggt d'Versioun op main, an d'CI baut se.
    2. dev: goatbuilder deployt se a leeft d'Akzeptanztester.
    3. preprod: goatbuilder deployt se nach eng Kéier, leeft d'Tester nach eng Kéier a registréiert d'Resultater.
    4. D'Versioun waart dann eng festgeluechten Zäit op preprod. Näischt geet vu selwer op prod.
    5. prod: no där Waardezäit leeft eng Persoun de Release mat `--from prod`. De goatbuilder CLI deployt op prod a kontrolléiert all dräi Goatnets.

    De Kommando goatbuilder `promote status` weist, wou all Komponent ass. E Kandidat ass eng Versioun, déi getest gëtt. Déi allgemeng Form vun engem Release ass `goatbuilder release <component> --tag <tag> --from dev`. D'Kommandoen hei drënner benotzen eng Komponent vun dësem Checkout an e Beispill-Tag. Jidderee vun hinnen huet `--dry-run`, also gëtt näischt getaggt oder deployt.

    D'Dry-Runs brauchen `gh` an de Mesh. Wann s du keen Admin vum Repository bass, setz `GOAT_RC_ROLE_CHECK=0` virun all `release`-Kommando.
tutor-l9-s1 = Maach e richtege Feeler mat engem Goatnet, deen et net gëtt
tutor-l9-text =
    Wann goatbuilder refuséiert, dréckt en e Code aus. Coden, déi mat `GB-` ufänken, kommen vu goatbuilder, `E_` vum goat CLI, an `X-` vun engem aneren Tool, dat goatbuilder gestart huet.

    De Kommando goatbuilder `explain` hëlt e Code, oder den Text vun enger Feelerzeil, a seet dir, wat geschitt ass, firwat, wéi s du et flécks, a wou s du méi liese kanns.

    Wann s du no engem Goatnet frees, deen dëse Checkout net deklaréiert, kriss du e richtege Feeler fir z'erklären.
