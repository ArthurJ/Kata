# PRD: Fios 3+4+9 — Actions, Enum Avançado, Closures, Escape Analysis, ARC

## Objetivo

Combinar três fios num único PRD porque suas dependências são circulares quando
separados: `?` (Fio 3) precisa de `Result` (Fio 4); `|` fallback (Fio 4) precisa de
Sum com payload (Fio 4); closures com captura (Fio 9) precisam de Actions (Fio 3)
para contexto de escape; ARC (Fio 9) precisa de escape analysis que precisa de
closures que precisam de Actions. A combinação resolve todas as dependências
circulares e entrega um sistema coerente: domínio impuro (Actions) com controle
de fluxo imperativo, tipos soma com payload para modelagem de erros, e closures
com captura léxica e gerenciamento automático de memória.

## Depende de

Fio 1 (pipeline end-to-end, TypeEnv, DispatchTable, Ty, FfiSymbol), Fio 2
(lambdas, Hole, match, guards, patterns, `|>`, `with`, TAST enriquecida com
`tail_pos` e `effect`).

## Estado herdado do Fio 2

O Fio 2 já deixou infraestrutura pronta para estes fios:

- **Tokens**: `Action`, `Var`, `Return`, `Bang` (`!`), `Question` (`?`), `Pipe`
  (`|`), `Semicolon` (`;`), `LBracket` (`[`), `RBracket` (`]`), `LBrace` (`{`),
  `RBrace` (`}`) já existem em `Token` e são produzidos pelo lexer. **Faltam**:
  `Loop`, `For`, `Break`, `Continue` — não existem nem no lexer nem no enum
  `Token`. São adicionados neste PRD.
- **`Ty::Function(Vec<Ty>, Box<Ty>)`** já existe e é usado por lambdas como
  valor (call_indirect).
- **`Ty::Tuple(Vec<Ty>)`** já existe (antecipado de Fio 5 em Fio 2). Sem field
  access, sem `.N` — só tipo estrutural para patterns.
- **`TypedExprKind::Closure`** já existe com `captures: Vec<CaptureInfo>` (sempre
  vazio em Fio 2) e `escapes: bool` (sempre `false` em Fio 2). Fio 9 promove
  `escapes: bool` para `escape: EscapeKind` (definido em kata-core, 3 estados).
- **`CaptureInfo`** e **`CaptureStorage`** (Stack/Heap) já existem como structs
  placeholder em `typed.rs`. Fio 9 preenche.
- **`Effect`** já tem `Puro`, `IO`, `Spawn`, `ChannelOp`. Fio 2 só produz `Puro`.
  Fio 3+4+9 não ativa `Effect` — campo continua `Puro` em todos os nós. Pureza
  é garantida por regra de tipo (função pura não pode conter `ActionCall`).
  `Effect` será revisitado em Fio 11 quando o scheduler precisar rastrear
  `Spawn`/`ChannelOp`.
- **`Effect::Spawn`** e **`Effect::ChannelOp`** já existem no enum. Não são
  usados neste PRD — ficam para Fio 11.
- **Arena**: `kata_rt_arena_create/alloc/destroy` já existem como C-ABI. A arena
  é thread-local (`thread_local! { ARENA }`). Fio 3 estende para caller's arena.
- **`FfiSymbol`** tem `ArenaCreate`, `ArenaAlloc`, `ArenaDestroy`. Fio 3 adiciona
  novos símbolos (Sum, ARC, Fiber).
- **Parser**: `parse_sig` já parseia cláusulas lambda após assinatura.
  `parse_lambda` já parseia lambda anônimo. `parse_match` já parseia match.
  Faltam: `parse_action`, `parse_var`, `parse_return`, `parse_loop`,
  `parse_break`, `parse_continue`, `parse_bang_call`, `parse_question`,
  `parse_pipe_fallback`, `parse_semicolon`, `parse_action_body`.
  (`parse_for` adiada para Fio 7+8.)
- **Inference**: `infer_expr` despacha por `TypedExprKind`. `infer_lambda` e
  `infer_match` já existem. Faltam: `infer_action`, `infer_return`, `infer_loop`,
  `infer_var`, `infer_bang_call`, `desugar_question`,
  `desugar_pipe_fallback`, escape analysis (4 passes), `collect_captures`.
- **Codegen**: `lower_expr` despacha por `TypedExprKind`. `lower_match` e
  `lower_clause_chain` existem. Pattern testing em `pattern.rs` só suporta
  Boolean (variantes unitárias, icmp_imm). Faltam: lower_action, lower_return,
  lower_loop, lower_bang_call, Sum com payload (tag + ponteiro),
  ARC pass, fiber integration.
- **`TypedFunction`** já existe com `name`, `param_types`, `ret_ty`, `clauses`.
  Actions seguem o mesmo padrão (função Cranelift separada) com ABI estendido
  (arena handle implícito).
- **`OverloadInfo`** já tem `is_action: bool` (sempre `false` em Fio 2). Fio 3
  usa para distinguir Actions de funções puras no DispatchTable.
- **`EnumRegistry`** já cataloga variantes por enum. Fio 4 estende para
  variantes com payload. (Variantes predicadas são Fio 6.)
- **`VariantDecl`** já tem `payload: Option<Spanned<TypeExpr>>` — sempre `None`
  em Fio 2. Fio 4 popula.

## Escopo

### Fio 3: Actions, return, ;, ?, var, loop/break/continue, Caller's Arena

#### Actions

```kata
action conectar_servidor
    let x := ler!()?
    echo!(show x)
    x

action greet
    echo!("hello")
    echo!("world");
```

Actions são o domínio impuro. Declaração com `action nome` (sem `!`).
Chamada com `!` sufixo e tupla obrigatória: `echo!("msg")`, `conectar!()`.

- Toda Action recebe exatamente uma tupla como argumento. `!()` para Actions
  sem parâmetros, `!(arg1, arg2)` para N parâmetros.
- Actions não têm `effect` especial na TAST — `Effect` permanece `Puro`.
  A impureza de Actions é garantida pela regra de que funções puras não podem
  conter `ActionCall` (erro de tipo no typeck).
- **Proibição de recursão**: o compilador aciona Erro Fatal se detectar chamadas
  recursivas dentro de Actions (call graph analysis). Protege contra stack
  overflow em fibers.
- Actions podem ser passadas como argumento sem `!` (referência, sem ativação):
  `fork!(minha_action)` passa a Action, `minha_action!()` ativa.
- Actions NÃO podem ser atribuídas a `let` como valor (diferente de funções
  puras, que são `Ty::Function`). Actions não são first-class — só podem ser
  chamadas ou passadas como argumento para outras Actions (fork).

#### `!` sufixo de chamada

```kata
echo!("mensagem")
conectar_servidor!()
fork!(minha_action)
```

`!` é o marcador de impureza. Na declaração, Actions não usam `!`. Na chamada,
`!` é obrigatório. O parser distingue `echo!` (Action call) de `echo` (função
pura ou identificador) pelo `!` após o callee.

