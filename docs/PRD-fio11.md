# PRD: Fio 11 — CSP, Concorrência, Paralelismo

## Objetivo

Implementar concorrência CSP (Communicating Sequential Processes) end-to-end:
canais (rendezvous, buffered, broadcast) com ends separados (sender/receiver),
`fork!` (spawn de fiber com args), `select` (multiplexação com timeout),
operadores `!>` (envio) e `<!` (recebimento), yield points cooperativos no
codegen, e `@parallel` (paralelismo via multiprocess com fork+IPC).

O Pré-11 entregou a árvore hierárquica de arenas, `EscapeTarget`, destruição
bottom-up, e ARC pass emitido. O Fio 11 constrói sobre essa infraestrutura para
fazer fibers comunicarem-se de forma segura.

## Três camadas

| Camada | Mecanismo | Isolamento | Quando |
|---|---|---|---|
| **Concorrência** | Fibers + yield cooperativo + canais | Memória compartilhada (árvore de arenas) | Fases 1-6 |
| **Preempção cooperativa** | Yield points no codegen (back-edge checks) | Mesma thread, cede periodicamente | Fase 7 |
| **Paralelismo** | `@parallel`, fork+IPC, processo OS separado | Memória isolada (processo OS) | Fase 9 |

**Multithreading (M:N scheduler) é post-1.0.** A estrutura do scheduler
(`thread_local!` como hoje) não impede a adição futura de worker pool — o run
loop, yield, e structured concurrency são independentes do número de threads.
Se workloads reais demonstrarem que yield points + `@parallel` não chegam,
M:N é adição incremental (não refactor).

## Depende de

- Pré-11 ✅ (árvore de arenas, `EscapeTarget`, destruição bottom-up)
- Fio 3 ✅ (Actions, `return`, `;`, arena per-fiber, scheduler básico)
- Fio 9 ✅ (closures com captura, `CaptureBox`, `Arc<T>`, escape analysis)

## Decisões de design

### A. Scheduler: single-threaded, estrutura estável

O scheduler permanece single-threaded com `thread_local!` (como hoje). Sem
`Arc<Mutex<>>`, sem worker pool, sem work-stealing. O run loop muda para
suportar yield e structured concurrency, mas a estrutura de contenção não
muda — N=1, sem locks.

Se M:N vier post-1.0, a migração é: `thread_local!` → `Arc<Mutex<>>`, adicionar
worker pool. O run loop, yield, blocked/wake, e structured concurrency são
independentes do container — só a concorrência de acesso muda.

### B. Yield: `wasmtime-fiber::Suspend` com `YieldReason`

Sem yield, CSP não funciona — `<!` em canal vazio trava a thread inteira.
`wasmtime-fiber` já está no projeto e suporta yield nativamente. O `Yield`
muda de `()` para `YieldReason`:

```rust
pub enum YieldReason {
    /// Esperando recebimento em canal.
    WaitingOnChannel(i64),      // receiver handle
    /// Esperando select.
    WaitingOnSelect(Vec<i64>),  // receiver handles
    /// Yield point cooperativo (Decisão G) — fiber cede voluntariamente.
    Cooperative,
    /// Fiber completou.
    Done,
}
```

**Mecânica:** o fiber executa código JIT. Quando faz `<!` em canal vazio,
chama `kata_rt_yield()` (FFI). O `kata_rt_yield` acessa o `Suspend` via TLS,
chama `suspend(YieldReason::WaitingOnChannel(handle))`, e o controle volta ao
scheduler. O scheduler marca o fiber como blocked, pega o próximo da
run_queue, e faz `resume(spawn_args)` nele. Quando o canal recebe dados, o
fiber é acordado via `pending_wakes` e volta para a run_queue.

Para `Cooperative` (yield point), o scheduler simplesmente coloca o fiber de
volta na `run_queue` e pega o próximo — não bloqueia.

**TLS para yield:** o código JIT não recebe `&Scheduler`. TLS mantém
`current_fiber_id` e o `Suspend` handle. `kata_rt_yield()` acessa ambos via
TLS.

### C. Deadlock: detecção trivial no run loop

A árvore hierárquica de fibers **não previne deadlock completamente**.
Siblings podem comunicar diretamente se o parent passar os dois ends a fibers
diferentes — espera circular entre siblings é possível. O LCA (parent) garante
**lifetime da memória**, não **liveness**.

**Detecção:** trivial e O(1) por iteração do run loop:

```
if run_queue.is_empty() && !blocked.is_empty() && !has_pending_timers() {
    abort("deadlock: N fibers bloqueados sem progresso possível")
}
```

Só verifica quando a run_queue esvazia. Custo zero em runtime normal.

### D. Select para envio — adiado

`select` para recebimento (`<!`) está no escopo. `select` para envio
(escolher qual canal enviar) é útil em roteamento, mas pode não ser compatível
com as três topologias. Discutir após a implementação de `@parallel`, com
experiência real dos padrões de uso.

### E. Structured concurrency: Action espera forks completarem

Uma Action **não termina** enquanto seus forks existirem. O return value é
computado quando o body completa, mas a entrega ao caller é postergada até
todos os fibers filhos terminarem.

Isto é automático — o programador não faz nada explícito. O `try_destroy` do
Pré-11 já tem a estrutura: `completed && children.is_empty()`. O que falta é
o run loop continuar processando filhos após o parent completar.

**O que isto resolve:**
- Parent não abandona channel ends (sem órfãos de canal)
- Return value só é entregue quando o parent e todos os filhos terminaram
- Lifecycle determinístico (bate com o modelo de árvore de arenas)

