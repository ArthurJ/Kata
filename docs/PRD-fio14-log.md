# PRD — Fio 14 `@log`: Telemetria via CSP

**Status:** Planejamento
**Data:** 2026-07-19
**Depende de:** Fio 11 ✅ (CSP, scheduler, fibers), Fio 4 ✅ (Result, enum payload)
**Não depende de:** `@parallel` (congelado)

## 1. Objetivo

Permitir que programas Kata emitam telemetria estruturada via canais CSP, sem
contaminar a assinatura matemática de funções e actions. Dois mecanismos
independentes:

1. **Diretiva `@log`** — anotação em definições de actions e funções nomeadas.
   O codegen injeta `kata_rt_log_publish` no wrapping (prólogo/epílogo). Mensagem
   é template compile-time com interpolação de variáveis do escopo.
2. **Action nativa `log!()`** — chamada explícita no corpo de actions. Dispara na
   execução da linha.

Ambos publicam em tópicos (canais nomeados). Políticas: `"drop"` (fire-and-forget
via Broadcast) ou `"block"` (fila bounded com ack, bloqueia até confirmação).

Configuração de tópico/policy/level default é herdada de fibers ancestrais via
snapshot no `kata_rt_spawn` (β).

## 2. Sintaxe

### 2.1. Diretiva `@log`

```
@log{msg: "processando {x}", level: LogLevel::Info, topic: "audit", policy: "block", when: "exit"}
action processar (Int) -> Int
  let x := 42
  let result := x * 2
  result
```

Campos da diretiva:

| Campo | Tipo | Default | Descrição |
|---|---|---|---|
| `msg` | `Text` | **obrigatório** | Template compile-time. `{expr}` interpola expressão do escopo. `{{` escapa `{` literal. Desugara para `format "..." (expr1, expr2, ...)` via `infer_format`. |
| `level` | `LogLevel` | `LogLevel::Info` | Variante do enum `LogLevel` do prelude. |
| `topic` | `Text` | herdado de fiber ancestral (ou `"default"` se nenhuma config) | Nome do canal onde publicar. |
| `policy` | `Text` | herdado de fiber ancestral (ou `"drop"`) | `"drop"` ou `"block"`. |
| `when` | `Text` | automático (ver §2.2) | `"enter"` = loga no prólogo. `"exit"` = loga no epílogo. |

### 2.2. Comportamento automático do `when`

Sem `when` explícito, o codegen decide:

- Se **todos** os placeholders `{expr}` referenciam apenas params da função →
  loga no **prólogo** (entrada).
- Se algum placeholder referencia variáveis do corpo → loga no **epílogo**
  (saída).
- Se há placeholders de ambos → loga no **epílogo** (saída), pois é o único ponto
  onde todas as variáveis existem.

`when: "enter"` força prólogo. `when: "exit"` força epílogo. Se `when: "enter"`
mas `msg` referencia variáveis do corpo que não existem no prólogo → erro de
compile-time (`UnknownDirective` ou mensagem dedicada).

### 2.3. Action nativa `log!()`

```
log!(LogLevel::Info, "mensagem dinâmica: {valor}", "audit", "drop")
```

Sintaxe posicional (action call existente: `Ident ! (tuple)`):

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `LogLevel` | Level da mensagem. |
| 1 | `Text` | Mensagem. Pode ser dinâmica (construída em runtime). |
| 2 | `Text` | Tópico. Default: herdado ou `"default"`. |
| 3 | `Text` | Policy. Default: herdado ou `"drop"`. |

Args 2 e 3 são opcionais (fallback para config herdada). O typeck aceita 2, 3,
ou 4 args.

### 2.4. Action nativa `log_recv!()`

```
log_recv!("audit")
```

Recebe a próxima mensagem de telemetria do tópico. Bloqueia (yield point via
`BlockedOnRecv`) até chegar mensagem. Retorna `Text` (a mensagem) ou `Unit`
se o canal fechou.

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `Text` | Tópico para consumir. |

### 2.5. Action nativa `log_config!()`

```
log_config!("audit", "block", LogLevel::Info)
```

Configura defaults de logging para o fiber atual e descendentes (herdado via
snapshot no `kata_rt_spawn`).

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `Text` | Tópico default. |
| 1 | `Text` | Policy default. |
| 2 | `LogLevel` | Level default. |

A config é armazenada num TLS `LOG_CONFIG: RefCell<Option<LogConfig>>`. No
`kata_rt_spawn`, o scheduler copia o `LOG_CONFIG` do fiber pai para o filho
(snapshot). Mudanças no pai após o spawn não propagam para filhos já spawnados.

### 2.6. Enum `LogLevel` no prelude

