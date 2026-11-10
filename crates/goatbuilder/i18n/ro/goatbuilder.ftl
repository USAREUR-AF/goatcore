### goatbuilder — ro (ro)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Construiește și livrează un mediu goat: declară, converge, verifică — sub un singur contract de deploy
help-flag-output = Format de ieșire: tabel pentru om; JSON sau YAML pentru program
help-flag-repo = Rădăcina repozitoriului. Implicit, checkout-ul din care rulează acest binar
help-flag-yes = Confirmă un verb declarativ fără prompt (pentru automatizare)
help-flag-frozen = Refuză orice rulare care ar necesita fast-forward al checkout-ului (strict C3)
help-flag-mock = Rulează pe un director de fixture în loc de gazde: fără registry, fără transport, determinist (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Limba pentru ajutor și explicațiile erorilor (un tag precum fr, uk, de); ieșirea citită de un pipeline rămâne în engleză. Implicit: locale-ul OS
help-cmd-validate = Verifică o configurație față de schema versionată și regulile între câmpuri, offline
help-cmd-doctor = Sondează ce îi trebuie unei rulări înainte de orice tăiere: dependențe, prospețimea checkout-ului, transportul fiecărui mediu și că playbook-ul fiecărei componente se rezolvă la un mod țintă definit
help-cmd-status = Declarat versus înregistrat, per mașină: converged, pending sau unknown (C7, C8). Doar citește. Exit 3 cât timp vreo mașină nu e converged
help-cmd-deploy = Pune pe un mediu pin-ul declarat de main și dovedește-l: rezolvă componenta și grupurile mediului din pin-uri, play-ul și modul său țintă, fiecare secret pe care play-ul îl șablonează, transportul; rulează play-ul prin ansible-preflight; citește înapoi ledger-ul de deploy de pe fiecare gazdă și consideră gata doar când cel mai nou rând conține digest-ul declarat cu action=deploy (C7). Pe un fixture (`--mock`), converger-ul scriptat ține locul gazdelor
help-cmd-release = Duce un release candidate de la un tag pe main, prin validarea live a goatnet-ului dev, până la treapta preprod (ADR 1173); după soak, cu `--from prod`, îl gradează, îl integrează în fleet:, face deploy în prod și reverifică fiecare treaptă. Înlocuiește ops/scripts/rc.sh pas cu pas
help-cmd-rollback = Întoarce o componentă deployată la a N-a cea mai recentă ref a sa, în funcție de tipul de artefact, citind ledger-ul de pe gazdă ca sursă de adevăr (ADR 1060). Înlocuiește ops/scripts/rollback.sh: exit 3 refuzat ca nesigur (data, cert, forward-only, reversible:false), 4 ghidat (config, acl)
help-cmd-standup = Un site de la zero la verificat: fazele standup-ului în ordinea doctrinei, cu o poartă fail-closed la fiecare capăt, gazda de observabilitate ca ramură paralelă, clasele de rescue și suprafețele cross-goatnet la final (ADR 1183). Înlocuiește ops/scripts/goat-standup.sh pas cu pas. Pe un fixture (`--mock`), fiecare componentă din fiecare clasă e convergată de converger-ul scriptat, iar timpul declarat → verificat este măsura goatnet-ului de o oră
help-cmd-join-node = Încă o mașină dintr-o clasă pe un site live, declarată → convergată → verificată: secvența executată din docs/tutorials/02-first-federated-node.md ca faze numerotate, cu porțile ei (reach, key, repo work first, baseline, WireGuard în kernel înainte și după enrolment, planul ACL din kernel ca poartă de enrolment, converge-ul clasei, acceptance)
help-cmd-federate = Un peering suveran declarat, convergat și verificat (ADR 1157): jumătatea noastră măsurată, pachetele schimbate out of band, un acord compus și verificat, frunzele emise din el, regulile randate de singurul writer, edge-ul deployat, transportul ales de acord deschis, acceptance în toate cele patru niveluri. Înlocuiește ops/scripts/peering-standup.sh pas cu pas
help-cmd-pins = O SINGURĂ trecere de rezolvare cu DOUĂ randări: pins.yml din group_vars al fiecărui membru listat și artefactul IntentV1 al fiecărui goatnet listat, din ops/pins/fleet-pins.yaml. Înlocuiește ops/pins/generate-pins.py octet cu octet. `--check` validează, regenerează și refuză drift-ul, un intent nesemnat sau alterat și un pin guvernat definit în afara pins.yml (exit 1)
help-cmd-explain = Interpretează o eroare: un cod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) sau orice text de eroare — al acestui binar sau cuvintele unei unelte transportate de el (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Afișează ce s-a întâmplat, de ce, ce de făcut și unde se citește mai mult; `-` citește textul de la stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-fastlane = O singură comandă de la un commit îmbinat până la fiecare goatnet (ADR 1127): excepția deliberată de la scară. Înlocuiește ops/scripts/fastlane.sh pas cu pas; refuză un digest nefixat sau un set de graduare deschis, și rulează deploy-urile prin acest binar
help-cmd-promote = Ghidează și urmărește graduarea release-urilor pe scara goatnet (ADR 1120, 1125): stare, deschidere candidat, graduare, graduare set, calea hotfix, review, pregătire, abandonare, verificare. Înlocuiește ops/release/graduate.py pas cu pas. Nimic de aici nu graduează de la sine: o mutație rulează doar când e invocată, afișează ce a făcut și cere --yes fără terminal (C10, ADR 1177)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Un site de la „verified" la nimic — oglinda lui standup, cu înregistrările păstrate (un burn-down adaugă rânduri teardown, nu șterge niciodată). Pe un fixture (`--mock`) repetă burn-down-ul în ordinea inversă a fazelor; pe hosturi reale afișează ordinea și refuză să execute până când fiecare fază nu-și declară play-ul de teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lecții în stil vimtutor: fiecare indică comanda exactă de rulat pe o copie descartabilă a unui fixture (sau, doar în citire, pe acest checkout), apoi verifică ce a făcut de fapt comanda — exit-ul și rândurile ei — înainte să se deblocheze următoarea. `tutor` arată lecția curentă și o marchează ca terminată când rulările tale o satisfac; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Ascuns (src/fun.rs)
help-cmd-argan = Ascuns (src/fun.rs)
help-cmd-bound = Ascuns (crates/goat-fun)
help-cmd-secrets = Postura declarată a secretelor (ADR 1204): ce store este în vigoare (1password, openbao, sops, git-crypt, env-file, env), ce este disponibil, cum se furnizează ce lipsește și verificările proprii ale store-ului. `init --env <goatnet>` scrie fișierul env al quickstart-ului zero-SaaS: o valoare aleatorie puternică pentru fiecare variabilă pe care playurile goatnet-ului o șablonează, 0600, ignorat de git, fără a roti vreodată o valoare deja existentă. Același modul ca ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Citește, pentru o componentă, ultimul rând din ledger-ul de deploy al fiecărui host și spune dacă indică ref-ul declarat de main (C7). Doar citește; exit 3 cât timp vreun host este pending sau unknown. Aceeași verificare pe care o rulează `deploy --verify-only`
help-cmd-diff = Ce diferă acum față de main (C8): ref-ul declarat de main față de ultimul rând din ledger al fiecărui host, pentru o componentă sau, fără una, pentru toate componentele mediului, citit într-o singură trecere pe fiecare grup de membri. Doar citește; exit 3 la orice drift sau host care nu poate fi citit
help-cmd-history = Ledger-ul de deploy ca timeline, cele mai noi primele, pe fiecare host al mediului (C8): fiecare rând este o înregistrare cu mașina, timpul, acțiunea (deploy sau rollback) și versiunea. Versiunea anterioară a înregistrării și operatorul care a rulat-o sunt în `-o json`. Doar citește
help-cmd-plan = Scrie într-un fișier de plan, pentru revizuire, ce ar face un verb declarativ: ref-urile declarate, play-ul și ținta, ledger-ul, commit-ul lui main, transportul (deploy); tipul de artefact și ref-ul țintă (rollback). Nu atinge starea niciunui host. Rulează-l exact cum a fost revizuit, cu `apply`
help-cmd-apply = Rulează exact un plan revizuit. Refuză (exit 1, indicând ce s-a schimbat) dacă declarația lui main, commit-ul pins, play-ul sau ținta rollback s-au schimbat de când a fost scris planul; un transport schimbat este raportat, nu refuzat
help-cmd-adopt = Declară ce rulează deja un mediu: citește ledger-ele fiecărui host și propune o abatere de pins pentru fiecare componentă al cărei ref în execuție diferă de declarația lui main. Implicit doar raportează; cu --yes, modificările sunt scrise într-un worktree nou din main, pentru revizuire. Nu face niciodată deploy

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Încă nu: { $why }
tutor-lesson-done = Lecția { $n } terminată: { $title }.
tutor-all-done-reset = Toate lecțiile sunt terminate. `goatbuilder tutor --reset` o ia de la capăt; tutorialele merg mai departe: docs/tutorials/.
tutor-all-done = Toate lecțiile sunt terminate. Tutorialele merg mai departe: docs/tutorials/.
tutor-workspace = Spațiu de lucru: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Verifică, strat cu strat, că un goatnet a fost construit corect: fiecare verificare de acceptanță care se aplică site-ului, inclusiv verificările pe care un operator le rulează cu goat CLI. Faza de acceptanță a standup-ului ca o comandă separată. Doar citește; ieșire 3 cât timp o verificare eșuează sau nu poate răspunde
help-cmd-survey = Cât de consecvent este software-ul în fiecare goatnet și între goatnet-urile scării, citit într-o singură trecere paralelă. Doar citește
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Site izolat: nu face fetch la trunk; evaluează checkout-ul în raport cu cel din cache și înregistrează rularea ca offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Verifică dacă un goatnet este în starea finală declarată: fiecare serviciu pe care declarațiile proprii spun că îl rulează este implementat, pornit, sănătos și își face treaba, cu legăturile zero-trust configurate, iar nimic nedeclarat nu rulează. Doar citește; iese cu codul 3 cât timp ceva nu e în regulă
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Cât de consecvent este software-ul în fiecare goatnet și între goatnet-urile din ladder, citit într-o singură trecere paralelă. Doar citire.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Înlocuiește acest binar cu versiunea servită de goatnet la `get.<site>.<zone>/goatbuilder/`: descărcarea este verificată cu `.sha256` și cu `SHA256SUMS` semnat înainte de orice modificare, iar înlocuirea este atomică. Deja la zi: un singur rând, exit 0. `--check` doar citește (exit 3 când ești în urmă). Sursa: `GOATBUILDER_UPDATE_URL`, altfel `--site` din group_vars, altfel cea folosită la ultima actualizare
tutor-prereq = Înainte de a începe: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Colorează glifa ok/FAIL și verdictul din tabel (auto|always|never). `auto` colorează doar într-un terminal; NO

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-verbose = Scrie în stderr ce face o rulare — comenzile externe pe care le lansează și faza de rețea a lui `update` — astfel încât `-o json|yaml` să rămână un document curat. Sau setează RUST_LOG=goatbuilder=debug
help-flag-timeout = Număr de secunde pentru limitarea fazei de conectare/accesare înainte de eșec rapid — singura fază care poate bloca pe o gazdă inaccesibilă (preluarea de către `update` a versiunii servite). NU limitează rularea unui play și nici descărcarea binarului (limitată la blocaj). Citește și GOATBUILDER_TIMEOUT
help-cmd-completion = Afișează la stdout un script de completare pentru shell (`goatbuilder completion zsh >> ~/.zshrc`). Pentru candidați dinamici pe care un script static nu îi poate ști — site-urile, componentele și goatnet-urile declarate de acest checkout — leagă un hook de `goatbuilder _complete` (vezi antetul scriptului generat)
help-cmd-docs = Referința comenzilor: `goatbuilder docs` trimite la ea; `--markdown` o emite integral, generată din arborele live de comenzi, deci nu poate diverge de `--help`
help-cmd-_complete = Ascuns: emite candidați de completare live pentru hook-urile shell, câte unul pe linie (site-urile, componentele și goatnet-urile declarate de acest checkout). Doar citește, nu eșuează niciodată zgomotos — un hook de completare nu trebuie să strice promptul

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Înainte de un rollout, raportul de pregătire și reversibilitate (ADR 1211): trei fapte — State (stateless | stateful), Rollback (pin-revert | restore-required | none) și Scope (single | coupled) — declarate per componentă și FAIL-CLOSED când lipsesc, cu jumătatea de pregătire pe care o dovedesc celelalte verbe (ref descărcabil, pins --check, poarta secretelor, sursă proaspătă, starea finală declarată) și o validare a faptului de rollback (rollback --dry-run pentru pin-revert, fereastra de retenție pentru restore-required). `deploy` îl rulează primul; evaluat pe criterii de onestitate, niciodată un fals verde. Doar citește; exit 3 când nu e curat

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Verifică-ți pregătirea
tutor-l1-s1 = Verifică fișierul de configurare
tutor-l2-title = Citește starea: converged, pending, unknown
tutor-l2-text =
    Comanda goatbuilder `status` compară două lucruri pentru fiecare mașină: versiunea pe care main o declară pentru ea, și versiunea pe care propria ei înregistrare spune că o rulează. Nu schimbă nimic.

    Fiecare mașină este într-una din trei stări:
    - converged: înregistrarea ei numește versiunea declarată. Asta e gata.
    - pending: înregistrarea ei numește o versiune mai veche, sau încă nimic.
    - unknown: goatbuilder nu poate citi înregistrarea ei.

    Doar converged contează ca gata. Dacă vreo mașină este pending sau unknown, `status` se termină cu codul 3.

    De la această lecție încolo, `--mock` îndreaptă goatbuilder către o copie de exercițiu din folderul tău de lecții, în loc de mașini reale. Nimic din ce faci acolo nu ajunge la o mașină reală.

    Nimic nu a fost încă deployat pe `dev`-ul copiei de exercițiu. Așa că așteaptă patru mașini pending și una unknown. Vei vedea mașini converged în lecția 3.
tutor-l2-s1 = Citește starea lui dev
tutor-l3-title = Deploy: planifică-l, rulează-l, dovedește-l
tutor-l3-text =
    Deploy-ul pune versiunea declarată pe fiecare mașină, apoi citește înregistrarea fiecărei mașini pentru a dovedi că a ajuns. Un deploy este gata doar când înregistrarea fiecărei mașini numește acea versiune. Un deploy care a rulat fără erori nu este suficient.

    Vei face deploy la `portal` pe `preprod`-ul copiei de exercițiu. `--dry-run` arată ce s-ar întâmpla și nu schimbă nimic. `--ack` înregistrează cine a făcut-o; comanda îți completează numele din git. După deploy, `status` ar trebui să arate ambele mașini ca `ok` și converged.
tutor-l3-s1 = Planifică deploy-ul
tutor-l3-s2 = Fă deploy la portal pe preprod
tutor-l3-s3 = Verifică că ambele mașini sunt converged
tutor-l4-title = Deviere și istoric
tutor-l4-text =
    Două comenzi te ajută să înțelegi un goatnet înainte și după o schimbare. Amândouă doar citesc.
    - Comanda goatbuilder `diff` listează fiecare mașină a cărei înregistrare diferă de versiunea declarată. O astfel de diferență se numește deviere.
    - Comanda goatbuilder `history` listează fiecare versiune pe care a avut-o o componentă pe fiecare mașină, cea mai nouă prima.

    Pe `dev`, fiecare mașină este rămasă în urmă. Istoricul lui `portal` pe `preprod` acum include deploy-ul tău din lecția 3.
tutor-l4-s1 = Listează devierea pe dev
tutor-l4-s2 = Citește istoricul lui portal pe preprod
tutor-l5-title = Fă rollback, și vezi un rollback nesigur refuzat
tutor-l5-text =
    Un rollback declară o versiune mai veche și o face deploy ca pe orice altă versiune. CLI-ul goatbuilder apoi citește înregistrarea fiecărei mașini pentru a dovedi că a ajuns.

    Unele schimbări nu pot fi anulate în acest fel. Deploy-ul unei versiuni mai vechi nu poate anula date pe care le-a scris o componentă, un certificat, sau o schimbare care se mișcă doar înainte. CLI-ul goatbuilder le refuză cu codul de ieșire 3 și spune ce să faci în loc.

    Componenta `db-schema` a copiei de exercițiu ține date, așa că goatbuilder refuză să îi facă rollback. Refuzul este rezultatul așteptat.
tutor-l5-s1 = Planifică rollback-ul
tutor-l5-s2 = Fă rollback la portal cu o versiune înapoi
tutor-l5-s3 = Încearcă să faci rollback la db-schema, pe care goatbuilder îl refuză
tutor-l6-title = Lansează o versiune nouă: dev, apoi preprod, apoi prod
tutor-l6-s1 = Vezi unde se află fiecare componentă
tutor-l6-s2 = Planifică release-ul din dev
tutor-l6-s3 = Planifică pasul pentru prod
tutor-l7-title = Fastlane, și când să nu îl folosești
tutor-l7-text =
    Fastlane pune o versiune pe fiecare goatnet deodată. Sare peste așteptarea pe preprod și peste înregistrările release-ului.

    Folosește-l doar pentru o remediere urgentă pe care o poți anula revenind la versiunea anterioară. Nu îl folosi niciodată pentru o schimbare de bază de date, o schimbare de identitate sau acces, sau pentru orice nu poți anula în acest fel.

    Fastlane refuză o versiune pe care CI nu a construit-o, și o componentă care face parte dintr-un release încă în curs.

    Forma generală este `goatbuilder fastlane <component> --ref <version> --reason <why>`. Rulează exemplul de mai jos. Are `--dry-run`, așa că nimic nu se schimbă. Are nevoie de `gh`.
tutor-l7-s1 = Planifică un fastlane
tutor-l8-title = Pins: găsește deviere și repară-o în modul corect
tutor-l8-text =
    Un fișier stabilește care versiune rulează fiecare goatnet: `ops/pins/fleet-pins.yaml`. CLI-ul goatbuilder generează celelalte fișiere de versiune din el: fișierele `pins.yml` și fișierele intent semnate.

    Comanda goatbuilder `pins --check` compară fișierele generate cu `fleet-pins.yaml` și refuză orice fișier care diferă. Comanda goatbuilder `pins` le regenerează.

    Folderul tău de lecții are o copie a repository-ului în care cineva a editat manual un fișier `pins.yml`. Nu repara niciodată o editare manuală cu altă editare manuală. Schimbă `fleet-pins.yaml`, apoi regenerează.
tutor-l8-s1 = Verifică acest checkout
tutor-l8-s2 = Verifică copia și găsește editarea manuală
tutor-l8-s3 = Regenerează fișierele copiei
tutor-l8-s4 = Verifică din nou copia
tutor-l9-title = Explică o eroare
tutor-l9-s2 = Explică eroarea prin codul ei
tutor-l9-s3 = Explică eroarea prin textul ei
tutor-l10-title = Tutorial 1: construiește primul site al unui goatnet nou
tutor-l10-text =
    Această lecție construiește un goatnet întreg într-o copie de exercițiu. Goatnet-ul exemplu este `zagros`. Primul său site, `alpha`, are două mașini de bază, o mașină de monitorizare și o mașină de salvare.

    Comanda goatbuilder `standup` pregătește fiecare mașină într-o ordine fixă și verifică fiecare înregistrare. Ultima lui linie spune că site-ul este gata. Pe mașini reale, linia aceea arată și cât a durat.

    Tutorialul complet este `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Construiește primul site, alpha
tutor-l10-s2 = Verifică rezultatul
tutor-l11-title = Tutorial 2: adaugă un al doilea site
tutor-l11-text =
    Această lecție începe de unde s-a terminat tutorialul 1. Adaugă un al doilea site, `beta`, într-o copie de exercițiu nouă, `zagros-2`.

    Un al doilea site copiază conturile de utilizator din primul site. Standup-ul lui schimbă și primul site o singură dată, astfel încât copierea să funcționeze în ambele direcții. Un pas numit `alpha/core` face asta.

    Tutorialul complet este `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Construiește al doilea site, beta
tutor-l11-s2 = Verifică ambele site-uri
tutor-l12-s1 = Planifică adăugarea unei mașini
tutor-l13-s1 = Planifică o federare
tutor-l14-title = Ține goatbuilder la zi
tutor-l14-s1 = Întreabă dacă există un goatbuilder mai nou
tutor-l15-title = Assay: verifică cum a fost construit un goatnet
tutor-l15-text =
    Comanda goatbuilder `assay` verifică dacă un goatnet a fost construit corect, strat cu strat, folosind testele de acceptanță. Fiecare test primește unul din trei rezultate:
    - PASS: testul a trecut.
    - FAIL: goatnet-ul are o problemă.
    - UNKNOWN: nimic nu a putut răspunde la test.

    Doar PASS contează. Orice FAIL sau UNKNOWN face codul de ieșire 3.

    `--plan` listează testele care se aplică unui goatnet, pe strat, și nu atinge nicio mașină. Folosește-l pentru a vedea ce ar acoperi un assay real. Forma generală este `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: sunt versiunile consecvente?
tutor-l16-text =
    Comanda goatbuilder `survey` verifică într-o singură trecere dacă aceleași versiuni rulează unde ar trebui:
    - În interiorul fiecărui goatnet: rulează fiecare mașină versiunea declarată, și mașinile care ar trebui să se potrivească se potrivesc cu adevărat?
    - De-a lungul dev, preprod și prod: un rând pe componentă, o coloană pe goatnet.

    O diferență pe care procesul de release o așteaptă este raportată, dar nu este o eroare. Un exemplu este o versiune nouă care este pe dev dar nu încă pe prod. O diferență pe care nimic nu o declară se citește ca `difference not declared`. O mașină pe care goatbuilder nu o poate citi este unknown, niciodată consecventă.

    Rulează-l pe copia de exercițiu. `dev`-ul ei este rămas în urmă intenționat, așa că așteaptă constatări și codul de ieșire 3.
tutor-l16-s1 = Rulează survey pe copia de exercițiu
tutor-l17-title = Endstate: rulează un goatnet exact ceea ce declară?
tutor-l17-text =
    Comanda goatbuilder `endstate` verifică două lucruri despre un goatnet:
    1. Fiecare serviciu pe care îl declară este deployat, rulează, este sănătos și conectat corect.
    2. Nimic nu rulează pe care nu îl declară. Un serviciu nedeclarat se numește orfan.

    CLI-ul goatbuilder calculează ce ar trebui să ruleze un site din propriile setări ale site-ului, nu dintr-o listă separată.

    O verificare care nu are nimic de citit este marcată ca UNOBSERVED. Acesta este un răspuns onest, nu un eșec. Un FAIL sau un orfan face codul de ieșire 3.

    Forma generală este `goatbuilder endstate <goatnet>`. Comanda de mai jos verifică un site exemplu din date salvate, așa că nu atinge nicio mașină.
tutor-l17-s1 = Verifică site-ul exemplu
tutor-l18-title = Bonus: fii curios
tutor-l18-text =
    Această lecție este opțională. Rulează comanda de mai jos și lecțiile se termină.

    CLI-ul goatbuilder are câteva funcții mici și ascunse. Nu sunt în ajutor sau în aceste lecții. Unele răspund la cuvinte pe care utilizatorii vechi de Unix le tastează des într-un program nou, doar ca să vadă ce se întâmplă. Majoritatea lucrurilor pe care le încerci nu vor face nimic.

    Această lecție nu spune ce sunt. Să le găsești este scopul.

    Comanda de mai jos listează fiecare eroare pe care goatbuilder o poate explica. Este un loc bun de unde să începi să explorezi.
tutor-l18-s1 = Listează fiecare eroare pe care goatbuilder o poate explica
tutor-heading = Lecția { $n } din { $total } · { $title }
tutor-setup = Mai întâi, rulează asta o dată în acest terminal:
tutor-not-recording = Acest terminal nu îți înregistrează rulările, așa că tutorialul nu poate bifa pașii tăi. Rulează asta o dată, apoi rulează pașii:
tutor-type = Fă acești pași în terminalul tău ({ $met } din { $total } finalizați):
tutor-rerun = După fiecare pas, rulează `goatbuilder tutor` din nou. Verifică ce ai făcut și bifează pasul.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Bun venit la GOATBUILDER Tutor
tutor-bar-progress = Progres
tutor-bar-done = { $done } din { $total } lecții finalizate
tutor-after-step = Pasul { $met } din { $total } finalizat. Următor:
tutor-after-lesson = Lecția { $n } este finalizată. Rulează `goatbuilder tutor` ca să începi lecția { $next }. Pentru a o citi din nou: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lecția { $n } este finalizată, și la fel și tutorialul. Rulează `goatbuilder tutor` ca să îl vezi marcat ca finalizat. Pentru a o citi din nou: `goatbuilder tutor --lesson { $n }`.

tutor-review = Ai terminat această lecție, așa că această pagină este doar de citit. Rulează `goatbuilder tutor` ca să te întorci unde ești.
tutor-other-lessons = Pentru a citi din nou o lecție: `goatbuilder tutor --lesson 1`. Pentru a le vedea pe toate: `goatbuilder tutor --list`.
tutor-l12-title = Adaugă o mașină la un site care rulează
tutor-l12-text =
    Comanda goatbuilder `join-node` adaugă încă o mașină la un site care deja rulează. Afișează mai întâi un plan, cu un rând pentru fiecare fază. Fiecare fază are un tip:
    - tool: goatbuilder rulează un script.
    - ansible: goatbuilder rulează un playbook.
    - gate: o verificare care trebuie să treacă înainte de faza următoare.
    - manual: faci asta tu însuți, și rularea se întrerupe.

    Comanda de mai jos are `--dry-run`. Citește acest checkout și nu schimbă nimic. Sub tabel, citește ce faci manual.
tutor-l13-title = Conectează-te la goatnetul altei organizații
tutor-l13-text =
    Comanda goatbuilder `federate` conectează goatnet-ul tău cu un goatnet pe care îl rulează altă organizație. Fiecare parte își construiește jumătatea ei de acord. Voi faceți schimb de jumătăți, și fiecare parte le combină în același acord.

    Confirmă cheia celeilalte părți într-un mod separat, cum ar fi un apel în care ai încredere. O cheie care ajunge înăuntrul pachetului lor nu dovedește nimic.

    Comanda de mai jos are `--dry-run`, deci nu schimbă nimic. O rulare reală se oprește la faza 4, pentru că faci tu acea fază manual.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Această comandă nu face parte din lecția { $n }. Rulează `goatbuilder tutor` ca să vezi unde ești.
tutor-l1-s2 = Întreabă doctor despre un goatnet
tutor-l1-text =
    CLI-ul goatbuilder construiește, implementează și lansează software-ul GOAT. Înainte să-l folosești, verifică-ți pregătirea cu două comenzi:
    - Comanda goatbuilder `validate` verifică fișierul de configurare, `ops/goatbuilder/goatbuilder.yaml`. Citește doar acest checkout și nu are nevoie de rețea.
    - Comanda goatbuilder `doctor` verifică de ce are nevoie un deploy real: uneltele tale, autentificările tale, dacă acest checkout corespunde cu main și cum ajunge goatbuilder la fiecare goatnet. Un goatnet este un mediu GOAT.

    Comanda goatbuilder `doctor` afișează câte verificări au trecut, apoi fiecare problemă cu remedierea ei. În această lecție, o problemă este o informație, nu o greșeală. Citește remedierile, dar nu trebuie să le aplici acum.

    Majoritatea lecțiilor nu au nevoie de autentificare. Lecțiile de la 2 la 5, 10, 11 și 16 folosesc o copie de exercițiu. Lecțiile 8, 9, 12, 13, 15, 17 și 18 doar citesc acest checkout. Lecția 6 are nevoie de `gh` și de mesh, lecția 7 are nevoie de `gh`, iar lecția 14 are nevoie de mesh. Nicio lecție nu are nevoie de 1Password.
tutor-l14-text =
    CLI-ul goatbuilder se poate actualiza din copia pe care goatnet-ul tău o servește la `get.<site>.<zone>/goatbuilder/`.

    Comanda goatbuilder `update --check` doar citește. Compară versiunea ta cu versiunea servită și nu schimbă nimic. Codul de ieșire 0 înseamnă că ești la zi. Codul de ieșire 3 înseamnă că există o versiune mai nouă.

    O actualizare reală, fără `--check`, verifică suma de control și semnătura descărcării înainte să înlocuiască ceva. Apoi schimbă fișierul într-un singur pas.

    CLI-ul goatbuilder trebuie să știe ce goatnet să întrebe: dă `--site` cu acel goatnet sau setează `GOATBUILDER_UPDATE_URL`. Trebuie să fii pe mesh-ul acelui goatnet. Dacă nu ești, `--check` spune că nu poate ajunge la server și numește adresa pe care a încercat-o. Acesta este un rezultat corect pentru această lecție.
tutor-l15-s1 = Listează testele care se aplică unui goatnet
tutor-l6-text =
    O lansare mută o versiune nouă prin trei goatnet-uri, în ordine: dev, apoi preprod, apoi prod.

    1. Cineva pune tag pe versiune pe main, iar CI o construiește.
    2. dev: goatbuilder o implementează și rulează testele de acceptanță.
    3. preprod: goatbuilder o implementează din nou, rulează din nou testele și înregistrează rezultatele.
    4. Versiunea așteaptă apoi pe preprod un timp stabilit. Nimic nu trece singur în prod.
    5. prod: după această așteptare, o persoană rulează lansarea cu `--from prod`. CLI-ul goatbuilder implementează în prod și verifică toate cele trei goatnet-uri.

    Comanda goatbuilder `promote status` arată unde se află fiecare componentă. Un candidat este o versiune aflată în testare. Forma generală a unei lansări este `goatbuilder release <component> --tag <tag> --from dev`. Comenzile de mai jos folosesc o componentă a acestui checkout și un tag de exemplu. Fiecare are `--dry-run`, așa că nimic nu primește tag și nimic nu se implementează.

    Rulările de probă au nevoie de `gh` și de mesh. Dacă nu ești admin al repository-ului, pune `GOAT_RC_ROLE_CHECK=0` înaintea fiecărei comenzi `release`.
tutor-l9-s1 = Provoacă o eroare reală cu un goatnet care nu există
tutor-l9-text =
    Când goatbuilder refuză, afișează un cod. Codurile care încep cu `GB-` vin de la goatbuilder, `E_` de la CLI-ul goat, iar `X-` de la o altă unealtă pe care goatbuilder a rulat-o.

    Comanda goatbuilder `explain` primește un cod, sau textul unei linii de eroare, și îți spune ce s-a întâmplat, de ce, cum repari și unde citești mai mult.

    Dacă ceri un goatnet pe care acest checkout nu îl declară, obții o eroare reală de explicat.
