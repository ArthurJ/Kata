# PRD — Reflexão de Funções via DotAccess: `f.name`, `f.arity`, `f.param_types`, `f.return_type`

**Status:** 🗑️ Obsoleto — remoção planejada
**Data:** 2026-08-04 (implementado), 2026-08-07 (obsoleto)
**Substituído por:** Variáveis de reflexão em diretivas (`docs/visao-diretivas-kata.md`, seção 3) — bindings `_name`, `_arity`, etc. sintetizados no desugaring, sem DotAccess nem sidecar table.

## 0. Por que este PRD está obsoleto

A reflexão de funções (`f.name`, `f.arity`, etc. via DotAccess) foi implementada
como infraestrutura para o sistema de diretivas Kata. O design de diretivas
evoluiu: em vez de o usuário escrever `f.name` no corpo da diretiva, o
compilador sintetiza variáveis de reflexão (`_name`, `_arity`, `_types`,
`_return_type`, `_is_action`) no escopo da action anotada com `@directive`.
Esses bindings são resolvidos no desugaring (pré-typeck) por substituição direta
de AST — o compilador conhece a função decorada e produz `TextLit("processar")`
sem passar por DotAccess, sidecar table, ou `kata_rt_fn_meta_lookup`.

A reflexão como feature de linguagem (sintaxe `f.name` acessível ao usuário)
não tem outros casos de uso além das diretivas. A complexidade que ela
introduz — sidecar table em runtime, binary search, relocations de fn_ptr,
registro em TLS, ordenação pós-finalize, branch dinâmico no typeck,
desambiguação `f.(Int Int)` — é desproporcional ao valor. A remoção elimina
tudo isso.

O documento a seguir preserva o histórico do que foi implementado e descreve
o escopo da remoção. As seções 1-8 não são mais especificação ativa — são
registro do que existe no código e precisa ser removido. Ver handoff
`/tmp/kata5-remove-reflection-handoff.md` para os passos de implementação.

---

## 1. Objetivo (histórico — implementado, agora marcado para remoção)

Permitir que funções e actions Kata sejam introspectáveis via DotAccess (`.`),
expondo metadata estática (nome, arity, tipos dos parâmetros, tipo de retorno) em
qualquer contexto — não apenas dentro de diretivas.

A representação de função em runtime **não muda**: continua sendo `I64` (fn ptr ou
box ptr). A metadata fica numa **sidecar table** — array estático no binário,
indexado por fn ptr, consultado via binary search em O(log N) quando `f.name` (ou
outro field) é acessado em contexto dinâmico. No contexto estático (`f` é `Ident`
direto para função nomeada), o typeck resolve para constante em compile-time —
zero overhead de runtime.

### Princípios de design

- **Zero overhead quando não usado.** Código que não acessa `f.name` não paga
  nenhum custo — a sidecar table é data estática, nunca carregada se não
  referenciada.
- **Zero mudança de ABI.** Função continua `I64`. `ty_to_clif` não muda.
  CaptureBox, call_indirect, spawn, channel send — nada muda.
- **Resolução em compile-time quando possível.** `f.name` onde `f` é `Ident`
  direto resolve para `ListLit` em compile-time. O codegen nem emite o lookup.
- **Fallback dinâmico quando necessário.** `g := f; g.name` onde `g` é variável
  emite um binary search na sidecar table em runtime. O(7 comparações para 100
  funções).
- **Sempre lista no caso estático.** `f.arity` retorna `List::Int` — um elemento
  por overload. Honesto sobre ambiguidade: o tipo não muda quando overloads são
  adicionadas. `f.(Int Int).arity` desambigua e retorna `Int` escalar.
- **Sem novo TypedExprKind.** O caso estático produz `ListLit` de `TextLit`/`IntLit`.
  O caso dinâmico produz um `Closure` chamando `kata_rt_fn_meta_lookup` — reusa
  infra existente.

### Inspiração

O modelo é análogo a como Python expõe `__name__`, `__qualname__`, `__module__`
em function objects. Em Python, a metadata vive no objeto função (overhead por
função). Em Kata, a metadata vive numa sidecar table (overhead zero por
função, lookup O(log N) por acesso).

## 2. Sintaxe

### 2.1. Fields disponíveis

```kata
f.name           # List::Text — ["processar"] (um por overload)
f.arity          # List::Int — [2] (um por overload)
f.param_types    # List::List::Text — [["Int", "Int"]] (um por overload)
f.return_type    # List::Text — ["Int"] (um por overload)
f.is_action      # List::Boolean — [False] (um por overload)
```

**Sempre lista.** Mesmo com uma única overload, o tipo é `List`. Isto garante
que código que compila com 1 overload continue compilando quando uma segunda
overload é adicionada. O tipo não muda com a quantidade de overloads.

### 2.2. Desambiguação por tipo — `f.(Type1 Type2 ...)`

Quando o usuário quer uma overload específica, usa `.(Tipos)` para selecioná-la:

```kata
soma :: Int Int => Int
    lambda a b: + a b

soma :: Text Text => Text
    lambda a b: + a b

echo!(soma.(Int Int).arity)       # 2 — Int escalar, overload específica
echo!(soma.(Text Text).arity)     # 2 — Int escalar, outra overload
echo!(soma.(Int Int).return_type) # "Int" — Text escalar
echo!(soma.arity)                 # [2, 2] — List::Int, todas as overloads
```

