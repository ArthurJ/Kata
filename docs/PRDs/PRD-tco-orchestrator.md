# PRD — Generalização do wrapper/inner split: TCO como orquestrador + TAST estruturada para diretivas

**Status:** Implementado (Fases 1-7, 28/28 DoDs ✅)
**Data:** 2026-08-28
**Depende de:** PRD-wrapper-inner-tco ✅ (Fases 1-5 implementadas — wrapper/inner split para `@cache`/`@timer`)
**Não depende de:** Diretivas customizadas novas, Fio 13+

## 1. Objetivo

Generalizar o wrapper/inner split para que:
1. O **TCO** seja o orquestrador do split — não as diretivas. O TCO detecta tail calls e cria a estrutura wrapper/inner. As diretivas associam seus comportamentos de prólogo/epílogo ao wrapper.
2. Diretivas **customizadas** (`@log{when: "exit"}`) coexistam com TCO. Hoje o desugar destrói `tail_pos` envolvendo o retorno em `let __result := ...; <diretiva>; __result`. A TAST estruturada preserva o body original separado do código sintético.

### Princípio: o split é disparado pelo TCO, não pelas diretivas

Hoje: `needs_split = has_epilogue_intrinsics && has_tail_pos_call`. As diretivas decidem se há split via `has_epilogue_intrinsics`.

Proposta: o TCO detecta tail calls. Se há tail calls, o wrapper é criado. As diretivas se acoplam ao wrapper se ele existe; caso contrário, fazem inline no body (approach atual).

### Princípio: o desugar é anotativo, não destrutivo

Hoje: `apply_exit_to_lambda_body` substitui `body` por `Expr::Block { let __result := body; ...; __result }`. A informação "este body tinha tail calls antes do desugar" se perde.

Proposta: `TypedLambdaClause` separa o body original do código sintético. O desugar popula campos `synthetic_pre`/`synthetic_post` em vez de reescrever `body`. O typeck infere `tail_pos` no `original_body` (preservando a informação). O codegen decide: split (original→inner, sintético→wrapper) ou inline (sintético no body, approach atual).

## 2. Mecanismo

### 2.1. TAST estruturada — `TypedLambdaClause`

Hoje (`typed_pattern.rs:83`):

```rust
pub struct TypedLambdaClause {
    pub patterns: Vec<Spanned<TypedPattern>>,
    pub body: Spanned<TypedExpr>,
    pub guards: Vec<TypedGuardClause>,
    pub with_bindings: Vec<TypedWithBinding>,
}
```

Novo:

```rust
pub struct TypedLambdaClause {
    pub patterns: Vec<Spanned<TypedPattern>>,
    /// Body original do usuário — preserva tail_pos.
    pub body: Spanned<TypedExpr>,
    /// Código injetado antes do body por diretivas (Enter hooks).
    /// Vazio quando não há diretivas. Vai para o wrapper quando há split.
    pub synthetic_pre: Vec<Spanned<TypedExpr>>,
    /// Código injetado em cada ponto de saída por diretivas (Exit hooks).
    /// Vazio quando não há diretivas. Vai para o wrapper (epílogo) quando há split.
    /// Quando não há split, é inserido inline em cada retorno do body.
    pub synthetic_post: Vec<Spanned<TypedExpr>>,
    pub guards: Vec<TypedGuardClause>,
    pub with_bindings: Vec<TypedWithBinding>,
}
```

Quando `synthetic_pre` e `synthetic_post` são vazios (maioria das funções), o comportamento é idêntico ao atual. O custo é dois `Vec` vazios por cláusula — negligenciável.

### 2.2. Desugar anotativo

Hoje (`lambda_hooks.rs:103`): `apply_exit_to_lambda_body` reescreve `body`:

```
body → Expr::Block {
    let __result := <body>
    <bindings estáticos>
    <_args binding>
    <args do site>
    <_return binding>
    <body da diretiva>
    __result
}
```

