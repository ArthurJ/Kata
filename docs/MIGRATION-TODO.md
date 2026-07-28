# TODO — Migração de exemplos legacy → Kata5

Inventário completo dos 29 exemplos não-legados e 112 legados feito em
2026-07-21. Abaixo: clusters priorizados para migração, ordenados por
valor didático × esforço.

## Clusters priorizados

### Cluster 1 — Tipos refinados + `Result` + `?` + `|` ✅ Concluído
- **Candidatos legacy:** `test_refined.kata`, `test_try.kata`, `test_fallback.kata`, `test_repr_refined.kata` — **removidos**
- **Destino:** `examples/refined_types.kata` — **criado** (6 blocos didáticos)
- **Estrutura:** 6 blocos — declaração + ascription + echo!, smart constructor + match, `?` em Action que retorna Result, `refines NUM` + aritmética delegada, `|` com Optional + coerção contextual, downcast + caso misto + múltiplos predicados
- **Status:** ✅ Concluído (2026-07-22). 4 scripts legacy removidos. Snapshot adicionado. 996 testes passando.

### Cluster 2 — `alias`/Newtype + `enum` predicado ✅ Concluído
- **Candidatos legacy:** `test_alias_bug.kata`, `test_imc.kata` — **removidos**
- **Destino:** `examples/alias_newtype.kata`, `examples/enum_refined_alias.kata` — **criados**
- **Estrutura:**
  - `alias_newtype.kata`: alias de primitivo (identity), construtor infalível, downcast alias→base
  - `enum_refined_alias.kata`: refined sobre Float, alias de refined (construtor falível delegante), enum predicado (IMC), `?` em Action, downcast encadeado alias→base
- **Bug fixes:**
  - `pass0.rs`: alias de refined não delegava ao construtor falível do target — gerava identity. Corrigido: copia predicados, `base_ty` e `RefinedDeclInfo` do target.
  - `ascription.rs`: downcast só olhava um nível de `alias_of`. Corrigido: percorre a cadeia de `alias_of` recursivamente (`Peso → PositiveFloat → Float`).
  - `apply.rs`: `try_refines_fallback` não seguia cadeia de `alias_of` para encontrar entradas de `refines`. Corrigido: segue alias_of quando `refines_registry.get(name)` retorna vazio.
- **Status:** ✅ Concluído (2026-07-27). 2 scripts legacy removidos. Snapshots adicionados. 1121 testes passando.

### Cluster 3 — Ranges + Tensor + `@ffi` (lacuna de coleções) ✅ Concluído
- **Candidatos legacy:** `test_range_step.kata`, `test_range_simple.kata` — mantidos em legacy como referência
- **Destino:** `examples/ranges.kata` — **criado** (4 blocos: simples, com step, inclusivo, decrescente + contains)
- **Bug fix do codegen:** `for_in`/`map`/`filter`/`fold`/`fused_stream`/`contains` assumiam step positivo e ignoravam `inclusive`. Range decrescente `[10..-1..=0]` não iterava; range inclusivo `[0..3..=10]` tratava como exclusive. Corrigido com helper `range_iter::range_done` que detecta step < 0 e flag inclusive. Runtime agora aloca 32 bytes (4ª word = flag inclusive como SMI).
- **Tensor:** `test_tensor_math.kata` ainda não migrado (bug intencional de dot com shapes incompatíveis — decisão de design pendente)
- **Status:** ✅ Concluído (2026-07-28). `ranges.kata` criado, bug de codegen corrigido, snapshot adicionado. 1146 testes passando.

### Cluster 4 — CSP expandido (parcial) ⚠️ Parcial
- **Candidatos legacy:** `test_broadcast.kata`, `test_parallel.kata`
- **Destino:** `broadcast.kata` ✅, `parallel.kata` ❌
- **Problemas:** `test_broadcast.kata` depende de `subscribe!` (não no manual moderno; `broadcast!` retorna `(Sender, ReceiverFactory)`); `test_parallel.kata` usa sintaxe legada de param (`n :: Int` em vez de `n::Int`)
- **`broadcast.kata`:** ✅ Criado (2026-07-28). Sintaxe idiomática: `let (tx, subscribe) := broadcast!()` + `subscribe!()` + `tx !> 42` + `rx <! a`. Snapshot adicionado. 1146 testes passando.
- **`parallel.kata`:** ❌ **Não implementado.** `@parallel` está documentado no sintaxe-mapa (linha 457) e no manual (§6.4), mas **não existe no resolver**. A string "parallel" não aparece em nenhum `.rs` do projeto. O resolver rejeita `@parallel` em actions com `UnknownDirective`. Necessita implementação no resolver antes de ter exemplo.

