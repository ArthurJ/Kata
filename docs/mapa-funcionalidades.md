# Mapa de Funcionalidades — Kata-Lang 1.0

Árvore de dependências das funcionalidades da linguagem, organizada em
camadas. Cada camada só pode ser implementada após as anteriores estarem
funcionando. A estratégia de implementação é tracer bullets — fios
verticais end-to-end que cortam camadas — mas a árvore de dependências
define o que é pré-requisito do quê.

## Camadas

### 0 — Lexer/Parser

Lexer indent-sensitive (emite INDENT/DEDENT sintéticos). Parser
recursive-descent, prefix-only (sem Pratt). Parênteses por vírgula: tem
vírgula = tuplo `(1, 2, 3)`; não tem vírgula = agrupamento `(+ 1 2)`.

Saída: AST plana (crate de dados puros, sem lógica).

### 1 — Expressões mínimas

Literal (Int, Float, Text). Apply (arity-aware: `f arg1 arg2` coleta exatamente
a aridade padrão do callee). Grouping (parênteses sem vírgula — transparente).
Tuple (parênteses com vírgula). Dict dispatch (`f{"k": v}` — args nomeados via
DictLit, sem `!` para funções puras).

Dependencies: 0.

### 2 — Lambda, Pattern Matching, Diretivas

- **Lambda:** anônima, múltiplas cláusulas. Pattern (Ident, Wildcard,
  Literal, Variant, Tuple, Cons). Guard (`> x 0: ... otherwise: ...`).
  `match` (scrutinee + clauses + otherwise). Cláusulas lambda múltiplas
  (dispatch nos parâmetros). Exhaustiveness checking.
- **Diretivas (infra):** lexer reconhece `@nome`, `@nome("arg")`,
  `@nome{chave: valor}`. Parser produz `Directive { name, args }` anexado
  ao item (prefixo — antes da assinatura em todos os contextos: top-level
  e métodos em implements/refines). AST carrega `Vec<Directive>` em toda
  declaração. Typeck consulta via `item.directives.find("nome")`. Semântica
  de cada diretiva é implementada pela feature que a consome — esta camada
  só fornece o canal.

Dependencies: 1.

### 3 — Barreira Functions vs Actions

`action` (keyword) + `!` (call suffix). `loop`/`for`/`break`/`continue`
(Actions only). `var` (Actions only). `?` (fail-fast, Actions only —
injeta `return Err` na TAST).

A barreira é sintática — o typeck bifurca por domínio desde o início.

Uniformização de aplicação: funções e actions aceitam args posicionais
(`f a b` / `f!(a, b)`) e nomeados (`f{"k": v}` / `f!{"k": v}`). O parser é
arity-aware (ciclo de dois passes: Pass 1 extrai aridades, Pass 2 parseia
com aridades). `!` marca side-effect — funções puras não usam `!`, mesmo
em dict dispatch.

Dependencies: 2.

### 4 — Type declarations

`data` (struct/produto). `enum` (sum, variantes por indentação). `alias`
(newtype, zero-cost). Variant como função de 1ª classe (`Some :: T =>
Optional`). Qualificação `Enum::Variante`.

Os três são construtores de tipo que o typeck registra no TypeEnv.

Dependencies: 2.

### 5 — Currying e closures

`_` (hole em posição de argumento) → `Lambda` com captures. Desugar no
typeck — a TAST nunca contém `Hole`. Captura léxica de variáveis
externas. `Closure` na TAST (campos: `name`, `args`, `holes`,
`captures`, `escapes`).

O mesmo mecanismo em qualquer posição de argumento: currying de
aplicação, predicados de refined, predicados de enum, posicionamento de
`|>`.

Dependencies: 2 (Lambda), 1 (Apply).

### 6 — Smart constructors básicos

Síntese de construtor para todo `CamelCase` invocável:
- `data` struct: infalível, mapeamento posicional
- `enum` variant: função de 1ª classe (sem síntese extra)
- `alias` não-refined: delega ao base (pass-through)

Dependencies: 4 (type declarations), 2 (Lambda — construtor é uma
lambda).

### 7 — Dispatch

Interfaces (super-traits, `implements`). Multiple dispatch (scoring por
dominância). Interoperabilidade (tipos da mesma interface operam juntos).
`with` (restrições de genéricos — `T implements ORD`). Cláusula
genérica de fallback (`T NUM`).

