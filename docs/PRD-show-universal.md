# PRD — SHOW Universal: Array, Set, Dict, Tuple

**Status:** Implementação completa — Tuple ✅, Array ✅, Set ✅, Dict ✅, repr standalone ✅, Unit ✅.
**Data:** 2026-08-13 (atualizado 2026-08-14)
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

- ✅ Set `{|1 2 3|}` — `Ty::Set(Box<Ty>)` — IMPLEMENTADO via `show_synthesis_set.rs`
- ✅ Dict `{"k": v}` — `Ty::Dict(Box<Ty>, Box<Ty>)` — IMPLEMENTADO via `show_synthesis_dict.rs`
- ✅ Array `{1 2 3}` — IMPLEMENTADO via `show_synthesis_array.rs`
- ✅ Tuple `(1, "a", True)` — IMPLEMENTADO via interceptador + `tuple_show.rs`

### Sintoma (antes da implementação)

```kata
echo!(show [1 2 3])          # ✅ funcionava (List)
echo!(show {1 2 3})          # ❌ type.no_overload → ✅ RESOLVIDO
echo!(show {|1 2 3|})        # ❌ type.no_overload → ✅ RESOLVIDO
echo!(show {"k": "v"})       # ❌ → ✅ RESOLVIDO
echo!(show (1, "a", True))   # ❌ type.no_overload → ✅ RESOLVIDO
```

---

## 2. Abordagem

### 2.1. Dois protocolos: `show` e `repr`

`show` e `repr` são dois métodos do interface SHOW, com semântica distinta:

- **`show :: T => Text`** — representação humana. `show "hello"` → `hello`.
- **`repr :: T => Text`** — representação round-tripable. `repr "hello"` → `"hello"`.

A diferença entre `show` e `repr` é **uma regra**: Text. Para todo outro tipo,
`repr` delega para `show` (Int, Bool, Struct, Enum, coleções...). A única
divergência é:

| Tipo     | `show`              | `repr`              |
|----------|---------------------|---------------------|
| Text     | `hello` (identity)  | `"hello"` (citado)  |
| Int      | `42`                | `42` (delega show)  |
| List     | `[1, 2, 3]`         | `[1, 2, 3]` (delega)|
| Struct   | `Nome(Ana)`         | `Nome(Ana)` (delega)|

**Containers chamam `repr` nos elementos**, não `show`. Isso garante que
Text aninhado seja citado em qualquer nível:

```
echo!(show "hello")          # hello
echo!(show ["a" "b"])        # ["a", "b"]       ← List chama repr nos elementos
echo!(show {"a" "b"})        # {"a", "b"}       ← Array chama repr nos elementos
echo!(show (1, "a"))         # (1, "a")         ← Tuple chama repr nos elementos
echo!(show MyStruct(n: "Ana")) # MyStruct(n: "Ana")  ← Struct chama repr nos campos
```

### 2.2. Implementação na síntese: `repr_expr`

Na síntese TAST, o dispatcher interno mudou de `show_expr` para `repr_expr`:

- `repr_expr(arg, Ty::Prim(PrimTy::Text))` → `string_concat("\"", string_concat(arg, "\""))`
- `repr_expr(arg, Ty::Var(_))` → Closure genérica `repr <arg>` com `ffi_symbol: None` (ver 2.3)
- `repr_expr(arg, ty)` para qualquer outro tipo concreto → delega para `show_expr(arg, ty)`

Toda síntese de container (Structs, Enums, List existentes + Array, Tuple novos)
passa a chamar `repr_expr` nos elementos/campos em vez de `show_expr`. Isso é
uma mudança de comportamento nos tipos existentes:
`show MyStruct(name: "Ana")` passa a produzir `MyStruct(name: "Ana")`
em vez de `MyStruct(name: Ana)`.

### 2.3. `repr` em tipos genéricos (List::A, Array::A) — Resolução no Monomorphizador

