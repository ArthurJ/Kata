# PRD — First-Class Actions

**Status:** ✅ Concluído
**Data:** 2026-07-22
**Depende de:** DispatchTable ✅ (`is_action: bool`), Typeck ✅, Codegen ✅ (ABI de Action), `Fork` ✅, Escape analysis ✅, Recursion check ✅
**Não depende de:** `type!()` (PRD separado,受益a deste PRD)

## 1. Objetivo

Permitir que Actions sejam valores de primeira classe — referenciadas sem
invocação, armazenadas em variáveis, e passadas como parâmetros para outras
Actions. Isto habilita o pattern de dispatch/strategy:

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)
    dispatcher!(worker_b, 42)
```

Hoje isso não compila — `worker_a` não é um valor, é um nome resolvido
somente em `ActionCall { callee: String }`. Com first-class actions,
`worker_a` sem `!()` é uma referência que carrega o tipo `Action(Int) => Unit`.

## 2. Sintaxe

### 2.1. Referência vs invocação

```kata
worker_a           # referência — valor do tipo Action(Int) => Unit
worker_a!(42)      # invocação — executa a action, retorna Unit
```

Hoje `worker_a` sem `!()` em contexto de Action é erro (unbound name ou
ignorado). Com este PRD, o parser já produz `Expr::Ident { name: "worker_a" }`
— não há mudança no parser. A mudança é no typeck: `Ident` cujo nome está no
DispatchTable com `is_action: true` recebe `Ty::Action(params, ret)` em
vez de erro.

### 2.2. Sintaxe de tipo

```
Action(Param1, Param2, ...) => Ret
```

Espelha a sintaxe de assinatura de actions (`action nome (p::T, ...) => Ret`),
sem os nomes dos params:

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
```

### 2.3. `fork!` recebe Action como valor

```kata
# Hoje (string extraída do Ident):
fork!(worker_a, (42,))

# Com este PRD (valor tipado):
fork!(worker_a, (42,))
```

A sintaxe não muda — `worker_a` continua sendo `Expr::Ident`. A mudança é
semântica: a inference de `fork!` recebe um `TypedExpr` com `ty: Action`
em vez de extrair uma string do `Ident`. O codegen extrai o fn_ptr do
TypedExpr em vez de fazer lookup por nome.

## 3. Semântica

### 3.1. `Ty::Action`

Nova variante em `Ty`:

```rust
pub enum Ty {
    ...
    /// Action como valor first-class.
    /// params: tipos dos parâmetros (sem nomes).
    /// ret: tipo de retorno.
    Action(Vec<Ty>, Box<Ty>),
}
```

Separada de `Ty::Function` porque as ABIs são semanticamente diferentes
(D1):
- `Function`: `(captures_ptr, arg1, ...) -> ret` — pura, sem scheduler
- `Action`: `(fiber_arena, caller_arena, args_ptr) -> i64` — impura, scheduler M:N

### 3.2. Referência de Action

Quando o typeck encontra `Ident { name }` onde `name` está no
DispatchTable com `is_action: true`:

1. Produz `TypedExpr` com `ty: Ty::Action(param_types, ret_ty)`
2. O valor em runtime é o `fn_ptr` (i64) da Action — obtido via
   `GlobalValue::Symbol` no codegen
3. O nome é preservado no TAST para def-use tracing (recursion check,
   tree shaking)

### 3.3. Invocação indireta

`f!(args)` onde `f` é uma variável com `ty: Ty::Action`:

1. O typeck trata como invocação de Action — valida que `args` matcham
   os param types de `f.ty`
2. O codegen emite `call_indirect` com o fn_ptr da variável, passando
   `fiber_arena` + `caller_arena` + `args_ptr` (mesma ABI de ActionCall
   direto)

Hoje o codegen faz `ctx.kata_refs.get(&key)` — lookup estático por
`(name, params, ret)`. Para invocação indireta, o fn_ptr já está na
variável SSA — não precisa lookup. Mas o codegen precisa saber que é
uma Action (não uma Function) para emitir a ABI correta.

