# PRD — STEPPABLE Neutral Check + Pipe Limitado (`|N>`)

**Data:** 2026-08-17
**Status:** Planejamento
**Autor:** Arthur + Hermes

## Contexto

O PRD-range-steppable implementou ranges com step opcional via interface STEPPABLE.
Durante a revisão, descobriu-se que step=0 produz loop infinito — o codegen não tem
guard contra degeneração.

Após discussão, Arthur decidiu que a degeneração deve ser rejeitada em **compile-time**,
não tratada em runtime. O typeck verifica se o step é neutro quando o range é criado.
Se for, é erro de tipo. **Não há range degenerado em runtime** — nenhum consumidor
precisa de caso especial.

Adicionalmente, um novo operador de pipe limitado (`|N>`) permite truncar pipelines
sobre iterables em N elementos. É uma feature independente — não existe para resolver
range degenerado, mas para dar terminação antecipada a pipelines.

## Decisões de design

### Step neutro é erro de compile-time

O step do range (seja explícito ou default via STEPPABLE) deve ser avaliado em
compile-time. Se o step é neutro (não progressivo — `start + step == start`),
o typeck emite erro: "range step é neutro — range degenerado".

Mecanismo: a interface STEPPABLE define `is_neutral :: Self => Boolean`.
O typeck chama `is_neutral(step)` em compile-time. Para Int, `is_neutral(0)` →
true. Para Float, `is_neutral(0.0)` → true. Step default (1 para Int, 1.0 para
Float) nunca é neutro — sempre passa.

Alternativa sem `is_neutral`: o typeck avalia `start + step == start` usando
NUM (`+`) e EQ (`==`). Mas isso exige avaliar `+` e `==` em compile-time, que
o typeck não faz hoje. `is_neutral` na interface é mais direto — o typeck só
precisa avaliar uma função que retorna Boolean para um literal.

### Step deve ser avaliável em compile-time

Hoje `step_default_literal` (collections.rs:321) hardcodeia Int→1, Float→1.0
— não avalia a lambda `step` da interface. Para o check de neutralidade
funcionar, o step precisa ser um valor conhecido em compile-time.

Para Int e Float literais, o typeck já tem o valor. Para `constant` (constantes
de módulo avaliadas em compile-time), o typeck pode usar o valor avaliado. Para
expressões dinâmicas, o step não é conhecido em compile-time e o check não
pode rodar — neste caso, o range é aceito sem check (step dinâmico não pode
ser verificado sem comptime pass).

`constant` é o mecanismo de compile-time existente. Step de range deve ser
literal ou `constant`. Expressões dinâmicas como step são aceitas sem check
(o usuário assume a responsabilidade).

### `|N>` é independente

O pipe limitado não existe para resolver range degenerado — é uma feature útil
por si só. Permite truncar um pipeline em N elementos sem materializar a coleção
completa. Funciona com qualquer iterable (Range, List, Array), finito ou grande.

## Fases

### Fase 1: `is_neutral` na interface STEPPABLE + check em compile-time

#### 1a. Interface

Adicionar `is_neutral` à interface STEPPABLE no prelude:

```kata
interface STEPPABLE
    step :: Self => Self
    is_neutral :: Self => Boolean
    lambda x:
        false
    lambda x:
        false
```

- Default method `is_neutral` retorna `false` (presume não-neutro). Tipos
  concretos fazem shadow com a lógica específica.
- `step` continua com default identidade (já existente).

#### 1b. Impls de Int e Float

```kata
Int implements STEPPABLE
    step :: Int => Int
    lambda x: 1
    is_neutral :: Int => Boolean
    lambda x: = x 0

Float implements STEPPABLE
    step :: Float => Float
    lambda x: 1.0
    is_neutral :: Float => Boolean
    lambda x: = x 0.0
```

#### 1c. Typeck: check de neutralidade

- **Arquivo:** `crates/kata-inference/src/infer/collections.rs`
  (`infer_range_lit`, linha 233)
- Após resolver o step (seja default via `step_default_literal`, seja explícito
  via `infer_expr`), verificar se o step é um literal conhecido em compile-time.
  - Se é `IntLit`: chamar `is_neutral` com o valor. Se true, erro de tipo.
  - Se é `FloatLit`: chamar `is_neutral` com o valor. Se true, erro de tipo.
  - Se não é literal (expressão dinâmica): aceitar sem check.
- O check é hardcodeado para Int e Float (igual `step_default_literal` já faz).
  Para tipos customizados no futuro: necessita avaliador de lambdas no typeck
  (comptime pass). Não existe hoje — aceitar limitação.

#### 1d. Testes

- Testes de inferência em `crates/kata-inference/tests/collections_inference.rs`:
  - `[0..0..10]` → erro de tipo (step neutro)
  - `[0..=10]` → ok (step default = 1, não neutro)
  - `[0..2..10]` → ok (step explícito = 2, não neutro)
  - `[0.0..0.0..10.0]` → erro de tipo (step neutro Float)
  - `[0..f()..10]` (step dinâmico) → ok (não pode verificar em compile-time)
