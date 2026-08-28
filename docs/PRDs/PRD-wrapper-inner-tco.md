# PRD — Wrapper/Inner split: TCO preservado com `@cache` + `@timer`

**Status:** Implementado (Fases 1-5)
**Data:** 2026-08-28
**Depende de:** Fio 12 ✅ (`@cache` LRU/FIFO/MRU/LFU + capacity), PRD-timer ✅ (`@timer` stack slot + canal)
**Não depende de:** Diretivas customizadas, Fio 11, Fio 13+

## 1. Objetivo

Permitir que `@cache` e `@timer` coexistam com TCO (Tail Call Optimization).
Hoje, `@cache` desativa TCO (`no_tail_calls = true`) porque o epílogo onde
`cache_insert` vive precisa executar — mas `return_call` destrói o frame.
`@timer` contorna isso com canal buffer-1 Drop (first-write-wins), mas é
complexidade que existe só para contornar a destruição de frame.

Este PRD introduz o **wrapper/inner split**: cada função com intrínsecas de
epílogo (`@cache`, `@timer`, `@log{when: "exit"}`) + tail calls vira duas
funções Cranelift — um wrapper sem TCO que executa as intrínsecas, e um inner
com TCO que executa o body. O wrapper tem 1 frame extra (O(1)). O inner faz
TCO (O(1) stack para recursão tail).

### Princípio: o split é geral, não específico de `@cache`

O wrapper/inner não é um mecanismo de cache nem de timer — é um mecanismo de
**codegen** que separa "prólogo/epílogo com intrínsecas" (wrapper) de "body
com TCO" (inner). Qualquer intrínseca que precise de epílogo se benefifica.

### Princípio: só ativa quando necessário

O split só ocorre quando a função tem **simultaneamente**:
1. Pelo menos uma intrínseca de epílogo (`@cache`, `@timer`, `@log{exit}`)
2. Pelo menos uma self-call em tail position (`return_call`)

Funções sem intrínsecas, ou sem tail calls, ou com intrínsecas mas sem tail
calls, geram uma função só (approach atual). Zero impacto nos casos existentes.

### Princípio: resolução por tail position

Self-calls no body são resolvidas para **inner** (tail position → TCO) ou
**wrapper** (non-tail position → cache). O codegen já sabe qual call está em
tail position (`tail_pos: true` na TAST). A resolução é natural — não há
heurística nem análise de fluxo.

## 2. Mecanismo

### 2.1. Estrutura

Para uma função `fat_tail :: Int Int => Int` com `@cache` + tail calls:

```
fat_tail (wrapper)                     ← símbolo público
  prólogo:
    bind params
    [timer: start = timer_now()]
    [cache: serialize key, cache_lookup → hit? return cached]
    call fat_tail__inner(rt, arena, box, args...)
  epílogo:
    result = block_param
    [cache: cache_insert(handle, key, result)]
    [timer: delta = timer_now() - start; publish]
    [log_exit: log msg com result]
    return result

fat_tail__inner                         ← símbolo privado
  TCO ativo (no_tail_calls = false)
  lambda 0 acc: return acc
  lambda n acc: return_call fat_tail__inner(rt, arena, box, n-1, * n acc)
```

O wrapper usa `call` (não `return_call`) — o frame sobrevive. O inner usa
`return_call` — TCO preservado. Stack total: wrapper (1 frame) + inner
(reusado via TCO) = **O(1)**.

### 2.2. Resolução de self-calls

No body do inner, cada self-call é resolvida conforme `tail_pos`:

| `tail_pos` | Resolve para | Razão |
|---|---|---|
| `true` | `inner` | TCO — frame destruído, não há epílogo |
| `false` | `wrapper` | Precisa de cache/timer — frame do wrapper sobrevive |

Para `fib` (non-tail recursion):
```
fib(35)                                ← wrapper
  miss → call fib__inner(35)
    fib__inner(35):
      + (fib(34)) (fib(33))            ← non-tail → wrapper (cache!)
        fib(34) → wrapper → miss → call fib__inner(34) → ...
```

