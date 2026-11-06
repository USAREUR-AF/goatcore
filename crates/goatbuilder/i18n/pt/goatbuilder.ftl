### goatbuilder — pt (pt)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Construa e entregue um ambiente goat: declare, convirja, verifique — sob um único contrato de deploy
help-flag-output = Formato de saída: tabela para pessoas; JSON ou YAML para programas
help-flag-repo = A raiz do repositório. Padrão: o checkout em que este binário é executado
help-flag-yes = Confirma um verbo declarativo sem perguntar (para automação)
help-flag-frozen = Recusa qualquer execução que precise avançar (fast-forward) o checkout (C3 estrito)
help-flag-mock = Executa contra um diretório de fixture em vez de hosts: sem registry, sem transporte, determinístico (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Idioma da ajuda e das explicações de erro (uma tag como fr, uk, de); a saída lida por um pipeline continua em inglês. Padrão: o locale do SO
help-cmd-validate = Verifica uma config contra o schema versionado e as regras entre campos, offline
help-cmd-doctor = Sonda o que uma execução precisa antes de qualquer corte: dependências, atualização do checkout, o transporte de cada ambiente e se o playbook de cada componente resolve para um modo de destino definido
help-cmd-status = Declarado contra registrado, por máquina: converged, pending ou unknown (C7, C8). Só leitura. Sai com 3 enquanto qualquer máquina não estiver converged
help-cmd-deploy = Coloca o pin que a main declara em um ambiente e o comprova: resolve o componente e os grupos do ambiente a partir dos pins, o play e seu modo de destino, cada segredo que o play usa em template, o transporte; executa o play via ansible-preflight; lê o ledger de deploy de volta de cada host e só dá por concluído quando a linha mais recente nomeia o digest declarado com action=deploy (C7). Em uma fixture (`--mock`), o converger roteirizado substitui os hosts
help-cmd-release = Leva um release candidate, de uma tag na main, pela validação ao vivo da goatnet dev até o degrau preprod (ADR 1173); após o soak, com `--from prod`, o gradua, incorpora em fleet:, faz deploy em prod e reverifica cada degrau. Substitui ops/scripts/rc.sh passo a passo
help-cmd-rollback = Reverte um componente implantado para sua N-ésima ref mais recente, por tipo de artefato, lendo o ledger do host como fonte da verdade (ADR 1060). Substitui ops/scripts/rollback.sh: sai com 3 se recusado por ser inseguro (data, cert, forward-only, reversible:false), 4 se guiado (config, acl)
help-cmd-fastlane = Um comando de um commit mergeado até todas as goatnets (ADR 1127): a exceção deliberada à escada. Substitui ops/scripts/fastlane.sh passo a passo; recusa digest sem pin ou conjunto de graduação aberto, e executa os deploys por este binário
help-cmd-standup = Um site do zero ao verificado: as fases do standup na ordem da doutrina, com gate fail-closed em cada ponta, o host de observabilidade como perna paralela, as classes de resgate e as superfícies entre goatnets por último (ADR 1183). Substitui ops/scripts/goat-standup.sh passo a passo. Em uma fixture (`--mock`), cada componente de cada classe é convergido pelo converger roteirizado e o tempo declarado → verificado é a medida da goatnet de uma hora
help-cmd-join-node = Mais uma máquina de uma classe em um site ao vivo, declarada → convergida → verificada: a sequência executada de docs/tutorials/02-first-federated-node.md em fases numeradas, com seus gates (alcance, chave, trabalho no repo primeiro, baseline, WireGuard do kernel antes e depois do enrolment, o plano ACL do kernel como gate de enrolment, o converge da classe, aceite)
help-cmd-federate = Um peering soberano declarado, convergido e verificado (ADR 1157): nossa metade medida, os pacotes trocados fora de banda, um acordo composto e verificado, as folhas emitidas a partir dele, as regras renderizadas pelo escritor único, a borda implantada, o transporte escolhido pelo acordo aberto, aceite nos quatro níveis. Substitui ops/scripts/peering-standup.sh passo a passo
help-cmd-pins = UMA passada de resolução com DUAS renderizações: o pins.yml de group_vars de cada membro listado e o artefato IntentV1 de cada goatnet listada, a partir de ops/pins/fleet-pins.yaml. Substitui ops/pins/generate-pins.py byte a byte. `--check` valida, regenera e recusa drift, intent sem assinatura ou adulterado, e pin governado definido fora de pins.yml (saída 1)
help-cmd-promote = Guia e acompanha a graduação de releases pela escada de goatnets (ADR 1120, 1125): status, abrir um candidato, graduar, graduar um conjunto, o caminho de hotfix, review, readiness, abandon, check. Substitui ops/release/graduate.py passo a passo. Nada aqui gradua sozinho: uma mutação só roda quando invocada, imprime o que fez e exige --yes sem terminal (C10, ADR 1177)
help-cmd-explain = Interpreta um erro: um código (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) ou qualquer texto de erro — deste binário ou de uma ferramenta que ele repassou (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Imprime o que aconteceu, por quê, o que fazer e onde ler mais; `-` lê o texto do stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Um site de verificado a nada — o espelho do standup, com os registros mantidos (um burn-down acrescenta linhas de teardown, nunca apaga). Em uma fixture (`--mock`), ensaia o burn-down na ordem inversa das fases; em hosts reais, imprime a ordem e se recusa a executar até que cada fase declare seu play de teardown (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lições no estilo vimtutor: cada uma indica o comando exato a executar contra uma cópia descartável de uma fixture (ou, somente leitura, contra este checkout) e depois confere o que esse comando realmente fez — seu código de saída e suas linhas — antes de liberar a próxima. `tutor` mostra a lição atual e a marca como concluída quando suas execuções a satisfazem; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Oculto (src/fun.rs)
help-cmd-argan = Oculto (src/fun.rs)
help-cmd-bound = Oculto (crates/goat-fun)
help-cmd-secrets = A postura de segredos declarada (ADR 1204): qual store está em vigor (1password, openbao, sops, git-crypt, env-file, env), o que está disponível, como fornecer o que falta e as verificações do próprio store. `init --env <goatnet>` grava o arquivo env do quickstart zero-SaaS: um valor aleatório forte para cada variável que os plays da goatnet geram por template, 0600, ignorado pelo git, sem nunca rotacionar um valor já existente. Mesmo módulo que ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Lê a linha mais recente do ledger de deploy de cada host para um componente e diz se ela cita o ref que o main declara (C7). Só lê; sai com 3 enquanto algum host estiver pendente ou desconhecido. É a mesma verificação que `deploy --verify-only` executa
help-cmd-diff = O que difere do main agora (C8): o ref declarado no main contra a linha mais recente do ledger de cada host, para um componente ou, sem indicar um, para todos os componentes do ambiente, lidos numa só passada por grupo de membros. Só lê; sai com 3 em qualquer divergência ou host ilegível
help-cmd-history = O ledger de deploy como linha do tempo, do mais recente ao mais antigo, em todos os hosts do ambiente (C8): cada linha é um registro com a máquina, a hora, a ação (deploy ou rollback) e a versão. A versão anterior do registro e o operador que o executou estão em `-o json`. Só lê
help-cmd-plan = Grava num arquivo de plano, para revisão, o que um verbo declarativo faria: os refs declarados, o play e o alvo, o ledger, o commit do main, o transporte (deploy); o tipo de artefato e o ref alvo (rollback). Não toca no estado de nenhum host. Execute como revisado com `apply`
help-cmd-apply = Executa exatamente um plano revisado. Recusa (saída 1, indicando o que mudou) quando a declaração do main, o commit dos pins, o play ou o alvo do rollback mudaram desde que o plano foi gravado; mudança de transporte é reportada, não recusada
help-cmd-adopt = Declara o que um ambiente já executa: lê os ledgers de cada host e propõe um desvio de pins para cada componente cujo ref em execução difere da declaração do main. Por padrão, só um relatório; com --yes as edições são gravadas num worktree novo do main para revisão. Nunca faz deploy

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Ainda não: { $why }
tutor-lesson-done = Lição { $n } concluída: { $title }.
tutor-all-done-reset = Todas as lições estão concluídas. `goatbuilder tutor --reset` recomeça; os tutoriais vão além: docs/tutorials/.
tutor-all-done = Todas as lições estão concluídas. Os tutoriais vão além: docs/tutorials/.
tutor-workspace = Espaço de trabalho: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Verifica se uma goatnet foi construída corretamente, camada por camada: todas as verificações de aceitação que se aplicam ao site, incluindo as que o operador executa com a CLI goat. A fase de aceitação do standup como comando próprio. Só lê; sai com código 3 enquanto qualquer verificação falhar ou não puder responder
help-cmd-survey = Quão consistente é o software dentro de cada goatnet e entre os goatnets da escada, lido numa única passagem paralela. Só lê
help-flag-offline = Site isolado: não busca o trunk; avalia o checkout contra o trunk em cache e registra a execução como offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Verifica se uma goatnet está no estado final declarado: todo serviço que as próprias declarações dizem que ela executa está implantado, em execução, saudável e cumprindo sua função, com as arestas zero-trust conectadas, e nada executa sem estar declarado. Somente leitura; sai com código 3 enquanto algo estiver fora do esperado
help-cmd-survey-long =
    Quão consistente é o software dentro de cada goatnet e entre os goatnets da escada, lido numa única passagem paralela. Só lê. Dentro de um goatnet: o ref que o main declara comparado com a linha mais recente do ledger de cada host e com os contentores que ele executa a partir dessa imagem (um em execução; um parado só é avaliado num standby a quente onde nada está em execução), hosts de um goatnet que discordam, o goat CLI de cada host comparado com o que o seu goatnet serve, e o que o goat-get serve comparado com o main. Entre goatnets: uma linha por componente, uma coluna por goatnet, com a classificação da escada para cada diferença, tal como o promote status a dá. Saída 3 em qualquer deriva ou em qualquer coisa não lida; uma diferença que a escada declara é reportada, não tratada como falha.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Substitui este binário pela release que o seu goatnet serve em `get.<site>.<zone>/goatbuilder/`: o download é verificado contra o `.sha256` e o `SHA256SUMS` assinado antes de qualquer alteração, e a troca é atômica. Já atualizado: uma linha, exit 0. `--check` só lê (exit 3 quando desatualizado). A fonte: `GOATBUILDER_UPDATE_URL`, senão `--site` do seu group_vars, senão a usada na última atualização
tutor-prereq = Antes de começar: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Colore o glifo ok/FAIL e o veredito da tabela (auto|always|never). `auto` só colore em terminal; NO_COLOR e CLICOLOR_FORCE têm precedência. JSON e YAML nunca são coloridos
help-flag-verbose = Registra o que a execução faz — os comandos externos que ela chama e a fase de rede do `update` — no stderr, para que `-o json|yaml` continue sendo um documento limpo. Ou defina RUST_LOG=goatbuilder=debug
help-flag-timeout = Segundos para limitar a fase de conexão/alcance antes de falhar rápido — a única fase que pode travar num host inacessível (a busca da versão servida pelo `update`). NÃO limita a execução de um play nem o download do binário (limitado por travamento). Também lê GOATBUILDER_TIMEOUT
help-cmd-completion = Imprime um script de completion de shell no stdout (`goatbuilder completion zsh >> ~/.zshrc`). Para candidatos ao vivo que um script estático não conhece — os sites, componentes e goatnets que este checkout declara — ligue um hook a `goatbuilder _complete` (veja o cabeçalho do script gerado)
help-cmd-docs = A referência de comandos: `goatbuilder docs` aponta para ela; `--markdown` emite tudo, extraído da árvore de comandos ativa, então nunca diverge de `--help`
help-cmd-_complete = Oculto: emite candidatos de completion ao vivo para hooks de shell, um por linha (os sites, componentes e goatnets que este checkout declara). Só lê, nunca falha de forma ruidosa — um hook de completion não pode corromper o prompt

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Antes de um rollout, o relatório de prontidão e reversibilidade (ADR 1211): três fatos — State (stateless | stateful), Rollback (pin-revert | restore-required | none) e Scope (single | coupled) — declarados por componente e FAIL-CLOSED quando não declarados, com a metade de prontidão que os outros verbos comprovam (ref pullable, pins --check, o gate de segredos, fonte atualizada, o estado final declarado) e uma validação do fato de rollback (rollback --dry-run para pin-revert, a janela de retenção para restore-required). `deploy` o executa primeiro; avaliado por honestidade, nunca um falso verde. Só lê; exit 3 quando não está limpo

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Verifique sua configuração
tutor-l1-s1 = Verifique o arquivo de configuração
tutor-l2-title = Leia o estado: converged, pending, unknown
tutor-l2-text =
    O comando goatbuilder `status` compara duas coisas para cada máquina: a versão que a main declara para ela, e a versão que o seu próprio ledger diz que ela executa. Isso não muda nada.

    Cada máquina está em um de três estados:
    - converged (convergida): o ledger dela indica a versão declarada. Isso está concluído.
    - pending (pendente): o ledger dela indica uma versão mais antiga, ou nada ainda.
    - unknown (desconhecida): o goatbuilder não consegue ler o ledger dela.

    Só converged conta como concluído. Se qualquer máquina estiver pending ou unknown, `status` sai com código 3.

    A partir desta lição, `--mock` aponta o goatbuilder para uma cópia de prática na sua pasta do tutor, em vez de máquinas reais. Nada que você fizer ali chega a uma máquina real.

    Ainda não foi feito nenhum deploy no `dev` da cópia de prática. Então espere quatro máquinas pending e uma unknown. Você verá máquinas converged na lição 3.
tutor-l2-s1 = Leia o estado de dev
tutor-l3-title = Deploy: planeje, execute, comprove
tutor-l3-text =
    Fazer o deploy coloca a versão declarada em cada máquina, depois lê o ledger de cada máquina para comprovar que ela chegou. Um deploy só está concluído quando o ledger de cada máquina indica essa versão. Um deploy que rodou sem erros não é suficiente.

    Você vai fazer o deploy de `portal` no `preprod` da cópia de prática. `--dry-run` mostra o que aconteceria e não muda nada. `--ack` registra quem fez; o comando preenche seu nome a partir do git. Depois do deploy, `status` deve mostrar as duas máquinas como `ok` e converged.
tutor-l3-s1 = Planeje o deploy
tutor-l3-s2 = Faça o deploy de portal para preprod
tutor-l3-s3 = Verifique que as duas máquinas estão converged
tutor-l4-title = Drift e histórico
tutor-l4-text =
    Dois comandos ajudam você a entender um goatnet antes e depois de uma mudança. Os dois só leem.
    - O comando goatbuilder `diff` lista cada máquina cujo ledger difere da versão declarada. Uma diferença é chamada de drift.
    - O comando goatbuilder `history` lista cada versão que um componente teve em cada máquina, da mais recente para a mais antiga.

    Em `dev`, cada máquina está atrasada. O histórico de `portal` em `preprod` agora inclui o seu deploy da lição 3.
tutor-l4-s1 = Liste o drift em dev
tutor-l4-s2 = Leia o histórico de portal em preprod
tutor-l5-title = Reverta, e veja um rollback inseguro ser recusado
tutor-l5-text =
    Um rollback declara uma versão anterior e faz o deploy dela como qualquer outra versão. A CLI do goatbuilder então lê o ledger de cada máquina para comprovar que ela chegou.

    Algumas mudanças não podem ser desfeitas assim. Fazer o deploy de uma versão mais antiga não desfaz dados que um componente já escreveu, um certificado, ou uma mudança que só avança. A CLI do goatbuilder recusa essas com código de saída 3 e diz o que fazer em vez disso.

    O componente `db-schema` da cópia de prática guarda dados, então o goatbuilder recusa revertê-lo. Essa recusa é o resultado esperado.
tutor-l5-s1 = Planeje o rollback
tutor-l5-s2 = Reverta portal uma versão
tutor-l5-s3 = Tente reverter db-schema, o que o goatbuilder recusa
tutor-l6-title = Faça release de uma nova versão: dev, depois preprod, depois prod
tutor-l6-s1 = Veja onde cada componente está
tutor-l6-s2 = Simule o release a partir de dev
tutor-l6-s3 = Simule o passo de prod
tutor-l7-title = Fastlane, e quando não usá-lo
tutor-l7-text =
    O Fastlane coloca uma versão em todos os goatnets de uma vez. Ele pula a espera no preprod e os ledgers do release.

    Use-o só para uma correção urgente que você possa desfazer voltando à versão anterior. Nunca o use para uma mudança de banco de dados, uma mudança de identidade ou acesso, ou qualquer coisa que você não possa desfazer assim.

    O Fastlane recusa uma versão que a CI não construiu, e um componente que faz parte de um release ainda em andamento.

    A forma geral é `goatbuilder fastlane <component> --ref <version> --reason <why>`. Execute o exemplo abaixo. Ele tem `--dry-run`, então nada muda. Ele precisa do `gh`.
tutor-l7-s1 = Simule um fastlane
tutor-l8-title = Pins: encontre o drift e corrija do jeito certo
tutor-l8-text =
    Um arquivo define qual versão cada goatnet executa: `ops/pins/fleet-pins.yaml`. A CLI do goatbuilder gera os outros arquivos de versão a partir dele: os arquivos `pins.yml` e os arquivos de intent assinados.

    O comando goatbuilder `pins --check` compara os arquivos gerados com `fleet-pins.yaml` e recusa qualquer arquivo que seja diferente. O comando goatbuilder `pins` os regenera.

    Sua pasta do tutor tem uma cópia do repositório na qual alguém editou um arquivo `pins.yml` manualmente. Nunca corrija uma edição manual com outra edição manual. Mude `fleet-pins.yaml`, depois regenere.
tutor-l8-s1 = Verifique este checkout
tutor-l8-s2 = Verifique a cópia e encontre a edição manual
tutor-l8-s3 = Regenere os arquivos da cópia
tutor-l8-s4 = Verifique a cópia de novo
tutor-l9-title = Explique um erro
tutor-l9-s2 = Explique o erro pelo código
tutor-l9-s3 = Explique o erro pelo texto
tutor-l10-title = Tutorial 1: construa o primeiro site de um novo goatnet
tutor-l10-text =
    Esta lição constrói um goatnet inteiro em uma cópia de prática. O goatnet de exemplo é `zagros`. O primeiro site dele, `alpha`, tem duas máquinas core, uma máquina de monitoramento e uma máquina de resgate.

    O comando goatbuilder `standup` configura cada máquina em uma ordem fixa e verifica cada ledger. A última linha dele diz que o site está concluído. Em máquinas reais, essa linha também mostra quanto tempo levou.

    O tutorial completo é `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Construa o primeiro site, alpha
tutor-l10-s2 = Verifique o resultado
tutor-l11-title = Tutorial 2: adicione um segundo site
tutor-l11-text =
    Esta lição começa onde o tutorial 1 terminou. Ela adiciona um segundo site, `beta`, em uma nova cópia de prática, `zagros-2`.

    Um segundo site copia as contas de usuário do primeiro site. O standup dele também muda o primeiro site uma vez, para que a cópia funcione nas duas direções. Um passo chamado `alpha/core` faz isso.

    O tutorial completo é `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Construa o segundo site, beta
tutor-l11-s2 = Verifique os dois sites
tutor-l12-s1 = Planeje adicionar uma máquina
tutor-l13-s1 = Planeje uma federação
tutor-l14-title = Mantenha o goatbuilder atualizado
tutor-l14-s1 = Pergunte se existe um goatbuilder mais novo
tutor-l15-title = Assay: verifique como um goatnet foi construído
tutor-l15-text =
    O comando goatbuilder `assay` verifica que um goatnet foi construído corretamente, uma camada por vez, usando as verificações de aceitação. Cada teste recebe um de três resultados:
    - PASS: o teste passou.
    - FAIL: o goatnet tem um problema.
    - UNKNOWN: nada pôde responder ao teste.

    Só PASS conta. Qualquer FAIL ou UNKNOWN faz o código de saída ser 3.

    `--plan` lista os testes que se aplicam a um goatnet, por camada, e não toca em nenhuma máquina. Use para ver o que um assay real cobriria. A forma geral é `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: as versões estão consistentes?
tutor-l16-text =
    O comando goatbuilder `survey` verifica em uma única passagem se as mesmas versões rodam onde deveriam:
    - Dentro de cada goatnet: cada máquina roda a versão declarada, e as máquinas que deveriam combinar realmente combinam?
    - Entre dev, preprod e prod: uma linha por componente, uma coluna por goatnet.

    Uma diferença que o processo de release espera é reportada, mas não é um erro. Um exemplo é uma versão nova que está em dev mas ainda não em prod. Uma diferença que nada declara lê-se como `difference not declared`. Uma máquina que o goatbuilder não consegue ler é unknown, nunca consistente.

    Execute na cópia de prática. O `dev` dela está atrasado de propósito, então espere descobertas e código de saída 3.
tutor-l16-s1 = Execute survey na cópia de prática
tutor-l17-title = Endstate: um goatnet executa exatamente o que declara?
tutor-l17-text =
    O comando goatbuilder `endstate` verifica duas coisas sobre um goatnet:
    1. Todo serviço que ele declara está com deploy feito, em execução, saudável e conectado corretamente.
    2. Nada executa que ele não declara. Um serviço não declarado é chamado de órfão.

    A CLI do goatbuilder calcula o que um site deveria executar a partir das próprias configurações do site, não de uma lista separada.

    Uma verificação que não tem nada para ler é marcada como UNOBSERVED. Essa é uma resposta honesta, não uma falha. Um FAIL ou um órfão faz o código de saída ser 3.

    A forma geral é `goatbuilder endstate <goatnet>`. O comando abaixo verifica um site de exemplo a partir de dados salvos, então não toca em nenhuma máquina.
tutor-l17-s1 = Verifique o site de exemplo
tutor-l18-title = Bônus: seja curioso
tutor-l18-text =
    Esta lição é opcional. Execute o comando abaixo e o tutor está concluído.

    A CLI do goatbuilder tem alguns pequenos recursos escondidos. Eles não estão na ajuda nem nestas lições. Alguns respondem a palavras que usuários antigos do Unix costumam digitar em um programa novo, só para ver o que acontece. A maioria das coisas que você tentar não vai fazer nada.

    Esta lição não diz o que são. Encontrá-los é o ponto.

    O comando abaixo lista todo erro que o goatbuilder pode explicar. É um bom lugar para começar a explorar.
tutor-l18-s1 = Liste todo erro que o goatbuilder pode explicar
tutor-heading = Lição { $n } de { $total } · { $title }
tutor-setup = Primeiro, execute isto uma vez neste terminal:
tutor-not-recording = Este terminal não está registrando suas execuções, então o tutor não pode marcar seus passos. Execute isto uma vez, depois execute os passos:
tutor-type = Faça estes passos no seu terminal ({ $met } de { $total } feitos):
tutor-rerun = Depois de cada passo, execute `goatbuilder tutor` de novo. Ele verifica o que você fez e marca o passo.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Bem-vindo ao GOATBUILDER Tutor
tutor-bar-progress = Progresso
tutor-bar-done = { $done } de { $total } lições feitas
tutor-after-step = Passo { $met } de { $total } feito. Próximo:
tutor-after-lesson = A lição { $n } está concluída. Execute `goatbuilder tutor` para começar a lição { $next }. Para ler de novo: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = A lição { $n } está concluída, e o tutor também. Execute `goatbuilder tutor` para ver que foi marcada como feita. Para ler de novo: `goatbuilder tutor --lesson { $n }`.

tutor-review = Você terminou esta lição, então esta página é só para leitura. Execute `goatbuilder tutor` para voltar a onde você está.
tutor-other-lessons = Para ler uma lição de novo: `goatbuilder tutor --lesson 1`. Para ver todas: `goatbuilder tutor --list`.
tutor-l12-title = Adicione uma máquina a um site em execução
tutor-l12-text =
    O comando goatbuilder `join-node` adiciona mais uma máquina a um site que já está em execução. Ele mostra um plano primeiro, com uma linha por fase. Cada fase tem um tipo:
    - tool: o goatbuilder executa um script.
    - ansible: o goatbuilder executa um playbook.
    - gate: uma verificação que deve passar antes da próxima fase.
    - manual: você faz isso você mesmo, e a execução pausa.

    O comando abaixo tem `--dry-run`. Ele lê este checkout e não muda nada. Abaixo da tabela, leia o que você faz manualmente.
tutor-l13-title = Conecte-se ao goatnet de outra organização
tutor-l13-text =
    O comando goatbuilder `federate` conecta seu goatnet a um goatnet que outra organização executa. Cada lado monta a sua metade de um acordo. Vocês trocam as metades, e cada lado combina as duas no mesmo acordo.

    Confirme a chave do outro lado de uma forma separada, como uma ligação em que você confia. Uma chave que chega dentro do pacote deles não prova nada.

    O comando abaixo tem `--dry-run`, então não muda nada. Uma execução real para na fase 4, porque você faz essa fase manualmente.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Esse comando não faz parte da lição { $n }. Execute `goatbuilder tutor` para ver onde você está.
tutor-l1-s2 = Pergunte ao doctor sobre uma goatnet
tutor-l1-text =
    A CLI goatbuilder compila, implanta e lança o software GOAT. Antes de usá-la, verifique sua configuração com dois comandos:
    - O comando goatbuilder `validate` verifica o arquivo de configuração, `ops/goatbuilder/goatbuilder.yaml`. Ele lê apenas este checkout e não precisa de rede.
    - O comando goatbuilder `doctor` verifica o que um deploy real precisa: suas ferramentas, seus logins, se este checkout corresponde à main e como o goatbuilder alcança cada goatnet. Uma goatnet é um ambiente GOAT.

    O comando goatbuilder `doctor` mostra quantas verificações passaram e depois cada problema com sua correção. Nesta lição, um problema é informação, não um erro. Leia as correções, mas você não precisa aplicá-las agora.

    A maioria das lições não precisa de login. As lições 2 a 5, 10, 11 e 16 usam uma cópia de prática. As lições 8, 9, 12, 13, 15, 17 e 18 apenas leem este checkout. A lição 6 precisa do `gh` e da mesh, a lição 7 precisa do `gh` e a lição 14 precisa da mesh. Nenhuma lição precisa do 1Password.
tutor-l14-text =
    A CLI goatbuilder pode se atualizar a partir da cópia que sua goatnet serve em `get.<site>.<zone>/goatbuilder/`.

    O comando goatbuilder `update --check` apenas lê. Ele compara sua versão com a versão servida e não muda nada. O código de saída 0 significa que você está atualizado. O código de saída 3 significa que existe uma versão mais nova.

    Uma atualização real, sem `--check`, verifica o checksum e a assinatura do download antes de substituir qualquer coisa. Depois troca o arquivo em um único passo.

    A CLI goatbuilder precisa saber qual goatnet consultar: passe `--site` com essa goatnet, ou defina `GOATBUILDER_UPDATE_URL`. Você precisa estar na mesh dessa goatnet. Se não estiver, `--check` diz que não consegue alcançar o servidor e mostra o endereço que tentou. Esse é um resultado correto para esta lição.
tutor-l15-s1 = Liste os testes que se aplicam a uma goatnet
tutor-l6-text =
    Um release move uma nova versão por três goatnets, em ordem: dev, depois preprod, depois prod.

    1. Alguém cria a tag da versão na main, e o CI a compila.
    2. dev: o goatbuilder a implanta e executa os testes de aceitação.
    3. preprod: o goatbuilder a implanta de novo, executa os testes de novo e registra os resultados.
    4. A versão então espera na preprod por um tempo definido. Nada passa para prod sozinho.
    5. prod: depois dessa espera, uma pessoa executa o release com `--from prod`. A CLI goatbuilder implanta em prod e verifica as três goatnets.

    O comando goatbuilder `promote status` mostra onde cada componente está. Um candidato é uma versão em teste. A forma geral de um release é `goatbuilder release <component> --tag <tag> --from dev`. Os comandos abaixo usam um componente deste checkout e uma tag de exemplo. Cada um tem `--dry-run`, então nada recebe tag nem é implantado.

    Os dry runs precisam do `gh` e da mesh. Se você não for admin do repositório, coloque `GOAT_RC_ROLE_CHECK=0` antes de cada comando `release`.
tutor-l9-s1 = Gere um erro real com uma goatnet que não existe
tutor-l9-text =
    Quando o goatbuilder recusa, ele mostra um código. Códigos que começam com `GB-` vêm do goatbuilder, `E_` da CLI goat e `X-` de outra ferramenta que o goatbuilder executou.

    O comando goatbuilder `explain` recebe um código, ou o texto de uma linha de erro, e diz o que aconteceu, por quê, como corrigir e onde ler mais.

    Pedir uma goatnet que este checkout não declara gera um erro real para explicar.
