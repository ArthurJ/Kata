# PRD — Exaustividade de Cláusulas e Guards

**Status:** ✅ Concluído
**Data:** 2026-08-23
**Implementado em:** `0697ad8` (Frente 1), `da22230` (Frente 2)
**Depende de:** `check_exhaustiveness` (patterns.rs) ✅, `check_redundant_clauses` (redundancy.rs) ✅, `check_clause_exhaustiveness` (function_infer.rs) ✅
**Não depende de:** Nenhum PRD pendente

## 1. Objetivo

Generalizar a verificação de exaustividade de cláusulas lambda para
qualquer aridade (N parâmetros) e garantir completude de guards dentro de
cada cláusula. Hoje:

- **Exaustividade de patterns (1 parâmetro):** funciona. `check_clause_exhaustiveness`
  coleta variantes cobertas e chama `check_exhaustiveness`.
- **Exaustividade de patterns (N parâmetros):** só verifica se todos os
  patterns são `Ident`/`Wildcard`. Se há pattern estrutural em alguma
  posição, não verifica — débito técnico.
- **Completude de guards:** não verificada. Uma cláusula com guards mas
  sem `otherwise` pode não produzir valor em runtime.

## 2. Motivação

### 2.1. Cláusulas não-exaustivas crasham em runtime

Antes do commit `50f386c` (2026-08-22), cláusulas lambda não-exaustivas
passavam no typeck e crashavam em runtime com SIGILL (trap de pattern
falho). O commit resolveu o caso de 1 parâmetro. O caso de N parâmetros
com patterns estruturais ainda passa sem verificação:

```kata
and :: Boolean Boolean => Boolean
    lambda True True: True
    lambda True False: False
    # Falta False×True e False×False — não detectado
```

### 2.2. Guards sem fallback produzem "nada" em runtime

```kata
foo :: Int => Int
    lambda x:          # pattern Ident cobre todo Int
        x > 0: x        # guard
        # sem otherwise — se x <= 0, nenhum guard dispara
```

O pattern `x` (Ident) cobre todo `Int`. A exaustividade de patterns
está satisfeita. Mas se `x <= 0`, nenhum guard dispara e a cláusula
não produz valor — o mesmo tipo de crash que a exaustividade de
patterns foi criada para evitar.

Hoje `infer_lambda_body` (apply_lambda.rs:232) processa guards
linearmente, verificando tipos, mas **não verifica se existe
`otherwise`** (guard com `condition: None`). Se nenhum guard tem
`condition: None` e nenhum dispara em runtime, a cláusula não
produz valor.

### 2.3. `check_exhaustiveness` não serve para guards

`check_exhaustiveness` opera sobre variantes de um tipo (Sum, List).
Guards são expressões booleanas arbitrárias sobre bindings — não há
"variantes" para enumerar. A verificação de completude de guards é
estruturalmente diferente: basta verificar que o último guard é
`otherwise` (sem condição), garantindo fallback.

## 3. Design

### 3.1. Exaustividade de N parâmetros — produto cartesiano

Generalizar `check_clause_exhaustiveness` para operar sobre o produto
cartesiano dos universos de cada parâmetro.

#### Universo de cada posição

Para cada parâmetro `i` com tipo `T_i`, determinar o conjunto de
valores possíveis:

| Tipo | Universo | Razoável? |
|------|----------|-----------|
| `Sum`/`Generic` | todas as variantes do enum | Sim, finito |
| `List` | `{Cons, Nil}` | Sim, 2 elementos |
| `Int`, `Float`, `Text`, `Byte`, `Bytes`, etc. | `{__ANY__}` | Sim, 1 sentinela |
| `Tuple(T1, ..., Tn)` | `{__ANY__}` | Sim — tratado como átomo, não decompõe |
| `Struct` | `{__ANY__}` | Sim — struct não tem variantes |
| `Unit` | `{}` | Sim — 0 elementos (único valor já é coberto por Ident/Wildcard) |

`__ANY__` é um sentinela que representa "qualquer valor deste tipo". Só
`Ident`/`Wildcard` cobre `__ANY__`. `Literal` não cobre (só um valor
específico). Isso preserva a semântica do caso de 1 parâmetro:
tipos infinitos exigem `Ident`/`Wildcard` em toda cláusula.

#### Produto cartesiano

O universo total é o produto dos universes de cada posição. Para 2
Booleans: `{True, False} × {True, False}` = 4 células. Para
`Boolean × Int`: `{True, False} × {__ANY__}` = 2 células.

Sem cap de tamanho. Para os tamanhos práticos (N pequeno, enums com
dezenas de variantes), o produto é trivial em compile-time.

