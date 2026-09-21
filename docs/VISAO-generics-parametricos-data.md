# Visão — Generics Paramétricos para `data`

**Status:** Design pronto. Implementação requer alterações em lexer, AST, resolution, inference, monomorph, e InterfaceRegistry.

**Motivação:** Hoje, `data Complex (re::Float im::Float)` é monomórfico — cada combinação de tipos de campo exige uma declaração separada. Tipos numéricos polimórficos (Complex sobre Int/Float/Rational, matrizes genéricas) precisam parametrização. O mecanismo existente de famílias/instâncias (refined polimórfico) é semanticamente inadequado para isso: famílias representam visões refinadas de tipos base existentes (`NonZero` é um `Int` restrito), não tipos novos com layout parametrizado (`Complex::(Int, Int)` é um produto novo, não uma restrição de `Int`).

---

## 1. Decisão central

Generics paramétricos com monomorfização on-demand. `data Complex (re::T im::T) where T implements SCALAR` declara type params; o monomorphizer instancia métodos on-demand para cada combinação de type args usada. Uma única entrada no StructRegistry; uma única entrada no InterfaceRegistry.

O struct em si não precisa de monomorfização — só os métodos. Todos os campos são words de 8 bytes (Int = i64, Float = f64, Rational = ponteiro opaco). `Complex::(Int, Float)` e `Complex::(Float, Int)` têm layout idêntico: 16 bytes, dois words. O invariante `offset = field_index * 8` preserva-se.

---

## 2. Sintaxe

### 2.1. Forma compartilhada — type param com bound

```kata
data Complex (re::T im::T) where T implements SCALAR
```

- `(re::T im::T)` — campos com type param T (PascalCase em posição de tipo, como `Ok(T)` em enums).
- `where T implements SCALAR` — bound: T precisa implementar SCALAR. `where` é novo keyword no lexer.
- T é o mesmo tipo em ambos os campos.

### 2.2. Forma independente — vars anônimas com bound

```kata
data Complex (re::SCALAR im::SCALAR)
```

- `SCALAR` na posição de tipo do campo é interpretado no pass0 como "var fresca anônima com bound SCALAR".
- Cada ocorrência de SCALAR é uma var distinta — desugar para `Complex (re::A im::B) where A implements SCALAR, B implements SCALAR`.
- Mecanismo análogo ao que `data (NUM, ...) as NonZero` já usa: `NUM` na posição de tipo significa "tipo que implementa NUM". A diferença é que sem `as Name` e sem predicados, é data paramétrico, não família refined.

### 2.3. Sem bounds — type param livre

```kata
data Par (first::A second::B)
```

- A e B são type params livres (sem bound). PascalCase em posição de tipo, detectados no pass0.
- Construtor aceita qualquer par de tipos.

### 2.4. Gramática

```
DataDecl ::= 'data' Name '(' Fields ')' WhereClause?
           | 'data' '(' IfacePreds ')' 'as' Name                     # família refined (atual)

WhereClause   ::= 'where' Bound (',' Bound)*
Bound         ::= PascalName 'implements' ALL_CAPS
```

Type params são detectados implicitamente: PascalName em posição de tipo nos fields é type param (como `Ok(T)` em enums). Sem lista explícita `::(...)`.

### 2.5. Token novo: `where`

- `where` é lowercase keyword. Não colide com PascalCase (tipos), ALL_CAPS (interfaces), nem snake_case (funções/variáveis).
- Adição ao lexer: uma linha em `ident.rs` (`"where" => Token::Where`).
- Adição ao AST: `Where` no enum `Token` + Display + match de exaustividade (~3 linhas em `token.rs`).

---

## 3. StructRegistry

### 3.1. Entrada única

Uma única entrada para `Complex` com campos `Ty::Var("T")`. O offset continua `field_index * 8` (invariante).

```rust
StructInfo {
    name: "Complex",
    fields: [
        FieldInfo { name: "re", ty: Ty::Var("T"), offset: 0 },
        FieldInfo { name: "im", ty: Ty::Var("T"), offset: 8 },
    ],
    type_params: Some(vec![TypeParamDecl { name: "T", bound: Some("SCALAR") }]),
    ...
}
```

