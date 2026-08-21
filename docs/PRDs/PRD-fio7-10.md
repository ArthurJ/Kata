# PRD: Fio 7+10 — Interfaces, Generics, Dispatch, Módulos, Prelude Kata

## Objetivo

Trazer interfaces (`interface NUM`), implementações (`Int implements NUM`), dispatch
por dominância com múltiplas overloads, generics (parametric polymorphism),
monomorphização nos call sites, `@commutative`, sistema de módulos (`import`/`export`),
module loader com filesystem e cycle detection, prelude reescrito em Kata
(`stdlib/core.kata` substituindo `prelude_sigs.rs`), e `Complex` como exemplo
canônico de tipo numérico implementado inteiramente em Kata sem `@ffi`.

Este é o fio que **unifica o prelude com a linguagem**: antes dele, o prelude é
hardcoded em Rust (`prelude_sigs.rs`). Depois dele, o prelude é um módulo Kata
como qualquer outro, e `+` despacha via `implements NUM for Int` em vez de
assinatura hardcoded.

Fusão dos Fios 7 e 10 da árvore de dependências original. A fusão é natural:
interfaces precisam do prelude em Kata para serem demonstradas (os tipos
primitivos implementam NUM/ORD/EQ/SHOW), e o prelude em Kata precisa de
`import`/`export` e module loader para ser carregado.

## Depende de

- **Fio 1** (TypeEnv, DispatchTable com scoring, `Ty`, `data` opaco, `enum` unitário, `::` postfix)
- **Fio 2** (lambdas, guards, match, Hole `_`, `hint: Option<&Ty>` top-down, partial dispatch)
- **Fio 4** (`Ty::Sum` com payload, `Result::(T, E)`, `Optional::T`, `Ty::Generic` para enums genéricos)
- **Fio 5** (`Ty::Struct`, `StructRegistry`, smart constructor infalível, `alias`, `format`, ~~`$` spread~~ removido)
- **Fio 6** (ascription-refined, ret-directed dispatch, `fits_return`)

## Estado herdado

O Fio 6 deixou infraestrutura pronta para este fio:

- **`Ty::Generic(String, Vec<Ty>)`** já existe (usado para `Result::(T,E)`, `Optional::T`).
  Fio 7 estende seu uso para generics de função e de tipo definidos pelo usuário.
- **`Ty::Var(String)`** já existe para type params nomeados (`T`, `E`).
- **`Ty::InferVar(u32)`** já existe para vars de inferência internas.
- **`DispatchTable`** com `Score { exact, alias, refined, iface, is_generic_origin }`.
  O campo `iface` é sempre zero hoje — Fio 7 o popula.
- **`commutative: HashSet<String>`** já existe no DispatchTable — nunca populado.
- **`OverloadInfo.is_generic: bool`** já existe — sempre `false`.
- **`match_score`** em `dispatch.rs` (kata-core) é `pub fn` — já exposta para o ret-directed dispatch.
- **`fits_return`** em `expr.rs` — direcional, recursiva em Generic.
- **Múltiplas overloads por nome** já funcionam (Fio 6 testou com overloads injetadas manualmente).
- **`prelude_sigs.rs`** — catálogo hardcoded em Rust. Fio 10 substitui por `stdlib/core.kata`.
- **Parser** reconhece `interface` e `implements` como keywords (`Token::Ident` matched
  por nome, não `Token::Interface` — o lexer não tem keywords além de um conjunto
  mínimo). Fio 7 precisa adicionar o parsing real.

## Decisões de design

### D1: Monomorphização em crate separado

Pipeline: lex → parse → resolve → infer → **monomorph** → optimize → codegen.

`kata-monomorph` é um novo crate que recebe o `TypedModule` (TAST com tipos
genéricos) e produz um `MonoModule` (TAST com tipos concretos). Cada call site
genérico é especializado — `List(Int)` vs `List(Text)` geram funções distintas
no codegen.

### D2: `implements` sempre tem corpo e registra no DispatchTable

`implements NUM for Int` traz as assinaturas concretas no bloco indentado.
O typeck registra cada método como overload no DispatchTable, substituindo
`Self` e nomes de interface pelo tipo concreto.

Para evitar duplicidade com `prelude_sigs.rs` durante a transição:
- **Fase 1-7**: interfaces + generics + monomorph são implementados. O prelude
  continua em `prelude_sigs.rs`.
