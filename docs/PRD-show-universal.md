# PRD — SHOW Universal: Array, Set, Dict, Tuple

**Status:** Rascunho
**Data:** 2026-08-13
**Motivação:** `echo!(show x)` falha para Array, Set, Dict e Tuple. O `show_synthesis`
só cobre Structs, Enums e List. Coleções estruturais não têm `show` no DispatchTable,
impedindo que `echo!` (que exige `SHOW`) funcione com esses tipos.

---

## 1. Problema

`echo!` exige `SHOW`. O `show_synthesis` sintetiza `show` para:

- ✅ Structs (com/sem campos, refineds)
- ✅ Enums (genéricos e não-genéricos)
- ✅ List (recursivo, via `show_synthesis_list.rs`)

Não sintetiza para:

- ❌ Array `{1 2 3}` — `Ty::Array(Box<Ty>)`
- ❌ Set `{|1 2 3|}` — `Ty::Set(Box<Ty>)`
- ❌ Dict `{"k": v}` — `Ty::Dict(Box<Ty>, Box<Ty>)`
- ❌ Tuple `(1, "a", True)` — `Ty::Tuple(Vec<Ty>)`

O `show_expr` em `show_synthesis_helpers.rs` **já sabe** produzir o body para Tuple
inline (linhas 228-262: `build_tuple_show_inline`), mas não há overload de `show`
no DispatchTable para Tuple, então o dispatch falha antes de chegar lá.

### Sintoma

```kata
echo!(show [1 2 3])          # ✅ funciona (List)
echo!(show {1 2 3})          # ❌ type.no_overload
echo!(show {|1 2 3|})        # ❌ type.no_overload
echo!(show {"k": "v"})       # ❌ type.mismatch (Dict literal é parseado como Array)
echo!(show (1, "a", True))   # ❌ type.no_overload
```

---

## 2. Abordagem

Duas opções foram consideradas:

### Opção A: FFI `kata_rt_repr_to_text` no runtime

Uma FFI única que recebe `ptr + type_id` e caminha o `TypeShape` para produzir
Text. O runtime já tem acesso aos dados e sabe iterá-los.

**Prós:**
- Uma única FFI cobre todos os tipos
- O runtime já tem TypeShape (para `typeof` e reflexão)
- Menos código na síntese TAST

**Contras:**
- `kata_rt_repr_to_text` e `pretty_print` **não existem** no runtime hoje
- Precisa implementar do zero no runtime: iterar Array, Set (HAMT), Dict
  (HAMT), Tuple (load por offset), com recursão para elementos aninhados
- O runtime não tem acesso ao `show` do elemento (precisaria de callback
  JIT → runtime, que não existe)
- TypeShape mapeia Array/Set/Dict para `TypeShape::Struct { name: "Array" }`
  — perde a informação do tipo do elemento. A FFI não saberia como mostrar
  `Array(List(Int))` sem saber que cada elemento é uma List
- Adia o problema: `pretty_print` precisaria reimplementar `show` para cada
  tipo primitivo (BigInt, Rational, etc.) dentro do runtime

**Decisão: rejeitada.** O runtime não tem informação de tipo suficiente (TypeShape
descarta o tipo do elemento), e reimplementar `show` no runtime duplica lógica
que já existe na síntese TAST.

### Opção B: Síntese TAST (escolhida)

Estender `show_synthesis` para sintetizar funções `show` para Array, Set, Dict
e Tuple, da mesma forma que já faz para List.

**Prós:**
- Segue o padrão existente (List já faz isso)
- O `show_expr` já sabe despachar para tipos primitivos, structs, enums, listas
- Elementos aninhados são cobertos: `show (Array(List(Int)))` despacha
  `show` para List, que despacha `show` para Int
- Sem mudanças no runtime

**Contras:**
- Mais código na síntese (4 novos módulos ou extensão dos existentes)
- Tuple é estrutural (não nominal) — precisa de abordagem diferente

**Decisão: adotada.**

---

## 3. Design

### 3.1. Tuple — Overload Genérico no DispatchTable

Tuple é estrutural, não nominal. Não está no `StructRegistry` nem no
`EnumRegistry`. A síntese atual itera sobre registries de tipos nomeados.

**Abordagem:** Registrar um overload genérico de `show` para `Tuple(T1, T2, ...)`
no DispatchTable. O body é `build_tuple_show_inline` (já existe em
`show_synthesis_helpers.rs`).

O problema: Tuple tem aridade variável. `Ty::Tuple(Vec<Ty>)` pode ter 0, 1, 2,
3, ... elementos. Cada aridade é um tipo distinto. Mas o `build_tuple_show_inline`
já aceita `&[Ty]` de qualquer tamanho.

**Síntese:** Para cada aridade N encontrada no programa (não para todas as
aridades possíveis), registrar um overload `show :: Tuple(T1, ..., TN) => Text`.