`f.(Int Int)` resolve para `Lambda(Int Int -> Int)` — a overload específica
como valor. A partir daí, `.arity` é escalar porque o valor não é ambíguo.

Desambiguação também funciona para atribuição:

```kata
let g := soma.(Int Int)   # g tem tipo Lambda(Int Int -> Int)
echo!(g.arity)            # 2 — Int escalar (via sidecar table, fn_ptr específico)
```

### 2.3. Uso em contexto estático (Ident direto, sem desambiguação)

```kata
processar :: Int Int => Int
    lambda a b: + a b

echo!(processar.name)           # ["processar"] — List::Text
echo!(processar.arity)          # [2] — List::Int
echo!(processar.param_types)    # [["Int", "Int"]] — List::List::Text
echo!(processar.return_type)    # ["Int"] — List::Text
```

O typeck reconhece que `processar` é `Ident` que resolve para função nomeada. Coleta
**todas as overloads** deste nome e produz um `ListLit` com um elemento por overload.

### 2.4. Uso em contexto estático desambiguado

```kata
echo!(processar.(Int Int).name)           # "processar" — Text escalar
echo!(processar.(Int Int).arity)          # 2 — Int escalar
echo!(processar.(Int Int).param_types)    # ["Int", "Int"] — List::Text
echo!(processar.(Int Int).return_type)    # "Int" — Text escalar
```

`processar.(Int Int)` resolve para um único `Lambda(Int Int -> Int)`. Os fields são extraídos
dessa assinatura única e produzidos como escalares (`TextLit`, `IntLit`, `ListLit`).

### 2.5. Uso em contexto dinâmico (variável)

```kata
g := soma.(Int Int)          # g tem tipo Lambda(Int Int -> Int) — overload específica
echo!(g.name)                # "soma" — Text escalar, binary search na sidecar table
echo!(g.arity)               # 2 — Int escalar
```

O caso dinâmico é **sempre escalar** porque o `fn_ptr` identifica uma overload
específica. O binary search na sidecar table encontra a entry exata.

```kata
fns := [soma.(Int Int), fatorial, dobrar]
picked := fns.(0)
echo!(picked.name)           # "soma" — binary search em runtime
```

### 2.6. Aplicação em lambdas

Lambdas anônimas recebem nomes sintéticos na sidecar table:

```kata
f := lambda x: + x 1
echo!(f.name)      # ["f"] — List::Text (uma overload, sempre lista)
```

Lambdas passadas diretamente (sem binding) recebem `__lambda_N`:

```kata
echo!((lambda x: + x 1).name)  # ["__lambda_0"] — List::Text
```

Lambdas sempre têm uma única "overload" (não podem ser sobrecarregadas), então
a lista tem sempre length 1.

### 2.7. Aplicação em actions — sempre estático, sempre lista

Actions **não são first-class** em Kata. `let g := processar` (onde `processar`
é action) é ilegal — actions não são registradas no `TypeEnv` como valores.
Esta restrição é preservada.

Como consequência, reflexão de actions é **sempre estática** — o caso dinâmico
nunca existe. O typeck resolve `processar.name` consultando a `DispatchTable`
(não o `TypeEnv`), que contém `OverloadInfo { name, params, ret, is_action }`
para cada action. Como no caso de funções, retorna **sempre lista** — uma
overload por elemento.

```kata
action processar(x::Int) => Int
    + x 1

echo!(processar.name)         # ["processar"] — List::Text
echo!(processar.arity)        # [1] — List::Int
echo!(processar.param_types)  # [["Int"]] — List::List::Text
echo!(processar.return_type)  # ["Int"] — List::Text
echo!(processar.is_action)    # [True] — List::Boolean
```

Desambiguação funciona igual a funções:

```kata
echo!(processar.(Int).arity)  # 1 — Int escalar, overload específica
```

### 2.8. O que NÃO funciona

```kata
42.name          # ERRO — Int não é Function/Action
pessoa.name      # OK — field access em struct (comportamento existente)
processar.foo    # ERRO — field desconhecido em função/action
g := processar   # ERRO — action não é first-class (restrição existente)
soma.(Int).foo   # ERRO — field desconhecido após desambiguação
```

### 2.9. Pipeline — parser de `.(Type1 Type2 ...)`

O parser precisa reconhecer `.(Int Int)` como `DotIndex::Type(Vec<Ty>)`.
A gramática atual de `.(...)` suporta `.(Int)` (indexing em tupla/lista) e
.`(field_name)` (field access em struct). A nova forma é `.(Type1 Type2 ...)`
onde os tipos são separados por espaço (mesma sintaxe de assinaturas Kata).

**Desambiguação no parser:** `.(42)` é `DotIndex::Int(42)` (indexing numérico
em coleção). `.(Int)` é `DotIndex::Type([Int])` (seleção de overload por tipo).
O parser distingue pela forma: se o conteúdo é um literal inteiro → `Int`;
se é um TypeExpr → `Type`. O lexer já distingue inteiros de identificadores.

## 3. Semântica

### 3.1. Sidecar table — layout

A sidecar table é um array estático no binário, ordenado por fn_ptr (para binary
search). Cada entry é:

```rust
// Layout em memória (serializado como bytes no data symbol):
struct FnMetaEntry {
    fn_ptr: i64,          // 8 bytes — endereço da função (resolved via relocation)
    name_ptr: i64,        // 8 bytes — ponteiro para string estática (__kata_str_N)
    arity: i64,           // 8 bytes — número de parâmetros
    param_types_ptr: i64, // 8 bytes — ponteiro para array de string ptrs
    param_types_len: i64, // 8 bytes — número de param types
    return_type_ptr: i64, // 8 bytes — ponteiro para string estática
    is_action: i64,       // 8 bytes — 0 = Function, 1 = Action
}
// Tamanho: 56 bytes por entry
```

A tabela é emitida como um data symbol `__kata_fn_meta_table` com:
1. Header: `count: i64` (número de entries)
2. Array de `FnMetaEntry` ordenado por `fn_ptr`

### 3.2. Sidecar table — emissão no codegen

O codegen já emite data symbols para strings (`__kata_str_N`) e snapshots
(`__kata_snap_bytes_N`). A sidecar table segue o mesmo padrão, com uma
diferença: cada entry contem um fn_ptr que é uma **relocation** para um
function symbol. O Cranelift resolve estas relocations durante
`finalize_definitions()`.

Fluxo de emissão (novo loop em `module.rs`, após `define_function` e antes de
`finalize`):

1. Para cada função no `symbol_table: HashMap<FuncKey, FuncId>`:
   - Adicionar o nome da função à `string_table` (se ainda nao existir)
   - Para cada param type e return type: adicionar à `string_table`
   - Construir `FnMetaEntry` com `DataDescription`:
     - `fn_ptr`: relocation via `declare_func_in_data(func_id, &mut data_desc)`
     - `name_ptr`: relocation para `__kata_str_N` via `declare_data_in_data`
     - `arity`: `i64::to_le_bytes(params.len())`
     - `param_types_ptr`/`param_types_len`: relocation para array de string ptrs
     - `return_type_ptr`: relocation para `__kata_str_N`
     - `is_action`: 0 ou 1
2. Ordenar entries por `fn_ptr` (necessário para binary search em runtime)
   - NOTA: o fn_ptr é unknown até `finalize`. Ordenação por `FuncId` como
     proxy, e reordenação em runtime após `finalize` (ver 3.3).
3. Serializar como `__kata_fn_meta_table` via `declare_data` + `define_data`

### 3.3. Registro no runtime

O runtime recebe um ponteiro para a sidecar table e o número de entries.
Registro via FFI análoga a `kata_rt_register_type_table`:

```rust
// kata-rt/src/reflection.rs (novo)
thread_local! {
    static FN_META_TABLE: RefCell<FnMetaTable> = RefCell::new(FnMetaTable::empty());
}

struct FnMetaTable {
    entries: Vec<FnMetaEntry>,  // ordenado por fn_ptr
}

pub fn kata_rt_register_fn_meta_table(ptr: i64, count: i64) {
    // Lê `count` entries a partir de `ptr`, armazena em TLS
    // Ordena por fn_ptr (os fn_ptrs só são conhecidos após JIT finalize)
}

pub fn kata_rt_fn_meta_lookup(fn_ptr: i64, field: i64) -> i64 {
    // field: 0=name, 1=arity, 2=param_types, 3=return_type, 4=is_action
    // Binary search por fn_ptr na tabela
    // Retorna o valor do field solicitado
    // Retorna 0 se fn_ptr nao encontrado (sentinel)
}
```

**Ordenação pós-finalize:** no JIT, os fn_ptrs só sao conhecidos após
`finalize_definitions()`. A tabela é registrada com os fn_ptrs resolvidos
(durante o prólogo do entry point, antes da execução do código do usuário).
O registro ordena a tabela por fn_ptr em runtime (sort O(N log N), N ~ 100,
nanossegundos).

Alternativa: emitir a tabela já ordenada por `FuncId` (estável em compile-time)
e usar `FuncId` como chave de lookup em vez de fn_ptr. Isto exige propagar
`FuncId` junto com o fn_ptr (volta ao problema B-register). **Rejeitado** — o
binary search por fn_ptr é O(log N) e nao requer mudança de ABI.

### 3.3a. Desambiguação — `DotIndex::Type(Vec<Ty>)`

Nova variante de `DotIndex`: `Type(Vec<Ty>)` — seleciona overload por tipos
de parâmetros. Quando o typeck encontra `Ident("soma") . (Int Int)`:

1. Resolve `soma` no DispatchTable/TypeEnv → coleta todas as overloads
2. Filtra por `params == [Int, Int]`
3. Se exatamente 1 match: retorna `TypedExprKind::Ident { name: "soma" }`
   com `ty: Ty::Function([Int, Int], Int)` (internamente) — o tipo na
   linguagem é `Lambda(Int Int -> Int)`
4. Se 0 matches: erro `NoOverloadForTypes`
5. Se 2+ matches (mesmos params, ret diferente): erro `AmbiguousOverload`

O `Ident` resultante com `Ty::Function` concreta é o ponto de entrada para
reflexão escalar: `soma.(Int Int).arity` encadeia `DotIndex::Type` depois
`DotIndex::Field`. O typeck resolve `.(Int Int)` → `Ty::Function`, depois
`.arity` no resultado — que é um valor não-ambíguo, portanto escalar.

O codegen não muda: `Ident("soma")` com `Ty::Function([Int, Int], Int)`
resolve via `symbol_table.get(("soma", [Int, Int], Int))` → `FuncId` exata.

### 3.3b. TypeEnv — registro de todas as overloads

