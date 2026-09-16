# Visão — Tensor no Kata

**Status:** Design pronto. Implementação depende de `kata_rt_tensor` (runtime básico).

**Motivação:** Kata foi concebida para computação numérica. Operações matriciais são a razão de existir da linguagem — não um recurso opcional.

---

## 1. Por que Tensores, não Arrays

Arrays são coleções: contíguos, homogêneos, tamanho dinâmico. Iterar, indexar, contar — comportamento de coleção. Multiplicar, transpor, contrair — álgebra.

Dar a Arrays operações matemáticas é arbitrário: qual o significado de `* {1 2 3} {4 5 6}`? Zip-multiply? Produto escalar? Repetição? Sem um tipo matemático distinto, `*` sobre Arrays é convenção, não matemática.

Tensores são objetos matemáticos com regras precisas:

- **`+`** é adição element-wise com broadcast (shapes compatíveis)
- **`*`** é multiplicação element-wise (Hadamard) — NÃO produto matricial
- **`dot`** é contração (matmul para 2D, contração de índices para N-D)
- **`shape`** retorna as dimensões
- **`at`** extrai elemento por índice flatten

---

## 2. Sintaxe de Literais

### 2.1. Desambiguação List vs Tensor

```
[1 2 3]          # List (Cons, persistente) — sem ;
[1 2 3; 4 5 6]   # Tensor 2×3 — com ; separando linhas
[1 2 3;]         # Tensor 1×3 — ; terminal opcional (vetor linha)
[1; 2; 3]        # Tensor 3×1 — vetor coluna
```

`;` dentro de `[]` é o discriminador: há `;` em qualquer posição → Tensor. Sem `;` → List.

`{1 2 3}` continua sendo Array. `{"k": v}` continua sendo Dict. `{|1 2 3|}` continua sendo Set.

### 2.2. Vírgula como separador opcional

Dentro de `[]`, vírgula é equivalente a espaço. A semântica é determinada pelo delimitador: `(1, 2, 3)` é tupla; `[1, 2, 3]` é tensor/lista.

```kata
[1 2 3; 4 5 6]       # espaços
[1, 2, 3; 4, 5, 6]  # vírgulas (mesmo tensor)
[+ x 1, * y 2; - z 3, / w 4]  # legibilidade com expressões longas
```

### 2.3. N-D via aninhamento

```kata
[
    [1 2; 3 4];
    [5 6; 7 8]
]
```

Cada `[` inicia um novo nível. `;` no nível externo separa fatias do eixo 0; `;` no nível interno separa linhas dentro de cada fatia. Aninhamento é preferido a `;;` `;;;` (Julia) — não polui o lexer com tokens novos e o leitor vê a estrutura visualmente.

### 2.4. Tabela comparativa

| Propriedade | List `[1 2 3]` | Tensor `[1 2 3; 4 5 6]` |
|---|---|---|
| Sintaxe | `[]` sem `;` | `[]` com `;` |
| Vírgula opcional | ✅ | ✅ |
| Topologia | Encadeada (Cons, partilha) | Contíguo (row-major) |
| Dimensionalidade | 1-D | N-D |
| Álgebra linear | ❌ | ✅ (`+`, `*`, `dot`) |
| SIMD | ❌ | ✅ |
| Tipo | `List::T` | `Tensor::T` |

---

## 3. Tipo

```
Tensor::T   # Tensor::Int, Tensor::Float, etc.
```

- `T` deve implementar NUM.
- Shape é armazenado no runtime, não no tipo — `Tensor::T` não carrega dimensões.
- Se futuro demandar shape no tipo, `Ty::Tensor` pode estender para `Ty::Tensor(Box<Ty>, Vec<Ty>)` sem const generics.

`Ty::Tensor` é intrínseco (não `data`) porque só assim o compilador controla: sintaxe literal `[1 2; 3 4]`, restrição de elemento (NUM), dispatch de operadores, e ABI otimizada (SIMD). Um `data Matrix` seria um Struct opaco sem nenhum desses.

---

## 4. Indexação

### 4.1. Sintaxe `.(...)`

Indexação N-D usa `.()` com tupla de índices — cada posição corresponde a um eixo.

```kata
let m := [1 2 3; 4 5 6]       # Tensor 2×3

m.(0 1)                        # elemento [0, 1] → Result::Int
m.(0)                          # linha 0 inteira → Tensor 1×3
```

### 4.2. Slicing com `..`

