# PRD: Ascription Refined Implícita na Fronteira de Chamada

## Status

🔴 Pendente
**Data:** 2026-09-02
**Depende de:** PRD-refined-collections-polymorphic ✅ (NonEmpty lazy), path conditions + Z3 na ascription ✅
**Não depende de:** nenhum PRD pendente

## 1. Objetivo

Quando uma função espera um tipo refined e recebe um valor do tipo base,
o typeck tenta provar os predicados do refined em compile-time (via
const-eval ou Z3 com path conditions). Se prova, insere a ascription
implicitamente — sem exigir `::Refined` do usuário. Se não prova ou Z3
retorna `Unknown`, rejeita como hoje.

A direção é exclusivamente **base→refined**. A direção refined→base
permanece como está (`refines` opt-in ou downcast explícito).

## 2. Motivação

### 2.1. Verbosidade sem ganho de segurança

`head :: NonEmpty::A => A` exige `head ([1 2 3]::NonEmpty)`. A ascription
`::NonEmpty` valida `>= (len _) 1` em compile-time sobre o literal — a
prova é trivial e o typeck já a faz. A verbosidade da ascription não
adiciona segurança: é fricção onde já há prova.

O caso extremo é encadeamento: `head (tail ((tail xs)::NonEmpty))`.
Cada `::NonEmpty` re-valida um predicado que o contexto já provou ou
que é trivialmente verdadeiro para literais.

### 2.2. A infraestrutura já existe

O mecanismo de prova já está implementado para ascription **explícita**:

- `path_conditions.rs` — acumula facts de guards e match arms, faz
  seeding de `let` bindings no Z3, prova implicação via UNSAT check.
- `ascription.rs` linhas 326-471 — quando o arg é não-literal e há path
  conditions, chama `try_prove_with_path_conditions`. Se Z3 prova,
  aceita; se refuta, erro; se `Unknown`, pending para comptime.
- `const_eval.rs` — const-avalia predicados sobre literais
  (Int/Float/Text/ListLit/ArrayLit/SetLit/DictLit).

O que falta é aplicar o mesmo mecanismo **na fronteira de chamada**
quando `match_score` falha porque o arg é base e o param é refined.

### 2.3. O princípio do atrito sadio

"Fricção onde há risco, não onde há prova." A ascription explícita
existe para forçar o programador a alegar pertencimento ao subconjunto.
Se Z3 prova essa alegação, a fricção é desnecessária. A direção oposta
(refined→base) descarta informação de tipo e permanece explícita por
design — o programador decide descartar a garantia via `refines` ou
downcast.

## 3. Design

### 3.1. Mecanismo

Quando `match_score` falha porque o arg é do tipo base e o param é
refined, o scoring tenta provar os predicados do refined **ali mesmo,
no mesmo fluxo** — não um segundo passe:

1. **É literal?** → const-eval do predicado (caminho existente).
2. **Há path conditions?** → Z3 prova implicação (caminho existente
   em `try_prove_with_path_conditions`).
3. **Z3 prova** → o score vira compatível para aquele overload; o arg
   recebe `TypeAscription { inner: arg, target: refined }` na TAST.
4. **Z3 refuta ou Unknown** → o score permanece incompatível.

O nó de ascription na TAST é idêntico ao que `xs::NonEmpty` produz hoje.
Codegen trata como no-op (refined é transparente em runtime). A única
diferença é que o typeck o insere sem o usuário escrever `::Refined`.

A prova é uma etapa a mais no scoring, não um fallback separado que
retenta o dispatch. O `match_score` ganha um caso: quando o casamento
direto falha e o arg é base de um refined do param, tenta provar.

### 3.2. Gate

A tentativa só dispara quando:

- O arg é exatamente o tipo base do refined do param (via `alias_of`
  no `StructRegistry`). Não tenta para tipos não-relacionados.
- O refined tem predicados const-avaliáveis ou há path conditions
  no escopo. Sem provas possíveis, não dispara Z3 (seria `Unknown`
  garantido).
- O arg não referencia `var` (mutável). O Z3 não tem material sound
  sobre mutáveis — já filtrado em `references_mutable`.

### 3.3. Onde vive no pipeline

A prova vive dentro do scoring — quando `match_score` falha para um
arg base vs param refined, ela tenta provar o predicado. Se prova, o
score vira compatível. O dispatch não ganha um segundo passe.

O `try_refines_fallback` (refined→base) permanece como está — é um
fallback pós-falha que substitui refined por base e retenta. A
direção base→refined não precisa de um fallback análogo porque a
prova acontece no scoring, antes da decisão de rejeitar.

### 3.4. Composição com Maranget

Para refineds sobre coleções como `NonEmpty` (predicado `>= (len _) 1`),
a prova pode ser estrutural ou aritmética:

- **Estrutural**: o arg vem de um braço de match que casou `Cons x rest`.
  O pattern estabelece que a lista é `Cons`, não `Nil`. `Cons` implica
  `len >= 1`. O fato estrutural é extraído pelo visitor de match
  (que já popula `path_conditions`) e traduzido para o Z3 como premissa.

