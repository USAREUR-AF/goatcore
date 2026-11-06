### goatbuilder — lv (lv)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Būvē un izlaid goat vidi: deklarē, konverģē, pārbaudi — zem viena izvietošanas līguma
help-flag-output = Izvades formāts: tabula cilvēkam; JSON vai YAML programmai
help-flag-repo = Repozitorija sakne. Pēc noklusējuma — checkout, no kura šis binārfails tiek palaists
help-flag-yes = Apstiprina deklaratīvu darbību bez vaicājuma (automatizācijai)
help-flag-frozen = Atsaka jebkuru palaidienu, kam būtu jāpārtin checkout uz priekšu (stingrais C3)
help-flag-mock = Darbojas pret fixture direktoriju, nevis hostiem: bez reģistra, bez transporta, deterministiski (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Palīdzības un kļūdu skaidrojumu valoda (tags, piemēram, fr, uk, de); izvade, ko lasa konveijers, paliek angļu valodā. Noklusējums: OS lokāle
help-cmd-validate = Pārbauda konfigurāciju pret versionēto shēmu un starplauku noteikumiem, bezsaistē
help-cmd-doctor = Pārbauda, kas vajadzīgs palaišanai, pirms kaut kas tiek griezts: atkarības, checkout svaigumu, katras vides transportu un to, ka katras komponentes playbook atrisinās uz konkrētu mērķa režīmu
help-cmd-status = Deklarētais pret ierakstīto, pa mašīnām: converged, pending vai unknown (C7, C8). Tikai lasa. Izejas kods 3, kamēr kāda mašīna nav converged
help-cmd-deploy = Uzliek vienai videi pin, ko deklarē main, un pierāda to: no pin atrisina komponenti un vides grupas, play un tā mērķa režīmu, katru noslēpumu, ko play izmanto veidnēs, un transportu; palaiž play caur ansible-preflight; nolasa izvietošanas žurnālu no katra hosta un uzskata par pabeigtu tikai tad, ja jaunākā rinda nosauc deklarēto digest ar action=deploy (C7). Uz fixture (`--mock`) hostu vietā darbojas skriptētais converger
help-cmd-release = Ved release candidate no taga uz main cauri dev goatnet dzīvajai validācijai līdz preprod pakāpei (ADR 1173); pēc soak ar `--from prod` to graduē, ieklāj fleet:, izvieto prod un atkārtoti pārbauda katru pakāpi. Aizstāj ops/scripts/rc.sh soli pa solim
help-cmd-rollback = Atritina izvietotu komponenti līdz tās N-tajai jaunākajai ref, pēc artefakta tipa, izmantojot hosta žurnālu kā patiesības avotu (ADR 1060). Aizstāj ops/scripts/rollback.sh: izejas kods 3 — atteikts kā nedrošs (data, cert, forward-only, reversible:false), 4 — vadīts (config, acl)
help-cmd-fastlane = Viena komanda no sapludinātā commit līdz katram goatnet (ADR 1127): apzināts izņēmums no kāpnēm. Aizstāj ops/scripts/fastlane.sh soli pa solim; atsaka nepinotu digest vai atvērtu graduācijas kopu, bet izvietošanu veic caur šo binārfailu
help-cmd-standup = Vietne no nulles līdz verificētai: standup fāzes doktrīnas secībā ar fail-closed vārtiem katrā galā, novērošanas hosts kā paralēls zars, rescue klases un starp-goatnet virsmas pēdējās (ADR 1183). Aizstāj ops/scripts/goat-standup.sh soli pa solim. Uz fixture (`--mock`) katras klases katru komponenti converģē skriptētais converger, un laiks no deklarēta līdz verificētam ir vienas stundas goatnet mērs
help-cmd-join-node = Vēl viena klases mašīna dzīvā vietnē: deklarēta → converged → verificēta: docs/tutorials/02-first-federated-node.md izpildītā secība kā numurētas fāzes ar vārtiem (sasniedzamība, atslēga, vispirms repo darbs, bāzlīnija, kodola WireGuard pirms un pēc reģistrācijas, kodola ACL plakne kā reģistrācijas vārti, klases converge, pieņemšana)
help-cmd-federate = Suverēna peering deklarēšana, konverģēšana un verificēšana (ADR 1157): mūsu puse izmērīta, pakotnes apmainītas ārpus joslas, viena vienošanās sastādīta un verificēta, no tās izgatavotas lapas, noteikumus renderē vienīgais rakstītājs, izvietota mala, atvērts vienošanās izvēlētais transports, pieņemšana visos četros līmeņos. Aizstāj ops/scripts/peering-standup.sh soli pa solim
help-cmd-pins = VIENS atrisināšanas cikls ar DIVĀM renderēšanām: katra norādītā dalībnieka group_vars pins.yml un katra norādītā goatnet IntentV1 artefakts no ops/pins/fleet-pins.yaml. Aizstāj ops/pins/generate-pins.py baitu pa baitam. `--check` validē, pārģenerē un atsaka novirzi, neparakstītu vai viltotu intent un pārvaldītu pin, kas definēts ārpus pins.yml (izejas kods 1)
help-cmd-promote = Vada un novēro release graduāciju pa goatnet kāpnēm (ADR 1120, 1125): status, atvērt kandidātu, graduēt, graduēt kopu, hotfix ceļš, review, readiness, abandon, check. Aizstāj ops/release/graduate.py soli pa solim. Nekas šeit negraduējas pats: mutācija notiek tikai pēc izsaukuma, izdrukā paveikto un bez termināļa prasa --yes (C10, ADR 1177)
help-cmd-explain = Interpretē kļūdu: kodu (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) vai jebkuru kļūdas tekstu — šī binārfaila paša vai tā pārnestos rīka vārdus (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Izdrukā, kas notika, kāpēc, ko darīt un kur lasīt vairāk; `-` nolasa tekstu no stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Vietne no verificētas līdz neesamībai — standup spogulis, ieraksti paliek (nojaukšana pievieno teardown rindas, nekad neizdzēš). Uz fikstūras (`--mock`) nojaukšanu atkārto apgrieztā fāžu secībā; uz īstiem hostiem izdrukā secību un atsakās izpildīt, kamēr katra fāze nav deklarējusi savu teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Mācības vimtutor stilā: katra nosauc precīzu komandu, ko palaist pret vienreizlietojamu fikstūras kopiju (vai, tikai lasīšanai, pret šo checkout), tad pārbauda, ko komanda patiesībā izdarīja — tās izejas kodu un rindas —, pirms atveras nākamā. `tutor` parāda pašreizējo mācību un atzīmē to kā pabeigtu, kad tavi palaidieni to apmierina; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Slēpta (src/fun.rs)
help-cmd-argan = Slēpta (src/fun.rs)
help-cmd-bound = Slēpta (crates/goat-fun)
help-cmd-secrets = Deklarētā noslēpumu nostāja (ADR 1204): kurš krātuves veids ir spēkā (1password, openbao, sops, git-crypt, env-file, env), kas ir pieejams, kā nodrošināt trūkstošo un paša krātuves pārbaudes. `init --env <goatnet>` ieraksta zero-SaaS ātrās sākšanas env failu: spēcīgu nejaušu vērtību katram mainīgajam, ko goatnet play veido no sagataves, 0600, gitignored, nekad nemainot jau esošu vērtību. Tas pats modulis, kas ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Nolasa katra hosta jaunāko deploy ledger rindu vienam komponentam un norāda, vai tajā ir ref, ko deklarē main (C7). Tikai lasa; izejas kods 3, kamēr kāds hosts ir pending vai unknown. Tā pati pārbaude, ko palaiž `deploy --verify-only`
help-cmd-diff = Kas pašlaik atšķiras no main (C8): main deklarētais ref pret katra hosta jaunāko ledger rindu vienam komponentam vai, ja nav norādīts, visiem vides komponentiem; nolasīts vienā piegājienā katrai dalībnieku grupai. Tikai lasa; izejas kods 3 pie jebkuras novirzes vai nenolasāma hosta
help-cmd-history = Deploy ledger kā laika līnija, jaunākais pirmais, visos vides hostos (C8): katra rinda ir atsevišķs ieraksts ar mašīnu, laiku, darbību (deploy vai rollback) un versiju. Ieraksta iepriekšējā versija un operators, kas to palaidis, ir `-o json`. Tikai lasa
help-cmd-plan = Ieraksta plāna failā pārskatīšanai, ko darītu deklaratīvā komanda: deklarētos ref, play un mērķi, ledger, main commit, transportu (deploy); artefakta tipu un mērķa ref (rollback). Nemaina nevienu hosta stāvokli. Palaid pārskatīto ar `apply`
help-cmd-apply = Izpilda tieši pārskatīto plānu. Atsakās (izejas kods 1, norādot, kas mainījies), ja kopš plāna sastādīšanas ir mainījusies main deklarācija, pins commit, play vai rollback mērķis; mainīts transports tiek ziņots, nevis noraidīts
help-cmd-adopt = Deklarē to, ko vide jau palaiž: nolasa katra hosta ledger un katram komponentam, kura darbojošais ref atšķiras no main deklarācijas, piedāvā pins novirzi. Pēc noklusējuma tikai atskaite; ar --yes izmaiņas tiek ierakstītas jaunā main worktree pārskatīšanai. Nekad neizvieto

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Vēl nē: { $why }
tutor-lesson-done = { $n }. nodarbība pabeigta: { $title }.
tutor-all-done-reset = Visas nodarbības pabeigtas. `goatbuilder tutor --reset` sāk no jauna; apmācības iet tālāk: docs/tutorials/.
tutor-all-done = Visas nodarbības pabeigtas. Apmācības iet tālāk: docs/tutorials/.
tutor-workspace = Darbvieta: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Pārbauda, vai goatnet ir uzbūvēts pareizi, slānis pa slānim: visas pieņemšanas pārbaudes, kas attiecas uz objektu, tostarp pārbaudes, ko operators veic ar goat CLI. Izvietošanas pieņemšanas fāze kā atsevišķa komanda. Tikai nolasa; beidzas ar kodu 3, kamēr kāda pārbaude neiztur vai nespēj atbildēt
help-flag-offline = Izolēta vietne: neielādēt galveno zaru (trunk); pārbaudīt izgūto kopiju (checkout) pret kešoto un atzīmēt palaidi kā bezsaistes (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Pārbauda, vai goatnet ir deklarētajā galastāvoklī: katrs pakalpojums, ko tās pašas deklarācijas norāda kā darbināmu, ir izvietots, darbojas, ir vesels un pilda savu uzdevumu, tā zero-trust savienojumi ir izveidoti, un nekas nedarbojas bez deklarācijas. Tikai lasa; iziet ar kodu 3, kamēr kaut kas nesakrīt
help-cmd-survey = Cik konsekventa ir programmatūra katrā goatnet iekšienē un starp ladder goatnet tīkliem, nolasīts vienā paralēlā piegājienā. Tikai lasa

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Cik konsekventa ir programmatūra katrā goatnet un starp kāpņu goatnet, nolasīts vienā paralēlā piegājienā. Tikai lasīšana.
help-cmd-update = Aizstāj šo bināro failu ar laidienu, ko goatnet nodrošina adresē `get.<site>.<zone>/goatbuilder/`: lejupielāde tiek pārbaudīta pret tās `.sha256` un parakstīto `SHA256SUMS`, pirms kaut kas tiek mainīts, un nomaiņa ir atomāra. Jau aktuāls: viena rinda, izejas kods 0. `--check` tikai lasa (izejas kods 3, ja atpaliek). Avots: `GOATBUILDER_UPDATE_URL`, citādi `--site` no tā group_vars, citādi tas, ko izmantoja pēdējais atjauninājums

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Pirms sākat: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Krāso tabulas ok/FAIL simbolu un verdiktu (auto|always|never). `auto` krāso tikai terminālī; NO_COLOR un CLICOLOR_FORCE ir augstākas prioritātes. JSON un YAML nekad netiek krāsoti
help-flag-verbose = Raksta uz stderr, ko palaišana dara — ārējās komandas, ko tā izsauc, un `update` tīkla fāzi —, lai `-o json|yaml` paliktu tīrs dokuments. Vai iestati RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekundes, cik ilgi ierobežot savienošanās/sasniedzamības fāzi pirms ātras kļūmes — vienīgā fāze, kas var iestrēgt pie nesasniedzama resursdatora (`update` apkalpotās versijas ielāde). NEierobežo paša play izpildi vai binārā faila lejupielādi (to ierobežo apstāšanās). Nolasa arī GOATBUILDER_TIMEOUT
help-cmd-completion = Izvada čaulas pabeigšanas skriptu uz stdout (`goatbuilder completion zsh >> ~/.zshrc`). Dzīviem kandidātiem, ko statisks skripts nevar zināt — vietnēm, komponentiem un goatnet, ko deklarē šis checkout — pieslēdz āķi pie `goatbuilder _complete` (skat. ģenerētā skripta galveni)
help-cmd-docs = Komandu atsauce: `goatbuilder docs` norāda uz to; `--markdown` izvada visu, izejot cauri dzīvajam komandu kokam, tāpēc tā nekad nevar atšķirties no `--help`
help-cmd-_complete = Slēpta: izvada dzīvos pabeigšanas kandidātus čaulas āķiem, pa vienam rindā (vietnes, komponenti un goatnet, ko deklarē šis checkout). Tikai lasa, nekad neziņo par kļūmi skaļi — pabeigšanas āķis nedrīkst sabojāt uzvedni

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Pirms izvietošanas: gatavības un atgriezeniskuma izvade (ADR 1211): trīs fakti — State (stateless | stateful), Rollback (pin-revert | restore-required | none) un Scope (single | coupled) — deklarēti katram komponentam un FAIL-CLOSED, ja nav deklarēti; gatavības daļa, ko pierāda pārējās komandas (ref ir velkams, pins --check, noslēpumu vārti, svaigs avots, deklarētais gala stāvoklis), un atgriezeniskuma fakta validācija (rollback --dry-run priekš pin-revert, glabāšanas logs priekš restore-required). `deploy` to palaiž pirmo; vērtēts pēc godīguma, nekad nav viltus zaļš. Tikai lasa; iziet ar kodu 3, ja nav tīrs

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Pārbaudi savu uzstādījumu
tutor-l1-s1 = Pārbaudi konfigurācijas failu
tutor-l2-title = Lasi stāvokli: converged, pending, unknown
tutor-l2-text =
    Komanda goatbuilder `status` salīdzina divas lietas katrai mašīnai: versiju, ko tai main deklarē, un versiju, ko nosauc tās pašas ieraksts. Tas nemaina nekā.

    Katra mašīna ir vienā no trim stāvokļiem:
    - converged (konvergēta): tās ieraksts nosauc deklarēto versiju. Tas ir izdarīts.
    - pending (gaida): tās ieraksts nosauc vecāku versiju, vai pagaidām neko.
    - unknown (nezināma): goatbuilder nevar izlasīt tās ierakstu.

    Tikai converged skaitās kā izdarīts. Ja kāda mašīna ir pending vai unknown, `status` beidzas ar kodu 3.

    No šīs nodarbības `--mock` novirza goatbuilder uz prakses kopiju tavā tutor mapē, nevis uz reālām mašīnām. Nekas, ko tu dari tur, nesasniedz reālu mašīnu.

    Prakses kopijas `dev` pagaidām nekas nav izvietots. Tāpēc sagaidi četras pending mašīnas un vienu unknown. Converged mašīnas tu redzēsi 3. nodarbībā.
tutor-l2-s1 = Izlasi dev stāvokli
tutor-l3-title = Izvieto: ieplāno, palaid, pierādi
tutor-l3-text =
    Izvietošana uzliek deklarēto versiju katrai mašīnai, tad lasa katras mašīnas ierakstu, lai pierādītu, ka tā ir atnākusi. Izvietošana ir izdarīta tikai tad, kad katras mašīnas ieraksts nosauc to versiju. Izvietošana, kas pagāja bez kļūdām, nav pietiekama.

    Tu izvietosi `portal` prakses kopijas `preprod`. `--dry-run` parāda, kas notiktu, un nemaina nekā. `--ack` ieraksta, kurš to izdarīja; komanda tavu vārdu ņem no git. Pēc izvietošanas `status` jāparāda abas mašīnas kā `ok` un converged.
tutor-l3-s1 = Ieplāno izvietošanu
tutor-l3-s2 = Izvieto portal uz preprod
tutor-l3-s3 = Pārbaudi, ka abas mašīnas ir converged
tutor-l4-title = Novirze un vēsture
tutor-l4-text =
    Divas komandas palīdz saprast goatnet pirms un pēc izmaiņas. Abas tikai lasa.
    - Komanda goatbuilder `diff` uzskaita katru mašīnu, kuras ieraksts atšķiras no deklarētās versijas. Atšķirību sauc par novirzi.
    - Komanda goatbuilder `history` uzskaita katru versiju, kāda komponentei bija katrā mašīnā, jaunākā pirmā.

    Uz `dev`, katra mašīna ir atpalikusi. `portal` vēsture uz `preprod` tagad ietver tavu izvietošanu no 3. nodarbības.
tutor-l4-s1 = Uzskaiti novirzi uz dev
tutor-l4-s2 = Izlasi portal vēsturi uz preprod
tutor-l5-title = Atritini atpakaļ un redzi, kā nedrošu atritināšanu atsaka
tutor-l5-text =
    Atritināšana deklarē agrāku versiju un izvieto to kā jebkuru citu versiju. Tad goatbuilder CLI lasa katras mašīnas ierakstu, lai pierādītu, ka tā ir atnākusi.

    Dažas izmaiņas šādi nevar atcelt. Vecākas versijas izvietošana nevar atcelt datus, ko komponente ir pierakstījusi, sertifikātu, vai izmaiņu, kas virzās tikai uz priekšu. goatbuilder CLI atsaka tādas ar kodu 3 un pasaka, ko darīt tā vietā.

    Prakses kopijas `db-schema` komponente glabā datus, tāpēc goatbuilder atsaka to atritināt. Atteikums ir sagaidāmais rezultāts.
tutor-l5-s1 = Ieplāno atritināšanu
tutor-l5-s2 = Atritini portal vienu versiju atpakaļ
tutor-l5-s3 = Pamēģini atritināt db-schema, ko goatbuilder atsaka
tutor-l6-title = Izlaid jaunu versiju: dev, tad preprod, tad prod
tutor-l6-s1 = Paskaties, kur ir katra komponente
tutor-l6-s2 = Izdari release izmēģinājuma palaidienu no dev
tutor-l6-s3 = Izdari prod soļa izmēģinājuma palaidienu
tutor-l7-title = Fastlane, un kad to nelietot
tutor-l7-text =
    Fastlane uzliek vienu versiju visiem goatnet reizē. Tas izlaiž gaidīšanu uz preprod un release ierakstus.

    Lieto to tikai steidzāmam labojumam, ko vari atcelt, atgriežoties pie iepriekšējās versijas. Nekad to nelieto datubāzes izmaiņai, identitātes vai pieejas izmaiņai, vai jebkam, ko šādi nevari atcelt.

    Fastlane atsaka versiju, ko CI nav uzbūvējis, un komponenti, kas ir daļa no release, kas vēl notiek.

    Vispārīgā forma ir `goatbuilder fastlane <component> --ref <version> --reason <why>`. Palaįd zemāk esošo piemēru. Tam ir `--dry-run`, tāpēc nekas nemainās. Tam vajadzīgs `gh`.
tutor-l7-s1 = Izdari fastlane izmēģinājuma palaidienu
tutor-l8-title = Pins: atrodi novirzi un labo to pareizi
tutor-l8-text =
    Viens fails nosaka, kuru versiju palaiž katrs goatnet: `ops/pins/fleet-pins.yaml`. No tā goatbuilder CLI generē pārējos versijas failus: `pins.yml` failus un parakstītos intent failus.

    Komanda goatbuilder `pins --check` salīdzina generētos failus ar `fleet-pins.yaml` un atsaka katru failu, kas atšķiras. Komanda goatbuilder `pins` tos pārgenerē.

    Tavā tutor mapē ir repozitorija kopija, kurā kāds ar roku rediģējis vienu `pins.yml` failu. Nekad nelabo roku labojumu ar citu roku labojumu. Maini `fleet-pins.yaml`, tad pārgenerē.
tutor-l8-s1 = Pārbaudi šo checkout
tutor-l8-s2 = Pārbaudi kopiju un atrodi roku labojumu
tutor-l8-s3 = Pārgenerē kopijas failus
tutor-l8-s4 = Pārbaudi kopiju vēlreiz
tutor-l9-title = Paskaidro kļūdu
tutor-l9-s2 = Paskaidro kļūdu pēc tās koda
tutor-l9-s3 = Paskaidro to pēc tās teksta
tutor-l10-title = 1. apmācība: izveido jauna goatnet pirmo vietni
tutor-l10-text =
    Šī nodarbība izveido veselu goatnet prakses kopijā. Piemēra goatnet ir `zagros`. Tā pirmajai vietnei, `alpha`, ir divas pamata mašīnas, viena novērošanas mašīna un viena glābšanas mašīna.

    Komanda goatbuilder `standup` sagatavo katru mašīnu noteiktā secībā un pārbauda katru ierakstu. Tā pēdējā rinda saka, ka vietne ir izdarīta. Uz reālām mašīnām tā rinda arī parāda, cik ilgi tas prasīja.

    Pilnā apmācība ir `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Izveido pirmo vietni, alpha
tutor-l10-s2 = Pārbaudi rezultātu
tutor-l11-title = 2. apmācība: pievieno otru vietni
tutor-l11-text =
    Šī nodarbība sākas tur, kur beidzās 1. apmācība. Tā pievieno otru vietni, `beta`, jaunā prakses kopijā, `zagros-2`.

    Otra vietne kopē lietotāju kontus no pirmās vietnes. Tās standup arī vienreiz maina pirmo vietni, lai kopēšana darbotos abos virzienos. To izdara solis ar nosaukumu `alpha/core`.

    Pilnā apmācība ir `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Izveido otro vietni, beta
tutor-l11-s2 = Pārbaudi abas vietnes
tutor-l12-s1 = Ieplāno vienas mašīnas pievienošanu
tutor-l13-s1 = Ieplāno goatnet savienošanu
tutor-l14-title = Turi goatbuilder aktuālu
tutor-l14-s1 = Pajautā, vai eksistē jaunāks goatbuilder
tutor-l15-title = Assay: pārbaudi, kā goatnet tika būvēts
tutor-l15-text =
    Komanda goatbuilder `assay` pārbauda, ka goatnet ir uzbūvēts pareizi, pa vienam slānim, lietojot pieņemšanas testus. Katrs tests saņem vienu no trim rezultātiem:
    - PASS: tests izturēja.
    - FAIL: goatnet ir problēma.
    - UNKNOWN: nekas nevarēja atbildēt uz testu.

    Skaitās tikai PASS. Jebkurš FAIL vai UNKNOWN padara kodu 3.

    `--plan` uzskaita testus, kas attiecas uz goatnet, pa slāņiem, un neskar nevienu mašīnu. Lieto to, lai redzētu, ko aptvertu reāls assay. Vispārīgā forma ir `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: vai versijas ir konsekventas?
tutor-l16-text =
    Komanda goatbuilder `survey` vienā piegājienā pārbauda, vai tās pašas versijas darbojas, kur tām jābūt:
    - Katra goatnet iekšienē: vai katra mašīna palaiž deklarēto versiju, un vai mašīnas, kurām jāsakrīt, patiesi sakrīt?
    - Starp dev, preprod un prod: viena rinda katrai komponentei, viena kolonna katram goatnet.

    Atšķirība, ko sagaida release process, tiek ziņota, bet nav kļūda. Piemērs ir jauna versija, kas ir uz dev, bet vēl nav uz prod. Atšķirība, ko nekas nedeklarē, tiek lasīta kā `difference not declared`. Mašīna, ko goatbuilder nevar izlasīt, ir unknown, nekad konsekventa.

    Palaid to prakses kopijā. Tās `dev` ir ar nolūku atpalicis, tāpēc sagaidi atradumus un kodu 3.
tutor-l16-s1 = Palaid survey prakses kopijā
tutor-l17-title = Endstate: vai goatnet palaiž tieši to, ko deklarē?
tutor-l17-text =
    Komanda goatbuilder `endstate` pārbauda divas lietas par goatnet:
    1. Katrs pakalpojums, ko tas deklarē, ir izvietots, darbojas, ir vesels un pareizi savienots.
    2. Nekas nedarbojas, ko tas nedeklarē. Nedeklarētu pakalpojumu sauc par bāreni.

    goatbuilder CLI nosaka, kas vietnei jāpalaiž, no pašas vietnes iestatījumiem, nevis no atsevišķa saraksta.

    Pārbaude, kurai nav ko lasīt, tiek atzīmēta kā UNOBSERVED. Tā ir godīga atbilde, nevis kļūme. FAIL vai bārenis padara kodu 3.

    Vispārīgā forma ir `goatbuilder endstate <goatnet>`. Zemāk esošā komanda pārbauda piemēra vietni no saglabātiem datiem, tāpēc tā neskar nevienu mašīnu.
tutor-l17-s1 = Pārbaudi piemēra vietni
tutor-l18-title = Bonuss: esi zinātkārs
tutor-l18-text =
    Šī nodarbība ir neobligāta. Palaid zemāk esošo komandu, un tutor ir pabeigts.

    goatbuilder CLI ir dažas mazas slēptas funkcijas. Tās nav palīdzībā vai šajās nodarbībās. Dažas atbild uz vārdiem, ko ilggadēji Unix lietotāji bieži ieraksta jaunā programmā, tikai lai redzētu, kas notiek. Lielākā daļa lietu, ko tu pamēģināsi, nedarīs nekā.

    Šī nodarbība nesaka, kas tās ir. To atrašana ir jēga.

    Zemāk esošā komanda uzskaita katru kļūdu, ko goatbuilder var paskaidrot. Tā ir laba vieta, kur sākt izzināt.
tutor-l18-s1 = Uzskaiti katru kļūdu, ko goatbuilder var paskaidrot
tutor-heading = { $n }. nodarbība no { $total } · { $title }
tutor-setup = Vispirms vienreiz palaid šo šajā terminālā:
tutor-not-recording = Šis termināls neieraksta tavus palaidienus, tāpēc tutor nevar atzīmēt tavus soļus. Palaid šo vienreiz, tad palaid soļus:
tutor-type = Izdari šos soļus savā terminālā ({ $met } no { $total } izdarīti):
tutor-rerun = Pēc katra soļa palaid `goatbuilder tutor` no jauna. Tas pārbauda, ko tu izdarīji, un atzīmē soli.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Laipni lūdzam GOATBUILDER Tutor
tutor-bar-progress = Progress
tutor-bar-done = { $done } no { $total } nodarbībām izdarītas
tutor-after-step = Solis { $met } no { $total } izdarīts. Tālāk:
tutor-after-lesson = Nodarbība { $n } ir izdarīta. Palaįd `goatbuilder tutor`, lai sāktu nodarbību { $next }. Lai to nolasītu atkal: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Nodarbība { $n } ir izdarīta, un tutors arī. Palaįd `goatbuilder tutor`, lai redzētu, ka tas atzīmēts kā izdarīts. Lai to nolasītu atkal: `goatbuilder tutor --lesson { $n }`.

tutor-review = Tu pabeidzi šo nodarbību, tāpēc šī lapa ir tikai lasīšanai. Palaįd `goatbuilder tutor`, lai atgrieztās tur, kur esi.
tutor-other-lessons = Lai nolasītu nodarbību atkal: `goatbuilder tutor --lesson 1`. Lai redzētu visas: `goatbuilder tutor --list`.
tutor-l12-title = Pievieno mašīnu vietnei, kas jau darbojas
tutor-l12-text =
    Komanda goatbuilder `join-node` pievieno vēl vienu mašīnu vietnei, kas jau darbojas. Tā vispirms parāda plānu, ar vienu rindu katrai fāzei. Katrai fāzei ir tips:
    - tool: goatbuilder palaiž skriptu.
    - ansible: goatbuilder palaiž playbook.
    - gate: pārbaude, kurai jābūt sekmīgai pirms nākamās fāzes.
    - manual: tu to dari pats, un izpilde apstājas.

    Komandai zemāk ir `--dry-run`. Tā lasa šo checkout un nemaina nekā. Zem tabulas lasi, ko tu dari ar roku.
tutor-l13-title = Savieno ar citas organizācijas goatnet
tutor-l13-text =
    Komanda goatbuilder `federate` savieno tavu goatnet ar goatnet, ko palaiž cita organizācija. Katra puse izveido savu puses daļu no vienošanās. Tu apmaini puses, un katra puse apvieno tās vienā un tajā pašā vienošanā.

    Apstiprini otras puses atslēgu atsevišķā veidā, piemēram, ar zvanu, kam tu tici. Atslēga, kas ierodas viņu pakā, nekā nepierāda.

    Komandai zemāk ir `--dry-run`, tāpēc tā nemaina nekā. Reāla izpilde apstājas 4. fāzē, jo šo fāzi tu dari ar roku.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Šī komanda nav nodarbības { $n } daļa. Palaid `goatbuilder tutor`, lai redzētu, kur esi.
tutor-l1-s2 = Pajautā doctor par vienu goatnet
tutor-l1-text =
    goatbuilder CLI būvē, izvieto un izlaiž GOAT programmatūru. Pirms to lieto, pārbaudi savu uzstādījumu ar divām komandām:
    - Komanda goatbuilder `validate` pārbauda konfigurācijas failu `ops/goatbuilder/goatbuilder.yaml`. Tā lasa tikai šo checkout, un tai nav vajadzīgs tīkls.
    - Komanda goatbuilder `doctor` pārbauda to, kas vajadzīgs īstai izvietošanai: tavus rīkus, tavas pieteikšanās, vai šis checkout atbilst main un kā goatbuilder sasniedz katru goatnet. Goatnet ir viena GOAT vide.

    Komanda goatbuilder `doctor` izdrukā, cik pārbaužu izdevās, un tad katru problēmu ar tās labojumu. Šajā nodarbībā problēma ir informācija, nevis kļūda. Izlasi labojumus, bet tev tie tagad nav jāpiemēro.

    Lielākajai daļai nodarbību pieteikšanās nav vajadzīga. Nodarbības no 2 līdz 5, 10, 11 un 16 izmanto prakses kopiju. Nodarbības 8, 9, 12, 13, 15, 17 un 18 tikai lasa šo checkout. 6. nodarbībai vajag `gh` un mesh, 7. nodarbībai vajag `gh`, un 14. nodarbībai vajag mesh. Nevienai nodarbībai nav vajadzīgs 1Password.
tutor-l14-text =
    goatbuilder CLI var pats sevi atjaunināt no kopijas, ko tavs goatnet piedāvā adresē `get.<site>.<zone>/goatbuilder/`.

    Komanda goatbuilder `update --check` tikai lasa. Tā salīdzina tavu versiju ar piedāvāto versiju un neko nemaina. Izejas kods 0 nozīmē, ka tev ir jaunākā versija. Izejas kods 3 nozīmē, ka ir pieejama jaunāka versija.

    Īsta atjaunināšana, bez `--check`, pārbauda lejupielādes kontrolsummu un parakstu, pirms kaut ko aizstāj. Pēc tam tā nomaina failu vienā solī.

    goatbuilder CLI ir jāzina, kuram goatnet jautāt: norādi `--site` ar šo goatnet vai iestati `GOATBUILDER_UPDATE_URL`. Tev jābūt šī goatnet mesh tīklā. Ja neesi, `--check` paziņo, ka nevar sasniegt serveri, un nosauc adresi, ko mēģināja. Šai nodarbībai tas ir pareizs rezultāts.
tutor-l15-s1 = Uzskaiti testus, kas attiecas uz vienu goatnet
tutor-l6-text =
    Laidiens pārvieto jaunu versiju caur trim goatnet pēc kārtas: dev, tad preprod, tad prod.

    1. Kāds atzīmē versiju ar tagu main zarā, un CI to uzbūvē.
    2. dev: goatbuilder to izvieto un palaiž pieņemšanas testus.
    3. preprod: goatbuilder to izvieto vēlreiz, vēlreiz palaiž testus un ieraksta rezultātus.
    4. Pēc tam versija noteiktu laiku gaida preprod. Nekas nepāriet uz prod pats no sevis.
    5. prod: pēc šīs gaidīšanas cilvēks palaiž laidienu ar `--from prod`. goatbuilder CLI izvieto uz prod un pārbauda visus trīs goatnet.

    Komanda goatbuilder `promote status` parāda, kur atrodas katrs komponents. Kandidāts ir versija, ko testē. Laidiena vispārējā forma ir `goatbuilder release <component> --tag <tag> --from dev`. Zemāk esošās komandas izmanto vienu šī checkout komponentu un parauga tagu. Katrai no tām ir `--dry-run`, tāpēc nekas netiek atzīmēts ar tagu vai izvietots.

    Izmēģinājuma palaišanām vajag `gh` un mesh. Ja neesi repozitorija administrators, pirms katras `release` komandas ieraksti `GOAT_RC_ROLE_CHECK=0`.
tutor-l9-s1 = Izraisi īstu kļūdu ar goatnet, kas neeksistē
tutor-l9-text =
    Kad goatbuilder atsaka, tas izdrukā kodu. Kodi, kas sākas ar `GB-`, nāk no goatbuilder, `E_` no goat CLI, un `X-` no cita rīka, ko goatbuilder palaida.

    Komanda goatbuilder `explain` pieņem kodu vai kļūdas rindas tekstu un pasaka, kas notika, kāpēc, kā to labot un kur lasīt vairāk.

    Pieprasot goatnet, ko šis checkout nedeklarē, rodas īsta kļūda, ko paskaidrot.