**O que isto restringe:**
- Fire-and-forget puro não existe. Para um worker long-lived, a Action que o
  forkou fica viva até ele terminar. A topologia natural é: Action raiz fork
  um supervisor que gerencia workers — a Action raiz só termina quando o
  supervisor termina.

### F. Broadcast: só mensagens futuras, latest only

Cada receiver nasce com `last_seen_version = current_version`. Late
subscribers não recebem histórico. Se o receiver é lento e perde mensagens
intermediárias, vê **a última** quando desbloqueia. Se precisa de cada
mensagem, usa `channel!` ou `queue!(N)`.

### G. Yield points: back-edge checks no codegen

Sem yield points, uma fiber em `loop` pesado nunca cede — head-of-line
blocking. Com M:N isto seria resolvido por preempção entre threads, mas sem
multithread, precisamos de preempção cooperativa.

**Mecanismo:** o codegen insere uma chamada para `kata_rt_yield_check()` no
header de cada loop (`Loop` e `ForIn`). A função decrementa um contador
per-fiber em TLS; a cada N iterações (ex: 1000), pergunta ao scheduler se há
outro fiber pronto. Se sim, suspende via `Suspend` com
`YieldReason::Cooperative`. Se não, continua.

```rust
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield_check() {
    YIELD_COUNTER.with(|c| {
        let mut counter = c.borrow_mut();
        *counter -= 1;
        if *counter <= 0 {
            *counter = YIELD_INTERVAL;  // reset
            SCHEDULER.with(|s| {
                if let Some(sched) = s.borrow_mut().as_mut() {
                    if sched.has_ready_fiber() {
                        // Suspende — volta ao scheduler
                        // O Suspend é acessado via TLS
                        let suspend = CURRENT_SUSPEND.with(|s| s.borrow().clone());
                        suspend.suspend(YieldReason::Cooperative);
                    }
                }
            });
        }
    });
}
```

**Custo no hot path:** decrement + branch — duas instruções. O slowpath (a
cada N iterações) é uma verificação de scheduler + possível suspend.

**O que muda:** apenas codegen (inserção da call no lowering de `Loop` e
`ForIn`) e runtime (`kata_rt_yield_check` + `YIELD_COUNTER` TLS). Parser,
AST, TAST, typeck — nenhum impacto. Yield point é invisível ao type system.

**Onde inserir:**

| Constructo | Tem back-edge? | Inserir yield point? |
|---|---|---|
| `Loop { body }` | Sim (jump para header) | **Sim** |
| `ForIn { body }` | Sim (jump para próximo elemento) | **Sim** |
| Recursão de função pura | Sim (call) | **Não** — função pura não roda em fiber |
| Recursão de Action | Proibida (Fio 3) | N/A |

## Problema

O scheduler atual é single-threaded. Fibers são criadas e executadas
sequencialmente — `run()` executa um fiber por vez, sem preempção. `yield_()`
existe mas é stub. Não há canais, não há `fork!`, não há `select`. O
`Effect::Spawn` e `Effect::ChannelOp` existem no enum mas nunca são produzidos
pelo typeck.

O codegen não sabe como lowerar `!>`, `<!`, `channel!`, `queue!`, `broadcast!`,
`fork!`, ou `select`. O parser não reconhece esses tokens. O runtime não tem
`kata_rt_channel_*`, `kata_rt_fork`, `kata_rt_select`.

## Modelo

### Três topologias de canal

| Canal | Sintaxe de criação | Retorno | Semântica |
|---|---|---|---|
| `channel!` | `let (tx, rx) := channel!()` | `(Sender::T, Receiver::T)` | Rendezvous — `!>` bloqueia até `<!` sincronizar |
| `queue!(N)` | `let (tx, rx) := queue!(8)` | `(Sender::T, Receiver::T)` | Buffered — `!>` bloqueia só se buffer cheio |
| `broadcast!` | `let (tx, rxf) := broadcast!()` | `(Sender::T, ReceiverFactory::T)` | Pub-sub — `!>` não bloqueia, `rxf!()` cria novo `Receiver::T` |

**Criação retorna tupla sender/receiver.** O sender (`tx`) só pode fazer `!>`.
O receiver (`rx`) só pode fazer `<!`. Para broadcast, uma **fábrica de
receivers** (`rxf`) permite criar múltiplos receivers independentes — cada um
vê apenas mensagens futuras (Decisão F).

```kata
let (tx, rxf) := broadcast!()
let rx1 := rxf!()
let rx2 := rxf!()
fork!(consumidor!(rx1))
fork!(consumidor!(rx2))
tx !> 42           -- ambos rx1 e rx2 recebem 42
```

### Operadores de canal

| Operador | Direção | Sintaxe | Tipos | Semântica |
|---|---|---|---|---|
| `!>` | envio | `tx !> valor` | `tx: Sender::T`, `valor: T` | Envia `valor` por `tx`. Bloqueia conforme topologia. |
| `<!` | recebimento | `rx <! nome` | `rx: Receiver::T` | Recebe valor de `rx`, binding em `nome: T`. Bloqueia até haver mensagem. |

**Domínio:** `!>` e `<!` são exclusivos de Actions (efeito `ChannelOp`). Não
existem em funções puras.

### `fork!`

```kata
fork!(minha_action, (arg1, arg2))
```

Submete uma Action ao scheduler como novo fiber, passando uma tupla de
argumentos. Retorna `Unit`. O novo fiber herda a arena do caller como
`caller_arena` (Pré-11 já suporta isto via árvore). A Action recebe os args
como tupla normal.

A Action submetida comunica-se com outros fibers exclusivamente via canais.
Actions forkadas não fazem `return` para ninguém — só comunicam via canais.

