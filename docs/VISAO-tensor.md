# Visão — Tensor no Kata

**Data:** 2026-08-17 (unificado de TENSOR-VISAO.md e VISAO-tensor.md)
**Revisão:** 2026-09-12 (sintaxe de literais e indexação refinadas)
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

## 2. Sintaxe de Literais

### 2.1. Desambiguação List vs Tensor

```
[1 2 3]          # List (Cons, persistente) — sem ;
[1 2 3; 4 5 6]   # Tensor 2×3 — com ; separando linhas
[1 2 3;]         # Tensor 1×3 — ; terminal opcional (vetor linha)
[1; 2; 3]        # Tensor 3×1 — vetor coluna
```

`;` dentro de `[]` é o discriminador: se há `;` em qualquer posição, é
Tensor. Sem `;`, é List.

`{1 2 3}` continua sendo Array (contíguo, imutável, tamanho dinâmico).
`{"k": v}` continua sendo Dict. `{|1 2 3|}` continua sendo Set.

### 2.2. Vírgula como separador opcional

Dentro de `[]`, a vírgula é um separador opcional — equivalente ao espaço.
A semântica da vírgula é determinada pelo delimitador que a envolve, não
globalmente: `(1, 2, 3)` é tupla dentro de `()`; `[1, 2, 3]` é tensor/lista
dentro de `[]`. Não há ambiguidade — o parser de `[]` consome vírgulas como
separador sintático entre elementos.

```kata
[1 2 3; 4 5 6]       # Tensor 2×3 — espaços
[1, 2, 3; 4, 5, 6]  # Mesmo tensor 2×3 — vírgulas (opcional)
[1, 2, 3;]          # Tensor 1×3 — vírgula + ; terminal
```

A vírgula é útil para legibilidade com expressões longas:

```kata
[+ x 1, * y 2; - z 3, / w 4]
```

### 2.3. `;` terminal

`[1 2 3;]` com `;` terminal é tensor 1×N. Pela regra "se há `;` em qualquer
posição dentro de `[]`, é tensor", sim. O `;` terminal é legal e opcional:
`[1 2 3; 4 5 6]` e `[1 2 3; 4 5 6;]` são a mesma matriz 2×3.

### 2.4. N-D via aninhamento

Tensor 3-D exige aninhamento de `[]`:

```kata
[
    [1 2; 3 4];
    [5 6; 7 8]
]
```

O parser recursivo: cada `[` inicia um novo nível. O `;` no nível externo
separa fatias (slices) do eixo 0. O `;` no nível interno separa linhas
dentro de cada fatia.

Aninhamento é preferido a `;;` `;;;` (Julia) por três razões:
- `;;` `;;;` não têm significado fora de tensores e poluem o lexer com
  tokens novos para cada nível
- Aninhamento de `[]` é recursivo — o parser já sabe lidar com `[]`
- O leitor vê a estrutura visualmente, igual à notação matemática de blocos

### 2.5. Desambiguação List vs Tensor — tabela

| Propriedade | List `[1 2 3]` | Tensor `[1 2 3; 4 5 6]` |
|---|---|---|
| Sintaxe | `[]` sem `;` | `[]` com `;` |
| Vírgula opcional | ✅ (equivalente a espaço) | ✅ (equivalente a espaço) |
| Topologia | Encadeada (Cons, partilha estrutural) | Contíguo (row-major) |
| Tamanho | Dinâmico (runtime) | Estático (compile-time, aspiracional) |
| Dimensionalidade | 1-D | N-D |
| Álgebra linear | ❌ | ✅ (`+`, `*`, `dot`) |
| SIMD | ❌ | ✅ |
| Tipo | `List::T` | `Tensor::T` |

Array `{1 2 3}` não tem ambiguidade com nenhum dos dois — usa `{}`.
Lists são para processamento funcional (imutabilidade, partilha).
Tensores são para cálculo matemático (rígido, acelerado).

---

## 3. Tipo

```
Tensor::T   # ex: Tensor::Int = matriz de Int, Tensor::Float = matriz de Float
```

- `T` é o tipo do elemento (deve implementar NUM).
- Shape é metadata de inference (`ShapeInfo`), não parte do tipo na primeira
  versão (ver §6 — Shape Inference).
- Futuro: const generics para shape no tipo (`Tensor::Int::(2 3)`).

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

### Const Generics (futuro)

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

## 4. Indexação

### 4.1. Sintaxe `.(...)`

A indexação N-D usa `.()` com uma tupla de índices. Cada posição na tupla
corresponde a um eixo do tensor.