- **Fase 8**: prelude migra para `stdlib/core.kata`. `implements NUM for Int`
  passa a trazer o corpo com `@ffi("kata_rt_bi_add")`. `prelude_sigs.rs` é removido.
- Em nenhum momento há duas fontes registrando a mesma overload.

### D3: `Ty::Interface(String)` — variant separado em `Ty`

Interfaces não são structs. Não têm campos, construtores, `alias_of`, ou
`predicates`. Usar `Ty::Struct("NUM")` com flag seria forçar uma relação
que não existe. `Ty::Interface(String)` é semanticamente correto.

Interfaces parametrizadas (`ITERABLE(A)`) usam `Ty::Generic("ITERABLE", [A])`
que já existe — não precisa de variant novo para o caso parametrizado.

A cascata de match arms em `Ty` é mecânica — `cargo check` lista todos os
sites E0004, cada um recebe um arm `Ty::Interface(_) => ...`. `shape.rs`
mapeia `Interface` para `Unit` graceful (igual `InferVar` e `Generic` não-resolvido).

### D4: Escopo de interfaces base — ORD, EQ, NUM, SHOW no Fio 7

ITERABLE, COUNTABLE, INDEXABLE ficam para Fio 8 onde coleções existem.
O Fio 7 define a infraestrutura de interfaces, e ORD/EQ/NUM/SHOW são as
interfaces usadas nos DoDs. Complex implementa NUM/ORD/EQ/SHOW.

### D5: `implements` é separado da declaração do tipo

```kata
data Complex (re::Float im::Float)

implements NUM for Complex
    + := lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
    ...
```

Permite implementar a mesma interface em tipos definidos em outros módulos
(orphan rule via `alias`).

### D6: Nome da interface nas assinaturas = "qualquer tipo que implementa"

`+ :: NUM NUM => NUM` significa "qualquer tipo que implementa NUM" — não
necessariamente o mesmo tipo em ambos os argumentos. Isso habilita
interoperabilidade: `+ Complex Int` é válido se ambos implementam NUM.

O tipo novo define cláusulas para tipos existentes + cláusula genérica
(`T NUM`) como fallback:

```kata
implements NUM for Complex
    + :: Complex Complex => Complex
    + :: Complex Int => Complex
    + :: Int Complex => Complex
    + :: T NUM => Complex          # fallback genérico
```

O dispatch scoring 4D reconhece o fallback: `iface++` no Score quando o
argumento implementa a interface esperada.

### D7: `Self` nas assinaturas de interface

`Self` é uma palavra especial dentro do bloco `interface`. Refere-se ao
tipo que implementa a interface. Na implementação, é substituído pelo tipo
concreto.

```kata
interface ITERABLE(A)
    next :: Self => Optional(A)
```

Na implementação `List(A) implements ITERABLE(A)`, `Self` vira `List(A)`.

### D8: `@commutative` — parser + typeck

`@commutative` é diretiva anexada a assinaturas. O parser já coleta
diretivas em `Vec<Directive>`. O resolution registra no
`commutative: HashSet<String>` do DispatchTable quando encontra
`@commutative` numa `Sig`. O dispatch tenta args invertidos quando 0
candidatos compatíveis são encontrados e a função é commutative com arity 2.

## Fases

### Fase 1: Parser — `interface`, `implements`, `import`, `export`

**Objetivo:** parser reconhece as novas declarações e produz AST.

**AST novos variants em `Item`:**

```rust
/// `interface NOME implements SUPER1 SUPER2 ...` + bloco indentado de assinaturas.
InterfaceDecl {
    name: String,
    supertraits: Vec<String>,
    /// Type params da interface (ex: `A` em `ITERABLE(A)`).
    type_params: Vec<String>,
    /// Assinaturas obrigatórias (apenas `::` e `=>`, sem corpo).
    signatures: Vec<InterfaceSig>,
},

/// `Tipo implements Interface` + bloco indentado com métodos.
ImplementsDecl {
    type_name: String,
    /// Type params do tipo (ex: `A` em `List(A) implements ITERABLE(A)`).
    type_params: Vec<String>,
    interface_name: String,
    /// Params da interface vinculados (ex: `A` em `ITERABLE(A)`).
    iface_params: Vec<String>,
    /// Métodos: assinaturas concretas + corpo (lambda ou @ffi).
    methods: Vec<ImplMethod>,
},

/// `import modulo.submodulo` ou `import modulo.submodulo as alias`
/// ou `import modulo.(Item1 Item2)`.
ImportDecl {
    path: Vec<String>,
    alias: Option<String>,
    items: Option<Vec<String>>,  // None = import tudo, Some = seletivo
},

/// `export item1 item2 ...` ou `export MOD.(itens)`.
ExportDecl {
    items: Vec<ExportItem>,
},
```