Hoje o `TypeEnv` usa `HashMap<String, TypeBinding>` — `define` sobrescreve.
Com overloads, a última função registrada vence e as anteriores são perdidas.

**Mudança:** `HashMap<String, Vec<TypeBinding>>` — `define` faz `push`.
`lookup` retorna `&[TypeBinding]` (todas as overloads). `lookup_single`
retorna `Option<&TypeBinding>` (a última, para compatibilidade com código
que não precisa disambiguar — ex: `infer_apply` que usa DispatchTable).

Isto permite `let g := soma.(Int Int)` resolver para a overload correta,
não a última registrada.

### 3.4. Typeck — caso estático sem desambiguação (sempre lista)

Em `infer_dot_access` (`dot_access.rs`), quando o receptor é `Ident` direto
para função nomeada (no TypeEnv ou DispatchTable) e o index é `Field`:

```rust
// Coletar TODAS as overloads do nome
let overloads = collect_all_overloads(name, env, ctx.table);
// overloads: Vec<(params: Vec<Ty>, ret: Ty, is_action: bool)>

// Para cada overload, resolver o field
let elements: Vec<Spanned<TypedExpr>> = overloads.iter().map(|(params, ret, is_action)| {
    resolve_reflection_field_scalar(field, name, params, ret, *is_action, span)
}).collect();

// Produzir ListLit
TypedExpr {
    ty: Ty::List(Box::new(field_type(field))),
    kind: TypedExprKind::ListLit { elements },
}
```

`resolve_reflection_field_scalar` produz um escalar por overload:
- `"name"` → `TextLit { text: name }`
- `"arity"` → `IntLit { text: params.len().to_string() }`
- `"param_types"` → `ListLit` de `TextLit` (tipos dos params desta overload)
- `"return_type"` → `TextLit { text: ty_to_text(ret) }`
- `"is_action"` → `VariantQual { Boolean::True/False }`

O `ListLit` externo tem um elemento por overload. Para `param_types`, o
resultado é `List::List::Text` — uma lista de listas de textos.

### 3.4a. Typeck — caso estático com desambiguação (escalar)

Quando o receptor é `Ident.(Types).field` — o `.(Types)` resolve primeiro
para `Ty::Function` específica, e `.field` é aplicado ao resultado:

```rust
// Passo 1: Ident . (Int Int) → Ty::Function([Int, Int], Int)
// (resolved pela seção 3.3a)

// Passo 2: .arity na Ty::Function específica → escalar
resolve_reflection_field_scalar(field, name, &params, &ret, is_action, span)
// → IntLit { text: "2" }
```

O resultado é escalar porque a overload foi selecionada — não há ambiguidade.

### 3.4b. Typeck — caso estático (actions via DispatchTable)

Actions não são registradas no `TypeEnv`. Quando o receptor é `Expr::Ident` que
**não** está no `TypeEnv`, o code já tenta module access. O novo código, após
module access falhar:

1. Se `DotIndex::Field` e `is_reflection_field`: coletar todas as overloads
   de action com este nome no DispatchTable → produzir `ListLit` (sempre lista)
2. Se `DotIndex::Type`: filtrar overloads por params → retornar `Ty::Function`
   específica (desambiguação)

### 3.5. Typeck — caso dinâmico (functions apenas, sempre escalar)

Quando o receptor é uma variável (`let g := f`) ou expressão complexa com
`Ty::Function`:

```rust
(Ty::Function(_, _), DotIndex::Field(field)) =>
{
    // Verificar que o field é válido
    let field_id = match field.as_str() {
        "name" => 0,
        "arity" => 1,
        "param_types" => 2,
        "return_type" => 3,
        "is_action" => 4,
        _ => return Err(UnknownField),
    };

    // Lowerar receptor para obter fn_ptr em runtime
    let fn_ptr = infer_expr(receptor, ...);

    // Construir chamada: kata_rt_fn_meta_lookup(fn_ptr, field_id)
    // Retorno: i64 (Text ptr para name/return_type, Int para arity,
    //          List ptr para param_types, Boolean para is_action)
    let lookup = TypedExprKind::Closure {
        callee: Ident { name: "kata_rt_fn_meta_lookup" },
        args: [fn_ptr, IntLit(field_id)],
        ffi_symbol: Some("kata_rt_fn_meta_lookup"),
    };

    // Coerção do retorno i64 para o tipo esperado:
    // name/return_type → Text (i64 é Text ptr na ABI)
    // arity → Int (i64 é Int na ABI, SMI-tagged)
    // param_types → List::Text (i64 é List ptr na ABI)
    // is_action → Boolean (i64 é Sum ptr na ABI)
}
```

O caso dinâmico é **sempre escalar** porque o `fn_ptr` identifica uma overload
específica. O binary search na sidecar table encontra a entry exata — não há
ambiguidade. O tipo de retorno é `Text` (name/return_type), `Int` (arity),
`List::Text` (param_types), ou `Boolean` (is_action) — nunca `List` desses.

O typeck sempre tenta o caso estático primeiro. Só recorre ao dinâmico quando
o receptor não é `Ident` direto para função nomeada (é variável, match result,
coleção, etc.).

### 3.6. `ty_to_text` — serialização de tipos

Cada `Ty` precisa ser convertido para sua representação textual. Isto ja
existe em `kata-rt` via `kata_rt_repr_to_text` (usado por pretty_print). O
typeck pode reusar esta função FFI ou implementar a serialização em Rust
(mais simples — os tipos são finitos e conhecidos):