**Problema:** `repr_expr` decide "isto é Text?" em tempo de síntese, mas para
tipos genéricos (`List::A`, `Array::A`), o tipo do elemento é `Ty::Var("A")` —
só resolvido na monomorfização. `repr_expr` com `Ty::Var` delegava para
`show_expr`, que não cita Text.

**Solução implementada:** `repr_expr` para `Ty::Var` gera uma Closure genérica
`repr <arg>` com `ffi_symbol: None`. O monomorphizador (Layer 7 em `rewrite_typed_expr`)
resolve essa Closure após instanciação:

- **Tipo concreto = Text:** substitui a Closure por `string_concat("\"", arg, "\"")` (cita)
- **Tipo concreto = outro:** troca o callee para `"show"` e resolve via DispatchTable/Layer 6

**Pitfall crítico:** O Layer 7 só resolve `repr` quando o tipo é **concreto** (não `Var`).
Se processar a template genérica com `Var("A")`, `resolve_repr_closure` mudaria o callee
para `"show"` — destruindo a Closure `repr` antes da instanciação copiá-la. A instância
resultante teria `show` em vez de `repr`, e Text não seria citado. Guard: `!matches!(arg_ty, Ty::Var(_))`.

### 2.4. `repr` como função standalone ✅

**Implementado** via interceptador `apply_repr.rs` na inference. Quando o usuário
escreve `repr <expr>`, o interceptador:
- Text concreto → gera `string_concat("\"", string_concat(arg, "\""))` inline (cita)
- Ty::Var → gera Closure genérica `repr <arg>` com `ffi_symbol: None` (monomorph resolve)
- Outro tipo concreto → delega para `show_expr` (mesma FFI)

O usuário pode chamar `repr` diretamente: `echo!(repr "hello")` → `"hello"`.
`repr 42` → `42` (delega para show).

### 2.5. Opção rejeitada: FFI `kata_rt_repr_to_text` no runtime

Uma FFI única que recebe `ptr + type_id` e caminha o `TypeShape` para produzir
Text. **Rejeitada** porque o runtime não tem informação de tipo suficiente
(TypeShape descarta o tipo do elemento), e reimplementar `show` no runtime
duplica lógica que já existe na síntese TAST.

---

## 3. Design

### 3.1. Tuple — Interceptador na inference + rewrite no monomorph ✅

**Implementado.** Tuple é estrutural (não nominal) e não registra overload de
`show` no DispatchTable. Dois mecanismos resolvem isso:

1. **`apply_show_tuple.rs`** (inference): intercepta `show <tuple>` antes do
   dispatch normal. Se o arg é `Ty::Tuple`, gera uma Closure genérica
   (`callee: Ident("show"), ffi_symbol: None`).

2. **`tuple_show.rs`** (monomorph, Layer 6): quando o monomorphizador encontra
   essa Closure com `ffi_symbol: None` e arg `Ty::Tuple`, substitui a Closure
   inteira por uma árvore de `string_concat` com `FieldAccess` para cada elemento.
   Cada elemento é despachado via `show_for_type` (que cita Text).

**Resultado:**
```kata
show (1, "hello")        # (1, "hello")
show (1, 2, 3)           # (1, 2, 3)
show (42,)               # (42)
show (1, "a", True)      # (1, "a", True)
```

**Limitação:** `show ()` (Unit) agora funciona — `try_show_tuple` detecta `Ty::Unit`
e retorna `TextLit("()")` diretamente.

### 3.2. Array — Síntese Recursiva com `len + at` ✅

**Implementado** via `show_synthesis_array.rs`. Duas funções genéricas mutuamente
recursivas registradas no DispatchTable:

- `__kata_show__Array :: Array::A => Text` — verifica `len == 0` (vazio: `{}`), senão
  faz `match (at __self 0) { Ok(h): "{" + repr(h) + rest(__self, 1) ; Err(_): "{}" }`
- `__kata_show__Array_rest :: Array::A Int => Text` — verifica `i == len` (fim: `}`), senão
  faz `match (at __self i) { Ok(h): ", " + repr(h) + rest(__self, i+1) ; Err(_): "}" }`

