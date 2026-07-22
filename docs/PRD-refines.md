# PRD — `refines`: Delegação de Interface para Tipos Refinados + T?

**Status:** 🚧 Rascunho
**Data:** 2026-07-21
**Depende de:** Refined types ✅ (data com predicados), DispatchTable ✅, InterfaceRegistry ✅
**Inclui:** T? — açúcar sintático para `Result::(T, Err)` (Fase 8, mesmo PRD)
**Não depende de:** Widening implícito (rejeitado), Ok implícito (rejeitado), `?` no-op (rejeitado)

## 1. Objetivo

Permitir que tipos refinados (`data (Int, > _ 0) as PositiveInt`) reutilizem
as implementações de interface do seu tipo base, sem criar overloads novos.
O typeck faz fallback no dispatch: quando `+ a b` falha porque os args são
`PositiveInt`, substitui pelo tipo base e retenta. Se o retorno implementa a
interface sendo refinada, passa pelo construtor falível do refined →
`Result::(Refined, Err)`. Se não implementa, retorna direto.

O usuário declara intenção com a keyword `refines`:

```kata
data (Int, > _ 0) as PositiveInt

PositiveInt refines NUM
```

Isso **não** registra PositiveInt no InterfaceRegistry e **não** cria overloads
no DispatchTable. Registra apenas: "PositiveInt delega NUM, base é Int."

SHOW é automático para todos os tipos, inclusive refined — não precisa de
`refines SHOW`. O `show_synthesis.rs` é estendido para tratar refined como
caso especial (mostra o valor base).

## 2. Sintaxe

### 2.1. Declaração `refines`

```
TipoRefinado refines INTERFACE
```

- `TipoRefinado` é um tipo refined declarado com `data (Base, predicados) as Nome`.
- `INTERFACE` é uma interface já declarada e já implementada pelo tipo base.
- Bloco indentado opcional com métodos parciais (caso misto, §2.2).

Sem bloco: delega todos os métodos da interface ao tipo base.

Com bloco: métodos com corpo = implementação do usuário (override); métodos
não-listados = delegação automática ao base.

### 2.2. Caso misto (partial delegation)

```kata
PositiveInt refines NUM
    - :: PositiveInt PositiveInt => PositiveInt
        lambda a b:
            match (PositiveInt (- a b))
                Result::Ok(v): v
                otherwise: 0::PositiveInt
    # +, *, <, >, = delegados automaticamente
```

Método com corpo lambda = override explícito do usuário. Cria overload real
no DispatchTable (encontrado antes do fallback). Métodos não-listados usam
o fallback automático.

**Restrições do corpo lambda:** lambda é função pura — `?` (operador de
runtime, exclusivo de Actions) e `panic!` (Action builtin) não existem.
Para desempacotar `Result` em lambda, usar `match` explícito. O tipo de
retorno pode ser `PositiveInt?` (açúcar de tipo para
`Result::(PositiveInt, Err)`) se o override propaga falha, ou `PositiveInt`
nu se o override resolve o erro internamente (ex: clamp via ascription
literal `0::PositiveInt`).

### 2.3. Restrições

- `refines` só se aplica a tipos refined (StructInfo com `alias_of.is_some() &&
  predicates.is_some()`). Aplicar a struct não-refined ou alias puro → erro
  compile-time.
- A interface deve já estar implementada pelo tipo base no InterfaceRegistry.
  Se o base não implementa a interface → erro compile-time.
- O tipo base é resolvido seguindo `alias_of` no StructRegistry.

## 3. Semântica

### 3.1. Fallback no dispatch — sem overloads sintetizados

`refines` **não** cria overloads no DispatchTable. O mecanismo é um fallback
em `apply.rs`, executado quando o dispatch normal falha:

1. Verificar se **todos** os args refined são o **mesmo** tipo refined.
2. Consultar `refines_registry` para o tipo.
3. Verificar se `func_name` é método de alguma interface que o refined delega.
4. Substituir todos os args refined pelo tipo base.
5. Retentar dispatch com os tipos base.
6. Se encontrado, examinar o tipo de retorno:
   - Se o retorno **implementa a interface** → passar pelo construtor falível
     do refined → `Result::(Refined, Err)`.
   - Se o retorno **não implementa a interface** → retornar direto, sem
     construtor.