### 3.4. Parâmetros de Action para Action

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)
```

Dentro de `dispatcher`, `job` é um parâmetro com `ty: Ty::Action([Int], Unit)`.
A invocação `job!(payload)` é indireta — o fn_ptr vem do parâmetro, não de
um nome estático. O codegen emite `call_indirect` com a ABI de Action.

### 3.5. `Fork` recebe Action como valor

Hoje `fork!` extrai o nome da Action do `Expr::Ident` e faz lookup no
DispatchTable. Com first-class actions, `fork!` recebe um `TypedExpr`
com `ty: Ty::Action`. O codegen:

1. Lowera o `TypedExpr` da action → `fn_ptr` (i64)
2. Lowera os args → `args_ptr`
3. Chama `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`

Sem lookup por nome — o fn_ptr já está disponível. Se o argumento é
`Ident { name: "worker_a" }`, o codegen lowera o `Ident` para
`GlobalValue::Symbol` (igual `Fork` faz hoje). Se é uma variável
`let f := worker_a; fork!(f, (42,))`, o codegen usa o fn_ptr da
variável.

### 3.6. Restrições

| Operação | Status | Racional |
|---|---|---|
| `let f := worker_a` | ✅ Permitido | Binding direto — nome rastreável |
| `let f := worker_a; let g := f; g!()` | ✅ Permitido | Cadeia de bindings — nome rastreável |
| `dispatcher!(worker_a, 42)` | ✅ Permitido | Action como param de Action |
| `f!(42)` onde `f` é param de Action | ✅ Permitido | Invocação indireta |
| `fork!(worker_a, (42,))` | ✅ Permitido | Fork recebe Action como valor |
| Action como campo de `data` | ❌ Proibido | Actions não são informações — `data` é reino de dados, não de comportamento |
| Action via canal (`ch!((worker_a, 42))`) | ❌ Proibido | Actions não são informações — canais transportam dados, não comportamento. Actions são importáveis, não precisam ser transportadas |
| Action como parâmetro de função pura | ❌ Proibido | Funções puras não podem invocar actions nem acessar dados mutáveis. Passar Action para função não tem uso — a função não pode chamá-la |
| Interface `CALLABLE` | ❌ Não existe | Functions e Actions são reinos separados com ABIs diferentes. Não há motivo para unificar |

### 3.7. Proibição de Action em `data` e canal — enforcement

O typeck rejeita `Ty::Action` em posições de `data` e canal:

- `data Worker (job :: Action(Int) => Unit)` → erro: "Action não é
  permitida em data"
- `ch!((worker_a, 42))` → erro: "Action não é permitida em canal"
- `Sender::Action(...)` / `Receiver::Action(...)` → erro

A verificação é no typeck: quando um `Ty::Action` aparece em
`StructConstruct`, `ChannelSend`, `ListLit`, `ArrayLit`, ou tipo de
campo de struct, é erro.

## 4. Recursion check — def-use interprocedural

### 4.1. Problema

Hoje `build_call_graph` em `recursion.rs` analisa `ActionCall { callee }`
por nome — aresta estática `g → callee`. Com first-class actions,
`f!(payload)` dentro de `dispatcher` é uma invocação indireta — `f` é
parâmetro, não nome estático. O checker não sabe quais actions fluem
para `f` sem analisar os call sites.

### 4.2. Solução: propagação de call sites

No call site `dispatcher!(worker_a, 42)`:

1. O typeck sabe que `worker_a` (Ident com `ty: Action`) flui para o
   parâmetro `job` de `dispatcher`
2. Dentro de `dispatcher`, `job!(payload)` é uma invocação do parâmetro
3. O recursion checker registra aresta `dispatcher → worker_a` — porque
   `worker_a` flui para um param que é invocado

Se `worker_a` invoca `dispatcher` (direta ou indiretamente), o ciclo é
detectado.

### 4.3. Algoritmo

1. Para cada call site `target!(args)` onde `target` é Action definida
   pelo usuário:
   - Para cada arg que é `Ident { ty: Action }` (referência direta a
     Action `A`), e o param correspondente em `target` é invocado
     indiretamente dentro de `target`:
     - Registrar aresta `target → A`

2. "Invocado indiretamente" = o param aparece em `ActionCall` onde o
   callee é o nome do param (não um nome estático). O typeck já
   distingue: `ActionCall { callee: "job" }` onde `job` é param, vs
   `ActionCall { callee: "worker_a" }` onde `worker_a` é nome estático.

3. Para cadeias de bindings (`let g := f`), o def-use resolve o nome
   original: `g!()` → aresta para o nome que flui para `f` no call site.

### 4.4. Limitação

Se `worker_a` é passada para `dispatcher` indiretamente (via outra
action intermediária), o def-use precisa propagar transitivamente.
Exemplo:

```kata
action intermediaria (f :: Action(Int) => Unit, x :: Int) => Unit
    dispatcher!(f, x)

