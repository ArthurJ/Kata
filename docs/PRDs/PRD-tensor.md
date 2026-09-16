# PRD — Tensores

**Status:** 🔴 Pendente
**Depende de:** `kata_rt_array` ✅ (runtime base — modelo de buffer contíguo)

## 1. Objetivo

Implementar tensores como cidadão de primeira classe em Kata — tipo intrínseco `Ty::Tensor`, sintaxe literal dedicada, interface TENSOR, e runtime `kata_rt_tensor` com backend de álgebra linear auto-contido.

Kata foi concebida para computação numérica. Operações matriciais são a razão de existir da linguagem, não um recurso opcional. Dar a Arrays operações matemáticas é arbitrário — `* {1 2 3} {4 5 6}` não tem significado canônico. Tensores são objetos matemáticos com regras precisas: `+` é adição element-wise com broadcast, `*` é Hadamard, `dot` é contração.

## 2. Motivação

Hoje, `{1 2 3}` (Array) é um buffer contíguo sem álgebra. Multiplicar arrays é convenção, não matemática — sem um tipo distinto, `*` sobre coleções é ambíguo entre zip-multiply, produto escalar, e repetição.

A visão (`VISAO-tensor.md`) define o design completo. Este PRD traduz a visão em fases executáveis com DoDs e oráculos de teste.

## 3. Design

### 3.1. Sintaxe de literais

`;` dentro de `[]` é o discriminador List vs Tensor:

```
[1 2 3]          # List (Cons) — sem ;
[1 2 3;]         # Tensor 1×3 — ; terminal (vetor linha)
[1 2 3; 4 5 6]   # Tensor 2×3
[1; 2; 3]        # Tensor 3×1 — vetor coluna
```

`{1 2 3}` continua sendo Array. `{"k": v}` continua sendo Dict. `{|1 2 3|}` continua sendo Set.

Vírgula é equivalente a espaço dentro de `[]`:

```kata
[1, 2, 3; 4, 5, 6]  # mesmo tensor que [1 2 3; 4 5 6]
```

N-D via aninhamento — cada `[` inicia um novo nível:

```kata
[
    [1 2; 3 4];
    [5 6; 7 8]
]
```

### 3.2. Tipo

`Ty::Tensor(Box<Ty>)` — novo variant em `kata-core/src/ty.rs`.

- `Tensor::Int`, `Tensor::Float` — o tipo parametriza o elemento.
- `T` deve implementar NUM.
- Shape é armazenado no runtime, não no tipo — `Tensor::T` não carrega dimensões.

`Ty::Tensor` é intrínseco (não `data`) porque o compilador controla: sintaxe literal `[1 2; 3 4]`, restrição de elemento (NUM), dispatch de operadores, e ABI otimizada (SIMD).

### 3.3. Indexação

Sintaxe `.()` com tupla de índices — cada posição corresponde a um eixo:

```kata
let m := [1 2 3; 4 5 6]       # Tensor 2×3

m.(0 1)                        # elemento [0,1] → Result::Int
m.(0)                          # linha 0 inteira → Tensor 1×3
m.(0..2)                       # linhas 0 a 1 → Tensor 2×3
m.(0..2 1..3)                  # sub-matriz → Tensor 2×2
m.(_ 1)                        # todas as linhas, coluna 1 → Tensor 2×1
m.at(5)                        # elemento flatten no índice 5 → Result::T
```

**Princípio de retorno:** índices completos (todos os eixos como inteiros) → `Result::T` (pode estar out-of-bounds). Algum eixo sem índice completo (range, `_`, ou omitido) → `Tensor` (sempre válido).

`m.at(i)` é indexação 1-D (flatten) via interface INDEXABLE — mesma interface de Array/List.

### 3.4. Interface TENSOR

```kata
interface TENSOR::T
    shape     :: Self => Tuple
    rank      :: Self => Int
    at        :: Self Int => Result::T
    +         :: Self Self => Result::Self       # element-wise + broadcast
    *         :: Self Self => Result::Self       # Hadamard (element-wise)
    dot       :: Self Self => Result::Self       # contração
    transpose :: Self => Self                    # transposição (zero-copy)
    scale     :: Self T => Self                  # multiplicar por escalar
    shift     :: Self T => Self                  # somar escalar a cada elemento
```

`scale` e `shift` operam tensor-escalar — nunca falham, retornam `Self` direto. `transpose` nunca falha (zero-copy via troca de strides).

`+`, `*`, e `dot` retornam `Result::Self` — shape incompatível é falha recuperável (como `div :: Self Self => Result::Self`). `|` (fallback) e `?` (propagação) tornam o `Result` ergonômico.

