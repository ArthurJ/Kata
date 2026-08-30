# PRD — Exaustividade Aninhada (Maranget + Z3 em Guards + Refined)

**Status:** 🔴 Pendente — Fase 1 não iniciada
**Data:** 2026-08-30 · **Emenda 1 (2026-08-30, Arthur):** F1 encolhe para
Fundação — §4.1 (cobertura recursiva ad-hoc) REMOVIDO, mandato absorvido
pela Fase 2; fall-through de codegen antecipado da F3 para a F1;
oráculos adversariais K adicionados. Sem prazo externo, a ponte
interina não se sustenta (ver §3 decisões 1–2, e §4).
**Tipo:** Planejamento — PRD único, 5 fases sequenciais
**Depende de:** `PRD-exaustividade.md` ✅ (guards via Z3, patterns de 1 nível)
**Não depende de:** nenhum PRD pendente

## 1. Objetivo

Fechar a classe de bugs estruturais em que os 3 checkers de cobertura
(exaustividade de `match`, exaustividade de cláusulas lambda, redundância
de cláusulas) ignoram o **payload** de patterns compostos (`Variant`,
`Cons`, `Tuple`), e unificar os três num motor único de usefulness
(Maranget), com Z3 compondo nas folhas — guards e predicados de tipos
refined — nunca substituindo o motor.

## 2. Motivação — 4 bugs da mesma classe estrutural

Todos reproduzidos empiricamente em `f64eff8` (2026-08-30), binário
`target/debug/kata`, probes em `/tmp/kata5-probe-nested/` (a copiar para
`tests/` como oráculos):

| # | Achado | Evidência |
|---|--------|-----------|
| 1 | Exaustividade ignora payload: `Some True` + `None` aceito sem `Some False` — o caso faltante atravessa o compile-time | probeA compila e roda (exit 0); probeB, o mesmo código chamado com `Some False`, **SIGILL exit 132** — é o `trap user(1)` de `lower_clause_chain` (`kata-codegen/src/lowering/clause.rs:260`) |
| 1b | Redundância ignora payload: cláusulas `lambda Optional::Some True:` seguida de `lambda Optional::Some False:` → a 2ª é rejeitada como `type.redundant_clause` | probeE2, exit 1 |
| 2 | Panic do compilador: `lambda Some True:` (desqualificado) em função de 1 param parseia como 2 patterns → `index out of bounds` em `helpers.rs:104` (`param_tys[i]` sem bound-check). Viola o invariante de compile-time gracioso | probeE, exit 101, panic reproduzido |
| 2b | Codegen rejeita `echo!(None)`: Closure com callee não-Ident (`closure.rs:349`) | probeC v1 — **fora deste PRD**, ver §10 |

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
   refactor de conveniência. Rota alternativa (tudo-Z3, axiomatizar
   datatypes) rejeitada: queries gigantes → mais `Unknown` → mais
   `MissingOtherwise` espúrios.
   **(Emenda 1)** A ponte ad-hoc de soundness que aqui ficava a cargo
   da Fase 1 (§4.1 original) foi REMOVIDA: sem prazo externo, escrever
   a recursão ad-hoc uma vez para descartá-la na Fase 2 não se
   sustenta — o mandato "cobertura recursiva" é absorvido pela Fase 2
   como **motor antes dos consumidores**: o motor puro aterrissa
   primeiro (testes table-driven, sem typeck), depois os 3
   consumidores migram, um por commit. A Fase 1 fica reduzida à
   Fundação: oráculos RED, bound-check, parser 2-A e o fix de
   fall-through de codegen (§4.4).
2. **1-B (EMENDADO): o interino `MissingOtherwise` deixa de existir.**
   Como a cobertura recursiva ad-hoc foi removida da Fase 1, o
   `MissingOtherwise` interino para payload infinito nunca embarca:
   `match r { Ok 0: ... Err _: ... }` sobre `Result::(Int, Text)`
   permanece verde (estado atual, aceito por cegueira) até a Fase 2,
   quando vira `NonExhaustiveMatch` com witness `missing: ["Ok _"]`.
   Nenhuma snapshot é escrita duas vezes; nenhum contrato interino
   precisa ser substituído depois.