- `!` é seguido obrigatoriamente por parênteses com tupla (mesmo que vazia).
- O parser produz `Expr::ActionCall { callee, args: Spanned<Expr::Tuple> }` quando
  vê `ident!(...)`. O `!` é o discriminador.

#### `var` — binding mutável (exclusivo de Actions)

```kata
action contador
    var count := 0
    loop
        count := + count 1
        echo!(show count)
        if count > 10
            break
```

`var` é o único binding mutável da linguagem. Exclusivo de Actions. Mutação na
stack da fiber, nunca nos dados imutáveis da arena.

- `var` permite reatribuição: `count := nova_expr`.
- `let` em Actions é imutável (mesma semântica de funções puras).
- O typeck verifica que `var` só aparece dentro de Action bodies.

#### `return` — early return (exclusivo de Actions)

```kata
action buscar
    let x := ler!()?
    match x
        Optional::Some(v): return v
        Optional::None: return 0
```

`return` aborta a Action e retorna o valor. O valor é alocado na **caller's
arena** (arena de quem chamou a Action), que persiste até o caller terminar.

- `return` é exclusivo de Actions. Não existe em funções puras (guards e pattern
  matching são o mecanismo de fluxo no domínio puro).
- Retorno implícito: a última expressão de uma Action sem `;` é o retorno
  implícito. Mesma semântica de caller's arena.
- `return` + caller's arena resolvem estruturalmente o problema de Actions que
  retornam coleções sem use-after-free.

#### `;` — terminador de statement

```kata
action processar
    let x := 5; echo!(x)       # dois statements na mesma linha
    let y := + x 1
    y                           # retorno implícito (última expr sem ;)

action greet
    echo!("hello")
    echo!("world");            # ; → statement, action retorna Unit
```

`;` é exclusivo de Actions. Distingue "computação local" de "valor que escapa":