### 3.2. Regra do construtor condicional

O construtor é chamado **só** quando o tipo de retorno implementa a interface
sendo refinada. A verificação é feita consultando o InterfaceRegistry.

`PositiveInt refines NUM` (base Int, interface NUM):

| Método | Fallback | Retorno | Implementa NUM? | Resultado |
|---|---|---|---|---|
| `+` | `+ :: Int Int => Int` | Int | Sim | `PositiveInt(resultado)` → `Result::(PositiveInt, Err)` |
| `-` | `- :: Int Int => Int` | Int | Sim | `PositiveInt(resultado)` → `Result::(PositiveInt, Err)` |
| `*` | `* :: Int Int => Int` | Int | Sim | `PositiveInt(resultado)` → `Result::(PositiveInt, Err)` |
| `<` | `< :: Int Int => Boolean` | Boolean | Não | `Boolean` direto |
| `>` | `> :: Int Int => Boolean` | Boolean | Não | `Boolean` direto |
| `=` | `= :: Int Int => Boolean` | Boolean | Não | `Boolean` direto |

O construtor falível já existe (sintetizado por `constructors_refined.rs`).
Ele avalia: (1) o tipo base é o esperado? (predicado implícito de tipo) e
(2) os predicados matemáticos (`> _ 0`). Se qualquer check falha → `Err`.

### 3.3. Interoperabilidade com tipos da interface — só com `refines`

Sem `refines`, PositiveInt **não interoperar** com Int. `+ a 0` onde
`a :: PositiveInt` e `0 :: Int` falha — não há overload e não há fallback.

Com `PositiveInt refines NUM`, o fallback passa a existir. PositiveInt
interage com qualquer tipo que implemente NUM da mesma forma que Int
interagiria:

`+ a b` onde `a :: PositiveInt, b :: Int`:
- Dispatch normal falha (não há `+ :: PositiveInt Int => ...`).
- Fallback: substitui PositiveInt por Int → `+ :: Int Int => Int` → encontrado.
- Retorno Int implementa NUM → construtor → `Result::(PositiveInt, Err)`.

`+ a b` onde `a :: PositiveInt, b :: Float`:
- Fallback: substitui → `+ :: Int Float => ...` → não existe hoje → falha.
- Se um dia existir `+ :: Int Float => Float`, funciona. Retorno Float não
  implementa NUM → retorna Float direto, sem construtor.

`+ a 0` onde `a :: PositiveInt, 0 :: Int` (com `refines NUM`):
- Fallback: substitui → `+ :: Int Int => Int` → encontrado.
- Retorno Int implementa NUM → construtor → `Result::(PositiveInt, Err)`.
- Funciona. O usuário optou in ao declarar `refines NUM`.

Sem `refines NUM`, o mesmo `+ a 0` falha. O fallback não existe.

### 3.4. Incompatibilidade nominal entre refineds distintos

Dois tipos refined sobre a mesma base, mesmo com os mesmos predicados, são
**nominalmente incompatíveis**. O fallback só dispara quando **todos** os args
refined são o **mesmo** tipo. Se há refineds diferentes, o fallback não
substitui nenhum — não há overload, falha.

```kata
data (Int, > _ 0) as PositiveInt
data (Int, > _ 0) as NonZeroInt

PositiveInt refines NUM
NonZeroInt refines NUM
```

`+ a b` onde `a :: PositiveInt` e `b :: NonZeroInt` → **falha**. Os refineds
são diferentes. O fallback não dispara. Atrito nominal preservado.

Para combinar refineds distintos, o usuário faz downcast explícito: `+ (a::Int)
b` ou `+ a (b::Int)` (quando downcast via `::` existir).

### 3.5. Caso misto — método com corpo (override)

Quando o usuário fornece um corpo lambda para um método, o typeck registra
um overload real no DispatchTable (como `implements` faz hoje). O dispatch
normal encontra esse overload antes de tentar o fallback. O corpo do usuário
é responsável por chamar o FFI do base e o construtor do refined se necessário.

Métodos não-listados no bloco usam o fallback automático.

### 3.6. SHOW é automático para todos os tipos

SHOW é automático para **todos** os tipos — sem exceção. O
`show_synthesis.rs` é estendido para garantir que todo tipo tenha uma
implementação de `show`, sintetizada quando o usuário não fornece uma
manual.