action main => Unit
    intermediaria!(worker_a, 42)
```

Aqui `worker_a` flui para `f` em `intermediaria`, que flui para `job`
em `dispatcher`. O checker precisa propagar: `main → intermediaria`,
`intermediaria → dispatcher` (via param `f`), `dispatcher → worker_a`
(via param `job`). Se `worker_a → intermediaria` formaria um ciclo.

O algoritmo propaga nomes literais dos call sites para os params que
eles preenchem, transitivamente, até fixpoint. Se em algum ponto o
nome não é rastreável (ex: vem de `match` com múltiplos branches), o
checker trata conservativamente — marca todas as actions do branch
como potenciais targets.

### 4.5. Conservadorismo em match

```kata
let f := match cond
    True: worker_a
    False: worker_b
f!()
```

O def-use vê que `f` pode ser `worker_a` ou `worker_b`. Registra
arestas para ambos. Se algum forma ciclo, detecta. Se nenhum forma,
não há falso positivo — apenas arestas conservativas.

## 5. Tree shaking

### 5.1. Referência de Action como aresta

Hoje `collect_refs` reconhece arestas via:
- `Closure { callee: Ident{name}, ffi_symbol: None }` → função
- `ActionCall { callee, ffi_symbol: None }` → action
- `Fork { action_name }` → action

Com first-class actions, uma nova aresta: `Ident { name, ty: Action }` —
referência sem invocação. O shaker marca a action como alcançável porque
o fn_ptr pode ser invocado indiretamente.

### 5.2. Invocação indireta

`f!(args)` onde `f` é variável (não nome estático) não cria aresta
direta. Mas o shaker já marcou a action original como alcançável no
ponto da referência (`let f := worker_a` ou `dispatcher!(worker_a, 42)`).

### 5.3. Projeção no codegen

O codegen declara todas as actions como `FuncId` em `kata_ids`. O tree
shaker remove actions não alcançadas do `TypedModule.actions`. O codegen
só declara as que sobrevivem. Referências a actions removidas não existem
— o shaker garantiu que toda action referenciada está no módulo.

## 6. Escape analysis

### 6.1. fn_ptr como i64

O fn_ptr de uma Action é um i64 — não é heap-allocated, não precisa
ARC. Mas o tipo carrega informação de ABI (`Ty::Action`), que o codegen
precisa para emitir a invocação correta.

### 6.2. Escape

O fn_ptr pode escapar? Com as restrições deste PRD:
- Não entra em `data` — proibido
- Não entra em canal — proibido
- Não entra em função pura — proibido
- Pode ser armazenado em `let` — local, não escapa
- Pode ser passado como param de Action — escapa para o escopo da Action
  receptora

"Escapar" aqui significa "sobreviver além do escopo atual". Um param de
Action vive no escopo da Action receptora — que é uma Action, no mesmo
reino. Não há cruzamento de reinos.

### 6.3. `EscapeTarget`

O `EscapeTarget` do fn_ptr é `Local` (vive no fiber atual). O fn_ptr
não é alocado na arena — é um i64 inline. Mas o `args_ptr` da invocação
indireta precisa ser alocado na arena apropriada (Local ou Caller),
determinado pelo escape analysis dos args, não do fn_ptr.

## 7. Codegen

### 7.1. Lowering de referência

`Ident { name: "worker_a", ty: Ty::Action(...) }`:

1. Lookup `FuncId` em `kata_ids` por `(name, param_types, ret_ty)`
2. `GlobalValue::Symbol` → `fn_ptr` (i64)
3. Retorna `fn_ptr` como valor SSA

Mesmo mecanismo que `Fork` usa hoje para obter fn_ptr.

### 7.2. Lowering de invocação indireta

`ActionCall { callee: "f", args }` onde `f` é variável com `ty: Action`:

Hoje `ActionCall` faz lookup estático em `kata_refs`. Com invocação
indireta, o codegen precisa distinguir:

- `callee` é nome estático (action definida pelo usuário) → lookup em
  `kata_refs` + `call` direto (como hoje)
- `callee` é variável/param com `ty: Action` → `call_indirect` com fn_ptr
  da variável

A distinção: o typeck marca `ActionCall` com `indirect: bool` ou o
codegen verifica se `callee` está em `kata_refs` (estático) vs
`var_map` (indireto).

### 7.3. ABI de invocação indireta

```
call_indirect(fn_ptr, [fiber_arena, caller_arena, args_ptr])
```

Mesma ABI de ActionCall direto dentro de Action (`scheduler_mode: false`):
`(fiber_arena, caller_arena, args_ptr) -> i64`. O fn_ptr vem da variável,
não de `kata_refs`.

Se ret_ty == Float: `bitcast(F64 ← I64)` (mesmo tratamento de hoje).

### 7.4. `Fork` com Action como valor

O codegen de `Fork` já extrai fn_ptr via `GlobalValue::Symbol`. A mudança:
se o arg é uma variável (não `Ident` direto), o fn_ptr vem da variável SSA
em vez de `GlobalValue::Symbol`.

## 8. Pipeline — componentes afetados

```
# Core — tipo
crates/kata-core/src/ty.rs                    # Ty::Action(Vec<Ty>, Box<Ty>)
crates/kata-core/src/shape.rs                 # TypeShape para Ty::Action (Func shape)