```rust
fn ty_to_text(ty: &Ty) -> String {
    match ty {
        Ty::Prim(PrimTy::Int) => "Int",
        Ty::Prim(PrimTy::Float) => "Float",
        Ty::Prim(PrimTy::Text) => "Text",
        Ty::Prim(PrimTy::Rational) => "Rational",
        Ty::Unit => "Unit",
        Ty::Struct(name) => name,
        Ty::Sum(name) => name,
        Ty::Function(params, ret) => {
            let param_strs: Vec<String> = params.iter().map(ty_to_text).collect();
            format!("{} => {}", param_strs.join(" "), ty_to_text(ret))
        }
        Ty::Action(params, ret) => { /* similar */ }
        Ty::List(inner) => format!("List::({})", ty_to_text(inner)),
        Ty::Array(inner) => format!("Array::({})", ty_to_text(inner)),
        Ty::Tuple(elems) => {
            format!("({})", elems.iter().map(ty_to_text).collect::<Vec<_>>().join(", "))
        }
        // ... outros Ty variants
    }
}
```

Esta função vive em `kata-inference` ou `kata-core` (disponível para o typeck
em compile-time e para o codegen na emissão da sidecar table).

### 3.7. Monomorfização

Funções genéricas instanciadas por `kata-monomorph` geram múltiplas cópias
especializadas. Cada cópia recebe um entry na sidecar table com nome
qualificado:

- `map__Int` para `map` instanciado com `A = Int`
- `map__Text` para `map` instanciado com `A = Text`

O esquema de mangling ja existe no codegen (`kata_refs` usa chaves compostas
`(name, param_types, ret)`). A sidecar table itera sobre o `symbol_table`
que já contém estas chaves.

Actions genéricas (action FFI com type_params) sao tratadas pelo typeck
estático (DispatchTable), nao pela sidecar table. O `OverloadInfo` da
action já carrega os `params` e `ret` concretos da instancia.

### 3.8. Lambdas anônimas

Lambda nao tem nome nem FuncId estável. O nome na sidecar table depende do
contexto:

1. **Lambda atribuída via `let`:** `f := lambda x: + x 1` → nome é `"f"` (nome
   do binding). O typeck extrai do `Let` node da TAST.
2. **Lambda passada diretamente:** `(lambda x: + x 1)` → nome é `"__lambda_N"`
   (contador sequencial no lowering).
3. **Lambda em coleção:** `[lambda x: x, lambda x: + x 1]` → cada uma recebe
   `"__lambda_N"`.

O nome do binding é preferido porque é mais útil para o usuário. Lambdas
anônimas raramente sao introspectadas em prática.

### 3.9. Edge case — fn_ptr desconhecido

Se o fn_ptr nao é encontrado na sidecar table (FFI dinamico, plugin carregado
em runtime — caso que Kata nao suporta hoje), `kata_rt_fn_meta_lookup` retorna
0. O tipo de retorno é:

- `name`/`return_type`: Text ptr 0 → string vazia `""`
- `arity`: 0
- `param_types`: List ptr 0 → lista vazia
- `is_action`: 0

Nao é erro de tipo. O usuário recebe valores default. Se FFI dinamico for
adicionado no futuro, pode revisitar.

### 3.10. Interação com DotAccess existente

DotAccess hoje despacha por tipo do receptor:
- `Ty::Struct` + `Field` → `FieldAccess`
- `Ty::Tuple` + `Int` → `IndexAccess`
- `Ty::List/Array/Bytes/Text` + `Int` → desugar para `at` via INDEXABLE
- Outro → `NotIndexable`

O novo case adiciona:
- `Ty::Function` + `Field` → caso estático (constante) ou dinâmico (lookup)
- `Ty::Action` + `Field` → **sempre estático** (via DispatchTable, antes do
  `infer_expr`)

A ordem do match importa. Para actions, o caminho é diferente de functions:
- **Actions** sao resolvidas **antes** de `infer_expr` (linhas 49-73), no
  bloco que tenta module access quando `Ident` nao está no `TypeEnv`. O novo
  código de action reflection é adicionado **após** a tentativa de module
  access e **antes** de chamar `infer_expr`.
- **Functions** sao resolvidas **após** `infer_expr` (linha 80), no match
  `(Ty::Function, DotIndex::Field)`. O `infer_expr` já resolveu o `Ident`
  para `Ty::Function` via `TypeEnv.lookup`.

Isto reflete a diferença fundamental: actions vivem na `DispatchTable` (nao
sao first-class), functions vivem no `TypeEnv` (sao first-class).

## 4. Fases de implementação

### Fase 1: `ty_to_text` — serialização de tipos

- Implementar `fn ty_to_text(ty: &Ty) -> String` em `kata-core/src/ty.rs`
  (ou `kata-inference`).
- Cobrir todos os variants de `Ty`: Prim, Unit, Struct, Sum, Function, Action,
  Tuple, List, Array, Range, Set, Dict, Sender, Receiver, Bytes, File, Socket,
  Byte, Generic, Interface, InferVar, Var.
- Testes unitários: `ty_to_text(Ty::int()) == "Int"`, etc.

**Arquivos:**
- `crates/kata-core/src/ty.rs` — `ty_to_text` (nova função pub)
- `crates/kata-core/tests/ty_to_text_test.rs` — testes unitários (novo)