**Estruturas auxiliares:**

```rust
/// Assinatura dentro de interface — sem corpo, sem diretivas.
struct InterfaceSig {
    name: String,
    params: Vec<Spanned<TypeExpr>>,
    ret: Spanned<TypeExpr>,
}

/// Método dentro de implements — assinatura + corpo.
struct ImplMethod {
    name: String,
    params: Vec<Spanned<TypeExpr>>,
    ret: Spanned<TypeExpr>,
    directives: Vec<Directive>,
    body: Option<Vec<Spanned<LambdaClause>>>,  // None = FFI (precisa @ffi)
}
```

**Parser:**
- `Token::Ident("interface")` em posição de top-level → `parse_interface_decl`
- `Token::Ident("implements")` em posição de top-level → `parse_implements_decl`
  - Sintaxe: `Tipo implements Interface` ou `Tipo(params) implements Interface(params)`
- `Token::Ident("import")` em posição de top-level → `parse_import_decl`
- `Token::Ident("export")` em posição de top-level → `parse_export_decl`
- `Token::Ident("Self")` em posição de tipo → `TypeExpr::SelfRef` (novo variant)

**TypeExpr novo variant:**
```rust
/// `Self` — referência ao tipo que implementa a interface.
/// Válido apenas dentro de blocos `interface` e `implements`.
SelfRef,
```

**Verificação:** `cargo check -p kata-ast && cargo check -p kata-parser`

### Fase 2: Resolution — InterfaceRegistry, module loader básico

**Objetito:** resolution registra interfaces e impls, carrega módulos importados.

**kata-core — novos tipos:**

```rust
/// Interface registrada no InterfaceRegistry.
pub struct InterfaceInfo {
    pub name: String,
    pub supertraits: Vec<String>,
    pub type_params: Vec<String>,
    pub signatures: Vec<InterfaceSignature>,
}

/// Assinatura dentro de interface.
pub struct InterfaceSignature {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

/// Implementação registrada.
pub struct ImplEntry {
    pub type_name: String,
    pub type_params: Vec<String>,
    pub interface_name: String,
    pub iface_params: Vec<String>,
    pub methods: Vec<ImplMethodInfo>,
}

/// Método dentro de impl.
pub struct ImplMethodInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub ffi_symbol: Option<String>,
}
```

**`InterfaceRegistry` em kata-core:**

```rust
pub struct InterfaceRegistry {
    interfaces: HashMap<String, InterfaceInfo>,
    impls: Vec<ImplEntry>,
}

impl InterfaceRegistry {
    pub fn new() -> Self;
    pub fn register_interface(&mut self, info: InterfaceInfo) -> Result<(), TypeError>;
    pub fn register_impl(&mut self, entry: ImplEntry) -> Result<(), TypeError>;
    pub fn get_interface(&self, name: &str) -> Option<&InterfaceInfo>;
    pub fn get_impls_for_type(&self, type_name: &str) -> Vec<&ImplEntry>;
    pub fn get_impls_for_interface(&self, iface_name: &str) -> Vec<&ImplEntry>;
    pub fn type_implements(&self, type_name: &str, iface_name: &str) -> bool;
    /// DFS com HashSet de visiting para detectar ciclos de supertraits.
    fn check_cycle(&self, iface: &str, visiting: &mut HashSet<String>) -> Result<(), TypeError>;
    pub fn merge(&mut self, other: InterfaceRegistry);
}
```

**`ResolvedModule` ganha `interface_registry: InterfaceRegistry`.**

Cascata: adicionar campo em `ResolvedModule` dispara atualização em todos os
`merge_resolved` (~20 arquivos de teste + driver + prelude_sigs). Usar `sed`
para `Vec::new()` e patch manual para merges com `extend`.

**Resolution Pass 0+:**
- `InterfaceDecl` → registra no `InterfaceInfo` (valida supertraits, detecta ciclos)
- `ImplementsDecl` → registra no `ImplEntry` (valida que interface existe, que
  tipo existe, que métodos conformam ao contrato)
- `ImportDecl` → carrega módulo do filesystem (Fase 3), faz merge do `ResolvedModule`
- `ExportDecl` → marca itens como exportados