3. **2-A: parser de cláusulas aridade-consciente** (refinada na
   verificação de 2026-08-30 — ver §4.3): a ÚLTIMA posição do pattern
   de cláusula usa `parse_match_pattern`; as demais mantêm
   `parse_pattern`. A aridade vem da assinatura, parseada antes das
   cláusulas nos 4 callers de `parse_sig_clauses` (`sig.rs:48`,
   `interface_decl.rs:150`, `interface_decl.rs:378`,
   `interface_decl.rs:501` — override de método em bloco `refines`).
4. **3-A: composição motor→Z3 NA FOLHA (ratificada nesta sessão).** A
   composição tem DIREÇÃO: o motor Maranget especializa estruturalmente e
   emite queries Z3 escopadas por célula; o Z3 nunca enxerga estrutura de
   datatype. `Unknown` → `MissingOtherwise` local à folha.
5. **3-B dentro do PRD como Fase 4 (Arthur, 2026-08-30): refined na
   folha.** O predicado de um tipo refined é uma premissa a mais na
   MESMA query de folha — mesmo mecanismo da Fase 3, sem motor novo.
   O que parecia justificar adiamento (literal não coerzido a refined
   em pattern, probe F) é pré-requisito interno da fase, não motivo
   de exclusão.
6. **Panic #2 → diagnóstico `ArityMismatch`** na Fase 1 (bound-check).
7. **Codegen `echo!(None)` (#2b) fora do PRD** — TODO.md com PRD próprio.
8. **Redundância de match arms → ERRO, com isenção de `otherwise`
   (Arthur, revisão de 2026-08-30).** Braço de match inútil
   (usefulness = falso w.r.t. braços anteriores) que NÃO é
   `otherwise` → `RedundantClause` (erro), estendendo o contrato
   existente de cláusulas. `otherwise` inútil (defensivo pós-cobertura)
   → silêncio: idioma sancionado pelo corpus
   (`enum_refined_alias.kata:54-57`, `refined_types.kata:26-29`).
   Supressão de diagnóstico (quando houver) é item separado no TODO —
   sistema de supressão a projetar.
9. **Rational na folha → Fase 5 (Arthur, revisão de 2026-08-30).**
   Refined sobre Rational fica FORA da Fase 4: não há literal Rational
   na linguagem (`rational 3` é Apply de FFI, não literal),
   `const_eval_predicate` suporta apenas Int/Float, e o tradutor Z3
   não conhece Rational (zero suporte em `z3_translate.rs`). A Fase 4
   fica restrita a Int/Float — paridade REAL com a ascription
   const-avaliável de hoje. A Fase 5 orça a extensão completa:
   const-avaliação de `rational <lit>` + mapeamento Rational no Z3
   como par (num, den) com invariantes.

## 4. Fase 1 — Fundação (oráculos + bound-check + parser + fall-through)

**(Emenda 1)** A cobertura recursiva ad-hoc desta fase (§4.1 original)
foi REMOVIDA — o mandato "descer o payload" é absorvido pela Fase 2
(motor antes dos consumidores). O que resta aqui é só o que o motor
NÃO substitui:

### 4.1. Oráculos RED (probe → teste é cópia mecânica)

Copiar para `tests/` os fontes EXATOS dos probes em
`/tmp/kata5-probe-nested/` (incluindo os K, abaixo). Estados medidos
em `b5e2d9e` (2026-08-30) são a linha de base dos oráculos:

- RED esperado na Fase 2 (hoje verdes por cegueira): probeA, probeB,
  probeM, probeJ, probeK_deep_hole.
- RED esperado na Fase 1 (hoje panic/verde): probeE (`ArityMismatch`),
  probeK_arity_tuple (`ArityMismatch` — ver §4.2), probeE2 (hoje
  falso-positivo `redundant_clause`).
- Controles que devem permanecer verdes: probeC, probeD, probeF2,
  probeG, probeJ2, probeK_deep (3 níveis completo), probeK_grid
  (grade 2×2 completa), `lambda True True:` (10 testes existentes).
- Parciais de fase: probeH/H_with — `non_exhaustive_match` até a
  Fase 3 (verde com output correto depois), probeF — `type.mismatch`
  até a Fase 4.

Marcadores: oráculos cuja virada é F2+ carregam `#[ignore]` até a
fase-alvo pousar (removido no slice da fase); nunca ficam sem o
marcador fora da fase-alvo. Probe → teste é cópia mecânica, não
reconstrução.

### 4.2. Bound-check com `ArityMismatch` — gatilho mais largo que o bug #2

`check_patterns` (`infer/helpers.rs:92`): validar
`patterns.len() == param_tys.len()` ANTES do loop; divergência →
`ArityMismatch { expected, found, span }` (erro já existente em
`kata-diagnostics/src/middleend.rs:72`). Defense-in-depth: mesmo com o
parser aridade-consciente, o lambda anônimo não tem assinatura e pode
produzir contagens divergentes por outros caminhos.

**(Emenda 1)** Medição nova (`b5e2d9e`, probeK_arity_tuple): o panic
NÃO exige pattern desqualificado — `lambda True True:` contra
assinatura de 1 param **tupla** `(Boolean, Boolean) => Text` também
panica em `helpers.rs:104` (2 patterns vs 1 param). O bound-check
cobre as duas rotas; o parser 2-A cobre só a desqualificada.

### 4.3. Parser de cláusulas aridade-consciente (2-A refinado)

O handoff original dizia "cláusulas lambda passam a usar
`parse_match_pattern`". A verificação empírica refine: aplicado
literalmente (todas as posições greedy), quebra `lambda True True:`
(`lambda_match_inference.rs:612-761`): o parser engole `True True` como
`Variant{True, [True]}` — Boolean com payload → falso `ArityMismatch`.

Design correto:

- `parse_sig_clauses` recebe a aridade da assinatura (disponível nos 4
  callers: `sig.rs:48`, `interface_decl.rs:150`, `interface_decl.rs:378`,
  `interface_decl.rs:501` — override de método em bloco `refines`).
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

### 4.4. Fall-through de guards no codegen (antecipado da Fase 3 — Emenda 1)

O fix do §6.2 pousa AQUI, na Fase 1, como commit isolado — não na
Fase 3. Racional: hoje o caminho é morto (`check_guard_completeness`
rejeita cláusulas com guards não-tautológicos sem otherwise), logo a
suite inteira verde é prova medida de no-op; qualquer regressão futura
de `check_guard_completeness` re-arma a mina de valor errado
silencioso descrita no §6.2, e o fix custa pouco enquanto os dois
lados são triviais de comparar. Quando a Fase 3 abrir o caminho, a
metade runtime já está provada e uma falha de probeH isola na metade
typeck. O `trap user(1)` final permanece como defesa de runtime.

DoD do item (na Fase 1): commit isolado alterando
`lower_guards`/`lower_clause_chain` (§6.2) + suite inteira verde
(no-op provado) — NENHUM oráculo novo deve mudar de estado por causa
deste fix.

## 5. Fase 2 — Motor unificado (Maranget)

**(Emenda 1)** Ordem interna invertida: **motor antes dos
consumidores**. O motor puro aterrissa primeiro em
`kata-inference/src/maranget.rs`, com testes table-driven próprios,
sem typeck; depois os 3 consumidores migram, um por commit.

Substitui a cobertura recursiva ad-hoc (removida da Fase 1 pela
Emenda 1) por UM motor servindo os 3 consumidores:

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
- **Redundância estendida a match arms (decisão 8):** braço de match
  inútil → `RedundantClause` (erro), exceto `otherwise` — o braço
  `otherwise` (`MatchArm.pattern == None`) inútil é **isento** (idioma
  defensivo sancionado pelo corpus). `Some True` duplicado e
  `Some _` após `Some True` são erros; `otherwise` após `Ok v`+`Err _`
  é silêncio.
- **Interface do motor (Emenda 1):** o motor recebe `Pattern` + um
  trait pequeno de ambiente (`constructors_of`, `field_tys`,
  `is_infinite`) — NÃO alcança `TypeEnv` da inference diretamente.
  Puro, testável table-driven sem arrastar typeck, reutilizável pelo
  futuro PRD de `arm.guard`.
- **Fim das sentinelas-string `__ANY__`** nos 3 checkers.

### 5.1. Oráculos adversariais K (adicionados na Emenda 1)

Medidos em `b5e2d9e` (2026-08-30), probes em `/tmp/kata5-probe-nested/`:

- **probeK_deep / probeK_deep_hole** — 3 níveis
  (`Optional::(Optional::(Boolean))`): completo = verde hoje (cegueira)
  e deve permanecer verde pós-F2; com buraco = **verde com buraco
  hoje** — mesmo bug #1 em 3 níveis, o flagship do witness de matriz
  (`missing: ["Some (Some False)"]`). Forma sintática: greedy com
  qualificado interno (`Some Optional::Some True:`) — a única que o
  parser de braço aceita hoje.
- **probeK_grid / probeK_grid_partial** — grade multi-param 2×2
  (`Boolean Boolean => Text`): completa = verde (regressão);
  parcial (3 de 4 células) = **JÁ RED hoje** (`non_exhaustive_match`)
  — o checker 1-nível multi-param já cobre colunas top-level
  conjuntamente; o motor precisa IGUALAR antes de estender.
- **probeK_arity_tuple** — ver §4.2 (RED F1: `ArityMismatch`).
- **probeK_deep_paren** — parse error esperado; documenta o limite
  sintático (parêntese interno não-greedy). Continua parse error
  pós-F2 — mudar o parser de pattern é decisão de linguagem, fora do
  escopo.

DoD Fase 2:
- 3 consumidores verdes (match, cláusulas, redundância) sobre o mesmo
  motor; zero chamadas a `__ANY__` (grep vazio).
- Witnesses legíveis nos três contextos (snapshots).
- Contrato final direto, sem interino: payload infinito →
  `NonExhaustiveMatch` com witness (`missing: ["Ok _"]`) — o 1-B
  nunca existiu (Emenda 1).
- probeJ (braço de `match` morto, `Some True` duplicado) →
  `RedundantClause` em compile-time (erro; hoje compila verde
  silencioso — verificado). `otherwise` inútil continua silêncio
  (`enum_refined_alias.kata` e `refined_types.kata` seguem verdes).
- probeA/B: `NonExhaustiveMatch` missing `["Some False"]`; probe sobre
  `Result::(Int, Text)` parcial → `missing: ["Ok _"]`.
- probeK_deep_hole → `NonExhaustiveMatch` com witness de 3 níveis;
  probeK_deep e probeK_grid permanecem verdes; probeK_grid_partial
  continua `NonExhaustiveMatch` (agora via motor).

## 6. Fase 3 — Z3 na folha (composição motor→solver, guards)

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

**(Emenda 1)** O fix foi ANTECIPADO para a Fase 1 (§4.4) como commit
isolado com no-op provado. O que resta nesta fase é apenas consumir:
probeH verde E2E valida o fall-through já pousado.

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

## 7. Fase 4 — Refined na folha (3-B)

A Fase 3 fecha cada folha com Z3 sobre os bindings do payload. A Fase 4
estende a MESMA query de folha com uma premissa estrutural: quando o
tipo da coluna é **refined**, o predicado do refined entra como
premissa. Sem motor novo, sem datatype no Z3 — a Fase 4 é a vitrine
completa da direção motor→solver: o splitting estrutural abre o bucket
"resto" do tipo infinito, a folha fecha com o predicado.

### 7.1. Pré-requisito: coerção de literal em posição de pattern (probe F)

`match` sobre scrutinee refined com pattern literal falha hoje com
`TypeMismatch` esperado `UmOuDois`, encontrado `Int` — o literal não é
coerzido ao refined em posição de pattern. Sem esse fix, a folha da
Fase 4 não tipa.

Fix pelo mesmo caminho da ascription de literal (`5::NonZero`):
`check_pattern` aceita `Literal` quando o tipo esperado é refined sobre
a mesma base numérica do literal e o literal **satisfaz o predicado**
(const-avaliação — mecanismo existente da ascription refined, manual
§4.2.2); literal que viola o predicado → erro de compile. Com o
predicado satisfeito, o literal já estreita a folha — o Z3 confirma com
a premissa adicional.

Probe F vira oráculo RED da Fase 4: `match n { 1: "um", 2: "dois" }`
sobre `data (Int, > _ 0, < _ 3) as UmOuDois` — os dois braços cobrem
exatamente o domínio {1, 2} do predicado.

### 7.2. Escopo

- Refined sobre base numérica **Int/Float** com predicado
  const-avaliável — paridade REAL com a ascription refined de hoje:
  `const_eval_predicate` (`infer/const_eval.rs`) reduz apenas
  `IntLit`/`FloatLit`. Rational sai da Fase 4 (decisão 9) → Fase 5.
- Outras bases (Text via construtor falível, Boolean, enum) fora — o
  construtor falível é a via geral e sound (manual §4.2.2).
- Literal que viola o predicado → `TypeMismatch` com a mensagem do
  mecanismo de ascription ("predicado i de X falhou para valor",
  `ascription.rs:308-312`) — reuso do diagnóstico existente, não
  runtime trap.

### 7.3. DoD Fase 4

- probe F compila E produz `um`/`dois` corretamente (verdade E2E, não
  só aprovação de typeck — mesmo critério da Fase 3 para probeH).
- Literal fora do domínio (`0: "zero"` sobre `UmOuDois`) → erro de
  compile com o literal na mensagem.
- `otherwise` sobre refined com cobertura provada: sem mudança de
  contrato — continua legal e exaustivo por definição (tautologia).
- Fase 3 sem regressão (probeH continua verde nos dois backends).
- O predicado entra como premissa da folha — nenhum datatype no Z3
  (critério negativo: grep vazio em `datatype` nos testes do tradutor).

### 7.4. Testes

Oráculos RED: probeF_match_refined.kata (Fase 4) e probeF2 (wildcard
sobre refined — controle negativo, já verde hoje, continua verde).

## 8. Fase 5 — Racional na folha (decisão 9)

A Fase 4 fecha refined Int/Float. A Fase 5 estende a MESMA premissa de
folha a **Rational** — três lacunas medidas no estado atual:

1. **Literal:** não há literal Rational na linguagem — `rational 3`
   é `Apply` de função FFI, não literal do parser. O pattern em
   posição de folha será `rational <IntLit>` (Apply const-avaliável)
   ou ascription `<IntLit>::Rational`.
2. **Const-eval:** `const_eval_predicate` reduz apenas
   `IntLit`/`FloatLit`. Estender para avaliar `rational <IntLit>`
   (conversão Int→Rational é total e const-avaliável por construção)
   e predicados sobre o par (num, den).
3. **Z3:** o tradutor não conhece Rational (zero suporte em
   `z3_translate.rs`). Mapear como par (num, den) com invariante
   `den > 0`, gcd irrelevante para predicados de comparação —
   operações `<`/`>`/`=` sobre pares via cross-multiplication
   (`num₁·den₂ ⋛ num₂·den₁`), sound para den > 0.

Sem axiomatização de datatype — Rational entra na query de folha como
UM PAR de Ints com premissa `den > 0`, na mesma direção motor→solver
(Z3 nunca enxerga o refined, só o par).

### 8.1. Escopo

- Refined sobre Rational (`data (Rational, > _ (rational 0), ...) as Q`)
  com pattern `rational <lit>` em folha.
- Predicados const-avaliáveis sobre o par; comparações via
  cross-multiplication no tradutor Z3.
- Aritmética racional completa (adição/multiplicação de pares) no
  tradutor fora do escopo — só o que os predicados de refined usam
  (`=`, `!=`, `<`, `<=`, `>`, `>=`).

### 8.2. DoD Fase 5

- Oráculo RED verde (§8.3): `RatUmOuDois` com `rational 1:` /
  `rational 2:` compila e produz output correto nos dois backends.
- `rational 0:` fora do domínio → `TypeMismatch` com literal na
  mensagem; wildcard sobre `RatUmOuDois` continua verde (controle).
- Fase 4 sem regressão (probeF continua verde nos dois backends).
- Critério negativo mantido: zero `datatype` Z3 (grep vazio).
- A premissa `den > 0` entra em TODAS as queries de folha sobre
  Rational (soundness da cross-multiplication).

### 8.3. Testes

Oráculo RED (Fase 5): `data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois`
— match com patterns `rational 1:` / `rational 2:` cobre {1, 2} exatos.
Controles: wildcard sobre RatUmOuDois verde; `rational 0:` fora do
domínio → TypeMismatch com literal na mensagem.

## 9. Estruturas afetadas

| Camada | Site | Fase |
|--------|------|------|
| Parser | `parse_sig_clauses`/`parse_lambda_clause` (`kata-parser/src/sig.rs`) + 4 callers (`sig.rs:48`, `interface_decl.rs:150/378/501`) — aridade + última posição `parse_match_pattern` | 1 |
| Typeck | `check_patterns` (`infer/helpers.rs:92`) — bound-check | 1 |
| Testes | oráculos RED copiados de `/tmp/kata5-probe-nested/` (incluindo K) | 1 |
| Codegen | `lower_guards`/`lower_clause_chain` (`lowering/clause.rs`) — fall-through (no-op provado: suite verde) | 1 (antecipado da 3) |
| Typeck | novo módulo do motor (matriz/usefulness/witness) — `kata-inference/src/maranget.rs`, com trait de ambiente (`constructors_of`, `field_tys`, `is_infinite`) | 2 |
| Typeck | `check_exhaustiveness` (`patterns.rs:442`) — migram para o motor | 2 |
| Typeck | `check_clause_exhaustiveness` (`function_infer.rs:250`) — idem | 2 |
| Typeck | `pattern_covers` (`redundancy.rs:165`) — idem | 2 |
| Typeck | redundância de match arms (novo consumidor; `MatchArm.pattern == None` isento) | 2 |
| Z3 | `guard_completeness.rs` + `z3_translate.rs` — queries de folha escopadas | 3 |
| Typeck | `check_pattern` (`patterns.rs:285+`) — literal aceito contra refined (const-avaliação do predicado) | 4 |
| Z3 | predicado do refined como premissa da query de folha | 4 |
| Typeck | `const_eval.rs` — `rational <lit>` const-avaliável | 5 |
| Z3 | `z3_translate.rs` — Rational como par (num, den), cross-multiplication, premissa `den > 0` | 5 |

## 10. Fora do escopo (registrar no TODO.md)

- **#2b:** `echo!(None)` → `codegen.unsupported` Closure com callee
  não-Ident (`closure.rs:349`) — bug separado, PRD próprio.
- **`arm.guard` em match arms** — sintaxe nova, exige decisão de Arthur.
- **Interp exit code:** `kata run --interp` em erro de runtime da action
  imprime o erro mas sai com exit 0 (observado no probeB) — comportamento
  do driver, não da classe estrutural.
- **(Emenda 1, medidos em `b5e2d9e`):**
  - **#K-call:** variante sem payload como ARGUMENTO não resolve
    overload — `foo None` e até `foo Optional::None` (qualificado)
    dão `type.no_overload` mesmo no 2º nível
    (`Optional::(Boolean)`). Consistente com o design ret-directed
    (`Expr::Ident` não consulta hint), mas torna a construção de
    `None` inexpressável em chamada — só pattern. Item separado;
    os probes K contornam chamando só células construíveis.
  - **#K-enum-payload:** enum user-defined como payload de genérico
    — `Optional::(Encoding)` → `type.mismatch` esperado `Encoding`,
    encontrado `Sum(Encoding) or Generic(Encoding)` dentro do pattern;
    `Result::(Int, Encoding)` falha até sem match (`Err(E|Text)`
    com E enum + variante qualificada contra enum-nu). Ortogonal à
    exaustividade — provável elaboração de união/payload, PRD próprio.
  - **#K-paren:** parêntese interno em braço de match não parseia
    (`Some (Some True):` → parse error) — o greedy de braço só aceita
    a forma qualificada interna (`Some Optional::Some True:`).
    Documentado como limite sintático (probeK_deep_paren); mudar o
    parser de pattern é decisão de linguagem.

## 11. Testes (TDD — oráculos RED primeiro)

Arquivos por responsabilidade: `nested_exhaustiveness_e2e.rs`,
`nested_redundancy_e2e.rs` (copiar os probes como fontes EXATOS — probe →
teste é cópia mecânica, não reconstrução). Snapshots insta para witnesses.

| Oráculo | Backend | Esperado (final: Fase 2) |
|---------|---------|--------------------------|
| probeA (match `Some True`+`None`) | JIT | `NonExhaustiveMatch` missing `["Some False"]` — hoje verde por cegueira; RED virado na F2 |
| probeB (idem, chamada `Some False`) | JIT+interp | mesmo erro de compile — hoje SIGILL 132; RED virado na F2 |
| probeC (completo) | JIT+interp | verde, output correto |
| probeD (match completo 3 braços) | JIT | verde |
| probeE (`lambda Some True:` 1 param) | JIT | nunca panic: `ArityMismatch` pós-bound-check, verde pós-parser-fix — RED virado na F1 |
| probeE2 (cláusulas qualificado aninhado) | JIT | verde, sem falso `redundant_clause` — RED virado na F2 (hoje falso-positivo exit 1) |
| probeG (guards na cláusula) | JIT+interp | verde (regressão) |
| probeH (guards entre cláusulas, inline) | JIT+interp | F2: `NonExhaustiveMatch` (como hoje); F3: verde com output correto |
| probeH_with (guards entre cláusulas via `with` — forma canônica) | JIT+interp | idem probeH — falso verde por forma sintática exige as DUAS formas |
| probeM (match `Result::(Int, Text)` parcial, `Ok 0`/`Err _`) | JIT | hoje verde por cegueira (sem interino); F2: `NonExhaustiveMatch` missing `["Ok _"]` |
| probeF (match sobre refined `{1, 2}` com literais) | JIT+interp | F4: verde, output `um`/`dois` nos dois backends (hoje `type.mismatch`) |
| probeF2 (wildcard sobre refined) | JIT | controle: verde hoje, continua verde |
| match sobre refined com literal fora do domínio (`0:` sobre `UmOuDois`) | JIT | F4: `TypeMismatch` com literal na mensagem |
| match sobre refined cobertura parcial (só `1:`, sem `2:`) | JIT | F4: `NonExhaustiveMatch` missing `["2"]` — witness do model, dentro do domínio do predicado |
| probeJ (match braço morto, `Some True` duplicado) | JIT | F2: `RedundantClause` (erro) — hoje verde silencioso |
| probeJ2 (otherwise inútil pós-cobertura, `Ok v`+`Err _`+`otherwise`) | JIT | verde nas DUAS pontas: hoje e pós-F2 (isenção do otherwise) |
| probeK_deep (3 níveis completo) | JIT | verde hoje (cegueira) e pós-F2 (verde provado) |
| probeK_deep_hole (3 níveis com buraco) | JIT | hoje verde com buraco; F2: `NonExhaustiveMatch` missing `["Some (Some False)"]` |
| probeK_deep_paren (parêntese interno) | JIT | parse error hoje e sempre (limite sintático, §10 #K-paren) |
| probeK_grid (grade 2×2 completa) | JIT | verde hoje e pós-F2 (regressão multi-param) |
| probeK_grid_partial (grade 3 de 4 células) | JIT | `NonExhaustiveMatch` hoje e pós-F2 (o motor IGUALA antes de estender) |
| probeK_arity_tuple (2 patterns vs 1 param tupla) | JIT | hoje panic 101; F1: `ArityMismatch` gracioso |
| `lambda True True:` multi-param (10 testes) | — | verdes (regressão 2-A) |
| `RatUmOuDois` (`rational 1:`/`rational 2:` sobre Rational refined) | JIT+interp | F5: verde com output correto nos dois backends |

Verificação entre fases: `cargo test --workspace --no-fail-fast` verde;
`cargo clippy --workspace --all-targets -- -D warnings` vazio.

## 12. Passos de implementação

**Fase 1 — Fundação (Emenda 1, ordem interna, menor → maior):**
1. Oráculos E2E RED copiados de `/tmp/kata5-probe-nested/` (fontes
   exatos, incluindo probeM, probeJ e a família K); oráculos de
   virada F2+ entram `#[ignore]` até a fase-alvo.
2. Bound-check `ArityMismatch` em `check_patterns` (cobre as DUAS
   rotas: pattern desqualificado e tupla multi-pattern vs 1 param).
3. Parser aridade-consciente (2-A) + teste `lambda True True:` regressão.
4. Fall-through de codegen (§4.4/§6.2) como commit isolado — no-op
   provado pela suite inteira verde.
5. `cargo test` + clippy verdes; commits em camadas (oráculos;
   bound-check; parser; fall-through; PRD; TODO.md).

**Fase 2 — motor antes dos consumidores (Emenda 1):**
1. `maranget.rs` puro (matriz/usefulness/witness, trait de ambiente)
   + testes table-driven — SEM typeck.
2. Migrar 3 consumidores, um por commit (match; cláusulas;
   redundância) — cada commit derruba os `#[ignore]` do seu slice.
3. Redundância de match arms (novo consumidor, isenção do otherwise).
4. Remover `__ANY__` (grep vazio).
5. Witnesses legíveis (1-B nunca existiu; probeM/probeK_deep_hole
   viram o contrato final direto) → snapshots.

**Fase 3:** queries de folha escopadas → probeH E probeH_with verdes
nos dois backends (fall-through já pousado na F1; falha de probeH
isola na metade typeck).

**Fase 4:** coerção de literal em pattern sobre refined (7.1) → predicado
como premissa da folha → probeF verde nos dois backends.

**Fase 5:** const-eval de `rational <lit>` → Rational no Z3 como par
(num, den) com premissa `den > 0` → oráculo `RatUmOuDois` verde nos
dois backends.

## 13. Atualização de documentação ao concluir

- Este PRD — status ✅ por fase.
- `docs/TODO.md` — item "Patterns aninhados (Maranget + SMT)"
  reescrito (bug ativo com PRD, não "avaliar"); entradas novas: #2b
  (codegen `echo!(None)`); #K-call (variante sem payload como
  argumento → `no_overload`); #K-enum-payload (enum user-defined como
  payload de genérico → `type.mismatch`); #K-paren (parêntese interno
  em braço — limite sintático); sistema de supressão de diagnóstico
  (braço redundante com isenção de `otherwise` — projeto a fazer).
- `docs/Kata-lang-manual.md` §16 (Condicionais Puras: Guards e Pattern
  Matching) — contratos de exaustividade aninhada + witnesses; §4.2.2
  (Tipos Refinados) — literal em pattern sobre refined. Manual é
  contrato: **pedir permissão a Arthur**.
- `docs/kata-book/06-*.md`, `10-*.md` — se comportamento visível ao
  iniciante mudar (match exaustivo aninhado dispensa otherwise).
- Skill `kata-code-authoring` — tabela de Patterns se a Fase 2/3 mudar o
  que se escreve.