#### Cobertura

Para cada célula do produto, verificar se alguma cláusula a cobre.
Uma cláusula cobre a célula se, em **todas** as posições, o pattern da
cláusula cobre a variante da célula:

| Pattern | Cobre `True` | Cobre `False` | Cobre `__ANY__` |
|---------|-------------|---------------|------------------|
| `Variant{True}` | ✅ | ❌ | ❌ |
| `Variant{False}` | ❌ | ✅ | ❌ |
| `Cons` | — | — | ❌ |
| `Nil` | — | — | ❌ |
| `Ident` | ✅ | ✅ | ✅ |
| `Wildcard` | ✅ | ✅ | ✅ |
| `Literal{42}` | ❌ | ❌ | ❌ |
| `Tuple(all Ident/Wildcard)` | — | — | ✅ |
| `Tuple(Literal/Variant dentro)` | — | — | ❌ |

Se alguma célula não for coberta por nenhuma cláusula →
`NonExhaustiveMatch` com as células faltantes na mensagem.

#### Caso N=1 degenera

Com 1 parâmetro, o produto tem tamanho 1 = o universo daquela posição.
Cada cláusula cobre a variante se seu pattern matches. Idêntico ao
que `check_exhaustiveness` faz hoje. O algoritmo de produto substitui
o special case de N=1 — uma função para toda aridade.

#### Guards ignorados na cobertura

Consistente com o `match` explícito atual. A cobertura é determinada
só pelos `clause.patterns`. Se os patterns cobrem todo o produto,
a exaustividade está satisfeita. Guards podem ter buracos — isso é
tratado em §3.2.

### 3.2. Completude de guards

Guards são expressões booleanas. Uma cláusula com guards é uma cadeia
de testes:

```
cond1: body1       # se cond1 = True
cond2: body2       # se cond2 = True
otherwise: body3   # se todas anteriores = False (açúcar para True:)
```

A cláusula é exaustiva se a disjunção de todas as condições é uma
tautologia: `cond1 ∨ cond2 ∨ ... ∨ True` (com `otherwise`) é
trivialmente `True`. Sem `otherwise`, precisamos provar que
`cond1 ∨ cond2 ∨ ...` cobre todos os casos.

Exigir `otherwise` sempre é conservador — rejeita código correto onde
as condições são complementares:

```kata
lambda x:
    > x 0: x
    <= x 0: x * 2     # (x > 0) ∨ (x <= 0) = True — exaustivo sem otherwise
```

Provar que uma expressão booleana é tautologia é indecidível em geral
(equivale a SAT). Mas **SMT solvers** podem decidir muitos casos
práticos, especialmente aritmética linear — o tipo mais comum em guards
de Kata5.

#### Abordagem: Z3 com fallback conservador

Usar Z3 (SMT solver da Microsoft Research) para verificar se a disjunção
das condições é uma tautologia. Z3 raciocina sobre aritmética linear,
lógica proposicional, igualdades — cobrindo os casos práticos de guards
em Kata5 (`> x 0`, `<= x 1`, `= 0 expr`, `and a b`).

O fluxo é:

1. **`otherwise` presente** → trivialmente exaustivo. Sem Z3.
2. **Sem `otherwise`** → traduzir condições para Z3, perguntar se
   `¬(cond1 ∨ cond2 ∨ ...)` é **insatisfazível**:
   - **UNSAT** (tautologia provada) → guards exaustivos, aprovar.
   - **SAT** (contra-exemplo encontrado) → guards não-exaustivos,
     erro `NonExhaustiveMatch` com o contra-exemplo na mensagem.
   - **UNKNOWN** (limite de esforço atingido) → exigir `otherwise`,
     erro `MissingOtherwise`.

Isso é **sound**: UNKNOWN é conservador (exige fallback), nunca aprova
código incompleto. E é menos restritivo que exigir `otherwise` sempre —
código com guards complementares passa.

#### Limite de esforço

Z3 pode retornar `unknown` para fórmulas complexas ou não-lineares.
Definir um `rlimit` (limite de conflicts internos do Z3, configurável,
default ~10000). O `rlimit` conta operações internas do solver, não
wall-clock — determinístico entre máquinas. A mesma fórmula consome
o mesmo número de passos Z3-internos em qualquer hardware, produzindo
o mesmo resultado (UNSAT/SAT/UNKNOWN) em qualquer máquina.

Passado o `rlimit`, Z3 retorna `unknown` → exigir `otherwise`.