**Validação de conformidade:**
Para cada `implements NUM for Complex`, o resolution verifica:
1. Interface `NUM` existe no `InterfaceRegistry`
2. Tipo `Complex` existe no `TypeEnv` ou `StructRegistry`
3. Cada assinatura da interface tem um método correspondente no impl
4. O método tem os mesmos params (com `Self` substituído) e retorno compatível

**Verificação:** `cargo test -p kata-core && cargo test -p kata-resolution`

### Fase 3: Module Loader — filesystem, cache, cycle detection

**Objetivo:** carregar `import` do filesystem com cache e detecção de ciclos.

**`ModuleLoader` em kata-resolution (ou novo crate `kata-module-loader`):**

```rust
pub struct ModuleLoader {
    cache: HashMap<PathBuf, Arc<ResolvedModule>>,
    loading: HashSet<PathBuf>,  // para detectar ciclos
    search_paths: Vec<PathBuf>,
}
```

- `load(path)` → resolve path → checa cache → se não está no cache, parse + resolve
  → registra no cache → retorna
- Se path está em `loading` → ciclo detectado → erro
- Paths de busca: diretório do arquivo atual + `stdlib/` (para prelude)
- `stdlib/core.kata` é carregado automaticamente como prelude

**Sintaxe de import:**
```kata
import utilidades.matematica              # módulo inteiro
import utilidades.matematica as mat       # com alias
import utilidades.(matematica TipoX NUM)  # seletivo
```

**Sintaxe de export:**
```kata
export + - TipoX                          # itens separados por espaço
export tipos.(Int Float Boolean)          # reexportação
```

**Verificação:** `cargo test -p kata-resolution`

### Fase 4: Inference — dispatch com interfaces, `iface++` no Score ✅

**Objetivo:** o match_score reconhece quando um argumento implementa a interface
esperada e pontua `iface++`.

**Mudança em `match_score` (kata-core/src/dispatch.rs):**

Para cada par `(arg, param)`:
- Se `arg == param` → `exact++` (já existe)
- Se alias match → `alias++` (já existe)
- Se refined subtype → `refined++` (já existe)
- **NOVO:** Se `param` é `Ty::Interface(iface_name)` e `arg` implementa `iface_name`
  (via `InterfaceRegistry::type_implements`) → `iface++`
- **NOVO:** Se `param` é `Ty::Generic(iface_name, _)` e `arg` implementa a interface
  instanciada → `iface++`
- Nenhum → `Score::incompatible()`

O `InterfaceRegistry` precisa ser acessível no `match_score`. Opções:
- Passar `&InterfaceRegistry` como parâmetro para `match_score` (muda assinatura)
- Guardar referência no `DispatchTable` (acoplamento)

**Decisão:** passar `&InterfaceRegistry` como parâmetro. `match_score` já é `pub fn`
e recebe args diretamente. O caller (`infer_apply`) tem acesso ao `InterfaceRegistry`
via `InferCtx`.

**`InferCtx` ganha `interface_registry: &'a InterfaceRegistry`.**

Cascata: 5 sites de construção do InferCtx (mod.rs:3, expr.rs:1,
constructors_refined.rs:1) + quaisquer outros. Buscar com
`search_files pattern="InferCtx \{"`.

**Quando o typeck encontra `Ty::Interface` como tipo de parâmetro:**
- O tipo real do argumento é concreto (`Int`, `Complex`, etc.)
- O dispatch verifica se o tipo concreto implementa a interface
- Se sim, `iface++` e o candidato é compatível
- Se múltiplos candidatos compatíveis → scoring por dominância
- Empate total → `AmbiguousDispatch`

**Retorno de função com interface:**
- Se a função retorna `Ty::Interface("NUM")`, o tipo concreto é determinado pelo
  body. Se o body é `+ a b` onde `a: Complex, b: Complex`, o retorno é `Complex`.
- O typeck resolve o tipo concreto a partir do body, não do tipo declarado.

**Verificação:** `cargo test -p kata-inference`

### Fase 5: Generics — `Ty::Generic` para funções e tipos definidos pelo usuário ✅

**Objetivo:** funções e tipos genéricos definidos pelo usuário, com type params
que são resolvidos nos call sites.

**AST:**
- Assinaturas com type params: `id :: T => T` (onde `T` é `TypeExpr::Var("T")`)
- O parser já produz `TypeExpr::Var` para identificadores em posição de tipo
- O resolution precisa registrar `T` como type param, não como tipo concreto

