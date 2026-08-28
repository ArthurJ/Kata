# PRD — Interpretador Tree-Walking sobre TAST

**Status:** Fases 1-6 ✅
**Data:** 2026-08-27
**Depende de:** Pipeline completo até `optimize()` ✅ (lex → parse → resolve → infer → monomorph → optimize)
**Não depende de:** Cranelift, codegen, tree-shaking, comptime

## 1. Objetivo

Adicionar um modo de execução interpretado (tree-walking) que consome a TAST
produzida pelo pipeline existente, sem passar pelo codegen Cranelift. O
interpretador executa o mesmo `TypedModule` que o JIT compila — sem recompilar,
sem linker, sem Cranelift.

### Princípio: a TAST é a fronteira

O pipeline até `optimize()` é compartilhado. O interpretador é um consumidor
adicional do `TypedModule`, paralelo ao `jit_eval` e `aot_emit`:

```
Pipeline::new(source)
    .lex()?.parse(TwoPass)?.resolve()?.desugar().infer()?
    .monomorph().optimize()
    ├── .build_type_table()?.jit_eval()       // JIT (existente)
    ├── .build_type_table()?.aot_emit()       // AOT (existente)
    └── .interpret()                           // Interpretador (novo)
```

O interpretador não re-parseia, não re-infere, não re-resolve. A TAST já tem
tipos resolvidos, dispatch decidido, escape analysis marcado, TRMA aplicado,
stream fusion estruturado. O interpretador só executa.

### Princípio: reusar o runtime

O interpretador chama as mesmas funções C-ABI de `kata-rt` que o codegen chama
via FFI. `kata_rt_list_cons`, `kata_rt_array_alloc`, `kata_rt_channel_send`,
`kata_rt_arena_alloc` — todas são funções Rust `extern "C"` que podem ser
chamadas diretamente de código Rust, sem overhead de FFI boundary. O
interpretador é Rust puro chamando Rust puro.

### Princípio: sem codegen, sem Cranelift

O interpretador não depende de `kata-codegen`. Não cria `JITModule`, não
declara símbolos FFI, não emite CLIF, não finaliza. A crate `kata-codegen` pode
não estar linkada no binário interpretador. Isso reduz o footprint e elimina
a dependência de Cranelift para modo interpretado.

## 2. Modelo

### 2.1. Nova crate: `kata-interp`

```
crates/kata-interp/
  src/
    lib.rs          — API pública: interpret(module, rt_ptr) -> InterpResult
    value.rs        — Value: representação de valores Kata em Rust
    env.rs          — Environment: escopo de variáveis
    eval.rs         — eval(&TypedExpr, &mut Env, &mut Runtime) -> Value
    ffi_dispatch.rs — mapeamento de ffi_symbol → função kata-rt
    actions.rs      — execução de Actions (corpo imperativo)
    csp.rs          — fork!, channels, select, spawn!
```

### 2.2. Value

O interpretador usa `i64` como representação de valor — idêntica ao runtime.
SMI-tagging é preservado: Int pequeno é `(val << 1) | 1`, Float é `f64::to_bits()
as i64`, ponteiros (List, Struct, Text, Array) são `i64` ponteiros para a arena.

Isso permite que o interpretador chame `kata_rt_list_cons(head_i64, tail_i64,
arena)` e receba um `i64` ponteiro que é exatamente o mesmo que o codegen
produziria. O display (`print_result`) funciona sem mudança — recebe `(raw_i64,
ty)` e formata.

```rust
/// Valor no interpretador — i64 cru, mesmo formato do runtime.
type Value = i64;

/// Float: reinterpretar bits.
fn value_to_f64(v: Value) -> f64 {
    f64::from_bits(v as u64)
}

fn f64_to_value(f: f64) -> Value {
    f.to_bits() as i64
}
```

### 2.3. Environment

```rust
pub(crate) struct Env {
    /// Variáveis locais — nome → valor (i64).
    /// Vec<HashMap> para escopo léxico: push em bloco, pop ao sair.
    scopes: Vec<HashMap<String, Value>>,
}
```

`let` faz `scopes.last_mut().insert(name, value)`. `Ident` faz lookup de fora
para dentro. `var` é igual — a diferença é que `Reassign` faz
`scopes.last_mut().insert(name, new_value)` em vez de falhar.

### 2.4. eval — dispatch central

```rust
fn eval(
    expr: &TypedExpr,
    env: &mut Env,
    rt: &mut Runtime,
    arena: i64,           // fiber_arena handle
    caller_arena: i64,    // caller_arena handle
) -> Result<Value, InterpError>
```

O match em `TypedExprKind` cobre todas as variantes. Cada variante chama a
função de runtime apropriada diretamente:

| TypedExprKind | Implementação |
|---|---|
| `IntLit { text }` | SMI inline: `encode_smi(val)` se cabe, senão `kata_rt_tag_int_from_str(ptr, len)` |
| `FloatLit { text }` | `f64_to_value(text.parse())` |
| `TextLit { text }` | Alocar string na arena via `kata_rt_text_from_str` |
| `Unit` | `0` |
| `Ident { name }` | `env.lookup(name)` |
| `Closure { callee, args, ffi_symbol }` | Ver §2.5 |
| `Lambda { clauses, captures, .. }` | Constrói closure value — ver §2.6 |
| `Let { name, value }` | `env.define(name, eval(value))`, retorna 0 |
| `Var { name, value }` | igual a `Let` |
| `Reassign { name, value }` | `env.reassign(name, eval(value))` |
| `Tuple { elements }` | Aloca N×8 na arena, store cada elemento |
| `StructConstruct { values, .. }` | Idêntico a Tuple |
| `FieldAccess { expr, field_index, .. }` | `load ptr + field_index * 8` |
| `IndexAccess { expr, element_index, .. }` | `load ptr + element_index * 8` |
| `ListLit { elements }` | Loop de `kata_rt_list_cons` |
| `ArrayLit { elements }` | `kata_rt_array_alloc(n, arena)` + `kata_rt_array_set` |
| `Match { scrutinee, arms }` | Pattern match — ver §2.7 |
| `Block { stmts }` | Avalia cada stmt, retorna o último |
| `ForIn { iterable, body, .. }` | Itera Cons/Array/Range, avalia body |
| `Map { callback, collection, .. }` | Loop iterando, chama callback via `call_closure` |
| `Filter { .. }` | Loop iterando, predicado via `call_closure` |
| `Fold { .. }` | Loop com acumulador |
| `FusedStream { .. }` | Loop único aplicando estágios em cadeia |
| `RangeLit { .. }` | Constrói range value na arena |
| `TypeAscription { .. }` | No-op em runtime (typeck já validou) |
| `Grouping { inner }` | `eval(inner)` |
| `VariantQual { tag, .. }` | `kata_rt_store_sum_result(tag, 0, arena)` |
| `VariantConstruct { tag, payload, .. }` | `kata_rt_store_sum_result(tag, eval(payload), arena)` |
| `In { item, collection }` | `kata_rt_list_contains` / `kata_rt_array_contains` |
| `Return(expr)` | Propaga via `InterpError::Return(value)` |
| `Break` | Propaga via `InterpError::Break` |
| `Continue` | Propaga via `InterpError::Continue` |
| `Loop { body }` | Loop until Break/Return |
| `HeapSnapshot { .. }` | `kata_rt_get_snapshot(id)` |
| `ConstantBinding { .. }` | Avalia value, define em env global |
| `ChannelSend` / `ChannelRecv` | `kata_rt_channel_send` / `kata_rt_channel_recv` |
| `ChannelCreate` | `kata_rt_channel_create` / `kata_rt_queue_create` / `kata_rt_broadcast_create` |
| `Fork` / `Spawn` | Ver §2.8 (CSP) |
| `Select` | Ver §2.8 (CSP) |
| `TypeOf { expr }` | Retorna `TextLit(ty_display(&expr.ty))` |
| `ActionCall { .. }` | Ver §2.9 |
| `ReceiverFactoryCall` | `kata_rt_broadcast_receiver_create` |
| `BytesLit` | `kata_rt_bytes_from_ptr` |

### 2.5. Closure com ffi_symbol

Quando `ffi_symbol` é `Some`:
1. Avaliar argumentos
2. Despachar para a função C-ABI correspondente via `ffi_dispatch`
3. `ffi_dispatch` é um match em `sym_name` que chama a função Rust diretamente

```rust
fn ffi_dispatch(sym: &str, args: &[Value], rt: &mut Runtime, arena: i64) -> Value {
    match sym {
        "kata_rt_bi_add" => {
            let a = untag_smi(args[0]);
            let b = untag_smi(args[1]);
            // Overflow → BigInt. Verificar overflow antes.
            encode_smi(a + b) // simplificado — ver §2.5.1
        }
        "kata_rt_bi_sub" => { ... }
        "kata_rt_list_cons" => kata_rt_list_cons(args[0], args[1], arena),
        "kata_rt_list_head" => kata_rt_list_head(args[0]),
        "kata_rt_list_tail" => kata_rt_list_tail(args[0]),
        "kata_rt_text_concat" => kata_rt_text_concat(args[0], args[1], arena),
        // ... ~80 símbolos FFI
        _ => panic!("FFI não implementado no interpretador: {sym}"),
    }
}
```

Isso é uma tabela de dispatch. Cada entrada é uma chamada de função Rust direta
— sem overhead de FFI boundary, sem calling convention, sem `unsafe` na maioria
dos casos. O overhead por operação é um match Rust + function call, não uma
`callq` C-ABI.

#### 2.5.1. Aritmética SMI inline

Para operações aritméticas comuns (`+`, `-`, `*`, comparações), o interpretador
pode inlinear o untag/tag sem chamar `kata_rt_bi_add`:

```rust
fn ffi_dispatch(sym: &str, args: &[Value], rt: &mut Runtime, arena: i64) -> Value {
    match sym {
        "kata_rt_bi_add" => {
            let a = untag_smi(args[0]);
            let b = untag_smi(args[1]);
            match a.checked_add(b) {
                Some(result) if fits_smi(result) => encode_smi(result),
                _ => {
                    // Overflow ou BigInt — delegar para runtime
                    kata_rt_tag_int(a + b) // ou BigInt path
                }
            }
        }
        // ... mesmo padrão para sub, mul
        "kata_rt_bi_lt" => {
            let a = untag_smi(args[0]);
            let b = untag_smi(args[1]);
            if a < b { 1 } else { 0 } // Boolean cru
        }
        // ...
    }
}
```

Isso faz aritmética de inteiros pequenos ser **3-4 instruções Rust** — comparável
ao que o codegen JIT produz (que é uma `callq` para `kata_rt_bi_add`). O
interpretador pode ser tão rápido ou mais rápido que o JIT para aritmética
de inteiros pequenos, porque não há calling convention overhead.

### 2.6. Closures (Lambda como valor)

O codegen usa CaptureBox: `fn_ptr | refcount | n_captures | captures[]`. O
interpretador precisa de uma representação diferente — não há fn_ptr JIT.

**Decisão: `Value::Closure` na arena.**

Uma closure é alocada na arena como um struct:
```
offset 0:   tag = CLOSURE_TAG (i64 magic, ex: 0x_CLOSURE_)
offset 8:   lambda_ptr  (ponteiro para &TypedLambdaClause no TypedModule)
offset 16:  n_captures
offset 24:  captures[0..n]
```

O `lambda_ptr` é um ponteiro Rust para as cláusulas da lambda no `TypedModule`.
O interpretador mantém o `TypedModule` vivo (via `Arc`) durante toda a execução.
As captures são `Value` (i64) armazenadas inline.

Para chamar uma closure:
```rust
fn call_closure(
    closure_val: Value,        // ponteiro para o struct na arena
    args: &[Value],
    env: &mut Env,
    rt: &mut Runtime,
    arena: i64,
    caller_arena: i64,
) -> Result<Value, InterpError> {
    let ptr = closure_val as *const u8;
    let lambda_ptr = unsafe { read_unaligned(ptr.add(8) as *const *const TypedLambdaClause) };
    let n_captures = unsafe { read_unaligned(ptr.add(16) as *const i64) };
    let clauses = unsafe { &*lambda_ptr };

    // Reconstruir env com captures
    env.push_scope();
    for i in 0..n_captures {
        let cap_val = unsafe { read_unaligned(ptr.add(24 + i*8) as *const i64) };
        // O nome da capture vem do Lambda.captures[i].name
        env.define(capture_name_i, cap_val);
    }
    // Match args contra patterns da cláusula, eval body
    let result = eval_clauses(clauses, args, env, rt, arena, caller_arena)?;
    env.pop_scope();
    Ok(result)
}
```

### 2.7. Pattern matching

O match da TAST já tem patterns tipados (`TypedPattern`). O interpretador faz
pattern matching sobre o `Value`:

```rust
fn match_pattern(
    pat: &TypedPattern,
    value: Value,
    env: &mut Env,
) -> bool {
    match pat {
        TypedPattern::Literal { text, ty } => {
            // Comparar value com literal parseado
            match ty {
                Ty::Prim(PrimTy::Int) => value == encode_smi(text.parse()),
                Ty::Prim(PrimTy::Float) => value == f64_to_value(text.parse()),
                _ => false,
            }
        }
        TypedPattern::Ident { name, .. } => {
            env.define(name, value);
            true
        }
        TypedPattern::Variant { tag, .. } => {
            // Sum: tag no offset 0 do Result box
            let actual_tag = kata_rt_get_sum_tag(value);
            actual_tag == *tag as i64
        }
        TypedPattern::VariantWithPayload { tag, payload_pat, .. } => {
            let actual_tag = kata_rt_get_sum_tag(value);
            if actual_tag != *tag as i64 { return false; }
            let payload = kata_rt_get_sum_payload(value);
            match_pattern(payload_pat, payload, env)
        }
        TypedPattern::Cons { head_pat, tail_pat, .. } => {
            if value == 0 { return false; } // Nil
            let head = kata_rt_list_head(value);
            let tail = kata_rt_list_tail(value);
            match_pattern(head_pat, head, env) && match_pattern(tail_pat, tail, env)
        }
        TypedPattern::Tuple { element_pats, .. } => {
            for (i, ep) in element_pats.iter().enumerate() {
                let elem = load value + i*8;
                if !match_pattern(ep, elem, env) { return false; }
            }
            true
        }
        // ... outros patterns
    }
}
```

`Match` avalia o scrutinee, percorre os arms, testa cada pattern + guard, e
avalia o body do primeiro arm que casar. `otherwise` (pattern `None`) sempre
casa.

### 2.8. CSP (fork!, channels, select, spawn!)

#### fork!

O scheduler de fibers espera function pointers nativos
(`extern "C" fn(i64) -> i64`). O interpretador não tem function pointers JIT.

**Decisão: trampoline adapter.**

O interpretador cria um trampoline que captura `(TypedAction ref, args, captures)`
e expõe como `extern "C" fn`. O scheduler não muda.