Diretivas que entram aqui:
- `@commutative` — dispatch tenta argumentos invertidos
- `@cache` — memoização de funções puras (strategy: LRU/FIFO/MRU/LFU, capacity configurável). Com tail calls, wrapper/inner split preserva TCO.
- `@timer` — medição de tempo de execução (topic, msg). Com tail calls, wrapper/inner split preserva TCO.

Dependencies: 2 (múltiplas cláusulas = dispatch), 4 (tipos concretos).

### 8 — Predicados e tipos refinados

Predicado = `T => Bool` via currying (camada 5). Smart constructor
consome `Vec<T => Bool>` com estratégia de combinação:
- **All(preds):** refined — N predicados, AND, saída `Result::(Self,
  Error)`. Guard aninhado: `p1: p2: ... pN: Ok(wrapped) | otherwise: Err`.
- **First(preds):** enum predicado — N−1 predicados, first-match, saída
  `Enum`. Guard plano: `p1: V1(val) | p2: V2(val) | otherwise:
  Vn(val)`.

O typeck decide a estratégia pela forma da declaração: `data` com
predicados → `All`; `enum` com predicados → `First`.

Ascription `expr::Type`: três modos semânticos (ver manual §4.2.7):
1. **Rebaixamento de literal** — texto bruto reinterpretado no tipo alvo
   desde o início (`42::Float`, `3.14::Rational`). Sem conversão em
   runtime. Disponível desde Fio 1.
2. **Confirmação de tipo** — verifica que a expressão já tem o tipo
   alvo (`42::Int`). No-op em runtime. Disponível desde Fio 1.
3. **Ascription-construção** (Fio 5+) — promove tupla anónima a tipo
   nominal (`("João" 30)::Pessoa`). Valida shape e anexa `type_id`.

Ascription para tipos refinados:
- **Literais:** validação compile-time (avaliação constante local ao
  typeck), entrega tipo refined sem `Result`. Zero-cost.
- **Não-literais:** produz `TypedExprKind::Ascription` para o comptime
  pass (Fio 12) avaliar via JIT. Se o comptime não consegue avaliar,
  erro de compilação.

**Ret-directed dispatch** (Fio 6): a ascription propaga o tipo anotado
como `hint_ret` para `DispatchTable::resolve`. O dispatch filtra
sobrecargas cujo retorno é compatível com o hint. Desambigua operações
polimórficas: `(/ 1 3)::Int` seleciona `idiv`, `(/ 1.0 3.0)::Float`
seleciona `fdiv`.

**Grouped como barreira de hint** (Fio 6): `Grouped(inner)` = strip
(hint atravessa). `Grouped(Grouped(...))` = barrier (avalia sem hint,
depois converte via `convert_typed_expr`). Permite `((/ 1 3))::Rational`
quando `(/ 1 3)::Rational` falha.

**Ascription vs construtor** (ver manual §4.2.8): quatro diferenças —
identidade (nominal vs estrutural), first-class, validação de shape,
refinamento. Ascription é zero-cost para literais; construtor é geral
para valores não-literais (retorna `Result`). Ambos falham com
`TypeMismatch`; ascription additionally falha com **refinamento não
atendido** (predicado avaliado em typeck rejeita antes do codegen).

**Cross-type `/`, `//`, `div`, `mod` com `NonZero` qualificado**: `/`, `//`,
`div`, `mod` operam entre tipos numéricos distintos (Int, Float, Rational).
`/` (divisão exata) e `mod` (resto) retornam o tipo do dividendo; `//`
(divisão inteira) retorna sempre `Int`. O segundo argumento é `NonZero`
(família polimórfica) — a instância concreta é qualificada na declaração:
`mod :: Float NonZero::Int => Float`. O `resolve_type_expr` resolve
`NonZero::Int` → `Instance("NonZero", "Int")`. Ascription de literal
(`3::NonZero`) promove `Family` → `Instance` baseado no tipo primitivo do
inner. Construtor falível (`NonZero(3)`, `NonZero(rational 3)`) retorna
`Result::(Instance("NonZero", "Int"), Text)` — não `Family` — permitindo
dispatch não-ambíguo no call-site. `fits_return` aceita `Instance(family,
_)` como compatível com `Family(family)`. `mod` Float/Rational usa
truncagem (`a - b*(float(int(a/b)))`) em Kata puro para evitar erro de
precisão IEEE 754 — sem FFI adicional. `//` implementado em Kata puro:
`int (/ a b)` — `int` trunca Float/Rational para Int; Int same-type usa
FFI `kata_rt_bi_div` diretamente.