`StructInfo` ganha campo `type_params: Option<Vec<TypeParamDecl>>`. `TypeParamDecl { name: String, bound: Option<String> }` — nome do param (detectado por PascalCase nos fields) e interface do bound (`None` = livre, `Some("SCALAR")` = do `where`).

### 3.2. Instanciação

Structs `data` são representados por `Ty::Struct(StructKey)`. Hoje `StructKey` é `Plain`, `Family`, ou `Instance`. Para structs paramétricos, `StructKey` ganha uma variante:

```rust
StructKey::Generic("Complex", vec![Ty::Prim(Int)])
```

Isso preserva o invariante atual: structs `data` vivem em `Ty::Struct`, enums e intrínsecos em `Ty::Generic`. Sites que fazem match `Ty::Struct(key)` continuam funcionando — extraem type args quando `key` é `Generic`. Usar `Ty::Generic` para structs paramétricos quebraria esse invariante e exigiria auditar todo site que distingue `Ty::Struct` de `Ty::Generic` (cache_key, snapshot, shape, caps, etc.).

O StructRegistry não cria entradas por combinação — o lookup resolve type args dinamicamente:

```rust
fn lookup_instantiated(&self, name: &str, type_args: &[Ty]) -> Option<InstantiatedStructInfo>
```

`InstantiatedStructInfo` substitui `Ty::Var("T")` pelos type args concretos nos tipos dos campos. O struct físico (layout) é idêntico — só os tipos annotados mudam para o type checker.

### 3.3. Não interage com StructKey::Family/Instance

`Complex` paramétrico é `StructKey::Generic("Complex", args)` com `type_params: Some(...)` no `StructInfo`. Famílias refined (`StructKey::Instance`) continuam funcionando como hoje. São mecanismos ortogonais: famílias expandem predicados sobre tipos base, generics paramétricos parametrizam layout de tipos novos.

---

## 4. Smart constructors

### 4.1. Overload genérico

Um único overload genérico é registrado no DispatchTable:

```kata
Complex :: T T => Complex    # onde T implements SCALAR
```

O `unify` em `generics.rs` já binda `Ty::Var("T")` em `type_params` para tipos concretos dos argumentos. Dado `Complex 3 4`, unify binda `T → Int`, retorna `Complex` instanciado como `Complex::(Int, Int)`. Dado `Complex 1.0 2.0`, binda `T → Float`, retorna `Complex::(Float, Float)`.

### 4.2. Verificação de bound

Após bindar T, o type checker verifica o bound: `Int implements SCALAR?` → consulta `InterfaceRegistry::type_implements("Int", "SCALAR")`. Se falha, erro cedo: `"Text não implementa SCALAR"`.

Para a forma independente (`data Complex (re::SCALAR im::SCALAR)`), o desugar gera dois params anônimos A e B, cada um com bound SCALAR. O construtor `Complex :: A B => Complex` verifica ambos os bounds.

### 4.3. match_score

`match_score` pontua `exact > iface > generic` — argumentos concretos casam com params genéricos com score `generic`. Overloads específicos (`Complex :: Int Int => Complex::(Int, Int)`) casam com score `exact` e ganham prioridade.

---

## 5. Interface impls

### 5.1. ImplEntry com type_params e bounds

```kata
Complex implements RING
    + :: Complex Complex => Complex
    lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
```

Uma única entrada no InterfaceRegistry:

```rust
ImplEntry {
    type_name: "Complex",
    type_params: vec!["T"],           // detectados dos fields, como em enums
    interface_name: "RING",
    type_bounds: vec![("T", "SCALAR")],   // campo novo — do `where`
    ...
}
```

`ImplEntry` já tem `type_params: Vec<String>`. Ganha `type_bounds: Vec<(String, String)>` — pares (param_name, iface_name), extraídos da cláusula `where` do `data`. Vazio para tipos não-genéricos ou sem bounds.

### 5.2. type_implements estendido

Hoje `type_implements(&self, type_name: &str, iface_name: &str) -> bool` compara strings. Precisa de uma versão que aceita type args:

```rust
fn type_implements_generic(
    &self,
    type_name: &str,
    type_args: &[Ty],
    iface_name: &str,
) -> bool
```

Lógica: encontra `ImplEntry` com `type_name` e `interface_name`. Se o ImplEntry tem `type_bounds`, extrai type args do `Ty::Generic`, verifica que cada arg satisfaz o bound correspondente via `type_implements(arg_type_name, bound_iface)`. Se todos satisfazem, retorna true.

### 5.3. Métodos paramétricos — uma definição, instanciada on-demand

`+ :: Complex Complex => Complex` é uma definição. O monomorphizer cria instâncias concretas (`+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)`) para cada combinação de type args que aparece em call sites alcançados.

O body `(+ a.re b.re)` usa dispatch normal: `a.re : T → + :: T T => T` resolve no DispatchTable. Após monomorfização, T é substituído por Int/Float/etc., e `+ :: Int Int => Int` é encontrado normalmente.

### 5.4. Sobrecargas cobrem coerção

Overloads explícitos como `+ :: Complex Int => Complex` continuam funcionando como hoje — são overloads adicionais com score `exact` nos args mistos. Não são o caminho genérico principal.

---

## 6. Monomorphização

### 6.1. Quando instanciar

O monomorphizer (`kata-monomorph/src/lib.rs`) já percorre a TAST procurando call sites genéricos e gera instâncias concretas em fixpoint. Para `Complex::(T, T)`, os call sites são:

1. Construtores: `Complex 3 4` → instancia `Complex::(Int, Int)`
2. Methods: `+ z1 z2` onde `z1 : Complex::(Int, Int)` → instancia `+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)`
3. Field access: `z.re` onde `z : Complex::(Int, Int)` → `re : Int`

O fixpoint já existe. A única extensão é: quando o monomorphizer encontra `Ty::Struct(StructKey::Generic("Complex", [Int, Int]))`, substitui `Ty::Var("T")` por `Ty::Prim(Int)` nos corpos dos métodos instanciados.

### 6.2. O que não precisa de instância

O struct em si — layout, offsets, alocação na arena — é idêntico para todas as combinações. O codegen não precisa de versões diferentes de `Complex::(Int, Int)` vs `Complex::(Float, Float)` para construir/destruir o struct. Só os métodos (que despacham para operações de T) precisam de instâncias concretas.

### 6.3. Tree-shaking

O tree-shaker já remove funções/actions não alcançadas. Métodos instanciados pelo monomorphizer que não são chamados são removidos. As entradas no StructRegistry e InterfaceRegistry permanecem (como já acontece hoje), mas são uma única entrada para Complex, não uma por implementor.

---

## 7. Por que não Family/Instance

### 7.1. Diferença semântica: refined view vs tipo novo

Família e generics paramétricos representam conceitos diferentes:

- **Família** = visão refinada de um tipo base existente. `NonZero` é um `Int` com predicado `!= _ (zero _)`. O `StructInfo` reflete isso: `alias_of = Some("Int")`, `predicates = Some(...)`, `fields = []`. O layout é o do tipo base — NonZero não tem campos próprios.
- **Generics paramétricos** = tipo novo cujo layout é parametrizado. `Complex::(Int, Int)` tem dois campos `re::Int` e `im::Int`. Não é uma restrição de Int — é um produto. `alias_of = None`, `fields = [re, im]`.

Adaptar famílias para Complex exigiria fingir que Complex é "uma família sobre SCALAR" — semanticamente falso. `StructInfo.is_instance_of` e `alias_of` codificariam a semântica errada: `alias_of = Some("Int")` quando Complex não é alias de Int. Os predicados da família existem para invariantes como `!= _ (zero _)`, não para declarar fields.

### 7.2. Famílias lazy já existem — mas para coleções, não para interfaces

`extract_lazy_type_param` em `pass0.rs:32` implementa famílias lazy para coleções: `data (List::A, >= (len _) 1) as NonEmpty` registra a família sem expandir instâncias, e `base_ty_subs` unifica `A` no call site. A infraestrutura de unificação lazy já está em produção.

