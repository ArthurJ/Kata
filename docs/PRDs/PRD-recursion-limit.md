# PRD: Limite de Profundidade de Recursão

## Estado-alvo

Toda recursão Kata que excede o limite de profundidade produz uma **falha
graciosa** — um erro de runtime Kata com mensagem clara — em vez de SIGSEGV.
Válido para interpretador e codegen JIT.

O limite é um **contador de profundidade em software**, incrementado no
prólogo e decrementado no epílogo de cada chamada de função Kata não-de-cauda.
Chamadas em tail position (trampoline no interpretador, `return_call` no
Cranelift) **não** incrementam — já são O(1) em stack.

O limite é configurável em código Kata via `constant` + `stdlib/config.kata`.

## Motivação

Hoje, recursão não-de-cauda infinita (ou muito profunda) estoura a stack do
processo — SIGSEGV bruto, sem mensagem, sem diagnóstico. Isso viola o
contrato fundamental de Kata: erros compile-time e runtime falham
graciosamente, nunca SIGSEGV/SIGILL.

O TCO existente (trampoline + `return_call` + TRMA) cobre recursão de cauda,
mas não cobre recursão não-de-cauda que o programador escreve por engano ou
que excede a stack legítima. O limite de profundidade é a **primeira linha
de defesa** — precisa estar de pé antes de qualquer outra melhoria de
recursão.

## Design

### Contador no Runtime (kata-rt)

Depth e limit são campos do `Runtime`, não globais. O ponteiro `rt` já
está disponível nos dois backends (param 0 de toda função JIT, campo em
`InterpCtx`). Isso elimina vazamento entre execuções (REPL, LSP, comptime)
e permite que cada Runtime tenha seu próprio limite.

```rust
// kata-rt/src/runtime.rs
pub struct Runtime {
    // ... campos existentes ...
    call_depth: Cell<u32>,
    depth_limit: Cell<u32>,
    overflowed: Cell<bool>,
}

impl Runtime {
    pub fn depth_inc(&self) -> u32 {
        let d = self.call_depth.get() + 1;
        self.call_depth.set(d);
        d
    }
    pub fn depth_dec(&self) {
        self.call_depth.set(self.call_depth.get().saturating_sub(1));
    }
    pub fn depth_get(&self) -> u32 { self.call_depth.get() }
    pub fn depth_set_limit(&self, limit: u32) { self.depth_limit.set(limit); }
    pub fn depth_limit(&self) -> u32 { self.depth_limit.get() }
    pub fn set_overflowed(&self) { self.overflowed.set(true); }
    pub fn overflowed(&self) -> bool { self.overflowed.get() }
    pub fn reset_depth(&self) {
        self.call_depth.set(0);
        self.overflowed.set(false);
    }
}
```

`Cell<u32>` em vez de `AtomicU32` porque o Runtime é single-threaded com
scheduler cooperativo — sem concorrência. O `rt_ptr` (i64) é passado para
as FFIs que encapsulam essas chamadas.

Default `depth_limit`: **1000** para ambos os backends (interpretador e
JIT), setado em `Runtime::new()`. Conservador para interpretador
(~600-900KB de stack Rust) e JIT (~64-256KB de stack nativa).
Compatível com threads de 2MB (Windows, pools secundários).

### Interpretador (kata-interp)

**Local:** `call_typed_clauses` em `eval.rs` (não `call_named_function`).

O incremento fica **depois** do cache check em `call_named_function`. Se
`@cache` hit, retorna sem incrementar — não cria frame, não consome stack.

```rust
fn call_typed_clauses(...) -> Result<Value, InterpError> {
    let depth = rt::kata_rt_depth_inc();
    if depth > rt::kata_rt_depth_limit() {
        rt::kata_rt_depth_dec();
        return Err(InterpError::Runtime(format!(
            "recursion depth exceeded: {depth} (limit: {})",
            rt::kata_rt_depth_limit()
        )));
    }
    let _guard = DepthGuard;
    // ... trampoline loop existente ...
    // trampoline NÃO incrementa — faz loop, reutiliza o frame
}

struct DepthGuard;
impl Drop for DepthGuard {
    fn drop(&mut self) { rt::kata_rt_depth_dec(); }
}
```