### 3.5. Variantes pânicas

```kata
_+ :: Self Self => Self    # element-wise + broadcast, panic se shapes incompatíveis
_* :: Self Self => Self    # Hadamard, panic se shapes incompatíveis
```

Panic é determinístico (exit 1), não UB. Para quem tem certeza que shapes casam — não desempacotar `Result` toda vez.

### 3.6. Display

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

### 3.7. Runtime

```c
struct kata_rt_tensor {
    void*  data;        // buffer contíguo, row-major
    int64_t rank;        // ≥ 1 — tensores 0-D não existem
    int64_t* shape;     // [rank] dimensões
    int64_t* strides;   // [rank] strides em elementos (não bytes)
    PrimTy  elem_type;  // Int, Float
};
```

### 3.8. FFI

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

### 3.9. Backend de álgebra linear

`matrixmultiply` (Rust puro, `no-std`, microkernels SIMD SSE2/AVX/NEON, link estático, MIT/Apache-2.0) é o default. OpenBLAS é opt-in via Cargo feature.

| Kata | Implementação |
|---|---|
| `dot` 2D×2D (Float) | `dgemm`/`sgemm` |
| `dot` 1D×1D (Float) | loop próprio (O(N)) |
| `dot` 2D×1D (Float) | `dgemm` com N=1 |
| `dot` (Int) | loop próprio (matrixmultiply é só Float) |
| `+`, `*`, `scale`, `shift` | loop próprio |
| `transpose` | troca de strides (zero-copy) |

## 4. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | Tensor 0-D proibido | Escalar é `Int`/`Float`. `kata_rt_tensor_new` com `rank = 0` é erro. Todo tensor tem rank ≥ 1. Um tensor 0-D seria um wrapper inútil sobre o escalar nativo. |
| D2 | `[1 2 3]` sem `;` é List | Vetor tensor exige `;`: `[1 2 3;]`. Mantém compatibilidade retroativa com listas existentes — nenhum programa válido deixa de compilar. |
| D3 | Shapes em runtime, não compile-time | O type system não rastreia dimensões. Shape incompatível é `Err` (como `div` por zero). Rastrear shapes em compile-time exige size parameters, unificação de variáveis de `Int`, e constraints aritméticas — complexidade que não se justifica no design atual. |
| D4 | `Ty::Tensor` intrínseco, não `data` | Um `data Matrix` seria um Struct opaco sem controle sobre sintaxe literal, restrição de elemento (NUM), dispatch de operadores, e ABI otimizada (SIMD). |
| D5 | `matrixmultiply` default, OpenBLAS opt-in | Binário auto-contido, portátil entre Linux x64 e macOS (x64 e Apple Silicon) sem exigir BLAS externo. A FFI não muda — `kata_rt_tensor_dot` despacha internamente. |
| D6 | `+`/`*`/`dot` retornam `Result`, `_+`/`_*` retornam `Self` | Shape incompatível é falha recuperável. Quem quer ergonomia sem desempacotar usa as variantes pânicas (determinísticas, não UB). |
| D7 | `*` é Hadamard, não matmul | Matmul é `dot`. Hadamard é element-wise. Convenção de NumPy/Julia. Evita ambiguidade sobre o significado de `*`. |
| D8 | Aninhamento para N-D, não `;;`/`;;;` | Cada `[` inicia um nível. Não polui o lexer com tokens novos e o leitor vê a estrutura visualmente. |
| D9 | `scale`/`shift` tensor-escalar, retornam `Self` | Operações tensor-escalar nunca falham — o escalar é aplicado a cada elemento. Retornam `Self` direto, não `Result`. |
| D10 | `transpose` zero-copy | Troca de strides no header — não copia dados. Nunca falha, retorna `Self`. |

## 5. Fases

### Fase 1: Runtime `kata_rt_tensor`

Criar `crates/kata-rt/src/tensor.rs` com a struct `kata_rt_tensor` e todas as FFIs da §3.8.

- `kata_rt_tensor_new` valida `rank ≥ 1` (panic se 0), aloca header + shape + strides + data buffer.
- `kata_rt_tensor_add`/`mul`: broadcast (regras NumPy — right aligned, dims de tamanho 1 ou iguais).
- `kata_rt_tensor_panic_add`/`mul`: mesmas operações, panic em vez de retornar Err.
- `kata_rt_tensor_dot`: despacha para `matrixmultiply` (Float 2D) ou loop próprio (Int, 1D). N-D segue convenção NumPy (último de A × penúltimo de B).
- `kata_rt_tensor_transpose`: troca strides, reusa data buffer.
- `kata_rt_tensor_scale`/`shift`: loop próprio sobre elementos.
- `kata_rt_tensor_at`: indexação flatten com bounds check → result (ok/err via i64 tag).
- `kata_rt_tensor_shape`/`rank`: accessors.
- `kata_rt_tensor_free`: libera header + shape + strides + data.
- Adicionar `matrixmultiply` como dependência em `kata-rt/Cargo.toml`.

