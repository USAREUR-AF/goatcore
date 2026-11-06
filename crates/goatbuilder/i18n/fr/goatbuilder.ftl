### goatbuilder — fr (fr)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Construire et livrer un environnement goat : déclarer, converger, vérifier — sous un seul contrat de déploiement
help-flag-output = Format de sortie : tableau pour un humain ; JSON ou YAML pour un programme
help-flag-repo = Racine du dépôt. Par défaut : le checkout depuis lequel ce binaire est lancé
help-flag-yes = Confirmer un verbe déclaratif sans invite (pour l'automatisation)
help-flag-frozen = Refuser toute exécution qui nécessiterait d'avancer le checkout en fast-forward (C3 strict)
help-flag-mock = Exécuter sur un répertoire de fixtures au lieu des hôtes : pas de registre, pas de transport, déterministe (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Langue de l'aide et des explications d'erreur (un tag comme fr, uk, de) ; la sortie lue par un pipeline reste en anglais. Défaut : la locale du système
help-cmd-validate = Vérifier une config contre le schéma versionné et les règles inter-champs, hors ligne
help-cmd-doctor = Sonder ce dont une exécution a besoin avant toute coupure : dépendances, fraîcheur du checkout, transport de chaque environnement, et que le playbook de chaque composant se résout en un mode cible défini
help-cmd-status = Déclaré contre enregistré, par machine : convergée, en attente ou inconnue (C7, C8). Lecture seule. Code de sortie 3 tant qu'une machine n'est pas convergée
help-cmd-deploy = Poser sur un environnement le pin que main déclare et le prouver : résoudre le composant et les groupes de l'environnement depuis les pins, le play et son mode cible, chaque secret que le play templatise, le transport ; exécuter le play via ansible-preflight ; relire le ledger de déploiement sur chaque hôte et ne conclure que si la ligne la plus récente nomme le digest déclaré avec action=deploy (C7). Sur une fixture (`--mock`), le converger scripté remplace les hôtes
help-cmd-release = Mener un release candidate d'un tag sur main, via la validation en direct du goatnet dev, jusqu'au palier preprod (ADR 1173) ; après le soak, avec `--from prod`, le graduer, l'intégrer à fleet:, déployer prod et revérifier chaque palier. Remplace ops/scripts/rc.sh étape par étape
help-cmd-rollback = Ramener un composant déployé à son N-ième ref le plus récent, selon le type d'artefact, en prenant le ledger sur l'hôte comme source de vérité (ADR 1060). Remplace ops/scripts/rollback.sh : code 3 refusé car non sûr (data, cert, forward-only, reversible:false), 4 guidé (config, acl)
help-cmd-fastlane = Une seule commande, du commit fusionné à chaque goatnet (ADR 1127) : l'exception délibérée à l'échelle. Remplace ops/scripts/fastlane.sh étape par étape ; refuse un digest non pinné ou un ensemble de graduation ouvert, et lance les déploiements via ce binaire
help-cmd-standup = Un site de zéro à vérifié : les phases du standup dans l'ordre de la doctrine avec une porte fail-closed à chaque extrémité, l'hôte d'observabilité en branche parallèle, les classes de secours et les surfaces inter-goatnet en dernier (ADR 1183). Remplace ops/scripts/goat-standup.sh étape par étape. Sur une fixture (`--mock`), chaque composant de chaque classe est convergé par le converger scripté et le temps déclaré → vérifié est la mesure du goatnet d'une heure
help-cmd-join-node = Une machine de plus d'une classe sur un site en production, déclarée → convergée → vérifiée : la séquence exécutée de docs/tutorials/02-first-federated-node.md en phases numérotées avec ses portes (accès, clé, travail de dépôt d'abord, baseline, WireGuard noyau avant et après enrôlement, plan ACL noyau comme porte d'enrôlement, converge de la classe, acceptation)
help-cmd-federate = Un peering souverain déclaré, convergé et vérifié (ADR 1157) : notre moitié mesurée, les paquets échangés hors bande, un accord composé et vérifié, les leaves émises depuis lui, les règles rendues par l'unique rédacteur, l'edge déployé, le transport choisi par l'accord ouvert, acceptation sur les quatre niveaux. Remplace ops/scripts/peering-standup.sh étape par étape
help-cmd-pins = UNE passe de résolution, DEUX rendus : le pins.yml de group_vars de chaque membre listé et l'artefact IntentV1 de chaque goatnet listé, depuis ops/pins/fleet-pins.yaml. Remplace ops/pins/generate-pins.py octet pour octet. `--check` valide, régénère et refuse la dérive, une intention non signée ou altérée, et un pin gouverné défini hors de pins.yml (code 1)
help-cmd-promote = Guider et observer la graduation des releases sur l'échelle des goatnets (ADR 1120, 1125) : status, ouvrir un candidat, graduer, graduer un ensemble, le chemin hotfix, review, readiness, abandon, check. Remplace ops/release/graduate.py étape par étape. Rien ici ne gradue de lui-même : une mutation ne s'exécute que sur invocation, affiche ce qu'elle a fait, et exige --yes sans terminal (C10, ADR 1177)
help-cmd-explain = Interpréter une erreur : un code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), ou n'importe quel texte d'erreur — celui de ce binaire, ou les mots d'un outil qu'il a relayés (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Affiche ce qui s'est passé, pourquoi, que faire et où lire la suite ; `-` lit le texte sur stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Un site de « vérifié » à rien — le miroir de standup, historique conservé (un burn-down ajoute des lignes de teardown, n'efface jamais). Sur une fixture (`--mock`), il répète le burn-down dans l'ordre inverse des phases ; sur de vrais hôtes, il affiche l'ordre et refuse d'exécuter tant que chaque phase n'a pas déclaré son play de teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Leçons, façon vimtutor : chacune indique la commande exacte à lancer sur une copie jetable d'une fixture (ou, en lecture seule, sur ce checkout), puis vérifie ce que cette commande a réellement fait — sa sortie et ses lignes — avant de déverrouiller la suivante. `tutor` affiche la leçon en cours et la marque faite quand vos exécutions la satisfont ; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Masqué (src/fun.rs)
help-cmd-argan = Masqué (src/fun.rs)
help-cmd-bound = Masqué (crates/goat-fun)
help-cmd-secrets = La posture de secrets déclarée (ADR 1204) : quel store est en vigueur (1password, openbao, sops, git-crypt, env-file, env), ce qui est disponible, comment fournir ce qui manque, et les vérifications propres au store. `init --env <goatnet>` écrit le fichier env du quickstart zéro SaaS : une valeur aléatoire forte pour chaque variable que les plays du goatnet templatent, en 0600, ignoré par git, sans jamais renouveler une valeur déjà présente. Même module que ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Lit la ligne la plus récente du registre de déploiement de chaque hôte pour un composant et indique si elle désigne la ref déclarée par main (C7). Lecture seule ; code de sortie 3 tant qu'un hôte est en attente ou inconnu. Même contrôle que `deploy --verify-only`
help-cmd-diff = Ce qui diffère de main à l'instant (C8) : la ref déclarée par main face à la ligne la plus récente du registre de chaque hôte, pour un composant ou, sans composant, pour tous ceux de l'environnement, lue en une passe par groupe de membres. Lecture seule ; code de sortie 3 en cas de dérive ou d'hôte illisible
help-cmd-history = Le registre de déploiement en chronologie, du plus récent au plus ancien, sur chaque hôte de l'environnement (C8) : chaque ligne est une entrée avec la machine, l'heure, l'action (deploy ou rollback) et la version. La version précédente de l'entrée et l'opérateur qui l'a exécutée sont dans `-o json`. Lecture seule
help-cmd-plan = Écrit dans un fichier de plan, pour relecture, ce que ferait un verbe déclaratif : les refs déclarées, le play et la cible, le registre, le commit de main, le transport (deploy) ; le type d'artefact et la ref cible (rollback). Ne touche à l'état d'aucun hôte. À exécuter tel que relu avec `apply`
help-cmd-apply = Exécute exactement un plan relu. Refuse (code de sortie 1, en nommant ce qui a bougé) si la déclaration de main, le commit des pins, le play ou la cible de rollback ont changé depuis l'écriture du plan ; un transport modifié est signalé, pas refusé
help-cmd-adopt = Déclare ce qu'un environnement exécute déjà : lit les registres de chaque hôte et propose une dérogation de pins pour chaque composant dont la ref en cours diffère de la déclaration de main. Simple rapport par défaut ; avec --yes, les modifications sont écrites dans un worktree neuf de main pour relecture. Ne déploie jamais

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Pas encore : { $why }
tutor-lesson-done = Leçon { $n } terminée : { $title }.
tutor-all-done-reset = Toutes les leçons sont terminées. `goatbuilder tutor --reset` recommence depuis le début ; les tutoriels vont plus loin : docs/tutorials/.
tutor-all-done = Toutes les leçons sont terminées. Les tutoriels vont plus loin : docs/tutorials/.
tutor-workspace = Espace de travail : { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Vérifie qu'un goatnet a été construit correctement, couche par couche : tous les contrôles d'acceptation applicables au site, y compris ceux qu'un opérateur lance avec la CLI goat. La phase d'acceptation du standup, en commande autonome. Lecture seule ; code de sortie 3 tant qu'un contrôle échoue ou ne peut pas répondre
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Site isolé : ne pas récupérer le trunk ; évaluer le checkout par rapport à celui en cache, et enregistrer l'exécution comme hors ligne (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Vérifie qu'un goatnet est dans son état final déclaré : chaque service que ses propres déclarations disent exécuter est déployé, en cours d'exécution, sain et remplit sa fonction, avec ses arêtes zero-trust câblées, et rien ne tourne sans être déclaré. Lecture seule ; code de sortie 3 tant que quelque chose cloche
help-cmd-survey = Cohérence du logiciel au sein de chaque goatnet et entre les goatnets de l'échelle, relevée en un seul passage parallèle. Lit uniquement

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Cohérence des logiciels au sein de chaque goatnet et entre les goatnets de l'échelle, relevée en un seul passage parallèle. Lecture seule.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Remplace ce binaire par la version publiée par son goatnet à `get.<site>.<zone>/goatbuilder/` : le téléchargement est vérifié avec son `.sha256` et le `SHA256SUMS` signé avant toute modification, et le remplacement est atomique. Déjà à jour : une ligne, code de sortie 0. `--check` est en lecture seule (code de sortie 3 si en retard). La source : `GOATBUILDER_UPDATE_URL`, sinon `--site` depuis ses group_vars, sinon celle utilisée lors de la dernière mise à jour
tutor-prereq = Avant de commencer : { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Colorer le glyphe ok/FAIL et le verdict du tableau (auto|always|never). `auto` ne colore que dans un terminal ; NO_COLOR et CLICOLOR_FORCE sont prioritaires. JSON et YAML ne sont jamais colorés
help-flag-verbose = Journaliser ce que fait une exécution — les commandes externes lancées et la phase réseau de `update` — sur stderr, pour que `-o json|yaml` reste un document propre. Ou définir RUST_LOG=goatbuilder=debug
help-flag-timeout = Nombre de secondes pour borner la phase de connexion/joignabilité avant d'échouer vite — la seule phase pouvant bloquer sur un hôte injoignable (la récupération par `update` de la version servie). Ne plafonne PAS l'exécution d'un play ni le téléchargement du binaire (borné par blocage). Lit aussi GOATBUILDER_TIMEOUT
help-cmd-completion = Afficher sur stdout un script de complétion shell (`goatbuilder completion zsh >> ~/.zshrc`). Pour des candidats dynamiques qu'un script statique ne peut pas connaître — les sites, composants et goatnets déclarés par ce checkout — brancher un hook sur `goatbuilder _complete` (voir l'en-tête du script généré)
help-cmd-docs = La référence des commandes : `goatbuilder docs` y renvoie ; `--markdown` émet le tout, généré depuis l'arbre de commandes réel, donc jamais en décalage avec `--help`
help-cmd-_complete = Masquée : émet les candidats de complétion dynamiques pour les hooks shell, un par ligne (les sites, composants et goatnets déclarés par ce checkout). Lecture seule, n'échoue jamais bruyamment — un hook de complétion ne doit pas corrompre le prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Avant un déploiement, le relevé de préparation et de réversibilité (ADR 1211) : trois faits — State (stateless | stateful), Rollback (pin-revert | restore-required | none) et Scope (single | coupled) — déclarés par composant et FAIL-CLOSED s'ils ne le sont pas, avec la part préparation que prouvent les autres verbes (ref récupérable, pins --check, la porte des secrets, source à jour, l'état final déclaré) et une validation du fait de rollback (rollback --dry-run pour pin-revert, la fenêtre de rétention pour restore-required). `deploy` l'exécute en premier ; évalué selon l'honnêteté, jamais un faux vert. Lecture seule ; exit 3 si non propre

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Vérifiez votre installation
tutor-l1-s1 = Vérifiez le fichier de configuration
tutor-l2-title = Lisez l'état : converged, pending, unknown
tutor-l2-text =
    La commande goatbuilder `status` compare deux choses pour chaque machine : la version que main déclare pour elle, et la version que son propre ledger dit qu'elle exécute. Cela ne change rien.

    Chaque machine est dans l'un de trois états :
    - converged (convergée) : son ledger nomme la version déclarée. C'est fait.
    - pending (en attente) : son ledger nomme une version plus ancienne, ou aucune encore.
    - unknown (inconnue) : goatbuilder ne peut pas lire son ledger.

    Seule converged compte comme faite. Si une machine est pending ou unknown, `status` se termine avec le code 3.

    À partir de cette leçon, `--mock` fait pointer goatbuilder vers une copie d'entraînement dans votre dossier tutor, au lieu de vraies machines. Rien de ce que vous y faites n'atteint une vraie machine.

    Rien n'a encore été déployé sur le `dev` de la copie d'entraînement. Attendez-vous donc à quatre machines pending et une unknown. Vous verrez des machines converged à la leçon 3.
tutor-l2-s1 = Lisez l'état de dev
tutor-l3-title = Déployez : planifiez-le, exécutez-le, prouvez-le
tutor-l3-text =
    Déployer met la version déclarée sur chaque machine, puis lit le ledger de chaque machine pour prouver qu'elle est arrivée. Un déploiement n'est fait que quand le ledger de chaque machine nomme cette version. Un déploiement qui s'est exécuté sans erreur ne suffit pas.

    Vous allez déployer `portal` sur le `preprod` de la copie d'entraînement. `--dry-run` montre ce qui se passerait et ne change rien. `--ack` enregistre qui l'a fait ; la commande remplit votre nom à partir de git. Après le déploiement, `status` doit montrer les deux machines comme `ok` et converged.
tutor-l3-s1 = Planifiez le déploiement
tutor-l3-s2 = Déployez portal sur preprod
tutor-l3-s3 = Vérifiez que les deux machines sont converged
tutor-l4-title = Drift et historique
tutor-l4-text =
    Deux commandes vous aident à comprendre un goatnet avant et après un changement. Les deux ne font que lire.
    - La commande goatbuilder `diff` liste chaque machine dont le ledger diffère de la version déclarée. Une telle différence s'appelle le drift.
    - La commande goatbuilder `history` liste chaque version qu'un composant a eue sur chaque machine, la plus récente d'abord.

    Sur `dev`, chaque machine est en retard. L'historique de `portal` sur `preprod` contient maintenant votre déploiement de la leçon 3.
tutor-l4-s1 = Listez le drift sur dev
tutor-l4-s2 = Lisez l'historique de portal sur preprod
tutor-l5-title = Ramenez une version en arrière, et voyez un rollback non sûr refusé
tutor-l5-text =
    Un rollback déclare une version plus ancienne et la déploie comme n'importe quelle autre version. La CLI de goatbuilder lit ensuite le ledger de chaque machine pour prouver qu'elle est arrivée.

    Certains changements ne peuvent pas être annulés ainsi. Déployer une version plus ancienne ne peut pas annuler des données qu'un composant a écrites, un certificat, ou un changement qui ne va que vers l'avant. La CLI de goatbuilder les refuse avec le code de sortie 3 et dit quoi faire à la place.

    Le composant `db-schema` de la copie d'entraînement contient des données, donc goatbuilder refuse de le ramener en arrière. Ce refus est le résultat attendu.
tutor-l5-s1 = Planifiez le rollback
tutor-l5-s2 = Ramenez portal d'une version en arrière
tutor-l5-s3 = Essayez de ramener db-schema en arrière, ce que goatbuilder refuse
tutor-l6-title = Release d'une nouvelle version : dev, puis preprod, puis prod
tutor-l6-s1 = Voyez où se trouve chaque composant
tutor-l6-s2 = Simulez le release depuis dev
tutor-l6-s3 = Simulez l'étape prod
tutor-l7-title = Fastlane, et quand ne pas l'utiliser
tutor-l7-text =
    Fastlane met une version sur chaque goatnet en une fois. Il saute l'attente sur preprod et les ledgers de release.

    Utilisez-le seulement pour un correctif urgent que vous pouvez annuler en revenant à la version précédente. Ne l'utilisez jamais pour un changement de base de données, un changement d'identité ou d'accès, ou tout ce que vous ne pouvez pas annuler ainsi.

    Fastlane refuse une version que CI n'a pas construite, et un composant qui fait partie d'un release encore en cours.

    La forme générale est `goatbuilder fastlane <component> --ref <version> --reason <why>`. Exécutez l'exemple ci-dessous. Il a `--dry-run`, donc rien ne change. Il a besoin de `gh`.
tutor-l7-s1 = Simulez un fastlane
tutor-l8-title = Pins : trouvez le drift et corrigez-le de la bonne façon
tutor-l8-text =
    Un fichier fixe la version que chaque goatnet exécute : `ops/pins/fleet-pins.yaml`. La CLI de goatbuilder en génère les autres fichiers de version : les fichiers `pins.yml` et les fichiers d'intent signés.

    La commande goatbuilder `pins --check` compare les fichiers générés avec `fleet-pins.yaml` et refuse tout fichier qui diffère. La commande goatbuilder `pins` les régénère.

    Votre dossier tutor a une copie du dépôt dans laquelle quelqu'un a modifié un fichier `pins.yml` à la main. Ne corrigez jamais une modification à la main par une autre modification à la main. Changez `fleet-pins.yaml`, puis régénérez.
tutor-l8-s1 = Vérifiez ce checkout
tutor-l8-s2 = Vérifiez la copie et trouvez la modification à la main
tutor-l8-s3 = Régénérez les fichiers de la copie
tutor-l8-s4 = Vérifiez la copie à nouveau
tutor-l9-title = Expliquez une erreur
tutor-l9-s2 = Expliquez l'erreur par son code
tutor-l9-s3 = Expliquez l'erreur par son texte
tutor-l10-title = Tutoriel 1 : construisez le premier site d'un nouveau goatnet
tutor-l10-text =
    Cette leçon construit un goatnet entier dans une copie d'entraînement. Le goatnet d'exemple est `zagros`. Son premier site, `alpha`, a deux machines principales, une machine de monitoring et une machine de secours.

    La commande goatbuilder `standup` met en place chaque machine dans un ordre fixe et vérifie chaque ledger. Sa dernière ligne dit que le site est fait. Sur de vraies machines, cette ligne montre aussi combien de temps cela a pris.

    Le tutoriel complet est `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Construisez le premier site, alpha
tutor-l10-s2 = Vérifiez le résultat
tutor-l11-title = Tutoriel 2 : ajoutez un second site
tutor-l11-text =
    Cette leçon commence là où le tutoriel 1 s'est arrêté. Elle ajoute un second site, `beta`, dans une nouvelle copie d'entraînement, `zagros-2`.

    Un second site copie ses comptes utilisateurs depuis le premier site. Son standup change aussi une fois le premier site, pour que la copie fonctionne dans les deux sens. Une étape nommée `alpha/core` fait cela.

    Le tutoriel complet est `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Construisez le second site, beta
tutor-l11-s2 = Vérifiez les deux sites
tutor-l12-s1 = Planifiez l'ajout d'une machine
tutor-l13-s1 = Planifiez une fédération
tutor-l14-title = Maintenez goatbuilder à jour
tutor-l14-s1 = Demandez si une version plus récente de goatbuilder existe
tutor-l15-title = Assay : vérifiez comment un goatnet a été construit
tutor-l15-text =
    La commande goatbuilder `assay` vérifie qu'un goatnet a été construit correctement, une couche à la fois, en utilisant les contrôles d'acceptation. Chaque test reçoit un de trois résultats :
    - PASS : le test a réussi.
    - FAIL : le goatnet a un problème.
    - UNKNOWN : rien n'a pu répondre au test.

    Seul PASS compte. Tout FAIL ou UNKNOWN fait le code de sortie 3.

    `--plan` liste les tests qui s'appliquent à un goatnet, par couche, et ne touche aucune machine. Utilisez-le pour voir ce qu'un vrai assay couvrirait. La forme générale est `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey : les versions sont-elles cohérentes ?
tutor-l16-text =
    La commande goatbuilder `survey` vérifie en une passe si les mêmes versions s'exécutent là où elles le devraient :
    - À l'intérieur de chaque goatnet : est-ce que chaque machine exécute la version déclarée, et est-ce que les machines qui devraient correspondre correspondent vraiment ?
    - Entre dev, preprod et prod : une ligne par composant, une colonne par goatnet.

    Une différence que le processus de release attend est signalée mais n'est pas une erreur. Un exemple est une nouvelle version qui est sur dev mais pas encore sur prod. Une différence que rien ne déclare se lit comme `difference not declared`. Une machine que goatbuilder ne peut pas lire est unknown, jamais cohérente.

    Exécutez-le sur la copie d'entraînement. Son `dev` est en retard volontairement, attendez-vous donc à des constats et au code de sortie 3.
tutor-l16-s1 = Exécutez survey sur la copie d'entraînement
tutor-l17-title = Endstate : un goatnet exécute-t-il exactement ce qu'il déclare ?
tutor-l17-text =
    La commande goatbuilder `endstate` vérifie deux choses sur un goatnet :
    1. Chaque service qu'il déclare est déployé, en cours d'exécution, sain et correctement connecté.
    2. Rien ne s'exécute qu'il ne déclare pas. Un service non déclaré s'appelle un orphan.

    La CLI de goatbuilder déduit ce qu'un site devrait exécuter à partir des propres réglages du site, pas d'une liste séparée.

    Un contrôle qui n'a rien à lire est marqué UNOBSERVED. C'est une réponse honnête, pas un échec. Un FAIL ou un orphan fait le code de sortie 3.

    La forme générale est `goatbuilder endstate <goatnet>`. La commande ci-dessous vérifie un site d'exemple à partir de données sauvegardées, donc elle ne touche aucune machine.
tutor-l17-s1 = Vérifiez le site d'exemple
tutor-l18-title = Bonus : soyez curieux
tutor-l18-text =
    Cette leçon est optionnelle. Exécutez la commande ci-dessous et le tutor est terminé.

    La CLI de goatbuilder a quelques petites fonctionnalités cachées. Elles ne sont ni dans l'aide ni dans ces leçons. Certaines répondent à des mots que les utilisateurs Unix de longue date tapent souvent dans un nouveau programme, juste pour voir ce qui se passe. La plupart des choses que vous essaierez ne feront rien.

    Cette leçon ne dit pas ce qu'elles sont. Les trouver, c'est le but.

    La commande ci-dessous liste chaque erreur que goatbuilder peut expliquer. C'est un bon endroit pour commencer à explorer.
tutor-l18-s1 = Listez chaque erreur que goatbuilder peut expliquer
tutor-heading = Leçon { $n } sur { $total } · { $title }
tutor-setup = Exécutez d'abord ceci une fois dans ce terminal :
tutor-not-recording = Ce terminal n'enregistre pas vos essais, donc le tutor ne peut pas cocher vos étapes. Exécutez ceci une fois, puis exécutez les étapes :
tutor-type = Faites ces étapes dans votre terminal ({ $met } sur { $total } faites) :
tutor-rerun = Après chaque étape, relancez `goatbuilder tutor`. Il vérifie ce que vous avez fait et coche l'étape.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Bienvenue dans GOATBUILDER Tutor
tutor-bar-progress = Progression
tutor-bar-done = { $done } sur { $total } leçons faites
tutor-after-step = Étape { $met } sur { $total } faite. Suivant :
tutor-after-lesson = La leçon { $n } est terminée. Relancez `goatbuilder tutor` pour commencer la leçon { $next }. Pour la relire : `goatbuilder tutor --lesson { $n }`.
tutor-after-final = La leçon { $n } est terminée, et le tutor aussi. Relancez `goatbuilder tutor` pour le voir marqué comme fait. Pour la relire : `goatbuilder tutor --lesson { $n }`.

tutor-review = Vous avez terminé cette leçon, donc cette page est seulement à lire. Exécutez `goatbuilder tutor` pour revenir là où vous en étiez.
tutor-other-lessons = Pour relire une leçon : `goatbuilder tutor --lesson 1`. Pour les voir toutes : `goatbuilder tutor --list`.
tutor-l12-title = Ajoutez une machine à un site en cours d'exécution
tutor-l12-text =
    La commande goatbuilder `join-node` ajoute une machine de plus à un site déjà en cours d'exécution. Il affiche d'abord un plan, avec une ligne par phase. Chaque phase a un type :
    - tool : goatbuilder exécute un script.
    - ansible : goatbuilder exécute un playbook.
    - gate : une vérification qui doit réussir avant la phase suivante.
    - manual : vous le faites vous-même, et l'exécution s'arrête.

    La commande ci-dessous a `--dry-run`. Elle lit ce checkout et ne change rien. Sous le tableau, lisez ce que vous faites à la main.
tutor-l13-title = Connectez-vous au goatnet d'une autre organisation
tutor-l13-text =
    La commande goatbuilder `federate` connecte votre goatnet à un goatnet qu'une autre organisation exécute. Chaque côté construit sa moitié d'un accord. Vous échangez les moitiés, et chaque côté les combine dans le même accord.

    Confirmez la clé de l'autre côté d'une façon séparée, comme un appel de confiance. Une clé qui arrive dans leur paquet ne prouve rien.

    La commande ci-dessous a `--dry-run`, donc elle ne change rien. Une exécution réelle s'arrête à la phase 4, car vous faites cette phase à la main.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Cette commande ne fait pas partie de la leçon { $n }. Exécutez `goatbuilder tutor` pour voir où vous en êtes.
tutor-l1-s2 = Interrogez doctor sur un goatnet
tutor-l1-text =
    La CLI de goatbuilder construit, déploie et release le logiciel GOAT. Avant de l'utiliser, vérifiez votre installation avec deux commandes :
    - La commande goatbuilder `validate` vérifie le fichier de configuration, `ops/goatbuilder/goatbuilder.yaml`. Il ne lit que ce checkout et n'a besoin d'aucun réseau.
    - La commande goatbuilder `doctor` vérifie ce dont un déploiement réel a besoin : vos outils, vos connexions, si ce checkout correspond à main, et comment goatbuilder atteint chaque goatnet. Un goatnet est un environnement GOAT.

    La commande goatbuilder `doctor` affiche combien de contrôles ont réussi, puis chaque problème avec sa solution. Dans cette leçon, un problème est une information, pas une erreur. Lisez les solutions, mais vous n'êtes pas obligé de les appliquer maintenant.

    La plupart des leçons n'ont besoin d'aucune connexion. Les leçons 2 à 5, 10, 11 et 16 utilisent une copie d'entraînement. Les leçons 8, 9, 12, 13, 15, 17 et 18 ne font que lire ce checkout. La leçon 6 a besoin de `gh` et du mesh, la leçon 7 a besoin de `gh`, et la leçon 14 a besoin du mesh. Aucune leçon n'a besoin de 1Password.
tutor-l14-text =
    La CLI de goatbuilder peut se mettre à jour lui-même depuis la copie que votre goatnet sert à `get.<site>.<zone>/goatbuilder/`.

    La commande goatbuilder `update --check` ne fait que lire. Il compare votre version avec la version servie et ne change rien. Le code de sortie 0 veut dire que vous êtes à jour. Le code de sortie 3 veut dire qu'il existe une version plus récente.

    Une vraie mise à jour, sans `--check`, vérifie la somme de contrôle et la signature du téléchargement avant de remplacer quoi que ce soit. Elle échange ensuite le fichier en une étape.

    La CLI de goatbuilder doit savoir quel goatnet interroger : passez `--site` avec ce goatnet, ou définissez `GOATBUILDER_UPDATE_URL`. Vous devez être sur le mesh de ce goatnet. Si ce n'est pas le cas, `--check` dit qu'il ne peut pas atteindre le serveur et nomme l'adresse qu'il a essayée. C'est le bon résultat pour cette leçon.
tutor-l15-s1 = Listez les tests qui s'appliquent à un goatnet
tutor-l6-text =
    Un release fait avancer une nouvelle version dans l'ordre à travers trois goatnets : dev, puis preprod, puis prod.

    1. Quelqu'un tague la version sur main, et CI la construit.
    2. dev : goatbuilder la déploie et exécute les contrôles d'acceptation.
    3. preprod : goatbuilder la déploie à nouveau, exécute les contrôles à nouveau et enregistre les résultats.
    4. La version attend alors sur preprod pendant un temps fixé. Rien n'avance vers prod par lui-même.
    5. prod : après cette attente, une personne exécute le release avec `--from prod`. La CLI de goatbuilder déploie vers prod et vérifie les trois goatnets.

    La commande goatbuilder `promote status` montre où se trouve chaque composant. Un candidat est une version en cours de test. La forme générale d'un release est `goatbuilder release <component> --tag <tag> --from dev`. Les commandes ci-dessous utilisent un composant de ce checkout et un tag d'exemple. Chacune a `--dry-run`, donc rien n'est tagué ni déployé.

    Les simulations ont besoin de `gh` et du mesh. Si vous n'êtes pas admin du dépôt, mettez `GOAT_RC_ROLE_CHECK=0` avant chaque commande `release`.
tutor-l9-s1 = Créez une vraie erreur avec un goatnet qui n'existe pas
tutor-l9-text =
    Quand goatbuilder refuse, il affiche un code. Les codes qui commencent par `GB-` viennent de goatbuilder, `E_` de la CLI goat, et `X-` d'un autre outil que goatbuilder a exécuté.

    La commande goatbuilder `explain` prend un code, ou le texte d'une ligne d'erreur, et vous dit ce qui s'est passé, pourquoi, comment le corriger, et où en lire plus.

    Demander un goatnet que ce checkout ne déclare pas crée une vraie erreur à expliquer.