Pela Decisão E (structured concurrency), a Action que faz `fork!` não termina
até todos os forks completarem.

### `select` com `timeout`

```kata
action exemplo
    let (tx1, rx1) := channel!()
    let (tx2, rx2) := channel!()
    fork!(produtor!(tx1))
    fork!(produtor2!(tx2))
    select
        rx1 <! msg: echo!(msg)
        rx2 <! item: echo!(item)
        timeout 5000: echo!("timeout")
```

`select` multiplexa operações de recebimento de canais. A Action cede ao
scheduler e é acordada quando um caso se concretiza. `timeout N` (ms) é
válvula de escape.

### `@parallel`

```kata
@parallel
action cpu_intensivo
    ...
```

Força a Action a executar num **processo OS separado** via fork + IPC. O
scheduler de fibers não gerencia isto — é uma ponte diferente. O resultado
volta via IPC channel.

## Sintaxe — mudanças no lexer

### Novos tokens

| Token | Sintaxe | Como lexar |
|---|---|---|
| `SendArrow` | `!>` | `!` seguido de `>` — verificar peek após consumir `!` |
| `RecvArrow` | `<!` | `<` seguido de `!` — `<` hoje vira `Ident("<")`; precisa de lookahead |

**Mudança no dispatch.rs:**

```rust
'!' => {
    lex.advance();
    if lex.ch == Some('>') {
        lex.advance();
        Token::SendArrow
    } else {
        Token::Bang
    }
}
'<' => {
    // Hoje: '<' vira Ident("<") via lex_ident.
    // Agora: '<' seguido de '!' → RecvArrow; senão lex_ident.
    if lex.peek_n(1) == Some('!') {
        lex.advance(); // consumir <
        lex.advance(); // consumir !
        Token::RecvArrow
    } else {
        return lex_ident(lex, &start);
    }
}
```

**Atenção:** `<` como operador de comparação (`> x 0`, `< y 5`) é `Ident("<")`
via notação prefixa. `<!` só é distinto quando `<` é seguido imediatamente por
`!`. O lookahead de 1 char resolve sem ambiguidade — `<!` não pode ser `< !`
(pois `!` como sufixo de Action vem após identificador+args, não após `<`).

### Nova palavra-chave: `select`

`select` é palavra-chave (`Token::Select`). O parser a reconhece como
construção de fluxo dentro de Actions, similar a `match`.

### Nova palavra-chave: `timeout`

`timeout` é palavra-chave (`Token::Timeout`). Só aparece dentro de `select`.

## Sintaxe — mudanças no parser

### `!>` e `<!` como operadores infixos

`!>` e `<!` têm precedência igual a `|` e `|>` — mesma camada de operadores
infixos baixos. O parser os trata no loop de pós-aplicação em `parse_expr`:

```rust
Token::SendArrow => {
    parser.advance();
    let rhs = parse_apply(parser)?;
    lhs = Expr::ChannelSend { channel: Box::new(lhs), value: Box::new(rhs) };
}
Token::RecvArrow => {
    parser.advance();
    // `<!` exige um Ident como destino (binding)
    let name = expect_ident(parser)?;
    lhs = Expr::ChannelRecv { channel: Box::new(lhs), bind_name: name };
}
```

### `select` como statement de Action

`select` é parseado como expressão dentro de Actions (produz `Expr::Select`).
Tem a mesma estrutura de `match` — braços indentados com `pattern: body`:

```kata
select
    rx <! msg: processa!(msg)
    rx2 <! item: handle!(item)
    timeout 5000: echo!("timeout")
```

### `channel!`, `queue!`, `broadcast!` como ActionCalls

O parser já trata `nome!(args)` como `Expr::ActionCall`. `channel!()`,
`queue!(8)`, `broadcast!()` são ActionCalls builtin — o typeck intercepta
(não despacha para DispatchTable). Igual a `panic!` e `assert!`.

### `fork!` como ActionCall builtin

`fork!(minha_action, (arg1, arg2))` é ActionCall builtin. O typeck intercepta
— o primeiro argumento é uma referência a Action (sem `!`), o segundo é a
tupla de argumentos.

## AST — novos nós

```rust
// Em Expr:
/// `tx !> valor` — envio por canal.
ChannelSend {
    channel: Box<Spanned<Expr>>,
    value: Box<Spanned<Expr>>,
},

/// `rx <! nome` — recebimento de canal.
ChannelRecv {
    channel: Box<Spanned<Expr>>,
    bind_name: String,
},

/// `select` com braços de canal e timeout.
Select {
    arms: Vec<SelectArm>,
    timeout_ms: Option<Spanned<Expr>>,
    timeout_body: Option<Spanned<Expr>>,
},
```

```rust
/// Um braço de `select`: `rx <! nome: body`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectArm {
    /// Receiver de onde receber (expressão que avalia para Receiver::T).
    pub channel: Spanned<Expr>,
    /// Nome do binding para o valor recebido.
    pub bind_name: String,
    /// Corpo do braço.
    pub body: Spanned<Expr>,
}
```

## TAST — novos nós

```rust
// Em TypedExprKind:
/// `tx !> valor` — envio por canal (effect = ChannelOp).
ChannelSend {
    channel: Box<Spanned<TypedExpr>>,
    value: Box<Spanned<TypedExpr>>,
},

/// `rx <! nome` — recebimento de canal (effect = ChannelOp).
ChannelRecv {
    channel: Box<Spanned<TypedExpr>>,
    /// Tipo do valor recebido (inferido do tipo do canal).
    recv_ty: Ty,
    bind_name: String,
},

/// `select` com braços.
Select {
    arms: Vec<TypedSelectArm>,
    timeout_ms: Option<Box<Spanned<TypedExpr>>>,
    timeout_body: Option<Box<Spanned<TypedExpr>>>,
},
```