Novo: o desugar **não reescreve** `body`. Em vez disso:

- **Enter hooks**: código injetado vai para `synthetic_pre`. O `body` permanece inalterado.
- **Exit hooks**: código do epílogo vai para `synthetic_post`. O `body` permanece inalterado. Os bindings estáticos (`_name`, `_args`, `_return`) e args do site são parte de `synthetic_post`.

O `__result` não é mais um `let` no body — o codegen/interp avalia `body` e usa o resultado como `_return` no `synthetic_post`.

### 2.3. Typeck — preservação de tail_pos

Hoje: o typeck vê `Expr::Block { let __result := fat_tail(...); ...; __result }` e marca `tail_pos: false` na call `fat_tail(...)` — está dentro de um `let`, não em tail position.

Novo: o typeck vê `body = fat_tail(...)` (sem o `let` wrapper) e marca `tail_pos: true`. `has_tail_pos_call` retorna true. O split é ativado.

O typeck precisa saber que `synthetic_pre` e `synthetic_post` existem, mas **não os infere como parte do body**. Eles são inferidos separadamente (para validação de tipos e binding de variáveis de reflexão como `_name`, `_args`, `_return`).

### 2.4. Codegen — split centralizado no TCO

Hoje: `needs_split(func) = has_epilogue_intrinsics && has_tail_pos_call`. `has_epilogue_intrinsics` checa `cache_spec.is_some() || timer_spec.is_some()`.

Novo: `needs_split(func) = has_tail_pos_call(&func.clauses) && has_wrapper_content(func)`.

`has_wrapper_content` pergunta: "há **algo** para o wrapper fazer além de `call inner; return`?" Isso inclui:

1. Intrínsecas chumbadas (`cache_spec`, `timer_spec`) — como hoje
2. `synthetic_pre` não-vazio em qualquer cláusula
3. `synthetic_post` não-vazio em qualquer cláusula

Se a função tem tail calls mas nenhum conteúdo para o wrapper (sem diretivas, sem intrínsecas), não há split — TCO puro, como hoje. Zero overhead.

### 2.5. Composição do wrapper

O wrapper compõe, em ordem:

**Prólogo (top-down):**
1. Bind params
2. `synthetic_pre` (Enter hooks — código das diretivas customizadas)
3. `@timer` start (intrínseca chumbada)
4. `@cache` lookup (intrínseca chumbada) → hit? return cached
5. `call inner(rt, arena, box, args...)`

**Epílogo (bottom-up):**
1. result = block_param
2. `@cache` insert (intrínseca chumbada)
3. `@timer` stop + publish (intrínseca chumbada)
4. `synthetic_post` (Exit hooks — código das diretivas customizadas, com `_return = result`)
5. return result

A ordem preserva a semântica atual: chumbadas executam antes de customizadas no prólogo e depois no epílogo. Se necessário, a ordem pode ser ajustada (ex: `synthetic_pre` antes de timer start para que `_name` esteja disponível).

### 2.6. Interpreter — consumo da TAST estruturada

O interpreter consome `clause.body` via `eval_tail` (`eval.rs:1406`). Hoje, o `@log{exit}` injetado pelo desugar já está no `body` como `Expr::Block` — o interpreter executa naturalmente.

Novo: o interpreter precisa saber que `synthetic_pre` e `synthetic_post` existem. Quando não há split (função sem tail calls), o interpreter avalia:

```
avaliar synthetic_pre (bindings de reflexão, código da diretiva enter)
resultado := eval_tail(body)
avaliar synthetic_post (bindings de reflexão, código da diretiva exit, com _return = resultado)
retornar resultado
```

Quando há split, o interpreter não precisa do wrapper/inner — ele já faz trampoline TCO no `eval_tail`. O `synthetic_pre` e `synthetic_post` são avaliados uma vez (na chamada externa), e o trampoline executa o `body` puro com TCO.