**Resolution:**
- Assinatura `id :: T => T` → `Signature` com `type_params: vec!["T"]`
  e `params: vec![Ty::Var("T")]`, `ret: Ty::Var("T")`
- `OverloadInfo` ganha `type_params: Vec<String>` (nomes dos type params)
  e `substitutions: Option<HashMap<String, Ty>>` (instanciação nos call sites)

**Inference:**
- Quando `infer_apply` encontra uma overload com `type_params` não-vazio:
  1. Cria `HashMap<String, Ty>` vazio para substitutions
  2. Para cada par `(arg_ty, param_ty)`:
     - Se `param_ty` é `Ty::Var(name)` e `name` está em `type_params`:
       - Se `name` já está nas substitutions → unifica (verifica compatibilidade)
       - Se não → `substitutions.insert(name, arg_ty)`
     - Se `param_ty` é concreto → match normal
  3. Aplica substitutions no tipo de retorno
  4. `OverloadInfo.is_generic = true` na TAST

**Unificação `unify`:**
- `unify(param, arg, substitutions) -> Result<HashMap<String, Ty>, TypeError>`
- Casamento posicional param→arg:
  - `Var(name)` → se já tem substitution, verifica `arg == existing`; senão, insere
  - `Generic(name, params)` → unifica cada param recursivamente
  - Outros → verifica `param == arg`
- Não é union-find — é casamento posicional top-down, reusando o padrão de Kata4

**`InferVar` resolution via dispatch:**
Quando partial dispatch encontra `InferVar` numa posição onde o overload espera
`Var(T)`, resolve `InferVar := T` via `unify`. Extensão do partial dispatch de
Fio 2 para suportar type params genéricos.

**Verificação:** `cargo test -p kata-inference`

### Fase 6: Monomorphização — crate `kata-monomorph` ✅

**Objetivo:** especializar call sites genéricos em funções concretas.

**Novo crate: `kata-monomorph`**

Recebe `TypedModule` (TAST com tipos genéricos) → produz `MonoModule` (TAST
com tipos concretos). Cada call site genérico é substituído por uma chamada
para uma função especializada.

**Algoritmo:**
1. Coletar todos os call sites genéricos na TAST (funções com
   `is_generic: true` ou `type_params` não-vazio)
2. Para cada call site, extrair as substitutions concretas (tipos dos argumentos)
3. Para cada combinação única de (função, substitutions), gerar uma instância
   monomorfizada:
   - Nome único: `original_name_T_Int` (ou hash se complexo)
   - Substituir todos os `Ty::Var("T")` pelo tipo concreto no body
   - Registrar como nova `TypedFunction` no module
4. Substituir o call site genérico por uma chamada para a instância monomorfizada
5. Repetir até fixpoint (instâncias monomorfizadas podem ter novos call sites genéricos)

**`mono_instance: u64` na TAST:**
Cada `TypedExpr` que resulta de monomorphização carrega um identificador único
da instância. Isso permite ao codegen rastrear qual especialização gerou cada nó.

**`OverloadInfo.substitutions` em `OverloadInfo`:**
- `None` para funções não-genéricas
- `Some(map)` para instâncias monomorfizadas

**Pipeline atualizado:**
```
lex → parse → resolve → infer → monomorph → optimize → codegen
```

`kata-codegen` recebe `MonoModule` em vez de `TypedModule`. Se `MonoModule`
não tem genéricos (módulo sem generics), é idêntico ao `TypedModule`.

**Verificação:** `cargo test -p kata-monomorph && cargo test -p kata-codegen`

### Fase 7: `@commutative` no dispatch ✅

**Objetivo:** funções marcadas com `@commutative` tentam args invertidos quando
0 candidatos compatíveis são encontrados.

**Parser:** `@commutative` já é coletado em `Vec<Directive>` pelo parser.

**Resolution:** quando encontra `@commutative` numa `Sig`, registra o nome da
função no `commutative: HashSet<String>` do DispatchTable.

**Dispatch:** no algoritmo de resolução, se passo 2 (FILTRAR) produz 0 candidatos
e a função está no `commutative` set e arity == 2:
- Tenta args invertidos: `match_score([arg2, arg1], params)`
- Se encontra candidatos → seleciona
- Se ainda 0 → continua para AmbiguousDispatch/NoOverload

**Verificação:** `cargo test -p kata-inference`

### Fase 8: Prelude em Kata — `stdlib/core.kata` ✅

**Objetivo:** substituir `prelude_sigs.rs` por um módulo Kata carregado do filesystem.