```rust
/// Entrada para o trampoline do scheduler.
struct InterpFiberEntry {
    action: *const TypedAction,
    args: Vec<Value>,
    arena: i64,
    // O TypedModule é Arc'd — sobrevive ao fiber.
    module: Arc<TypedModule>,
}

/// Trampoline — chamado pelo scheduler.
extern "C" fn interp_trampoline(rt: i64, arena: i64, entry_ptr: i64) -> i64 {
    let entry = unsafe { &*(entry_ptr as *const InterpFiberEntry) };
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    let mut env = Env::new();
    // Bindar args...
    let result = eval_action_body(entry.action, &mut env, runtime, arena, 0);
    // Result → i64
    result.unwrap_or(0)
}
```

O `fork!` no interpretador:
1. Cria `InterpFiberEntry` na heap (via `Box::into_raw`)
2. Chama `kata_rt_fork(rt, interp_trampoline, entry_ptr, arena)`
3. O scheduler agenda o fiber

Quando o fiber completa, o scheduler chama o cleanup. O `InterpFiberEntry` é
liberado.

**Alternativa (mais limpa): generalizar o scheduler.** O fiber carrega um
`enum FiberEntry { Native(fn_ptr), Interpreted(InterpFiberEntry) }`. O scheduler
faz dispatch. Isto evita `unsafe` no trampoline mas requer mudar o scheduler —
blast radius maior. **Decisão: trampoline primeiro. Generalizar depois se o
trampoline provar ser frágil.**

#### spawn!

`spawn!` cria um processo OS via `fork`. No modo interpretado, o child processo
precisa re-executar o interpretador com a action isolada. Duas opções:

**A) Re-exec do binário interpretador:** O child faz `exec(argv[0], "--internal-spawn", module_path, action_name, args_serialized)`. O processo filho re-carrega o módulo, interpreta a action, envia resultado via pipe. Funciona mas requer serialização de args.

