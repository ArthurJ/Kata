# PRD: Fio 1 — Fundação + Aritmética + CLI

## Objetivo

Tracer bullet. Estabelecer o pipeline end-to-end mínimo: source → lexer →
parser → resolution → inference → codegen → CLIF → Cranelift JIT → runtime →
resultado. Todo o esqueleto do compilador nasce aqui — todas as crates são
criadas, mesmo que com funcionalidade mínima.

## Escopo

### Tipos

Cinco tipos no prelude, todos declarados em Kata (sem primitivos do compilador):

```kata
# Tipos opacos (anchored via @ffi)
@ffi("i64")
data Int ()

@ffi("f64")
data Float ()

@ffi("kata_rt_string")
data Text ()

@ffi("kata_rt_rat")
data Rational ()

# Boolean
enum Boolean
    False
    True
```

`Int` é BigInt com SMI tagging no runtime desde o início. O compilador vê `i64`
em todo o pipeline; o runtime decide representação (SMI inline ou heap BigInt).
`PrimTy` em `kata-core` é o mapeamento de representação FFI (`i64`, `f64`,
`kata_rt_string`, `kata_rt_rat`), não tipo da linguagem.

### Operações

Operadores aritméticos e de comparação, definidos no prelude via `@ffi`:

```kata
# Aritmética Int (BigInt/SMI no runtime)
@ffi("kata_rt_bi_add")
@associative(0)
+ :: Int Int => Int

@ffi("kata_rt_bi_sub")
- :: Int Int => Int

@ffi("kata_rt_bi_mul")
@associative(1)
* :: Int Int => Int

@ffi("kata_rt_bi_div")
/ :: Int Int => Int

# Comparação Int
@ffi("kata_rt_bi_eq")
= :: Int Int => Boolean

@ffi("kata_rt_bi_lt")
< :: Int Int => Boolean

@ffi("kata_rt_bi_gt")
> :: Int Int => Boolean

# Aritmética Float
@ffi("kata_rt_fadd")
+ :: Float Float => Float

# ... etc

# Aritmética Rational
@ffi("kata_rt_rat_add")
@associative(0)
+ :: Rational Rational => Rational

# ... etc

# Conversões
@ffi("kata_rt_rat_to_float")
to_float :: Rational => Float

@ffi("kata_rt_rat_from_float")
from_float :: Float => Rational

@ffi("kata_rt_bi_to_rational")
from_int :: Int => Rational

# I/O
@ffi("kata_rt_print")
echo :: (Text) => Unit
```

### Sintaxe

- Literais: Int (decimal, hex, oct, bin, separador `_`), Float (decimal,
  científico), Text (dupla, simples, tripla crua), Rational (`3.14::Rational`)
- `let` bindings (`let nome := expr`)
- Aplicação prefixa (`+ 1 2`)
- `@ffi("simbolo")` directive (parser + codegen import)
- `data` (tipos opacos sem campos: `data Int ()`)
- `enum` básico (variantes unitárias: `enum Boolean { True, False }`)
- `::` em assinatura (`+ :: Int Int => Int`)
- `::` em qualificação de variante (`Boolean::True`)
- Assinaturas de função (`nome :: T1 T2 => TRet`)
- `@associative(neutro)` directive (parser reconhece, typeck registra para TRMA futuro)

### CLI

- `kata lex <arquivo.kata>` — imprime tokens com spans
- `kata parse <arquivo.kata>` — imprime AST via Debug pretty-print
- `kata eval '<expr>'` — avalia expressão via JIT, imprime resultado
- `kata run <arquivo.kata>` — compila e executa arquivo via JIT

## Crates Criadas

```
kata-core/           Ty, PrimTy, TypeEnv, FfiSymbol, TypeShape, type_id
kata-ast/            AST de dados puros (Expr, Span, Spanned, Pattern, TypeExpr)
kata-lexer/          Lexer indent-sensitive
kata-parser/         Recursive-descent prefix-only
kata-diagnostics/    Erros estruturados (submódulo frontend)
kata-resolution/     Pass 0+1: TypeEnv, assinaturas, smart constructors
kata-inference/      Pass 2: type-check, inferência, dispatch
kata-codegen/        Lowering TAST→CLIF + MetadataTable + emit
kata-optimizer/      (stub — TRMA/StreamFusion/ARC pass em fios posteriores)
kata-rt/             Runtime: BigInt/SMI, Float, Rational, Text, arena, print
kata-driver/         CLI: lex, parse, eval, run
```

## Maquinaria de Tipos Construída

### kata-core

- `Ty` canônico: `Prim(Int|Float|Text|Rational)`, `Unit`, `Struct`, `Sum`,
  `Function`, `InferVar`
- `PrimTy` enum: `Int`, `Float`, `Text`, `Rational` — mapeamento de
  representação FFI (`i64`, `f64`, `kata_rt_string`, `kata_rt_rat`). Método
  `from_ffi(&str) -> Option<PrimTy>` e `to_repr() -> PrimitiveRepr`.
