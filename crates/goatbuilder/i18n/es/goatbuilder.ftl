### goatbuilder — es (es)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Construye y despliega un entorno goat: declarar, converger, verificar — bajo un único contrato de despliegue
help-flag-output = Formato de salida: tabla para personas; JSON o YAML para programas
help-flag-repo = Raíz del repositorio. Por defecto, el checkout desde el que se ejecuta este binario
help-flag-yes = Confirma un verbo declarativo sin preguntar (para automatización)
help-flag-frozen = Rechaza cualquier ejecución que requiera avanzar (fast-forward) el checkout (C3 estricto)
help-flag-mock = Ejecuta contra un directorio de fixture en lugar de hosts: sin registro, sin transporte, determinista (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Idioma de la ayuda y las explicaciones de error (una etiqueta como fr, uk, de); la salida que lee un pipeline sigue en inglés. Por defecto: la configuración regional del SO
help-cmd-validate = Comprueba una config contra el esquema versionado y las reglas entre campos, sin conexión
help-cmd-doctor = Sondea lo que necesita una ejecución antes de cortar nada: dependencias, frescura del checkout, el transporte de cada entorno, y que el playbook de cada componente resuelva a un modo de destino definido
help-cmd-status = Declarado frente a registrado, por máquina: converged, pending o unknown (C7, C8). Solo lee. Sale con 3 mientras alguna máquina no esté converged
help-cmd-deploy = Pone en un entorno el pin que declara main y lo demuestra: resuelve desde los pins el componente y los grupos del entorno, el play y su modo de destino, cada secreto que el play usa en plantillas, el transporte; ejecuta el play con ansible-preflight; lee de vuelta el ledger de despliegue de cada host y lo da por terminado solo cuando la fila más reciente nombra el digest declarado con action=deploy (C7). Sobre un fixture (`--mock`), el converger con guion sustituye a los hosts
help-cmd-release = Lleva un release candidate desde un tag en main, pasando por la validación en vivo de la goatnet dev, hasta el escalón preprod (ADR 1173); tras el soak, con `--from prod`, lo gradúa, lo integra en fleet:, despliega prod y reverifica cada escalón. Sustituye ops/scripts/rc.sh paso a paso
help-cmd-rollback = Revierte un componente desplegado a su N-ésima ref más reciente, según el tipo de artefacto, leyendo el ledger del host como fuente de verdad (ADR 1060). Sustituye ops/scripts/rollback.sh: salida 3 rechazado por inseguro (data, cert, forward-only, reversible:false), 4 guiado (config, acl)
help-cmd-fastlane = Un solo comando desde un commit fusionado hasta cada goatnet (ADR 1127): la excepción deliberada a la escalera. Sustituye ops/scripts/fastlane.sh paso a paso; rechaza un digest sin pin o un conjunto de graduación abierto, y ejecuta los despliegues a través de este binario
help-cmd-standup = Un sitio de cero a verificado: las fases del standup en el orden de la doctrina con una compuerta fail-closed en cada extremo, el host de observabilidad como rama paralela, las clases de rescate y las superficies entre goatnets al final (ADR 1183). Sustituye ops/scripts/goat-standup.sh paso a paso. Sobre un fixture (`--mock`), el converger con guion converge cada componente de cada clase y el tiempo declarado → verificado es la medida de la goatnet de una hora
help-cmd-join-node = Una máquina más de una clase en un sitio activo, declarada → convergida → verificada: la secuencia ejecutada de docs/tutorials/02-first-federated-node.md como fases numeradas con sus compuertas (alcance, clave, trabajo de repo primero, baseline, WireGuard del kernel antes y después del alta, el plano ACL del kernel como compuerta del alta, el converge de la clase, aceptación)
help-cmd-federate = Un peering soberano declarado, convergido y verificado (ADR 1157): se mide nuestra mitad, se intercambian los paquetes fuera de banda, se compone y verifica un acuerdo, se acuñan las hojas a partir de él, las reglas las renderiza el único escritor, se despliega el edge, se abre el transporte que eligió el acuerdo, aceptación en los cuatro niveles. Sustituye ops/scripts/peering-standup.sh paso a paso
help-cmd-pins = UNA pasada de resolución con DOS renderizados: el pins.yml de group_vars de cada miembro listado y el artefacto IntentV1 de cada goatnet listada, desde ops/pins/fleet-pins.yaml. Sustituye ops/pins/generate-pins.py byte a byte. `--check` valida, regenera y rechaza deriva, un intent sin firmar o manipulado, y un pin gobernado definido fuera de pins.yml (salida 1)
help-cmd-promote = Guía y observa la graduación de releases por la escalera de goatnets (ADR 1120, 1125): status, abrir un candidato, graduar, graduar un conjunto, la vía hotfix, review, readiness, abandon, check. Sustituye ops/release/graduate.py paso a paso. Nada aquí gradúa por sí solo: una mutación solo corre cuando se invoca, imprime lo que hizo y pide --yes sin terminal (C10, ADR 1177)
help-cmd-explain = Interpreta un error: un código (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) o cualquier texto de error — propio de este binario o palabras de una herramienta que transportó (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Imprime qué pasó, por qué, qué hacer y dónde leer más; `-` lee el texto de stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Un sitio de verificado a nada — el espejo de standup, con los registros conservados (un burn-down agrega filas de teardown, nunca borra). Sobre un fixture (`--mock`) ensaya el burn-down en orden inverso de fases; en hosts reales imprime el orden y se niega a ejecutar hasta que cada fase declare su play de teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lecciones al estilo vimtutor: cada una indica el comando exacto a ejecutar contra una copia desechable de un fixture (o, en solo lectura, contra este checkout), y luego verifica lo que ese comando realmente hizo — su código de salida y sus filas — antes de desbloquear la siguiente. `tutor` muestra la lección en curso y la marca como hecha cuando tus ejecuciones la cumplen; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Oculto (src/fun.rs)
help-cmd-argan = Oculto (src/fun.rs)
help-cmd-bound = Oculto (crates/goat-fun)
help-cmd-secrets = La postura de secretos declarada (ADR 1204): qué almacén rige (1password, openbao, sops, git-crypt, env-file, env), qué hay disponible, cómo aportar lo que falta y las verificaciones propias del almacén. `init --env <goatnet>` escribe el archivo env del quickstart sin SaaS: un valor aleatorio fuerte para cada variable que los plays de la goatnet usan en plantillas, 0600, ignorado por git, sin rotar nunca un valor ya presente. Mismo módulo que ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Lee la fila más reciente del ledger de despliegues de cada host para un componente y dice si nombra el ref que declara main (C7). Solo lectura; sale con código 3 mientras algún host esté pendiente o desconocido. Es la misma comprobación que ejecuta `deploy --verify-only`
help-cmd-diff = Qué difiere de main ahora mismo (C8): el ref declarado en main frente a la fila más reciente del ledger de cada host, para un componente o, sin indicarlo, para todos los componentes del entorno, leído en una pasada por grupo de miembros. Solo lectura; sale con código 3 ante cualquier deriva o host ilegible
help-cmd-history = El ledger de despliegues como línea de tiempo, de más reciente a más antiguo, en todos los hosts del entorno (C8): cada fila es una entrada con la máquina, la hora, la acción (deploy o rollback) y la versión. La versión anterior de la entrada y el operador que la ejecutó están en `-o json`. Solo lectura
help-cmd-plan = Escribe en un archivo de plan, para revisión, lo que haría un verbo declarativo: los refs declarados, el play y el destino, el ledger, el commit de main, el transporte (deploy); el tipo de artefacto y el ref de destino (rollback). No toca el estado de ningún host. Ejecútalo tal como se revisó con `apply`
help-cmd-apply = Ejecuta exactamente un plan revisado. Se niega (sale con código 1 e indica qué cambió) cuando la declaración de main, el commit de pins, el play o el destino del rollback cambiaron desde que se escribió el plan; un transporte distinto se informa, no se rechaza
help-cmd-adopt = Declara lo que un entorno ya ejecuta: lee los ledgers de cada host y propone una desviación de pins por cada componente cuyo ref en ejecución difiere de la declaración de main. Por defecto solo informa; con --yes las ediciones se escriben en un worktree nuevo de main para revisión. Nunca despliega

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Todavía no: { $why }
tutor-lesson-done = Lección { $n } completada: { $title }.
tutor-all-done-reset = Todas las lecciones están completas. `goatbuilder tutor --reset` empieza de nuevo; los tutoriales van más allá: docs/tutorials/.
tutor-all-done = Todas las lecciones están completas. Los tutoriales van más allá: docs/tutorials/.
tutor-workspace = Espacio de trabajo: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Comprueba que una goatnet se construyó correctamente, capa por capa: todas las comprobaciones de aceptación que aplican al sitio, incluidas las que un operador ejecuta con la CLI de goat. La fase de aceptación del despliegue como comando propio. Solo lee; sale con código 3 mientras alguna comprobación falle o no pueda responder
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Sitio aislado: no descarga el trunk; evalúa el checkout contra el trunk en caché y registra la ejecución como sin conexión (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Comprueba que una goatnet está en su estado final declarado: todo servicio que sus propias declaraciones dicen que ejecuta está desplegado, en ejecución, sano y cumpliendo su función, con sus conexiones zero-trust establecidas, y que no se ejecuta nada que no declare. Solo lectura; sale con código 3 mientras algo no cuadre
help-cmd-survey = Qué tan consistente es el software dentro de cada goatnet y entre los goatnets de la escalera, leído en una sola pasada paralela. Solo lee

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Qué tan consistente es el software dentro de cada goatnet y entre las goatnets de la escalera, leído en una sola pasada paralela. Solo lectura.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Reemplaza este binario con la versión publicada que su goatnet sirve en `get.<site>.<zone>/goatbuilder/`: la descarga se verifica contra su `.sha256` y contra el `SHA256SUMS` firmado antes de tocar nada, y el reemplazo es atómico. Ya actualizado: una fila, salida 0. `--check` solo lee (salida 3 si está desactualizado). La fuente: `GOATBUILDER_UPDATE_URL`; si no, `--site` desde su group_vars; si no, la que usó la última actualización
tutor-prereq = Antes de empezar: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Colorea el glifo ok/FAIL y el veredicto de la tabla (auto|always|never). `auto` colorea solo en un terminal; NO_COLOR y CLICOLOR_FORCE tienen prioridad. JSON y YAML nunca se colorean
help-flag-verbose = Registra lo que hace una ejecución —los comandos externos que lanza y la fase de red de `update`— en stderr, para que `-o json|yaml` siga siendo un documento limpio. O define RUST_LOG=goatbuilder=debug
help-flag-timeout = Segundos máximos para la fase de conexión/alcance antes de fallar rápido: la única fase que puede bloquearse ante un host inaccesible (la consulta de `update` a la versión servida). NO limita la ejecución

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-completion = Imprime un script de autocompletado de shell en stdout (`goatbuilder completion zsh >> ~/.zshrc`). Para candidatos en vivo que un script estático no puede conocer (los sitios, componentes y goatnets que declara este checkout), conecta un hook a `goatbuilder _complete` (ver la cabecera del script generado)
help-cmd-docs = La referencia de comandos: `goatbuilder docs` apunta a ella; `--markdown` emite todo el contenido, generado a partir del árbol de comandos real para que nunca se desincronice de `--help`
help-cmd-_complete = Oculto: emite candidatos de autocompletado en vivo para hooks de shell, uno por línea (los sitios, componentes y goatnets que declara este checkout). Solo lee, nunca falla de forma ruidosa: un hook de autocompletado no debe corromper el prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Antes de un despliegue, el informe de preparación y reversibilidad (ADR 1211): tres datos — State (stateless | stateful), Rollback (pin-revert | restore-required | none) y Scope (single | coupled) — declarados por componente y FAIL-CLOSED si no se declaran, con la mitad de preparación que prueban los demás verbos (ref descargable, pins --check, la puerta de secretos, fuente actualizada, el estado final declarado) y una validación del dato de rollback (rollback --dry-run para pin-revert, la ventana de retención para restore-required). `deploy` lo ejecuta primero; evaluado con honestidad, nunca un falso verde. Solo lectura; sale con código 3 si no está limpio

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Comprueba tu configuración
tutor-l1-s1 = Comprueba el archivo de configuración
tutor-l2-title = Lee el estado: converged, pending, unknown
tutor-l2-text =
    El comando goatbuilder `status` compara dos cosas para cada máquina: la versión que main declara para ella, y la versión que dice su propio ledger que ejecuta. No cambia nada.

    Cada máquina está en uno de tres estados:
    - converged (convergida): su ledger nombra la versión declarada. Esto está hecho.
    - pending (pendiente): su ledger nombra una versión más antigua, o ninguna todavía.
    - unknown (desconocida): goatbuilder no puede leer su ledger.

    Solo converged cuenta como hecho. Si alguna máquina está pending o unknown, `status` termina con el código 3.

    Desde esta lección, `--mock` apunta goatbuilder a una copia de práctica en tu carpeta tutor en lugar de máquinas reales. Nada de lo que hagas ahí llega a una máquina real.

    Todavía no se ha desplegado nada al `dev` de la copia de práctica. Así que espera cuatro máquinas pending y una unknown. Verás máquinas converged en la lección 3.
tutor-l2-s1 = Lee el estado de dev
tutor-l3-title = Despliega: planifícalo, ejecútalo, demuéstralo
tutor-l3-text =
    Desplegar pone la versión declarada en cada máquina, y luego lee el ledger de cada máquina para probar que llegó. Un despliegue solo está hecho cuando el ledger de cada máquina nombra esa versión. Un despliegue que se ejecutó sin errores no es suficiente.

    Vas a desplegar `portal` en el `preprod` de la copia de práctica. `--dry-run` muestra qué pasaría y no cambia nada. `--ack` registra quién lo hizo; el comando rellena tu nombre desde git. Después del despliegue, `status` debe mostrar las dos máquinas como `ok` y converged.
tutor-l3-s1 = Planifica el despliegue
tutor-l3-s2 = Despliega portal en preprod
tutor-l3-s3 = Comprueba que las dos máquinas estén converged
tutor-l4-title = Drift e historial
tutor-l4-text =
    Dos comandos te ayudan a entender un goatnet antes y después de un cambio. Los dos solo leen.
    - El comando goatbuilder `diff` lista cada máquina cuyo ledger difiere de la versión declarada. Una diferencia así se llama drift.
    - El comando goatbuilder `history` lista cada versión que tuvo un componente en cada máquina, la más reciente primero.

    En `dev`, cada máquina está atrasada. El historial de `portal` en `preprod` ahora incluye tu despliegue de la lección 3.
tutor-l4-s1 = Lista el drift en dev
tutor-l4-s2 = Lee el historial de portal en preprod
tutor-l5-title = Revierte una versión, y ve cómo se rechaza un rollback no seguro
tutor-l5-text =
    Un rollback declara una versión más antigua y la despliega como cualquier otra versión. La CLI de goatbuilder luego lee el ledger de cada máquina para probar que llegó.

    Algunos cambios no se pueden deshacer así. Desplegar una versión más antigua no puede deshacer datos que un componente ha escrito, un certificado, o un cambio que solo avanza. La CLI de goatbuilder los rechaza con el código de salida 3 y dice qué hacer en su lugar.

    El componente `db-schema` de la copia de práctica contiene datos, así que goatbuilder rechaza revertirlo. Ese rechazo es el resultado esperado.
tutor-l5-s1 = Planifica el rollback
tutor-l5-s2 = Revierte portal una versión
tutor-l5-s3 = Intenta revertir db-schema, lo cual goatbuilder rechaza
tutor-l6-title = Release de una versión nueva: dev, luego preprod, luego prod
tutor-l6-s1 = Mira dónde está cada componente
tutor-l6-s2 = Simula el release desde dev
tutor-l6-s3 = Simula el paso de prod
tutor-l7-title = Fastlane, y cuándo no usarlo
tutor-l7-text =
    Fastlane pone una versión en cada goatnet a la vez. Se salta la espera en preprod y los ledgers de release.

    Úsalo solo para una corrección urgente que puedas deshacer volviendo a la versión anterior. Nunca lo uses para un cambio de base de datos, un cambio de identidad o de acceso, o algo que no puedas deshacer así.

    Fastlane rechaza una versión que CI no ha construido, y un componente que es parte de un release todavía en curso.

    La forma general es `goatbuilder fastlane <component> --ref <version> --reason <why>`. Ejecuta el ejemplo de abajo. Tiene `--dry-run`, así que nada cambia. Necesita `gh`.
tutor-l7-s1 = Simula un fastlane
tutor-l8-title = Pins: encuentra el drift y corrígelo de la forma correcta
tutor-l8-text =
    Un archivo fija qué versión ejecuta cada goatnet: `ops/pins/fleet-pins.yaml`. La CLI de goatbuilder genera los otros archivos de versión a partir de él: los archivos `pins.yml` y los archivos de intent firmados.

    El comando goatbuilder `pins --check` compara los archivos generados con `fleet-pins.yaml` y rechaza cualquier archivo que difiera. El comando goatbuilder `pins` los regenera.

    Tu carpeta tutor tiene una copia del repositorio en la que alguien editó un archivo `pins.yml` a mano. Nunca corrijas una edición a mano con otra edición a mano. Cambia `fleet-pins.yaml`, y luego regenera.
tutor-l8-s1 = Comprueba este checkout
tutor-l8-s2 = Comprueba la copia y encuentra la edición a mano
tutor-l8-s3 = Regenera los archivos de la copia
tutor-l8-s4 = Comprueba la copia otra vez
tutor-l9-title = Explica un error
tutor-l9-s2 = Explica el error por su código
tutor-l9-s3 = Explica el error por su texto
tutor-l10-title = Tutorial 1: construye el primer site de un goatnet nuevo
tutor-l10-text =
    Esta lección construye un goatnet entero en una copia de práctica. El goatnet de ejemplo es `zagros`. Su primer site, `alpha`, tiene dos máquinas principales, una máquina de monitoreo y una máquina de rescate.

    El comando goatbuilder `standup` configura cada máquina en un orden fijo y comprueba cada ledger. Su última línea dice que el site está hecho. En máquinas reales, esa línea también muestra cuánto tardó.

    El tutorial completo es `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Construye el primer site, alpha
tutor-l10-s2 = Comprueba el resultado
tutor-l11-title = Tutorial 2: añade un segundo site
tutor-l11-text =
    Esta lección empieza donde terminó el tutorial 1. Añade un segundo site, `beta`, en una copia de práctica nueva, `zagros-2`.

    Un segundo site copia sus cuentas de usuario del primer site. Su standup también cambia una vez el primer site, para que la copia funcione en ambas direcciones. Un paso llamado `alpha/core` hace esto.

    El tutorial completo es `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Construye el segundo site, beta
tutor-l11-s2 = Comprueba los dos sites
tutor-l12-s1 = Planifica añadir una máquina
tutor-l13-s1 = Planifica una federación
tutor-l14-title = Mantén goatbuilder actualizado
tutor-l14-s1 = Pregunta si existe un goatbuilder más nuevo
tutor-l15-title = Assay: comprueba cómo se construyó un goatnet
tutor-l15-text =
    El comando goatbuilder `assay` comprueba que un goatnet se construyó correctamente, una capa a la vez, usando las comprobaciones de aceptación. Cada prueba recibe uno de tres resultados:
    - PASS: la prueba pasó.
    - FAIL: el goatnet tiene un problema.
    - UNKNOWN: nada pudo responder la prueba.

    Solo PASS cuenta. Cualquier FAIL o UNKNOWN hace el código de salida 3.

    `--plan` lista las pruebas que aplican a un goatnet, por capa, y no toca ninguna máquina. Úsalo para ver qué cubriría un assay real. La forma general es `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: ¿son las versiones consistentes?
tutor-l16-text =
    El comando goatbuilder `survey` comprueba en una sola pasada si las mismas versiones se ejecutan donde deberían:
    - Dentro de cada goatnet: ¿ejecuta cada máquina la versión declarada, y coinciden realmente las máquinas que deberían coincidir?
    - Entre dev, preprod y prod: una fila por componente, una columna por goatnet.

    Una diferencia que el proceso de release espera se informa pero no es un error. Un ejemplo es una versión nueva que está en dev pero todavía no en prod. Una diferencia que nada declara se lee como `difference not declared`. Una máquina que goatbuilder no puede leer es unknown, nunca consistente.

    Ejecútalo en la copia de práctica. Su `dev` está atrasado a propósito, así que espera hallazgos y el código de salida 3.
tutor-l16-s1 = Ejecuta survey en la copia de práctica
tutor-l17-title = Endstate: ¿ejecuta un goatnet exactamente lo que declara?
tutor-l17-text =
    El comando goatbuilder `endstate` comprueba dos cosas sobre un goatnet:
    1. Cada servicio que declara está desplegado, en ejecución, sano y conectado correctamente.
    2. No se ejecuta nada que no declare. Un servicio no declarado se llama orphan.

    La CLI de goatbuilder deduce lo que un site debería ejecutar a partir de los propios ajustes del site, no de una lista separada.

    Una comprobación que no tiene nada que leer se marca como UNOBSERVED. Esa es una respuesta honesta, no un fallo. Un FAIL o un orphan hace el código de salida 3.

    La forma general es `goatbuilder endstate <goatnet>`. El comando de abajo comprueba un site de ejemplo a partir de datos guardados, así que no toca ninguna máquina.
tutor-l17-s1 = Comprueba el site de ejemplo
tutor-l18-title = Bonus: sé curioso
tutor-l18-text =
    Esta lección es opcional. Ejecuta el comando de abajo y el tutor está terminado.

    La CLI de goatbuilder tiene algunas funciones pequeñas y ocultas. No están en la ayuda ni en estas lecciones. Algunas responden a palabras que los usuarios veteranos de Unix suelen escribir en un programa nuevo, solo para ver qué pasa. La mayoría de las cosas que intentes no harán nada.

    Esta lección no dice cuáles son. Encontrarlas es el objetivo.

    El comando de abajo lista cada error que goatbuilder puede explicar. Es un buen lugar para empezar a explorar.
tutor-l18-s1 = Lista cada error que goatbuilder puede explicar
tutor-heading = Lección { $n } de { $total } · { $title }
tutor-setup = Primero, ejecuta esto una vez en esta terminal:
tutor-not-recording = Esta terminal no está registrando tus ejecuciones, así que el tutor no puede marcar tus pasos. Ejecuta esto una vez, y luego ejecuta los pasos:
tutor-type = Haz estos pasos en tu terminal ({ $met } de { $total } hechos):
tutor-rerun = Después de cada paso, vuelve a ejecutar `goatbuilder tutor`. Comprueba lo que hiciste y marca el paso.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Bienvenido a GOATBUILDER Tutor
tutor-bar-progress = Progreso
tutor-bar-done = { $done } de { $total } lecciones hechas
tutor-after-step = Paso { $met } de { $total } hecho. Siguiente:
tutor-after-lesson = La lección { $n } está completa. Ejecuta `goatbuilder tutor` para empezar la lección { $next }. Para volver a leerla: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = La lección { $n } está completa, y el tutor también. Ejecuta `goatbuilder tutor` para verlo marcado como hecho. Para volver a leerla: `goatbuilder tutor --lesson { $n }`.

tutor-review = Terminaste esta lección, así que esta página es solo para leer. Ejecuta `goatbuilder tutor` para volver a donde estás.
tutor-other-lessons = Para volver a leer una lección: `goatbuilder tutor --lesson 1`. Para verlas todas: `goatbuilder tutor --list`.
tutor-l12-title = Añade una máquina a un site en marcha
tutor-l12-text =
    El comando goatbuilder `join-node` añade una máquina más a un site que ya está en marcha. Primero imprime un plan, con una fila por fase. Cada fase tiene un tipo:
    - tool: goatbuilder ejecuta un script.
    - ansible: goatbuilder ejecuta un playbook.
    - gate: una comprobación que debe pasar antes de la siguiente fase.
    - manual: lo haces tú mismo, y la ejecución se detiene.

    El comando de abajo tiene `--dry-run`. Lee este checkout y no cambia nada. Debajo de la tabla, lee lo que haces a mano.
tutor-l13-title = Conéctate al goatnet de otra organización
tutor-l13-text =
    El comando goatbuilder `federate` conecta tu goatnet a un goatnet que ejecuta otra organización. Cada lado construye su mitad de un acuerdo. Intercambiáis las mitades, y cada lado las combina en el mismo acuerdo.

    Confirma la clave del otro lado por una vía separada, como una llamada de confianza. Una clave que llega dentro de su paquete no prueba nada.

    El comando de abajo tiene `--dry-run`, así que no cambia nada. Una ejecución real se detiene en la fase 4, porque esa fase la haces a mano.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Ese comando no es parte de la lección { $n }. Ejecuta `goatbuilder tutor` para ver dónde estás.
tutor-l1-s2 = Pregúntale a doctor por un goatnet
tutor-l1-text =
    La CLI de goatbuilder construye, despliega y hace release del software GOAT. Antes de usarlo, comprueba tu configuración con dos comandos:
    - El comando goatbuilder `validate` comprueba el archivo de configuración, `ops/goatbuilder/goatbuilder.yaml`. Solo lee este checkout y no necesita red.
    - El comando goatbuilder `doctor` comprueba lo que necesita un despliegue real: tus herramientas, tus inicios de sesión, si este checkout coincide con main, y cómo goatbuilder llega a cada goatnet. Un goatnet es un entorno GOAT.

    El comando goatbuilder `doctor` imprime cuántas comprobaciones pasaron, y luego cada problema con su solución. En esta lección, un problema es información, no un error. Lee las soluciones, pero no tienes que aplicarlas ahora.

    La mayoría de las lecciones no necesitan ningún inicio de sesión. Las lecciones 2 a 5, 10, 11 y 16 usan una copia de práctica. Las lecciones 8, 9, 12, 13, 15, 17 y 18 solo leen este checkout. La lección 6 necesita `gh` y el mesh, la lección 7 necesita `gh`, y la lección 14 necesita el mesh. Ninguna lección necesita 1Password.
tutor-l14-text =
    La CLI de goatbuilder puede actualizarse a sí mismo desde la copia que tu goatnet sirve en `get.<site>.<zone>/goatbuilder/`.

    El comando goatbuilder `update --check` solo lee. Compara tu versión con la versión servida y no cambia nada. El código de salida 0 significa que estás actualizado. El código de salida 3 significa que existe una versión más nueva.

    Una actualización real, sin `--check`, verifica la suma de comprobación y la firma de la descarga antes de reemplazar nada. Luego cambia el archivo en un paso.

    La CLI de goatbuilder necesita saber a qué goatnet preguntar: pasa `--site` con ese goatnet, o fija `GOATBUILDER_UPDATE_URL`. Debes estar en el mesh de ese goatnet. Si no lo estás, `--check` dice que no puede alcanzar el servidor y nombra la dirección que intentó. Ese es el resultado correcto para esta lección.
tutor-l15-s1 = Lista las pruebas que aplican a un goatnet
tutor-l6-text =
    Un release mueve una versión nueva por tres goatnets en orden: dev, luego preprod, luego prod.

    1. Alguien etiqueta la versión en main, y CI la construye.
    2. dev: goatbuilder la despliega y ejecuta las comprobaciones de aceptación.
    3. preprod: goatbuilder la despliega otra vez, ejecuta las comprobaciones otra vez y registra los resultados.
    4. La versión entonces espera en preprod durante un tiempo fijo. Nada avanza a prod por sí solo.
    5. prod: después de esa espera, una persona ejecuta el release con `--from prod`. La CLI de goatbuilder despliega a prod y comprueba los tres goatnets.

    El comando goatbuilder `promote status` muestra dónde está cada componente. Un candidato es una versión que se está probando. La forma general de un release es `goatbuilder release <component> --tag <tag> --from dev`. Los comandos de abajo usan un componente de este checkout y un tag de ejemplo. Cada uno tiene `--dry-run`, así que no se etiqueta ni se despliega nada.

    Las simulaciones necesitan `gh` y el mesh. Si no eres admin del repositorio, pon `GOAT_RC_ROLE_CHECK=0` antes de cada comando `release`.
tutor-l9-s1 = Crea un error real con un goatnet que no existe
tutor-l9-text =
    Cuando goatbuilder rechaza, imprime un código. Los códigos que empiezan con `GB-` vienen de goatbuilder, `E_` de la CLI de goat, y `X-` de otra herramienta que goatbuilder ejecutó.

    El comando goatbuilder `explain` toma un código, o el texto de una línea de error, y te dice qué pasó, por qué, cómo arreglarlo, y dónde leer más.

    Pedir un goatnet que este checkout no declara crea un error real para explicar.