Coerção contextual no `|`: fallback literal validado em compile-time
contra predicados.

Dependencies: 5 (currying produz `T => Bool`), 6 (smart constructor
consome predicados), 7 (dispatch resolve operadores dentro dos
predicados).

### 9 — Error handling

`Result::(T, E)` e `Optional::T` (enums da stdlib). `|` (coalescência —
última variante = falha, desempacota resto; funciona com qualquer enum
cujas variantes não-cauda carregam payload). `panic!` /
`unwrap_or_panic!`.

`?` já existe na camada 3, mas `|` depende de predicados (coerção
contextual).

Dependencies: 4 (enum), 8 (coerção contextual no `|`).

### 10 — FFI e stdlib mínima

`@ffi("kata_rt_symbol")` — ponte para símbolo C nativo. `FfiSymbol` enum
tipado. `kata-rt` (aritmética, comparação, I/O, strings, arena). Stdlib
`core.kata` define `+`, `-`, `=`, `<`, etc. via `@ffi`.

CLI que entra aqui: `lex`, `parse` (camada 0, mas só faz sentido com
binário), `eval` (JET one-liner — pipeline mínimo sem optimizer), `run`
(`eval` lendo de arquivo).

Dependencies: 2 (assinaturas), 7 (dispatch seleciona sobrecarga FFI).

### 11 — Collections e `@builtin`

List `[T]` (Cons, persistente). Array `{T}` (contíguo). Range `[0..10]`
(lazy). Interface `ITERABLE`. `@builtin("map"/"filter"/"fold")` —
typeck intercepta diretiva e gera nós TAST estruturados. Stream fusion
(fusão de cadeias map/filter/fold na TAST — `stream_fusion.rs`,
`FusedStream` na TAST, `stream_fusion_e2e.rs`).

Dependencies: 4 (ADTs para Cons), 7 (ITERABLE como interface), 10 (FFI
para operações de coleção no runtime).

### 12 — `|>` pipeline

Desugar total no typeck — a TAST nunca contém `Pipe`. Usa `_` para
posicionamento. Associatividade à esquerda.

Dependencies: 5 (currying — `_` posiciona argumento).

### 13 — Newtype / Orphan Rule

`alias implements IFACE externa` — resolver impasse de implementar
interface externa em tipo externo. Esta é a parte do `alias` que
precisa de interfaces (a declaração de alias em si está na camada 4).

Dependencies: 4 (alias), 7 (interfaces).

### 14 — Modelo de memória

Arena (bump allocator, O(1) free ao término da Action). Escape Analysis
(4 passes sobre TAST: retorno de função, posições de escape,
propagação de aliases, promoção de CaptureStorage). ARC (incref/decref
injetados pelo codegen no epílogo da Action). `FnValueCall`
(call_indirect para closures escapadas).

Dependencies: 3 (Actions — arena é por Action), 5 (closures — escape
analysis opera sobre captures).

### 15 — Concorrência (CSP)

`fork!` (fibers via wasmtime-fiber). `channel!` (rendezvous),
`queue!(N)` (buffer), `broadcast!` (pub-sub). `<!` / `!>`
(send/receive). `select` (multiplexação, timeout).

Diretivas: `@log` (telemetria via canais CSP). Special forms: `spawn!`
(multiprocess — processo OS isolado via fork+IPC).

Dependencies: 3 (concorrência só no domínio impuro), 14 (escape analysis
— dados em canais escapam → ARC), 10 (FFI — channels são runtime).

### 16 — Otimização de TAST

Otimizações que operam sobre a TAST antes do lowering:

- Monomorfização (especializar genéricos nos call sites)
- Ascription evaluation (`5::PositiveInt` — predicado avaliado em
  compile-time, entregue como literal)
- Tree shaking / dead code elimination (remover declarações
  inalcançáveis a partir de Actions; elimina `@test` e `assert!` em
  `build --release`)
- Stream fusion (`@builtin` map/filter/fold → fusão de cadeias)

