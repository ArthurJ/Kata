# Roadmap — Kata-Lang 1.0

## Estratégia

Cada fio é uma **tracer bullet**: vai do frontend ao backend, implementando uma
feature end-to-end. Nenhum fio é "só typeck" ou "só codegen" — se o typeck
aprova, o codegen executa. A árvore de dependências entre features determina a
ordem dos fios.

Zeladorias são planejadas após marcos naturais, não como afterthought. O modelo
vertical acumula débito horizontal; as zeladorias pagam essa dívida.

## Maquinaria do Sistema de Tipos

O sistema de tipos não é um fio — é o esqueleto que sustenta todos os fios. Cada
fio constrói parte da maquinaria. O roadmap marca explicitamente qual
maquinaria cada fio traz, para que a infraestrutura seja construída na ordem
correta e não retrofitted.

### Linha do tempo da maquinaria

| Maquinaria | Fio | Por quê |
|---|---|---|
| `TypeEnv` (escopos, name resolution) | Fio 1 | Toda inferência precisa resolver nomes |
| `Ty` canônico: `Prim`, `Unit`, `Struct`, `Sum` | Fio 1 | Tipos básicos (Prim = mapeamento FFI, não tipo da linguagem) |
| `PrimTy` (mapeamento de representação nativa) | Fio 1 | Contrato FFI: `i64`, `f64`, `kata_rt_string`. Não é tipo da linguagem — é como o codegen mapeia `Ty → ABI` |
| `data` (tipos opacos: `data Int ()` com `@ffi`) | Fio 1 | Prelude declara `Int`, `Float`, `Text` como `data` opacos com `@ffi` — não primitivos |
| `enum` básico (variantes unitárias) | Fio 1 | Prelude declara `enum Boolean { True, False }` — não primitivo |
| `Ty::Sum` com variantes unitárias | Fio 1 | `Boolean` |
| `::` em assinatura (parser → `Signature`) | Fio 1 | Prelude declara `+ :: Int Int => Int` |
| `::` em qualificação de variante (`Boolean::True`) | Fio 1 | Enums unitários |
| `::` postfix reconhecido pelo parser | Fio 1 | Parser tem a tabela postfix completa desde o início |
| `DispatchTable` com scoring por dominância | Fio 1 | Nasce com scoring, mesmo com 1 overload |
| `FfiSymbol` enum tipado | Fio 1 | Catálogo de símbolos FFI |
| Inferência básica (tipo de expressões) | Fio 1 | `+ 1 2` precisa inferir `Int` |
| Assinaturas de função (`=>`) | Fio 2 | Lambdas |
| Tipo de função como valor (`->`) | Fio 2 | Higher-order functions |
| Hole `_` (desugar no typeck → `lambda`) | Fio 2 | Currying explícito, predicados |
| `tail_pos: bool` na TAST | Fio 2 | TCO delegado ao Cranelift |
| `Ty::Sum` com payload | Fio 4 | `Ok(T)`, `Some(T)`, variantes com dados |
| `::` em type params (`Result::(T, E)`) | Fio 4 | Enums genéricos (params posicionais, não interfaces) |
| Smart constructor synthesis (falível) | Fio 6 | Construtores de enum predicado e refined types (reusa Hole de Fio 2 + Result de Fio 4) |
| `Ty::Struct` com campos, `Ty::Tuple` | Fio 5 | Data com campos, tuples |
| `::` em campos de struct (`nome::Text`) | Fio 5 | Structs |
| Smart constructor synthesis (infalível) | Fio 5 | Construtores de struct |
| `Ty::Generic`, `Ty::Interface` | Fio 7 | Interfaces, generics |
| Dispatch por dominância com múltiplas overloads | Fio 7 | Scoring já existe; agora múltiplos candidatos |
| Monomorphization nos call sites | Fio 7 | Especialização genérica |
| `mono_instance: u64` na TAST | Fio 7 | Monomorph rastreia instância |
| `::` em ascription de expressão (`expr::Type`) | Fio 6 | Ascription |
| Avaliação constante de predicados (typeck local) | Fio 6 | `5::PositiveInt` valida `> _ 0` em compile-time |
| Ret-directed dispatch (hint de retorno) | Fio 6 | `(/ 1 3)::Int` seleciona sobrecarga por retorno |
| `escape: EscapeKind` na TAST | Fio 9 | Escape analysis (definido em kata-core, não inference, para evitar dependência circular) |
| `capture: Vec<CaptureInfo>` na TAST | Fio 9 | Closures com captura |
| `CaptureStorage` Stack/Heap | Fio 9 | Escape analysis promoção |

### Nota: ascription refined — predicados triviais vs complexos

Predicados triviais (`> _ 0`, `= _ 0`) são **avaliação constante local ao
typeck** — não usam JIT-and-execute. O typeck reduz a expressão booleana com o
literal substituído e verifica `True`/`False`.

Predicados complexos (`is_prime _`) envolvem chamada de função e não podem
ser avaliados localmente. A partir da Fase 4 do Fio 12, o typeck delega
predicados complexos ao comptime pass, que JIT-executa a função predicado.
Não criar dependência Fio 6 → Fio 12 para predicados triviais.

### Nota: `::` é um operador, contextos são typeck

O parser reconhece `::` desde Fio 1 (tabela postfix). Os contextos (assinatura,
campo, type param, variante, ascription) são tratados progressivamente pelo
typeck, não pelo parser. O parser produz a AST; o typeck interpreta por
contexto. Não há mudança no parser entre Fio 1 e Fio 6 — só o typeck ganha
interpretações novas.

### Nota: dispatch por dominância nasce em Fio 1

O `DispatchTable` nasce com scoring por dominância em Fio 1, mesmo que só tenha
uma overload por nome. O algoritmo (coletar candidatos, pontuar, selecionar) é o
mesmo com 1 ou 100 overloads. A diferença é que ambiguidades (duas overloads com
score igual) só surgem em Fio 7 com interfaces. Mas a estrutura do algoritmo
está pronta — não precisa retrofit.

## Árvore de Dependências