```rust
pub struct TypedSelectArm {
    pub channel: Spanned<TypedExpr>,
    pub recv_ty: Ty,
    pub bind_name: String,
    pub body: Spanned<TypedExpr>,
}
```

## Sistema de tipos

### Tipos de canal — sender e receiver

Canais são tipos intrínsecos do compilador (não `data` opacos com `@ffi`).
O typeck conhece os tipos diretamente. Seguindo o padrão `Nome::T` da
linguagem:

```rust
// Em crates/kata-core/src/ty.rs:
pub enum Ty {
    // ... variantes existentes ...
    /// Sender de canal — `Sender::T`. Pode fazer `!>`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Sender(Box<Ty>),
    /// Receiver de canal — `Receiver::T`. Pode fazer `<!`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Receiver(Box<Ty>),
    /// Fábrica de receivers para broadcast — `ReceiverFactory::T`.
    /// Chamada como action produz `Receiver::T`.
    ReceiverFactory(Box<Ty>),
}
```

**Por que sender/receiver separados:** direcionalidade é enforced pelo tipo.
`!>` só aceita `Sender::T`. `<!` só aceita `Receiver::T`. Isto previne erro
comum de tentar receber de um sender ou enviar por um receiver.

**Por que intrínseco e não `data`:** canais têm semântica de runtime
específica (rendezvous, buffer, pub-sub) que não pode ser expressa via `@ffi`.
O codegen precisa saber que é um end de canal para emitir a FFI correta. O
runtime sabe a topologia pelo handle — o tipo `Sender::T` não distingue
rendezvous de queue de broadcast; a FFI despacha pelo handle.

**Type inference:** `channel!()` infere `T` do primeiro `!>` ou `<!` usado
nos ends. Se o canal é usado antes de qualquer operação, o tipo é
`Sender::Unit` / `Receiver::Unit` (erro se usado com valor de outro tipo
depois).

### `fork!` — tipo

```kata
fork!(minha_action, (arg1, arg2))
```

- Primeiro argumento: referência a Action (identificador sem `!`).
- Segundo argumento: tupla de argumentos para a Action.
- Retorno: `Unit`.

O typeck verifica que o primeiro argumento é nome de Action declarada e que
a tupla de argumentos tem os tipos esperados pela Action.

### `Effect` — exercitando variantes existentes

```rust
// Effect::Spawn — fork!()
// Effect::ChannelOp — !>, <!, channel!, queue!, broadcast!, select
```

O typeck marca `Effect::ChannelOp` em toda expressão que usa `!>`, `<!`, ou
`select`. Marca `Effect::Spawn` em `fork!`. O effect system propaga
(`IO | ChannelOp = IO`, pois ChannelOp subsume IO).

### Escape analysis para canais — LCA

Valores enviados por `!>` escapam para outro fiber. O `EscapeTarget` do
Pré-11 precisa saber o destino — e o destino é o **LCA (Lowest Common
Ancestor)** entre o fiber que envia e o fiber que recebe, não
necessariamente a raiz.

Pré-Fio 11, os únicos escape points eram retorno e closure (LCA = caller
direto ou raiz). Agora:

- `canal !> valor` → `valor` escapa para o fiber que fará `<!`. O LCA é
  determinado em compile-time analisando a árvore de fibers:
  - Se o canal foi criado no mesmo escopo que `fork!`, o LCA é o fiber que
    criou o canal (pai comum dos fibers que compartilham o canal).
  - Se o canal é passado como argumento para `fork!`, o LCA é o fiber que
    passou o canal (o pai que criou ambos os fibers).
  - Se o canal circula por múltiplos níveis, o LCA é o ancestral comum mais
    próximo dos fibers que têm acesso ao canal.

**Implementação:** o typeck rastreia a "profundidade" do canal na árvore de
fibers. Quando um valor é enviado por `!>`, o `EscapeTarget` é
`Ancestor(profundidade_do_lca)`. Se o LCA não pode ser determinado
estaticamente (canal passado por múltiplas funções), fallback para
`Ancestor(0)` (raiz) — conservador mas correto.

**O ARC pass (Pré-11) já emite `incref`/`decref`** — canais usam `Arc<T>`
para valores que cruzam fibers, garantindo lifetime correto. A arena do LCA
sobrevive enquanto os fibers que a referenciam estiverem vivos (destruição
bottom-up do Pré-11).

### Channel no TypeEnv e prelude

Canais não são declarados no prelude. São tipos intrínsecos do compilador
(como `Ty::Sum` e `Ty::Struct`). O typeck conhece `Sender::T`, `Receiver::T`,
`ReceiverFactory::T` diretamente.

## Runtime

### Canais

```rust
// crates/kata-rt/src/channel.rs

/// Handle de canal — opaco ao codegen (i64).
/// O runtime sabe a topologia pelo handle.

/// Canal rendezvous — sender bloqueia até receptor sincronizar.
struct ChannelInner {
    slot: Mutex<Option<i64>>,
    sender_ready: Condvar,
    receiver_ready: Condvar,
}

/// Fila bufferizada — bloqueia se buffer cheio.
struct QueueInner {
    buffer: Mutex<VecDeque<i64>>,
    capacity: usize,
    not_full: Condvar,
    not_empty: Condvar,
}

/// Broadcast pub-sub — fire-and-forget, latest only (Decisão F).
struct BroadcastInner {
    value: Mutex<Option<i64>>,    // última mensagem
    version: Mutex<u64>,          // incrementa a cada send
    new_msg: Condvar,             // acorda receivers bloqueados
}
```

