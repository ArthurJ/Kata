# Tensores N-Dimensionais no Kata5 — Documento de Visão

**Estado:** Design aberto. Nem todas as decisões estão fechadas.
**Motivação:** Kata foi concebida para computação numérica. Operações matriciais
são a razão de existir da linguagem — não um recurso opcional.

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

## 2. Const Generics — Definição

**Const generics** são parâmetros de tipo preenchidos com valores conhecidos
em compile-time (constantes), não com tipos. O exemplo clássico é um array
cujo tamanho é parte do tipo:

```
Array<Int, 3>   # array de 3 inteiros — o 3 é um const generic
```

Em Kata5, `Ty::Array(Box<Ty>)` carrega apenas o tipo do elemento. Não há
segundo parâmetro para o tamanho. Se existisse const generics, poderíamos ter:

```
Tensor<Float, 2, 3>   # matriz 2x3 de Float — shape conhecido em compile-time
```

Isso permitiria ao type checker **rejeitar** `dot` com shapes incompatíveis
estaticamente:

```
let a := Tensor<Float, 2, 3> (...)   # 2x3
let b := Tensor<Float, 4, 5> (...)   # 4x5
dot a b                               # ERRO em compile-time: 3 ≠ 4
```

### Por que const generics são caros

Const generics exigem mudanças profundas no sistema de tipos:

1. **`Ty` precisa carregar valores inteiros como parâmetros** — não apenas tipos.
   Hoje `Ty::Generic(String, Vec<Ty>)` carrega apenas `Ty`s. Precisaria de
   algo como `Ty::Generic(String, Vec<TypeArg>)` onde `TypeArg` pode ser
   tipo OU valor constante.

2. **Unificação de tipos precisa comparar valores** — não apenas tipos.
   `Tensor<Int, 2, 3>` vs `Tensor<Int, 2, 3>` são iguais, mas
   `Tensor<Int, 2, 3>` vs `Tensor<Int, 3, 2>` são diferentes.

3. **Monomorphização precisa especializar por valor** — não apenas por tipo.
   `Tensor<Float, 2, 3>` e `Tensor<Float, 3, 2>` geram código diferente
   (layouts de memória distintos).

4. **Inferência precisa resolver constraints aritméticas** — se
   `dot :: Tensor<T, M, K> Tensor<T, K, N> => Tensor<T, M, N>`, o type checker
   precisa deduzir que o `K` dos dois argumentos é o mesmo e que o resultado
   é `M x N`. Isso é unificação de inteiros em compile-time, significativamente
   mais complexo que unificação de tipos.

### Sintaxe proposta para const generics (se implementados)

A sintaxe `{1 2; 3 4; (Int 2 2)}` é interessante. O `(Int 2 2)` seria uma
**ascription de tipo** análoga a `3.14::Rational`:

```
{1 2; 3 4}::(Int 2 2)        # tensor literal 2x2 de Int
{1 2 3; 4 5 6}::(Int 2 3)    # tensor 2x3
```

O `::` já é o operador de ascription na linguagem. A ascription de tipo
para um tensor carregaria não apenas o tipo do elemento (`Int`) mas também
as dimensões (`2 2`). Isso é const generic aplicado ao tensor literal.

**Sem const generics**, a ascription seria apenas o tipo do elemento:

```
{1 2; 3 4}::Tensor<Int>      # tensor de Int, shape verificado em runtime
```

### Decisão: const generics são pré-requisito para tensores?

**Não.** Tensores podem existir com shape conhecido apenas em runtime. A
diferença é onde a validação acontece:

| Aspecto | Com const generics | Sem const generics |
|---|---|---|
| Validação de shape | Compile-time (type checker rejeita) | Runtime (operação retorna Result ou pânico) |
| Custo de implementação | Alto (sistema de tipos + inferência) | Médio (novo Ty + parser + codegen) |
| Experiência do usuário | Erro antes de executar | Erro durante execução |
| Código gerado | Especializado por shape | Genérico, shape lido em runtime |

**Este documento assume que a primeira versão NÃO usa const generics.**
O design deve ser feito de forma que const generics possam ser adicionados
no futuro sem mudar a sintaxe da linguagem.

---

## 3. `Ty::Tensor` — Tipo Intrínseco

### 3.1. Por que intrínseco, não `data`

Um `data Matrix::T (shape::Tuple data::Array::T)` seria possível sem tocar
no compilador. Mas seria limitado:

- **Sem sintaxe literal**: `{1 2; 3 4}` não poderia produzir um Matrix
  diretamente — seria um Array que precisa ser convertido.
- **Sem dispatch distinto**: `*` sobre Matrix e `*` sobre Array seriam
  indistinguíveis sem o tipo intrínseco.