- Sem `;`: a expressão é retorno (alocada na caller's arena).
- Com `;`: a expressão é computação local (alocada na arena local da Action,
  liberada no epílogo).

Não existe em funções puras — o domínio puro não tem statements, só expressões.

#### `?` — fail-fast (exclusivo de Actions)

```kata
action validar
    let x := PositiveInt 42 ?
    echo!(show x)
```

`?` desempacota `Ok(v)` ou `Some(v)`. Se `Err(e)` ou `None`, aborta a Action
retornando `Err(e)` ou `None`.

- **Domínio**: exclusivo de Actions. A Action precisa ter `Result` (ou
  `Optional`) como tipo de retorno — `?` injeta `return Err(e)` na TAST.
- **Desugar no typeck**: `expr ?` vira:
  - Se `expr` é `Result::(T, E)`: `match expr { Ok(v) => v, Err(e) => return Err(e) }`
  - Se `expr` é `Optional::T`: `match expr { Some(v) => v, None => return None }`
- O desugar é no typeck — a TAST contém `Match` + `Return`, não `Question`.

#### `loop`, `break`, `continue`

```kata
action contador
    var i := 0
    loop
        i := + i 1
        echo!(show i)
        match > i 5
            True: break
            False: continue
```

- `loop`: laço infinito. Só sai via `break`.
- `break`: sai do laço. `continue`: próxima iteração.
- `break` e `continue` são exclusivos de Actions (dentro de `loop`).
- **Implementação no codegen**: `loop` → block com back-edge. `break` → jump
  para block de saída. `continue` → jump para início do body.
- `for` é adiada para Fio 7+8 (exige ITERABLE e coleções como tipos).

#### Caller's Arena — Arena handle implícito no ABI

Toda Action recebe um **arena handle** extra no nível do ABI (invisível ao
programador Kata). O codegen passa o handle na chamada.

- **Prólogo da Action**: cria arena local (`kata_rt_arena_create`), salva o
  handle do caller.
- **`;` statements**: alocam na arena local (`kata_rt_arena_alloc(local_handle, size)`).
- **`return v` / `v` sem `;`**: alocam na arena do caller
  (`kata_rt_arena_alloc(caller_handle, size)`).
- **Epílogo**: destrói arena local (`kata_rt_arena_destroy(local_handle)`).
  Valores na caller's arena sobrevivem.
- **Entry point**: recebe handle da arena global (criada no início de
  `__kata_entry`).
- **Aninhamento**: Action A chama Action B. A passa `caller_arena` de A
  (se B está em tail_pos em A) ou `local_arena` de A (se B está em `;`)
  como `caller_handle` para B. B aloca retornos na arena recebida. Se B
  está em tail_pos, o valor vai para a caller_arena de A — que persiste
  até o caller de A terminar. Se B está em `;`, o valor vai para a
  local_arena de A — destruída no epílogo de A (valor é descartado).

O `LowerCtx` ganha dois handles:
```rust
pub struct LowerCtx {
    // ...
    pub caller_arena: i64,  // handle da arena do caller (para retornos)
    pub local_arena: i64,   // handle da arena local (para statements)
}
```

Na função `__kata_entry`: cria arena global no prólogo via
`kata_rt_arena_create()` (handle 0 no pool de arenas). `caller_arena` =
handle da arena global. `local_arena` = None (entry point não é Action,
não tem `;` statements). Em Actions: `caller_arena` vem do parâmetro
implícito, `local_arena` é criada no prólogo.

O runtime mantém um **pool de arenas** thread-local (`Vec<Arena>`)
indexado por handle. `kata_rt_arena_create` retorna um handle único
(índice no Vec). `kata_rt_arena_destroy(handle)` reseta SÓ a arena
daquele handle — outras arenas não são afetadas.

### Fio 4: Enum Avançado — Payload, Result, Optional, |

#### Sum com payload

```kata
enum Result
    Ok(T)
    Err(E)

enum Optional
    Some(T)
    None
```

Variantes com payload carregam um tipo. `Ok(T)` é uma variante que carrega um
valor do tipo `T`. `Some(T)` carrega `T`. `None` é unitária.

- **Invariante de codegen: Sum com payload é sempre ponteiro (box 8 bytes).**
  Tag (discriminant) + ponteiro para payload. Uniforme — não há caso inline vs
  boxed. O codegen faz `arena_alloc` (8 bytes para tag+ptr), `store` da tag,
  `store` do payload ptr.
- **Runtime**: `kata_rt_store_sum_result(tag: i64, payload: i64) -> i64` — aloca
  box, armazena tag e payload, retorna ponteiro. `kata_rt_sum_tag_int(val: i64)
  -> i64` — extrai tag de um Sum. Distinto de `kata_rt_tag_int` (SMI tagging de
  BigInt).
- **Match em Sum com payload**: o codegen extrai a tag (`kata_rt_sum_tag_int`),
  despacha por tag, e extrai o payload (load do offset da tag).

#### `Result::(T, E)` e `Optional::T`

Definidos no prelude. São enums genéricos com type params posicionais.

- `Result::(T, E)` tem variantes `Ok(T)` e `Err(E)`.
- `Optional::T` tem variantes `Some(T)` e `None`.
- `::` em type params: `Result::(Int, Text)` especifica `T=Int, E=Text`.
- O parser já reconhece `TypeExpr::ParamApp { name, params }` (Fio 2). O typeck
  resolve os params posicionais.

#### `|` — fallback local (coalescência de erro)

```kata
let x := PositiveInt 25 | 25          # Ok(25) desempacotado; fallback 25
let y := PositiveInt (-5) | 0         # Err(0) desempacotado; fallback 0
let z := arr.0 | 0                     # Result desempacotado; fallback 0
```

`|` é infixo entre duas expressões. Se a esquerda é `Ok(v)` ou `Some(v)`,
desempacota `v`. Se `Err(_)` ou `None`, avalia e retorna a direita.

- **Domínio**: funções puras e Actions.
- **Invariante de enum**: `|` só se aplica a enums onde todas as variantes
  exceto a última carregam payload. A última variante é a "cauda" (unitária,
  default). Se a esquerda é a cauda, avalia a direita. Enums que não seguem
  esta estrutura (variantes unitárias intercaladas, payload na última) não
  são compatíveis com `|` — erro de typeck.
- **Coerção contextual**: se o fallback é literal do tipo base, o compilador
  validará predicados em compile-time (para tipos refined — Fio 6, não
  implementado neste PRD).
- **Desugar no typeck**: `lhs | rhs` vira `match lhs { Ok(v) => v, Err(_) =>
  rhs }` (ou equivalente para o enum específico). A TAST contém `Match`, não
  `Pipe`.
- **Distinção de `|>`**: `|` é coalescência de erro (fallback). `|>` é pipeline
  de transformação pura.

#### `panic!`, `assert!`

```kata
panic!("estado impossível")
assert!(* x 0, "x deve ser positivo")
```

- `panic!`: aborta imediatamente. Destrói a arena local da Action. Recebe uma
  tupla com mensagem. Registrada no DispatchTable como Action builtin com
  `FfiSymbol::Panic` (`kata_rt_panic`). O typeck trata como ActionCall normal.
- `assert!`: verifica condição. Se falsa, `panic!`. Recebe tupla com condição e
  mensagem opcional. Desugared pelo typeck em Guard + Panic:
  `match cond { True: Unit, False: panic!(msg) }`.
- Ambos são builtins do compilador (Actions com `!`), não stdlib.

#### Match general case (3+ variantes)

```kata
match status
    OK: "ok"
    Created: "criado"
    BadRequest: "erro"
    otherwise: "desconhecido"
```

Match em 3+ variantes com switch/branch chain. Fio 2 já faz match em 2 variantes
(Boolean); Fio 4 generaliza para N.

- O codegen emite branch chain (brif para cada variante). Para variantes
  consecutivas com tags consecutivas, o Cranelift pode otimizar para switch.
- Match em Sum com payload: extrai tag, despacha por tag, extrai payload no
  body do arm.

### Fio 9: Closures, Escape Analysis, ARC

#### Closures com captura léxica

```kata
let n := 10
let add_n := + _ n        # captura n do escopo externo
add_n 5                    # 15
```

O Hole de Fio 2 (`+ 10 _`) já gera lambdas sem captura (o `10` é literal). Fio 9
traz captura de variáveis externas:

- `+ _ n` vira `lambda x: + x n` onde `n` é capturado do escopo externo.
- A TAST `Closure.captures` é populada com `CaptureInfo { name: "n", ty: Int,
  storage: Stack }`.
- O codegen passa captures como argumentos extras (ou via `CaptureBox` se a
  closure escapa para heap).

#### Escape analysis (4 passes)

Determina se uma closure escapa (retornada, enviada por canal, armazenada em
lista) ou fica local:

| `EscapeKind` | `CaptureStorage` | Mecanismo |
|---|---|---|
| `NãoEscapa` | Stack | Captures na arena local, O(1) |
| `EscapaParaHeap` | Heap | Captures promovidas para `Arc<T>` |
| `EscapaParaClosure` | Heap | Captures em closure aninhada |

4 passes sobre a TAST:

- **Pass 0**: closures em retorno de funções puras → `EscapaParaHeap`.
- **Pass 1**: inspeção sintática (Send/Fork/ListLit/...) → marca escapes.
- **Pass 2**: propagação de aliases.
- **Pass 3**: promoção Stack → Heap.

#### `Arc<T>` nativo

```rust
Arc<ClosureBox { fn_ptr, captures }>
```

Reference counting thread-safe via Rust nativo. O codegen injeta
`incref`/`decref` via FFI. O ARC pass consulta a `MetadataTable` (sidecar
pós-lowering) para saber onde inserir incref/decref.

- **Runtime**: `kata_rt_alloc_arc`, `kata_rt_incref`, `kata_rt_decref`.
- **Layout**: `Arc<ClosureBox { fn_ptr: *const fn, captures: [*const val; N] }>`.
- **Corretude**: garantida pelo Rust — não há ARC pass manual suscetível a bugs
  de double-free ou leak. O Rust `Arc<T>` é thread-safe (atomic refcount).

#### `FnValueCall` — chamada a closure escapada

1. Carrega ponteiro da struct `Arc<ClosureBox>` do stack.
2. Extrai `fn_ptr` e `captures`.
3. Monta argumentos.
4. Emite `call_indirect`.

#### TRMA — adiada

TRMA (Tail Recursion Modulo Associativity) foi adiada para Zeladoria
pós-Fio 3+4+9. `@associative` já existe no resolution (parseado e resolvido
desde Fio 1). O TRMA pass no `kata-optimizer` é ortogonal a este PRD — não
depende de Actions, Sum com payload, nem closures. Ver "Não Inclui".

### Fibers (wasmtime-fiber + scheduler struct mínimo)

Cada Action corre numa fiber. A fiber é a unidade de escalonamento.

- **wasmtime-fiber**: stack switching. Cada fiber tem sua própria stack. Yield
  cede controle ao scheduler. Sem canais (Fio 11), não há o que bloquear — mas
  a infraestrutura de fibers está pronta.
- **Scheduler struct mínima**: `run_queue`, `blocked`, `pending_wakes`,
  `current_fiber`. Em Fio 3+4+9, só há uma fiber ativa por vez (sem `fork!`).
  A struct existe para Fio 11 estender.
- **Arena per-fiber**: cada fiber tem sua arena local. O caller's arena handle
  viaja pela call stack, não pela fiber — Actions chamadas diretamente
  compartilham a mesma fiber.
- **Prólogo da Action**: cria fiber, salva stack pointer. **Epílogo**: reseta
  fiber, libera arena local.

## Crates Afetadas

```
kata-core/           Novo: EscapeKind (3 estados), CaptureInfo preenchida,
                     FfiSymbol estendido (StoreSumResult, SumTagInt, AllocArc,
                     IncRef, DecRef, FiberCreate, FiberYield, FiberSwitch, Panic)
                     Modificado: OverloadInfo.is_action já existe (usado agora),
                     TypedExprKind::Closure: escapes: bool → escape: EscapeKind
kata-ast/           Novos: Expr::ActionCall, Expr::Return, Expr::Loop,
                    Expr::Break, Expr::Continue, Expr::Var, Expr::Question,
                    Expr::PipeFallback, Item::ActionDecl, ActionClause struct
                    Modificado: VariantDecl.payload agora populado (Fio 4),
                    Pattern::Variant estendido para payload extraction
                    (Expr::Semicolon removido — ; é separador, não nó de AST)
kata-lexer/         Novo: tokens Loop, Break, Continue (palavras-chave)
                    Nenhuma mudança nos tokens existentes (Action, Var, Return,
                    Bang, Question, Pipe, Semicolon já existem)
                    (For é adiada para Fio 7+8)
kata-parser/        Novo: parse_action, parse_var, parse_return, parse_loop,
                    parse_break, parse_continue, parse_bang_call,
                    parse_question, parse_pipe_fallback, parse_semicolon,
                    parse_action_body
                    Modificado: parse_enum_decl (variantes com payload),
                    parse_expr_atom (reconhecer return, loop, break, continue,
                    var, action call com !), can_start_expr (adicionar Return,
                    Loop, Break, Continue, Var, Action)
                    (parse_for adiada para Fio 7+8)
kata-resolution/    Modificado: ActionDecl registra no DispatchTable com
                    is_action=true; EnumDecl com payload popula VariantDecl
                    Novo: ActionDef struct (como FunctionDef mas para Actions)
                    (Variantes predicadas adiadas para Fio 6)
kata-inference/     Novo: infer_action, infer_return, infer_loop,
                    infer_var, infer_bang_call, desugar_question,
                    desugar_pipe_fallback, escape_analysis (4 passes),
                    collect_captures, check_action_recursion,
                    infer_sum_with_payload, check_pattern_payload
                    Modificado: infer_expr (dispatch para novos nós),
                    check_pattern (variantes com payload), check_exhaustiveness
                    (Sum com payload)
                    (infer_for, infer_variant_predicada adiados)
kata-codegen/       Novo: lower_action, lower_return, lower_loop,
                    lower_bang_call, lower_sum_with_payload (tag + ptr),
                    lower_pattern_variant_payload, arc_pass, lower_fiber,
                    lower_action_abi (caller_arena handle)
                    Modificado: lower_expr (dispatch para novos nós),
                    lower_match (Sum com payload: extrair tag + payload),
                    test_single_pattern (Variant com payload), LowerCtx
                    (caller_arena, local_arena handles), define_function_body
                    (ABI de Actions: +1 param implícito)
                    Novo módulo: escape.rs (ARC pass via MetadataTable)
                    (lower_for adiada para Fio 7+8)
kata-optimizer/    (TRMA adiada para Zeladoria pós-Fio 3+4+9 — não modificado
                    neste PRD)
kata-rt/            Novo: kata_rt_store_sum_result, kata_rt_sum_tag_int,
                    kata_rt_alloc_arc, kata_rt_incref, kata_rt_decref,
                    kata_rt_panic, fiber integration (wasmtime-fiber),
                    scheduler struct mínima,
                    kata_rt_fiber_create, kata_rt_fiber_yield, kata_rt_fiber_switch
                    Modificado: arena.rs (suporte a caller's arena: múltiplos
                    handles, arena_get(handle) para selecionar arena)
                    Novo módulo: fiber.rs, scheduler.rs
kata-driver/        Modificado: run_pipeline (Actions no entry point, fiber init,
                    caller_arena handle para entry)
                    Novo: subcomando `test` (Fio 14 — placeholder, não implementado
                    neste PRD)
Cargo.toml          Novo: wasmtime-fiber dependency
```

## Maquinaria de Tipos Construída

### kata-ast

#### Novos variants de `Expr`

```rust
/// `nome!(args)` — chamada de Action.
/// `!` é o marcador de impureza. O parser produz `ActionCall` quando vê `!`
/// após um identificador seguido de parênteses.
Expr::ActionCall {
    callee: String,
    /// Tupla de argumentos (sempre uma tupla, mesmo que vazia).
    args: Box<Spanned<Expr>>,
},

/// `return expr` — early return em Actions.
/// Exclusivo de Actions. Não existe em funções puras.
Expr::Return(Box<Spanned<Expr>>),

/// `loop` — laço infinito. Só sai via `break`.
Expr::Loop {
    body: Vec<Spanned<Expr>>,
},

/// `break` — sai do laço.
Expr::Break,

/// `continue` — próxima iteração.
Expr::Continue,

/// `var nome := expr` — binding mutável (exclusivo de Actions).
Expr::Var {
    name: String,
    value: Box<Spanned<Expr>>,
},

/// `expr ?` — fail-fast (exclusivo de Actions).
/// Desugared pelo typeck em Match + Return. Nunca chega à TAST.
Expr::Question(Box<Spanned<Expr>>),

/// `lhs | rhs` — fallback local (coalescência de erro).
/// Desugared pelo typeck em Match. Nunca chega à TAST.
/// Distinto de `|>` (PipeForward — pipeline de transformação pura).
Expr::PipeFallback {
    lhs: Box<Spanned<Expr>>,
    rhs: Box<Spanned<Expr>>,
},
```

**Nota sobre `;`**: O `;` não é um nó de AST separado. O parser produz statements
sequenciais dentro de Action bodies. O `;` é consumido pelo parser como separador
de statements (como `StmtSep` em contexto de top-level). A distinção "última expr
sem `;` = retorno" é feita no parser: a última expressão do body da Action é
marcada como retorno implícito.

#### `Item::ActionDecl`

```rust
/// `action nome` com body indentado.
pub enum Item {
    // ... existentes ...
    ActionDecl {
        name: String,
        /// Parâmetros da Action (uma tupla tipada, ou vazia).
        /// A assinatura da Action especifica o tipo da tupla de argumentos.
        params: Vec<Spanned<TypeExpr>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        /// Body da Action (statements sequenciais).
        body: Vec<Spanned<Expr>>,
    },
}
```

#### Modificação em `VariantDecl` (Fio 4)

```rust
pub struct VariantDecl {
    pub name: String,
    /// Payload da variante. None = unitária (`True`).
    /// Some(ty) = carrega tipo (`Ok(T)`).
    pub payload: Option<Spanned<TypeExpr>>,
}
```

Variante predicada (com guard no payload) é adiada para Fio 6.

### kata-core

#### `EscapeKind`

Definido em kata-core (não em kata-inference) para que `TypedExprKind` possa
referenciá-lo sem dependência circular. 3 estados:

```rust
/// Resultado da escape analysis (Fio 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeKind {
    /// A closure não escapa — captures na arena local, O(1).
    NãoEscapa,
    /// A closure escapa para heap — captures promovidas para Arc<T>.
    EscapaParaHeap,
    /// A closure escapa para outra closure — captures em closure aninhada.
    EscapaParaClosure,
}
```

#### `FfiSymbol` — novos símbolos

```rust
pub enum FfiSymbol {
    // ... existentes ...

    // ── Sum com payload (Fio 4) ──────────────────────────
    /// Aloca box Sum: tag + payload. Retorna ponteiro.
    StoreSumResult,
    /// Extrai tag de um Sum (i64). Distinto de TagInt (SMI tagging de BigInt).
    SumTagInt,

    // ── ARC (Fio 9) ─────────────────────────────────────
    /// Aloca Arc<T> na heap global. Retorna ponteiro.
    AllocArc,
    /// Incrementa refcount.
    IncRef,
    /// Decrementa refcount. Se chega a 0, libera.
    DecRef,

    // ── Fiber (Fio 3) ───────────────────────────────────
    /// Cria uma fiber nova. Retorna handle.
    FiberCreate,
    /// Yield da fiber atual (cede controle ao scheduler).
    FiberYield,
    /// Troca para outra fiber.
    FiberSwitch,

    // ── Panic/Assert (Fio 4) ─────────────────────────────
    /// Aborta com mensagem. `panic!("msg")` → FfiSymbol::Panic.
    Panic,
}
```

### `TypedExprKind` — novos variants

```rust
pub enum TypedExprKind {
    // ... existentes ...

    // Modificação em variant existente (Fio 9):
    // Closure { ..., escapes: bool } → Closure { ..., escape: EscapeKind }
    // O campo `escapes: bool` de Fio 2 é promovido para `escape: EscapeKind`
    // (3 estados: NãoEscapa, EscapaParaHeap, EscapaParaClosure).
    // EscapeKind é definido em kata-core.

    /// Chamada de Action (`nome!(args)`).
    /// O callee é sempre um nome no DispatchTable com is_action=true.
    /// O codegen emite call para a função Cranelift da Action, passando
    /// caller_arena handle como primeiro parâmetro implícito.
    ActionCall {
        callee: String,
        args: Box<Spanned<TypedExpr>>,
        /// caller_arena handle para a Action chamada alocar retornos.
        /// É o local_arena do caller (que se torna caller_arena do callee).
        caller_arena: i64,
    },

    /// `return expr` — early return em Actions.
    /// O valor é alocado na caller's arena. O codegen emite return_.
    Return(Box<Spanned<TypedExpr>>),

    /// `loop` — laço infinito.
    Loop {
        body: Vec<Spanned<TypedExpr>>,
    },

    /// `break` — sai do laço.
    Break,

    /// `continue` — próxima iteração.
    Continue,

    /// `var nome := expr` — binding mutável.
    Var {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },
}
```

### `TypedModule` — Actions

```rust
pub struct TypedModule {
    pub pre_entry: Vec<Spanned<TypedExpr>>,
    pub entry: Spanned<TypedExpr>,
    pub dispatch_table: DispatchTable,
    pub type_env: TypeEnv,
    pub functions: Vec<TypedFunction>,
    /// Actions tipadas (Fio 3). Cada Action vira uma função Cranelift
    /// com ABI estendido (caller_arena handle como primeiro param).
    pub actions: Vec<TypedAction>,
}

/// Action tipada — pronta para o codegen.
#[derive(Debug, Clone)]
pub struct TypedAction {
    /// Nome da Action no JITModule.
    pub name: String,
    /// Tipos dos parâmetros (elementos da tupla de argumentos).
    pub param_types: Vec<Ty>,
    /// Tipo de retorno.
    pub ret_ty: Ty,
    /// Body da Action (statements sequenciais).
    pub body: Vec<Spanned<TypedExpr>>,
}
```

### `TypedFunction` — estendido para escape (Fio 9)

```rust
pub struct TypedFunction {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub ret_ty: Ty,
    pub clauses: Vec<TypedLambdaClause>,
    /// Resultado da escape analysis (Fio 9).
    /// NãoEscapa por padrão. Preenchido pelos 4 passes.
    pub escape: EscapeKind,
    /// Captures coletadas (Fio 9). Vazio se não há captura.
    pub captures: Vec<CaptureInfo>,
}
```

### kata-resolution

#### `ActionDef`

```rust
/// Definição de Action com body Kata.
/// Produzida no resolution quando `Item::ActionDecl` é encontrado.
#[derive(Debug, Clone)]
pub struct ActionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub body: Vec<Spanned<Expr>>,
}
```

`ResolvedModule` ganha:

```rust
pub struct ResolvedModule {
    pub type_env: TypeEnv,
    pub signatures: Vec<Signature>,
    pub enum_registry: EnumRegistry,
    pub functions: Vec<FunctionDef>,
    /// Actions definidas no módulo (Fio 3).
    pub actions: Vec<ActionDef>,
}
```

### kata-inference

#### `infer_action`

`infer_action(action_def, table, enum_registry)` produz `TypedAction`:

1. Cria escopo filho no TypeEnv.
2. Define parâmetros com tipos da assinatura.
3. Verifica proibição de recursão (call graph analysis).
4. Infere cada statement do body em sequência.
5. O último statement sem `;` é o retorno implícito — verifica tipo contra
   `ret_ty`.
6. Statements com `;` retornam Unit (computação local).
7. `Effect` permanece `Puro` — não há ativação de `Effect` neste PRD.

#### `check_action_recursion`

Análise de call graph: percorre o body da Action procurando `ActionCall` para
o próprio nome. Se encontrar, `RecursiveAction` error. Verifica também
recursão indireta (A chama B que chama A) via DFS no call graph de Actions.

#### `desugar_question`

`expr ?` vira:

- Se `expr.ty == Result::(T, E)`:
  `Match expr { Ok(v) => v, Err(e) => Return(Err(e)) }`
- Se `expr.ty == Optional::T`:
  `Match expr { Some(v) => v, None => Return(None) }`

O desugar é no typeck — a TAST contém `Match` + `Return`.

#### `desugar_pipe_fallback`

`lhs | rhs` vira:

- Se `lhs.ty == Result::(T, E)`:
  `Match lhs { Ok(v) => v, Err(_) => rhs }`
- Se `lhs.ty == Optional::T`:
  `Match lhs { Some(v) => v, None => rhs }`

O desugar é no typeck — a TAST contém `Match`.

#### Escape analysis (4 passes)

**Pass 0 — Closures em retorno de funções puras:**
Percorre a TAST. Se uma `TypedExprKind::Lambda` aparece em posição de retorno de
uma função pura, marca `escape = EscapaParaHeap`.

**Pass 1 — Inspeção sintática:**
Procura por patterns que indicam escape: `Send` (canal, Fio 11 — stub),
`Fork` (fork!, Fio 11 — stub), `ListLit` (lista contém closure), `ActionCall`
(closure passada como argumento para Action). Marca `escape` apropriadamente.

**Pass 2 — Propagação de aliases:**
Se `let f := g` e `g` tem `escape = EscapaParaHeap`, então `f` também tem.
Propaga por aliases.

**Pass 3 — Promoção Stack → Heap:**
Para cada `CaptureInfo` com `storage = Stack`, se a closure que a contém tem
`escape != NãoEscapa`, promove para `Heap`.

#### `collect_captures`

Coleta free variables do body do lambda contra o escopo externo. Para cada
variável livre, cria `CaptureInfo { name, ty, storage: Stack }` (storage pode
ser promovido para Heap no Pass 3).

### kata-codegen

#### Lower Action

Cada Action vira uma função Cranelift separada com ABI estendido:

```
fn action_name(caller_arena: i64, arg1: ty1, arg2: ty2, ...) -> ret_ty
```

- Primeiro parâmetro: `caller_arena` (i64, handle da arena do caller).
- Parâmetros seguintes: elementos da tupla de argumentos.
- Prólogo: `local_arena = kata_rt_arena_create()`.
- Statements com `;`: alocam na `local_arena`.
- `return v` / retorno implícito: aloca na `caller_arena`.
- Epílogo: `kata_rt_arena_destroy(local_arena)`, `return_`.

#### Lower `return`

`return v`:
1. Lowera `v` — se é heap type (struct/tuple/list/sum), aloca na `caller_arena`.
2. Emite `return_(&[val])`.

#### Lower `loop`

```
loop:
  <body>
  jump loop
exit:
  <after loop>
```

- Cria block `loop` com back-edge para si mesmo.
- `break` → jump para `exit`.
- `continue` → jump para `loop` (início do body).

#### Lower Sum com payload

**Construção** (`Ok(v)`):
1. Lowera `v` → `payload_val`.
2. `box = kata_rt_store_sum_result(tag, payload_val)` → `box_ptr`.
3. Retorna `box_ptr`.

**Match em Sum com payload**:
1. `tag = kata_rt_sum_tag_int(scrutinee_val)`.
2. Branch chain por tag.
3. No body do arm, `payload = load(scrutinee_val, offset=8)` (após tag).
4. Bind payload ao pattern.

#### Lower `?` (desugared para Match + Return)

O desugar já produziu `Match + Return`. O codegen lowera Match normalmente e
Return aloca na caller's arena.

#### Lower `|` (desugared para Match)

O desugar já produziu `Match`. O codegen lowera Match normalmente.

#### ARC pass

Após o lowering, o ARC pass consulta a `MetadataTable` para inserir
`incref`/`decref`:

1. Para cada `Arc<T>` allocation (`kata_rt_alloc_arc`): insert `incref` após
   criação (refcount = 1).
2. Para cada cópia de um `Arc<T>` (assignment, passagem de argumento): insert
   `incref`.
3. Para cada drop de um `Arc<T>` (fim de escopo, retorno sem uso): insert
   `decref`.

O ARC pass é opcional neste PRD — pode ser stub (sem incref/decref, leak
aceitável em Fio 3+4+9). A maquinaria de escape analysis é o que importa; o
ARC pass automático pode ser refinado na Zeladoria 2.

#### Fiber integration

Cada Action corre numa fiber wasmtime-fiber:

1. **Entry point**: cria fiber principal, inicia scheduler.
2. **Action call**: cria fiber nova (ou reusa pool), passa caller_arena.
3. **Yield**: não há yield em Fio 3+4+9 (sem canais bloqueantes). A infraestrutura
   existe para Fio 11.
4. **Fiber struct**: stack pointer, arena handle, state (Running/Blocked/Done).

## Sintaxe

### Tokens (lexer)

Novos tokens a adicionar:

```
"loop"     => Token::Loop
"break"    => Token::Break
"continue" => Token::Continue
```

Tokens já existentes (não precisam ser adicionados):

- `Action`, `Var`, `Return`, `Bang` (`!`), `Question` (`?`), `Pipe` (`|`),
  `Semicolon` (`;`), `LBracket` (`[`), `RBracket` (`]`), `LBrace` (`{`),
  `RBrace` (`}`)

`for` é adiada para Fio 7+8.

### Gramática (extensões)

```
item        ::= sig lambda_clause*
              | action_decl
              | data_decl
              | enum_decl
              | ...

action_decl ::= 'action' ident params? ':' ret_type
                INDENT action_stmt+ DEDENT

action_stmt ::= expr ';'               -- statement (computação local)
              | expr                    -- retorno implícito (última sem ;)
              | 'return' expr            -- early return
              | 'var' ident ':=' expr    -- binding mutável
              | 'loop' INDENT action_stmt+ DEDENT
              | 'break'
              | 'continue'

expr        ::= ... (existente)
              | ident '!' '(' tuple ')'  -- action call
              | 'return' expr             -- early return
              | 'loop' INDENT stmt+ DEDENT
              | 'break'
              | 'continue'
              | 'var' ident ':=' expr
              | expr '?'                  -- fail-fast
              | expr '|' expr             -- fallback local

enum_decl   ::= 'enum' ident INDENT variant+ DEDENT
variant     ::= ident                     -- unitária
              | ident '(' type ')'        -- com payload
```

### Precedência

```
|>   — mais baixa que aplicação, left-assoc (já existe)
|    — mais baixa que aplicação, left-assoc (fallback)
?    — sufixo (mais alto que |, mais baixo que aplicação)
!    — sufixo (action call, mais alto que aplicação? — ver abaixo)
;    — separador de statement (fora de delimitadores)
```

`!` em action call: `echo!("msg")` — o `!` é parte da sintaxe de chamada,
parseado pelo parser como `ident ! (tuple)`. Não é um operador pós-fixado
genérico.

## Exemplos

### Action básica com echo

```kata
# examples/hello_action.kata
action greet
    echo!("hello")
    echo!("world")

greet!()
```

DoD: imprime "hello" e "world".

### Action com return e caller's arena

```kata
# examples/action_return.kata
action make_tuple
    let x := 42
    let y := 99
    (x, y)

let t := make_tuple!()
echo!(show t.0)
```

DoD: imprime `42`. A tupla sobrevive à Action via caller's arena.

### Action com loop, var, break

```kata
# examples/loop_action.kata
action contador
    var i := 0
    loop
        echo!(show i)
        i := + i 1
        match > i 5
            True: break
            False: continue

contador!()
```

DoD: imprime 0, 1, 2, 3, 4, 5.

### Action com ? (fail-fast)

```kata
# examples/question_action.kata
action validar
    let x := PositiveInt 42 ?
    echo!(show x)

validar!()
```

DoD: imprime `42`. `?` desempacota `Ok(42)`.

### Result com | (fallback)

```kata
# examples/fallback.kata
let x := PositiveInt 25 | 25
echo!(show x)
```

DoD: imprime `25`. `|` desempacota `Ok(25)`.

### Match em Sum com payload

```kata
# examples/match_payload.kata
match Result::Ok 42
    Ok(v): echo!(show v)
    Err(e): echo!("erro")
```

DoD: imprime `42`. Match extrai payload de `Ok(42)`.

### Closure com captura

```kata
# examples/closure_capture.kata
let n := 10
let add_n := + _ n
echo!(show (add_n 5))
```

DoD: imprime `15`. `add_n` captura `n`.

### Closure que escapa (Arc<T>)

```kata
# examples/closure_escape.kata
make_adder :: Int => (Int -> Int)
lambda n:
    + _ n

let f := make_adder 10
echo!(show (f 5))
```

DoD: imprime `15`. `f` é uma closure escapada (retornada de `make_adder`).
Captures promovidas para `Arc<T>`.

### TRMA (adiado)

TRMA foi adiada para Zeladoria pós-Fio 3+4+9. Ver "Não Inclui".

### panic!

```kata
# examples/panic.kata
action crash
    panic!("estado impossível")

crash!()
```

DoD: aborta com mensagem "estado impossível".

## Prelude

O prelude ganha definições de `Result` e `Optional`:

```kata
# stdlib/core.kata (extensão)
enum Result
    Ok(T)
    Err(E)

enum Optional
    Some(T)
    None
```

Estes enums são genéricos (type params posicionais). O prelude também registra
`Ok`, `Err`, `Some`, `None` como construtores (funções de primeira classe) no
DispatchTable.

**`panic!` e `assert!`** são builtins do compilador, não stdlib. `panic!`
é registrada no DispatchTable como Action builtin com `ffi_symbol =
Some("kata_rt_panic")` (FfiSymbol::Panic em kata-core). `assert!(cond, msg)`
é desugared pelo typeck em Guard + Panic: `match cond { True: Unit, False:
panic!(msg) }`.

## Definition of Done

Os DoDs estão agrupados por **fase de implementação** (ordem de dependência),
não por fio. Cada fase depende apenas das anteriores.

### Fase 1 — Actions básicas ✅

1. `kata run examples/hello_action.kata` imprime "hello" e "world". ✅
2. `;` distingue computação local de retorno. Action com `;` no último
   statement retorna `Unit`. ✅
3. `var` permite reatribuição. `let` em Action é imutável. ✅
4. `var` fora de Action produz erro de parser. ✅
5. `return` fora de Action produz erro de parser. ✅
6. `?` fora de Action produz erro de typeck (desugar no typeck precisa do
   tipo da expressão para distinguir `Result` de `Optional`).

### Fase 2 — return, var, ; semântica ✅

7. `kata run examples/action_return.kata` imprime `42` (tupla retorna via
   caller's arena sem use-after-free). ✅

### Fase 3 — Caller's Arena ✅

8. Action retorna coleção (tupla) sem use-after-free. O valor sobrevive à
   destruição da arena local da Action. ✅
9. `;` statements não vazam ponteiros para a caller's arena — computação
   local é liberada no epílogo. ✅
10. Aninhamento: Action A chama Action B. B retorna valor na arena de A.
    A retorna valor na arena do caller de A. Sem use-after-free em nenhum
    nível. ✅ (validado por `examples/action_stress.kata` — 3 Actions encadeadas)

### Fase 4 — loop, break, continue ✅

11. `kata run examples/loop_action.kata` imprime 0-5 (loop com var, break). ✅
12. `break` sai de `loop`. `continue` próxima iteração. ✅
13. `loop`/`break`/`continue` fora de Action produzem erro de parser. ✅

### Fase 5 — Sum com payload ✅

14. `Result::Ok 42` constrói Sum com payload. Match extrai payload. ✅
15. `Optional::Some 42` e `Optional::None` funcionam. ✅
16. Match em 3+ variantes (general case) executa sem trap. ✅
17. Sum com payload é sempre ponteiro (box 8 bytes). Invariante de codegen. ✅
18. `kata_rt_store_sum_result` e `kata_rt_sum_tag_int` implementados no runtime. ✅

### Fase 6 — Result/Optional no prelude

19. `Result::(T, E)` com type params posicionais resolve no typeck. ✅

### Fase 7 — ? (fail-fast)

20. Action com `?` desempacota `Result`/`Optional` corretamente. ✅
21. `?` desugared para Match + Return no typeck — TAST nunca contém `Question`. ✅

### Fase 8 — | (fallback)

22. `|` fallback desempacota `Ok(v)`/`Some(v)`, avalia direita se `Err`/`None`.
23. `|` funciona em funções puras e Actions.
24. `|` desugared para Match no typeck — TAST nunca contém `PipeFallback`.
25. `effect = Puro` em `|` fallback (coalescência é pura — não aborta fluxo).

### Fase 9 — panic!, assert!

26. `panic!("msg")` aborta com mensagem.
27. `assert!(cond, "msg")` verifica condição, panic se falsa.

### Fase 10 — Fibers

28. Actions executam em fibers wasmtime-fiber. Cada fiber tem sua própria
    stack e arena local.
29. Scheduler struct existe (mesmo que single-fiber em Fio 3). `run_queue`,
    `current_fiber` presentes.
30. Yield infrastructure existe (mesmo que não usado — sem canais em Fio 3).

### Fase 11 — Proibição de recursão

31. Proibição de recursão em Actions: `RecursiveAction` error se Action chama
    a si mesma (direta ou indireta).

### Fase 12 — Closures com captura

32. `let add_n := + _ n` captura `n` do escopo externo.
33. `collect_captures` coleta free variables do body do lambda.

### Fase 13 — Escape analysis

34. Closure retornada por função pura escapa para heap (`Arc<T>`).
35. Escape analysis 4 passes marca `escape` corretamente:
    - NãoEscapa para closures locais.
    - EscapaParaHeap para closures retornadas.
36. `CaptureStorage` Stack → Heap promoção funciona.
36a. Closure que escapa por caminho não-óbvio (ex: `let t := (f, 42)` onde
     `f` é closure) é marcada `EscapaParaHeap` — não há falso-negativo que
     cause use-after-free.

### Fase 14 — Arc<T> + FnValueCall

37. `FnValueCall` (call_indirect com CaptureBox) funciona.
38. `kata_rt_alloc_arc`, `kata_rt_incref`, `kata_rt_decref` implementados.

### Fase 15 — ARC pass (stub aceitável)

39. ARC pass insere incref/decref nos pontos apropriados (ou stub aceitável
    — leak em Fio 3+4+9, refino na Zeladoria 2).

### Fase 16 — Reservada (TRMA adiada)

~~40. `@associative(0)` em `+` habilita TRMA. `soma 1000000` executa sem stack~~
~~    overflow — sem TRMA, este valor causa stack overflow; com TRMA, executa~~
~~    normalmente via recursão de cauda com acumulador.~~
~~41. TRMA só funciona com auto-recursão direta. Recursão mútua não é otimizada.~~

TRMA foi adiada para Zeladoria pós-Fio 3+4+9 (ver "Não Inclui").

### Geral

40. Manual atualizado se implementação divergiu do PRD. Seções afetadas: §4.x
    Actions, §4.x Enum com payload, §4.x Closures.
41. `Effect` não é ativado neste PRD — campo continua `Puro` em todos os nós.
    Pureza é garantida por regra de tipo: funções puras não podem conter
    `ActionCall` (erro de tipo, não de efeito). Revisitar em Fio 11 quando o
    scheduler precisar rastrear `Spawn`/`ChannelOp`.

## Não Inclui

- `for` (iteração sobre ITERABLE) — adiada para Fio 7+8 (exige interfaces
  e coleções como tipos)
- Variantes predicadas (enum com guard no payload) — adiada para Fio 6
  (reusa maquinaria de refined types)
- Módulos/import/export (Fio 10)
- Comptime/@cache_strategy (Fio 12)
- Dict/Set/HAMT (Fio 13)
- @log/@test/Test Runner (Fio 14)
- AOT/REPL (Fio 15)
- CSP/canais/fork!/select/timeout (Fio 11) — fibers existem mas sem canais
- @parallel (multiprocess — Fio 11)
- Interfaces/generics/monomorph (Fio 7)
- Coleções List/Array/Range como tipos (Fio 8)
- Stream fusion map/filter/fold (Fio 8)
- Tipos refinados/ascription de expressão (Fio 6)
- Structs com campos/field access (Fio 5) — Ty::Tuple já existe, sem .N
- TRMA (Tail Recursion Modulo Associativity) — adiada para Zeladoria
  pós-Fio 3+4+9. `@associative` já existe no resolution (parseado e
  resolvido desde Fio 1); o TRMA pass no `kata-optimizer` é ortogonal a
  este PRD (não depende de Actions, Sum com payload, nem closures).
  Será implementado com teste que força stack overflow sem TRMA
  (`soma 1000000`) e executa normalmente com TRMA.

## Arquitetura

### Pipeline da Action

```
Item::ActionDecl { name, params, ret, body }
    │
    ▼
kata-resolution
  registra "name" no DispatchTable com is_action=true
  preserva body no ActionDef
    │
    ▼
kata-inference
  infer_action(action_def, table, enum_registry)
  → TypedAction { name, param_types, ret_ty, body }
  verifica proibição de recursão
  desugar ? e | para Match + Return
  effect permanece Puro (não ativado neste PRD)
    │
    ▼
kata-codegen
  declara função "name" no JITModule com ABI estendido:
    (caller_arena: i64, arg1, arg2, ...) -> ret_ty
  prólogo: local_arena = kata_rt_arena_create()
  lowera body: statements com ; na local_arena,
              return/retorno implícito na caller_arena
  epílogo: kata_rt_arena_destroy(local_arena)
  calls to "name!" passam caller_arena = local_arena do caller
```

### Pipeline do Sum com payload

```
Expr::VariantQual { enum_name: "Result", variant: "Ok" }
  com argumento 42
    │
    ▼
kata-inference
  resolve variant: Ok é variante de Result com payload T
  infere tipo: Result::(Int, E)
    │
    ▼
kata-codegen
  lowera payload: 42 → i64 (SMI)
  kata_rt_store_sum_result(tag=0, payload=42) → box_ptr
  box_ptr é o valor do Result::Ok(42)

Match em Result:
  scrutinee_val = box_ptr
  tag = kata_rt_sum_tag_int(scrutinee_val)
  brif tag == 0 → Ok arm (payload = load(scrutinee_val, offset=8))
  brif tag == 1 → Err arm
```

### Pipeline do ? (desugar)

```
let x := PositiveInt 42 ?
    │
    ▼ desugar_question (typeck)
Expr::Let { x, Match (PositiveInt 42) {
    Ok(v) => v,
    Err(e) => Return(Err(e))
} }
    │
    ▼ infer + codegen (Match + Return normais)
```

### Pipeline do | (desugar)

```
PositiveInt 25 | 25
    │
    ▼ desugar_pipe_fallback (typeck)
Match (PositiveInt 25) {
    Ok(v) => v,
    Err(_) => 25
}
    │
    ▼ infer + codegen (Match normal)
```

### Pipeline da closure com captura (escape analysis)

```
let n := 10
let add_n := + _ n        # lambda captura n
    │
    ▼ desugar_hole (typeck — já existe em Fio 2)
Expr::Lambda { patterns: [x], body: + x n }
    │
    ▼ infer
TypedExprKind::Lambda { clauses, captures: [CaptureInfo { n, Int, Stack }] }
    │
    ▼ escape analysis (4 passes)
Pass 0: add_n é atribuído a let — não retorna de função, NãoEscapa
  (se fosse retorno de função: EscapaParaHeap, captures → Heap/Arc)
    │
    ▼ codegen
  captures passadas como argumentos extras na função anônima
  (se NãoEscapa: direto na stack; se EscapaParaHeap: via Arc<ClosureBox>)
```

### Pipeline do TRMA (adiado)

TRMA foi adiada para Zeladoria pós-Fio 3+4+9. Ver "Não Inclui".

## Riscos

1. **wasmtime-fiber dependency**: wasmtime-fiber é uma dependência externa.
   Verificar compatibilidade com Cranelift 0.133 (mesmo ecossistema wasmtime).
   Se incompatível, alternativa: implementar fibers com `genawaiter` ou
   coroutines manuais (stack switching via assembly). Risco baixo —
   wasmtime-fiber é parte do mesmo projeto que Cranelift.

2. **Caller's arena ABI**: Adicionar um parâmetro implícito a toda Action muda
   o ABI. O codegen precisa passar o handle corretamente em toda chamada de
   Action. Se uma Action é chamada sem o handle (bug no codegen), a arena do
   caller é inválida → crash. Testar com Actions aninhadas (A chama B chama C).

3. **Fibers + arena**: Cada fiber tem sua arena local. Se uma fiber cede
   (yield, Fio 11) com ponteiros para sua arena, outra fiber não pode acessar.
   Em Fio 3 (sem yield), não há problema. Mas a infraestrutura precisa estar
   correta para Fio 11 não introduzir bugs sutis.

4. **Sum com payload no codegen**: O codegen atual só suporta Boolean
   (icmp_imm 0/1). Sum com payload exige extrair tag (`kata_rt_sum_tag_int`) e
   payload (load offset). O pattern matching em `_match.rs` e `pattern.rs`
   precisa ser estendido. Risco: esquecer de extrair payload em algum caminho.

5. **Escape analysis correctness**: 4 passes é complexo. Se o Pass 0 marca
   `EscapaParaHeap` mas a closure não escapa de fato, há overhead de Arc<T>
   desnecessário. Se o Pass 1 não detecta um escape, há use-after-free. Testar
   com casos de edge: closures aninhadas, closures em listas, closures em
   tuplas.

6. **TRMA**: adiada para Zeladoria pós-Fio 3+4+9. Sem risco neste PRD.

7. **`?` e `|` desugar para Match**: O desugar produz Match com braços
   específicos. Se o enum tem mais de 2 variantes, o desugar de `|` precisa
   saber qual é a "cauda" (última variante) e quais são as "não-cauda". O
   EnumRegistry precisa expor esta informação.

8. **Escala do PRD**: Três fios combinados. Muitos novos variants em `Expr`,
   `TypedExprKind`, `FfiSymbol`. Novos módulos (escape.rs, fiber.rs,
   scheduler.rs). Risco de tentar fazer tudo de uma vez. Abordagem incremental
   (DoDs agrupados por fase de implementação): (1) Actions básicas, (2) return/
   var/;, (3) caller's arena, (4) loop/break/continue, (5) Sum com payload,
   (6) Result/Optional, (7) ?, (8) |, (9) panic!/assert!, (10) fibers,
   (11) proibição de recursão, (12) closures com captura, (13) escape analysis,
   (14) Arc<T>, (15) ARC pass. (TRMA adiada.)

9. **Parser ambiguidade `|` vs `|>`**: `|` é fallback, `|>` é pipeline. O
   lexer já produz tokens distintos (`Pipe` vs `PipeForward`). O parser
   distingue por lookahead. Risco baixo — já funcionam em Fio 2 (|>).

10. **Parser `!` em action call vs `!` em outras posições**: `!` só aparece
    em action calls (`echo!(...)`). Não é um operador genérico. O parser
    produz `ActionCall` quando vê `ident ! (`. Se `!` aparece em outra
    posição, erro de sintaxe.