Isso é consistente com a semântica do wrapper/inner no codegen: o wrapper executa intrínsecas uma vez, o inner faz TCO.

### 2.7. Optimizer — preservação da estrutura

O optimizer (tree shaking, comptime, stream fusion) opera sobre a TAST. Hoje, o código do `@log` injetado é TAST normal e pode ser otimizado.

Novo: `synthetic_pre` e `synthetic_post` são campos separados. O optimizer precisa:
- Não mover nós entre `body` e `synthetic_pre`/`synthetic_post`
- Pode otimizar dentro de cada campo independentemente
- Tree shaking vê referências em todos os campos

## 3. O que muda por camada

### 3.1. `kata-ast` — `LambdaClause` (AST)

A AST `LambdaClause` (usada pelo parser) não muda — ela não tem noção de diretivas. O desugar lê `custom_directives` do `FunctionDef` e produz os campos `synthetic_pre`/`synthetic_post` na TAST.

### 3.2. `kata-inference/src/typed_pattern.rs` — `TypedLambdaClause`

Adicionar `synthetic_pre: Vec<Spanned<TypedExpr>>` e `synthetic_post: Vec<Spanned<TypedExpr>>`. Default: `Vec::new()`.

### 3.3. `kata-inference/src/desugar_directives/lambda_hooks.rs`

`apply_directives_to_lambda_clause` deixa de reescrever `clause.body`. Em vez disso:
- Enter hooks: código vai para `synthetic_pre`
- Exit hooks: código vai para `synthetic_post`
- `body` permanece inalterado

`apply_enter_to_lambda_body` e `apply_exit_to_lambda_body` mudam de assinatura: em vez de receber e retornar `Spanned<Expr>`, recebem o body original e populam `synthetic_pre`/`synthetic_post`.

### 3.4. `kata-inference/src/desugar_directives/action_hooks.rs`

Actions não têm TCO. O desugar de actions **não muda** — continua reescrevendo o body. A TAST estruturada é específica de funções puras (que são onde TCO existe).

### 3.5. `kata-inference/src/infer/` — typeck

O typeck precisa:
- Inferir `body` normalmente (com `tail_pos` preservado)
- Inferir `synthetic_pre` e `synthetic_post` (para validação de tipos e binding de variáveis de reflexão)
- Disponibilizar `_name`, `_args`, `_return` no escopo ao inferir `synthetic_post`

### 3.6. `kata-codegen/src/lowering/function_def.rs`

`needs_split` muda:

```rust
fn needs_split(func: &TypedFunction) -> bool {
    if !has_tail_pos_call(&func.clauses) {
        return false;
    }
    // Há conteúdo para o wrapper além de call inner; return?
    let has_chumbed = func.cache_spec.is_some() || func.timer_spec.is_some();
    let has_custom = func.clauses.iter().any(|c|
        !c.synthetic_pre.is_empty() || !c.synthetic_post.is_empty()
    );
    has_chumbed || has_custom
}
```

`define_wrapper` muda: além das intrínsecas chumbadas (timer, cache), compõe `synthetic_pre` no prólogo e `synthetic_post` no epílogo.

`define_function_body` (inner e funções sem split) muda: quando não há split, insere `synthetic_pre` antes do body e `synthetic_post` em cada retorno do body. Quando há split, o inner recebe só `body` (sem sintético).

### 3.7. `kata-codegen/src/lowering/closure.rs`

Sem mudança. A resolução tail vs non-tail já funciona via `kata_refs_inner`. O inner executa `body` puro (sem `synthetic_pre`/`synthetic_post`).

### 3.8. `kata-interp` — interpreter

`eval_tail` e `call_typed_clauses` precisam:
- Avaliar `synthetic_pre` antes do body
- Avaliar `synthetic_post` após o resultado do body (com `_return` bindado)
- O trampoline TCO executa só o `body` — `synthetic_pre`/`synthetic_post` executam uma vez na chamada externa

### 3.9. `kata-inference/src/infer/walk/` — visitors