### Cluster 5 — `@log` + imports ✅ Concluído
- **Candidatos legacy:** `test_log.kata`, `test_imports.kata` — mantidos em legacy como referência
- **Destino:** `log_telemetry.kata` ✅, `imports.kata` ✅
- **`log_telemetry.kata`:** ✅ Criado (2026-07-28). Demonstra `@log` (diretiva: `when: "enter"` no prólogo, `when: "exit"` no epílogo, com `topic` e `policy` explícitos), `log!()` (action nativa de publicação explícita), e `log_recv!()` (consumo de três tópicos distintos). Três produtores em fibers separadas via `fork!()`, um consumidor que recebe de "default", "metrics", e "events". Snapshot adicionado.
- **`imports.kata`:** ✅ Criado (2026-07-28). Demonstra `import modules.mock_math` (WholeModule — acesso via `mock_math.dobrar`) e `import modules.mock_math.(triplicar)` (Selective — `triplicar` no escopo direto). Usa `examples/modules/mock_math.kata` já migrado. Snapshot adicionado.
- **Diferenças do legacy:** `@log` usa `msg` (não `format`), `when` obrigatório, `{expr}` interpola expressão do escopo (não placeholders mágicos). `log_recv!("topic")` substitui `log_subscribe!()` + `<! rx`. `$()` removido — chamadas diretas. `mock_math` em `examples/modules/` (não legacy).
- **Status:** ✅ Concluído (2026-07-28). 1146 testes passando.

### Cluster 6 — `@test` + `assert!`
- **Candidato legacy:** `test_assert.kata`
- **Problemas:** `assert_eq!` não existe no manual moderno (só `assert!` 1 ou 2 args); `test_pure_equality` usa `() => Bool` em assinatura (legado). Migração direta impossível sem decisão de design.

### Cluster 7 — Closure escape + tipo função `->`
- **Candidato legacy:** `test_make_adder_typed.kata`
- **Destino:** `closure_escape.kata`
- **Problemas:** `lambda n :: Int:` (sintaxe legada de tipo em param lambda), `echo! a` (sem parênteses). Migração simples.

## Prioridade de migração (ordem por valor × esforço)

1. `test_fallback.kata` → `refined_fallback.kata` — limpo, preenche lacuna dupla (`|` + refined), zero retrabalho
2. `test_refined.kata` → fundir com acima em `refined_types.kata` — preenche lacuna central, retrabalho de pattern só
3. `test_alias_bug.kata` → `alias_newtype.kata` — limpo, preenche lacuna de `alias`
4. `test_make_adder_typed.kata` → `closure_escape.kata` — preenche lacuna de `->` + escape, retrabalho mínimo
5. `test_range_step.kata` → `ranges.kata` — preenche lacuna de ranges, retrabalho de `format` só
6. `test_imc.kata` → `enum_refined_alias.kata` — mais rico, mais retrabalho (decisão sobre `/` e `|`)
7. `test_parallel.kata` → `parallel.kata` — preenche `@parallel`, retrabalho de assinatura
8. `test_log.kata` → `log_telemetry.kata` — único candidato a `@log`, mas é reescrita

## Bloqueios conhecidos (não tentar até resolver)

- **Interoperabilidade refined→base:** `echo!`, `show`, `+` rejeitam `PositiveInt`. O typeck não implementa widening de refined para tipo base. Arthur decidiu discutir o design disso antes de continuar o Cluster 1.
- **`failure_test_*.kata`:** `@test{expects: "CompileError"}` não implementado (C1). NÃO migrar.
- **`mod`/`and` em lambdas:** bug de codegen (`collect_free_vars` marca DispatchTable functions como free vars). Usar só FFI primitives em HOF callbacks até o fix.
- **TRMA multi-clause:** `trma.rs:94` exige 1 clause; multi-clause não otimiza. Usar `match` explicit form para TRMA-eligible.