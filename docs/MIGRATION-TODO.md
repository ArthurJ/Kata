# TODO — Migração de exemplos legacy → Kata5

Inventário completo dos 29 exemplos não-legados e 112 legados feito em
2026-07-21. Abaixo: clusters priorizados para migração, ordenados por
valor didático × esforço.

## Clusters priorizados

### Cluster 1 — Tipos refinados + `Result` + `?` + `|` (lacuna grave)
- **Candidatos legacy:** `test_refined.kata`, `test_try.kata`, `test_fallback.kata`, `test_repr_refined.kata`
- **Destino:** `examples/refined_types.kata` (arquivo único, narrativa didática)
- **Estrutura acordada:** 6 blocos — declaração + ascription, smart constructor + match, `?` em action, `|` com coerção, `|` com Optional refined, múltiplos predicados + demo
- **Status:** INTERROMPIDO no Bloco 1
- **Bloqueio descoberto:** refined types (`Ty::Struct("PositiveInt")`) não interoperam com operações base (`+`, `show`, `echo!`). `echo!(a)` onde `a :: PositiveInt` → `type.no_overload` (SHOW não sintetizado para refined). `show a` também falha. `+ a 0` também falha. O manual §4.2.10 diz que refined deveria interoperar com operações base (flexibilidade estrutural), mas o typeck não implementa widening/coerção refined→base. Ver discussão "interoperabilidade refined" com Arthur (2026-07-21).
- **Pontos de incerteza ainda não validados:**
  - `?` em Action com `Result` (nenhum teste E2E exercita; só legado `test_try.kata`)
  - `PositiveInt n` onde `n` é variável (testes E2E só usam literais)
  - `Optional::Some(n::PositiveInt)` com `n` variável → type error (ascription-refined exige literal, §4.2.8)

### Cluster 2 — `alias`/Newtype + `enum` predicado (lacuna grave)
- **Candidatos legacy:** `test_alias_bug.kata`, `test_imc.kata`
- **Destino:** `alias_newtype.kata`, `enum_refined_alias.kata`
- **Problemas:** `test_imc.kata` usa `|` na declaração de enum (sintaxe removida), `| 1` como fallback de divisão esconde que `/` exige `NonZero` (§22.1), `typed_input!` comentado

### Cluster 3 — Ranges + Tensor + `@ffi` (lacuna de coleções)
- **Candidatos legacy:** `test_range_step.kata`, `test_tensor_math.kata`
- **Destino:** `ranges.kata`, `tensor.kata`
- **Problemas:** `test_tensor_math.kata` tem bug intencional (dot com shapes incompatíveis → Type Mismatch) — precisa decidir: remover o mismatch ou converter em teste negativo `expects: "CompileError"`

### Cluster 4 — CSP expandido (lacuna parcial)
- **Candidatos legacy:** `test_broadcast.kata`, `test_parallel.kata`
- **Destino:** `broadcast.kata`, `parallel.kata`
- **Problemas:** `test_broadcast.kata` depende de `subscribe!` (não no manual moderno; `broadcast!` retorna `(Sender, ReceiverFactory)`); `test_parallel.kata` usa sintaxe legada de param (`n :: Int` em vez de `n::Int`)

### Cluster 5 — `@log` + imports (lacuna de diretivas)
- **Candidatos legacy:** `test_log.kata`, `test_imports.kata`
- **Destino:** `log_telemetry.kata`, `imports.kata`
- **Problemas:** `test_log.kata` usa API legada (`log_subscribe!` → `log_recv!` moderno, campos `@log` divergentes: `format` vs `msg`, placeholders `name`/`elapsed` não documentados); `test_imports.kata` depende de `mock_math` módulo legado

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