`immut.rs` e `mut_vis.rs` percorrem `clause.body`. Precisam também percorrer `synthetic_pre` e `synthetic_post` (para free_vars, captures, etc).

## 4. Casos que NÃO mudam

| Caso | Razão |
|---|---|
| Função sem diretivas + TCO | `synthetic_pre`/`synthetic_post` vazios, sem intrínsecas. `needs_split` = false. TCO puro |
| Função sem TCO + `@cache` | `has_tail_pos_call` = false. `needs_split` = false. Approach atual |
| `fib 35` com `@cache` (non-tail) | `has_tail_pos_call` = false. Sem split |
| Actions com `@log` | Actions não têm TCO. Desugar de actions não muda |
| `@log{when: "enter"}` + TCO | Enter vai para `synthetic_pre`. Body preserva tail_pos. Split ativado. Enter executa no wrapper, TCO no inner |
| `@associative` + TCO | `@associative` não injeta código. `synthetic_pre`/`synthetic_post` vazios. Sem split |

## 5. Casos que mudam

| Caso | Antes | Depois |
|---|---|---|
| `@log{when: "exit"}` + TCO | Desugar envolve retorno em `let`, destrói tail_pos. Stack O(n) | `synthetic_post` separado. tail_pos preservado. Split: exit no wrapper, TCO no inner. Stack O(1) |
| `@cache` + TCO | Wrapper/inner split (já implementado) | Igual — chumbadas continuam no wrapper |
| `@timer` + TCO | Wrapper/inner split (já implementado) | Igual |
| `@log{enter}` + `@cache` + TCO | `@log` no body (desugar), `@cache` no codegen. tail_pos destruído pelo enter? Não — enter prependa, não envolve. TCO preservado, mas enter dispara a cada iteração | Enter em `synthetic_pre` (wrapper). Cache no wrapper. TCO no inner. Enter dispara 1 vez (wrapper) |

## 6. Diferenças observáveis

### 6.1. `@log{exit}` com TCO: 1 vez em vez de N

Hoje (sem TCO): `@log{exit}` dispara em cada chamada intermediária (stack O(n), cada frame executa o exit). Com a TAST estruturada + split: exit executa 1 vez no wrapper (resultado final). Consistente com `@timer` (mede cadeia, não cada step).

### 6.2. `@log{enter}` com TCO: 1 vez em vez de N

Hoje: `@log{enter}` prependa código antes do body. Se TCO está ativo, `return_call` salta para o entry da função, que inclui o código do enter — enter dispara em **cada iteração** da chain. Stack O(1), mas enter N vezes.

Com a TAST estruturada + split: enter em `synthetic_pre` (wrapper). TCO no inner. Enter dispara 1 vez (chamada externa). Stack O(1), enter 1 vez.

Isso é uma **mudança semântica**: hoje enter dispara N vezes com TCO, depois dispara 1 vez. Se o usuário depende de "enter a cada iteração", precisa usar `@log` no body explicitamente (não como diretiva).

### 6.3. `_return` em `synthetic_post`

`_return` é a variável sintética que recebe o resultado da função. Em `synthetic_post`, `_return` é o resultado do inner (com split) ou do body (sem split). O binding é responsabilidade do codegen/interp, não do desugar.

## 7. Migração

### 7.1.Compatibilidade com TAST existente

A adição de `synthetic_pre` e `synthetic_post` a `TypedLambdaClause` é aditiva. Todo código que constrói `TypedLambdaClause` precisa inicializar os novos campos como `Vec::new()`. Sites:

- `function_infer.rs` — construção principal
- `apply_lambda.rs` — lambda anônimo
- `constructors.rs` / `constructors_enum_pred.rs` / `constructors_refined.rs` — construtores
- Testes e helpers que constroem `TypedLambdaClause`

### 7.2. Ordem de implementação

