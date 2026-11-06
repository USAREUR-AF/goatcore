### goatbuilder — hu (hu)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Goat környezet építése és szállítása: deklarálás, konvergálás, ellenőrzés — egyetlen deploy-szerződés alatt
help-flag-output = Kimeneti formátum: táblázat embernek; JSON vagy YAML programnak
help-flag-repo = A repó gyökere. Alapértelmezés: az a checkout, amelyben a bináris fut
help-flag-yes = Deklaratív művelet megerősítése rákérdezés nélkül (automatizáláshoz)
help-flag-frozen = Elutasít minden futást, amely a checkout fast-forward léptetését igényelné (szigorú C3)
help-flag-mock = Hostok helyett fixture-könyvtáron fut: nincs registry, nincs transzport, determinisztikus (goatbuilder-conformance.md §"The fixture")
help-flag-lang = A súgó és a hibamagyarázatok nyelve (címke, pl. fr, uk, de); a pipeline által olvasott kimenet angol marad. Alapértelmezés: az OS nyelve
help-cmd-validate = Konfig ellenőrzése a verziózott sémával és a mezők közötti szabályokkal, offline
help-cmd-doctor = Felméri, mit igényel egy futás, mielőtt bármi élesedik: függőségek, checkout frissessége, minden környezet transzportja, és hogy minden komponens playbookja egyértelmű célmódra oldódik fel
help-cmd-status = Deklarált és rögzített állapot gépenként: converged, pending vagy unknown (C7, C8). Csak olvas. 3-as kilépési kód, amíg bármelyik gép nem converged
help-cmd-deploy = A main által deklarált pint felteszi egy környezetre, és bizonyítja: feloldja a komponenst és a környezet csoportjait a pinekből, a playt és célmódját, minden secretet, amit a play sablonoz, és a transzportot; lefuttatja a playt az ansible-preflighton át; minden hostról visszaolvassa a deploy ledgert, és csak akkor kész, ha a legújabb sor a deklarált digestet nevezi meg action=deploy mellett (C7). Fixture-ön (`--mock`) a szkriptelt converger helyettesíti a hostokat
help-cmd-release = Release candidate-et visz a main egy tagjétől a dev goatnet éles validációján át a preprod szintig (ADR 1173); a soak után, `--from prod` mellett, graduálja, beolvasztja a fleet:-be, deployolja a prodot és újraellenőriz minden szintet. Lépésről lépésre leváltja az ops/scripts/rc.sh-t
help-cmd-rollback = Telepített komponenst visszagörget az N-edik legutóbbi refjére, artifact-típus szerint, a hoston lévő ledgert tekintve igazságforrásnak (ADR 1060). Leváltja az ops/scripts/rollback.sh-t: 3-as kód: elutasítva, nem biztonságos (data, cert, forward-only, reversible:false), 4: vezetett (config, acl)
help-cmd-fastlane = Egy parancs a merge-elt committól minden goatnetig (ADR 1127): a létra szándékos kivétele. Lépésről lépésre leváltja az ops/scripts/fastlane.sh-t; elutasítja a nem pinelt digestet vagy a nyitott graduációs halmazt, és a deployokat ezen a binárison át futtatja
help-cmd-standup = Egy site a semmiből az ellenőrzöttig: a standup fázisai a doktrína sorrendjében, mindkét végén fail-closed kapuval, a megfigyelhetőségi host párhuzamos ágon, a rescue osztályok és a goatnetek közötti felületek a végén (ADR 1183). Lépésről lépésre leváltja az ops/scripts/goat-standup.sh-t. Fixture-ön (`--mock`) minden osztály minden komponensét a szkriptelt converger konvergálja, és a deklarált → ellenőrzött idő az egyórás goatnet mércéje
help-cmd-join-node = Még egy adott osztályú gép egy élő site-on, deklarált → konvergált → ellenőrzött: a docs/tutorials/02-first-federated-node.md végrehajtott sorrendje számozott fázisokként, kapukkal (elérés, kulcs, előbb a repó-munka, baseline, kernel WireGuard a felvétel előtt és után, a kernel ACL sík mint felvételi kapu, az osztály converge-e, átvétel)
help-cmd-federate = Szuverén peering deklarálva, konvergálva és ellenőrizve (ADR 1157): a mi felünk megmérve, a csomagok sávon kívül cserélve, egy megállapodás összeállítva és ellenőrizve, belőle a leafek kibocsátva, a szabályok az egyetlen író által renderelve, az edge kitelepítve, a megállapodásban választott transzport megnyitva, átvétel mind a négy szinten. Lépésről lépésre leváltja az ops/scripts/peering-standup.sh-t
help-cmd-pins = EGY feloldási menet KÉT renderelessel: minden felsorolt tag group_vars pins.yml-je és minden felsorolt goatnet IntentV1 artifactja, az ops/pins/fleet-pins.yaml-ból. Bájtra pontosan leváltja az ops/pins/generate-pins.py-t. A `--check` validál, újragenerál, és elutasítja az eltérést (drift), az aláíratlan vagy módosított intentet, valamint a pins.yml-en kívül definiált kormányzott pint (1-es kód)
help-cmd-promote = Release-graduáció vezetése és megfigyelése a goatnet-létrán (ADR 1120, 1125): status, jelölt nyitása, graduálás, halmaz graduálása, hotfix út, review, readiness, abandon, check. Lépésről lépésre leváltja az ops/release/graduate.py-t. Itt semmi sem graduál magától: a módosítás csak híváskor fut, kiírja, mit tett, és terminál nélkül --yes-t kér (C10, ADR 1177)
help-cmd-explain = Hiba értelmezése: kód (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) vagy bármilyen hibaszöveg — a bináris sajátja, vagy egy eszköz általa továbbított szavai (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Kiírja, mi történt, miért, mit tegyél és hol olvashatsz tovább; a `-` a szöveget stdin-ről olvassa

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Egy site az ellenőrzött állapotból a semmibe — a standup tükörképe, a nyilvántartás megmarad (a lebontás teardown sorokat fűz hozzá, soha nem töröl). Fixture-ön (`--mock`) fordított fázissorrendben gyakorolja be a lebontást; valódi hosztokon kiírja a sorrendet, és megtagadja a végrehajtást, amíg minden fázis nem deklarálja a saját teardown playét (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Leckék vimtutor-stílusban: mindegyik megnevezi a pontos parancsot, amelyet egy fixture eldobható másolatán kell futtatni (vagy csak olvasásra ezen a checkouton), majd ellenőrzi, mit csinált valójában a parancs — a kilépési kódját és a sorait —, mielőtt a következő feloldódik. A `tutor` megmutatja az aktuális leckét, és késznek jelöli, ha a futtatásaid teljesítik; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Rejtett (src/fun.rs)
help-cmd-argan = Rejtett (src/fun.rs)
help-cmd-bound = Rejtett (crates/goat-fun)
help-cmd-secrets = A deklarált secrets-állapot (ADR 1204): melyik tároló van érvényben (1password, openbao, sops, git-crypt, env-file, env), mi érhető el, hogyan pótolható, ami hiányzik, és a tároló saját ellenőrzései. Az `init --env <goatnet>` megírja a zéró-SaaS gyorsindítás env fájlját: erős véletlen érték minden olyan változóhoz, amelyet a goatnet playei sablonba tesznek, 0600, gitignore-ban, a már meglévő értéket soha nem rotálja. Ugyanaz a modul, mint az ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Egy komponensre beolvassa minden host legfrissebb deploy-ledger sorát, és megmondja, hogy az a main által deklarált refet nevezi-e meg (C7). Csak olvas; 3-mal lép ki, amíg bármely host pending vagy unknown. Ugyanez az ellenőrzés fut a `deploy --verify-only` alatt
help-cmd-diff = Miben tér el most a main-től (C8): a main deklarált refje minden host legfrissebb ledger sorával szemben, egy komponensre, vagy ha nincs megadva, az environment összes komponensére, tagcsoportonként egy menetben olvasva. Csak olvas; 3-mal lép ki bármilyen eltérésnél vagy olvashatatlan hostnál
help-cmd-history = A deploy ledger idővonalként, legújabb elöl, az environment minden hostján (C8): minden sor egy nyilvántartás a géppel, az idővel, a művelettel (deploy vagy rollback) és a verzióval. A nyilvántartás korábbi verziója és a műveletet futtató operátor a `-o json`-ban van. Csak olvas
help-cmd-plan = Tervfájlba írja átnézésre, mit tenne egy deklaratív parancs: a deklarált refeket, a play-t és a célt, a ledgert, a main commitját, a transportot (deploy); az artifact típusát és a cél refet (rollback). Egyetlen host állapotához sem nyúl. Átnézés után futtasd az `apply` paranccsal
help-cmd-apply = Pontosan az átnézett tervet futtatja. Elutasítja (1-gyel lép ki, megnevezi, mi mozdult), ha a terv megírása óta elmozdult a main deklarációja, a pins commit, a play vagy a rollback cél; a megváltozott transportot jelzi, de nem utasítja el
help-cmd-adopt = Deklarálja, amit egy environment már futtat: beolvassa minden host ledgereit, és pins eltérést javasol minden komponensre, amelynek futó refje különbözik a main deklarációjától. Alapból csak jelentés; --yes esetén a módosításokat a main friss worktree-jébe írja átnézésre. Soha nem deployol

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Még nem: { $why }
tutor-lesson-done = { $n }. lecke kész: { $title }.
tutor-all-done-reset = Minden lecke kész. A `goatbuilder tutor --reset` elölről indítja; a tutorialok továbbmennek: docs/tutorials/.
tutor-all-done = Minden lecke kész. A tutorialok továbbmennek: docs/tutorials/.
tutor-workspace = Munkaterület: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Rétegenként ellenőrzi, hogy egy goatnet helyesen épült-e fel: minden, a helyszínre vonatkozó átvételi ellenőrzés, beleértve azokat is, amelyeket az operátor a goat CLI-vel futtat. A standup átvételi fázisa önálló parancsként. Csak olvas; kilépési kód 3, amíg bármely ellenőrzés hibás vagy nem tud válaszolni
help-cmd-survey = Mennyire egységes a szoftver az egyes goatneteken belül és a létra goatnetjei között, egyetlen párhuzamos olvasással. Csak olvas
help-flag-offline = Elszigetelt telephely: ne töltse le a trunköt; a checkoutot a gyorsítótárazott példányhoz mérje, és a futást offline-ként rögzítse (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Ellenőrzi, hogy a goatnet a deklarált végállapotában van-e: minden szolgáltatás, amelyet a saját deklarációi szerint futtatnia kell, telepítve van, fut, egészséges és ellátja a feladatát, a zero-trust élei be vannak kötve, és semmi sem fut, amit nem deklarál. Csak olvas; 3-as kilépési kóddal tér vissza, amíg bármi eltér
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Mennyire egységes a szoftver az egyes goatnetekben és a létra goatnetjei között, egyetlen párhuzamos menetben felmérve. Csak olvas.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Lecseréli ezt a binárist arra a kiadásra, amelyet a goatnet a `get.<site>.<zone>/goatbuilder/` címen szolgál ki: a letöltést a `.sha256` fájllal és az aláírt `SHA256SUMS` fájllal ellenőrzi, mielőtt bármihez hozzányúlna, és a csere atomi. Ha már friss: egy sor, kilépési kód 0. A `--check` csak olvas (3-as kilépési kód, ha le van maradva). A forrás: `GOATBUILDER_UPDATE_URL`, ennek hiányában a `--site` a group_vars-ból, végül az, amelyet az utolsó frissítés használt
tutor-prereq = Mielőtt elkezdenéd: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = A táblázat ok/FAIL jelét és az ítéletet színezi (auto|always|never). Az `auto` csak terminálon színez; a NO_COLOR és a CLICOLOR_FORCE felülbírálja. A JSON és a YAML soha ninc

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-verbose = Naplózza, mit csinál egy futás — az általa indított külső parancsokat és az `update` hálózati fázisát — az stderr-re, így az `-o json|yaml` tiszta dokumentum marad. Vagy állítsd be: RUST_LOG=goatbuilder=debug
help-flag-timeout = Másodpercben korlátozza a csatlakozási/elérési fázist, mielőtt gyorsan hibával leáll — ez az egyetlen fázis, amely elérhetetlen gazdagépen blokkolhat (az `update` lekéri a kiszolgált verziót). NEM korlátozza a play saját futását és a bináris letöltését (megakadás-korlátos). Olvassa a GOATBUILDER_TIMEOUT változót is
help-cmd-completion = Shell-kiegészítő szkriptet ír az stdout-ra (`goatbuilder completion zsh >> ~/.zshrc`). Élő jelöltekhez, amelyeket statikus szkript nem ismerhet — az ebben a checkoutban deklarált site-ok, komponensek és goatnetek — kösd be a hookot a `goatbuilder _complete` parancsra (lásd a generált szkript fejlécét)
help-cmd-docs = A parancsreferencia: a `goatbuilder docs` erre mutat; a `--markdown` az egészet kiírja, az élő parancsfából bejárva, így sosem térhet el a `--help`-től
help-cmd-_complete = Rejtett: élő kiegészítési jelölteket ír ki shell hookoknak, soronként egyet (az ebben a checkoutban deklarált site-ok, komponensek és goatnetek). Csak olvas, soha nem hibázik hangosan — a kiegészítő hook nem rontsa el a promptot

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Rollout előtt: az üzemkészségi és visszafordíthatósági kimutatás (ADR 1211): három tény — State (stateless | stateful), Rollback (pin-revert | restore-required | none) és Scope (single | coupled) — komponensenként deklarálva, deklaráció hiányában FAIL-CLOSED, az üzemkészségi résszel, amelyet a többi parancs bizonyít (ref pullable, pins --check, a secrets gate, fresh source, a deklarált végállapot), valamint a rollback tény ellenőrzésével (rollback --dry-run a pin-revert esetén, a megőrzési ablak a restore-required esetén). A `deploy` ezt futtatja először; őszinteség szerint minősített, soha nem hamis zöld. Csak olvas; 3-as kilépési kóddal tér vissza, ha nem tiszta

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Ellenőrizd a felkészültségedet
tutor-l1-s1 = Ellenőrizd a konfigurációs fájlt
tutor-l2-title = Olvasd az állapotot: converged, pending, unknown
tutor-l2-text =
    A goatbuilder `status` parancs két dolgot hasonlít össze minden gépre: a verziót, amit a main deklarál rá, és a verziót, amit a saját nyilvántartása szerint futtat. Semmit nem változtat.

    Minden gép három állapot egyikében van:
    - converged: a nyilvántartása a deklarált verziót nevezi meg. Ez kész.
    - pending: a nyilvántartása egy régebbi verziót nevez meg, vagy még semmit.
    - unknown: a goatbuilder nem tudja elolvasni a nyilvántartását.

    Csak a converged számít késznek. Ha bármelyik gép pending vagy unknown, a `status` 3-as kóddal lép ki.

    Ettől a leckétől kezdve a `--mock` a goatbuildert egy gyakorló másolat felé irányítja a lecke mappádban, valódi gépek helyett. Semmi, amit ott csinálsz, nem ér el egy valódi gépet.

    A gyakorló másolat `dev`-jén még semmi nincs deployolva. Tehát négy pending gépre és egy unknownra számíthatsz. Converged gépeket a 3. leckében fogsz látni.
tutor-l2-s1 = Olvasd a dev állapotát
tutor-l3-title = Deploy: tervezd meg, futtasd le, bizonyítsd be
tutor-l3-text =
    A deploy a deklarált verziót teszi fel minden gépre, majd elolvassa minden gép nyilvántartását, hogy bizonyítsa, megérkezett. Egy deploy csak akkor kész, ha minden gép nyilvántartása azt a verziót nevezi meg. Egy deploy, amely hibák nélkül futott le, nem elég.

    A `portal`-t fogod deployolni a gyakorló másolat `preprod`-jára. A `--dry-run` megmutatja, mi történne, és semmit nem változtat. Az `--ack` rögzíti, ki csinálta; a parancs kitölti a nevedet a gitből. A deploy után a `status`-nak mindkét gépet `ok`-ként és convergedként kell mutatnia.
tutor-l3-s1 = Tervezd meg a deployt
tutor-l3-s2 = Deployold a portalt a preprodra
tutor-l3-s3 = Ellenőrizd, hogy mindkét gép converged
tutor-l4-title = Eltérés és előzmények
tutor-l4-text =
    Két parancs segít megérteni egy goatnetet egy változás előtt és után. Mindkettő csak olvas.
    - A goatbuilder `diff` parancs felsorol minden gépet, amelynek nyilvántartása eltér a deklarált verziótól. Egy ilyen különbséget eltérésnek neveznek.
    - A goatbuilder `history` parancs felsorolja minden verziót, amit egy komponens egy adott gépen futtatott, a legújabbal kezdve.

    A `dev`-en minden gép le van maradva. A `portal` előzményei a `preprod`-on most már tartalmazzák a 3. leckéből a deploydat.
tutor-l4-s1 = Listázd az eltérést a dev-en
tutor-l4-s2 = Olvasd el a portal előzményeit a preprodon
tutor-l5-title = Görgess vissza, és nézd meg, hogy egy nem biztonságos visszagörgetést hogyan utasítanak el
tutor-l5-text =
    A visszagörgetés egy korábbi verziót deklarál, és úgy deployolja, mint bármely más verziót. A goatbuilder CLI ezután elolvassa minden gép nyilvántartását, hogy bizonyítsa, megérkezett.

    Néhány változást nem lehet így visszavonni. Egy régebbi verzió deploylása nem tudja visszavonni azokat az adatokat, amelyeket egy komponens írt, egy tanúsítványt, vagy egy változást, amely csak előrefelé mozog. A goatbuilder CLI 3-as kilépési kóddal elutasítja ezeket, és megmondja, mit kell helyette tenni.

    A gyakorló másolat `db-schema` komponense adatokat tartalmaz, így a goatbuilder elutasítja a visszagörgetését. Az elutasítás a várt eredmény.
tutor-l5-s1 = Tervezd meg a visszagörgetést
tutor-l5-s2 = Görgesd vissza a portalt egy verzióval
tutor-l5-s3 = Próbáld visszagörgetni a db-schemát, amit a goatbuilder elutasít
tutor-l6-title = Adj ki egy új verziót: dev, majd preprod, majd prod
tutor-l6-s1 = Nézd meg, hol van minden komponens
tutor-l6-s2 = Futtasd le próbaként a release-t a devről
tutor-l6-s3 = Futtasd le próbaként a prod lépést
tutor-l7-title = Fastlane, és mikor ne használd
tutor-l7-text =
    A Fastlane egy verziót tesz fel minden goatnetre egyszerre. Kihagyja a preprodon történő várakozást és a release nyilvántartásait.

    Csak egy sürgős javításra használd, amit visszavonhatsz az előző verzióra visszatérve. Soha ne használd egy adatbázis-változáshoz, egy identitás- vagy hozzáférés-változáshoz, vagy bármihez, amit nem tudsz így visszavonni.

    A Fastlane elutasít egy verziót, amit a CI nem épített meg, és egy komponenst, amely egy még folyamatban lévő release része.

    Az általános forma `goatbuilder fastlane <component> --ref <version> --reason <why>`. Futtasd le az alábbi példát. Van `--dry-run`-ja, így semmi nem változik. `gh` szükséges hozzá.
tutor-l7-s1 = Futtasd le próbaként egy fastlane-t
tutor-l8-title = Pins: találd meg az eltérést, és javítsd ki helyesen
tutor-l8-text =
    Egy fájl határozza meg, melyik verziót futtatja minden goatnet: a `ops/pins/fleet-pins.yaml`. A goatbuilder CLI ebből generálja a többi verziófájlt: a `pins.yml` fájlokat és az aláírt intent fájlokat.

    A goatbuilder `pins --check` parancs összehasonlítja a generált fájlokat a `fleet-pins.yaml`-lal, és elutasít minden fájlt, amely eltér. A goatbuilder `pins` parancs újra generálja őket.

    A lecke mappádban van a repository egy másolata, amelyben valaki kézzel szerkesztett egy `pins.yml` fájlt. Soha ne javíts ki egy kézi szerkesztést egy másik kézi szerkesztéssel. Változtasd meg a `fleet-pins.yaml`-t, majd generáld újra.
tutor-l8-s1 = Ellenőrizd ezt a checkoutot
tutor-l8-s2 = Ellenőrizd a másolatot, és találd meg a kézi szerkesztést
tutor-l8-s3 = Generáld újra a másolat fájljait
tutor-l8-s4 = Ellenőrizd újra a másolatot
tutor-l9-title = Magyarázz meg egy hibát
tutor-l9-s2 = Magyarázd meg a hibát a kódja alapján
tutor-l9-s3 = Magyarázd meg a hibát a szövege alapján
tutor-l10-title = 1. tutorial: építsd meg egy új goatnet első site-ját
tutor-l10-text =
    Ez a lecke egy teljes goatnetet épít fel egy gyakorló másolatban. A példa goatnet a `zagros`. Az első site-ja, az `alpha`, két fő gépből, egy megfigyelő gépből és egy mentő gépből áll.

    A goatbuilder `standup` parancs fix sorrendben állít be minden gépet, és ellenőriz minden nyilvántartást. Az utolsó sora azt mondja, hogy a site kész. Valódi gépeken az a sor azt is megmutatja, mennyi ideig tartott.

    A teljes tutorial a `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Építsd fel az első site-ot, az alphát
tutor-l10-s2 = Ellenőrizd az eredményt
tutor-l11-title = 2. tutorial: adj hozzá egy második site-ot
tutor-l11-text =
    Ez a lecke ott kezdődik, ahol az 1. tutorial befejezte. Hozzáad egy második site-ot, a `beta`-t, egy új gyakorló másolatban, a `zagros-2`-ben.

    A második site átmásolja a felhasználói fiókokat az első site-ról. A standupja az első site-ot is megváltoztatja egyszer, hogy a másolás mindkét irányban működjön. Egy `alpha/core` nevű lépés csinálja ezt.

    A teljes tutorial a `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Építsd fel a második site-ot, a betát
tutor-l11-s2 = Ellenőrizd mindkét site-ot
tutor-l12-s1 = Tervezd meg egy gép hozzáadását
tutor-l13-s1 = Tervezz föderációt
tutor-l14-title = Tartsd naprakészen a goatbuildert
tutor-l14-s1 = Kérdezd meg, létezik-e újabb goatbuilder
tutor-l15-title = Assay: ellenőrizd, hogyan épült fel egy goatnet
tutor-l15-text =
    A goatbuilder `assay` parancs ellenőrzi, hogy egy goatnet helyesen épült-e fel, réteg rétegre, az átvételi teszteket használva. Minden teszt három eredmény egyikét kapja:
    - PASS: a teszt sikeres volt.
    - FAIL: a goatnetnek problémája van.
    - UNKNOWN: semmi nem tudott válaszolni a tesztre.

    Csak a PASS számít. Bármelyik FAIL vagy UNKNOWN 3-as kilépési kódot eredményez.

    A `--plan` felsorolja a goatnetre vonatkozó teszteket, réteg szerint, és nem érint semmilyen gépet. Használd, hogy lásd, mit fedne le egy valódi assay. Az általános forma `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: egyeznek a verziók?
tutor-l16-text =
    A goatbuilder `survey` parancs egy menetben ellenőrzi, hogy ugyanazok a verziók futnak-e ott, ahol kellene:
    - Minden goatneten belül: minden gép a deklarált verziót futtatja-e, és azok a gépek, amelyeknek egyezniük kellene, valóban egyeznek-e?
    - A dev, a preprod és a prod között: egy sor komponensenként, egy oszlop goatnetenként.

    Egy különbséget, amit a release folyamat elvár, jelentenek, de az nem hiba. Egy példa erre egy új verzió, amely a devben van, de még nincs a prodban. Egy különbség, amit semmi nem deklarál, `difference not declared`-ként olvasható. Egy gép, amit a goatbuilder nem tud elolvasni, unknown, soha nem egyező.

    Futtasd le a gyakorló másolaton. A `dev`-je szándékosan le van maradva, így számíthatsz találatokra és 3-as kilépési kódra.
tutor-l16-s1 = Futtasd a survey-t a gyakorló másolaton
tutor-l17-title = Endstate: pontosan azt futtatja egy goatnet, amit deklarál?
tutor-l17-text =
    A goatbuilder `endstate` parancs két dolgot ellenőriz egy goatnetről:
    1. Minden szolgáltatás, amit deklarál, deployolva van, fut, egészséges, és helyesen csatlakozik.
    2. Semmi nem fut, amit nem deklarál. Egy nem deklarált szolgáltatást árvának neveznek.

    A goatbuilder CLI a site saját beállításaiból, nem egy külön listából dolgozza ki, mit kellene futtatnia egy site-nak.

    Egy ellenőrzést, amelynek nincs mit olvasnia, UNOBSERVED-ként jelölnek. Ez egy őszinte válasz, nem egy kudarc. Egy FAIL vagy egy árva 3-as kilépési kódot eredményez.

    Az általános forma `goatbuilder endstate <goatnet>`. Az alábbi parancs egy példa site-ot ellenőriz mentett adatokból, így nem érint semmilyen gépet.
tutor-l17-s1 = Ellenőrizd a példa site-ot
tutor-l18-title = Bónusz: légy kíváncsi
tutor-l18-text =
    Ez a lecke opcionális. Futtasd le az alábbi parancsot, és a leckék véget érnek.

    A goatbuilder CLI-nek van néhány kicsi, rejtett funkciója. Nincsenek benne a súgóban vagy ezekben a leckékben. Néhány olyan szavakra válaszol, amiket a régi Unix-felhasználók gyakran beírnak egy új programba, csak hogy lássák, mi történik. A legtöbb dolog, amit kipróbálsz, nem fog semmit tenni.

    Ez a lecke nem mondja meg, mik azok. Az a lényeg, hogy megtaláld őket.

    Az alábbi parancs felsorol minden hibát, amit a goatbuilder meg tud magyarázni. Ez egy jó hely, ahonnan elindulhatsz felfedezni.
tutor-l18-s1 = Sorold fel minden hibát, amit a goatbuilder meg tud magyarázni
tutor-heading = { $n }. lecke { $total }-ból · { $title }
tutor-setup = Először futtasd ezt egyszer ebben a terminálban:
tutor-not-recording = Ez a terminál nem rögzíti a futásaidat, így a tutor nem tudja kijelölni a lépéseidet. Futtasd ezt egyszer, majd futtasd a lépéseket:
tutor-type = Végezd el ezeket a lépéseket a terminálban ({ $met } { $total }-ból kész):
tutor-rerun = Minden lépés után futtasd újra a `goatbuilder tutor`-t. Ellenőrzi, mit tettél, és kijelöli a lépést.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Üdvözlünk a GOATBUILDER Tutor oldalán
tutor-bar-progress = Folyamat
tutor-bar-done = { $done } { $total }-ból lecke kész
tutor-after-step = { $met }. lépés { $total }-ból kész. Következő:
tutor-after-lesson = { $n }. lecke kész. Futtasd a `goatbuilder tutor`-t, hogy elindítsd a { $next }. leckét. Hogy újra elolvasd: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = { $n }. lecke kész, és a tutor is. Futtasd a `goatbuilder tutor`-t, hogy lásd készként megjelölve. Hogy újra elolvasd: `goatbuilder tutor --lesson { $n }`.

tutor-review = Befejezted ezt a leckét, így ez az oldal csak olvasásra való. Futtasd a `goatbuilder tutor`-t, hogy visszatérj oda, ahol voltál.
tutor-other-lessons = Hogy újra elolvasd egy leckét: `goatbuilder tutor --lesson 1`. Hogy mindet lásd: `goatbuilder tutor --list`.
tutor-l12-title = Adj hozzá egy gépet egy már futó site-hoz
tutor-l12-text =
    A goatbuilder `join-node` parancs hozzáad még egy gépet egy már futó site-hoz. Először kiír egy tervet, egy sorral minden fázishoz. Minden fázisnak van egy típusa:
    - tool: a goatbuilder futtat egy szkriptet.
    - ansible: a goatbuilder futtat egy playbookot.
    - gate: egy ellenőrzés, amely a következő fázis előtt sikeresnek kell lennie.
    - manual: ezt magad csinálod, és a futás megáll.

    A lenti parancsban van `--dry-run`. Ez olvassa ezt a checkoutot, és semmit nem változtat. A táblázat alatt olvasd el, mit csinálsz kézzel.
tutor-l13-title = Kapcsolódj egy másik szervezet goatnetjéhez
tutor-l13-text =
    A goatbuilder `federate` parancs összekapcsolja a goatnetedet egy goatnettel, amit egy másik szervezet futtat. Mindkét oldal felépíti a megállapodás saját felét. Kicserélitek a feleket, és mindkét oldal összerakja azokat ugyanazzá a megállapodássá.

    Erősítsd meg a másik oldal kulcsát egy másik módon, például egy hívással, amiben bízol. Egy kulcs, amely a csomagjukon belül érkezik, semmit nem bizonyít.

    A lenti parancsban van `--dry-run`, így semmit nem változtat. Egy valódi futás a 4. fázisnál megáll, mert azt a fázist kézzel csinálod.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ez a parancs nem része a { $n }. leckének. Futtasd a `goatbuilder tutor`-t, hogy lásd, hol vagy.
tutor-l1-s2 = Kérdezd a doctor parancsot egy goatnetről
tutor-l1-text =
    A goatbuilder CLI építi, telepíti és kiadja a GOAT szoftvert. Mielőtt használod, két paranccsal ellenőrizd a beállításodat:
    - A goatbuilder `validate` parancs ellenőrzi a konfigurációs fájlt, a `ops/goatbuilder/goatbuilder.yaml` fájlt. Csak ezt a checkoutot olvassa, és nem kell hozzá hálózat.
    - A goatbuilder `doctor` parancs azt ellenőrzi, ami egy valódi deployhoz kell: az eszközeidet, a bejelentkezéseidet, azt, hogy ez a checkout egyezik-e a main ággal, és hogy a goatbuilder hogyan éri el az egyes goatneteket. Egy goatnet egy GOAT környezet.

    A goatbuilder `doctor` parancs kiírja, hány ellenőrzés sikerült, majd minden problémát a javításával együtt. Ebben a leckében a probléma információ, nem hiba. Olvasd el a javításokat, de most nem kell alkalmaznod őket.

    A legtöbb leckéhez nem kell bejelentkezés. A 2–5., a 10., a 11. és a 16. lecke gyakorló másolatot használ. A 8., 9., 12., 13., 15., 17. és 18. lecke csak ezt a checkoutot olvassa. A 6. leckéhez `gh` és a mesh kell, a 7. leckéhez `gh`, a 14. leckéhez pedig a mesh. Egyik leckéhez sem kell 1Password.
tutor-l14-text =
    A goatbuilder CLI frissíteni tudja magát abból a példányból, amelyet a goatneted a `get.<site>.<zone>/goatbuilder/` címen szolgál ki.

    A goatbuilder `update --check` parancs csak olvas. Összeveti a verziódat a kiszolgált verzióval, és semmit nem változtat. A 0-s kilépési kód azt jelenti, hogy naprakész vagy. A 3-as kilépési kód azt jelenti, hogy van újabb verzió.

    Egy valódi frissítés, `--check` nélkül, ellenőrzi a letöltés ellenőrzőösszegét és aláírását, mielőtt bármit lecserél. Ezután egy lépésben cseréli ki a fájlt.

    A goatbuilder CLI-nek tudnia kell, melyik goatnetet kérdezze: add meg a `--site` kapcsolót azzal a goatnettel, vagy állítsd be a `GOATBUILDER_UPDATE_URL` változót. Annak a goatnetnek a meshén kell lenned. Ha nem vagy rajta, a `--check` azt mondja, hogy nem éri el a szervert, és megnevezi a címet, amellyel próbálkozott. Ez helyes eredmény ebben a leckében.
tutor-l15-s1 = Listázd az egy goatnetre vonatkozó teszteket
tutor-l6-text =
    Egy kiadás sorrendben három goatneten viszi végig az új verziót: dev, aztán preprod, aztán prod.

    1. Valaki megcímkézi a verziót a main ágon, és a CI megépíti.
    2. dev: a goatbuilder telepíti, és lefuttatja az elfogadási teszteket.
    3. preprod: a goatbuilder újra telepíti, újra lefuttatja a teszteket, és rögzíti az eredményeket.
    4. A verzió ezután meghatározott ideig vár a preprodon. Semmi nem kerül magától a prodra.
    5. prod: a várakozás után egy ember futtatja a kiadást a `--from prod` kapcsolóval. A goatbuilder CLI telepít a prodra, és mindhárom goatnetet ellenőrzi.

    A goatbuilder `promote status` parancs megmutatja, hol tart minden komponens. A jelölt egy tesztelés alatt álló verzió. A kiadás általános formája `goatbuilder release <component> --tag <tag> --from dev`. Az alábbi parancsok ennek a checkoutnak egy komponensét és egy példacímkét használnak. Mindegyikben ott van a `--dry-run`, így semmi nem kap címkét, és semmi nem települ.

    A próbafuttatásokhoz `gh` és a mesh kell. Ha nem vagy a repository adminja, írd a `GOAT_RC_ROLE_CHECK=0` értéket minden `release` parancs elé.
tutor-l9-s1 = Hozz létre valódi hibát egy nem létező goatnettel
tutor-l9-text =
    Amikor a goatbuilder elutasít valamit, kiír egy kódot. A `GB-` kezdetű kódok a goatbuildertől jönnek, az `E_` kezdetűek a goat CLI-től, az `X-` kezdetűek pedig egy másik eszköztől, amelyet a goatbuilder futtatott.

    A goatbuilder `explain` parancs egy kódot vagy egy hibasor szövegét kapja, és elmondja, mi történt, miért, hogyan javítsd, és hol olvashatsz többet.

    Ha olyan goatnetet kérsz, amelyet ez a checkout nem deklarál, valódi hibát kapsz, amelyet meg lehet magyarázni.