```kata
m.(0..2)                      # linhas 0 a 1 → Tensor 2×3
m.(0..2 1..3)                 # sub-matriz → Tensor 2×2
m.(0..=1 0..=2)               # inclusivo
```

### 4.3. Wildcard `_`

`_` significa "todas as posições deste eixo":

```kata
m.(0 _)                       # linha 0, todas as colunas → Tensor 1×3
m.(_ 1)                       # todas as linhas, coluna 1 → Tensor 2×1
m.(_ 0..2)                    # todas as linhas, colunas 0-1 → Tensor 2×2
```

### 4.4. Indexação 1-D (flatten)

```kata
m.at(5)                       # elemento flatten no índice 5 → Result::T
```

Via interface INDEXABLE (como Array/List).

### 4.5. Regras de retorno

| Expressão | Eixo 0 | Eixo 1 | Retorno |
|---|---|---|---|
| `m.(0 1)` | índice 0 | índice 1 | `Result::T` (elemento) |
| `m.(0)` | índice 0 | — | `Tensor` (sub-tensor 1×3) |
| `m.(0..2 1)` | range 0..2 | índice 1 | `Tensor` (sub-tensor 2×1) |
| `m.(_ 1)` | todas | índice 1 | `Tensor` (sub-tensor 2×1) |
| `m.at(5)` | flatten 5 | — | `Result::T` (elemento) |

**Princípio:** índices completos (todos os eixos como inteiros) → `Result::T` (pode estar out-of-bounds). Algum eixo sem índice completo (range, `_`, ou omitido) → `Tensor` (sempre válido).

---

## 5. Display

`show` exibe tensores em formato tabular. Largura de coluna dinâmica (baseada no maior elemento), células centralizadas com padding, espaços entre colunas — sem bordas, `|`, ou `-`.

```kata
echo!([1 22 333; 4444 55 6; 7 88888 99])
```
```
    1    22    333
 4444    55      6
    7 88888    99
```

Rank > 2: cada fatia 2-D ao longo do eixo 0, separadas por linha em branco com índice `[k]:`.

Tensores 0-D não existem — escalar é o tipo nativo (`Int`, `Float`). Todo tensor tem rank ≥ 1.

---

## 6. Interface TENSOR

```kata
interface TENSOR::T
    shape     :: Self => Tuple
    rank      :: Self => Int
    at        :: Self Int => Result::T
    +         :: Self Self => Result::Self       # element-wise + broadcast
    *         :: Self Self => Result::Self       # Hadamard (element-wise)
    dot       :: Self Self => Result::Self       # contração
    transpose :: Self => Self                    # transposição
    scale     :: Self T => Self                  # multiplicar por escalar
    shift     :: Self T => Self                  # somar escalar a cada elemento
```

`scale` e `shift` operam tensor-escalar — nunca falham, retornam `Self` direto.

### Variantes pânicas (_+ / _*)

```kata
_+ :: Self Self => Self    # element-wise + broadcast, panic se shapes incompatíveis
_* :: Self Self => Self    # Hadamard, panic se shapes incompatíveis
```

Para quem tem certeza que shapes casam — não desempacotar `Result` toda vez. Panic é determinístico, não UB.

```kata
let c := _+ a b    # direto — panic se shapes não casam
let c := + a b ?   # equivalente seguro — propaga Err
```

### Tabela de operações

| Operação | Retorno | Falha quando |
|---|---|---|
| `+` | `Result::Self` | shapes não broadcastable |
| `*` | `Result::Self` | shapes não broadcastable |
| `dot` | `Result::Self` | inner dims não casam |
| `transpose` | `Self` | nunca (zero-copy) |
| `scale` | `Self` | nunca |
| `shift` | `Self` | nunca |
| `shape` | `Tuple` | nunca |
| `rank` | `Int` | nunca |
| `at` | `Result::T` | out-of-bounds |

`+`, `*`, e `dot` retornam `Result::Self` — shape incompatível é falha recuperável, como `div :: Self Self => Result::Self`. `|` (fallback) e `?` (propagação) tornam o `Result` ergonômico.

```kata
let c := + a b | zeros             # fallback se shapes não casam
let c := + a b ?                   # propaga erro na action
match + a b
    Ok result: echo!(show (shape result))
    Err msg: echo!(\"shapes incompatíveis: \" + msg)
```

---

## 7. Runtime

### Representação