Cada subcall non-tail vai through o wrapper, faz cache lookup/insert. O cache
funciona normalmente. O inner só executa o `+`.

Para `fat_tail` (tail recursion):
```
fat_tail(5, 1)                         ← wrapper
  miss → call fat_tail__inner(5, 1)
    fat_tail__inner(5, 1): return_call fat_tail__inner(4, 5)  ← tail → inner
      ... return_call fat_tail__inner(0, 120)
        return 120
    ← inner retorna 120 para wrapper
  cache_insert((5,1), 120)             ← epílogo do wrapper
  return 120
```

Intermediate tail calls vão para o inner (TCO, sem cache). O cache é inserido
uma vez no wrapper. Intermediate calls não são cachedadas — em tail recursion
cada step muda os args, reuso é improvável.

### 2.3. Funções mistas

Cláusulas com tail e non-tail calls coexistem no mesmo inner:

```kata
@cache
f :: Int => Int
lambda 0: 0
lambda n:
    >= n 10: f (- n 1)            ← tail → inner (TCO)
    otherwise: + (f (- n 1)) 1     ← non-tail → wrapper (cache)
```

- `return_call f__inner(14)` na cláusula tail → TCO.
- `f(8)` na cláusula non-tail → wrapper → cache lookup/insert.
- Ambos coexistem no mesmo body. O codegen resolve cada call site
  independentemente.

## 3. Mudanças por camada

### 3.1. TAST — sem mudança

A TAST já carrega `tail_pos: true/false` em cada `Closure`. A detecção
`has_tail_pos_call(clauses)` já existe em `tail_call.rs`. Nenhuma mudança na
TAST, inference, resolution, ou qualquer fase anterior ao codegen.

### 3.2. Codegen — `module.rs`

Hoje, `module.rs` declara e define cada função uma vez:

```rust
for func in &typed.functions {
    let func_id = declare_kata_function(func, ...);
    symbol_table.insert(key, func_id);
}
for func in &typed.functions {
    define_kata_function(func, func_id, ...);
}
```

Novo: quando `needs_split(func)` é true, declara **duas** funções:

```rust
for func in &typed.functions {
    let func_id = declare_kata_function(func, &cranelift_name, ...);
    symbol_table.insert(key, func_id);  // símbolo público = wrapper

    if needs_split(func) {
        let inner_name = format!("__kata_fn_inner_{fn_counter}");
        let inner_id = declare_kata_function(func, &inner_name, ...);
        inner_table.insert(key, inner_id);  // símbolo privado = inner
    }
}
```

`needs_split(func)`:
```rust
fn needs_split(func: &TypedFunction) -> bool {
    let has_epilogue_intrinsics = func.cache_spec.is_some()
        || func.timer_spec.is_some();
    // TODO: @log{when: "exit"} quando suportado
    has_epilogue_intrinsics && has_tail_pos_call(&func.clauses)
}
```

### 3.3. Codegen — `function_def.rs`

`define_kata_function` bifurca:

**Caso 1: sem split** (approach atual, sem mudança):
```rust
define_function_body(name, params, ret, clauses, ..., cache_spec, timer_spec, ...)
```

**Caso 2: com split:**

a) **Definir inner**: `define_function_body` com `no_tail_calls = false`,
   `cache_spec = None`, `timer_spec = None`. O inner tem o body puro com TCO.
   Self-calls em tail position resolvem para o inner (via `kata_refs_inner`).
   Self-calls em non-tail position resolvem para o wrapper (via `kata_refs`).

b) **Definir wrapper**: função simples que faz:
   - Bind params (todos Ident — padrão único)
   - Prólogo: `[timer_start]`, `[cache_lookup → hit? return]`
   - `call inner(rt, arena, box, args...)`
   - Epílogo: `[cache_insert]`, `[timer_stop + publish]`, `return result`