Hoje `show_synthesis.rs` cobre:
- Structs com campos → sintetizado.
- Enums → sintetizado.
- Primitives (Int, Float, Text, Boolean, Rational) → `implements SHOW`
  manual na stdlib.

Hoje **não** cobre:
- Structs sem campos (incluindo refined) → pulados (`fields.is_empty()`).
- Qualquer tipo user-defined sem `implements SHOW` manual.

O fix é: antes de pular structs sem campos, verificar se o struct é refined
(`alias_of.is_some() && predicates.is_some()`). Se sim, sintetizar
`show :: Refined => Text` chamando o show do tipo base (FFI direto, ex:
`kata_rt_bi_show` para base Int). Para structs sem campos que não são
refined, sintetizar `show :: Struct => Text` com body `TextLit("StructName")`
(representação trivial — não há campos para mostrar).

`echo!(a)` onde `a :: PositiveInt` funciona sem `refines SHOW`, sem
declaração do usuário. `echo!(x)` para qualquer `x` de qualquer tipo
funciona.

### 3.7. InterfaceRegistry

`refines` **não** registra o tipo no InterfaceRegistry. PositiveInt não é
formalmente NUM. `soma :: T implements NUM => T T => T` não aceita
PositiveInt. O polimorfismo via interface fica para quando T? existir com
semântica de subtyping (pós-1.0).

### 3.8. Relação com `implements`

| | `implements` | `refines` |
|---|---|---|
| Quem usa | Qualquer tipo | Apenas tipos refined |
| Corpo | Usuário escreve | Fallback no typeck (ou override do usuário) |
| InterfaceRegistry | Registra | Não registra |
| Polimorfismo via interface | Sim | Não |
| DispatchTable | Cria overloads | Não cria (exceto override) |
| Retorno de métodos que devolvem tipo que implementa a interface | O que o usuário escrever | `Result::(Refined, Err)` via construtor |
| Retorno de métodos que devolvem tipo que não implementa a interface | O que o usuário escrever | Direto do base |

Um tipo pode ter ambos: `implements` para interfaces que define explicitamente,
`refines` para interfaces que delega ao base.

## 4. Fases de implementação

### Fase 1: Lexer — token `Refines`

- `crates/kata-lexer/src/ident.rs`: adicionar `"refines" => Token::Refines`.
- `crates/kata-ast/src/token.rs`: adicionar variante `Refines` ao enum `Token`,
  atualizar `Display` e `is_keyword()` (ou equivalente).

**Verificação:** `cargo check -p kata-lexer --all-targets`

### Fase 2: AST — `RefinesDecl`

- `crates/kata-ast/src/item.rs`: adicionar variante `RefinesDecl` ao enum `Item`.
  Campos: `type_name: String`, `interface_name: String`, `methods: Vec<ImplMethod>`.
  Sem `type_params` ou `iface_params` — refined types não são genéricos em 1.0.

**Verificação:** `cargo check --workspace --all-targets`

### Fase 3: Parser — `parse_refines_decl`

- `crates/kata-parser/src/declarations.rs`: adicionar `is_refines_start()`
  (lookahead por `Token::Refines` após `Ident`), adicionar branch no dispatch
  de top-level items.
- `crates/kata-parser/src/interface_decl.rs`: adicionar `parse_refines_decl()`.
  Reusa a lógica de parse de bloco de métodos de `parse_implements_decl`, mas:
  - Sem type params do tipo (refined não é genérico).
  - Sem iface params (interfaces não-parametrizadas em 1.0).
  - Retorna `Item::RefinesDecl`.

**Verificação:** `cargo test -p kata-parser --all-targets`

### Fase 4: Resolution — processar `RefinesDecl` + RefinesRegistry

- `crates/kata-resolution/src/pass0.rs`: adicionar match arm para `RefinesDecl`.
  - Validar que `type_name` é refined (StructInfo com `alias_of` e `predicates`).
  - Resolver o tipo base via `alias_of` no StructRegistry.
  - Validar que o base implementa a interface no InterfaceRegistry.
  - Registrar entrada no `RefinesRegistry`: `type_name → (base_type, interface_name)`.
  - Métodos com corpo de usuário (override): processar como `ImplementsDecl`
    normal — cria overload real no DispatchTable.