**`stdlib/core.kata`:**

```kata
# Tipos primitivos
data Int () @ffi("i64")
data Float () @ffi("f64")
data Text () @ffi("kata_rt_string")
data Rational () @ffi("kata_rt_rat")

# Boolean
enum Boolean
    True
    False

# Enums genéricos
enum Result
    Ok(T)
    Err(E)

enum Optional
    Some(T)
    None

# Interfaces base
interface EQ
    = :: Self Self => Boolean

interface ORD implements EQ
    < :: Self Self => Boolean
    > :: Self Self => Boolean
    <= :: Self Self => Boolean
    >= :: Self Self => Boolean

interface NUM implements ORD EQ
    + :: NUM NUM => NUM
    - :: NUM NUM => NUM
    * :: NUM NUM => NUM
    abs :: NUM => NUM
    div :: NUM NUM => Result::(NUM, Err)

interface SHOW
    show :: Self => Text

# Implementações para Int
implements NUM for Int
    + :: Int Int => Int @ffi("kata_rt_bi_add")
    - :: Int Int => Int @ffi("kata_rt_bi_sub")
    * :: Int Int => Int @ffi("kata_rt_bi_mul")
    abs :: Int => Int @ffi("kata_rt_bi_abs")
    div :: Int Int => Result::(Int, Err) @ffi("kata_rt_bi_div")
    < :: Int Int => Boolean @ffi("kata_rt_bi_lt")
    > :: Int Int => Boolean @ffi("kata_rt_bi_gt")
    <= :: Int Int => Boolean @ffi("kata_rt_bi_le")
    >= :: Int Int => Boolean @ffi("kata_rt_bi_ge")
    = :: Int Int => Boolean @ffi("kata_rt_bi_eq")

implements SHOW for Int
    show :: Int => Text @ffi("kata_rt_bi_show")

# Implementações para Float (similar)
implements NUM for Float
    + :: Float Float => Float @ffi("kata_rt_fadd")
    ...

implements SHOW for Float
    show :: Float => Text @ffi("kata_rt_float_to_text")

# Implementações para Rational (similar)
implements NUM for Rational
    ...

implements SHOW for Rational
    show :: Rational => Text @ffi("kata_rt_rat_show")

# Implementações para Text
implements EQ for Text
    = :: Text Text => Boolean @ffi("kata_rt_string_eq")

implements ORD for Text
    < :: Text Text => Boolean @ffi("kata_rt_string_lt")
    ...

implements SHOW for Text
    show :: Text => Text @ffi("kata_rt_string_identity")

# Operadores com @commutative
@commutative
= :: EQ EQ => Boolean

@commutative
+ :: NUM NUM => NUM
```

**Mudança no driver:**
- `load_prelude()` chama `ModuleLoader::load("stdlib/core.kata")` em vez de
  construir `ResolvedModule` manualmente
- `prelude_sigs.rs` é removido
- O `ModuleLoader` injeta `stdlib/` no search path

**Atenção:** a migração precisa ser gradual para não quebrar todos os testes E2E
de uma vez. Estratégia:
1. Escrever `stdlib/core.kata`
2. Carregar via ModuleLoader
3. Comparar `ResolvedModule` resultante com o de `prelude_sigs.rs`
4. Quando idêntico, remover `prelude_sigs.rs`
5. Atualizar todos os `merge_resolved` nos testes E2E

**Verificação:** `cargo test --workspace --no-fail-fast` (todos os testes
existentes devem passar com o prelude em Kata)

### Fase 9: Complex — exemplo canônico

**Objetivo:** demonstrar que um tipo numérico implementado inteiramente em Kata,
sem `@ffi`, funciona com dispatch via interfaces.

**`stdlib/complex.kata` (ou `examples/complex.kata`):**

```kata
import core

data Complex (re::Float im::Float)

implements NUM for Complex
    + :: Complex Complex => Complex
        lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
    - :: Complex Complex => Complex
        lambda a b: Complex (- a.re b.re) (- a.im b.im)
    * :: Complex Complex => Complex
        lambda a b: Complex (- (* a.re b.re) (* a.im b.im))
                            (+ (* a.re b.im) (* a.im b.re))
    abs :: Complex => Complex
        lambda a: Complex (sqrt (+ (* a.re a.re) (* a.im a.im))) 0.0
    div :: Complex Complex => Result::(Complex, Err)
        lambda a b:
            let denom := + (* b.re b.re) (* b.im b.im)
            Complex (/ (- (* a.re b.re) (* a.im b.im)) denom)
                    (/ (+ (* a.re b.im) (* a.im b.re)) denom)
    < :: Complex Complex => Boolean
        lambda a b: < (+ (* a.re a.re) (* a.im a.im))
                      (+ (* b.re b.re) (* b.im b.im))
    > :: Complex Complex => Boolean
        lambda a b: > (+ (* a.re a.re) (* a.im a.im))
                      (+ (* b.re b.re) (* b.im b.im))
    = :: Complex Complex => Boolean
        lambda a b: and (= a.re b.re) (= a.im b.im)

implements SHOW for Complex
    show :: Complex => Text
        lambda a: format "({} + {}i)" (a.re, a.im)
```

