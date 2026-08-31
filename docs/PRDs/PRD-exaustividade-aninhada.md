# PRD — Exaustividade Aninhada (Maranget + Z3 em Guards + Refined)

**Status:** Fase 0 ✅ — Fase 1 ✅ — Fase 2 ✅ — Fase 3 ✅ — Fase 4 ✅ (refatoração na Fase 5) — Fase 5 🟡
**Data:** 2026-08-31 (atualizado 2026-08-31)
**Tipo:** Planejamento — PRD único, 5 fases sequenciais
**Depende de:** `PRD-exaustividade.md` ✅ (guards via Z3, patterns de 1 nível)

## 1. Objetivo

Fechar a classe de bugs estruturais em que os 3 checkers de cobertura
ignoram o **payload** de patterns compostos, e unificar os três num motor
único de usefulness (Maranget), com Z3 compondo nas folhas — guards e
predicados de tipos refined — nunca substituindo o motor.

A Fase 5 generaliza o motor para **todos os tipos** — não só numéricos
built-in — derivando comportamento de capacidade (implementa ORD? EQ?
NUM?) e transparência (dá pra const-eval/inline?) das registries que o
compilador já constrói dinamicamente. Tipos definidos pelo usuário
funcionam automaticamente, sem registry estático.

## 2. Motivação — 4 bugs da mesma classe estrutural

Reproduzidos em `f64eff8` (2026-08-30). Diagnóstico: 3 checkers
ortogonais com cobertura duplicada, todos operando por nomes de
variantes de 1 nível com sentinela-string `__ANY__`.

| # | Achado | Evidência |
|---|--------|-----------|
| 1 | Exaustividade ignora payload: `Some True` + `None` aceito sem `Some False` | probeA compila; probeB → SIGILL exit 132 |
| 1b | Redundância ignora payload: `Some True:`/`Some False:` → 2ª rejeitada como `redundant_clause` | probeE2 |
| 2 | Panic: `lambda Some True:` parseia como 2 patterns → `index out of bounds` | probeE, exit 101 |
| 2b | Codegen rejeita `echo!(None)` | fora deste PRD, §11 |

## 3. Decisões de design

1. **Maranget é o objetivo.** Tudo-Z3 rejeitado: mais `Unknown` → mais
   `MissingOtherwise` espúrios.
2. **Sem contrato interino.** Payload infinito → `NonExhaustiveMatch`
   com witness.
3. **Parser aridade-consciente.** Última posição usa
   `parse_match_pattern`; demais `parse_pattern`. 4 callers de
   `parse_sig_clauses`.
4. **Composição motor→Z3 na folha.** Z3 nunca enxerga datatype.
   `Unknown` → `MissingOtherwise` local à folha.
5. **Capacidade vs transparência (Fase 5).** Capacidade = o que a
   linguagem permite (ORD? EQ? NUM? — via `TypeGraph`, dinâmico).
   Transparência = o que o compilador consegue provar (const-eval?
   inline? — via `InlineFnTable`, dinâmico). Capacidade sem
   transparência → fallback estrutural conservador, nunca erro falso.
6. **Sem registry estático de tipos.** O motor deriva comportamento de
   `TypeGraph`, `StructRegistry`, `InterfaceRegistry`, `InlineFnTable`
   — todas populadas pelas declarações do usuário. Tipos de usuário
   funcionam automaticamente.
7. **Redundância de match arms → ERRO**, com isenção de `otherwise`.
8. **Rational → Fase 5.** Fase 4 fica restrita a Int/Float; Fase 5
   generaliza para todos os tipos via `TypeCaps`.
