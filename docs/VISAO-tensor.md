# Visão — Tensor no Kata5

**Data:** 2026-08-17 (unificado de TENSOR-VISAO.md e VISAO-tensor.md)
**Status:** Design (não implementado)
**Motivação:** Kata foi concebida para computação numérica. Operações
matriciais são a razão de existir da linguagem — não um recurso opcional.

---

## 1. Por que Tensores, não Arrays

Arrays são coleções de dados: contíguos, homogêneos, de tamanho dinâmico.
**Não são objetos matemáticos.** Iterar, indexar, contar — isso é comportamento
de coleção. Multiplicar, transpor, contrair — isso é álgebra.

Dar a Arrays operações matemáticas é arbitrário. Qual o significado de
`* {1 2 3} {4 5 6}`? Zip-multiply? Produto escalar? Repetição? Sem um tipo
matemático distinto, o operador `*` sobre Arrays é uma convenção, não
matemática.

Tensores são objetos matemáticos com regras bem definidas:
- **`+`** é adição element-wise com broadcast (requisito: shapes compatíveis)
- **`*`** é multiplicação element-wise (Hadamard) — NÃO produto matricial
- **`dot`** é contração (produto matricial para 2D, contração de índices para N-D)
- **`shape`** retorna as dimensões
- **`scalar`** extrai o valor de um tensor 0-D

Cada operação tem semântica matemática precisa, não convenção de biblioteca.

---

## 2. Sintaxe

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

### Desambiguação Array vs Tensor

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

### Aninhamento N-D

Tensor 3-D exige aninhamento de `[]`:

```
[
    [1 2; 3 4];
    [5 6; 7 8]
]
```

O parser recursivo: cada `[` inicia um novo nível. O `;` no nível externo
separa fatias (slices) do eixo 0. O `;` no nível interno separa linhas
dentro de cada fatia.

---

## 3. Tipo

```
Tensor::T::(Int...)   # ex: Tensor::Int::(2 3) = matriz 2×3 de Int
```

- `T` é o tipo do elemento (deve implementar NUM).
- `(Int...)` é uma tupla de dimensões — cada elemento é o tamanho de uma
  dimensão. Conhecida em compile-time (Const Generics).
- Tensor 0-D (`Tensor::T::()`) representa um escalar — mas `()` é `Unit`
  no Kata5, então a representação de 0-D precisa ser resolvida (ver
  Questões Abertas).

### Por que `Ty::Tensor` intrínseco, não `data`

Um `data Matrix::T (shape::Tuple data::Array::T)` seria possível sem tocar
no compilador. Mas seria limitado:

- **Sem sintaxe literal**: `[1 2; 3 4]` não poderia produzir um Matrix
  diretamente — seria um Array que precisa ser convertido.
- **Sem dispatch distinto**: `*` sobre Matrix e `*` sobre Array seriam
  indistinguíveis sem o tipo intrínseco.
- **Sem ABI otimizada**: o codegen não poderia emitir SIMD com base no
  tipo, porque Matrix seria um Struct opaco.
- **Sem restrição numérica**: um `data` aceita qualquer tipo de elemento.
  O type checker não poderia garantir que o elemento implementa NUM.

Como `Ty::Tensor`, o compilador tem controle sobre sintaxe literal,
restrição de elemento, dispatch de operadores, ABI de representação, e
caminho futuro para const generics.

### Const Generics

`Ty::Generic` existe para enums genéricos, mas Const Generics (inteiros
como parâmetros de tipo) é uma extensão. Como representar `(Int...)` no
`Ty`? Hoje `Ty::Generic(String, Vec<Ty>)` carrega `Ty` — mas dimensões
são `Int`, não `Ty`. Precisa de:

- Ou aceitar `Ty::Prim(Int)` como argumento de `Generic` e tratar
  como dimensão
- Ou criar `Ty::Tensor(Box<Ty>, Vec<usize>)` dedicado

Const generics exigem mudanças profundas no sistema de tipos:

1. **`Ty` precisa carregar valores inteiros como parâmetros** — não apenas tipos.
2. **Unificação de tipos precisa comparar valores** — `Tensor<Int, 2, 3>` vs
   `Tensor<Int, 3, 2>` são diferentes.