**Broadcast — mecânica por receiver:**

Cada receiver mantém seu próprio `last_seen_version` (inicializado =
`current_version` na criação). Ao fazer `<!`:

```
if global_version > last_seen_version:
    nome = value                       -- recebe imediatamente
    last_seen_version = global_version
else:
    block on condvar                   -- espera próximo send
```

`tx !> valor`: `value = Some(valor)`, `version += 1`, `notify_all()`.
Múltiplos receivers compartilham o mesmo `value`/`version`. O `value` é um
`i64` (handle/ponteiro para `Arc<T>`) — múltiplos readers é seguro via Arc.

### FFI functions

```rust
// C-ABI para o codegen:

// ── Criação de canais ──────────────────────────────────
/// Cria canal rendezvous. Retorna (sender_handle, receiver_handle)
/// empacotado como i64 (high 32 bits = sender, low 32 = receiver).
kata_rt_channel_create() -> i64
/// Cria fila bufferizada com capacidade N. Retorna (sender, receiver).
kata_rt_queue_create(capacity: i64) -> i64
/// Cria broadcast. Retorna (sender, receiver_factory).
kata_rt_broadcast_create() -> i64

/// Cria um novo receiver a partir de uma receiver_factory de broadcast.
kata_rt_broadcast_receiver_create(factory_handle: i64) -> i64

// ── Envio (operador !>) ────────────────────────────────
/// Envia valor por sender. O runtime despacha pela topologia do handle.
kata_rt_channel_send(handle: i64, value: i64)

// ── Recebimento (operador <!) ──────────────────────────
/// Recebe valor por receiver. Bloqueia conforme topologia.
kata_rt_channel_recv(handle: i64) -> i64

// ── Select ────────────────────────────────────────────
/// Tenta receber de qualquer receiver na lista. Retorna
/// (índice_do_canal, valor) empacotado como i64.
/// Bloqueia até qualquer canal ter dado ou timeout.
kata_rt_select(handles: *const i64, n_handles: i64, timeout_ms: i64) -> i64

// ── Fork ───────────────────────────────────────────────
/// Já existe: kata_rt_spawn(fn_ptr, caller_arena, args_ptr) -> FiberId
/// Fio 11: spawn registra fiber na árvore (Pré-11 já faz isto).
/// fork!(action, args) passa args_ptr = ponteiro para tupla de args.

// ── Scheduler ─────────────────────────────────────────
/// Scheduler single-threaded. Inicializa com arena raiz.
kata_rt_scheduler_init() -> i64  // já existe, estende
/// Run loop — processa fibers até completar (structured concurrency).
kata_rt_run() -> i64             // já existe, estende
/// Yield do fiber atual (bloqueia em canal).
kata_rt_yield()                   // já existe, implementa de verdade

// ── Yield points (Decisão G) ──────────────────────────
/// Check de yield point — inserido pelo codegen em back-edges de loop.
/// Decrementa contador TLS; a cada N iterações, pergunta ao scheduler
/// se há fiber pronto. Se sim, suspende com YieldReason::Cooperative.
kata_rt_yield_check()
```

### Scheduler

O scheduler permanece single-threaded (Decisão A). `thread_local!` como hoje.

```rust
pub struct Scheduler {
    run_queue: VecDeque<FiberId>,       // pronto
    blocked: HashMap<FiberId, BlockReason>, // esperando canal/timer
    pending_wakes: HashSet<FiberId>,    // semântica unpark
    timers: TimerQueue,                 // timeouts de select
    current_fiber: Option<FiberId>,     // fiber em execução
    fibers: HashMap<FiberId, FiberEntry>, // árvore (herdado do Pré-11)
    next_id: u64,                       // herdado
    root_arena: i64,                    // herdado do Pré-11
}
```

### Run loop com structured concurrency, yield points, e deadlock detection

```rust
fn run(&mut self) -> i64 {
    loop {
        // 1. Tentar executar próximo fiber pronto
        if let Some(fiber_id) = self.run_queue.pop_front() {
            let result = self.resume_fiber(fiber_id);
            // Se completou, try_destroy (só destrói se children.is_empty)
            // Se yieldou (Cooperative), volta para run_queue
            // Se yieldou (WaitingOnChannel/Select), vai para blocked
            continue;
        }

        // 2. run_queue vazia — verificar se há blocked
        if !self.blocked.is_empty() {
            if self.has_pending_timers() {
                // Há timers — sleep até o próximo timer expirar
                self.sleep_until_next_timer();
                self.expire_timers();  // move fibers acordados para run_queue
                continue;
            } else {
                // Sem timers, sem ready, blocked não vazio = deadlock
                let n = self.blocked.len();
                panic!("deadlock: {n} fibers bloqueados sem progresso");
            }
        }

        // 3. run_queue vazia, blocked vazia — todos terminaram
        // root_arena é destruída pelo try_destroy do fiber raiz
        return self.root_result;
    }
}
```

**Tratamento do resultado de `resume()`:**

```rust
match self.fibers[&fiber_id].fiber.resume(spawn_args) {
    Ok(result) => {
        // Fiber completou
        entry.completed = true;
        self.try_destroy(fiber_id);
        // Se children não vazio, fica zombie (structured concurrency)
    }
    Err(YieldReason::Cooperative) => {
        // Yield point — volta para a fila
        self.run_queue.push_back(fiber_id);
    }
    Err(YieldReason::WaitingOnChannel(handle)) => {
        // Bloqueia em canal
        self.blocked.insert(fiber_id, BlockReason::WaitingOnChannel(handle));
    }
    Err(YieldReason::WaitingOnSelect(handles)) => {
        // Bloqueia em select
        self.blocked.insert(fiber_id, BlockReason::WaitingOnSelect(handles));
    }
}
```