### 3.4. Codegen — `closure.rs` (resolução de self-calls)

Hoje, `closure.rs:263`:
```rust
if let Some(&func_ref) = ctx.kata_refs.get(&key) {
    if expr.tail_pos && !ctx.no_tail_calls {
        ctx.builder.ins().return_call(func_ref, &tail_args);  // TCO
    } else {
        ctx.builder.ins().call(func_ref, &call_args);          // normal
    }
}
```

Novo: quando o split está ativo, a resolução usa `kata_refs_inner` para
tail calls e `kata_refs` (wrapper) para non-tail:

```rust
if expr.tail_pos && !ctx.no_tail_calls {
    // Tail call → inner (TCO)
    let func_ref = ctx.kata_refs_inner.get(&key)
        .or_else(|| ctx.kata_refs.get(&key))
        .expect("func_ref");
    ctx.builder.ins().return_call(func_ref, &tail_args);
} else {
    // Non-tail call → wrapper (cache/timer)
    let func_ref = ctx.kata_refs.get(&key)
        .expect("func_ref");
    ctx.builder.ins().call(func_ref, &call_args);
}
```

Quando não há split, `kata_refs_inner` está vazio e o fallback para
`kata_refs` mantém o comportamento atual.

### 3.5. Codegen — `LowerCtx` (mod.rs)

`LowerCtx` ganha um campo:

```rust
pub kata_refs_inner: &'a HashMap<FuncKey, cranelift_codegen::ir::FuncRef>,
```

Default: referência para um `HashMap` vazio quando não há split.

### 3.6. Codegen — `module.rs` (entry point e Actions)

O entry point e as Actions referenciam funções pelo `symbol_table` (wrapper).
Sem mudança — o wrapper é o símbolo público. O inner é privado e só é
referenciado pelo wrapper e por self-calls em tail position.

Function pointers (first-class values) apontam para o wrapper. `map fat_tail
[1 2 3]` chama o wrapper, que faz cache + inner. Correto.

### 3.7. Timer — eliminação do canal buffer-1 Drop

Com o split, `@timer` + tail calls usa o wrapper approach:

```
fat_tail (wrapper)
  start = timer_now()        ← stack slot do wrapper
  call fat_tail__inner(...)
    ... return_call chain ...
      return result
  delta = timer_now() - start   ← wrapper frame sobreviveu
  publish(topic, msg)
  return result
```

**O canal buffer-1 Drop desaparece.** O `timer_use_channel` flag, o
`inject_timer_start_channel`, o `inject_timer_stop_channel` — tudo removido.
O `@timer` usa sempre stack slot no wrapper. A árvore de decisão do
PRD-timer §4.2 colapsa para um único caminho.

### 3.8. `no_tail_calls` flag

Hoje: `no_tail_calls: cache_spec.is_some()`.

Novo:
- **Inner**: `no_tail_calls: false` (sempre — TCO ativo)
- **Wrapper**: `no_tail_calls: true` (mas o wrapper só faz `call inner`, sem
  return_call — a flag é irrelevante)
- **Função sem split**: `no_tail_calls: false` (approach atual, sem mudança
  para funções sem `@cache`/`@timer`)

A condição `cache_spec.is_some() && !has_tail_pos_call(clauses)` continua
usando o approach atual (uma função, sem TCO, epílogo com cache_insert).
Isso cobre `fib` (non-tail recursion com `@cache`) — o epílogo corre
naturalmente porque `fib` não faz `return_call`.

## 4. Composição de intrínsecas no wrapper

O wrapper compõe as intrínsecas no prólogo e epílogo na ordem já definida
(PRD-timer §4.7):

**Prólogo (top-down):**
1. `@log{when: "enter"}` — log de entrada
2. `@timer` — `start = timer_now()`
3. `@cache` — serialize key, cache_lookup → hit? return cached

**Epílogo (bottom-up):**
1. `@cache` — `cache_insert(handle, key, result)`
2. `@timer` — `delta = timer_now() - start; publish`
3. `@log{when: "exit"}` — log de saída com `_return`