3. **Monomorphização precisa especializar por valor** — layouts de memória distintos.
4. **Inferência precisa resolver constraints aritméticas** — se
   `dot :: Tensor<T, M, K> Tensor<T, K, N> => Tensor<T, M, N>`, o type checker
   precisa deduzir que o `K` dos dois argumentos é o mesmo.

**Decisão:** tensores podem existir com shape conhecido apenas em runtime
como primeira versão. O design deve ser feito de forma que const generics
possam ser adicionados no futuro sem mudar a sintaxe da linguagem.

---

## 4. Interface TENSOR

```kata
interface TENSOR::T
    shape :: Self => Tuple
    rank  :: Self => Int
    at    :: Self Int => Result::T        # indexação 1-D (flatten)
    +     :: Self Self => Self            # element-wise + broadcast
    *     :: Self Self => Self            # Hadamard (element-wise)
    dot   :: Self Self => Self            # contração
    transpose :: Self => Self             # transposição (2D → swap axes)
    scalar :: Self => T                   # extrair escalar de 0-D
```

| Operação | Matemática | Observações |
|---|---|---|
| `+` | `C[i] = A[i] + B[i]` | Broadcast: se `B` é escalar, `C[i] = A[i] + B`. |
| `*` | `C[i] = A[i] * B[i]` | Hadamard. NÃO é produto matricial. |
| `dot` | Contração de índices | 2D: `C[i,j] = Σ A[i,k]·B[k,j]`. N-D: contração no último eixo de A com o primeiro de B. |
| `transpose` | Permutação de eixos | 2D: swap rows/cols. N-D: permutação geral (futuro). |
| `shape` | Dimensões | `(2 3)` para matriz 2x3. |
| `rank` | Número de dimensões | 0 para escalar, 1 para vetor, 2 para matriz. |
| `scalar` | Extração | `scalar [42;]` = 42. Requer rank 0. |
| `at` | Indexação | Retorna Result (pode estar out-of-bounds). |

A interface TENSOR não herda de NUM. `+` e `*` são redefinidos com semântica
matricial, não escalar. O dispatch resolve pelo tipo: `+ Int Int` despacha
para NUM, `+ Tensor Int` despacha para TENSOR (broadcast).

---

## 5. Operações

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
tem as dimensões externas.

### Introspecção

```
shape :: Tensor::T::(D...) => Tuple   # retorna (D1, D2, ...)
scalar :: Tensor::T::() => T          # extrai escalar de tensor 0-D
```

### Coerção Array → Tensor (fronteira dinâmica)

```kata
let dados := ler_banco          # Array::Int (tamanho dinâmico)
let tentativa := Tensor::Int::(3 3) dados   # Result — falha se shape não bater
```

A conversão de Array (tamanho dinâmico) para Tensor (tamanho estático) não
é implícita. O construtor `Tensor::T::(dims)` recebe o Array e valida as
dimensões em runtime. Retorna `Result` — falha se o número de elementos
não corresponde ao esperado.

---

## 6. Shape Inference

Sem const generics, o compilador não sabe o shape de um tensor em geral.
Mas "não sabe" não é binário. Há três níveis:

1. **Shape conhecido em compile-time** — literais fixam o shape. Operações
   sobre literais produzem shapes derivados.
2. **Shape parcialmente conhecido** — parâmetros de função têm shape
   desconhecido, mas relações podem ser rastreadas.
3. **Shape totalmente desconhecido (runtime)** — shape depende de I/O.

### Proposta: shape inference sem const generics

Rastrear shapes conhecidos sem exigir const generics:

```rust
/// Shape conhecido em compile-time, se possível.
/// None = desconhecido (runtime). Some = conhecido.
enum ShapeInfo {
    Unknown,
    Known(Vec<usize>),     // ex: [2, 3] para matriz 2x3
    Symbolic(String),      // futuro: rastreamento parcial
}
```

Isso **não** é const generic. O `Ty` permanece `Tensor(Box<Ty>)` — sem
dimensões no tipo. O shape é metadata de inference, não parte do tipo.

| Expressão | ShapeInfo |
|---|---|
| `[1 2; 3 4]` | `Known([2, 2])` |
| `dot a b` (a, b conhecidos) | `Known([m, n])` se a=[m,k], b=[k,n] |
| `dot a b` (um desconhecido) | `Unknown` |
| `transpose a` (conhecido) | `Known(reversed(shape))` |
| `+ a b` (conhecidos) | `Known(shape_a)` se shapes compatíveis |
| `t` (parâmetro de função) | `Unknown` |
| `Tensor (r, c) data` | `Unknown` (r, c são runtime) |