- `TypeEnv`: árvore de escopos (parent + bindings). `lookup(name)`,
  `define(name, ty)`, `push_scope()`, `pop_scope()`.
- `FfiSymbol` enum tipado: cada variante carrega `symbol_name()`, `return_type()`.
  Substitui strings soltas — se você errar o símbolo, é erro de compilação do
  compilador, não bug silencioso.
- `TypeShape`: projeção runtime de `Ty` para reflexão estrutural. `is_heap_type()`
  retorna `true` para `Text`, `Rational`, `Struct`, `Sum`, `Tuple`.
- `type_id`: `u32` atribuído em compile-time para cada `Ty` distinto.

### kata-resolution

- Pass 0: popula `TypeEnv` com tipos declarados (`data` → `Struct`, `enum` →
  `Sum`)
- Pass 1: coleta assinaturas de funções `@ffi` e registra no `DispatchTable`
- `ResolvedModule`: artefato imutável produzido

### kata-inference

- Pass 2: type-check de expressões
- Inferência: literal `42` → `Int`, `3.14` → `Float`, `"hello"` → `Text`,
  `3.14::Rational` → `Rational`
- `DispatchTable` com scoring por dominância — nasce com scoring, mesmo com 1
  overload. Algoritmo: coletar candidatos por nome, pontuar por compatibilidade
  de tipos de argumentos, selecionar o de maior score. Se empate →
  `AmbiguousDispatch`.
- `TypedModule` (TAST): artefato produzido com `ty` em cada nó
- TAST enriquecida: `tail_pos: bool` e `effect: Effect` em cada `TypedExpr`
  (ambos inicializados — `tail_pos` marcado, `effect = Puro`)

### kata-codegen

- Lowering TAST → CLIF direto (sem IR intermediária)
- Block arguments nativos (Cranelift 0.133)
- `MetadataTable` sidecar: `inst_origins`, `block_origins`, `value_types`,
  `closure_info` (vazio por enquanto), `escape_flags` (vazio por enquanto)
- Emit: tradução CLIF → código nativo via Cranelift JIT

### kata-rt

- **BigInt/SMI tagging**: `kata_rt_bi_add`, `kata_rt_bi_sub`, `kata_rt_bi_mul`,
  `kata_rt_bi_div`, `kata_rt_bi_eq`, `kata_rt_bi_neq`, `kata_rt_bi_lt`,
  `kata_rt_bi_le`, `kata_rt_bi_gt`, `kata_rt_bi_ge`, `kata_rt_bi_show`,
  `kata_rt_bi_to_rational`, `kata_rt_tag_int`
- **Float**: `kata_rt_fadd`, `kata_rt_fsub`, `kata_rt_fmul`, `kata_rt_fdiv`,
  `kata_rt_fcmp_eq`, `kata_rt_fcmp_neq`, `kata_rt_fcmp_lt`, `kata_rt_fcmp_le`,
  `kata_rt_fcmp_gt`, `kata_rt_fcmp_ge`
- **Rational**: `kata_rt_rat_add`, `kata_rt_rat_sub`, `kata_rt_rat_mul`,
  `kata_rt_rat_div`, `kata_rt_rat_eq`, `kata_rt_rat_neq`, `kata_rt_rat_lt`,
  `kata_rt_rat_le`, `kata_rt_rat_gt`, `kata_rt_rat_ge`, `kata_rt_rat_show`,
  `kata_rt_rat_literal`, `kata_rt_rat_to_float`, `kata_rt_rat_from_float`,
  `kata_rt_int_to_rational`
- **Text**: `kata_rt_string_concat`, `kata_rt_string_len`, `kata_rt_text_literal`,
  `kata_rt_int_to_text`, `kata_rt_bool_to_text`
- **I/O**: `kata_rt_print`
- **Arena**: `kata_rt_arena_create`, `kata_rt_arena_alloc`, `kata_rt_arena_destroy`

## Prelude

O prelude de Fio 1 é hardcoded em `kata-rt` (uma string constante que o
`kata-module-loader` injeta). Fio 10 substitui isto por `stdlib/core.kata`
carregado do filesystem.

Conteúdo do prelude hardcoded:
- Declarações `data Int ()`, `data Float ()`, `data Text ()`, `data Rational ()`
  com `@ffi`
- `enum Boolean { False, True }`
- Operadores `+`, `-`, `*`, `/`, `=`, `<`, `>` para Int, Float, Rational via
  `@ffi` + `@associative` onde aplicável
- `to_float`, `from_float`, `from_int` (conversões explícitas)
- `echo` via `@ffi("kata_rt_print")`

## Exemplos

```kata
# examples/arithmetic.kata
+ 1 2

# examples/float.kata
+ 3.14 2.71

# examples/rational.kata
show (1::Rational / 3::Rational)

# examples/boolean.kata
= 1 1

# examples/bigint.kata
* 99999999999999999999 99999999999999999999
```

