### goatbuilder — is (is)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Smíða og afhenda goat-umhverfi: lýsa, samstilla, sannreyna — undir einum dreifingarsamningi
help-flag-output = Úttakssnið: tafla fyrir fólk; JSON eða YAML fyrir forrit
help-flag-repo = Rót hirslunnar. Sjálfgefið er checkout-afritið sem þetta forrit er keyrt í
help-flag-yes = Staðfesta yfirlýsandi skipun án fyrirspurnar (fyrir sjálfvirkni)
help-flag-frozen = Hafna hverri keyrslu sem þyrfti að spóla checkout-afritið áfram (strangt C3)
help-flag-mock = Keyra gegn fixture-möppu í stað hýsla: engin skrá, enginn flutningur, ákvarðandi (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Tungumál fyrir hjálp og villuskýringar (merki eins og fr, uk, de); úttak sem ferli les er áfram á ensku. Sjálfgefið: staðfærsla stýrikerfis
help-cmd-validate = Athuga stillingu gegn útgefnu skema og víxlreitareglum, án nettengingar
help-cmd-doctor = Kanna hvað keyrsla þarfnast áður en nokkuð er skorið: kröfur, ferskleika checkout-afrits, flutningsleið hvers umhverfis og að playbook hvers íhlutar leysist í ákveðinn markham
help-cmd-status = Lýst á móti skráðu, fyrir hverja vél: samstillt, í bið eða óþekkt (C7, C8). Les aðeins. Skilar kóða 3 meðan einhver vél er ekki samstillt
help-cmd-deploy = Setja pinnann sem main lýsir á eitt umhverfi og sanna það: leysa íhlutinn og hópa umhverfisins úr pinnunum, leikinn og markham hans, öll leyndarmál sem leikurinn sniðmátar, flutningsleiðina; keyra leikinn gegnum ansible-preflight; lesa dreifingarbókina til baka af hverjum hýsli og telja verkið búið aðeins þegar nýjasta færslan nefnir lýsta digest-gildið með action=deploy (C7). Á fixture (`--mock`) kemur handritaður samstillir í stað hýslanna
help-cmd-release = Fara með útgáfuframbjóðanda frá tagi á main gegnum lifandi sannprófun dev goatnet upp á preprod-þrepið (ADR 1173); eftir þrautaprófun, með `--from prod`, útskrifa hann, fella hann inn í fleet:, dreifa á prod og sannreyna öll þrep aftur. Kemur í stað ops/scripts/rc.sh skref fyrir skref
help-cmd-rollback = Rúlla dreifðum íhlut aftur á N-ta nýjasta ref hans, eftir gerð afurðar, með hýslabókina sem sannleiksgjafa (ADR 1060). Kemur í stað ops/scripts/rollback.sh: kóði 3 hafnað sem óöruggt (data, cert, forward-only, reversible:false), 4 leiðbeint (config, acl)
help-cmd-fastlane = Ein skipun frá sameinðu commit á alla goatnet (ADR 1127): meðvituð undantekning frá stiganum. Kemur í stað ops/scripts/fastlane.sh skref fyrir skref; hafnar ópinnuðu digest-gildi eða opnu útskriftarsetti, og keyrir dreifingarnar gegnum þtta forrit
help-cmd-standup = Staður frá engu í sannreyndan: fasar uppsetningarinnar í röð kenningarinnar með lokað-við-bilun hlið í hvorum enda, vöktunarhýsill sem samhliða leggur, björgunarflokkarnir og þvert-á-goatnet fletirnir síðast (ADR 1183). Kemur í stað ops/scripts/goat-standup.sh skref fyrir skref. Á fixture (`--mock`) er hver íhlutur hvers flokks samstilltur af handritaða samstillinum og tíminn lýst → sannreynt er mælikvarði klukkustundar-goatnet
help-cmd-join-node = Ein vél til viðbótar af flokki á lifandi stað, lýst → samstillt → sannreynt: framkvæmd runa úr docs/tutorials/02-first-federated-node.md sem númeraðir fasar með hliðum sínum (aðgangur, lykill, hirsluvinna fyrst, grunnlína, kjarna-WireGuard fyrir og eftir skráningu, kjarna-ACL-planið sem skráningarhlið, samstilling flokksins, samþykki)
help-cmd-federate = Fullvalda tengsl lýst, samstillt og sannreynd (ADR 1157): okkar helmingur mældur, pakkar skiptast utan bands, einn samningur saminn og sannreyndur, laufin slegin úr honum, reglur sniðnar af einum rithöfundi, jaðarinn dreifður, flutningsleiðin sem samningurinn valdi opnuð, samþykki á öllum fjórum þrepum. Kemur í stað ops/scripts/peering-standup.sh skref fyrir skref
help-cmd-pins = EIN úrlausnarumferð með TVÆR úttaksgerðir: pins.yml í group_vars hvers skráðs meðlims og IntentV1-afurð hvers skráðs goatnet, úr ops/pins/fleet-pins.yaml. Kemur í stað ops/pins/generate-pins.py bæti fyrir bæti. `--check` sannreynir, endurgerir og hafnar frávikum, ósignuðu eða átt við ásetningi og stýrðum pinna sem er skilgreindur utan pins.yml (kóði 1)
help-cmd-promote = Leiðbeina og fylgjast með útskrift útgáfa upp goatnet-stigann (ADR 1120, 1125): staða, opna frambjóðanda, útskrifa, útskrifa safn, neyðarleiðin, rýni, reiðubúnaður, hætta við, athuga. Kemur í stað ops/release/graduate.py skref fyrir skref. Ekkert hér útskrifar sjálfkrafa: breyting keyrir aðeins þegar henni er kallað á, skrifar hvað hún gerði og biður um --yes án útstöðvar (C10, ADR 1177)
help-cmd-explain = Túlka villu: kóða (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) eða hvaða villutexta sem er — þessa forrits eða orð tóls sem það bar áfram (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Skrifar hvað gerðist, hvers vegna, hvað á að gera og hvar má lesa meira; `-` les textann af stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Svæði frá staðfestu í ekkert — spegilmynd standup, skrár haldast (niðurrif bætir við teardown-línum, eyðir aldrei). Á prófunargrunni (`--mock`) er niðurrif æft í öfugri áfangaröð; á raunverulegum hýsingum prentar það röðina og neitar að keyra fyrr en hver áfangi lýsir sinni teardown-áætlun (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Kennslustundir í anda vimtutor: hver nefnir nákvæma skipun til að keyra á einnota afriti af prófunargrunni (eða, skrifvarið, á þessari útgáfu), og athugar svo hvað skipunin gerði í raun — skilakóða og raðir — áður en sú næsta opnast. `tutor` sýnir kennslustundina sem þú ert á og merkir hana lokið þegar keyrslur þínar uppfylla hana; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Falið (src/fun.rs)
help-cmd-argan = Falið (src/fun.rs)
help-cmd-bound = Falið (crates/goat-fun)
help-cmd-secrets = Yfirlýst leyndarmálastaða (ADR 1204): hvaða geymsla gildir (1password, openbao, sops, git-crypt, env-file, env), hvað er tiltækt, hvernig á að láta í té það sem vantar, og eigin athuganir geymslunnar. `init --env <goatnet>` skrifar env-skrá fyrir hraðræsingu án SaaS: sterkt slembigildi fyrir hverja breytu sem play-skrár goatnetsins sníða, 0600, gitignored, endurnýjar aldrei gildi sem þegar er til staðar. Sama eining og ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Les nýjustu færslu hvers hýsils í dreifingarskránni fyrir einn íhlut og segðu hvort hún nefnir það ref sem main skilgreinir (C7). Les aðeins; hættir með kóða 3 meðan einhver hýsill er í bið eða óþekktur. Sama athugun og `deploy --verify-only` keyrir
help-cmd-diff = Hvað er frábrugðið main núna (C8): ref sem main skilgreinir borið saman við nýjustu skráarfærslu hvers hýsils, fyrir einn íhlut eða, án hans, alla íhluti umhverfisins, lesið í einni umferð fyrir hvern meðlimahóp. Les aðeins; hættir með kóða 3 við frávik eða hýsil sem ekki er hægt að lesa
help-cmd-history = Dreifingarskráin sem tímalína, nýjast efst, á öllum hýslum umhverfisins (C8): hver færsla nefnir vélina, tímann, aðgerðina (deploy eða rollback) og útgáfuna. Fyrri útgáfa færslunnar og notandinn sem keyrði hana eru í `-o json`. Les aðeins
help-cmd-plan = Skrifar í áætlunarskrá til yfirferðar hvað yfirlýsandi skipun myndi gera: skilgreind ref, spilun (play) og skotmark, skráin, commit á main, flutningsleiðin (deploy); gerð afurðar og ref skotmarksins (rollback). Snertir ekki stöðu neins hýsils. Keyrðu hana eins og hún var yfirfarin með `apply`
help-cmd-apply = Keyrir nákvæmlega yfirfarna áætlun. Neitar (hættir með kóða 1 og nefnir hvað breyttist) ef skilgreining main, pins-commit, spilunin (play) eða skotmark rollback hefur breyst síðan áætlunin var skrifuð; breytt flutningsleið er tilkynnt, ekki hafnað
help-cmd-adopt = Skilgreindu það sem umhverfið keyrir nú þegar: les skrár allra hýsla og leggur til pins-frávik fyrir hvern íhlut þar sem ref í keyrslu er frábrugðið skilgreiningu main. Sjálfgefið er skýrsla; með --yes eru breytingarnar skrifaðar í nýtt worktree af main til yfirferðar. Dreifir aldrei

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ekki enn: { $why }
tutor-lesson-done = Lexíu { $n } lokið: { $title }.
tutor-all-done-reset = Öllum lexíum er lokið. `goatbuilder tutor --reset` byrjar upp á nýtt; kennsluleiðbeiningarnar ganga lengra: docs/tutorials/.
tutor-all-done = Öllum lexíum er lokið. Kennsluleiðbeiningarnar ganga lengra: docs/tutorials/.
tutor-workspace = Vinnusvæði: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Athugar lag fyrir lag að goatnet hafi verið rétt byggt: allar viðtökuprófanir sem eiga við staðinn, þar á meðal þær sem rekstraraðili keyrir með goat CLI. Viðtökuáfangi standup sem sérstök skipun. Les eingöngu; skilakóði 3 á meðan einhver prófun mistekst eða getur ekki svarað
help-cmd-survey = Hversu samræmdur hugbúnaðurinn er innan hvers goatnet og milli goatnet-a stigans, lesið í einni samhliða yfirferð. Les eingöngu
help-flag-offline = Einangruð starfsstöð: ekki sækja aðalgreinina (trunk); bera checkout saman við skyndiminnisútgáfuna og skrá keyrsluna sem án nettengingar (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Athugar hvort goatnet sé í yfirlýstu lokaástandi: hver þjónusta sem eigin yfirlýsingar segja að eigi að keyra er uppsett, í gangi, heilbrigð og sinnir hlutverki sínu, zero-trust-tengingar hennar eru komnar upp og ekkert keyrir sem er ekki yfirlýst. Les aðeins; skilar kóða 3 meðan eitthvað er í ólagi
help-cmd-survey-long =
    Hversu samræmdur hugbúnaðurinn er innan hvers goatnet og milli goatnet-a stigans, lesið í einni samhliða yfirferð. Les eingöngu. Innan goatnet: ref sem main lýsir yfir borið saman við nýjustu ledger-línu hvers hýsils og gámana sem hann keyrir af þeirri mynd (gámur í gangi; stöðvaður gámur er aðeins metinn á heitum varahýsli þar sem ekkert keyrir), hýslar sama goatnet sem eru ósammála, goat CLI á hverjum hýsli borið saman við það sem goatnet hans býður, og það sem goat-get býður borið saman við main. Milli goatnet-a: ein lína á íhlut, einn dálkur á goatnet, með einkunn stigans fyrir hvern mun eins og promote status gefur hana. Útgangskóði 3 við hvert frávik eða allt sem ekki var lesið; munur sem stiginn lýsir yfir er tilkynntur en telst ekki villa.
help-cmd-update = Skiptir þessari keyrsluskrá út fyrir útgáfuna sem goatnet býður á `get.<site>.<zone>/goatbuilder/`: niðurhalið er borið saman við `.sha256` þess og undirritaða `SHA256SUMS` áður en nokkru er breytt, og skiptin eru atómísk. Þegar nýjasta: ein lína, útgangur 0. `--check` les eingöngu (útgangur 3 ef á eftir). Uppsprettan: `GOATBUILDER_UPDATE_URL`, annars `--site` úr group_vars þess, annars sú sem síðasta uppfærsla notaði

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Áður en þú byrjar: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Litar ok/FAIL-tákn og niðurstöðu töflunnar (auto|always|never). `auto` litar aðeins í útstöð; NO_COLOR og CLICOLOR_FORCE ráða yfir. JSON og YAML eru aldrei lituð
help-flag-verbose = Skráir hvað keyrsla gerir — ytri skipanirnar sem hún ræsir og netfasa `update` — á stderr, svo `-o json|yaml` haldist hreint skjal. Eða stilltu RUST_LOG=goatbuilder=debug
help-flag-timeout = Sekúndur sem tengi-/aðgangsfasinn má taka áður en hann bregst strax — eini fasinn sem getur hangið á óaðgengilegum hýsli (sókn `update` á útgefinni útgáfu). Takmarkar EKKI keyrslu eigin play né niðurhal tvíundarskrárinnar (afmarkað við stöðnun). Les líka GOATBUILDER_TIMEOUT
help-cmd-completion = Prentar skeljarlokunarskriftu á stdout (`goatbuilder completion zsh >> ~/.zshrc`). Fyrir lifandi tillögur sem föst skrifta þekkir ekki — staðina, íhlutina og goatnet sem þetta checkout skilgreinir — tengdu krók við `goatbuilder _complete` (sjá haus skriftunnar)
help-cmd-docs = Skipanahandbókin: `goatbuilder docs` vísar á hana; `--markdown` skilar henni allri, gengið úr lifandi skipanatrénu svo hún geti aldrei vikið frá `--help`
help-cmd-_complete = Falið: gefur út lifandi lokunartillögur fyrir skeljakróka, eina í línu (staðina, íhlutina og goatnet sem þetta checkout skilgreinir). Les aðeins, bilar aldrei með látum — lokunarkrókur má ekki skemma kvaðninguna

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Fyrir útgáfu: yfirlit um viðbúnað og afturkræfni (ADR 1211): þrjár staðreyndir — State (stateless | stateful), Rollback (pin-revert | restore-required | none) og Scope (single | coupled) — skilgreindar fyrir hvern íhlut og FAIL-CLOSED ef þær vantar. Viðbúnaðarhlutinn nýtir það sem hinar skipanirnar sannreyna (ref sótthæft, pins --check, leyndarmálahliðið, ný útgáfa frumkóða, tilgreint lokaástand) og auk þess er staðreyndin um afturköllun staðfest (rollback --dry-run fyrir pin-revert, varðveislutímabilið fyrir restore-required). `deploy` keyrir það fyrst; metið eftir heiðarleika, aldrei falskt grænt. Les aðeins; hættir með kóða 3 ef ekki hreint

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Athuga uppsetningu þína
tutor-l1-s1 = Athugaðu stillingarskrána
tutor-l2-title = Lesa stöðu: converged, pending, unknown
tutor-l2-text =
    Skipunin goatbuilder `status` ber saman tvö atriði fyrir hverja vél: útgáfuna sem main tilgreinir fyrir hana, og útgáfuna sem færsla hennar segir að hún keyri. Þetta breytir engu.

    Hver vél er í einu af þremur ástandum:
    - converged (samstillt): færsla hennar nefnir tilgreindu útgáfuna. Þessu er lokið.
    - pending (í vændum): færsla hennar nefnir eldri útgáfu, eða enga ennþá.
    - unknown (óþekkt): goatbuilder getur ekki lesið færslu hennar.

    Aðeins converged telst lokið. Ef einhver vél er pending eða unknown, endar `status` með kóða 3.

    Frá þessari lexíu og áfram vísar `--mock` goatbuilder á æfingaraafrit í lexíumöppunni þinni í stað raunverulegra véla. Allt sem þú gerir þar nær ekki til raunverulegrar vélar.

    Engu hefur verið dreift til `dev` í æfingaraafritinu ennþá. Búist því við fjórum vélum pending og einni unknown. Þú munt sjá converged vélar í lexíu 3.
tutor-l2-s1 = Lestu stöðuna fyrir dev
tutor-l3-title = Dreifing: áætla hana, keyra hana, sanna hana
tutor-l3-text =
    Dreifing setur tilgreindu útgáfuna á hverja vél og les svo færslu hverrar vélar til að sanna að hún hafi borist. Dreifingu er aðeins lokið þegar færsla hverrar vélar nefnir þá útgáfu. Dreifing sem keyrði án villna er ekki nóg.

    Þú munt dreifa `portal` á `preprod` í æfingaraafritinu. `--dry-run` sýnir hvað myndi gerast og breytir engu. `--ack` skráir hver gerði það, skipunin fyllir sjálf inn nafnið þitt úr git. Eftir dreifinguna ætti `status` að sýna báðar vélarnar sem `ok` og converged.
tutor-l3-s1 = Áætlaðu dreifinguna
tutor-l3-s2 = Dreifðu portal á preprod
tutor-l3-s3 = Athugaðu að báðar vélarnar séu converged
tutor-l4-title = Frávik og saga
tutor-l4-text =
    Tvær skipanir hjálpa þér að skilja goatnet fyrir og eftir breytingu. Báðar lesa aðeins.
    - Skipunin goatbuilder `diff` skráir hverja vél sem færsla er frábrugðin tilgreindu útgáfunni. Munur er kallaður frávik.
    - Skipunin goatbuilder `history` skráir hverja útgáfu sem íhlutur hefur haft á hverri vél, nýjasta fyrst.

    Á `dev` er hver vél á eftir. Saga `portal` á `preprod` innifelur nú dreifingu þína úr lexíu 3.
tutor-l4-s1 = Skráðu frávikin á dev
tutor-l4-s2 = Lestu sögu portal á preprod
tutor-l5-title = Rúlla til baka, og sjá óöruggri afturköllun hafnað
tutor-l5-text =
    Afturköllun tilgreinir fyrri útgáfu og dreifir henni eins og hverri annarri útgáfu. Verkfærið goatbuilder CLI les síðan færslu hverrar vélar til að sanna að hún hafi borist.

    Sumum breytingum er ekki hægt að afturkalla svona. Að dreifa eldri útgáfu getur ekki afturkallað gögn sem íhlutur hefur skrifað, skírteini, eða breytingu sem aðeins gengur áfram. Verkfærið goatbuilder CLI hafnar þeim með kóða 3 og segir hvað á að gera í staðinn.

    Æfingarafritsins `db-schema` íhlutur heldur gögnum, svo goatbuilder hafnar að rúlla honum til baka. Þessi höfnun er niðurstaðan sem vænst er.
tutor-l5-s1 = Áætlaðu afturköllunina
tutor-l5-s2 = Rúllaðu portal til baka um eina útgáfu
tutor-l5-s3 = Reyndu að rúlla db-schema til baka, sem goatbuilder hafnar
tutor-l6-title = Gefa út nýja útgáfu: dev, síðan preprod, síðan prod
tutor-l6-s1 = Skoðaðu hvar hver íhlutur er
tutor-l6-s2 = Gerðu æfingakeyrslu á release frá dev
tutor-l6-s3 = Gerðu æfingakeyrslu á prod-skrefinu
tutor-l7-title = Fastlane, og hvenær á ekki að nota það
tutor-l7-text =
    Fastlane setur eina útgáfu á hvert goatnet í einu. það sleppir biðinni á preprod og release-færslunum.

    Notaðu það aðeins fyrir brýna lagfæringu sem þú getur afturkallað með því að fara aftur í fyrri útgáfu. Notaðu það aldrei fyrir gagnagrunnsbreytingu, auðkennis- eða aðgangsbreytingu, eða neitt sem þú getur ekki afturkallað þannig.

    Fastlane hafnar útgáfu sem CI hefur ekki smíðað, og íhlut sem er hluti af release sem er enn í vinnslu.

    Almennt form er `goatbuilder fastlane <component> --ref <version> --reason <why>`. Keyrðu dæmið hér fyrir neðan. það hefur `--dry-run`, svo engu er breytt. það þarf `gh`.
tutor-l7-s1 = Gerðu æfingakeyrslu á fastlane
tutor-l8-title = Pins: finna frávik og laga það á réttan hátt
tutor-l8-text =
    Ein skrá ákveður hvaða útgáfu hvert goatnet keyrir: `ops/pins/fleet-pins.yaml`. Verkfærið goatbuilder CLI myndar hinar útgáfuskrárnar út frá henni: `pins.yml` skrárnar og undirrituðu intent-skrárnar.

    Skipunin goatbuilder `pins --check` ber saman mynduðu skrárnar við `fleet-pins.yaml` og hafnar hverri skrá sem er frábrugðin. Skipunin goatbuilder `pins` endurmyndar þær.

    Lexíumappan þín hefur afrit af hirslunni þar sem einhver breytti einni `pins.yml` skrá í höndunum. Lagaðu handvirka breytingu aldrei með annarri handvirkri breytingu. Breyttu `fleet-pins.yaml`, og endurmynda svo.
tutor-l8-s1 = Athugaðu þetta checkout
tutor-l8-s2 = Athugaðu afritið og finn handvirku breytinguna
tutor-l8-s3 = Endurmyndaðu skrár afritsins
tutor-l8-s4 = Athugaðu afritið aftur
tutor-l9-title = Útskýra villu
tutor-l9-s2 = Útskýrðu villuna með kóðanum hennar
tutor-l9-s3 = Útskýrðu villuna með textanum hennar
tutor-l10-title = Leiðarvísir 1: byggja fyrsta stað nýs goatnet
tutor-l10-text =
    Þessi lexía byggir heilt goatnet í æfingaraafriti. Dæmið um goatnet er `zagros`. Fyrsti staður þess, `alpha`, hefur tvær kjarnavélar, eina vöktunarvél og eina björgunarvél.

    Skipunin goatbuilder `standup` setur upp hverja vél í fastri röð og athugar hverja færslu. Síðasta lína hennar segir að staðnum sé lokið. Á raunverulegum vélum sýnir sú lína einnig hversu langan tíma það tók.

    Allur leiðarvísirinn er `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Byggðu fyrsta staðinn, alpha
tutor-l10-s2 = Athugaðu niðurstöðuna
tutor-l11-title = Leiðarvísir 2: bæta við öðrum stað
tutor-l11-text =
    Þessi lexía heldur áfram þar sem leiðarvísir 1 endaði. Hún bætir við öðrum stað, `beta`, í nýju æfingaraafriti, `zagros-2`.

    Annar staður afritar notandaaðganga sína frá fyrsta staðnum. Standup hans breytir einnig fyrsta staðnum einu sinni, svo að afritunin virki í báðar áttir. Skref sem kallast `alpha/core` gerir þetta.

    Allur leiðarvísirinn er `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Byggðu annan staðinn, beta
tutor-l11-s2 = Athugaðu báða staðina
tutor-l12-s1 = Áætlaðu að bæta við einni vél
tutor-l13-s1 = Áætlaðu federation
tutor-l14-title = Halda goatbuilder uppfærðu
tutor-l14-s1 = Spurðu hvort til sé nýrri útgáfa af goatbuilder
tutor-l15-title = Assay: athuga hvernig goatnet var byggt
tutor-l15-text =
    Skipunin goatbuilder `assay` athugar að goatnet hafi verið byggt réttilega, eitt lag í senn, með viðtökuprófunum. Hver prófun fær eina af þremur niðurstöðum:
    - PASS: prófunin stóðst.
    - FAIL: goatnet er með vandamál.
    - UNKNOWN: ekkert gat svarað prófuninni.

    Aðeins PASS telst gilt. Hver FAIL eða UNKNOWN skilar útgangskóða 3.

    `--plan` skráir prófanirnar sem gilda fyrir goatnet, eftir lagi, og snertir enga vél. Nota það til að sjá hvað raunveruleg assay myndi ná yfir. Almennt form er `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: eru útgáfurnar samræmdar?
tutor-l16-text =
    Skipunin goatbuilder `survey` athugar í einni yfirferð hvort sömu útgáfur keyri þar sem þær eiga að gera:
    - Innan hvers goatnet: keyrir hver vél tilgreindu útgáfuna, og passa vélar sem eiga að passa saman í raun saman?
    - Milli dev, preprod og prod: ein röð á íhlut, einn dálkur á goatnet.

    Munur sem release ferlið gerir ráð fyrir er tilkynntur en er ekki villa. Dæmi er ný útgáfa sem er á dev en ekki á prod ennþá. Munur sem ekkert tilgreinir lesist sem `difference not declared`. Vél sem goatbuilder getur ekki lesið er unknown, aldrei samræmd.

    Keyrðu það á æfingaraafritinu. `dev` þess er vísvitandi á eftir, svo búist við niðurstöðum og útgangskóða 3.
tutor-l16-s1 = Survey æfingaraafritið
tutor-l17-title = Endstate: keyrir goatnet nákvæmlega það sem það tilgreinir?
tutor-l17-text =
    Skipunin goatbuilder `endstate` athugar tvö atriði um goatnet:
    1. Hver þjónusta sem það tilgreinir er dreift, í keyrslu, heilbrigð og réttilega tengd.
    2. Ekkert keyrir sem það tilgreinir ekki. Ótilgreind þjónusta kallast munaðarlaus þjónusta.

    Verkfærið goatbuilder CLI vinnur út hvað staður ætti að keyra út frá eigin stillingum staðarins, ekki úr sérstökum lista.

    Athugun sem hefur ekkert að lesa er merkt UNOBSERVED. Það er heiðarlegt svar, ekki bilun. FAIL eða munaðarlaus þjónusta skilar útgangskóða 3.

    Almennt form er `goatbuilder endstate <goatnet>`. Skipunin hér fyrir neðan athugar dæmi um stað út frá vistuðum gögnum, svo hún snertir enga vél.
tutor-l17-s1 = Athugaðu dæmisstaðinn
tutor-l18-title = Bónus: vertu forvitinn
tutor-l18-text =
    Þessi lexía er valkvæð. Keyrðu skipunina hér fyrir neðan og þá er tutor lokið.

    Verkfærið goatbuilder CLI hefur fáeina litla falda eiginleika. Þeir eru ekki í hjálpinni eða í þessum lexíum. Einhverjir svara orðum sem langvarandi Unix notendur skrifa oft inn í nýtt forrit, bara til að sjá hvað gerist. Flest sem þú reynir munu ekki gera neitt.

    Þessi lexía segir ekki hvað þeir eru. Að finna þá er tilgangurinn.

    Skipunin hér fyrir neðan skráir hverja villu sem goatbuilder getur útskýrt. Það er góður staður til að byrja að kanna.
tutor-l18-s1 = Skráðu hverja villu sem goatbuilder getur útskýrt
tutor-heading = Lexía { $n } af { $total } · { $title }
tutor-setup = Keyrðu þetta fyrst einu sinni í þessum skjáhermi:
tutor-not-recording = Þessi skjáhermir skráir ekki keyrslur þínar, svo tutor getur ekki merkt skrefin þín. Keyrðu þetta einu sinni, og keyrðu svo skrefin:
tutor-type = Gerðu þessi skref í skjáhermi þínum ({ $met } af { $total } lokið):
tutor-rerun = Eftir hvert skref, keyrðu `goatbuilder tutor` aftur. Það athugar hvað þú gerðir og merkir skrefið.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Velkomin í GOATBUILDER Tutor
tutor-bar-progress = Framvinda
tutor-bar-done = { $done } af { $total } lexíum lokið
tutor-after-step = Skrefi { $met } af { $total } lokið. Næst:
tutor-after-lesson = Lexíu { $n } er lokið. Keyrðu `goatbuilder tutor` til að byrja á lexíu { $next }. Til að lesa hana aftur: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lexíu { $n } er lokið, og tutorinum líka. Keyrðu `goatbuilder tutor` til að sjá það merkt lokið. Til að lesa hana aftur: `goatbuilder tutor --lesson { $n }`.

tutor-review = Þú hefur lokið þssari lexíu, svo þssa síðu er aðeins til að lesa. Keyrðu `goatbuilder tutor` til að fara aftur þngað sem þú varst.
tutor-other-lessons = Til að lesa lexíu aftur: `goatbuilder tutor --lesson 1`. Til að sjá þær allar: `goatbuilder tutor --list`.
tutor-l12-title = Bæta vél við keyrandi stað
tutor-l12-text =
    Skipunin goatbuilder `join-node` bætir einni vél í viðbót við stað sem er þegar í keyrslu. Hún prentar fyrst áætlun, með einni línu fyrir hvern áfanga. Hver áfangi hefur tegund:
    - tool: goatbuilder keyrir skrift.
    - ansible: goatbuilder keyrir leikbók.
    - gate: athugun sem þarf að standast áður en næsti áfangi byrjar.
    - manual: þú gerir þetta, og keyrslan stoppar.

    Skipunin hér fyrir neðan hefur `--dry-run`. Hún les þetta checkout og breytir engu. Undir töflunni lest þú hvað þú gerir handvirkt.
tutor-l13-title = Tengjast goatnet annars fyrirtækis
tutor-l13-text =
    Skipunin goatbuilder `federate` tengir goatnet þitt við goatnet sem annað fyrirtæki keyrir. Hvor hliðin byggir sinn helming af samkomulagi. Þú skiptir á helmingunum, og hvor hliðin sameinar þá í sama samkomulagið.

    Staðfestu lykil hinnar hliðarins á annan hátt, til dæmis með símtali sem þú treystir. Lykill sem kemur innan í pakka þeirra sannar ekkert.

    Skipunin hér fyrir neðan hefur `--dry-run`, svo hún breytir engu. Raunveruleg keyrsla stoppar á áfanga 4, því að þú gerir þann áfanga handvirkt.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Þessi skipun er ekki hluti af lexíu { $n }. Keyrðu `goatbuilder tutor` til að sjá hvar þú ert.
tutor-l1-s2 = Spurðu doctor um eitt goatnet
tutor-l1-text =
    Verkfærið goatbuilder CLI byggir, setur upp og gefur út GOAT hugbúnað. Áður en þú notar það skaltu athuga uppsetninguna þína með tveimur skipunum:
    - Skipunin goatbuilder `validate` athugar stillingarskrána, `ops/goatbuilder/goatbuilder.yaml`. Hún les aðeins þetta checkout og þarf ekkert net.
    - Skipunin goatbuilder `doctor` athugar það sem raunveruleg uppsetning þarf: verkfærin þín, innskráningarnar þínar, hvort þetta checkout passar við main og hvernig goatbuilder nær til hvers goatnet. Goatnet er eitt GOAT umhverfi.

    Skipunin goatbuilder `doctor` prentar hversu margar athuganir stóðust og síðan hvert vandamál með lagfæringu þess. Í þessari kennslustund er vandamál upplýsingar, ekki mistök. Lestu lagfæringarnar, en þú þarft ekki að beita þeim núna.

    Flestar kennslustundir þurfa enga innskráningu. Kennslustundir 2 til 5, 10, 11 og 16 nota æfingarafrit. Kennslustundir 8, 9, 12, 13, 15, 17 og 18 lesa aðeins þetta checkout. Kennslustund 6 þarf `gh` og mesh-netið, kennslustund 7 þarf `gh` og kennslustund 14 þarf mesh-netið. Engin kennslustund þarf 1Password.
tutor-l14-text =
    Verkfærið goatbuilder CLI getur uppfært sig sjálft úr afritinu sem goatnet-ið þitt býður upp á á `get.<site>.<zone>/goatbuilder/`.

    Skipunin goatbuilder `update --check` les aðeins. Hún ber útgáfuna þína saman við útgáfuna sem er í boði og breytir engu. Útgangskóði 0 þýðir að útgáfan þín er nýjust. Útgangskóði 3 þýðir að nýrri útgáfa er til.

    Raunveruleg uppfærsla, án `--check`, sannreynir gátsummu og undirskrift niðurhalsins áður en hún skiptir nokkru út. Síðan skiptir hún skránni út í einu skrefi.

    Verkfærið goatbuilder CLI þarf að vita hvaða goatnet það á að spyrja: gefðu upp `--site` með því goatnet eða stilltu `GOATBUILDER_UPDATE_URL`. Þú verður að vera á mesh-neti þess goatnet. Ef þú ert það ekki segir `--check` að það nái ekki til þjónsins og nefnir vistfangið sem það reyndi. Það er rétt niðurstaða í þessari kennslustund.
tutor-l15-s1 = Listaðu prófin sem eiga við eitt goatnet
tutor-l6-text =
    Útgáfuferli færir nýja útgáfu í gegnum þrjú goatnet í röð: dev, síðan preprod, síðan prod.

    1. Einhver merkir útgáfuna á main og CI byggir hana.
    2. dev: goatbuilder setur hana upp og keyrir samþykktarprófin.
    3. preprod: goatbuilder setur hana upp aftur, keyrir prófin aftur og skráir niðurstöðurnar.
    4. Útgáfan bíður síðan á preprod í ákveðinn tíma. Ekkert færist á prod af sjálfu sér.
    5. prod: eftir þá bið keyrir manneskja útgáfuferlið með `--from prod`. Verkfærið goatbuilder CLI setur upp á prod og athugar öll þrjú goatnet.

    Skipunin goatbuilder `promote status` sýnir hvar hver íhlutur er. Kandídat er útgáfa sem verið er að prófa. Almennt form útgáfuferlis er `goatbuilder release <component> --tag <tag> --from dev`. Skipanirnar hér fyrir neðan nota einn íhlut úr þessu checkout og dæmi um merki. Hver þeirra hefur `--dry-run`, svo ekkert er merkt eða sett upp.

    Prufukeyrslurnar þurfa `gh` og mesh-netið. Ef þú ert ekki stjórnandi gagnageymslunnar skaltu setja `GOAT_RC_ROLE_CHECK=0` á undan hverri `release` skipun.
tutor-l9-s1 = Búðu til raunverulega villu með goatnet sem er ekki til
tutor-l9-text =
    Þegar goatbuilder hafnar prentar það kóða. Kóðar sem byrja á `GB-` koma frá goatbuilder, `E_` frá goat CLI og `X-` frá öðru verkfæri sem goatbuilder keyrði.

    Skipunin goatbuilder `explain` tekur kóða, eða texta villulínu, og segir þér hvað gerðist, hvers vegna, hvernig á að laga það og hvar má lesa meira.

    Að biðja um goatnet sem þetta checkout skilgreinir ekki býr til raunverulega villu til að útskýra.