# AST — sem mudança (Ident já existe, ActionCall já existe)
# Parser — sem mudança (worker_a já parseia como Ident)

# Resolution
crates/kata-resolution/src/types.rs           # assinatura Action(Int) => Unit em tipo de param

# Inference
crates/kata-inference/src/infer/expr.rs       # Ident com is_action → Ty::Action
crates/kata-inference/src/infer/action_call.rs # invocação indireta (callee é variável)
crates/kata-inference/src/infer/action_call.rs # fork! recebe TypedExpr, não string
crates/kata-inference/src/infer/constructors.rs # proibir Ty::Action em StructConstruct
crates/kata-inference/src/infer/csp.rs        # proibir Ty::Action em ChannelSend
crates/kata-inference/src/infer/expr.rs       # proibir Ty::Action em posição de função pura

# Recursion check
crates/kata-inference/src/infer/recursion.rs  # def-use interprocedural (propagar nomes de call sites)

# Monomorphization
crates/kata-monomorph/src/lib.rs              # Ty::Action em instantiate_kind

# Tree shaking
crates/kata-tree-shaking/src/lib.rs           # Ident { ty: Action } como aresta

# Codegen
crates/kata-codegen/src/lowering/expr.rs      # Ident com ty: Action → fn_ptr
crates/kata-codegen/src/lowering/action_call.rs # call_indirect para invocação via variável
crates/kata-codegen/src/lowering/csp.rs       # fork! recebe fn_ptr de TypedExpr

# Escape analysis
crates/kata-core/src/escape.rs                     # fn_ptr de Action é Local (i64 inline)

# Testes
crates/kata-driver/tests/                      # E2E: dispatch/strategy, fork! com valor
crates/kata-parser/tests/                      # parser: sem mudança (confirmar)

