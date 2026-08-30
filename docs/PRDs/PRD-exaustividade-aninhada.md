# PRD — Exaustividade Aninhada (Maranget + Z3 em Guards)

**Status:** 🔴 Pendente — Fase 1 não iniciada
**Data:** 2026-08-30
**Tipo:** Planejamento — PRD único, 3 fases sequenciais
**Depende de:** `PRD-exaustividade.md` ✅ (guards via Z3, patterns de 1 nível)
**Não depende de:** nenhum PRD pendente

## 1. Objetivo

Fechar a classe de bugs estruturais em que os 3 checkers de cobertura
(exaustividade de `match`, exaustividade de cláusulas lambda, redundância
de cláusulas) ignoram o **payload** de patterns compostos (`Variant`,
`Cons`, `Tuple`), e unificar os três num motor único de usefulness
(Maranget), com Z3 compondo nas folhas/guards — nunca substituindo o
motor.

## 2. Motivação — 4 bugs da mesma classe estrutural

Todos reproduzidos empiricamente em `f64eff8` (2026-08-30), binário
`target/debug/kata`, probes em `/tmp/kata5-probe-nested/` (a copiar para
`tests/` como oráculos):

| # | Achado | Evidência |
|---|--------|-----------|
| 1 | Exaustividade ignora payload: `Some True` + `None` aceito sem `Some False` — o caso faltante atravessa o compile-time | probeA compila e roda (exit 0); probeB, o mesmo código chamado com `Some False`, **SIGILL exit 132** — é o `trap user(1)` de `lower_clause_chain` (`kata-codegen/src/lowering/clause.rs:260`) |
| 1b | Redundância ignora payload: cláusulas `lambda Optional::Some True:` seguida de `lambda Optional::Some False:` → a 2ª é rejeitada como `type.redundant_clause` | probeE2, exit 1 |
| 2 | Panic do compilador: `lambda Some True:` (desqualificado) em função de 1 param parseia como 2 patterns → `index out of bounds` em `helpers.rs:104` (`param_tys[i]` sem bound-check). Viola o invariante de compile-time gracioso | probeE, exit 101, panic reproduzido |
| 2b | Codegen rejeita `echo!(None)`: Closure com callee não-Ident (`closure.rs:349`) | probeC v1 — **fora deste PRD**, ver §8 |

Diagnóstico estrutural: existem **3 checkers ortogonais** com lógica de
cobertura parcialmente duplicada (`check_exhaustiveness` —
`kata-inference/src/patterns.rs:442`; `check_clause_exhaustiveness` —
`infer/function_infer.rs:250`; `pattern_covers` —
`kata-inference/src/redundancy.rs:165`), todos operando por nomes de
variantes de 1 nível com sentinela-string `__ANY__`. O runtime já está
correto: interp desce payload (`eval.rs:1837+`), JIT desce payload e
trapaça graciosamente quando nada casa — o SIGILL do probeB é o trap de
defesa disparando porque **o checker deixou passar**, não um bug de
codegen de match.

## 3. Decisões de design (ratificadas por Arthur, 2026-08-30)

1. **Maranget é o objetivo.** O motor da Fase 2 É o algoritmo de
   Maranget (matriz/usefulness, com witnesses como subproduto), não um
   refactor de conveniência. A Fase 1 é a ponte de soundness, não o
   destino final. Rota alternativa (tudo-Z3, axiomatizar datatypes)
   rejeitada: queries gigantes → mais `Unknown` → mais `MissingOtherwise`
   espúrios.
2. **1-B: payload infinito na Fase 1 → `MissingOtherwise` (INTERINO).**
   Ex.: `match r { Ok 0: ... Err _: ... }` sobre `Result::(Int, Text)` →
   `MissingOtherwise`. Na Fase 2, com usefulness, o witness `Ok _` passa a
   ser o contrato final (`NonExhaustiveMatch` com witness) e 1-B é
   substituído.
3. **2-A: parser de cláusulas aridade-consciente** (refinada na
   verificação de 2026-08-30 — ver §3.1.3): a ÚLTIMA posição do pattern
   de cláusula usa `parse_match_pattern`; as demais mantêm
   `parse_pattern`. A aridade vem da assinatura, parseada antes das
   cláusulas nos 3 callers de `parse_sig_clauses`.
4. **3-A: composição motor→Z3 NA FOLHA (ratificada nesta sessão).** A
   composição tem DIREÇÃO: o motor Maranget especializa estruturalmente e
   emite queries Z3 escopadas por célula; o Z3 nunca enxerga estrutura de
   datatype. `Unknown` → `MissingOtherwise` local à folha. 3-B (Z3 prova
   exaustividade sobre scrutinee refined) fica no TODO.md como extensão
   de premissas da mesma folha — PRD futuro.