**Structured concurrency (Decisão E):** quando um fiber completa mas tem
filhos vivos, `try_destroy` não o destrói (`completed && children.is_empty()`
é false). O fiber fica no map como zombie — sem CPU, sem run_queue, sem
blocked. Quando o último filho termina, `try_destroy` propaga bottom-up.

### `wasmtime-fiber` e yield (Decisão B)

```rust
pub enum YieldReason {
    WaitingOnChannel(i64),
    WaitingOnSelect(Vec<i64>),
    Cooperative,
    Done,
}

pub type KataFiber = Fiber<'static, SpawnArgs, YieldReason, i64>;
```

O scheduler interpreta o `YieldReason` e trata conforme a tabela acima.

### Yield points (Decisão G)

**TLS:**
```rust
thread_local! {
    static YIELD_COUNTER: Cell<i32> = const { Cell::new(YIELD_INTERVAL) };
}
```

**`kata_rt_yield_check()`:** decrementa o contador. A cada `YIELD_INTERVAL`
iterações, verifica se `run_queue` não está vazia. Se há fibers prontos,
suspende com `YieldReason::Cooperative`. O scheduler coloca o fiber de volta
na `run_queue` e executa o próximo.

**No codegen:** o lowering de `Loop` e `ForIn` insere uma `call kata_rt_yield_check()`
no header do loop, antes do body. Isto é uma `Inst::CallFfi` — mesma
infraestrutura de qualquer FFI call.

## Codegen

### `channel!`, `queue!`, `broadcast!`

ActionCalls builtin interceptados pelo typeck. O typeck produz o nó TAST
`ChannelCreate` com a topologia. O codegen emite a FFI correspondente:

```rust
// TAST:
ChannelCreate {
    kind: ChannelKind,  // Rendezvous | Buffered(u64) | Broadcast
},

enum ChannelKind {
    Rendezvous,
    Buffered(u64),
    Broadcast,
}
```

O typeck atribui os tipos dos ends:
- `channel!()` → `Sender::T0`, `Receiver::T0` (T0 inferido do uso)
- `queue!(8)` → `Sender::T0`, `Receiver::T0`
- `broadcast!()` → `Sender::T0`, `ReceiverFactory::T0`

```rust
// Codegen:
match kind {
    ChannelKind::Rendezvous => emit_call("kata_rt_channel_create", &[]),
    ChannelKind::Buffered(n) => emit_call("kata_rt_queue_create", &[iconst(n)]),
    ChannelKind::Broadcast => emit_call("kata_rt_broadcast_create", &[]),
}
// O retorno é um handle i64 que codifica os dois ends.
// O typeck sabe qual end é tx e qual é rx pelo padrão de binding.
```

### `!>` (envio)

O codegen despacha para a FFI de envio. O runtime sabe a topologia pelo
handle:

```rust
// TAST:
ChannelSend { channel, value }

// Codegen:
let ch = lower(channel);  // sender handle (i64)
let val = lower(value);    // valor (i64)
emit_call("kata_rt_channel_send", &[ch, val])
```

### `<!` (recebimento)

```rust
// TAST:
ChannelRecv { channel, recv_ty, bind_name }

// Codegen:
let ch = lower(channel);  // receiver handle (i64)
let result = emit_call("kata_rt_channel_recv", &[ch]);
// result é o valor recebido (i64)
// bind_name é definido no escopo da Action com tipo recv_ty
```

### `fork!`

```rust
// TAST:
Fork {
    action_ref: String,    // nome da Action
    args: Box<Spanned<TypedExpr>>,  // tupla de argumentos
}

// Codegen:
// Já existe: kata_rt_spawn(fn_ptr, caller_arena, args_ptr)
// fn_ptr = ponteiro da Action (compilada pelo Cranelift)
// args_ptr = ponteiro para a tupla de args (alocada na arena do caller)
// O scheduler cria o fiber filho na árvore (Pré-11 já faz isto).
```

### `select`

```rust
// TAST:
Select { arms, timeout_ms, timeout_body }

// Codegen:
// 1. Coletar receiver handles dos braços
// 2. Emitir kata_rt_select(handles, n, timeout_ms)
// 3. Retorno: índice do braço que disparou + valor recebido
// 4. Despachar para o body do braço correspondente
// 5. Se timeout dispara, executar timeout_body
```

### Yield points no lowering de loops (Decisão G)

O lowering de `Loop` e `ForIn` insere `kata_rt_yield_check()` no header:

```clif
block0:                                    ; loop header
    call kata_rt_yield_check()             ; ← inserido pelo codegen
    ; ... body do loop ...
    jump block0                            ; back-edge
```

Isto é uma mudança localizada no lowering — não afeta typeck, AST, parser,
ou qualquer outra camada. É puramente codegen + runtime.

### `@parallel`

```rust
// Codegen:
// 1. fork() do processo OS
// 2. No filho: executar a Action num runtime isolado
// 3. No pai: criar pipe/IPC channel para receber resultado
// 4. Serializar args via TypeShape walk (não HeapSnapshot)
```

**Serialização sem HeapSnapshot:** em vez de depender do Fio 12
(JIT-and-execute / HeapSnapshot), usamos `Box` + `TypeShape` para
serialização. O runtime já tem `TypeShape` para o decref walk (Fio 9). A
serialização para IPC caminha a mesma estrutura:

- `Int`, `Float` → memcpy do `i64`/`f64` (8 bytes)
- `Text` → memcpy do ponteiro + len + contents
- `Tuple`, `Struct` → walk recursivo pelos campos via `TypeShape`
- `List`, `Array` → walk recursivo pelos elementos
- `Sum` → tag + walk do payload