**DoD:** Todas as FFIs compilam e passam unit tests em Rust (alocação, add, mul, dot, transpose, scale, shift, at, free).

**Oráculos:**
- `tensor_new` com `rank=0` → panic.
- `add` de `[1 2; 3 4]` + `[5 6; 7 8]` → `[6 8; 10 12]`.
- `mul` (Hadamard) de `[1 2; 3 4]` * `[5 6; 7 8]` → `[5 12; 21 32]`.
- `dot` de `[1 2; 3 4]` · `[5 6; 7 8]` → `[19 22; 43 50]`.
- `dot` 1D: `[1 2 3]` · `[4 5 6]` → `32`.
- `transpose` de `[1 2 3; 4 5 6]` → `[1 4; 2 5; 3 6]`.
- `scale` de `[1 2; 3 4]` por `2` → `[2 4; 6 8]`.
- `shift` de `[1 2; 3 4]` por `10` → `[11 12; 13 14]`.
- `add` shapes incompatíveis (`[1 2; 3 4]` + `[1 2 3]`) → Err.
- `panic_add` shapes incompatíveis → panic.

### Fase 2: AST — `Ty::Tensor` + `Expr::TensorLit`

- Adicionar `Tensor(Box<Ty>)` em `kata-core/src/ty.rs`.
- Adicionar `TensorLit { rows: Vec<Vec<Spanned<Expr>>>, trailing_semi: bool }` em `kata-ast/src/expr.rs`.
- Adicionar `Tensor` em `TypeExpr::Named` resolution (`Tensor::Int` → `Ty::Tensor(Box::new(Ty::Prim(PrimTy::Int)))`).
- Mapeamento de ABI: `Ty::Tensor` → ponteiro (como `Ty::Array`, `Ty::Text`).

**DoD:** `cargo check --workspace` compila com os novos variants. Parser ainda não produz `TensorLit` (Fase 3).

### Fase 3: Parser — `;` como discriminador

- Em `parse_list_or_range` (`kata-parser/src/expr_containers.rs`): após parsear o primeiro elemento, se o próximo token é `;`, mudar para modo Tensor — coletar linhas separadas por `;`.
- `;` terminal (`[1 2 3;]`): uma linha, `trailing_semi = true` → Tensor 1×N.
- N-D via aninhamento: se um elemento de uma linha é `[`, parsear recursivamente como TensorLit aninhado.
- Vírgula equivalente a espaço dentro de `[]` (já é o caso para List — manter para Tensor).
- Sem `;` → ListLit (comportamento atual inalterado).

**DoD:** Parser produz `Expr::TensorLit` para `[1 2 3; 4 5 6]`, `[1 2 3;]`, `[1; 2; 3]`, e mantém `Expr::ListLit` para `[1 2 3]`.

**Oráculos:**
- `[1 2 3]` → `ListLit` (sem mudança).
- `[1 2 3; 4 5 6]` → `TensorLit` com 2 rows.
- `[1 2 3;]` → `TensorLit` com 1 row, `trailing_semi = true`.
- `[1; 2; 3]` → `TensorLit` com 3 rows de 1 elemento.
- `[1, 2, 3; 4, 5, 6]` → mesmo `TensorLit` que sem vírgulas.
- N-D aninhado: `[[1 2; 3 4]; [5 6; 7 8]]` → `TensorLit` com 2 rows, cada row contém `TensorLit`.

### Fase 4: Inference — typeck de `TensorLit` + interface TENSOR

- Em `kata-inference/src/infer/expr.rs`: novo braço para `TensorLit`.
  - Inferir tipo de cada elemento; unificar para `T`.
  - Validar que `T` implementa NUM (via `type_implements`).
  - Validar que todas as rows têm o mesmo número de colunas (shape consistency em compile-time quando possível — erro se número de colunas difere entre rows do mesmo nível).
  - Retornar `Ty::Tensor(Box<Ty>)`.
