### goatbuilder — cnr (cnr)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Izgradite i isporučite goat okruženje: deklarišite, konvergirajte, provjerite — pod jednim ugovorom o isporuci
help-flag-output = Format izlaza: tabela za čovjeka; JSON ili YAML za program
help-flag-repo = Korijen repozitorijuma. Podrazumijeva se checkout u kojem se ovaj binarni fajl pokreće
help-flag-yes = Potvrdi deklarativnu komandu bez upita (za automatizaciju)
help-flag-frozen = Odbij svako pokretanje kojem bi trebalo premotati (fast-forward) checkout (strogi C3)
help-flag-mock = Pokreni nad direktorijumom fixture-a umjesto nad hostovima: bez registra, bez transporta, deterministički (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Jezik za pomoć i objašnjenja grešaka (oznaka poput fr, uk, de); izlaz koji čita pipeline ostaje na engleskom. Podrazumijeva se: jezik OS-a
help-cmd-validate = Provjeri konfiguraciju prema verzionisanoj šemi i pravilima među poljima, offline
help-cmd-doctor = Ispitaj šta pokretanju treba prije nego što se bilo šta presiječe: zavisnosti, svježinu checkouta, transport svakog okruženja i da se playbook svake komponente razrješava u određen ciljni režim
help-cmd-status = Deklarisano naspram zabilježenog, po mašini: konvergirano, na čekanju ili nepoznato (C7, C8). Samo čita. Izlazni kôd 3 dok god neka mašina nije konvergirana
help-cmd-deploy = Postavi pin koji main deklariše na jedno okruženje i dokaži to: iz pinova razriješi komponentu i grupe okruženja, play i njegov ciljni režim, svaku tajnu koju play šablonizuje, transport; pokreni play kroz ansible-preflight; pročitaj deploy ledger sa svakog hosta i smatraj posao gotovim tek kad najnoviji red navodi deklarisani digest sa action=deploy (C7). Na fixture-u (`--mock`) skriptovani converger zamjenjuje hostove
help-cmd-release = Provedi release candidate od oznake na main-u kroz živu validaciju dev goatneta do preprod stepenika (ADR 1173); nakon soak-a, uz `--from prod`, diplomiraj ga, uvrsti u fleet:, isporuči prod i ponovo provjeri svaki stepenik. Zamjenjuje ops/scripts/rc.sh korak po korak
help-cmd-rollback = Vrati isporučenu komponentu na njen N-ti najnoviji ref, po tipu artefakta, uz čitanje ledgera na hostu kao izvora istine (ADR 1060). Zamjenjuje ops/scripts/rollback.sh: izlazni kôd 3 odbijeno kao nebezbjedno (data, cert, forward-only, reversible:false), 4 vođeno (config, acl)
help-cmd-fastlane = Jedna komanda od spojenog commita do svakog goatneta (ADR 1127): namjerni izuzetak od ljestvice. Zamjenjuje ops/scripts/fastlane.sh korak po korak; odbija nepinovan digest ili otvoren skup za diplomiranje, a isporuke pokreće kroz ovaj binarni fajl
help-cmd-standup = Lokacija od nule do provjerene: faze standup-a redoslijedom iz doktrine, sa zatvorenom kapijom (fail-closed) na oba kraja, host za observability kao paralelna grana, rescue klase i međugoatnet površine na kraju (ADR 1183). Zamjenjuje ops/scripts/goat-standup.sh korak po korak. Na fixture-u (`--mock`) svaku komponentu svake klase konvergira skriptovani converger, a vrijeme od deklarisanog do provjerenog je mjera jednosatnog goatneta
help-cmd-join-node = Još jedna mašina neke klase na živoj lokaciji, deklarisana → konvergirana → provjerena: izvršeni redoslijed iz docs/tutorials/02-first-federated-node.md kao numerisane faze sa svojim kapijama (dostupnost, ključ, prvo rad na repou, baseline, kernel WireGuard prije i poslije enrolment-a, kernel ACL ravan kao kapija enrolment-a, converge klase, prihvatanje)
help-cmd-federate = Suvereno peering povezivanje deklarisano, konvergirano i provjereno (ADR 1157): izmjerena naša polovina, paketi razmijenjeni van kanala, sastavljen i provjeren jedan sporazum, iz njega izdati leaf-ovi, pravila iscrtana jedinim writer-om, isporučen edge, otvoren transport koji je sporazum izabrao, prihvatanje u sva četiri nivoa. Zamjenjuje ops/scripts/peering-standup.sh korak po korak
help-cmd-pins = JEDAN prolaz razrješavanja sa DVA izlaza: pins.yml u group_vars svakog navedenog člana i IntentV1 artefakt svakog navedenog goatneta, iz ops/pins/fleet-pins.yaml. Zamjenjuje ops/pins/generate-pins.py bajt po bajt. `--check` validira, ponovo generiše i odbija drift, nepotpisan ili izmijenjen intent i upravljan pin definisan izvan pins.yml (izlazni kôd 1)
help-cmd-promote = Vodi i prati diplomiranje izdanja kroz ljestvicu goatnetova (ADR 1120, 1125): status, otvaranje kandidata, diplomiranje, diplomiranje skupa, hotfix putanja, pregled, spremnost, odustajanje, provjera. Zamjenjuje ops/release/graduate.py korak po korak. Ništa se ovdje ne diplomira samo od sebe: izmjena se izvršava tek kad se pozove, ispisuje šta je uradila i traži --yes bez terminala (C10, ADR 1177)
help-cmd-explain = Protumači grešku: kôd (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) ili bilo koji tekst greške — ovog binarnog fajla ili tuđe poruke koju je prenio (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Ispisuje šta se desilo, zašto, šta uraditi i gdje pročitati više; `-` čita tekst sa stdin-a

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Lokacija od verifikovane do ničega — ogledalo standup-a, zapisi se čuvaju (burn-down dodaje teardown redove, nikad ne briše). Na fixture-u (`--mock`) probno izvodi burn-down obrnutim redom faza; na stvarnim hostovima ispisuje redoslijed i odbija izvršenje dok svaka faza ne deklariše svoj teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekcije u stilu vimtutor-a: svaka navodi tačnu komandu koju treba pokrenuti nad privremenom kopijom fixture-a (ili, samo za čitanje, nad ovim checkout-om), pa provjerava šta je ta komanda zaista uradila — izlazni kod i redove — prije nego što se otključa sljedeća. `tutor` prikazuje lekciju na kojoj si i označava je završenom kad tvoja pokretanja zadovolje uslove; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Skriveno (src/fun.rs)
help-cmd-argan = Skriveno (src/fun.rs)
help-cmd-bound = Skriveno (crates/goat-fun)
help-cmd-secrets = Deklarisani stav o tajnama (ADR 1204): koje skladište važi (1password, openbao, sops, git-crypt, env-file, env), šta je dostupno, kako dostaviti ono što nedostaje i sopstvene provjere skladišta. `init --env <goatnet>` upisuje env fajl za zero-SaaS quickstart: jaku slučajnu vrijednost za svaku varijablu koju play-ovi goatnet-a šablonizuju, 0600, gitignored, nikad ne rotira vrijednost koja već postoji. Isti modul kao ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Pročitaj najnoviji red deploy-ledger-a svakog hosta za jednu komponentu i reci imenuje li ref koji main deklariše (C7). Samo čita; izlaz 3 dok god je neki host na čekanju ili nepoznat. Ista provjera koju pokreće `deploy --verify-only`
help-cmd-diff = Šta se trenutno razlikuje od main (C8): deklarisani ref iz main naspram najnovijeg ledger reda svakog hosta, za jednu komponentu ili, bez nje, za svaku komponentu okruženja, čitano u jednom prolazu po grupi članova. Samo čita; izlaz 3 pri bilo kakvom odstupanju ili nečitljivom hostu
help-cmd-history = Deploy ledger kao vremenska linija, najnovije prvo, na svakom hostu okruženja (C8): svaki red je zaseban zapis sa mašinom, vremenom, akcijom (deploy ili rollback) i verzijom. Prethodna verzija zapisa i operator koji ga je izvršio su u `-o json`. Samo čita
help-cmd-plan = Upiši u plan fajl, radi pregleda, šta bi deklarativni glagol uradio: deklarisane refove, play i cilj, ledger, main commit, transport (deploy); tip artefakta i ciljni ref (rollback). Ne dira stanje nijednog hosta. Pregledani plan izvrši pomoću `apply`
help-cmd-apply = Izvrši tačno pregledani plan. Odbija (izlaz 1, navodi šta se promijenilo) kad se deklaracija iz main, pins commit, play ili cilj rollback-a promijenio otkako je plan napisan; promijenjen transport se prijavljuje, ne odbija
help-cmd-adopt = Deklariši ono što okruženje već pokreće: pročitaj ledgere svakog hosta i predloži pins odstupanje za svaku komponentu čiji se ref u radu razlikuje od deklaracije iz main. Podrazumijevano je izvještaj; uz --yes izmjene se upisuju u svjež worktree od main radi pregleda. Nikad ne deploy-uje

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Još ne: { $why }
tutor-lesson-done = Lekcija { $n } završena: { $title }.
tutor-all-done-reset = Sve lekcije su završene. `goatbuilder tutor --reset` kreće ispočetka; tutorijali idu dalje: docs/tutorials/.
tutor-all-done = Sve lekcije su završene. Tutorijali idu dalje: docs/tutorials/.
tutor-workspace = Radni prostor: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Provjerava da li je goatnet ispravno izgrađen, sloj po sloj: sve prihvatne provjere koje važe za lokaciju, uključujući provjere koje operator pokreće pomoću goat CLI-ja. Prihvatna faza standupa kao zasebna komanda. Samo čita; izlaz 3 dok god neka provjera ne prođe ili ne može da odgovori
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Izolovana lokacija: ne preuzimaj trunk; ocijeni checkout prema keširanom i zabilježi pokretanje kao offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Provjerava da li je goatnet u deklarisanom krajnjem stanju: svaki servis za koji njegove vlastite deklaracije kažu da radi je postavljen, pokrenut, ispravan i obavlja svoj posao, zero-trust veze su povezane, a ništa ne radi što nije deklarisano. Samo čita; izlazni kôd 3 dok god nešto odstupa
help-cmd-survey = Koliko je softver konzistentan unutar svakog goatneta i između goatnetova na ljestvici, očitano u jednom paralelnom prolazu. Samo čita

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Koliko je softver usklađen unutar svakog goatnet-a i između goatnet-ova na ljestvici, očitano u jednom paralelnom prolazu. Samo čita.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Zamijeni ovaj binarni fajl izdanjem koje njegov goatnet servira na `get.<site>.<zone>/goatbuilder/`: preuzimanje se provjerava prema njegovom `.sha256` i potpisanom `SHA256SUMS` prije nego što se išta dirne, a zamjena je atomična. Već aktuelno: jedan red, izlaz 0. `--check` samo čita (izlaz 3 kad zaostaje). Izvor: `GOATBUILDER_UPDATE_URL`, inače `--site` iz njegovih group_vars, inače onaj koji je koristilo posljednje ažuriranje
tutor-prereq = Prije nego počnete: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Oboji ok/FAIL glif i presudu u tabeli (auto|always|never). `auto` boji samo terminal; NO_COLOR i CLICOLOR_FORCE imaju prednost. JSON i YAML se nikad ne boje
help-flag-verbose = Upiši na stderr šta pokretanje radi — spoljne komande koje poziva i mrežnu fazu `update` — da `-o json|yaml` ostane čist dokument. Ili postavi RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekunde ograničenja za fazu povezivanja/dosega prije brzog otkaza — jedinu fazu koja može da blokira na nedostupnom hostu (`update`-ovo preuzimanje servirane verzije). NE ograničava izvršavanje samog play-a niti preuzimanje binarnog fajla (ograničeno zastojem). Čita i GOATBUILDER_TIMEOUT
help-cmd-completion = Ispiši skriptu za dopunu komandi ljuske na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Za žive kandidate koje statička skripta ne može znati — sajtove, komponente i goatnetove koje ovaj checkout deklariše — veži hook na `goatbuilder _complete` (vidi zaglavlje generisane skripte)
help-cmd-docs = Referenca komandi: `goatbuilder docs` upućuje na nju; `--markdown` ispisuje cijelu, obilaskom živog stabla komandi, pa se nikad ne može razići sa `--help`
help-cmd-_complete = Skriveno: ispisuje žive kandidate za dopunu za hookove ljuske, jedan po liniji (sajtovi, komponente i goatnetovi koje ovaj checkout deklariše). Samo čita, nikad ne prijavljuje grešku glasno — hook za dopunu ne smije pokvariti prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Prije rollouta, izvještaj o spremnosti i reverzibilnosti (ADR 1211): tri činjenice — State (stateless | stateful), Rollback (pin-revert | restore-required | none) i Scope (single | coupled) — deklarišu se po komponenti i FAIL-CLOSED su kad nisu deklarisane, uz polovinu spremnosti koju dokazuju ostali glagoli (ref se može povući, pins --check, secrets gate, svjež izvor, deklarisano krajnje stanje) i provjeru činjenice o rollbacku (rollback --dry-run za pin-revert, retention prozor za restore-required). `deploy` ga pokreće prvi; ocjenjuje se po iskrenosti, nikad lažno zeleno. Samo čita; izlaz 3 kad nije čisto

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Provjeri svoje postavke
tutor-l1-s1 = Provjeri konfiguracioni fajl
tutor-l2-title = Čitanje stanja: converged, pending, unknown
tutor-l2-text =
    Komanda goatbuilder `status` za svaku mašinu poredi dvije stvari: verziju koju za nju deklariše main, i verziju za koju njen sopstveni zapis kaže da je pokrenuta. Ništa ne mijenja.

    Svaka mašina je u jednom od tri stanja:
    - converged: njen zapis navodi deklarisanu verziju. To je gotovo.
    - pending: njen zapis navodi stariju verziju, ili još ništa.
    - unknown: goatbuilder ne može pročitati njen zapis.

    Samo converged se računa kao gotovo. Ako je neka mašina pending ili unknown, `status` izlazi sa kôdom 3.

    Od ove lekcije nadalje `--mock` usmjerava goatbuilder na kopiju za vježbu u tvom folderu tutora, umjesto na stvarne mašine. Ništa što tamo uradiš ne dopire do stvarne mašine.

    Na `dev` kopije za vježbu još ništa nije postavljeno. Očekuj zato četiri mašine pending i jednu unknown. Mašine converged ćeš vidjeti u lekciji 3.
tutor-l2-s1 = Pročitaj stanje dev
tutor-l3-title = Postavljanje: isplaniraj ga, izvrši ga, dokaži ga
tutor-l3-text =
    Postavljanje stavlja deklarisanu verziju na svaku mašinu, a onda čita zapis svake mašine da to dokaže. Postavljanje je gotovo samo kad zapis svake mašine navodi tu verziju. Postavljanje koje je prošlo bez greški nije dovoljno.

    `portal` ćeš postaviti na `preprod` kopije za vježbu. `--dry-run` pokazuje šta bi se desilo i ništa ne mijenja. `--ack` zapisuje ko je to uradio; komanda tvoje ime popunjava iz git-a. Posle postavljanja `status` bi trebalo da pokaže obje mašine kao `ok` i converged.
tutor-l3-s1 = Isplaniraj postavljanje
tutor-l3-s2 = Postavi portal na preprod
tutor-l3-s3 = Provjeri da su obje mašine converged
tutor-l4-title = Odstupanje i istorija
tutor-l4-text =
    Dvije komande ti pomažu da razumiješ goatnet prije i posle promjene. Obje samo čitaju.
    - Komanda goatbuilder `diff` ispisuje svaku mašinu čiji se zapis razlikuje od deklarisane verzije. Takva razlika se zove odstupanje.
    - Komanda goatbuilder `history` ispisuje svaku verziju koju je komponenta imala na svakoj mašini, od najnovije.

    Na `dev` je svaka mašina u zakašnjenju. Istorija `portal` na `preprod` sad već sadrži tvoje postavljanje iz lekcije 3.
tutor-l4-s1 = Ispiši odstupanje na dev
tutor-l4-s2 = Pročitaj istoriju portal na preprod
tutor-l5-title = Vrati verziju i vidi kako se odbija nebezbjedno vraćanje
tutor-l5-text =
    Vraćanje deklariše raniju verziju i postavlja je kao svaku drugu verziju. goatbuilder CLI onda čita zapis svake mašine da to dokaže.

    Neke promjene se tako ne mogu poništiti. Postavljanje starije verzije ne može poništiti podatke koje je komponenta zapisala, sertifikat, ili promjenu koja ide samo unaprijed. goatbuilder CLI takva vraćanja odbija sa kôdom 3 i kaže šta da se uradi umjesto toga.

    Komponenta `db-schema` kopije za vježbu sadrži podatke, pa goatbuilder odbija da je vrati. To odbijanje je očekivan rezultat.
tutor-l5-s1 = Isplaniraj vraćanje
tutor-l5-s2 = Vrati portal jednu verziju nazad
tutor-l5-s3 = Probaj da vratiš db-schema, što goatbuilder odbija
tutor-l6-title = Izdavanje nove verzije: dev, pa preprod, pa prod
tutor-l6-s1 = Pogledaj gdje je svaka komponenta
tutor-l6-s2 = Pokreni suho pokretanje izdanja iz dev
tutor-l6-s3 = Pokreni suho pokretanje koraka prod
tutor-l7-title = Fastlane, i kad ga ne koristiti
tutor-l7-text =
    Fastlane stavlja jednu verziju na sve goatnete odjednom. Preskače čekanje na preprodu i zapise izdanja.

    Koristi ga samo za hitan popravak koji možeš poništiti povratkom na prethodnu verziju. Nikad ga ne koristi za promjenu baze podataka, promjenu identiteta ili pristupa, ni za bilo šta što se tako ne može poništiti.

    Fastlane odbija verziju koju CI nije izgradio, i komponentu koja je dio izdanja koje je još u toku.

    Opšti oblik je `goatbuilder fastlane <component> --ref <version> --reason <why>`. Pokreni primjer ispod. Ima `--dry-run`, pa se ništa ne mijenja. Treba mu `gh`.
tutor-l7-s1 = Pokreni suho pokretanje fastlanea
tutor-l8-title = Pinovi: pronađi odstupanje i popravi ga na pravi način
tutor-l8-text =
    Jedan fajl određuje koju verziju pokreće svaki goatnet: `ops/pins/fleet-pins.yaml`. goatbuilder CLI iz njega generiše ostale fajlove verzija: fajlove `pins.yml` i potpisane fajlove intent.

    Komanda goatbuilder `pins --check` poredi generisane fajlove sa `fleet-pins.yaml` i odbija svaki fajl koji se razlikuje. Komanda goatbuilder `pins` ih ponovo generiše.

    Tvoj folder tutora ima kopiju repozitorijuma u kojoj je neko ručno izmijenio jedan fajl `pins.yml`. Ručnu izmjenu nikad ne popravljaj drugom ručnom izmjenom. Izmijeni `fleet-pins.yaml`, pa onda ponovo generiši.
tutor-l8-s1 = Provjeri ovaj checkout
tutor-l8-s2 = Provjeri kopiju i pronađi ručnu izmjenu
tutor-l8-s3 = Ponovo generiši fajlove kopije
tutor-l8-s4 = Provjeri kopiju ponovo
tutor-l9-title = Protumači grešku
tutor-l9-s2 = Protumači grešku po njenom kôdu
tutor-l9-s3 = Protumači je po njenom tekstu
tutor-l10-title = Vodič 1: izgradi prvi sajt novog goatneta
tutor-l10-text =
    Ova lekcija gradi cijeli goatnet u kopiji za vježbu. Primjer goatneta je `zagros`. Njegov prvi sajt, `alpha`, ima dvije core mašine, jednu mašinu za monitoring i jednu mašinu za spasavanje.

    Komanda goatbuilder `standup` postavlja svaku mašinu određenim redom i provjerava svaki zapis. Njegov zadnji red kaže da je sajt gotov. Na stvarnim mašinama taj red i pokazuje koliko je to trajalo.

    Cio vodič je `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Izgradi prvi sajt, alpha
tutor-l10-s2 = Provjeri rezultat
tutor-l11-title = Vodič 2: dodaj drugi sajt
tutor-l11-text =
    Ova lekcija počinje gdje je vodič 1 završio. Dodaje drugi sajt, `beta`, u novoj kopiji za vježbu, `zagros-2`.

    Drugi sajt kopira korisničke naloge sa prvog sajta. Njegov standup takođe jednom mijenja prvi sajt, tako da to kopiranje radi u oba pravca. To radi korak nazvan `alpha/core`.

    Cio vodič je `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Izgradi drugi sajt, beta
tutor-l11-s2 = Provjeri oba sajta
tutor-l12-s1 = Isplaniraj dodavanje jedne mašine
tutor-l13-s1 = Isplaniraj federaciju
tutor-l14-title = Održavaj goatbuilder ažurnim
tutor-l14-s1 = Pitaj da li postoji noviji goatbuilder
tutor-l15-title = Assay: provjeri kako je goatnet izgrađen
tutor-l15-text =
    Komanda goatbuilder `assay` provjerava da je goatnet izgrađen pravilno, sloj po sloj, koristeći prihvatne testove. Svaki test dobija jedan od tri rezultata:
    - PASS: test je prošao.
    - FAIL: goatnet ima problem.
    - UNKNOWN: ništa nije moglo da odgovori na test.

    Samo PASS se računa. Svaki FAIL ili UNKNOWN uzrokuje izlazni kôd 3.

    `--plan` ispisuje testove koji se odnose na goatnet, po slojevima, i ne dira nijednu mašinu. Koristi ga da vidiš šta bi pokrio stvarni assay. Opšti oblik je `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: da li su verzije usklađene?
tutor-l16-text =
    Komanda goatbuilder `survey` u jednom prolazu provjerava da li iste verzije rade tamo gdje treba:
    - Unutar svakog goatneta: da li svaka mašina pokreće deklarisanu verziju, i da li se mašine koje treba da se slažu stvarno slažu?
    - Između dev, preprod i prod: jedan red po komponenti, jedna kolona po goatnetu.

    Razlika koju proces izdanja očekuje prijavljuje se, ali nije greška. Primjer je nova verzija koja je na dev, ali još nije na prod. Razlika koju ništa ne deklariše čita se kao `difference not declared`. Mašina koju goatbuilder ne može pročitati je unknown, nikad consistent.

    Pokreni ga na kopiji za vježbu. Njen `dev` je u zakašnjenju namjerno, pa očekuj nalaze i izlazni kôd 3.
tutor-l16-s1 = Pokreni survey na kopiji za vježbu
tutor-l17-title = Endstate: da li goatnet pokreće tačno ono što deklariše?
tutor-l17-text =
    Komanda goatbuilder `endstate` kod goatneta provjerava dvije stvari:
    1. Svaki servis koji deklariše je postavljen, pokrenut, zdrav i pravilno povezan.
    2. Ništa ne radi što nije deklarisano. Nedeklarisani servis se zove siroče.

    goatbuilder CLI utvrđuje šta bi sajt trebalo da pokreće iz sopstvenih podešavanja sajta, a ne iz odvojenog spiska.

    Provjera koja nema šta da čita oznacena je sa UNOBSERVED. To je iskren odgovor, a ne neuspjeh. FAIL ili siroče uzrokuju izlazni kôd 3.

    Opšti oblik je `goatbuilder endstate <goatnet>`. Komanda ispod provjerava primjer sajta iz sačuvanih podataka, pa ne dira nijednu mašinu.
tutor-l17-s1 = Provjeri primjer sajta
tutor-l18-title = Bonus: budi radoznao
tutor-l18-text =
    Ova lekcija je neobavezna. Pokreni komandu ispod i tutor je završen.

    goatbuilder CLI ima nekoliko malih skrivenih mogućnosti. Nijesu u pomoći ni u ovim lekcijama. Neke odgovaraju na riječi koje dugogodišnji korisnici Unix-a često ukucaju u novi program, samo da vide šta će se desiti. Većina stvari koje probaš neće uraditi ništa.

    Ova lekcija ne govori šta su one. Poenta je da ih pronađeš.

    Komanda ispod ispisuje svaku grešku koju goatbuilder zna da protumači. To je dobro mjesto za početak istraživanja.
tutor-l18-s1 = Ispiši svaku grešku koju goatbuilder zna da protumači
tutor-heading = Lekcija { $n } od { $total } · { $title }
tutor-setup = Prvo pokreni ovo jednom u ovom terminalu:
tutor-not-recording = Ovaj terminal ne snima tvoja pokretanja, pa tutor ne može štiklirati tvoje korake. Pokreni ovo jednom, a onda pokreni korake:
tutor-type = Napravi ove korake u svom terminalu ({ $met } od { $total } gotovo):
tutor-rerun = Posle svakog koraka pokreni `goatbuilder tutor` ponovo. Provjerava šta si uradio i štiklira korak.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Dobro došao u GOATBUILDER Tutor
tutor-bar-progress = Napredak
tutor-bar-done = { $done } od { $total } lekcija gotovo
tutor-after-step = Korak { $met } od { $total } gotov. Sljedeće:
tutor-after-lesson = Lekcija { $n } je gotova. Pokreni `goatbuilder tutor` da započneš lekciju { $next }. Da je pročitaš ponovo: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekcija { $n } je gotova, a gotov je i tutor. Pokreni `goatbuilder tutor` da vidiš da je štikliran. Da je pročitaš ponovo: `goatbuilder tutor --lesson { $n }`.

tutor-review = Završio si ovu lekciju, pa je ova stranica samo za čitanje. Pokreni `goatbuilder tutor` da se vratiš tamo gdje si.
tutor-other-lessons = Da pročitaš lekciju ponovo: `goatbuilder tutor --lesson 1`. Da ih vidiš sve: `goatbuilder tutor --list`.
tutor-l12-title = Dodaj mašinu na sajt koji već radi
tutor-l12-text =
    Komanda goatbuilder `join-node` dodaje još jednu mašinu na sajt koji već radi. Prvo ispisuje plan, sa jednim redom po fazi. Svaka faza ima tip:
    - tool: goatbuilder pokreće skriptu.
    - ansible: goatbuilder pokreće playbook.
    - gate: provjera koja mora proći prije sledeće faze.
    - manual: to radiš sam, i izvršavanje se pauzira.

    Komanda ispod ima `--dry-run`. Ona čita ovaj checkout i ništa ne mijenja. Ispod tabele pročitaj šta radiš rukom.
tutor-l13-title = Poveži se sa goatnetom druge organizacije
tutor-l13-text =
    Komanda goatbuilder `federate` povezuje tvoj goatnet sa goatnetom koji pokreće druga organizacija. Svaka strana gradi svoju polovinu sporazuma. Razmjenjuješ polovine, i svaka strana ih spaja u isti sporazum.

    Potvrdi ključ druge strane na drugi način, kao poziv kojem vjeruješ. Ključ koji stigne unutar njihovog paketa ne dokazuje ništa.

    Komanda ispod ima `--dry-run`, tako da ništa ne mijenja. Pravo izvršavanje se zaustavlja na fazi 4, jer tu fazu radiš rukom.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ta komanda nije dio lekcije { $n }. Pokreni `goatbuilder tutor` da vidiš gdje si.
tutor-l1-s2 = Pitaj doctor o jednom goatnetu
tutor-l1-text =
    goatbuilder CLI gradi, postavlja i izdaje GOAT softver. Prije nego ga upotrijebiš, provjeri svoje postavke sa dvije komande:
    - Komanda goatbuilder `validate` provjerava konfiguracioni fajl, `ops/goatbuilder/goatbuilder.yaml`. Čita samo ovaj checkout i ne treba mu mreža.
    - Komanda goatbuilder `doctor` provjerava šta treba stvarno postavljanje: tvoje alate, tvoje prijave, da li se ovaj checkout slaže sa main-om i kako goatbuilder dolazi do svakog goatneta. Goatnet je jedno GOAT okruženje.

    Komanda goatbuilder `doctor` ispisuje koliko provjera je prošlo, pa onda svaki problem sa svojim popravkom. U ovoj lekciji je problem informacija, a ne greška. Pročitaj popravke, ali ih sad ne moraš primijeniti.

    Većina lekcija ne traži prijavu. Lekcije od 2 do 5, 10, 11 i 16 koriste kopiju za vježbu. Lekcije 8, 9, 12, 13, 15, 17 i 18 samo čitaju ovaj checkout. Lekciji 6 treba `gh` i mesh, lekciji 7 treba `gh`, a lekciji 14 treba mesh. Nijednoj lekciji ne treba 1Password.
tutor-l14-text =
    goatbuilder CLI se može ažurirati iz kopije koju tvoj goatnet servira na `get.<site>.<zone>/goatbuilder/`.

    Komanda goatbuilder `update --check` samo čita. Poredi tvoju verziju sa serviranom verzijom i ništa ne mijenja. Izlazni kôd 0 znači da si ažuran. Izlazni kôd 3 znači da postoji novija verzija.

    Stvarno ažuriranje, bez `--check`, provjerava kontrolnu sumu i potpis preuzetog fajla prije nego nešto izmijeni. Onda zamjenjuje fajl u jednom koraku.

    goatbuilder CLI treba da zna kom goatnetu da se obrati: proslijedi `--site` sa tim goatnetom, ili podesi `GOATBUILDER_UPDATE_URL`. Moraš biti na mesh-u tog goatneta. Ako nijesi, `--check` kaže da ne može da dosegne server i navodi adresu koju je probao. To je za ovu lekciju ispravan rezultat.
tutor-l15-s1 = Ispiši testove koji se odnose na jedan goatnet
tutor-l6-text =
    Izdanje premješta novu verziju po redu kroz tri goatneta: dev, pa preprod, pa prod.

    1. Neko označi verziju tagom na main-u, a CI je izgradi.
    2. dev: goatbuilder je postavlja i pokreće prihvatne testove.
    3. preprod: goatbuilder je postavlja ponovo, ponovo pokreće testove i zapisuje rezultate.
    4. Verzija onda čeka na preprod-u određeno vrijeme. Ništa samo od sebe ne prelazi u prod.
    5. prod: posle tog čekanja, neko pokreće izdanje sa `--from prod`. goatbuilder CLI postavlja u prod i provjerava sva tri goatneta.

    Komanda goatbuilder `promote status` pokazuje gdje je svaka komponenta. Kandidat je verzija koja se testira. Opšti oblik izdanja je `goatbuilder release <component> --tag <tag> --from dev`. Komande ispod koriste jednu komponentu ovog checkouta i primjer taga. Svaka ima `--dry-run`, pa se ništa ne označava tagom ni postavlja.

    Suhim pokretanjima treba `gh` i mesh. Ako nijesi administrator repozitorijuma, stavi `GOAT_RC_ROLE_CHECK=0` prije svake komande `release`.
tutor-l9-s1 = Napravi stvarnu grešku sa goatnetom koji ne postoji
tutor-l9-text =
    Kad goatbuilder odbije, ispisuje kôd. Kodovi koji počinju sa `GB-` dolaze od goatbuildera, `E_` od goat CLI-ja, a `X-` od drugog alata koji je goatbuilder pokrenuo.

    Komanda goatbuilder `explain` uzima kôd, ili tekst reda greške, i kaže ti šta se desilo, zašto, kako to popraviti i gdje da pročitaš više.

    Traženje goatneta koji ovaj checkout ne deklariše pravi stvarnu grešku za tumačenje.
