### goatbuilder — it (it)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Costruisci e rilascia un ambiente goat: dichiara, converge, verifica — sotto un unico contratto di deploy
help-flag-output = Formato di output: tabella per una persona; JSON o YAML per un programma
help-flag-repo = La radice del repository. Predefinito: il checkout in cui gira questo binario
help-flag-yes = Conferma un verbo dichiarativo senza chiedere (per l'automazione)
help-flag-frozen = Rifiuta ogni esecuzione che richiederebbe il fast-forward del checkout (C3 rigoroso)
help-flag-mock = Esegui su una directory di fixture invece che sugli host: niente registry, niente trasporto, deterministico (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Lingua per help e spiegazioni degli errori (un tag come fr, uk, de); l'output letto da una pipeline resta in inglese. Predefinito: la lingua del sistema operativo
help-cmd-validate = Controlla una config rispetto allo schema versionato e alle regole tra campi, offline
help-cmd-doctor = Verifica ciò che serve a un'esecuzione prima di tagliare qualsiasi cosa: dipendenze, aggiornamento del checkout, trasporto di ogni ambiente e che il playbook di ogni componente si risolva in una modalità target definita
help-cmd-status = Dichiarato contro registrato, per macchina: converged, pending o unknown (C7, C8). Solo lettura. Exit 3 finché una macchina non è converged
help-cmd-deploy = Porta il pin dichiarato da main su un ambiente e dimostralo: risolve dai pin il componente e i gruppi dell'ambiente, il play e la sua modalità target, ogni segreto che il play applica a template, il trasporto; esegue il play tramite ansible-preflight; rilegge il ledger di deploy da ogni host e lo dà per concluso solo quando la riga più recente indica il digest dichiarato con action=deploy (C7). Su una fixture (`--mock`) il converger scriptato sostituisce gli host
help-cmd-release = Porta una release candidate da un tag su main, attraverso la validazione live della goatnet dev, fino al gradino preprod (ADR 1173); dopo il soak, con `--from prod`, la promuove, la integra in fleet:, fa il deploy in prod e riverifica ogni gradino. Sostituisce ops/scripts/rc.sh passo per passo
help-cmd-rollback = Riporta un componente distribuito al suo N-esimo ref più recente, per tipo di artefatto, leggendo il ledger sull'host come fonte di verità (ADR 1060). Sostituisce ops/scripts/rollback.sh: exit 3 rifiutato perché non sicuro (data, cert, forward-only, reversible:false), 4 guidato (config, acl)
help-cmd-fastlane = Un solo comando da un commit integrato a ogni goatnet (ADR 1127): l'eccezione deliberata alla scala. Sostituisce ops/scripts/fastlane.sh passo per passo; rifiuta un digest non pinnato o un set di graduation aperto, ed esegue i deploy tramite questo binario
help-cmd-standup = Un sito da zero a verificato: le fasi dello standup nell'ordine della dottrina con un gate fail-closed a ogni estremo, l'host di osservabilità come ramo parallelo, le classi di rescue e le superfici cross-goatnet per ultime (ADR 1183). Sostituisce ops/scripts/goat-standup.sh passo per passo. Su una fixture (`--mock`) ogni componente di ogni classe viene convergito dal converger scriptato e il tempo dichiarato → verificato è la misura della goatnet da un'ora
help-cmd-join-node = Una macchina in più di una classe su un sito live, dichiarata → convergita → verificata: la sequenza eseguita di docs/tutorials/02-first-federated-node.md come fasi numerate con i suoi gate (raggiungibilità, chiave, prima il lavoro sul repo, baseline, WireGuard del kernel prima e dopo l'enrolment, il piano ACL del kernel come gate di enrolment, il converge della classe, accettazione)
help-cmd-federate = Un peering sovrano dichiarato, convergito e verificato (ADR 1157): la nostra metà misurata, i pacchetti scambiati fuori banda, un unico accordo composto e verificato, le foglie coniate da esso, le regole rese dall'unico writer, l'edge distribuito, il trasporto scelto dall'accordo aperto, accettazione in tutti e quattro i livelli. Sostituisce ops/scripts/peering-standup.sh passo per passo
help-cmd-pins = UN passaggio di risoluzione con DUE rendering: il pins.yml di group_vars di ogni membro elencato e l'artefatto IntentV1 di ogni goatnet elencata, da ops/pins/fleet-pins.yaml. Sostituisce ops/pins/generate-pins.py byte per byte. `--check` valida, rigenera e rifiuta drift, un intent non firmato o manomesso, e un pin governato definito fuori da pins.yml (exit 1)
help-cmd-promote = Guida e osserva la graduation delle release lungo la scala delle goatnet (ADR 1120, 1125): status, apertura di una candidate, graduate, graduate di un set, il percorso hotfix, review, readiness, abandon, check. Sostituisce ops/release/graduate.py passo per passo. Nulla qui promuove da solo: una mutazione parte solo se invocata, stampa ciò che ha fatto e chiede --yes senza un terminale (C10, ADR 1177)
help-cmd-explain = Interpreta un errore: un codice (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) o un qualsiasi testo di errore — di questo binario o le parole di uno strumento che ha riportato (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Stampa cosa è successo, perché, cosa fare e dove leggere di più; `-` legge il testo da stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Un sito da verificato a niente: lo specchio di standup, i record restano (un burn-down aggiunge righe di teardown, non cancella mai nulla). Su una fixture (`--mock`) simula il burn-down in ordine inverso delle fasi; su host reali stampa l'ordine e rifiuta di eseguire finché ogni fase non dichiara il proprio play di teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lezioni in stile vimtutor: ognuna indica il comando esatto da eseguire su una copia usa e getta di una fixture (o, in sola lettura, su questo checkout), poi verifica cosa ha fatto davvero quel comando — exit e righe prodotte — prima di sbloccare la successiva. `tutor` mostra la lezione in corso e la segna come completata quando le tue esecuzioni la soddisfano; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Nascosto (src/fun.rs)
help-cmd-argan = Nascosto (src/fun.rs)
help-cmd-bound = Nascosto (crates/goat-fun)
help-cmd-secrets = La postura dei segreti dichiarata (ADR 1204): quale store è in vigore (1password, openbao, sops, git-crypt, env-file, env), cosa è disponibile, come fornire ciò che manca e i controlli propri dello store. `init --env <goatnet>` scrive il file env del quickstart zero-SaaS: un valore casuale robusto per ogni variabile che i play della goatnet usano nei template, 0600, in gitignore, senza mai ruotare un valore già presente. Stesso modulo di ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Legge per ogni host la riga più recente del deploy-ledger per un componente e dice se indica il ref dichiarato da main (C7). Solo lettura; exit 3 finché un host è pending o unknown. Lo stesso controllo eseguito da `deploy --verify-only`
help-cmd-diff = Cosa differisce da main adesso (C8): il ref dichiarato da main a confronto con la riga più recente del ledger di ogni host, per un componente o, senza, per tutti i componenti dell'ambiente, letti in un solo passaggio per gruppo di membri. Solo lettura; exit 3 in caso di drift o host illeggibile
help-cmd-history = Il deploy ledger come timeline, dal più recente, su ogni host dell'ambiente (C8): ogni riga è una voce con la macchina, l'ora, l'azione (deploy o rollback) e la versione. La versione precedente della voce e l'operatore che l'ha eseguita sono in `-o json`. Solo lettura
help-cmd-plan = Scrive in un file di piano, per la revisione, cosa farebbe un verbo dichiarativo: i ref dichiarati, il play e il target, il ledger, il commit di main, il transport (deploy); il tipo di artefatto e il ref di destinazione (rollback). Non tocca lo stato di nessun host. Eseguilo come rivisto con `apply`
help-cmd-apply = Esegue esattamente un piano rivisto. Rifiuta (exit 1, indicando cosa è cambiato) quando la dichiarazione di main, il commit dei pin, il play o il target del rollback sono cambiati dopo la scrittura del piano; un transport cambiato viene segnalato, non rifiutato
help-cmd-adopt = Dichiara ciò che un ambiente esegue già: legge i ledger di ogni host e propone una deviazione dei pin per ogni componente il cui ref in esecuzione differisce dalla dichiarazione di main. Per impostazione predefinita è solo un report; con --yes le modifiche vengono scritte in un worktree nuovo di main per la revisione. Non esegue mai deploy

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Non ancora: { $why }
tutor-lesson-done = Lezione { $n } completata: { $title }.
tutor-all-done-reset = Tutte le lezioni sono completate. `goatbuilder tutor --reset` riparte da capo; i tutorial vanno oltre: docs/tutorials/.
tutor-all-done = Tutte le lezioni sono completate. I tutorial vanno oltre: docs/tutorials/.
tutor-workspace = Workspace: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Verifica che una goatnet sia stata costruita correttamente, livello per livello: tutti i controlli di accettazione applicabili al sito, inclusi quelli che un operatore esegue con la CLI goat. La fase di accettazione dello standup come comando a sé. Solo lettura; exit 3 finché un controllo fallisce o non può rispondere
help-flag-offline = Sito isolato: non scaricare il trunk; valuta il checkout rispetto a quello in cache e registra l'esecuzione come offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Verifica che una goatnet sia nel suo stato finale dichiarato: ogni servizio che le sue dichiarazioni indicano come attivo è distribuito, in esecuzione, integro e operativo, con i bordi zero-trust collegati e nessun servizio non dichiarato in esecuzione. Sola lettura; exit 3 finché qualcosa non è a posto
help-cmd-survey = Quanto è coerente il software all'interno di ogni goatnet e tra le goatnet della ladder, letto in un unico passaggio parallelo. Legge solo
help-cmd-survey-long =
    Quanto è coerente il software all'interno di ogni goatnet e tra i goatnet della scala, letto in un unico passaggio parallelo. Solo lettura. All'interno di un goatnet: il ref dichiarato in main confrontato con la riga più recente del ledger di ogni host e con i container che esegue da quell'immagine (uno in esecuzione; uno fermo viene valutato solo su un warm standby dove nulla è in esecuzione), gli host di un goatnet che non concordano, il goat CLI di ogni host rispetto a quello servito dal suo goatnet, e ciò che goat-get serve rispetto a main. Tra i goatnet: una riga per componente, una colonna per goatnet, con il giudizio della scala per ogni differenza, come lo dà promote status. Uscita 3 per qualsiasi deriva o qualsiasi cosa non letta; una differenza dichiarata dalla scala viene segnalata, non trattata come fallimento.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Sostituisce questo binario con la release servita dalla sua goatnet su `get.<site>.<zone>/goatbuilder/`: il download viene verificato con il suo `.sha256` e con il `SHA256SUMS` firmato prima di toccare qualsiasi cosa, e la sostituzione è atomica. Già aggiornato: una riga, exit 0. `--check` è in sola lettura (exit 3 se indietro). La superficie: `GOATBUILDER_UPDATE_URL`, altrimenti `--site` dai suoi group_vars, altrimenti quello usato dall'ultimo aggiornamento
tutor-prereq = Prima di iniziare: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Colora il glifo ok/FAIL e il verdetto della tabella (auto|always|never). `auto` colora solo se è un terminale; NO_COLOR e CLICOLOR_FORCE hanno la precedenza. JSON e YAML non sono mai colorati

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-verbose = Scrive su stderr cosa fa un'esecuzione — i comandi esterni che lancia e la fase di rete di `update` — così `-o json|yaml` resta un documento pulito. Oppure imposta RUST_LOG=goatbuilder=debug
help-flag-timeout = Secondi di limite per la fase di connessione/raggiungibilità prima di fallire subito — l'unica fase che può bloccarsi su un host irraggiungibile (il fetch della versione servita da `update`). NON limita l'esecuzione di un play né il download del binario (limitato dallo stallo). Legge anche GOATBUILDER_TIMEOUT
help-cmd-completion = Stampa su stdout uno script di completamento per la shell (`goatbuilder completion zsh >> ~/.zshrc`). Per i candidati dinamici che uno script statico non può conoscere — i siti, i componenti e le goatnet dichiarati da questo checkout — collega un hook a `goatbuilder _complete` (vedi l'intestazione dello script generato)
help-cmd-docs = Il riferimento dei comandi: `goatbuilder docs` rimanda a esso; `--markdown` emette tutto, ricavato dall'albero dei comandi attivo, così non può mai divergere da `--help`
help-cmd-_complete = Nascosto: emette i candidati di completamento dinamici per gli hook della shell, uno per riga (i siti, i componenti e le goatnet dichiarati da questo checkout). Solo lettura, non fallisce mai rumorosamente — un hook di completamento non deve corrompere il prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Prima di un rollout, il resoconto di prontezza e reversibilità (ADR 1211): tre fatti — State (stateless | stateful), Rollback (pin-revert | restore-required | none) e Scope (single | coupled) — dichiarati per componente e FAIL-CLOSED se non dichiarati, con la metà di prontezza che gli altri verbi dimostrano (ref scaricabile, pins --check, il gate dei segreti, sorgente aggiornato, lo stato finale dichiarato) e una validazione del fatto di rollback (rollback --dry-run per pin-revert, la finestra di retention per restore-required). `deploy` lo esegue per primo; valutato con onestà, mai un falso verde. Solo lettura; exit 3 se non pulito

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Controlla la tua configurazione
tutor-l1-s1 = Controlla il file di configurazione
tutor-l2-title = Leggi lo stato: converged, pending, unknown
tutor-l2-text =
    Il comando goatbuilder `status` confronta due cose per ogni macchina: la versione che main dichiara per essa, e la versione che il suo ledger dice che sta eseguendo. Non cambia nulla.

    Ogni macchina è in uno di tre stati:
    - converged (convergita): il suo ledger indica la versione dichiarata. Questo è fatto.
    - pending (in attesa): il suo ledger indica una versione più vecchia, o ancora nessuna.
    - unknown (sconosciuta): goatbuilder non può leggere il suo ledger.

    Solo converged conta come fatto. Se una macchina è pending o unknown, `status` esce con codice 3.

    Da questa lezione in poi, `--mock` punta goatbuilder a una copia di prova nella tua cartella tutor invece che a macchine reali. Niente di quello che fai lì raggiunge una macchina reale.

    Nulla è ancora stato deployato nel `dev` della copia di prova. Quindi aspettati quattro macchine pending e una unknown. Vedrai macchine converged nella lezione 3.
tutor-l2-s1 = Leggi lo stato di dev
tutor-l3-title = Deploy: pianificalo, eseguilo, dimostralo
tutor-l3-text =
    Fare il deploy porta la versione dichiarata su ogni macchina, poi legge il ledger di ogni macchina per dimostrare che è arrivata. Un deploy è fatto solo quando il ledger di ogni macchina indica quella versione. Un deploy che è andato senza errori non basta.

    Farai il deploy di `portal` sul `preprod` della copia di prova. `--dry-run` mostra cosa succederebbe e non cambia nulla. `--ack` registra chi lo ha fatto; il comando inserisce il tuo nome da git. Dopo il deploy, `status` dovrebbe mostrare entrambe le macchine come `ok` e converged.
tutor-l3-s1 = Pianifica il deploy
tutor-l3-s2 = Fai il deploy di portal su preprod
tutor-l3-s3 = Controlla che entrambe le macchine siano converged
tutor-l4-title = Drift e history
tutor-l4-text =
    Due comandi ti aiutano a capire un goatnet prima e dopo una modifica. Entrambi leggono soltanto.
    - Il comando goatbuilder `diff` elenca ogni macchina il cui ledger differisce dalla versione dichiarata. Una differenza si chiama drift.
    - Il comando goatbuilder `history` elenca ogni versione che un componente ha avuto su ogni macchina, dalla più recente.

    Su `dev`, ogni macchina è indietro. La history di `portal` su `preprod` ora include il tuo deploy della lezione 3.
tutor-l4-s1 = Elenca il drift su dev
tutor-l4-s2 = Leggi la history di portal su preprod
tutor-l5-title = Fai il rollback, e osserva un rollback non sicuro essere rifiutato
tutor-l5-text =
    Un rollback dichiara una versione precedente e ne fa il deploy come qualsiasi altra versione. La CLI di goatbuilder poi legge il ledger di ogni macchina per dimostrare che è arrivata.

    Alcune modifiche non si possono annullare in questo modo. Fare il deploy di una versione più vecchia non può annullare i dati che un componente ha scritto, un certificato, o una modifica che va solo avanti. La CLI di goatbuilder rifiuta questi casi con codice di uscita 3 e dice cosa fare invece.

    Il componente `db-schema` della copia di prova contiene dati, quindi goatbuilder rifiuta di farne il rollback. Questo rifiuto è il risultato previsto.
tutor-l5-s1 = Pianifica il rollback
tutor-l5-s2 = Fai il rollback di portal di una versione
tutor-l5-s3 = Prova a fare il rollback di db-schema, che goatbuilder rifiuta
tutor-l6-title = Rilascia una nuova versione: dev, poi preprod, poi prod
tutor-l6-s1 = Guarda dove si trova ogni componente
tutor-l6-s2 = Simula la release da dev
tutor-l6-s3 = Simula il passo prod
tutor-l7-title = Fastlane, e quando non usarlo
tutor-l7-text =
    Fastlane porta una versione su ogni goatnet in una sola volta. Salta l'attesa su preprod e i ledger della release.

    Usalo solo per una correzione urgente che puoi annullare tornando alla versione precedente. Non usarlo mai per una modifica al database, una modifica di identità o di accesso, o qualsiasi cosa che non puoi annullare in questo modo.

    Fastlane rifiuta una versione che CI non ha costruito, e un componente che fa parte di una release ancora in corso.

    La forma generale è `goatbuilder fastlane <component> --ref <version> --reason <why>`. Esegui l'esempio sotto. Ha `--dry-run`, quindi niente cambia. Richiede `gh`.
tutor-l7-s1 = Simula un fastlane
tutor-l8-title = Pins: trova il drift e correggilo nel modo giusto
tutor-l8-text =
    Un file stabilisce quale versione esegue ogni goatnet: `ops/pins/fleet-pins.yaml`. La CLI di goatbuilder genera gli altri file di versione da questo: i file `pins.yml` e i file di intent firmati.

    Il comando goatbuilder `pins --check` confronta i file generati con `fleet-pins.yaml` e rifiuta qualsiasi file che differisce. Il comando goatbuilder `pins` li rigenera.

    La tua cartella tutor ha una copia del repository in cui qualcuno ha modificato a mano un file `pins.yml`. Non correggere mai una modifica a mano con un'altra modifica a mano. Cambia `fleet-pins.yaml`, poi rigenera.
tutor-l8-s1 = Controlla questo checkout
tutor-l8-s2 = Controlla la copia e trova la modifica a mano
tutor-l8-s3 = Rigenera i file della copia
tutor-l8-s4 = Controlla di nuovo la copia
tutor-l9-title = Spiega un errore
tutor-l9-s2 = Spiega l'errore con il suo codice
tutor-l9-s3 = Spiega l'errore con il suo testo
tutor-l10-title = Tutorial 1: costruisci il primo site di un nuovo goatnet
tutor-l10-text =
    Questa lezione costruisce un intero goatnet in una copia di prova. Il goatnet di esempio è `zagros`. Il suo primo site, `alpha`, ha due macchine core, una macchina di monitoraggio e una macchina di rescue.

    Il comando goatbuilder `standup` configura ogni macchina in un ordine fisso e controlla ogni ledger. La sua ultima riga dice che il site è fatto. Su macchine reali, quella riga mostra anche quanto tempo ci è voluto.

    Il tutorial completo è `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Costruisci il primo site, alpha
tutor-l10-s2 = Controlla il risultato
tutor-l11-title = Tutorial 2: aggiungi un secondo site
tutor-l11-text =
    Questa lezione inizia da dove è finito il tutorial 1. Aggiunge un secondo site, `beta`, in una nuova copia di prova, `zagros-2`.

    Un secondo site copia i suoi account utente dal primo site. Il suo standup modifica anche il primo site una volta, così la copia funziona in entrambe le direzioni. Un passo chiamato `alpha/core` fa questo.

    Il tutorial completo è `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Costruisci il secondo site, beta
tutor-l11-s2 = Controlla entrambi i site
tutor-l12-s1 = Pianifica l'aggiunta di una macchina
tutor-l13-s1 = Pianifica una federazione
tutor-l14-title = Mantieni goatbuilder aggiornato
tutor-l14-s1 = Chiedi se esiste un goatbuilder più recente
tutor-l15-title = Assay: controlla come è stato costruito un goatnet
tutor-l15-text =
    Il comando goatbuilder `assay` controlla che un goatnet sia stato costruito correttamente, uno strato alla volta, usando i controlli di accettazione. Ogni controllo ottiene uno di tre risultati:
    - PASS: il controllo è passato.
    - FAIL: il goatnet ha un problema.
    - UNKNOWN: niente ha potuto rispondere al controllo.

    Solo PASS conta. Qualsiasi FAIL o UNKNOWN rende il codice di uscita 3.

    `--plan` elenca i controlli che si applicano a un goatnet, per strato, e non tocca nessuna macchina. Usalo per vedere cosa coprirebbe un assay reale. La forma generale è `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: le versioni sono consistenti?
tutor-l16-text =
    Il comando goatbuilder `survey` controlla in un solo passaggio se le stesse versioni sono in esecuzione dove dovrebbero:
    - Dentro ogni goatnet: ogni macchina esegue la versione dichiarata, e le macchine che dovrebbero corrispondere corrispondono davvero?
    - Tra dev, preprod e prod: una riga per componente, una colonna per goatnet.

    Una differenza che il processo di release prevede è segnalata ma non è un errore. Un esempio è una nuova versione che è su dev ma non ancora su prod. Una differenza che niente dichiara si legge come `difference not declared`. Una macchina che goatbuilder non può leggere è unknown, mai consistente.

    Eseguilo sulla copia di prova. Il suo `dev` è indietro apposta, quindi aspettati risultati e codice di uscita 3.
tutor-l16-s1 = Esegui survey sulla copia di prova
tutor-l17-title = Endstate: un goatnet esegue esattamente quello che dichiara?
tutor-l17-text =
    Il comando goatbuilder `endstate` controlla due cose di un goatnet:
    1. Ogni servizio che dichiara è deployato, in esecuzione, sano e correttamente connesso.
    2. Non è in esecuzione nulla che non dichiara. Un servizio non dichiarato si chiama orfano.

    La CLI di goatbuilder calcola cosa dovrebbe eseguire un site dalle impostazioni proprie del site, non da una lista separata.

    Un controllo che non ha nulla da leggere è marcato UNOBSERVED. Questa è una risposta onesta, non un fallimento. Un FAIL o un orfano rende il codice di uscita 3.

    La forma generale è `goatbuilder endstate <goatnet>`. Il comando sotto controlla un site di esempio da dati salvati, quindi non tocca nessuna macchina.
tutor-l17-s1 = Controlla il site di esempio
tutor-l18-title = Bonus: sii curioso
tutor-l18-text =
    Questa lezione è opzionale. Esegui il comando sotto e il tutor è finito.

    La CLI di goatbuilder ha alcune piccole funzioni nascoste. Non sono nell'help né in queste lezioni. Alcune rispondono a parole che gli utenti Unix di lunga data spesso digitano in un nuovo programma, solo per vedere cosa succede. La maggior parte delle cose che provi non farà nulla.

    Questa lezione non dice quali sono. Trovarle è lo scopo.

    Il comando sotto elenca ogni errore che goatbuilder può spiegare. È un buon posto da cui iniziare a esplorare.
tutor-l18-s1 = Elenca ogni errore che goatbuilder può spiegare
tutor-heading = Lezione { $n } su { $total } · { $title }
tutor-setup = Prima di tutto, esegui questo una volta in questo terminale:
tutor-not-recording = Questo terminale non sta registrando le tue esecuzioni, quindi il tutor non può segnare i tuoi passi. Esegui questo una volta, poi esegui i passi:
tutor-type = Esegui questi passi nel tuo terminale ({ $met } di { $total } fatti):
tutor-rerun = Dopo ogni passo, esegui di nuovo `goatbuilder tutor`. Controlla cosa hai fatto e segna il passo.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Benvenuto in GOATBUILDER Tutor
tutor-bar-progress = Avanzamento
tutor-bar-done = { $done } su { $total } lezioni fatte
tutor-after-step = Passo { $met } su { $total } fatto. Successivo:
tutor-after-lesson = La lezione { $n } è completa. Esegui `goatbuilder tutor` per iniziare la lezione { $next }. Per leggerla di nuovo: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = La lezione { $n } è completa, e lo è anche il tutor. Esegui `goatbuilder tutor` per vederlo segnato come fatto. Per leggerla di nuovo: `goatbuilder tutor --lesson { $n }`.

tutor-review = Hai finito questa lezione, quindi questa pagina è solo da leggere. Esegui `goatbuilder tutor` per tornare al punto in cui eri.
tutor-other-lessons = Per leggere di nuovo una lezione: `goatbuilder tutor --lesson 1`. Per vederle tutte: `goatbuilder tutor --list`.
tutor-l12-title = Aggiungi una macchina a un site in esecuzione
tutor-l12-text =
    Il comando goatbuilder `join-node` aggiunge un'altra macchina a un site già in esecuzione. Prima stampa un piano, con una riga per ogni fase. Ogni fase ha un tipo:
    - tool: goatbuilder esegue uno script.
    - ansible: goatbuilder esegue un playbook.
    - gate: un controllo che deve passare prima della fase successiva.
    - manual: lo fai tu, e l'esecuzione si ferma.

    Il comando qui sotto ha `--dry-run`. Legge questo checkout e non cambia nulla. Sotto la tabella, leggi cosa fai a mano.
tutor-l13-title = Connettiti al goatnet di un'altra organizzazione
tutor-l13-text =
    Il comando goatbuilder `federate` collega il tuo goatnet a un goatnet gestito da un'altra organizzazione. Ogni lato costruisce la sua metà di un accordo. Scambi le metà, e ogni lato le combina nello stesso accordo.

    Conferma la chiave dell'altro lato in un modo separato, per esempio una chiamata di cui ti fidi. Una chiave che arriva dentro il loro pacchetto non prova nulla.

    Il comando qui sotto ha `--dry-run`, quindi non cambia nulla. Un'esecuzione reale si ferma alla fase 4, perché quella fase la fai tu a mano.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Questo comando non fa parte della lezione { $n }. Esegui `goatbuilder tutor` per vedere dove sei.
tutor-l1-s2 = Chiedi al doctor di un goatnet
tutor-l1-text =
    La CLI di goatbuilder compila, fa il deploy e rilascia il software GOAT. Prima di usarla, controlla la tua configurazione con due comandi:
    - Il comando goatbuilder `validate` controlla il file di configurazione, `ops/goatbuilder/goatbuilder.yaml`. Legge solo questo checkout e non richiede la rete.
    - Il comando goatbuilder `doctor` controlla ciò che serve a un deploy reale: i tuoi strumenti, i tuoi accessi, se questo checkout corrisponde a main e come goatbuilder raggiunge ogni goatnet. Un goatnet è un ambiente GOAT.

    Il comando goatbuilder `doctor` stampa quanti controlli sono passati, poi ogni problema con la sua correzione. In questa lezione un problema è un'informazione, non un errore. Leggi le correzioni, ma non devi applicarle ora.

    La maggior parte delle lezioni non richiede l'accesso. Le lezioni da 2 a 5, 10, 11 e 16 usano una copia di prova. Le lezioni 8, 9, 12, 13, 15, 17 e 18 leggono solo questo checkout. La lezione 6 richiede `gh` e la mesh, la lezione 7 richiede `gh` e la lezione 14 richiede la mesh. Nessuna lezione richiede 1Password.
tutor-l14-text =
    La CLI di goatbuilder può aggiornarsi da sola dalla copia che il tuo goatnet serve su `get.<site>.<zone>/goatbuilder/`.

    Il comando goatbuilder `update --check` legge soltanto. Confronta la tua versione con quella servita e non cambia nulla. Il codice di uscita 0 significa che sei aggiornato. Il codice di uscita 3 significa che esiste una versione più recente.

    Un aggiornamento reale, senza `--check`, verifica il checksum e la firma del download prima di sostituire qualcosa. Poi sostituisce il file in un solo passaggio.

    La CLI di goatbuilder deve sapere quale goatnet interrogare: passa `--site` con quel goatnet, oppure imposta `GOATBUILDER_UPDATE_URL`. Devi essere sulla mesh di quel goatnet. Se non lo sei, `--check` dice che non riesce a raggiungere il server e indica l'indirizzo che ha provato. Per questa lezione è un risultato corretto.
tutor-l15-s1 = Elenca i test che si applicano a un goatnet
tutor-l6-text =
    Un rilascio porta una nuova versione attraverso tre goatnet in ordine: dev, poi preprod, poi prod.

    1. Qualcuno applica un tag alla versione su main e la CI la compila.
    2. dev: goatbuilder ne fa il deploy ed esegue i test di accettazione.
    3. preprod: goatbuilder ne fa di nuovo il deploy, esegue di nuovo i test e registra i risultati.
    4. La versione poi attende su preprod per un tempo stabilito. Niente passa a prod da solo.
    5. prod: dopo quell'attesa, una persona esegue il rilascio con `--from prod`. La CLI di goatbuilder fa il deploy su prod e controlla tutti e tre i goatnet.

    Il comando goatbuilder `promote status` mostra dove si trova ogni componente. Un candidato è una versione in fase di test. La forma generale di un rilascio è `goatbuilder release <component> --tag <tag> --from dev`. I comandi qui sotto usano un componente di questo checkout e un tag di esempio. Ognuno ha `--dry-run`, quindi niente viene taggato né distribuito.

    Le prove a secco richiedono `gh` e la mesh. Se non sei un admin del repository, metti `GOAT_RC_ROLE_CHECK=0` prima di ogni comando `release`.
tutor-l9-s1 = Provoca un errore reale con un goatnet che non esiste
tutor-l9-text =
    Quando goatbuilder rifiuta, stampa un codice. I codici che iniziano con `GB-` vengono da goatbuilder, quelli con `E_` dalla CLI goat e quelli con `X-` da un altro strumento che goatbuilder ha eseguito.

    Il comando goatbuilder `explain` accetta un codice, o il testo di una riga di errore, e ti dice cosa è successo, perché, come correggerlo e dove leggere di più.

    Chiedere un goatnet che questo checkout non dichiara produce un errore reale da spiegare.
