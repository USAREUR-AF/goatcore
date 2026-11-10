### goatbuilder — et (et)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Ehita ja tarni goat-keskkond: deklareeri, konvergeeri, kontrolli — ühe deploy-lepingu alusel
help-flag-output = Väljundivorming: tabel inimesele; JSON või YAML programmile
help-flag-repo = Hoidla juurkataloog. Vaikimisi checkout, kus seda binaari käivitatakse
help-flag-yes = Kinnita deklaratiivne verb ilma küsimata (automaatika jaoks)
help-flag-frozen = Keeldu igast käivitusest, mis vajaks checkouti fast-forward'i (range C3)
help-flag-mock = Käivita fixture-kataloogi vastu hostide asemel: ei registrit, ei transporti, deterministlik (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Spikri ja veaselgituste keel (silt nagu fr, uk, de); väljund, mida pipeline loeb, jääb inglise keelde. Vaikimisi: OS-i lokaat
help-cmd-validate = Kontrolli konfiguratsiooni versioonitud skeemi ja väljadevaheliste reeglite vastu, võrguühenduseta
help-cmd-doctor = Uuri, mida käivitus vajab, enne kui midagi lõigatakse: sõltuvused, checkouti värskus, iga keskkonna transport ning see, et iga komponendi playbook lahendub kindlale target-režiimile
help-cmd-status = Deklareeritud versus registreeritud, masina kaupa: converged, pending või unknown (C7, C8). Ainult loeb. Exit 3, kuni mõni masin pole converged
help-cmd-deploy = Pane main'i deklareeritud pin ühele keskkonnale ja tõesta see: lahenda komponent ja keskkonna grupid pinnide põhjal, play ja selle target-režiim, iga secret, mida play templeerib, transport; käivita play ansible-preflight'i kaudu; loe deploy ledger igalt hostilt tagasi ja loe valmis alles siis, kui uusim rida nimetab deklareeritud digesti ja action=deploy (C7). Fixture'il (`--mock`) asendab hoste skriptitud converger
help-cmd-release = Vii release candidate main'i sildist läbi dev goatneti live-valideerimise preprod-astmeni (ADR 1173); pärast soak'i `--from prod` abil gradueeri see, põimi fleet:, deploy prod ja kontrolli uuesti iga aste. Asendab ops/scripts/rc.sh sammhaaval
help-cmd-rollback = Keruta deploy'tud komponent tagasi tema N-ndale viimasele ref'ile artefakti tüübi järgi, võttes tõe allikaks hostisisese ledgeri (ADR 1060). Asendab ops/scripts/rollback.sh: exit 3 keeldutud kui ohtlik (data, cert, forward-only, reversible:false), 4 juhendatud (config, acl)
help-cmd-fastlane = Üks käsk liitunud commitist iga goatnetini (ADR 1127): tahtlik erand astmestikust. Asendab ops/scripts/fastlane.sh sammhaaval; keeldub pinnimata digestist või avatud graduation set'ist ning käivitab deploy'd selle binaari kaudu
help-cmd-standup = Sait tühjalt verifitseerituks: standup'i faasid doktriini järjekorras, mõlemas otsas fail-closed värav, observability host paralleelharuna, rescue-klassid ja goatnetidevahelised pinnad viimasena (ADR 1183). Asendab ops/scripts/goat-standup.sh sammhaaval. Fixture'il (`--mock`) konvergeerib skriptitud converger iga klassi iga komponendi ning aeg deklareeritud → verifitseeritud on ühetunnise goatneti mõõt
help-cmd-join-node = Veel üks klassi masin töötavas saidis, deklareeritud → converged → verifitseeritud: docs/tutorials/02-first-federated-node.md täidetud järjestus nummerdatud faasidena koos väravatega (ligipääs, võti, repo töö kõigepealt, baseline, kernel WireGuard enne ja pärast enrolment'i, kernel ACL plane kui enrolment'i värav, klassi converge, vastuvõtt)
help-cmd-federate = Suveräänne peering deklareeritud, konvergeeritud ja verifitseeritud (ADR 1157): meie pool mõõdetud, paketid vahetatud out of band, üks agreement koostatud ja verifitseeritud, lehed sellest vermitud, reeglid renderdatud ühe kirjutaja poolt, edge deploy'tud, agreement'i valitud transport avatud, vastuvõtt kõigil neljal tasemel. Asendab ops/scripts/peering-standup.sh sammhaaval
help-cmd-pins = ÜKS lahenduspass KAHE renderdusega: iga loetletud liikme group_vars pins.yml ja iga loetletud goatneti IntentV1 artefakt, failist ops/pins/fleet-pins.yaml. Asendab ops/pins/generate-pins.py baidi baidi haaval. `--check` valideerib, regenereerib ja keeldub driftist, allkirjastamata või rikutud intent'ist ning pins.yml-ist väljaspool defineeritud valitsetud pinnist (exit 1)
help-cmd-promote = Juhenda ja jälgi release'i graduation'it goatneti astmestikus (ADR 1120, 1125): status, ava candidate, gradueeri, gradueeri set, hotfix-tee, review, readiness, abandon, check. Asendab ops/release/graduate.py sammhaaval. Siin ei gradueeru miski ise: mutatsioon jookseb ainult kutsumisel, prindib, mida tegi, ja küsib --yes ilma terminalita (C10, ADR 1177)
help-cmd-explain = Tõlgenda viga: kood (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) või mis tahes veatekst — selle binaari enda või tema kantud tööriista sõnad (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Prindib, mis juhtus, miks, mida teha ja kust lugeda lisaks; `-` loeb teksti stdin-ist

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Sait kontrollitud olekust tühjaks — standupi peegelpilt, kirjed säilivad (mahavõtt lisab teardown-ridu, ei kustuta kunagi midagi). Testkeskkonnas (`--mock`) harjutab mahavõttu faaside vastupidises järjekorras; päris hostidel prindib järjekorra ja keeldub täitmast, kuni iga faas pole oma teardown-play'd deklareerinud (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Õppetunnid vimtutori stiilis: iga tund nimetab täpse käsu, mida käivitada fixture'i ühekordsel koopial (või kirjutuskaitstult selles checkoutis), ning kontrollib siis, mida käsk tegelikult tegi — selle väljumiskoodi ja ridu — enne kui järgmine avaneb. `tutor` näitab sinu praegust tundi ja märgib selle tehtuks, kui sinu käivitused seda rahuldavad; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Peidetud (src/fun.rs)
help-cmd-argan = Peidetud (src/fun.rs)
help-cmd-bound = Peidetud (crates/goat-fun)
help-cmd-secrets = Deklareeritud saladuste hoiak (ADR 1204): milline hoidla kehtib (1password, openbao, sops, git-crypt, env-file, env), mis on saadaval, kuidas puuduv ette anda ja hoidla enda kontrollid. `init --env <goatnet>` kirjutab null-SaaS kiirstardi env-faili: tugev juhuslik väärtus igale muutujale, mida goatneti play'd mallivad, 0600, gitignore'is, kunagi ei roteeri seal juba olevat väärtust. Sama moodul mis ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Loe ühe komponendi kohta iga hosti värskeim deploy-ledger'i rida ja ütle, kas see nimetab ref'i, mille main deklareerib (C7). Ainult loeb; väljumiskood 3, kuni mõni host on pending või unknown. Sama kontroll, mida käivitab `deploy --verify-only`
help-cmd-diff = Mis erineb praegu mainist (C8): main'i deklareeritud ref iga hosti värskeima ledger'i rea vastu, ühe komponendi kohta või, kui komponenti pole, keskkonna kõigi komponentide kohta, loetuna ühe läbimisega liikmegrupi kohta. Ainult loeb; väljumiskood 3 mis tahes triivi või loetamatu hosti korral
help-cmd-history = Deploy-ledger ajajoonena, uuemad ees, keskkonna igal hostil (C8): iga rida on eraldi kirje masina, aja, tegevuse (deploy või rollback) ja versiooniga. Kirje eelmine versioon ja seda käivitanud operaator on `-o json`'is. Ainult loeb
help-cmd-plan = Kirjuta ülevaatuseks plaanifaili, mida deklaratiivne verb teeks: deklareeritud ref'id, play ja siht, ledger, main'i commit, transport (deploy); artefakti tüüp ja siht-ref (rollback). Ei puuduta ühegi hosti olekut. Käivita üle vaadatud kujul käsuga `apply`
help-cmd-apply = Käivita täpselt üle vaadatud plaan. Keeldub (väljumiskood 1, nimetab, mis liikus), kui main'i deklaratsioon, pins'i commit, play või rollback'i siht on pärast plaani kirjutamist liikunud; muutunud transport raporteeritakse, mitte ei keelduta
help-cmd-adopt = Deklareeri, mida keskkond juba jooksutab: loe kõigi hostide ledger'id ja paku pins'i erand iga komponendi kohta, mille jooksev ref erineb main'i deklaratsioonist. Vaikimisi raport; koos --yes kirjutatakse muudatused ülevaatuseks main'i uude worktree'sse. Ei deploy'i kunagi

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Veel mitte: { $why }
tutor-lesson-done = Õppetund { $n } tehtud: { $title }.
tutor-all-done-reset = Kõik õppetunnid on tehtud. `goatbuilder tutor --reset` alustab otsast; õpetused lähevad kaugemale: docs/tutorials/.
tutor-all-done = Kõik õppetunnid on tehtud. Õpetused lähevad kaugemale: docs/tutorials/.
tutor-workspace = Tööruum: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Kontrollib kiht kihi haaval, et goatnet on õigesti üles ehitatud: iga asukohale kehtiv vastuvõtukontroll, sealhulgas kontrollid, mida operaator goat CLI-ga käivitab. Standupi vastuvõtufaas eraldi käsuna. Ainult loeb; väljumiskood 3, kuni mõni kontroll ebaõnnestub või ei saa vastata
help-cmd-survey = Kui järjepidev on tarkvara iga goatneti sees ja redeli goatnetide vahel, loetuna ühe paralleelse läbimisega. Ainult loeb
help-flag-offline = Isoleeritud sait: ära tõmba trunk'i; hinda checkout'i vahemälus oleva trunk'i vastu ja märgi käivitus offline-käivituseks (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Kontrolli, kas goatnet on oma deklareeritud lõppseisundis: iga teenus, mida selle enda deklaratsioonid väidavad töötavat, on paigaldatud, töötab, on terve ja täidab oma ülesannet, selle zero-trust servad on ühendatud ning ükski deklareerimata teenus ei tööta. Ainult loeb; väljumiskood 3, kuni midagi on valesti
help-cmd-survey-long =
    Kui järjepidev on tarkvara iga goatneti sees ja redeli goatnetide vahel, loetuna ühe paralleelse läbimisega. Ainult loeb. Goatneti sees: main'is deklareeritud ref iga hosti uusima ledgeri rea ja selle kujutise konteinerite vastu (töötav konteiner; peatatut hinnatakse ainult soojal varul, kus midagi ei tööta), ühe goatneti hostid, mis ei ühti, iga hosti goat CLI selle vastu, mida tema goatnet jagab, ning mida goat-get jagab main'i vastu. Goatnetide vahel: üks rida komponendi kohta, üks veerg goatneti kohta, redeli hinnanguga iga erinevuse jaoks, nagu promote status selle annab. Väljumiskood 3 iga triivi või lugemata jäänu korral; redeli deklareeritud erinevusest antakse teada, see ei ole viga.
help-cmd-update = Asendab selle binaari väljalaskega, mida goatnet pakub aadressil `get.<site>.<zone>/goatbuilder/`: allalaadimist kontrollitakse enne millegi puudutamist selle `.sha256` ja allkirjastatud `SHA256SUMS` vastu ning vahetus on atomaarne. Juba ajakohane: üks rida, väljumiskood 0. `--check` ainult loeb (väljumiskood 3, kui maha jäänud). Allikas: `GOATBUILDER_UPDATE_URL`, muidu `--site` selle group_vars-ist, muidu viimase uuenduse oma

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Enne alustamist: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Värvi tabeli ok/FAIL-sümbol ja verdikt (auto|always|never). `auto` värvib ainult terminali; NO_COLOR ja CLICOLOR_FORCE on ülimuslikud. JSON ja YAML ei ole kunagi värvitud
help-flag-verbose = Logi stderr-i, mida käivitus teeb — välised käsud, mida see kutsub, ja `update` võrgufaas —, et `-o json|yaml` jääks puhtaks dokumendiks. Või määra RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekundid, mille piires ühendumis-/kättesaadavusfaas peab lõppema, muidu tekib kiire tõrge — see on ainus faas, mis võib kättesaamatu hosti taga blokeeruda (`update`'i serveeritud versiooni pärimine). EI piira play enda käivitust ega binaari allalaadimist (seda piirab seiskumine). Loeb ka GOATBUILDER_TIMEOUT
help-cmd-completion = Trüki shelli täienduskript stdout-i (`goatbuilder completion zsh >> ~/.zshrc`). Reaalajas kandidaatide jaoks, mida staatiline skript teada ei saa — saidid, komponendid ja goatnetid, mida see checkout deklareerib — ühenda hook käsuga `goatbuilder _complete` (vaata genereeritud skripti päist)
help-cmd-docs = Käsuviide: `goatbuilder docs` viitab sellele; `--markdown` väljastab kogu viite, läbitud elavast käsupuust, nii et see ei saa `--help`-st kunagi lahku minna
help-cmd-_complete = Peidetud: väljasta shelli hookidele reaalajas täienduskandidaadid, üks rea kohta (saidid, komponendid ja goatnetid, mida see checkout deklareerib). Ainult loeb, ei anna kunagi valjusti tõrget — täienduse hook ei tohi viipa rikkuda

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Enne väljalaset: valmisoleku ja tagasipööratavuse ülevaade (ADR 1211). Kolm fakti: State (stateless | stateful), Rollback (pin-revert | restore-required | none) ja Scope (single | coupled). Deklareeritakse komponendi kohta ja deklareerimata jätmisel FAIL-CLOSED. Valmisoleku poole tõestavad teised verbid (ref on tõmmatav, pins --check, saladuste värav, värske lähtekood, deklareeritud lõppseisund) ning lisandub tagasipöördefakti valideerimine (rollback --dry-run pin-revert'i jaoks, säilitusaken restore-required'i jaoks). `deploy` käivitab selle esimesena; hinnatud aususe alusel, mitte kunagi valerohelist. Ainult loeb; väljumiskood 3, kui pole puhas

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kontrolli oma seadistust
tutor-l1-s1 = Kontrolli konfiguratsioonifaili
tutor-l2-title = Loe olekut: converged, pending, unknown
tutor-l2-text =
    Goatbuilderi `status` käsk võrdleb iga masina kohta kahte asja: versiooni, mille main sellele deklareerib, ja versiooni, mida masin oma kirje järgi käivitab. See ei muuda midagi.

    Igal masinal on üks kolmest olekust:
    - converged (konvergeerunud): tema kirje nimetab deklareeritud versiooni. See on tehtud.
    - pending (ootel): tema kirje nimetab vanemat versiooni, või mitte midagi veel.
    - unknown (tundmatu): goatbuilder ei saa tema kirjet lugeda.

    Tehtuks loetakse ainult converged. Kui mingi masin on pending või unknown, lõpetab `status` koodiga 3.

    Alates sellest õppetunnist suunab `--mock` goatbuilderi tegelike masinate asemel harjutuskoopiale sinu tutor kaustas. Mitte midagi, mida sa seal teed, ei jõua pärismasinale.

    Harjutuskoopia `dev`'ile ei ole veel midagi juurutatud. Nii et oota nelja masinat pending olekus ja üht unknown olekus. Converged masinaid näed 3. õppetunnis.
tutor-l2-s1 = Loe dev'i olekut
tutor-l3-title = Juurutus: planeeri, käivita, tõesta
tutor-l3-text =
    Juurutamine paneb deklareeritud versiooni igale masinale, seejärel loeb iga masina kirjet, et tõestada selle kohalejõudmist. Juurutus on tehtud ainult siis, kui iga masina kirje nimetab seda versiooni. Juurutus, mis läks läbi vigadeta, ei piisa sellest.

    Sa juurutad `portal`'i harjutuskoopia `preprod`'ile. `--dry-run` näitab, mis juhtuks, ja ei muuda midagi. `--ack` salvestab, kes selle tegi; käsk täidab sinu nime git'ist. Pärast juurutust peaks `status` näitama mõlemat masinat kui `ok` ja converged.
tutor-l3-s1 = Planeeri juurutus
tutor-l3-s2 = Juuruta portal preprod'ile
tutor-l3-s3 = Kontrolli, et mõlemad masinad on converged
tutor-l4-title = Triiv ja ajalugu
tutor-l4-text =
    Kaks käsku aitavad sul mõista goatnetti enne ja pärast muudatust. Mõlemad vaid loevad.
    - Goatbuilderi `diff` käsk loetleb iga masina, mille kirje erineb deklareeritud versioonist. Erinevust nimetatakse triiviks.
    - Goatbuilderi `history` käsk loetleb iga versiooni, mis komponendil on olnud igal masinal, uuem enne.

    `dev`'il on kõik masinad maha jäänud. `portal`'i ajalugu `preprod`'il sisaldab nüüd sinu juurutust 3. õppetunnist.
tutor-l4-s1 = Loetle triiv dev'il
tutor-l4-s2 = Loe portal'i ajalugu preprod'il
tutor-l5-title = Keri tagasi, ja näe, kuidas goatbuilder keeldub ohtlikust tagasikerimisest
tutor-l5-text =
    Tagasikerimine deklareerib varasema versiooni ja juurutab selle nagu iga teise versiooni. Goatbuilderi CLI loeb siis iga masina kirjet, et tõestada selle kohalejõudmist.

    Mõnda muudatust ei saa sel viisil tagasi võtta. Vanema versiooni juurutamine ei saa tagasi võtta andmeid, mida komponent on kirjutanud, sertifikaati, ega muudatust, mis liigub ainult edasi. Goatbuilderi CLI keeldub neist koodiga 3 ja ütleb, mida teha selle asemel.

    Harjutuskoopia `db-schema` komponent hoiab andmeid, nii et goatbuilder keeldub seda tagasi kerimast. See keeldumine on oodatud tulemus.
tutor-l5-s1 = Planeeri tagasikerimine
tutor-l5-s2 = Keri portal üks versioon tagasi
tutor-l5-s3 = Proovi keerata tagasi db-schema, millest goatbuilder keeldub
tutor-l6-title = Avalda uus versioon: dev, seejärel preprod, seejärel prod
tutor-l6-s1 = Vaata, kus iga komponent on
tutor-l6-s2 = Tee release'i proovikäik devist
tutor-l6-s3 = Tee prod-sammu proovikäik
tutor-l7-title = Fastlane, ja millal seda mitte kasutada
tutor-l7-text =
    Fastlane paneb ühe versiooni korraga igale goatnetile. See jätab vahele ootamise preprod'il ja release'i kirjed.

    Kasuta seda ainult kiireloomulise paranduse jaoks, mille saad tagasi võtta, minnes tagasi eelmisele versioonile. Ära kasuta seda kunagi andmebaasi muudatuse, identiteedi või pääsu muudatuse jaoks, ega midagi, mida sa sel moel tagasi võtta ei saa.

    Fastlane keeldub versioonist, mida CI ei ole ehitanud, ja komponendist, mis on osa veel pooleli olevast release'ist.

    Üldine kuju on `goatbuilder fastlane <component> --ref <version> --reason <why>`. Käivita allolev näide. Sellel on `--dry-run`, nii et midagi ei muutu. See vajab `gh`.
tutor-l7-s1 = Tee fastlane'i proovikäik
tutor-l8-title = Pins: leia triiv ja paranda see õigel moel
tutor-l8-text =
    Üks fail määrab, mis versiooni iga goatnet käivitab: `ops/pins/fleet-pins.yaml`. Goatbuilderi CLI genereerib sellest teised versioonifailid: `pins.yml` failid ja allkirjastatud intent-failid.

    Goatbuilderi `pins --check` käsk võrdleb genereeritud faile failiga `fleet-pins.yaml` ja keeldub igast failist, mis erineb. Goatbuilderi `pins` käsk genereerib need uuesti.

    Sinu tutor kaustas on hoidla koopia, kus keegi on käsitsi muutnud üht `pins.yml` faili. Ära paranda käsitsi tehtud muudatust teise käsitsi tehtud muudatusega. Muuda `fleet-pins.yaml`, siis genereeri uuesti.
tutor-l8-s1 = Kontrolli seda checkouti
tutor-l8-s2 = Kontrolli koopiat ja leia käsitsi tehtud muudatus
tutor-l8-s3 = Genereeri koopia failid uuesti
tutor-l8-s4 = Kontrolli koopiat uuesti
tutor-l9-title = Seleta viga
tutor-l9-s2 = Seleta viga tema koodi järgi
tutor-l9-s3 = Seleta viga tema teksti järgi
tutor-l10-title = Õpetus 1: ehita uue goatneti esimene sait
tutor-l10-text =
    See õppetund ehitab terve goatneti harjutuskoopias. Näidisgoatnet on `zagros`. Selle esimesel saidil, `alpha`, on kaks tuummasinat, üks jälgimismasin ja üks päästemasin.

    Goatbuilderi `standup` käsk seadistab iga masina kindlas järjekorras ja kontrollib igat kirjet. Selle viimane rida ütleb, et sait on valmis. Päris masinatel näitab see rida ka, kui kaua see aega võttis.

    Täielik õpetus on `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Ehita esimene sait, alpha
tutor-l10-s2 = Kontrolli tulemust
tutor-l11-title = Õpetus 2: lisa teine sait
tutor-l11-text =
    See õppetund algab sealt, kus õpetus 1 lõppes. See lisab teise saidi, `beta`, uues harjutuskoopias, `zagros-2`.

    Teine sait kopeerib oma kasutajakontod esimeselt saidilt. Selle standup muudab ka esimest saiti ühe korra, et kopeerimine töötaks mõlemas suunas. Selle teeb samm nimega `alpha/core`.

    Täielik õpetus on `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Ehita teine sait, beta
tutor-l11-s2 = Kontrolli mõlemat saiti
tutor-l12-s1 = Planeeri ühe masina lisamist
tutor-l13-s1 = Planeeri föderatsioon
tutor-l14-title = Hoia goatbuilder ajakohasena
tutor-l14-s1 = Küsi, kas uuem goatbuilder on olemas
tutor-l15-title = Assay: kontrolli, kuidas goatnet on ehitatud
tutor-l15-text =
    Goatbuilderi `assay` käsk kontrollib, et goatnet on ehitatud õigesti, üks kiht korraga, kasutades vastuvõtuteste. Igal testil on üks kolmest tulemusest:
    - PASS: test läbis.
    - FAIL: goatnetil on probleem.
    - UNKNOWN: mitte miski ei suutnud testile vastata.

    Loetakse ainult PASS. Mis tahes FAIL või UNKNOWN teeb väljumiskoodiks 3.

    `--plan` loetleb testid, mis kehtivad goatnetile, kihtide kaupa, ja ei puutu ühtegi masinat. Kasuta seda, et näha, mida päris assay katab. Üldine kuju on `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: on versioonid kooskõlas?
tutor-l16-text =
    Goatbuilderi `survey` käsk kontrollib ühe läbimisega, kas samad versioonid töötavad seal, kus peaks:
    - Igas goatnetis: kas iga masin käivitab deklareeritud versiooni, ja kas masinad, mis peaksid ühtima, tegelikult ühtivad?
    - Üle dev, preprod ja prod: üks rida komponendi kohta, üks veerg goatneti kohta.

    Erinevus, mida release'i protsess ootab, raporteeritakse, aga see ei ole viga. Näide on uus versioon, mis on dev'il, aga ei ole veel prod'il. Erinevust, mida mitte midagi ei deklareeri, loetakse kui `difference not declared`. Masin, mida goatbuilder ei saa lugeda, on unknown, mitte kunagi kooskõlas.

    Käivita see harjutuskoopial. Selle `dev` on meelega maha jäänud, nii et oota leide ja väljumiskoodi 3.
tutor-l16-s1 = Survey harjutuskoopiat
tutor-l17-title = Endstate: kas goatnet käivitab täpselt seda, mida ta deklareerib?
tutor-l17-text =
    Goatbuilderi `endstate` käsk kontrollib goatneti kohta kahte asja:
    1. Iga teenus, mida see deklareerib, on juurutatud, töötab, on terve ja õigesti ühendatud.
    2. Midagi ei tööta, mida see ei deklareeri. Deklareerimata teenust nimetatakse orvuteenuseks.

    Goatbuilderi CLI arvutab välja, mida sait peaks käivitama, saidi oma sätetest, mitte eraldi nimekirjast.

    Kontroll, millel ei ole midagi lugeda, märgitakse UNOBSERVED. See on aus vastus, mitte ebaõnnestumine. FAIL või orvuteenus teeb väljumiskoodiks 3.

    Üldine kuju on `goatbuilder endstate <goatnet>`. Allolev käsk kontrollib näidissaiti salvestatud andmetest, nii et see ei puutu ühtegi masinat.
tutor-l17-s1 = Kontrolli näidissaiti
tutor-l18-title = Boonus: ole uudishimulik
tutor-l18-text =
    See õppetund on valikuline. Käivita allolev käsk ja tutor on lõpetatud.

    Goatbuilderi CLI-l on paar väikest peidetud funktsiooni. Neid ei ole abis ega nendes õppetundides. Mõni vastab sõnadele, mida kauaaegsed Unix kasutajad sageli uude programmi sisestavad, lihtsalt et näha, mis juhtub. Enamik, mida sa proovid, ei tee midagi.

    See õppetund ei ütle, mis need on. Nende leidmine on mõte.

    Allolev käsk loetleb iga vea, mida goatbuilder saab seletada. See on hea koht, kust avastamist alustada.
tutor-l18-s1 = Loetle iga vea, mida goatbuilder saab seletada
tutor-heading = Õppetund { $n } / { $total } · { $title }
tutor-setup = Käivita see kõigepealt üks kord selles terminalis:
tutor-not-recording = See terminal ei salvesta sinu käike, nii et tutor ei saa sinu samme linnukesega märkida. Käivita see üks kord ja käivita siis sammud:
tutor-type = Tee need sammud oma terminalis ({ $met } / { $total } tehtud):
tutor-rerun = Pärast igat sammu käivita `goatbuilder tutor` uuesti. See kontrollib, mida tegid, ja märgib sammu linnukesega.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Tere tulemast GOATBUILDER Tutor
tutor-bar-progress = Edenemine
tutor-bar-done = { $done } / { $total } õppetundi tehtud
tutor-after-step = Samm { $met } / { $total } tehtud. Järgmine:
tutor-after-lesson = Õppetund { $n } on valmis. Käivita `goatbuilder tutor`, et alustada õppetundi { $next }. Et lugeda seda uuesti: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Õppetund { $n } on valmis, ja tutor on ka. Käivita `goatbuilder tutor`, et näha, et see on märgitud tehtud. Et lugeda seda uuesti: `goatbuilder tutor --lesson { $n }`.

tutor-review = Sa lõpetasid selle õppetunni, nii et see leht on ainult lugemiseks. Käivita `goatbuilder tutor`, et minna tagasi sinna, kus olid.
tutor-other-lessons = Et lugeda õppetundi uuesti: `goatbuilder tutor --lesson 1`. Et näha neid kõiki: `goatbuilder tutor --list`.
tutor-l12-title = Lisa masin töötavale saidile
tutor-l12-text =
    Goatbuilderi `join-node` käsk lisab saidile, mis juba töötab, veel ühe masina. Ta prindib esmalt plaani, ühe rea iga faasi kohta. Igal faasil on tüüp:
    - tool: goatbuilder käivitab skripti.
    - ansible: goatbuilder käivitab playbooki.
    - gate: kontroll, mis peab enne järgmist faasi õnnestuma.
    - manual: sa tee seda ise, ja käivitus peatub.

    Allpool olevas käsus on `--dry-run`. See loeb seda checkouti ja ei muuda midagi. Tabeli all loe, mida sa teed käsitsi.
tutor-l13-title = Ühenda teise organisatsiooni goatnetiga
tutor-l13-text =
    Goatbuilderi `federate` käsk ühendab sinu goatneti goatnetiga, mida käitab teine organisatsioon. Kumbki pool koostab oma poole lepingust. Te vahetate pooled, ja kumbki pool liidab need samaks lepinguks.

    Kinnita teise poole võti eraldi viisil, näiteks kõnega, mida sa usaldad. Võti, mis saabub nende paketi sees, ei tõesta midagi.

    Allpool olevas käsus on `--dry-run`, nii et see ei muuda midagi. Päris käivitus peatub faasis 4, kuna selle faasi tee käsitsi.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = See käsk ei ole õppetunni { $n } osa. Käivita `goatbuilder tutor`, et näha, kus oled.
tutor-l1-s2 = Küsi doctorilt ühe goatneti kohta
tutor-l1-text =
    Goatbuilderi CLI ehitab, juurutab ja avaldab GOAT tarkvara. Enne kasutamist kontrolli oma seadistust kahe käsuga:
    - Goatbuilderi `validate` käsk kontrollib konfiguratsioonifaili, `ops/goatbuilder/goatbuilder.yaml`. See loeb ainult seda checkouti ja ei vaja võrku.
    - Goatbuilderi `doctor` käsk kontrollib, mida päris juurutus vajab: sinu tööriistu, sinu sisselogimisi, kas see checkout ühtib main'iga, ja kuidas goatbuilder jõuab igale goatnetile. Goatnet on üks GOAT keskkond.

    Goatbuilderi `doctor` käsk prindib, mitu kontrolli läbis, ja seejärel iga probleemi koos lahendusega. Selles õppetunnis on probleem lihtsalt info, mitte viga. Loe lahendused läbi, aga sa ei pea neid praegu rakendama.

    Enamik õppetunde ei vaja sisselogimist. Õppetunnid 2 kuni 5, 10, 11 ja 16 kasutavad harjutuskoopiat. Õppetunnid 8, 9, 12, 13, 15, 17 ja 18 loevad vaid seda checkouti. Õppetund 6 vajab `gh` ja mesh'i, õppetund 7 vajab `gh`, ja õppetund 14 vajab mesh'i. Ükski õppetund ei vaja 1Passwordi.
tutor-l14-text =
    Goatbuilderi CLI saab end uuendada koopiast, mida sinu goatnet jagab aadressil `get.<site>.<zone>/goatbuilder/`.

    Goatbuilderi `update --check` käsk ainult loeb. See võrdleb sinu versiooni jagatava versiooniga ja ei muuda midagi. Väljumiskood 0 tähendab, et oled ajakohane. Väljumiskood 3 tähendab, et uuem versioon on olemas.

    Pärisuuendus, ilma `--check`, kontrollib allalaadimise kontrollsummat ja signatuuri, enne kui midagi asendab. Seejärel vahetab see faili ühe sammuga.

    Goatbuilderi CLI peab teadma, millelt goatnetilt küsida: anna `--site` selle goatnetiga, või määra `GOATBUILDER_UPDATE_URL`. Sa pead olema sellel goatnetil mesh'is. Kui sa ei ole, ütleb `--check`, et see ei suuda serverit kätte saada, ja nimetab aadressi, mida ta proovis. See on selle õppetunni jaoks õige tulemus.
tutor-l15-s1 = Loetle testid, mis kehtivad ühele goatnetile
tutor-l6-text =
    Release liigutab uue versiooni läbi kolme goatneti, järjekorras: dev, seejärel preprod, seejärel prod.

    1. Keegi märgistab versiooni main'il ja CI ehitab selle.
    2. dev: goatbuilder juurutab selle ja käivitab vastuvõtutestid.
    3. preprod: goatbuilder juurutab selle uuesti, käivitab testid uuesti ja salvestab tulemused.
    4. Versioon ootab seejärel preprod'il kindla aja. Midagi ei liigu prod'i omal käel.
    5. prod: pärast seda ootamist käivitab inimene release'i käsuga `--from prod`. Goatbuilderi CLI juurutab prod'i ja kontrollib kõiki kolme goatnetti.

    Goatbuilderi `promote status` käsk näitab, kus iga komponent on. Kandidaat on versioon, mida testitakse. Release'i üldine kuju on `goatbuilder release <component> --tag <tag> --from dev`. Allolevad käsud kasutavad ühte selle checkouti komponenti ja näidismärgist. Igaühel on `--dry-run`, nii et midagi ei märgistata ega juurutata.

    Proovikäigud vajavad `gh` ja mesh'i. Kui sa ei ole hoidla admin, pane `GOAT_RC_ROLE_CHECK=0` enne igat `release` käsku.
tutor-l9-s1 = Tekita pärisviga goatnetiga, mida ei ole olemas
tutor-l9-text =
    Kui goatbuilder keeldub, trükib see koodi. Koodid, mis algavad `GB-`, tulevad goatbuilderilt, `E_` tuleb goat CLI-lt, ja `X-` tuleb teiselt tööriistalt, mida goatbuilder käivitas.

    Goatbuilderi `explain` käsk võtab koodi, või veareal oleva teksti, ja ütleb, mis juhtus, miks, kuidas seda parandada, ja kust lugeda rohkem.

    Kui küsid goatnetti, mida see checkout ei deklareeri, tekib pärisviga, mida seletada.