9. **Panic #2 → `ArityMismatch`** (bound-check na Fase 1).
10. **Codegen `echo!(None)` (#2b) fora do PRD.**

## 4. Fase 0 — Parser recursivo ✅

`parse_match_pattern` recursivo em sub-patterns; desembrulhamento de
`(p)` sem vírgula; remoção do ramo morto `else if LParen`.

Commits: Fase 0 integrada nos commits da Fase 1.

## 5. Fase 1 — Fundação ✅

Oráculos E2E RED copiados de `tests/probe-nested/` (família K);
bound-check `ArityMismatch` em `check_patterns`; parser aridade-consciente
+ payload entre parênteses em qualquer posição; fall-through de codegen
(`lower_guards` recebe `fallthrough_block`).

Commits: `b9cfba4`, `274d7b5`, `4750233`, `de88c41`, `f8441a7`.

## 6. Fase 2 — Motor unificado (Maranget) ✅

Motor puro em `maranget.rs` (~1110 linhas). Trait `PatternEnv`
(`constructors_of`, `field_tys`, `is_infinite`). `Constructor` enum:
`Variant`, `Cons`, `Nil`, `Literal`, `Tuple`, `Missing`.
`is_useful` (para na primeira witness — redundância) vs
`collect_all_witnesses` (coleta todas — exaustividade). 3 consumidores
migrados (match, lambda, redundância). Sentinelas `__ANY__` removidas.

Commits: `f193635`, `32fab2f`, `503d717`, `6e27d39`, `a86c27d`,
`eb0d0a5`, `31fea5a`.

## 7. Fase 3 — Z3 na folha (guards) ✅

`collect_guard_leaves` + `check_guard_coverage` +
`check_exhaustiveness_with_guards`. Guards entre cláusulas com mesmo
pattern verificados por disjunção Z3. Tradutor por braço (semearado com
`with_bindings`). probeH/probeH_with verdes nos dois backends.

Commits: `92c7dbe`.

## 8. Fase 4 — Refined na folha ✅ (refatoração na Fase 5)

**Implementado:** coerção de literal em pattern sobre refined via
`const_eval_predicate` (reuso); enumeração de domínio finito de refined
sobre Int no motor Maranget (`enum_refined_domain`, `extract_bound`,
`MarangetEnv::with_refined`). `refined_decls` threaded por 7 callers.
probeF (3 testes) verdes.

**Limitação a refatorar na Fase 5:** 6 sites hard-codam reconhecimento
de tipo por nome:

| Site | Hard-code | Problema |
|------|-----------|----------|
| `literal_expr_ty` (patterns.rs:480) | Match em `IntLit`/`FloatLit` | `Apply` (rational 1) cai no fallback |
| Ramo refined (patterns.rs:200-206) | `matches!` de 4 pares (nome, nome) | Só 1 nível de alias; `Peso→PositiveFloat→Float` falha |
| `eval_numeric` (const_eval.rs:93) | `Option<f64>` — Int/Float só | Não representa Rational/Complex |
| `extract_bound` (maranget.rs:146) | `(String, i64)` — IntLit só | `rational 0` é `Apply`, não reconhece |
| `enum_refined_domain` (maranget.rs:217) | `alias_of != "Int"` | Só Int; usa `alias_of` string em vez de `base_ty: Ty` |
| `z3_translate.rs` | `VarKind { Int, Bool }` | Sem Rat/Real; Float cai em `fresh_bool` |

Commits: `cbff78c`.

## 9. Fase 5 — TypeCaps: generalização para todos os tipos

### 9.1. Princípio

O motor deixa de match em nomes de tipo ("Int", "rational") e passa a
consultar **capacidade** (implementa ORD? EQ? NUM? — via `TypeGraph`,
já dinâmico) e **transparência** (dá pra const-eval/inline? — via
`InlineFnTable`, já dinâmico) das registries que o compilador já
constrói a partir das declarações do usuário.

### 9.2. Núcleo: `TypeCaps`

```rust
// kata-core/src/caps.rs (novo módulo, sem ciclo)
pub struct TypeCaps {
    base: Option<Ty>,       // follow_alias até o tipo base
    repr: Repr,             // representação para const-eval/Z3
    ord: bool,              // type_implements(name, "ORD")
    eq: bool,               // type_implements(name, "EQ")
    num: bool,              // type_implements(name, "NUM")
    inlineable_ord: bool,   // método ORD tem corpo Kata puro
    inlineable_eq: bool,    // método EQ tem corpo Kata puro
}
```

`Repr` é automático para todo tipo — é representação de dados, não
capacidade. `ord`/`eq` são ortogonais: dizem quais operações são
válidas sobre a representação. Sem `ord`, `extract_bound` não tenta
`> _ 0`. Sem `eq`, `Domain::Points` não se aplica.

```rust
pub enum Repr {
    Int, Float, Rat, Text, Bool, Unit,
    Struct(Vec<Repr>),  // Ty::Struct com fields — recursivo em StructRegistry
    Sum,                // enums — motor estrutural já cobre
    Opaque,             // sem fields, sem representação conhecida
}
```

Derivação 100% dinâmica:
- `repr`: `Ty::Prim` espelha a representação ABI. `Ty::Struct` consulta
  `StructRegistry::fields` recursivamente — `data MyRat (num::Int den::Int)`
  → `Repr::Struct([Int, Int])` automaticamente, independente de implements.
- `ord`/`eq`/`num`: `TypeGraph::type_implements` com herança de
  supertraits (ORD implementa EQ). Memoizados em `TypeCaps` para evitar
  consultas repetidas em `constructors_of`/`is_infinite` (chamados por
  coluna da matriz). Consulta direta em `TypeGraph` também é válida —
  a memoização é otimização, não necessidade semântica.
- `inlineable_*`: `InlineFnTable` já extrai corpos de funções puras.

`CapsIndex = HashMap<Ty, TypeCaps>` construído a partir de `InferCtx`
no início de `infer_module` (InferCtx já tem `type_graph`,
`interface_registry`, `struct_registry`, `refined_decls` — todas as
fontes). `MarangetEnv` recebe `&CapsIndex` (struct de dados leve, não
contexto de inferência) — preserva a separação entre motor puro e
typeck. Motor permanece testável sem InferCtx (testes table-driven
constroem `CapsIndex` leve).

### 9.3. `ConstVal` substitui `f64` e `i64`

```rust
pub enum ConstVal {
    Int(i64), Float(f64), Rat(i64, i64),  // (num, den) com den > 0
    Bool(bool), Text(String), Unit,
    Struct(Vec<ConstVal>),
}
```

`ConstVal::cmp` exige `ord` (ou `eq` para `=`/`!=`). `Rat` compara por
cross-multiplication exato em `i128`, nunca `f64`.

### 9.4. `Domain` generaliza `enum_refined_domain`

Um único processo — não dois modos. `Domain` é sempre `Vec<ConstVal>`:
o conjunto de valores que satisfazem os predicados. O método de
construí-lo varia conforme a representação:

- Se `Repr` é discreta (Int-like) e há bounds (`> _ N`, `< _ M`),
  enumera o intervalo `lo..=hi` e filtra por `const_eval_predicate`.
- Se há predicados `= _ c` com `c` const-avaliável, coleta os pontos.
- Se ambos, intersecta (enumera intervalo, filtra por `=`).

O gate é `eq` (para `= _ c`) e `ord` (para bounds de intervalo). Sem
nenhum dos dois, `Domain` é `None` (infinito/não-enumerável).

Rational refined com `= _ (rational 1)` → `[Rat(1, 1)]`.
Complex refined com `= _ (complex 1 2)` → `[Complex(1.0, 2.0)]`.
`data MyRat (num::Int den::Int)` com `= _ (MyRat 1 2)` →
`[Struct([Int(1), Int(2)])])`.

### 9.5. Como os 6 sites mudam

| Site | Hoje | Com TypeCaps |
|------|------|-------------|
| `literal_expr_ty` + ramo refined | `matches!` de 4 pares | `caps.base` via `follow_alias` + `caps.num` |
| `eval_numeric` | `Option<f64>` | `Option<ConstVal>` parametrizado por `Repr` |
| `extract_bound` | `(String, i64)` | `(BoundOp, ConstVal)` gated por `caps.ord` |
| `enum_refined_domain` | `alias_of != "Int"` | `Domain: Vec<ConstVal>` — enumera intervalo (Repr discreta + ord) ou coleta pontos (eq + const-eval) |
| `z3_translate` | `VarKind { Int, Bool }` | `VarKind { Int, Bool, Rat(Int, Int) }` — cross-multiplication; EQ inline |

### 9.6. Z3 — Rational como par (num, den)

`VarKind::Rat(Int, Int)` com side-condition `den > 0`. Comparações via
cross-multiplication: `num₁·den₂ ⋛ num₂·den₁`. `Unknown` →
`MissingOtherwise` local (conservador). Float fica opaco nesta fase.
Tipos EQ-only com corpo inlinável (Complex) — `try_inline` formalizado
como estratégia por capacidade. Invariante preservado: zero `datatype`
Z3.

### 9.7. Pontos a investigar na implementação

**`TypedExprKind` para literais FFI (Ponto 4):** `literal_to_typed_kind`
hoje produz `TypedExprKind::Unit` para `Apply` (perde tipo e valor).
`literal_to_string` (maranget.rs:433) já tem fallback
`format!("Other:{:?}", expr.ty)` que preserva o tipo — mas só funciona
se o `TypedExprKind` produzido carregar `expr.ty`. Opções: (a)
`Closure` (preserva `Apply`, `literal_to_string` usa fallback
`"Other:{ty}"`), (b) `IntLit` (desembrulha `rational 1` → `IntLit("1")`,
perde tipo mas round-trip com `"Int:1"` funciona), (c) variante nova
`Const { val: ConstVal }` (genérica, round-trip com `serialize`).
**Descoberta:** o oráculo F5 espera `type.unbound_name` hoje —
`rational` é parseado como `Ident` e não resolve como pattern.
Verificar empiricamente como o parser produz `rational 1` em pattern
position antes de decidir.

**`ConstVal::Struct` no Z3 (Ponto 5):** Descoberta empírica:
- **Rational ORD é @ffi** (`kata_rt_rat_lt`, etc.) — `try_inline` NÃO
  funciona. `VarKind::Rat(Int, Int)` é **necessário**.
- **Complex EQ tem corpo puro** (`lambda a b: and (= a.re b.re) (= a.im
  b.im)`) — `try_inline` funcionaria, **mas** `guard_completeness.rs`
  usa `Z3Translator::new()` (sem `inline_fns`). Só
  `path_conditions.rs` passa `with_inline_fns`. Para Complex EQ
  funcionar em guards, precisa passar `inline_fns` para o tradutor de
  guards.
- **User type com corpo ORD puro:** `try_inline` funciona (mesmo caso
  que Complex) — também precisa de `inline_fns` no tradutor de guards.
- **User type com @ffi ORD:** fallback opaco (`fresh_bool` → Maranget
  estrutural). Degrada com honestidade.
- **Ação:** F5.4 precisa (a) adicionar `VarKind::Rat(Int, Int)` para
  Rational, e (b) passar `inline_fns` para o tradutor de guards em
  `guard_completeness.rs` (estende `GuardArm` ou
  `check_guard_coverage` com `Option<&InlineFnTable>`).

### 9.8. Tipos de usuário — funcionam automaticamente

| Tipo | Capacidade | Transparência | Comportamento |
|------|-----------|--------------|---------------|
| `data MyRat (num::Int den::Int) implements ORD` (corpo puro) | ord, eq | inlineable | Z3 traduz cross-multiplication; refined com `=` e `>` funcionam |
| `data MyRat () implements ORD` via `@ffi` | ord, eq | opaco | Maranget estrutural, `otherwise` exigido — degrada com honestidade |
| `complex 1 2` (NUM+EQ, sem ORD) | eq | inlineable | `Domain::Points` via `=`; `<` não existe na linguagem |
| `alias Peso as PositiveFloat` | num | — | `follow_alias` resolve cadeia; literal 80.5 aceito por `caps.num` |

### 9.9. Escopo

- **Dentro:** Rational refined (oráculo `RatUmOuDois`); refatoração dos
  6 sites para `TypeCaps`; `Domain::Points` para tipos com EQ;
  `VarKind::Rat` no Z3; tipos de usuário com fields + implements.
- **Fora:** Aritmética Rational completa no Z3 (soma/produto);
  Float como Z3 Real (IEEE 754 vs Real é decisão futura); `arm.guard`
  explícito em match arms (sintaxe nova, outro PRD).

### 9.10. Passos de implementação

**F5.1 — `TypeCaps`/`CapsIndex` com paridade total (refatoração pura)**
- Criar `kata-core/src/caps.rs` com `TypeCaps`, `Repr`, `CapsIndex`.
- Derivar `TypeCaps` de `TypeGraph`, `StructRegistry`, `InterfaceRegistry`,
  `InlineFnTable` — todas já existentes.
- Memoizar `CapsIndex` no início de `infer_module`.
- Migrar os 6 sites para consultar `TypeCaps` em vez de hard-codar.
- **Paridade exata:** todos os tipos existentes mapeiam idêntico.
  1986 testes permanecem verdes. Nenhum tipo novo ainda.
- Verificar: `cargo test --workspace --no-fail-fast` verde.

**F5.2 — `ConstVal` + `Domain` unificado + cache**
- Substituir `eval_numeric → Option<f64>` por `eval_const → Option<ConstVal>`.
- Substituir `extract_bound → (String, i64)` por `(BoundOp, ConstVal)`.
- `enum_refined_domain` unificado: `Vec<ConstVal>` — enumera intervalo
  (Repr discreta + ord) ou coleta pontos (eq + const-eval) ou ambos
  (intersecta). Gate por `caps.ord` e `caps.eq`.
- Cache do domínio por tipo (hoje `enum_refined_domain` roda 2x sem
  memo em `constructors_of` e `is_infinite`).
- `serialize(repr, val)` para `Constructor::Literal` — round-trip
  consistente com `literal_to_string`/`pattern_ctor`. Formato por `Repr`:
  `Int` → `"Int:1"`, `Rat` → `"Rat:1/1"`, `Struct` → `"Struct:1|2"`.
- Verificar: `cargo test` verde; oráculo `RatUmOuDois` ainda `#[ignore]`.

**F5.3 — `literal_expr_ty`/ramo refined via `follow_alias` + `caps.num`**
- Substituir `matches!` de 4 pares por `caps.base` + `caps.num`.
- `follow_alias` resolve cadeias de alias (Peso→PositiveFloat→Float).
- Verificar: `cargo test` verde.

**F5.4 — Z3 `Rat(Int, Int)` + EQ-capability inline + `inline_fns` em guards**
- `VarKind::Rat(Int, Int)` com side-condition `den > 0`.
- Comparações Rat por cross-multiplication.
- Float opaco (mantém `fresh_bool`).
- **Passar `inline_fns` para o tradutor de guards.** Hoje
  `guard_completeness.rs` usa `Z3Translator::new()` (sem `inline_fns`).
  Estender `GuardArm` ou `check_guard_coverage` com
  `Option<&InlineFnTable>`. Sem isso, Complex EQ e user types com corpo
  puro não funcionam em guards — `try_inline` não tem a tabela.
- EQ-capability: antes de `fresh_bool` para `=` sobre tipo EQ, tentar
  inline do método via `InlineFnTable` (agora disponível em guards).
- Verificar: Complex implements EQ (corpo puro em `stdlib/complex.kata`)
  funciona em guards após passar `inline_fns`.
- Verificar: `cargo test` verde; zero `datatype` Z3 (grep vazio).

**F5.5 — Oráculos F5 + testes e2e com user types**
- Derrubar `#[ignore]` dos 2 oráculos F5 (`rat_um_ou_dois_f5`,
  `rat_um_ou_dois_zero_f5`).
- Adicionar teste e2e com user type: `data MyRat (num::Int den::Int)
  implements ORD` com refined e match.
- Verificar nos dois backends (JIT e interp).
- `cargo fmt`, `cargo test`, `cargo clippy --workspace --all-targets -- -D warnings`.

## 10. Estruturas afetadas

| Camada | Site | Fase |
|--------|------|------|
| Parser | `parse_match_pattern` recursivo, desembrulhar `(p)`, ramo morto | 0 ✅ |
| Parser | `parse_sig_clauses`/`parse_lambda_clause` + 4 callers | 1 ✅ |
| Typeck | `check_patterns` — bound-check `ArityMismatch` | 1 ✅ |
| Codegen | `lower_guards`/`lower_clause_chain` — fall-through | 1 ✅ |
| Typeck | `maranget.rs` — motor + trait `PatternEnv` | 2 ✅ |
| Typeck | 3 consumidores migrados ao motor | 2 ✅ |
| Z3 | `guard_completeness.rs` + `z3_translate.rs` — queries de folha | 3 ✅ |
| Typeck | `check_pattern` — literal contra refined | 4 ✅ |
| Z3 | predicado do refined como premissa da folha | 4 ✅ |
| Typeck | `caps.rs` (novo) — `TypeCaps`, `Repr`, `CapsIndex` | 5 |
| Typeck | `const_eval.rs` — `ConstVal` substitui `f64` | 5 |
| Typeck | `maranget.rs` — `Domain::Interval`/`Points`, `extract_bound` generalizado | 5 |
| Typeck | `patterns.rs` — ramo refined via `follow_alias` + `caps.num` | 5 |
| Z3 | `z3_translate.rs` — `VarKind::Rat`, cross-multiplication | 5 |

## 11. Fora do escopo (registrar no TODO.md)

- **#2b:** `echo!(None)` → `codegen.unsupported` — PRD próprio.
- **`arm.guard` em match arms** — sintaxe nova, exige decisão de Arthur.
- **Interp exit code:** `kata run --interp` em erro de runtime sai exit 0.
- **#K-call:** variante sem payload como argumento não resolve overload.
- **#K-enum-payload:** enum user-defined como payload de genérico →
  `type.mismatch`. PRD próprio.
- **Aritmética Rational completa no Z3** (soma/produto) — decisão futura.
- **Float como Z3 Real** (IEEE 754 vs Real) — decisão futura.

## 12. Testes

`nested_exhaustiveness_e2e.rs`, `nested_redundancy_e2e.rs`.

| Oráculo | Backend | Esperado final |
|---------|---------|----------------|
| probeA (`Some True`+`None`) | JIT | ✅ `NonExhaustiveMatch` `[\"Some False\"]` |
| probeB (idem, `Some False`) | JIT+interp | ✅ mesmo erro |
| probeC / probeD (completos) | JIT+interp | ✅ verde |
| probeE (`lambda Some True:` 1 param) | JIT | ✅ `ArityMismatch` |
| probeE2 (cláusulas aninhado) | JIT | ✅ verde |
| probeG (guards na cláusula) | JIT+interp | ✅ verde |
| probeH / probeH_with (guards entre cláusulas) | JIT+interp | ✅ verde |
| probeM (`Result::(Int, Text)` parcial) | JIT | ✅ `NonExhaustiveMatch` `[\"Ok _\"]` |
| probeF (refined `{1,2}`) | JIT+interp | ✅ verde |
| probeF2 (wildcard sobre refined) | JIT | ✅ verde |
| refined literal fora do domínio | JIT | ✅ `TypeMismatch` |
| refined cobertura parcial | JIT | ✅ `NonExhaustiveMatch` |
| probeJ (braço morto) | JIT | ✅ `RedundantClause` |
| probeJ2 (otherwise inútil) | JIT | ✅ verde (isenção) |
| probeK_deep (3 níveis completo) | JIT | ✅ verde |
| probeK_deep_hole (3 níveis buraco) | JIT | ✅ `NonExhaustiveMatch` |
| probeK_deep_paren | JIT | ✅ verde |
| probeK_grid / grid_partial | JIT | ✅ completa: verde; parcial: `NonExhaustiveMatch` |
| probeK_arity_tuple | JIT | ✅ `ArityMismatch` |
| `lambda True True:` (10 testes) | — | ✅ verdes |
| `RatUmOuDois` | JIT+interp | F5: verde com output correto |
| `RatUmOuDois` zero fora domínio | JIT | F5: `TypeMismatch` |
| User type `MyRat` refined | JIT+interp | F5: verde (teste novo) |

Verificação entre fases: `cargo test --workspace --no-fail-fast` verde;
`cargo clippy --workspace --all-targets -- -D warnings` vazio.

## 13. Atualização de documentação ao concluir

- Este PRD — status ✅ por fase.
- `docs/TODO.md` — items atualizados.
- `docs/Kata-lang-manual.md` §16 (exaustividade aninhada + witnesses);
  §4.2.2 (literal em pattern sobre refined). **Pedir permissão a Arthur.**
- `docs/kata-book/06-*.md`, `10-*.md` — se comportamento visível mudar.
- Skill `kata-compiler` — atualizar `references/maranget-exhaustiveness.md`
  com seção Fase 5.
- Skill `kata-code-authoring` — tabela de Patterns se necessário.