A detecção de aridades usadas pode ser feita no inference pass, coletando
todos os `Ty::Tuple(...)` que aparecem na TAST e registrando overloads
sob demanda.

Alternativa mais simples: registrar um único overload genérico com
`type_params: ["T1", "T2", ...]` — mas o número de type params é variável,
o que não encaixa no modelo atual de type_params de tamanho fixo.

**Solução pragmática:** Registrar overloads de `show` para tuplas de aridade
1, 2, 3, 4, 5, 6, 7, 8 (cobertura prática) com type params `T1..TN`, no
momento da síntese. O monomorphizador instancia quando encontra um call
site `show` com `Tuple(Int, Text)` → resolve o overload de aridade 2.

### 3.2. Array — Síntese Recursiva como List

Array é `Ty::Array(Box<Ty>)` — contíguo, indexável O(1). A síntese é
mais simples que List: não precisa de recursão entre duas funções (Array
não é Cons/Nil, é contíguo com len).

**Body:**
```kata
__kata_show__Array :: Array::A => Text
lambda __self:
    # Se len == 0: "[]"
    # Senão: "[" + show(self.0) + ", " + show(self.1) + ... + "]"
```

Mas Array é indexável via `at` (retorna `Result`), não via `.N` direto
(como Tuple). A síntese precisa:
1. `len __self` — obtém o tamanho
2. Para cada índice i de 0 a len-1: `match (at __self i) { Ok v: show v, ... }`
3. Concatenar tudo com `", "` entre elementos

Isso exige um loop, que **não existe em funções puras**. A síntese de List
resolve isso com recursão (Cons/Nil pattern matching). Array não é recursivo.

**Alternativa:** Usar `ITERABLE` — Array implementa `ITERABLE(A)` com
`next :: Array::A => Optional::A`. A síntese pode usar `next` recursivamente:

```kata
__kata_show__Array :: Array::A => Text
lambda __self:
    match (next __self)
        Optional::Some(h): "[" + show h + __kata_show__Array_rest __self
        Optional::None: "[]"

__kata_show__Array_rest :: Array::A => Text
lambda __self:
    match (next __self)
        Optional::Some(h): ", " + show h + __kata_show__Array_rest __self
        Optional::None: "]"
```

Mas há um problema: `next` é `@ffi("kata_rt_array_next")` — mantém cursor
interno (estado mutável no handle). Chamar `next` duas vezes na mesma
instância avança o cursor. A síntese não pode assumir que `next` é
idempotente.

**Solução:** Para Array, usar `at` (indexação por índice) com recursão
por índice:

```kata
__kata_show__Array :: Array::A => Text
lambda __self:
    match (at __self 0)
        Result::Ok(h): "[" + show h + __kata_show__Array_rest __self 1
        Result::Err(_): "[]"

__kata_show__Array_rest :: Array::A Int => Text
lambda __self i:
    match (at __self i)
        Result::Ok(h): ", " + show h + __kata_show__Array_rest __self (+ i 1)
        Result::Err(_): "]"
```

Isto é recursão em cauda (TCO aplicável). `at` é `@ffi("kata_rt_array_get_checked")`
que retorna `Result::(A, Err)`. O `Result::Err` sinaliza fim (índice out of bounds).

**Atenção:** `at` sobre Array é O(1). A recursão é O(n) em profundidade, mas
TCO a transforma em loop. O `Result::Err` como sentinela de fim é funcional
mas semanticamente estranho — `Err` significa "out of bounds", não "fim da
iteração". Alternativa: usar `len __self` e comparar índice:

```kata
__kata_show__Array :: Array::A => Text
lambda __self:
    match (= 0 (len __self))
        Boolean::True: "[]"
        Boolean::False: "[" + show (at __self 0 ?) + __kata_show__Array_rest __self 1

__kata_show__Array_rest :: Array::A Int => Text
lambda __self i:
    match (= i (len __self))
        Boolean::True: "]"
        Boolean::False: ", " + show (at __self i ?) + __kata_show__Array_rest __self (+ i 1)
```

O `?` desempacota `Result` — mas `?` é exclusivo de Actions. Em funções
puras, usar `match` explícito:

```kata
__kata_show__Array_rest :: Array::A Int => Text
lambda __self i:
    match (= i (len __self))
        Boolean::True: "]"
        Boolean::False:
            match (at __self i)
                Result::Ok(h): ", " + show h + __kata_show__Array_rest __self (+ i 1)
                Result::Err(_): "]"
```

**Decisão:** Usar `len + at` com match em `Result`. É mais código mas
semanticamente correto e compatível com funções puras.

### 3.3. Set — Síntese Recursiva via ITERABLE

Set é `Ty::Set(Box<Ty>)`. Não é indexável (não implementa INDEXABLE). É
ITERABLE. A única forma de iterar é `next :: Set::A => Optional::A`.