1. **Adicionar campos** — `synthetic_pre`/`synthetic_post` em `TypedLambdaClause`, default vazio. Compilar, testar (deve passar — campos vazios = comportamento atual).
2. **Desugar anotativo** — mudar `lambda_hooks.rs` para popular `synthetic_pre`/`synthetic_post` em vez de reescrever `body`. Testar: testes de `@log` devem continuar passando (o codegen/interp ainda precisa consumir os novos campos).
3. **Codegen sem split** — `define_function_body` insere `synthetic_pre` antes do body e `synthetic_post` em cada retorno. Testar: `@log` sem TCO funciona igual.
4. **Interpreter** — `eval_tail`/`call_typed_clauses` avalia `synthetic_pre`/`synthetic_post`. Testar: `@log` no interp funciona.
5. **`needs_split` generalizado** — mudar para `has_tail_pos_call && has_wrapper_content`. Testar: `@log{exit}` + TCO ativa split.
6. **`define_wrapper` compõe sintético** — `synthetic_pre` no prólogo, `synthetic_post` no epílogo. Testar: `@log{exit}` + TCO = stack O(1), exit 1 vez.
7. **Typeck** — inferir `synthetic_pre`/`synthetic_post` (validação de tipos, binding de `_return`). Testar: tipos corretos.
8. **Optimizer/visitors** — percorrer novos campos. Testar: free_vars, captures corretos.

### 7.3. Actions não mudam

O desugar de actions (`action_hooks.rs`) continua reescrevendo o body. Actions não têm TCO, não têm `TypedLambdaClause`, não têm split. A TAST estruturada é específica de funções puras.

## 8. Decisões de design

| # | Decisão | Racional |
|---|---------|---------|
| D1 | TCO é o orquestrador do split | O TCO detecta tail calls. Se há conteúdo para o wrapper, o split é ativado. As diretivas não decidem — elas se acoplam. |
| D2 | `has_wrapper_content` pergunta se há **algo** para o wrapper | Inclui chumbadas (`cache_spec`, `timer_spec`) e customizadas (`synthetic_pre`/`synthetic_post` não-vazios). Generaliza sem enumerar diretivas. |
| D3 | `synthetic_pre`/`synthetic_post` em `TypedLambdaClause` | Separa body original de código sintético na TAST. O typeck preserva `tail_pos` no body. O codegen decide onde colocar cada parte. |
| D4 | Desugar é anotativo, não destrutivo | O desugar popula campos em vez de reescrever `body`. A informação "tinha tail calls antes do desugar" é preservada. |
| D5 | Actions não mudam | Actions não têm TCO. O desugar de actions continua reescrevendo o body. A TAST estruturada é específica de funções puras. |
| D6 | `@log{enter}` dispara 1 vez com TCO (não N) | Hoje dispara N vezes porque `return_call` salta para o entry. Com split, enter está no wrapper (1 vez). Mudança semântica consistente com `@timer` e `@log{exit}`. |
| D7 | `synthetic_post` associado ao conceito de retorno, não a posição fixa | Uma cláusula com guards tem múltiplos pontos de saída. `synthetic_post` executa em cada retorno do inner (sem split) ou no epílogo do wrapper (com split). |
| D8 | Chumbadas executam antes de customizadas no prólogo e depois no epílogo | Preserva a ordem atual. Se `_name` precisa estar disponível em `synthetic_pre`, a ordem pode ser ajustada. |

## 9. Fora do escopo

- **`@log{when: "shortcircuit"}` com TCO** — ShortCircuit não se aplica a funções puras (o resolution já rejeita). Sem mudança.
- **`@log{when: "transform"}` com TCO** — Transform não se aplica a funções puras. Sem mudança.
- **Múltiplos níveis de split** — Se o inner também tem diretivas que precisam de epílogo, poderia haver recursão de splits. Não ocorre: o inner recebe `body` puro (sem `synthetic_pre`/`synthetic_post`).
- **Inlining do wrapper** — O wrapper tem 1 frame extra. Se perf for mensurada como problema, o otimizador pode inlinar. Adiar.