O `Box<T>` é alocado para conter os bytes serializados. O pipe transmite os
bytes crus. No filho, `Box<T>` é alocado novamente e os bytes são
desserializados de volta para a estrutura, usando o mesmo `TypeShape`.

Isto permite `@parallel` funcionar com qualquer tipo, não apenas `Int`/`Unit`,
sem depender do Fio 12.

**Bloqueio do parent:** o parent (fiber) bloqueia em `read(pipe)` esperando o
child. Como o parent é um fiber, o `read()` é uma syscall blocking — ocupa a
thread OS. As outras fibers só executam se houver yield points no código do
parent entre o `fork()` e o `read()`, ou se o `read()` for precedido por um
yield. Na prática, o parent faz `fork()` → `yield` (cede a outras fibers) →
`read()` (bloqueia a thread até o child terminar). Quando o `read()` retorna,
o parent continua.

## Fases de implementação

### Fase 1: Tokens e AST (lexer + parser) ✅

**Lexer:**
- Adicionar `Token::SendArrow` (`!>`)
- Adicionar `Token::RecvArrow` (`<!`)
- Adicionar `Token::Select` (keyword)
- Adicionar `Token::Timeout` (keyword)
- Modificar dispatch.rs: `!` com lookahead `>` → `SendArrow`; `<` com lookahead `!` → `RecvArrow`

**Parser:**
- `!>` e `<!` no loop de `parse_expr` (mesma camada que `|` e `|>`)
- `select` como expressão de Action (parse braços indentados)
- `channel!`, `queue!`, `broadcast!` já são ActionCalls — nenhum parser novo
- `fork!` já é ActionCall — nenhum parser novo (mas args é tupla)
- Novos nós AST: `ChannelSend`, `ChannelRecv`, `Select`, `SelectArm`

**DoD Fase 1:** `kata parse` de programa com `!>`, `<!`, `select` produz AST
correta. Snapshots insta dos novos nós. ✅

### Fase 2: Sistema de tipos (`Sender::T`, `Receiver::T`, `ReceiverFactory::T`) ✅

**kata-core:**
- `Ty::Sender(Box<Ty>)`, `Ty::Receiver(Box<Ty>)`, `Ty::ReceiverFactory(Box<Ty>)`
- `type_name_str`, `ty_to_clif`, `to_shape` para as três variantes

**kata-inference:**
- Typeck de `ChannelSend`: channel deve ser `Sender::T`, value deve ser `T`
- Typeck de `ChannelRecv`: channel deve ser `Receiver::T`, infere `T`, cria binding `bind_name: T`
- Typeck de `Select`: todos os braços devem ter receivers do mesmo `T` (ou erro)
- Typeck de `channel!()`: cria `(Sender::T0, Receiver::T0)` onde T0 é inferido do uso
- Typeck de `queue!(N)`: N deve ser `Int` literal, cria `(Sender::T0, Receiver::T0)`
- Typeck de `broadcast!()`: cria `(Sender::T0, ReceiverFactory::T0)`
- Typeck de `rxf!()` (receiver factory call): cria `Receiver::T`
- Typeck de `fork!(action, args)`: action deve ser nome de Action declarada, args deve matchar params, retorna `Unit`
- `Effect::ChannelOp` marcado em `!>`, `<!`, `select`
- `Effect::Spawn` marcado em `fork!`
- Escape analysis: `!>` marca valor como `EscapeTarget::Ancestor(lca_depth)` onde lca_depth é calculado pela árvore de fibers

**DoD Fase 2:** Typeck rejeita `tx <! nome` (tx é Sender, não Receiver). Rejeita
`rx !> valor` (rx é Receiver, não Sender). Rejeita `select` com braços de tipos
diferentes. `fork!(nao_eh_action, ())` é erro. `channel!()` infere tipo
corretamente. `broadcast!()` produz receiver factory que cria receivers.

### Fase 3: Runtime — canais

**kata-rt:**
- `channel.rs`: `Channel`, `Queue`, `Broadcast` structs
- FFI functions: `kata_rt_channel_create/send/recv`, `kata_rt_queue_*`, `kata_rt_broadcast_*`
- `kata_rt_broadcast_receiver_create` para receiver factory
- Testes unitários do runtime (sem JIT): criar, enviar, receber

**DoD Fase 3:** Testes de runtime passam. Canal rendezvous sincroniza
sender/receiver. Queue respeita capacidade. Broadcast entrega última msg a
múltiplos receivers (latest only, future only). Receiver factory cria
receivers independentes.

### Fase 4: Scheduler — yield e structured concurrency

**kata-rt:**
- `YieldReason` enum em `fiber.rs`
- `Fiber` muda de `Yield = ()` para `Yield = YieldReason`
- `kata_rt_yield()` implementado de verdade (suspende via `Suspend`)
- `Scheduler::block_fiber(fiber_id, reason)` — marca como blocked
- `Scheduler::make_ready(fiber_id)` — move de blocked para run_queue
- `pending_wakes` semântica unpark (já existe no struct, agora usado)
- Run loop: continua processando até todos completarem (structured concurrency)
- Deadlock detection: `run_queue` vazia + `blocked` não vazio + sem timers = abort

**DoD Fase 4:** Fiber que faz `<!` em canal vazio bloqueia (yield). Scheduler
executa outro fiber. Quando canal recebe dado, fiber bloqueado é acordado.
Action que completa com forks vivos fica zombie até filhos terminarem.

### Fase 5: Codegen — `!>`, `<!`, `channel!`, `queue!`, `broadcast!`, `fork!`