- **Aritmética**: o arg é um literal `[1 2 3]` e `const_eval_predicate`
  computa `len = 3 >= 1`.

A divisão é a mesma do PRD-exaustividade-aninhada: estrutura vira
premissa para o Z3, Z3 prova o predicado na folha. O Z3 não enxerga
estrutura de datatype — o bridge estrutura→aritmética (ex: `Cons`
→ `len >= 1`) acontece na coleta de path conditions, não no solver.

### 3.5. Bridge estrutura→aritmética para path conditions

Hoje, `path_conditions` acumula facts de guards Boolean e match arms.
O visitor de match (`_match.rs`) extrai facts como `> n 0` de
`Boolean::True` (onde o scrutinee é `> n 0`). Mas não extrai fatos
estruturais de patterns sobre ADTs — `Cons x rest` não registra
`len(scrutinee) >= 1` como fact.

O bridge é uma extensão do visitor de match: quando um braço casa um
construtor `Cons`, registrar que o scrutinee satisfaz as condições
estruturais que o construtor implica. Para `NonEmpty`, isso é
`len(scrutinee) >= 1`. Para refineds sobre struct fields
(`data (Pessoa, > _.idade 18) as Adulto`), o pattern `Pessoa(nome, idade)`
registra `idade` como binding, e Z3 prova `> idade 18` se há fact sobre
`idade` no escopo.

O bridge é genérico: para cada refined sobre um tipo base, o visitor
verifica se o construtor casado implica o predicado. Se sim, registra
como fact. A verificação usa a infraestrutura de construtores que o
Maranget já consome (`constructors_of`, `field_tys`).

## 4. Decisões de design

### 4.1. Prova no scoring, não fallback separado

**Escolha:** a tentativa de prova acontece dentro de `match_score`
(quando o casamento direto falha e o arg é base de um refined do
param). Se prova, o score vira compatível e o arg recebe
`TypeAscription` na TAST.

**Alternativa rejeitada:** fallback separado pós-falha
(`try_implicit_refinement`) que retenta o dispatch com o arg
tipado como refined. Motivo: duplica a estrutura de "tentar de novo
com tipos ajustados" — o mesmo padrão do `try_refines_fallback` na
direção oposta. A prova é uma etapa a mais no scoring, não um
segundo passe de dispatch.

### 4.2. Direção base→refined apenas

**Escolha:** o mecanismo só atua na direção base→refined.

**Alternativa rejeitada:** aplicar nas duas direções. Motivo: refined→base
descarta informação de tipo (a garantia do predicado some). Se
automático, refined vira advisory — toda função que aceita `Int`
aceita `PositiveInt` silenciosamente, sem o programador declarar
`refines`. O `refines` é o mecanismo opt-in que resolve isso: o
programador decide que o refined delega. A assimetria é coerente:
"provar pertencimento ao subconjunto elimina fricção (Z3 prova);
descartar a garantia exige decisão explícita (refines ou downcast)."

### 4.3. Gate conservador para Z3

**Escolha:** só tenta Z3 quando há path conditions e o arg não
referencia `var`.

**Alternativa rejeitada:** sempre tentar Z3. Motivo: sem path
conditions, a conjunção é `true` e `true ⟹ ¬predicado` refutaria
qualquer predicado não-tautológico — todo programa seria rejeitado.
O gate `is_empty()` já existe em `try_prove_with_path_conditions` e
retorna `None` (não decide). O custo do gate é zero quando não há
path conditions.

### 4.4. Bridge estrutura→aritmética no visitor de match

**Escolha:** extrair fatos estruturais no visitor de match
(`_match.rs`), registrar como path conditions.