- **Sem ABI otimizada**: o codegen não poderia emitir SIMD com base no
  tipo, porque Matrix seria um Struct opaco.
- **Sem restrição numérica**: um `data` aceita qualquer tipo de elemento.
  O type checker não poderia garantir que o elemento implementa NUM.

Como `Ty::Tensor`, o compilador tem controle sobre:
1. Sintaxe literal (`{1 2; 3 4}`)
2. Restrição de elemento (deve implementar NUM)
3. Dispatch de operadores (`*` é Hadamard para Tensor, indefinido para Array)
4. ABI de representação (ponteiro + shape, como Array + metadata)
5. Caminho futuro para const generics sem mudança de sintaxe

### 3.2. Variante no `Ty`

```rust
/// Tensor N-dimensional: `{1 2; 3 4; 5 6}` — bloco contíguo com shape.
/// Genérico sobre o tipo do elemento (deve implementar NUM).
/// Shape é runtime (não const generic nesta versão).
Tensor(Box<Ty>),
```

Apenas o tipo do elemento é carregado no `Ty`. O shape (dimensões) é
runtime — mora no valor, não no tipo.

### 3.3. Por que `Tensor` e não `Matrix`

`Matrix` implica 2D. `Tensor` é N-D. A sintaxe `{1 2; 3 4; 5 6}` já é
naturalmente N-D (3 linhas, 2 colunas). Um tensor 3-D seria:

```
{
    {1 2; 3 4};
    {5 6; 7 8}
}
```

Começar com `Tensor` não significa implementar todas as operações N-D
imediatamente. As operações podem ser introduzidas incrementalmente:

1. **2D first**: `dot` (produto matricial), `+` (element-wise), `*` (Hadamard),
   `transpose`, `shape`, `scalar`
2. **N-D depois**: broadcast, contração de índices, reshape

Mas o tipo é `Tensor` desde o início. Não há `Matrix` que depois vira `Tensor`.

---

## 4. Sintaxe e Parsing

### 4.1. Sintaxe literal

```kata
# Escalar (0-D)
{42;}

# Vetor linha (1-D)
{1 2 3}

# Vetor coluna (1-D)
{1; 2; 3}

# Matriz (2-D)
{1 2; 3 4}

# Tensor 3-D
{
    {1 2; 3 4};
    {5 6; 7 8}
}
```

### 4.2. Desambiguação Array vs Tensor

Hoje: `{1 2 3}` é ArrayLit. A presença de `;` ativa modo tensor no parser
do Kata4 (`is_tensor = true`).

Proposta: o parser continua produzindo `ArrayLit` quando não há `;`.
Quando encontra `;` dentro de `{}`, produz `TensorLit`:

```rust
// kata-ast/src/expr.rs
/// `{1 2; 3 4}` — tensor literal N-D.
/// `dimensions` é Vec<Vec<Spanned<Expr>>> — cada sub-vec é uma linha/dimensão.
/// Elementos são avaliados e armazenados contiguamente em row-major order.
TensorLit { dimensions: Vec<Vec<Spanned<Expr>> },
```

**Caso edge: `{1 2 3}` (sem `;`) é Array ou Tensor 1-D?**

Se `{1 2 3}` é Array, então um vetor linha é `{1 2 3;}` (com `;` final)?
O Kata4 usava essa convenção. Alternativa: `{1 2 3}` é Tensor 1-D e
Arrays literais usam uma sintaxe diferente.

**Decisão pendente.** Ver §7.

### 4.3. Reviver `parse_array_or_tensor` do Kata4

O Kata4 já tinha essa função. A lógica é simples:

```
- Encontra `{` → começa coletando elementos
- Encontra `;` → is_tensor = true, fecha dimensão atual, começa nova
- Encontra `}` → fecha
- Se is_tensor → produz TensorLit
- Senão → produz ArrayLit
```

### 4.4. Aninhamento

Tensor 3-D exige aninhamento de `{}`:

```
{
    {1 2; 3 4};
    {5 6; 7 8}
}
```

O parser recursivo: cada `{` inicia um novo nível. O `;` no nível externo
separa fatias (slices) do eixo 0. O `;` no nível interno separa linhas
dentro de cada fatia.

Isto é mais complexo que o Kata4, que só lidava com 2-D. Precisa de
parsing recursivo para N-D.

**Decisão pendente:** suportar aninhamento arbitrário desde o início,
ou começar com 2-D flat e adicionar aninhamento depois? Ver §7.

---

## 5. Interface TENSOR

### 5.1. Definição

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

### 5.2. Por que estas operações