Quando o compilador conhece os shapes de ambos os operandos de `dot`,
verifica compatibilidade em compile-time. Shape inference é local, não
sobrevive fronteiras de função.

---

## 7. Runtime e Codegen

### Representação runtime

```c
struct kata_rt_tensor {
    void*  data;        // buffer contíguo, row-major
    int64_t rank;        // número de dimensões
    int64_t* shape;      // [rank] dimensões
    int64_t* strides;    // [rank] strides em elementos (não bytes)
    PrimTy  elem_type;   // Int, Float, Rational
};
```

### FFI symbols

```
kata_rt_tensor_new      (data, rank, shape) → tensor*
kata_rt_tensor_shape    (tensor*) → int64_t* (shape array)
kata_rt_tensor_rank     (tensor*) → int64_t
kata_rt_tensor_at       (tensor*, int64_t flat_index) → void* (element ptr)
kata_rt_tensor_add      (tensor*, tensor*) → tensor*    (element-wise + broadcast)
kata_rt_tensor_mul      (tensor*, tensor*) → tensor*    (Hadamard)
kata_rt_tensor_dot      (tensor*, tensor*) → tensor*    (contração)
kata_rt_tensor_transpose (tensor*) → tensor*
kata_rt_tensor_scalar   (tensor*) → void* (element ptr)
kata_rt_tensor_free     (tensor*)
```

Operações element-wise (`+`, `*`) são loops próprios no `kata-rt`.
`transpose` é zero-copy (apenas inverte strides e shape).
Operações de contração (`dot`) delegam para `matrixmultiply` quando o
elemento é Float (ver §8). Para Int e Rational, loops próprios.

---

## 8. Backend de Álgebra Linear

### Por que `matrixmultiply`

Reescrever matmul do zero é reinventar a roda — e fazer isso mal. GEMM
tem décadas de otimização: blocking, tiling, cache-aware layout, microkernels
SIMD. A auto-vectorização do Cranelift não chega nesse nível.

Mas o binário Kata deve ser **auto-contido** — portátil entre Linux x64 e
macOS (x64 e Apple Silicon) sem exigir que o usuário instale BLAS, OpenBLAS,
ou qualquer biblioteca C externa.

