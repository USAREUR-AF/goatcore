### goatbuilder — fi (fi)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Rakenna ja toimita goat-ympäristö: määrittele, konvergoi, varmista – yhden deploy-sopimuksen alla
help-flag-output = Tulostusmuoto: taulukko ihmiselle; JSON tai YAML ohjelmalle
help-flag-repo = Repositorion juuri. Oletus: checkout, jossa tätä binääriä ajetaan
help-flag-yes = Vahvista deklaratiivinen verbi kysymättä (automaatiota varten)
help-flag-frozen = Hylkää ajo, joka vaatisi checkoutin pikakelausta (tiukka C3)
help-flag-mock = Aja fixture-hakemistoa vasten isäntien sijaan: ei rekisteriä, ei siirtoa, deterministinen (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Ohjeen ja virheselitysten kieli (tunniste kuten fr, uk, de); putken lukema tuloste pysyy englanniksi. Oletus: käyttöjärjestelmän kieli
help-cmd-validate = Tarkista konfiguraatio versioitua skeemaa ja kenttien välisiä sääntöjä vasten, offline
help-cmd-doctor = Selvitä, mitä ajo tarvitsee ennen kuin mitään leikataan: riippuvuudet, checkoutin ajantasaisuus, kunkin ympäristön siirtotie ja se, että jokaisen komponentin playbook ratkeaa yksiselitteiseen kohdetilaan
help-cmd-status = Määritelty vs. kirjattu, koneittain: converged, pending tai unknown (C7, C8). Vain luku. Poistumiskoodi 3, kun jokin kone ei ole converged
help-cmd-deploy = Vie main:in määrittelemä pin yhteen ympäristöön ja todista se: ratkaise komponentti ja ympäristön ryhmät pineistä, play ja sen kohdetila, jokainen secret, jota play templatoi, sekä siirtotie; aja play ansible-preflightin läpi; lue deploy-ledger takaisin jokaiselta isännältä ja julista valmiiksi vasta, kun uusin rivi nimeää määritellyn digestin ja action=deploy (C7). Fixturella (`--mock`) isäntien tilalla on skriptattu converger
help-cmd-release = Vie release candidate main:in tagista dev-goatnetin live-validoinnin kautta preprod-askelmalle (ADR 1173); soak-ajan jälkeen `--from prod` ylentää sen, sulauttaa fleet:-osioon, deployaa prodin ja varmistaa uudelleen jokaisen askelman. Korvaa ops/scripts/rc.sh:n askel askeleelta
help-cmd-rollback = Palauta deployattu komponentti N:nteen viimeisimpään refiinsä artefaktityypin mukaan, isännän ledger totuuden lähteenä (ADR 1060). Korvaa ops/scripts/rollback.sh:n: poistumiskoodi 3 kieltäytyi epäturvallisena (data, cert, forward-only, reversible:false), 4 ohjattu (config, acl)
help-cmd-fastlane = Yksi komento yhdistetystä commitista jokaiseen goatnetiin (ADR 1127): harkittu poikkeus portaikosta. Korvaa ops/scripts/fastlane.sh:n askel askeleelta; kieltäytyy pinnaamattomasta digestistä tai avoimesta ylennysjoukosta, ja ajaa deployt tämän binäärin kautta
help-cmd-standup = Sivusto tyhjästä varmistettuun: standupin vaiheet doktriinin järjestyksessä, fail-closed-portti kummassakin päässä, observability-isäntä rinnakkaisena haarana, rescue-luokat ja goatnetien väliset pinnat viimeisenä (ADR 1183). Korvaa ops/scripts/goat-standup.sh:n askel askeleelta. Fixturella (`--mock`) skriptattu converger konvergoi jokaisen luokan jokaisen komponentin, ja määritelty → varmistettu -aika on yhden tunnin goatnetin mittari
help-cmd-join-node = Yksi lisäkone luokkaan live-sivustolla, määritelty → converged → varmistettu: docs/tutorials/02-first-federated-node.md:n suoritettu sekvenssi numeroituina vaiheina portteineen (tavoitettavuus, avain, repotyö ensin, baseline, kernel WireGuard ennen ja jälkeen enrolmentin, kernel ACL -taso enrolment-porttina, luokan converge, hyväksyntä)
help-cmd-federate = Suvereeni peering määriteltynä, konvergoituna ja varmistettuna (ADR 1157): oma puolemme mitataan, paketit vaihdetaan kaistan ulkopuolella, yksi sopimus sävelletään ja varmistetaan, lehdet lyödään siitä, säännöt renderöi yksi kirjoittaja, reuna deployataan, sopimuksen valitsema siirtotie avataan, hyväksyntä kaikilla neljällä tasolla. Korvaa ops/scripts/peering-standup.sh:n askel askeleelta
help-cmd-pins = YKSI ratkaisukierros KAHDELLA renderöinnillä: jokaisen listatun jäsenen group_vars-tiedosto pins.yml ja jokaisen listatun goatnetin IntentV1-artefakti, tiedostosta ops/pins/fleet-pins.yaml. Korvaa ops/pins/generate-pins.py:n tavu tavulta. `--check` validoi, regeneroi ja kieltäytyy driftistä, allekirjoittamattomasta tai peukaloidusta intentistä sekä pins.yml:n ulkopuolella määritellystä hallitusta pinistä (poistumiskoodi 1)
help-cmd-promote = Ohjaa ja tarkkaile release-ylennystä goatnet-portaikossa (ADR 1120, 1125): status, avaa candidate, ylennä, ylennä joukko, hotfix-polku, review, readiness, abandon, check. Korvaa ops/release/graduate.py:n askel askeleelta. Mikään ei ylenny itsestään: mutaatio ajetaan vain kutsuttaessa, tulostaa tekemänsä ja pyytää --yes ilman terminaalia (C10, ADR 1177)
help-cmd-explain = Tulkitse virhe: koodi (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) tai mikä tahansa virheteksti – tämän binäärin oma tai sen välittämä työkalun sanoma (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Tulostaa mitä tapahtui, miksi, mitä tehdä ja mistä lukea lisää; `-` lukee tekstin stdinistä

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Kohde vahvistetusta tilasta tyhjäksi — standupin peilikuva, tietueet säilyvät (purku lisää teardown-rivejä, ei koskaan poista mitään). Fixturella (`--mock`) se harjoittelee purun käänteisessä vaihejärjestyksessä; oikeilla koneilla se tulostaa järjestyksen ja kieltäytyy suorittamasta, kunnes jokainen vaihe julistaa oman teardown-playnsa (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Oppitunnit vimtutorin tapaan: jokainen nimeää tarkan komennon, joka ajetaan fixturen kertakäyttökopiota vastaan (tai vain luku -tilassa tätä checkoutia vastaan), ja tarkistaa sitten, mitä komento todella teki — sen poistumiskoodin ja rivit — ennen kuin seuraava aukeaa. `tutor` näyttää nykyisen oppitunnin ja merkitsee sen tehdyksi, kun ajosi täyttävät sen ehdot; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Piilotettu (src/fun.rs)
help-cmd-argan = Piilotettu (src/fun.rs)
help-cmd-bound = Piilotettu (crates/goat-fun)
help-cmd-secrets = Julistettu salaisuuksien asetus (ADR 1204): mikä säilö on käytössä (1password, openbao, sops, git-crypt, env-file, env), mitä on saatavilla, miten puuttuva toimitetaan ja säilön omat tarkistukset. `init --env <goatnet>` kirjoittaa nollaSaaS-pikaoppaan env-tiedoston: vahva satunnaisarvo jokaiselle muuttujalle, jonka goatnetin playt templatoivat, 0600, gitignored, eikä koskaan kierrätä jo olemassa olevaa arvoa. Sama moduuli kuin ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Lukee komponentin jokaisen isännän uusimman deploy-ledger-rivin ja kertoo, nimeääkö se refin, jonka main julistaa (C7). Vain lukee; poistumiskoodi 3, kunnes jokainen isäntä on selvillä (pending tai unknown). Sama tarkistus, jonka `deploy --verify-only` ajaa
help-cmd-diff = Mikä eroaa mainista juuri nyt (C8): mainin julistettu ref vs. jokaisen isännän uusin ledger-rivi, yhdelle komponentille tai ilman sitä ympäristön kaikille komponenteille, luettuna yhdellä läpikäynnillä jäsenryhmää kohti. Vain lukee; poistumiskoodi 3, jos missä tahansa on driftiä tai isäntää ei voi lukea
help-cmd-history = Deploy-ledger aikajanana, uusin ensin, ympäristön jokaisella isännällä (C8): joka rivi on oma kirjauksensa koneen, ajan, toiminnon (deploy tai rollback) ja version kanssa. Kirjauksen edellinen versio ja sen ajanut operaattori ovat kohdassa `-o json`. Vain lukee
help-cmd-plan = Kirjoittaa suunnitelmatiedostoon katselmointia varten, mitä deklaratiivinen verbi tekisi: julistetut refit, play ja kohde, ledger, mainin commit, siirtotie (deploy); artefaktityyppi ja kohde-ref (rollback). Ei koske minkään isännän tilaan. Aja katselmoituna komennolla `apply`
help-cmd-apply = Ajaa täsmälleen katselmoidun suunnitelman. Kieltäytyy (poistumiskoodi 1, nimeää muuttuneen), jos mainin julistus, pins-commit, play tai rollback-kohde on muuttunut suunnitelman kirjoittamisen jälkeen; muuttunut siirtotie raportoidaan, ei estä ajoa
help-cmd-adopt = Julistaa sen, mitä ympäristö jo ajaa: lukee jokaisen isännän ledgerit ja ehdottaa pins-poikkeamaa jokaiselle komponentille, jonka ajossa oleva ref eroaa mainin julistuksesta. Oletuksena pelkkä raportti; --yes kirjoittaa muutokset mainin tuoreeseen worktreehen katselmointia varten. Ei koskaan deployaa

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ei vielä: { $why }
tutor-lesson-done = Oppitunti { $n } valmis: { $title }.
tutor-all-done-reset = Kaikki oppitunnit on tehty. `goatbuilder tutor --reset` aloittaa alusta; tutoriaalit menevät pidemmälle: docs/tutorials/.
tutor-all-done = Kaikki oppitunnit on tehty. Tutoriaalit menevät pidemmälle: docs/tutorials/.
tutor-workspace = Työtila: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Tarkista, että goatnet on rak
help-cmd-survey = Kuinka yhdenmukainen ohjelmisto on kunkin goatnetin sisällä ja tikapuiden goatnetien välillä, luettuna yhdellä rinnakkaisella läpikäynnillä. Vain luku
help-flag-offline = Verkosta erotettu kohde: älä hae runkoa; arvioi työkopio välimuistissa olevaa runkoa vasten ja kirjaa ajo offline-ajoksi (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Tarkista, että goatnet on määritellyssä lopputilassa: jokainen palvelu, jonka sen omat määrittelyt sanovat olevan käynnissä, on otettu käyttöön, käynnissä, terve ja hoitaa tehtävänsä, sen zero-trust-yhteydet on kytketty, eikä mikään ylimääräinen ole käynnissä. Vain lukee; poistuu koodilla 3, jos jokin on pielessä
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Kuinka yhdenmukainen ohjelmisto on kunkin goatnetin sisällä ja ladderin goatnettien välillä, luettuna yhdellä rinnakkaisella ajolla. Vain luku.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Korvaa tämä binääri julkaisulla, jota sen goatnet tarjoaa osoitteessa `get.<site>.<zone>/goatbuilder/`: lataus tarkistetaan `.sha256`-tiedostoa ja allekirjoitettua `SHA256SUMS`-tiedostoa vasten ennen kuin mitään muutetaan, ja vaihto on atominen. Jo ajan tasalla: yksi rivi, poistumiskoodi 0. `--check` vain lukee (poistumiskoodi 3, jos jäljessä). Lähde: `GOATBUILDER_UPDATE_URL`, muuten `--site` sen group_vars-tiedostosta, muuten se, jota viimeisin päivitys käytti
tutor-prereq = Ennen aloitusta: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Väritä taulukon ok/FAIL-symboli ja tuomio (auto|always|never). `auto` värittää vain terminaalin; NO_COLOR ja CLICOLOR_FORCE ohittavat. JSON ja YAML eivät koskaan väritetä
help-flag-verbose = Kirjaa stderriin, mitä ajo tekee — kutsutut ulkoiset komennot ja `update`-komennon verkkovaihe — jotta `-o json|yaml` pysyy puhtaana dokumenttina. Tai aseta RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekuntia, joihin yhteyden muodostus- ja tavoitettavuusvaihe rajataan ennen nopeaa epäonnistumista — ainoa vaihe, joka voi jäädä odottamaan tavoittamatonta isäntää (`update`-komennon palvelemansa version haku). EI rajaa playn omaa ajoa eikä binäärin latausta (jumiutumisraja). Lukee myös GOATBUILDER_TIMEOUT
help-cmd-completion = Tulosta komentotulkin täydennyskomentosarja stdoutiin (`goatbuilder completion zsh >> ~/.zshrc`). Ajantasaisille ehdokkaille, joita staattinen skripti ei voi tietää — tämän checkoutin määrittelemät sitet, komponentit ja goatnetit — kytke koukku komentoon `goatbuilder _complete` (katso luodun skriptin otsake)
help-cmd-docs = Komentoviite: `goatbuilder docs` osoittaa siihen; `--markdown` tulostaa koko viitteen, joka kävellään läpi elävästä komentopuusta, joten se ei voi koskaan erota `--help`-tulosteesta
help-cmd-_complete = Piilotettu: tulosta ajantasaiset täydennysehdokkaat komentotulkin koukuille, yksi per rivi (tämän checkoutin määrittelemät sitet, komponentit ja goatnetit). Vain lukee, ei koskaan epäonnistu äänekkäästi — täydennyskoukku ei saa sotkea kehotetta

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Ennen käyttöönottoa: valmiuden ja palautettavuuden yhteenveto (ADR 1211). Kolme tietoa — State (stateless | stateful), Rollback (pin-revert | restore-required | none) ja Scope (single | coupled) — ilmoitetaan komponenttikohtaisesti ja FAIL-CLOSED, jos ilmoitus puuttuu. Valmiuspuolen todentavat muut komennot (ref haettavissa, pins --check, secrets-portti, tuore lähde, ilmoitettu lopputila), ja rollback-tieto validoidaan (rollback --dry-run pin-revertille, säilytysaika restore-required-tilalle). `deploy` ajaa tämän ensin; rehellisyysarvioitu, ei koskaan väärää vihreää. Vain luku; poistumiskoodi 3, jos ei puhdas

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Tarkista asetuksesi
tutor-l1-s1 = Tarkista konfiguraatiotiedosto
tutor-l2-title = Lue tila: converged, pending, unknown
tutor-l2-text =
    Goatbuilderin `status`-komento vertaa kahta asiaa jokaiselle koneelle: versiota, jonka main ilmoittaa sille, ja versiota, jonka koneen oma kirjaus kertoo sen ajavan. Se ei muuta mitään.

    Jokainen kone on yhdessä kolmesta tilasta:
    - converged (konvergoitunut): sen kirjaus nimeää ilmoitetun version. Tämä on valmis.
    - pending (odottava): sen kirjaus nimeää vanhemman version, tai ei mitään vielä.
    - unknown (tuntematon): goatbuilder ei voi lukea sen kirjausta.

    Vain converged lasketaan valmiiksi. Jos jokin kone on pending tai unknown, `status` päättyy koodilla 3.

    Tästä oppitunnista alkaen `--mock` ohjaa goatbuilderin harjoituskopioon oppitunnikansiossasi oikeiden koneiden sijaan. Mikään, mitä teet siellä, ei koske oikeaa konetta.

    Harjoituskopion `dev`-ympäristöön ei ole vielä deployattu mitään. Odota siis neljää pending-konetta ja yhtä unknown-konetta. Näet converged-koneita oppitunnissa 3.
tutor-l2-s1 = Lue devin tila
tutor-l3-title = Deploy: suunnittele, suorita, todista
tutor-l3-text =
    Deployaaminen asettaa ilmoitetun version jokaiselle koneelle ja lukee sitten jokaisen koneen kirjauksen todistaakseen, että se saapui. Deploy on valmis vain, kun jokaisen koneen kirjaus nimeää tämän version. Deploy, joka suoritettiin virheittä, ei riitä.

    Deployaat `portal`-komponentin harjoituskopion `preprod`-ympäristöön. `--dry-run` näyttää, mitä tapahtuisi, eikä muuta mitään. `--ack` kirjaa, kuka sen teki; komento täyttää nimesi gitistä. Deployn jälkeen `status`-komennon pitäisi näyttää molemmat koneet `ok`-tilassa ja converged-tilassa.
tutor-l3-s1 = Suunnittele deploy
tutor-l3-s2 = Deployaa portal preprodiin
tutor-l3-s3 = Tarkista, että molemmat koneet ovat converged
tutor-l4-title = Drift ja historia
tutor-l4-text =
    Kaksi komentoa auttaa ymmärtämään goatnetin tilan ennen muutosta ja sen jälkeen. Molemmat vain lukevat.
    - Goatbuilderin `diff`-komento listaa joka koneen, jonka kirjaus eroaa ilmoitetusta versiosta. Eroa kutsutaan driftiksi.
    - Goatbuilderin `history`-komento listaa joka version, joka komponentilla on ollut jokaisella koneella, uusin ensin.

    `dev`-ympäristössä joka kone on jäljessä. `portal`-komponentin historia kohteessa `preprod` sisältää nyt deployisi oppitunnista 3.
tutor-l4-s1 = Listaa drift devissä
tutor-l4-s2 = Lue portalin historia preprodissa
tutor-l5-title = Palauta ja näe turvaton rollback hylättynä
tutor-l5-text =
    Rollback ilmoittaa aiemman version ja deployaa sen kuin minkä tahansa muun version. Goatbuilderin CLI lukee sitten jokaisen koneen kirjauksen todistaakseen, että se saapui.

    Joitakin muutoksia ei voi perua tällä tavalla. Vanhemman version deployaaminen ei voi perua dataa, jonka komponentti on kirjoittanut, varmennetta, tai muutosta, joka etenee vain yhteen suuntaan. Goatbuilderin CLI hylkää ne koodilla 3 ja kertoo, mitä tehdä sen sijaan.

    Harjoituskopion `db-schema`-komponentti sisältää dataa, joten goatbuilder kieltäytyy perumasta sitä. Tämä kieltäytyminen on odotettu tulos.
tutor-l5-s1 = Suunnittele rollback
tutor-l5-s2 = Palauta portal yhden version taakse
tutor-l5-s3 = Yritä palauttaa db-schema, jonka goatbuilder hylkää
tutor-l6-title = Julkaise uusi versio: dev, sitten preprod, sitten prod
tutor-l6-s1 = Katso, missä joka komponentti on
tutor-l6-s2 = Koeaja julkaisu devistä
tutor-l6-s3 = Koeaja prod-vaihe
tutor-l7-title = Fastlane, ja milloin sitä ei käytetä
tutor-l7-text =
    Fastlane asettaa yhden version joka goatnetiin samalla kertaa. Se ohittaa preprodin odotuksen ja julkaisun kirjaukset.

    Käytä sitä vain kiireelliseen korjaukseen, jonka voit perua palaamalla edelliseen versioon. Älä koskaan käytä sitä tietokantamuutokseen, identiteetti- tai pääsymuutokseen, tai mihinkään, jota et voi perua tällä tavalla.

    Fastlane hylkää version, jota CI ei ole rakentanut, ja komponentin, joka on osa vielä käynnissä olevaa julkaisua.

    Yleinen muoto on `goatbuilder fastlane <component> --ref <version> --reason <why>`. Suorita alla oleva esimerkki. Siinä on `--dry-run`, joten mikään ei muutu. Se tarvitsee `gh`:n.
tutor-l7-s1 = Koeaja fastlane
tutor-l8-title = Pins: löydä drift ja korjaa se oikealla tavalla
tutor-l8-text =
    Yksi tiedosto määrittää, mitä versiota jokainen goatnet ajaa: `ops/pins/fleet-pins.yaml`. Goatbuilderin CLI luo siitä muut versiotiedostot: `pins.yml`-tiedostot ja allekirjoitetut intent-tiedostot.

    Goatbuilderin `pins --check`-komento vertaa luotuja tiedostoja tiedostoon `fleet-pins.yaml` ja hylkää jokaisen tiedoston, joka eroaa. Goatbuilderin `pins`-komento luo ne uudelleen.

    Oppitunnikansiossasi on kopio repositoriosta, jossa joku on muokannut yhtä `pins.yml`-tiedostoa käsin. Älä koskaan korjaa käsinmuokkausta toisella käsinmuokkauksella. Muuta `fleet-pins.yaml`, ja luo sitten uudelleen.
tutor-l8-s1 = Tarkista tämä checkout
tutor-l8-s2 = Tarkista kopio ja löydä käsinmuokkaus
tutor-l8-s3 = Luo kopion tiedostot uudelleen
tutor-l8-s4 = Tarkista kopio uudelleen
tutor-l9-title = Selitä virhe
tutor-l9-s2 = Selitä virhe koodilla
tutor-l9-s3 = Selitä virhe tekstillä
tutor-l10-title = Opastus 1: rakenna uuden goatnetin ensimmäinen sivusto
tutor-l10-text =
    Tämä oppitunti rakentaa kokonaisen goatnetin harjoituskopioon. Esimerkkigoatnet on `zagros`. Sen ensimmäisessä sivustossa, `alpha`, on kaksi ydinkonetta, yksi valvontakone ja yksi pelastuskone.

    Goatbuilderin `standup`-komento asettaa jokaisen koneen kiinteässä järjestyksessä ja tarkistaa joka kirjauksen. Sen viimeinen rivi sanoo, että sivusto on valmis. Oikeilla koneilla se rivi näyttää myös, kuinka kauan se kesti.

    Koko opastus on `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Rakenna ensimmäinen sivusto, alpha
tutor-l10-s2 = Tarkista tulos
tutor-l11-title = Opastus 2: lisää toinen sivusto
tutor-l11-text =
    Tämä oppitunti alkaa siitä, mihin opastus 1 päättyi. Se lisää toisen sivuston, `beta`, uuteen harjoituskopioon, `zagros-2`.

    Toinen sivusto kopioi käyttäjätilinsä ensimmäiseltä sivustolta. Sen standup muuttaa myös ensimmäistä sivustoa kerran, jotta kopiointi toimii molempiin suuntiin. Vaihe nimeltä `alpha/core` tekee tämän.

    Koko opastus on `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Rakenna toinen sivusto, beta
tutor-l11-s2 = Tarkista molemmat sivustot
tutor-l12-s1 = Suunnittele yhden koneen lisäämistä
tutor-l13-s1 = Suunnittele federaatio
tutor-l14-title = Pidä goatbuilder ajan tasalla
tutor-l14-s1 = Kysy, onko uudempi goatbuilder olemassa
tutor-l15-title = Assay: tarkista, miten goatnet rakennettiin
tutor-l15-text =
    Goatbuilderin `assay`-komento tarkistaa, että goatnet rakennettiin oikein, yksi kerros kerrallaan, hyväksyntätestien avulla. Joka testi saa yhden kolmesta tuloksesta:
    - PASS: testi läpäisi.
    - FAIL: goatnetissa on ongelma.
    - UNKNOWN: mikään ei pystynyt vastaamaan testiin.

    Vain PASS lasketaan. Jokainen FAIL tai UNKNOWN tekee koodin 3.

    `--plan` listaa testit, jotka koskevat goatnetia, kerroksittain, eikä koske mitään konetta. Käytä sitä nähdäksesi, mitä oikea assay kattaisi. Yleinen muoto on `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: ovatko versiot yhtenäiset?
tutor-l16-text =
    Goatbuilderin `survey`-komento tarkistaa yhdellä kertaa, ajavatko samat versiot siellä, missä niiden pitäisi:
    - Jokaisen goatnetin sisällä: ajaako joka kone ilmoitettua versiota, ja täsmäävätkö koneet, joiden pitäisi täsmätä, todella toisiinsa?
    - Goatnetien dev, preprod ja prod välillä: yksi rivi per komponentti, yksi sarake per goatnet.

    Ero, jonka julkaisuprosessi odottaa, raportoidaan, mutta se ei ole virhe. Esimerkki on uusi versio, joka on devissä, mutta ei vielä prodissa. Ero, jota mikään ei ilmoita, näkyy muodossa `difference not declared`. Kone, jota goatbuilder ei voi lukea, on unknown, ei koskaan yhtenäinen.

    Suorita se harjoituskopiossa. Sen `dev` on jäljessä tarkoituksella, joten odota löydöksiä ja koodia 3.
tutor-l16-s1 = Survey harjoituskopio
tutor-l17-title = Endstate: ajaako goatnet tarkalleen sitä, mitä se ilmoittaa?
tutor-l17-text =
    Goatbuilderin `endstate`-komento tarkistaa kaksi asiaa goatnetista:
    1. Joka palvelu, jonka se ilmoittaa, on deployattu, käynnissä, terve ja oikein yhdistetty.
    2. Mikään ei ole käynnissä, jota se ei ilmoita. Ilmoittamattomalle palvelulle sanotaan orpopalvelu.

    Goatbuilderin CLI päättää, mitä sivuston pitäisi ajaa, sivuston omista asetuksista, ei erillisestä listasta.

    Tarkistus, jolla ei ole mitään luettavaa, merkitään UNOBSERVED. Se on rehellinen vastaus, ei epäonnistuminen. FAIL tai orpopalvelu tekee koodin 3.

    Yleinen muoto on `goatbuilder endstate <goatnet>`. Alla oleva komento tarkistaa esimerkkisivuston tallennetusta datasta, eikä se koske mitään konetta.
tutor-l17-s1 = Tarkista esimerkkisivusto
tutor-l18-title = Bonus: ole utelias
tutor-l18-text =
    Tämä oppitunti on valinnainen. Suorita alla oleva komento, ja tutor on valmis.

    Goatbuilderin CLI:ssä on muutama pieni piilotettu ominaisuus. Ne eivät ole ohjeessa tai näissä oppitunneissa. Jotkut vastaavat sanoihin, joita pitkäaikaiset Unix-käyttäjät usein kirjoittavat uuteen ohjelmaan vain nähdäkseen, mitä tapahtuu. Useimmat asiat, joita kokeilet, eivät tee mitään.

    Tämä oppitunti ei kerro, mitä ne ovat. Niiden löytäminen on pointti.

    Alla oleva komento listaa joka virheen, jonka goatbuilder voi selittää. Se on hyvä paikka aloittaa tutkiminen.
tutor-l18-s1 = Listaa joka virhe, jonka goatbuilder voi selittää
tutor-heading = Oppitunti { $n } / { $total } · { $title }
tutor-setup = Suorita tämä ensin kerran tässä terminaalissa:
tutor-not-recording = Tämä terminaali ei tallenna suorituksiasi, joten tutor ei voi merkitä vaiheitasi. Suorita tämä kerran, ja suorita sitten vaiheet:
tutor-type = Tee näitä vaiheita terminaalissa ({ $met } / { $total } valmiina):
tutor-rerun = Suorita `goatbuilder tutor` uudelleen joka vaiheen jälkeen. Se tarkistaa, mitä teit, ja merkitsee vaiheen tehdyksi.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Tervetuloa GOATBUILDER Tutor
tutor-bar-progress = Edistyminen
tutor-bar-done = { $done } / { $total } oppituntia valmiina
tutor-after-step = Vaihe { $met } / { $total } valmis. Seuraava:
tutor-after-lesson = Oppitunti { $n } on valmis. Suorita `goatbuilder tutor` aloittaaksesi oppitunnin { $next }. Lukeaksesi sen uudelleen: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Oppitunti { $n } on valmis, ja niin on tutorkin. Suorita `goatbuilder tutor` nähdäksesi sen merkittynä valmiiksi. Lukeaksesi sen uudelleen: `goatbuilder tutor --lesson { $n }`.

tutor-review = Suoritit tämän oppitunnin, niin tämä sivu on vain luettavaksi. Suorita `goatbuilder tutor` palataksesi takaisin siihen, missä olit.
tutor-other-lessons = Lukeaksesi oppitunnin uudelleen: `goatbuilder tutor --lesson 1`. Nähdäksesi ne kaikki: `goatbuilder tutor --list`.
tutor-l12-title = Lisää kone käynnissä olevalle sivustolle
tutor-l12-text =
    Goatbuilderin `join-node`-komento lisää yhden koneen lisää sivustolle, joka on jo käynnissä. Se tulostaa ensin suunnitelman, yhden rivin per vaihe. Jokaisella vaiheella on tyyppi:
    - tool: goatbuilder ajaa skriptin.
    - ansible: goatbuilder ajaa playbookin.
    - gate: tarkistus, jonka täytyy läpäistä ennen seuraavaa vaihetta.
    - manual: teet tämän itse, ja ajo pysähtyy.

    Alla olevassa komennossa on `--dry-run`. Se lukee tätä checkoutia eikä muuta mitään. Taulukon alla, lue mitä teet käsin.
tutor-l13-title = Yhdistä toisen organisaation goatnetiin
tutor-l13-text =
    Goatbuilderin `federate`-komento yhdistää goatnetisi goatnetiin, jota toinen organisaatio ajaa. Kumpikin puoli rakentaa oman puoliskonsa sopimuksesta. Vaihdatte puoliskot, ja kumpikin puoli yhdistää ne samaksi sopimukseksi.

    Vahvista toisen puolen avain erillisellä tavalla, kuten puhelulla, jota luotat. Avain, joka saapuu heidän pakettinsa sisällä, ei todista mitään.

    Alla olevassa komennossa on `--dry-run`, niin se ei muuta mitään. Oikea ajo pysähtyy vaiheessa 4, koska teet sen vaiheen käsin.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Tämä komento ei ole osa oppituntia { $n }. Suorita `goatbuilder tutor` nähdäksesi, missä olet.
tutor-l1-s2 = Kysy doctorilta yhdestä goatnetista
tutor-l1-text =
    Goatbuilderin CLI rakentaa, deployaa ja julkaisee GOAT-ohjelmistoa. Tarkista asetuksesi kahdella komennolla ennen käyttöä:
    - Goatbuilderin `validate`-komento tarkistaa konfiguraatiotiedoston, `ops/goatbuilder/goatbuilder.yaml`. Se lukee vain tätä checkoutia eikä tarvitse verkkoyhteyttä.
    - Goatbuilderin `doctor`-komento tarkistaa, mitä oikea deploy tarvitsee: työkalut, kirjautumiset, vastaako tämä checkout mainia, ja miten goatbuilder tavoittaa jokaisen goatnetin. Goatnet on yksi GOAT-ympäristö.

    Goatbuilderin `doctor`-komento tulostaa, kuinka monta tarkistusta läpäistiin, ja sitten kunkin ongelman korjauksineen. Tässä oppitunnissa ongelma on tietoa, ei virhe. Lue korjaukset, mutta sinun ei tarvitse tehdä niitä juuri nyt.

    Useimmat oppitunnit eivät vaadi kirjautumista. Oppitunnit 2-5, 10, 11 ja 16 käyttävät harjoituskopiota. Oppitunnit 8, 9, 12, 13, 15, 17 ja 18 lukevat vain tätä checkoutia. Oppitunti 6 tarvitsee `gh`:n ja meshin, oppitunti 7 tarvitsee `gh`:n, ja oppitunti 14 tarvitsee meshin. Yksikään oppitunti ei tarvitse 1Passwordia.
tutor-l14-text =
    Goatbuilderin CLI voi päivittää itsensä kopiosta, jota goatnetisi tarjoaa osoitteessa `get.<site>.<zone>/goatbuilder/`.

    Goatbuilderin `update --check`-komento vain lukee. Se vertaa versiotasi tarjottuun versioon eikä muuta mitään. Koodi 0 tarkoittaa, että olet ajan tasalla. Koodi 3 tarkoittaa, että uudempi versio on olemassa.

    Oikea päivitys, ilman `--check`, varmistaa latauksen tarkistussumman ja allekirjoituksen ennen kuin mitään korvataan. Se vaihtaa sitten tiedoston yhdessä vaiheessa.

    Goatbuilderin CLI:n täytyy tietää, mitä goatnetia kysyä: anna `--site` ja kyseinen goatnet, tai aseta `GOATBUILDER_UPDATE_URL`. Sinun täytyy olla tämän goatnetin meshissä. Jos et ole, `--check` kertoo, että se ei tavoita palvelinta, ja nimeää osoitteen, jota se yritti. Se on oikea tulos tässä oppitunnissa.
tutor-l15-s1 = Listaa testit, jotka koskevat yhtä goatnetia
tutor-l6-text =
    Julkaisu siirtää uuden version kolmen goatnetin läpi järjestyksessä: dev, sitten preprod, sitten prod.

    1. Joku tägää version mainissa, ja CI rakentaa sen.
    2. dev: goatbuilder deployaa sen ja ajaa hyväksyntätestit.
    3. preprod: goatbuilder deployaa sen uudelleen, ajaa testit uudelleen ja kirjaa tulokset.
    4. Versio odottaa sitten preprodissa määrätyn ajan. Mikään ei siirry prodiin itsestään.
    5. prod: tämän odotuksen jälkeen henkilö ajaa julkaisun komennolla `--from prod`. Goatbuilderin CLI deployaa prodiin ja tarkistaa kaikki kolme goatnetia.

    Goatbuilderin `promote status`-komento näyttää, missä kukin komponentti on. Kandidaatti on versio, jota testataan. Julkaisun yleinen muoto on `goatbuilder release <component> --tag <tag> --from dev`. Alla olevat komennot käyttävät yhtä tämän checkoutin komponenttia ja esimerkkitägiä. Joka komennossa on `--dry-run`, joten mitään ei tägätä tai deployata.

    Koeajot tarvitsevat `gh`:n ja meshin. Jos et ole repositorion ylläpitäjä, aseta `GOAT_RC_ROLE_CHECK=0` ennen jokaista `release`-komentoa.
tutor-l9-s1 = Tee oikea virhe goatnetilla, jota ei ole olemassa
tutor-l9-text =
    Kun goatbuilder hylkää, se tulostaa koodin. Koodit, jotka alkavat `GB-`, tulevat goatbuilderilta, `E_` goat CLI:ltä, ja `X-` toiselta työkalulta, jonka goatbuilder ajoi.

    Goatbuilderin `explain`-komento ottaa koodin tai virherivin tekstin ja kertoo, mitä tapahtui, miksi, miten se korjataan, ja mistä voit lukea lisää.

    Kun pyydät goatnetia, jota tämä checkout ei määrittele, syntyy oikea virhe selitettäväksi.