No entanto, famílias sobre interfaces (`data (NUM, ...) as NonZero`) são eager: `extend_families_for_implementors` cria uma instância por implementor no pass0. Isso é uma escolha de implementação, não uma limitação intrínseca — poderia ser adaptada para lazy estendendo `base_ty_subs` para verificar `type_implements` em vez de consultar `Instance`s no registry.

Mesmo que famílias-sobre-interface fossem adaptadas para lazy, o problema semântico (§7.1) permanece: a representação `alias_of`/`predicates` é errada para tipos com layout próprio.

### 7.3. Bounds por-parâmetro

`where T implements SCALAR, R implements NAT` escala para múltiplos type params com bounds independentes. O mecanismo de famílias não tem onde pendurar bounds por-parâmetro — só tem "a interface da família", que é um único bound para um único base.

### 7.4. Metadata inerte — problema de integração, não arquitetural

O tree-shaker não chama `retain_by_closure` nos registries (confirmado: o método existe em ambos mas é chamado apenas no `module_loader` para filter_exports, não no tree-shaker). Isso é corrigível sem novo mecanismo. Se fosse o único argumento, não justificaria generics paramétricos — bastaria chamar `retain_by_closure` no tree-shaker. O argumento real é semântico (§7.1).

---

## 8. Estruturas afetadas

| Camada | Mudança |
|---|---|
| **Lexer** (`kata-lexer/src/ident.rs`) | Token `Where` — uma linha no match de keywords |
| **AST** (`kata-ast/src/token.rs`) | `Where` no enum `Token` + Display + exaustividade |
| **AST** (`kata-ast/src/item.rs`) | `DataDecl` ganha `type_params: Vec<TypeParamDecl>` e `where_bounds: Vec<(String, String)>` |
| **AST** (`kata-ast/src/item.rs`) | `TypeParamDecl { name: String, bound: Option<String> }` — struct nova |
| **Core** (`kata-core/src/struct_registry.rs`) | `StructKey::Generic(String, Vec<Ty>)` — nova variante para structs paramétricos |
| **Parser** | Regra de `data` detecta type params em fields (PascalCase) e consome `where` clause |
| **Resolution** (`pass0.rs`) | Registra struct com `Ty::Var` nos campos, guarda `type_params` no `StructInfo` |
| **Resolution** (`pass0.rs`) | Forma independente: detecta interface em campo, gera var anônima com bound |
| **StructRegistry** (`struct_registry.rs`) | `StructInfo` ganha `type_params: Option<Vec<TypeParamDecl>>` |
| **StructRegistry** (`struct_registry.rs`) | `lookup_instantiated(name, type_args)` — substitui vars nos tipos dos campos |
| **InterfaceRegistry** (`interface_registry.rs`) | `ImplEntry` ganha `type_bounds: Vec<(String, String)>` |
| **InterfaceRegistry** (`interface_registry.rs`) | `type_implements_generic(type_name, type_args, iface)` — unifica type args com bounds |
| **Inference** (`generics.rs`) | `unify_one` já binda `Ty::Var` em `type_params` — sem mudança estrutural |
| **Inference** (`apply_dispatch.rs`) | Smart constructor genérico: após unify, verificar bounds antes de aceitar |
| **Monomorph** (`lib.rs`) | Substituir `Ty::Var` por type args concretos nos corpos dos métodos instanciados |
| **Stdlib** (`complex.kata`) | Migração para `data Complex (re::T im::T) where T implements SCALAR` |
| **Stdlib** (`core.kata`) | `interface SCALAR extends NUM` com `one :: Self => Self` |

---

## 9. Fases

### Fase 1 — Token `where` e AST

- Adicionar `Token::Where` no lexer e AST.
- Adicionar `TypeParamDecl` e campos em `DataDecl`.
- Parser parseia `data Name (fields) where Bounds` e detecta type params por PascalCase nos fields.
- **DoD:** `cargo check` limpo. Parser aceita e rejeita sintaxe corretamente.

### Fase 2 — StructRegistry