- `crates/kata-core/src/refines_registry.rs` (novo): estrutura que mapeia
  `type_name → Vec<(base_type, interface_name)>`. Um tipo pode refinar
  múltiplas interfaces.
- `crates/kata-resolution/src/lib.rs`: exportar `RefinesRegistry`.

**Verificação:** `cargo test -p kata-resolution --all-targets`

### Fase 5: Inference — fallback no dispatch

- `crates/kata-inference/src/infer/apply.rs`: após `match_score` falhar e antes
  de retornar `NoOverload`, adicionar o caminho de fallback:
  1. Verificar se todos os arg_types são `Ty::Struct(name)` onde `name` é o
     **mesmo** tipo e tem entrada no `RefinesRegistry`.
  2. Se sim, verificar se `func_name` é método de alguma interface que o
     refined delega (consultar InterfaceRegistry para listar métodos da
     interface).
  3. Substituir todos os args refined pelo tipo base.
  4. Retentar dispatch: `ctx.table.resolve(func_name, base_arg_types, ...)`.
  5. Se encontrado, examinar o tipo de retorno:
     - Se o retorno implementa a interface (InterfaceRegistry) → o tipo
       esperado é o refined. O codegen precisa emitir chamada ao construtor
       falível do refined sobre o resultado. O TAST reflete isso:
       `Apply(Ident(refined_name), [Closure(base_ffi, args)])`.
       O tipo de retorno do TAST é `Result::(Refined, Err)`.
     - Se o retorno não implementa a interface → retornar o TAST do base
       direto, sem construtor. O tipo de retorno é o do base.
  6. Se não encontrado → `NoOverload` original.
- `crates/kata-inference/src/infer/mod.rs`: passar `RefinesRegistry` no
  `InferCtx`.

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 6: SHOW automático para todos os tipos

- `crates/kata-inference/src/infer/show_synthesis.rs`: garantir que todo tipo
  tenha `show`:
  - Structs com campos → já sintetizado (caso existente).
  - Enums → já sintetizado (caso existente).
  - Refined (struct sem campos com `alias_of` e `predicates`) → sintetizar
    `show :: Refined => Text` chamando o show do tipo base (FFI direto).
  - Struct sem campos não-refined → sintetizar `show :: Struct => Text` com
    body `TextLit("StructName")` (representação trivial).
  - Tipos com `implements SHOW` manual → já cobertos (skip, como hoje).
- Registrar `Refined implements SHOW` no InterfaceRegistry (mostrar o valor
  é legítimo — retorno é Text, não afeta predicados).
- `echo!(x)` funciona para qualquer `x` de qualquer tipo.

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 7: Testes E2E

Testes em `crates/kata-codegen/tests/` ou `crates/kata-driver/tests/`, nomeados
por responsabilidade:

- `refines_num_arithmetic.kata` — `PositiveInt refines NUM`, `+ a b` retorna
  `Result::(PositiveInt, Err)`, desempacotar com `?` em Action.
- `refines_num_comparison.kata` — `< a b` retorna `Boolean` direto, sem Result.
- `refines_show_automatic.kata` — `echo!(a)` onde `a :: PositiveInt` funciona
  sem `refines SHOW`. Mostra o valor base.
- `refines_partial_delegation.kata` — método com corpo override, outros delegados.
- `refines_mixed_args.kata` — `+ a b` onde `a :: PositiveInt, b :: Int` funciona
  (fallback substitui PositiveInt por Int) **com** `refines NUM`.
- `refines_sem_refines_falha.kata` — sem `refines NUM`, `+ a 0` onde
  `a :: PositiveInt` falha (sem fallback, sem overload).
- `refines_atrito_nominal.kata` — `+ a b` onde `a :: PositiveInt` e
  `b :: NonZeroInt` falha (refineds distintos não interoperam, mesmo ambos
  com `refines NUM`).
- `refines_base_not_implementing.kata` — erro compile-time quando base não
  implementa a interface.
- `refines_non_refined_type.kata` — erro compile-time quando tipo não é refined.
- `refines_predicado_reprova.kata` — operação que produz valor inválido
  (ex: `- 1 5` com PositiveInt) retorna `Err`, não crash.
- `refines_incompatibilidade_nominal.kata` — `+ a b` onde `a :: PositiveInt`
  e `b :: NonZeroInt` falha (refineds distintos não interoperam).

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed.