## Definition of Done

1. ✅ `kata eval '+ 1 2'` imprime `3`
2. ✅ `kata run examples/arithmetic.kata` executa e imprime resultado
3. ✅ `kata eval '* 99999999999999999999 99999999999999999999'` imprime resultado
   correto (BigInt, não overflow) — `9999999999999999999800000000000000000001`
4. ✅ `kata eval '+ 3.14 2.71'` imprime `5.85`
5. ✅ `kata eval 'show (/ 1::Rational 3::Rational)'` imprime `1/3`
6. ✅ `kata eval '= 1 1'` imprime `True`
7. ✅ `kata lex examples/arithmetic.kata` imprime tokens com spans
8. ✅ `kata parse examples/arithmetic.kata` imprime AST
9. ✅ Pipeline completo funciona end-to-end: source → lexer → parser → resolution
   → inference → codegen → CLIF → Cranelift JIT → runtime → resultado
10. ✅ `DispatchTable` faz scoring por dominância (mesmo que só tenha 1 candidato
    por nome)
11. ✅ `Boolean` é um `enum` no prelude, não primitivo do compilador
12. ✅ `Int` é `data Int ()` com `@ffi("i64")` no prelude, não primitivo
13. ✅ SMI tagging funciona no runtime (BigInt para valores grandes, SMI inline
    para pequenos)
14. ✅ `FfiSymbol` enum tipado usado em todo o compilador (sem strings soltas)
15. ✅ Cranelift 0.133 com block arguments nativos (sem stack slots)
16. ✅ `MetadataTable` sidecar produzida (inst_origins, block_origins, value_types,
    closure_info vazio, escape_flags vazio)
17. ✅ TAST enriquecida com `tail_pos: bool` e `effect: Effect` em cada nó
18. ✅ Manual atualizado se implementação divergiu do PRD

## Não Inclui

- Funções/lambdas/match/guards (Fio 2)
- Actions/return/`;`/`?` (Fio 3)
- Enums com payload/Result/Optional/`|` (Fio 4)
- Structs com campos/Tuples/alias (Fio 5)
- Tipos refinados/Ascription (Fio 6)
- Interfaces/Generics/Dispatch polimórfico (Fio 7)
- Coleções/ITERABLE/Stream Fusion (Fio 8)
- Closures/Escape Analysis/ARC/TRMA (Fio 9)
- Módulos filesystem/Prelude de arquivo (Fio 10)
- CSP/Scheduler multithread (Fio 11)
- Comptime/@cache_strategy (Fio 12)
- Dict/Set/HAMT (Fio 13)
- @log/@test/Test runner (Fio 14)
- AOT/REPL (Fio 15)

## Arquitetura

```
source string
    │
    ▼
kata-lexer → Vec<(Token, Span)>
    │
    ▼
kata-parser → Spanned<Expr> (AST)
    │
    ▼
kata-module-loader → injeta prelude hardcoded → AST + TypeEnv seeds
    │
    ▼
kata-resolution → ResolvedModule (TypeEnv populado, assinaturas coletadas)
    │
    ▼
kata-inference → TypedModule (TAST com ty em cada nó)
    │
    ▼
kata-codegen (lowering) → cranelift::Function + MetadataTable
    │
    ▼
kata-optimizer → (pass-through neste fio — sem passes ainda)
    │
    ▼
kata-codegen (emit) → Cranelift JIT
    │
    ▼
kata-rt → resultado (i64)
```

## Riscos

1. **Cranelift 0.133 API:** versão nova pode ter mudanças de API vs 0.125.
   Verificar `cranelift-codegen`, `cranelift-jit`, `cranelift-frontend`
   docs antes de começar.

2. **SMI tagging desde o início:** o runtime nasce com BigInt/SMI. Se houver
   bugs na lógica de tagging/promoção, todo o pipeline é afetado. Testar
   isoladamente o runtime antes de integrar com o codegen.

3. **DispatchTable com scoring:** o algoritmo de scoring é o mesmo com 1 ou 100
   overloads, mas precisa ser desenhado corretamente desde o início. Se o
   scoring for trivial (sempre seleciona o único candidato), vai precisar de
   retrofit em Fio 7. Projetar o scoring para lidar com múltiplos candidatos
   desde o início, mesmo que só tenha 1.

4. **Prelude hardcoded:** injetar uma string constante como prelude é
   temporário. O `kata-module-loader` precisa saber injetar tipos no `TypeEnv`
   sem parsear arquivo — isso será substituído em Fio 10 por carregamento de
   `stdlib/core.kata` do filesystem.

5. **TAST enriquecida:** adicionar `tail_pos` desde o início evita
   retrofit. Mas o typeck precisa populá-lo corretamente — `tail_pos` para
   toda expressão.