- Registrar interface TENSOR no prelude com todas as assinaturas da §3.4.
- Implementar dispatch de `+`, `*`, `dot`, `transpose`, `scale`, `shift`, `shape`, `rank`, `at` para `Ty::Tensor`.
- Implementar `_+`, `_*` como variants pânicos.
- `m.(i j)` — indexação N-D via `.()` com tupla: novo caso em DotAccess ou novo node de indexação. Cada índice pode ser `Int`, `Range`, ou `_` (wildcard). Retorno `Result::T` ou `Tensor` conforme §3.3.
- `m.at(i)` — despacha via INDEXABLE (já existe para Array/List).

**DoD:** `cargo test --workspace --no-fail-fast` passa. Typeck infere `Tensor::Int` para `[1 2 3; 4 5 6]`, rejeita `[1 2; 3 4 5]` (colunas inconsistentes), rejeita `["a" 1; 2 3]` (elemento não-NUM).

**Oráculos:**
- `[1 2 3; 4 5 6]` → `Tensor::Int`.
- `[1.0 2.0; 3.0 4.0]` → `Tensor::Float`.
- `[1 2; 3 4 5]` → erro compile-time (colunas inconsistentes).
- `+ [1 2; 3 4] [5 6; 7 8]` → `Result::(Tensor::Int)`.
- `dot [1 2; 3 4] [5 6; 7 8]` → `Result::(Tensor::Int)`.
- `shape [1 2 3; 4 5 6]` → `Tuple`.
- `rank [1 2 3; 4 5 6]` → `Int`.
- `[1 2 3; 4 5 6].(0 1)` → `Result::Int`.
- `[1 2 3; 4 5 6].(0)` → `Tensor::Int`.

### Fase 5: Codegen — lowering de `TensorLit` + operações

- Em `kata-codegen/src/lowering/collections_literal.rs`: novo braço para `TensorLit`.
  - Para cada elemento, lowerar e armazenar no buffer contíguo.
  - Calcular shape a partir da estrutura do literal.
  - Chamar `kata_rt_tensor_new(data, rank, shape)`.
- Lowering de operações TENSOR: cada operação chama a FFI correspondente.
  - `+`/`*`/`dot` → `kata_rt_tensor_add`/`mul`/`dot` → retorna result (ok/err tag).
  - `_+`/`_*` → `kata_rt_tensor_panic_add`/`mul` → retorna tensor* direto.
  - `transpose` → `kata_rt_tensor_transpose`.
  - `scale`/`shift` → `kata_rt_tensor_scale`/`shift`.
  - `shape`/`rank` → `kata_rt_tensor_shape`/`rank`.
  - `at` → `kata_rt_tensor_at`.
- Registrar todas as FFIs de tensor em `ffi_registry.rs` e `ffi_sigs/`.
- Indexação `.()`: lowerar para chamada FFI que extrai sub-tensor ou elemento.
- `Ty::Tensor` → ABI ponteiro (como `Ty::Array`).

**DoD:** `cargo test --workspace --no-fail-fast` passa. Programas Kata com tensores compilam e executam.

**Oráculos (E2E):**
- `echo!([1 2 3; 4 5 6])` imprime o tensor formatado.
- `echo!(show (+ [1 2; 3 4] [5 6; 7 8]))` → `[6 8; 10 12]`.
- `echo!(show (dot [1 2; 3 4] [5 6; 7 8]))` → `[19 22; 43 50]`.
- `echo!(show (transpose [1 2 3; 4 5 6]))` → `[1 4; 2 5; 3 6]`.
- `echo!(show (scale [1 2; 3 4] 2))` → `[2 4; 6 8]`.
- `echo!(show (shift [1 2; 3 4] 10))` → `[11 12; 13 14]`.
- `echo!(show [1 2 3; 4 5 6].(0 1))` → `2` (Result::Ok).
- `echo!(show [1 2 3; 4 5 6].(0))` → `[1 2 3]` (Tensor 1×3).

### Fase 6: Display

- Em `kata-rt/src/display.rs`: adicionar `TYPE_TENSOR` tag.
- Implementar formatação tabular: largura de coluna dinâmica, células centralizadas, espaços entre colunas.
- Rank > 2: fatias 2-D separadas por linha em branco com índice `[k]:`.
- Integrar com `show` — `show` de `Tensor::T` delega para o display do runtime.

**DoD:** `echo!([1 22 333; 4444 55 6; 7 88888 99])` produce o output formatado da §3.6.

**Oráculos:**
- `echo!([1 22 333; 4444 55 6; 7 88888 99])` → colunas centralizadas, largura dinâmica.
- `echo!([1 2; 3 4])` → `1 2\n3 4` (simples, sem bordas).
- Tensor 3-D: fatias separadas por `[0]:` e `[1]:`.

### Fase 7: Snapshot + testes E2E