O trampoline em `call_typed_clauses` já não incrementa — faz loop com novos
args em vez de recursar. Uma cadeia de N tail calls fica com contador = 1
do início ao fim.

### Codegen (kata-codegen)

**Local:** prólogo e epílogo da função **inner** em `function_def.rs`.

O incremento fica no inner, não no wrapper. O wrapper faz cache lookup —
se hit, retorna sem incrementar (não cria frame, não consome stack). Se
miss, chama inner, que incrementa no prólogo.

O JIT emite no prólogo de cada função inner Kata (não entry point, não
FFI, não wrapper):

1. `call kata_rt_depth_inc` → retorna `depth` em `rax`
2. `load kata_rt_depth_limit` → `limit` em `rcx`
3. `cmp depth, limit`; se `depth > limit`, `jump overflow_block`
4. Corpo normal

`overflow_block`:
1. `call kata_rt_depth_dec` (equilibrar o inc)
2. Setar flag de overflow no Runtime via FFI `kata_rt_set_overflowed()`
3. `iconst i64 0` (valor dummy — o entry point checa a flag, não o valor)
4. `return_ 0`

Epílogo (antes de cada `return_` não-tail e antes de cada `return_call`):
1. `call kata_rt_depth_dec`

**Tail call (`return_call`):** decrementa antes de pular — o callee fará seu
próprio prólogo (que incrementa). Sem o decremento, uma cadeia de N tail
calls deixaria o contador em N (não em 1), e `fat_tail 100000 1` estouraria
o limite — contradizendo o propósito do TCO. O decremento antes do
`return_call` faz o contador refletir "frames ativos" em vez de
"chamadas acumuladas".

**@cache hit:** o wrapper faz cache lookup antes de chamar o inner. Se hit,
retorna sem chamar o inner — o contador não é incrementado. Cache hit não
cria frame, não consome stack.

#### FFI de depth

Registrar no `ffi_registry` oito FFIs de depth:

```
kata_rt_depth_inc        -> (rt) -> i64 (depth atual)
kata_rt_depth_dec        -> (rt) -> ()
kata_rt_depth_get        -> (rt) -> i64
kata_rt_depth_get_limit  -> (rt) -> i64
kata_rt_depth_set_limit  -> (rt, limit) -> ()
kata_rt_set_overflowed   -> (rt) -> ()
kata_rt_overflowed       -> (rt) -> i64
kata_rt_reset_depth      -> (rt) -> ()
```

Todas recebem `rt: i64` (rt_ptr) como primeiro parâmetro.

O codegen chama essas FFIs no prólogo/epílogo. O overhead é uma `call` por
função Kata (prólogo) + uma `call` por return não-tail (epílogo). Em
benchmark, `call` nativo é ~3-5ns — negligenciável frente ao custo de
chamada de função Kata (que já envolve stack frame setup, argument
marshalling com rt + arena + box_ptr).

#### Detecção no caller

O overflow_block seta uma flag `overflowed: bool` no `Runtime` via FFI
`kata_rt_set_overflowed(rt_ptr)`. O entry point checa a flag após o
retorno, em vez de comparar um valor sentinela — elimina colisão por
construção.

```
// jit.rs, após func(rt_ptr):
let val = func(rt_ptr);
if rt::kata_rt_overflowed(rt_ptr) {
    let depth = rt::kata_rt_depth_get();
    let limit = rt::kata_rt_depth_limit();
    return Err(CodegenError::Runtime(format!(
        "recursion depth exceeded: {depth} (limit: {limit})"
    )));
}
```

A flag é resetada no início de cada `jit_eval` / `call_named_function`.

### Configuração via `constant` — limite per-module

O limite é declarado no módulo via `constant`, avaliado em comptime, e
armazenado como metadata no `TypedModule`. O driver lê essa metadata e
aplica ao Runtime antes da execução. Imutável em runtime.

#### stdlib/config.kata

```kata
set_recursion_limit :: PositiveInt => Unit
@ffi("kata_rt_depth_set_limit")
lambda n: Unit
```

A função acessível ao usuário recebe `PositiveInt` — validação > 0 em
compile-time via o type system. A FFI subjacente recebe `Int` (i64 bruto);
o downcast `PositiveInt → Int` é um no-op em runtime (mesmos bits).

#### Uso

