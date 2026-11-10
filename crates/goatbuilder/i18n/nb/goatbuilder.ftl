### goatbuilder — nb (nb)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Bygg og lever et goat-miljø: deklarer, konvergér, verifisér – under én deploy-kontrakt
help-flag-output = Utdataformat: tabell for mennesker; JSON eller YAML for programmer
help-flag-repo = Repoets rotmappe. Standard er utsjekken denne binærfilen kjøres i
help-flag-yes = Bekreft en deklarativ verb uten å spørre (for automatisering)
help-flag-frozen = Avvis enhver kjøring som må spole utsjekken framover (streng C3)
help-flag-mock = Kjør mot en fixture-mappe i stedet for verter: uten register, uten transport, deterministisk (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Språk for hjelp og feilforklaringer (en kode som fr, uk, de); utdata som en pipeline leser forblir engelsk. Standard: OS-ets locale
help-cmd-validate = Sjekk en konfig mot det versjonerte skjemaet og kryssfeltreglene, offline
help-cmd-doctor = Undersøk hva en kjøring trenger før noe kuttes: avhengigheter, utsjekkens ferskhet, hvert miljøs transport, og at hver komponents playbook løses til en entydig målmodus
help-cmd-status = Deklarert mot registrert, per maskin: konvergert, ventende eller ukjent (C7, C8). Kun lesing. Avslutter med 3 så lenge en maskin ikke er konvergert
help-cmd-deploy = Legg pinnen main deklarerer ut på ett miljø og bevis det: løs komponenten og miljøets grupper fra pinnene, playen og målmodusen, hver hemmelighet playen maler inn, og transporten; kjør playen gjennom ansible-preflight; les deploy-ledgeren tilbake fra hver vert og regn det som ferdig først når den nyeste raden angir den deklarerte digesten med action=deploy (C7). På en fixture (`--mock`) står den skriptede konvergeren i for vertene
help-cmd-release = Ta en release candidate fra en tag på main gjennom dev-goatnets live-validering til preprod-trinnet (ADR 1173); etter soak, med `--from prod`, graduér den, fold den inn i fleet:, deployer prod og verifisér alle trinn på nytt. Erstatter ops/scripts/rc.sh steg for steg
help-cmd-rollback = Rull en deployet komponent tilbake til dens N-te nyeste ref, etter artefakttype, med ledgeren på verten som sannhetskilde (ADR 1060). Erstatter ops/scripts/rollback.sh: avslutter 3 avvist som utrygt (data, cert, forward-only, reversible:false), 4 veiledet (config, acl)
help-cmd-fastlane = Én kommando fra en merget commit til alle goatnet (ADR 1127): det bevisste unntaket fra stigen. Erstatter ops/scripts/fastlane.sh steg for steg; avviser en upinnet digest eller et åpent graduation-sett, og kjører deployene gjennom denne binærfilen
help-cmd-standup = Et site fra ingenting til verifisert: standupens faser i doktrinens rekkefølge med en fail-closed-port i hver ende, observability-verten som et parallelt løp, rescue-klassene og tverr-goatnet-flatene sist (ADR 1183). Erstatter ops/scripts/goat-standup.sh steg for steg. På en fixture (`--mock`) konvergeres hver komponent i hver klasse av den skriptede konvergeren, og tiden deklarert → verifisert er målet til én-times-goatnet
help-cmd-join-node = Én maskin til av en klasse på et live site, deklarert → konvergert → verifisert: den utførte sekvensen i docs/tutorials/02-first-federated-node.md som nummererte faser med sine porter (nåbarhet, nøkkel, repo-arbeid først, baseline, kjerne-WireGuard før og etter enrolment, kjernens ACL-plan som enrolment-port, klassens converge, akseptanse)
help-cmd-federate = En suverén peering deklarert, konvergert og verifisert (ADR 1157): vår halvdel målt, pakkene utvekslet utenfor båndet, én avtale komponert og verifisert, bladene utstedt fra den, reglene rendret av den ene skriveren, kanten deployet, transporten avtalen valgte åpnet, akseptanse i alle fire nivåer. Erstatter ops/scripts/peering-standup.sh steg for steg
help-cmd-pins = ÉN løsningsrunde med TO renderinger: hver oppførte medlems group_vars pins.yml og hver oppførte goatnets IntentV1-artefakt, fra ops/pins/fleet-pins.yaml. Erstatter ops/pins/generate-pins.py byte for byte. `--check` validerer, regenererer og avviser drift, en usignert eller manipulert intent, og en styrt pin definert utenfor pins.yml (exit 1)
help-cmd-promote = Veiled og observer release-graduering på tvers av goatnet-stigen (ADR 1120, 1125): status, åpne en kandidat, graduér, graduér et sett, hotfix-veien, review, readiness, abandon, check. Erstatter ops/release/graduate.py steg for steg. Ingenting her graduerer av seg selv: en mutasjon kjører bare når den kalles, skriver ut hva den gjorde, og ber om --yes uten terminal (C10, ADR 1177)
help-cmd-explain = Tolk en feil: en kode (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), eller en hvilken som helst feiltekst – denne binærfilens egen, eller et verktøys ord den bar videre (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Skriver ut hva som skjedde, hvorfor, hva du bør gjøre og hvor du kan lese mer; `-` leser teksten fra stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Et site fra verifisert til ingenting. Speilbildet av standup, med bevarte oppføringer (en nedrigging legger til teardown-rader, sletter aldri). På en fixture (`--mock`) øves nedriggingen i omvendt faserekkefølge. På ekte verter skrives rekkefølgen ut, og kjøring nektes til hver fase har deklarert sitt teardown-play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Leksjoner i vimtutor-stil: hver nevner nøyaktig hvilken kommando som skal kjøres mot en engangskopi av en fixture (eller, skrivebeskyttet, mot denne utsjekken), og sjekker så hva kommandoen faktisk gjorde (exit-kode og rader) før neste låses opp. `tutor` viser leksjonen du er på og markerer den som ferdig når kjøringene dine oppfyller den; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skjult (src/fun.rs)
help-cmd-argan = Skjult (src/fun.rs)
help-cmd-bound = Skjult (crates/goat-fun)
help-cmd-secrets = Den deklarerte hemmelighetsposituren (ADR 1204): hvilket lager som gjelder (1password, openbao, sops, git-crypt, env-file, env), hva som er tilgjengelig, hvordan du leverer det som mangler, og lagerets egne sjekker. `init --env <goatnet>` skriver env-filen til null-SaaS-hurtigstarten: en sterk tilfeldig verdi for hver variabel goatnetets plays templater, 0600, gitignorert, uten å rotere en verdi som allerede finnes. Samme modul som ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Les hver verts nyeste rad i deploy-loggen for én komponent og si om den angir refen main deklarerer (C7). Kun lesing; avslutter med 3 så lenge noen vert er ventende eller ukjent. Samme sjekk som `deploy --verify-only` kjører
help-cmd-diff = Hva som er annerledes fra main nå (C8): mains deklarerte ref mot hver verts nyeste loggrad, for én komponent eller, uten, alle komponenter i miljøet, lest i ett pass per medlemsgruppe. Kun lesing; avslutter med 3 ved avvik eller uleselig vert
help-cmd-history = Deploy-loggen som tidslinje, nyeste først, på alle verter i miljøet (C8): hver rad er en post med maskinen, tidspunktet, handlingen (deploy eller rollback) og versjonen. Postens forrige versjon og operatøren som kjørte den, står i `-o json`. Kun lesing
help-cmd-plan = Skriv det et deklarativt verb ville gjort til en planfil for gjennomgang: deklarerte refs, play og mål, loggen, mains commit, transporten (deploy); artefakttypen og målref (rollback). Rører ingen verts tilstand. Kjør den som gjennomgått med `apply`
help-cmd-apply = Kjør nøyaktig en gjennomgått plan. Avviser (avslutter med 1, med angivelse av hva som har flyttet seg) når mains deklarasjon, pins-committen, play eller rollback-målet har flyttet seg siden planen ble skrevet; endret transport rapporteres, ikke avvist
help-cmd-adopt = Deklarer det et miljø allerede kjører: les alle verters logger og foreslå et pins-avvik for hver komponent der kjørende ref avviker fra mains deklarasjon. Som standard en rapport; med --yes skrives endringene til et nytt worktree av main for gjennomgang. Deployer aldri

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ikke ennå: { $why }
tutor-lesson-done = Leksjon { $n } fullført: { $title }.
tutor-all-done-reset = Alle leksjoner er fullført. `goatbuilder tutor --reset` starter på nytt; veiledningene går lenger: docs/tutorials/.
tutor-all-done = Alle leksjoner er fullført. Veiledningene går lenger: docs/tutorials/.
tutor-workspace = Arbeidsområde: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Sjekk at et goatnet er bygget riktig, lag for lag: alle akseptsjekker som gjelder for stedet, inkludert sjekkene en operatør kjører med goat CLI. Akseptfasen i oppstarten som egen kommando. Kun lesing; avslutter med kode 3 så lenge en sjekk feiler eller ikke kan svare
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Isolert lokasjon: ikke hent trunk; vurder utsjekken mot den hurtiglagrede kopien, og registrer kjøringen som frakoblet (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Sjekk at et goatnet er i deklarert sluttilstand: hver tjeneste som egne deklarasjoner sier at det kjører, er utplassert, kjører, er frisk og gjør jobben sin, med zero-trust-kantene koblet opp, og ingenting kjører som ikke er deklarert. Kun lesing; avslutter med 3 så lenge noe er feil
help-cmd-survey = Hvor konsistent programvaren er innenfor hvert goatnet og på tvers av stigens goatnet, lest i én parallell kjøring. Leser bare

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Hvor konsistent programvaren er innad i hvert goatnet og på tvers av stigens goatnet, lest i én parallell runde. Kun lesing.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Erstatt denne binæren med utgivelsen goatnet-et serverer på `get.<site>.<zone>/goatbuilder/`: nedlastingen sjekkes mot `.sha256` og den signerte `SHA256SUMS` før noe røres, og byttet er atomisk. Allerede oppdatert: én rad, exit 0. `--check` kun leser (exit 3 når du ligger bak). Flaten: `GOATBUILDER_UPDATE_URL`, ellers `--site` fra group_vars, ellers den forrige oppdatering brukte
tutor-prereq = Før du starter: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Farg tabellens ok/FAIL-symbol og dom (auto|always|never). `auto` farger bare en terminal; NO_COLOR og CLICOLOR_FORCE overstyrer. JSON og YAML fargelegges aldri
help-flag-verbose = Logg hva en kjøring gjør — de eksterne kommandoene den starter, og nettverksfasen i `update` — til stderr, slik at `-o json|yaml` forblir et rent dokument. Eller sett RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunder som begrenser tilkoblings-/nåbarhetsfasen før den feiler raskt — den ene fasen som kan blokkere på en utilgjengelig vert (`update` sin henting av den tilbudte versjonen). Begrenser IKKE et plays egen kjøring eller nedlastingen av binærfilen (stopp-begrenset). Leser også GOATBUILDER_TIMEOUT
help-cmd-completion = Skriv ut et shell-fullføringsskript til stdout (`goatbuilder completion zsh >> ~/.zshrc`). For levende kandidater et statisk skript ikke kan kjenne — stedene, komponentene og goatnets denne utsjekkingen deklarerer — koble en hook til `goatbuilder _complete` (se toppteksten i det genererte skriptet)
help-cmd-docs = Kommandoreferansen: `goatbuilder docs` peker til den; `--markdown` skriver ut hele greia, generert fra det levende kommandotreet slik at den aldri kan avvike fra `--help`
help-cmd-_complete = Skjult: skriv ut levende fullføringskandidater for shell-hooks, én per linje (stedene, komponentene og goatnets denne utsjekkingen deklarerer). Leser bare, feiler aldri høylytt — en fullføringshook må ikke ødelegge prompten

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Før en utrulling: lesebildet for beredskap og reversibilitet (ADR 1211) — tre fakta: State (stateless | stateful), Rollback (pin-revert | restore-required | none) og Scope (single | coupled) — deklareres per komponent og FAIL-CLOSED når de mangler, med beredskapsdelen som de andre verbene beviser (ref kan hentes, pins --check, hemmelighetsporten, ferske kilder, den deklarerte sluttilstanden) og en validering av rollback-faktaet (rollback --dry-run for pin-revert, oppbevaringsvinduet for restore-required). `deploy` kjører den først; ærlighetsgradert, aldri en falsk grønn. Leser bare; exit 3 når ikke ren

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kontroller oppsettet ditt
tutor-l1-s1 = Kontroller konfigurasjonsfilen
tutor-l2-title = Les tilstand: converged, pending, unknown
tutor-l2-text =
    Kommandoen goatbuilder `status` sammenligner to ting for hver maskin: versjonen main deklarerer for den, og versjonen posten dens sier den kjører. Den endrer ingenting.

    Hver maskin er i en av tre tilstander:
    - converged (konvergert): posten dens navngir den deklarerte versjonen. Dette er ferdig.
    - pending (ventende): posten dens navngir en eldre versjon, eller ingen ennå.
    - unknown (ukjent): goatbuilder kan ikke lese posten dens.

    Bare converged teller som ferdig. Hvis en maskin er pending eller unknown, avslutter `status` med kode 3.

    Fra denne leksjonen av peker `--mock` goatbuilder mot en øvingskopi i tutor-mappen din i stedet for virkelige maskiner. Ingenting du gjør der når en virkelig maskin.

    Ingenting er deployet til øvingskopiens `dev` ennå. Vent derfor fire maskiner pending og en unknown. Du vil se converged maskiner i leksjon 3.
tutor-l2-s1 = Les tilstanden for dev
tutor-l3-title = Deploy: planlegg det, kjør det, bevis det
tutor-l3-text =
    Å deploye legger den deklarerte versjonen på hver maskin, og leser så hver maskins post for å bevise at den kom frem. En deploy er ferdig bare når hver maskins post navngir den versjonen. En deploy som kjørte uten feil er ikke nok.

    Du skal deploye `portal` til øvingskopiens `preprod`. `--dry-run` viser hva som ville skje og endrer ingenting. `--ack` registrerer hvem som gjorde det; kommandoen fyller inn navnet ditt fra git. Etter deployen skal `status` vise begge maskinene som `ok` og converged.
tutor-l3-s1 = Planlegg deployen
tutor-l3-s2 = Deploy portal til preprod
tutor-l3-s3 = Kontroller at begge maskinene er converged
tutor-l4-title = Drift og historie
tutor-l4-text =
    To kommandoer hjelper deg å forstå et goatnet før og etter en endring. Begge bare leser.
    - Kommandoen goatbuilder `diff` lister hver maskin hvis post avviker fra den deklarerte versjonen. Et slikt avvik kalles drift.
    - Kommandoen goatbuilder `history` lister hver versjon en komponent har hatt på hver maskin, nyeste først.

    På `dev` ligger hver maskin bak. Historien til `portal` på `preprod` inkluderer nå din deploy fra leksjon 3.
tutor-l4-s1 = List driften på dev
tutor-l4-s2 = Les historien til portal på preprod
tutor-l5-title = Rull tilbake, og se en utrygg rollback bli avvist
tutor-l5-text =
    En rollback deklarerer en eldre versjon og deployer den som enhver annen versjon. goatbuilder CLI-en leser så hver maskins post for å bevise at den kom frem.

    Noen endringer kan ikke angres på denne måten. Å deploye en eldre versjon kan ikke angre data en komponent har skrevet, et sertifikat, eller en endring som bare går fremover. goatbuilder CLI-en avviser disse med avslutningskode 3 og sier hva du skal gjøre i stedet.

    Øvingskopiens `db-schema`-komponent holder data, så goatbuilder avviser å rulle den tilbake. Den avvisningen er det forventede resultatet.
tutor-l5-s1 = Planlegg rollbacken
tutor-l5-s2 = Rull portal tilbake en versjon
tutor-l5-s3 = Prøv å rulle tilbake db-schema, som goatbuilder avviser
tutor-l6-title = Release en ny versjon: dev, så preprod, så prod
tutor-l6-s1 = Se hvor hver komponent er
tutor-l6-s2 = Prøvekjør releasen fra dev
tutor-l6-s3 = Prøvekjør prod-steget
tutor-l7-title = Fastlane, og når du ikke skal bruke det
tutor-l7-text =
    Fastlane legger én versjon på hvert goatnet på én gang. Den hopper over ventetiden på preprod og release-postene.

    Bruk det bare for en akutt fiks du kan angre ved å gå tilbake til den forrige versjonen. Bruk det aldri for en databaseendring, en identitets- eller tilgangsendring, eller noe annet du ikke kan angre på den måten.

    Fastlane avviser en versjon CI ikke har bygget, og en komponent som er del av en release fremdeles i gang.

    Den generelle formen er `goatbuilder fastlane <component> --ref <version> --reason <why>`. Kjør eksempelet under. Det har `--dry-run`, så ingenting endres. Det krever `gh`.
tutor-l7-s1 = Prøvekjør en fastlane
tutor-l8-title = Pins: finn drift og fiks det på riktig måte
tutor-l8-text =
    En fil bestemmer hvilken versjon hvert goatnet kjører: `ops/pins/fleet-pins.yaml`. goatbuilder CLI-en genererer de andre versjonsfilene fra den: `pins.yml`-filene og de signerte intent-filene.

    Kommandoen goatbuilder `pins --check` sammenligner de genererte filene med `fleet-pins.yaml` og avviser enhver fil som avviker. Kommandoen goatbuilder `pins` regenererer dem.

    Tutor-mappen din har en kopi av repositoriet der noen redigerte en `pins.yml`-fil for hånd. Fiks aldri en håndredigering med en ny håndredigering. Endre `fleet-pins.yaml`, og regenerer så.
tutor-l8-s1 = Kontroller denne utsjekken
tutor-l8-s2 = Kontroller kopien og finn håndredigeringen
tutor-l8-s3 = Regenerer kopiens filer
tutor-l8-s4 = Kontroller kopien igjen
tutor-l9-title = Forklar en feil
tutor-l9-s2 = Forklar feilen med koden
tutor-l9-s3 = Forklar feilen med teksten
tutor-l10-title = Veiledning 1: bygg et nytt goatnets første site
tutor-l10-text =
    Denne leksjonen bygger et helt goatnet i en øvingskopi. Eksempel-goatnetet er `zagros`. Det første sitet, `alpha`, har to kjernemaskiner, en overvåkingsmaskin og en redningsmaskin.

    Kommandoen goatbuilder `standup` setter opp hver maskin i en fast rekkefølge og kontrollerer hver post. Siste linje sier at sitet er ferdig. På virkelige maskiner viser linjen også hvor lang tid det tok.

    Den fullstendige veiledningen er `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Bygg det første sitet, alpha
tutor-l10-s2 = Kontroller resultatet
tutor-l11-title = Veiledning 2: legg til et andre site
tutor-l11-text =
    Denne leksjonen starter der veiledning 1 sluttet. Den legger til et andre site, `beta`, i en ny øvingskopi, `zagros-2`.

    Et andre site kopierer sine brukerkontoer fra det første sitet. Standupen dens endrer også det første sitet én gang, så kopieringen fungerer i begge retninger. Et steg kalt `alpha/core` gjør dette.

    Den fullstendige veiledningen er `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Bygg det andre sitet, beta
tutor-l11-s2 = Kontroller begge sitene
tutor-l12-s1 = Planlegg å legge til en maskin
tutor-l13-s1 = Planlegg en federation
tutor-l14-title = Hold goatbuilder oppdatert
tutor-l14-s1 = Spør om en nyere goatbuilder finnes
tutor-l15-title = Assay: kontroller hvordan et goatnet ble bygget
tutor-l15-text =
    Kommandoen goatbuilder `assay` kontrollerer at et goatnet ble bygget korrekt, ett lag om gangen, ved hjelp av akseptansetestene. Hver test får ett av tre resultater:
    - PASS: testen passerte.
    - FAIL: goatnetet har et problem.
    - UNKNOWN: ingenting kunne besvare testen.

    Bare PASS teller. Enhver FAIL eller UNKNOWN gir avslutningskode 3.

    `--plan` lister testene som gjelder for et goatnet, lag for lag, og rører ingen maskin. Bruk den for å se hva en reell assay ville dekket. Den generelle formen er `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: er versjonene konsistente?
tutor-l16-text =
    Kommandoen goatbuilder `survey` kontrollerer i én omgang om de samme versjonene kjører der de skal:
    - Inne i hvert goatnet: kjører hver maskin den deklarerte versjonen, og stemmer maskiner som skal stemme, faktisk?
    - Mellom dev, preprod og prod: én rad per komponent, én kolonne per goatnet.

    En forskjell som releaseprosessen forventer blir rapportert, men er ikke en feil. Et eksempel er en ny versjon som er på dev, men ikke på prod ennå. En forskjell som ingenting deklarerer, leses som `difference not declared`. En maskin goatbuilder ikke kan lese er unknown, aldri konsistent.

    Kjør den på øvingskopien. Dens `dev` ligger bevisst bak, så forvent funn og avslutningskode 3.
tutor-l16-s1 = Survey øvingskopien
tutor-l17-title = Endstate: kjører et goatnet nøyaktig det det deklarerer?
tutor-l17-text =
    Kommandoen goatbuilder `endstate` kontrollerer to ting om et goatnet:
    1. Hver tjeneste det deklarerer er deployet, kjører, er sunn og korrekt tilkoblet.
    2. Ingenting kjører som det ikke deklarerer. En udeklarert tjeneste kalles en foreldreløs tjeneste.

    goatbuilder CLI-en regner ut hva et site skal kjøre fra sitets egne innstillinger, ikke fra en separat liste.

    En kontroll som ikke har noe å lese merkes UNOBSERVED. Det er et ærlig svar, ikke en feil. En FAIL eller en foreldreløs tjeneste gir avslutningskode 3.

    Den generelle formen er `goatbuilder endstate <goatnet>`. Kommandoen under kontrollerer et eksempelsite fra lagrede data, så den rører ingen maskin.
tutor-l17-s1 = Kontroller eksempelsitet
tutor-l18-title = Bonus: vær nysgjerrig
tutor-l18-text =
    Denne leksjonen er valgfri. Kjør kommandoen under, og tutoren er ferdig.

    goatbuilder CLI-en har noen få små skjulte funksjoner. De er ikke i hjelpen eller i disse leksjonene. Noen svarer på ord som erfarne Unix-brukere ofte skriver inn i et nytt program, bare for å se hva som skjer. De fleste ting du prøver vil ikke gjøre noe.

    Denne leksjonen sier ikke hva de er. Å finne dem er poenget.

    Kommandoen under lister hver feil goatbuilder kan forklare. Det er et godt sted å starte utforskingen.
tutor-l18-s1 = List hver feil goatbuilder kan forklare
tutor-heading = Leksjon { $n } av { $total } · { $title }
tutor-setup = Kjør først dette én gang i denne terminalen:
tutor-not-recording = Denne terminalen registrerer ikke kjøringene dine, så tutoren kan ikke krysse av stegene dine. Kjør dette én gang, og kjør så stegene:
tutor-type = Gjør disse stegene i terminalen din ({ $met } av { $total } ferdige):
tutor-rerun = Kjør `goatbuilder tutor` igjen etter hvert steg. Den kontrollerer hva du gjorde og krysser av steget.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Velkommen til GOATBUILDER Tutor
tutor-bar-progress = Fremdrift
tutor-bar-done = { $done } av { $total } leksjoner ferdige
tutor-after-step = Steg { $met } av { $total } ferdig. Neste:
tutor-after-lesson = Leksjon { $n } er ferdig. Kjør `goatbuilder tutor` for å starte leksjon { $next }. For å lese den igjen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Leksjon { $n } er ferdig, og det er tutoren også. Kjør `goatbuilder tutor` for å se den merket som ferdig. For å lese den igjen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Du er ferdig med denne leksjonen, så denne siden er bare til lesing. Kjør `goatbuilder tutor` for å gå tilbake til der du er.
tutor-other-lessons = For å lese en leksjon igjen: `goatbuilder tutor --lesson 1`. For å se dem alle: `goatbuilder tutor --list`.
tutor-l12-title = Legg til en maskin på et kjørende site
tutor-l12-text =
    Kommandoen goatbuilder `join-node` legger én maskin til på et site som allerede kjører. Den skriver ut en plan først, med én rad per fase. Hver fase har en type:
    - tool: goatbuilder kjører et skript.
    - ansible: goatbuilder kjører en playbook.
    - gate: en kontroll som må passere før neste fase.
    - manual: du gjør dette selv, og kjøringen pauser.

    Kommandoen under har `--dry-run`. Den leser denne utsjekken og endrer ingenting. Under tabellen kan du lese hva du gjør manuelt.
tutor-l13-title = Koble til en annen organisasjons goatnet
tutor-l13-text =
    Kommandoen goatbuilder `federate` kobler ditt goatnet til et goatnet en annen organisasjon kjører. Hver side bygger sin halvdel av en avtale. Dere bytter halvdeler, og hver side kombinerer dem til samme avtale.

    Bekreft den andre sidens nøkkel på en annen måte, som en samtale du har tiltro til. En nøkkel som kommer inni pakken deres, beviser ingenting.

    Kommandoen under har `--dry-run`, så den endrer ingenting. En reell kjøring stopper i fase 4, fordi du gjør den fasen manuelt.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Denne kommandoen er ikke en del av leksjon { $n }. Kjør `goatbuilder tutor` for å se hvor du er.
tutor-l1-s2 = Spør doctor om ett goatnet
tutor-l1-text =
    goatbuilder CLI-en bygger, deployer og utgir GOAT-programvare. Før du bruker den, kontroller oppsettet ditt med to kommandoer:
    - Kommandoen goatbuilder `validate` kontrollerer konfigurasjonsfilen, `ops/goatbuilder/goatbuilder.yaml`. Den leser bare denne checkouten og trenger ikke nettverk.
    - Kommandoen goatbuilder `doctor` kontrollerer det en ekte deploy trenger: verktøyene dine, innloggingene dine, om denne checkouten samsvarer med main, og hvordan goatbuilder når hvert goatnet. Et goatnet er ett GOAT-miljø.

    Kommandoen goatbuilder `doctor` skriver ut hvor mange kontroller som besto, og deretter hvert problem med løsningen. I denne leksjonen er et problem informasjon, ikke en feil. Les løsningene, men du trenger ikke å bruke dem nå.

    De fleste leksjoner trenger ingen innlogging. Leksjon 2 til 5, 10, 11 og 16 bruker en øvingskopi. Leksjon 8, 9, 12, 13, 15, 17 og 18 leser bare denne checkouten. Leksjon 6 trenger `gh` og meshet, leksjon 7 trenger `gh`, og leksjon 14 trenger meshet. Ingen leksjon trenger 1Password.
tutor-l14-text =
    goatbuilder CLI-en kan oppdatere seg selv fra kopien som goatnetet ditt serverer på `get.<site>.<zone>/goatbuilder/`.

    Kommandoen goatbuilder `update --check` leser bare. Den sammenligner versjonen din med den serverte versjonen og endrer ingenting. Avslutningskode 0 betyr at du er oppdatert. Avslutningskode 3 betyr at en nyere versjon finnes.

    En ekte oppdatering, uten `--check`, verifiserer sjekksummen og signaturen til nedlastingen før den erstatter noe. Deretter bytter den ut filen i ett steg.

    goatbuilder CLI-en må vite hvilket goatnet den skal spørre: send `--site` med det goatnetet, eller sett `GOATBUILDER_UPDATE_URL`. Du må være på meshet til det goatnetet. Hvis du ikke er det, sier `--check` at den ikke når serveren, og navngir adressen den prøvde. Det er et riktig resultat for denne leksjonen.
tutor-l15-s1 = List opp testene som gjelder for ett goatnet
tutor-l6-text =
    En utgivelse flytter en ny versjon gjennom tre goatnet i rekkefølge: dev, deretter preprod, deretter prod.

    1. Noen tagger versjonen på main, og CI bygger den.
    2. dev: goatbuilder deployer den og kjører akseptansetestene.
    3. preprod: goatbuilder deployer den igjen, kjører testene igjen og registrerer resultatene.
    4. Versjonen venter deretter på preprod i en fastsatt tid. Ingenting flytter seg til prod av seg selv.
    5. prod: etter den ventetiden kjører en person utgivelsen med `--from prod`. goatbuilder CLI-en deployer til prod og kontrollerer alle tre goatnettene.

    Kommandoen goatbuilder `promote status` viser hvor hver komponent er. En kandidat er en versjon som testes. Den generelle formen for en utgivelse er `goatbuilder release <component> --tag <tag> --from dev`. Kommandoene nedenfor bruker én komponent fra denne checkouten og en eksempeltag. Hver av dem har `--dry-run`, så ingenting blir tagget eller deployet.

    Prøvekjøringene trenger `gh` og meshet. Hvis du ikke er admin for repositoriet, sett `GOAT_RC_ROLE_CHECK=0` foran hver `release`-kommando.
tutor-l9-s1 = Lag en ekte feil med et goatnet som ikke finnes
tutor-l9-text =
    Når goatbuilder avviser, skriver den ut en kode. Koder som begynner med `GB-` kommer fra goatbuilder, `E_` fra goat CLI-en, og `X-` fra et annet verktøy som goatbuilder kjørte.

    Kommandoen goatbuilder `explain` tar en kode, eller teksten i en feillinje, og forteller deg hva som skjedde, hvorfor, hvordan du løser det, og hvor du kan lese mer.

    Å be om et goatnet som denne checkouten ikke deklarerer, gir en ekte feil å forklare.
