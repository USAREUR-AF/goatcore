### goatbuilder — nl (nl)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Bouw en lever een goat-omgeving: declareren, convergeren, verifiëren — onder één deploy-contract
help-flag-output = Uitvoerformaat: een tabel voor mensen; JSON of YAML voor een programma
help-flag-repo = De repository-root. Standaard de checkout waarin deze binary draait
help-flag-yes = Bevestig een declaratief commando zonder prompt (voor automatisering)
help-flag-frozen = Weiger elke run die de checkout zou moeten fast-forwarden (strikte C3)
help-flag-mock = Draai tegen een fixture-map in plaats van hosts: geen registry, geen transport, deterministisch (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Taal voor help en foutuitleg (een tag zoals fr, uk, de); uitvoer die een pipeline leest blijft Engels. Standaard: de OS-locale
help-cmd-validate = Controleer een config tegen het geversioneerde schema en de cross-field-regels, offline
help-cmd-doctor = Test wat een run nodig heeft voordat er iets wordt aangeraakt: deps, versheid van de checkout, het transport van elke omgeving, en of de playbook van elke component naar een eenduidige doelmodus resolvet
help-cmd-status = Gedeclareerd tegenover vastgelegd, per machine: converged, pending of unknown (C7, C8). Leest alleen. Exit 3 zolang een machine niet converged is
help-cmd-deploy = Zet de pin die main declareert op één omgeving en bewijs het: resolve de component en de groepen van de omgeving uit de pins, de play en zijn doelmodus, elk secret dat de play templatet, het transport; draai de play via ansible-preflight; lees het deploy-ledger terug van elke host en beschouw het pas als klaar wanneer de nieuwste rij de gedeclareerde digest noemt met action=deploy (C7). Op een fixture (`--mock`) neemt de gescripte converger de plaats van de hosts in
help-cmd-release = Breng een release candidate van een tag op main via de live validatie van het dev-goatnet naar de preprod-trede (ADR 1173); laat na de soak, met `--from prod`, hem graduëren, voeg hem samen in fleet:, deploy prod en verifieer elke trede opnieuw. Vervangt ops/scripts/rc.sh stap voor stap
help-cmd-rollback = Draai een gedeployde component terug naar zijn N-de meest recente ref, per artefacttype, met het ledger op de host als bron van waarheid (ADR 1060). Vervangt ops/scripts/rollback.sh: exit 3 geweigerd als onveilig (data, cert, forward-only, reversible:false), 4 begeleid (config, acl)
help-cmd-fastlane = Eén commando van een gemergede commit naar elk goatnet (ADR 1127): de bewuste uitzondering op de ladder. Vervangt ops/scripts/fastlane.sh stap voor stap; weigert een niet-gepinde digest of een open graduatieset, en draait de deploys via deze binary
help-cmd-standup = Een site van niets tot geverifieerd: de fases van de standup in de volgorde van de doctrine met een fail-closed gate aan elk uiteinde, de observability-host als parallelle tak, de rescue-klassen en de cross-goatnet-oppervlakken als laatste (ADR 1183). Vervangt ops/scripts/goat-standup.sh stap voor stap. Op een fixture (`--mock`) wordt elke component van elke klasse geconvergeerd door de gescripte converger en is de tijd gedeclareerd → geverifieerd de maat van het één-uur-goatnet
help-cmd-join-node = Nog een machine van een klasse op een live site, gedeclareerd → converged → geverifieerd: de uitgevoerde reeks van docs/tutorials/02-first-federated-node.md als genummerde fases met zijn gates (bereik, sleutel, eerst repo-werk, baseline, kernel-WireGuard voor en na enrollment, het kernel-ACL-vlak als enrollment-gate, de converge van de klasse, acceptatie)
help-cmd-federate = Een soevereine peering gedeclareerd, geconvergeerd en geverifieerd (ADR 1157): onze helft gemeten, de pakketten out-of-band uitgewisseld, één agreement opgesteld en geverifieerd, de leaves daaruit gemint, de regels gerenderd door de ene writer, de edge gedeployd, het transport dat het agreement koos geopend, acceptatie in alle vier de tiers. Vervangt ops/scripts/peering-standup.sh stap voor stap
help-cmd-pins = ÉÉN resolutiepass met TWEE renderingen: de group_vars pins.yml van elk genoemd lid en het IntentV1-artefact van elk genoemd goatnet, uit ops/pins/fleet-pins.yaml. Vervangt ops/pins/generate-pins.py byte voor byte. `--check` valideert, regenereert en weigert drift, een niet-ondertekende of gemanipuleerde intent, en een gereguleerde pin die buiten pins.yml is gedefinieerd (exit 1)
help-cmd-promote = Begeleid en bekijk release-graduatie over de goatnet-ladder (ADR 1120, 1125): status, een candidate openen, graduëren, een set graduëren, het hotfix-pad, review, readiness, abandon, check. Vervangt ops/release/graduate.py stap voor stap. Niets hier graduëert uit zichzelf: een mutatie draait alleen als hij wordt aangeroepen, meldt wat hij deed, en vraagt om --yes zonder terminal (C10, ADR 1177)
help-cmd-explain = Duid een fout: een code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), of willekeurige fouttekst — van deze binary zelf, of woorden van een tool die hij doorgaf (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Toont wat er gebeurde, waarom, wat te doen en waar meer te lezen; `-` leest de tekst van stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Een site van geverifieerd naar niets — de spiegel van standup, administratie blijft behouden (een burn-down voegt teardown-rijen toe, wist nooit iets). Op een fixture (`--mock`) oefent hij de burn-down in omgekeerde fasevolgorde; op echte hosts toont hij de volgorde en weigert hij uit te voeren tot elke fase zijn teardown-play declareert (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lessen in vimtutor-stijl: elke les noemt het exacte commando dat je draait op een wegwerpkopie van een fixture (of, alleen-lezen, op deze checkout) en controleert daarna wat dat commando echt deed — exitcode en rijen — voordat de volgende opengaat. `tutor` toont de les waar je op zit en markeert die als klaar zodra je runs eraan voldoen; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Verborgen (src/fun.rs)
help-cmd-argan = Verborgen (src/fun.rs)
help-cmd-bound = Verborgen (crates/goat-fun)
help-cmd-secrets = De gedeclareerde secrets-houding (ADR 1204): welke store geldt (1password, openbao, sops, git-crypt, env-file, env), wat beschikbaar is, hoe je aanlevert wat ontbreekt, en de eigen controles van de store. `init --env <goatnet>` schrijft het env-bestand voor de zero-SaaS-quickstart: een sterke willekeurige waarde voor elke variabele die de plays van de goatnet templaten, 0600, gitignored, een bestaande waarde wordt nooit geroteerd. Zelfde module als ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Leest voor één component de nieuwste deploy-ledgerrij van elke host en meldt of die de ref noemt die main declareert (C7). Alleen lezen; exit 3 zolang een host pending of unknown is. Dezelfde controle als `deploy --verify-only`
help-cmd-diff = Wat nu afwijkt van main (C8): de gedeclareerde ref van main tegenover de nieuwste ledgerrij van elke host, voor één component of, zonder component, voor elk component van de omgeving, in één pass per ledengroep. Alleen lezen; exit 3 bij elke afwijking of onleesbare host
help-cmd-history = De deploy-ledger als tijdlijn, nieuwste eerst, op elke host van de omgeving (C8): elke rij is een item met de machine, de tijd, de actie (deploy of rollback) en de versie. De vorige versie van het item en de operator die het uitvoerde staan in `-o json`. Alleen lezen
help-cmd-plan = Schrijft naar een planbestand ter review wat een declaratieve verb zou doen: de gedeclareerde refs, de play en het doel, de ledger, de commit van main, het transport (deploy); het artefacttype en de doelref (rollback). Raakt de status van geen enkele host aan. Voer het gereviewde plan uit met `apply`
help-cmd-apply = Voert exact een gereviewd plan uit. Weigert (exit 1, met vermelding van wat is verschoven) als de declaratie van main, de pins-commit, de play of het rollback-doel is verschoven sinds het plan is geschreven; een gewijzigd transport wordt gemeld, niet geweigerd
help-cmd-adopt = Declareert wat een omgeving al draait: leest de ledgers van elke host en stelt een pins-afwijking voor bij elk component waarvan de draaiende ref afwijkt van de declaratie van main. Standaard alleen een rapport; met --yes worden de wijzigingen ter review in een verse worktree van main geschreven. Deployt nooit

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Nog niet: { $why }
tutor-lesson-done = Les { $n } gedaan: { $title }.
tutor-all-done-reset = Alle lessen zijn gedaan. Met `goatbuilder tutor --reset` begin je opnieuw; de tutorials gaan verder: docs/tutorials/.
tutor-all-done = Alle lessen zijn gedaan. De tutorials gaan verder: docs/tutorials/.
tutor-workspace = Werkruimte: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Controleer laag voor laag of een goatnet correct is gebouwd: alle acceptatiecontroles die voor de site gelden, inclusief de controles die een operator met de goat CLI uitvoert. De acceptatiefase van de standup als eigen commando. Leest alleen; exit 3 zolang een controle faalt of geen antwoord kan geven
help-flag-offline = Geïsoleerde site: haal de trunk niet op; beoordeel de checkout tegen de gecachete trunk en registreer de run als offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Controleer of een goatnet in zijn gedeclareerde eindtoestand is: elke service die volgens de eigen declaraties draait is uitgerold, actief, gezond en doet zijn werk, met de zero-trust-verbindingen aangelegd, en er draait niets wat niet gedeclareerd is. Alleen lezen; exit 3 zolang er iets niet klopt
help-cmd-survey = Hoe consistent de software is binnen elk goatnet en over de goatnets van de ladder heen, gelezen in één parallelle doorgang. Leest alleen

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Hoe consistent de software is binnen elk goatnet en tussen de goatnets van de ladder, uitgelezen in één parallelle pass. Alleen lezen.
help-cmd-update = Vervangt dit binaire bestand door de release die het goatnet aanbiedt op `get.<site>.<zone>/goatbuilder/`: de download wordt gecontroleerd tegen de bijbehorende `.sha256` en de ondertekende `SHA256SUMS` voordat er iets wordt aangeraakt, en de wissel is atomair. Al actueel: één rij, exit 0. `--check` leest alleen (exit 3 als achterstallig). De bron: `GOATBUILDER_UPDATE_URL`, anders `--site` uit de group_vars, anders die van de laatste update

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Voordat je begint: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Kleur de ok/FAIL-glyph en het oordeel van de tabel (auto|always|never). `auto` kleurt alleen in een terminal; NO_COLOR en CLICOLOR_FORCE gaan voor. JSON en YAML worden nooit gekleurd
help-flag-verbose = Log wat een run doet — de externe commando's die hij aanroept en de netwerkfase van `update` — naar stderr, zodat `-o json|yaml` een schoon document blijft. Of zet RUST_LOG=goatbuilder=debug
help-flag-timeout = Aantal seconden om de connect/bereikfase te begrenzen voordat er snel wordt gefaald — de enige fase die kan blokkeren op een onbereikbare host (het ophalen van de aangeboden versie door `update`). Begrenst NIET de eigen run van een play of de binary-download (stall-begrensd). Leest ook GOATBUILDER_TIMEOUT
help-cmd-completion = Print een shell-completionscript naar stdout (`goatbuilder completion zsh >> ~/.zshrc`). Voor live kandidaten die een statisch script niet kan kennen — de sites, componenten en goatnets die deze checkout declareert — koppel je een hook aan `goatbuilder _complete` (zie de header van het gegenereerde script)
help-cmd-docs = De commandoreferentie: `goatbuilder docs` verwijst ernaar; `--markdown` geeft het geheel weer, doorlopen vanuit de live commandoboom, zodat het nooit kan afwijken van `--help`
help-cmd-_complete = Verborgen: geeft live completionkandidaten voor shell-hooks, één per regel (de sites, componenten en goatnets die deze checkout declareert). Leest alleen, faalt nooit luid — een completion-hook mag de prompt niet verstoren

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Vóór een rollout: de gereedheids- en omkeerbaarheidsrapportage (ADR 1211): drie feiten — State (stateless | stateful), Rollback (pin-revert | restore-required | none) en Scope (single | coupled) — per component gedeclareerd en FAIL-CLOSED als ze ontbreken, met de gereedheidshelft die de andere verbs bewijzen (ref pullable, pins --check, de secrets-gate, verse bron, de gedeclareerde eindtoestand) en een validatie van het rollback-feit (rollback --dry-run voor pin-revert, het retentievenster voor restore-required). `deploy` draait dit eerst; eerlijkheidsgetoetst, nooit een vals groen. Leest alleen; exit 3 als niet schoon

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Controleer je installatie
tutor-l1-s1 = Controleer het configuratiebestand
tutor-l2-title = Status lezen: converged, pending, unknown
tutor-l2-text =
    Het goatbuilder-commando `status` vergelijkt twee dingen voor elke machine: de versie die main ervoor declareert, en de versie die het eigen ledger zegt dat draait. Het verandert niets.

    Elke machine staat in een van drie toestanden:
    - converged (geconvergeerd): het ledger noemt de gedeclareerde versie. Dit is klaar.
    - pending (wachtend): het ledger noemt een oudere versie, of nog niets.
    - unknown (onbekend): goatbuilder kan het ledger niet lezen.

    Alleen converged telt als klaar. Als een machine pending of unknown is, sluit `status` af met code 3.

    Vanaf deze les wijst `--mock` goatbuilder naar een oefenkopie in je tutor-map in plaats van naar echte machines. Niets wat je daar doet, bereikt een echte machine.

    Er is nog niets gedeployd naar de `dev` van de oefenkopie. Verwacht dus vier machines die pending zijn en een die unknown is. Je ziet converged machines in les 3.
tutor-l2-s1 = Lees de status van dev
tutor-l3-title = Deploy: plannen, uitvoeren, bewijzen
tutor-l3-text =
    Deployen zet de gedeclareerde versie op elke machine, en leest dan het ledger van elke machine om te bewijzen dat die is aangekomen. Een deploy is pas klaar als het ledger van elke machine die versie noemt. Een deploy die zonder fouten draaide, is niet genoeg.

    Je deployt `portal` naar de `preprod` van de oefenkopie. `--dry-run` toont wat er zou gebeuren en verandert niets. `--ack` legt vast wie het deed; het commando vult je naam aan vanuit git. Na de deploy moet `status` beide machines tonen als `ok` en converged.
tutor-l3-s1 = Plan de deploy
tutor-l3-s2 = Deploy portal naar preprod
tutor-l3-s3 = Controleer dat beide machines converged zijn
tutor-l4-title = Drift en geschiedenis
tutor-l4-text =
    Twee commando's helpen je een goatnet te begrijpen voor en na een wijziging. Beide lezen alleen.
    - Het goatbuilder-commando `diff` toont elke machine waarvan het ledger afwijkt van de gedeclareerde versie. Zo'n verschil heet drift.
    - Het goatbuilder-commando `history` toont elke versie die een component had op elke machine, nieuwste eerst.

    Op `dev` loopt elke machine achter. De geschiedenis van `portal` op `preprod` bevat nu je deploy uit les 3.
tutor-l4-s1 = Toon de drift op dev
tutor-l4-s2 = Lees de geschiedenis van portal op preprod
tutor-l5-title = Terugdraaien, en een onveilige rollback zien weigeren
tutor-l5-text =
    Een rollback declareert een eerdere versie en deployt die zoals elke andere versie. De goatbuilder-CLI leest dan het ledger van elke machine om te bewijzen dat die is aangekomen.

    Sommige wijzigingen kun je zo niet ongedaan maken. Het deployen van een oudere versie kan data die een component heeft geschreven niet ongedaan maken, net zomin als een certificaat, of een wijziging die alleen voorwaarts gaat. De goatbuilder-CLI weigert die met exitcode 3 en zegt wat je in plaats daarvan moet doen.

    Het `db-schema`-component van de oefenkopie bevat data, dus goatbuilder weigert om het terug te draaien. Die weigering is het verwachte resultaat.
tutor-l5-s1 = Plan de rollback
tutor-l5-s2 = Draai portal een versie terug
tutor-l5-s3 = Probeer db-schema terug te draaien, wat goatbuilder weigert
tutor-l6-title = Een nieuwe versie releasen: dev, dan preprod, dan prod
tutor-l6-s1 = Bekijk waar elk component staat
tutor-l6-s2 = Doe een dry run van de release vanaf dev
tutor-l6-s3 = Doe een dry run van de prod-stap
tutor-l7-title = Fastlane, en wanneer je het niet moet gebruiken
tutor-l7-text =
    Fastlane zet één versie meteen op elk goatnet. Het slaat de wachttijd op preprod en de release-ledgers over.

    Gebruik het alleen voor een dringende fix die je ongedaan kunt maken door terug te gaan naar de vorige versie. Gebruik het nooit voor een databasewijziging, een identiteits- of toegangswijziging, of iets wat je zo niet ongedaan kunt maken.

    Fastlane weigert een versie die CI niet heeft gebouwd, en een component dat deel is van een release die nog loopt.

    De algemene vorm is `goatbuilder fastlane <component> --ref <version> --reason <why>`. Voer het voorbeeld hieronder uit. Het heeft `--dry-run`, dus er verandert niets. Het heeft `gh` nodig.
tutor-l7-s1 = Doe een dry run van een fastlane
tutor-l8-title = Pins: drift vinden en op de juiste manier oplossen
tutor-l8-text =
    Eén bestand bepaalt welke versie elk goatnet draait: `ops/pins/fleet-pins.yaml`. De goatbuilder-CLI genereert de andere versiebestanden daaruit: de `pins.yml`-bestanden en de ondertekende intent-bestanden.

    Het goatbuilder-commando `pins --check` vergelijkt de gegenereerde bestanden met `fleet-pins.yaml` en weigert elk bestand dat afwijkt. Het goatbuilder-commando `pins` regenereert ze.

    Je tutor-map heeft een kopie van de repository waarin iemand één `pins.yml`-bestand met de hand heeft bewerkt. Los een handmatige wijziging nooit op met een andere handmatige wijziging. Wijzig `fleet-pins.yaml`, en regenereer dan.
tutor-l8-s1 = Controleer deze checkout
tutor-l8-s2 = Controleer de kopie en vind de handmatige wijziging
tutor-l8-s3 = Regenereer de bestanden van de kopie
tutor-l8-s4 = Controleer de kopie opnieuw
tutor-l9-title = Een fout uitleggen
tutor-l9-s2 = Leg de fout uit met de code
tutor-l9-s3 = Leg de fout uit met de tekst
tutor-l10-title = Tutorial 1: de eerste site van een nieuw goatnet bouwen
tutor-l10-text =
    Deze les bouwt een heel goatnet in een oefenkopie. Het voorbeeldgoatnet is `zagros`. Zijn eerste site, `alpha`, heeft twee kernmachines, een monitoringmachine en een reddingsmachine.

    Het goatbuilder-commando `standup` zet elke machine op in een vaste volgorde en controleert elk ledger. De laatste regel zegt dat de site klaar is. Op echte machines toont die regel ook hoe lang het duurde.

    De volledige tutorial is `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Bouw de eerste site, alpha
tutor-l10-s2 = Controleer het resultaat
tutor-l11-title = Tutorial 2: een tweede site toevoegen
tutor-l11-text =
    Deze les begint waar tutorial 1 ophield. Hij voegt een tweede site toe, `beta`, in een nieuwe oefenkopie, `zagros-2`.

    Een tweede site kopieert zijn gebruikersaccounts van de eerste site. Zijn standup wijzigt ook eenmalig de eerste site, zodat kopiëren in beide richtingen werkt. Een stap met de naam `alpha/core` doet dit.

    De volledige tutorial is `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Bouw de tweede site, beta
tutor-l11-s2 = Controleer beide sites
tutor-l12-s1 = Plan het toevoegen van een machine
tutor-l13-s1 = Plan een federatie
tutor-l14-title = Houd goatbuilder actueel
tutor-l14-s1 = Vraag of er een nieuwere goatbuilder bestaat
tutor-l15-title = Assay: controleren hoe een goatnet is gebouwd
tutor-l15-text =
    Het goatbuilder-commando `assay` controleert of een goatnet correct is gebouwd, laag voor laag, met de acceptatiecontroles. Elke test krijgt een van drie resultaten:
    - PASS: de test is geslaagd.
    - FAIL: het goatnet heeft een probleem.
    - UNKNOWN: niets kon de test beantwoorden.

    Alleen PASS telt. Elke FAIL of UNKNOWN maakt de exitcode 3.

    `--plan` toont de tests die op een goatnet van toepassing zijn, per laag, en raakt geen machine aan. Gebruik het om te zien wat een echte assay zou behandelen. De algemene vorm is `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: zijn de versies consistent?
tutor-l16-text =
    Het goatbuilder-commando `survey` controleert in één keer of dezelfde versies draaien waar ze moeten:
    - Binnen elk goatnet: draait elke machine de gedeclareerde versie, en komen machines die zouden moeten overeenkomen ook echt overeen?
    - Tussen dev, preprod en prod: een rij per component, een kolom per goatnet.

    Een verschil dat het releaseproces verwacht, wordt gemeld maar is geen fout. Een voorbeeld is een nieuwe versie die op dev staat maar nog niet op prod. Een verschil dat niets declareert, leest als `difference not declared`. Een machine die goatbuilder niet kan lezen is unknown, nooit consistent.

    Voer het uit op de oefenkopie. De `dev` daarvan loopt met opzet achter, verwacht dus bevindingen en exitcode 3.
tutor-l16-s1 = Voer survey uit op de oefenkopie
tutor-l17-title = Endstate: draait een goatnet precies wat het declareert?
tutor-l17-text =
    Het goatbuilder-commando `endstate` controleert twee dingen over een goatnet:
    1. Elke dienst die het declareert, is gedeployd, draait, is gezond en correct verbonden.
    2. Er draait niets wat het niet declareert. Een niet-gedeclareerde dienst heet een orphan.

    De goatbuilder-CLI leidt af wat een site moet draaien uit de eigen instellingen van de site, niet uit een losse lijst.

    Een controle die niets heeft om te lezen, wordt gemarkeerd als UNOBSERVED. Dat is een eerlijk antwoord, geen mislukking. Een FAIL of een orphan maakt de exitcode 3.

    De algemene vorm is `goatbuilder endstate <goatnet>`. Het commando hieronder controleert een voorbeeldsite vanuit opgeslagen data, dus het raakt geen machine aan.
tutor-l17-s1 = Controleer de voorbeeldsite
tutor-l18-title = Bonus: wees nieuwsgierig
tutor-l18-text =
    Deze les is optioneel. Voer het commando hieronder uit en de tutor is klaar.

    De goatbuilder-CLI heeft een paar kleine verborgen functies. Ze staan niet in de help of in deze lessen. Sommige reageren op woorden die langetermijngebruikers van Unix vaak in een nieuw programma typen, gewoon om te zien wat er gebeurt. De meeste dingen die je probeert, doen niets.

    Deze les zegt niet wat ze zijn. Ze vinden is het hele punt.

    Het commando hieronder toont elke fout die goatbuilder kan uitleggen. Het is een goede plek om te beginnen met verkennen.
tutor-l18-s1 = Toon elke fout die goatbuilder kan uitleggen
tutor-heading = Les { $n } van { $total } · { $title }
tutor-setup = Voer dit eerst eenmaal uit in deze terminal:
tutor-not-recording = Deze terminal legt je runs niet vast, dus de tutor kan je stappen niet afvinken. Voer dit eenmaal uit, en voer dan de stappen uit:
tutor-type = Doe deze stappen in je terminal ({ $met } van { $total } klaar):
tutor-rerun = Voer na elke stap `goatbuilder tutor` opnieuw uit. Het controleert wat je deed en vinkt de stap af.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Welkom bij GOATBUILDER Tutor
tutor-bar-progress = Voortgang
tutor-bar-done = { $done } van { $total } lessen klaar
tutor-after-step = Stap { $met } van { $total } klaar. Volgende:
tutor-after-lesson = Les { $n } is klaar. Voer `goatbuilder tutor` uit om les { $next } te starten. Om deze opnieuw te lezen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Les { $n } is klaar, en de tutor ook. Voer `goatbuilder tutor` uit om het als klaar gemarkeerd te zien. Om deze opnieuw te lezen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Je hebt deze les afgerond, dus deze pagina is alleen om te lezen. Voer `goatbuilder tutor` uit om terug te gaan naar waar je bent.
tutor-other-lessons = Om een les opnieuw te lezen: `goatbuilder tutor --lesson 1`. Om ze allemaal te zien: `goatbuilder tutor --list`.
tutor-l12-title = Een machine toevoegen aan een site die al draait
tutor-l12-text =
    Het goatbuilder-commando `join-node` voegt nog een machine toe aan een site die al draait. Het toont eerst een plan, met één rij per fase. Elke fase heeft een type:
    - tool: goatbuilder voert een script uit.
    - ansible: goatbuilder voert een playbook uit.
    - gate: een controle die moet slagen voordat de volgende fase begint.
    - manual: je doet dit zelf, en de run pauzeert.

    Het commando hieronder heeft `--dry-run`. Het leest deze checkout en verandert niets. Onder de tabel lees je wat je handmatig doet.
tutor-l13-title = Verbinden met het goatnet van een andere organisatie
tutor-l13-text =
    Het goatbuilder-commando `federate` verbindt je goatnet met een goatnet dat een andere organisatie draait. Elke kant bouwt zijn helft van een overeenkomst. Jullie wisselen de helften uit, en elke kant combineert ze tot dezelfde overeenkomst.

    Bevestig de sleutel van de andere kant op een andere manier, zoals een telefoontje dat je vertrouwt. Een sleutel die binnen hun pakket aankomt, bewijst niets.

    Het commando hieronder heeft `--dry-run`, dus het verandert niets. Een echte run stopt bij fase 4, omdat jij die fase handmatig doet.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Dit commando is geen onderdeel van les { $n }. Voer `goatbuilder tutor` uit om te zien waar je bent.
tutor-l1-s2 = Vraag doctor naar één goatnet
tutor-l1-text =
    De goatbuilder-CLI bouwt, deployt en publiceert GOAT-software. Controleer je setup met twee commando's voordat je hem gebruikt:
    - Het goatbuilder-commando `validate` controleert het configuratiebestand, `ops/goatbuilder/goatbuilder.yaml`. Het leest alleen deze checkout en heeft geen netwerk nodig.
    - Het goatbuilder-commando `doctor` controleert wat een echte deploy nodig heeft: je tools, je aanmeldingen, of deze checkout overeenkomt met main, en hoe goatbuilder elk goatnet bereikt. Een goatnet is één GOAT-omgeving.

    Het goatbuilder-commando `doctor` toont hoeveel controles geslaagd zijn, en daarna elk probleem met de oplossing. In deze les is een probleem informatie, geen fout. Lees de oplossingen, maar je hoeft ze nu niet toe te passen.

    De meeste lessen hebben geen aanmelding nodig. Lessen 2 tot en met 5, 10, 11 en 16 gebruiken een oefenkopie. Lessen 8, 9, 12, 13, 15, 17 en 18 lezen alleen deze checkout. Les 6 heeft `gh` en de mesh nodig, les 7 heeft `gh` nodig, en les 14 heeft de mesh nodig. Geen enkele les heeft 1Password nodig.
tutor-l14-text =
    De goatbuilder-CLI kan zichzelf bijwerken vanuit de kopie die je goatnet aanbiedt op `get.<site>.<zone>/goatbuilder/`.

    Het goatbuilder-commando `update --check` leest alleen. Het vergelijkt je versie met de aangeboden versie en verandert niets. Exitcode 0 betekent dat je bijgewerkt bent. Exitcode 3 betekent dat er een nieuwere versie is.

    Een echte update, zonder `--check`, controleert de checksum en de handtekening van de download voordat hij iets vervangt. Daarna verwisselt hij het bestand in één stap.

    De goatbuilder-CLI moet weten welk goatnet hij moet vragen: geef `--site` mee met dat goatnet, of stel `GOATBUILDER_UPDATE_URL` in. Je moet op de mesh van dat goatnet zitten. Als dat niet zo is, zegt `--check` dat hij de server niet kan bereiken en noemt hij het adres dat hij probeerde. Dat is een correct resultaat voor deze les.
tutor-l15-s1 = Toon de tests die gelden voor één goatnet
tutor-l6-text =
    Een release brengt een nieuwe versie in volgorde door drie goatnets: dev, dan preprod, dan prod.

    1. Iemand tagt de versie op main, en CI bouwt hem.
    2. dev: goatbuilder deployt hem en draait de acceptatietests.
    3. preprod: goatbuilder deployt hem opnieuw, draait de tests opnieuw en legt de resultaten vast.
    4. De versie wacht daarna een vaste tijd op preprod. Niets gaat vanzelf naar prod.
    5. prod: na die wachttijd draait een persoon de release met `--from prod`. De goatbuilder-CLI deployt naar prod en controleert alle drie de goatnets.

    Het goatbuilder-commando `promote status` toont waar elke component staat. Een kandidaat is een versie die getest wordt. De algemene vorm van een release is `goatbuilder release <component> --tag <tag> --from dev`. De commando's hieronder gebruiken één component van deze checkout en een voorbeeldtag. Elk ervan heeft `--dry-run`, dus er wordt niets getagd of gedeployd.

    De dry runs hebben `gh` en de mesh nodig. Als je geen admin van de repository bent, zet dan `GOAT_RC_ROLE_CHECK=0` voor elk `release`-commando.
tutor-l9-s1 = Maak een echte fout met een goatnet dat niet bestaat
tutor-l9-text =
    Als goatbuilder weigert, toont hij een code. Codes die beginnen met `GB-` komen van goatbuilder, `E_` van de goat-CLI, en `X-` van een ander hulpmiddel dat goatbuilder heeft uitgevoerd.

    Het goatbuilder-commando `explain` neemt een code, of de tekst van een foutregel, en vertelt je wat er gebeurde, waarom, hoe je het oplost en waar je meer kunt lezen.

    Vragen naar een goatnet dat deze checkout niet declareert, geeft een echte fout om uit te leggen.