```kata
import stdlib.config

constant _ := config.set_recursion_limit(10000)

soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))

soma 5000  # funciona — limite é 10000
```

O comptime pass avalia o `constant`, executa a FFI no comptime Runtime
(setando `depth_limit`), e o resultado fica registrado. O driver captura
`depth_limit` do comptime Runtime e propaga para o Runtime da execução
principal.

Tentar usar `config.set_recursion_limit` fora de `constant` é um erro
comum de linguagem: a FFI só existe em comptime, não é registrada no
ffi_registry do codegen de runtime. O typeck rejeita a chamada com
"unknown function" ou mensagem equivalente.

#### Pipeline timing

O comptime pass roda **após** o typeck. Isso significa que `constant` só
pode configurar coisas que afetam a **execução** (runtime), não coisas que
afetam o **typeck** ou o **optimizer** (que já rodaram).

- ✓ recursion_limit (runtime)
- ✓ Qualquer configuração que só precisa estar ativa no momento da execução
- ✗ allow/warn/deny de lints (typeck já rodou)
- ✗ Configuração que afeta o optimizer

Para lints/allow/warn, ver seção TODO abaixo.

### Pureza

Constants não exigem pureza. O `check_purity` atual em `kata-comptime` é
uma restrição que assume que o desenvolvedor não controla o ambiente de
compilação. Mas compilação é o momento onde o desenvolvedor tem **mais**
controle.

**Decisão:** remover `check_purity` para constants. O comptime JIT executa
o que o desenvolvedor escreveu.

Nota: `check_purity` pode continuar existindo como função utility se for
usada em outros contextos (ex: `@comptime let` dentro de actions). O que
muda é que `evaluate_constants` não a chama.

### Infrastructure do comptime JIT

O comptime JIT hoje cria um `Runtime::new()` vazio via `leak_rt_ptr()` —
sem scheduler, sem fibers, sem I/O. Isso limita o que constants podem
fazer: expressões que dependem dessa infrastructure falham.

A solução é levantar a infrastructure completa no comptime JIT, igual ao
que o driver faz para execução normal. Assim, `constant x :=
read_file("config.txt")` funciona — lê o arquivo em compile-time e bakes
o conteúdo no binário. É o mesmo padrão de `include_str!` em Rust,
`@embedFile` em Zig, `#include` em C.

Implementação: o comptime pass cria um `Runtime` completo (com scheduler
init) uma vez por módulo, e o reutiliza para todas as constants. O
`rt_ptr` passado para `jit_execute_expr` é o mesmo para todas as
avaliações do módulo. O Runtime é destruído no fim do comptime pass.

**Propagação de config comptime→runtime:** o `constant` executa
`set_recursion_limit(N)` no comptime Runtime, setando `depth_limit`. O
driver captura `depth_limit` do comptime Runtime antes de destruí-lo e
aplica ao Runtime da execução principal. O limite é per-module —
imutável em runtime.

Se uma expression precisa de infrastructure que não está disponível (ex:
fiber cross-process), falha graciosa com erro compile-time — o
`catch_unwind` em `jit_execute_expr` garante que todo panic vira
`ComptimeError`, não abort.

**Reset de depth no entry point:** panic no JIT pula os decs do RAII
guard — o contador fica positivo. O entry point Rust (`jit_eval` /
`call_named_function`) reseta `call_depth = 0` e `overflowed = false` no
início de cada execução.

### Limite configurável

- Default: **1000 frames** para ambos os backends. Conservador para
  interpretador (~600-900KB de stack Rust) e JIT (~64-256KB de stack
  nativa). Compatível com threads de 2MB (Windows, pools secundários).
- `constant _ := config.set_recursion_limit(N)` permite ao usuário
  ajustar em código Kata (compile-time only, per-module). O desenvolvedor
  assume os riscos de um limite maior.
- CLI flag `--recursion-limit N` no driver (futuro)

## O que NÃO está no escopo

1. **Signal handler / guard page interception.** Defense in depth futura.
   O contador de profundidade captura recursão Kata-Kata. Stack overflow
   vindo de FFI profunda ou de frames enormes escapa ao contador e continua
   produzindo SIGSEGV. Uma segunda camada com `sigaltstack` + handler de
   SIGSEGV pode converter isso em erro gracoso no futuro, mas é
   platform-specific e complexo.