```kata
let m := [1 2 3; 4 5 6]       # Tensor 2×3

m.(0 1)                        # elemento [0, 1] → Result::Int
m.(0)                         # linha 0 inteira → Tensor 1×3
m.(1)                         # linha 1 inteira → Tensor 1×3
```

- **Índice simples** (`m.(0)`): sub-tensor ao longo do eixo 0 (linha inteira).
  Retorna `Tensor`, não `Result` — sempre válido se o eixo existe.
- **Índices completos** (`m.(0 1)`): elemento específico. Retorna
  `Result::T` — bounds check é runtime (shape pode ser desconhecido em
  compile-time).

### 4.2. Slicing com `..`

Ranges (`..`) já existem na linguagem para List/Range. Dentro de `.()`,
um range seleciona um sub-intervalo do eixo:

```kata
m.(0..2)                      # linhas 0 a 1 → Tensor 2×3
m.(0..2 1..3)                 # sub-matriz: linhas 0-1, cols 1-2 → Tensor 2×2
m.(0..=1 0..=2)               # inclusivo — mesmo resultado
```

### 4.3. Wildcard `_` para "todas as posições do eixo"

`_` (hole/wildcard) já é parte da linguagem (pattern matching, currying).
Dentro de `.()`, `_` significa "todas as posições deste eixo":

```kata
m.(0 _)                       # linha 0, todas as colunas → Tensor 1×3
m.(_ 1)                       # todas as linhas, coluna 1 → Tensor 2×1
m.(_ _)                       # matriz inteira → cópia → Tensor 2×3
m.(_ 0..2)                    # todas as linhas, colunas 0-1 → Tensor 2×2
```

### 4.4. Indexação 1-D (flatten)

```kata
m.at(5)                       # elemento flatten no índice 5 → Result::T
```

Indexação 1-D via interface INDEXABLE (como Array/List). Útil para
iteração e interop com coleções.

### 4.5. Regras de tipo de retorno

| Expressão | Eixo 0 | Eixo 1 | Retorno |
|---|---|---|---|
| `m.(0 1)` | índice 0 | índice 1 | `Result::T` (elemento) |
| `m.(0)` | índice 0 | — | `Tensor` (sub-tensor 1×3) |
| `m.(0..2 1)` | range 0..2 | índice 1 | `Tensor` (sub-tensor 2×1) |
| `m.(_ 1)` | todas | índice 1 | `Tensor` (sub-tensor 2×1) |
| `m.(_ _)` | todas | todas | `Tensor` (cópia 2×3) |
| `m.at(5)` | flatten 5 | — | `Result::T` (elemento) |

**Princípio:** índices completos (todas as posições especificadas como
inteiros) → `Result::T` (pode estar out-of-bounds). Algum eixo sem índice
completo (range, `_`, ou omitido) → `Tensor` (sempre produz um sub-tensor
válido).

---

## 5. Interface TENSOR