## 10. DoDs (Definitions of Done)

### Fase 1 — TAST estruturada ✅

1. ✅ `synthetic_pre` e `synthetic_post` adicionados a `TypedLambdaClause` com default `Vec::new()`.
2. ✅ Todos os sites que constroem `TypedLambdaClause` inicializam os novos campos.
3. ✅ `cargo test --workspace` passa (campos vazios = comportamento atual).

### Fase 2 — Desugar anotativo ✅

4. ✅ `apply_enter_to_lambda_body` popula `synthetic_pre` em vez de reescrever `body`.
5. ✅ `apply_exit_to_lambda_body` popula `synthetic_post` em vez de reescrever `body`.
6. ✅ `body` permanece inalterado após o desugar.
7. ✅ Testes de `@log` (sem TCO) passam com o codegen/interp consumindo os novos campos.

### Fase 3 — Codegen sem split ✅

8. ✅ `define_function_body` insere `synthetic_pre` antes do body quando não há split.
9. ✅ `define_function_body` insere `synthetic_post` em cada retorno quando não há split.
10. ✅ Testes de `@log` (sem TCO) passam no codegen.

### Fase 4 — Interpreter ✅

11. ✅ `eval_tail`/`call_typed_clauses` avalia `synthetic_pre` antes do body.
12. ✅ `eval_tail`/`call_typed_clauses` avalia `synthetic_post` após o resultado.
13. ✅ Testes de `@log` (sem TCO) passam no interpreter.

### Fase 5 — Split generalizado ✅

14. ✅ `needs_split` usa `has_tail_pos_call && has_wrapper_content`.
15. ✅ `has_wrapper_content` checa chumbadas + `synthetic_pre`/`synthetic_post` não-vazios.
16. ✅ `define_wrapper` compõe `synthetic_pre` no prólogo e `synthetic_post` no epílogo.
17. ✅ `@log{exit}` + TCO: split ativado, exit no wrapper, TCO no inner. Stack O(1).
18. ✅ `@log{enter}` + TCO: split ativado, enter no wrapper (1 vez), TCO no inner.
19. ✅ `@log{exit}` + `@cache` + TCO: ambos no wrapper, TCO no inner.
20. ✅ Testes existentes de `@cache`/`@timer` + TCO continuam passando.

### Fase 6 — Typeck e optimizer ✅

21. ✅ Typeck infere `synthetic_pre`/`synthetic_post` (tipos, bindings de reflexão).
22. ✅ `_return` disponível no escopo de `synthetic_post`.
23. ✅ Visitors (`walk/immut.rs`, `walk/mut_vis.rs`) percorrem novos campos.
24. ✅ Free vars, captures corretos com `synthetic_pre`/`synthetic_post`.

### Fase 7 — Regressão ✅

25. ✅ 1876 testes passam (1872 originais + 4 novos da Fase 5).
26. ✅ Teste E2E: `@log{exit}` + função tail-recursive + n=100000 — completa sem stack overflow.
27. ✅ Teste E2E: `@log{enter}` + função tail-recursive + n=100000 — enter dispara 1 vez (não 100000).
28. ✅ Teste E2E: `@log{exit}` + `@cache` + função tail-recursive — resultado correto, exit 1 vez, cache hit funciona.

## 11. Cronograma

| Fase | Escopo | Estimativa |
|------|--------|------------|
| 1 | TAST estruturada (campos + inicialização) | ~30 min |
| 2 | Desugar anotativo (lambda_hooks.rs) | ~50 min |
| 3 | Codegen sem split (define_function_body) | ~40 min |
| 4 | Interpreter (eval_tail) | ~40 min |
| 5 | Split generalizado (needs_split + define_wrapper) | ~60 min |
| 6 | Typeck + optimizer + visitors | ~50 min |
| 7 | Testes E2E + regressão | ~40 min |

Total: ~5h. Build + testes: ~10 min.