Isso garante que a compilação nunca trava por causa da verificação de
guards e que dois binários do mesmo source são idênticos
(reprodutibilidade de build). Inputs adversariais podem falhar a
prova, mas o fallback é conservador (exige `otherwise`), não unsound.

Para garantir determinismo total, fixar também o random seed do Z3
(`smt.random_seed`), senão heurísticas internas com seeds diferentes
podem seguir ramos diferentes e atingir `rlimit` em pontos distintos.

#### Tradução de TypedExpr → Z3

Nem toda expressão de Kata5 é traduzível para Z3. A tradução é
best-effort:

| Expressão Kata5 | Tradução Z3 | Teoria |
|-----------------|-------------|--------|
| Literais Int/Float | Constantes Z3 | LIA/RA |
| `> a b`, `< a b`, `>= a b`, `<= a b` | `>`, `<`, `>=`, `<=` Z3 | LIA |
| `= a b`, `!= a b` | `=`, `!=` Z3 | LIA + igualdade |
| `+ a b`, `- a b`, `* a b` | `+`, `-`, `*` Z3 | LIA |
| `/ a b`, `mod a b` | `div`, `mod` Z3 | LIA (Int division) |
| `and a b`, `or a b`, `not a` | `and`, `or`, `not` Z3 | Propositional |
| Variáveis (`x`, bindings) | Variáveis Z3 | — |
| `with` bindings | Inlinados (macro-expandidos antes da tradução) | — |
| Chamada de função pura conhecida | Inlinar a definição (se recursiva, limite de profundidade) | — |
| Chamada de função FFI (`@ffi`) | Variável booleana opaca (sound, imprecisa) | — |
| Qualquer outra expressão | Variável booleana opaca | — |

Quando uma sub-expressão não é traduzível, vira uma variável booleana
opaca no Z3. Isso é sound (Z3 trata como unknown) mas pode fazer a prova
falhar — se a prova depende da sub-expressão opaca, Z3 retorna `unknown`
→ exige `otherwise`.

#### `with` bindings

`with` bindings (`doubled := * x 2`) são açúcar para `let` e já são
macro-expandidos antes da inferência. Na tradução para Z3, são
substituídos inline — `> doubled 10` vira `> (* x 2) 10`. Se o binding
envolve uma expressão não-traduzível, vira variável opaca.

#### Onde verificar

Em `infer_lambda_body` (apply_lambda.rs), após processar todos os
guards. Se `guards` é não-vazio:

1. Se algum guard tem `condition: None` (`otherwise`) → exaustivo, Ok.
2. Senão, coletar todas as condições (`condition: Some(...)`) e chamar
   Z3 para verificar tautologia da disjunção.

#### Dependência: crate `z3` (sistema ou vendored)

O crate `z3` (Rust bindings) suporta dois modos de build:

- **Sistema**: linka contra a lib nativa instalada no sistema (via
  `pkg-config`). Build instantâneo (~1s), mas exige Z3 instalado.
- **Vendored** (`features = ["vendored"]`): compila o Z3 em C++ como
  parte do build. Sem dependência externa, mas primeira compilação
  lenta (~3min; builds subsequentes usam cache).

O projeto Kata5 valoriza não ter dependências externas para facilitar
instalação em sistemas diversos. O design acomoda os dois modos:

```toml
[dependencies]
z3 = { version = "0.20", default-features = false }

[features]
# Compila Z3 do source — sem dependência de sistema (distribuição)
vendored-z3 = ["z3/vendored"]
```

**Dev local** (com Z3 instalado via `pacman`/`apt`/`brew`):
```bash
cargo build                 # linka contra lib do sistema, ~1s
```

**Distribuição / CI / sistemas sem Z3**:
```bash
cargo build --features vendored-z3   # compila Z3 do source, ~3min
```

Validado empiricamente: ambos os modos funcionam com o mesmo código.
Testes de tautologia e contra-exemplo passam em ambos (~30ms com sistema,
~30ms com vendored após build).

#### Erro

Dois erros possíveis:

1. **SAT (contra-exemplo)** → `NonExhaustiveMatch` com o contra-exemplo:
   ```
   missing: ["x = -1"]
   hint: "guards não cobrem o caso x = -1. Adicione um guard ou use `otherwise:`"
   ```

2. **UNKNOWN (limite)** → `MissingOtherwise`:
   ```
   erro: "não foi possível provar exaustividade dos guards"
   hint: "Z3 não decidiu em tempo hábil. Adicione `otherwise:` como fallback"
   ```

#### Interação com exaustividade de patterns

