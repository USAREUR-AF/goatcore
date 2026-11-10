### goatbuilder — sv (sv)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Bygg och leverera en goat-miljö: deklarera, konvergera, verifiera — under ett och samma driftsättningskontrakt
help-flag-output = Utdataformat: tabell för människor; JSON eller YAML för program
help-flag-repo = Repots rotkatalog. Standard: den utcheckning som den här binären körs i
help-flag-yes = Bekräfta ett deklarativt verb utan fråga (för automation)
help-flag-frozen = Vägra varje körning som skulle kräva snabbspolning av utcheckningen (strikt C3)
help-flag-mock = Kör mot en fixturkatalog i stället för värdar: inget register, ingen transport, deterministiskt (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Språk för hjälp och felförklaringar (en tagg som fr, uk, de); utdata som en pipeline läser förblir engelsk. Standard: OS-lokalen
help-cmd-validate = Kontrollera en konfig mot det versionerade schemat och reglerna mellan fälten, offline
help-cmd-doctor = Sondera vad en körning behöver innan något skärs: beroenden, utcheckningens färskhet, varje miljös transport, och att varje komponents playbook leder till ett entydigt målläge
help-cmd-status = Deklarerat mot registrerat, per maskin: konvergerad, väntande eller okänd (C7, C8). Läser bara. Exit 3 så länge någon maskin inte är konvergerad
help-cmd-deploy = Lägg den pin som main deklarerar på en miljö och bevisa det: lös ut komponenten och miljöns grupper från pinnarna, spelet och dess målläge, varje hemlighet som spelet mallar, transporten; kör spelet via ansible-preflight; läs tillbaka driftsättningsloggboken från varje värd och räkna det som klart först när den nyaste raden anger den deklarerade digesten med action=deploy (C7). På en fixtur (`--mock`) ersätter den skriptade konvergeraren värdarna
help-cmd-release = Ta en releasekandidat från en tagg på main genom dev-goatnets live-validering till preprod-steget (ADR 1173); efter soak, med `--from prod`, graduera den, vik in den i fleet:, driftsätt prod och verifiera om varje steg. Ersätter ops/scripts/rc.sh steg för steg
help-cmd-rollback = Rulla tillbaka en driftsatt komponent till dess N:te senaste ref, per artefakttyp, med loggboken på värden som sanningskälla (ADR 1060). Ersätter ops/scripts/rollback.sh: exit 3 nekad som osäker (data, cert, forward-only, reversible:false), 4 guidad (config, acl)
help-cmd-fastlane = Ett kommando från en mergad commit till alla goatnet (ADR 1127): det medvetna undantaget från stegen. Ersätter ops/scripts/fastlane.sh steg för steg; vägrar en ofastnålad digest eller en öppen graduerings­uppsättning, och kör driftsättningarna genom den här binären
help-cmd-standup = En site från ingenting till verifierad: standup-faserna i doktrinens ordning med en fail-closed-grind i vardera änden, observabilitetsvärden som parallellt spår, rescue-klasserna och ytorna mellan goatnet sist (ADR 1183). Ersätter ops/scripts/goat-standup.sh steg för steg. På en fixtur (`--mock`) konvergeras varje komponent i varje klass av den skriptade konvergeraren, och tiden deklarerad → verifierad är en-timmes-goatnets mått
help-cmd-join-node = En maskin till av en klass på en live-site, deklarerad → konvergerad → verifierad: docs/tutorials/02-first-federated-node.md:s utförda sekvens som numrerade faser med sina grindar (nåbarhet, nyckel, repoarbete först, baslinje, kernel-WireGuard före och efter enrollment, kernel-ACL-planet som enrollment-grind, klassens converge, acceptans)
help-cmd-federate = En suverän peering deklarerad, konvergerad och verifierad (ADR 1157): vår halva uppmätt, paketen utbytta utanför bandet, ett avtal komponerat och verifierat, löven mintade från det, reglerna renderade av den enda skribenten, kanten driftsatt, den transport avtalet valde öppnad, acceptans i alla fyra nivåer. Ersätter ops/scripts/peering-standup.sh steg för steg
help-cmd-pins = EN upplösningspassning med TVÅ renderingar: varje listad medlems group_vars pins.yml och varje listat goatnets IntentV1-artefakt, från ops/pins/fleet-pins.yaml. Ersätter ops/pins/generate-pins.py byte för byte. `--check` validerar, regenererar och vägrar drift, en osignerad eller manipulerad intent, och en styrd pin definierad utanför pins.yml (exit 1)
help-cmd-promote = Guida och observera releasegradueringen över goatnet-stegen (ADR 1120, 1125): status, öppna en kandidat, graduera, graduera en uppsättning, hotfix-vägen, granskning, beredskap, överge, kontroll. Ersätter ops/release/graduate.py steg för steg. Inget här graduerar av sig självt: en mutation körs bara när den anropas, skriver ut vad den gjorde och kräver --yes utan terminal (C10, ADR 1177)
help-cmd-explain = Tolka ett fel: en kod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), eller valfri feltext — binärens egen, eller ett verktygs ord som den förde vidare (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Skriver ut vad som hände, varför, vad du ska göra och var du läser mer; `-` läser texten från stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = En site från verifierad till ingenting — standups spegelbild, poster behålls (en nedmontering lägger till teardown-rader, raderar aldrig). På en fixtur (`--mock`) repeteras nedmonteringen i omvänd fasordning; på riktiga värdar skrivs ordningen ut och körningen vägras tills varje fas deklarerar sin teardown-play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lektioner i vimtutor-stil: varje lektion anger exakt vilket kommando som ska köras mot en engångskopia av en fixtur (eller, skrivskyddat, mot den här utcheckningen) och kontrollerar sedan vad kommandot faktiskt gjorde — exitkod och rader — innan nästa låses upp. `tutor` visar lektionen du är på och markerar den klar när dina körningar uppfyller den; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Dold (src/fun.rs)
help-cmd-argan = Dold (src/fun.rs)
help-cmd-bound = Dold (crates/goat-fun)
help-cmd-secrets = Den deklarerade hemlighetsposturen (ADR 1204): vilket lager som gäller (1password, openbao, sops, git-crypt, env-file, env), vad som finns tillgängligt, hur du tillhandahåller det som saknas samt lagrets egna kontroller. `init --env <goatnet>` skriver env-filen för snabbstarten utan SaaS: ett starkt slumpvärde för varje variabel som goatnetets plays mallar in, 0600, gitignorerad, och roterar aldrig ett värde som redan finns. Samma modul som ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Läs varje värds senaste deploy-ledger-rad för en komponent och säg om den anger den ref som main deklarerar (C7). Läser bara; exit 3 så länge någon värd är pending eller unknown. Samma kontroll som `deploy --verify-only` kör
help-cmd-diff = Vad som skiljer sig från main just nu (C8): mains deklarerade ref mot varje värds senaste ledger-rad, för en komponent eller, utan angiven komponent, alla komponenter i miljön, lästa i ett pass per medlemsgrupp. Läser bara; exit 3 vid avvikelse eller oläsbar värd
help-cmd-history = Deploy-ledgern som tidslinje, nyast först, på varje värd i miljön (C8): varje rad är en post med maskinen, tiden, åtgärden (deploy eller rollback) och versionen. Postens tidigare version och operatören som körde den finns i `-o json`. Läser bara
help-cmd-plan = Skriv till en planfil, för granskning, vad ett deklarativt verb skulle göra: de deklarerade refs, play och mål, ledgern, mains commit, transporten (deploy); artefakttypen och målref (rollback). Rör ingen värds tillstånd. Kör den granskade planen med `apply`
help-cmd-apply = Kör exakt en granskad plan. Vägrar (exit 1, med angivelse av vad som flyttats) när mains deklaration, pins-committen, play eller rollback-målet har flyttats sedan planen skrevs; en ändrad transport rapporteras, vägras inte
help-cmd-adopt = Deklarera vad en miljö redan kör: läs varje värds ledgers och föreslå en pins-avvikelse för varje komponent vars körande ref skiljer sig från mains deklaration. Som standard en rapport; med --yes skrivs ändringarna till ett nytt worktree av main för granskning. Deployar aldrig

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ännu inte: { $why }
tutor-lesson-done = Lektion { $n } klar: { $title }.
tutor-all-done-reset = Alla lektioner är klara. `goatbuilder tutor --reset` börjar om; handledningarna går längre: docs/tutorials/.
tutor-all-done = Alla lektioner är klara. Handledningarna går längre: docs/tutorials/.
tutor-workspace = Arbetsyta: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Kontrollera att ett goatnet byggts korrekt, lager för lager: alla acceptanskontroller som gäller för anläggningen, inklusive de kontroller en operatör kör med goat CLI. Standup-fasen för acceptans som eget kommando. Läser bara; avslutar med kod 3 så länge någon kontroll misslyckas eller inte kan svara
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Isolerad site: hämta inte trunk; bedöm utcheckningen mot den cachade, och registrera körningen som offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Kontrollera att ett goatnet är i sitt deklarerade slutläge: varje tjänst som dess egna deklarationer säger ska köras är utplacerad, körs, är frisk och gör sitt jobb, med sina zero-trust-kanter kopplade, och inget körs som inte deklareras. Läser bara; avslutar med kod 3 så länge något avviker
help-cmd-survey = Hur konsekvent mjukvaran är inom varje goatnet och mellan stegens goatnet, avläst i en enda parallell körning. Läser endast

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Hur konsekvent programvaran är inom varje goatnet och mellan goatnets i laddern, avläst i ett parallellt pass. Endast läsning.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Ersätt den här binären med den release som dess goatnet serverar på `get.<site>.<zone>/goatbuilder/`: nedladdningen kontrolleras mot sin `.sha256` och den signerade `SHA256SUMS` innan något rörs, och bytet är atomärt. Redan aktuell: en rad, exit 0. `--check` läser bara (exit 3 när den ligger efter). Källan: `GOATBUILDER_UPDATE_URL`, annars `--site` från dess group_vars, annars den som senaste uppdateringen använde
tutor-prereq = Innan du börjar: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Färglägg tabellens ok/FAIL-symbol och utfall (auto|always|never). `auto` färglägger bara i en terminal; NO_COLOR och CLICOLOR_FORCE har företräde. JSON och YAML färglägns aldrig
help-flag-verbose = Logga vad en körning gör — de externa kommandon den anropar och nätverksfasen i `update` — till stderr, så att `-o json|yaml` förblir ett rent dokument. Eller sätt RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunder att begränsa anslutnings-/nåbarhetsfasen innan körningen snabbt misslyckas — den enda fasen som kan blockera på en onåbar värd (`update`s hämtning av den serverade versionen). Begränsar INTE en plays egen körning eller nedladdningen av binären (stallbegränsad). Läser även GOATBUILDER_TIMEOUT
help-cmd-completion = Skriv ut ett skalkompletteringsskript till stdout (`goatbuilder completion zsh >> ~/.zshrc`). För levande kandidater som ett statiskt skript inte kan känna till — de sites, komponenter och goatnets som den här utcheckningen deklarerar — koppla en hook till `goatbuilder _complete` (se det genererade skriptets header)
help-cmd-docs = Kommandoreferensen: `goatbuilder docs` pekar på den; `--markdown` skriver ut hela referensen, genererad från det levande kommandoträdet så att den aldrig kan avvika från `--help`
help-cmd-_complete = Dold: skriver ut levande kompletteringskandidater för skalhookar, en per rad (de sites, komponenter och goatnets som den här utcheckningen deklarerar). Läser bara, misslyckas aldrig högljutt — en kompletteringshook får inte förstöra prompten

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Före en utrullning: beredskaps- och reversibilitetsrapporten (ADR 1211): tre fakta — State (stateless | stateful), Rollback (pin-revert | restore-required | none) och Scope (single | coupled) — deklareras per komponent och FAIL-CLOSED när de saknas, med beredskapsdelen som de andra verben bevisar (ref pullable, pins --check, hemlighetsgrinden, färsk källa, deklarerat slutläge) samt en validering av rollback-faktan (rollback --dry-run för pin-revert, retentionsfönstret för restore-required). `deploy` kör den först; ärlighetsgraderad, aldrig en falsk grön. Läser bara; exit 3 när inte ren

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kontrollera din installation
tutor-l1-s1 = Kontrollera konfigurationsfilen
tutor-l2-title = Läs tillstånd: converged, pending, unknown
tutor-l2-text =
    Kommandot goatbuilder `status` jämför två saker för varje maskin: den version som main deklarerar för den, och den version som dess egen post säger att den kör. Det ändrar ingenting.

    Varje maskin är i ett av tre tillstånd:
    - converged (konvergerad): dess post anger den deklarerade versionen. Detta är klart.
    - pending (väntande): dess post anger en äldre version, eller ingen än.
    - unknown (okänd): goatbuilder kan inte läsa dess post.

    Endast converged räknas som klart. Om någon maskin är pending eller unknown avslutar `status` med kod 3.

    Från den här lektionen och framåt pekar `--mock` goatbuilder mot en övningskopia i din handledningsmapp istället för verkliga maskiner. Inget du gör där når en verklig maskin.

    Inget har driftsatts till övningskopians `dev` än. Så förvänta dig fyra maskiner som är pending och en som är unknown. Du kommer att se converged maskiner i lektion 3.
tutor-l2-s1 = Läs tillståndet för dev
tutor-l3-title = Driftsätt: planera det, kör det, bevisa det
tutor-l3-text =
    Att driftsätta lägger den deklarerade versionen på varje maskin, och läser sedan varje maskins post för att bevisa att den kom fram. En driftsättning är klar först när varje maskins post anger den versionen. En driftsättning som kördes utan fel räcker inte.

    Du kommer att driftsätta `portal` till övningskopians `preprod`. `--dry-run` visar vad som skulle hända och ändrar ingenting. `--ack` registrerar vem som gjorde det; kommandot fyller i ditt namn från git. Efter driftsättningen ska `status` visa båda maskinerna som `ok` och converged.
tutor-l3-s1 = Planera driftsättningen
tutor-l3-s2 = Driftsätt portal till preprod
tutor-l3-s3 = Kontrollera att båda maskinerna är converged
tutor-l4-title = Drift och historik
tutor-l4-text =
    Två kommandon hjälper dig att förstå ett goatnet före och efter en ändring. Båda läser bara.
    - Kommandot goatbuilder `diff` listar varje maskin vars post skiljer sig från den deklarerade versionen. En skillnad kallas drift.
    - Kommandot goatbuilder `history` listar varje version en komponent haft på varje maskin, nyaste först.

    På `dev` ligger varje maskin efter. Historiken för `portal` på `preprod` innehåller nu din driftsättning från lektion 3.
tutor-l4-s1 = Lista driften på dev
tutor-l4-s2 = Läs historiken för portal på preprod
tutor-l5-title = Rulla tillbaka, och se en osäker rollback nekas
tutor-l5-text =
    En rollback deklarerar en tidigare version och driftsätter den som varje annan version. Verktyget goatbuilder CLI läser sedan varje maskins post för att bevisa att den kom fram.

    Vissa ändringar kan inte ångras på det sättet. Att driftsätta en äldre version kan inte ångra data som en komponent har skrivit, ett certifikat, eller en ändring som bara går framåt. Verktyget goatbuilder CLI nekar dessa med felkod 3 och säger vad du ska göra istället.

    Övningskopians `db-schema`-komponent innehåller data, så goatbuilder nekar att rulla tillbaka den. Det nekandet är det förväntade resultatet.
tutor-l5-s1 = Planera rollbacken
tutor-l5-s2 = Rulla tillbaka portal en version
tutor-l5-s3 = Försök rulla tillbaka db-schema, vilket goatbuilder nekar
tutor-l6-title = Släpp en ny version: dev, sedan preprod, sedan prod
tutor-l6-s1 = Se var varje komponent är
tutor-l6-s2 = Gör en torrkörning av releasen från dev
tutor-l6-s3 = Gör en torrkörning av prod-steget
tutor-l7-title = Fastlane, och när du inte ska använda det
tutor-l7-text =
    Fastlane lägger en version på varje goatnet på en gång. Det hoppar över väntetiden på preprod och release-posterna.

    Använd det bara för en akut åtgärd som du kan ångra genom att gå tillbaka till den föregående versionen. Använd det aldrig för en databasändring, en identitets- eller åtkomständring, eller något du inte kan ångra på det sättet.

    Fastlane nekar en version som CI inte har byggt, och en komponent som är del av en release som fortfarande pågår.

    Den allmänna formen är `goatbuilder fastlane <component> --ref <version> --reason <why>`. Kör exemplet nedan. Det har `--dry-run`, så ingenting ändras. Det behöver `gh`.
tutor-l7-s1 = Gör en torrkörning av en fastlane
tutor-l8-title = Pins: hitta drift och åtgärda det på rätt sätt
tutor-l8-text =
    En fil bestämmer vilken version varje goatnet kör: `ops/pins/fleet-pins.yaml`. Verktyget goatbuilder CLI genererar de andra versionsfilerna från den: `pins.yml`-filerna och de signerade intent-filerna.

    Kommandot goatbuilder `pins --check` jämför de genererade filerna med `fleet-pins.yaml` och nekar varje fil som skiljer sig. Kommandot goatbuilder `pins` regenererar dem.

    Din handledningsmapp har en kopia av repositoryt där någon redigerade en `pins.yml`-fil för hand. Åtgärda aldrig en handredigering med en annan handredigering. Ändra `fleet-pins.yaml`, och regenerera sedan.
tutor-l8-s1 = Kontrollera den här utcheckningen
tutor-l8-s2 = Kontrollera kopian och hitta handredigeringen
tutor-l8-s3 = Regenerera kopians filer
tutor-l8-s4 = Kontrollera kopian igen
tutor-l9-title = Förklara ett fel
tutor-l9-s2 = Förklara felet med dess kod
tutor-l9-s3 = Förklara felet med dess text
tutor-l10-title = Handledning 1: bygg ett nytt goatnets första site
tutor-l10-text =
    Den här lektionen bygger ett helt goatnet i en övningskopia. Exempelgoatnetet är `zagros`. Dess första site, `alpha`, har två kärnmaskiner, en övervakningsmaskin och en räddningsmaskin.

    Kommandot goatbuilder `standup` sätter upp varje maskin i en fast ordning och kontrollerar varje post. Dess sista rad säger att siten är klar. På verkliga maskiner visar den raden också hur lång tid det tog.

    Hela handledningen är `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Bygg den första siten, alpha
tutor-l10-s2 = Kontrollera resultatet
tutor-l11-title = Handledning 2: lägg till en andra site
tutor-l11-text =
    Den här lektionen börjar där handledning 1 slutade. Den lägger till en andra site, `beta`, i en ny övningskopia, `zagros-2`.

    En andra site kopierar sina användarkonton från den första siten. Dess standup ändrar också den första siten en gång, så att kopieringen fungerar i båda riktningarna. Ett steg som heter `alpha/core` gör detta.

    Hela handledningen är `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Bygg den andra siten, beta
tutor-l11-s2 = Kontrollera båda siterna
tutor-l12-s1 = Planera att lägga till en maskin
tutor-l13-s1 = Planera en federation
tutor-l14-title = Håll goatbuilder uppdaterad
tutor-l14-s1 = Fråga om en nyare goatbuilder finns
tutor-l15-title = Assay: kontrollera hur ett goatnet byggdes
tutor-l15-text =
    Kommandot goatbuilder `assay` kontrollerar att ett goatnet byggdes korrekt, ett lager i taget, med hjälp av acceptanstesterna. Varje test får ett av tre resultat:
    - PASS: testet gick igenom.
    - FAIL: goatnetet har ett problem.
    - UNKNOWN: ingenting kunde besvara testet.

    Endast PASS räknas. Varje FAIL eller UNKNOWN ger felkod 3.

    `--plan` listar testerna som gäller för ett goatnet, per lager, och rör ingen maskin. Använd det för att se vad ett verkligt assay skulle täcka. Den allmänna formen är `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: är versionerna konsekventa?
tutor-l16-text =
    Kommandot goatbuilder `survey` kontrollerar i en genomgång om samma versioner körs där de ska:
    - Inom varje goatnet: kör varje maskin den deklarerade versionen, och matchar maskiner som ska matcha varandra?
    - Mellan dev, preprod och prod: en rad per komponent, en kolumn per goatnet.

    En skillnad som releaseprocessen förväntar sig rapporteras men är inget fel. Ett exempel är en ny version som finns på dev men ännu inte på prod. En skillnad som ingenting deklarerar läses som `difference not declared`. En maskin goatbuilder inte kan läsa är unknown, aldrig konsekvent.

    Kör det på övningskopian. Dess `dev` ligger efter med avsikt, så förvänta dig fynd och felkod 3.
tutor-l16-s1 = Survey övningskopian
tutor-l17-title = Endstate: kör ett goatnet exakt det som det deklarerar?
tutor-l17-text =
    Kommandot goatbuilder `endstate` kontrollerar två saker om ett goatnet:
    1. Varje tjänst det deklarerar är driftsatt, körs, är frisk och korrekt ansluten.
    2. Ingenting körs som det inte deklarerar. En odeklarerad tjänst kallas en föräldralös tjänst.

    Verktyget goatbuilder CLI räknar ut vad en site ska köra från sitens egna inställningar, inte från en separat lista.

    En kontroll som inte har något att läsa märks UNOBSERVED. Det är ett ärligt svar, inte ett misslyckande. En FAIL eller en föräldralös tjänst ger felkod 3.

    Den allmänna formen är `goatbuilder endstate <goatnet>`. Kommandot nedan kontrollerar en exempel-site från sparad data, så det rör ingen maskin.
tutor-l17-s1 = Kontrollera exempelsitet
tutor-l18-title = Bonus: var nyfiken
tutor-l18-text =
    Den här lektionen är valfri. Kör kommandot nedan och handledningen är klar.

    Verktyget goatbuilder CLI har några små dolda funktioner. De finns inte i hjälpen eller i dessa lektioner. Några svarar på ord som erfarna Unix-användare ofta skriver in i ett nytt program, bara för att se vad som händer. Det mesta du testar gör ingenting.

    Den här lektionen säger inte vad de är. Att hitta dem är poängen.

    Kommandot nedan listar varje fel som goatbuilder kan förklara. Det är en bra plats att börja utforska.
tutor-l18-s1 = Lista varje fel som goatbuilder kan förklara
tutor-heading = Lektion { $n } av { $total } · { $title }
tutor-setup = Kör först detta en gång i den här terminalen:
tutor-not-recording = Den här terminalen registrerar inte dina körningar, så handledningen kan inte bocka av dina steg. Kör detta en gång, och kör sedan stegen:
tutor-type = Gör dessa steg i din terminal ({ $met } av { $total } klara):
tutor-rerun = Kör `goatbuilder tutor` igen efter varje steg. Den kontrollerar vad du gjorde och bockar av steget.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Välkommen till GOATBUILDER Tutor
tutor-bar-progress = Framsteg
tutor-bar-done = { $done } av { $total } lektioner klara
tutor-after-step = Steg { $met } av { $total } klart. Nästa:
tutor-after-lesson = Lektion { $n } är klar. Kör `goatbuilder tutor` för att starta lektion { $next }. För att läsa den igen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lektion { $n } är klar, och det är handledningen också. Kör `goatbuilder tutor` för att se den markerad som klar. För att läsa den igen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Du har slutfört den här lektionen, så den här sidan är bara för läsning. Kör `goatbuilder tutor` för att komma tillbaka dit du är.
tutor-other-lessons = För att läsa en lektion igen: `goatbuilder tutor --lesson 1`. För att se alla: `goatbuilder tutor --list`.
tutor-l12-title = Lägg till en maskin på en site som körs
tutor-l12-text =
    Kommandot goatbuilder `join-node` lägger till en maskin till på en site som redan körs. Den skriver ut en plan först, med en rad per fas. Varje fas har en typ:
    - tool: goatbuilder kör ett skript.
    - ansible: goatbuilder kör en playbook.
    - gate: en kontroll som måste godkännas innan nästa fas.
    - manual: du gör detta själv, och körningen pausar.

    Kommandot nedan har `--dry-run`. Det läser den här utcheckningen och ändrar ingenting. Under tabellen kan du läsa vad du gör för hand.
tutor-l13-title = Koppla till en annan organisations goatnet
tutor-l13-text =
    Kommandot goatbuilder `federate` kopplar ditt goatnet till ett goatnet som en annan organisation kör. Varje sida bygger sin halva av en överenskommelse. Halvorna byts, och varje sida kombinerar dem till samma överenskommelse.

    Bekräfta den andra sidans nyckel på ett separat sätt, till exempel ett samtal du litar på. En nyckel som kommer inuti deras paket bevisar ingenting.

    Kommandot nedan har `--dry-run`, så det ändrar ingenting. En verklig körning stoppar vid fas 4, eftersom du gör den fasen för hand.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Det här kommandot är inte en del av lektion { $n }. Kör `goatbuilder tutor` för att se var du är.
tutor-l1-s2 = Fråga doctor om ett goatnet
tutor-l1-text =
    goatbuilder-CLI:t bygger, driftsätter och släpper GOAT-programvara. Innan du använder det, kontrollera din installation med två kommandon:
    - Kommandot goatbuilder `validate` kontrollerar konfigurationsfilen, `ops/goatbuilder/goatbuilder.yaml`. Det läser bara den här utcheckningen och behöver inget nätverk.
    - Kommandot goatbuilder `doctor` kontrollerar vad en riktig driftsättning behöver: dina verktyg, dina inloggningar, om den här utcheckningen stämmer med main och hur goatbuilder når varje goatnet. Ett goatnet är en GOAT-miljö.

    Kommandot goatbuilder `doctor` skriver ut hur många kontroller som gick igenom, och sedan varje problem med dess åtgärd. I den här lektionen är ett problem information, inte ett misstag. Läs åtgärderna, men du behöver inte tillämpa dem nu.

    De flesta lektioner behöver ingen inloggning. Lektion 2 till 5, 10, 11 och 16 använder en övningskopia. Lektion 8, 9, 12, 13, 15, 17 och 18 läser bara den här utcheckningen. Lektion 6 behöver `gh` och meshen, lektion 7 behöver `gh`, och lektion 14 behöver meshen. Ingen lektion behöver 1Password.
tutor-l14-text =
    goatbuilder-CLI:t kan uppdatera sig självt från kopian som ditt goatnet serverar på `get.<site>.<zone>/goatbuilder/`.

    Kommandot goatbuilder `update --check` läser bara. Det jämför din version med den serverade versionen och ändrar ingenting. Slutkod 0 betyder att du är uppdaterad. Slutkod 3 betyder att det finns en nyare version.

    En riktig uppdatering, utan `--check`, verifierar nedladdningens kontrollsumma och signatur innan den ersätter något. Sedan byter den ut filen i ett steg.

    goatbuilder-CLI:t behöver veta vilket goatnet det ska fråga: ange `--site` med det goatnetet, eller sätt `GOATBUILDER_UPDATE_URL`. Du måste vara på det goatnetets mesh. Om du inte är det säger `--check` att det inte kan nå servern och anger adressen det försökte med. Det är ett korrekt resultat för den här lektionen.
tutor-l15-s1 = Lista testerna som gäller för ett goatnet
tutor-l6-text =
    En release flyttar en ny version genom tre goatnet i ordning: dev, sedan preprod, sedan prod.

    1. Någon taggar versionen på main, och CI bygger den.
    2. dev: goatbuilder driftsätter den och kör acceptanstesterna.
    3. preprod: goatbuilder driftsätter den igen, kör testerna igen och registrerar resultaten.
    4. Versionen väntar sedan på preprod under en bestämd tid. Ingenting flyttas till prod av sig självt.
    5. prod: efter den väntan kör en person releasen med `--from prod`. goatbuilder-CLI:t driftsätter till prod och kontrollerar alla tre goatnet.

    Kommandot goatbuilder `promote status` visar var varje komponent är. En kandidat är en version som testas. Den allmänna formen för en release är `goatbuilder release <component> --tag <tag> --from dev`. Kommandona nedan använder en komponent i den här utcheckningen och en exempeltagg. Vart och ett har `--dry-run`, så ingenting taggas eller driftsätts.

    Torrkörningarna behöver `gh` och meshen. Om du inte är admin för repot, sätt `GOAT_RC_ROLE_CHECK=0` före varje `release`-kommando.
tutor-l9-s1 = Skapa ett riktigt fel med ett goatnet som inte finns
tutor-l9-text =
    När goatbuilder vägrar skriver det ut en kod. Koder som börjar med `GB-` kommer från goatbuilder, `E_` från goat-CLI:t och `X-` från ett annat verktyg som goatbuilder körde.

    Kommandot goatbuilder `explain` tar en kod, eller texten i en felrad, och berättar vad som hände, varför, hur du åtgärdar det och var du kan läsa mer.

    Att be om ett goatnet som den här utcheckningen inte deklarerar ger ett riktigt fel att förklara.
