### goatbuilder — hr (hr)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Izgradi i isporuči goat okruženje: deklariraj, konvergiraj, provjeri — pod jednim deploy ugovorom
help-flag-output = Format izlaza: tablica za čovjeka; JSON ili YAML za program
help-flag-repo = Korijen repozitorija. Zadano: checkout u kojem se ovaj binarni program pokreće
help-flag-yes = Potvrdi deklarativni glagol bez upita (za automatizaciju)
help-flag-frozen = Odbij svako pokretanje koje bi zahtijevalo fast-forward checkouta (strogi C3)
help-flag-mock = Pokreni nad direktorijem fixturea umjesto nad hostovima: bez registryja, bez transporta, deterministički (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Jezik pomoći i objašnjenja grešaka (oznaka poput fr, uk, de); izlaz koji čita pipeline ostaje na engleskom. Zadano: locale OS-a
help-cmd-validate = Provjeri konfiguraciju prema verzioniranoj shemi i pravilima među poljima, offline
help-cmd-doctor = Provjeri što pokretanju treba prije nego išta krene: ovisnosti, svježinu checkouta, transport svakog okruženja i da se playbook svake komponente razrješava u određeni target mode
help-cmd-status = Deklarirano naspram zabilježenog, po stroju: converged, pending ili unknown (C7, C8). Samo čita. Izlaz 3 dok god neki stroj nije converged
help-cmd-deploy = Postavi pin koji main deklarira na jedno okruženje i dokaži to: iz pinova razriješi komponentu i grupe okruženja, play i njegov target mode, svaku tajnu koju play templatira, transport; pokreni play kroz ansible-preflight; pročitaj deploy ledger sa svakog hosta i smatraj gotovim tek kad najnoviji redak navodi deklarirani digest s action=deploy (C7). Na fixtureu (`--mock`) hostove zamjenjuje skriptirani converger
help-cmd-release = Provedi release candidate od taga na mainu preko žive validacije dev goatneta do preprod stepenice (ADR 1173); nakon soaka, s `--from prod`, promoviraj ga, uključi u fleet:, deployaj prod i ponovno provjeri svaku stepenicu. Zamjenjuje ops/scripts/rc.sh korak po korak
help-cmd-rollback = Vrati deployanu komponentu na njezin N-ti najnoviji ref, prema tipu artefakta, uz on-host ledger kao izvor istine (ADR 1060). Zamjenjuje ops/scripts/rollback.sh: izlaz 3 odbijeno kao nesigurno (data, cert, forward-only, reversible:false), 4 vođeno (config, acl)
help-cmd-fastlane = Jedna naredba od mergeanog commita do svakog goatneta (ADR 1127): namjerna iznimka od ljestvice. Zamjenjuje ops/scripts/fastlane.sh korak po korak; odbija nepinirani digest ili otvoreni skup promocije, a deploye pokreće kroz ovaj binarni program
help-cmd-standup = Lokacija od nule do provjerene: faze standupa u redoslijedu doktrine s fail-closed vratima na oba kraja, observability host kao paralelna grana, rescue klase i cross-goatnet površine na kraju (ADR 1183). Zamjenjuje ops/scripts/goat-standup.sh korak po korak. Na fixtureu (`--mock`) svaku komponentu svake klase konvergira skriptirani converger, a vrijeme od deklariranog do provjerenog mjera je jednosatnog goatneta
help-cmd-join-node = Još jedan stroj klase na živoj lokaciji, deklariran → converged → provjeren: izvršeni slijed iz docs/tutorials/02-first-federated-node.md kao numerirane faze s vratima (dosegljivost, ključ, prvo rad na repou, baseline, kernel WireGuard prije i nakon enrolmenta, kernel ACL plane kao vrata enrolmenta, converge klase, prihvaćanje)
help-cmd-federate = Suvereno peeranje deklarirano, konvergirano i provjereno (ADR 1157): izmjerena naša polovica, paketi razmijenjeni izvan pojasa, jedan sporazum složen i provjeren, iz njega izdani leafovi, pravila renderirana jedinim writerom, edge deployan, otvoren transport koji je sporazum odabrao, prihvaćanje u sva četiri tiera. Zamjenjuje ops/scripts/peering-standup.sh korak po korak
help-cmd-pins = JEDAN prolaz razrješavanja s DVA renderiranja: group_vars pins.yml svakog navedenog člana i IntentV1 artefakt svakog navedenog goatneta, iz ops/pins/fleet-pins.yaml. Zamjenjuje ops/pins/generate-pins.py bajt za bajt. `--check` validira, regenerira i odbija drift, nepotpisani ili izmijenjeni intent te upravljani pin definiran izvan pins.yml (izlaz 1)
help-cmd-promote = Vodi i prati promociju releasea kroz ljestvicu goatneta (ADR 1120, 1125): status, otvaranje kandidata, promocija, promocija skupa, hotfix put, review, readiness, abandon, check. Zamjenjuje ops/release/graduate.py korak po korak. Ništa se ovdje ne promovira samo: mutacija se pokreće tek na poziv, ispisuje što je učinila i traži --yes bez terminala (C10, ADR 1177)
help-cmd-explain = Protumači grešku: kod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) ili bilo koji tekst greške — ovog programa ili riječi alata koje je prenio (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Ispisuje što se dogodilo, zašto, što učiniti i gdje čitati dalje; `-` čita tekst sa stdina

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Site od verificiranog do ničega — zrcalo standupa, zapisi ostaju (burn-down dodaje teardown retke, nikad ih ne briše). Na fixtureu (`--mock`) uvježbava burn-down obrnutim redoslijedom faza; na stvarnim hostovima ispisuje redoslijed i odbija izvršenje dok svaka faza ne deklarira svoj teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekcije u stilu vimtutora: svaka navodi točnu naredbu koju treba pokrenuti nad jednokratnom kopijom fixturea (ili, samo za čitanje, nad ovim checkoutom), a zatim provjerava što je ta naredba stvarno napravila — izlazni kod i retke — prije nego što se otključa sljedeća. `tutor` prikazuje lekciju na kojoj si i označava je gotovom kad je ispuniš svojim pokretanjima; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skriveno (src/fun.rs)
help-cmd-argan = Skriveno (src/fun.rs)
help-cmd-bound = Skriveno (crates/goat-fun)
help-cmd-secrets = Deklarirani stav o tajnama (ADR 1204): koji je store na snazi (1password, openbao, sops, git-crypt, env-file, env), što je dostupno, kako dostaviti ono što nedostaje i vlastite provjere storea. `init --env <goatnet>` piše env datoteku za zero-SaaS quickstart: jaku slučajnu vrijednost za svaku varijablu koju playovi goatneta templatiraju, 0600, gitignored, nikad ne rotira vrijednost koja već postoji. Isti modul kao ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Pročitaj najnoviji redak deploy-ledgera na svakom hostu za jednu komponentu i reci navodi li ref koji main deklarira (C7). Samo čita; izlaz 3 dok je bilo koji host na čekanju ili nepoznat. Ista provjera kao `deploy --verify-only`
help-cmd-diff = Što se trenutno razlikuje od maina (C8): ref koji main deklarira naspram najnovijeg retka ledgera svakog hosta, za jednu komponentu ili, bez nje, za sve komponente okruženja, čitano u jednom prolazu po grupi članova. Samo čita; izlaz 3 kod bilo kojeg odstupanja ili nečitljivog hosta
help-cmd-history = Deploy-ledger kao vremenska crta, najnovije prvo, na svim hostovima okruženja (C8): svaki redak je zaseban zapis sa strojem, vremenom, radnjom (deploy ili rollback) i verzijom. Prethodna verzija zapisa i operator koji ga je izvršio nalaze se u `-o json`. Samo čita
help-cmd-plan = Zapiši u plan-datoteku za pregled što bi deklarativni verb napravio: deklarirane refove, play i cilj, ledger, commit maina, transport (deploy); tip artefakta i ciljni ref (rollback). Ne dira stanje nijednog hosta. Pokreni pregledano s `apply`
help-cmd-apply = Izvrši točno pregledani plan. Odbija (izlaz 1, uz navod što se promijenilo) ako se otkad je plan napisan promijenila deklaracija maina, commit pinova, play ili cilj rollbacka; promijenjeni transport se prijavljuje, ne odbija
help-cmd-adopt = Deklariraj ono što okruženje već pokreće: pročitaj ledgere svakog hosta i predloži pins odstupanje za svaku komponentu čiji se pokrenuti ref razlikuje od deklaracije maina. Prema zadanom samo izvještaj; uz --yes izmjene se zapisuju u svježi worktree maina za pregled. Nikad ne deploya

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Još ne: { $why }
tutor-lesson-done = Lekcija { $n } gotova: { $title }.
tutor-all-done-reset = Sve lekcije su gotove. `goatbuilder tutor --reset` kreće ispočetka; tutoriji idu dalje: docs/tutorials/.
tutor-all-done = Sve lekcije su gotove. Tutoriji idu dalje: docs/tutorials/.
tutor-workspace = Radni prostor: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Provjerava je li goatnet ispravno izgrađen, sloj po sloj: sve pr
help-flag-offline = Izolirana lokacija: ne dohvaćaj trunk; ocijeni checkout prema keširanom i zabilježi izvršavanje kao offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Provjeri je li goatnet u deklariranom završnom stanju: svaki servis koji njegove vlastite deklaracije navode kao pokrenut je isporučen, pokrenut, ispravan i obavlja svoj posao, s povezanim zero-trust vezama, i ništa ne radi što nije deklarirano. Samo čita; izlazni kod 3 dok god nešto odstupa
help-cmd-survey = Koliko je softver dosljedan unutar svakog goatneta i među goatnetovima na ladderu, očitano u jednom paralelnom prolazu. Samo čita

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Koliko je softver dosljedan unutar svake goatnet mreže i među goatnet mrežama na ljestvici, očitano u jednom paralelnom prolazu. Samo čita.
help-cmd-update = Zamjenjuje ovu binarnu datoteku izdanjem koje goatnet poslužuje na `get.<site>.<zone>/goatbuilder/`: preuzimanje se provjerava prema njegovom `.sha256` i potpisanom `SHA256SUMS` prije nego što se išta dira, a zamjena je atomska. Već ažurno: jedan redak, izlaz 0. `--check` samo čita (izlaz 3 kad zaostaje). Izvor: `GOATBUILDER_UPDATE_URL`, inače `--site` iz njegovih group_vars, inače onaj iz zadnjeg ažuriranja

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Prije početka: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Oboji ok/FAIL glif i presudu u tablici (auto|always|never). `auto` boji samo terminal; NO_COLOR i CLICOLOR_FORCE imaju prednost. JSON i YAML nikad se ne boje
help-flag-verbose = Zapiši na stderr što pokretanje radi — vanjske naredbe koje poziva i mrežnu fazu naredbe `update` — da `-o json|yaml` ostane čist dokument. Ili postavi RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunde ograničenja za fazu povezivanja/dosega prije brzog neuspjeha — jedina faza koja se može blokirati na nedostupnom hostu (dohvat poslužene verzije u naredbi `update`). NE ograničava izvođenje samog playa ni preuzimanje binarne datoteke (ograničeno zastojem). Čita i GOATBUILDER_TIMEOUT
help-cmd-completion = Ispiši skriptu za dovršavanje naredbi ljuske na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Za žive kandidate koje statička skripta ne može znati — lokacije, komponente i goatnete koje ovaj checkout deklarira — poveži hook na `goatbuilder _complete` (vidi zaglavlje generirane skripte)
help-cmd-docs = Referenca naredbi: `goatbuilder docs` upućuje na nju; `--markdown` ispisuje cijelu referencu, generiranu iz živog stabla naredbi pa se nikad ne može razlikovati od `--help`
help-cmd-_complete = Skriveno: ispisuje žive kandidate za dovršavanje u hookovima ljuske, jedan po retku (lokacije, komponente i goatnete koje ovaj checkout deklarira). Samo čita, nikad ne javlja glasnu grešku — hook za dovršavanje ne smije pokvariti prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Prije rollouta, ispis spremnosti i reverzibilnosti (ADR 1211): tri činjenice — State (stateless | stateful), Rollback (pin-revert | restore-required | none) i Scope (single | coupled) — deklarirane po komponenti i FAIL-CLOSED kad nisu deklarirane, uz polovicu spremnosti koju dokazuju ostali glagoli (ref se može povući, pins --check, secrets gate, svjež izvor, deklarirano završno stanje) i provjeru činjenice o rollbacku (rollback --dry-run za pin-revert, retencijski prozor za restore-required). `deploy` ga pokreće prvi; ocjenjuje se po iskrenosti, nikad lažno zeleno. Samo čita; izlaz 3 kad nije čisto

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Provjeri svoju postavku
tutor-l1-s1 = Provjeri konfiguracijsku datoteku
tutor-l2-title = Čitanje stanja: converged, pending, unknown
tutor-l2-text =
    Naredba goatbuilder `status` za svaki stroj uspoređuje dvije stvari: verziju koju za njega deklarira main, i verziju za koju njegov vlastiti zapis kaže da je pokrenuta. Ništa ne mijenja.

    Svaki stroj je u jednom od tri stanja:
    - converged: njegov zapis navodi deklariranu verziju. To je gotovo.
    - pending: njegov zapis navodi stariju verziju, ili još ništa.
    - unknown: goatbuilder ne može pročitati njegov zapis.

    Samo converged se računa kao gotovo. Ako je neki stroj pending ili unknown, `status` izlazi s kodom 3.

    Od ove lekcije nadalje `--mock` usmjerava goatbuilder na kopiju za vježbu u tvojoj mapi tutora, umjesto na stvarne strojeve. Ništa što tamo napraviš ne dopire do stvarnog stroja.

    Na `dev` kopije za vježbu još ništa nije postavljeno. Očekuj zato četiri stroja pending i jedan unknown. Strojeve converged vidjet ćeš u lekciji 3.
tutor-l2-s1 = Pročitaj stanje dev
tutor-l3-title = Postavljanje: planiraj, pokreni, dokaži
tutor-l3-text =
    Postavljanje stavlja deklariranu verziju na svaki stroj, a zatim čita zapis svakog stroja da to dokaže. Postavljanje je gotovo samo kad zapis svakog stroja navodi tu verziju. Postavljanje koje je prošlo bez pogrešaka nije dovoljno.

    `portal` ćeš postaviti na `preprod` kopije za vježbu. `--dry-run` pokazuje što bi se dogodilo i ništa ne mijenja. `--ack` zapisuje tko je to učinio; naredba tvoje ime popunjava iz gita. Nakon postavljanja `status` bi trebao pokazati oba stroja kao `ok` i converged.
tutor-l3-s1 = Planiraj postavljanje
tutor-l3-s2 = Postavi portal na preprod
tutor-l3-s3 = Provjeri da su oba stroja converged
tutor-l4-title = Odstupanje i povijest
tutor-l4-text =
    Dvije naredbe pomažu ti razumjeti goatnet prije i nakon promjene. Obje samo čitaju.
    - Naredba goatbuilder `diff` ispisuje svaki stroj čiji se zapis razlikuje od deklarirane verzije. Takva razlika se zove odstupanje.
    - Naredba goatbuilder `history` ispisuje svaku verziju koju je komponenta imala na svakom stroju, od najnovije.

    Na `dev` je svaki stroj u zaostatku. Povijest `portal` na `preprod` sada već uključuje tvoje postavljanje iz lekcije 3.
tutor-l4-s1 = Ispiši odstupanje na dev
tutor-l4-s2 = Pročitaj povijest portal na preprod
tutor-l5-title = Vrati verziju i vidi kako se odbija nesigurno vraćanje
tutor-l5-text =
    Vraćanje deklarira raniju verziju i postavlja ju kao svaku drugu verziju. goatbuilder CLI zatim čita zapis svakog stroja da to dokaže.

    Neke promjene se tako ne mogu poništiti. Postavljanje starije verzije ne može poništiti podatke koje je komponenta zapisala, certifikat, ili promjenu koja ide samo unaprijed. goatbuilder CLI takva vraćanja odbija s kodom 3 i kaže što učiniti umjesto toga.

    Komponenta `db-schema` kopije za vježbu sadrži podatke, pa goatbuilder odbija je vratiti. Ovo odbijanje je očekivani rezultat.
tutor-l5-s1 = Planiraj vraćanje
tutor-l5-s2 = Vrati portal jednu verziju nazad
tutor-l5-s3 = Pokušaj vratiti db-schema, što goatbuilder odbija
tutor-l6-title = Izdavanje nove verzije: dev, zatim preprod, zatim prod
tutor-l6-s1 = Pogledaj gdje je svaka komponenta
tutor-l6-s2 = Pokreni suho pokretanje izdanja iz dev
tutor-l6-s3 = Pokreni suho pokretanje koraka prod
tutor-l7-title = Fastlane, i kad ga ne koristiti
tutor-l7-text =
    Fastlane stavlja jednu verziju na sve goatnete odjednom. Preskače čekanje na preprodu i zapise izdanja.

    Koristi ga samo za hitan popravak koji možeš poništiti povratkom na prethodnu verziju. Nikad ga ne koristi za promjenu baze podataka, promjenu identiteta ili pristupa, ni za ništa što se tako ne može poništiti.

    Fastlane odbija verziju koju CI nije izgradio, i komponentu koja je dio izdanja koje je još u tijeku.

    Opći oblik je `goatbuilder fastlane <component> --ref <version> --reason <why>`. Pokreni primjer ispod. Ima `--dry-run`, pa se ništa ne mijenja. Treba `gh`.
tutor-l7-s1 = Pokreni suho pokretanje fastlanea
tutor-l8-title = Pinovi: pronađi odstupanje i popravi ga na pravi način
tutor-l8-text =
    Jedna datoteka određuje koju verziju pokreće svaki goatnet: `ops/pins/fleet-pins.yaml`. goatbuilder CLI iz nje generira ostale datoteke verzija: datoteke `pins.yml` i potpisane datoteke intent.

    Naredba goatbuilder `pins --check` uspoređuje generirane datoteke s `fleet-pins.yaml` i odbija svaku datoteku koja se razlikuje. Naredba goatbuilder `pins` ih ponovno generira.

    Tvoja mapa tutora ima kopiju repozitorija u kojoj je netko ručno uredio jednu datoteku `pins.yml`. Ručnu promjenu nikad ne popravljaj drugom ručnom promjenom. Promijeni `fleet-pins.yaml`, pa onda ponovno generiraj.
tutor-l8-s1 = Provjeri ovaj checkout
tutor-l8-s2 = Provjeri kopiju i pronađi ručnu promjenu
tutor-l8-s3 = Ponovno generiraj datoteke kopije
tutor-l8-s4 = Provjeri kopiju ponovno
tutor-l9-title = Protumači pogrešku
tutor-l9-s2 = Protumači pogrešku prema njezinu kodu
tutor-l9-s3 = Protumači je prema njezinu tekstu
tutor-l10-title = Vodič 1: izgradi prvi site novog goatneta
tutor-l10-text =
    Ova lekcija gradi cijeli goatnet u kopiji za vježbu. Primjer goatneta je `zagros`. Njegov prvi site, `alpha`, ima dva core stroja, jedan nadzorni stroj i jedan spasilački stroj.

    Naredba goatbuilder `standup` postavlja svaki stroj određenim redoslijedom i provjerava svaki zapis. Njegov zadnji red kaže da je site gotov. Na stvarnim strojevima taj red i pokazuje koliko je to trajalo.

    Cijeli vodič je `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Izgradi prvi site, alpha
tutor-l10-s2 = Provjeri rezultat
tutor-l11-title = Vodič 2: dodaj drugi site
tutor-l11-text =
    Ova lekcija počinje gdje je vodič 1 završio. Dodaje drugi site, `beta`, u novoj kopiji za vježbu, `zagros-2`.

    Drugi site kopira korisničke račune s prvog sitea. Njegov standup također jednom mijenja prvi site, tako da kopiranje radi u oba smjera. To radi korak nazvan `alpha/core`.

    Cijeli vodič je `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Izgradi drugi site, beta
tutor-l11-s2 = Provjeri oba sitea
tutor-l12-s1 = Planiraj dodavanje jednog stroja
tutor-l13-s1 = Planiraj federaciju
tutor-l14-title = Održavaj goatbuilder ažurnim
tutor-l14-s1 = Pitaj postoji li noviji goatbuilder
tutor-l15-title = Assay: provjeri kako je goatnet izgrađen
tutor-l15-text =
    Naredba goatbuilder `assay` provjerava da je goatnet izgrađen pravilno, sloj po sloj, koristeći prihvatne testove. Svaki test dobiva jedan od tri rezultata:
    - PASS: test je prošao.
    - FAIL: goatnet ima problem.
    - UNKNOWN: ništa nije moglo odgovoriti na test.

    Samo PASS se računa. Svaki FAIL ili UNKNOWN uzrokuje izlazni kod 3.

    `--plan` ispisuje testove koji se odnose na goatnet, po slojevima, i ne dira nijedan stroj. Koristi ga da vidiš što bi pokrio stvarni assay. Opći oblik je `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: jesu li verzije dosljedne?
tutor-l16-text =
    Naredba goatbuilder `survey` u jednom prolazu provjerava teku li iste verzije tamo gdje bi trebale:
    - Unutar svakog goatneta: pokreće li svaki stroj deklariranu verziju, i podudaraju li se strojevi koji bi se trebali podudarati?
    - Između dev, preprod i prod: jedan red po komponenti, jedan stupac po goatnetu.

    Razlika koju očekuje proces izdavanja prijavljuje se, ali nije pogreška. Primjer je nova verzija koja je na dev, ali još nije na prod. Razlika koju ništa ne deklarira čita se kao `difference not declared`. Stroj koji goatbuilder ne može pročitati je unknown, nikad consistent.

    Pokreni ga na kopiji za vježbu. Njezin `dev` je u zaostatku namjerno, pa očekuj nalaze i izlazni kod 3.
tutor-l16-s1 = Pokreni survey na kopiji za vježbu
tutor-l17-title = Endstate: pokreće li goatnet točno ono što deklarira?
tutor-l17-text =
    Naredba goatbuilder `endstate` kod goatneta provjerava dvije stvari:
    1. Svaka usluga koju deklarira postavljena je, pokrenuta, zdrava i pravilno spojena.
    2. Ništa ne radi što ne deklarira. Nedeklariranu uslugu zovemo siroče.

    goatbuilder CLI utvrđuje što bi site trebao pokretati iz vlastitih postavki sitea, a ne iz odvojenog popisa.

    Provjera koja nema što čitati označena je UNOBSERVED. To je iskren odgovor, ne neuspjeh. FAIL ili siroče uzrokuju izlazni kod 3.

    Opći oblik je `goatbuilder endstate <goatnet>`. Naredba ispod provjerava primjer sitea iz spremljenih podataka, pa ne dira nijedan stroj.
tutor-l17-s1 = Provjeri primjer sitea
tutor-l18-title = Bonus: budi radoznao
tutor-l18-text =
    Ova lekcija je neobavezna. Pokreni naredbu ispod i tutor je završen.

    goatbuilder CLI ima nekoliko malih skrivenih značajki. Nisu u pomoći ni u ovim lekcijama. Neke se odazivaju na riječi koje dugogodišnji korisnici Unixa često upisuju u novi program, samo da vide što će se dogoditi. Većina stvari koje isprobaš neće napraviti ništa.

    Ova lekcija ne govori što su one. Pronaći ih je poanta.

    Naredba ispod ispisuje svaku pogrešku koju goatbuilder može protumačiti. To je dobro mjesto za početak istraživanja.
tutor-l18-s1 = Ispiši svaku pogrešku koju goatbuilder može protumačiti
tutor-heading = Lekcija { $n } od { $total } · { $title }
tutor-setup = Prvo pokreni ovo jednom u ovom terminalu:
tutor-not-recording = Ovaj terminal ne snima tvoja pokretanja, pa tutor ne može štiklirati tvoje korake. Pokreni ovo jednom, a zatim pokreni korake:
tutor-type = Napravi ove korake u svom terminalu ({ $met } od { $total } gotovo):
tutor-rerun = Nakon svakog koraka pokreni `goatbuilder tutor` ponovno. Provjerava što si učinio i štiklira korak.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Dobro došao u GOATBUILDER Tutor
tutor-bar-progress = Napredak
tutor-bar-done = { $done } od { $total } lekcija gotovo
tutor-after-step = Korak { $met } od { $total } gotov. Sljedeće:
tutor-after-lesson = Lekcija { $n } je gotova. Pokreni `goatbuilder tutor` da započneš lekciju { $next }. Da je ponovno pročitaš: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekcija { $n } je gotova, a gotov je i tutor. Pokreni `goatbuilder tutor` da vidiš da je označeno kao gotovo. Da je ponovno pročitaš: `goatbuilder tutor --lesson { $n }`.

tutor-review = Ova lekcija je gotova, pa je ova stranica samo za čitanje. Pokreni `goatbuilder tutor` da se vratiš na svoje mjesto.
tutor-other-lessons = Da ponovno pročitaš lekciju: `goatbuilder tutor --lesson 1`. Da ih vidiš sve: `goatbuilder tutor --list`.
tutor-l12-title = Dodaj stroj na site koji već radi
tutor-l12-text =
    Naredba goatbuilder `join-node` dodaje još jedan stroj na site koji već radi. Prvo ispisuje plan, s jednim redom po fazi. Svaka faza ima tip:
    - tool: goatbuilder pokreće skriptu.
    - ansible: goatbuilder pokreće playbook.
    - gate: provjera koja mora proći prije sljedeće faze.
    - manual: to radiš sam, i izvođenje se zaustavlja.

    Naredba ispod ima `--dry-run`. Čita ovaj checkout i ništa ne mijenja. Ispod tablice, pročitaj što radiš ručno.
tutor-l13-title = Spoji se na goatnet druge organizacije
tutor-l13-text =
    Naredba goatbuilder `federate` spaja tvoj goatnet s goatnetom koji pokreće druga organizacija. Svaka strana gradi svoju polovicu sporazuma. Vi razmijenite polovice, i svaka strana ih spaja u isti sporazum.

    Potvrdi ključ druge strane na odvojeni način, kao poziv kojem vjeruješ. Ključ koji stigne unutar njihovog paketa ne dokazuje ništa.

    Naredba ispod ima `--dry-run`, tako da ništa ne mijenja. Stvarno izvođenje zastaje u fazi 4, jer tu fazu radiš ručno.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ta naredba nije dio lekcije { $n }. Pokreni `goatbuilder tutor` da vidiš gdje si.
tutor-l1-s2 = Pitaj naredbu doctor o jednom goatnetu
tutor-l1-text =
    goatbuilder CLI gradi, postavlja i izdaje GOAT softver. Prije nego što ga upotrijebiš, provjeri svoju postavku s dvije naredbe:
    - Naredba goatbuilder `validate` provjerava konfiguracijsku datoteku, `ops/goatbuilder/goatbuilder.yaml`. Čita samo ovaj checkout i ne treba mrežu.
    - Naredba goatbuilder `doctor` provjerava što treba stvarnom deployu: tvoje alate, tvoje prijave, odgovara li ovaj checkout grani main i kako goatbuilder dolazi do svakog goatneta. Goatnet je jedno GOAT okruženje.

    Naredba goatbuilder `doctor` ispisuje koliko je provjera prošlo, a zatim svaki problem s njegovim popravkom. U ovoj lekciji problem je informacija, a ne pogreška. Pročitaj popravke, ali ih ne moraš sada primijeniti.

    Većini lekcija ne treba prijava. Lekcije 2 do 5, 10, 11 i 16 koriste kopiju za vježbu. Lekcije 8, 9, 12, 13, 15, 17 i 18 samo čitaju ovaj checkout. Lekciji 6 trebaju `gh` i mesh, lekciji 7 treba `gh`, a lekciji 14 treba mesh. Nijednoj lekciji ne treba 1Password.
tutor-l14-text =
    goatbuilder CLI može se sam ažurirati iz kopije koju tvoj goatnet poslužuje na `get.<site>.<zone>/goatbuilder/`.

    Naredba goatbuilder `update --check` samo čita. Uspoređuje tvoju verziju s posluženom verzijom i ništa ne mijenja. Izlazni kod 0 znači da imaš najnoviju verziju. Izlazni kod 3 znači da postoji novija verzija.

    Stvarno ažuriranje, bez `--check`, provjerava kontrolni zbroj i potpis preuzimanja prije nego što išta zamijeni. Zatim zamjenjuje datoteku u jednom koraku.

    goatbuilder CLI mora znati koji goatnet pitati: proslijedi `--site` s tim goatnetom ili postavi `GOATBUILDER_UPDATE_URL`. Moraš biti na meshu tog goatneta. Ako nisi, `--check` kaže da ne može doći do poslužitelja i navodi adresu koju je pokušao. To je ispravan rezultat za ovu lekciju.
tutor-l15-s1 = Navedi testove koji se odnose na jedan goatnet
tutor-l6-text =
    Izdanje provodi novu verziju kroz tri goatneta redom: dev, zatim preprod, zatim prod.

    1. Netko označi verziju na main, a CI je izgradi.
    2. dev: goatbuilder je postavlja i pokreće testove prihvaćanja.
    3. preprod: goatbuilder je ponovno postavlja, ponovno pokreće testove i bilježi rezultate.
    4. Verzija zatim čeka na preprod određeno vrijeme. Ništa ne prelazi na prod samo od sebe.
    5. prod: nakon tog čekanja osoba pokreće izdanje s `--from prod`. goatbuilder CLI postavlja na prod i provjerava sva tri goatneta.

    Naredba goatbuilder `promote status` pokazuje gdje je svaka komponenta. Kandidat je verzija koja se testira. Opći oblik izdanja je `goatbuilder release <component> --tag <tag> --from dev`. Naredbe ispod koriste jednu komponentu ovog checkouta i primjer oznake. Svaka ima `--dry-run`, pa se ništa ne označava ni ne postavlja.

    Suha pokretanja trebaju `gh` i mesh. Ako nisi admin repozitorija, stavi `GOAT_RC_ROLE_CHECK=0` ispred svake naredbe `release`.
tutor-l9-s1 = Izazovi stvarnu pogrešku s goatnetom koji ne postoji
tutor-l9-text =
    Kad goatbuilder odbije, ispiše kod. Kodovi koji počinju s `GB-` dolaze iz goatbuildera, `E_` iz goat CLI-ja, a `X-` iz drugog alata koji je goatbuilder pokrenuo.

    Naredba goatbuilder `explain` prima kod ili tekst retka pogreške i kaže ti što se dogodilo, zašto, kako to popraviti i gdje pročitati više.

    Traženje goatneta koji ovaj checkout ne deklarira izaziva stvarnu pogrešku koju možeš protumačiti.