Usa `kata_rt_array_len` (retorna SMI-tagged) e `kata_rt_array_get_checked` (recebe idx
SMI-tagged, retorna Result Sum). `=` e `+` são Closures genéricas (`ffi_symbol: None`)
resolvidas pelo monomorphizador via DispatchTable.

**Resultado:**
```kata
show {1 2 3}            # {1, 2, 3}
show {}                 # {}
show {"a" "b"}          # {"a", "b"}
```

**Formato:** curly braces `{...}` — consistente com a sintaxe literal de Array.

### 3.3. Set — Síntese via `kata_rt_set_next` ✅

**Implementado** via `show_synthesis_set.rs`. Duas funções genéricas mutuamente
recursivas registradas no DispatchTable. Usa `kata_rt_set_next(set, iter_state, arena)`.

**SMI decode:** O codegen gera IntLit como SMI (`encode_smi(0) = 1`). `kata_rt_set_next`
decodifica SMI antes de delegar para `kata_rt_dict_next` (guard `& 1 == 1`).

**Resultado:**
```kata
show {|1 2 3|}       # {|3, 2, 1|}  (ordem reversa: Cons prepend)
show {|1|}           # {|1|}
show {|"a" "b" "c"|} # {|"c", "b", "a"|}  (repr cita Text)
```

**Formato:** `{|1, 2, 3|}` — sintaxe literal de Set.

### 3.4. Dict — Síntese via `kata_rt_dict_next_smi` ✅

**Implementado** via `show_synthesis_dict.rs`. Duas funções genéricas mutuamente
recursivas registradas no DispatchTable. Usa `kata_rt_dict_next_smi(dict, iter_state, arena)`
— wrapper que decodifica SMI do `iter_state` antes de delegar para `kata_rt_dict_next`.

**SMI decode:** O codegen gera IntLit como SMI. `kata_rt_dict_next_smi` decodifica
SMI (guard `& 1 == 1`) e chama `kata_rt_dict_next` com valor bruto. O `dict_next`
original permanece sem decode (testes Rust passam valores brutos).

**Extração de K e V:** `dict_next` retorna `Optional::(K, V)` — payload é tupla 16 bytes
(key@0, value@8). O synthesis extrai via `FieldAccess(kv, field_index=0)` para K e
`FieldAccess(kv, field_index=1)` para V — o codegen faz `load ptr + field_index * 8`.

**Resultado:**
```kata
show {"nome": "Ana"}          # {"nome": "Ana"}
show {"a": 1 "b": 2 "c": 3}  # {"c": 3, "b": 2, "a": 1}  (ordem reversa)
show {"a": "hello" "b": "world"} # {"b": "world", "a": "hello"}  (repr cita Text)
```

**Formato:** `{"k1": v1, "k2": v2}` — K e V via `repr_expr`.

### 3.5. Ordem de implementação

1. ✅ **Tuple** — interceptador + `tuple_show.rs` (monomorph)
2. ✅ **Array** — `show_synthesis_array.rs` com `len + at`
3. ✅ **repr em genéricos** — Layer 7 no monomorph resolver `repr` para tipo concreto
4. ✅ **Set** — `show_synthesis_set.rs` via `kata_rt_set_next` (iter_state explícito)
5. ✅ **Dict** — `show_synthesis_dict.rs` via `kata_rt_dict_next_smi` (iter_state SMI-safe)
6. ✅ **`repr` standalone** — interceptador `apply_repr.rs` na inference
7. ✅ **`show ()` (Unit)** — `try_show_tuple` aceita `Ty::Unit`, retorna `TextLit("()")`

---

## 4. Estrutura de Arquivos