```
enum LogLevel
  Debug
  Info
  Warn
  Error
```

Adicionado a `stdlib/core.kata`. Fixo por ora; extensibilidade (interfaces,
herança de enum) fica para iteração futura.

## 3. Semântica

### 3.1. Pureza

`@log` não muda a assinatura. O codegen insere o efeito colateral
invisivelmente — na semântica da linguagem, a pureza não muda; no máximo a
resposta é adiada (com `policy: "block"`).

### 3.2. Template de interpolação

`msg: "processando {x}, resultado: {result}"` desugara em compile-time para:

```
format "processando {}, resultado: {}" (x, result)
```

Reusa `infer_format` de `format_synthesis.rs`. O parser de template:
- `{expr}` → placeholder. `expr` é parseado como expressão Kata, inferido
  contra o escopo no ponto de injeção.
- `{{` → `{` literal.
- `}}` → `}` literal.
- `{` sem `}` → erro de compile-time.
- Variável/expressão inexistente no escopo → erro de compile-time.

A extração de placeholders e construção da tupla de args acontece no typeck,
não no parser. O parser só vê `msg` como `Text` literal; o typeck interprete o
template, extrai as expressões, e chama `infer_format`.

### 3.3. Canais e tópicos

Tópicos são canais nomeados. O runtime mantém um registry de tópicos:
`HashMap<String, i64>` mapeando nome → handle de canal.

- **`"drop"`** → canal Broadcast (fire-and-forget). Reusa
  `kata_rt_broadcast_create` + `kata_rt_broadcast_send`. Não bloqueia.
- **`"block"`** → canal Queue bounded (capacidade 1) com ack. Reusa
  `kata_rt_channel_create` + `kata_rt_channel_send` (bloqueia se cheio,
  `YieldReason::BlockedOnSend`). O consumidor envia ack de volta via um canal
  de ack dedicado.

O registry de tópicos é criado sob demanda: primeira referência a `"audit"`
cria o canal; referências subsequentes reusam o mesmo handle.

### 3.4. `kata_rt_log_recv`

FFI interna do runtime. `log_recv!("audit")` desugara para chamada
`kata_rt_log_recv` que:

1. Resolve o tópico `"audit"` no registry.
2. Se Broadcast: obtém um receiver (`kata_rt_broadcast_receiver_create`) e
   faz `kata_rt_channel_recv` (bloqueia com `BlockedOnRecv`).
3. Se Queue: faz `kata_rt_channel_recv` diretamente.
4. Retorna `Text` (payload da mensagem) ou `Unit` se canal fechou.

O host (driver) também pode consumir via FFI externa `kata_rt_log_recv`.

## 4. Fases de implementação

### Fase 1: Parser — aceitar `@log` e `log!()`

- Adicionar `log` à lista de diretivas aceitas nos contextos de Action e Sig
  (função nomeada). Ver `pass0.rs` e `directives.rs` — 4 contextos (Sig,
  Action, Implements, Data); adicionar `log` em Sig e Action.
- `log!()` e `log_recv!()` e `log_config!()` já parseiam como `ActionCall`
  (sintaxe existente `Ident ! (tuple)`). Nenhuma mudança no parser para estas.
- Validar args da diretiva `@log`: `msg` (Text, obrigatório), `level`
  (opcional), `topic` (opcional), `policy` (opcional), `when` (opcional).

**Verificação:** `cargo check --workspace --all-targets`

### Fase 2: Resolution — aceitar `@log` sem erro

- `pass0.rs`: aceitar `@log` em Sig e Action. Não rejeitar como
  `UnknownDirective`.
- Validar que `msg` é `Text`, `policy` é `Text` (se presente), `when` é `Text`
  (se presente).

**Verificação:** `cargo check --workspace --all-targets`

### Fase 3: Prelude — adicionar `LogLevel`

- Adicionar `enum LogLevel Debug Info Warn Error` a `stdlib/core.kata`.
- Verificar que o enum é registrado no `enum_registry` e visível no typeck.

**Verificação:** `cargo test -p kata-resolution -- prelude`

### Fase 4: Runtime — FFIs de log

Implementar em `crates/kata-rt/src/`:

- `kata_rt_log_publish(topic: i64, level: i64, msg: i64, policy: i64) -> i64`
  - Resolve tópico no registry. Cria canal se não existe.
  - Se policy=drop: publica via Broadcast (fire-and-forget).
  - Se policy=block: publica via Queue bounded, bloqueia se cheio.
- `kata_rt_log_recv(topic: i64) -> i64`
  - Resolve tópico. Recebe mensagem. Bloqueia se vazio.
- `kata_rt_log_config(topic: i64, policy: i64, level: i64)`
  - Setta `LOG_CONFIG` TLS.
