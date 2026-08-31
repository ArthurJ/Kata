# PRD — Exaustividade Aninhada (Maranget + Z3 em Guards + Refined)

**Status:** 🟡 Fase 0 ✅ — Fase 1 ✅ (passos 1-4) — Fase 2 ✅ (passos 1-6) — Fase 3 não iniciada
**Data:** 2026-08-30
**Tipo:** Planejamento — PRD único, 5 fases sequenciais
**Depende de:** `PRD-exaustividade.md` ✅ (guards via Z3, patterns de 1 nível)

## 1. Objetivo

Fechar a classe de bugs estruturais em que os 3 checkers de cobertura
(exaustividade de `match`, exaustividade de cláusulas lambda, redundância
de cláusulas) ignoram o **payload** de patterns compostos (`Variant`,
`Cons`, `Tuple`), e unificar os três num motor único de usefulness
(Maranget), com Z3 compondo nas folhas — guards e predicados de tipos
refined — nunca substituindo o motor.

## 2. Motivação — 4 bugs da mesma classe estrutural

Reproduzidos em `f64eff8` (2026-08-30), binário `target/debug/kata`,
probes em `tests/probe-nested/` (oráculos in-repo):

| # | Achado | Evidência |
|---|--------|-----------|
| 1 | Exaustividade ignora payload: `Some True` + `None` aceito sem `Some False` | probeA compila (exit 0); probeB com `Some False` → **SIGILL exit 132** (`trap user(1)` de `lower_clause_chain`, `clause.rs:260`) |
| 1b | Redundância ignora payload: `lambda Optional::Some True:` + `lambda Optional::Some False:` → 2ª rejeitada como `type.redundant_clause` | probeE2, exit 1 |
| 2 | Panic: `lambda Some True:` (desqualificado) em função de 1 param parseia como 2 patterns → `index out of bounds` em `helpers.rs:104` | probeE, exit 101 |
| 2b | Codegen rejeita `echo!(None)`: Closure com callee não-Ident | **fora deste PRD**, ver §11 |

Diagnóstico: **3 checkers ortogonais** com cobertura duplicada
(`check_exhaustiveness` — `patterns.rs:442`;
`check_clause_exhaustiveness` — `function_infer.rs:250`;
`pattern_covers` — `redundancy.rs:165`), todos operando por nomes de
variantes de 1 nível com sentinela-string `__ANY__`. O runtime já está
correto — o SIGILL do probeB é o trap de defesa disparando porque **o
checker deixou passar**.

## 3. Decisões de design

1. **Maranget é o objetivo.** Motor de matriz/usefulness com witnesses
   como subproduto. Alternativa tudo-Z3 (axiomatizar datatypes) rejeitada:
   queries gigantes → mais `Unknown` → mais `MissingOtherwise` espúrios.
2. **Sem contrato interino.** Payload infinito → `NonExhaustiveMatch`
   com witness. `match r { Ok 0: ... Err _: ... }` sobre `Result::(Int, Text)`
   permanece verde (aceito por cegueira) **até a Fase 2** — é o estado
   interim, não permanente. A Fase 2 (motor Maranget) torna-o
   `NonExhaustiveMatch missing: ["Ok _"]`.
3. **Parser aridade-consciente.** Última posição da cláusula usa
   `parse_match_pattern`; demais mantêm `parse_pattern`. Aridade vem da
   assinatura nos 4 callers de `parse_sig_clauses` (`sig.rs:48`,
   `interface_decl.rs:150/378/501`). Payload entre parênteses (`Ok(v)`)
   funciona em qualquer posição — `(` é delimitador sintático, não
   exige semântica (ver §5.3).
4. **Composição motor→Z3 na folha.** Maranget especializa estruturalmente
   e emite queries Z3 escopadas por célula; Z3 nunca enxerga datatype.
   `Unknown` → `MissingOtherwise` local à folha.
5. **Refined na folha como Fase 4.** Predicado do refined é premissa a
   mais na mesma query de folha — mesmo mecanismo, sem motor novo.