```
crates/kata-inference/src/infer/
├── show_synthesis.rs           # Structs + Enums (existente, migrado para repr_expr)
├── show_synthesis_helpers.rs   # Helpers (existente, adicionado repr_expr)
├── show_synthesis_list.rs      # List (existente, migrado para repr_expr)
├── show_synthesis_array.rs     # NOVO — Array (len + at, duas funções recursivas)
├── show_synthesis_set.rs       # Set (via kata_rt_set_next, SMI decode)
├── show_synthesis_dict.rs      # Dict (via kata_rt_dict_next_smi, FieldAccess K/V)
├── apply_show_tuple.rs         # Interceptador show para Tuple/Unit na inference
├── apply_repr.rs               # Interceptador repr standalone na inference
└── mod.rs                      # Registro dos módulos (estendido)

crates/kata-monomorph/src/
├── tuple_show.rs               # Existente, atualizado: repr cita Text, helpers pub(crate)
├── lib.rs                      # Layer 6 (Tuple) + Layer 7 (repr) adicionados
└── array_show.rs               # REMOVIDO (placeholder descartado)
```

---

## 5. Validação

Cada tipo deve passar nos seguintes testes:

```kata
# Tuple ✅
show (1, "hello")           # (1, "hello")
show (1, 2, 3)              # (1, 2, 3)
show (42,)                  # (42)
show (1, "a", Boolean::True) # (1, "a", True)
# show ()                   # ❌ pendente (Ty::Unit)

# Array ✅
show {1 2 3}               # {1, 2, 3}
show {}                     # {}
show {"a" "b"}             # {"a", "b"}

# List ✅ (repr em genéricos resolvido)
show [1 2 3]               # [1, 2, 3]
show ["a" "b"]             # ["a", "b"]

# Set ✅
show {|1 2 3|}             # {|3, 2, 1|}  (ordem reversa)

# Dict ✅
show {"nome": "Ana"}       # {"nome": "Ana"}

# Unit ✅
show ()                    # ()

# repr standalone ✅
repr "hello"               # "hello"
repr 42                    # 42

# Aninhados ✅
show [(1, "a") (2, "b")]   # [(1, "a"), (2, "b")]
show {(1, "a") (2, "b")}   # {(1, "a"), (2, "b")}
show [[1 2] [3 4]]        # [[1, 2], [3, 4]]
```

---

## 6. Decisões Tomadas

1. **Formato de saída de Set:** `{|1, 2, 3|}` (sintaxe literal, consistente
   com List `[1, 2, 3]` e Array `{1, 2, 3}`).

2. **Formato de saída de Dict:** Aspas em K e V quando Text, via `repr_expr`.
   Exemplo: `show {"nome": "Ana"}` → `{"nome": "Ana"}`.

3. **Set/Dict iteração com cursor:** `kata_rt_set_next` e `kata_rt_dict_next`
   recebem `iter_state` como **parâmetro explícito** (0=init, N=Nth). Não consomem
   o handle — não precisa copiar. A síntese usa um contador inteiro como arg
   de recursão, igual ao `i` de Array.

4. **Tuple aridade:** Sem limite. O interceptador `apply_show_tuple.rs` detecta
   qualquer `Ty::Tuple(...)` e gera a Closure genérica. O `tuple_show.rs` no
   monomorph desdobra a árvore para qualquer número de elementos.

5. **Formato de Array:** `{1, 2, 3}` (curly braces) — consistente com a sintaxe
   literal `{1 2 3}`. NÃO usa colchetes (que é de List).

6. **repr em genéricos:** Layer 7 no monomorphizador. `repr_expr` gera Closure
   `repr <arg>` com `ffi_symbol: None` para `Ty::Var`. O monomorph resolve:
   Text → cita, outro → delega para `show`. Guard: não processar `Ty::Var`
   (preserva a Closure para instanciação).

---

## 7. Não-escopo

- `kata_rt_repr_to_text` / `pretty_print` no runtime — não será implementado
- `show` para tipos exóticos: Channel, Queue, Broadcast, Range, Function,
  OverloadSet — fora do escopo
- `show` para Bytes — já implementado (`show :: Bytes => Text`)
- Personalização de formato pelo usuário — o `show` sintetizado é fixo
- `show ()` (Unit) — ✅ implementado via `try_show_tuple`