- `StructInfo` ganha `type_params`.
- `lookup_instantiated` substitui vars nos tipos dos campos.
- pass0 registra struct com `Ty::Var` nos campos.
- **DoD:** StructRegistry consulta `Complex::(Int, Int)` e retorna campos com tipos concretos.

### Fase 3 — Smart constructor

- Overload genérico registrado no DispatchTable.
- `unify` binda type params a partir dos argumentos.
- Verificação de bound após bind.
- **DoD:** `Complex 3 4` tipa como `Complex::(Int, Int)`. `Complex "a" "b"` falha com "Text não implementa SCALAR".

### Fase 4 — SCALAR com one

- `interface SCALAR extends NUM` com `one :: Self => Self`.
- `zero :: Self => Self` já está em RING (herdado por NUM/FIELD/SCALAR).
- Int e Float implementam SCALAR com `one` via FFI ou literais.
- **DoD:** `(one x)` tipa como Int quando `x : Int`, Float quando `x : Float`.

### Fase 5 — InterfaceRegistry

- `ImplEntry` ganha `type_bounds`.
- `type_implements_generic` unifica type args com bounds.
- `Complex::(T, T) implements RING` registra uma única entrada.
- **DoD:** `type_implements_generic("Complex", [Int, Int], "RING")` retorna true.

### Fase 6 — Monomorphização de métodos

- Monomorphizer substitui `Ty::Var` por type args concretos nos corpos.
- Instância on-demand por combinação de type args usada.
- **DoD:** `+ z1 z2` onde `z1 : Complex::(Int, Int)` despacha para `+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)` com body `(+ a.re b.re)` onde `a.re : Int`.

### Fase 7 — Migração da stdlib

- `complex.kata` migra para generics paramétricos.
- Overloads específicos (`+ :: Complex Int => Complex`) mantidos como overloads adicionais.
- **DoD:** `cargo test` passa. `kata run` em exemplos de Complex produz output correto.

---

## 10. Fora do escopo

- **Higher-kinded types:** `Functor::(F)` onde F é um type constructor não entra. Type params são sempre de kind `*`.
- **Generics em enum:** `enum Result` já é genérico via convenção UPPER_CASE. Este PRD é sobre `data` only.
- **Const generics:** Dimensões no tipo (ex: `Matrix::(3, 3)`) não entram. Shapes ficam em runtime.
- **Default methods em interface:** Interfaces com `default_body` já existem. Não muda com este PRD.
- **Refinados polimórficos:** `data (NUM, ...) as NonZero` continua funcionando. É mecanismo ortogonal (expande predicados, não parametriza layout).

---

## 11. Decisões de design

### 11.1. `where` em vez de bounds inline

**Escolha:** `data Complex (re::T im::T) where T implements SCALAR`.
**Alternativa rejeitada:** `data Complex (re::T implements SCALAR im::T)` — mistura declaração estrutural (campos) com constraints (bounds) na mesma linha. `where` separa as duas fases: primeiro os campos, depois as restrições. Mais legível e escala para múltiplos bounds.

### 11.2. Type params implícitos em vez de lista explícita `::(...)`

**Escolha:** Type params detectados por PascalCase em posição de tipo nos fields, como já funciona em enums (`Ok(T)`).
**Alternativa rejeitada:** `data Complex::(T) (re::T im::T)` — lista explícita de type params na declaração. Redundante: todo type param aparece em pelo menos um field (type param não usado no layout é type-level only, fora do escopo). A sintaxe `::(...)` pertence à instanciação (`Complex::(Int, Int)`), não à declaração.

### 11.3. Generics paramétricos em vez de adaptar famílias

**Escolha:** Mecanismo novo de generics paramétricos para `data` com layout parametrizado.
**Alternativa rejeitada:** Adaptar famílias-sobre-interface para ser lazy e suportar tipos com fields. Rejeitada por duas razões: (1) semântica — `alias_of`/`predicates` em `StructInfo` codificam "visão refinada de tipo base", não "tipo novo com fields"; Complex não é alias de Int. (2) bounds por-parâmetro — famílias têm um único bound (a interface da família), `where` suporta bounds independentes por type param. A adaptação exigiria reinterpretar a representação inteira de StructInfo para um caso que não é o que ela foi projetada para representar.