# Docs
docs/Kata-lang-manual.md                       # Actions como first-class values
docs/sintaxe-mapa.md                           # tipo Action(Params) => Ret
```

## 9. Fora do escopo

- **Action em `data`** — proibido. Actions não são informações.
- **Action via canal** — proibido. Actions não são informações.
- **Action como parâmetro de função pura** — proibido. Funções não
  podem invocar actions.
- **Interface `CALLABLE`** — não existe. Functions e Actions são reinos
  separados.
- **`type!()`** — introspecção de tipos. PRD separado. Beneficia deste
  PRD: `type!(f)` onde `f :: Action` retorna a assinatura formatada.
- **Overloading de Actions** — múltiplas actions com mesmo nome e params
  diferentes. Já existe no DispatchTable (`is_action: true`), mas a
  resolução de referência ambígua (qual overload?) fica para depois.

## 10. DoDs (Definitions of Done)

1. `let f := worker_a` produz variável com `ty: Ty::Action([Int], Unit)`.
2. `f!(42)` onde `f` é variável Action invoca a action original.
3. `dispatcher!(worker_a, 42)` passa `worker_a` como valor Action.
4. `job!(payload)` dentro de `dispatcher` invoca indiretamente via param.
5. `fork!(worker_a, (42,))` funciona com `worker_a` como valor tipado.
6. `fork!(f, (42,))` onde `f := worker_a` funciona (fn_ptr da variável).
7. `Action(Int) => Unit` é sintaxe de tipo válida em assinaturas.
8. `data Worker (job :: Action(Int) => Unit)` → erro compile-time.
9. `ch!((worker_a, 42))` → erro compile-time.
10. `soma(worker_a)` onde `soma` é função pura → erro compile-time.
11. Recursion indireta detectada: `a → dispatcher → a` via param.
12. Recursion indireta via cadeia de intermediárias detectada.
13. `match` com múltiplas actions: todas registradas como arestas.
14. Tree shaking: action referenciada mas não invocada diretamente é
    preservada.
15. Tree shaking: action não referenciada é removida.
16. `cargo test --workspace --no-fail-fast` passa sem regressão.
17. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 11. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `Ty::Action` separada de `Ty::Function` | ABIs semanticamente diferentes. Function: `(captures, args) -> ret`. Action: `(fiber_arena, caller_arena, args_ptr) -> i64`. Reusar `Function` seria reusar código para coisas semanticamente diferentes. |
| D2 | Referência sem `!()` é valor | `worker_a` é referência, `worker_a!()` é invocação. Mesma distinção de `f` vs `f()` em linguagens com function pointers. O parser já produz `Ident` — mudança é no typeck. |
| D3 | `fork!` recebe Action como `TypedExpr` | Hoje `fork!` extrai string do `Ident`. Com first-class, recebe valor tipado. Se o arg é `Ident` direto, codegen usa `GlobalValue::Symbol`. Se é variável, usa fn_ptr da variável. |
| D4 | Tree shaker reconhece `Ident { ty: Action }` como aresta | Referência sem invocação ainda torna a action alcançável — pode ser invocada indiretamente. |
| D5 | Action em `data` proibida | `data` é reino de informação. Actions são comportamento, não informação. Misturar confunde categorias. |
| D6 | Action via canal proibida | Canais transportam informação. Actions são importáveis, não precisam ser transportadas. Proibir simplifica def-use e escape analysis. |
| D7 | Action como param de função pura proibida | Funções puras não podem invocar actions nem acessar dados mutáveis. Passar Action para função não tem uso. |
| D8 | Sem interface `CALLABLE` | Functions e Actions são reinos separados com ABIs diferentes. Não há motivo para unificar. Se precisa de callable, escolhe o reino. |
| D9 | Recursion check via def-use interprocedural | Propaga nomes literais dos call sites para params que eles preenchem. Transitivo até fixpoint. Conservativo em `match` (todas as actions do branch são targets). |
| D10 | Escape: fn_ptr é Local (i64 inline) | fn_ptr não é heap-allocated. `EscapeTarget::Local` — vive no fiber atual. Não cruza reinos (não entra em data/canal/função). |

## 12. Riscos

| Risco | Mitigação |
|---|---|
| `call_indirect` no Cranelift não suporta a ABI de Action | Verificar suporte. Se não, hoisting para `call` direto quando o fn_ptr é rastreável a nome estático em compile-time. |
| Def-use interprocedural é complexo demais | Primeira versão: só propaga 1 nível (call site direto). Cadeias de intermediárias são raras — se surgirem, estende. |
| `match` com actions de tipos diferentes | Ty::Action é nominal — `match` requer que todos os branches tenham o mesmo `Ty::Action`. Se tipos diferentes, erro de tipo. |
| Overloading: `worker_a` com múltiplos overloads | Referência ambígua — qual overload? Primeira versão: erro se ambíguo. Depois: resolution por tipo esperado do param. |
| Performance de `call_indirect` vs `call` | `call_indirect` impede inlining. Aceitável — first-class actions são para dispatch dinâmico, não hot path. |

## 13. Exemplos

### 13.1. Dispatch/Strategy

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)   # imprime 43
    dispatcher!(worker_b, 42)   # imprime 44
```

### 13.2. Fork com valor

```kata
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    fork!(f, (42,))
    # fiber spawna worker(42)
```

### 13.3. Seleção por match

```kata
action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    let f := match cond
        True: worker_a
        False: worker_b
    f!(42)
```

### 13.4. Recursion indireta detectada

```kata
action a (f :: Action(Int) => Unit) => Unit
    f!(1)

action b (n :: Int) => Unit
    a!(b)   # ERRO: a → b → a (ciclo detectado via def-use)
```