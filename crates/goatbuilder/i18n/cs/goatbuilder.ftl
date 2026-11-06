### goatbuilder — cs (cs)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Sestavte a nasaďte prostředí goat: deklarovat, konvergovat, ověřit — v rámci jedné deploy smlouvy
help-flag-output = Formát výstupu: tabulka pro člověka; JSON nebo YAML pro program
help-flag-repo = Kořen repozitáře. Výchozí je checkout, ve kterém běží tento binární soubor
help-flag-yes = Potvrdí deklarativní příkaz bez dotazu (pro automatizaci)
help-flag-frozen = Odmítne každý běh, který by vyžadoval fast-forward checkoutu (přísné C3)
help-flag-mock = Běh proti adresáři s fixturami místo hostů: bez registru, bez transportu, deterministicky (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Jazyk nápovědy a vysvětlení chyb (značka jako fr, uk, de); výstup, který čte pipeline, zůstává anglicky. Výchozí: locale OS
help-cmd-validate = Zkontroluje konfiguraci proti verzovanému schématu a mezipolním pravidlům, offline
help-cmd-doctor = Ověří, co běh potřebuje, než se cokoli vydá: závislosti, aktuálnost checkoutu, transport každého prostředí a že playbook každé komponenty vede na jednoznačný cílový režim
help-cmd-status = Deklarováno oproti zaznamenanému, po strojích: converged, pending nebo unknown (C7, C8). Jen čte. Návratový kód 3, dokud některý stroj není converged
help-cmd-deploy = Nasadí pin deklarovaný v main na jedno prostředí a prokáže to: z pinů určí komponentu a skupiny prostředí, play a jeho cílový režim, každé tajemství, které play šablonuje, a transport; spustí play přes ansible-preflight; přečte deploy ledger zpět z každého hostu a za hotové ho prohlásí, až když nejnovější řádek uvádí deklarovaný digest s action=deploy (C7). Na fixture (`--mock`) hosty nahrazuje skriptovaný konvergátor
help-cmd-release = Provede release candidate od tagu na main přes živou validaci na dev goatnetu až na příčku preprod (ADR 1173); po soaku s `--from prod` ho povýší, zapracuje do fleet:, nasadí na prod a znovu ověří každou příčku. Nahrazuje ops/scripts/rc.sh krok za krokem
help-cmd-rollback = Vrátí nasazenou komponentu na její N-tý nejnovější ref podle typu artefaktu; zdrojem pravdy je ledger na hostu (ADR 1060). Nahrazuje ops/scripts/rollback.sh: kód 3 odmítnuto jako nebezpečné (data, cert, forward-only, reversible:false), 4 řízený postup (config, acl)
help-cmd-standup = Lokalita od nuly po ověřenou: fáze standupu v pořadí podle doktríny, na každém konci fail-closed brána, observability host jako paralelní větev, záchranné třídy a povrchy mezi goatnety nakonec (ADR 1183). Nahrazuje ops/scripts/goat-standup.sh krok za krokem. Na fixture (`--mock`) skriptovaný konvergátor zkonverguje každou komponentu každé třídy a čas deklarováno → ověřeno je měřítkem jednohodinového goatnetu
help-cmd-join-node = Další stroj dané třídy na živé lokalitě, deklarovaný → zkonvergovaný → ověřený: provedená sekvence z docs/tutorials/02-first-federated-node.md jako číslované fáze s branami (dosažitelnost, klíč, nejdřív práce s repozitářem, baseline, kernel WireGuard před enrolmentem i po něm, kernel ACL rovina jako brána enrolmentu, konvergence třídy, akceptace)
help-cmd-federate = Suverénní peering deklarovaný, zkonvergovaný a ověřený (ADR 1157): naše polovina změřena, balíčky vyměněny mimo pásmo, sestavena a ověřena jedna dohoda, z ní vyraženy leaves, pravidla vyrenderována jediným writerem, nasazen edge, otevřen transport zvolený dohodou, akceptace ve všech čtyřech úrovních. Nahrazuje ops/scripts/peering-standup.sh krok za krokem
help-cmd-pins = JEDEN průchod rozlišení se DVĚMA výstupy: pins.yml ve group_vars každého uvedeného člena a artefakt IntentV1 každého uvedeného goatnetu, z ops/pins/fleet-pins.yaml. Nahrazuje ops/pins/generate-pins.py bajt po bajtu. `--check` validuje, znovu generuje a odmítne drift, nepodepsaný nebo pozměněný intent a řízený pin definovaný mimo pins.yml (exit 1)
help-cmd-explain = Vysvětlí chybu: kód (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) nebo libovolný text chyby — vlastní tohoto binárního souboru, nebo slova nástroje, která nesl (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Vypíše, co se stalo, proč, co dělat a kde číst dál; `-` čte text ze stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-fastlane = Jeden příkaz od mergnutého commitu ke každé goatnet (ADR 1127): záměrná výjimka z žebříčku. Nahrazuje ops/scripts/fastlane.sh krok za krokem; odmítne nepinnutý digest nebo otevřený set povýšení a nasazení spouští přes tento binární soubor
help-cmd-promote = Vede a sleduje povýšení vydání napříč žebříčkem goatnet (ADR 1120, 1125): stav, otevření kandidáta, povýšení, povýšení setu, cesta hotfixu, review, připravenost, zrušení, kontrola. Nahrazuje ops/release/graduate.py krok za krokem. Nic se tu nepovýší samo: mutace proběhne jen na vyžádání, vypíše, co udělala, a bez terminálu vyžaduje --yes (C10, ADR 1177)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Lokalita z ověřeného stavu na nic — protějšek standupu, záznamy zůstávají (burn-down řádky teardown přidává, nikdy nemaže). Na fixture (`--mock`) nacvičí burn-down v obráceném pořadí fází; na skutečných hostech vypíše pořadí a odmítne běžet, dokud každá fáze nedeklaruje svůj teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekce ve stylu vimtutoru: každá uvede přesný příkaz ke spuštění nad jednorázovou kopií fixture (nebo, jen pro čtení, nad tímto checkoutem), pak ověří, co ten příkaz skutečně udělal — jeho exit a řádky — a teprve pak odemkne další. `tutor` ukáže aktuální lekci a označí ji za hotovou, když ji vaše běhy splní; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skryté (src/fun.rs)
help-cmd-argan = Skryté (src/fun.rs)
help-cmd-bound = Skryté (crates/goat-fun)
help-cmd-secrets = Deklarovaný postoj k secretům (ADR 1204): který store platí (1password, openbao, sops, git-crypt, env-file, env), co je k dispozici, jak doplnit chybějící a vlastní kontroly storu. `init --env <goatnet>` zapíše env soubor pro quickstart bez SaaS: silnou náhodnou hodnotu pro každou proměnnou, kterou plays goatnetu šablonují, 0600, v gitignore, nikdy nepřepíše hodnotu, která už tam je. Stejný modul jako ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Přečte u každého hostitele nejnovější řádek deploy-ledgeru pro jednu komponentu a zjistí, zda uvádí ref, který deklaruje main (C7). Jen čte; konec s kódem 3, dokud je některý hostitel pending nebo unknown. Stejná kontrola, jakou spouští `deploy --verify-only`
help-cmd-diff = Čím se to právě liší od mainu (C8): deklarovaný ref v mainu proti nejnovějšímu řádku ledgeru každého hostitele, pro jednu komponentu, nebo bez ní pro všechny komponenty prostředí, načtené jedním průchodem na skupinu členů. Jen čte; konec s kódem 3 při jakémkoli driftu nebo nečitelném hostiteli
help-cmd-history = Deploy ledger jako časová osa, od nejnovějšího, na každém hostiteli prostředí (C8): každý řádek je samostatný záznam se strojem, časem, akcí (deploy nebo rollback) a verzí. Předchozí verze záznamu a operátor, který ho spustil, jsou v `-o json`. Jen čte
help-cmd-plan = Zapíše do souboru plánu k revizi, co by udělalo deklarativní sloveso: deklarované refy, play a cíl, ledger, commit mainu, transport (deploy); typ artefaktu a cílový ref (rollback). Nemění stav žádného hostitele. Po revizi ho spusťte přes `apply`
help-cmd-apply = Spustí přesně zrevidovaný plán. Odmítne (konec s kódem 1, s uvedením, co se změnilo), pokud se od zápisu plánu změnila deklarace v mainu, commit pinů, play nebo cíl rollbacku; změněný transport se jen oznámí, neodmítne se
help-cmd-adopt = Deklaruje, co prostředí už běží: přečte ledgery všech hostitelů a navrhne odchylku pinů pro každou komponentu, jejíž běžící ref se liší od deklarace v mainu. Ve výchozím stavu jen report; s --yes se úpravy zapíšou do nového worktree mainu k revizi. Nikdy nenasazuje

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ještě ne: { $why }
tutor-lesson-done = Lekce { $n } hotová: { $title }.
tutor-all-done-reset = Všechny lekce jsou hotové. `goatbuilder tutor --reset` začne od začátku; tutoriály jdou dál: docs/tutorials/.
tutor-all-done = Všechny lekce jsou hotové. Tutoriály jdou dál: docs/tutorials/.
tutor-workspace = Pracovní prostor: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Ověří, že goatnet byl postaven správně, vrstvu po vr
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Izolovaná lokalita: nestahovat trunk; checkout posoudit proti verzi v cache a běh zaznamenat jako offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Ověří, že goatnet je ve svém deklarovaném cílovém stavu: každá služba, o které si goatnet sám deklaruje, že běží, je nasazená, běží, je zdravá a plní svou úlohu, její zero-trust hrany jsou propojené a nic neběží, co není deklarované. Pouze čte; dokud něco nesedí, končí s kódem 3
help-cmd-survey = Jak konzistentní je software uvnitř každého goatnetu a napříč goatnety žebříku, zjištěno jedním paralelním průchodem. Čte pouze

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Jak konzistentní je software uvnitř každého goatnetu a napříč goatnety žebříku, zjištěno jedním paralelním průchodem. Pouze čtení.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Nahradí tento binárník vydáním, které servíruje jeho goatnet na `get.<site>.<zone>/goatbuilder/`: stažený soubor se před jakýmkoli zásahem ověří proti jeho `.sha256` a podepsanému `SHA256SUMS`, výměna je atomická. Už aktuální: jeden řádek, exit 0. `--check` jen čte (exit 3, když je verze pozadu). Povrch: `GOATBUILDER_UPDATE_URL`, jinak `--site` z jeho group_vars, jinak ten, který použil poslední update
tutor-prereq = Než začnete: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Obarví glyf ok/FAIL a verdikt v tabulce (auto|always|never). `auto` barví jen terminál; NO_COLOR a CLICOLOR_FORCE mají přednost. JSON a YAML se nikdy nebarví
help-flag-verbose = Loguje, co běh dělá — externí příkazy, které spouští, a síťovou fázi `update` — na stderr, takže `-o json|yaml` zůstane čistý dokument. Nebo nastav RUST_LOG=goatbuilder=debug
help-flag-timeout = Počet sekund, po které se omezí fáze připojení/dosažení, než selže hned — jediná fáze, která může viset na nedostupném hostiteli (`update` při stahování nabízené verze). Neomezuje běh samotného playe ani stahování binárky (hlídá se stagnace). Čte také GOATBUILDER_TIMEOUT
help-cmd-completion = Vypíše skript pro doplňování shellu na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Pro živé kandidáty, které statický skript znát nemůže — sites, components a goatnets deklarované v tomto checkoutu — napoj hook na `goatbuilder _complete` (viz hlavička vygenerovaného skriptu)
help-cmd-docs = Referenční příručka příkazů: `goatbuilder docs` na ni odkáže; `--markdown` vypíše celou, procházením živého stromu příkazů, takže se nikdy nerozejde s `--help`
help-cmd-_complete = Skrytý: vypíše živé kandidáty doplňování pro hooky shellu, jednoho na řádek (sites, components a goatnets deklarované v tomto checkoutu). Jen čte, nikdy hlasitě nechybuje — doplňovací hook nesmí rozbít prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Před rolloutem výpis připravenosti a vratnosti (ADR 1211): tři fakta — State (stateless | stateful), Rollback (pin-revert | restore-required | none) a Scope (single | coupled) — deklarovaná per komponenta a FAIL-CLOSED, pokud chybí; k tomu část o připravenosti, kterou dokládají ostatní verby (ref lze stáhnout, pins --check, secrets gate, čerstvý zdroj, deklarovaný koncový stav), a validace faktu o rollbacku (rollback --dry-run pro pin-revert, retenční okno pro restore-required). `deploy` ho spouští jako první; hodnoceno podle poctivosti, nikdy falešně zelená. Jen čte; exit 3, když není čisté

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Zkontrolujte svoje nastavení
tutor-l1-s1 = Zkontrolujte konfigurační soubor
tutor-l2-title = Čtení stavu: converged, pending, unknown
tutor-l2-text =
    Příkaz goatbuilder `status` porovná u každého stroje dvě věci: verzi, kterou pro něj deklaruje main, a verzi, o které jeho vlastní záznam říká, že běží. Nic nemění.

    Každý stroj je v jednom ze tří stavů:
    - converged: jeho záznam uvádí deklarovanou verzi. Je to hotové.
    - pending: jeho záznam uvádí starší verzi, nebo ještě nic.
    - unknown: goatbuilder neumí jeho záznam přečíst.

    Za hotové se počítá jen converged. Pokud je některý stroj pending nebo unknown, `status` skončí s návratovým kódem 3.

    Od této lekce dál `--mock` ukáže goatbuilderu na cvičnou kopii ve vaší složce tutoru místo na skutečné stroje. Nic, co tam uděláte, se neprojeví na skutečném stroji.

    Na `dev` cvičné kopie ještě nic nasazeno nebylo. Čekejte proto čtyři stroje pending a jeden unknown. Stroje converged uvidíte v lekci 3.
tutor-l2-s1 = Přečtěte stav dev
tutor-l3-title = Nasazení: naplánuj ho, spusť ho, dokaž to
tutor-l3-text =
    Nasazování umístí deklarovanou verzi na každý stroj, pak přečte záznam každého stroje, aby to dokázalo. Nasazení je hotové jen tehdy, když záznam každého stroje uvádí tuto verzi. Nasazení, které proběhlo bez chyb, nestačí.

    `portal` nasadíte do `preprod` cvičné kopie. `--dry-run` ukáže, co by se stalo, a nic nemění. `--ack` zapíše, kdo to udělal; příkaz vaše jméno doplní z gitu. Po nasazení by `status` měl ukázat oba stroje jako `ok` a converged.
tutor-l3-s1 = Naplánujte nasazení
tutor-l3-s2 = Nasaďte portal na preprod
tutor-l3-s3 = Zkontrolujte, že jsou oba stroje converged
tutor-l4-title = Drift a historie
tutor-l4-text =
    Dva příkazy vám pomohou pochopit goatnet před změnou a po ní. Oba jen čtou.
    - Příkaz goatbuilder `diff` vypíše každý stroj, jehož záznam se liší od deklarované verze. Takový rozdíl se nazývá drift.
    - Příkaz goatbuilder `history` vypíše každou verzi, kterou komponenta měla na každém stroji, od nejnovější.

    Na `dev` je každý stroj pozadu. Historie `portal` na `preprod` nyní obsahuje vaše nasazení z lekce 3.
tutor-l4-s1 = Vypište drift na dev
tutor-l4-s2 = Přečtěte historii portal na preprod
tutor-l5-title = Vraťte verzi zpět a uvidíte, jak je nebezpečné vrácení odmítnuto
tutor-l5-text =
    Vrácení deklaruje starší verzi a nasadí ji jako každou jinou verzi. goatbuilder CLI pak přečte záznam každého stroje, aby to dokázal.

    Některé změny takto vrátit nelze. Nasazení starší verze nemůže vrátit data, která komponenta zapsala, certifikát, ani změnu, která jde jen vpřed. goatbuilder CLI taková vrácení odmítne s návratovým kódem 3 a řekne, co udělat místo toho.

    Komponenta `db-schema` cvičné kopie obsahuje data, takže ji goatbuilder odmítne vrátit zpět. Toto odmítnutí je očekávaný výsledek.
tutor-l5-s1 = Naplánujte vrácení
tutor-l5-s2 = Vraťte portal o jednu verzi zpět
tutor-l5-s3 = Zkuste vrátit zpět db-schema, což goatbuilder odmítá
tutor-l6-title = Vydání nové verze: dev, potom preprod, potom prod
tutor-l6-s1 = Zjistěte, kde je každá komponenta
tutor-l6-s2 = Spusťte suchý běh vydání z dev
tutor-l6-s3 = Spusťte suchý běh kroku prod
tutor-l7-title = Fastlane, a kdy ho nepoužívat
tutor-l7-text =
    Fastlane umístí jednu verzi na všechny goatnety najednou. Vynechá čekání na preprodu a záznamy vydání.

    Používejte ho jen pro naléhavou opravu, kterou umíte vrátit návratem k předchozí verzi. Nikdy ho nepoužívejte pro změnu databáze, změnu identity nebo přístupu, ani pro nic, co takto vrátit nejde.

    Fastlane odmítne verzi, kterou CI nesestavilo, a komponentu, která je součástí vydání, jež ještě probíhá.

    Obecná forma je `goatbuilder fastlane <component> --ref <version> --reason <why>`. Spusťte příklad níže. Má `--dry-run`, takže se nic nemění. Potřebuje `gh`.
tutor-l7-s1 = Spusťte suchý běh fastlane
tutor-l8-title = Piny: najděte drift a opravte ho správně
tutor-l8-text =
    Jeden soubor určuje, kterou verzi každý goatnet provozuje: `ops/pins/fleet-pins.yaml`. goatbuilder CLI z něj generuje ostatní soubory verzí: soubory `pins.yml` a podepsané soubory intent.

    Příkaz goatbuilder `pins --check` porovná vygenerované soubory s `fleet-pins.yaml` a odmítne každý soubor, který se liší. Příkaz goatbuilder `pins` je znovu vygeneruje.

    Vaše složka tutoru má kopii repozitáře, ve které někdo ručně upravil jeden soubor `pins.yml`. Ruční úpravu nikdy neopravujte další ruční úpravou. Změňte `fleet-pins.yaml`, a pak znovu vygenerujte.
tutor-l8-s1 = Zkontrolujte tento checkout
tutor-l8-s2 = Zkontrolujte kopii a najděte ruční úpravu
tutor-l8-s3 = Znovu vygenerujte soubory kopie
tutor-l8-s4 = Zkontrolujte kopii znovu
tutor-l9-title = Vysvětlení chyby
tutor-l9-s2 = Vysvětlete chybu podle jejího kódu
tutor-l9-s3 = Vysvětlete ji podle jejího textu
tutor-l10-title = Tutoriál 1: postavte první site nového goatnetu
tutor-l10-text =
    Tato lekce postaví celý goatnet v cvičné kopii. Ukázkový goatnet je `zagros`. Jeho první site, `alpha`, má dva core stroje, jeden monitorovací stroj a jeden záchranný stroj.

    Příkaz goatbuilder `standup` nastaví každý stroj v pevně daném pořadí a zkontroluje každý záznam. Jeho poslední řádek řekne, že site je hotová. Na skutečných strojích ten řádek také ukáže, jak dlouho to trvalo.

    Celý tutoriál je `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Postavte první site, alpha
tutor-l10-s2 = Zkontrolujte výsledek
tutor-l11-title = Tutoriál 2: přidejte druhou site
tutor-l11-text =
    Tato lekce začíná tam, kde skončil tutoriál 1. Přidá druhou site, `beta`, v nové cvičné kopii, `zagros-2`.

    Druhá site si zkopíruje uživatelské účty z první site. Její standup také jednou změní první site, aby kopírování fungovalo oběma směry. Dělá to krok s názvem `alpha/core`.

    Celý tutoriál je `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Postavte druhou site, beta
tutor-l11-s2 = Zkontrolujte obě site
tutor-l12-s1 = Naplánujte přidání jednoho stroje
tutor-l13-s1 = Naplánujte federaci
tutor-l14-title = Udržujte goatbuilder aktuální
tutor-l14-s1 = Zeptejte se, zda existuje novější goatbuilder
tutor-l15-title = Assay: zkontrolujte, jak byl goatnet postaven
tutor-l15-text =
    Příkaz goatbuilder `assay` zkontroluje, že byl goatnet postaven správně, vrstvu po vrstvě, pomocí akceptačních testů. Každý test dostane jeden ze tří výsledků:
    - PASS: test prošel.
    - FAIL: goatnet má problém.
    - UNKNOWN: na test nešlo odpovědět.

    Započítá se jen PASS. Jakýkoli FAIL nebo UNKNOWN způsobí návratový kód 3.

    `--plan` vypíše testy, které se na goatnet vztahují, podle vrstvy, a nedotkne se žádného stroje. Použijte ho, abyste viděli, co by skutečný assay pokryl. Obecná forma je `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: jsou verze shodné?
tutor-l16-text =
    Příkaz goatbuilder `survey` v jednom průchodu zkontroluje, zda stejné verze běží tam, kde by měly:
    - Uvnitř každého goatnetu: běží na každém stroji deklarovaná verze, a shodují se stroje, které by se shodovat měly?
    - Mezi dev, preprod a prod: jeden řádek na komponentu, jeden sloupec na goatnet.

    Rozdíl, který proces vydání očekává, se nahlásí, ale není to chyba. Příkladem je nová verze, která je na dev, ale ještě ne na prod. Rozdíl, který nic nedeklaruje, se čte jako `difference not declared`. Stroj, který goatbuilder neumí přečíst, je unknown, nikdy consistent.

    Spusťte ho na cvičné kopii. Její `dev` je pozadu úmyslně, čekejte proto nálezy a návratový kód 3.
tutor-l16-s1 = Spusťte survey na cvičné kopii
tutor-l17-title = Endstate: provozuje goatnet přesně to, co deklaruje?
tutor-l17-text =
    Příkaz goatbuilder `endstate` zkontroluje u goatnetu dvě věci:
    1. Každá služba, kterou deklaruje, je nasazená, běží, je zdravá a správně připojená.
    2. Nic neběží, co nedeklaruje. Nedeklarovaná služba se nazývá sirotek.

    goatbuilder CLI odvodí, co by site měla provozovat, z vlastního nastavení site, ne z odděleného seznamu.

    Kontrola, která nemá co číst, se označí UNOBSERVED. To je upřímná odpověď, ne neúspěch. FAIL nebo sirotek způsobí návratový kód 3.

    Obecná forma je `goatbuilder endstate <goatnet>`. Příkaz níže zkontroluje ukázkovou site z uložených dat, takže se nedotkne žádného stroje.
tutor-l17-s1 = Zkontrolujte ukázkovou site
tutor-l18-title = Bonus: buďte zvídaví
tutor-l18-text =
    Tato lekce je nepovinná. Spusťte příkaz níže a tutor skončí.

    goatbuilder CLI má pár malých skrytých funkcí. Nejsou v nápovědě ani v těchto lekcích. Některé reagují na slova, která dlouholetí uživatelé Unixu často zkouší napsat do nového programu, jen aby viděli, co se stane. Většina věcí, které zkusíte, nic neudělá.

    Tato lekce neříká, co to je. Smysl je najít je.

    Příkaz níže vypíše každou chybu, kterou umí goatbuilder vysvětlit. Je to dobré místo, kde začít zkoumat.
tutor-l18-s1 = Vypište každou chybu, kterou umí goatbuilder vysvětlit
tutor-heading = Lekce { $n } z { $total } · { $title }
tutor-setup = Nejdřív to jednou spusťte v tomto terminálu:
tutor-not-recording = Tento terminál nezaznamenává vaše běhy, takže tutor nemůže zaškrtávat vaše kroky. Spusťte to jednou, a pak spusťte kroky:
tutor-type = Udělejte tyto kroky ve svém terminálu ({ $met } z { $total } hotovo):
tutor-rerun = Po každém kroku spusťte `goatbuilder tutor` znovu. Zkontroluje, co jste udělali, a zaškrtne krok.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Vítejte v GOATBUILDER Tutor
tutor-bar-progress = Průběh
tutor-bar-done = { $done } z { $total } lekcí hotovo
tutor-after-step = Krok { $met } z { $total } hotový. Další:
tutor-after-lesson = Lekce { $n } je hotová. Spusťte `goatbuilder tutor` a začněte lekci { $next }. Pro opětovné přečtení: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekce { $n } je hotová, a tutor také. Spusťte `goatbuilder tutor` a uvidíte, že je označen jako hotový. Pro opětovné přečtení: `goatbuilder tutor --lesson { $n }`.

tutor-review = Tuto lekci jste dokončili, takže tato stránka je jen ke čtení. Spusťte `goatbuilder tutor`, abyste se vrátili tam, kde jste.
tutor-other-lessons = Pro opětovné přečtení lekce: `goatbuilder tutor --lesson 1`. Pro zobrazení všech: `goatbuilder tutor --list`.
tutor-l12-title = Přidání stroje do běžící site
tutor-l12-text =
    Příkaz goatbuilder `join-node` přidá ještě jeden stroj na site, která už běží. Nejdřív vypíše plán, s jedním řádkem na fázi. Každá fáze má typ:
    - tool: goatbuilder spustí skript.
    - ansible: goatbuilder spustí playbook.
    - gate: kontrola, která musí projít před další fází.
    - manual: to děláte sami, a běh se pozastaví.

    Příkaz níže má `--dry-run`. Čte tento checkout a nic nemění. Pod tabulkou si přečtěte, co děláte rukou.
tutor-l13-title = Připojení k goatnetu jiné organizace
tutor-l13-text =
    Příkaz goatbuilder `federate` spojí váš goatnet s goatnetem, který provozuje jiná organizace. Každá strana sestaví svou polovinu dohody. Vyměníte si poloviny, a každá strana je spojí do stejné dohody.

    Potvrďte klíč druhé strany jiným způsobem, například hovorem, kterému věříte. Klíč, který dorazí uvnitř jejich balíčku, nedokazuje nic.

    Příkaz níže má `--dry-run`, takže nic nemění. Skutečný běh se zastaví na fázi 4, protože tuto fázi děláte rukou.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Tento příkaz není součástí lekce { $n }. Spusťte `goatbuilder tutor`, abyste viděli, kde jste.
tutor-l1-s2 = Zeptejte se doctora na jeden goatnet
tutor-l1-text =
    goatbuilder CLI sestavuje, nasazuje a vydává software GOAT. Než ho použijete, zkontrolujte své nastavení dvěma příkazy:
    - Příkaz goatbuilder `validate` zkontroluje konfigurační soubor, `ops/goatbuilder/goatbuilder.yaml`. Čte jen tento checkout a nepotřebuje síť.
    - Příkaz goatbuilder `doctor` zkontroluje, co potřebuje skutečné nasazení: vaše nástroje, vaše přihlášení, zda tento checkout odpovídá mainu, a jak goatbuilder dosáhne na každý goatnet. Goatnet je jedno prostředí GOAT.

    Příkaz goatbuilder `doctor` vypíše, kolik kontrol prošlo, a pak každý problém s jeho opravou. V této lekci je problém informace, ne chyba. Opravy si přečtěte, ale nemusíte je teď provádět.

    Většina lekcí nepotřebuje přihlášení. Lekce 2 až 5, 10, 11 a 16 používají cvičnou kopii. Lekce 8, 9, 12, 13, 15, 17 a 18 jen čtou tento checkout. Lekce 6 potřebuje `gh` a mesh, lekce 7 potřebuje `gh` a lekce 14 potřebuje mesh. Žádná lekce nepotřebuje 1Password.
tutor-l14-text =
    goatbuilder CLI se umí aktualizovat z kopie, kterou váš goatnet servíruje na `get.<site>.<zone>/goatbuilder/`.

    Příkaz goatbuilder `update --check` jen čte. Porovná vaši verzi s servírovanou verzí a nic nemění. Návratový kód 0 znamená, že máte aktuální verzi. Návratový kód 3 znamená, že existuje novější verze.

    Skutečná aktualizace, bez `--check`, ověří kontrolní součet a podpis staženého souboru, než cokoli vymění. Pak soubor vymění v jednom kroku.

    goatbuilder CLI musí vědět, kterého goatnetu se zeptat: zadejte `--site` s tím goatnetem, nebo nastavte `GOATBUILDER_UPDATE_URL`. Musíte být na meshi toho goatnetu. Pokud nejste, `--check` řekne, že se nemůže dostat na server, a uvede adresu, kterou zkoušel. To je pro tuto lekci správný výsledek.
tutor-l15-s1 = Vypište testy, které se vztahují na jeden goatnet
tutor-l6-text =
    Vydání posune novou verzi postupně přes tři goatnety: dev, potom preprod, potom prod.

    1. Někdo označí verzi tagem na mainu a CI ji sestaví.
    2. dev: goatbuilder ji nasadí a spustí akceptační testy.
    3. preprod: goatbuilder ji nasadí znovu, znovu spustí testy a zapíše výsledky.
    4. Verze pak čeká na preprodu po stanovenou dobu. Do produ nic nepostupuje samo.
    5. prod: po tomto čekání někdo spustí vydání s `--from prod`. goatbuilder CLI nasadí na prod a zkontroluje všechny tři goatnety.

    Příkaz goatbuilder `promote status` ukáže, kde je každá komponenta. Kandidát je verze, která se testuje. Obecná forma vydání je `goatbuilder release <component> --tag <tag> --from dev`. Příkazy níže používají jednu komponentu tohoto checkoutu a ukázkový tag. Každý má `--dry-run`, takže se nic neotaguje ani nenasadí.

    Suché běhy potřebují `gh` a mesh. Pokud nejste administrátorem repozitáře, dejte `GOAT_RC_ROLE_CHECK=0` před každý příkaz `release`.
tutor-l9-s1 = Vyrobte skutečnou chybu s goatnetem, který neexistuje
tutor-l9-text =
    Když goatbuilder odmítne, vytiskne kód. Kódy začínající `GB-` pocházejí od goatbuilderu, `E_` od CLI goat, a `X-` od jiného nástroje, který goatbuilder spustil.

    Příkaz goatbuilder `explain` přijme kód, nebo text řádku chyby, a řekne vám, co se stalo, proč, jak to opravit a kde si přečíst víc.

    Dotaz na goatnet, který tento checkout nedeklaruje, vytvoří skutečnou chybu k vysvětlení.