### Fase 8: T? — açúcar sintático para `Result::(T, Err)`

`T?` é açúcar puro de sintaxe de tipo. Lê-se "T ou falha". Desaçuca para
`Result::(T, Err)` em todo lugar onde aparece um tipo: assinaturas de função,
tipos de retorno, tipos de campos, ascriptions.

`PositiveInt?` ≡ `Result::(PositiveInt, Err)`. `Int?` ≡ `Result::(Int, Err)`.

O `?` como operador de runtime **não muda** — continua significando uma coisa
só: desempacotar `Result`. Aplicar `?` em não-Result é erro de tipo, como hoje.

Sem subtyping, sem widening, sem Ok implícito. `Int` não cabe em `Int?` — são
tipos distintos. Quem precisa de `Int?` retorna `Result::(Int, Err)` explícito
(o que o açúcar `Int?` produz).

#### Implementação

- `crates/kata-ast/src/expr.rs`: adicionar variante `TypeExpr::Question(Box<Spanned<TypeExpr>>)`
  — representa `T?` na AST de tipos.
- `crates/kata-parser/src/types.rs`: no `parse_type_expr_inner`, após parsear
  um tipo base, verificar se o próximo token é `Token::Question`. Se sim,
  consumir e envolver em `TypeExpr::Question`.
- `crates/kata-resolution/src/type_resolve.rs`: em `resolve_type_expr`, adicionar
  arm para `TypeExpr::Question(inner)`:
  ```rust
  TypeExpr::Question(inner) => {
      let inner_ty = resolve_type_expr(&inner.node, env, iface_reg);
      Ty::Generic("Result".into(), vec![inner_ty, Ty::text()])
  }
  ```
  `Err` é `Text` (mensagens de erro), consistente com o construtor falível
  que já usa `Result::(T, Text)` em `constructors_refined.rs`.

#### Uso com `refines`

O usuário pode escrever o tipo de retorno de uma função que recebe refined
como `PositiveInt?` em vez de `Result::(PositiveInt, Err)`:

```kata
soma_positiva :: PositiveInt PositiveInt => PositiveInt?
lambda a b: PositiveInt (+ a b)
```

Isso é apenas açúcar — o typeck resolve `PositiveInt?` para
`Result::(PositiveInt, Err)` antes de qualquer verificação.

#### O que T? **não** faz

- **Não cria subtyping.** `Int` não é subtipo de `Int?`.
- **Não cria Ok implícito.** Uma função que retorna `Int` não satisfaz
  `=> Int?` sem wrap explícito.
- **Não muda o operador `?`.** `?` em runtime continua sendo
  desempacotamento de Result. Não é no-op em não-Result.
- **Não habilita polimorfismo via interface.** PositiveInt continua não
  sendo NUM no InterfaceRegistry. T? é açúcar de escrita, não de semântica.

#### Testes

- `t_question_desugar.kata` — `PositiveInt?` em assinatura é equivalente a
  `Result::(PositiveInt, Err)`.
- `t_question_in_field.kata` — campo de struct com tipo `T?`.
- `t_question_not_subtype.kata` — função que retorna `Int` não satisfaz
  `=> Int?` sem wrap explícito (erro de tipo).

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed.

## 5. Fora do escopo

- **Ok implícito / `?` no-op / widening** — rejeitado nesta conversa. O
  construtor é chamado só quando o retorno implementa a interface. T? é
  açúcar de escrita, não cria subtyping.
- **Polimorfismo via interface** — PositiveInt não é NUM no InterfaceRegistry.
  Funções genéricas `soma :: T implements NUM => ...` não aceitam PositiveInt.
- **Interfaces parametrizadas** — `refines ITERABLE(A)` fica para depois.
  Refined types não são genéricos em 1.0.
- **Propagação de predicado** — `+ a b` retorna `Result::(PositiveInt, Err)`,
  não `PositiveInt` direto. O usuário desempacota. Propagar o predicado
  automaticamente (provar que `+ a b` preserva `> _ 0`) é pós-1.0.
- **Downcast via `::`** — `a::Int` onde `a :: PositiveInt` é uma feature
  complementar, discutida separadamente. Não está neste PRD.

## 6. DoDs (Definitions of Done)