### 11.4. Struct não monomorfiza, só métodos

**Escolha:** Uma entrada no StructRegistry para Complex. Layout é idêntico para todas as combinações.
**Alternativa rejeitada:** Uma entrada por combinação. Não há ganho — offset é `field_index * 8` independente do tipo do campo. Só os métodos (que despacham para operações de T) precisam de versões concretas.

### 11.5. `StructKey::Generic` em vez de `Ty::Generic` para structs paramétricos

**Escolha:** `Ty::Struct(StructKey::Generic("Complex", [Int, Int]))` representa structs paramétricos.
**Alternativa rejeitada:** `Ty::Generic("Complex", [Int, Int])` — quebra o invariante atual de que `Ty::Struct` representa `data` e `Ty::Generic` representa enums/intrínsecos. Sites que fazem match `Ty::Struct` vs `Ty::Generic` (cache_key, snapshot, shape, caps) precisariam ser auditados. `StructKey::Generic` preserva o invariante: todo `data` vive em `Ty::Struct`, independente de parametrização.

### 11.6. SCALAR declara `one`

**Escolha:** SCALAR formaliza `one :: Self => Self` — o construtor de valor neutro de `*`. `zero :: Self => Self` já está em RING e é herdado por NUM/FIELD/SCALAR. Ambos são unários: o argumento é testemunha do tipo, permitindo despacho correto após monomorfização.
**Alternativa rejeitada:** Generalização parcial — methods com args são paramétricos, `one` fica como overload específico por combinação. Reintroduz o problema de uma definição por combinação nos overloads. A generalização parcial é pior que nenhuma — cria a ilusão de que uma definição serve para todas as combinações quando não serve.

### 11.7. Forma independente desugara para compartilhada

**Escolha:** `data Complex (re::SCALAR im::SCALAR)` desugara para `data Complex (re::A im::B) where A implements SCALAR, B implements SCALAR`.
**Alternativa rejeitada:** Tratar a forma independente como mecanismo separado. O desugar reusa a mesma infraestrutura — um mecanismo, duas sintaxes.

---

## 12. Riscos

### 12.1. Cascata de novo campo em StructInfo/ImplEntry

`StructInfo` ganha `type_params` e `ImplEntry` ganha `type_bounds`. Pitfall documentado: novo campo em struct/enum toca ~10 sites (clonagem, comparação, serde, testes). Mitigação: usar `Option<Vec<...>>` com `None` como default para tipos não-genéricos — minimiza mudança em sites existentes.

### 12.2. `type_implements` string-based

`type_implements` é chamado em dezenas de sites com `&str`. A versão genérica (`type_implements_generic`) é uma função nova, não um patch. Mitigação: manter `type_implements` original intacto; adicionar `type_implements_generic` como função separada. Sites que precisam de generics chamam a nova.

### 12.3. Intereração com famílias refined

Famílias (`StructKey::Instance`) e generics paramétricos (`StructKey::Generic`) coexistem. Um tipo não pode ser ambos. Mitigação: pass0 rejeita `data (NUM, ...) as NonZero` com type params nos fields — família refined não aceita type params.

### 12.4. SCALAR quebra builtins

Redefinir `interface SCALAR` parcialmente pode sombrear métodos do prelude (pitfall documentado: declarar `interface NUM` só com `+` sombreia `*`). Mitigação: SCALAR é declarada uma vez no prelude com todos os métodos. Implementors de SCALAR (Int, Float, Rational) fornecem todos os métodos.

### 12.5. `match_score` com verificação de bounds

Hoje `match_score` compara tipos estruturalmente sem consultar `InterfaceRegistry`. Ligar verificação de bounds ao dispatch — para que `Complex :: T T => Complex` só case quando T implementa SCALAR — exige consultar `InterfaceRegistry` no meio do dispatch, o que hoje não acontece. É o ponto mais arriscado do plano. Mitigação: a verificação de bounds pode ficar no inference (após `unify` bindar T), não no `match_score` — `match_score` continua estrutural, e `apply_dispatch` rejeita após bind se o bound falha.