5. **Panic #2 → diagnóstico `ArityMismatch`** na Fase 1 (bound-check).
6. **Codegen `echo!(None)` (#2b) fora do PRD** — TODO.md com PRD próprio.

## 4. Fase 1 — Soundness (cobertura estrutural recursiva)

### 4.1. Cobertura recursiva nos 3 checkers

Regra única, aplicada aos três consumidores:

- `Variant{v, sub}` com payload de tipo **finito** (enum, Boolean) →
  desce: o sub-pattern participa da cobertura como coluna filha.
- `Variant{v, sub}` com payload **infinito** (Int, Float, Text, Byte,
  …) → exige `Ident`/`Wildcard` no payload; literal/pattern restrito →
  `MissingOtherwise` (decisão 1-B, interino).
- `Cons{h, t}` → `[]` cobre `Nil`; `[h:t]` cobre `Cons`; head com
  literal → elemento infinito → regra do payload infinito acima.
- `Tuple` → cada elemento é uma coluna recursiva.
- `Literal` sobre tipo infinito nunca cobre o universo.

Sites: `check_exhaustiveness` (match, `patterns.rs:442`),
`check_clause_exhaustiveness` (cláusulas, `function_infer.rs:250`),
`pattern_covers` (redundância, `redundancy.rs:165`).

DoD Fase 1:
- probeA → `NonExhaustiveMatch` missing `["Some False"]` em compile-time
  (nunca mais exit 0 com buraco).
- probeB (mesmo fonte) → o mesmo erro de compile; nunca SIGILL.
- probeE → nunca panic: após o bound-check, `ArityMismatch` (gracioso);
  após o fix do parser (mesma fase), parseia aninhado e compila verde.
- probeE2 → compila limpo; sem `type.redundant_clause` falso-positivo.
- probeD, probeG → continuam verdes (regressão).
- `lambda True True:` multi-param (10 testes existentes em
  `lambda_match_inference.rs`) → continuam verdes (regressão do 2-A).

### 4.2. Bound-check com `ArityMismatch`

`check_patterns` (`infer/helpers.rs:92`): validar
`patterns.len() == param_tys.len()` ANTES do loop; divergência →
`ArityMismatch { expected, found, span }` (erro já existente em
`kata-diagnostics/src/middleend.rs:72`). Defense-in-depth: mesmo com o
parser aridade-consciente, o lambda anônimo não tem assinatura e pode
produzir contagens divergentes por outros caminhos.

### 4.3. Parser de cláusulas aridade-consciente (2-A refinado)

O handoff original dizia "cláusulas lambda passam a usar
`parse_match_pattern`". A verificação empírica refine: aplicado
literalmente (todas as posições greedy), quebra `lambda True True:`
(`lambda_match_inference.rs:612-761`): o parser engole `True True` como
`Variant{True, [True]}` — Boolean com payload → falso `ArityMismatch`.

Design correto:

- `parse_sig_clauses` recebe a aridade da assinatura (disponível nos 3
  callers: `sig.rs:48`, `interface_decl.rs:150`, `interface_decl.rs:378`).
- `parse_lambda_clause` parseia exatamente `arity` patterns: as
  primeiras `arity-1` posições com `parse_pattern` (comportamento
  atual), a **última** com `parse_match_pattern`
  (`allow_unqualified_variant=true`).
- A última posição nunca engole demais: o greedy só consome o que vem
  depois dela, e depois da última só vem `:`.
- Lambda anônimo (`lambda.rs:19`) mantém `parse_patterns` (sem
  assinatura, sem aridade) — divergência de aridade já falha graciosa
  via `ArityMismatch` em `apply_lambda.rs:40`.

Efeito: `lambda Some True:` em função de 1 param parseia como UM
pattern `Variant{Some, [Ident(True)]}` (aninhado, como o match já faz) —
mata o bug #2 na raiz e alinha cláusulas com match arms.

**Limitação aceita:** pattern aninhado DESQUALIFICADO funciona apenas na
última posição de cláusulas com assinatura. Nas demais posições (e em
lambda anônimo), usar a forma qualificada (`lambda Optional::Some x:`) —
que já parseia aninhado hoje.

## 5. Fase 2 — Motor unificado (Maranget)

Substitui a cobertura recursiva ad-hoc da Fase 1 (que permanece como
ponte) por UM motor servindo os 3 consumidores:

- **Matriz de pattern-tuples**: linhas = braços/cláusulas, colunas =
  parâmetros/payloads abertos.
- **Especialização por construtor**: escolhe construtor presente na
  coluna, descarta linhas incompatíveis, abre campos como novas colunas.
- **Constructor splitting para tipos infinitos**: construtores ausentes
  agrupados no bucket "resto" (`Missing`) — não enumera Int.
- **Usefulness como noção única**:
  - Exaustividade: match exaustivo sse `_` NÃO é útil; o witness do `_`
    É o caso faltante (`missing: Some False`, `missing: Ok _`).
  - Redundância: braço/cláusula inútil = nenhum witness.
- **Fim das sentinelas-string `__ANY__`** nos 3 checkers.

DoD Fase 2:
- 3 consumidores verdes (match, cláusulas, redundância) sobre o mesmo
  motor; zero chamadas a `__ANY__` (grep vazio).
- Witnesses legíveis nos três contextos (snapshots).
- 1-B substituído: payload infinito → `NonExhaustiveMatch` com witness
  (`missing: Ok _`) em vez de `MissingOtherwise`.
- Bônus mensurável: braço de `match` morto (`Some True` duplicado)
  diagnosticado — hoje passa silencioso.
- probeA/B: `NonExhaustiveMatch` missing `["Some False"]`; probe sobre
  `Result::(Int, Text)` parcial → `missing: ["Ok _"]`.

## 6. Fase 3 — Z3 na folha (composição motor→solver)

### 6.1. Contrato de direção (3-A ratificado)

- O **motor** Maranget conduz: especializa estruturalmente, e quando
  numa folha só resta decidir por guards, emite uma query Z3 **escopada
  por célula** com os bindings da folha semeados (mecanismo
  `seed_with_bindings` existente em `z3_translate.rs`).
- O **Z3 nunca enxerga estrutura de datatype**: nenhum `Optional`/
  `Result` entra na query; o tradutor recebe variáveis de payload e
  guards. `Some` é consumido pelo motor, não pelo solver.
- Query de folha: dados os bindings, `¬(g₁ ∨ … ∨ gₙ)` é UNSAT?
  Unsat → folha coberta; Sat → contraexemplo no witness;
  `Unknown` → `MissingOtherwise` **local à folha** (contrato já
  firmado no PRD-exaustividade).
- Guards só enxergam variáveis ligadas pelo pattern (o pattern já
  consumiu a forma), portanto o escopo de folha não perde poder
  expressivo frente à codificação completa.

### 6.2. Pré-requisito crítico: fall-through de guards no codegen

Descoberto na verificação de 2026-08-30 — sem isto, a Fase 3 cria um
bug de valor errado silencioso:

- **Interp** (`eval.rs:1536-1538`): pattern casa, nenhum guard dispara →
  tenta a próxima cláusula. Correto.
- **Codegen** (`clause.rs:142-155`): pattern casa, nenhum guard dispara,
  sem `otherwise` → lowera `fallback_body` (o body do último guard)
  **incondicionalmente**.

Hoje o caminho é morto (`check_guard_completeness` rejeita cláusulas com
guards não-tautológicos sem otherwise). A Fase 3 o abre: probeH aprovado
no typeck chegaria ao JIT executando o body errado para `x <= 0`.

Fix (na Fase 3, antes de aprovar probeH): `lower_clause_chain` passa um
block de fall-through para `lower_clause_body`/`lower_guards`; guards
esgotados sem `otherwise` pulam para a próxima cláusula (espelha o
interp). O `trap user(1)` final permanece como defesa de runtime.

### 6.3. Escopo

- Guards entre cláusulas com mesmo pattern provam exaustivos (probeH é o
  teste: `Some x` com `> x 0` numa cláusula, `<= x 0` na outra, `None`
  na terceira — sem `otherwise`).
- Per-cláusula permanece: cláusula individual não-tautológica sem
  cobertura complementar em outra cláusula → mesmo erro de hoje.
- `arm.guard` do `match` explícito (AST existe, parser nunca popula,
  `_match.rs:176`) **fora do escopo** — ativar é sintaxe nova, decisão
  de linguagem para outro PRD.

DoD Fase 3: probeH compila E produz `positivo`/`zero ou negativo`
corretos nos DOIS backends (interp e JIT — o teste E2E valida o
fall-through, não só a aprovação do typeck).

## 7. Estruturas afetadas

| Camada | Site | Fase |
|--------|------|------|
| Parser | `parse_sig_clauses`/`parse_lambda_clause` (`kata-parser/src/sig.rs`) — aridade + última posição `parse_match_pattern` | 1 |
| Typeck | `check_patterns` (`infer/helpers.rs:92`) — bound-check | 1 |
| Typeck | `check_exhaustiveness` (`patterns.rs:442`) — cobertura recursiva → motor | 1→2 |
| Typeck | `check_clause_exhaustiveness` (`function_infer.rs:250`) — idem | 1→2 |
| Typeck | `pattern_covers` (`redundancy.rs:165`) — idem | 1→2 |
| Typeck | novo módulo do motor (matriz/usefulness/witness) | 2 |
| Z3 | `guard_completeness.rs` + `z3_translate.rs` — queries de folha escopadas | 3 |
| Codegen | `lower_guards`/`lower_clause_chain` (`lowering/clause.rs`) — fall-through | 3 |

## 8. Fora do escopo (registrar no TODO.md)

- **#2b:** `echo!(None)` → `codegen.unsupported` Closure com callee
  não-Ident (`closure.rs:349`) — bug separado, PRD próprio.
- **probe F:** match sobre scrutinee refined com literal falha
  `TypeMismatch` (literal não coerzido a refined em posição de pattern).
  Anotar independentemente da Fase 3.
- **3-B:** Z3 prova exaustividade de `match` sobre refined via
  predicado como premissa da folha — extensão natural do mecanismo da
  Fase 3, PRD futuro.
- **`arm.guard` em match arms** — sintaxe nova, exige decisão de Arthur.
- **Interp exit code:** `kata run --interp` em erro de runtime da action
  imprime o erro mas sai com exit 0 (observado no probeB) — comportamento
  do driver, não da classe estrutural.

## 9. Testes (TDD — oráculos RED primeiro)

Arquivos por responsabilidade: `nested_exhaustiveness_e2e.rs`,
`nested_redundancy_e2e.rs` (copiar os probes como fontes EXATOS — probe →
teste é cópia mecânica, não reconstrução). Snapshots insta para witnesses.

| Oráculo | Backend | Esperado (final: Fase 2) |
|---------|---------|--------------------------|
| probeA (match `Some True`+`None`) | JIT | `NonExhaustiveMatch` missing `["Some False"]` |
| probeB (idem, chamada `Some False`) | JIT+interp | mesmo erro de compile |
| probeC (completo) | JIT+interp | verde, output correto |
| probeD (match completo 3 braços) | JIT | verde |
| probeE (`lambda Some True:` 1 param) | JIT | nunca panic: `ArityMismatch` pós-bound-check, verde pós-parser-fix |
| probeE2 (cláusulas qualificado aninhado) | JIT | verde, sem falso `redundant_clause` |
| probeG (guards na cláusula) | JIT+interp | verde (regressão) |
| probeH (guards entre cláusulas) | JIT+interp | Fase 1/2: `NonExhaustiveMatch`; Fase 3: verde com output correto |
| match `Result::(Int, Text)` parcial (`Ok 0`/`Err _`) | JIT | F1: `MissingOtherwise`; F2: `NonExhaustiveMatch` missing `["Ok _"]` |
| `lambda True True:` multi-param (10 testes) | — | verdes (regressão 2-A) |
| match braço morto (`Some True` duplicado) | JIT | F2: redundante diagnosticado |

Verificação entre fases: `cargo test --workspace --no-fail-fast` verde;
`cargo clippy --workspace --all-targets -- -D warnings` vazio.

## 10. Passos de implementação

**Fase 1 (ordem interna, menor → maior):**
1. Oráculos E2E RED dos probes (copiar fontes exatos).
2. Bound-check `ArityMismatch` em `check_patterns`.
3. Parser aridade-consciente (2-A) + teste `lambda True True:` regressão.
4. Cobertura recursiva: match (`check_exhaustiveness`).
5. Cobertura recursiva: cláusulas (`check_clause_exhaustiveness`).
6. Cobertura recursiva: redundância (`pattern_covers`).
7. `cargo test` + clippy verdes; commits em camadas (bound-check; parser;
   cobertura match; cobertura clauses; redundância; PRD; TODO.md).

**Fase 2:** motor matriz/usefulness → migrar 3 consumidores (um por
commit) → remover `__ANY__` → witnesses (1-B substituído) → snapshots.

**Fase 3:** fall-through codegen → queries de folha escopadas → probeH
verde nos dois backends.

## 11. Atualização de documentação ao concluir

- Este PRD — status ✅ por fase.
- `docs/TODO.md` — item "Patterns aninhados (Maranget + SMT)"
  reescrito (bug ativo com PRD, não "avaliar"); entradas novas: #2b,
  probe F, 3-B.
- `docs/Kata-lang-manual.md` §16 (Condicionais Puras: Guards e Pattern
  Matching) — contratos de exaustividade aninhada + witnesses. Manual é
  contrato: **pedir permissão a Arthur**.
- `docs/kata-book/06-*.md`, `10-*.md` — se comportamento visível ao
  iniciante mudar (match exaustivo aninhado dispensa otherwise).
- Skill `kata-code-authoring` — tabela de Patterns se a Fase 2/3 mudar o
  que se escreve.