- Testes E2A em `crates/kata-driver/tests/build_e2e.rs`:
  - `for x in [0..0..10]: echo!(x)` → erro de compilação (não runtime)

#### 1e. Atualizar documentação

- `docs/sintaxe-mapa.md` — seção Ranges: adicionar nota sobre step neutro
- `docs/Kata-lang-manual.md` — seção 8.1: adicionar nota sobre step neutro

### Fase 2: `|N>` pipe limitado

#### Sintaxe

```
lhs |N> rhs
```

onde `N` é um literal inteiro ≥ 0. Limita a iteração do consumidor à direita a
no máximo N elementos.

#### Semântica

- `|N>` só faz sentido quando `lhs` é iterable e `rhs` é um consumidor que itera
  (`map`, `filter`, `fold`).
- `|>` sem número é composição normal como hoje.
- `|0>` produz iterable vazio (zero iterações).
- Com lista finita: `|N>` age como `take N` — pega os primeiros N.
- Com range muito grande: `|N>` limita iteração, evitando materialização completa.
- Posição importa: `[0..1..1000000] |> map f |3> filter pred` pega 3 do map,
  filtra. `[0..1..1000000] |> map f |> filter pred |3>` filtra tudo e pega 3.

#### Decisões pendentes (confirmar com Arthur)

1. **N literal ou variável?** `|5>` (literal) é simples no lexer. `|n>` (variável)
   exige que o lexer diferencie `|ident>` de `|>` e que o typeck avalie `n`.
   Recomendação: começar com literal, adicionar variável depois se necessário.

2. **`|N>` com filter**: `|5> filter pred` itera 5 elementos do source e filtra
   — pode retornar menos de 5 (elementos que não passam no predicado).
   Alternativa: "primeiros 5 que passam" — itera até achar 5 que satisfazem
   (potencialmente percorre muito mais). Recomendação: "itera N do source e
   filtra" (take-then-filter), mais simples e consistente com a posição no
   pipeline.

3. **`|N>` com fold**: `|5> fold op init` reduz apenas os primeiros 5 elementos.
   fold sem `|N>` reduz tudo (comportamento atual). Sem ambiguidade — `|N>` é
   posicional.

4. **Onde mora no typeck**: hoje `|>` é desugarado em composição de função
   (`desugar_pipes` em `desugar.rs`). `|N>` não desagua para nada existente.
   Opções:
   - **Novo nó TAST** `PipeLimit { lhs, rhs, limit }` — o codegen dos builtins
     recebe o limite como parâmetro.
   - **Desugar para `take`** — se `take` existisse, `|N>` desugararia para
     `take lhs N |> rhs`. Mas `take` não existe.
   Recomendação: novo nó TAST, o codegen de cada builtin incorpora o contador.

5. **Codegen**: map/filter/fold têm lowering dedicado com loops próprios. Cada
   um precisa de um contador adicional (0..N) que para o loop em N iterações.
   `fused_stream` (map|>filter) também. `for_in` não é afetado (tem `break`).

#### Camadas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| Lexer | `kata-lexer/src/dispatch.rs:75-83` | Detectar `\|<número>>` |
| Token | `kata-ast/src/token.rs:112` | Novo token `PipeLimit` ou estender `PipeForward` |
| AST | `kata-ast/src/expr.rs:121-123` | Novo variant `Expr::PipeLimit { lhs, rhs, limit }` |
| Parser | `kata-parser/src/expr_apply.rs:38` | Detectar `\|N>` após aplicação |
| Desugar | `kata-inference/src/desugar.rs:37-43` | `|N>` não desigua para Apply. Novo nó ou desugar especial |
| Codegen | `map.rs`, `filter.rs`, `collections_hof.rs`, `fused_stream.rs` | Contador de limite no loop |

#### Testes E2E

- `[0 1 2 3 4 5 6 7 8 9] |3> map (+ _ 1)` → `[1, 2, 3]` (lista finita, take 3)
- `[0..2..10] |3> map (+ _ 1)` → `[1, 3, 5]` (range finito, take 3)
- `[0..1..1000000] |5> map (+ _ 1)` → `[1, 2, 3, 4, 5]` (range grande, take 5)
- `[0 1 2 3 4] |0> map (+ _ 1)` → `[]` (zero iterações)

## Ordem de implementação

1. **Fase 1** (is_neutral + check compile-time) — interface, impls, typeck
2. **Fase 2** (`|N>`) — feature nova, 6 camadas

## Não-objetivos

- **`take` como HOF separada** — `|N>` substitui a necessidade. Se `take` for
  adicionada depois, `|N>` desugaria para ela, mas não é necessária agora.
- **Range como `Result`** — range não vira tipo algebraico. Degeneração é
  rejeitada em compile-time, não propagada como valor de erro.
- **Check de degeneração em runtime** — não há range degenerado em runtime.
  `contains` e `len` não mudam — funcionam como hoje.
- **Range infinito como tipo distinto** — não há tipo `InfiniteRange`.
- **Avaliar `step` de tipos customizados em compile-time** — exige comptime
  pass que não existe. Aceitar limitação: só Int e Float são verificados.