**Alternativa rejeitada:** axiomatizar construtores no Z3 (o solver
enxerga `Cons`/`Nil` diretamente). Motivo: o PRD-exaustividade-aninhada
rejeitou esta abordagem ("Z3 nunca enxerga estrutura de datatype;
queries gigantes → mais Unknown"). A estrutura já está no pattern;
usar Z3 para redescobrir o que o pattern já disse é desperdício.

## 5. Fases

### Fase 1 — Prova no scoring para base→refined

**Escopo:** `crates/kata-inference/src/infer/apply_dispatch.rs`,
`crates/kata-core/src/dispatch.rs` (`match_score`),
`crates/kata-inference/src/infer/ascription.rs` (fatorar lógica de prova)

- Fatorar a lógica de prova de `ascription.rs` (const-eval + Z3 com
  path conditions) numa função reutilizável.
- Em `match_score`, quando o casamento direto falha e o arg é
  exatamente o tipo base de um refined do param, chamar a função
  de prova. Se prova, retornar score compatível.
- Em `apply_dispatch.rs`, quando o score virou compatível via prova,
  inserir `TypeAscription { inner: arg, target: refined }` na TAST
  para o argumento correspondente.

**DoD:** `head [1 2 3]` compila sem ascription explícita. `head []`
continua rejeitado.

### Fase 2 — Bridge estrutura→aritmética no visitor de match

**Escopo:** `crates/kata-inference/src/infer/_match.rs`,
`crates/kata-inference/src/infer/path_conditions.rs`

- Quando um braço de match casa um construtor `Cons` (ou equivalente
  para outras coleções), registrar como path condition o fato
  estrutural correspondente (ex: `len(scrutinee) >= 1`).
- Para refineds sobre struct fields, o pattern já extrai bindings;
  o bridge verifica se o construtor implica o predicado e registra.
- Genérico: percorre `refined_decls` e verifica se o construtor casado
  implica algum predicado de algum refined sobre o tipo do scrutinee.

**DoD:** `match xs [h : t]: head (tail xs)` compila — o pattern
`[h : t]` prova que `xs` é `Cons` (len >= 1), e `tail xs` dentro do
braço recebe ascription implícita de `NonEmpty` se o predicado for
provado. (Nota: `tail xs` retorna `List::A` — o gate tenta provar
`len(tail xs) >= 1`, que pode ou não ser provável dependendo dos
facts disponíveis sobre o tamanho de `xs`.)

### Fase 3 — Testes E2E

**Escopo:** `crates/kata-codegen/tests/`

Oráculos:

| Caso | Esperado |
|---|---|
| `head [1 2 3]` (literal) | compila sem ascription, retorna 1 |
| `head []` (literal vazio) | `type.no_overload` — predicado refutado |
| `match xs [h : t]: head xs` | compila — pattern prova NonEmpty |
| `head xs` onde `xs :: List::A` sem path conditions | `type.no_overload` — sem prova |
| `head (tail [1 2 3])` | depende: `tail` retorna `List`, Z3 tenta provar `len(tail) >= 1` |
| `f :: PositiveInt => Int` chamado com `5` | compila — const-eval prova `> 5 0` |
| `f :: PositiveInt => Int` chamado com `n` onde `> n 0` no escopo | compila — Z3 prova |
| `f :: PositiveInt => Int` chamado com `n` sem facts | `type.no_overload` |
| `f :: PositiveInt => Int` chamado com `(-5)` | `type.no_overload` — predicado refutado |

**DoD:** todos os oráculos passam em ambos backends (JIT e interp).

## 6. Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| core | `dispatch.rs` (`match_score`) | Novo caso: arg base vs param refined → tenta prova |
| inference | `infer/ascription.rs` | Fatorar lógica de prova (const-eval + Z3) em função reutilizável |
| inference | `infer/apply_dispatch.rs` | Inserir `TypeAscription` na TAST quando score virou compatível via prova |
| inference | `infer/_match.rs` | Bridge estrutura→aritmética: extrair facts estruturais de patterns |
| inference | `infer/path_conditions.rs` | (Possivelmente) novo método para adicionar facts estruturais |
| codegen | (nenhuma) | `TypeAscription` já é no-op em runtime |
| interp | (nenhuma) | Idem |

## 7. Fora do escopo

- **Refinement propagation entre operações** — aprender que `tail` de
  NonEmpty com len > 1 ainda é NonEmpty. Arthur rejeitou como design.
  A ascription implícita prova predicados no ponto de chamada, não
  propaga refinamentos através de cadeias de funções.
- **Direção refined→base automática** — permanece `refines` (opt-in)
  ou downcast explícito (`::Int`).
- **Alias→nomeado automático** — alias não tem predicados, não há
  prova a fazer. Alias é nominalmente distinto do base sem downcast
  explícito (Orphan Rule). A ascription implícita só se aplica a
  refineds com predicados.
- **Predicados sobre `var` (mutáveis)** — o Z3 não tem material sound
  sobre mutáveis. Já filtrado pelo gate `references_mutable`.

## 8. Riscos

### 8.1. Custo de compile-time

Toda rejeição `type.no_overload` onde arg é base e param é refined
dispara a tentativa. O gate (só quando há path conditions ou é
literal) limita o custo. Programas sem refined types não são afetados.

### 8.2. Interação com `try_refines_fallback`

`try_refines_fallback` (refined→base) é um fallback pós-falha que
retenta o dispatch. A prova base→refined acontece no scoring, antes
da decisão de rejeitar. As duas direções não competem: se o arg é
refined e o param é base, o casamento direto pode falhar e
`try_refines_fallback` tenta substituir por base. Se o arg é base e
o param é refined, o casamento direto falha e o scoring tenta provar
— `try_refines_fallback` não aplica (não há refined no arg).

### 8.3. Falsos positivos do Z3

Se o Z3 prova um predicado que é falso em runtime (soundness bug no
seeding ou na tradução), o programa compila mas crasha em runtime.
Mitigação: o mecanismo reusa a mesma infraestrutura de path conditions
que já é sound para ascription explícita (testada em
`t_refinement_propagation_e2e.rs`).

## 9. Documentação

Ao concluir:
- `docs/base/Kata-lang-manual.md` §4.2.11 (Atrito Sadio) — atualizar
  para descrever ascription implícita na fronteira de chamada. Pedir
  permissão a Arthur.
- `docs/TODO.md` — registrar item se aplicável.
- Skill `kata-compiler` — atualizar seção de ascription/refined.