- Criar testes E2E em `crates/kata-driver/tests/` cobrindo:
  - Literais (1-D, 2-D, 3-D, vetor linha, vetor coluna, aninhamento).
  - Operações (`+`, `*`, `dot`, `transpose`, `scale`, `shift`).
  - Variantes pânicas (`_+`, `_*`).
  - Indexação (`.()` com int, range, wildcard; `.at()`).
  - Display (formatação, rank > 2).
  - Erros (shapes incompatíveis, out-of-bounds, colunas inconsistentes, elemento não-NUM).
- Snapshot tests com `cargo insta` para output de display.

**DoD:** `cargo test --workspace --no-fail-fast` passa com 0 falhas. `cargo insta accept` executado para novos snapshots.

## 6. Estruturas afetadas

| Camada | Crate | Mudança |
|---|---|---|
| AST | `kata-ast` | `Expr::TensorLit`, `TypeExpr` reconhece `Tensor` |
| Tipos | `kata-core` | `Ty::Tensor(Box<Ty>)` |
| Parser | `kata-parser` | `parse_list_or_range` bifurca em `;` |
| Inference | `kata-inference` | Typeck de `TensorLit`, dispatch TENSOR, indexação `.()` |
| Codegen | `kata-codegen` | Lowering de `TensorLit` + operações, registro de FFIs |
| Runtime | `kata-rt` | `tensor.rs` (struct + FFIs), `display.rs` (TYPE_TENSOR) |
| Prelude | `stdlib/` | Interface TENSOR, operadores `_+`/`_*` |
| Docs | `docs/` | Manual, book, sintaxe-mapa, mapa-funcionalidades |

## 7. Fora do escopo

- **Shape no tipo (size parameters)** — rastreamento de dimensões em compile-time. Exige size params no type system, unificação de variáveis de `Int`, e constraints aritméticas. Futuro PRD próprio.
- **Coerção Array → Tensor** — `Tensor::Int::(2 2) arr` construtor. Futuro.
- **ITERABLE para Tensor** — iterar sobre elementos flattened. Futuro.
- **Broadcast explícito** — função `broadcast` para reshape com expansão. Apenas broadcast implícito em `+`/`*` por enquanto.
- **`contract` com eixos explícitos** — contração N-D não-convencional. `dot` usa convenção fixa (NumPy) por enquanto.
- **OpenBLAS** — feature opt-in. A infraestrutura (Cargo feature + dispatch interno) fica pronta, mas o backend OpenBLAS é futuro.
- **Slicing com step** — `m.(0..2..1)` com step explícito. Ranges sem step por enquanto.

## 8. Riscos

| Risco | Mitigação |
|---|---|
| Parser: `;` como discriminador quebra código existente que usa `;` dentro de `[]` | `;` dentro de `[]` não é válido hoje (ListLit não usa `;`). A mudança é aditiva — só introduz um novo caso, não altera o existente. |
| N-D aninhamento: parser precisa distinguir `[[1 2; 3 4]; [5 6; 7 8]]` de lista de listas | O `;` no nível externo força TensorLit. Elementos que são `[` são parseados recursivamente — se contêm `;`, são TensorLit aninhado; senão, ListLit (erro em tempo de typeck — Tensor não contém List). |
| `dot` N-D: convenção NumPy (último × penúltimo) pode confundir usuários esperando matmul 2-D | Para 2-D, `dot` é matmul (comportamento esperado). Para N-D, documentar claramente a convenção. `contract` com eixos explícitos é futuro. |
| `matrixmultiply` adiciona dependência ao runtime | É Rust puro, `no-std` compatível, link estático, ~50KB. Aceitável para um runtime que já tem `num-rational`, `num-bigint`. |
| Indexação `.()` colide com `DotAccess` existente (field, int, range) | `.()` com parênteses é sintaxe nova — distinta de `.nome` (field) e `.0` (int). Novo `DotIndex` variant ou novo node de indexação. |

## 9. Documentação

Ao concluir:
- `docs/base/Kata-lang-manual.md` — nova seção sobre Tensor: sintaxe literal, tipo, interface, indexação, display.
- `docs/base/kata-book/` — capítulo sobre tensores (álgebra linear como caso de uso central da linguagem).
- `docs/base/sintaxe-mapa.md` — entrada para `[...; ...]` (Tensor) na tabela de literais.
- `docs/base/mapa-funcionalidades.md` — entrada para Tensor na árvore de dependências.
- `docs/TODO.md` — marcar Tensor como concluído, remover entrada de futuro.
- `docs/VISAO-tensor.md` — marcar como implementado ou remover (conteúdo migrado para manual/book).