- TLS `LOG_CONFIG: RefCell<Option<LogConfig>>` com `LogConfig { topic, policy, level }`.
- `kata_rt_spawn` copia `LOG_CONFIG` do pai para o filho (snapshot).

Registrar FFIs em todos os sites (pitfall #31):
- `FfiSymbol::LogPublish`, `LogRecv`, `LogConfig` em `kata-core/ffi.rs`
- `symbol_name()`, `return_type()`, `from_name()`, `ffi_signature()` em
  `ffi_sigs.rs`
- `all_ffi_symbols()`, `declare_ffi_symbols`, `register_ffi_symbols` em
  `ffi_registry.rs`
- `builder.symbol()` em `lib.rs`

**Verificação:** `cargo check --workspace --all-targets`

### Fase 5: Codegen — `@log` wrapping

- No lowering da função/action anotada com `@log`:
  1. Extrair placeholders do template `msg` no typeck (não no codegen).
  2. Construir tupla de args (expressões do escopo).
  3. Sintetizar `format "template" (args)` via `infer_format`.
  4. Inserir `kata_rt_log_publish(topic, level, formatted_msg, policy)` no
     prólogo (se `when: "enter"`) ou epílogo (se `when: "exit"` ou automático).
  5. Se `topic`/`policy`/`level` não especificados na diretiva, usar config
     herdada do TLS `LOG_CONFIG` (lida em runtime, não compile-time).

- A injeção no epílogo precisa que as variáveis do corpo existam. O codegen
  insere a chamada de log imediatamente antes do `return` da função. Se a
  função tem múltiplos pontos de saída (match arms, early return), inserir
  em todos.

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 6: Codegen — `log!()` e `log_recv!()` e `log_config!()`

- `log!()` desugara para `kata_rt_log_publish(topic, level, msg, policy)` no
  ponto da chamada. Args opcionais (topic, policy) recebem fallback da config
  herdada.
- `log_recv!()` desugara para `kata_rt_log_recv(topic)`.
- `log_config!()` desugara para `kata_rt_log_config(topic, policy, level)`.
- Todas as três são interceptadas no typeck (como `format`, `map`, etc.),
  não no DispatchTable.

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 7: Testes E2E

Testes em `crates/kata-driver/tests/`, nomeados por responsabilidade:

- `log_directive_prologo.kata` — `@log` com só params, loga na entrada.
- `log_directive_epilogo.kata` — `@log` com vars do corpo, loga na saída.
- `log_directive_when_enter.kata` — `when: "enter"` explícito.
- `log_directive_when_exit.kata` — `when: "exit"` explícito.
- `log_action_basico.kata` — `log!()` no corpo de action.
- `log_action_com_topic.kata` — `log!()` com tópico explícito.
- `log_policy_drop.kata` — policy drop, não bloqueia.
- `log_policy_block.kata` — policy block, bloqueia até ack.
- `log_config_heranca.kata` — `log_config!()` em fiber ancestral, filho herda.
- `log_recv_consomo.kata` — `log_recv!()` consome telemetria.
- `log_template_interpolacao.kata` — `{expr}` em msg, interpola corretamente.
- `log_template_escape.kata` — `{{` produz `{` literal.
- `log_level_enum.kata` — `LogLevel::Warn` etc. validado em compile-time.
- `log_diretiva_e_action_independentes.kata` — ambos disparam independentemente.

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed.

### Fase 8: Sintaxe nomeada `log!{...}` (opcional, última)

- Estender parser de `ActionCall` para aceitar `{...}` (dict nomeado) além
  de `(...)` (tupla posicional).
- `log!{level: LogLevel::Info, msg: "..."}` parseia como dict nomeado.
- O typeck despacha: se recebe dict, mapeia chaves para posições; se recebe
  tupla, usa posicional.

**Verificação:** `cargo test --workspace --no-fail-fast`

## 5. Fora do escopo

- Extensibilidade de `LogLevel` (interfaces, herança de enum) — iteração futura.
- `@log` em métodos de `implements` — só actions e funções nomeadas.
- Log condicional (só loga se level >= threshold) — pode ser runtime check no
  consumidor.
- Múltiplos `@log` na mesma definição — não suportado na primeira iteração.
- `log!()` com sintaxe nomeada — Fase 8, última.

## 6. DoDs (Definitions of Done)

1. `@log{msg: "..."}` em action loga na chamada (wrapping), sem mudar assinatura.
2. `@log{msg: "..."}` em função nomeada loga na chamada, sem mudar assinatura.
3. `log!()` no corpo de action loga na execução da linha.
4. `@log` e `log!()` são independentes — ambos podem coexistir na mesma action.
5. Template `{expr}` interpola expressões do escopo. `{{` escapa `{` literal.
6. `policy: "drop"` publica via Broadcast, não bloqueia.
7. `policy: "block"` publica via Queue bounded, bloqueia até ack.
8. `log_config!()` seta defaults no fiber atual; filhos herdam via snapshot.
9. `log_recv!()` consome telemetria de um tópico, bloqueia até chegar.
10. `LogLevel` é enum do prelude, validado em compile-time.
11. `cargo test --workspace --no-fail-fast` passa sem regressão (899+ testes
    novos).
12. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 7. Arquitetura — componentes afetados

```
stdlib/core.kata                           # enum LogLevel (novo)
crates/kata-parser/src/directives.rs        # @log aceita em Sig, Action
crates/kata-parser/src/expressions.rs      # ActionCall aceita {...} (Fase 8)
crates/kata-resolution/src/pass0.rs        # @log aceita, valida args
crates/kata-resolution/src/lib.rs          # @log no contexto de Sig, Action
crates/kata-inference/src/infer/            # typeck aceita @log, log!()
crates/kata-inference/src/infer/log_synthesis.rs  # template parse + infer_format (novo)
crates/kata-codegen/src/lowering/          # injeta kata_rt_log_publish
crates/kata-codegen/src/ffi_sigs.rs        # ty_to_clif, ffi_signature
crates/kata-codegen/src/ffi_registry.rs    # all_ffi_symbols, declare/register
crates/kata-core/src/ffi.rs                # FfiSymbol::LogPublish, LogRecv, LogConfig
crates/kata-rt/src/log.rs                  # FFIs de log (novo)
crates/kata-rt/src/scheduler.rs            # spawn copia LOG_CONFIG (snapshot)
crates/kata-rt/src/lib.rs                  # re-exports, builder.symbol
crates/kata-driver/tests/                 # testes E2E
```

## 8. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `@log` em actions e funções nomeadas (não em implements) | Consistente com `@test`; preserves pureza invariant. Implements methods são resolvable via DispatchTable, adicionar wrapping de log lá é complexidade desnecessária. |
| D2 | `@log` wrapping (chamada), `log!()` expressão (linha) | Dois mecanismos independentes: automático (diretiva) e explícito (action). |
| D3 | Canais existentes: Broadcast (drop), Queue bounded (block) | Reusa maquinaria de CSP. Sem `YieldReason` novo. |
| D4 | `when` automático + override opcional | Default sensato (params → prólogo, vars do corpo → epílogo) sem limitar flexibilidade. |
| D5 | `log!()` posicional no MVP, nomeado na Fase 8 | Sintaxe nomeada exige mudança no parser de ActionCall; posicional funciona hoje. |
| D6 | Tópicos = canais nomeados via registry | Isolamento por tópico. Registry cria sob demanda. |
| D7 | Herança (β) snapshot no spawn | Alinhado com padrão existente (spawn já copia dados do pai). Simples, sem walk na árvore. |
| D8 | `LogLevel` enum fixo no prelude | Validado em compile-time. Extensibilidade fica para depois. |
| D9 | Template desugara para `format` | Reusa `infer_format` e toda a infra de `convert_to_text`. |
| D10 | `log_recv!()` — programa Kata e host consomem | D2=(iii). Telemetria é consumível de dentro e fora do programa. |
| D11 | `log_config!()` é action nativa (não diretiva) | Setta config em runtime, dinamicamente. Diretiva seria compile-time. |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| `policy: "block"` com Queue bounded pode deadlockar se nenhum consumidor existe | Fallback: timeout ou detecção de deadlock pelo scheduler (já existe `DEADLOCK_SENTINEL`). |
| Múltiplos pontos de saída na função (match arms) complicam injeção no epílogo | Codegen insere antes de cada `return`/saída. Se complexidade crescer, restringir a funções de corpo único. |
| Template `{expr}` com expressões complexas pode gerar código grande | Aceitar expressões simples (Ident, FieldAccess, Apply). Se crescer, limitar. |
| `log!()` como action nativa interceptada no typeck — não passa pelo DispatchTable | Segue padrão de `format`, `map`, `filter`, `len` — todos interceptados por nome. |

## 10. Atualização da documentação

Ao concluir:
- `docs/PRD-fio14-log.md` — este arquivo (status → concluído)
- `docs/ROADMAP.md` Fio 14 — marcar `@log` ✅
- `docs/PRD-fio14.md` — atualizar status: `@log` concluído
- `docs/Kata-lang-manual.md` — confirmar compatibilidade com sintaxe implementada (manual é aspiracional; se implementação divergir, solicitar permissão)
- `docs/sintaxe-mapa.md` — confirmar `@log` listada (já está, linha 427)