1. `PositiveInt refines NUM` sem bloco delega todos os métodos de NUM ao base
   via fallback no dispatch.
2. `+ a b` onde `a, b :: PositiveInt` retorna `Result::(PositiveInt, Err)`.
3. `< a b` onde `a, b :: PositiveInt` retorna `Boolean` direto (Boolean não
   implementa NUM — sem construtor).
4. `echo!(x)` funciona para qualquer `x` de qualquer tipo, inclusive refined
   (SHOW automático universal, sem `refines SHOW`).
5. Caso misto: método com corpo override é encontrado antes do fallback;
   outros delegados.
6. `+ a b` onde `a :: PositiveInt, b :: Int` funciona **com** `refines NUM`
   (fallback substitui PositiveInt por Int).
7. `+ a 0` onde `a :: PositiveInt` e `0 :: Int` **falha sem** `refines NUM`
   (sem fallback, sem overload — interoperabilidade é opt-in).
8. `+ a b` onde `a :: PositiveInt, b :: NonZeroInt` falha mesmo com ambos
   `refines NUM` (refineds distintos não interoperam — incompatibilidade
   nominal).
9. Aplicar `refines` a tipo não-refined → erro compile-time.
10. Aplicar `refines` a interface que o base não implementa → erro compile-time.
11. `PositiveInt?` em assinatura é açúcar para `Result::(PositiveInt, Err)`.
12. `Int` não satisfaz `=> Int?` sem wrap explícito (sem Ok implícito).
13. `?` em runtime continua sendo desempacotamento de Result — não é no-op
    em não-Result.
14. `cargo test --workspace --no-fail-fast` passa sem regressão.
15. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 7. Arquitetura — componentes afetados

```
crates/kata-lexer/src/ident.rs              # "refines" → Token::Refines
crates/kata-ast/src/token.rs                # variante Refines + Display
crates/kata-ast/src/item.rs                 # variante RefinesDecl
crates/kata-parser/src/declarations.rs      # is_refines_start(), dispatch
crates/kata-parser/src/interface_decl.rs   # parse_refines_decl()
crates/kata-parser/tests/parser_test/       # testes do parser
crates/kata-core/src/refines_registry.rs    # RefinesRegistry (novo)
crates/kata-resolution/src/pass0.rs         # processar RefinesDecl, popular RefinesRegistry
crates/kata-resolution/src/lib.rs           # exportar RefinesRegistry
crates/kata-inference/src/infer/apply.rs    # fallback no dispatch
crates/kata-inference/src/infer/mod.rs     # passar RefinesRegistry no InferCtx
crates/kata-inference/src/infer/show_synthesis.rs  # SHOW automático para refined
crates/kata-codegen/tests/                 # testes E2E
crates/kata-driver/tests/                  # testes E2E end-to-end
# Fase 8: T?
crates/kata-ast/src/expr.rs                # TypeExpr::Question
crates/kata-parser/src/types.rs           # parse T? após tipo base
crates/kata-resolution/src/type_resolve.rs # resolve Question → Result::(T, Text)
docs/Kata-lang-manual.md                    # §4.2.10 emendar: opt-in via refines
docs/sintaxe-mapa.md                        # adicionar refines, T?
```