| Operação | Matemática | Observações |
|---|---|---|
| `+` | `C[i] = A[i] + B[i]` | Broadcast: se `B` é escalar, `C[i] = A[i] + B`. |
| `*` | `C[i] = A[i] * B[i]` | Hadamard. NÃO é produto matricial. |
| `dot` | Contração de índices | 2D: `C[i,j] = Σ A[i,k]·B[k,j]`. N-D: contração no último eixo de A com o primeiro de B. |
| `transpose` | Permutação de eixos | 2D: swap rows/cols. N-D: permutação geral (futuro). |
| `shape` | Dimensões | `(2 3)` para matriz 2x3. |
| `rank` | Número de dimensões | 0 para escalar, 1 para vetor, 2 para matriz. |
| `scalar` | Extração | `scalar {42;}` = 42. Requer rank 0. |
| `at` | Indexação | Retorna Result (pode estar out-of-bounds). |

### 5.3. Sobre `dot` e validação de shape

Sem const generics, `dot` não pode rejeitar shapes incompatíveis em
compile-time. As opções:

**(a) `dot` retorna `Result`**

```kata
dot :: Tensor::T Tensor::T => Result::(Tensor::T, Text)
```

Toda chamada de `dot` exige `match` ou `?`. Em código numérico, isso é
verboso. `dot (dot a b) c` vira:

```kata
match dot a b
    Ok ab:
        match dot ab c
            Ok abc: ...
            Err e: ...
    Err e: ...
```

**(b) `dot` assume compatibilidade, erro é undefined behavior**

```kata
dot :: Tensor::T Tensor::T => Tensor::T
# Pre-condição: shapes compatíveis. Violar é UB.
```

Alinhado com a filosofia de tipos refinados (`NonZero` para divisão). O
programador garante via `shape` antes de chamar. `dot (dot a b) c` é direto.

O custo é que um bug de shape vira UB, não um erro diagnosticável.

**(c) `dot` valida em runtime e pânica**

```kata
dot :: Tensor::T Tensor::T => Tensor::T
# Panic se shapes incompatíveis.
```

Meio-termo entre (a) e (b). Erro é diagnosticável mas não controlável.

**Decisão pendente.** Ver §7.

### 5.4. Numeração das operações

A interface TENSOR não herda de NUM. `+` e `*` são redefinidos com semântica
matricial, não escalar. O dispatch resolve pelo tipo: `+ Int Int` despacha
para NUM, `+ Tensor Int` despacha para TENSOR (broadcast).

Se Tensor implementa NUM, há conflito: `+ :: Tensor Tensor => Tensor` (TENSOR)
vs `+ :: NUM NUM => NUM` (NUM). Como NUM é interface, e Tensor não é NUM
(elemento é NUM, não o tensor), o conflito não existe — Tensor define suas
próprias operações via TENSOR.

---

## 6. Runtime e Codegen

### 6.1. Representação runtime

```c
struct kata_rt_tensor {
    void*  data;        // buffer contíguo, row-major
    int64_t rank;        // número de dimensões
    int64_t* shape;       // [rank] dimensões
    int64_t* strides;     // [rank] strides em elementos (não bytes)
    PrimTy  elem_type;    // Int, Float, Rational
};
```

- `data` é um buffer contíguo em row-major order (C order).
- `shape` é array de dimensões: `[2, 3]` para matriz 2x3.
- `strides` é precomputado para indexação O(1): stride[i] = produto de
  shape[i+1..rank-1].
- `elem_type` determina o tamanho do elemento (i64, f64, ponteiro).

### 6.2. FFI symbols

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

### 6.3. Estratégia de implementação

Operações element-wise (`+`, `*`) e Hadamard são loops próprios no
`kata-rt` — simples, não há ganho em delegar para uma biblioteca.

Operações de contração (`dot`) delegam para `matrixmultiply` quando o
elemento é Float32/Float64 (ver §7). Para Int e Rational, loops próprios.

`transpose` é zero-copy quando possível: apenas inverte strides e shape,
sem copiar dados.

**Futuro:** intrínsecos SIMD diretos no codegen para operações específicas
(matmul blocking, FMA). Isso é pós-1.0.

---

## 7. Backend de Álgebra Linear

### 7.1. Por que não reimplementar

Reescrever matmul do zero é reinventar a roda — e fazer isso mal. GEMM
(general matrix-matrix multiply) tem décadas de otimização: blocking,
tiling, cache-aware layout, microkernels SIMD. A auto-vectorização do
Cranelift não chega nesse nível.

Mas também não podemos depender de bibliotecas do sistema host. O
binário Kata deve ser **auto-contido** — portátil entre Linux x64 e macOS
(x64 e Apple Silicon) sem exigir que o usuário instale BLAS, OpenBLAS,
ou qualquer biblioteca C externa. Isso é o princípio I8 do manual
estendido ao runtime.