**B) Fork sem exec:** O child herda o espaço de memória (COW). O interpretador continua no child processo executando a action. O `TypedModule` (Arc'd) sobrevive ao fork. O pipe é herdado. **Decisão: B.** É mais simples e consistente com o modelo JIT (que também faz fork sem exec).

#### select

`kata_rt_select` espera handles de canal e function pointers para os braços.
O interpretador passa os handles e avalia o body do braço que disparar
após o select retornar. O scheduler faz o bloqueio cooperativo — o
interpretador não precisa fazer nada especial, só chamar `kata_rt_select` e
processar o resultado.

### 2.9. Actions

Actions têm corpo imperativo: `var`, `Reassign`, `return`, `loop`, `break`,
`continue`, `for`, `echo!`, I/O. O interpretador avalia o corpo como um `Block`.

O `ActionCall` na TAST tem `callee` (nome), `args` (tupla), e `ffi_symbol`
(para builtins como `echo!`).

Para Actions definidas pelo usuário: o interpretador procura a action no
`TypedModule.actions`, cria uma nova fiber arena, e avalia o corpo.

Para Actions FFI (`echo!`, `input!`, `sleep!`, etc.): despacha via
`ffi_dispatch`.

`return` é implementado via `InterpError::Return(value)` — um early-return
via `Err` que é capturado no nível da action.

`break`/`continue` via `InterpError::Break`/`InterpError::Continue` —
capturados no nível do `Loop`.

### 2.10. Arena management

O interpretador usa o mesmo sistema de arenas do runtime:
- `kata_rt_arena_create(rt)` cria uma fiber arena (bumpalo)
- `kata_rt_arena_destroy(rt, handle)` libera no fim da action
- `kata_rt_get_root_arena_handle(rt)` retorna a root arena para closures

Cada Action chamada pelo interpretador cria sua própria arena e destrói no
fim. Funções puras não criam arena — usam a arena do caller (passada como
parâmetro).

### 2.11. Comptime

O comptime pass (`kata-comptime`) já executa expressões via JIT-and-execute.
No modo interpretado, `constant` é avaliada pelo próprio interpretador antes
de executar o entry point. O comptime pass do pipeline pode ser skipado no
modo interpretado — o interpretador avalia `ConstantBinding` diretamente.

**Alternativa:** rodar o comptime pass com o interpretador em vez do JIT. O
comptime pass chama `jit_execute_expr` — substituir por `interp_execute_expr`.
Isso requer mudar o comptime pass para aceitar um executor trait. **Decisão:
skipar comptime pass no modo interpretado. Avaliar constants no prólogo do
interpretador.** Mais simples, e constants em modo interpretado não precisam
de serialização (HeapSnapshot) — o valor vive na arena do interpretador.

### 2.12. Tree-shaking

Tree-shaking remove funções/actions não alcançadas. No modo interpretado, o
tree-shaking pode ser skipado — o interpretador só avalia funções que são
realmente chamadas. Não há custo em ter funções mortas no `TypedModule` (não
há codegen que as materialize).

**Decisão: skipar tree-shaking no modo interpretado.** O pipeline para em
`optimize()` — não chama `tree_shake()`, `comptime()`, `build_type_table()`.

## 3. Integração no driver

### 3.1. Novo subcomando

```
kata run file.kata           → JIT (comportamento atual)
kata run --interp file.kata  → Interpretador
kata eval "expr"             → JIT (comportamento atual)
kata eval --interp "expr"    → Interpretador
kata repl                    → Interpretador (ver §3.2)
kata repl --jit              → JIT (comportamento atual)
```

`--interp` é uma flag em `Run` e `Eval`. O default de `repl` muda para
interpretador.

### 3.2. REPL

O REPL muda de JIT para interpretador como modo padrão. O interpretador é
naturalmente incremental: cada linha avalia uma expressão no env persistente.
Não há recompilação, não há `PrevFuncMap`, não há leak de JITModule.

A `ReplSession` do interpretador:
```rust
pub struct InterpReplSession {
    env: Env,                         // Variáveis acumuladas entre linhas
    rt: Box<Runtime>,                 // Runtime persistente
    fiber_arena: i64,                 // Arena reusada entre linhas
    items: Vec<Spanned<Item>>,        // Declarações acumuladas (Sig, Data, Enum)
    functions: HashMap<String, Arc<TypedFunction>>,  // Funções nomeadas
    actions: HashMap<String, Arc<TypedAction>>,      // Actions nomeadas
    module: Arc<TypedModule>,         // Último TypedModule compilado
}
```

Cada linha:
1. Lex + parse (Single mode — não há aridade para resolver)
2. Resolve + infer com prelude + items acumulados
3. Se `let x := expr`: avalia expr, `env.define("x", result)`
4. Se expressão: avalia, display
5. Se declaração (Sig, Data, Enum): adiciona a `items`, re-resolve

A persistência é trivial: `env` e `items` acumulam entre linhas. Não há
snapshots, não há function pointers leaked, não há recompilação.

### 3.3. Pipeline

```rust
impl Pipeline {
    /// Para no optimize() — não precisa de tree_shake/comptime/type_table.
    pub fn interpret(self) -> miette::Result<InterpResult> {
        let mono = self.mono.expect("interpret chamado antes de optimize");
        let rt = Box::new(kata_rt::Runtime::new());
        let rt_ptr = Box::into_raw(rt) as i64;
        let result = kata_interp::interpret(&mono.inner, rt_ptr)
            .map_err(|e| e.into_report_with_source(&self.source, self.file_path.as_deref()))?;
        // Leak do Runtime — valores retornados são ponteiros para a arena.
        // Para scripts efêmeros, aceitável. Para REPL, o Runtime persiste.
        std::mem::forget(unsafe { Box::from_raw(rt_ptr as *mut kata_rt::Runtime) });
        Ok(result)
    }
}
```

`kata_interp::interpret` recebe o `TypedModule` e o `rt_ptr`, cria a fiber
arena, avalia o entry point, e retorna `InterpResult { raw: i64, ty: Ty }` —
mesma estrutura de `JitResult`.

## 4. Fases

### Fase 1 — Núcleo: expressões puras + aritmética ✅

**Status:** Completa e commitada (`96fff96`).

**Escopo:** `+ 3 4`, `let x := 5`, `echo!(x)`, fatorial recursivo.

**Mudanças:**
1. Criar crate `kata-interp` com `lib.rs`, `value.rs`, `env.rs`, `eval.rs`
2. Implementar `eval` para: IntLit, FloatLit, TextLit, Unit, Ident, Let, Var,
   Reassign, Grouping, Tuple, StructConstruct, FieldAccess, IndexAccess,
   Block, TypeAscription, Lambda (construção), Closure (chamada)
3. Implementar `ffi_dispatch` para aritmética SMI inline (+, -, *, /, //, mod,
   <, >, =, <=, >=, !=)
4. Implementar `call_closure` — match de cláusulas, pattern matching básico
   (Literal, Ident, VariantQual)
5. Implementar `Match` com guards
6. Implementar `ActionCall` para `echo!` (FFI) e actions definidas pelo usuário
7. `Pipeline::interpret()` no driver
8. `Command::Run { --interp }` e `Command::Eval { --interp }`

**Verificação:**
- `kata eval --interp "+ 3 4"` → 7
- `kata eval --interp "* 6 7"` → 42
- `kata run --interp examples/fatorial.kata` → 120
- `kata run --interp examples/fib_ramified.kata` → output correto
- `kata eval --interp "let x := 10 echo!(* x 2)"` → 20

### Fase 2 — Coleções + HOFs ✅ (exceto Dict/Set)

**Status:** Completa e commitada. Show para List, Array, Tuple, Boolean funcionando.
map/filter/fold, quicksort, stream fusion, pattern matching (Cons, Tuple, Variant),
with bindings, e exemplos canônicos todos funcionando.

**Pendências:**
- Dict/Set: stubs retornam erro. Requer investigar hash_fn/eq_fn do codegen.
- Struct show: esquema de campos não disponível no show.rs (placeholder).
- Sum/Enum show: tag numérica sem mapeamento para nome da variante.

**Bugs corrigidos durante a implementação:**
- `set_rt_ptr` em TLS: FFIs de coleção lêem Runtime via thread_local, não parâmetro
- Boolean pattern matching: Boolean é i64 cru (1/0), não Sum box
- `with_bindings` ordem: avaliados após pattern match (podem referenciar variáveis do pattern)
- `kata_rt_array_len` retorna SMI-encoded: `decode_smi` necessário no show

**Escopo:** List, Array, Dict, Set, Range, map, filter, fold, stream fusion.

**Mudanças:**
1. `eval` para ListLit, ArrayLit, DictLit, SetLit, RangeLit
2. `eval` para Map, Filter, Fold — chama callback via `call_closure`
3. `eval` para FusedStream — loop único aplicando estágios
4. `eval` para ForIn, In
5. `ffi_dispatch` para list/array/dict/set operations (cons, head, tail,
   concat, reverse, len, get, contains, insert, etc.)
6. Pattern matching para Cons, Tuple, StructDestruct

**Verificação:**
- `kata run --interp examples/map_filter_fold.kata` → output correto
- `kata eval --interp "fold (+) 0 [1 2 3 4 5]"` → 15
- `kata eval --interp "map (* _ 2) [1 2 3]"` → [2, 4, 6]
- `kata eval --interp "filter (lambda x: > x 2) [1 2 3 4]"` → [3, 4]
- `kata run --interp examples/quicksort.kata` → output correto

### Fase 3 — Actions + controle de fluxo ✅

**Status:** Completa e commitada. Todos os exemplos canônicos batem com JIT.

**Implementação:**
- `eval` para Loop, Break, Continue, Return, Var, Reassign, TypeOf, BytesLit — já existiam desde Fase 1
- `InterpError::Return/Break/Continue` para controle de fluxo
- `eval_action_body` — cria fiber arena, avalia corpo, destrói arena
- `echo!`/`println!` converte args via `show_value` antes de chamar `kata_rt_print`
- `__stdin__/__stdout__/__stderr__` definidos no env de runtime em `eval_entry`
- `__param_{i}` ligados em `call_typed_clauses` para hooks `@log`
- Boolean `VariantQual` retorna i64 cru (1/0) em vez de Sum box
- Loop `break`/`continue` armazenados como `Err` (não `Ok`) para despachar corretamente

**Bugs corrigidos:**
- `echo!(True)` segfault: `echo` recebe Text mas Boolean era i64 cru. Fix: interceptar `echo!` e converter via `show_value`
- `show.rs` Boolean: usava `kata_rt_sum_tag_int` (Sum box) mas Boolean é i64 cru
- Loop `continue`: armazenado como `Ok(0)` confundia com retorno normal. Fix: armazenar como `Err(Continue)`
- `@log` + `import stdio`: `__param_0` e `__stdout__` não definidos no env

**Pendências (não bloqueantes):**
- `input!` (stdin interativo): FFI `kata_rt_input` existe mas não testado
- `rational` show: misaligned pointer (Rational precisa de ponteiro alinhado)
- `ranges` contains: `In` operator para Range não despacha corretamente
- File I/O: `read!`/`write!`/`close!` não testados

**Escopo:** var, return, loop, break, continue, for, actions com corpo
imperativo, echo!, input!, I/O.

**Mudanças:**
1. `eval` para Loop, Break, Continue, Return
2. `InterpError::Return/Break/Continue` para controle de fluxo
3. `eval_action_body` — cria fiber arena, avalia corpo, destrói arena
4. `ffi_dispatch` para I/O: echo!, input!, file operations, sleep!
5. `eval` para BytesLit, TypeOf

**Verificação:**
- `kata run --interp examples/guessing_game.kata` → funciona interativamente
- `kata eval --interp "action main var i := 0 loop if ... echo!(i) ... main!()"` →
  loop funciona
- `kata run --interp examples/is_prime.kata` → output correto

### Fase 4 — REPL interpretado ✅

**Status:** Completa e commitada. Todos os exemplos canônicos funcionam via `:load`.

**Implementação:**
- `InterpReplSession` em `repl/interp_session.rs` — sessão REPL com Env persistente
- `interp_loop.rs` — loop rustyline idêntico ao JIT, despachando para `InterpReplSession`
- `cmd_repl(interp: bool)` despacha entre JIT (`ReplSession`) e interp (`InterpReplSession`)
- `interpret_with_env` no `kata-interp` — reusa `Env` persistente em vez de criar novo
- `eval_entry_with_env` no `InterpCtx` — avalia entry point com Env fornecido
- `Env` tornado `pub` para uso cross-crate
- `:reset` recria Env + Runtime + recarrega prelude
- `:load` carrega arquivo .kata (items entram no env)
- `:type` infere tipo sem executar
- `:env` mostra bindings e tipos
- Redefinição de `let` remove binding anterior (igual ao JIT REPL)
- Runtime persistente (Box::into_raw) — arena Bump preserva valores entre linhas

**Diferença do JIT REPL:**
- Não congela bindings (não precisa — Env persiste valores i64)
- Não gerencia snapshots (não precisa — valores ficam na arena)
- Não persiste function pointers (funções são reavaliadas a cada linha)
- pre_entry é reavaliado a cada linha (bindings já no Env são sobrescritos)

**Verificação:**
- `let x := 42` depois `echo!(x)` → 42 ✅
- `fat :: Int Int => Int` / lambdas / `echo!(fat 5 1)` → 120 ✅
- `let f := lambda n: * n 2` / `echo!(f 21)` → 42 ✅
- `:reset` limpa env ✅
- `:load examples/quicksort.kata` → [-1, 1, 2, 5, 5, 6, 9, 12] ✅
- `:load examples/loop_action.kata` → 0 1 2 3 4 5 ✅
- `:type x` → Int ✅
- `:env` → x: Int ✅
- `cargo test --workspace` → 1851 passed, 0 failed ✅

**Limitação conhecida:**
- Actions multiline no input direto (não via `:load`) falham no parser REPL — bug de heurística multiline que afeta ambos JIT e interp (`action` não está na lista de triggers multiline)

### Fase 5 — CSP (fork!, channels, select) — Completa ✅

**Status:** Nível 1 (canais síncronos) e Nível 2 (fork!/spawn! + scheduler) completos.

**Nível 1 — Completo ✅:**
- `ChannelCreate` — cria canal (Rendezvous/Buffered/Broadcast), aloca tupla (handle, handle)
- `ChannelSend` — chama `kata_rt_channel_send(handle, value)`
- `ChannelRecv` — chama `kata_rt_channel_recv(handle)`, define binding
- `ReceiverFactoryCall` — chama `kata_rt_broadcast_receiver_create(arena, factory)`
- `Select` (canais) — chama `kata_rt_select(handles, n, timeout_ms)`, avalia body do braço
- Cross-process: não suportado (retorna erro)
- `broadcast.kata` — funciona ✅ (bate com JIT)

**Nível 2 — Completo ✅:**
- `Fork` — implementado via `interp_trampoline` (`extern "C"` que despacha de volta
  para o interpretador). Tabela global `INTERP_ACTIONS` (Mutex<Vec>) registra a action
  antes de `kata_rt_spawn`. O trampoline lê `action_id` do `args_ptr`, recupera
  `(action_name, module)` da tabela, cria `InterpCtx::new_with_arena`, e executa o body.
- `Spawn` — implementado via `kata_rt_spawn_process` (fork OS). Mesmo mecanismo do
  `Fork` mas com `kata_rt_spawn_process` em vez de `kata_rt_spawn`.
- `eval_entry_scheduler_mode` — se o entry point é uma `ActionCall` definida pelo
  usuário (sem `ffi_symbol`), faz spawn + `kata_rt_run` em vez de `call_action` direto.
  O fiber raiz executa a action dentro do scheduler de fibers, permitindo que
  `fork!` dentro da action crie fibers filhas drenadas pelo `run`.
- `Select` com timeout — `decode_smi` no `timeout_ms` para alinhar com o codegen.
- Despacho de overloads por aridade — `call_action` e trampoline despacham por
  nome **E** aridade (número de params), não apenas primeiro match. Necessário
  para actions com múltiplos overloads (`log`, `_log_publish`).
- FFIs de log despachadas: `kata_rt_log_publish`, `kata_rt_log_publish_default`,
  `kata_rt_log_publish_topic`, `kata_rt_log_publish_full`, `kata_rt_log_recv`,
  `kata_rt_log_config`.

**Verificação Nível 1:**
- `kata run --interp examples/broadcast.kata` → 42 ✅ (bate com JIT)
- `cargo test --workspace` → 1851 passed, 0 failed ✅

**Verificação Nível 2:**
- `kata run --interp examples/select_queue.kata` → 2 linhas ✅ (bate com JIT)
- `kata run --interp examples/log_telemetry.kata` → entrada (41) / resultado 42 /
  evento-manual / 0 ✅ (bate com JIT)
- `cargo test --workspace` → 1851 passed, 0 failed ✅
- 18/22 exemplos canônicos batem com JIT (4 divergências pré-existentes: rational,
  ranges, quicksort, trma — não relacionadas a CSP)

### Fase 6 — Testes + portabilidade

**Escopo:** `kata test --interp`, validação cross-platform.

**Mudanças:**
1. `Command::Test { --interp }` — executa testes via interpretador
2. Doctests via interpretador (substitui JIT-and-execute no `handle`)
3. Validar em plataformas sem Cranelift (wasm32, RISC-V — futuro)

**Verificação:**
- `kata test --interp` roda todos os `@test` via interpretador
- Doctests passam
- `cargo test --workspace --all` → sem regressões

## 5. O que não muda

- **Pipeline até `optimize()`** — idêntico. Lex, parse, resolve, infer,
  monomorph, optimize. O interpretador consome o mesmo `TypedModule`.
- **`kata-rt`** — nenhuma mudança. O interpretador chama as mesmas funções
  C-ABI. Arena, list, array, dict, channel, scheduler — tudo reusado.
- **`kata-codegen`** — nenhuma mudança. JIT e AOT continuam funcionando.
- **`kata-comptime`** — nenhuma mudança. Skipado no modo interpretado.
- **`kata-tree-shaking`** — nenhuma mudança. Skipado no modo interpretado.
- **Display (`print_result`)** — funciona sem mudança. Recebe `(raw_i64, ty)`.
- **LSP** — nenhuma mudança. Usa o pipeline de inference, não o codegen.
- **Stdlib** — nenhuma mudança. Stdlib é código Kata, compilado pelo pipeline
  normal. O interpretador executa o `TypedModule` resultante.

## 6. Decisões de Design

| Decisão | Escolha | Razão |
|---|---|---|
| Representação de valor | `i64` (SMI-tagged, mesmo formato do runtime) | Compartilha display, arena, e funções de runtime sem conversão |
| Chamadas FFI | Dispatch direto em Rust (match → function call) | Sem overhead de C-ABI calling convention. Para SMI, é 3-4 instruções Rust |
| Aritmética SMI | Inline no interpretador (checked_add + encode_smi) | Elimina call para `kata_rt_bi_add` — mais rápido que JIT para Int pequeno |
| Closures | Struct na arena com ponteiro Rust para `&TypedLambdaClause` | Sem fn_ptr JIT. O `TypedModule` (Arc'd) sobrevive à execução |
| Scheduler de fibers | Trampoline adapter (`interp_trampoline`) | Não muda o scheduler. Generalização fica para depois se necessário |
| Comptime pass | Skipado. Constants avaliadas no prólogo do interpretador | Sem necessidade de HeapSnapshot — valores vivem na arena do interpretador |
| Tree-shaking | Skipado | Sem codegen — funções mortas não têm custo no interpretador |
| REPL default | Interpretador | Startup instantâneo, persistência trivial, sem leak de JITModule |
| Arena management | Mesmo sistema do runtime (bumpalo fiber + tracked root) | Reusar infraestrutura existente, sem nova gestão de memória |
| `spawn!` | Fork sem exec (COW) | Consistente com modelo JIT. TypedModule (Arc'd) sobrevive ao fork |

## 7. Riscos e Mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| Performance para código pesado | ~50-100x mais lento que JIT | `--jit` flag disponível. Interpretador é para scripts curtos e REPL, não para computação densa |
| Trampoline frágil (unsafe) | Segfault em fork! | Fase 5 valida com testes E2E de CSP. Generalizar scheduler se necessário |
| FFI dispatch incompleto | Panic em operação não coberta | Fase 1-3 cobrem as FFIs mais comuns. Tabela completa em Fase 5-6. |
| `TypedModule` lifetime (Arc) | Use-after-free se Arc dropa antes do fiber terminar | Scheduler structured concurrency garante que fibers completam antes do Runtime dropar |
| Recursão profunda estoura stack Rust | Stack overflow em fatorial(1000000) | TRMA + TCO já reescrevem recursão em cauda. O interpretador avalia a recursão de cauda como loop via match. Para recursão não-de-cauda sem TRMA, aceitar o limite da stack Rust. |
| Inconsistência semântica com JIT | Comportamento diferente entre modos | Ambos consomem o mesmo `TypedModule`. A semântica é definida pela TAST, não pelo backend. Testes E2E rodam em ambos os modos. |
| Comptime sem JIT | `constant` com expressão complexa pode falhar | O interpretador avalia a expressão diretamente — sem necessidade de JIT. Se a expressão chama uma função que não está no módulo, o interpretador avalia via `call_closure`. |

## 8. Evolução Futura (Não Escopo)

- **Bytecode VM:** Compilar TAST → bytecode compacto. Elimina overhead de
  tree-walking (match em enum + recursão). `eval_bytecode` em vez de
  `eval_tast`. Pré-requisito: interpretador tree-walking provando que o modo
  interpretado é útil na prática.
- **Tiered JIT:** Interpretar primeiro, compilar hot spots via Cranelift.
  Profiling no interpretador identifica funções quentes. O JIT compila apenas
  essas. Modelo V8/LuaJIT.
- **Generalizar scheduler:** `FiberEntry { Native, Interpreted }` elimina o
  trampoline unsafe. Faz sense se o interpretador for o modo padrão de execução.
- **Interpretador em wasm:** O interpretador não depende de Cranelift. Compilar
  `kata-interp` + `kata-rt` para wasm32. Kata rodando no browser sem
  mudança na linguagem.
- **Debugging:** Breakpoints, step-through, watchpoints — naturais em
  tree-walking. O interpretador tem acesso à TAST com spans, tipos, e nomes.
  Stack trace mostra a expressão exata, não um endereço de assembly.

## 9. Critérios de Aceitação

1. `kata eval --interp "+ 3 4"` → `7`
2. `kata run --interp examples/fatorial.kata` → `120`
3. `kata run --interp examples/quicksort.kata` → output correto
4. `kata run --interp examples/map_filter_fold.kata` → output correto
5. `kata run --interp examples/guessing_game.kata` → funciona interativamente
6. REPL interpretado: `let x := 42` depois `echo!(x)` → `42`
7. REPL interpretado: função nomeada persiste entre linhas
8. `kata test --interp` roda todos os `@test` sem regressões
9. `cargo test --workspace --all` → sem regressões no modo JIT
10. `kata run --interp` executa em <50ms (startup + execução de script simples)
11. CSP: `fork!` + channels funcionam no interpretador
12. `spawn!` funciona no interpretador (Linux)