## 8. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `refines` não cria overloads no DispatchTable | O fallback no typeck substitui refined por base e retenta dispatch. Reusa overloads existentes do base. Sem síntese de TypedFunctions, sem poluição do DispatchTable. |
| D2 | `refines` não registra no InterfaceRegistry | O retorno `Result::(Refined, Err)` não satisfaz `=> NUM`. Registrar PositiveInt como NUM seria uma mentira no type system. |
| D3 | Construtor chamado só quando retorno implementa a interface | Boolean não implementa NUM — `< a b` retorna Boolean direto. Int implementa NUM — `+ a b` passa pelo construtor. A regra é única e simples: o construtor valida o retorno quando o retorno está na "família" da interface. |
| D4 | Fallback só dispara quando todos os args refined são o mesmo tipo | Preserva incompatibilidade nominal entre refineds distintos. `+ a b` onde `a :: PositiveInt, b :: NonZeroInt` falha — não há overload e o fallback não substitui. |
| D5 | Interoperabilidade com tipos da interface é opt-in via `refines` | Sem `refines`, PositiveInt não interoperar com Int. `+ a 0` falha. Com `refines NUM`, o fallback substitui PositiveInt por Int e funciona. O usuário declara intenção explicitamente. |
| D6 | SHOW é automático para todos os tipos, sem `refines` | SHOW não tem predicados para avaliar — mostra o valor. Estender `show_synthesis.rs` para cobrir refined (mostra base) e structs sem campos (mostra nome). Sem declaração do usuário. |
| D7 | Caso misto: método com corpo = override no DispatchTable | Override cria overload real, encontrado antes do fallback. Métodos não-listados usam fallback. Parser já aceita métodos com e sem body. |
| D8 | `refines` só para tipos refined | A semântica de "delegar + avaliar predicado no retorno" só faz sentido para refined. Para non-refined, `implements` com `@ffi` já existe. |
| D9 | Sem type_params/iface_params em 1.0 | Refined types não são genéricos. Interfaces parametrizadas (ITERABLE) são ortogonais e ficam para depois. |
| D10 | Keyword `refines` em vez de `delegates` | `delegates` exigiria preposição (`delegates to`). `refines` é uma palavra, conecta com o conceito de refined type, e lê naturalmente: "PositiveInt refines NUM". |
| D11 | `refines` vs `implements` como keywords distintas | Semânticas distintas: `implements` registra no InterfaceRegistry e cria overloads; `refines` não registra e usa fallback. Manter separadas evita ambiguidade no typeck. |
| D12 | T? é açúcar puro, sem subtyping | `T?` desaçuca para `Result::(T, Err)` no parser/resolution. Não cria relação de subtipo entre T e T?. Não cria Ok implícito. Não muda o operador `?` de runtime. É apenas uma forma mais curta de escrever `Result::(T, Err)`. |
| D13 | T? usa `Err = Text` | Consistente com o construtor falível de `constructors_refined.rs` que já usa `Result::(T, Text)`. Mensagens de erro são Text. |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| FFI do base recebe PositiveInt mas espera Int | Em runtime, PositiveInt é Int (mesmo bits, refined é só type tag). FFI não distingue. Sem custo. |
| Construtor do refined falha em runtime para operações válidas | O predicado é avaliado no resultado. Se o resultado viola o predicado (ex: `- 1 5` = -4 para PositiveInt), o construtor retorna `Err`. O usuário desempacota com `?` (Action) ou match (função pura). É o atrito sadio. |
| Conflito entre SHOW automático e `show_synthesis.rs` | `show_synthesis.rs` já pula structs sem campos. O fix é adicionar um check de refined antes de pular. Se refined, sintetiza show do base. |
| Caso misto: método override precisa chamar construtor manualmente | O usuário escreve `match (PositiveInt (- a b))` no corpo da lambda. `?` (operador de runtime) e `panic!` (Action builtin) não existem em lambdas — são exclusivos de Actions. O override usa `match` explícito para desempacotar, ou ascription literal (`0::PositiveInt`) para resolver o erro internamente. É o padrão documentado no manual §4.2.2. |
| `refines` em tipo não-refined passa pelo parser | O parser aceita a sintaxe. O erro é detectado no resolution (pass0.rs) quando consulta o StructRegistry. Mensagem clara: "refines só se aplica a tipos refined". |
| Fallback ambíguo: dois refineds diferentes na mesma call | O fallback só dispara quando todos os args refined são o mesmo tipo. Se há refineds diferentes, não substitui nenhum. Não há ambiguidade. |
| `+ a 0` onde `a :: PositiveInt, 0 :: Int` — funciona com `refines NUM`, falha sem | Intencional. Sem `refines`, não há fallback — PositiveInt é nominalmente distinto de Int. Com `refines NUM`, o usuário optou in à interoperabilidade. O construtor valida o resultado. |
| Conflito entre SHOW automático e `implements SHOW` manual | `show_synthesis.rs` já verifica `has_manual_show` antes de sintetizar. Implementação manual tem prioridade. |

## 10. Atualização da documentação

Ao concluir:
- `docs/PRD-refines.md` — este arquivo (status → concluído)
- `docs/Kata-lang-manual.md` — §4.2.10 emendar: "mediante declaração explícita
  de `refines`, o compilador delega as operações da interface ao tipo base"
- `docs/sintaxe-mapa.md` — adicionar `refines` e `T?` à lista de keywords