A ordem é a mesma já implementada em `function_def.rs`. O wrapper é
exatamente o que o `define_function_body` já faz no prólogo/epílogo — a
diferença é que o body é substituído por `call inner(args)`.

## 5. Casos que NÃO mudam

| Caso | Razão |
|---|---|
| `dobro 5` com `@cache` (sem self-calls) | `needs_split` = false (sem tail calls). Approach atual |
| `fib 35` com `@cache` (non-tail recursion) | `needs_split` = false (sem tail_pos calls). Approach atual |
| `fat_tail 5 1` sem `@cache` (tail recursion) | `needs_split` = false (sem intrínsecas). TCO puro |
| Actions com `@log` | Actions não têm TCO. Sem split |
| Entry point | SystemV call conv, sem return_call. Sem split |
| REPL redefinição | `fn_id` muda → wrapper e inner novos. Mecanismo existente |
| Interpreter | Roda TAST, não conhece Cranelift. `@cache` no interpreter é no-op |
| Monomorphization | Clona `TypedFunction`. O codegen gera o par por instância |
| Tree shaking | Roda antes do codegen. Vê uma função chamada → mantém. Inner não existe na TAST |

## 6. Diferenças observáveis

### 6.1. Exit hooks com TCO: 1 vez em vez de N

Hoje (sem TCO), `@log{when: "exit"}` dispara para cada call intermediária.
Com wrapper/inner, o Exit roda no wrapper, chamado uma vez. Intermediate tail
calls vão para o inner (sem Exit).

Consistente com `@timer` — o PRD-timer já diz "TCO mede a cadeia inteira, não
cada chamada". Exit com TCO segue a mesma semântica: observa o resultado final,
não cada step.

### 6.2. Intermediate tail calls não são cachedadas

`fat_tail(4, 5)` como step intermediário não gera entrada de cache. Se
chamado diretamente depois, é miss. Em tail recursion cada step muda os args
— reuso é improvável. Aceitável.

### 6.3. `@timer` sem canal

O `@timer` com TCO deixa de usar canal buffer-1 Drop. O delta é medido no
stack slot do wrapper. Resultado idêntico (mede outer call → inner chain →
base case → return → wrapper epílogue), mas sem overhead de canal.

## 7. Mudanças no runtime

### 7.1. Remoção do código de canal do timer

`inject_timer_start_channel` e `inject_timer_stop_channel` em `timer.rs`
podem ser removidos (ou mantidos como dead code até confirmação de que
nenhum path os usa). O `timer_use_channel` flag torna-se sempre `false`.

### 7.2. Sem mudanças no cache.rs

O runtime de cache não muda — o wrapper chama as mesmas FFIs
(`kata_rt_cache_get_or_create`, `_lookup`, `_insert`) no prólogo/epílogo.

## 8. Mudanças na FFI

Nenhuma. As FFIs de cache e timer não mudam. O wrapper chama as mesmas FFIs
que o `define_function_body` já chama hoje. O inner não chama FFIs de
cache/timer.

## 9. Testes E2E

### 9.1. Regressão — casos existentes

Os 23 testes de `cache_e2e.rs` devem continuar passando. Os casos sem tail
calls (maioria) usam o approach atual. Os casos com tail calls (`fat_tail`,
`fib_tail`) agora usam wrapper/inner, mas o resultado é o mesmo.

### 9.2. TCO preservado com `@cache`

**TCO com cache — stack não cresce:**

`fat_tail 1000000 1` com `@cache{strategy: "LRU"}` deve completar sem
stack overflow. Hoje (sem TCO), isto já funciona porque `no_tail_calls =
true` usa `call` em vez de `return_call` — mas stack cresce O(n). Com
wrapper/inner, o inner faz TCO e o stack é O(1).

Teste: `fat_tail 1000000 1` → deve retornar o fatorial e completar em tempo
razoável (não estourar stack).