A crate [`matrixmultiply`](https://crates.io/crates/matrixmultiply) resolve
exatamente esse problema:

- **Rust puro** — sem dependências C, sem FFI para bibliotecas do sistema
- **`no-std` compatível** — pode ser usada no runtime isolado
- **Microkernels SIMD** para x86-64 (SSE2, AVX, AVX-512) e AArch64 (NEON)
- **f32 e f64 GEMM** com strides arbitrários
- **Link estático** — compila dentro do binário, zero dependência do host
- **Licença MIT/Apache-2.0**

| Plataforma | Suporte matrixmultiply |
|---|---|
| Linux x86_64 | ✅ microkernel SSE2/AVX |
| macOS x86_64 (Intel) | ✅ microkernel SSE2/AVX |
| macOS aarch64 (Apple Silicon) | ✅ microkernel NEON |

### Mapeamento Kata → matrixmultiply

| Kata | matrixmultiply | Observação |
|---|---|---|
| `dot a b` (2D × 2D, Float) | `dgemm` / `sgemm` | GEMM principal |
| `dot a b` (1D × 1D, Float) | loop próprio (dot product trivial) | O(N), não justifica GEMM |
| `dot a b` (2D × 1D, Float) | `dgemm` com N=1 | GEMM com vetor como matriz coluna |
| `+ a b` (element-wise) | loop próprio | Simples, BLAS não cobre |
| `* a b` (Hadamard) | loop próprio | Simples, BLAS não cobre |
| `transpose a` | troca de strides (zero-copy) | Não precisa de GEMM |
| `dot a b` (Int) | loop próprio | matrixmultiply é só Float |
| `dot a b` (Rational) | loop próprio | matrixmultiply é só Float |

### Por que não OpenBLAS linkado estaticamente

- **Build pesado**: OpenBLAS em C/Fortran, exige `gcc` e `gfortran`. O build
  do Kata é rápido hoje — adicionar OpenBLAS quebra isso.
- **Tamanho do binário**: OpenBLAS estático adiciona ~20MB. `matrixmultiply`
  adiciona ~50KB.
- **Complexidade de cross-compile**: `matrixmultiply` é Rust puro —
  cross-compile funciona nativamente via `cargo build --target`.

Se no futuro a performance do `matrixmultiply` for insuficiente, OpenBLAS
pode ser adicionada como feature opt-in.

---

## 9. `dot` com shape desconhecido

Quando o compilador não pode verificar o shape em compile-time, três opções:

**(a) `dot` retorna `Result`**

```kata
dot :: Tensor::T Tensor::T => Result::(Tensor::T, Text)
```

Seguro mas verboso. `dot (dot a b) c` vira pattern matching encadeado.

**(b) `dot` assume compatibilidade, erro é UB**

```kata
dot :: Tensor::T Tensor::T => Tensor::T
# Pre-condição: shapes compatíveis. Violar é UB.
```

Alinhado com a filosofia de tipos refinados (`NonZero` para divisão). O
programador garante via `shape` antes de chamar. `dot (dot a b) c` é direto.

**(c) `dot` valida em runtime e pânica**

Meio-termo. Erro é diagnosticável mas não controlável.

**Acoplamento com shape inference:** com shape inference, a opção (b) UB
fica mais defensável — o compilador já pega os erros óbvios (literais), e
UB só aplica em shapes genuinamente runtime.

**Recomendação provisória:** shape inference + (b) UB.

---

## 10. Questões Abertas

### D1. `[1 2 3]` (sem `;`) é List ou Tensor 1-D?

Sem `;`, é List (Cons, persistente). Vetor tensor exige `;`: `[1 2 3;]`.
Confirmar que esta convenção é natural e que vetor linha não precisa de
sintaxe literal sem `;`.

### D2. `dot` retorna Result, é UB, ou pânica?

Ver §9 para análise completa. Recomendação provisória: shape inference + UB.

### D3. Tensor 0-D e escalar

`scalar :: Tensor::T::() => T` extrai escalar de tensor 0-D. Mas `()`
é `Unit` no Kata5 — `(Int...)` com zero dimensões colide com `Unit`.
Opções:
- `Tensor::T` sem tupla de dimensões = 0-D por convenção
- Proibir 0-D (todo tensor é pelo menos 1-D)
- Usar um tipo dedicado para a tupla de dimensões que distingue vazio de Unit

### D4. Suportar aninhamento N-D na primeira versão?

Tensor 3-D exige `[ [1 2; 3 4]; [5 6; 7 8] ]`. O parser precisa de recursão.
Implementar desde o início ou começar com 2-D flat? O tipo `Ty::Tensor` já
é N-D, mas o parser pode aceitar só 2-D inicialmente.

### D5. Indexação N-D

`t.0` hoje é indexação 1-D (Tuple/Array). Para tensor N-D, como acessar
elementos? `t.(0 1)` (tupla de índices)? `t.0.1` (encadeado)? Precisa ser
definido.

### D6. `;` terminal

`[1 2 3;]` com `;` terminal — é tensor 1×3? Pela regra "se há `;` em
qualquer posição dentro de `[]`, é tensor", sim. Confirmar que o `;`
terminal é legal e opcional: `[1 2 3; 4 5 6]` e `[1 2 3; 4 5 6;]` são a
mesma matriz 2×3.

### D7. Coerção Array → Tensor

```kata
let arr := {1 2 3 4}
let t := Tensor::Int::(2 2) arr     # construtor: shape + data → Result::Tensor
```

O construtor valida que `len(data) == product(shape)`. Retorna `Result`.

### D8. Tensor implementa ITERABLE?

Se sim, `for x in tensor` itera sobre elementos flattened (row-major).
Útil mas potencialmente confuso — iterar sobre linhas vs elementos?

**Proposta:** Tensor implementa ITERABLE sobre elementos flattened.
Para iterar sobre linhas, usar `slice` ou indexação explícita (futuro).

### D9. Rational como elemento de Tensor?

Rational é exato mas não tem SIMD. Tensor de Rational seria correto
matematicamente mas lento. A interface NUM é implementada por Int, Float
e Rational. Se Tensor exige NUM, Rational é automaticamente permitido.
O custo é de runtime, não de correção.

---

## 11. Plano de Implementação (esboço)

### Fase 1: Fundação

1. **`kata-ast`**: Adicionar `Expr::TensorLit { dimensions: Vec<Vec<Spanned<Expr>>> }`
2. **`kata-core`**: Adicionar `Ty::Tensor(Box<Ty>)`. Atualizar `extract_type_name`,
   `TypeShape`, display, hash.
3. **`kata-parser`**: Implementar parsing de `[]` com `;` (recursão para N-D).
4. **`kata-resolution`**: `resolve_type_expr` reconhece `Tensor::(T)` →
   `Ty::Tensor(Box<Ty>)`.
5. **`kata-rt`**: Implementar `kata_rt_tensor` struct e FFI functions básicas.

### Fase 2: Type Checking

6. **`kata-inference`**: Inference de `TensorLit` — elementos devem implementar
   NUM, linhas da mesma dimensão devem ter mesmo comprimento.
7. **`kata-inference`**: Coerção Array → Tensor via construtor.

### Fase 3: Interface TENSOR

8. **`stdlib/core.kata`**: Declarar `interface TENSOR::T` com assinaturas.
9. **`stdlib/core.kata`**: `Tensor::T implements TENSOR::T` com métodos `@ffi`.
10. **`kata-inference`**: Dispatch de `+`, `*`, `dot` para TENSOR.

### Fase 4: Codegen

11. **`kata-codegen`**: Lowering de `TensorLit` → `kata_rt_tensor_new`.
12. **`kata-codegen`**: Lowering de operações TENSOR → calls FFI.

### Fase 5: Shape Inference

13. **`kata-core`**: Adicionar `ShapeInfo` (sidecar ao `TypedExpr`).
14. **`kata-inference`**: Propagação de shapes — literais produzem `Known`,
    operações derivam, parâmetros produzem `Unknown`.
15. **`kata-inference`**: Verificação de compatibilidade em compile-time
    quando ambos os operandos de `dot` têm `Known` shapes.

### Fase 6: Backend de Álgebra Linear

16. **`kata-rt/Cargo.toml`**: Adicionar `matrixmultiply = "0.3"`.
17. **`kata-rt/src/tensor/`**: Implementar `dot` para Float via `matrixmultiply`.
18. **`kata-rt/src/tensor/`**: Implementar `dot` para Int e Rational (loops próprios).
19. **`kata-rt/src/tensor/`**: Implementar `+`, `*` (element-wise), `transpose`.

### Fase 7: Monomorphização e Tree Shaking

20. **`kata-monomorph`**: Instanciar `Tensor::(Int)`, `Tensor::(Float)`, etc.
21. **`kata-tree-shaking`**: Marcar FFI symbols de tensor como reachable.

### Fase 8: Testes

22. **`examples/`**: Migrar `test_tensor_math.kata` do Kata4.
23. **`kata-codegen/tests/`**: Testes E2E de tensor add, mul, dot, transpose.
24. **`kata-rt/tests/`**: Testes de `dot` Float comparando `matrixmultiply`
    vs loop de referência.
25. **`kata-inference/tests/`**: Testes de shape inference — erros de shape
    em compile-time para literais, `Unknown` para parâmetros.

---

## 12. Histórico

### Design original (Kata4 / Specs OLD)

- Tensores como "família de elite" com const generics
- `Tensor::T::(Int...)` — shape no tipo, conhecido em compile-time
- DOT_BEHAVIOR: type-level validation de compatibilidade de shapes
- Tradução direta para SIMD no Cranelift
- Coerção Array → Tensor via construtor falível (retorna Result)
- Parser tinha `parse_array_or_tensor` que distinguia por `;`

### Estado no Kata5

- `Expr::Tensor` removido do AST
- `Ty::Tensor` não existe
- `parse_array_or_tensor` removido do parser
- Tensores listados em "Fora do Escopo 1.0" no ROADMAP e TODO
- `test_tensor_math.kata` não migrado — "Bug intencional de dot com shapes
  incompatíveis — decisão de design pendente"

### Este documento

Unificado de TENSOR-VISAO.md (9 ago, documento extenso com discussão de
runtime, codegen, matrixmultiply, shape inference) e VISAO-tensor.md
(17 ago, sintaxe `[]` com const generics). O mais recente tem precedência
na sintaxe e no modelo de tipos; o conteúdo único do antigo (backend de
álgebra linear, shape inference, representação runtime, plano de
implementação) foi incorporado.