**Problema:** Set implementa ITERABLE com `next` que consome (move cursor).
O mesmo problema de Array — mas Set não tem `at` para indexação alternativa.

**Solução:** A síntese para Set precisa criar uma **cópia** do Set antes de
iterar, ou o `next` original consumiria o Set do chamador.

Mas Set é imutável (HAMT persistente). O `next` retorna `Optional::A` e
presumably avança um cursor interno no handle. Se o handle é a mesma
referência, chamar `next` move o cursor.

**Verificar:** Como `kata_rt_set_next` funciona? Se cria um novo iterador
a cada chamada, o problema não existe. Se mantém estado no próprio Set
(ao lado dos dados), é um problema.

**Investigação necessária:** Verificar a implementação de
`kata_rt_set_next` no runtime antes de decidir a abordagem.

### 3.4. Dict — Síntese Recursiva via ITERABLE

Dict é `Ty::Dict(Box<Ty>, Box<Ty>)`. ITERABLE sobre pares `(K, V)`. Mesmo
problema de Set: iteração via `next` com cursor.

**Output esperado:** `{"k1": v1, "k2": v2}` ou `{k1: v1, k2: v2}`?

O sintaxe-mapa diz que Dict literal é `{"k": v}`. O `show` deve produzir
formato legível: `{"k1": v1, "k2": v2}` (chaves como show K, valores como
show V).

### 3.5. Ordem de implementação

1. **Tuple** — mais simples (já tem `build_tuple_show_inline`), só precisa
   registrar overloads no DispatchTable
2. **Array** — `len + at` com recursão por índice
3. **Set** — investigar `kata_rt_set_next` e implementar
4. **Dict** — investigar `kata_rt_dict_next` e implementar

---

## 4. Estrutura de Arquivos

```
crates/kata-inference/src/infer/
├── show_synthesis.rs           # Structs + Enums (existente)
├── show_synthesis_helpers.rs   # Helpers (existente, estendido)
├── show_synthesis_list.rs      # List (existente)
├── show_synthesis_tuple.rs     # NOVO — Tuple overloads
├── show_synthesis_array.rs     # NOVO — Array (len + at)
├── show_synthesis_set.rs       # NOVO — Set (via ITERABLE)
├── show_synthesis_dict.rs      # NOVO — Dict (via ITERABLE)
└── mod.rs                      # Registro dos módulos (estendido)
```

---

## 5. Validação

Cada tipo deve passar nos seguintes testes:

```kata
# Tuple
echo!(show (1, "hello"))           # (1, hello)
echo!(show (1, 2, 3))              # (1, 2, 3)
echo!(show (42,))                  # (42,)
echo!(show ())                     # ()

# Array
echo!(show {1 2 3})               # [1, 2, 3]  (ou similar)
echo!(show {})                     # []

# Set
echo!(show {|1 2 3|})             # {|1, 2, 3|} (ou similar)

# Dict
echo!(show {"nome": "Ana"})       # {"nome": Ana} (ou similar)

# Aninhados
echo!(show [(1, "a") (2, "b")])   # [(1, a), (2, b)]
echo!(show {[(1 2) (3 4)]})       # [[1, 2], [3, 4]]
```

---

## 6. Decisões Pendentes

1. **Formato de saída de Set:** `{|1, 2, 3|}` (sintaxe literal) ou `{1, 2, 3}`
   (sintaxe simplificada)? O `show` de List usa `[1, 2, 3]` (sintaxe literal).
   Set deveria usar `{|1, 2, 3|}` para ser consistente.

2. **Formato de saída de Dict:** `{"k": v}` (sintaxe literal com chaves
   entre aspas) ou `{k: v}` (chaves sem aspas)? Se a chave é Text, as aspas
   ajudam a distinguir. Se a chave é Int, `{"1": "v"}` é estranho.
   Decisão: usar `show k` para a chave (sem aspas extras) — `show` de Text
   já não inclui aspas, então `{nome: Ana}` é o output natural.

3. **Set/Dict iteração com cursor:** Verificar se `kata_rt_set_next` e
   `kata_rt_dict_next` consomem o iterável ou criam novo iterador. Se
   consomem, a síntese precisa copiar o handle antes de iterar.

4. **Tuple aridade máxima:** Registrar overloads para aridades 0-8 é
   suficiente? Tuplas de 9+ elementos são raras. Se aparecerem, o
   dispatch falha graciosamente (type.no_overload) — o usuário recebe
   mensagem de erro clara.

---

## 7. Não-escopo

- `kata_rt_repr_to_text` / `pretty_print` no runtime — não será implementado
- `show` para tipos exóticos: Channel, Queue, Broadcast, Range, Function,
  OverloadSet — fora do escopo
- `show` para Bytes — já implementado (`show :: Bytes => Text`)
- Personalização de formato pelo usuário — o `show` sintetizado é fixo