2. **Limite por fiber.** Atualmente o contador é global. Se fibers com
   stacks independentes chegarem, migrar para per-fiber.

3. **Controle de stack em bytes.** O limite conta frames Kata, não bytes.
   Um único frame com um array local grande poderia estourar a stack sem
   atingir o limite. Para Kata, frames são finos (alocação de heap via
   arena), então isso é aceitável.

## TODO (fora do escopo, semente para futuro)

### Regulação de erros e warnings (lints)

`constant` só funciona para configuração de runtime porque o comptime pass
roda **após** o typeck. Lints (allow/warn/deny) precisam afetar o typeck
**antes** dele rodar — o mecanismo será diferente.

Possíveis direções (a investigar):
- Directiva de módulo processada no resolve (antes do typeck)
- `#!pragma` processado no parser
- Campo no `ResolvedModule` populado por directivas top-level

Necessidade concreta: relaxar "branch inútil num match" de erro para
warning, ou suprimir warnings de binding não-utilizado.

## DoD

1. **Interpretador:** recursão não-de-cauda com profundidade > limite
   retorna `InterpError::Runtime("recursion depth exceeded: N (limit: M)")`
   — sem SIGSEGV.

2. **Codegen:** recursão não-de-cauda com profundidade > limite retorna
   `CodegenError::Runtime("recursion depth exceeded: N (limit: M)")` — sem
   SIGSEGV.

3. **TCO preservado:** `soma 1000000` (TRMA) e `fat_tail 100000 1`
   (tail-recursiva) executam sem atingir o limite. O contador não incrementa
   em tail calls.

4. **Recursão mútua:** `is_even`/`is_odd` com profundidade > limite falha
   graciosamente.

5. **Configuração via constant (per-module):**
   `constant _ := config.set_recursion_limit(N)` muda o limite. Teste com
   `set_recursion_limit(10)` + recursão 20 falha; `set_recursion_limit(100)`
   + recursão 50 passa. `PositiveInt` garante N > 0 em compile-time —
   `set_recursion_limit(-5)` é rejeitado pelo typeck antes de chegar na FFI.

6. **Reset entre execuções:** entry point reseta `call_depth = 0` e
   `overflowed = false` no início de cada execução. Após execução
   bem-sucedida, `depth_get() == 0`. Após panic/erro, próxima execução
   começa limpa.

7. **Overhead medido (futuro):** `fat 10` (10 chamadas) com e sem o
   contador — diferença de tempo < 5%. Fora do escopo atual — é
   verificação de performance, não funcionalidade.

8. **Comptime com infrastructure (futuro):** `constant` que usa I/O
   (ex: `read_file`) executa em comptime com `Runtime` completo. O
   valor é baked no binário. Falhas de infrastructure produzem
   `ComptimeError` gracioso, não SIGSEGV. Fora do escopo atual — é
   melhoria geral do comptime JIT, não específica do recursion limit.