6. **Redundância de match arms → ERRO, com isenção de `otherwise`.**
   Braço inútil → `RedundantClause`; `otherwise` inútil (defensivo
   pós-cobertura) → silêncio (idioma sancionado pelo corpus:
   `enum_refined_alias.kata:54-57`, `refined_types.kata:26-29`).
7. **Rational → Fase 5.** Sem literal Rational na linguagem, sem suporte
   no `const_eval_predicate` nem no `z3_translate.rs`. Fase 4 fica
   restrita a Int/Float; Fase 5 orça a extensão completa.
8. **Panic #2 → `ArityMismatch`** (bound-check na Fase 1).
9. **Codegen `echo!(None)` (#2b) fora do PRD** — PRD próprio.

## 4. Fase 0 — `parse_match_pattern` recursivo

Sub-patterns de variantes (linhas 82, 88, 129 de `patterns.rs`)
chamam `self.parse_pattern()` (allow=false). Mudar para
`self.parse_match_pattern()` quando `allow_unqualified_variant` for
true. Efeito: `Some(Some(True))` em match arm parseia como
`Variant{Some, [Variant{Some, [True]}]}` — aninhamento correto.

**Não quebra casos verdes.** Variant unitária seguida de token
(`Some True False`) → typeck rejeita (unitária sem payload). Binding
seguido de token (`x 42`) → typeck rejeita (não é variante). Apenas
casos que eram parse error viram verde (aninhamento) ou typeck error
(mensagem melhor).

**Pré-requisito:** desembrulhamento de `(p)` sem vírgula em
`parse_tuple_pattern` (já aplicado).

**Cleanup:** o ramo `else if LParen` (linhas 83-101) é código morto
— `can_start_pattern()` inclui `LParen`, captura `(` antes. Remover
após Fase 0.

DoD: `Some(Some(True))` compila e roda; suite verde; clippy limpo;
ramo morto removido.

## 5. Fase 1 — Fundação

Tudo o que o motor da Fase 2 não substitui.

### 4.1. Oráculos RED (probe → teste é cópia mecânica)

Copiar para `tests/` os fontes EXATOS dos probes em
`tests/probe-nested/` (incluindo família K, §6.1). Estados medidos
em `acb6099` (2026-08-30), baselines em
`tests/probe-nested/results-acb6099/`:

- **RED F2** (hoje verdes por cegueira): probeA, probeB, probeM, probeJ,
  probeK_deep_hole.
- **RED F1** (hoje panic/verde): probeE (`ArityMismatch`),
  probeK_arity_tuple (`ArityMismatch`), probeE2 (falso-positivo
  `redundant_clause`).
- **Controles verdes**: probeC, probeD, probeF2, probeG, probeJ2,
  probeK_deep, probeK_grid, `lambda True True:` (10 testes existentes).
- **Pariais de fase**: probeH/H_with — `non_exhaustive_match` até F3;
  probeF — `type.mismatch` até F4.

Oráculos de virada F2+ carregam `#[ignore]` até a fase-alvo.

### 4.2. Bound-check com `ArityMismatch`

`check_patterns` (`infer/helpers.rs:92`): validar
`patterns.len() == param_tys.len()` ANTES do loop; divergência →
`ArityMismatch` (erro em `middleend.rs:72`). Defense-in-depth: o lambda
anônimo não tem assinatura e pode divergir por outros caminhos.

Medição (`b5e2d9e`): o panic NÃO exige pattern desqualificado —
`lambda True True:` contra 1 param **tupla** `(Boolean, Boolean) => Text`
também panica (2 patterns vs 1 param). O bound-check cobre as duas rotas.

### 4.3. Parser aridade-consciente

Aplicar `parse_match_pattern` em todas as posições quebra `lambda True True:`
(`lambda_match_inference.rs:612-761`): engole `True True` como
`Variant{True, [True]}` — Boolean com payload → falso `ArityMismatch`.

Design: `parse_lambda_clause` parseia `arity` patterns — primeiras
`arity-1` com `parse_pattern`, **última** com `parse_match_pattern`.
Lambda anônimo mantém `parse_patterns` (sem assinatura).

Efeito: `lambda Some True:` em função de 1 param parseia como UM pattern
`Variant{Some, [Ident(True)]}` — alinha cláusulas com match arms.

**Extensão: payload entre parênteses em qualquer posição.** O ramo
desqualificado de `parse_pattern_inner` (linhas 128-140) hoje só trata
sub-pattern sem parênteses. Estender para tratar `Ident(` como payload
entre parênteses — exatamente como o ramo qualificado já faz (linhas
83-101: `Result::Ok(v)` funciona em qualquer posição). `(` é
delimitador sintático: não há ambiguidade com a próxima posição porque
abre um escopo delimitado. Não invade semântica — o parser não precisa
saber se `Ident` é unitário ou tem payload; o `(` é inequívoco.

Regra de linguagem: payload desqualificado sem parênteses só na última
posição (genuinamente ambíguo); com parênteses, qualquer posição.

**Limitação:** pattern aninhado desqualificado sem parênteses só
funciona na última posição de cláusulas com assinatura. Demais posições
exigem forma qualificada (`lambda Optional::Some x:`) ou parênteses
(`lambda Some(x):`).

### 4.4. Fall-through de guards no codegen ✅

- **Interp** (`eval.rs:1536-1538`): pattern casa, sem guard → próxima
  cláusula. Correto.
- **Codegen** (`clause.rs`): `lower_guards` recebe
  `fallthrough_block: Option<Block>`. Com `Some` (multi-cláusula),
  guards sem otherwise que não passam emitem `jump(fallthrough_block)`
  — fall-through para a próxima cláusula. Com `None` (fast-path de
  cláusula única), mantém fallback_body. **Corrigido (de88c41).**

Hoje o caminho é morto (`check_guard_completeness` rejeita guards sem
`otherwise`). O fix foi commit isolado — no-op provado pela suite verde.
Quando a Fase 3 abrir o caminho, a metade runtime já está correta.
DoD: suite inteira verde, nenhum oráculo muda de estado. ✅

## 6. Fase 2 — Motor unificado (Maranget)

Motor puro em `kata-inference/src/maranget.rs` com testes table-driven
(sem typeck); depois 3 consumidores migram, um por commit.

- **Matriz de pattern-tuples**: linhas = braços/cláusulas, colunas =
  payloads abertos.
- **Especialização por construtor**: descarta incompatíveis, abre campos
  como novas colunas.
- **Constructor splitting para tipos infinitos**: ausentes agrupados no
  bucket `Missing` — não enumera Int.
- **Usefulness**: exaustividade = `_` não é útil (witness = caso
  faltante); redundância = nenhum witness.
- **Redundância de match arms** (decisão 6): inútil → `RedundantClause`,
  exceto `otherwise` (isento).
- **Interface**: motor recebe `Pattern` + trait de ambiente
  (`constructors_of`, `field_tys`, `is_infinite`) — não alcança `TypeEnv`.
- **Fim das sentinelas `__ANY__`**.

### 5.1. Oráculos adversariais K

Medidos em `acb6099`, probes em `tests/probe-nested/`:

- **probeK_deep / deep_hole** — 3 níveis (`Optional::(Optional::(Boolean))`):
  completo = verde (regressão); com buraco = verde hoje por cegueira,
  F2: `NonExhaustiveMatch` `missing: ["Some (Some False)"]`.
- **probeK_grid / grid_partial** — grade 2×2 (`Boolean Boolean => Text`):
  completa = verde (regressão); parcial (3 de 4) = já RED hoje — motor
  precisa igualar antes de estender.
- **probeK_arity_tuple** — ver §5.2.
- **probeK_deep_paren** — `Some (Some True):` com parênteses internos.
  **Resolvido pela Fase 0** (desembrulhamento + recursão). Era parse error;
  agora compila e roda verde. Oráculo reclassificado de limite sintático
  para controle verde.

DoD: 3 consumidores verdes no mesmo motor; zero `__ANY__` (grep vazio);
witnesses legíveis (snapshots); probeJ → `RedundantClause`; probeA/B →
`NonExhaustiveMatch` `["Some False"]`; probeM → `missing: ["Ok _"]`;
probeK_deep_hole → witness de 3 níveis; controles permanecem verdes.

## 7. Fase 3 — Z3 na folha (guards)

O motor conduz até a folha; quando só resta decidir por guards, emite
query Z3 escopada por célula com bindings semeados (`seed_with_bindings`).
Query: `¬(g₁ ∨ … ∨ gₙ)` UNSAT → folha coberta; Sat → contraexemplo no
witness; `Unknown` → `MissingOtherwise` local à folha. Z3 nunca enxerga
datatype — só variáveis de payload e guards.

Pré-requisito: fall-through de codegen (§5.4). probeH valida E2E nos dois
backends: `Some x` com `> x 0` / `<= x 0` em cláusulas separadas, `None`
na terceira — sem `otherwise`.

`arm.guard` do `match` explícito (AST existe, parser nunca popula) fora
do escopo — sintaxe nova, outro PRD.

DoD: probeH E probeH_with verdes nos dois backends (interp e JIT).

## 8. Fase 4 — Refined na folha

Predicado do refined entra como premissa na mesma query de folha. Sem
motor novo, sem datatype no Z3.

### 7.1. Coerção de literal em pattern sobre refined (probe F)

`match` sobre scrutinee refined com literal falha hoje com `TypeMismatch`
— o literal não é coerzido ao refined em posição de pattern.

Fix pelo caminho da ascription (`5::NonZero`): `check_pattern` aceita
`Literal` quando o tipo esperado é refined sobre a mesma base numérica e
o literal satisfaz o predicado (const-avaliação, `const_eval.rs`); viola
→ `TypeMismatch` com mensagem de ascription (`ascription.rs:308-312`).

**Reuso, não path novo.** A ascription-refined já faz essa verificação
em `ascription.rs:305` chamando `const_eval_predicate(pred, expr)`. A
Fase 4 não cria um novo avaliador — estende `check_pattern_inner` (ramo
`Pattern::Literal`, linhas 168-190) para, antes de rejeitar com
`TypeMismatch`, se o scrutinee é refined sobre a mesma base do literal,
extrair o predicado e chamar a **mesma** `const_eval_predicate`. Se
`Some(true)`, aceitar; `Some(false)`, rejeitar; `None`, cair no
`TypeMismatch` existente (predicado não-avaliável = fora do escopo).
Novo caller para função existente, não novo código de avaliação.

Oráculo: `match n { 1: "um", 2: "dois" }` sobre
`data (Int, > _ 0, < _ 3) as UmOuDois` — cobre o domínio {1, 2}.

### 7.2. Escopo

- Refined sobre **Int/Float** com predicado const-avaliável — paridade
  com a ascription refined de hoje. Rational → Fase 5.
- Outras bases (Text, Boolean, enum) fora — construtor falível é a via
  geral (manual §4.2.2).

DoD: probeF verde E2E nos dois backends; literal fora do domínio →
`TypeMismatch` com literal na mensagem; `otherwise` sobre refined sem
mudança; Fase 3 sem regressão; zero `datatype` Z3 (grep vazio).

## 9. Fase 5 — Rational na folha

Estende a Fase 4 a Rational. Três lacunas:

1. **Literal:** `rational 3` é `Apply` de FFI, não literal do parser.
   Pattern de folha será `rational <IntLit>` (Apply const-avaliável) ou
   ascription `<IntLit>::Rational`.
2. **Const-eval:** estender `const_eval_predicate` para avaliar
   `rational <IntLit>` (conversão Int→Rational é total).
3. **Z3:** mapear Rational como par (num, den) com invariante `den > 0`.
   Comparações via cross-multiplication (`num₁·den₂ ⋛ num₂·den₁`).
   Aritmética completa (soma/produto) fora do escopo.

Sem axiomatização de datatype — Rational entra como par de Ints com
premissa `den > 0`.

Oráculo: `data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois`
com `rational 1:` / `rational 2:`. Controles: wildcard verde;
`rational 0:` → `TypeMismatch`.

DoD: oráculo verde nos dois backends; Fase 4 sem regressão; premissa
`den > 0` em todas as queries; zero `datatype` Z3.

## 10. Estruturas afetadas

| Camada | Site | Fase |
|--------|------|------|
| Parser | `parse_tuple_pattern` — desembrulhar `(p)` sem vírgula | 0 ✅ |
| Parser | `parse_pattern_inner` — recursão de `allow_unqualified_variant` | 0 ✅ |
| Parser | `patterns.rs` — remoção do ramo morto `else if LParen` | 0 ✅ |
| Parser | `parse_sig_clauses`/`parse_lambda_clause` (`sig.rs`) + 4 callers | 1 |
| Typeck | `check_patterns` (`helpers.rs:92`) — bound-check | 1 |
| Codegen | `lower_guards`/`lower_clause_chain` (`clause.rs`) — fall-through | 1 |
| Typeck | `maranget.rs` (novo) — motor + trait de ambiente | 2 |
| Typeck | `check_exhaustiveness` / `check_clause_exhaustiveness` / `pattern_covers` — migram ao motor | 2 |
| Typeck | redundância de match arms (novo consumidor) | 2 |
| Z3 | `guard_completeness.rs` + `z3_translate.rs` — queries de folha | 3 |
| Typeck | `check_pattern` (`patterns.rs:285+`) — literal contra refined | 4 |
| Z3 | predicado do refined como premissa da folha | 4 |
| Typeck | `const_eval.rs` — `rational <lit>` const-avaliável | 5 |
| Z3 | `z3_translate.rs` — Rational como par (num, den) | 5 |

## 11. Fora do escopo (registrar no TODO.md)

- **#2b:** `echo!(None)` → `codegen.unsupported` (`closure.rs:349`) — PRD próprio.
- **`arm.guard` em match arms** — sintaxe nova, exige decisão de Arthur.
- **Interp exit code:** `kata run --interp` em erro de runtime imprime mas
  sai exit 0 — comportamento do driver, não da classe estrutural.
- **#K-call:** variante sem payload como argumento não resolve overload
  (`foo None` → `type.no_overload`). Consistente com ret-directed, mas
  `None` é inexpressável em chamada — só pattern.
- **#K-enum-payload:** enum user-defined como payload de genérico →
  `type.mismatch` dentro do pattern. Ortogonal à exaustividade — PRD próprio.
- **#K-paren:** parêntese interno em braço (`Some (Some True):`).
  **Resolvido pela Fase 0** — desembrulhamento de `(p)` sem vírgula +
  `parse_match_pattern` recursivo. Era limite de implementação (patterns
  sem `Grouping`); agora alinha a gramática de patterns com a de
  expressões. Sem mudança de linguagem.

## 12. Testes (TDD — oráculos RED primeiro)

`nested_exhaustiveness_e2e.rs`, `nested_redundancy_e2e.rs` (probes como
fontes EXATOS — cópia mecânica). Snapshots insta para witnesses.

| Oráculo | Backend | Esperado final |
|---------|---------|----------------|
| probeA (`Some True`+`None`) | JIT | F2: `NonExhaustiveMatch` `["Some False"]` |
| probeB (idem, `Some False`) | JIT+interp | F2: mesmo erro (hoje SIGILL 132) |
| probeC / probeD (completos) | JIT+interp | verde |
| probeE (`lambda Some True:` 1 param) | JIT | F1: `ArityMismatch` (hoje panic 101) |
| probeE2 (cláusulas aninhado) | JIT | F2: verde (hoje falso `redundant_clause`) |
| probeG (guards na cláusula) | JIT+interp | verde (regressão) |
| probeH / probeH_with (guards entre cláusulas) | JIT+interp | F3: verde com output correto |
| probeM (`Result::(Int, Text)` parcial) | JIT | F2: `NonExhaustiveMatch` `["Ok _"]` |
| probeF (refined `{1,2}`) | JIT+interp | F4: verde (hoje `type.mismatch`) |
| probeF2 (wildcard sobre refined) | JIT | controle: verde |
| refined literal fora do domínio | JIT | F4: `TypeMismatch` com literal |
| refined cobertura parcial | JIT | F4: `NonExhaustiveMatch` `["2"]` |
| probeJ (braço morto) | JIT | F2: `RedundantClause` (hoje verde silencioso) |
| probeJ2 (otherwise inútil) | JIT | verde (isenção do otherwise) |
| probeK_deep (3 níveis completo) | JIT | verde (regressão) |
| probeK_deep_hole (3 níveis buraco) | JIT | F2: `NonExhaustiveMatch` `["Some (Some False)"]` |
| probeK_deep_paren | JIT | F0: verde (era parse error) |
| probeK_grid / grid_partial | JIT | completa: verde; parcial: `NonExhaustiveMatch` |
| probeK_arity_tuple | JIT | F1: `ArityMismatch` (hoje panic 101) |
| `lambda True True:` (10 testes) | — | verdes (regressão 2-A) |
| `RatUmOuDois` | JIT+interp | F5: verde com output correto |

Verificação entre fases: `cargo test --workspace --no-fail-fast` verde;
`cargo clippy --workspace --all-targets -- -D warnings` vazio.

## 13. Passos de implementação

**Fase 0** (já implementada):
1. Desembrulhamento de `(p)` sem vírgula em `parse_tuple_pattern`.
2. `parse_match_pattern` recursivo (sub-patterns herdam
   `allow_unqualified_variant`).
3. Remoção do ramo morto `else if LParen` (linhas 83-101).
4. `cargo test` + clippy verdes.

**Fase 1** (implementada — passos 1-4):
1. Oráculos E2E RED copiados de `tests/probe-nested/` (incluindo
   família K); oráculos de virada F2+ entram `#[ignore]`. ✅ b9cfba4
2. Bound-check `ArityMismatch` em `check_patterns`. ✅ 274d7b5
3. Parser aridade-consciente + teste `lambda True True:` regressão. ✅ 4750233
4. Fall-through de codegen (§5.4) como commit isolado — no-op provado. ✅ de88c41
5. `cargo test` + clippy verdes; commits em camadas. ✅

**Fase 2** (implementada — passos 1-6):
1. `maranget.rs` puro (matriz/usefulness/witness, trait de ambiente) +
   testes table-driven — sem typeck. ✅ `f193635`
2. Migrar 3 consumidores, um por commit:
   - Match (`_match.rs`): ✅ `32fab2f`
   - Lambda (`function_infer.rs`): ✅ `503d817` (substitute_ty + collect_all_witnesses)
   - Redundância (`redundancy.rs`): ✅ `6e27d39` (is_arm_redundant como gate estrutural)
3. Redundância de match arms (novo consumidor, isenção do otherwise) — pendente Fase 3 (guards)
4. Remover `__ANY__` (grep vazio). ✅ `a86c27d`
5. Witnesses legíveis → snapshots. ✅ `eb0d0a5` (oráculos F2 des-ignorados)
6. Fix codegen: selar next_clause_block após body (probeG crash). ✅ `31fea5a`

**Fase 3:** queries de folha escopadas → probeH E probeH_with verdes
nos dois backends.

**Fase 4:** coerção de literal em pattern sobre refined → predicado
como premissa da folha → probeF verde nos dois backends.

**Fase 5:** const-eval de `rational <lit>` → Rational no Z3 como par
(num, den) com premissa `den > 0` → oráculo `RatUmOuDois` verde.

## 14. Atualização de documentação ao concluir

- Este PRD — status ✅ por fase.
- `docs/TODO.md` — item \"Patterns aninhados\" reescrito; entradas novas:
  #2b, #K-call, #K-enum-payload, #K-paren, sistema de supressão de
  diagnóstico.
- `docs/Kata-lang-manual.md` §16 (exaustividade aninhada + witnesses);
  §4.2.2 (literal em pattern sobre refined). **Pedir permissão a Arthur.**
- `docs/kata-book/06-*.md`, `10-*.md` — se comportamento visível ao
  iniciante mudar.
- Skill `kata-code-authoring` — tabela de Patterns se a Fase 2/3 mudar o
  que se escreve.