```kata
interface TENSOR::T
    shape :: Self => Tuple
    rank  :: Self => Int
    at    :: Self Int => Result::T        # indexação 1-D (flatten)
    +     :: Self Self => Self             # element-wise + broadcast
    *     :: Self Self => Self             # Hadamard (element-wise)
    dot   :: Self Self => Self            # contração
    transpose :: Self => Self              # transposição (2D → swap axes)
    scalar :: Self => T                    # extrair escalar de 0-D
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
é `Unit` no Kata — `(Int...)` com zero dimensões colide com `Unit`.
Opções:
- `Tensor::T` sem tupla de dimensões = 0-D por convenção
- Proibir 0-D (todo tensor é pelo menos 1-D)
- Usar um tipo dedicado para a tupla de dimensões que distingue vazio de Unit

### D4. Suportar aninhamento N-D na primeira versão?

Tensor 3-D exige `[ [1 2; 3 4]; [5 6; 7 8] ]`. O parser precisa de recursão.
Implementar desde o início ou começar com 2-D flat? O tipo `Ty::Tensor` já
é N-D, mas o parser pode aceitar só 2-D inicialmente.

### D5. Coerção Array → Tensor

```kata
let arr := {1 2 3 4}
let t := Tensor::Int::(2 2) arr     # construtor: shape + data → Result::Tensor
```

O construtor valida que `len(data) == product(shape)`. Retorna `Result`.

### D6. Tensor implementa ITERABLE?

Se sim, `for x in tensor` itera sobre elementos flattened (row-major).
Útil mas potencialmente confuso — iterar sobre linhas vs elementos?

**Proposta:** Tensor implementa ITERABLE sobre elementos flattened.
Para iterar sobre linhas, usar indexação `.()` explícita.

### D7. Rational como elemento de Tensor?

Rational é exato mas não tem SIMD. Tensor de Rational seria correto
matematicamente mas lento. A interface NUM é implementada por Int, Float
e Rational. Se Tensor exige NUM, Rational é automaticamente permitido.
O custo é de runtime, não de correção.

### D8. Broadcast — quais shapes são compatíveis?

NumPy/Julia têm regras de broadcast bem estabelecidas (right-aligned, dims
de tamanho 1 ou iguais). Kata pode adotar as mesmas regras, mas precisa
documentar quais shapes são broadcastable para `+` e `*`.

---

## 11. Plano de Implementação (esboço)

### Fase 1: Fundação

1. **`kata-ast`**: Adicionar `Expr::TensorLit { dimensions: Vec<Vec<Spanned<Expr>>> }`
2. **`kata-core`**: Adicionar `Ty::Tensor(Box<Ty>)`. Atualizar `extract_type_name`,
   `TypeShape`, display, hash.
3. **`kata-parser`**: Implementar parsing de `[]` com `;` (recursão para N-D).
   Vírgula como separador opcional equivalente a espaço.
4. **`kata-resolution`**: `resolve_type_expr` reconhece `Tensor::(T)` →
   `Ty::Tensor(Box<Ty>)`.
5. **`kata-rt`**: Implementar `kata_rt_tensor` struct e FFI functions básicas.

### Fase 2: Type Checking

6. **`kata-inference`**: Inference de `TensorLit` — elementos devem implementar
   NUM, linhas da mesma dimensão devem ter mesmo comprimento.
7. **`kata-inference`**: Coerção Array → Tensor via construtor.
8. **`kata-inference`**: Indexação `.()` — desugar para FFI com bounds check.

### Fase 3: Interface TENSOR

9. **`stdlib/core.kata`**: Declarar `interface TENSOR::T` com assinaturas.
10. **`stdlib/core.kata`**: `Tensor::T implements TENSOR::T` com métodos `@ffi`.
11. **`kata-inference`**: Dispatch de `+`, `*`, `dot` para TENSOR.

### Fase 4: Codegen

12. **`kata-codegen`**: Lowering de `TensorLit` → `kata_rt_tensor_new`.
13. **`kata-codegen`**: Lowering de operações TENSOR → calls FFI.
14. **`kata-codegen`**: Lowering de indexação `.()` → `kata_rt_tensor_at` ou slicing.

### Fase 5: Shape Inference

15. **`kata-core`**: Adicionar `ShapeInfo` (sidecar ao `TypedExpr`).
16. **`kata-inference`**: Propagação de shapes — literais produzem `Known`,
    operações derivam, parâmetros produzem `Unknown`.
17. **`kata-inference`**: Verificação de compatibilidade em compile-time
    quando ambos os operandos de `dot` têm `Known` shapes.

### Fase 6: Backend de Álgebra Linear

18. **`kata-rt/Cargo.toml`**: Adicionar `matrixmultiply = "0.3"`.
19. **`kata-rt/src/tensor/`**: Implementar `dot` para Float via `matrixmultiply`.
20. **`kata-rt/src/tensor/`**: Implementar `dot` para Int e Rational (loops próprios).
21. **`kata-rt/src/tensor/`**: Implementar `+`, `*` (element-wise), `transpose`.

### Fase 7: Monomorphização e Tree Shaking

22. **`kata-monomorph`**: Instanciar `Tensor::(Int)`, `Tensor::(Float)`, etc.
23. **`kata-tree-shaking`**: Marcar FFI symbols de tensor como reachable.

### Fase 8: Testes

24. **`examples/`**: Migrar `test_tensor_math.kata` do Kata4.
25. **`kata-codegen/tests/`**: Testes E2E de tensor add, mul, dot, transpose.
26. **`kata-rt/tests/`**: Testes de `dot` Float comparando `matrixmultiply`
    vs loop de referência.
27. **`kata-inference/tests/`**: Testes de shape inference — erros de shape
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

Revisão de 2026-09-12 refinou:
- Vírgula como separador opcional dentro de `[]` (equivalente a espaço)
- Indexação `.()` com tupla de índices, slicing via `..`, wildcard `_` para
  "todas as posições do eixo"
- N-D via aninhamento de `[]` (confirmado, rejeitado `;;` `;;;`)
- Operações removidas do escopo imediato (foco em literais + indexação)