**Testes E2E:**
- `Complex 3.0 4.0` constrói
- `+ (Complex 1.0 2.0) (Complex 3.0 4.0)` → `Complex 4.0 6.0` via dispatch
- `show (Complex 3.0 4.0)` → `"(3.0 + 4.0i)"` via SHOW
- `+ (Complex 1.0 0.0) 5` → `Complex 6.0 0.0` via interoperabilidade NUM
- `* (Complex 2.0 3.0) (Complex 1.0 1.0)` → `Complex -1.0 5.0`

**Verificação:** testes E2E em `crates/kata-codegen/tests/fio7_complex_e2e.rs`

## DoDs (Definition of Done)

| # | Descrição | Fase |
|---|---|---|
| 1 | `interface NUM implements ORD EQ` parseia e registra no InterfaceRegistry | 1-2 |
| 2 | `implements NUM for Int` parseia e registra overloads no DispatchTable | 1-2 |
| 3 | `import modulo` carrega de filesystem com cache e cycle detection | 3 |
| 4 | `export item1 item2` marca itens como exportados | 3 |
| 5 | `+ (Complex 1.0 2.0) (Complex 3.0 4.0)` despacha via `iface++` no Score ✅ | 4 |
| 6 | `id 42` infere `T = Int` via `unify` e retorna `Int` ✅ | 5 |
| 7 | `List(Int)` e `List(Text)` geram instâncias monomorfizadas distintas ✅ | 6 |
| 8 | `@commutative` em `=` tenta args invertidos quando 0 candidatos ✅ | 7 |
| 9 | `stdlib/core.kata` carregado como prelude substitui `prelude_sigs.rs` ✅ | 8 |
| 10 | `implements NUM for Int` em Kata registra `+` com `@ffi("kata_rt_bi_add")` ✅ | 8 |
| 11 | `Complex 3.0 4.0` constrói via smart constructor ✅ | 9 |
| 12 | `+ (Complex 1.0 2.0) (Complex 3.0 4.0)` → `Complex 4.0 6.0` ✅ | 9 |
| 13 | `show (Complex 3.0 4.0)` → `"(3.0 + 4.0i)"` via SHOW ✅ | 9 |
| 14 | `+ (Complex 1.0 0.0) 5` → interoperabilidade NUM (Complex + Int) ✅ | 9 |
| 15 | Todos os testes existentes passam com prelude em Kata ✅ | 8 |

## Atualização da documentação

Ao concluir o fio:
- `docs/ROADMAP.md` — marcar Fio 7 e Fio 10 como ✅ Concluído. Fundir as entradas.
- `docs/PRDs/PRD-fio7-10.md` — marcar fases e DoDs como ✅
- `docs/Kata-lang-manual.md` — **NÃO atualizar** (manual é aspiracional)
- `docs/maquinaria-interna.md` — atualizar seções de InterfaceRegistry e
  DispatchTable com o estado final implementado
- `docs/sintaxe-mapa.md` — adicionar seções para `interface`, `implements`,
  `import`, `export` se não existirem (as keywords já estão listadas)

## Regras críticas

- **Ler o manual ao iniciar.** Invariantes I1-I8. Seção §4.1 (interfaces), §4.1.1
  (interfaces parametrizadas), §4.2 (smart constructors).
- **Testes SEMPRE em `tests/` separado**, não inline no `src/`.
- **Edições cegas proibidas.** `write_file` exige leitura COMPLETA prévia.
  `patch` tool: NÃO usar `new_string` >15 linhas Rust com aspas.
- **Manual Kata5 é aspiracional** — não propor atualizações baseado em fases
  implementadas.
