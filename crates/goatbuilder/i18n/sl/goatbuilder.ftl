### goatbuilder — sl (sl)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Zgradi in dostavi okolje goat: deklariraj, konvergiraj, preveri — v okviru enega pogodbenega modela uvajanja
help-flag-output = Izhodni format: tabela za človeka; JSON ali YAML za program
help-flag-repo = Koren repozitorija. Privzeto je checkout, v katerem se zažene ta program
help-flag-yes = Potrdi deklarativni glagol brez poziva (za avtomatizacijo)
help-flag-frozen = Zavrni vsak zagon, ki bi moral checkout previti naprej (strogi C3)
help-flag-mock = Zaženi proti imeniku s fixturami namesto gostiteljev: brez registra, brez transporta, deterministično (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Jezik pomoči in razlag napak (oznaka, npr. fr, uk, de); izhod, ki ga bere cevovod, ostane angleški. Privzeto: jezik OS
help-cmd-validate = Preveri konfiguracijo glede na verzioniran shemo in medpoljska pravila, brez povezave
help-cmd-doctor = Preveri, kaj zagon potrebuje, preden se karkoli začne: odvisnosti, svežino checkouta, transport vsakega okolja in da se playbook vsake komponente razreši v določen ciljni način
help-cmd-status = Deklarirano proti zabeleženemu, po strojih: konvergirano, v čakanju ali neznano (C7, C8). Samo bere. Izhod 3, dokler kateri stroj ni konvergiran
help-cmd-deploy = Namesti pin, ki ga main deklarira, v eno okolje in to dokaži: iz pinov razreši komponento in skupine okolja, play in njegov ciljni način, vsako skrivnost, ki jo play šablonira, ter transport; zaženi play prek ansible-preflight; preberi deploy ledger z vsakega gostitelja in štej za končano šele, ko najnovejša vrstica navaja deklarirani digest z action=deploy (C7). Na fixturi (`--mock`) gostitelje nadomesti skriptirani konvergator
help-cmd-release = Pelji release candidate od oznake na main prek žive validacije na razvojnem goatnetu do stopnje preprod (ADR 1173); po soaku ga z `--from prod` diplomiraj, vključi v fleet:, namesti v prod in znova preveri vsako stopnjo. Korak za korakom nadomešča ops/scripts/rc.sh
help-cmd-rollback = Povrni nameščeno komponento na njen N-ti najnovejši ref, po vrsti artefakta, pri čemer je vir resnice ledger na gostitelju (ADR 1060). Nadomešča ops/scripts/rollback.sh: izhod 3 zavrnjeno kot nevarno (data, cert, forward-only, reversible:false), 4 z vodenjem (config, acl)
help-cmd-fastlane = En ukaz od združenega commita do vseh goatnetov (ADR 1127): premišljena izjema od lestvice. Korak za korakom nadomešča ops/scripts/fastlane.sh; zavrne nepripet digest ali odprt set diplomiranja in uvajanja izvaja prek tega programa
help-cmd-standup = Lokacija od nič do preverjene: faze standupa v vrstnem redu doktrine s fail-closed vrati na vsakem koncu, observability gostitelj kot vzporedna veja, rescue razredi in medgoatnet površine na koncu (ADR 1183). Korak za korakom nadomešča ops/scripts/goat-standup.sh. Na fixturi (`--mock`) skriptirani konvergator konvergira vsako komponento vsakega razreda, čas od deklariranega do preverjenega pa je mera enournega goatneta
help-cmd-join-node = Še en stroj razreda na živi lokaciji, deklariran → konvergiran → preverjen: izvedeno zaporedje iz docs/tutorials/02-first-federated-node.md kot oštevilčene faze z vrati (dosegljivost, ključ, najprej delo v repozitoriju, osnova, jedrni WireGuard pred vpisom in po njem, jedrna ACL ravnina kot vrata vpisa, converge razreda, sprejem)
help-cmd-federate = Suvereno povezovanje, deklarirano, konvergirano in preverjeno (ADR 1157): izmerjena naša polovica, paketi izmenjani zunaj pasu, sestavljen in preverjen en sporazum, iz njega izdani listi, pravila izrisana z enim pisalcem, nameščen rob, odprt transport, ki ga je izbral sporazum, sprejem v vseh štirih nivojih. Korak za korakom nadomešča ops/scripts/peering-standup.sh
help-cmd-pins = ENO razreševanje z DVEMA izrisoma: pins.yml v group_vars vsakega navedenega člana in artefakt IntentV1 vsakega navedenega goatneta, iz ops/pins/fleet-pins.yaml. Bajt za bajtom nadomešča ops/pins/generate-pins.py. `--check` preveri, znova generira in zavrne odstopanje, nepodpisan ali spremenjen intent ter upravljan pin, definiran zunaj pins.yml (izhod 1)
help-cmd-promote = Vodi in opazuj diplomiranje izdaj po lestvici goatnetov (ADR 1120, 1125): status, odpri kandidata, diplomiraj, diplomiraj set, pot hotfix, pregled, pripravljenost, opusti, preveri. Korak za korakom nadomešča ops/release/graduate.py. Nič tu ne diplomira samo od sebe: sprememba teče le ob klicu, izpiše, kaj je naredila, brez terminala pa zahteva --yes (C10, ADR 1177)
help-cmd-explain = Razloži napako: kodo (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) ali poljubno besedilo napake — lastno temu programu ali besede orodja, ki jih je prenesel (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Izpiše, kaj se je zgodilo, zakaj, kaj storiti in kje brati več; `-` prebere besedilo iz stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Lokacija od preverjenega stanja do nič — zrcalo standupa, zapisi ostanejo (razgradnja doda vrstice teardown, nikoli ne briše). Na fixturi (`--mock`) razgradnjo vadi v obratnem vrstnem redu faz; na pravih gostiteljih izpiše vrstni red in zavrne izvedbo, dokler vsaka faza ne deklarira svojega teardown playa (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekcije v slogu vimtutorja: vsaka navede točen ukaz, ki ga zaženeš na zavrženi kopiji fixture (ali, samo za branje, na tem checkoutu), nato preveri, kaj je ukaz res naredil — izhodno kodo in vrstice — preden se odklene naslednja. `tutor` prikaže trenutno lekcijo in jo označi za opravljeno, ko jo tvoji zagoni izpolnijo; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skrito (src/fun.rs)
help-cmd-argan = Skrito (src/fun.rs)
help-cmd-bound = Skrito (crates/goat-fun)
help-cmd-secrets = Deklarirana drža skrivnosti (ADR 1204): kateri store velja (1password, openbao, sops, git-crypt, env-file, env), kaj je na voljo, kako dobaviti manjkajoče in lastni pregledi storea. `init --env <goatnet>` zapiše env datoteko za quickstart brez SaaS: močno naključno vrednost za vsako spremenljivko, ki jo plays goatneta predlagajo, 0600, v gitignore, obstoječe vrednosti nikoli ne zamenja. Isti modul kot ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Prebere najnovejšo vrstico deploy-ledgerja vsakega gostitelja za eno komponento in pove, ali navaja ref, ki ga deklarira main (C7). Samo bere; izhod 3, dokler je kateri gostitelj v stanju pending ali unknown. Isto preverjanje izvede `deploy --verify-only`
help-cmd-diff = Kaj se zdaj razlikuje od maina (C8): deklarirani ref maina proti najnovejši vrstici ledgerja vsakega gostitelja, za eno komponento ali, brez nje, za vse komponente okolja, prebrano v enem prehodu na skupino članov. Samo bere; izhod 3 ob kakršnem koli odstopanju ali neberljivem gostitelju
help-cmd-history = Deploy ledger kot časovnica, najnovejše prvo, na vseh gostiteljih okolja (C8): vsaka vrstica je ločen zapis s strojem, časom, dejanjem (deploy ali rollback) in verzijo. Prejšnja verzija zapisa in operater, ki ga je izvedel, sta v `-o json`. Samo bere
help-cmd-plan = Zapiše, kaj bi deklarativni ukaz naredil, v plan datoteko za pregled: deklarirani refi, play in cilj, ledger, commit maina, transport (deploy); vrsta artefakta in ciljni ref (rollback). Ne spremeni stanja nobenega gostitelja. Pregledan plan zaženite z `apply`
help-cmd-apply = Izvede natanko pregledan plan. Zavrne (izhod 1, z navedbo, kaj se je premaknilo), če se je od zapisa plana premaknila deklaracija maina, commit pinov, play ali cilj rollbacka; spremenjen transport je javljen, ne zavrnjen
help-cmd-adopt = Deklarira, kar okolje že izvaja: prebere ledgerje vseh gostiteljev in predlaga odstopanje pinov za vsako komponento, katere tekoči ref se razlikuje od deklaracije maina. Privzeto poročilo; z --yes se popravki zapišejo v svež worktree maina za pregled. Nikoli ne izvede deploya

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Še ne: { $why }
tutor-lesson-done = Lekcija { $n } opravljena: { $title }.
tutor-all-done-reset = Vse lekcije so opravljene. `goatbuilder tutor --reset` začne znova; vodniki gredo dlje: docs/tutorials/.
tutor-all-done = Vse lekcije so opravljene. Vodniki gredo dlje: docs/tutorials/.
tutor-workspace = Delovni prostor: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Plast za plastjo preveri, ali je bil goatnet pravilno zgrajen: vsako prevzemno preverjanje, ki velja za lokacijo, vključno s preverjanji, ki jih operater zažene z goat CLI. Prevzemna faza standupa kot samostojen ukaz. Samo bere; izhod 3, dokler katero preverjanje ne uspe ali ne more odgovoriti
help-flag-offline = Izolirana lokacija: ne pridobivaj glavne veje (trunk); delovno kopijo primerjaj s predpomnjeno in zagon zabeleži kot brez povezave (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Preveri, ali je goatnet v deklariranem končnem stanju: vsaka storitev, ki jo njegove lastne deklaracije navajajo, je nameščena, teče, je zdrava in opravlja svojo nalogo, njeni zero-trust robovi so povezani in nič ne teče, česar ne deklarira. Samo bere; izhod 3, dokler karkoli ne ustreza
help-cmd-survey = Kako skladna je programska oprema znotraj vsakega goatneta in med goatneti lestvice, prebrano v enem vzporednem prehodu. Bere samo
help-cmd-survey-long =
    Kako skladna je programska oprema znotraj vsakega goatneta in med goatneti lestve, prebrano v enem vzporednem prehodu. Samo bere. Znotraj goatneta: ref, ki ga razglaša main, v primerjavi z najnovejšo vrstico ledgerja vsakega gostitelja in vsebniki, ki jih ta poganja iz te slike (tekoči vsebnik; ustavljeni se presoja le na toplem rezervnem gostitelju, kjer nič ne teče), gostitelji enega goatneta, ki se ne ujemajo, goat CLI na vsakem gostitelju v primerjavi s tistim, ki ga ponuja njegov goatnet, in kar ponuja goat-get v primerjavi z main. Med goatneti: ena vrstica na komponento, en stolpec na goatnet, z oceno lestve za vsako razliko, kot jo poda promote status. Izhod 3 ob vsakem odmiku ali čemer koli neprebranem; razlika, ki jo razglaša lestev, se sporoči, a ne šteje za napako.
help-cmd-update = Zamenja to binarno datoteko z izdajo, ki jo goatnet ponuja na `get.<site>.<zone>/goatbuilder/`: prenos se preveri glede na njegov `.sha256` in podpisani `SHA256SUMS`, preden se karkoli spremeni, zamenjava pa je atomska. Že posodobljeno: ena vrstica, izhod 0. `--check` samo bere (izhod 3, ko zaostaja). Vir: `GOATBUILDER_UPDATE_URL`, sicer `--site` iz njegovih group_vars, sicer tisti iz zadnje posodobitve

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Preden začnete: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Obarva glif ok/FAIL in sodbo v tabeli (auto|always|never). `auto` obarva samo terminal; NO_COLOR in CLICOLOR_FORCE prevladata. JSON in YAML nista nikoli obarvana
help-flag-verbose = Na stderr zapiše, kaj izvede zagon — zunanje ukaze, ki jih požene, in omrežno fazo ukaza `update` — tako da `-o json|yaml` ostane čist dokument. Ali nastavi RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunde, po katerih se faza povezave/dosegljivosti hitro prekine — edina faza, ki se lahko zatakne pri nedosegljivem gostitelju (`update`-ov prenos objavljene različice). NE omeji izvajanja posamezne igre (play) ali prenosa binarne datoteke (omejen z zastojem). Prebere tudi GOATBUILDER_TIMEOUT
help-cmd-completion = Izpiše skript za dopolnjevanje lupine na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Za žive kandidate, ki jih statičen skript ne more poznati — lokacije, komponente in goatnete, ki jih ta checkout deklarira — poveži kavelj z `goatbuilder _complete` (glej glavo ustvarjenega skripta)
help-cmd-docs = Referenca ukazov: `goatbuilder docs` kaže nanjo; `--markdown` izpiše celotno referenco, zgrajeno iz živega drevesa ukazov, zato se nikoli ne razlikuje od `--help`
help-cmd-_complete = Skrito: izpiše žive kandidate za dopolnjevanje za kavlje lupine, po enega v vrstici (lokacije, komponente in goatnete, ki jih ta checkout deklarira). Samo bere, nikoli ne javlja glasnih napak — kavelj za dopolnjevanje ne sme pokvariti poziva

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Pred uvedbo: izpis pripravljenosti in povratnosti (ADR 1211): tri dejstva — State (stateless | stateful), Rollback (pin-revert | restore-required | none) in Scope (single | coupled) — deklarirana za vsako komponento in ob manjkajoči deklaraciji FAIL-CLOSED; del o pripravljenosti vsebuje to, kar dokazujejo drugi ukazi (ref je mogoče potegniti, pins --check, vrata za skrivnosti, svež vir, deklarirano končno stanje), poleg tega pa preveri dejstvo o povratku (rollback --dry-run za pin-revert, retencijsko okno za restore-required). `deploy` ga požene prvega; ocena po pošteni presoji, nikoli lažno zelen. Samo bere; izhodna koda 3, če ni čisto

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Preveri svojo nastavitev
tutor-l1-s1 = Preveri konfiguracijsko datoteko
tutor-l2-title = Branje stanja: converged, pending, unknown
tutor-l2-text =
    Ukaz goatbuilder `status` za vsak stroj primerja dve stvari: verzijo, ki jo zanj deklarira main, in verzijo, za katero njegov lastni zapis pravi, da teče. Nič ne spremeni.

    Vsak stroj je v enem od treh stanj:
    - converged: njegov zapis navaja deklarirano verzijo. To je opravljeno.
    - pending: njegov zapis navaja starejšo verzijo, ali pa še nič.
    - unknown: goatbuilder ne more prebrati njegovega zapisa.

    Kot opravljeno šteje samo converged. Če je kateri stroj pending ali unknown, se `status` konča z izhodno kodo 3.

    Od te lekcije naprej `--mock` goatbuilder usmeri na vadbeno kopijo v tvoji mapi tutorja namesto na resnične stroje. Nič, kar narediš tam, ne doseže resničnega stroja.

    Na `dev` vadbene kopije še ni bilo nič nameščenega. Pričakuj torej štiri stroje pending in enega unknown. Stroje converged boš videl v lekciji 3.
tutor-l2-s1 = Preberi stanje dev
tutor-l3-title = Namestitev: načrtuj, izvedi, dokaži
tutor-l3-text =
    Nameščanje postavi deklarirano verzijo na vsak stroj, nato prebere zapis vsakega stroja, da to dokaže. Namestitev je opravljena šele, ko zapis vsakega stroja navaja to verzijo. Namestitev, ki je potekla brez napak, ni dovolj.

    `portal` boš namestil na `preprod` vadbene kopije. `--dry-run` pokaže, kaj bi se zgodilo, in nič ne spremeni. `--ack` zapiše, kdo je to naredil; ukaz tvoje ime vpiše iz gita. Po namestitvi naj `status` pokaže oba stroja kot `ok` in converged.
tutor-l3-s1 = Načrtuj namestitev
tutor-l3-s2 = Namesti portal na preprod
tutor-l3-s3 = Preveri, da sta oba stroja converged
tutor-l4-title = Odstopanje in zgodovina
tutor-l4-text =
    Dva ukaza ti pomagata razumeti goatnet pred spremembo in po njej. Oba samo bereta.
    - Ukaz goatbuilder `diff` izpiše vsak stroj, katerega zapis se razlikuje od deklarirane verzije. Tako razliko imenujemo odstopanje.
    - Ukaz goatbuilder `history` izpiše vsako verzijo, ki jo je komponenta imela na vsakem stroju, od najnovejše.

    Na `dev` je vsak stroj zadaj. Zgodovina `portal` na `preprod` zdaj že vsebuje tvojo namestitev iz lekcije 3.
tutor-l4-s1 = Izpiši odstopanje na dev
tutor-l4-s2 = Preberi zgodovino portal na preprod
tutor-l5-title = Povrni verzijo in poglej, kako je nevarna povrnitev zavrnjena
tutor-l5-text =
    Povrnitev deklarira zgodnejšo verzijo in jo namesti tako kot vsako drugo verzijo. goatbuilder CLI nato prebere zapis vsakega stroja, da to dokaže.

    Nekaterih sprememb se tako ne da razveljaviti. Namestitev starejše verzije ne more razveljaviti podatkov, ki jih je komponenta zapisala, certifikata, ali spremembe, ki gre samo naprej. goatbuilder CLI take povrnitve zavrne z izhodno kodo 3 in pove, kaj storiti namesto tega.

    Komponenta `db-schema` vadbene kopije vsebuje podatke, zato jo goatbuilder zavrne povrniti. Ta zavrnitev je pričakovan rezultat.
tutor-l5-s1 = Načrtuj povrnitev
tutor-l5-s2 = Povrni portal eno verzijo nazaj
tutor-l5-s3 = Poskusi povrniti db-schema, kar goatbuilder zavrne
tutor-l6-title = Izdaja nove verzije: dev, nato preprod, nato prod
tutor-l6-s1 = Poglej, kje je vsaka komponenta
tutor-l6-s2 = Zaženi suh zagon izdaje iz dev
tutor-l6-s3 = Zaženi suh zagon koraka prod
tutor-l7-title = Fastlane, in kdaj ga ne uporabiti
tutor-l7-text =
    Fastlane postavi eno verzijo na vse goatnete hkrati. Izpusti čakanje na preprodu in zapise izdaje.

    Uporabi ga samo za nujen popravek, ki ga lahko razveljaviš z vrnitvijo na prejšnjo verzijo. Nikoli ga ne uporabi za spremembo podatkovne baze, spremembo identitete ali dostopa, ali za kar koli, česar se tako ne da razveljaviti.

    Fastlane zavrne verzijo, ki je CI ni zgradil, in komponento, ki je del izdaje, ki še poteka.

    Splošna oblika je `goatbuilder fastlane <component> --ref <version> --reason <why>`. Zaženi primer spodaj. Ima `--dry-run`, zato se nič ne spremeni. Potrebuje `gh`.
tutor-l7-s1 = Zaženi suh zagon fastlane
tutor-l8-title = Pini: najdi odstopanje in ga popravi na pravi način
tutor-l8-text =
    Ena datoteka določa, katero verzijo poganja vsak goatnet: `ops/pins/fleet-pins.yaml`. goatbuilder CLI iz nje generira druge datoteke verzij: datoteke `pins.yml` in podpisane datoteke intent.

    Ukaz goatbuilder `pins --check` primerja generirane datoteke z `fleet-pins.yaml` in zavrne vsako datoteko, ki se razlikuje. Ukaz goatbuilder `pins` jih znova generira.

    Tvoja mapa tutorja ima kopijo repozitorija, v kateri je nekdo eno datoteko `pins.yml` uredil ročno. Ročne spremembe nikoli ne popravljaj z drugo ročno spremembo. Spremeni `fleet-pins.yaml`, nato znova generiraj.
tutor-l8-s1 = Preveri ta checkout
tutor-l8-s2 = Preveri kopijo in najdi ročno spremembo
tutor-l8-s3 = Znova generiraj datoteke kopije
tutor-l8-s4 = Preveri kopijo znova
tutor-l9-title = Razloži napako
tutor-l9-s2 = Razloži napako po njeni kodi
tutor-l9-s3 = Razloži jo po njenem besedilu
tutor-l10-title = Vodnik 1: zgradi prvo lokacijo novega goatneta
tutor-l10-text =
    Ta lekcija zgradi cel goatnet v vadbeni kopiji. Primer goatneta je `zagros`. Njegova prva lokacija, `alpha`, ima dva core stroja, en nadzorni stroj in en reševalni stroj.

    Ukaz goatbuilder `standup` postavi vsak stroj v določenem vrstnem redu in preveri vsak zapis. Njegova zadnja vrstica pove, da je lokacija opravljena. Na resničnih strojih ta vrstica tudi pokaže, kako dolgo je trajalo.

    Celoten vodnik je `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Zgradi prvo lokacijo, alpha
tutor-l10-s2 = Preveri rezultat
tutor-l11-title = Vodnik 2: dodaj drugo lokacijo
tutor-l11-text =
    Ta lekcija se začne, kjer se je vodnik 1 končal. Doda drugo lokacijo, `beta`, v novi vadbeni kopiji, `zagros-2`.

    Druga lokacija prekopira svoje uporabniške račune iz prve lokacije. Njen standup tudi enkrat spremeni prvo lokacijo, da kopiranje deluje v obe smeri. To naredi korak z imenom `alpha/core`.

    Celoten vodnik je `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Zgradi drugo lokacijo, beta
tutor-l11-s2 = Preveri obe lokaciji
tutor-l12-s1 = Načrtuj dodajanje enega stroja
tutor-l13-s1 = Načrtuj federacijo
tutor-l14-title = Posodabljaj goatbuilder
tutor-l14-s1 = Vprašaj, ali obstaja novejši goatbuilder
tutor-l15-title = Assay: preveri, kako je bil goatnet zgrajen
tutor-l15-text =
    Ukaz goatbuilder `assay` preveri, da je bil goatnet zgrajen pravilno, plast za plastjo, s sprejemnimi testi. Vsak test dobi enega od treh rezultatov:
    - PASS: test je uspel.
    - FAIL: goatnet ima težavo.
    - UNKNOWN: na test ni mogel odgovoriti nihče.

    Šteje samo PASS. Vsak FAIL ali UNKNOWN povzroči izhodno kodo 3.

    `--plan` izpiše teste, ki veljajo za goatnet, po plasteh, in se ne dotakne nobenega stroja. Uporabi ga, da vidiš, kaj bi pokril resnični assay. Splošna oblika je `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: so verzije skladne?
tutor-l16-text =
    Ukaz goatbuilder `survey` v enem prehodu preveri, ali enake verzije tečejo tam, kjer bi morale:
    - Znotraj vsakega goatneta: ali vsak stroj poganja deklarirano verzijo, in ali se stroji, ki bi se morali ujemati, res ujemajo?
    - Med dev, preprod in prod: ena vrstica na komponento, en stolpec na goatnet.

    Razlika, ki jo postopek izdaje pričakuje, je sporočena, a ni napaka. Primer je nova verzija, ki je na dev, a še ni na prod. Razlika, ki je nič ne deklarira, se bere kot `difference not declared`. Stroj, ki ga goatbuilder ne more prebrati, je unknown, nikoli consistent.

    Zaženi ga na vadbeni kopiji. Njen `dev` je zadaj namenoma, zato pričakuj ugotovitve in izhodno kodo 3.
tutor-l16-s1 = Zaženi survey na vadbeni kopiji
tutor-l17-title = Endstate: ali goatnet poganja natanko to, kar deklarira?
tutor-l17-text =
    Ukaz goatbuilder `endstate` pri goatnetu preveri dve stvari:
    1. Vsaka storitev, ki jo deklarira, je nameščena, teče, je zdrava in pravilno povezana.
    2. Nič ne teče, česar ne deklarira. Nedeklarirano storitev imenujemo sirota.

    goatbuilder CLI ugotovi, kaj bi morala lokacija poganjati, iz nastavitev lokacije same, ne iz ločenega seznama.

    Preverjanje, ki nima nič za branje, je označeno UNOBSERVED. To je pošten odgovor, ne neuspeh. FAIL ali sirota povzročita izhodno kodo 3.

    Splošna oblika je `goatbuilder endstate <goatnet>`. Ukaz spodaj preveri primer lokacije iz shranjenih podatkov, zato se ne dotakne nobenega stroja.
tutor-l17-s1 = Preveri primer lokacije
tutor-l18-title = Bonus: bodi radoveden
tutor-l18-text =
    Ta lekcija je neobvezna. Zaženi ukaz spodaj in tutor je končan.

    goatbuilder CLI ima nekaj majhnih skritih funkcij. Niso v pomoči niti v teh lekcijah. Nekatere se odzovejo na besede, ki jih dolgoletni uporabniki Unixa pogosto vtipkajo v nov program, samo da vidijo, kaj se zgodi. Večina stvari, ki jih poskusiš, ne naredi nič.

    Ta lekcija ne pove, kaj so. Poanta je, da jih najdeš.

    Ukaz spodaj izpiše vsako napako, ki jo goatbuilder zna razložiti. To je dobro mesto za začetek raziskovanja.
tutor-l18-s1 = Izpiši vsako napako, ki jo goatbuilder zna razložiti
tutor-heading = Lekcija { $n } od { $total } · { $title }
tutor-setup = Najprej to enkrat zaženi v tem terminalu:
tutor-not-recording = Ta terminal ne zapisuje tvojih zagonov, zato tutor ne more odkljukati tvojih korakov. Zaženi to enkrat, nato zaženi korake:
tutor-type = Naredi te korake v svojem terminalu ({ $met } od { $total } opravljenih):
tutor-rerun = Po vsakem koraku zaženi `goatbuilder tutor` znova. Preveri, kaj si naredil, in odkljuka korak.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Pozdravljen v GOATBUILDER Tutor
tutor-bar-progress = Napredek
tutor-bar-done = { $done } od { $total } lekcij opravljenih
tutor-after-step = Korak { $met } od { $total } opravljen. Naslednji:
tutor-after-lesson = Lekcija { $n } je opravljena. Zaženi `goatbuilder tutor`, da začneš lekcijo { $next }. Da jo prebereš znova: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekcija { $n } je opravljena, in tudi tutor. Zaženi `goatbuilder tutor`, da vidiš, da je označeno kot opravljeno. Da jo prebereš znova: `goatbuilder tutor --lesson { $n }`.

tutor-review = Ta lekcija je končana, zato je ta stran samo za branje. Zaženi `goatbuilder tutor`, da se vrneš, kje si.
tutor-other-lessons = Da lekcijo prebereš znova: `goatbuilder tutor --lesson 1`. Da jih vidiš vse: `goatbuilder tutor --list`.
tutor-l12-title = Dodaj stroj v delujočo lokacijo
tutor-l12-text =
    Ukaz goatbuilder `join-node` doda še en stroj na lokacijo, ki že teče. Najprej izpiše načrt, z eno vrstico na fazo. Vsaka faza ima tip:
    - tool: goatbuilder zažene skript.
    - ansible: goatbuilder zažene playbook.
    - gate: preverjanje, ki mora uspeti, preden se nadaljuje na naslednjo fazo.
    - manual: to narediš sam, in zagon se ustavi.

    Spodnji ukaz ima `--dry-run`. Bere ta checkout in nič ne spremeni. Pod tabelo preberi, kaj narediš ročno.
tutor-l13-title = Poveži se z goatnetom druge organizacije
tutor-l13-text =
    Ukaz goatbuilder `federate` poveže tvoj goatnet z goatnetom, ki ga poganja druga organizacija. Vsaka stran zgradi svojo polovico dogovora. Izmenjaš polovici, in vsaka stran ju združi v isti dogovor.

    Potrdi ključ druge strani na ločen način, na primer s klicem, ki mu zaupaš. Ključ, ki prispe znotraj njihovega paketa, ne dokazuje nič.

    Spodnji ukaz ima `--dry-run`, zato nič ne spremeni. Resničen zagon se ustavi pri fazi 4, ker to fazo narediš ročno.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ta ukaz ni del lekcije { $n }. Zaženi `goatbuilder tutor`, da vidiš, kje si.
tutor-l1-s2 = Vprašaj doctor o enem goatnetu
tutor-l1-text =
    CLI goatbuilder gradi, nameša in izdaja programsko opremo GOAT. Preden ga uporabiš, preveri svojo nastavitev z dvema ukazoma:
    - Ukaz goatbuilder `validate` preveri konfiguracijsko datoteko `ops/goatbuilder/goatbuilder.yaml`. Bere samo ta checkout in ne potrebuje omrežja.
    - Ukaz goatbuilder `doctor` preveri, kaj potrebuje pravo nameščanje: tvoja orodja, tvoje prijave, ali se ta checkout ujema z main in kako goatbuilder doseže vsak goatnet. Goatnet je eno okolje GOAT.

    Ukaz goatbuilder `doctor` izpiše, koliko preverjanj je uspelo, nato pa vsako težavo z njenim popravkom. V tej lekciji je težava informacija, ne napaka. Preberi popravke, vendar jih zdaj ni treba uporabiti.

    Večina lekcij ne potrebuje prijave. Lekcije od 2 do 5, 10, 11 in 16 uporabljajo vadbeno kopijo. Lekcije 8, 9, 12, 13, 15, 17 in 18 samo berejo ta checkout. Lekcija 6 potrebuje `gh` in mesh, lekcija 7 potrebuje `gh`, lekcija 14 pa potrebuje mesh. Nobena lekcija ne potrebuje 1Password.
tutor-l14-text =
    CLI goatbuilder se lahko posodobi iz kopije, ki jo tvoj goatnet streže na `get.<site>.<zone>/goatbuilder/`.

    Ukaz goatbuilder `update --check` samo bere. Primerja tvojo verzijo s streženo verzijo in nič ne spremeni. Izhodna koda 0 pomeni, da imaš najnovejšo verzijo. Izhodna koda 3 pomeni, da obstaja novejša verzija.

    Prava posodobitev, brez `--check`, preveri kontrolno vsoto in podpis prenosa, preden karkoli zamenja. Nato zamenja datoteko v enem koraku.

    CLI goatbuilder mora vedeti, kateri goatnet naj vpraša: podaj `--site` s tem goatnetom ali nastavi `GOATBUILDER_UPDATE_URL`. Biti moraš na meshu tega goatneta. Če nisi, `--check` pove, da ne more doseči strežnika, in navede naslov, ki ga je poskusil. To je v tej lekciji pravilen rezultat.
tutor-l15-s1 = Izpiši teste, ki veljajo za en goatnet
tutor-l6-text =
    Izdaja premakne novo verzijo skozi tri goatnete po vrsti: dev, nato preprod, nato prod.

    1. Nekdo verzijo na main označi z oznako, CI pa jo zgradi.
    2. dev: goatbuilder jo namesti in zažene prevzemne teste.
    3. preprod: goatbuilder jo znova namesti, znova zažene teste in zapiše rezultate.
    4. Verzija nato določen čas čaka na preprod. Nič se ne premakne v prod samo od sebe.
    5. prod: po tem čakanju oseba zažene izdajo z `--from prod`. CLI goatbuilder namesti na prod in preveri vse tri goatnete.

    Ukaz goatbuilder `promote status` pokaže, kje je vsaka komponenta. Kandidat je verzija, ki se testira. Splošna oblika izdaje je `goatbuilder release <component> --tag <tag> --from dev`. Spodnji ukazi uporabljajo eno komponento tega checkouta in primer oznake. Vsak ima `--dry-run`, zato nič ni označeno ali nameščeno.

    Poskusni zagoni potrebujejo `gh` in mesh. Če nisi skrbnik repozitorija, pred vsak ukaz `release` postavi `GOAT_RC_ROLE_CHECK=0`.
tutor-l9-s1 = Povzroči pravo napako z goatnetom, ki ne obstaja
tutor-l9-text =
    Ko goatbuilder zavrne, izpiše kodo. Kode, ki se začnejo z `GB-`, prihajajo iz goatbuilderja, `E_` iz CLI goat, `X-` pa iz drugega orodja, ki ga je zagnal goatbuilder.

    Ukaz goatbuilder `explain` sprejme kodo ali besedilo vrstice z napako in ti pove, kaj se je zgodilo, zakaj, kako to popraviti in kje prebrati več.

    Če zahtevaš goatnet, ki ga ta checkout ne deklarira, dobiš pravo napako za razlago.