```
Fio 1: Fundação + Aritmética + CLI
│   (Int/BigInt/SMI, Float, Text, Rational, Boolean, +, -, *, /, =, <, >,
│    let, @ffi, data, enum)
│   Maquinaria: TypeEnv, Ty, PrimTy (mapeamento FFI), DispatchTable com scoring,
│               FfiSymbol, :: em assinatura, data opaco, enum unitário (Boolean)
│
├── Fio 2: Funções, Lambdas, Guards, Match, Hole
│   │   (=>, ->, lambda, clauses, guards, otherwise, match, patterns,
│   │    Hole _, |>, with, tail_pos, effect)
│   │   Maquinaria: assinaturas, tipo de função, Hole desugar, TAST enriquecida
│   │
│   └── Fio 9: Closures, Escape Analysis, ARC, TRMA
│       (captures, Arc<T>, FnValueCall, @associative → TRMA)
│       Maquinaria: escape, capture, CaptureStorage
│
├── Fio 3: Actions, return, ;, Caller's Arena
│   │   (action, !, var, loop, break, continue, return, ;, ?, arena)
│   │   (`for` adiada para Fio 7+8)
│   │
│   └── Pré-11: Infraestrutura de Memória Hierárquica
│       │   (árvore de arenas, escape analysis para LCA, ARC pass emitido)
│       │
│       └── Fio 11: CSP, Concorrência, Paralelismo
│       │   (channel!, queue!, broadcast!, fork!, select, !>, <!,
│       │    yield points, structured concurrency, spawn!)
│       │
│       └── Fio 14: @log, @test, Test Runner
│           (@log telemetria via CSP, @test positivo/negativo, kata test)
│
├── Fio 4: Enum Avançado — Payload, Result, Optional, |
│   │   (variantes com payload, Result::(T, E), Optional::T, |,
│   │    match general case, panic!, assert!)
│   │   (variantes predicadas adiadas para Fio 6)
│   │   Maquinaria: Ty::Sum com payload, :: em type params, match
│   │              general case
│   │
│   └── Fio 8: Coleções, ITERABLE, Stream Fusion — ✅ Concluído
│       │   (List, Array, Range, map/filter/fold, .N, len, INDEXABLE,
│       │    COUNTABLE, `for x in` iteration)
│       │   Fases 1-9 ✅ (DoDs 1-60, 788 testes). Fio 8 Concluído.
│       │
│       └── Fio 13: Dict, Set (HAMT) ✅ Concluído
│
├── Fio 5: Data, Structs, Tuples, alias ✅ Concluído
│   │   (data, field access, Tuple, .N em tupla, alias/newtype)
│   │   Maquinaria: Ty::Struct, Ty::Tuple, :: em campos, smart constructor
│   │
│   └── Fio 6: Tipos Refinados, Ascription ✅ Concluído
│       (data com predicados, smart constructors falíveis, ::Type,
│        atrito sadio, avaliação constante de predicados)
│       Maquinaria: :: em ascription, smart constructor falível,
│                   avaliação constante, ret-directed dispatch
│
├── Fio 7: Interfaces, Generics, Dispatch ✅ Concluído
│   │   (interface, implements, multiple dispatch, ITERABLE/COUNTABLE/INDEXABLE,
│   │    ORD/EQ/NUM/SHOW, generics, monomorph, @commutative)
│   │   Maquinaria: Ty::Generic, Ty::Interface, dispatch com múltiplas overloads,
│   │              monomorph, mono_instance
│   │
│   └── Fio 8 depende deste (ITERABLE para map/filter/fold)
│
├── Fio 10: Módulos, Prelude, FFI Completo ✅ Concluído
│   (import, export, as, module loader, filesystem, cycle detection,
│    prelude de stdlib/core.kata substituindo prelude hardcoded)
│
├── Fio 11: CSP, Concorrência, Paralelismo ✅ Concluído
│   (channel!, queue!, broadcast!, fork!, !>, <!, select com timeout,
│    yield cooperativo, structured concurrency, scheduler com fibers)
│   (spawn! — multiprocess via fork+IPC — redesign: special form ao lado de fork!,
│    aceita tupla ou dict com raw:/serialized:, to_bytes() FFI — Fase 5 ✅, spawn! Fase 9 ✅)
│
├── Fio 12: Comptime, @cache ✅ Concluído — PRD: docs/PRD-fio12-comptime.md
│   (@comptime call-site explícito, JIT-and-execute, HeapSnapshot com arenas,
│    @cache{strategy: "LRU"} em caller_arena, ascription refined delega ao comptime)
│   Fases 1-6 ✅ + constant folding de funções com args literais (Ponto 7)
│
└── Fio 15: AOT, REPL
    (kata build — Cranelift object + linker, kata repl — TypeEnv persistente)
```

## Fios

### Fio 1: Fundação + Aritmética + CLI ✅ Concluído

**Tracer bullet.** Estabelece o pipeline end-to-end mínimo: source → lexer →
parser → resolution → inference → codegen → CLIF → Cranelift JIT → runtime →
resultado. Tudo o que não existe ainda é stub ou trivial.

**Camadas criadas:** kata-core, kata-ast, kata-lexer, kata-parser,
kata-diagnostics (frontend), kata-resolution, kata-inference, kata-codegen,
kata-rt, kata-driver.

**Maquinaria de tipos construída:**
- `TypeEnv` (árvore de escopos, name resolution)
- `Ty` canônico: `Prim(Int|Float|Text|Rational)` (mapeamento de representação
  FFI — `i64`, `f64`, `kata_rt_string`, `kata_rt_rat`), `Unit`, `Struct`, `Sum`
- `PrimTy` — mapeamento de representação nativa, não tipo da linguagem. Os
  tipos da linguagem (`Int`, `Float`, `Text`, `Rational`) são `data` com `@ffi`
  no prelude; `Boolean` é `enum` no prelude. `PrimTy` é o contrato FFI que o
  codegen usa para mapear `Ty → ABI`.
- Inferência básica (tipo de expressões literais e aritméticas)
- `DispatchTable` com scoring por dominância (nasce com scoring, mesmo com 1
  overload por nome — não é lookup simples)
- `FfiSymbol` enum tipado (catálogo de símbolos FFI com metadados)
- `::` em assinatura (parser reconhece e produz `Signature`; typeck valida)
- `::` postfix reconhecido na tabela postfix do parser (pronto para todos os
  contextos futuros — typeck interpreta progressivamente)
- `data` declaração (tipos opacos: `data Int ()` com `@ffi("i64")`)
- `enum` declaração básica (variantes unitárias: `enum Boolean { True, False }`)
- `Ty::Sum` com variantes unitárias (Boolean — sem payload ainda)
- `::` em qualificação de variante (`Boolean::True`)

**Features:**
- Literais: Int (BigInt/SMI tagging nativo do runtime desde o início), Float,
  Text, Rational
- `let` bindings
- Aplicação prefixa (`+ 1 2`)
- Operadores aritméticos (`+`, `-`, `*`, `/`) e comparação (`=`, `<`, `>`)
- `@ffi` directive (parser + codegen import)
- `data` (tipos opacos: `data Int ()`, `data Float ()`, `data Text ()`,
  `data Rational ()`)
