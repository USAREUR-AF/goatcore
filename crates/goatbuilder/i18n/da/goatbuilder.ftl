### goatbuilder — da (da)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Byg og lever et goat-miljø: deklarér, konvergér, verificér — under én deploy-kontrakt
help-flag-output = Outputformat: en tabel til mennesker; JSON eller YAML til programmer
help-flag-repo = Repositoryets rod. Standard er det checkout, denne binær køres i
help-flag-yes = Bekræft et deklarativt verbum uden prompt (til automatisering)
help-flag-frozen = Afvis enhver kørsel, der kræver fast-forward af checkout (streng C3)
help-flag-mock = Kør mod en fixture-mappe i stedet for værter: intet register, ingen transport, deterministisk (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Sprog til hjælp og fejlforklaringer (et tag som fr, uk, de); output, som en pipeline læser, forbliver engelsk. Standard: OS-locale
help-cmd-validate = Tjek en config mod det versionerede skema og krydsfeltreglerne, offline
help-cmd-doctor = Undersøg, hvad en kørsel kræver, før noget skæres: afhængigheder, checkoutets friskhed, hvert miljøs transport, og at hver komponents playbook peger på en entydig målmode
help-cmd-status = Deklareret mod registreret, pr. maskine: converged, pending eller unknown (C7, C8). Kun læsning. Exit 3, så længe en maskine ikke er converged
help-cmd-deploy = Læg den pin, main deklarerer, på ét miljø og bevis det: opløs komponenten og miljøets grupper ud fra pins, playet og dets målmode, hver hemmelighed playet templater, transporten; kør playet via ansible-preflight; læs deploy-ledgeren tilbage fra hver vært og kald det først færdigt, når den nyeste række angiver den deklarerede digest med action=deploy (C7). På en fixture (`--mock`) står den scriptede converger i stedet for værterne
help-cmd-release = Før en release candidate fra et tag på main gennem dev-goatnettets live-validering til preprod-trinnet (ADR 1173); efter soak, med `--from prod`, graduér den, fold den ind i fleet:, deploy prod og re-verificér hvert trin. Erstatter ops/scripts/rc.sh trin for trin
help-cmd-rollback = Rul en deployet komponent tilbage til dens N'te seneste ref, efter artefakttype, med ledgeren på værten som sandhedskilde (ADR 1060). Erstatter ops/scripts/rollback.sh: exit 3 afvist som usikker (data, cert, forward-only, reversible:false), 4 guidet (config, acl)
help-cmd-fastlane = Én kommando fra en merged commit til alle goatnets (ADR 1127): den bevidste undtagelse fra stigen. Erstatter ops/scripts/fastlane.sh trin for trin; afviser en upinnet digest eller et åbent graduation-sæt, og kører deploys gennem denne binær
help-cmd-standup = Et site fra ingenting til verificeret: standup'ens faser i doktrinens rækkefølge med en fail-closed-gate i hver ende, observability-værten som parallelt ben, rescue-klasserne og de tværgående goatnet-flader til sidst (ADR 1183). Erstatter ops/scripts/goat-standup.sh trin for trin. På en fixture (`--mock`) bliver hver komponent i hver klasse konvergeret af den scriptede converger, og tiden deklareret → verificeret er et-times-goatnettets mål
help-cmd-join-node = Én maskine mere af en klasse på et live site, deklareret → converged → verificeret: docs/tutorials/02-first-federated-node.md's udførte sekvens som nummererede faser med dens gates (reach, nøgle, repo-arbejde først, baseline, kernel-WireGuard før og efter enrolment, kernens ACL-plan som enrolment-gate, klassens converge, accept)
help-cmd-federate = En suveræn peering deklareret, konvergeret og verificeret (ADR 1157): vores halvdel målt, pakkerne udvekslet uden for båndet, én aftale komponeret og verificeret, bladene mintet ud fra den, reglerne renderet af den ene skribent, kanten deployet, den transport aftalen valgte åbnet, accept i alle fire tiers. Erstatter ops/scripts/peering-standup.sh trin for trin
help-cmd-pins = ÉN opløsningspas med TO renderinger: hvert opført medlems group_vars pins.yml og hvert opført goatnets IntentV1-artefakt, fra ops/pins/fleet-pins.yaml. Erstatter ops/pins/generate-pins.py byte for byte. `--check` validerer, regenererer og afviser drift, en usigneret eller manipuleret intent og en styret pin defineret uden for pins.yml (exit 1)
help-cmd-promote = Vejled og overvåg release-graduation på tværs af goatnet-stigen (ADR 1120, 1125): status, åbn en candidate, graduér, graduér et sæt, hotfix-vejen, review, readiness, abandon, check. Erstatter ops/release/graduate.py trin for trin. Intet her graduerer af sig selv: en mutation kører kun, når den kaldes, udskriver hvad den gjorde og beder om --yes uden en terminal (C10, ADR 1177)
help-cmd-explain = Fortolk en fejl: en kode (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) eller vilkårlig fejltekst — denne binærs egen eller et værktøjs ord, den bar med (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Udskriver hvad der skete, hvorfor, hvad man skal gøre, og hvor man kan læse mere; `-` læser teksten fra stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Et site fra verificeret til intet — standups spejlbillede, optegnelser bevares (en nedbrydning tilføjer teardown-rækker, sletter aldrig). På en fixture (`--mock`) prøves nedbrydningen af i omvendt faserækkefølge; på rigtige værter udskriver den rækkefølgen og nægter at køre, før hver fase deklarerer sit teardown-play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lektioner à la vimtutor: hver angiver den præcise kommando, der skal køres mod en engangskopi af en fixture (eller skrivebeskyttet mod denne checkout), og tjekker så, hvad kommandoen faktisk gjorde — dens exitkode og rækker — før den næste låses op. `tutor` viser den lektion, du er på, og markerer den færdig, når dine kørsler opfylder den; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skjult (src/fun.rs)
help-cmd-argan = Skjult (src/fun.rs)
help-cmd-bound = Skjult (crates/goat-fun)
help-cmd-secrets = Den deklarerede secrets-holdning (ADR 1204): hvilket lager der er gældende (1password, openbao, sops, git-crypt, env-file, env), hvad der er tilgængeligt, hvordan du leverer det, der mangler, og lagerets egne tjek. `init --env <goatnet>` skriver env-filen til zero-SaaS-quickstarten: en stærk tilfældig værdi til hver variabel, som goatnettets plays templater, 0600, gitignoreret, og roterer aldrig en værdi, der allerede er der. Samme modul som ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Læs hver værts nyeste deploy-ledger-række for én komponent og sig, om den navngiver den ref, som main erklærer (C7). Kun læsning; exit 3, så længe en vært er pending eller unknown. Samme tjek som `deploy --verify-only` kører
help-cmd-diff = Hvad er anderledes end main lige nu (C8): mains erklærede ref mod hver værts nyeste ledger-række, for én komponent eller, uden angivelse, alle miljøets komponenter, læst i ét gennemløb pr. medlemsgruppe. Kun læsning; exit 3 ved enhver drift eller ulæselig vært
help-cmd-history = Deploy-ledgeren som tidslinje, nyeste først, på alle miljøets værter (C8): hver række er en post med maskinen, tidspunktet, handlingen (deploy eller rollback) og versionen. Postens forrige version og operatøren, der kørte den, er i `-o json`. Kun læsning
help-cmd-plan = Skriv til en plan-fil til gennemsyn, hvad en deklarativ verbum ville gøre: de erklærede refs, play og mål, ledgeren, mains commit, transporten (deploy); artefakttypen og mål-ref (rollback). Rører ingen værts tilstand. Kør den som gennemset med `apply`
help-cmd-apply = Kør præcis en gennemset plan. Afviser (exit 1, med angivelse af, hvad der er flyttet), når mains erklæring, pins-commit, play eller rollback-mål er flyttet, siden planen blev skrevet; en ændret transport rapporteres, men afvises ikke
help-cmd-adopt = Erklær, hvad et miljø allerede kører: læs alle værters ledgere og foreslå en pins-afvigelse for hver komponent, hvis kørende ref afviger fra mains erklæring. Som standard en rapport; med --yes skrives ændringerne i et nyt worktree af main til gennemsyn. Deployer aldrig

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ikke endnu: { $why }
tutor-lesson-done = Lektion { $n } gennemført: { $title }.
tutor-all-done-reset = Alle lektioner er gennemført. `goatbuilder tutor --reset` starter forfra; tutorials går længere: docs/tutorials/.
tutor-all-done = Alle lektioner er gennemført. Tutorials går længere: docs/tutorials/.
tutor-workspace = Arbejdsområde: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Kontrollér at et goatnet er bygget korrekt, lag for lag: alle acceptkontroller, der gælder for stedet, inklusive de kontroller, en operatør kører med goat CLI. Standup'ens acceptfase som en selvstændig kommando. Kun læsning; exit 3 så længe en kontrol fejler eller ikke kan svare
help-flag-offline = Isoleret site: hent ikke trunken; vurdér checkouten mod den cachede, og registrér kørslen som offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Kontrollér, at et goatnet er i sin deklarerede sluttilstand: hver tjeneste, som dets egne deklarationer siger kører, er udrullet, kørende, sund og udfører sit job, med dets zero-trust-forbindelser koblet til, og intet kører, som det ikke deklarerer. Kun læsning; exit 3, så længe noget er galt
help-cmd-survey = Hvor ensartet softwaren er inden for hvert goatnet og på tværs af stigens goatnets, aflæst i ét parallelt gennemløb. Læser kun

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Hvor ensartet softwaren er i hvert goatnet og på tværs af stigens goatnets, aflæst i ét parallelt gennemløb. Kun læsning.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Erstat denne binær med den release, dens goatnet serverer på `get.<site>.<zone>/goatbuilder/`: downloaden kontrolleres mod dens `.sha256` og den signerede `SHA256SUMS`, før noget ændres, og udskiftningen er atomisk. Allerede aktuel: én række, exit 0. `--check` kun læser (exit 3 når den er bagud). Kilden: `GOATBUILDER_UPDATE_URL`, ellers `--site` fra dens group_vars, ellers den, som sidste opdatering brugte
tutor-prereq = Før du starter: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Farv tabellens ok/FAIL-symbol og dom (auto|always|never). `auto` farver kun i en terminal; NO_COLOR og CLICOLOR_FORCE har forrang. JSON og YAML farves aldrig
help-flag-verbose = Log hvad en kørsel gør — de eksterne kommandoer den kalder, og netværksfasen i `update` — til stderr, så `-o json|yaml` forbliver et rent dokument. Eller sæt RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunder at begrænse forbindelses-/nåbarhedsfasen til, før den fejler hurtigt — den ene fase, der kan blokere på en uopnåelig vært (`update`s hentning af den udbudte version). Begrænser IKKE et plays egen kørsel eller download af binæren (stall-begrænset). Læser også GOATBUILDER_TIMEOUT
help-cmd-completion = Udskriv et shell-completion-script til stdout (`goatbuilder completion zsh >> ~/.zshrc`). Til live-forslag, som et statisk script ikke kan kende — de sites, komponenter og goatnets, som denne checkout deklarerer — kobl en hook til `goatbuilder _complete` (se det genererede scripts header)
help-cmd-docs = Kommandoreferencen: `goatbuilder docs` peger på den; `--markdown` udskriver det hele, gennemgået fra det levende kommandotræ, så den aldrig kan afvige fra `--help`
help-cmd-_complete = Skjult: udskriv live completion-forslag til shell-hooks, ét pr. linje (de sites, komponenter og goatnets, som denne checkout deklarerer). Kun læsning, fejler aldrig højlydt — en completion-hook må ikke ødelægge prompten

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Før en udrulning: parathed- og reversibilitetsoversigten (ADR 1211): tre fakta — State (stateless | stateful), Rollback (pin-revert | restore-required | none) og Scope (single | coupled) — erklæret per komponent og FAIL-CLOSED, hvis de ikke er erklæret, med den parathedsdel, de andre verber beviser (ref kan trækkes, pins --check, secrets-gaten, frisk kilde, den erklærede slutstilstand) og en validering af rollback-faktummet (rollback --dry-run for pin-revert, retentionsvinduet for restore-required). `deploy` kører den først; ærlighedsgraderet, aldrig et falsk grønt. Læser kun; exit 3 når ikke ren

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Tjek din opsætning
tutor-l1-s1 = Tjek konfigurationsfilen
tutor-l2-title = Læs tilstanden: converged, pending, unknown
tutor-l2-text =
    Kommandoen goatbuilder `status` sammenligner to ting for hver maskine: den version, main erklærer for den, og den version, dens egen post siger, den kører. Den ændrer ikke noget.

    Hver maskine er i en af tre tilstande:
    - converged (konvergeret): dens post angiver den erklærede version. Dette er færdigt.
    - pending (afventer): dens post angiver en ældre version, eller ingen endnu.
    - unknown (ukendt): goatbuilder kan ikke læse dens post.

    Kun converged tæller som færdigt. Hvis en maskine er pending eller unknown, afslutter `status` med kode 3.

    Fra denne lektion bruger `--mock` en øvelseskopi i din tutor-mappe i stedet for rigtige maskiner. Intet, du gør der, rammer en rigtig maskine.

    Der er endnu ikke deployet noget til øvelseskopiens `dev`. Så forvent fire maskiner pending og en unknown. Du vil se converged maskiner i lektion 3.
tutor-l2-s1 = Læs tilstanden for dev
tutor-l3-title = Deploy: planlæg det, kør det, bevis det
tutor-l3-text =
    At deploye lægger den erklærede version på hver maskine og læser så hver maskines post for at bevise, at den nåede frem. En deploy er kun færdig, når hver maskines post angiver den version. En deploy, der kørte uden fejl, er ikke nok.

    Du skal deploye `portal` til øvelseskopiens `preprod`. `--dry-run` viser, hvad der ville ske, og ændrer ikke noget. `--ack` registrerer, hvem der gjorde det; kommandoen udfylder dit navn fra git. Efter deployet skal `status` vise begge maskiner som `ok` og converged.
tutor-l3-s1 = Planlæg deployen
tutor-l3-s2 = Deploy portal til preprod
tutor-l3-s3 = Tjek at begge maskiner er converged
tutor-l4-title = Drift og historik
tutor-l4-text =
    To kommandoer hjælper dig med at forstå et goatnet før og efter en ændring. Begge kun læser.
    - Kommandoen goatbuilder `diff` lister hver maskine, hvis post afviger fra den erklærede version. En afvigelse kaldes drift.
    - Kommandoen goatbuilder `history` lister hver version, en komponent har haft på hver maskine, nyeste først.

    På `dev` er hver maskine bagud. Historikken for `portal` på `preprod` inkluderer nu din deploy fra lektion 3.
tutor-l4-s1 = List driften på dev
tutor-l4-s2 = Læs historikken for portal på preprod
tutor-l5-title = Rul tilbage, og se en usikker rollback blive afvist
tutor-l5-text =
    En rollback erklærer en ældre version og deployer den som enhver anden version. goatbuilder CLI læser så hver maskines post for at bevise, at den nåede frem.

    Nogle ændringer kan ikke fortrydes på denne måde. At deploye en ældre version kan ikke fortryde data, en komponent har skrevet, et certifikat, eller en ændring, der kun går fremad. goatbuilder CLI afviser dem med kode 3 og siger, hvad du skal gøre i stedet.

    Øvelseskopiens `db-schema`-komponent indeholder data, så goatbuilder afviser at rulle den tilbage. Den afvisning er det forventede resultat.
tutor-l5-s1 = Planlæg rollbacken
tutor-l5-s2 = Rul portal tilbage en version
tutor-l5-s3 = Forsøg at rulle db-schema tilbage, hvilket goatbuilder afviser
tutor-l6-title = Release en ny version: dev, så preprod, så prod
tutor-l6-s1 = Se hvor hver komponent er
tutor-l6-s2 = Prøvekør releasen fra dev
tutor-l6-s3 = Prøvekør prod-trinet
tutor-l7-title = Fastlane, og hvornår du ikke skal bruge det
tutor-l7-text =
    Fastlane lægger en version på alle goatnet på én gang. Det springer ventetiden på preprod og releasens poster over.

    Brug det kun til en akut rettelse, du kan fortryde ved at gå tilbage til den forrige version. Brug det aldrig til en databaseændring, en identitets- eller adgangsændring, eller noget, du ikke kan fortryde på den måde.

    Fastlane afviser en version, CI ikke har bygget, og en komponent, der er del af en release, der stadig er i gang.

    Den generelle form er `goatbuilder fastlane <component> --ref <version> --reason <why>`. Kør eksemplet nedenfor. Det har `--dry-run`, så intet ændres. Det kræver `gh`.
tutor-l7-s1 = Prøvekør en fastlane
tutor-l8-title = Pins: find drift, og ret det på den rigtige måde
tutor-l8-text =
    En fil bestemmer, hvilken version hvert goatnet kører: `ops/pins/fleet-pins.yaml`. goatbuilder CLI genererer de andre versionsfiler fra den: `pins.yml`-filerne og de signerede intent-filer.

    Kommandoen goatbuilder `pins --check` sammenligner de genererede filer med `fleet-pins.yaml` og afviser enhver fil, der afviger. Kommandoen goatbuilder `pins` regenererer dem.

    Din tutor-mappe har en kopi af repositoriet, hvor nogen har redigeret en `pins.yml`-fil i hånden. Ret aldrig en håndredigering med endnu en håndredigering. Ret `fleet-pins.yaml`, og regenerer derefter.
tutor-l8-s1 = Tjek dette checkout
tutor-l8-s2 = Tjek kopien og find håndredigeringen
tutor-l8-s3 = Regenerer kopiens filer
tutor-l8-s4 = Tjek kopien igen
tutor-l9-title = Forklar en fejl
tutor-l9-s2 = Forklar fejlen ved dens kode
tutor-l9-s3 = Forklar fejlen ved dens tekst
tutor-l10-title = Tutorial 1: byg et nyt goatnets første site
tutor-l10-text =
    Denne lektion bygger et helt goatnet i en øvelseskopi. Eksempel-goatnettet er `zagros`. Dets første site, `alpha`, har to kernemaskiner, en overvågningsmaskine og en redningsmaskine.

    Kommandoen goatbuilder `standup` sætter hver maskine op i en fast rækkefølge og tjekker hver post. Dens sidste linje siger, at sitet er færdigt. På rigtige maskiner viser den linje også, hvor lang tid det tog.

    Hele tutorialen er `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Byg det første site, alpha
tutor-l10-s2 = Tjek resultatet
tutor-l11-title = Tutorial 2: tilføj et andet site
tutor-l11-text =
    Denne lektion starter, hvor tutorial 1 sluttede. Den tilføjer et andet site, `beta`, i en ny øvelseskopi, `zagros-2`.

    Et andet site kopierer sine brugerkonti fra det første site. Dets standup ændrer også det første site én gang, så kopieringen virker i begge retninger. Et trin kaldet `alpha/core` gør dette.

    Hele tutorialen er `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Byg det andet site, beta
tutor-l11-s2 = Tjek begge sites
tutor-l12-s1 = Planlæg at tilføje en maskine
tutor-l13-s1 = Planlæg en federation
tutor-l14-title = Hold goatbuilder opdateret
tutor-l14-s1 = Spørg om der findes en nyere goatbuilder
tutor-l15-title = Assay: tjek, hvordan et goatnet blev bygget
tutor-l15-text =
    Kommandoen goatbuilder `assay` tjekker, at et goatnet er bygget korrekt, et lag ad gangen, ved hjælp af accepttestene. Hver test får et af tre resultater:
    - PASS: testen bestod.
    - FAIL: goatnettet har et problem.
    - UNKNOWN: intet kunne svare på testen.

    Kun PASS tæller. Enhver FAIL eller UNKNOWN giver kode 3.

    `--plan` lister de tests, der gælder for et goatnet, lag for lag, og rører ingen maskine. Brug det til at se, hvad en rigtig assay ville dække. Den generelle form er `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: er versionerne konsistente?
tutor-l16-text =
    Kommandoen goatbuilder `survey` tjekker i ét gennemløb, om de samme versioner kører, hvor de skal:
    - Inden i hvert goatnet: kører hver maskine den erklærede version, og matcher maskiner, der skal matche, rent faktisk?
    - Mellem dev, preprod og prod: en række pr. komponent, en kolonne pr. goatnet.

    En forskel, releaseprocessen forventer, bliver rapporteret, men er ikke en fejl. Et eksempel er en ny version, der er på dev, men ikke på prod endnu. En forskel, intet erklærer, læses som `difference not declared`. En maskine, goatbuilder ikke kan læse, er unknown, aldrig konsistent.

    Kør det på øvelseskopien. Dens `dev` er bagud med vilje, så forvent fund og kode 3.
tutor-l16-s1 = Survey øvelseskopien
tutor-l17-title = Endstate: kører et goatnet præcis det, det erklærer?
tutor-l17-text =
    Kommandoen goatbuilder `endstate` tjekker to ting om et goatnet:
    1. Hver tjeneste, det erklærer, er deployet, kører, er sund og korrekt forbundet.
    2. Intet kører, som det ikke erklærer. En uerklæret tjeneste kaldes en forældreløs tjeneste.

    goatbuilder CLI regner ud, hvad et site bør køre, fra sitets egne indstillinger, ikke fra en separat liste.

    Et tjek, der intet har at læse, markeres UNOBSERVED. Det er et ærligt svar, ikke en fejl. En FAIL eller en forældreløs tjeneste giver kode 3.

    Den generelle form er `goatbuilder endstate <goatnet>`. Kommandoen nedenfor tjekker et eksempel-site fra gemte data, så den rører ingen maskine.
tutor-l17-s1 = Tjek eksempel-sitet
tutor-l18-title = Bonus: vær nysgerrig
tutor-l18-text =
    Denne lektion er valgfri. Kør kommandoen nedenfor, og tutoren er færdig.

    goatbuilder CLI har et par små skjulte funktioner. De står ikke i hjælpen eller i disse lektioner. Nogle svarer på ord, erfarne Unix-brugere ofte skriver ind i et nyt program, bare for at se, hvad der sker. De fleste ting, du prøver, gør ingenting.

    Denne lektion siger ikke, hvad de er. At finde dem er pointen.

    Kommandoen nedenfor lister hver fejl, goatbuilder kan forklare. Det er et godt sted at begynde at udforske.
tutor-l18-s1 = List hver fejl, goatbuilder kan forklare
tutor-heading = Lektion { $n } af { $total } · { $title }
tutor-setup = Kør først dette én gang i denne terminal:
tutor-not-recording = Denne terminal registrerer ikke dine kørsler, så tutoren kan ikke sætte kryds ved dine trin. Kør dette én gang, og kør så trinene:
tutor-type = Gør disse trin i din terminal ({ $met } af { $total } færdige):
tutor-rerun = Kør `goatbuilder tutor` igen efter hvert trin. Den tjekker, hvad du gjorde, og sætter kryds ved trinet.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Velkommen til GOATBUILDER Tutor
tutor-bar-progress = Fremskridt
tutor-bar-done = { $done } af { $total } lektioner færdige
tutor-after-step = Trin { $met } af { $total } færdigt. Næste:
tutor-after-lesson = Lektion { $n } er færdig. Kør `goatbuilder tutor` for at starte lektion { $next }. For at læse den igen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lektion { $n } er færdig, og det er tutoren også. Kør `goatbuilder tutor` for at se den markeret som færdig. For at læse den igen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Du har afsluttet denne lektion, så denne side er kun til læsning. Kør `goatbuilder tutor` for at komme tilbage til, hvor du er.
tutor-other-lessons = For at læse en lektion igen: `goatbuilder tutor --lesson 1`. For at se dem alle: `goatbuilder tutor --list`.
tutor-l12-title = Tilføj en maskine til et kørende site
tutor-l12-text =
    Kommandoen goatbuilder `join-node` tilføjer en maskine mere til et site, der allerede kører. Den udskriver først en plan, med en linje per fase. Hver fase har en type:
    - tool: goatbuilder kører et script.
    - ansible: goatbuilder kører en playbook.
    - gate: et tjek, der skal godkendes, før næste fase.
    - manual: du gør det selv, og kørslen pauser.

    Kommandoen nedenfor har `--dry-run`. Den læser dette checkout og ændrer ikke noget. Under tabellen kan du læse, hvad du gør i hånden.
tutor-l13-title = Forbind til en anden organisations goatnet
tutor-l13-text =
    Kommandoen goatbuilder `federate` forbinder dit goatnet til et goatnet, en anden organisation kører. Hver side bygger sin halvdel af en aftale. Du udskifter halvdele, og hver side samler dem til den samme aftale.

    Bekræft den anden sides nøgle på en anden måde, som et opkald, du har tillid til. En nøgle, der kommer inde i deres pakke, beviser ikke noget.

    Kommandoen nedenfor har `--dry-run`, så den ændrer ikke noget. En rigtig kørsel stopper ved fase 4, fordi du gør den fase i hånden.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Denne kommando er ikke en del af lektion { $n }. Kør `goatbuilder tutor` for at se, hvor du er.
tutor-l1-s2 = Spørg doctor om ét goatnet
tutor-l1-text =
    goatbuilder CLI bygger, deployer og udgiver GOAT-software. Før du bruger det, tjek din opsætning med to kommandoer:
    - Kommandoen goatbuilder `validate` tjekker konfigurationsfilen, `ops/goatbuilder/goatbuilder.yaml`. Den læser kun dette checkout og har ikke brug for netværk.
    - Kommandoen goatbuilder `doctor` tjekker, hvad en rigtig deploy kræver: dine værktøjer, dine logins, om dette checkout matcher main, og hvordan goatbuilder når hvert goatnet. Et goatnet er ét GOAT-miljø.

    Kommandoen goatbuilder `doctor` viser, hvor mange tjek der er bestået, og derefter hvert problem med dets løsning. I denne lektion er et problem information, ikke en fejl. Læs løsningerne, men du behøver ikke rette dem nu.

    De fleste lektioner kræver ikke login. Lektion 2 til 5, 10, 11 og 16 bruger en øvelseskopi. Lektion 8, 9, 12, 13, 15, 17 og 18 læser kun dette checkout. Lektion 6 kræver `gh` og meshet, lektion 7 kræver `gh`, og lektion 14 kræver meshet. Ingen lektion kræver 1Password.
tutor-l14-text =
    goatbuilder CLI kan opdatere sig selv fra den kopi, dit goatnet server på `get.<site>.<zone>/goatbuilder/`.

    Kommandoen goatbuilder `update --check` læser kun. Den sammenligner din version med den serverede version og ændrer ikke noget. Kode 0 betyder, at du er opdateret. Kode 3 betyder, at der findes en nyere version.

    En rigtig opdatering, uden `--check`, verificerer downloadets checksum og signatur, før den erstatter noget. Den udskifter så filen i et trin.

    goatbuilder CLI skal vide, hvilket goatnet det skal spørge: angiv `--site` med det goatnet, eller sæt `GOATBUILDER_UPDATE_URL`. Du skal være på det goatnets mesh. Hvis du ikke er det, siger `--check`, at den ikke kan nå serveren, og nævner den adresse, den forsøgte. Det er det korrekte resultat for denne lektion.
tutor-l15-s1 = List testene der gælder for ét goatnet
tutor-l6-text =
    En release flytter en ny version gennem tre goatnet i rækkefølge: dev, så preprod, så prod.

    1. Nogen tagger versionen på main, og CI bygger den.
    2. dev: goatbuilder deployer den og kører accepttestene.
    3. preprod: goatbuilder deployer den igen, kører testene igen og gemmer resultaterne.
    4. Versionen venter så på preprod i en fastsat tid. Intet flytter til prod af sig selv.
    5. prod: efter den ventetid kører en person releasen med `--from prod`. goatbuilder CLI deployer til prod og tjekker alle tre goatnet.

    Kommandoen goatbuilder `promote status` viser, hvor hver komponent er. En kandidat er en version, der testes. Den generelle form for en release er `goatbuilder release <component> --tag <tag> --from dev`. Kommandoerne nedenfor bruger én komponent fra dette checkout og et eksempel-tag. Hver har `--dry-run`, så intet tagges eller deployes.

    Prøvekørslerne kræver `gh` og meshet. Hvis du ikke er admin for repositoriet, så sæt `GOAT_RC_ROLE_CHECK=0` før hver `release`-kommando.
tutor-l9-s1 = Lav en rigtig fejl med et goatnet, der ikke findes
tutor-l9-text =
    Når goatbuilder afviser, skriver det en kode. Koder, der starter med `GB-`, kommer fra goatbuilder, `E_` fra goat CLI'en, og `X-` fra et andet værktøj, goatbuilder kørte.

    Kommandoen goatbuilder `explain` tager en kode, eller teksten fra en fejllinje, og siger, hvad der skete, hvorfor, hvordan du retter det, og hvor du kan læse mere.

    At bede om et goatnet, som dette checkout ikke deklarerer, skaber en rigtig fejl at forklare.