Diretiva: `@test` (tree shaking remove em build).

Dependencies: 7 (monomorfização precisa de dispatch), 8 (ascription
eval), 11 (stream fusion precisa de @builtin).

### 17 — Otimização de IR

Otimizações que operam sobre o IR depois do lowering. Constant folding,
DCE, inlining e TCO são delegados ao Cranelift — o `JITModule` já aplica
otimizações nativas durante a compilação CLIF → machine code. Não há
loop de ponto fixo próprio; `optimize()` aplica os passes próprios uma
vez cada.

Passes próprios em `kata-optimizer`:

- TRMA (Tail Recursion Modulo Associativity — depende de
  `@associative`/`@commutative`: injeta acumulador, converte recursão
  bloqueada em cauda)
- Stream Fusion (`@builtin` map/filter/fold → fusão de cadeias)

ARC (incref/decref) é injetado no codegen, não no optimizer — o epílogo
da Action emite decref de variáveis ARC e close de handles de I/O.

Diretiva: `@associative` (habilita TRMA).

Dependencies: 10 (IR existe), 14 (ARC no codegen precisa de info de
escape).

Se a TAST carregar `tail_pos`, `escape`, `mono_instance` (Recomendação
2 do post-mortem), o lowering preserva essa informação no IR e o
optimizer não precisa rederivar. TCO vira transformação direta; heap-to-
stack torna-se viável.

### 18 — `@comptime`

JIT-and-execute: compila a expressão via pipeline normal e executa no
runtime real. Sem interpretador de TAST. Semântica dual:
- Definition-site (hint): calls com args constantes → compile-time;
  args não-constantes → runtime normal.
- Call-site (guarantee): força compile-time; se não consegue → erro.

Dependencies: 10 (JIT/eval), 14 (runtime linkado no processo).

### 19 — Build AOT e REPL

`build` (AOT com cranelift-object + linker, pipeline completo com
optimizer). `repl` (TypeEnv persistente entre expressões, comandos
`:type`, `:env`).

Dependencies: 16-17 (build precisa de optimizer), 3 (repl precisa de
module system para persistência).

## Funcionalidades transversais

Funcionalidades implementadas que atravessam múltiplas camadas do
pipeline e não se encaixam no esquema de camadas sequenciais:

- **Sistema de módulos (import/export):** `module_loader.rs`,
  `merge_imports.rs`. `mod.kata` (arquivo de módulo por diretório),
  `super.` (referência ao módulo pai), `stdlib.` (referência à
  biblioteca padrão). Resolution e merge de imports em
  `kata-resolution`.
- **File I/O:** `File` (handles abertos/fechados pelo epílogo da
  Action). PRD-file-io.
- **stdio como valores:** `__stdin__`, `__stdout__`, `__stderr__` —
  bindings `Ty::File` injetados estruturalmente pelo resolution no
  módulo `stdio`. Disponíveis via `import stdio`. Lazy materialization:
  o handle só é criado na primeira referência ao valor (FFI
  `kata_rt_stdin/stdout/stderr`). `close!` é no-op (`is_stdio` flag),
  read/write enforcement por `IoMode` (runtime, não tipo).
  PRD-stdio-values.
- **Sockets:** `Socket` (TCP/UDP), `select` (multiplexação de I/O
  handles — file e socket). PRD-socket-io, PRD-select-io.
- **Doctests:** `kata-driver/src/doctest.rs` — extração e execução de
  blocos de documentação como testes.
- **LSP:** crate `kata-lsp` — Language Server Protocol para
  integração com editores.

## Caminho crítico

```
0 → 1 → 2 → 4 → 6 → 5 → 8 → 9 → 7 → 10 → 14 → 15
```

13 passos até "linguagem concorrente que faz I/O".

## Pontos de bifurcação

Após camada 7 (Dispatch), três frentes paralelas:
- 8-9 (predicados + error handling) — sistema de tipos
- 10-11 (FFI + collections) — runtime bridge
- 12-13 (`|>` + newtype) — syntactic sugar / orphan rule

Após camada 14 (Memória), duas frentes:
- 15 (Concorrência)
- 16-17 (Otimização TAST + IR)

Após camada 10 (FFI/eval), CLI básica (`lex`, `parse`, `eval`, `run`)
já é possível — não precisa de optimizer.