**TCO com cache — resultado correto:**

`fib_tail 30 0 1` com `@cache` → 832040. Já testado, deve continuar passando.

### 9.3. TCO preservado com `@timer`

**Timer com TCO — mede cadeia inteira:**

`fat_tail 100000 1` com `@timer` → deve medir o tempo da cadeia inteira
(outer call → inner chain → base case → return), não ~0.

Teste: rodar com `@timer` e verificar que o delta é > 0 e proporcional a N.

### 9.4. `@cache` + `@timer` + TCO

`fat_tail 100000 1` com `@cache{strategy: "LRU"}` + `@timer{topic: "perfil"}`
→ deve completar (TCO), cachear o resultado (hit na 2ª chamada), e medir o
tempo (delta > 0).

### 9.5. Função mista (tail + non-tail)

```kata
@cache
f :: Int => Int
lambda 0: 0
lambda n:
    >= n 10: f (- n 1)
    otherwise: + (f (- n 1)) 1

f 15
```

- `f(15)` → wrapper → miss → inner → `f(14)` (tail → inner, TCO) → ...
- `f(9)` → inner → `+ (f(8)) 1` → `f(8)` (non-tail → wrapper, cache) → ...
- Resultado correto: 6 (f(15)=f(14)=...=f(10)=f(9)=f(8)+1=...=f(0)+6=6)

### 9.6. `@timer` sem canal — regressão

Os testes existentes de `@timer` com TCO devem continuar passando, mas agora
usando stack slot no wrapper em vez de canal. O delta medido deve ser o mesmo
(ordem de grandeza).

## 10. Decisões de design

| # | Decisão | Racional |
|---|---------|---------|
| D1 | Split só quando intrínsecas + tail calls coexistem | Minimiza impacto. Funções sem tail calls ou sem intrínsecas não mudam. Zero overhead nos casos existentes. |
| D2 | Self-call em tail position → inner; non-tail → wrapper | Tail calls não precisam de cache (a key muda a cada step). Non-tail calls precisam de cache (há reuso). A resolução é natural pela TAST. |
| D3 | Wrapper é o símbolo público | Function pointers, entry point, Actions — todos referenciam o wrapper. O inner é privado. Sem mudança na resolução externa. |
| D4 | Inner não tem intrínsecas | O inner é o body puro com TCO. Cache/timer vivem no wrapper. Simplifica o inner — uma função normal com TCO. |
| D5 | Remoção do canal buffer-1 Drop do timer | O canal existia só porque `return_call` destrói o frame. Com wrapper, o frame do wrapper sobrevive. Stack slot é mais simples, mais previsível, sem overhead de canal. |
| D6 | `no_tail_calls` deixa de ser `cache_spec.is_some()` | O inner sempre tem `no_tail_calls = false`. O wrapper tem `true` mas é irrelevante (só faz `call inner`). Funções sem split mantêm o approach atual. |
| D7 | Intermediate tail calls não são cachedadas | Em tail recursion cada step muda os args. Caching intermediate calls não traz benefício e adiciona overhead. O cache no wrapper cachesa o outermost call — o que custa caro para recomputar. |
| D8 | Exit hook dispara 1 vez com TCO | Consistente com `@timer` (mede cadeia, não cada step). Exit observa o resultado final. Se o usuário quer log de cada step, usa `@log{when: "enter"}` (dispara a cada step no inner). |
| D9 | Inner nomeado `__kata_fn_inner_N` — não colide com código do usuário | O parser já rejeita identificadores começando com `__` (`FrontendError::ReservedName`, `kata-parser/src/lib.rs:131`). O nome `__kata_fn_inner_N` é um símbolo Cranelift (nível JIT module), não um identificador Kata. Os nomes internos do codegen (`__param_{i}`, `__result`, `__kata_fn_N`) seguem o mesmo padrão e já coexistem sem colisão. |

## 11. Fora do escopo

