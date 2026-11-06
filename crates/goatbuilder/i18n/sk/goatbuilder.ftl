### goatbuilder — sk (sk)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Zostavte a nasaďte prostredie goat: deklarujte, konvergujte, overte — pod jednou nasadzovacou zmluvou
help-flag-output = Formát výstupu: tabuľka pre človeka; JSON alebo YAML pre program
help-flag-repo = Koreň repozitára. Predvolene checkout, v ktorom beží tento binárny súbor
help-flag-yes = Potvrdí deklaratívny príkaz bez výzvy (pre automatizáciu)
help-flag-frozen = Odmietne každý beh, ktorý by musel posunúť checkout dopredu (fast-forward; prísne C3)
help-flag-mock = Beh proti adresáru s fixtúrami namiesto hostiteľov: bez registra, bez transportu, deterministicky (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Jazyk nápovedy a vysvetlení chýb (značka ako fr, uk, de); výstup, ktorý číta pipeline, ostáva anglický. Predvolene: jazyk OS
help-cmd-validate = Skontroluje konfiguráciu voči verzovanej schéme a krížovým pravidlám polí, offline
help-cmd-doctor = Preverí, čo beh potrebuje, skôr než sa čokoľvek spustí: závislosti, aktuálnosť checkoutu, transport každého prostredia a že playbook každého komponentu vedie na jednoznačný cieľový režim
help-cmd-status = Deklarované oproti zaznamenanému, po jednotlivých strojoch: konvergované, čakajúce alebo neznáme (C7, C8). Len číta. Kód 3, kým nie je každý stroj konvergovaný
help-cmd-deploy = Nasadí pin, ktorý deklaruje main, na jedno prostredie a dokáže to: z pinov určí komponent a skupiny prostredia, play a jeho cieľový režim, každé tajomstvo, ktoré play šablónuje, a transport; spustí play cez ansible-preflight; prečíta deploy ledger z každého hostiteľa a hotovo vyhlási, až keď najnovší riadok uvádza deklarovaný digest s action=deploy (C7). Na fixtúre (`--mock`) hostiteľov nahrádza skriptovaný konvergátor
help-cmd-release = Vedie release candidate od tagu na main cez živú validáciu dev goatnetu až na stupeň preprod (ADR 1173); po soak fáze s `--from prod` ho povýši, zapracuje do fleet:, nasadí prod a znova overí každý stupeň. Nahrádza ops/scripts/rc.sh krok za krokom
help-cmd-rollback = Vráti nasadený komponent na jeho N-tý najnovší ref podľa typu artefaktu, pričom zdrojom pravdy je ledger na hostiteľovi (ADR 1060). Nahrádza ops/scripts/rollback.sh: kód 3 odmietnuté ako nebezpečné (data, cert, forward-only, reversible:false), 4 riadené (config, acl)
help-cmd-fastlane = Jeden príkaz od zlúčeného commitu po každý goatnet (ADR 1127): zámerná výnimka z rebríčka. Nahrádza ops/scripts/fastlane.sh krok za krokom; odmietne nepinnutý digest alebo otvorenú množinu povýšení, a nasadenia spúšťa cez tento binárny súbor
help-cmd-standup = Lokalita od nuly po overenie: fázy standupu v poradí podľa doktríny s fail-closed bránou na oboch koncoch, observability hostiteľ ako paralelná vetva, rescue triedy a medzi-goatnetové povrchy nakoniec (ADR 1183). Nahrádza ops/scripts/goat-standup.sh krok za krokom. Na fixtúre (`--mock`) skriptovaný konvergátor konvergne každý komponent každej triedy a čas od deklarácie po overenie je mierou jednohodinového goatnetu
help-cmd-join-node = Ďalší stroj danej triedy na živej lokalite, deklarovaný → konvergovaný → overený: vykonaná postupnosť z docs/tutorials/02-first-federated-node.md ako číslované fázy s bránami (dosiahnuteľnosť, kľúč, najprv práca s repozitárom, baseline, kernel WireGuard pred enrolmentom aj po ňom, kernel ACL rovina ako brána enrolmentu, konvergencia triedy, akceptácia)
help-cmd-federate = Suverénny peering deklarovaný, konvergovaný a overený (ADR 1157): zmeraná naša polovica, balíky vymenené mimo pásma, zložená a overená jedna dohoda, z nej vyrazené leaves, pravidlá vyrenderované jediným zapisovateľom, nasadený edge, otvorený transport, ktorý dohoda zvolila, akceptácia vo všetkých štyroch úrovniach. Nahrádza ops/scripts/peering-standup.sh krok za krokom
help-cmd-pins = JEDEN prechod rozlíšenia s DVOMA výstupmi: pins.yml v group_vars každého uvedeného člena a IntentV1 artefakt každého uvedeného goatnetu, z ops/pins/fleet-pins.yaml. Nahrádza ops/pins/generate-pins.py bajt za bajt. `--check` validuje, pregeneruje a odmietne drift, nepodpísaný alebo pozmenený intent a riadený pin definovaný mimo pins.yml (kód 1)
help-cmd-promote = Vedie a sleduje povyšovanie release naprieč rebríčkom goatnetov (ADR 1120, 1125): stav, otvorenie kandidáta, povýšenie, povýšenie množiny, hotfix cesta, review, pripravenosť, zrušenie, kontrola. Nahrádza ops/release/graduate.py krok za krokom. Nič tu nepovyšuje samo: zmena sa vykoná, len keď ju vyvoláte, vypíše, čo urobila, a bez terminálu žiada --yes (C10, ADR 1177)
help-cmd-explain = Vysvetlí chybu: kód (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) alebo ľubovoľný text chyby — vlastný tohto binárneho súboru alebo slová nástroja, ktoré niesol (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Vypíše, čo sa stalo, prečo, čo robiť a kde čítať viac; `-` číta text zo stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Lokalita od overenej po nič — zrkadlo standupu, záznamy ostávajú (burn-down pridáva riadky teardownu, nikdy nič nemaže). Na fixture (`--mock`) nacvičí burn-down v opačnom poradí fáz; na reálnych hostoch vypíše poradie a odmietne vykonať, kým každá fáza nedeklaruje svoj teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekcie v štýle vimtutor: každá pomenuje presný príkaz, ktorý spustíš na jednorazovej kópii fixture (alebo, len na čítanie, na tomto checkoute), a potom skontroluje, čo príkaz naozaj urobil — jeho exit a riadky — kým sa odomkne ďalšia. `tutor` ukáže aktuálnu lekciu a označí ju za hotovú, keď ju tvoje behy splnia; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skryté (src/fun.rs)
help-cmd-argan = Skryté (src/fun.rs)
help-cmd-bound = Skryté (crates/goat-fun)
help-cmd-secrets = Deklarovaný postoj k secretom (ADR 1204): ktorý store platí (1password, openbao, sops, git-crypt, env-file, env), čo je dostupné, ako doplniť chýbajúce a vlastné kontroly storu. `init --env <goatnet>` zapíše env súbor pre zero-SaaS quickstart: silnú náhodnú hodnotu pre každú premennú, ktorú plays goatnetu šablónujú, 0600, v gitignore, nikdy nerotuje hodnotu, ktorá tam už je. Ten istý modul ako ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Prečíta najnovší riadok deploy-ledgeru každého hostiteľa pre jeden komponent a povie, či uvádza ref, ktorý deklaruje main (C7). Iba číta; exit 3, kým je niektorý hostiteľ pending alebo unknown. Rovnaká kontrola ako `deploy --verify-only`
help-cmd-diff = Čo sa teraz líši od main (C8): deklarovaný ref z main oproti najnovšiemu riadku ledgeru každého hostiteľa, pre jeden komponent alebo – bez neho – pre každý komponent prostredia, čítané jedným prechodom na skupinu členov. Iba číta; exit 3 pri akomkoľvek drifte alebo nečitateľnom hostiteľovi
help-cmd-history = Deploy-ledger ako časová os, od najnovšieho, na každom hostiteľovi prostredia (C8): každý riadok je samostatný záznam so strojom, časom, akciou (deploy alebo rollback) a verziou. Predchádzajúca verzia záznamu a operátor, ktorý ho spustil, sú v `-o json`. Iba číta
help-cmd-plan = Zapíše do plan súboru na revíziu, čo by urobilo deklaratívne sloveso: deklarované refy, play a cieľ, ledger, commit main, transport (deploy); typ artefaktu a cieľový ref (rollback). Nemení stav žiadneho hostiteľa. Spustite ho tak, ako bol skontrolovaný, cez `apply`
help-cmd-apply = Spustí presne skontrolovaný plan. Odmietne (exit 1, s uvedením, čo sa zmenilo), ak sa od zápisu plánu zmenila deklarácia main, commit pins, play alebo cieľ rollbacku; zmenený transport sa nahlási, neodmietne
help-cmd-adopt = Deklaruje, čo prostredie už beží: prečíta ledgery každého hostiteľa a navrhne odchýlku v pins pre každý komponent, ktorého bežiaci ref sa líši od deklarácie main. Predvolene len report; s --yes sa úpravy zapíšu do čerstvého worktree main na kontrolu. Nikdy nenasadzuje

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ešte nie: { $why }
tutor-lesson-done = Lekcia { $n } hotová: { $title }.
tutor-all-done-reset = Všetky lekcie sú hotové. `goatbuilder tutor --reset` začne odznova; tutoriály idú ďalej: docs/tutorials/.
tutor-all-done = Všetky lekcie sú hotové. Tutoriály idú ďalej: docs/tutorials/.
tutor-workspace = Pracovný priestor: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Overí vrstvu po vrstve, či bol goatnet postavený správne: každá akceptačná kontrola, ktorá sa vzťahuje na lokalitu, vrátane kontrol, ktoré operátor spúšťa cez goat CLI. Akceptačná fáza standupu ako samostatný príkaz. Iba číta; výstup 3, kým niektorá kontrola zlyháva alebo nemôže odpovedať
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Izolovaná lokalita: nesťahovať trunk; checkout posúdiť voči kešovanému a beh zaznamenať ako offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Skontroluje, či je goatnet vo svojom deklarovanom cieľovom stave: každá služba, o ktorej vlastné deklarácie tvrdia, že beží, je nasadená, spustená, zdravá a plní svoju úlohu, jej zero-trust hrany sú zapojené a nič nebeží, čo nie je deklarované. Len číta; kým je niečo mimo, skončí s kódom 3
help-cmd-survey = Aká konzistentný je softvér v rámci každého goatnetu a naprieč goatnetmi rebríka, zisťované jedným paralelným prechodom. Iba číta

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Aký konzistentný je softvér v každom goatnete a naprieč goatnetmi rebríka, zistené jedným paralelným prechodom. Iba čítanie.
help-cmd-update = Nahradí tento binárny súbor vydaním, ktoré goatnet poskytuje na `get.<site>.<zone>/goatbuilder/`: stiahnutý súbor sa overí voči jeho `.sha256` a podpísanému `SHA256SUMS` skôr, než sa čokoľvek zmení, a výmena je atomická. Už aktuálne: jeden riadok, návratový kód 0. `--check` iba číta (návratový kód 3, ak zaostáva). Zdroj: `GOATBUILDER_UPDATE_URL`, inak `--site` z jeho group_vars, inak ten z poslednej aktualizácie

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Skôr než začnete: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Zafarbí glyf ok/FAIL a verdikt v tabuľke (auto|always|never). `auto` farbí iba terminál; NO_COLOR a CLICOLOR_FORCE majú prednosť. JSON a YAML sa nikdy nefarbia
help-flag-verbose = Zapisuje na stderr, čo beh robí — externé príkazy, ktoré spúšťa, a sieťovú fázu `update` — takže `-o json|yaml` zostane čistý dokument. Alebo nastavte RUST_LOG=goatbuilder=debug
help-flag-timeout = Počet sekúnd na obmedzenie fázy pripojenia/dosiahnutia pred rýchlym zlyhaním — jedinej fázy, ktorá sa môže zaseknúť na nedostupnom hostiteľovi (`update` a získanie verzie z servera). NEobmedzuje samotný beh playu ani sťahovanie binárky (tá je ohraničená zaseknutím). Číta aj GOATBUILDER_TIMEOUT
help-cmd-completion = Vypíše skript dopĺňania pre shell na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Pre živé kandidáty, ktoré statický skript nemôže poznať — lokality, komponenty a goatnety, ktoré tento checkout deklaruje — napojte hook na `goatbuilder _complete` (pozrite hlavičku vygenerovaného skriptu)
help-cmd-docs = Referencia príkazov: `goatbuilder docs` na ňu odkazuje; `--markdown` vypíše celú, zostavenú prechodom živého stromu príkazov, takže sa nikdy nerozíde s `--help`
help-cmd-_complete = Skryté: vypíše živých kandidátov dopĺňania pre hooky shellu, jedného na riadok (lokality, komponenty a goatnety, ktoré tento checkout deklaruje). Iba číta, nikdy hlučne nezlyhá — hook dopĺňania nesmie pokaziť prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Pred nasadením: výpis pripravenosti a vratnosti (ADR 1211) — tri fakty: State (stateless | stateful), Rollback (pin-revert | restore-required | none) a Scope (single | coupled) — deklarované pre každý komponent a FAIL-CLOSED, ak chýbajú; k tomu časť o pripravenosti, ktorú dokazujú ostatné príkazy (ref sa dá stiahnuť, pins --check, brána tajomstiev, čerstvý zdroj, deklarovaný koncový stav) a overenie faktu Rollback (rollback --dry-run pre pin-revert, retenčné okno pre restore-required). `deploy` ho spúšťa ako prvý; hodnotené podľa pravdivosti, nikdy falošná zelená. Len číta; exit 3, ak nie je čisté

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Skontroluj svoje nastavenie
tutor-l1-s1 = Skontroluj konfiguračný súbor
tutor-l2-title = Čítanie stavu: converged, pending, unknown
tutor-l2-text =
    Príkaz goatbuildera `status` porovná pre každý stroj dve veci: verziu, ktorú pre neho deklaruje main, a verziu, o ktorej jeho vlastný záznam hovorí, že beží. Nič nemení.

    Každý stroj je v jednom z troch stavov:
    - converged: jeho záznam uvádza deklarovanú verziu. To je hotové.
    - pending: jeho záznam uvádza staršiu verziu, alebo ešte nič.
    - unknown: goatbuilder nevie jeho záznam prečítať.

    Za hotové sa počíta len converged. Ak je niektorý stroj pending alebo unknown, `status` skončí s kódom 3.

    Od tejto lekcie `--mock` ukáže goatbuilderu na cvičnú kópiu v tvojom priečinku tutora namiesto na skutočné stroje. Nič, čo tam urobíš, sa neprejaví na skutočnom stroji.

    Na `dev` cvičnej kópie ešte nič nasadené nebolo. Čakaj preto štyri stroje pending a jeden unknown. Stroje converged uvidíš v lekcii 3.
tutor-l2-s1 = Prečítaj stav dev
tutor-l3-title = Nasadenie: naplánuj ho, spusti ho, dokáž to
tutor-l3-text =
    Nasadzovanie umiestni deklarovanú verziu na každý stroj a potom prečíta záznam každého stroja, aby to dokázalo. Nasadenie je hotové len tak, keď záznam každého stroja uvádza túto verziu. Nasadenie, ktoré prebehlo bez chýb, nestačí.

    `portal` nasadíš do `preprod` cvičnej kópie. `--dry-run` ukáže, čo by sa stalo, a nič nemení. `--ack` zapíše, kto to urobil; príkaz tvoje meno doplní z gitu. Po nasadení by `status` mal ukázať oba stroje ako `ok` a converged.
tutor-l3-s1 = Naplánuj nasadenie
tutor-l3-s2 = Nasaď portal na preprod
tutor-l3-s3 = Skontroluj, že sú oba stroje converged
tutor-l4-title = Drift a história
tutor-l4-text =
    Dva príkazy ti pomôžu pochopiť goatnet pred zmenou a po nej. Oba len čítajú.
    - Príkaz goatbuildera `diff` vypíše každý stroj, ktorého záznam sa líši od deklarovanej verzie. Taký rozdiel sa nazýva drift.
    - Príkaz goatbuildera `history` vypíše každú verziu, ktorú komponent mal na každom stroji, od najnovšej.

    Na `dev` je každý stroj pozadu. História `portal` na `preprod` teraz obsahuje tvoje nasadenie z lekcie 3.
tutor-l4-s1 = Vypíš drift na dev
tutor-l4-s2 = Prečítaj históriu portal na preprod
tutor-l5-title = Vráť verziu späť a uvidíš, ako je nebezpečné vrátenie odmietnuté
tutor-l5-text =
    Vrátenie deklaruje staršiu verziu a nasadí ju ako každú inú verziu. CLI goatbuildera potom prečíta záznam každého stroja, aby to dokázal.

    Niektoré zmeny sa takto vrátiť nedajú. Nasadenie staršej verzie nemôže vrátiť dáta, ktoré komponent zapísal, certifikát, ani zmenu, ktorá ide len vpred. CLI goatbuildera takéto vrátenia odmietne s kódom 3 a povie, čo urobiť namiesto toho.

    Komponent `db-schema` cvičnej kópie obsahuje dáta, takže ho goatbuilder odmietne vrátiť späť. Toto odmietnutie je očakávaný výsledok.
tutor-l5-s1 = Naplánuj vrátenie
tutor-l5-s2 = Vráť portal o jednu verziu späť
tutor-l5-s3 = Skús vrátiť späť db-schema, čo goatbuilder odmietne
tutor-l6-title = Vydanie novej verzie: dev, potom preprod, potom prod
tutor-l6-s1 = Zisti, kde je každý komponent
tutor-l6-s2 = Spusti suchý beh vydania z dev
tutor-l6-s3 = Spusti suchý beh kroku prod
tutor-l7-title = Fastlane, a kedy ho nepoužívať
tutor-l7-text =
    Fastlane umiestni jednu verziu na všetky goatnety naraz. Vynechá čakanie na preprode a záznamy vydania.

    Používaj ho len na naliehavú opravu, ktorú vieš vrátiť návratom k predchádzajúcej verzii. Nikdy ho nepoužívaj na zmenu databázy, zmenu identity alebo prístupu, ani na nič, čo sa takto vrátiť nedá.

    Fastlane odmietne verziu, ktorú CI nezostavilo, a komponent, ktorý je súčasťou vydania, ktoré ešte prebieha.

    Všeobecná forma je `goatbuilder fastlane <component> --ref <version> --reason <why>`. Spusti príklad nižšie. Má `--dry-run`, takže sa nič nemení. Potrebuje `gh`.
tutor-l7-s1 = Spusti suchý beh fastlane
tutor-l8-title = Piny: nájdi drift a oprav ho správne
tutor-l8-text =
    Jeden súbor určuje, ktorú verziu každý goatnet prevádzkuje: `ops/pins/fleet-pins.yaml`. CLI goatbuildera z neho generuje ostatné súbory verzií: súbory `pins.yml` a podpísané súbory intent.

    Príkaz goatbuildera `pins --check` porovná vygenerované súbory s `fleet-pins.yaml` a odmietne každý súbor, ktorý sa líši. Príkaz goatbuildera `pins` ich znova vygeneruje.

    Tvoj priečinok tutora má kópiu repozitára, v ktorej niekto ručne upravil jeden súbor `pins.yml`. Ručnú úpravu nikdy neopravuj ďalšou ručnou úpravou. Zmeň `fleet-pins.yaml`, a potom znova vygeneruj.
tutor-l8-s1 = Skontroluj tento checkout
tutor-l8-s2 = Skontroluj kópiu a nájdi ručnú úpravu
tutor-l8-s3 = Znova vygeneruj súbory kópie
tutor-l8-s4 = Skontroluj kópiu znova
tutor-l9-title = Vysvetlenie chyby
tutor-l9-s2 = Vysvetli chybu podľa jej kódu
tutor-l9-s3 = Vysvetli ju podľa jej textu
tutor-l10-title = Tutoriál 1: postav prvú site nového goatnetu
tutor-l10-text =
    Táto lekcia postaví celý goatnet v cvičnej kópii. Ukážkový goatnet je `zagros`. Jeho prvá site, `alpha`, má dva core stroje, jeden monitorovací stroj a jeden záchranný stroj.

    Príkaz goatbuildera `standup` nastaví každý stroj v pevne danom poradí a skontroluje každý záznam. Jeho posledný riadok povie, že site je hotová. Na skutočných strojoch ten riadok tiež ukáže, ako dlho to trvalo.

    Celý tutoriál je `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Postav prvú site, alpha
tutor-l10-s2 = Skontroluj výsledok
tutor-l11-title = Tutoriál 2: pridaj druhú site
tutor-l11-text =
    Táto lekcia začína tam, kde skončil tutoriál 1. Pridá druhú site, `beta`, v novej cvičnej kópii, `zagros-2`.

    Druhá site si skopíruje používateľské účty z prvej site. Jej standup tiež raz zmení prvú site, aby kopírovanie fungovalo oboma smermi. Robí to krok s názvom `alpha/core`.

    Celý tutoriál je `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Postav druhú site, beta
tutor-l11-s2 = Skontroluj obe site
tutor-l12-s1 = Naplánuj pridanie jedného stroja
tutor-l13-s1 = Naplánuj federáciu
tutor-l14-title = Udržuj goatbuilder aktuálny
tutor-l14-s1 = Spýtaj sa, či existuje novší goatbuilder
tutor-l15-title = Assay: skontroluj, ako bol goatnet postavený
tutor-l15-text =
    Príkaz goatbuildera `assay` skontroluje, že bol goatnet postavený správne, vrstvu po vrstve, pomocou akceptačných testov. Každý test dostane jeden z troch výsledkov:
    - PASS: test prešiel.
    - FAIL: goatnet má problém.
    - UNKNOWN: na test nešlo odpovedať.

    Započíta sa len PASS. Akýkoľvek FAIL alebo UNKNOWN spôsobí kód 3.

    `--plan` vypíše testy, ktoré sa na goatnet vzťahujú, podľa vrstvy, a nedotkne sa žiadneho stroja. Použi ho, aby si videl, čo by skutočný assay pokryl. Všeobecná forma je `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: sú verzie zhodné?
tutor-l16-text =
    Príkaz goatbuildera `survey` v jednom priechode skontroluje, či rovnaké verzie bežia tam, kde by mali:
    - Vnútri každého goatnetu: beží na každom stroji deklarovaná verzia, a zhodujú sa stroje, ktoré by sa zhodovať mali?
    - Medzi dev, preprod a prod: jeden riadok na komponent, jeden stĺpec na goatnet.

    Rozdiel, ktorý proces vydania očakáva, sa nahlási, ale nie je to chyba. Príkladom je nová verzia, ktorá je na dev, ale ešte nie na prod. Rozdiel, ktorý nič nedeklaruje, sa číta ako `difference not declared`. Stroj, ktorý goatbuilder nevie prečítať, je unknown, nikdy consistent.

    Spusti ho na cvičnej kópii. Jej `dev` je pozadu úmyselne, čakaj preto nálezy a kód 3.
tutor-l16-s1 = Spusti survey na cvičnej kópii
tutor-l17-title = Endstate: prevádzkuje goatnet presne to, čo deklaruje?
tutor-l17-text =
    Príkaz goatbuildera `endstate` skontroluje pri goatnete dve veci:
    1. Každá služba, ktorú deklaruje, je nasadená, beží, je zdravá a správne pripojená.
    2. Nič nebeží, čo nedeklaruje. Nedeklarovaná služba sa nazýva sirota.

    CLI goatbuildera odvodí, čo by site mala prevádzkovať, z vlastného nastavenia site, nie z oddeleného zoznamu.

    Kontrola, ktorá nemá čo čítať, sa označí UNOBSERVED. To je čestná odpoveď, nie neúspech. FAIL alebo sirota spôsobia kód 3.

    Všeobecná forma je `goatbuilder endstate <goatnet>`. Príkaz nižšie skontroluje ukážkovú site z uložených dát, takže sa nedotkne žiadneho stroja.
tutor-l17-s1 = Skontroluj ukážkovú site
tutor-l18-title = Bonus: buď zvedavý
tutor-l18-text =
    Táto lekcia je nepovinná. Spusti príkaz nižšie a tutor skončí.

    CLI goatbuildera má zopár malých skrytých funkcií. Nie sú v nápovede ani v týchto lekciách. Niektoré reagujú na slová, ktoré dlhoroční používatelia Unixu často skúšajú napísať do nového programu, len aby videli, čo sa stane. Väčšina vecí, ktoré skúsiš, nič neurobí.

    Táto lekcia nehovorí, čo to je. Zmyslom je nájsť ich.

    Príkaz nižšie vypíše každú chybu, ktorú goatbuilder vie vysvetliť. Je to dobré miesto, kde začať skúmať.
tutor-l18-s1 = Vypíš každú chybu, ktorú goatbuilder vie vysvetliť
tutor-heading = Lekcia { $n } z { $total } · { $title }
tutor-setup = Najprv to raz spusti v tomto terminále:
tutor-not-recording = Tento terminál nezaznamenáva tvoje behy, takže tutor nemôže zaškrtávať tvoje kroky. Spusti to raz, a potom spusti kroky:
tutor-type = Urob tieto kroky vo svojom terminále ({ $met } z { $total } hotovo):
tutor-rerun = Po každom kroku spusti `goatbuilder tutor` znova. Skontroluje, čo si urobil, a zaškrtne krok.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Vitaj v GOATBUILDER Tutor
tutor-bar-progress = Priebeh
tutor-bar-done = { $done } z { $total } lekcií hotovo
tutor-after-step = Krok { $met } z { $total } hotový. Ďalej:
tutor-after-lesson = Lekcia { $n } je hotová. Spusti `goatbuilder tutor`, aby si začal lekciu { $next }. Ak ju chceš znova prečítať: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekcia { $n } je hotová, a tutor tiež. Spusti `goatbuilder tutor`, aby si videl, že je označený ako hotový. Ak ju chceš znova prečítať: `goatbuilder tutor --lesson { $n }`.

tutor-review = Táto lekcia je dokončená, takže táto stránka je len na čítanie. Spusti `goatbuilder tutor`, aby si sa vrátil tam, kde si.
tutor-other-lessons = Ak chceš lekciu znova prečítať: `goatbuilder tutor --lesson 1`. Ak ich chceš vidieť všetky: `goatbuilder tutor --list`.
tutor-l12-title = Pridaj stroj na bežiacu site
tutor-l12-text =
    Príkaz goatbuildera `join-node` pridá ešte jeden stroj na site, ktorá už beží. Najprv vypíše plán, s jedným riadkom na fázu. Každá fáza má typ:
    - tool: goatbuilder spustí skript.
    - ansible: goatbuilder spustí playbook.
    - gate: kontrola, ktorá musí prejsť pred ďalšou fázou.
    - manual: urobíš to sám, a priebeh sa zastaví.

    Príkaz nižšie má `--dry-run`. Číta tento checkout a nič nemení. Pod tabuľkou si prečítaj, čo urobíš manuálne.
tutor-l13-title = Pripoj sa na goatnet inej organizácie
tutor-l13-text =
    Príkaz goatbuildera `federate` spojí tvoj goatnet s goatnetom, ktorý prevádzkuje iná organizácia. Každá strana si zostaví svoju polovicu dohody. Vymeníte si polovice, a každá strana ich spojí do tej istej dohody.

    Potvrď kľúč druhej strany iným spôsobom, napríklad telefonátom, ktorému dôveruješ. Kľúč, ktorý prichádza vo vnútri ich balíka, nič nedokazuje.

    Príkaz nižšie má `--dry-run`, takže nič nemení. Skutočný priebeh sa zastaví vo fáze 4, pretože tú fázu robíš ty manuálne.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Tento príkaz nie je súčasťou lekcie { $n }. Spusti `goatbuilder tutor`, aby si videl, kde si.
tutor-l1-s2 = Opýtaj sa doctora na jeden goatnet
tutor-l1-text =
    CLI goatbuilder zostavuje, nasadzuje a vydáva softvér GOAT. Skôr než ho použiješ, skontroluj svoje nastavenie dvoma príkazmi:
    - Príkaz goatbuildera `validate` skontroluje konfiguračný súbor `ops/goatbuilder/goatbuilder.yaml`. Číta iba tento checkout a nepotrebuje sieť.
    - Príkaz goatbuildera `doctor` skontroluje, čo potrebuje skutočné nasadenie: tvoje nástroje, tvoje prihlásenia, či sa tento checkout zhoduje s main a ako sa goatbuilder dostane ku každému goatnetu. Goatnet je jedno prostredie GOAT.

    Príkaz goatbuildera `doctor` vypíše, koľko kontrol prešlo, a potom každý problém s jeho opravou. V tejto lekcii je problém informácia, nie chyba. Prečítaj si opravy, ale teraz ich nemusíš použiť.

    Väčšina lekcií nepotrebuje prihlásenie. Lekcie 2 až 5, 10, 11 a 16 používajú cvičnú kópiu. Lekcie 8, 9, 12, 13, 15, 17 a 18 iba čítajú tento checkout. Lekcia 6 potrebuje `gh` a mesh, lekcia 7 potrebuje `gh` a lekcia 14 potrebuje mesh. Žiadna lekcia nepotrebuje 1Password.
tutor-l14-text =
    CLI goatbuilder sa vie aktualizovať z kópie, ktorú tvoj goatnet poskytuje na `get.<site>.<zone>/goatbuilder/`.

    Príkaz goatbuildera `update --check` iba číta. Porovná tvoju verziu s poskytovanou verziou a nič nezmení. Návratový kód 0 znamená, že máš aktuálnu verziu. Návratový kód 3 znamená, že existuje novšia verzia.

    Skutočná aktualizácia, bez `--check`, overí kontrolný súčet a podpis stiahnutého súboru skôr, než čokoľvek nahradí. Potom vymení súbor v jednom kroku.

    CLI goatbuilder musí vedieť, ktorého goatnetu sa má opýtať: zadaj `--site` s tým goatnetom alebo nastav `GOATBUILDER_UPDATE_URL`. Musíš byť na meshi toho goatnetu. Ak nie si, `--check` povie, že sa nevie dostať k serveru, a uvedie adresu, ktorú skúšal. To je v tejto lekcii správny výsledok.
tutor-l15-s1 = Vypíš testy, ktoré platia pre jeden goatnet
tutor-l6-text =
    Vydanie presúva novú verziu cez tri goatnety v poradí: dev, potom preprod, potom prod.

    1. Niekto otaguje verziu na main a CI ju zostaví.
    2. dev: goatbuilder ju nasadí a spustí akceptačné testy.
    3. preprod: goatbuilder ju nasadí znova, znova spustí testy a zaznamená výsledky.
    4. Verzia potom čaká na preprod stanovený čas. Nič sa nepresunie do prod samo.
    5. prod: po tomto čakaní osoba spustí vydanie s `--from prod`. CLI goatbuilder nasadí na prod a skontroluje všetky tri goatnety.

    Príkaz goatbuildera `promote status` ukazuje, kde je každý komponent. Kandidát je verzia, ktorá sa testuje. Všeobecný tvar vydania je `goatbuilder release <component> --tag <tag> --from dev`. Príkazy nižšie používajú jeden komponent tohto checkoutu a príkladový tag. Každý má `--dry-run`, takže sa nič neotaguje ani nenasadí.

    Skúšobné behy potrebujú `gh` a mesh. Ak nie si admin repozitára, daj `GOAT_RC_ROLE_CHECK=0` pred každý príkaz `release`.
tutor-l9-s1 = Vyvolaj skutočnú chybu s goatnetom, ktorý neexistuje
tutor-l9-text =
    Keď goatbuilder odmietne, vypíše kód. Kódy, ktoré začínajú `GB-`, pochádzajú z goatbuildera, `E_` z CLI goat a `X-` z iného nástroja, ktorý goatbuilder spustil.

    Príkaz goatbuildera `explain` prijme kód alebo text chybového riadku a povie ti, čo sa stalo, prečo, ako to opraviť a kde si prečítať viac.

    Požiadavka na goatnet, ktorý tento checkout nedeklaruje, vyvolá skutočnú chybu na vysvetlenie.