- `enum` básico (variantes unitárias: `enum Boolean { True, False }`)
- Prelude em `stdlib/core.kata` (declara tipos, Boolean, e operadores via @ffi)

**Runtime (kata-rt):**
- `kata_rt_bi_*` (BigInt com SMI tagging — nativo desde o início, não retrofit)
- `kata_rt_iadd`, `kata_rt_isub`, `kata_rt_imul`, `kata_rt_idiv` (legacy, se
  necessário)
- `kata_rt_fadd`, `kata_rt_fsub`, `kata_rt_fmul`, `kata_rt_fdiv`
- `kata_rt_icmp_*`, `kata_rt_fcmp_*`
- `kata_rt_rat_*` (Rational: add, sub, mul, div, eq, lt, show, to_float,
  from_float, int_to_rational)
- `kata_rt_bi_eq`, `kata_rt_bi_lt`, `kata_rt_bi_show`, `kata_rt_bi_to_rational`
- `kata_rt_print`, `kata_rt_string_concat`, `kata_rt_text_literal`

**CLI:** `lex`, `parse`, `eval`, `run`

**Depende de:** nada (fundação)

**DoD:** ✅ `kata eval '+ 1 2'` imprime `3`. `kata run examples/arithmetic.kata`
executa e imprime resultado. Pipeline completo funciona end-to-end. DispatchTable
faz scoring por dominância (mesmo que só tenha 1 candidato). `Boolean` é um
`enum` no prelude, não primitivo do compilador. Commit `2aab7ba`. 289 testes.

---

### Fio 2: Funções, Lambdas, Guards, Match, Hole

**Maquinaria de tipos construída:**
- Assinaturas de função (`=>`) — typeck valida `nome :: T1 T2 => TRet`
- Tipo de função como valor (`->`) — `(A -> B)` como tipo transitável
- Hole `_` — desugar no typeck: `+ 10 _` vira `lambda x: + 10 x` com captures
- `tail_pos: bool` na TAST — marcado pelo typeck, usado pelo Cranelift para TCO
- `Boolean` já existe no TypeEnv via prelude (Fio 1) — Fio 2 usa para guards,
  não constrói a maquinaria de enum (já existe em Fio 1 para variantes unitárias)
- Partial dispatch no DispatchTable: resolve overloads com argumentos
  ausentes (Hole) — casa apenas os args presentes, extrai tipos esperados
  para posições ausentes do overload único casado
- Hint top-down (`hint: Option<&Ty>`) em `infer_expr` — ascription propaga
  tipo esperado para dentro de lambdas; `infer_lambda` extrai tipos dos
  parâmetros do hint quando é `Ty::Function`
- Holes com ascription (`_::Int`) — o tipo anotado resolve o hole sem
  partial dispatch; desambigua overloads quando combined com partial dispatch
- `LambdaInferenceFail` — erro distinto quando nenhum mecanismo fornece
  tipos dos parâmetros (não `NoOverload` opaco)
- Apply de lambda inline — args fornecem tipos dos parâmetros do lambda
  (síntese bottom-up: `42 → Int` define `x: Int` no escopo do lambda)

**Features:**
- Assinaturas (`nome :: T1 T2 => TRet`)
- Tipo de função como valor (`(A -> B)`)
- `lambda`/`λ`, múltiplas cláusulas
- Guards (`> x 0: ...`, `otherwise: ...`)
- `match` com pattern matching e verificação de exaustividade
- Patterns: Ident, Wildcard, Literal, Variant, Tuple, Cons
- `with` block (computações prévias para guards, restrições de genéricos)
- Pipeline `|>` (desugar no typeck)
- Hole `_` (currying explícito: `+ 10 _`, `- _ 10`, `+ _ _`)
- TCO delegado ao Cranelift (`tail_pos: bool` na TAST)

**Depende de:** Fio 1

**DoD:** Fatorial recursivo executa sem stack overflow (TCO via Cranelift).
Match exaustivo em Boolean funciona. Guards com `otherwise` validam.
`let soma_dez := + 10 _` gera closure de aridade 1 (partial dispatch
resolve tipo do hole). `(lambda x: + x 1)::(Int -> Int)` extrai tipos dos
params do hint. `LambdaInferenceFail` quando nenhum mecanismo resolve.

---

### Fio 3: Actions, return, ;, Caller's Arena

**Status:** Fase 1-3 COMPLETO (427 testes, 5 exemplos E2E). Pool de arenas
real implementado — `Vec<Arena>` thread-local, handles indexados,
`arena_destroy(handle)` reseta SÓ a arena do handle. Modelo D: ActionCall
passa `caller_arena` em tail_pos, `local_arena` em `;`. Fase 4 ✅
(loop, break, continue implementados). Fase 5 ✅
(Sum com payload, 8 testes E2E). Fase 6 ✅
(Result/Optional genéricos no prelude, 8 testes E2E). Fase 7 ✅
(? fail-fast, 9 testes E2E). Fase 8 ✅
(| fallback, 9 testes E2E). Fase 9 ✅
(panic!, assert!, 4 testes E2E). Fase 10 ✅
(fibers, scheduler, 8 testes E2E). Fase 11 ✅
**(proibição de recursão em Actions, 8 testes E2E). **Fase 12 ✅**
**(closures com captura, wrapper-only, 10 testes E2E). **Fase 13
ELIMINADA** (escape analysis cancelada — wrapper-only: toda closure
com captures aloca CaptureBox via `kata_rt_alloc_arc`). **Fase 14 ✅**
(Arc<T>: `alloc_arc`/`incref`/`decref` + testes E2E avançados —
closure aninhada, closure em tupla, closure com Float, 500 testes).
**Fase 15 TRANSFERIDA** para o Pré-11 (ARC pass depende de EscapeTarget
e árvore hierárquica de arenas). **Fase 16 ✅** (TRMA pass —
`@associative(0)` em `+` habilita reescrita com acumulador, 5 testes E2E,
505 testes no workspace).

**Maquinaria de tipos construída:**
- Verificação de proibição de recursão em Actions (call graph analysis)
- `?` desugar: injeta `return Err(e)` na TAST