```c
struct kata_rt_tensor {
    void*  data;        // buffer contíguo, row-major
    int64_t rank;        // ≥ 1 — tensores 0-D não existem
    int64_t* shape;      // [rank] dimensões
    int64_t* strides;    // [rank] strides em elementos (não bytes)
    PrimTy  elem_type;   // Int, Float
};
```

### FFI

```
kata_rt_tensor_new       (data, rank, shape) → tensor*
kata_rt_tensor_shape     (tensor*) → int64_t*
kata_rt_tensor_rank      (tensor*) → int64_t
kata_rt_tensor_at        (tensor*, int64_t) → result
kata_rt_tensor_add       (tensor*, tensor*) → result      # element-wise + broadcast
kata_rt_tensor_mul       (tensor*, tensor*) → result      # Hadamard
kata_rt_tensor_panic_add (tensor*, tensor*) → tensor*     # _+ (panic se incompatível)
kata_rt_tensor_panic_mul (tensor*, tensor*) → tensor*     # _* (panic se incompatível)
kata_rt_tensor_dot       (tensor*, tensor*) → result      # contração
kata_rt_tensor_transpose (tensor*) → tensor*              # zero-copy
kata_rt_tensor_scale     (tensor*, void* scalar) → tensor*
kata_rt_tensor_shift     (tensor*, void* scalar) → tensor*
kata_rt_tensor_free      (tensor*)
```

`dot` delega para `matrixmultiply` quando elemento é Float; para Int, loops próprios.

---

## 8. Backend de Álgebra Linear

O binário Kata deve ser auto-contido — portátil entre Linux x64 e macOS (x64 e Apple Silicon) sem exigir BLAS externo. A crate `matrixmultiply` resolve: Rust puro, `no-std` compatível, microkernels SIMD (SSE2/AVX/NEON), link estático, MIT/Apache-2.0.

| Plataforma | Suporte |
|---|---|
| Linux x86_64 | ✅ SSE2/AVX |
| macOS x86_64 (Intel) | ✅ SSE2/AVX |
| macOS aarch64 (Apple Silicon) | ✅ NEON |

### Mapeamento

| Kata | Implementação |
|---|---|
| `dot` 2D×2D (Float) | `dgemm`/`sgemm` |
| `dot` 1D×1D (Float) | loop próprio (O(N)) |
| `dot` 2D×1D (Float) | `dgemm` com N=1 |
| `dot` (Int) | loop próprio (matrixmultiply é só Float) |
| `+`, `*`, `scale`, `shift` | loop próprio |
| `transpose` | troca de strides (zero-copy) |

### Backend configurável

`matrixmultiply` é o default. OpenBLAS é opt-in via Cargo feature para performance máxima. A FFI não muda — `kata_rt_tensor_dot` despacha internamente.

| | matrixmultiply (default) | OpenBLAS (opt-in) |
|---|---|---|
| Performance GEMM Float | Boa | Máxima |
| Tamanho do binário | ~50KB | ~20MB |
| Dependência de build | Zero (Rust puro) | gcc + gfortran |
| Cross-compile | Nativa | Complexa |
| Threading | Single-thread | Multi-thread |

---

## 9. Decisões e Questões Abertas

### Decidido

- **Tensor 0-D proibido.** Escalar é `Int`/`Float`. `kata_rt_tensor_new` com `rank = 0` é erro. Todo tensor tem rank ≥ 1.
- **`[1 2 3]` sem `;` é List.** Vetor tensor exige `;`: `[1 2 3;]`.
- **Shapes em runtime, não compile-time.** O type system não rastreia dimensões. Shape incompatível é `Err` (como `div` por zero). Rastrear shapes em compile-time exige size parameters, unificação de variáveis de `Int`, e constraints aritméticas — complexidade que não se justifica no design atual.

### Aberto

- **N-D na primeira versão?** O tipo `Ty::Tensor` já é N-D, mas o parser pode aceitar só 2-D inicialmente.
- **Coerção Array → Tensor.** `Tensor::Int::(2 2) arr` — construtor valida `len(data) == product(shape)`, retorna `Result`.
- **ITERABLE?** Proposta: itera sobre elementos flattened (row-major). Para iterar sobre linhas, usar `.()` explícita.
- **Broadcast.** Adotar regras NumPy/Julia (right-aligned, dims de tamanho 1 ou iguais) — documentar quais shapes são broadcastable.
- **`dot` N-D.** Proposta: convenção fixa (último de A × penúltimo de B, como NumPy `dot`). Para contrações não-convencionais, função `contract` separada com eixos explícitos.