Ortogonal. A exaustividade de patterns verifica se os patterns cobrem
o produto dos tipos. A completude de guards verifica se, dentro de
cada cláusula que dispara, as condições cobrem todos os casos. Uma
cláusula pode ter patterns exaustivos e guards incompletos, ou
patterns não-exaustivos e guards completos. São verificações
independentes.

### 3.3. Unificação com `check_exhaustiveness`

A função `check_exhaustiveness` (patterns.rs:392) é usada por:
- `infer_match` (_match.rs:297) — match explícito com 1 scrutinee
- `check_clause_exhaustiveness` (function_infer.rs:270) — cláusulas de 1 parâmetro

O algoritmo de produto substitui a chamada em
`check_clause_exhaustiveness`, mas **não** substitui
`check_exhaustiveness` em `infer_match`. O match explícito continua
usando a função original (1 scrutinee, sem produto).

`check_clause_exhaustiveness` passa a ser a função que:
1. Computa os universos de cada parâmetro
2. Computa o produto
3. Verifica cobertura célula-por-célula
4. Reporta `NonExhaustiveMatch` com células faltantes

Não chama mais `check_exhaustiveness`. A lógica de "enumerar variantes
de um enum" é extraída para uma função auxiliar compartilhada se for
reaproveitada por ambos.

## 4. Estruturas afetadas

### 4.1. `check_clause_exhaustiveness` (function_infer.rs)

Reescrita completa. Substitui o special case de N=1 e o early return
de N>1 por uma função que:
1. Constrói os universos de cada parâmetro a partir de `param_types`
   e `ctx.enum_registry`
2. Computa o produto cartesiano
3. Para cada célula, verifica se alguma cláusula cobre
4. Se alguma célula descoberta, retorna `NonExhaustiveMatch`

### 4.2. `infer_lambda_body` (apply_lambda.rs)

Adicionar verificação de completude de guards após o loop de
processamento. Se `guards` é não-vazio e nenhum guard tem
`condition: None`, invocar Z3 para verificar tautologia da disjunção
das condições. Se Z3 retorna SAT ou UNKNOWN, retornar erro
(`NonExhaustiveMatch` ou `MissingOtherwise` respectivamente).

### 4.3. Novo módulo: `guard_completeness.rs` (kata-inference)

Módulo novo com a tradução de `TypedExpr` → expressões Z3 e a
verificação de tautologia. Isola a dependência Z3 neste módulo —
o resto do crate não precisa saber sobre Z3.

### 4.4. `check_exhaustiveness` (patterns.rs)

Inalterada. Continua sendo usada por `infer_match` para match explícito.


## 5. Mensagens de erro

### 5.1. NonExhaustiveMatch para N parâmetros

O erro existente `NonExhaustiveMatch { missing, span, hint }` acomoda
múltiplas variantes faltantes. Para N parâmetros, `missing` lista as
células não cobertas:

```
missing: ["(False, True)", "(False, False)"]
hint: "combinações faltantes: (False, True), (False, False). \
       Adicione cláusulas para cada uma ou use `otherwise:` como fallback"
```

Para 1 parâmetro, mantém o formato atual:
```
missing: ["False"]
hint: "variantes faltantes: False. Adicione um caso para cada uma \
       ou use `otherwise:` como fallback"
```

### 5.2. Erros de completude de guards

Dois casos:

**SAT (contra-exemplo encontrado):**
```
erro: "guards não cobrem todos os casos"
missing: ["x = -1"]
hint: "guards não cobrem o caso x = -1. Adicione um guard ou use `otherwise:`"
```

**UNKNOWN (Z3 não decidiu a tempo):**
```
erro: "não foi possível provar exaustividade dos guards"
hint: "Z3 não decidiu em tempo hábil. Adicione `otherwise:` como fallback"
```

Reusar `MiddleError::NonExhaustiveMatch` para SAT (com contra-exemplo
no campo `missing`) e `MiddleError::MissingOtherwise` para UNKNOWN.

## 6. Testes

### 6.1. Exaustividade de N parâmetros

| Teste | Entrada | Esperado |
|-------|---------|----------|
| 2 Boolean exaustivo | `True True`, `True False`, `False True`, `False False` | Ok |
| 2 Boolean não-exaustivo | `True True`, `True False` | `NonExhaustiveMatch` missing `(False, True)`, `(False, False)` |
| Boolean × Int com Ident | `True x`, `False _` | Ok (Ident/Wildcard cobre `__ANY__`) |
| Boolean × Int sem Ident | `True 0`, `False 1` | `NonExhaustiveMatch` (Int não coberto) |
| 1 Boolean (degenera) | `True`, `False` | Ok — idêntico ao comportamento atual |
| 1 List (degenera) | `[]`, `[h:t]` | Ok — idêntico ao comportamento atual |
| 3 Boolean exaustivo | 8 cláusulas cobrindo todas as combinações | Ok |
| 3 Boolean não-exaustivo | 4 cláusulas | `NonExhaustiveMatch` com 4 células faltantes |
| Tuple como parâmetro | `(a, b)` com Ident | Ok — Tuple é átomo |