- **Kata NUNCA teve `if`** — condicional = pattern matching + guards.
- **`prelude_sigs.rs` só é removido na Fase 8.** Nas Fases 1-7, o prelude
  continua em Rust e os testes E2E continuam usando `merge_resolved` com
  `prelude_sigs`.
- **Adicionar campo a `ResolvedModule` dispara cascata em ~20 arquivos.** Usar
  `sed` para `Vec::new()` e patch manual para merges.
- **Adicionar campo a `InferCtx` dispara cascata em 5+ sites.** Buscar com
  `search_files pattern="InferCtx \{"`.
- **Adicionar variant a `Ty` dispara cascata E0004 em todos os match arms.**
  `cargo check --workspace` lista todos. `shape.rs` mapeia para `Unit` graceful.
- **Adicionar variant a `Item` dispara cascata em todos os `match item`.** Buscar
  com `search_files pattern="Item::DataDecl"` para encontrar sites.
- **Adicionar FfiSymbol exige 7 lugares** (enum, symbol_name, return_type,
  from_name, all_ffi_symbols, builder.symbol, ffi_signature).
- **`kata-core` tem `kata-ast` no Cargo.toml mas NÃO usa no código** (dependência
  morta). Não introduzir uso de `Spanned<Expr>` em kata-core.
- **Arthur rejeita implementações parciais.** Reusar infra. Pede análise de
  impacto. Estados inválidos irrepresentáveis.
- **`patch` tool pode corromper arquivos com `\\n` literais** — ver pitfall #11
  do skill kata-compiler. Bypass: head/tail + write_file + cat.
- **Pitfall — PRD planning: apresentar design decisions ANTES de escrever o PRD.**
  Este PRD foi escrito após 8 decisões resolvidas com Arthur.
- **Pitfall — Adicionar campo a `TypedExpr` dispara cascata E0063 em TODOS os
  crates.** `mono_instance: u64` (se adicionado a TypedExpr em vez de
  TypedModule) afeta kata-inference + kata-optimizer + kata-codegen. Buscar
  exaustivamente com `search_files pattern="TypedExpr \{" path="crates"`.
- **Pitfall — `merge_resolved` do DRIVER precisa merge de TODOS os campos.**
  Ao adicionar `interface_registry` em `ResolvedModule`, o driver precisa
  `let mut interface_registry = prelude.interface_registry; interface_registry.merge(user.interface_registry);`.

## Estrutura final esperada

```
crates/
  kata-core/
    src/ty.rs                    # + Ty::Interface(String)
    src/interface_registry.rs    # NOVO — InterfaceRegistry, InterfaceInfo, ImplEntry
    src/dispatch.rs              # match_score ganha iface++ com &InterfaceRegistry
    src/enum_registry.rs         # inalterado
    src/struct_registry.rs       # inalterado
    src/lib.rs                   # + pub mod interface_registry
  kata-ast/
    src/expr.rs                  # + Item::InterfaceDecl, Item::ImplementsDecl,
                                  #   Item::ImportDecl, Item::ExportDecl
                                  # + TypeExpr::SelfRef
                                  # + InterfaceSig, ImplMethod structs
  kata-parser/
    src/declarations.rs          # + parse_interface_decl, parse_implements_decl
    src/imports.rs               # NOVO — parse_import_decl, parse_export_decl
    src/lib.rs                   # + mod imports
  kata-resolution/
    src/lib.rs                   # + InterfaceRegistry em ResolvedModule
                                  # + resolve de interface/implements/import/export
    src/prelude_sigs.rs          # REMOVIDO na Fase 8 (substituído por stdlib/core.kata)
    src/module_loader.rs         # NOVO — ModuleLoader com cache + cycle detection
  kata-inference/
    src/infer/
      expr.rs                    # InferCtx ganha interface_registry
      apply.rs                   # unify para generics, iface++ no dispatch
      mod.rs                     # InferCtx construção com interface_registry
  kata-monomorph/                 # NOVO — crate separado
    src/lib.rs                   # MonoModule, monomorphize()
    Cargo.toml
  kata-optimizer/                 # inalterado (TRMA não mudou)
  kata-codegen/
    src/lowering/                # recebe MonoModule em vez de TypedModule
  kata-rt/                        # inalterado
  kata-driver/
    src/main.rs                   # load_prelude via ModuleLoader (Fase 8)
stdlib/
  core.kata                       # NOVO — prelude em Kata
  complex.kata                    # NOVO — exemplo canônico
docs/
  PRD-fio7-10.md                  # ✅ fases marcadas
  ROADMAP.md                      # Fio 7+10 ✅
```