**Features:**
- `action` declaração, `!` sufixo de chamada
- `var` (binding mutável, exclusivo de Actions)
- `loop`, `break`, `continue` (`for` adiada para Fio 7+8)
- `return` (early return explícito, caller's arena)
- `;` (terminador de statement, retorno implícito vs Unit)
- `?` (fail-fast, injeta `return Err(e)`)
- Proibição de recursão em Actions (hard error)
- Arena per-fiber (bump allocator, liberação O(1))
- Scheduler básico (single fiber, wasmtime-fiber)

**Runtime:**
- `kata_rt_arena_create`, `kata_rt_arena_alloc`, `kata_rt_arena_destroy`
- wasmtime-fiber integration (yield em operações bloqueantes)

**Depende de:** Fio 1, 2

**DoD:** Action com `loop`, `var`, `break` executa. Action retorna coleção sem
use-after-free (caller's arena). `?` propaga erro corretamente.

---

### Fio 4: Enum Avançado — Payload, Result, Optional, | ✅ Concluído

**Status:** Implementado como fases do Fio 3 (commits `13787c3` Fase 5 — Sum
com payload, `d4386ea` Fase 6+7 — Generics de Enum + ? fail-fast, `0752bc5`
Fase 8+9 — | fallback + panic!/assert!). DoD satisfeito: variantes com
payload, `Result::(T, E)`, `Optional::T`, `|`, `?`, `panic!`, `assert!`,
match general case. Fio 8 e Fio 6 já consumiram as dependências deste fio.

**Maquinaria de tipos construída:**
- `Ty::Sum` com variantes que carregam payload (já existe para unitárias de
  Fio 1; agora variantes com `Ok(T)`, `Some(T)`)
- `::` em type params (`Result::(T, E)`) — typeck resolve params posicionais
- Match general case (3+ variantes com switch/branch chain) — match em 2
  variantes já funciona de Fio 2; general case é 3+
- (Smart constructor synthesis falível adiado para Fio 6)

**Features:**
- Variantes com payload (`Ok(T)`, `Some(T)`, `Aprovada(Valor)`)
- Sum com payload = sempre ponteiro (invariante de codegen)
- `Result::(T, E)`, `Optional::T` (definidos no prelude)
- `|` (fallback local, generalizado para qualquer enum com payload)
- (Variantes predicadas adiadas para Fio 6)
- Match general case (3+ variantes com switch/branch chain)
- `panic!`, `assert!`

**Runtime:**
- `kata_rt_store_sum_result`, `kata_rt_tag_int`

**Depende de:** Fio 2 (match, Hole), Fio 3 (Actions para `?`)

**DoD:** `Result` com `|` e `?` funciona. Match em 4+ variantes executa sem
trap. (Enum predicado `IMC(17.0)` é Fio 6.)

---

### Fio 5: Data, Structs, Tuples, alias ✅ Concluído

**Maquinaria de tipos construída:**
- `Ty::Struct` com campos tipados
- `Ty::Tuple` com elementos tipados (tamanho estático conhecido)
- `::` em campos de struct (`nome::Text`) — typeck valida
- Smart constructor synthesis (infalível: `Pessoa :: Text Int => Pessoa`)
- Tuple como heap type (invariante de codegen: sempre ponteiro)
- `.N` em tupla (IndexAccess compile-time, bounds check, índice negativo)

**Features:**
- `data` declaração (posicional e indentada)
- Field access (`expr.nome`)
- Tuple (vírgula obrigatória, heap type, `.N` com compile-time bounds check,
  índice negativo `t.(-1)`)
- `alias` (newtype, construtor sintetizado, orphan rule)
- Smart constructors para structs (infalíveis)
- `format` (builtin sintetizado, substitui `{}`)
- `$` spread (interceptado pelo typeck)

**Runtime:**
- Struct/tuple arena alloc + Store por campo/elemento

**Depende de:** Fio 1, 2

**DoD:** Struct com field access funciona. Tuple com `.N` e `t.(-1)` funciona.
`alias` permite implementar interface em tipo externo. `format "{} {}" (a, b)`
interpola.

---

### Fio 6: Tipos Refinados, Ascription

**Maquinaria de tipos construída:**
- `::` em ascription de expressão (`expr::Type`) — typeck valida
- Smart constructor falível (construído neste fio) — usado para refined types
  (`data (Int, > _ 0) as PositiveInt` gera construtor `Int => Result::(T, Error)`)
  e para variantes predicadas de enum (`enum IMC` com `Magreza(< _ 18.5)`)
  — reusa Hole de Fio 2, Result de Fio 4, guard chain de Fio 2
- Avaliação constante de predicados (typeck local, NÃO comptime): substitui `_`
  por literal, reduz expressão booleana, verifica `True`/`False`
- Ret-directed dispatch: hint de retorno na ascription seleciona sobrecarga
  (reusa `hint: Option<&Ty>` de Fio 2 — agora o hint também participa da
  seleção de overload, não apenas da inferência de lambda)
- Grouped (barreira vs strip — `((expr))::Type`)

**Features:**
- `data (Int, > _ 0) as PositiveInt` (predicados com Hole — Hole já existe de
  Fio 2; smart constructor falível é construído neste fio, reusando
  maquinaria de guard chain + Result de Fio 4)
- Smart constructors falíveis para refined types (`Result::(T, Error)`)
- Variantes predicadas de enum (`enum IMC`, smart constructors falíveis —
  movido de Fio 4 para cá)
- Ascription `expr::Type` (compile-time validation para literais)
- Ret-directed dispatch (hint de retorno na ascription)
- Grouped (barreira vs strip — `((expr))::Type`)
- Coerção contextual no `|` (fallback literal validado em compile-time)

**Depende de:** Fio 5 (data, smart constructor infalível), Fio 4 (Result,
Sum com payload), Fio 2 (Hole para predicados, guard chain)

**DoD:** `5::PositiveInt` é `PositiveInt` direto. `(-5)::PositiveInt` é type
error. `PositiveInt 25 | 0` desempacota com fallback validado. `(/ 1 3)::Int`
seleciona idiv por ret-directed dispatch. Enum predicado `IMC(17.0)` despacha
para `Magreza`.

**Nota:** Predicados triviais são avaliação constante local ao typeck.
Predicados complexos (desde Fase 4 Fio 12) são delegados ao comptime pass
(JIT-and-execute).

---

### Fio 7: Interfaces, Generics, Dispatch

**Maquinaria de tipos construída:**
- `Ty::Generic` (type params não-resolvidos)
- `Ty::Interface` (contratos nominais)
- `InterfaceRegistry` (catálogo de interfaces e seus impls)
- Dispatch por dominância com múltiplas overloads (scoring já existe de Fio 1;
  agora múltiplos candidatos competem)
- Monomorphization nos call sites (especializa `List(Int)` vs `List(Text)`)
- `mono_instance: u64` na TAST (monomorph rastreia qual instância cada call
  resolve)
- `@commutative` (dispatch tenta argumentos invertidos ao procurar sobrecargas)
- Unificação `unify` para `Ty::Generic` (type params de generics) — resolve
  substituições top-down nos call sites (não é union-find; é casamento
  posicional param→arg, reusando o padrão de Kata4)
- `InferVar` resolution via dispatch: quando partial dispatch de Fio 2
  encontra `InferVar` numa posição onde o overload espera `Generic(T)`,
  resolve `InferVar := T` via `unify` (extensão do partial dispatch para
  suportar type params genéricos)

**Features:**
- `interface NOME` com `implements SUPERINTERFACE...`
- Interfaces parametrizadas (`ITERABLE(A)`, `INDEXABLE(A)`)
- Multiple dispatch por dominância (múltiplos candidatos, scoring, seleção)
- Interfaces base: `ORD`, `EQ`, `NUM`, `SHOW`
- `COUNTABLE` (`len :: Self => Int`)
- `INDEXABLE(A)` (`at :: Self Int => Result::(A, Err)`)
- Generics (parametric polymorphism)
- Monomorphization nos call sites
- `@commutative` (dispatch tenta argumentos invertidos)

**Depende de:** Fio 4 (enums para variantes de interface), Fio 5 (data/struct
para tipos concretos)

**DoD:** `Int implements NUM` com `+`, `-`, `*`, `abs`, `div`. `List(A)
implements ITERABLE(A)` despacha corretamente. Monomorphization especializa
`List(Int)` vs `List(Text)`. Dispatch com 2+ candidatos seleciona por dominância.

---

### Fio 8: Coleções, ITERABLE, Stream Fusion

**Status:** Fases 1-9 ✅ (DoDs 1-60, 788 testes, 0 falhas, 3 ignored).
Runtime + codegen de coleções implementados (List/Array/Range, ForIn, `in`,
Pattern Cons, map/filter/fold, stream fusion). HEAD `9eea5d8`. Fio 8 Concluído.

**Maquinaria de tipos construída:**
- `.N` em coleções (desugar para `at` via INDEXABLE, retorna `Result`)
- `len` (síntese compile-time para Tuple — special case; COUNTABLE dispatch para
  coleções)
- `@builtin("map"/"filter"/"fold")` — typeck gera nós TAST especializados

**Features:**
- List `[T]` (Cons, pattern `[h : t]`, `[]`)
- Array `{T}` (contíguo, imutável)
- Range `[a..b]`, `[a..=b]`, `[a..step..b]` (lazy, ITERABLE)
- `map`, `filter`, `fold` (`@builtin`, nós TAST especializados)
- Stream fusion (Map/Filter aninhados → único loop)
- `.N` em coleções (desugar para `at`, retorna `Result`)
- `len` (COUNTABLE dispatch para coleções, síntese para Tuple)
- `for x in colecao` (iteração via ITERABLE — movido de Fio 3 para cá, onde
  ITERABLE e coleções existem)

**Runtime:**
- `kata_rt_list_nil/cons/is_empty/head/tail`
- `kata_rt_array_alloc/len/get/set`
- `kata_rt_array_get_checked` (retorna Result)
- `kata_rt_string_len`, `kata_rt_string_get_checked`

**Depende de:** Fio 7 (ITERABLE, INDEXABLE, COUNTABLE), Fio 5 (Tuple para
special case de `len` e `.N`)

**DoD:** `map (+ 10 _) [1 2 3]` produz `[11 12 13]`. `filter (> _ 5) {1 8 3 9}`
produz `{8 9}`. `arr.0 ?` desempacota. `len (10, 20)` é `2` (compile-time).
`for x in {1 2 3 4 5}` itera via ITERABLE.

---

### Fio 9: Closures, Escape Analysis, ARC, TRMA

**Maquinaria de tipos construída:**
- `escape: EscapeKind` na TAST (NãoEscapa / EscapaParaHeap / EscapaParaClosure)
- `capture: Vec<CaptureInfo>` na TAST (o que esta lambda captura e como)
- `CaptureStorage` Stack/Heap (promoção quando closure escapa)
- Escape analysis em 4 passes
- Capture analysis (`collect_captures`): coleta free variables do body do
  lambda contra o escopo externo (reusa padrão de Kata4 — `apply_lambda.rs`
  e `lambda.rs` fazem coleta após inferir o body)
- `Pattern::Ident` com anotação de tipo (`lambda x::Int: ...`) — extensão
  do AST e parser para permitir anotação de tipo em parâmetros de pattern;
  alternativa ao hint top-down e partial dispatch para resolver tipos de
  parâmetros (Fio 2 usa ascription no lambda inteiro; Fio 9 pode trazer
  anotação por parâmetro se a DX justificar)

**Features:**
- Closures com captura léxica (Hole desugar já existe de Fio 2; agora captura
  variáveis externas)
- Escape analysis (4 passes, CaptureStorage Stack/Heap)
- `Arc<T>` nativo para closures que escapam
- `FnValueCall` (chamada a closure escapada via `call_indirect`)
- `@associative(0)` → TRMA (reescrita com acumulador no TAST)

**Runtime:**
- `kata_rt_alloc_arc`, `kata_rt_incref`, `kata_rt_decref`
- Layout ARC: `Arc<ClosureBox { fn_ptr, captures }>`

**Depende de:** Fio 2 (lambdas, Hole), Fio 3 (Actions para fork/escape context)

**DoD:** `let add_n := + _ n` captura `n`. Closure retornada por função pura
escapa para heap (Arc). TRMA converte fatorial com `@associative(0)` em
recursão de cauda.

---

### Fio 10: Módulos, Prelude, FFI Completo

**Features:**
- `import`, `export`, `as`
- Module loader (filesystem, cache, cycle detection)
- Path resolution (file direto + agregador de diretório `mod.kata`)
- Prelude de `stdlib/core.kata` (substitui prelude hardcoded)
- Reexportação (`export MOD.(itens)`)
- **`Complex`** — tipo numérico implementado inteiramente em Kata, sem `@ffi`.
  Demonstra que o princípio "sem builtins" funciona: `data Complex (re::Float,
  im::Float)` com `implements NUM/ORD/EQ/SHOW` usando lambdas puras. É o exemplo
  canônico de que o compilador não precisa saber que um tipo existe para que ele
  funcione — `Complex` é um tipo como qualquer outro que o usuário poderia
  definir. Vive em `stdlib/complex.kata` como módulo da stdlib.

**Depende de:** Fio 1-9 (prelude precisa de todas as features implementadas)

**DoD:** `import utilidades.matematica` carrega de filesystem. Ciclo de imports
detectado. Prelude carregado de `stdlib/core.kata`. Hardcoded prelude removido.
`Complex` funciona com `+`, `show`, e comparação — sem `@ffi`, sem tratamento
especial no compilador.

---

### Pré-11: Infraestrutura de Memória Hierárquica ✅

**PRD:** `docs/PRD-pre-11.md`

**Problema:** Todo objeto que escapa do fiber (CaptureBox, Sum results,
tuplas em função pura) cai na arena global (handle 0), que nunca é
destruída — vazamento permanente. O refcount (`incref`/`decref`) é
registrado no FFI mas nunca emitido pelo codegen. A análise de escape
(`tail_pos`) é binária e não sabe classificar o destino do escape.

**Solução:** Árvore hierárquica de arenas — cada fiber tem sua arena e
acesso às arenas dos ancestrais. Objetos que precisam ser compartilhados
são alocados na arena do LCA via escape analysis em compile-time. A
árvore garante a segurança: um pai só é destruído quando todos os filhos
terminaram.

**Features:**
- Scheduler rastreia árvore de fibers (parent_id, children, completed)
- Destruição bottom-up (arena do pai sobrevive até filhos terminarem)
- Arena raiz criada no scheduler_init, destruída no fim do run
- `EscapeTarget` na TAST (Local / Caller) coexiste com `tail_pos`
  (`tail_pos` governa TCO, `escape` governa arena selection).
  `Ancestor(n)` foi removido — era código morto forward-looking para LCA
  que nunca foi implementado. Será re-adicionado quando o LCA real existir.
- ARC pass emitido pelo codegen (`incref`/`decref` nos pontos apropriados)
- CaptureBox e Sum results alocam na arena do escape target, não hardcoded em handle 0

**Depende de:** Fio 3 (Actions, arena, scheduler), Fio 9 (escape analysis, CaptureBox)

**DoD:** Arena raiz destruída no fim do scheduler run (zero vazamento).
Destruição bottom-up garantida. `EscapeTarget` anotado na TAST e usado
pelo codegen. ARC pass emitido.

---

### Fio 11: CSP, Concorrência, Paralelismo ✅ Concluído

**Status:** Fases 1-14 implementadas (Fases 1-14 do Fio 3, que incluem CSP).
`channel!`, `queue!(N)`, `broadcast!()`, `fork!`, `!>`, `<!`, `select` com
`timeout`, yield cooperativo, structured concurrency, scheduler com fibers.
Testes E2E: `csp_channels_e2e.rs` (11 testes), `csp_broadcast_e2e.rs` (4 testes),
`select_timeout_e2e.rs`, `scheduler_test.rs`. Exemplos: `broadcast.kata`,
`select_queue.kata`.

**spawn! (Fase 9) ✅:** Multiprocess via fork+IPC. `spawn!(action, (args))`
executa a Action em processo OS separado via `fork()`. O child herda a arena
via copy-on-write, executa a Action, serializa o resultado via `to_bytes()`,
e envia pelo pipe. O parent faz `yield`, lê o pipe, e desserializa via
`from_bytes()`. `TypedExprKind::Spawn` na inference, `lower_spawn` no codegen,
`kata_rt_spawn_process` no runtime. Type table registrada pelo driver antes
do JIT (`build_and_register_type_table` → `type_id_map: HashMap<Ty, i64>`
propagado via `LowerCtx`). 3 testes E2E em `spawn_e2e.rs`: básico (Int),
tupla (args compostos), sem args (Unit).

**Maquinaria de tipos construída:**
- `Ty::Sender(Box<Ty>)`, `Ty::Receiver(Box<Ty>)`, `Ty::ReceiverFactory(Box<Ty>)`
- `TypedExprKind::ChannelSend`, `ChannelRecv`, `Fork`, `ChannelCreate`
- `ChannelKind`: Rendezvous, Buffered(N), Broadcast
- Escape analysis para LCA entre fibers que compartilham canais
- `EscapeTarget::Heap` para valores enviados via canal (root_arena)

**Features:**
- `channel!()` (rendezvous), `queue!(N)` (buffered), `broadcast!()` (pub-sub)
- Criação retorna tupla `(Sender::T, Receiver::T)` ou `(Sender::T, ReceiverFactory::T)`
- `fork!()` (submete Action ao scheduler com args)
- `select` com `timeout` (multiplexação de canais)
- `!>` (envio), `<!` (recebimento) — operadores infixos
- Yield cooperativo via `wasmtime-fiber::Suspend` com `YieldReason`
- Yield points no codegen (back-edge checks em `Loop` e `ForIn`)
- Structured concurrency (Action espera forks completarem)
- Deadlock detection trivial no run loop

**Runtime:**
- `kata_rt_channel_create/send/recv`, `kata_rt_queue_*`, `kata_rt_broadcast_*`
- `kata_rt_select`, `kata_rt_yield`, `kata_rt_yield_check`
- `kata_rt_broadcast_receiver_create` (receiver factory)
- Scheduler: run_queue, blocked, pending_wakes, timers, árvore de fibers

**Depende de:** Pré-11 ✅ (árvore de arenas, escape analysis para LCA),
Fio 3 ✅ (Actions, arena), Fio 9 ✅ (escape analysis para dados em canais → Arc<T>)

**DoD:** ✅ `fork!` submete Action em fiber separada com args. Channel rendezvous
sincroniza sender/receiver. `select` multiplexa 2+ receivers. Yield points
previnem head-of-line blocking. Structured concurrency garante lifecycle.
`spawn!` não implementado (redesign aprovado, Fase 9 do PRD-fio11).

---

### Fio 12: Comptime, `@cache` ✅ Concluído

**Status:** Fases 1-6 ✅ + constant folding de funções com args literais (Ponto 7).
1239 testes no workspace, 0 falhas. Commits: `8c3d299` (Fase 1), `b7f7485`/`b75f867`
(Fase 2), `718f74e`/`138af02`/`d2dc096`/`377c7b3` (Fase 3), `08d0c8f` (Fase 4),
`519a539` (Fase 5), `e5c89b5` (Fase 6), `2289ed6` (Ponto 7 — constant folding).

**PRD:** `docs/PRD-fio12-comptime.md`

**Maquinaria de tipos construída:**
- `TypedExprKind::HeapSnapshot { snapshot_id, ty }` — resultado de comptime
  embedado na TAST
- `HeapSnapshotData { bytes, rebase_offsets, ty }` — tabela de snapshots por módulo
- Comptime pass: dataflow de constness, pureza verification, JIT-and-execute
  em arena temporária dedicada
- `@cache{strategy: "LRU"}` — cache hashmap em `caller_arena`, key via TypeShape

**Features:**
- `@comptime` em `let` top-level, expressão top-level, e call-site dentro de body
- JIT-and-execute (compila via pipeline normal, executa no `kata-rt` real)
- HeapSnapshot (bytes + rebasing na arena contígua — Pré-11 torna trivial)
- Constness binário: literal, resultado `@comptime`, `let` comptime-available,
  definição de função do módulo. O resto não. Sem propagação automática.
- `@comptime` é explícito e opt-in (estilo Zig, não D). Sem definition-site hint.
- Tipos preservados exactamente — snapshot tem o mesmo `ty` que a expressão
- Ascription refined delega predicados complexos ao comptime pass (quando valor
  é comptime-available). Comportamento do Fio 6 preservado para predicados triviais
- `@cache{strategy: "LRU"}` anota a definição. Codegen emite cache lookup no
  prólogo (antes da primeira cláusula, antes do primeiro lambda) e insert no
  epílogo. Cache lazy-allocated em `caller_arena`. Sem reescrita de TAST.
- Pureza verification (walk TAST: se contém ActionCall, é impura → erro)
- Constant folding de funções com args literais (Ponto 7): o comptime pass
  percorre a TAST procurando `Closure` com callee `Ident` (função pura nomeada)
  e todos os args literais. JIT-executa e substitui por literal/snapshot.
  Bottom-up com fixpoint — folds em cascade. Não dobra construtores falíveis
  (Result) nem funções FFI.

**Runtime:**
- `kata_rt_load_snapshots(root_arena, snapshot_table, n)` — load-time, memcpy + rebasing
- `kata_rt_cache_get_or_create(arena, fn_id, capacity) -> handle`
- `kata_rt_cache_lookup(handle, key_bytes, key_len) -> i64` (0=miss, ptr=hit)
- `kata_rt_cache_insert(handle, key_bytes, key_len, value_ptr)`

**Depende de:** Fio 1-10 ✅ (pipeline completo, módulos, arenas hierárquicas),
Fio 11 ✅ (TypeShape para serialização de args em `@cache`)

**DoD:** ✅ `@comptime fatorial 10` substitui por literal `3628800`.
`@comptime range 1 100` substitui por HeapSnapshot. `@comptime` com arg
não-constante → erro. `5::Prime` com predicado complexo valida em compile-time.
`dobro :: Int => Int @cache{strategy: "LRU"}` memoiza função pura repetida.
`kata build` produz executável com snapshots embedados. Constant folding
dobra `dobro 5` → `10` literal sem `@comptime` explícito.

---

### Fio 13: Dict, Set (HAMT) ✅ Concluído

**Status:** Implementado. HAMT no runtime (`kata-rt/src/dict/hamt.rs`,
`kata-rt/src/set.rs`), parser, codegen (`dict_set_lit.rs`), inference
(`dict_set.rs`). 16 testes E2E (10 Dict + 6 Set), todos passando.

**Features:**
- `Dict::(K, V)` — dicionário persistente imutável (HAMT)
- `Set::T` — conjunto persistente imutável (HAMT)
- `Dict implements ITERABLE((K, V)), COUNTABLE, INDEXABLE(V)`
- `Set implements ITERABLE(T), COUNTABLE`
- `at dict key` → `Result::(V, Err)` (lookup por chave)
- `contains set elem` → `Boolean`

**Runtime:**
- HAMT implementation (hash, trie nodes, bitmap, persistent sharing)
- `kata_rt_dict_empty/insert/get_checked/contains/len/remove/next/merge`
- `kata_rt_set_empty/insert/contains/len/remove/next/union/intersection/difference`

**Depende de:** Fio 7 ✅ (interfaces), Fio 8 ✅ (ITERABLE/COUNTABLE/INDEXABLE)

**DoD:** ✅ `Dict::(Text, Int)` com insert/get funciona. `Set::Int` com
contains/union/intersection funciona. Iteração via ITERABLE produz pares.

---

### Fio 14: @log, @test, Test Runner

**Status:** `@test` ✅ Concluído (Fases 1-7, 899 testes). `@log` ✅ Concluído (Fases 1-7, 14 testes E2E).

**Features:**
- `@log{level, msg, topic, policy}` (telemetria via canais CSP) — ✅
  - Política `"drop"` (descarta se sobrecarregado)
  - Política `"block"` (bloqueia até confirmação)
- `log!()` action nativa (posicional) — ✅
- `log_recv!()` consume telemetria — ✅
- `log_config!()` herança via snapshot no spawn — ✅
- `@test("descrição")` (teste positivo) — ✅
- `@test{desc, expects: "CompileError"}` (teste negativo) — ✅
- `kata test` (test runner: descobre @test, executa em JIT isolado) — ✅
- Tree shaking de `@test` em produção — ✅ (Fio 15)

**Depende de:** Fio 11 (CSP para @log), Fio 4 (Result para testes)

**DoD:** `kata test examples/test_assert.kata` roda testes e reporta
pass/fail/error. Teste negativo que falha compilação = PASS. @log envia
telemetria sem contaminar pureza.

---

### Fio 15: AOT, REPL

**Status:** Concluído (Fases 1-8 ✅)

**Features:**
- `kata build` (AOT: Cranelift object file + linker → executável) ✅
  - `cranelift-object` para emissão de `.o`
  - Link com `kata-rt` estático (`libkata_rt.a`) ou dinâmico (`--dynamic`, `libkata_rt.so`)
  - Tree shaking incondicional (sem `--release`)
  - Shim C trivial que chama `__kata_entry` + `kata_rt_print_result` (display no runtime)
- `kata repl` (REPL interativo) ✅
  - Persistência de `TypeEnv` entre expressões (via items acumulados)
  - Histórico persistente (rustyline, `~/.kata_repl_history`)
  - Comandos: `:help`, `:type <expr>`, `:env`, `:load <file>`, `:reset`, `:quit`
  - Multiline: Sig+lambda, action, match, enum, interface, implements
  - Rollback em caso de erro (item removido, sessão continua)
  - 31 testes E2E

**Depende de:** Fio 1-14 (todas as features precisam funcionar em AOT)

**DoD:** `kata build examples/fatorial.kata` produz executável nativo que
executa sem o compilador. `kata repl` mantém bindings entre expressões.

**Progresso (2026-07-30):**
- Fase 1 ✅ — `ModuleBackend` trait + `JitBackend`
- Fase 2 ✅ — `AotBackend` + `aot_emit`
- Fase 3 ✅ — `kata-tree-shaking` crate
- Fase 4 ✅ — `kata-rt` staticlib/cdylib + linker (shim C + `cc`)
- Fase 5 ✅ — `kata build` subcomando + pipeline completo + 8 testes E2E
- Fase 6 ✅ — REPL: TypeEnv persistente + `:type` + `:env` + `:load` + `:reset`
- Fase 7 ✅ — Multiline (match, enum, interface, implements, Sig+lambda, action)
- Fase 8 ✅ — Documentação (Manual §26: arquitetura, comandos, multiline, :load, :type, :env, :reset, erros, histórico)

---

## Resumo Visual

```
Fio 1  ──────────────────────────────────────────────── CLI day-1
  │       TypeEnv, Ty, PrimTy (FFI), DispatchTable com scoring, FfiSymbol,
  │       ::, data opaco, enum unitário (Boolean), Int/BigInt/SMI, Rational
  ├── Fio 2 ── Fio 9 (closures, escape)
  │       assinaturas, ->, Hole, tail_pos             escape, capture, Arc, TRMA
  ├── Fio 3 ── Pré-11 ── Fio 11 ✅ ── Fio 14 ✅ (@log, @test)
  │       Actions, return, ;, ?   Memória hierárquica  CSP, yield points
  ├── Fio 4 ── Fio 8 ✅ ── Fio 13 ✅ (Dict/Set HAMT)
  │       Ty::Sum payload, :: params          ITERABLE, .N, len, stream fusion
  ├── Fio 5 ✅ ── Fio 6 ✅ (refined)
  │       Ty::Struct/Tuple, :: campos         :: ascription, avaliação constante
  ├── Fio 7 ✅ ── Fio 8 ✅ (dependência)
  │       Ty::Generic/Interface, monomorph    (desbloqueia coleções)
  ├── Fio 10 ✅ (módulos)
  ├── Fio 12 ✅ (Comptime, @cache)
  │       @comptime call-site, HeapSnapshot, @cache LRU, constant folding
  └── Fio 15 ✅ (AOT, REPL)

Zeladorias removidas — manutenção diária via skill `zeladoria-kata5` substitui zeladorias planejadas.
```

## Princípios do Roadmap

1. **Typeck + codegen no mesmo fio:** se o typeck aprova, o codegen executa.
   Não aprovar no typeck o que o codegen não implementa.
2. **Manual sync como DoD:** cada fio atualiza o manual se a implementação
   divergiu do PRD.
3. **`run` day-1:** disponível desde Fio 1. `test` vem com `@test` (Fio 14).
   `build` e `repl` são Fio 15.
4. **Zeladorias planejadas:** não são falhas do processo, são consequência do
   modelo vertical.
5. **Cross-fio test runner:** cada fio adiciona testes ao suite global.
6. **Sem builtins:** tudo via `@ffi` ou `@builtin`. O compilador conhece apenas
   `FfiSymbol`, 3 strings de mapeamento, e `@builtin`.
7. **Comptime via JIT-and-execute:** não interpretador de TAST.
8. **SMI tagging transparente:** runtime decide representação, compilador vê
   tipo canônico.
9. **`@builtin` como padrão de interceptação:** funções interceptadas pelo
   typeck seguem o padrão (stdlib + diretiva + nó TAST especializado).
10. **Return + caller's arena:** design de Actions desde o início, não
    zeladoria tardia.
11. **Dispatch por dominância nasce em Fio 1:** scoring desde o início, mesmo
    com 1 overload. Não retrofit em Fio 7.
12. **Hole nasce em Fio 2:** currying explícito desde funções/lambdas. Não
    adiar para Fio 9.
13. **Ascription refined — triviais vs complexos:** predicados triviais são
    avaliação constante local ao typeck. Predicados complexos (Fase 4 Fio 12)
    são delegados ao comptime pass (JIT-and-execute).
14. **`::` é um operador, contextos são typeck:** parser reconhece `::` desde
    Fio 1. Contextos (assinatura, campo, type param, variante, ascription) são
    interpretados progressivamente pelo typeck.
15. **Sem primitivos:** `Int`, `Float`, `Text` são `data` opacos com `@ffi` no
    prelude; `Boolean` é `enum` no prelude. O compilador não tem tipos
    primitivos da linguagem — `PrimTy` é mapeamento de representação FFI
    (`i64`, `f64`, `kata_rt_string`), não tipo da linguagem. `enum` básico
    (variantes unitárias) existe em Fio 1 porque o prelude precisa declarar
    `Boolean`.

## Nota: GC / Reclamation para Fibers Long-Lived

O modelo hierárquico de arenas (PRD pré-11) garante que todo objeto tem um
lifetime definido pela árvore de fibers — um pai só é destruído quando todos
os filhos terminaram, então qualquer objeto promovido para o pai está vivo
enquanto algum filho o referencia. Isso cobre o caso geral: fibers que nascem,
comunicam-se, e morrem num ciclo fechado.

O caso **não coberto** é o fiber long-lived — um fiber que permanece vivo
indefinidamente (server, worker pool, event loop) enquanto spawna filhos
curtos. Objetos promovidos para a arena desse fiber acumulam-se sem bound,
porque a arena só seria liberada quando o próprio fiber terminar — o que
não acontece.

Para esse cenário, um mecanismo de reclamation granular é necessário. As
opções incluem GC local à arena (mark-sweep / compactação sobre a arena
do fiber long-lived) ou um allocator separado com free individual para
objetos promovidos. Este mecanismo é **localizado** — só afeta fibers
long-lived, não o modelo geral. Fica fora do escopo do PRD pré-11 e será
abordado quando o Fio 11 introduzir casos reais de fibers long-lived.

## Fora do Escopo 1.0

- Tensor/SIMD
- `@heapstack` (otimização heurística de arena em loops)
- `@restart` (retry policy para Actions)
- Doc comments (`///`, `"""doc"""`)
- **Tuplas variádicas (`T...`)** — sintaxe `Text...` numa tupla indica "pelo menos um
  elemento do tipo precedente" `(Int, Text..., Float)`. Permite que actions recebam
  múltiplos argumentos de tipo heterogêneo com aridade variável. Exige extensão do
  type system (`Ty::Tuple` com "rest element"), parser (`Token::TripleDot`), pattern
  matching (rest binding em tupla), e codegen (loop sobre os elementos rest).