**kata-codegen:**
- Lowering de `ChannelCreate`, `ChannelSend`, `ChannelRecv`, `Fork`
- FFI signatures para todas as funções de canal
- `fork!(action, args)` → `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`

**DoD Fase 5:** Programa Kata com `channel!`, `!>`, `<!`, `fork!` compila
e executa. Producer/consumer via fork funciona.

### Fase 6: Codegen — `select`

**kata-codegen:**
- Lowering de `Select`: coleta handles, emite `kata_rt_select`, despacha braço
- `timeout_ms` como argumento (ou -1 se sem timeout)

**DoD Fase 6:** `select` com 2+ receivers funciona. `timeout` dispara após N ms.

### Fase 7: Yield points no codegen

**kata-codegen:**
- Inserir `call kata_rt_yield_check()` no header de `Loop` e `ForIn`
- `YIELD_INTERVAL` constante (ex: 1000 iterações)

**kata-rt:**
- `kata_rt_yield_check()` FFI function
- `YIELD_COUNTER` TLS (inicializado com `YIELD_INTERVAL` no `resume()`)
- `Scheduler::has_ready_fiber()` — verifica se `run_queue` não está vazia
- `YieldReason::Cooperative` tratado no run loop (volta para `run_queue`)

**DoD Fase 7:** Fiber em `loop` pesado cede periodicamente. Outras fibers
executam durante o loop. Sem yield points, loop pesado bloqueia outras fibers
(verificado com teste de head-of-line blocking).

### Fase 8: Testes E2E

**kata-codegen/tests/:**
- Producer/consumer via `channel!`
- Buffer overflow/backpressure via `queue!(N)`
- Pub-sub via `broadcast!` com múltiplos receivers (latest only)
- `fork!` com múltiplas fibers e args
- `select` com 2 receivers
- `select` com timeout
- Structured concurrency: parent espera forks
- Yield points: loop pesado não bloqueia outras fibers
- Escape analysis: valor enviado por canal sobrevive ao sender (LCA correto)
- Receiver factory: múltiplos receivers independentes
- Deadlock detection: programa que deadlocka é abortado com mensagem

**DoD Fase 8:** Todos os testes E2E passam. Zero vazamentos (arena raiz
destruída).

### Fase 9: `@parallel` (paralelismo / multiprocess)

**kata-codegen:**
- `@parallel` directive em `ActionDecl`
- Codegen: `fork()` syscall, pipe entre pai e filho
- No filho: executar Action em runtime isolado
- No pai: receber resultado via pipe
- Serialização via `Box` + `TypeShape` walk (caminha a estrutura
  recursivamente, como o decref walk do Fio 9)
- Desserialização no filho usando o mesmo `TypeShape`
- Funciona com qualquer tipo, não apenas `Int`/`Unit`
- Parent faz yield antes de `read(pipe)` para não bloquear outras fibers

**DoD Fase 9:** `@parallel action` executa em processo OS separado. Resultado
volta via IPC. Serialização de tuplas, structs, listas funciona. Teste E2E
com `@parallel` passa.

## DoD (Definition of Done)

1. **`channel!` rendezvous funciona.** Retorna `(Sender::T, Receiver::T)`.
   Sender bloqueia até receptor sincronizar.
2. **`queue!(N)` buffered funciona.** Retorna `(Sender::T, Receiver::T)`.
   Backpressure quando buffer cheio.
3. **`broadcast!` pub-sub funciona.** Retorna `(Sender::T, ReceiverFactory::T)`.
   Receiver factory cria receivers independentes. Fire-and-forget. Latest only,
   future only.
4. **`fork!` submete Action em fiber separada com args.** A Action executa
   concorrentemente com os argumentos fornecidos.
5. **`select` multiplexa 2+ receivers.** `timeout` dispara após N ms.
6. **Yield cooperativo.** `<!` em canal vazio bloqueia o fiber, não a thread.
7. **Yield points.** Fiber em `loop` pesado cede periodicamente — outras
   fibers executam. Sem head-of-line blocking.
8. **Structured concurrency.** Action não termina até todos os forks
   completarem.
9. **Escape analysis para canais.** Valores enviados por `!>` sobrevivem ao
   sender, alocados na arena do LCA (não necessariamente raiz).
10. **Deadlock detection.** Fibers bloqueados sem progresso possível são
    detectados e abortados.
11. **`@parallel` spawn processo OS.** Action executa em processo isolado.
    Serialização via `Box` + `TypeShape` funciona com qualquer tipo.
12. **Zero vazamentos.** Arena raiz destruída após scheduler run completar.
13. **Testes E2E.** Mínimo 15 testes cobrindo todas as features acima.

## Não faz parte deste PRD

- **M:N multithread (scheduler com worker pool).** Post-1.0. A estrutura do
  scheduler (run loop, yield, blocked/wake, structured concurrency) é
  independente do número de threads. Se workloads reais demonstrarem que
  yield points + `@parallel` não chegam, M:N é adição incremental.
- GC para fibers long-lived (ver nota no ROADMAP.md)
- `@log` via CSP (Fio 14)
- `@test` (Fio 14)
- Comptime / `@cache_strategy` (Fio 12)
- `select` para envio (Decisão D — discutir após implementação)

## Casos não cobertos

### Canais de canais

`Sender::(Sender::T)` — canais que transportam canais. Tipos aninhados
devem funcionar pela estrutura do `Ty`, mas não há testes E2E planejados
para isto nesta fase.

### Fechamento explícito de canais

Não há `close!(ch)`. Canais são destruídos quando a arena do fiber que os
criou é liberada. Receptores bloqueados são acordados com valor sentinela
(0) ou erro. Fechamento explícito é uma extensão futura.