**Verificação:** `cargo test -p kata-core -- ty_to_text`

### Fase 2: Runtime — sidecar table + lookup

- Implementar `kata_rt_register_fn_meta_table(ptr, count)` em
  `kata-rt/src/reflection.rs` (novo módulo):
  - Lê `count` entries a partir de `ptr` (cada entry: 56 bytes).
  - Armazena em TLS `FN_META_TABLE`.
  - Ordena por `fn_ptr` (sort em runtime — os fn_ptrs só sao conhecidos após
    JIT finalize).
- Implementar `kata_rt_fn_meta_lookup(fn_ptr, field) -> i64`:
  - Binary search por `fn_ptr` na tabela ordenada.
  - `field`: 0=name, 1=arity, 2=param_types, 3=return_type, 4=is_action.
  - Retorna o valor do field. Retorna 0 se nao encontrado.
- Re-exportar de `kata-rt/src/lib.rs`.
- Registrar símbolos FFI no codegen:
  - `FfiSymbol::FnMetaRegister`, `FfiSymbol::FnMetaLookup` em
    `kata-core/src/ffi.rs`
  - Assinaturas em `kata-codegen/src/ffi_sigs.rs`:
    - `FnMetaRegister`: `(ptr: i64, count: i64) -> void`
    - `FnMetaLookup`: `(fn_ptr: i64, field: i64) -> i64`
  - Registro em `ffi_registry.rs` e `builder.symbol()` em `lib.rs` (pitfall #31).

**Arquivos:**
- `crates/kata-rt/src/reflection.rs` — `FnMetaTable`, `register`, `lookup` (novo)
- `crates/kata-rt/src/lib.rs` — re-export
- `crates/kata-core/src/ffi.rs` — `FfiSymbol::FnMetaRegister`, `FnMetaLookup`
- `crates/kata-codegen/src/ffi_sigs.rs` — assinaturas
- `crates/kata-codegen/src/ffi_registry.rs` — registro
- `crates/kata-rt/src/lib.rs` — `builder.symbol`

**Verificação:** `cargo check --workspace --all-targets`

### Fase 3: Codegen — emissão da sidecar table (functions apenas)

- Em `lowering/module.rs`, após `define_function` e antes de `finalize`:
  1. Coletar todas as **funções** de `symbol_table` (HashMap<FuncKey, FuncId>).
     **Actions nao sao incluídas** — reflexão de actions é sempre estática
     (via DispatchTable no typeck), sem lookup em runtime.
  2. Para cada função:
     - Adicionar nome (qualificado se monomorfizado) à `string_table`.
     - Adicionar cada param type e return type à `string_table` via
       `ty_to_text`.
     - Construir `FnMetaEntry` como `DataDescription`:
       - `fn_ptr`: `declare_func_in_data(func_id, &mut data_desc, 0)`
       - `name_ptr`: `declare_data_in_data(str_did, &mut data_desc, 8)`
       - `arity`: `i64::to_le_bytes(params.len())` em offset 16
       - `param_types_ptr`: ponteiro para sub-array de string ptrs (outro
         data symbol `__kata_fn_meta_params_N`)
       - `param_types_len`: `i64::to_le_bytes` em offset 32
       - `return_type_ptr`: `declare_data_in_data` em offset 40
       - `is_action`: 0 ou 1 em offset 48
  3. Ordenar entries (por FuncId como proxy — reordenação em runtime pelo
     `register`).
  4. Serializar como `__kata_fn_meta_table`:
     - Header: `count: i64` (8 bytes)
     - Array de entries (56 bytes cada)
     - `declare_data` + `define_data`
- Para lambdas:
  - Se a lambda é atribuída via `let f := lambda...`, usar `"f"` como nome.
    Detectar via `Let` node na TAST onde `value.kind` é `Lambda`.
  - Se anônima, usar `"__lambda_N"` (contador sequencial).
  - Lambda entries sao adicionadas ao `symbol_table` durante o lowering
    (ja sao hoje — cada lambda vira uma função JIT).
- Após `finalize`, no prólogo do entry point:
  - Carregar ponteiro de `__kata_fn_meta_table` via `global_value`.
  - Carregar `count` do header.
  - Chamar `kata_rt_register_fn_meta_table(ptr, count)`.

**Arquivos:**
- `crates/kata-codegen/src/lowering/module.rs` — emissão da tabela + registro
- `crates/kata-codegen/src/lowering/mod.rs` — `add_string` ja existe (reusar)

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 4: Typeck — caso estático (functions via TypeEnv + actions via DispatchTable)

- Em `infer_dot_access` (`dot_access.rs`), **dois pontos de inserção**:

**4a. Actions — antes de `infer_expr` (após module access, linhas 49-73):**
- Se `Ident` nao está no `TypeEnv` e module access falhou:
  - Verificar se `field_name` é um field de reflexão (`name`, `arity`,
    `param_types`, `return_type`, `is_action`)
  - Se sim, buscar `ctx.table.get_overloads(name)` (DispatchTable)
  - Se encontra overload com `is_action: true`:
    - Resolver field para constante em compile-time
    - `"name"` → `TextLit { text: name }`
    - `"arity"` → `IntLit { text: params.len().to_string() }`
    - `"param_types"` → `List` literal de `TextLit` com `ty_to_text(p)`
    - `"return_type"` → `TextLit { text: ty_to_text(ret) }`
    - `"is_action"` → `VariantQual { Boolean::True }`
  - Se nao encontra overload, cai para erro `UnboundName` (como hoje)
- helper `is_reflection_field(field: &str) -> bool` e
  `resolve_reflection_field(field, name, params, ret, is_action) -> TypedExpr`

**4b. Functions — após `infer_expr` (novo case no match, linha 80+):**
- `(Ty::Function(params, ret), DotIndex::Field(field))`:
  - Verificar se o receptor era `Expr::Ident` que resolveu para função nomeada
    no `TypeEnv` (nao variável local com `Ty::Function`)
  - Se sim, resolver field para constante (mesmo helper
    `resolve_reflection_field`, com `is_action = false`)
  - Se o receptor é variável (`let g := f`), cair no caso dinâmico (Fase 5)
- O `List` literal para `param_types` precisa ser lowerável pelo codegen. Ja
  existe lowering para `List` literals (`collections_literal.rs`). O typeck
  produz `TypedExprKind::List { elements: Vec<TextLit> }`.

**Arquivos:**
- `crates/kata-inference/src/infer/dot_access.rs` — action reflection (4a) +
  function estático (4b)
- `crates/kata-core/src/ty.rs` — `ty_to_text` (ja feita na Fase 1)

**Verificação:** `cargo test -p kata-inference -- dot_access`

### Fase 5: Typeck — caso dinâmico (functions apenas)

- Em `infer_dot_access`, se o receptor é `Ty::Function` mas
  **nao** é `Ident` direto para função nomeada (é variável, match result,
  coleção, etc.):
  - Mapear field para `field_id` (0-4).
  - Inferir o receptor para obter `fn_ptr` em runtime.
  - Construir `TypedExprKind::Closure` chamando `kata_rt_fn_meta_lookup`:
    - `callee`: `Ident { name: "kata_rt_fn_meta_lookup" }`
    - `args`: `[fn_ptr_value, IntLit(field_id)]`
    - `ffi_symbol`: `Some("kata_rt_fn_meta_lookup")`
  - Tipo de retorno:
    - `name`/`return_type` → `Ty::text()`
    - `arity` → `Ty::int()`
    - `param_types` → `Ty::List(Box::new(Ty::text()))`
    - `is_action` → `Ty::Sum("Boolean")`
  - O retorno `i64` do FFI é coercionado automaticamente (Text/Int/List/Sum
    sao todos `I64` na ABI).

**Nota:** O caso dinâmico nao existe para actions. `Ty::Action` nunca aparece
como tipo de variável no TypeEnv (actions nao sao first-class). Se o code
chega ao match `(Ty::Function, DotIndex::Field)` com receptor que é
variável, o typeck emite o lookup. `Ty::Action` nao tem case aqui.

**Arquivos:**
- `crates/kata-inference/src/infer/dot_access.rs` — caso dinâmico (fallback)

**Verificação:** `cargo test -p kata-inference -- dot_access`

### Fase 6: Prelude — sem mudanças necessárias

A sidecar table é emitida automaticamente pelo codegen. Nenhuma adição ao
prelude é necessária — `f.name` é despachado pelo typeck, nao por uma função
da stdlib.

**Verificação:** N/A

### Fase 7: Testes E2E

Testes novos em `crates/kata-driver/tests/`:

- `fn_reflection_static_name.kata` — `processar.name` em Ident direto →
  "processar" (compile-time).
- `fn_reflection_static_arity.kata` — `processar.arity` → 2.
- `fn_reflection_static_param_types.kata` — `processar.param_types` →
  ["Int", "Int"].
- `fn_reflection_static_return_type.kata` — `processar.return_type` → "Int".
- `fn_reflection_dynamic_name.kata` — `g := f; g.name` → "f" (runtime
  lookup).
- `fn_reflection_dynamic_arity.kata` — `g := f; g.arity` → 2 (runtime).
- `fn_reflection_list_of_fns.kata` — `[f, g].(0).name` → "f" (runtime).
- `fn_reflection_action_name.kata` — `action processar ...;
  processar.name` → "processar".
- `fn_reflection_action_is_action.kata` — `processar.is_action` → True.
- `fn_reflection_function_is_action.kata` — `f.is_action` → False.
- `fn_reflection_lambda_named.kata` — `g := lambda x: x; g.name` → "g".
- `fn_reflection_lambda_anon.kata` — `(lambda x: x).name` → "__lambda_0".
- `fn_reflection_unknown_field.kata` — `f.foo` → erro de tipo.
- `fn_reflection_on_int.kata` — `42.name` → erro de tipo (NotIndexable ou
  UnknownField).

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed.

### Fase 8: Atualização do manual técnico

- Adicionar seção "Reflexão de Funções" em `docs/Kata-lang-manual.md`:
  - Fields disponíveis (`name`, `arity`, `param_types`, `return_type`,
    `is_action`).
  - Distinção estático/dinâmico.
  - Comportamento com lambdas.
  - Edge cases (fn_ptr desconhecido → defaults).
- Atualizar `kata.tmLanguage.json` se necessário (nao precisa — `.` ja é
  tokenizado como DotAccess e `name` ja é `entity.name.function`).

**Arquivos:**
- `docs/Kata-lang-manual.md`

**Verificação:** review manual.

## 5. Pitfalls

### 5.1. Ordenação da sidecar table pós-finalize

No JIT, os fn_ptrs só sao conhecidos após `finalize_definitions()`. A tabela
é emitida com entries ordenadas por `FuncId` (proxy estável). O runtime
reordena por `fn_ptr` durante `kata_rt_register_fn_meta_table`. O sort é
O(N log N) para N ~ 100 funções — nanossegundos, executado uma vez no prólogo.

No AOT, o linker resolve os fn_ptrs durante a linkagem. A tabela pode ser
ordenada em link-time se o formatador de object file suportar (ou reordenada
em runtime, same as JIT).

### 5.2. Monomorfização e nomes qualificados

`map` instanciado com `A = Int` e `A = Text` gera duas funções. O nome na
sidecar table é qualificado: `map__Int`, `map__Text`. O esquema de mangling
ja existe no codegen (`kata_refs` usa `(name, param_types, ret)` como chave).
A sidecar table itera sobre o `symbol_table` que já tem estas chaves.

### 5.3. `param_types` como `List::Text`

O typeck estático produz `TypedExprKind::List { elements: Vec<TextLit> }`. O
codegen ja sabe lowerar `List` literals (`collections_literal.rs`). Nenhum
novo mecanismo de codegen é necessário.

Para o caso dinâmico, `kata_rt_fn_meta_lookup` retorna um `i64` que é ponteiro
para `List` na arena. O codegen trata como qualquer outro `List` value.

### 5.4. Registro da FFI — checklist completo

Seguir o pitfall #31 da skill `kata-compiler`: registrar novas FFIs em TODOS
os sites:
1. `FfiSymbol::FnMetaRegister`, `FnMetaLookup` em `kata-core/src/ffi.rs`
2. `symbol_name()`, `return_type()`, `from_name()`, `ffi_signature()` em
   `ffi_sigs.rs`
3. `all_ffi_symbols()`, `declare_ffi_symbols`, `register_ffi_symbols` em
   `ffi_registry.rs`
4. `builder.symbol()` em `kata-rt/src/lib.rs`

### 5.5. `declare_func_in_data` — API Cranelift

A API `declare_func_in_data(func_id, &mut data_desc, offset)` adiciona uma
relocation no `DataDescription` que `finalize_definitions()` resolve para o
endereço da função. Isto é o mesmo mecanismo que vtables em C. Verificar que
o `cranelift-module` crate expõe esta API (deveria — é parte da API pública
de `Module`).

### 5.6. Interação com module access (`mod.fn`)

O case existente em `infer_dot_access` (linhas 49-73) despacha `mod.fn` quando
o receptor é `Ident` cujo nome nao está no `TypeEnv` mas existe como
`mod.fn` no `DispatchTable`. Este case é verificdo **antes** do `infer_expr`
do receptor. O novo case `Ty::Function` + `Field` é verificado **após**
`infer_expr` (que resolve o tipo do receptor). Nao há conflito — module
access produz `Ty::Function` e seria capturado pelo novo case se seguido de
`.name`. Mas `mod.fn.name` é dois DotAccess aninhados: o interno resolve
`mod.fn` → `Ty::Function`, o externo resolve `.name`. O typeck ja lida com
isto — cada DotAccess é um nível.

### 5.7. Performance do binary search

Para N = 100 funções: ~7 comparações de `i64`. Cada comparação é 1 `load` +
1 `icmp` no codegen. Total: ~14 instruções + overhead de call. Em
nanossegundos. Para N = 1000 (projeto grande): ~10 comparações. Ainda
nanossegundos. Nao vale otimizar com perfect hash.

## 6. Não-escopo

- **Diretivas Kata (decorators):** Este PRD habilita reflexão de funções.
  Diretivas que usam `f.name` em before/after blocks dependerão deste PRD
  mas são especificadas separadamente.
- **Inspeção de body/AST:** Não expõe o corpo da função. Isto exigiria
  quotation/splicing (sistema de macros). Fora de escopo.
- **Reflexão de structs/enums:** `Pessoa.field_names` ou `Boolean.variants`
  não são cobertos. Mesmo mecanismo (sidecar table) poderia ser estendido,
  mas é um PRD separado.
- **Mudança de ABI:** Nenhuma. Função continua `I64`. Não há B-register
  nem fat ptr.
- **`f.call(args)`:** Não adiciona invocação via DotAccess. `f(args)` já
  funciona (call syntax existente).

## 7. Critérios de aceite (DoD)

1. `f.name` retorna `List::Text` com o nome de cada overload no caso estático.
2. `f.arity`, `f.param_types`, `f.return_type`, `f.is_action` retornam
   `List::*` correspondentes no caso estático (sempre lista).
3. `f.(Int Int).arity` desambigua e retorna `Int` escalar (overload específica).
4. `f.(Int Int)` resolve para `Lambda(Int Int -> Int)` como valor.
5. No caso dinâmico (`g := f.(Int Int); g.arity`), o binary search executa
   em O(log N) e retorna escalar.
6. Código que não usa reflexão não tem overhead de runtime.
7. Lambdas atribuídas via `let` usam o nome do binding (lista de length 1).
8. Actions suportam os mesmos fields que functions, sempre estático, sempre
   lista.
9. `let g := processar` (action) continua sendo erro de tipo (restrição
   preservada — actions não são first-class).
10. TypeEnv registra todas as overloads (não sobrescreve).
11. Monomorfização gera entries com nomes qualificados (functions).
12. Todos os testes E2E passam.
13. `cargo build --workspace` sem warnings novos.