### 6.2. Completude de guards

| Teste | Entrada | Esperado |
|-------|---------|----------|
| Guards com otherwise | `x > 0: x`, `otherwise: 0` | Ok (trivialmente exaustivo) |
| Guards complementares sem otherwise | `> x 0: x`, `<= x 0: x * 2` | Ok (Z3 prova tautologia) |
| Guards não-exaustivos sem otherwise | `> x 0: x` (sem fallback) | `NonExhaustiveMatch` (Z3 acha contra-exemplo `x = -1`) |
| Guard único sem condição | `otherwise: x` | Ok (trivialmente completo) |
| Sem guards | `x` (body direto) | Ok (não se aplica) |
| Guards com expressão opaca | `check_divisor x y: False`, `otherwise: True` | Ok (otherwise presente) |
| Guards com expressão opaca sem otherwise | `check_divisor x y: False`, `> y limit: True` | `MissingOtherwise` (Z3 retorna unknown) |

### 6.3. Snapshot

Snapshots insta em `tests/tast_snapshot.rs` para os casos de
exaustividade de N parâmetros, seguindo o padrão do
`tast_cons_pattern` existente.

## 7. Passos de implementação

1. **Extrair `enum_universe`** — função auxiliar que dado um `Ty` e
   `EnumRegistry`, retorna `Vec<String>` com as variantes (ou
   `["Cons", "Nil"]` para List, ou `["__ANY__"]` para tipos infinitos).
   Colocar em `patterns.rs` ao lado de `check_exhaustiveness`.

2. **Extrair `pattern_covers_cell`** — função que dado um `&TypedPattern`
   e uma variante (`&str`), retorna `bool` indicando se o pattern cobre
   a variante. Refatorar a lógica que já existe no loop de coleta de
   variantes em `check_clause_exhaustiveness`.

3. **Reescrever `check_clause_exhaustiveness`** — computar universos,
   produto, cobertura célula-por-célula. Eliminar special case de N=1
   e early return de N>1.

4. **Criar `guard_completeness.rs`** — módulo novo com tradução
   `TypedExpr` → Z3 e verificação de tautologia. Isolar dependência Z3
   neste módulo.

5. **Adicionar verificação de guards em `infer_lambda_body`** — após
   processar guards, se sem `otherwise`, chamar Z3 via
   `guard_completeness.rs`.

6. **Testes** — casos de §6.1 e §6.2 em
   `crates/kata-inference/tests/lambda_match_inference.rs` (ou arquivo
   novo dedicado a exaustividade). Snapshots em `tast_snapshot.rs`.

7. `cargo test --workspace` — zero regressão.

8. `graphify update .`

## 8. Atualização de TODO.md

Após implementação:
- Remover item `src/infer/function_infer.rs:219` dos TODOs esparsos
- Remover comentário `TODO` do código-fonte
- Registrar completude de guards como resolvida

## 9. Fora do escopo

- **Algoritmo de Maranget completo** — o algoritmo de produto é uma
  generalização direta do caso de 1 parâmetro, não um algoritmo de
  pattern matching formal. Não cobre patterns aninhados profundamente
  (ex: `Cons(Variant{True}, _)` — o Cons é estrutural, mas o Variant
  dentro da cabeça não é verificado). Para os casos práticos de Kata5
  (patterns rasos sobre variantes), é suficiente. Maranget seria
  necessário se Kata5 adotasse patterns profundamente aninhados.

- **Patterns aninhados** — `Variant` com `sub_patterns` (payload) não
  é decomposto. `Some(True)` não é verificado contra `Some(False)` —
  o `Some` é tratado como átomo. Estender isso é um item separado
  (requer recursão no produto).

- **Inlinamento de funções recursivas em guards** — a tradução para Z3
  inlinaria funções puras conhecidas, mas funções recursivas precisam
  de um limite de profundidade. Se a prova falha por limite, Z3
  retorna unknown → exige `otherwise`. Estender o inlinamento com
  unrolling ou axioms é item separado.

- **Float em guards** — Z3 suporta aritmética não-linear sobre floats
  (NRA), mas a teoria é mais lenta e menos robusta que LIA (Int). Se
  guards com Float forem comuns, pode ser necessário ajustar o timeout
  ou tratar Float como opaco (conservador).