- **`@log{when: "exit"}` com wrapper** — a mesma estrutura suporta Exit hooks
  no wrapper, mas a integração específica (interpolação de `_return`,
  template de mensagem) é uma implementação separada. O PRD descreve a
  estrutura, a integração com `@log` é futura.
- **Cache de intermediate tail calls** — intermediate calls não são
  cachedadas. Se um caso de uso real surgir onde o reuso de intermediate
  args é significativo, pode-se adicionar cache_lookup (sem insert) no inner.
  Por ora, não há evidência de que seja necessário.
- **Inlining do wrapper** — o wrapper tem 1 frame extra. Se perf for
  mensurada como problema, o otimizador pode inlinar o wrapper no caller
  (eliminando o frame extra). Adiar.
- **Múltiplos níveis de split** — se o inner também tem intrínsecas que
  precisam de epílogo, poderia haver recursão de splits. Não ocorre na
  prática: o inner não tem intrínsecas (D4).

## 12. DoDs (Definitions of Done)

### Fase 1 — Estrutura do split ✅

1. ✅ `needs_split(func)` função em `function_def.rs` detecta intrínsecas + tail calls.
2. ✅ `module.rs` declara wrapper (Export, símbolo público) + inner (Export,
   símbolo privado `__kata_fn_inner_N`) quando `needs_split` é true.
3. ✅ `inner_table: HashMap<FuncKey, FuncId>` populado para inner FuncIds.
4. ✅ `kata_refs_inner` em `LowerCtx` (default: HashMap vazio).

### Fase 2 — Codegen do inner ✅

5. ✅ `define_function_body` chamado para inner com `cache_spec = None`,
   `timer_spec = None`, `no_tail_calls = false`.
6. ✅ Self-calls em tail position resolvem para `kata_refs_inner`.
7. ✅ Self-calls em non-tail position resolvem para `kata_refs` (wrapper).
8. ✅ `closure.rs` usa `kata_refs_inner` para tail calls quando disponível.

### Fase 3 — Codegen do wrapper ✅

9. ✅ Wrapper faz: bind params → prólogo (timer/cache) → `call inner` → epílogo
   (cache_insert/timer_stop) → return.
10. ✅ Ordem do epílogo: cache_insert → timer_stop (PRD-timer §4.7).
11. ✅ Hit no cache_lookup retorna direto (não chama inner).

### Fase 4 — Remoção do canal do timer ✅

12. ✅ `timer_use_channel` removido — `define_function_body` sempre usa stack slot.
13. ✅ `inject_timer_start_channel` e `inject_timer_stop_channel` marcados
    `#[allow(dead_code)]` em `timer.rs`.
14. ✅ `@timer` + tail calls usa stack slot no wrapper.

### Fase 5 — Testes E2E ✅

15. ✅ 23 testes existentes de `cache_e2e.rs` passam sem mudança (26 total com os novos).
16. ✅ `count_down 1000000 1` com `@cache` completa sem stack overflow (`cache_tco_large_n`).
17. ✅ `count_down 100000 0` com `@timer` mede delta > 0 (`timer_tco_large_n`).
18. ✅ `count_down 100000 0` com `@cache` + `@timer` completa, cachear, medir (`cache_timer_tco`).
19. ✅ Função mista (tail + non-tail + `@cache`) produz resultado correto = 9 (`cache_mixed_tail_nontail`).
20. ✅ Testes existentes de `@timer` passam (regressão, sem canal — 6 testes em `timer_e2e.rs`).

**Total: 1872 testes, 0 falhas.**

## 13. Cronograma

| Fase | Escopo | Estimativa |
|------|--------|------------|
| 1 | Estrutura do split (module.rs, LowerCtx) | ~40 min |
| 2 | Codegen do inner (resolução de self-calls) | ~40 min |
| 3 | Codegen do wrapper (prólogo/epílogo + call inner) | ~50 min |
| 4 | Remoção do canal do timer | ~30 min |
| 5 | Testes E2E | ~40 min |

Total: ~3.5h. Build + testes: ~10 min.