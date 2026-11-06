### goatbuilder — sq (sq)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Ndërto dhe dërgo një mjedis goat: deklaro, konvergjo, verifiko — nën një kontratë të vetme deploy
help-flag-output = Formati i daljes: tabelë për njeriun; JSON ose YAML për programin
help-flag-repo = Rrënja e repozitorit. Si parazgjedhje, checkout-i ku xhirohet ky binar
help-flag-yes = Konfirmo një folje deklarative pa pyetur (për automatizim)
help-flag-frozen = Refuzo çdo xhirim që do të kërkonte fast-forward të checkout-it (C3 strikte)
help-flag-mock = Xhiro mbi një direktori fixture në vend të hosteve: pa regjistër, pa transport, deterministik (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Gjuha për ndihmën dhe shpjegimet e gabimeve (një etiketë si fr, uk, de); dalja që e lexon një pipeline mbetet në anglisht. Parazgjedhja: locale e OS-it
help-cmd-validate = Kontrollo një konfigurim kundrejt skemës së versionuar dhe rregullave ndërmjet fushave, offline
help-cmd-doctor = Provo çfarë i duhet një xhirimi përpara se të pritet gjë: varësitë, freskia e checkout-it, transporti i çdo mjedisi, dhe që playbook-u i çdo komponenti zgjidhet në një mënyrë objektivi të përcaktuar
help-cmd-status = E deklaruar kundrejt të regjistruarës, për çdo makinë: converged, pending ose unknown (C7, C8). Vetëm lexon. Exit 3 për sa kohë ndonjë makinë nuk është converged
help-cmd-deploy = Vendos pin-in që deklaron main në një mjedis dhe provoje: zgjidh komponentin dhe grupet e mjedisit nga pin-et, play-n dhe mënyrën e objektivit, çdo sekret që play templaton, transportin; xhiro play-n përmes ansible-preflight; lexo ledger-in e deploy nga çdo host dhe shpall përfunduar vetëm kur rreshti më i ri emërton digest-in e deklaruar me action=deploy (C7). Në një fixture (`--mock`) hosteve u zzëvendëson converger-i i skriptuar
help-cmd-release = Çoje një release candidate nga një tag në main, përmes validimit live të goatnet-it dev, deri në shkallën preprod (ADR 1173); pas soak-ut, me `--from prod`, graduaje, paloseni në fleet:, bëj deploy në prod dhe riverifiko çdo shkallë. Zëvendëson ops/scripts/rc.sh hap pas hapi
help-cmd-rollback = Kthe prapa një komponent të deploy-uar te ref-i i tij i N-të më i fundit, sipas llojit të artifact-it, duke lexuar ledger-in në host si burim të së vërtetës (ADR 1060). Zëvendëson ops/scripts/rollback.sh: exit 3 refuzuar si e pasigurt (data, cert, forward-only, reversible:false), 4 e udhëzuar (config, acl)
help-cmd-fastlane = Një komandë nga një commit i bashkuar te çdo goatnet (ADR 1127): përjashtimi i qëllimshëm nga shkalla. Zëvendëson ops/scripts/fastlane.sh hap pas hapi; refuzon një digest të papin-uar ose një set graduimi të hapur, dhe i xhiron deploy-et përmes këtij binari
help-cmd-standup = Një site nga zeroja te i verifikuari: fazat e standup-it sipas rendit të doktrinës me një gate fail-closed në secilin skaj, hosti i vëzhgimit si degë paralele, klasat e shpëtimit dhe sipërfaqet ndër-goatnet në fund (ADR 1183). Zëvendëson ops/scripts/goat-standup.sh hap pas hapi. Në një fixture (`--mock`) çdo komponent i çdo klase konvergjohet nga converger-i i skriptuar dhe koha nga deklaruar → verifikuar është masa e goatnet-it të një ore
help-cmd-join-node = Edhe një makinë e një klase në një site live, deklaruar → converged → verifikuar: sekuenca e ekzekutuar e docs/tutorials/02-first-federated-node.md si fazat e numëruara me gate-t e saj (arritshmëria, çelësi, puna në repo së pari, baseline, WireGuard i kernelit para dhe pas regjistrimit, plani ACL i kernelit si gate i regjistrimit, converge i klasës, pranimi)
help-cmd-federate = Një peering sovran i deklaruar, i konvergjuar dhe i verifikuar (ADR 1157): gjysma jonë e matur, paketat e shkëmbyera jashtë kanalit, një marrëveshje e hartuar dhe e verifikuar, leaf-et e prodhuara prej saj, rregullat e renderuara nga shkrues i vetëm, edge-i i deploy-uar, transporti që zgjodhi marrëveshja i hapur, pranimi në të katër nivelet. Zëvendëson ops/scripts/peering-standup.sh hap pas hapi
help-cmd-pins = NJË kalim zgjidhjeje me DY renderime: pins.yml i group_vars të çdo anëtari të listuar dhe artifact-i IntentV1 i çdo goatnet-i të listuar, nga ops/pins/fleet-pins.yaml. Zëvendëson ops/pins/generate-pins.py bajt për bajt. `--check` validon, rigjeneron dhe refuzon drift-in, një intent të pasinjuar ose të ndryshuar, dhe një pin të qeverisur të përcaktuar jashtë pins.yml (exit 1)
help-cmd-promote = Udhëzo dhe vëzhgo graduimin e release-it nëpër shkallët e goatnet-it (ADR 1120, 1125): status, hap një kandidat, graduaj, graduaj një set, rruga hotfix, review, readiness, abandon, check. Zëvendëson ops/release/graduate.py hap pas hapi. Asgjë këtu nuk graduohet vetë: një mutacion xhirohet vetëm kur thirret, printon çfarë bëri, dhe kërkon --yes pa terminal (C10, ADR 1177)
help-cmd-explain = Interpreto një gabim: një kod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), ose çdo tekst gabimi — të këtij binari, ose fjalët e një mjeti që ai përcolli (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Printon çfarë ndodhi, pse, çfarë të bësh dhe ku të lexosh më shumë; `-` e lexon tekstin nga stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Një site nga i verifikuar në asgjë — pasqyra e standup, regjistrimet mbeten (një burn-down shton rreshta teardown, nuk fshin kurrë). Në një fixture (`--mock`) e provon burn-down në rend të kundërt të fazave; në hostet reale e printon rendin dhe refuzon të ekzekutojë derisa çdo fazë të deklarojë play-n e vet të teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Mësime, në stilin vimtutor: secili tregon komandën e saktë për ta ekzekutuar mbi një kopje të përkohshme të një fixture (ose, vetëm për lexim, mbi këtë checkout), pastaj kontrollon çfarë bëri vërtet ajo komandë — daljen (exit) dhe rreshtat e saj — përpara se të hapet tjetri. `tutor` tregon mësimin ku ndodhesh dhe e shënon të kryer kur ekzekutimet e tua e plotësojnë; `tutor <n>`, `--list`, `--reset`
help-cmd-make = I fshehur (src/fun.rs)
help-cmd-argan = I fshehur (src/fun.rs)
help-cmd-bound = I fshehur (crates/goat-fun)
help-cmd-secrets = Qëndrimi i deklaruar për sekretet (ADR 1204): cili store është në fuqi (1password, openbao, sops, git-crypt, env-file, env), çfarë ofrohet, si të vendosësh atë që mungon, dhe kontrollet e vetë store-it. `init --env <goatnet>` shkruan skedarin env të quickstart-it pa SaaS: një vlerë e fortë e rastësishme për çdo variabël që plays e goatnet-it e përdorin në template, 0600, e shtuar në gitignore, pa rrotulluar kurrë një vlerë që ekziston tashmë. I njëjti modul me ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Lexon për çdo host rreshtin më të ri të ledger-it të deploy për një komponent dhe thotë nëse emëron ref-in që deklaron main (C7). Vetëm lexon; del me kod 3 sa kohë që ndonjë host është pending ose unknown. E njëjta kontroll që ekzekuton `deploy --verify-only`
help-cmd-diff = Çfarë ndryshon nga main tani (C8): ref-i i deklaruar nga main përballë rreshtit më të ri të ledger-it të çdo hosti, për një komponent ose, pa të, për çdo komponent të mjedisit, lexuar në një kalim për çdo grup anëtarësh. Vetëm lexon; del me kod 3 në çdo shmangie (drift) ose host të palexueshëm
help-cmd-history = Ledger-i i deploy si afat kohor, nga më i riu, në çdo host të mjedisit (C8): çdo rresht është një regjistrim i veçantë me makinën, kohën, veprimin (deploy ose rollback) dhe versionin. Versioni i mëparshëm i regjistrimit dhe operatori që e kreu janë në `-o json`. Vetëm lexon
help-cmd-plan = Shkruan në një skedar plani, për rishikim, çfarë do të bënte një folje deklarative: ref-et e deklaruara, play-n dhe objektivin, ledger-in, commit-in e main, transportin (deploy); llojin e artefaktit dhe ref-in objektiv (rollback). Nuk prek gjendjen e asnjë hosti. Ekzekutoje siç është rishikuar me `apply`
help-cmd-apply = Ekzekuton pikërisht një plan të rishikuar. Refuzon (del me kod 1, duke emërtuar çfarë ka lëvizur) kur deklarata e main, commit-i i pins, play ose objektivi i rollback kanë lëvizur që nga shkrimi i planit; një transport i ndryshuar raportohet, nuk refuzohet
help-cmd-adopt = Deklaro çfarë ekzekuton tashmë një mjedis: lexon ledger-et e çdo hosti dhe propozon një devijim pins për çdo komponent ref-i ekzekutuar i të cilit ndryshon nga deklarata e main. Si parazgjedhje është raport; me --yes ndryshimet shkruhen në një worktree të ri të main për rishikim. Nuk bën kurrë deploy

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ende jo: { $why }
tutor-lesson-done = Mësimi { $n } përfundoi: { $title }.
tutor-all-done-reset = Të gjitha mësimet janë kryer. `goatbuilder tutor --reset` fillon nga e para; tutorialët vazhdojnë më tej: docs/tutorials/.
tutor-all-done = Të gjitha mësimet janë kryer. Tutorialët vazhdojnë më tej: docs/tutorials/.
tutor-workspace = Hapësira e punës: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Kontrollon, shtresë pas shtrese, që një goatnet është ndërtuar saktë: çdo kontroll pranimi që vlen për vendndodhjen, përfshirë kontrollet që një operator ekzekuton me goat CLI. Faza e pranimit të standup-it si komandë më vete. Vetëm lexon; dalje 3 për sa kohë që një kontroll dështon ose nuk mund të përgjigjet
help-cmd-survey = Sa i qëndrueshëm është softueri brenda çdo goatnet dhe ndërmjet goatnet-eve të shkallës, i lexuar në një kalim paralel. Vetëm lexon
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Site i izoluar: mos e merr trunk-un; vlerëso checkout-in kundrejt kopjes në cache dhe regjistro ekzekutimin si offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Kontrollo nëse një goatnet është në gjendjen e deklaruar përfundimtare: çdo shërbim që deklarimet e veta thonë se ekzekutohet është i vendosur, në punë, i shëndetshëm dhe po bën punën e tij, me skajet zero-trust të lidhura, dhe asgjë nuk ekzekutohet pa u deklaruar. Vetëm lexon; del me kod 3 sa kohë diçka është jashtë rregullit
help-cmd-survey-long =
    Sa i qëndrueshëm është softueri brenda çdo goatnet dhe ndërmjet goatnet-eve të shkallës, i lexuar në një kalim paralel. Vetëm lexon. Brenda një goatnet: ref-i që deklaron main krahasuar me rreshtin më të ri të ledger-it të çdo hosti dhe me kontejnerët që ai ekzekuton nga ajo imazh (një që punon; një i ndalur vlerësohet vetëm në një rezervë të ngrohtë ku nuk punon asgjë), hostet e një goatnet që nuk përputhen, goat CLI në çdo host krahasuar me atë që ofron goatnet-i i tij, dhe çfarë ofron goat-get krahasuar me main. Ndërmjet goatnet-eve: një rresht për komponent, një kolonë për goatnet, me vlerësimin e shkallës për çdo ndryshim, siç e jep promote status. Dalja 3 për çdo zhvendosje ose çdo gjë të palexuar; një ndryshim që e deklaron shkalla raportohet, por nuk llogaritet si dështim.
help-cmd-update = Zëvendëson këtë skedar binar me versionin që goatnet ofron në `get.<site>.<zone>/goatbuilder/`: shkarkimi kontrollohet kundrejt `.sha256` të tij dhe `SHA256SUMS` të nënshkruar para se të preket çfarëdo, dhe ndërrimi është atomik. Tashmë i përditësuar: një rresht, dalje 0. `--check` vetëm lexon (dalje 3 kur mbetet prapa). Burimi: `GOATBUILDER_UPDATE_URL`, përndryshe `--site` nga group_vars e tij, përndryshe ai i përditësimit të fundit

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Para se të nisësh: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Ngjyros simbolin ok/FAIL dhe verdiktin e tabelës (auto|always|never). `auto` ngjyros vetëm në terminal; NO_COLOR dhe CLICOLOR_FORCE kanë përparësi. JSON dhe YAML nuk ngjyrosen kurrë
help-flag-verbose = Regjistro çfarë bën një ekzekutim — komandat e jashtme që thërret dhe fazën e rrjetit të `update` — në stderr, që `-o json|yaml` të mbetet dokument i pastër. Ose vendos RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekonda për kufizimin e fazës së lidhjes/arritjes para dështimit të shpejtë — faza e vetme që mund të bllokohet te një host i paarritshëm (marrja e versionit të shërbyer nga `update`). NUK kufizon ekzekutimin e vetë një play-i as shkarkimin e binarit (kufizohet me bllokim). Lexon edhe GOATBUILDER_TIMEOUT
help-cmd-completion = Printo në stdout një skript plotësimi për shell (`goatbuilder completion zsh >> ~/.zshrc`). Për kandidatët e gjallë që një skript statik nuk i njeh — sitet, komponentët dhe goatnet-et që deklaron ky checkout — lidh një hook me `goatbuilder _complete` (shih kokën e skriptit të gjeneruar)
help-cmd-docs = Referenca e komandave: `goatbuilder docs` të çon te ajo; `--markdown` nxjerr të gjithën, të përshkruar nga pema e gjallë e komandave, kështu që nuk shkëputet kurrë nga `--help`
help-cmd-_complete = E fshehur: nxjerr kandidatë të gjallë plotësimi për hook-et e shell-it, një për rresht (sitet, komponentët dhe goatnet-et që deklaron ky checkout). Vetëm lexon, nuk dështon kurrë me zhurmë — një hook plotësimi nuk duhet të prishë prompt-in

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Para një rollout, leximi i gatishmërisë dhe i kthyeshmërisë (ADR 1211): tre fakte — State (stateless | stateful), Rollback (pin-revert | restore-required | none) dhe Scope (single | coupled) — deklarohen për çdo komponent dhe FAIL-CLOSED kur mungojnë, me gjysmën e gatishmërisë që e provojnë verbat e tjera (ref i tërhiqshëm, pins --check, porta e sekreteve, burim i freskët, gjendja përfundimtare e deklaruar) dhe një validim të faktit rollback (rollback --dry-run për pin-revert, dritarja e ruajtjes për restore-required). `deploy` e nis së pari; e vlerësuar sipas ndershmërisë, kurrë një e gjelbër e rreme. Vetëm lexon; del me kod 3 kur nuk është i pastër

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kontrollo konfigurimin tënd
tutor-l1-s1 = Kontrollo skedarin e konfigurimit
tutor-l2-title = Lexo gjendjen: converged, pending, unknown
tutor-l2-text =
    Komanda goatbuilder `status` krahason dy gjëra për çdo makinë: versionin që main deklaron për të, dhe versionin që regjistrimi i saj thotë se xhiron. Nuk ndryshon asgjë.

    Çdo makinë është në një nga tri gjendjet:
    - converged (konvergjuar): regjistrimi i saj emërton versionin e deklaruar. Kjo është e kryer.
    - pending (në pritje): regjistrimi i saj emërton një version më të vjetër, ose asgjë ende.
    - unknown (e panjohur): goatbuilder nuk mund të lexojë regjistrimin e saj.

    Vetëm converged llogaritet e kryer. Nëse ndonjë makinë është pending ose unknown, `status` del me kod 3.

    Nga ky mësim, `--mock` e drejton goatbuilder te një kopje praktike në dosjen tënde tutor, në vend të makinave reale. Asgjë që bën atje nuk arrin një makinë reale.

    Asgjë nuk është deploy-uar në `dev` e kopjes praktike deri tani. Prandaj prit katër makina pending dhe një unknown. Makinat converged do t'i shohësh në mësimin 3.
tutor-l2-s1 = Lexo gjendjen e dev
tutor-l3-title = Deploy: planifikoje, ekzekutoje, provoje
tutor-l3-text =
    Deploy vendos versionin e deklaruar në çdo makinë, pastaj lexon regjistrimin e çdo makine për ta provuar se mbërriti. Një deploy konsiderohet i kryer vetëm kur regjistrimi i çdo makine emërton atë version. Një deploy që ka vrapuar pa gabime nuk mjafton.

    Do të bësh deploy të `portal` në `preprod` e kopjes praktike. `--dry-run` tregon çfarë do të ndodhte dhe nuk ndryshon asgjë. `--ack` regjistron kush e bëri; komanda plotëson emrin tënd nga git. Pas deploy-it, `status` duhet të tregojë të dy makinat si `ok` dhe converged.
tutor-l3-s1 = Planifiko deploy-in
tutor-l3-s2 = Bëj deploy të portal në preprod
tutor-l3-s3 = Kontrollo që të dy makinat janë converged
tutor-l4-title = Devijimi dhe historia
tutor-l4-text =
    Dy komanda të ndihmojnë të kuptosh një goatnet para dhe pas një ndryshimi. Të dyja vetëm lexojnë.
    - Komanda goatbuilder `diff` liston çdo makinë regjistrimi i së cilës ndryshon nga versioni i deklaruar. Një ndryshim quhet devijim.
    - Komanda goatbuilder `history` liston çdo version që pati një komponent në çdo makinë, më i riu i pari.

    Në `dev`, çdo makinë është prapa. Historia e `portal` në `preprod` tani përfshin deploy-in tënd nga mësimi 3.
tutor-l4-s1 = Listo devijimin në dev
tutor-l4-s2 = Lexo historinë e portal në preprod
tutor-l5-title = Kthe mbrapsht, dhe shiko si refuzohet një kthim mbrapsht i pasigurt
tutor-l5-text =
    Një kthim mbrapsht deklaron një version më të hershëm dhe e bën deploy si çdo version tjetër. goatbuilder CLI pastaj lexon regjistrimin e çdo makine për ta provuar se mbërriti.

    Disa ndryshime nuk mund të zhbëhen kështu. Deploy i një versioni më të vjetër nuk mund të zhbëjë të dhëna që një komponent ka shkruar, një certifikatë, ose një ndryshim që lëviz vetëm përpara. goatbuilder CLI i refuzon ato me kod 3 dhe thotë çfarë të bësh në vend të kësaj.

    Komponenti `db-schema` i kopjes praktike mban të dhëna, kështu goatbuilder refuzon ta kthejë mbrapsht. Refuzimi është rezultati i pritur.
tutor-l5-s1 = Planifiko kthimin mbrapsht
tutor-l5-s2 = Kthe portal mbrapsht një version
tutor-l5-s3 = Provo të kthesh mbrapsht db-schema, që goatbuilder refuzon
tutor-l6-title = Lësho një version të ri: dev, pastaj preprod, pastaj prod
tutor-l6-s1 = Shiko ku është çdo komponent
tutor-l6-s2 = Bëj një provë të release-it nga dev
tutor-l6-s3 = Bëj një provë të hapit prod
tutor-l7-title = Fastlane, dhe kur mos ta përdorësh
tutor-l7-text =
    Fastlane vendos një version në çdo goatnet njëherësh. Ai anashkalon pritjen në preprod dhe regjistrimet e release.

    Përdore vetëm për një ndreqje urgjente që mund ta zhbësh duke u kthyer te versioni i mëparshëm. Mos e përdor kurrë për një ndryshim të bazës së të dhënave, një ndryshim identiteti ose aksesi, ose çdo gjë që nuk mund ta zhbësh kështu.

    Fastlane refuzon një version që CI nuk e ka ndërtuar, dhe një komponent që është pjesë e një release ende në vazhdim.

    Forma e përgjithshme është `goatbuilder fastlane <component> --ref <version> --reason <why>`. Ekzekuto shembullin më poshtë. Ai ka `--dry-run`, kështu asgjë nuk ndryshon. Ai ka nevojë për `gh`.
tutor-l7-s1 = Bëj një provë të fastlane
tutor-l8-title = Pins: gjej devijimin dhe rregulloje si duhet
tutor-l8-text =
    Një skedar vendos cilin version xhiron çdo goatnet: `ops/pins/fleet-pins.yaml`. goatbuilder CLI gjeneron prej tij skedarët e tjerë të versionit: skedarët `pins.yml` dhe skedarët e nënshkruar intent.

    Komanda goatbuilder `pins --check` krahason skedarët e gjeneruar me `fleet-pins.yaml` dhe refuzon çdo skedar që ndryshon. Komanda goatbuilder `pins` i rigjeneron.

    Dosja jote tutor ka një kopje të repozitorit në të cilën dikush redaktoi me dorë një skedar `pins.yml`. Mos rregullo kurrë një redaktim me dorë me një redaktim tjetër me dorë. Ndrysho `fleet-pins.yaml`, pastaj rigjeneroje.
tutor-l8-s1 = Kontrollo këtë checkout
tutor-l8-s2 = Kontrollo kopjen dhe gjej redaktimin me dorë
tutor-l8-s3 = Rigjenero skedarët e kopjes
tutor-l8-s4 = Kontrollo kopjen përsëri
tutor-l9-title = Shpjego një gabim
tutor-l9-s2 = Shpjego gabimin me kodin e tij
tutor-l9-s3 = Shpjegoje me tekstin e tij
tutor-l10-title = Tutoriali 1: ndërto site-in e parë të një goatnet-i të ri
tutor-l10-text =
    Ky mësim ndërton një goatnet të tërë në një kopje praktike. Goatnet-i shembull është `zagros`. Site-i i tij i parë, `alpha`, ka dy makina kryesore, një makinë vëzhgimi dhe një makinë shpëtimi.

    Komanda goatbuilder `standup` ngre çdo makinë në një rend fiks dhe kontrollon çdo regjistrim. Rreshti i tij i fundit thotë që site-i është kryer. Në makina reale, ai rresht tregon edhe sa kohë mori.

    Tutoriali i plotë është `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Ndërto site-in e parë, alpha
tutor-l10-s2 = Kontrollo rezultatin
tutor-l11-title = Tutoriali 2: shto një site të dytë
tutor-l11-text =
    Ky mësim fillon ku mbaroi tutoriali 1. Ai shton një site të dytë, `beta`, në një kopje praktike të re, `zagros-2`.

    Një site i dytë kopjon llogaritë e përdoruesve nga site-i i parë. Standup-i i tij gjithashtu ndryshon site-in e parë një herë, që kopjimi të funksionojë në të dy drejtimet. Një hap i quajtur `alpha/core` e bën këtë.

    Tutoriali i plotë është `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Ndërto site-in e dytë, beta
tutor-l11-s2 = Kontrollo të dy site-t
tutor-l12-s1 = Planifiko shtimin e një makine
tutor-l13-s1 = Planifiko lidhjen me një goatnet tjetër
tutor-l14-title = Mbaje goatbuilder të përditësuar
tutor-l14-s1 = Pyet nëse ekziston një goatbuilder më i ri
tutor-l15-title = Assay: kontrollo si u ndërtua një goatnet
tutor-l15-text =
    Komanda goatbuilder `assay` kontrollon që një goatnet u ndërtua saktë, një shtresë në herë, duke përdorur testet e pranimit. Çdo test merr një nga tri rezultatet:
    - PASS: testi kaloi.
    - FAIL: goatnet-i ka një problem.
    - UNKNOWN: asgjë nuk mundi t'i përgjigjej testit.

    Vetëm PASS llogaritet. Çdo FAIL ose UNKNOWN e bën kodin 3.

    `--plan` liston testet që vlejnë për një goatnet, sipas shtresës, dhe nuk prek asnjë makinë. Përdore për të parë çfarë do të mbulonte një assay real. Forma e përgjithshme është `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: a janë versionet konsistente?
tutor-l16-text =
    Komanda goatbuilder `survey` kontrollon në një kalim të vetëm nëse të njëjtat versione xhirojnë ku duhet:
    - Brenda çdo goatnet-i: a xhiron çdo makinë versionin e deklaruar, dhe a përputhen vërtet makinat që duhet të përputhen?
    - Nëpër dev, preprod dhe prod: një rresht për komponent, një kolonë për goatnet.

    Një ndryshim që procesi i release-it e pret raportohet, por nuk është gabim. Një shembull është një version i ri që është në dev, por jo akoma në prod. Një ndryshim që asgjë nuk e deklaron lexohet si `difference not declared`. Një makinë që goatbuilder nuk mund ta lexojë është unknown, kurrë konsistente.

    Ekzekutoje në kopjen praktike. `dev` e saj është prapa me qëllim, kështu prit gjetje dhe kod 3.
tutor-l16-s1 = Bëj survey të kopjes praktike
tutor-l17-title = Endstate: a xhiron një goatnet ekzaktësisht çfarë deklaron?
tutor-l17-text =
    Komanda goatbuilder `endstate` kontrollon dy gjëra për një goatnet:
    1. Çdo shërbim që ai deklaron është i bërë deploy, duke xhiruar, i shëndetshëm dhe i lidhur saktë.
    2. Asgjë nuk xhiron që ai nuk e deklaron. Një shërbim i padeklaruar quhet jetim.

    goatbuilder CLI nxjerr çfarë duhet të xhirojë një site nga vendosjet e vetë site-it, jo nga një listë e veçantë.

    Një kontroll që nuk ka asgjë për të lexuar shënohet UNOBSERVED. Ky është një përgjigje e ndershme, jo një dështim. Një FAIL ose një jetim e bën kodin 3.

    Forma e përgjithshme është `goatbuilder endstate <goatnet>`. Komanda më poshtë kontrollon një site shembull nga të dhëna të ruajtura, kështu nuk prek asnjë makinë.
tutor-l17-s1 = Kontrollo site-in shembull
tutor-l18-title = Bonus: ji kurioz
tutor-l18-text =
    Ky mësim është opsional. Ekzekuto komandën më poshtë dhe tutor-i mbaron.

    goatbuilder CLI ka disa veçori të vogla të fshehura. Ato nuk janë te ndihma ose te këto mësime. Disa përgjigjen ndaj fjalëve që përdoruesit e vjetër të Unix shpesh shkruajnë në një program të ri, thjesht për të parë çfarë ndodh. Shumica e gjërave që provon nuk do të bëjnë asgjë.

    Ky mësim nuk thotë çfarë janë. Gjetja e tyre është qëllimi.

    Komanda më poshtë liston çdo gabim që goatbuilder mund të shpjegojë. Është një vend i mirë për të nisur eksplorimin.
tutor-l18-s1 = Listo çdo gabim që goatbuilder mund të shpjegojë
tutor-heading = Mësimi { $n } nga { $total } · { $title }
tutor-setup = Së pari, ekzekuto këtë një herë në këtë terminal:
tutor-not-recording = Ky terminal nuk po regjistron ekzekutimet e tua, kështu tutor-i nuk mund të shënojë hapat e tu. Ekzekuto këtë një herë, pastaj ekzekuto hapat:
tutor-type = Bëj këto hapa në terminalin tënd ({ $met } nga { $total } të kryera):
tutor-rerun = Pas çdo hapi, ekzekuto përsëri `goatbuilder tutor`. Ai kontrollon çfarë bëre dhe shënon hapin.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Mirë se vjen në GOATBUILDER Tutor
tutor-bar-progress = Progresi
tutor-bar-done = { $done } nga { $total } mësime të kryera
tutor-after-step = Hapi { $met } nga { $total } u krye. Tjetra:
tutor-after-lesson = Mësimi { $n } është kryer. Ekzekuto `goatbuilder tutor` për të filluar mësimin { $next }. Për ta lexuar përsëri: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Mësimi { $n } është kryer, dhe kështu është edhe tutor-i. Ekzekuto `goatbuilder tutor` për ta parë të shënuar si kryer. Për ta lexuar përsëri: `goatbuilder tutor --lesson { $n }`.

tutor-review = E ke përfunduar këtë mësim, kështu që kjo faqe është vetëm për lexim. Ekzekuto `goatbuilder tutor` për t'u kthyer ku ishe.
tutor-other-lessons = Për të lexuar një mësim përsëri: `goatbuilder tutor --lesson 1`. Për t'i parë të gjitha: `goatbuilder tutor --list`.
tutor-l12-title = Shto një makinë në një site që tashmë xhiron
tutor-l12-text =
    Komanda goatbuilder `join-node` shton një makinë tjetër te një site që tashmë xhiron. Fillimisht shtyp një plan, me një rresht për çdo fazë. Çdo fazë ka një lloj:
    - tool: goatbuilder ekzekuton një script.
    - ansible: goatbuilder ekzekuton një playbook.
    - gate: një kontroll që duhet të kalojë para fazës tjetër.
    - manual: ti e bën këtë vetë, dhe ekzekutimi ndalon.

    Komanda më poshtë ka `--dry-run`. Ajo lexon këtë checkout dhe nuk ndryshon asgjë. Poshtë tabelës, lexo çfarë bën me dorë.
tutor-l13-title = Lidhu me goatnet-in e një organizate tjetër
tutor-l13-text =
    Komanda goatbuilder `federate` lidh goatnet-in tënd me një goatnet që xhiron një organizatë tjetër. Çdo anë ndërton gjysmën e saj të një marrëveshjeje. Shkëmben gjysmat, dhe çdo anë i kombinon në të njëjtën marrëveshje.

    Konfirmo çelësin e anës tjetër në një mënyrë tjetër, si një telefonatë që i beson. Një çelës që vjen brenda paketës së tyre nuk provon asgjë.

    Komanda më poshtë ka `--dry-run`, kështu që nuk ndryshon asgjë. Një ekzekutim i vërtetë ndalon në fazën 4, sepse atë fazë e bën me dorë.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Kjo komandë nuk është pjesë e mësimit { $n }. Ekzekuto `goatbuilder tutor` për të parë ku je.
tutor-l1-s2 = Pyet doctor për një goatnet
tutor-l1-text =
    CLI goatbuilder ndërton, vendos dhe lëshon softuerin GOAT. Para se ta përdorësh, kontrollo konfigurimin tënd me dy komanda:
    - Komanda goatbuilder `validate` kontrollon skedarin e konfigurimit, `ops/goatbuilder/goatbuilder.yaml`. Lexon vetëm këtë checkout dhe nuk ka nevojë për rrjet.
    - Komanda goatbuilder `doctor` kontrollon çfarë i duhet një deploy-i të vërtetë: mjetet e tua, hyrjet e tua, nëse ky checkout përputhet me main dhe si arrin goatbuilder te çdo goatnet. Një goatnet është një mjedis GOAT.

    Komanda goatbuilder `doctor` shfaq sa kontrolle kaluan, pastaj çdo problem me rregullimin e tij. Në këtë mësim një problem është informacion, jo gabim. Lexo rregullimet, por nuk ke nevojë t'i zbatosh tani.

    Shumica e mësimeve nuk kanë nevojë për hyrje. Mësimet 2 deri 5, 10, 11 dhe 16 përdorin një kopje praktike. Mësimet 8, 9, 12, 13, 15, 17 dhe 18 vetëm lexojnë këtë checkout. Mësimi 6 ka nevojë për `gh` dhe mesh-in, mësimi 7 ka nevojë për `gh`, dhe mësimi 14 ka nevojë për mesh-in. Asnjë mësim nuk ka nevojë për 1Password.
tutor-l14-text =
    CLI goatbuilder mund të përditësohet nga kopja që goatnet-i yt shërben në `get.<site>.<zone>/goatbuilder/`.

    Komanda goatbuilder `update --check` vetëm lexon. Krahason versionin tënd me versionin e shërbyer dhe nuk ndryshon asgjë. Kodi i daljes 0 do të thotë që je i përditësuar. Kodi i daljes 3 do të thotë që ekziston një version më i ri.

    Një përditësim i vërtetë, pa `--check`, verifikon checksum-in dhe nënshkrimin e shkarkimit para se të zëvendësojë ndonjë gjë. Pastaj e ndërron skedarin me një hap.

    CLI goatbuilder duhet të dijë cilin goatnet të pyesë: jep `--site` me atë goatnet, ose vendos `GOATBUILDER_UPDATE_URL`. Duhet të jesh në mesh-in e atij goatnet-i. Nëse nuk je, `--check` thotë që nuk mund ta arrijë serverin dhe tregon adresën që provoi. Ky është një rezultat i saktë për këtë mësim.
tutor-l15-s1 = Listo testet që vlejnë për një goatnet
tutor-l6-text =
    Një lëshim e kalon një version të ri nëpër tre goatnet me radhë: dev, pastaj preprod, pastaj prod.

    1. Dikush i vë tag versionit në main, dhe CI e ndërton.
    2. dev: goatbuilder e vendos dhe ekzekuton testet e pranimit.
    3. preprod: goatbuilder e vendos përsëri, ekzekuton përsëri testet dhe regjistron rezultatet.
    4. Versioni pastaj pret në preprod për një kohë të caktuar. Asgjë nuk kalon në prod vetvetiu.
    5. prod: pas kësaj pritjeje, një person ekzekuton lëshimin me `--from prod`. CLI goatbuilder vendos në prod dhe kontrollon të tre goatnet-et.

    Komanda goatbuilder `promote status` tregon ku ndodhet çdo komponent. Një kandidat është një version që po testohet. Forma e përgjithshme e një lëshimi është `goatbuilder release <component> --tag <tag> --from dev`. Komandat më poshtë përdorin një komponent të këtij checkout-i dhe një tag shembull. Secila ka `--dry-run`, kështu që asgjë nuk merr tag dhe asgjë nuk vendoset.

    Ekzekutimet provë kanë nevojë për `gh` dhe mesh-in. Nëse nuk je admin i repository-t, vendos `GOAT_RC_ROLE_CHECK=0` para çdo komande `release`.
tutor-l9-s1 = Krijo një gabim të vërtetë me një goatnet që nuk ekziston
tutor-l9-text =
    Kur goatbuilder refuzon, shfaq një kod. Kodet që fillojnë me `GB-` vijnë nga goatbuilder, `E_` nga CLI goat, dhe `X-` nga një mjet tjetër që goatbuilder ekzekutoi.

    Komanda goatbuilder `explain` merr një kod, ose tekstin e një rreshti gabimi, dhe të thotë çfarë ndodhi, pse, si ta rregullosh dhe ku të lexosh më shumë.

    Kërkimi i një goatnet-i që ky checkout nuk e deklaron krijon një gabim të vërtetë për t'u shpjeguar.
