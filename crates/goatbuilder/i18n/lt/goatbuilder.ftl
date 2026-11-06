### goatbuilder — lt (lt)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Sukurkite ir pristatykite goat aplinką: deklaruokite, konverguokite, patikrinkite — pagal vieną diegimo sutartį
help-flag-output = Išvesties formatas: lentelė žmogui; JSON arba YAML programai
help-flag-repo = Saugyklos šaknis. Pagal numatymą — checkout, kuriame paleistas šis binarinis failas
help-flag-yes = Patvirtinti deklaratyvų veiksmą be klausimo (automatizacijai)
help-flag-frozen = Atmesti bet kokį paleidimą, kuriam reikėtų perkelti checkout pirmyn (griežtas C3)
help-flag-mock = Vykdyti su fixture katalogu vietoj mazgų: be registro, be transporto, deterministiškai (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Žinyno ir klaidų paaiškinimų kalba (žymė, pvz., fr, uk, de); išvestis, kurią skaito pipeline, lieka anglų kalba. Pagal numatymą: OS lokalė
help-cmd-validate = Patikrinti konfigūraciją pagal versijuotą schemą ir tarplaukių taisykles, neprisijungus
help-cmd-doctor = Patikrinti, ko reikia paleidimui, kol dar nieko nesukirpta: priklausomybes, checkout šviežumą, kiekvienos aplinkos transportą ir tai, kad kiekvieno komponento playbook išsispręstų į apibrėžtą tikslinį režimą
help-cmd-status = Deklaruota prieš užfiksuotą, pagal mašiną: konverguota, laukia arba nežinoma (C7, C8). Tik skaito. Išėjimo kodas 3, kol bent viena mašina nekonverguota
help-cmd-deploy = Paleisti ant vienos aplinkos pin, kurį deklaruoja main, ir tai įrodyti: iš pins nustatyti komponentą ir aplinkos grupes, play ir jo tikslinį režimą, kiekvieną paslaptį, kurią play įtraukia į šablonus, ir transportą; vykdyti play per ansible-preflight; nuskaityti diegimo žurnalą iš kiekvieno mazgo ir laikyti darbą baigtu tik kai naujausia eilutė nurodo deklaruotą digest su action=deploy (C7). Su fixture (`--mock`) mazgų vietoje veikia scenarijumi valdomas konverteris
help-cmd-release = Nuvesti release candidate nuo žymos ant main per dev goatnet gyvą validaciją iki preprod pakopos (ADR 1173); po soak su `--from prod` jį graduoti, įtraukti į fleet:, diegti prod ir iš naujo patikrinti kiekvieną pakopą. Pakeičia ops/scripts/rc.sh žingsnis į žingsnį
help-cmd-rollback = Atšaukti įdiegtą komponentą iki N-tos naujausios nuorodos pagal artefakto tipą, tiesos šaltiniu laikant mazgo žurnalą (ADR 1060). Pakeičia ops/scripts/rollback.sh: išėjimo kodas 3 — atsisakyta kaip nesaugu (data, cert, forward-only, reversible:false), 4 — vedamas rankinis kelias (config, acl)
help-cmd-fastlane = Viena komanda nuo sujungto commit iki kiekvieno goatnet (ADR 1127): sąmoningas kopėčių išimtis. Pakeičia ops/scripts/fastlane.sh žingsnis į žingsnį; atmeta nepinintaą digest arba atvirą graduacijos rinkinį, o diegimus vykdo per šį binarinį failą
help-cmd-standup = Svetainė nuo nulio iki patikrintos: standup fazės doktrinos tvarka su fail-closed vartais abiejuose galuose, stebėsenos mazgas kaip lygiagreti šaka, rescue klasės ir tarp-goatnet paviršiai paskiausiai (ADR 1183). Pakeičia ops/scripts/goat-standup.sh žingsnis į žingsnį. Su fixture (`--mock`) kiekvienos klasės kiekvieną komponentą konverguoja scenarijumi valdomas konverteris, o laikas deklaruota → patikrinta yra vienos valandos goatnet matas
help-cmd-join-node = Dar viena klasės mašina veikiančioje svetainėje: deklaruota → konverguota → patikrinta. docs/tutorials/02-first-federated-node.md vykdoma seka kaip sunumeruotos fazės su vartais (pasiekiamumas, raktas, pirma repo darbai, baseline, kernel WireGuard prieš ir po enrolment, kernel ACL plokštuma kaip enrolment vartai, klasės konverguotė, priėmimas)
help-cmd-federate = Suvereni peering sąsaja deklaruota, konverguota ir patikrinta (ADR 1157): išmatuojama mūsų pusė, paketai apsikeičiami ne kanalu, sudaromas ir tikrinamas vienas susitarimas, iš jo išleidžiami leaves, taisykles sugeneruoja vienas rašytojas, diegiamas edge, atidaromas susitarimo pasirinktas transportas, priėmimas visose keturiose pakopose. Pakeičia ops/scripts/peering-standup.sh žingsnis į žingsnį
help-cmd-pins = VIENAS sprendimo praėjimas su DVIEM atvaizdavimais: kiekvieno nurodyto nario group_vars pins.yml ir kiekvieno nurodyto goatnet IntentV1 artefaktas iš ops/pins/fleet-pins.yaml. Pakeičia ops/pins/generate-pins.py baitas į baitą. `--check` patikrina, perkuria ir atmeta dreifą, nepasirašytą ar sugadintą intent bei valdomą pin, apibrėžtą ne pins.yml (išėjimo kodas 1)
help-cmd-promote = Vesti ir stebėti release graduaciją per goatnet kopėčias (ADR 1120, 1125): būsena, kandidato atidarymas, graduacija, rinkinio graduacija, hotfix kelias, peržiūra, pasirengimas, atsisakymas, patikra. Pakeičia ops/release/graduate.py žingsnis į žingsnį. Niekas čia pats negraduoja: mutacija vykdoma tik iškvietus, išspausdina ką padarė ir be terminalo prašo --yes (C10, ADR 1177)
help-cmd-explain = Išaiškinti klaidą: kodą (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) arba bet kokį klaidos tekstą — paties šio binarinio failo ar įrankio žodžius, kuriuos jis perdavė (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Parodo kas įvyko, kodėl, ką daryti ir kur skaityti daugiau; `-` skaito tekstą iš stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Svetainė iš patikrintos būsenos į nieką — standup veidrodis, įrašai paliekami (išmontavimas prideda teardown eilutes, niekada netrina). Su fixture (`--mock`) repetuoja išmontavimą atvirkštine fazių tvarka; tikruose hostuose išspausdina tvarką ir atsisako vykdyti, kol kiekviena fazė nedeklaruoja savo teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Pamokos vimtutor stiliumi: kiekviena nurodo tikslią komandą, kurią reikia paleisti su vienkartine fixture kopija (arba, tik skaitymui, su šiuo checkout), o tada patikrina, ką komanda iš tikrųjų padarė — jos exit kodą ir eilutes — prieš atrakinant kitą. `tutor` rodo dabartinę pamoką ir pažymi ją atliktą, kai jūsų paleidimai ją atitinka; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Paslėpta (src/fun.rs)
help-cmd-argan = Paslėpta (src/fun.rs)
help-cmd-bound = Paslėpta (crates/goat-fun)
help-cmd-secrets = Deklaruota paslapčių politika (ADR 1204): kuri saugykla galioja (1password, openbao, sops, git-crypt, env-file, env), kas prieinama, kaip pateikti trūkstamą ir pačios saugyklos patikros. `init --env <goatnet>` įrašo zero-SaaS quickstart env failą: stiprią atsitiktinę reikšmę kiekvienam kintamajam, kurį goatnet play šablonuoja, 0600, gitignored, niekada nekeičiant jau esamos reikšmės. Tas pats modulis kaip ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Perskaito kiekvieno hosto naujausią diegimo žurnalo įrašą vienam komponentui ir pasako, ar jame nurodytas ref, kurį deklaruoja main (C7). Tik skaito; išėjimo kodas 3, kol bent vienas hostas laukiantis arba nežinomas. Tai pati patikra, kurią vykdo `deploy --verify-only`
help-cmd-diff = Kuo dabar skiriasi nuo main (C8): main deklaruotas ref palyginamas su kiekvieno hosto naujausiu žurnalo įrašu – vienam komponentui arba, jei nenurodytas, visiems aplinkos komponentams; vienu perėjimu per kiekvieną narių grupę. Tik skaito; išėjimo kodas 3, jei yra bet koks neatitikimas arba hostas neprieinamas
help-cmd-history = Diegimo žurnalas kaip laiko juosta, naujausi viršuje, visuose aplinkos hostuose (C8): kiekviena eilutė yra atskiras įrašas su mašina, laiku, veiksmu (deploy arba rollback) ir versija. Ankstesnė įrašo versija ir operatorius, kuris jį paleido, yra `-o json`. Tik skaito
help-cmd-plan = Įrašo į plano failą peržiūrai, ką darytų deklaratyvus veiksmas: deklaruotus ref, play ir taikinį, žurnalą, main commit, transportą (deploy); artefakto tipą ir taikinio ref (rollback). Nekeičia jokio hosto būsenos. Peržiūrėtą planą paleiskite su `apply`
help-cmd-apply = Vykdo tiksliai peržiūrėtą planą. Atsisako (išėjimo kodas 1, nurodo, kas pasikeitė), jei nuo plano sukūrimo pasikeitė main deklaracija, pins commit, play arba rollback taikinys; pasikeitęs transportas pranešamas, bet neatmetamas
help-cmd-adopt = Deklaruoja tai, ką aplinka jau vykdo: perskaito visų hostų žurnalus ir pasiūlo pins nukrypimą kiekvienam komponentui, kurio veikiantis ref skiriasi nuo main deklaracijos. Pagal numatytąją tik ataskaita; su --yes pakeitimai įrašomi į naują main worktree peržiūrai. Niekada nediegia

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Dar ne: { $why }
tutor-lesson-done = { $n } pamoka baigta: { $title }.
tutor-all-done-reset = Visos pamokos baigtos. `goatbuilder tutor --reset` pradeda iš naujo; mokymo vadovai gilesni: docs/tutorials/.
tutor-all-done = Visos pamokos baigtos. Mokymo vadovai gilesni: docs/tutorials/.
tutor-workspace = Darbo sritis: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Sluoksnis po sluoksnio patikrina, ar goatnet pastatytas teisingai: kiekvienas vietai taikomas priėmimo patikrinimas, įskaitant tuos, kuriuos operatorius vykdo su goat CLI. Standup priėmimo fazė kaip atskira komanda. Tik skaito; išėjimo kodas 3, kol kuris nors patikrinimas nepavyksta arba negali atsakyti
help-cmd-survey = Kiek nuosekli programinė įranga kiekviename goatnet ir tarp kopėčių goatnet, nuskaityta vienu lygiagrečiu perėjimu. Tik skaito
help-flag-offline = Izoliuota vietovė: negauti pagrindinės šakos (trunk); vertinti darbinę kopiją pagal talpykloje esančią ir paleidimą įrašyti kaip atliktą neprisijungus (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Patikrina, ar goatnet yra deklaruotoje galutinėje būsenoje: kiekviena paslauga, kurią pagal savo deklaracijas turi vykdyti, yra įdiegta, veikia, yra sveika ir atlieka savo darbą, jos zero-trust jungtys sujungtos, o nieko nedeklaruoto nevykdoma. Tik skaito; kol bent kas nors neatitinka, išeina su kodu 3
help-cmd-survey-long =
    Kiek nuosekli programinė įranga kiekviename goatnet ir tarp kopėčių goatnet, nuskaityta vienu lygiagrečiu perėjimu. Tik skaito. Goatnet viduje: main deklaruotas ref palyginamas su kiekvieno hosto naujausia ledger eilute ir to atvaizdo konteineriais (veikiantis konteineris; sustabdytas vertinamas tik šiltame rezerve, kuriame niekas neveikia), vieno goatnet hostai, kurie nesutampa, kiekvieno hosto goat CLI palyginamas su tuo, kurį teikia jo goatnet, ir tai, ką teikia goat-get, palyginama su main. Tarp goatnet: viena eilutė kiekvienam komponentui, vienas stulpelis kiekvienam goatnet, su kopėčių įvertinimu kiekvienam skirtumui, kaip jį pateikia promote status. Išėjimo kodas 3 esant bet kokiam nukrypimui ar neperskaitytam dalykui; kopėčių deklaruotas skirtumas pranešamas, bet nelaikomas klaida.
help-cmd-update = Pakeičia šį dvejetainį failą leidimu, kurį goatnet teikia adresu `get.<site>.<zone>/goatbuilder/`: atsisiuntimas patikrinamas pagal jo `.sha256` ir pasirašytą `SHA256SUMS` prieš ką nors keičiant, o pakeitimas yra atominis. Jau naujausias: viena eilutė, išėjimo kodas 0. `--check` tik skaito (išėjimo kodas 3, kai atsilieka). Šaltinis: `GOATBUILDER_UPDATE_URL`, kitaip `--site` iš jo group_vars, kitaip tas, kurį naudojo paskutinis atnaujinimas

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Prieš pradėdami: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Nuspalvinti lentelės ok/FAIL simbolį ir verdiktą (auto|always|never). `auto` spalvina tik terminale; NO_COLOR ir CLICOLOR_FORCE turi pirmenybę. JSON ir YAML niekada nespalvinami
help-flag-verbose = Į stderr rašyti, ką daro paleidimas — išorines komandas, kurias jis iškviečia, ir `update` tinklo fazę — kad `-o json|yaml` liktų švarus dokumentas. Arba nustatykite RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekundės, kiek riboti prisijungimo/pasiekimo fazę prieš greitai nutraukiant — vienintelę fazę, galinčią strigti dėl nepasiekiamo hosto (`update` aptarnaujamos versijos gavimas). NEriboja paties play vykdymo ar dvejetainio failo atsisiuntimo (ribojamas pagal strigimą). Taip pat skaito GOATBUILDER_TIMEOUT
help-cmd-completion = Išspausdina shell užbaigimo skriptą į stdout (`goatbuilder completion zsh >> ~/.zshrc`). Gyviems kandidatams, kurių statinis skriptas žinoti negali — šio checkout deklaruotoms svetainėms, komponentams ir goatnet — prijunkite hooką prie `goatbuilder _complete` (žr. sugeneruoto skripto antraštę)
help-cmd-docs = Komandų žinynas: `goatbuilder docs` nurodo į jį; `--markdown` išveda visą, sugeneruotą iš gyvo komandų medžio, todėl jis niekada nesiskirs nuo `--help`
help-cmd-_complete = Paslėpta: išveda gyvus užbaigimo kandidatus shell hookams, po vieną eilutėje (šio checkout deklaruotos svetainės, komponentai ir goatnet). Tik skaito, niekada nesiskundžia garsiai — užbaigimo hookas negali sugadinti raginimo eilutės

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Prieš diegimą: pasirengimo ir grąžinamumo ataskaita (ADR 1211): trys faktai — State (stateless | stateful), Rollback (pin-revert | restore-required | none) ir Scope (single | coupled) — deklaruojami kiekvienam komponentui, o nedeklaruoti FAIL-CLOSED; pasirengimo dalį įrodo kitos komandos (ref ištraukiamas, pins --check, paslapčių vartai, šviežias šaltinis, deklaruota galutinė būsena), be to, patikrinamas Rollback faktas (rollback --dry-run esant pin-revert, saugojimo langas esant restore-required). `deploy` ją paleidžia pirmiausia; vertinama pagal sąžiningumą, niekada klaidingai žalia. Tik skaito; išėjimo kodas 3, kai nėra švaru

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Patikrinkite savo sąranką
tutor-l1-s1 = Patikrinkite konfigūracijos failą
tutor-l2-title = Perskaitykite būseną: converged, pending, unknown
tutor-l2-text =
    Komanda goatbuilder `status` palygina du dalykus kiekvienai mašinai: versiją, kurią jai deklaruoja main, ir versiją, kurią nurodo jos pačios įrašas. Jis nekeičia nieko.

    Kiekviena mašina yra viena iš trijų būsenų:
    - converged (konvergavo): jos įrašas nurodo deklaruotą versiją. Tai atlikta.
    - pending (laukia): jos įrašas nurodo senesnę versiją arba kol kas neturi nieko.
    - unknown (nežinoma): goatbuilder negali perskaityti jos įrašo.

    Tik converged skaitoma kaip atlikta. Jei bet kuri mašina yra pending arba unknown, `status` baigia darbą su kodu 3.

    Nuo šios pamokos `--mock` nukreipia goatbuilder į praktikos kopiją jūsų tutor aplanke, o ne į tikras mašinas. Nieko, ką jūs ten darote, nepasieks tikros mašinos.

    Praktikos kopijos `dev` kol kas neturi nieko įdiegto. Taigi tikėkitės keturių pending mašinų ir vienos unknown. Converged mašinas pamatysite 3 pamokoje.
tutor-l2-s1 = Perskaitykite dev būseną
tutor-l3-title = Diegimas: planuokite, paleiskite, įrodykite
tutor-l3-text =
    Diegimas uždeda deklaruotą versiją ant kiekvienos mašinos, tada skaito kiekvienos mašinos įrašą, kad įrodytų, jog ji atkeliavo. Diegimas yra atliktas tik tada, kai kiekvienos mašinos įrašas nurodo tą versiją. Diegimo, kuris praėjo be klaidų, nepakanka.

    Jūs įdiegsite `portal` į praktikos kopijos `preprod`. `--dry-run` parodo, kas įvyktų, ir nekeičia nieko. `--ack` įrašo, kas tai padarė; komanda jūsų vardą paima iš git. Po diegimo `status` turėtų parodyti abi mašinas kaip `ok` ir converged.
tutor-l3-s1 = Suplanuokite diegimą
tutor-l3-s2 = Įdiekite portal į preprod
tutor-l3-s3 = Patikrinkite, kad abi mašinos yra converged
tutor-l4-title = Nuokrypis ir istorija
tutor-l4-text =
    Dvi komandos padeda suprasti goatnet prieš ir po pakeitimo. Abi tik skaito.
    - Komanda goatbuilder `diff` išvardija kiekvieną mašiną, kurios įrašas skiriasi nuo deklaruotos versijos. Skirtumas vadinamas nuokrypiu.
    - Komanda goatbuilder `history` išvardija kiekvieną versiją, kurią komponentas turėjo kiekvienoje mašinoje, naujausią pirmiausia.

    Ant `dev`, kiekviena mašina yra atsilikusi. `portal` istorija ant `preprod` dabar apima jūsų diegimą iš 3 pamokos.
tutor-l4-s1 = Išvardykite nuokrypį ant dev
tutor-l4-s2 = Perskaitykite portal istoriją ant preprod
tutor-l5-title = Atšaukite, ir pamatykite, kaip nesaugus atšaukimas atmetamas
tutor-l5-text =
    Atšaukimas deklaruoja ankstesnę versiją ir diegia ją kaip ir bet kurią kitą versiją. Tada goatbuilder CLI skaito kiekvienos mašinos įrašą, kad įrodytų, jog ji atkeliavo.

    Kai kurių pakeitimų šitaip atšaukti negalima. Senesnės versijos diegimas negali atšaukti duomenų, kuriuos komponentas užrašė, sertifikato, ar pakeitimo, kuris juda tik į priekį. goatbuilder CLI atmeta tokius su kodu 3 ir nurodo, ką daryti vietoj to.

    Praktikos kopijos `db-schema` komponentas laiko duomenis, todėl goatbuilder atmeta jo atšaukimą. Atmetimas yra tikėtasis rezultatas.
tutor-l5-s1 = Suplanuokite atšaukimą
tutor-l5-s2 = Atšaukite portal viena versija atgal
tutor-l5-s3 = Pabandykite atšaukti db-schema, kurį goatbuilder atmeta
tutor-l6-title = Pristatykite naują versiją: dev, tada preprod, tada prod
tutor-l6-s1 = Pažiūrėkite, kur yra kiekvienas komponentas
tutor-l6-s2 = Padarykite release bandomąjį paleidimą iš dev
tutor-l6-s3 = Padarykite prod žingsnio bandomąjį paleidimą
tutor-l7-title = Fastlane, ir kada jo nenaudoti
tutor-l7-text =
    Fastlane uždeda vieną versiją ant visų goatnet vienu metu. Jis apeina laukimą ant preprod ir release įrašus.

    Naudokite jį tik dėl skubaus pataisymo, kurį galite atšaukti grįždami į ankstesnę versiją. Niekada nenaudokite jo duomenų bazės pakeitimui, identiteto ar prieigos pakeitimui, ar bet kam, ko šitaip negalite atšaukti.

    Fastlane atmeta versiją, kurios CI nesukūrė, ir komponentą, kuris yra dar vykstančio release dalis.

    Bendra forma yra `goatbuilder fastlane <component> --ref <version> --reason <why>`. Paleiskite žemiau esantį pavyzdį. Jis turi `--dry-run`, todėl nieko nesikeičia. Jam reikia `gh`.
tutor-l7-s1 = Padarykite fastlane bandomąjį paleidimą
tutor-l8-title = Pins: raskite nuokrypį ir pataisykite jį teisingai
tutor-l8-text =
    Vienas failas nustato, kokią versiją paleidžia kiekvienas goatnet: `ops/pins/fleet-pins.yaml`. Iš jo goatbuilder CLI sugeneruoja kitus versijos failus: `pins.yml` failus ir pasirašytus intent failus.

    Komanda goatbuilder `pins --check` palygina sugeneruotus failus su `fleet-pins.yaml` ir atmeta bet kokį failą, kuris skiriasi. Komanda goatbuilder `pins` juos sugeneruoja iš naujo.

    Jūsų tutor aplanke yra saugyklos kopija, kurioje kažkas rankomis redagavo vieną `pins.yml` failą. Niekada netaisykite rankinio pakeitimo kitu rankiniu pakeitimu. Pakeiskite `fleet-pins.yaml`, tada sugeneruokite iš naujo.
tutor-l8-s1 = Patikrinkite šį checkout
tutor-l8-s2 = Patikrinkite kopiją ir raskite rankinį pakeitimą
tutor-l8-s3 = Sugeneruokite kopijos failus iš naujo
tutor-l8-s4 = Patikrinkite kopiją dar kartą
tutor-l9-title = Paaiškinkite klaidą
tutor-l9-s2 = Paaiškinkite klaidą pagal jos kodą
tutor-l9-s3 = Paaiškinkite klaidą pagal jos tekstą
tutor-l10-title = 1 mokymo vadovas: sukurkite naujo goatnet pirmąją svetainę
tutor-l10-text =
    Šioje pamokoje sukuriamas visas goatnet praktikos kopijoje. Pavyzdinis goatnet yra `zagros`. Jo pirmoji svetainė, `alpha`, turi dvi pagrindines mašinas, vieną stebėjimo mašiną ir vieną gelbėjimo mašiną.

    Komanda goatbuilder `standup` sutvarko kiekvieną mašiną nustatyta tvarka ir patikrina kiekvieną įrašą. Jo paskutinė eilutė sako, kad svetainė baigta. Ant tikrų mašinų ta eilutė dar parodo, kiek laiko tai užėmė.

    Visas mokymo vadovas yra `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Sukurkite pirmąją svetainę, alpha
tutor-l10-s2 = Patikrinkite rezultatą
tutor-l11-title = 2 mokymo vadovas: pridėkite antrą svetainę
tutor-l11-text =
    Šia pamoka pradedama nuo to, kur baigėsi 1 mokymo vadovas. Ji prideda antrą svetainę, `beta`, naujoje praktikos kopijoje, `zagros-2`.

    Antra svetainė kopijuoja naudotojų paskyras iš pirmos svetainės. Jos standup taip pat vieną kartą pakeičia pirmą svetainę, kad kopijavimas veiktų abiem kryptimis. Tai padaro žingsnis, pavadintas `alpha/core`.

    Visas mokymo vadovas yra `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Sukurkite antrąją svetainę, beta
tutor-l11-s2 = Patikrinkite abi svetaines
tutor-l12-s1 = Suplanuokite vienos mašinos pridėjimą
tutor-l13-s1 = Suplanuokite goatnet sujungimą
tutor-l14-title = Palaikykite goatbuilder atnaujintą
tutor-l14-s1 = Paklauskite, ar yra naujesnis goatbuilder
tutor-l15-title = Assay: patikrinkite, kaip goatnet buvo pastatytas
tutor-l15-text =
    Komanda goatbuilder `assay` patikrina, ar goatnet buvo pastatytas teisingai, po vieną sluoksnį, naudodamas priėmimo testus. Kiekvienas testas gauna vieną iš trijų rezultatų:
    - PASS: testas praėjo.
    - FAIL: goatnet turi problemą.
    - UNKNOWN: nieko negalėjo atsakyti į testą.

    Tik PASS skaitomas. Bet koks FAIL ar UNKNOWN padaro kodą 3.

    `--plan` išvardija testus, kurie taikomi goatnet, pagal sluoksnį, ir neprisiliečia prie jokios mašinos. Naudokite jį, kad pamatytumėte, ką apimtų tikras assay. Bendra forma yra `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: ar versijos nuoseklios?
tutor-l16-text =
    Komanda goatbuilder `survey` vienu praėjimu patikrina, ar tos pačios versijos veikia tose vietose, kur turi:
    - Kiekvieno goatnet viduje: ar kiekviena mašina paleidžia deklaruotą versiją, ir ar mašinos, kurios turėtų sutapti, iš tiesų sutampa?
    - Per dev, preprod ir prod: viena eilutė kiekvienam komponentui, vienas stulpelis kiekvienam goatnet.

    Skirtumas, kurio tikisi release procesas, pranešamas, bet nėra klaida. Pavyzdys yra nauja versija, kuri yra dev, bet kol kas nėra prod. Skirtumas, kurio nieko nedeklaruoja, skaitomas kaip `difference not declared`. Mašina, kurios goatbuilder negali perskaityti, yra unknown, niekada nelaikoma nuoseklia.

    Paleiskite jį praktikos kopijoje. Jos `dev` tyčia atsilikęs, taigi tikėkitės radinių ir kodo 3.
tutor-l16-s1 = Paleiskite survey praktikos kopijoje
tutor-l17-title = Endstate: ar goatnet paleidžia lygiai tai, ką deklaruoja?
tutor-l17-text =
    Komanda goatbuilder `endstate` patikrina du dalykus apie goatnet:
    1. Kiekviena paslauga, kurią jis deklaruoja, yra įdiegta, veikia, sveika ir teisingai sujungta.
    2. Nieko nepaleista, ko jis nedeklaruoja. Nedeklaruota paslauga vadinama našlaičiu.

    goatbuilder CLI nustato, ką svetainė turėtų paleisti, iš pačios svetainės nuostatų, ne iš atskiro sąrašo.

    Patikrinimas, kuriam nėra ko skaityti, pažymimas UNOBSERVED. Tai sąžiningas atsakymas, ne klaida. FAIL ar našlaitis padaro kodą 3.

    Bendra forma yra `goatbuilder endstate <goatnet>`. Žemiau esanti komanda patikrina pavyzdinę svetainę iš išsaugotų duomenų, taigi ji neprisiliečia prie jokios mašinos.
tutor-l17-s1 = Patikrinkite pavyzdinę svetainę
tutor-l18-title = Priedas: būkite smalsūs
tutor-l18-text =
    Ši pamoka nebūtina. Paleiskite žemiau esančią komandą, ir tutor bus baigtas.

    goatbuilder CLI turi kelias mažas paslėptas funkcijas. Jų nėra žinyne ar šiose pamokose. Kai kurios reaguoja į žodžius, kuriuos ilgamečiai Unix naudotojai dažnai įveda į naują programą, tiesiog norėdami pamatyti, kas atsitiks. Dauguma dalykų, kuriuos pabandysite, nieko nepadarys.

    Šioje pamokoje nepasakyta, kokios jos yra. Jų radimas yra visa esmė.

    Žemiau esanti komanda išvardija kiekvieną klaidą, kurią goatbuilder gali paaiškinti. Tai gera vieta pradėti tyrinėti.
tutor-l18-s1 = Išvardykite kiekvieną klaidą, kurią goatbuilder gali paaiškinti
tutor-heading = { $n } pamoka iš { $total } · { $title }
tutor-setup = Pirmiausia vieną kartą paleiskite tai šiame terminale:
tutor-not-recording = Šis terminalas neįrašo jūsų paleidimų, todėl tutor negali pažymėti jūsų žingsnių. Vieną kartą paleiskite tai, tada paleiskite žingsnius:
tutor-type = Atlikite šiuos žingsnius savo terminale ({ $met } iš { $total } atlikta):
tutor-rerun = Po kiekvieno žingsnio iš naujo paleiskite `goatbuilder tutor`. Jis patikrina, ką padarėte, ir pažymi žingsnį.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Sveiki atvykę į GOATBUILDER Tutor
tutor-bar-progress = Eiga
tutor-bar-done = { $done } iš { $total } pamokų atlikta
tutor-after-step = Žingsnis { $met } iš { $total } atliktas. Toliau:
tutor-after-lesson = Pamoka { $n } baigta. Paleiskite `goatbuilder tutor`, kad pradėtumėte pamoką { $next }. Norėdami ją vėl perskaityti: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Pamoka { $n } baigta, ir tutor taip pat. Paleiskite `goatbuilder tutor`, kad pamatytumėte, kad jis pažymėtas kaip atliktas. Norėdami ją vėl perskaityti: `goatbuilder tutor --lesson { $n }`.

tutor-review = Jūs baigėte šią pamoką, todėl šis puslapis yra tik skaitymui. Paleiskite `goatbuilder tutor`, kad sugrįžtumėte, kur buvote.
tutor-other-lessons = Norėdami vėl perskaityti pamoką: `goatbuilder tutor --lesson 1`. Norėdami pamatyti visas: `goatbuilder tutor --list`.
tutor-l12-title = Pridėkite mašiną į veikiančią svetainę
tutor-l12-text =
    Komanda goatbuilder `join-node` prideda vieną papildomą mašiną į jau veikiančią svetainę. Jis pirmiausia parodo planą, su viena eilute kiekvienai fazei. Kiekviena fazė turi tipą:
    - tool: goatbuilder paleidžia scenarijų.
    - ansible: goatbuilder paleidžia playbook.
    - gate: patikra, kuri turi būti sėkminga prieš kitą fazę.
    - manual: jūs tai padarote patys, ir vykdymas sustoja.

    Komanda žemiau turi `--dry-run`. Ji skaito šį checkout ir nekeičia nieko. Po lentele skaitykite, ką jūs darote ranka.
tutor-l13-title = Prisijunkite prie kitos organizacijos goatnet
tutor-l13-text =
    Komanda goatbuilder `federate` sujungia jūsų goatnet su goatnet, kurį valdo kita organizacija. Kiekviena pusė sukuria savo pusę sutarties. Jūs pasikeičiate pusėmis, ir kiekviena pusė sujungia jas į tą pačią sutartį.

    Patvirtinkite kitos pusės raktą atskiru būdu, pavyzdžiui, skambučiu, kuriuo pasitikite. Raktas, kuris ateina jų pakete, nieko neįrodo.

    Komanda žemiau turi `--dry-run`, todėl ji nekeičia nieko. Tikras vykdymas sustoja 4 fazėje, nes tą fazę jūs darote ranka.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ši komanda nėra pamokos { $n } dalis. Paleiskite `goatbuilder tutor`, kad pamatytumėte, kur esate.
tutor-l1-s2 = Paklauskite doctor apie vieną goatnet
tutor-l1-text =
    goatbuilder CLI kuria, diegia ir išleidžia GOAT programinę įrangą. Prieš ją naudodami, patikrinkite savo sąranką dviem komandomis:
    - Komanda goatbuilder `validate` tikrina konfigūracijos failą `ops/goatbuilder/goatbuilder.yaml`. Ji skaito tik šį checkout ir jai nereikia tinklo.
    - Komanda goatbuilder `doctor` tikrina tai, ko reikia tikram diegimui: jūsų įrankius, jūsų prisijungimus, ar šis checkout atitinka main ir kaip goatbuilder pasiekia kiekvieną goatnet. Goatnet yra viena GOAT aplinka.

    Komanda goatbuilder `doctor` išspausdina, kiek patikrinimų praėjo, o tada kiekvieną problemą su jos taisymu. Šioje pamokoje problema yra informacija, o ne klaida. Perskaitykite taisymus, bet dabar jų taikyti nereikia.

    Daugumai pamokų prisijungti nereikia. Pamokos nuo 2 iki 5, 10, 11 ir 16 naudoja praktikos kopiją. Pamokos 8, 9, 12, 13, 15, 17 ir 18 tik skaito šį checkout. 6 pamokai reikia `gh` ir mesh, 7 pamokai reikia `gh`, o 14 pamokai reikia mesh. Nė vienai pamokai nereikia 1Password.
tutor-l14-text =
    goatbuilder CLI gali atsinaujinti pati iš kopijos, kurią jūsų goatnet teikia adresu `get.<site>.<zone>/goatbuilder/`.

    Komanda goatbuilder `update --check` tik skaito. Ji palygina jūsų versiją su teikiama versija ir nieko nekeičia. Išėjimo kodas 0 reiškia, kad turite naujausią versiją. Išėjimo kodas 3 reiškia, kad yra naujesnė versija.

    Tikras atnaujinimas, be `--check`, patikrina atsisiuntimo kontrolinę sumą ir parašą prieš ką nors pakeisdamas. Tada jis pakeičia failą vienu žingsniu.

    goatbuilder CLI turi žinoti, kurio goatnet klausti: perduokite `--site` su tuo goatnet arba nustatykite `GOATBUILDER_UPDATE_URL`. Turite būti to goatnet mesh tinkle. Jei nesate, `--check` praneša, kad negali pasiekti serverio, ir nurodo adresą, kurį bandė. Tai teisingas šios pamokos rezultatas.
tutor-l15-s1 = Išvardykite testus, kurie taikomi vienam goatnet
tutor-l6-text =
    Išleidimas perkelia naują versiją per tris goatnet iš eilės: dev, tada preprod, tada prod.

    1. Kas nors pažymi versiją žyme main šakoje, o CI ją sukuria.
    2. dev: goatbuilder ją įdiegia ir paleidžia priėmimo testus.
    3. preprod: goatbuilder ją įdiegia dar kartą, dar kartą paleidžia testus ir įrašo rezultatus.
    4. Tada versija nustatytą laiką laukia preprod. Niekas nepereina į prod savaime.
    5. prod: po to laukimo žmogus paleidžia išleidimą su `--from prod`. goatbuilder CLI įdiegia į prod ir patikrina visus tris goatnet.

    Komanda goatbuilder `promote status` rodo, kur yra kiekvienas komponentas. Kandidatas yra testuojama versija. Bendra išleidimo forma yra `goatbuilder release <component> --tag <tag> --from dev`. Toliau pateiktos komandos naudoja vieną šio checkout komponentą ir pavyzdinę žymę. Kiekviena turi `--dry-run`, todėl niekas nepažymimas ir neįdiegiamas.

    Bandomiesiems paleidimams reikia `gh` ir mesh. Jei nesate repozitorijos administratorius, prieš kiekvieną `release` komandą įrašykite `GOAT_RC_ROLE_CHECK=0`.
tutor-l9-s1 = Sukelkite tikrą klaidą su goatnet, kurio nėra
tutor-l9-text =
    Kai goatbuilder atsisako, jis išspausdina kodą. Kodai, prasidedantys `GB-`, ateina iš goatbuilder, `E_` iš goat CLI, o `X-` iš kito įrankio, kurį paleido goatbuilder.

    Komanda goatbuilder `explain` priima kodą arba klaidos eilutės tekstą ir pasako, kas nutiko, kodėl, kaip tai ištaisyti ir kur skaityti daugiau.

    Paprašius goatnet, kurio šis checkout nedeklaruoja, gaunama tikra klaida, kurią galima paaiškinti.