9. **Propagação comptime→runtime:** `constant _ :=`
   config.set_recursion_limit(N)` seta o limite no comptime Runtime.
   O driver propaga para o Runtime da execução principal. O limite é
   per-module e imutável em runtime.

## Status (2026-09-01)

- Fase 1 — Runtime + FFI: ✅
- Fase 2 — Interpretador: ✅ (depth tracking + 5 testes E2E)
- Fase 3 — Codegen: ✅
- Fase 4 — Configuração (stdlib/config.kata): ✅
- Fase 5 — Propagação comptime→runtime: ✅
- Fase 6 — Testes E2E: ✅ (14 testes: 8 codegen + 5 interpretador + 1 cache_hit, 2003 total)
- Fase 7 — Infrastructure do comptime JIT (Runtime completo): futuro
- Fase 8 — Overhead medido: futuro

### Débito técnico

1. **`stdlib/config.kata`**: mudar `set_recursion_limit :: Int => Unit` para
   `PositiveInt => Unit`. O typeck e codegen já tratam refined types em
   assinaturas FFI (ascription é no-op em runtime). Verificar se
   `try_exec_comptime_ffi` desembrulha `Ascription` além de `Grouping`.
2. **Testes do interpretador**: 5 testes E2E listados acima não existem.
3. **`cache_hit_not_counted`** no codegen: teste listado não existe.

## Testes

### Interpretador (kata-interp/tests)

- `recursion_limit_interp`: recursão não-de-cauda `count N` com N > limite
  → erro gracioso, sem SIGSEGV
- `recursion_limit_mutual`: is_even/is_odd (não-de-cauda via match) com
  profundidade > limite → erro gracioso
- `tco_not_limited`: `fat_tail 100000 1` executa com sucesso (não atinge
  limite)
- `depth_resets`: após execução, `kata_rt_depth_get() == 0`
- `configurable_limit`: `constant _ := set_recursion_limit(10)` +
  recursão 20 falha; `set_recursion_limit(100)` + recursão 50 passa

### Codegen (kata-codegen/tests)

- `recursion_limit_codegen`: recursão não-de-cauda via JIT com N > limite
  → `CodegenError::Runtime`, exit code 1 (não 139 = SIGSEGV). Usa `fib`
  (não-TCO: duas chamadas recursivas) para garantir que o TRMA rewrite
  não transforme em TCO.
- `tco_not_limited_codegen`: `soma 1000000` (TRMA) via JIT executa com
  sucesso
- `trma_not_limited_codegen`: `soma_acc 1000000 0` (TRMA) via JIT executa
  com sucesso
- `tail_call_not_limited_codegen`: `fat_tail 100000 1` via JIT executa com
  sucesso (confirma que decremento antes de `return_call` mantém contador
  constante)
- `cache_hit_not_counted`: função `@cache` com hit não incrementa contador
  — `depth_get() == 0` após cache hit
- `recursion_limit_indirect`: `let g := fat; g 100000 1` (call_indirect,
  não-de-cauda) → erro gracioso (confirma que o contador cobre
  call_indirect, ao contrário do TCO)
- `recursion_limit_mutual_codegen`: `is_even`/`is_odd` com profundidade >
  limite via JIT → erro gracioso
- `depth_resets_codegen`: após execução JIT, `kata_rt_depth_get() == 0`
- `recursion_within_limit_codegen`: recursão com profundidade dentro do
  limite funciona normalmente
- `recursion_limit_configurable_codegen`: `constant _ :=`
  `set_recursion_limit(2000)` via JIT afeta o limite (cobre DoD 9 —
  propagação comptime→runtime)

## Riscos

1. **Overhead do FFI call no prólogo.** Cada função inner faz uma `call`
   para `kata_rt_depth_inc` no prólogo. Em funções muito curtas (ex: `id x
   = x`), o overhead relativo pode ser significativo. Mitigação: o
   Cranelift pode inlinar a FFI se a declarar com `Linkage::Import` e o
   corpo for trivial (load + increment). Alternativa: emitir o
   load/increment/store diretamente no IR em vez de `call` FFI — o codegen
   lê o offset de `call_depth` no `Runtime` e faz `load`+`add`+`store` no
   IR.

2. **Sentinela de overflow (resolvido).** O design original usava um
   sentinela `i32::MAX as i64` no valor de retorno. Substituído por flag
   `overflowed: Cell<bool>` no `Runtime` — elimina colisão por construção.

3. **Decrement em todos os paths.** Se o JIT tiver paths de early return
   que pulam o epílogo (ex: `return` dentro de loop), o decrement pode ser
   esquecido. Mitigação: usar o padrão de epilogue_block existente — todo
   `return_` já jump para o epilogue_block, que é onde o decrement fica.

4. **Recursão via callback HOF.** `map (lambda x: f x) list` onde `f`
  chama `map` recursivamente — cada chamada de callback via
  `call_indirect` incrementa o contador. Isso é correto (é recursão
  genuína) mas pode atingir o limite em programas legítimos que usam HOF
  profundamente. Mitigação: limite configurável; usuário pode aumentar com
  `config.set_recursion_limit`.

5. **Infrastructure do comptime JIT.** Levantar `Runtime` completo no
   comptime pass significa que I/O em compile-time pode ter side-effects
   no ambiente de compilação (escrever arquivos, abrir sockets). Isso é
   intencional e esperado — é o mesmo contrato de build scripts em Rust.
   O `catch_unwind` em `jit_execute_expr` garante que todo panic vira
   `ComptimeError`, não abort.