### 7.2. `matrixmultiply` — Rust puro, link estático

A crate [`matrixmultiply`](https://crates.io/crates/matrixmultiply) resolve
exatamente esse problema:

- **Rust puro** — sem dependências C, sem FFI para bibliotecas do sistema
- **`no-std` compatível** — pode ser usada no runtime isolado
- **Microkernels SIMD** para x86-64 (SSE2, AVX, AVX-512) e AArch64 (NEON)
- **f32 e f64 GEMM** com strides arbitrários — funciona com qualquer layout
- **Link estático** — compila dentro do binário, zero dependência do host
- **Licença MIT/Apache-2.0**

Plataformas suportadas (alvos do Kata5):

| Plataforma | Suporte matrixmultiply |
|---|---|
| Linux x86_64 | ✅ microkernel SSE2/AVX |
| macOS x86_64 (Intel) | ✅ microkernel SSE2/AVX |
| macOS aarch64 (Apple Silicon) | ✅ microkernel NEON |

O binário Kata carrega a implementação GEMM dentro de si. Não há
`-lcblas`, não há `DYLD_LIBRARY_PATH`, não há "instale OpenBLAS primeiro".

### 7.3. Mapeamento Kata → matrixmultiply

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

### 7.4. Arquitetura de link

```
kata-rt (staticlib)
  ├── kata_rt_tensor_new, shape, rank, at, free  (próprio — metadata)
  ├── kata_rt_tensor_add, mul                    (próprio — element-wise)
  └── kata_rt_tensor_dot
        ├── Float64 → matrixmultiply::dgemm     (Rust puro, link estático)
        ├── Float32 → matrixmultiply::sgemm     (Rust puro, link estático)
        ├── Int     → loop próprio (acumulação i64)
        └── Rational → loop próprio (BigRational exato)
```

`Cargo.toml` do kata-rt:

```toml
[dependencies]
matrixmultiply = "0.3"
```

Sem `build.rs`, sem `links`, sem `cargo:rustc-link-lib`. A crate é Rust
puro e linka estaticamente.

### 7.5. Tipos de elemento

`matrixmultiply` opera apenas em **f32** e **f64**. Não cobre Int nem
Rational. Isso significa:

- **`Tensor<Float>`**: `dot` delega para `matrixmultiply::sgemm`/`dgemm`.
  Performance próxima de OpenBLAS para matrizes médias e grandes.
- **`Tensor<Int>`**: `dot` usa loop próprio (acumulação i64). Correto mas
  sem otimização SIMD. Para Int grande,BigInt exigiria acumulação
  multi-princisão — sem atalho.
- **`Tensor<Rational>`**: `dot` usa loop próprio com BigRational. Exato
  mas lento. Sem SIMD possível.

O monomorphizador gera variantes distintas por tipo de elemento. O
dispatch acontece em compile-time — `Tensor<Float>` e `Tensor<Int>`
chamam funções FFI diferentes.

### 7.6. N-D e contração

`matrixmultiply` implementa GEMM (2D × 2D). Para tensors N-D (rank > 2),
a contração é reduzida a uma sequência de GEMMs:

1. **Reshape** o tensor N-D para 2D (merge dos eixos não contraídos)
2. **GEMM** via `matrixmultiply`
3. **Reshape** o resultado de volta para N-D

Isso é exatamente o que NumPy e PyTorch fazem internamente. O reshape é
zero-copy (apenas ajusta shape e strides no `kata_rt_tensor`).

### 7.7. Futuro: GPU e backend trocável

A arquitetura permite trocar o backend no futuro sem mudar a interface
TENSOR:

- **CPU (padrão)**: `matrixmultiply` — Rust puro, link estático
- **GPU AMD (futuro)**: rocBLAS/hipBLAS — link dinâmico com `/opt/rocm/lib`
- **GPU NVIDIA (futuro)**: cuBLAS — link dinâmico com CUDA toolkit

A troca pode ser via diretiva `@backend("rocm")` ou inferência automática
(baseada em onde o tensor está alocado). O tipo `Ty::Tensor` e a interface
TENSOR não mudam — só a implementação de `dot` no runtime.

### 7.8. Por que não OpenBLAS linkado estaticamente

`openblas-src` permite compilar OpenBLAS from source e linkar
estaticamente. Isso daria performance máxima. Mas:

- **Build pesado**: OpenBLAS em C/Fortran, exige `gcc` e `gfortran` no
  build. Compila por vários minutos. O build do Kata é rápido hoje —
  adicionar OpenBLAS quebra isso.
- **Tamanho do binário**: OpenBLAS estático adiciona ~20MB ao binário.
  `matrixmultiply` adiciona ~50KB.
- **Complexidade de cross-compile**: OpenBLAS precisa de configuração
  por arquitetura (TARGET, HOSTCC). `matrixmultiply` é Rust puro —
  cross-compile funciona nativamente via `cargo build --target`.
- **Princípio I8**: "Sem dependências externas pesadas". OpenBLAS é
  pesada. `matrixmultiply` é leve e suficiente.

Se no futuro a performance do `matrixmultiply` for insuficiente para casos
específicos (matrizes muito grandes, HPC), a opção de linkar OpenBLAS
pode ser adicionada como feature opt-in — mas não como padrão.

---

## 8. Shapes Desconhecidos em Compile-Time

### 8.1. O problema

Sem const generics, `Ty::Tensor(Box<Ty>)` carrega apenas o tipo do elemento.
O shape (dimensões) mora no valor, não no tipo. Isso significa que o
compilador **não sabe** o shape de um tensor em geral.

Mas "não sabe" não é binário. Há três níveis de conhecimento:

### 8.2. Níveis de conhecimento de shape

**(1) Shape conhecido em compile-time**

```kata
let a := {1 2; 3 4}        # shape (2, 2) — literal, óbvio pela sintaxe
let b := {5 6; 7 8}        # shape (2, 2)
let c := dot a b            # (2,2)·(2,2) — válido, resultado (2,2)
let d := transpose a        # (2,2) → (2,2)
```

O compilador **tem** a informação. Os literais fixam o shape. Operações
sobre literais produzem shapes derivados. Tudo isso é visível sem
executar nada.

**(2) Shape parcialmente conhecido**

```kata
f :: Tensor<Float> => Tensor<Float>
lambda t:
    dot t t     # dentro de f, shape de t é desconhecido
```

O parâmetro `t` tem shape desconhecido. Mas dentro da função, se `t`
passa por operações, o compilador pode rastrear relações: `dot t t`
exige que o shape de `t` seja quadrado — mas não sabe se é.

**(3) Shape totalmente desconhecido (runtime)**

```kata
let data := ler_banco           # Array de tamanho runtime
let t := Tensor (r, c) data?    # shape (r, c) — r e c são runtime
let u := dot t t                # compilador não sabe se (r,c)·(r,c) é válido
```

O shape depende de I/O, input do usuário, ou computação runtime. O
compilador não pode verificar — tem que delegar para runtime.

### 8.3. A questão: rastrear ou ignorar?

O compilador pode adotar uma de duas posturas:

**Postura A — Rastrear (shape inference)**

O compilador mantém uma side table (análoga ao `TypeEnv`) com shapes
conhecidos para tensores locais. Quando vê um literal, anota o shape.
Quando vê uma operação, deriva o shape do resultado. Quando os shapes
são conhecidos nos dois lados de um `dot`, verifica compatibilidade em
compile-time.

- `dot {1 2; 3 4} {1 2 3; 4 5 6}` → **erro em compile-time** (2,2)·(2,3)
- `dot t t` onde `t` vem de I/O → gera runtime check, sem erro em compile-time
- `dot (dot a b) c` onde `a`, `b`, `c` são literais → tudo verificado em compile-time

**Custo:** novo pass de inference. `ShapeEnv` alongside `TypeEnv`. Cada
`TypedExpr` de tensor carrega um `Option<Shape>` — `Some` se conhecido,
`None` se desconhecido. Operações propagam: `dot(a, b)` com shapes
conhecidos produz shape conhecido; com um desconhecido produz
desconhecido.

**Ganho:** erros de shape em código literal são pegos antes de executar.
`dot {1 2; 3 4} {1 2 3; 4 5 6}` não precisa chegar ao runtime para falhar.

**Postura B — Ignorar (tudo runtime)**

Todo shape check é runtime, inclusive em literais. O compilador não
rastrea shapes. `dot` sempre valida em runtime.

- Mais simples — sem `ShapeEnv`, sem pass extra.
- Mas `dot {1 2; 3 4} {1 2 3; 4 5 6}` compila e falha em runtime com um
  erro que o compilador poderia ter pegado.

**Análise:** a Postura B me parece errada. O compilador **tem** a
informação — escolher ignorá-la degrada a experiência do usuário sem
ganho de simplicidade que justifique. A informação está literalmente na
AST: `{1 2; 3 4}` tem 2 linhas de 2 elementos.

### 8.4. Proposta: shape inference sem const generics

A proposta é rastrear shapes conhecidos sem exigir const generics:

1. **`ShapeInfo`** — sidecar ao `TypedExpr`, não ao `Ty`:

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
Dois tensores com shapes diferentes têm o mesmo `Ty` — `Tensor<Float>`.
A diferença está no `ShapeInfo`, que é local e não sobrevive fronteiras
de função.

2. **Propagação** — o pass de inference deriva shapes:

| Expressão | ShapeInfo |
|---|---|
| `{1 2; 3 4}` | `Known([2, 2])` |
| `dot a b` (a, b conhecidos) | `Known([m, n])` se a=[m,k], b=[k,n] |
| `dot a b` (um desconhecido) | `Unknown` |
| `transpose a` (conhecido) | `Known(reversed(shape))` |
| `+ a b` (conhecidos) | `Known(shape_a)` se shapes compatíveis |
| `t` (parâmetro de função) | `Unknown` |
| `Tensor (r, c) data` | `Unknown` (r, c são runtime) |

3. **Verificação** — quando o compilador conhece os shapes de ambos os
   operandos de `dot`, verifica compatibilidade:

```
dot a b  onde shape(a) = [m, k], shape(b) = [k', n]
  se k == k': OK, resultado [m, n]
  se k != k': ERRO em compile-time — "shape mismatch: k != k'"
```

4. **Limite** — shape inference é local, não sobrevive fronteiras:

```kata
f :: Tensor<Float> => Tensor<Float>
lambda t:
    # ShapeInfo(t) = Unknown — o tipo não carrega shape
    dot t t   # não dá para verificar em compile-time
```

Isso é inevitável sem const generics. A inferência pega o que pode —
literais e operações locais — e delega o resto para runtime.

### 8.5. Comportamento de `dot` com shape desconhecido

Quando o compilador não pode verificar (Nível 2 ou 3 acima), a decisão
sobre o que `dot` faz em runtime é a **D2**. As três opções:

**(a) `dot` retorna `Result`**

```kata
dot :: Tensor::T Tensor::T => Result::(Tensor::T, Text)
```

Toda chamada de `dot` exige `match` ou `?`. Em código numérico, isso é
verboso: `dot (dot a b) c` vira um exercício de pattern matching encadeado.

Alinhado com a filosofia de "sem exceções, `Result` para operações
falíveis". Mas o custo ergonômico é alto para o use case principal da
linguagem.

**(b) `dot` assume compatibilidade, erro é UB**

```kata
dot :: Tensor::T Tensor::T => Tensor::T
# Pre-condição: shapes compatíveis. Violar é UB.
```

Alinhado com a filosofia de tipos refinados. `NonZero` existe para
divisão segura — o programador garante ≠ 0 via tipo. Aqui, o
programador garante shapes compatíveis via `shape` antes de chamar.

`dot (dot a b) c` é direto, sem cerimônia. O custo é que um bug de shape
vira UB, não um erro diagnosticável.

**(c) `dot` valida em runtime e pânica**

```kata
dot :: Tensor::T Tensor::T => Tensor::T
# Panic se shapes incompatíveis.
```

Meio-termo. Erro é diagnosticável (mensagem de panic com shapes
envolvidos) mas não controlável (não há `Result` para tratar).

### 8.6. Acoplamento entre D2 e shape inference

As decisões são acopladas. Se há shape inference (Postura A), o cenário
onde `dot` precisa decidir o que fazer com shapes desconhecidos é mais
restrito:

- **Com shape inference:** `dot` com shapes conhecidos é verificado em
  compile-time. `dot` com shapes desconhecidos é o único caso runtime.
  O programador só paga o custo ergonômico de `Result` (ou o risco de UB)
  quando o shape é genuinamente runtime.

- **Sem shape inference:** todo `dot` é runtime, inclusive em literais.
  O custo ergonômico de `Result` cai em todas as chamadas.

Com shape inference, a opção (b) UB fica mais defensável: o compilador
já pega os erros óbvios (literais), e UB em shapes desconhecidos é o
mesmo contrato de `NonZero` — pré-condição do caller.

Sem shape inference, a opção (a) `Result` fica mais necessária: sem
verificação em compile-time, o programador precisa de uma forma de
tratar o erro em runtime.

### 8.7. Resumo da discussão

A questão de shapes desconhecidos não é "como verificar" — é "onde
verificar". A resposta tem duas partes:

1. **Compile-time onde possível:** shape inference rastreia shapes de
   literais e operações locais. `dot {1 2; 3 4} {1 2 3; 4 5 6}` é erro
   de compile-time, não de runtime.

2. **Runtime onde necessário:** shapes que dependem de I/O ou atravessam
   fronteiras de função são verificados em runtime. O comportamento de
   `dot` nesse caso é a decisão D2, acoplada à decisão de implementar
   shape inference.

A recomendação provisória é: implementar shape inference (Postura A,
§8.4) e adotar (b) UB para shapes desconhecidos (§8.5). O compilador
pega o que pode em compile-time; o programador é responsável por shapes
runtime, como é responsável por `NonZero` na divisão.

---

## 9. Decisões Pendentes

### D1. `{1 2 3}` (sem `;`) é Array ou Tensor 1-D?

**Opção A:** `{1 2 3}` é Array. Vetor tensor exige `;` final: `{1 2 3;}`.
- Pró: Arrays mantêm sintaxe atual. Zero breaking change.
- Contra: `{1 2 3;}` é estranho. `;` como sufixo vazio não é intuitivo.

**Opção B:** `{1 2 3}` é Tensor 1-D. Arrays literais mudam de sintaxe.
- Pró: vetores são naturais.
- Contra: breaking change. Todo `{...}` literal existente muda de tipo.
  Arrays precisariam de nova sintaxe (qual?).

**Opção C:** `{1 2 3}` é Array. Tensor exige pelo menos um `;` interno.
Vetor 1-D é `{1; 2; 3}` (vetor coluna) ou construído via `tensor [1 2 3]`.
- Pró: sem breaking change. Vetor coluna é natural.
- Contra: vetor linha não tem sintaxe literal direta.

### D2. `dot` retorna Result, é UB, ou pânica?

Ver §8.5 para análise completa. Resumo:
- (a) `Result` — seguro mas verboso
- (b) UB — alinhado com NonZero, mas perigoso
- (c) Panic — meio-termo

**Acoplada à decisão de shape inference (§8.4).** Com shape inference,
a opção (b) UB fica mais defensável — o compilador pega erros em
literais, e UB só aplica em shapes genuinamente runtime.

**Recomendação provisória:** shape inference + (b) UB.

### D3. Suportar aninhamento N-D na primeira versão?

Tensor 3-D exige `{ {1 2; 3 4}; {5 6; 7 8} }`. O parser precisa de
recursão. Implementar isso desde o início ou começar com 2-D flat?

Se 2-D first: `{1 2; 3 4}` é a sintaxe máxima. Tensor 3-D fica para depois.
O tipo `Ty::Tensor` já é N-D, mas o parser só aceita 2-D inicialmente.

### D4. Coerção Array → Tensor

```kata
let arr := {1 2 3 4}
let t := Tensor (2 2) arr     # construtor: shape + data → Result::Tensor
```

O construtor `Tensor (shape) data` pega um Array e um shape, valida que
`len(data) == product(shape)`, e retorna `Result::(Tensor, Text)`.

Se o shape não bate, `Err`. Se bate, `Ok Tensor`.

### D5. Tensor implementa ITERABLE?

Se sim, `for x in tensor` itera sobre elementos flattened (row-major).
Útil mas potencialmente confuso — iterar sobre linhas vs elementos?

**Proposta:** Tensor implementa ITERABLE sobre elementos flattened.
Para iterar sobre linhas, usar `slice` ou indexação explícita (futuro).

### D6. Racional como elemento de Tensor?

Rational é exato mas não tem SIMD. Tensor de Rational seria correto
matematicamente mas lento. Permitir ou restringir a Int/Float?

A interface NUM é implementada por Int, Float e Rational. Se Tensor
exige NUM, Rational é automaticamente permitido. O custo é de runtime,
não de correção.

---

## 10. Plano de Implementação (esboço)

### Fase 1: Fundação

1. **`kata-ast`**: Adicionar `Expr::TensorLit { dimensions: Vec<Vec<Spanned<Expr>>> }`
2. **`kata-core`**: Adicionar `Ty::Tensor(Box<Ty>)`. Atualizar `extract_type_name`,
   `TypeShape`, display, hash.
3. **`kata-parser`**: Reviver `parse_array_or_tensor` com suporte a aninhamento
   (recursão para N-D, não apenas 2-D).
4. **`kata-resolution`**: `resolve_type_expr` reconhece `Tensor::(T)` →
   `Ty::Tensor(Box<Ty>)`.
5. **`kata-rt`**: Implementar `kata_rt_tensor` struct e FFI functions básicas
   (`new`, `shape`, `rank`, `at`, `free`).

### Fase 2: Type Checking

6. **`kata-inference`**: Inference de `TensorLit` — todos os elementos devem
   implementar NUM, todas as linhas da mesma dimensão devem ter mesmo
   comprimento. Produz `Ty::Tensor(elem)`.
7. **`kata-inference`**: Restrição de elemento — `Ty::Tensor(T)` exige
   `T` implementa NUM.
8. **`kata-inference`**: Coerção Array → Tensor via construtor `Tensor shape data`.

### Fase 3: Interface TENSOR

9. **`stdlib/core.kata`**: Declarar `interface TENSOR::T` com assinaturas.
10. **`stdlib/core.kata`**: `Tensor::T implements TENSOR::T` com métodos `@ffi`.
11. **`kata-resolution`**: Registrar TENSOR no InterfaceRegistry. Registrar
    implementação de Tensor.
12. **`kata-inference`**: Dispatch de `+`, `*`, `dot` para TENSOR quando
    args são `Ty::Tensor`.

### Fase 4: Codegen

13. **`kata-codegen`**: Lowering de `TensorLit` → call `kata_rt_tensor_new`
    com dados contíguos e shape.
14. **`kata-codegen`**: Lowering de operações TENSOR → calls FFI.
15. **`kata-codegen`**: Representação ABI — Tensor é ponteiro na ABI
    (como Array, Text, Struct).

### Fase 5: Shape Inference

16. **`kata-core`**: Adicionar `ShapeInfo` (sidecar ao `TypedExpr`, não
    ao `Ty`). `Unknown`, `Known(Vec<usize>)`, `Symbolic(String)` (futuro).
17. **`kata-inference`**: Propagação de shapes — literais produzem
    `Known`, operações derivam, parâmetros produzem `Unknown`.
18. **`kata-inference`**: Verificação de compatibilidade em compile-time
    quando ambos os operandos de `dot` têm `Known` shapes. Erro de
    compile-time se `k != k'`.
19. **`kata-inference`**: Verificação de shapes para `+`, `*`
    (element-wise) — shapes devem ser iguais ou compatíveis com broadcast.

### Fase 6: Backend de Álgebra Linear

20. **`kata-rt/Cargo.toml`**: Adicionar `matrixmultiply = "0.3"` como
    dependência (Rust puro, link estático, sem `build.rs`).
21. **`kata-rt/src/tensor/`**: Implementar `dot` para Float64 e Float32
    via `matrixmultiply::dgemm`/`sgemm`.
22. **`kata-rt/src/tensor/`**: Implementar `dot` para Int e Rational
    (loops próprios — matrixmultiply não cobre).
23. **`kata-rt/src/tensor/`**: Implementar `+`, `*` (element-wise) — loops
    próprios.
24. **`kata-rt/src/tensor/`**: Implementar `transpose` — troca de strides
    (zero-copy) quando possível.
25. **`kata-rt/src/tensor/`**: Implementar contração N-D via reshape +
    GEMM (merge de eixos, GEMM, reshape de volta).

### Fase 7: Monomorphização e Tree Shaking

26. **`kata-monomorph`**: Instanciar `Tensor::(Int)`, `Tensor::(Float)`, etc.
    Naming: `Tensor_Int`, `Tensor_Float`.
27. **`kata-tree-shaking`**: Marcar FFI symbols de tensor como reachable.

### Fase 8: Testes

28. **`examples/`**: Migrar `test_tensor_math.kata` do Kata4.
29. **`examples/`**: Criar `examples/tensor_basic.kata` com operações
    elementares.
30. **`kata-codegen/tests/`**: Testes E2E de tensor add, mul, dot, transpose.
31. **`kata-rt/tests/`**: Testes de `dot` Float comparando resultado
    `matrixmultiply` vs loop próprio de referência (mesmo resultado).
32. **`kata-inference/tests/`**: Testes de shape inference — erros de
    shape em compile-time para literais, `Unknown` para parâmetros.

---

## 11. Histórico

### Design original (Kata4 / Specs OLD)

- Tensores como "família de elite" com const generics
- `Tensor::T::(Int...)` — shape no tipo, conhecido em compile-time
- DOT_BEHAVIOR: type-level validation de compatibilidade de shapes
- Tradução direta para SIMD no Cranelift
- Coerção Array → Tensor via construtor falível (retorna Result)
- Parser tinha `parse_array_or_tensor` que distinguia por `;`
- `Expr::Tensor { elements: Vec<Vec<Spanned<Expr>> }` no AST

### Estado no Kata5

- `Expr::Tensor` removido do AST
- `Ty::Tensor` não existe
- `parse_array_or_tensor` removido do parser
- Tensores listados em "Fora do Escopo 1.0" no ROADMAP e TODO
- `sintaxe-mapa.md` ainda lista `{1; 2; 3}` como "Tensor N-D" — fantasma
- `test_tensor_math.kata` não migrado — "Bug intencional de dot com shapes
  incompatíveis — decisão de design pendente"

### Este documento

Criado para revisitar a decisão de adiar tensores. A motivação original
da linguagem (computação numérica) não desapareceu. O design original era
ambicioso (const generics + DOT_BEHAVIOR + SIMD direto). Este documento
propõe um caminho mais pragmático: Tensor como tipo intrínseco com shape
runtime, sem const generics, evoluindo em direção ao design original
conforme o compilador amadurece.