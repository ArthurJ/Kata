# Visão — Tensor no Kata5

**Data:** 2026-08-17
**Status:** Design (não implementado)

## Síntese

Tensor é o tipo de array N-dimensional com dimensionalidade conhecida em
compile-time (Const Generics). É a estrutura para processamento matemático
acelerado — o typeck desbloqueia operações de álgebra linear (`+`, `*`,
`dot`) que o codegen traduz para SIMD no Cranelift sem overhead.

## Sintaxe

```
[1 2 3]          # List (Cons, persistente) — sem ;
[1 2 3; 4 5 6]    # Tensor 2×3 — com ; separando dimensões
[1 2 3;]          # Tensor 1×3 — ; terminal opcional
[1; 2; 3]         # Tensor 3×1 — vetor coluna
```

`;` dentro de `[]` é o discriminador: se há `;` em qualquer posição, é
Tensor. Sem `;`, é List.

`{1 2 3}` continua sendo Array (contíguo, imutável, tamanho dinâmico).
`{"k": v}` continua sendo Dict. `{|1 2 3|}` continua sendo Set.

## Tipo

```
Tensor::T::(Int...)   # ex: Tensor::Int::(2 3) = matriz 2×3 de Int
```

- `T` é o tipo do elemento (deve implementar NUM).
- `(Int...)` é uma tupla de dimensões — cada elemento é o tamanho de uma
  dimensão. Conhecida em compile-time (Const Generics).
- Tensor 0-D (`Tensor::T::()`) representa um escalar — mas `()` é `Unit`
  no Kata5, então a representação de 0-D precisa ser resolvida (ver
  Questões Abertas).

## Distinção Array vs Tensor

| Propriedade | Array `{1 2 3}` | Tensor `[1 2 3;]` |
|---|---|---|
| Sintaxe | `{}` sem `;` | `[]` com `;` |
| Tamanho | Dinâmico (runtime) | Estático (compile-time) |
| Dimensionalidade | 1-D | N-D |
| Álgebra linear | ❌ | ✅ (`+`, `*`, `dot`) |
| SIMD | ❌ | ✅ |
| Tipo | `Array::T` | `Tensor::T::(Int...)` |

Arrays são para I/O de bloco (cache-friendly, tamanho desconhecido).
Tensores são para cálculo matemático (rígido, acelerado).

## Operações

### Aritmética elemento-a-elemento

```
+ :: Tensor::T::(D...) Tensor::T::(D...) => Tensor::T::(D...)
* :: Tensor::T::(D...) Tensor::T::(D...) => Tensor::T::(D...)
```

Requer mesmas dimensões (shape matching em compile-time). `+` é
`@commutative`. Traduzido para SIMD no codegen.

### Álgebra linear

```
dot :: Tensor::T::(D1...) Tensor::T::(D2...) => Tensor::T::(D3...)
```

Produto de matrizes/vetores. As dimensões devem ser compatíveis pela
regra da álgebra linear (inner dimensions must match). O tipo de retorno
tem as dimensões externas. Requer `DOT_BEHAVIOR`.

### Introspecção

```
shape :: Tensor::T::(D...) => Tuple   # retorna (D1, D2, ...)
scalar :: Tensor::T::() => T           # extrai escalar de tensor 0-D
```

### Coerção Array → Tensor (fronteira dinâmica)

```kata
let dados := ler_banco          # Array::Int (tamanho dinâmico)
let tentativa := Tensor::Int::(3 3) dados   # Result — falha se shape não bater
```

A conversão de Array (tamanho dinâmico) para Tensor (tamanho estático) não
é implícita. O construtor `Tensor::T::(dims)` recebe o Array e valida as
dimensões em runtime. Retorna `Result` — falha se o número de elementos
não corresponde ao esperado. Força tratamento de incompatibilidade de
forma pura.

## Codegen

Operações de tensor são traduzidas para instruções SIMD no Cranelift sem
overhead. Como as dimensões são conhecidas em compile-time, o codegen pode:

- Unroll loops completamente
- Usar instruções vectorizadas (AVX/SSE) para operações elemento-a-elemento
- Pré-calcular strides e offsets sem dispatch em runtime

## Questões Abertas

### 1. Shape compatibility do `dot`

`dot` de (1,3)×(3,1) é válido (produto escalar = escalar)? Ou `dot`
exige mesmas dimensões? A spec antiga menciona `DOT_BEHAVIOR` mas não
define as regras. Precisa ser especificado.

### 2. Tensor 0-D e escalar

`scalar :: Tensor::T::() => T` extrai escalar de tensor 0-D. Mas `()`
é `Unit` no Kata5 — `(Int...)` com zero dimensões colide com `Unit`.
Como representar? Opções:
- `Tensor::T` sem tupla de dimensões = 0-D por convenção
- Proibir 0-D (todo tensor é pelo menos 1-D)
- Usar um tipo dedicado para a tupla de dimensões que distingue vazio
  de Unit

### 3. Const Generics no sistema de tipos

`Ty::Generic` existe para enums genéricos, mas Const Generics (inteiros
como parâmetros de tipo) é uma extensão. Como representar `(Int...)` no
`Ty`? Hoje `Ty::Generic(String, Vec<Ty>)` carrega `Ty` — mas dimensões
são `Int`, não `Ty`. Precisa de:
- Ou aceitar `Ty::Prim(Int)` como argumento de `Generic` e tratar
  como dimensão
- Ou criar `Ty::Tensor(Box<Ty>, Vec<usize>)` dedicado

### 4. `;` terminal

`[1 2 3;]` com `;` terminal — é tensor 1×3? Pela regra "se há `;` em
qualquer posição dentro de `[]`, é tensor", sim. Confirmar que o `;`
terminal é legal e opcional: `[1 2 3; 4 5 6]` e `[1 2 3; 4 5 6;]` são a
mesma matriz 2×3.

### 5. Indexação N-D

`t.0` hoje é indexação 1-D (Tuple/Array). Para tensor N-D, como acessar
elementos? `t.(0 1)` (tupla de índices)? `t.0.1` (encadeado)? Precisa ser
definido.

## Referências históricas

- `docs/Kata-lang Specs OLD.md` §1.2 (Interface Tensor) e §2.4 (Tensores
  Estáticos) — spec original
- `examples/legacy/test_tensor_math.kata` — exemplo legacy (não compila
  hoje — parser não aceita `;` dentro de `[]`)
- `examples/legacy/test_tensor_boundary.kata` — exemplo de coerção
  Array→Tensor (não compila hoje)
- `docs/Kata-lang-manual.md:1977` — menção a tensores com `;` em `{}`
  (migrado para `[]` nesta visão)