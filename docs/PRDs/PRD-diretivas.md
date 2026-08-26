# PRD — Diretivas Customizadas Kata

**Status:** ✅ Implementado (Fases 1-6 completas)
**Data:** 2026-08-08 (revisão: constructo `directive`)
**Substitui:** `docs/visao-diretivas-kata.md` (documento de exploração — ideias absorvidas e refinadas aqui)
**Pré-requisito:** Variáveis de reflexão (`_name`, `_arity`, etc.) substituem a sidecar table e o `kata_rt_fn_meta_lookup` do PRD de reflexão de funções (`docs/PRDs/PRD-fn-reflection.md`, status obsoleto).

## 0. Resumo

`directive` é um constructo de linguagem que define um hook de instrumentação.
O usuário declara uma diretiva com `directive nome{when: ..., on: ...}` e a aplica
com `@nome` em actions ou funções. O compilador **inlinea** o body da diretiva no
ponto de injeção, antes do typeck, com as variáveis de reflexão sintetizadas
como `let` bindings no escopo do body inlined.

Diretivas **não são actions** — não estão no `dispatch_table`, não são chamáveis
diretamente. `trace!()` não resolve no typeck. O body da diretiva é um template
que o desugaring copia para dentro da função decorada.

As diretivas intrínsecas existentes (`@ffi`, `@builtin`, `@commutative`,
`@associative`, `@cache`, `@test`, `@log`) **não migram** — continuam chumbadas
no compilador com suas semânticas específicas. Este PRD introduz apenas o
sistema de diretivas **customizadas** via `directive`.

---

## 1. Objetivo

Permitir que o usuário crie hooks de instrumentação (tracing, logging, auth,
profiling) sem modificar o compilador. Hoje, `@log` é a única diretiva de
instrumentação disponível, e seu poder (interpolação de template, policies de
canal, herança de config) é chumbado no codegen. Com diretivas customizadas, o
usuário escreve tracing, auth gates, ou qualquer hook de entrada/saída como
diretiva.

### Princípios de design

- **Constructo mínimo.** `directive` é o único novo keyword. A aplicação
  `@nome` reusa a sintaxe de diretivas que já existe. Não há meta-diretiva
  `@directive`.
- **Inlining, não chamada.** O body da diretiva é copiado para dentro da
  função decorada, não chamado. Variáveis de reflexão são `let` bindings no
  escopo inlined, não parâmetros implícitos. Isto resolve o polimorfismo de
  `_args`/`_return` sem generics — cada site de inlining type-checka com os
  tipos concretos da função decorada.
- **Zero overhead de runtime para bindings estáticos.** `_name`, `_arity`,
  `_types`, `_return_type`, `_is_action` são resolvidos em compile-time por
  substituição direta de AST — literais no body inlined. Sem sidecar table,
  sem `kata_rt_fn_meta_lookup`.
- **Desugaring antes do typeck.** O desugaring de diretivas acontece entre a
  resolução de módulos e a inferência, produzindo AST expandida que o typeck
  valida normalmente.
- **Garantia de pureza por impossibilidade estrutural.** `ShortCircuit` e
  `Transform` só decoram actions (`Target::Action`). Funções puras não podem
  ser interceptadas — o compilador rejeita a combinação na declaração.
- **Cada braço tem sua própria assinatura.** Overloading por `(when, on)` —
  cada combinação de Hook e Target é uma declaração distinta que type-checka
  independentemente no inlining.

---

## 2. Sintaxe

### 2.1. Declaração de diretiva

```kata
directive log{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, "enter: " + _name)
```

`directive` é um constructo de declaração (como `action`, `enum`, `data`). O
header é `directive nome{when: Hook::..., on: Target::...}`. O body é uma
sequência de statements (`ActionStmt`), idêntico ao body de uma action.

O dict `{when: ..., on: ...}` é parseado pelo mesmo mecanismo que
`parse_directive_args` (ramo `{}`) — `{key: value}` onde value é `Expr` livre.
`Hook::Enter` parseia como `Expr::VariantQual` e o resolution valida contra
`enum Hook` do prelude.

Campos do dict:

| Campo | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `when` | `Hook` | **sim** | Ponto de injeção |
| `on` | `Target` | **sim** | Que tipo de item pode decorar |

### 2.2. Enums de configuração

```kata
enum Hook
    Enter           # injeta no prólogo (antes do corpo)
    Exit            # injeta no epílogo (após o corpo, observacional)
    ShortCircuit    # decide se o corpo executa (retorna Optional)
    Transform       # modifica o resultado após o corpo executar

enum Target
    Action          # só decora actions
    Function        # só decora funções puras
    Any             # decora ambos
```

**Restrições estruturais:**

- `ShortCircuit` exige `Target::Action`. O compilador rejeita `ShortCircuit`
  com `Target::Function` ou `Target::Any`.
- `Transform` exige `Target::Action`. Mesma restrição.
- `Enter` e `Exit` aceitam qualquer `Target`.

A garantia de pureza das funções é por impossibilidade (o compilador rejeita
`ShortCircuit`/`Transform` com `Target::Function`/`Any` na declaração), não
por convenção. O leitor de código Kata5 nunca precisa auditar uma diretiva
para saber se uma função pura é segura.

### 2.3. Aplicação de diretiva

```kata
@log
action processar(x :: Int) => Int
    x + 1
```

O compilador encontra `@log`, resolve `log` no `DirectiveRegistry`, e
inlinea o body conforme o `when` e `on` declarado. Múltiplas diretivas podem
ser empilhadas no mesmo item:

```kata
@trace_enter
@trace_exit
action processar(x :: Int) => Int
    x + 1
```

### 2.4. Múltiplas declarações por diretiva (overloading por Hook e Target)

Uma diretiva pode ter **múltiplas declarações com o mesmo nome**, cada uma com
`when` e/ou `on` diferentes. O nome agrupa; `when` e `on` distinguem. Ao
aplicar `@log` num item, o compilador inlinea **todas** as declarações
aplicáveis ao Hook e ao Target daquele item — simultaneamente.

```kata
directive log{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, "enter action: " + _name)

directive log{when: Hook::Enter, on: Target::Function}
    log!(LogLevel::Info, "enter function: " + _name)

directive log{when: Hook::Exit, on: Target::Any}
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
```

Uso — uma anotação dispara todos os hooks cujo `when` e `on` casam:

```kata
@log
action processar(x :: Int) => Int
    x + 1
```

Desugaring (conceptual — Target::Action casa com Enter::Action e Exit::Any):

```kata
action processar(x :: Int) => Int
    # ── inlined: directive log{Enter, Action} ──
    let _name := "processar"
    let _arity := 1
    let _types := ["Int"]
    let _return_type := "Int"
    let _is_action := True
    let _args := (x,)
    log!(LogLevel::Info, "enter action: " + _name)
    # ── body original ──
    let __result := x + 1
    # ── inlined: directive log{Exit, Any} ──
    let _name := "processar"
    let _arity := 1
    let _return_type := "Int"
    let _is_action := True
    let _args := (x,)
    let _return := __result
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
    # ── retorno ──
    __result
```

Se `processar` fosse função pura, casaria com Enter::Function e Exit::Any —
action e function disparam hooks diferentes para o mesmo nome de diretiva.

O **nome da diretiva é o nome da declaração** — não há campo de agrupamento
nem constructo sintático de bloco. Múltiplas declarações com o mesmo nome
coexistem quando diferem por `when` e/ou `on`.

### 2.5. Regras de validação de overloading

1. Duas declarações com o mesmo nome e o **mesmo** par `(when, on)` é
   conflito — erro compile-time.
2. O compilador permite múltiplas declarações com o mesmo nome no mesmo
   escopo quando diferem pelo par `(when, on)`. Isto é overloading por Hook
   e Target, discriminado em compile-time.
3. `on` pode diferir entre as declarações da mesma diretiva — cada
   Hook/Target tem necessidades diferentes (Enter não tem `_return`, Exit
   tem; ShortCircuit/Transform exigem `Target::Action`).
4. `Target::Any` só pode coexistir com outras declarações da mesma diretiva
   se for a única definição para aquele `when`. Para um dado `(nome, when)`,
   ou você tem `on: Any` ou `on: Action`/`on: Function`, mas não mistura `Any`
   com específico. O compilador rejeita a mistura na declaração.
5. `directive` e `action` com o mesmo nome no mesmo escopo é conflito — erro.
   Diretivas não são actions; o namespace é disjunto.

### 2.6. Resolução

Quando o compilador encontra `@nome` num item:

1. É intrínseca conhecida (`@ffi`, `@cache`, `@log`, etc.)? → trata como
   anotação hardcoded (comportamento existente, sem mudança).
2. Senão, resolve `nome` no `DirectiveRegistry` → encontrou? → inlinea o
   body conforme `when` e `on` declarados.
3. Senão → erro: `nome não é uma diretiva válida`.

O `DirectiveRegistry` é populado durante o resolution, processando
`Item::DirectiveDecl`. A validação de `@nome` aplicada em `Item::ActionDecl`
e `Item::Sig` consulta o registry. Isto exige dois passes no resolution:
primeiro coletar todas as declarações de diretiva, depois validar as
aplicações de `@nome` nos outros items.

---

## 3. Variáveis de reflexão

O body de uma diretiva tem acesso a **variáveis de reflexão** — bindings
prefixados com `_` que o desugaring sintetiza como `let` bindings no escopo
do body inlined, referenciando a entidade decorada.

| Variável | Tipo | Origem | Disponível em |
|---|---|---|---|
| `_name` | `Text` | estático (compile-time) | sempre |
| `_arity` | `Int` | estático | sempre |
| `_types` | `List::Text` | estático | sempre |
| `_return_type` | `Text` | estático | sempre |
| `_is_action` | `Bool` | estático | sempre |
| `_args` | tupla runtime | sintetizado dos params | Enter, Exit, ShortCircuit, Transform |
| `_return` | valor de retorno runtime | capturado do corpo | Exit, Transform |

**Estáticos** (`_name`, `_arity`, `_types`, `_return_type`, `_is_action`) são
resolvidos em compile-time por substituição direta de AST. O desugaring
conhece a função decorada (está processando o item) e produz literais
(`TextLit`, `IntLit`, etc.) no body inlined. Zero overhead de runtime — são
literais na AST.

**Dinâmicos** (`_args`, `_return`) são valores de runtime — o desugaring
sintetiza `_args` como `Expr::Tuple` dos parâmetros da função decorada e
captura `_return` do valor de retorno via `let __result := ...; let _return
:= __result`.

### 3.1. Polimorfismo via inlining

Como o body da diretiva é inlined (não chamado), cada site de aplicação é
type-checkado independentemente com os tipos concretos da função decorada.
Se o body faz `format("{}", _return)`, `_return` tem o tipo de retorno da
função decorada — funciona com qualquer tipo que tenha `Format`. Se faz
`_return + 1`, `_return` é `Int` (a função decorada deve retornar `Int`).

O polimorfismo é natural do inlining: a diretiva é tão polimórfica quanto o
uso de `_args`/`_return` permitir. Não há generics, não há type parameters
— o typeck resolve cada inlining com os tipos concretos. Se o body é
monomórfico (`_return + 1`), a diretiva só decora funções que retornam `Int`.
Se é polimórfico (`format("{}", _return)`), decora qualquer função cujo
retorno implementa `Format`.

### 3.2. Reserva de identificadores `_`

Identificadores começando com `_` são reservados para o compilador. O
usuário não pode declarar `let __result` nem `let _temp` nem `let _name`. O
`_` simples continua válido como hole (`+ 10 _`), wildcard em pattern
matching (`Result::Err(_)`), e predicados em tipos refinados (`> _ 0`) —
esses são símbolos sintáticos, não identificadores.

As variáveis de reflexão (`_name`, `_arity`, `_types`, `_return_type`,
`_is_action`, `_args`, `_return`) são disponibilizadas pelo desugaring no
body inlined. Fora do contexto de inlining, referenciar essas variáveis é
erro — elas não existem no escopo global nem em actions comuns.

O prefixo `__` (dois underscores) também é reservado para variáveis geradas
pelo desugaring (`__result`, `__decision`, `__body`). O prefixo `__hole_` já
é usado pelo desugaring de holes (`desugar_holes.rs`).

---

## 4. Os quatro modos de injeção

### 4.1. Enter

Injeta o body da diretiva **antes** do corpo da função decorada.

```kata
directive log{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, "enter: " + _name)
```

Desugaring de `@log` em `processar` (conceptual):

```kata
action processar(x :: Int) => Int
    let _name := "processar"
    let _arity := 1
    let _types := ["Int"]
    let _return_type := "Int"
    let _is_action := True
    let _args := (x,)
    log!(LogLevel::Info, "enter: " + _name)
    x + 1
```

`_args` está disponível e é sintetizado como tupla dos parâmetros: `(x,)`.

### 4.2. Exit

Injeta o body da diretiva **após** o corpo, capturando o resultado.
**Observacional** — o body da diretiva é statement (Unit) e não pode
modificar o resultado. Para modificar, use `Transform` (seção 4.4).

Precisa cobrir todos os pontos de saída: `return` explícito, retorno
implícito (última expr), e braços de `match`.

```kata
directive trace_exit{when: Hook::Exit, on: Target::Any}
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
```

Desugaring de `@trace_exit` em `processar` (retorno implícito):

```kata
action processar(x :: Int) => Int
    let __result := x + 1
    # <vars de reflexão: _name, _arity, _args, _return, ...>
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
    __result
```

Com `return` explícito:

```kata
action buscar(x :: Int) => Int
    match x
        Optional::Some(v): return v
        Optional::None: return 0
```

Desugaring:

```kata
action buscar(x :: Int) => Int
    match x
        Optional::Some(v):
            let __result := v
            # <vars de reflexão>
            log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
            return __result
        Optional::None:
            let __result := 0
            # <vars de reflexão>
            log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
            return __result
```

### 4.3. ShortCircuit

ShortCircuit é o modo de **decisão** — a diretiva decide se o corpo executa
ou não. O body da diretiva retorna `Optional::Some(valor)` para short-circuit
(pular o corpo, retornar `valor`) ou `Optional::None` para prosseguir (executar
o corpo normalmente).

**Restrição estrutural: `ShortCircuit` só decora actions (`Target::Action`).**

Esta restrição é estrutural, não pragmática. Funções puras têm a garantia de
que nenhuma diretiva customizada pode decidir se seu corpo executa. A
garantia vale por impossibilidade (o compilador rejeita `Hook::ShortCircuit`
com `Target::Function` ou `Target::Any`), não por convenção.

Em actions, short-circuit é explicitamente **não-transparente**: a diretiva
pode mudar o comportamento observável. O autor da diretiva assume essa
responsabilidade. `panic!` na diretiva aborta o processo, como em qualquer
action — não há isolamento de fiber nem regra de fallback.

```kata
directive auth_gate{when: Hook::ShortCircuit, on: Target::Action}
    match check_token(_args.0)
        Boolean::True: Optional::None          # prossegue
        Boolean::False: Optional::Some(deny())  # short-circuit
```

Desugaring de `@auth_gate` em `handler` (conceptual):

```kata
action handler(req :: Request) => Response
    let _name := "handler"
    let _arity := 1
    let _args := (req,)
    # <outras vars de reflexão>
    let __decision :=
        match check_token(_args.0)
            Boolean::True: Optional::None
            Boolean::False: Optional::Some(deny())
    match __decision
        Optional::Some(r): r          # short-circuit — retorna r
        Optional::None:               # prossegue
            let __result := process(req)
            __result
```

Se o body da diretiva tem múltiplos statements, o desugaring os envolve num
`Expr::Block` — o último statement (sem `;`) é o valor de `__decision`.

### 4.4. Transform

Transform é o modo de **transformação** — modifica o resultado após o corpo
executar. Diferente de Exit (observacional, body é Unit), Transform
**produz um valor** que substitui o resultado original. O último statement
do body inlined é o valor transformado.

**Restrição estrutural: `Transform` só decora actions (`Target::Action`).**
Mesma garantia de pureza que ShortCircuit.

```kata
directive redact_sensitive{when: Hook::Transform, on: Target::Action}
    sanitize(_return)
```

Desugaring de `@redact_sensitive` em `handler` (conceptual):

```kata
action handler(req :: Request) => Response
    let __result := process(req)
    let _name := "handler"
    let _arity := 1
    let _args := (req,)
    let _return := __result
    sanitize(_return)    # último statement — valor transformado
```

Com `return` explícito, o desugaring cobre cada ponto de saída:

```kata
action handler(req :: Request) => Response
    match check(req)
        Boolean::True:
            let __result := process(req)
            # <vars de reflexão: _return = __result, ...>
            let __transformed := sanitize(_return)
            return __transformed
        Boolean::False:
            let __result := deny(req)
            # <vars de reflexão>
            let __transformed := sanitize(_return)
            return __transformed
```

Se o body da diretiva tem múltiplos statements, o último statement (sem `;`)
é o valor transformado. O desugaring captura esse valor como `__transformed`.

### 4.5. Distinção Exit vs Transform

| Característica | Exit | Transform |
|---|---|---|
| Body produz | `Unit` (statement) | valor (última expr do body) |
| Pode modificar resultado | não | sim |
| Disponível em funções | sim (`Target::Any`/`Function`) | não (`Target::Action` only) |
| `_return` disponível | sim (observacional) | sim (transformacional) |

Exit é para **telemetria** — observar o resultado sem tocar nele. Transform
é para **instrumentação ativa** — sanitizar, envolver, adaptar o resultado.
A separação garante que o leitor sabe, pelo Hook, se a diretiva é
observacional ou transformacional.

---

## 5. Stacking de diretivas

Modelo cebola (onion), como Python decorators. Diretivas intrínsecas e
customizadas coexistem no mesmo stack — cada uma é aplicada na sua fase
(intrínsecas na codegen/lowering, customizadas no desugaring pré-typeck), mas
o modelo de composição é o mesmo:

```kata
@log
@cache
fib :: Int => Int
    lambda n: ...
```

Expande para:

```
trace.before           # customizada — inlinea body no prólogo
    cache.before       # intrínseca — cache lookup
        <body → __result>
    cache.after        # intrínseca — cache store
trace.after            # customizada — inlinea body no epílogo
```

Primeira diretiva = camada mais externa. `Enter` executa de cima para baixo,
`Exit` de baixo para cima. Com `ShortCircuit`, a diretiva mais externa envolve
a mais interna.

A interação entre intrínsecas e customizadas é bem-definida: o desugaring de
customizadas acontece antes do typeck, produzindo o código que as intrínsecas
então transformam. Cada camada é independente — não há ordem de aplicação
ambígua.

### 5.1. Propagação de short-circuit

Quando uma diretiva `ShortCircuit` short-circuita (retorna um valor sem
executar o corpo), a propagação segue o modelo middleware:

- **Tudo interno ao ShortCircuit é pulado** — o corpo não executa, nem
  diretivas Enter/Exit/Transform internas ao ShortCircuit.
- **Exit/Transform externo ao ShortCircuit dispara** com o valor
  short-circuitado.
- **Enter externo já disparou** (Enter é top-down, anterior ao ShortCircuit).

Exemplo:

```kata
@trace_exit                          # Exit — camada externa
@auth_gate                           # ShortCircuit — camada interna
action handler(req :: Request) => Response
    process(req)
```

Se `auth_gate` short-circuita (retorna `Optional::Some(deny)`):

1. `trace_exit` (externo) **dispara** com `_return = deny`.
2. `auth_gate` executou e decidiu short-circuit.
3. O corpo (`process(req)`) **não executa**.

Se `auth_gate` prossegue (retorna `Optional::None`):

1. O corpo executa normalmente.
2. `trace_exit` (externo) dispara com `_return = <valor do corpo>`.

`_return` em Exit e Transform é o **valor de retorno observável** — pode vir
do corpo ou de uma diretiva ShortCircuit interna que short-circuitou. O autor
da diretiva Exit/Transform não precisa distinguir os dois casos.

### 5.2. Interação ShortCircuit + Transform

Se ambos estão no stack, ShortCircuit é a camada que decide execução,
Transform é a camada que modifica o resultado. A ordem determina quem envolve
quem:

```kata
@sanitize                            # Transform — camada externa
@auth_gate                           # ShortCircuit — camada interna
action handler(req :: Request) => Response
    process(req)
```

Se `auth_gate` short-circuita:
1. `sanitize` (Transform externo) **não dispara** — está após o corpo, que
   não executou. O valor short-circuitado é o valor de retorno observável.
2. `auth_gate` decidiu short-circuit.
3. Corpo não executa.

Se `auth_gate` prossegue:
1. Corpo executa.
2. `sanitize` (Transform externo) dispara com `_return = <valor do corpo>`.
3. Resultado transformado é o valor de retorno.

Se a ordem for invertida (`@auth_gate` externo, `@sanitize` interno):
- ShortCircuit externo decide antes de tudo. Se short-circuit, nada interno
  executa (nem Transform).
- Se prossegue, Enter interno executa, corpo executa, Transform interno
  modifica o resultado, Exit interno observa.

---

## 6. Diretivas intrínsecas (não migradas)

As diretivas intrínsecas continuam chumbadas no compilador. Este PRD não
as modifica:

| Diretiva | Modelo | Por que continua chumbada |
|---|---|---|
| `@ffi` | anotação | informa linker de símbolo externo — semântica de compilação, não hook |
| `@builtin` | anotação | marca função para síntese de nó TAST — codegen, não hook |
| `@commutative` | anotação | habilita TRMA — transformação algébrica, não hook |
| `@associative` | anotação | idem |
| `@cache` | intercept | intercept transparente em função pura — customizadas não suportam intercept em funções por design |
| `@test` | anotação | tree shaking em produção — compilação condicional |
| `@log` | enter/exit | tem poder compile-time (template interpolation, policies de canal) que customizadas ainda não cobrem |

`@log` é a candidata natural a ser reexpressa como `directive log{...}` no
futuro. Hoje ela tem poder que o sistema de diretivas customizadas ainda não
cobre: interpolação de template (`{expr}` em `msg`), policies de canal
(`"drop"`/`"block"`), herança de config via `log_config!()`. A migração só
faz sentido quando o sistema de diretivas customizadas atingir esse nível.
A decisão de migrar ou não `@log` fica fora do escopo deste PRD.

---

## 7. Desugaring — implementação no pipeline

### 7.1. Onde encaixar no pipeline

O pipeline atual (visto em `kata-driver/src/main.rs:run_pipeline_with_file`):

```
lex → scan_lambdas → parse_decls_only → resolve → extract_arities
    → parse_with_arity → resolve → [desugar_directives] → infer_module
    → monomorphize → optimize → tree_shake → comptime → codegen
```

O desugaring de diretivas precisa acontecer **depois** da resolução de
módulos (para saber qual diretiva `@log` se refere, consultando o
`DirectiveRegistry`) mas **antes** do typeck (para validar o código
expandido). O encaixe é entre o segundo `resolve` e `infer_module`, no
driver.

O desugaring de pipes/holes existente (`desugar.rs`) opera em
`Spanned<Expr>` — uma única expressão. `desugar_directives` é uma **passada
separada** que opera em `Module` (nível de declaração), não uma terceira
fase de `desugar(expr)`:

```
desugar(expr) = desugar_pipes → desugar_holes       # existente, sem mudança
desugar_directives(module, registry) → module      # nova passada, separada
```

### 7.2. O que o inlining produz

O desugaring transforma a AST (`kata_ast::Module`) antes do typeck. Ele não
produz TAST — produz AST expandida que o typeck consome normalmente.

**Enter:** para cada `Item::ActionDecl` ou `Item::Sig` com `@log` (onde
`log` tem `when: Hook::Enter`), prependa ao body:
- `let` bindings das variáveis de reflexão (estáticas e `_args`)
- statements do body da diretiva (copiados, não chamados)

**Exit:** para cada ponto de saída no body, envolve o valor com:
- `let __result := <expr>`
- `let` bindings das variáveis de reflexão (estáticas, `_args`, `_return`)
- statements do body da diretiva
- `__result` (como valor de retorno)

**ShortCircuit:** insere no início do body:
- `let` bindings das variáveis de reflexão (estáticas e `_args`)
- `let __decision := <body da diretiva como Block>` — se múltiplos
  statements, envolve em `Expr::Block`; o último statement é o valor
- `match __decision { Optional::Some(r): r, Optional::None: <body original> }`

**Transform:** para cada ponto de saída, envolve o valor com:
- `let __result := <expr>`
- `let` bindings das variáveis de reflexão (estáticas, `_args`, `_return`)
- statements do body da diretiva — o último statement (sem `;`) é o valor
  transformado
- o valor transformado substitui o resultado original

### 7.3. Síntese das variáveis de reflexão

Ao inlinear o body da diretiva, o desugaring sintetiza as variáveis de
reflexão como `let` bindings no escopo do body inlined. Para as estáticas:

- `_name` → `let _name := "processar"` (`Expr::Let { name: "_name", value:
  TextLit("processar") }`)
- `_arity` → `let _arity := 1` (`IntLit("1")`)
- `_types` → `let _types := ["Int"]` (`ListLit [TextLit("Int")]`)
- `_return_type` → `let _return_type := "Int"` (`TextLit("Int")`)
- `_is_action` → `let _is_action := True` (`VariantQual("Boolean", "True")`)

O desugaring conhece a função decorada (está processando o item) e extrai
essas constantes da assinatura. Zero overhead — são literais na AST.

Para as dinâmicas:
- `_args` → `let _args := (x, ...)` (`Expr::Tuple { elements: [Ident("x"),
  ...] }`) sintetizado dos parâmetros da função decorada.
- `_return` → `let _return := __result` (`Expr::Ident { name: "__result" }`)
  referenciando a variável gerada pelo desugaring de Exit/Transform.

### 7.4. Cobertura de pontos de saída

O desugaring de Exit e Transform precisa cobrir todos os pontos de saída
do body. O body de `Item::ActionDecl` é `Vec<ActionStmt>` onde cada
`ActionStmt` tem `expr: Spanned<Expr>` e `has_semicolon: bool`. O último
statement sem `;` é o retorno implícito.

A investigação do codegen revelou:

- **`return` explícito** é `Expr::Return(Box<Spanned<Expr>>)` no AST. O
  codegen lowera com `jump` para `epilogue_block`
  (`control_flow.rs:22-25`). O desugaring precisa envolver cada `Return`
  com a injeção de Exit/Transform.
- **Retorno implícito** é o último `ActionStmt` sem `;`. O desugaring
  envolve essa expressão com `let __result := <expr>; <injeção>; __result`.
- **`match` com `return` em braços** — cada braço que faz `return` precisa
  ser envolvido individualmente. O desugaring percorre a AST recursivamente,
  encontrando todos os `Expr::Return` e pontos de saída implícitos.

O mecanismo é uma transformação de AST que caminha o body (`Vec<ActionStmt>`),
identifica pontos de saída (Return explícito e expressão final de cada
bloco), e envolve com a injeção. O codegen hoje já tem o padrão de
`epilogue_block` para centralizar o retorno — o desugaring produz código
que usa `return` normalmente, e o codegen continua centralizando no
`epilogue_block`.

Para `Item::Sig` (funções puras com body), o body é
`Option<Vec<Spanned<LambdaClause>>>`. Cada cláusula tem `body:
Spanned<Expr>`. O desugaring transforma cada cláusula envolvendo o body em
`Expr::Block { stmts: [let _name := ..., <injeção Enter>, <body original
envolvido com injeção Exit>] }`.

### 7.5. Validação de contrato

A validação de Target acontece em dois pontos:

1. **Na declaração da diretiva** — o compilador valida que `ShortCircuit`/
   `Transform` têm `on: Target::Action`. Se não, erro na declaração.
2. **Na aplicação da diretiva** — ao aplicar `@log` num item, o
   compilador verifica que o Target da diretiva é compatível com o tipo do
   item. `@log` com `on: Target::Action` aplicada numa função pura é erro.
   `@log` com `on: Target::Function` aplicada numa action é erro.

A validação na aplicação acontece no desugaring, antes do typeck. Se o
item é `Item::Sig` (função), o Target deve ser `Function` ou `Any`. Se é
`Item::ActionDecl` (action), o Target deve ser `Action` ou `Any`.

### 7.6. Importação de diretivas

Diretivas são declarações de top-level. São exportadas com `export log` e
importadas com `import mod.trace` — mesma sintaxe de import/export de
actions e funções. O `ModuleLoader` carrega diretivas importadas e as
adiciona ao `DirectiveRegistry` do módulo importador. A resolução de
`@nome` no desugaring consulta o `DirectiveRegistry` (que inclui diretivas
importadas). Sem mecanismo extra.

O `merge_two` (que combina prelude + módulo do usuário) precisa mesclar
`DirectiveRegistry` preservando overloads por `(when, on)` — diferente de
`actions` (onde nomes duplicados se substituem), diretivas com mesmo nome
coexistem quando diferem por `(when, on)`.

---

## 8. Escape hatch para funções puras

Se for necessário interceptar uma função pura (short-circuit ou transform),
o caminho é embrulhar a função numa action que apenas a chama e aplicar a
diretiva na action. O custo é visível no ponto de uso (uma action extra),
não espalhado no type system.

```kata
# Função pura que precisa de interceptação
fazer_algo :: Int => Int
    lambda x: ...

# Wrapper action para aplicar ShortCircuit
@auth_gate
action fazer_algo_safe(x :: Int) => Int
    fazer_algo(x)
```

Isto cobre casos raros como profiling — que tipicamente é instrumentação de
plataforma, não diretiva de linguagem.

---

## 9. Pipeline — componentes afetados

```
# AST — novo Item para declaração de diretiva
crates/kata-ast/src/item.rs
    # Item::DirectiveDecl { name, args: Vec<DirectiveArg>, body: Vec<ActionStmt> }
    # Novo struct DirectiveDef para o DirectiveRegistry

# Lexer — novo keyword `directive`
crates/kata-lexer/src/dispatch.rs           # adicionar `directive` como keyword

# Parser — parse de directive name{when: ..., on: ...}
crates/kata-parser/src/declarations.rs      # parse_directive_decl (novo)
                                            # reusa parse_directive_args para o dict {}

# Resolution — DirectiveRegistry + validação de @nome
crates/kata-resolution/src/lib.rs           # dois passes: coletar diretivas, validar @nome
crates/kata-resolution/src/directives.rs    # extract_directive_spec (novo)
crates/kata-resolution/src/types.rs         # DirectiveDef, DirectiveRegistry (novos)

# Desugar — desugar_directives (nova passada)
crates/kata-driver/src/main.rs              # chamar desugar_directives entre resolve e infer
crates/kata-inference/src/desugar_directives.rs  # (novo arquivo) inlining de bodies

# Inference — sem mudança (typeck consome AST expandida)
# Codegen — sem mudança (desugaring produz código que usa nós existentes)

# Prelude — enums Hook e Target (já existentes, sem mudança)
stdlib/core.kata                           # enum Hook { Enter, Exit, ShortCircuit, Transform }
                                           # enum Target { Action, Function, Any }

# Testes
crates/kata-driver/tests/                  # E2E: Enter, Exit, ShortCircuit, Transform, stacking
```

---

## 10. Fora do escopo

- **Migração de `@log`** — `@log` continua intrínseca. A migração para
  `directive log{...}` é uma decisão separada que depende de o sistema de
  diretivas customizadas atingir paridade com o poder compile-time de
  `@log`.
- **Migração de `@cache`** — `@cache` é intercept transparente em função
  pura. Customizadas não suportam intercept em funções por design. `@cache`
  nunca migra.
- **Diretivas em `implements`** — `directive` só se aplica a actions e
  funções nomeadas. Não decora métodos de `implements`.
- **Diretivas parametrizadas** — o dict da diretiva aceita apenas `when` e
  `on`. Parâmetros adicionais (ex: `directive log{when: ..., on: ...,
  level: "Info"}`) ficam para um PRD futuro.
- **Actions genéricas** — se o polimorfismo via inlining não é suficiente
  para casos extremos, actions genéricas poderiam resolver. Verificar se
  Kata5 suporta actions genéricas é pré-requisito, mas a implementação de
  actions genéricas (se necessária) é um PRD separado. O inlining já
  resolve a maioria dos casos sem generics.
- **`@directive` em funções puras com `Target::Function`** — suportado pela
  sintaxe, mas o valor real é limitado sem Intercept/Transform. Enter e
  Exit em funções puras são puramente observacionais.

---

## 11. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `directive` é um constructo novo, não action anotada | Diretivas têm semântica diferente de actions: são templates inlined, não funções chamáveis. O constructo comunica a diferença sintaticamente. |
| D2 | `ShortCircuit`/`Transform` exigem `Target::Action` | Garantia de pureza das funções por impossibilidade estrutural, não por convenção. O leitor não precisa auditar diretivas. |
| D3 | `Exit` é observacional (body é Unit), `Transform` é transformacional (body produz valor) | O Hook comunica se a diretiva é observacional ou ativa. O leitor sabe pelo nome do Hook sem ler o corpo. |
| D4 | Inlining, não chamada | O body da diretiva é copiado para dentro da função decorada. Variáveis de reflexão são `let` bindings no escopo inlined. Resolve polimorfismo de `_args`/`_return` sem generics — cada site type-checka com tipos concretos. |
| D5 | Diretivas não estão no `dispatch_table` | Diretivas não são chamáveis diretamente. `trace!()` não resolve. Erro claro: "`log` é diretiva, não é chamável". |
| D6 | `desugar_directives` é passada separada no driver, não fase de `desugar(expr)` | `desugar(expr)` opera em `Spanned<Expr>`; `desugar_directives` opera em `Module` (nível de declaração). Precisa de `DirectiveRegistry` (do resolution) que não está disponível em `desugar(expr)`. |
| D7 | Overloading por `(when, on)` — nome agrupa, Hook/Target distingue | Cada combinação é uma declaração distinta com seu próprio body. Type-checking limpo no inlining, separação de responsabilidades, especialização por Target. |
| D8 | `Target::Any` não coexiste com específico para o mesmo `when` | Elimina ambiguidade de resolução na declaração. Any existe para evitar duplicação quando o comportamento é idêntico para actions e funções. |
| D9 | Diretivas intrínsecas não migram | `@ffi`, `@builtin`, `@commutative`, `@associative`, `@cache`, `@test` têm semântica de compilação/codegen, não de hook. `@log` migra quando (e se) customizadas atingirem paridade. |
| D10 | Identificadores `_` são reservados | Variáveis de reflexão e variáveis geradas (`__result`, `__decision`) nunca colidem com código de usuário. `_` simples continua válido como hole e wildcard. |
| D11 | Escape hatch via wrapper action | Interceptação de função pura é explicitamente visível (uma action extra), não espalhada no type system. Cobre casos raros sem complicar o design. |
| D12 | `directive` e `action` têm namespaces disjuntos | Diretivas não são actions. `directive log` e `action trace` no mesmo escopo é erro. Evita ambiguidade na resolução de `@log` vs `trace!()`. |

---

## 12. DoDs (Definitions of Done)

### Fase 1 — Infraestrutura ✅

1. ✅ `enum Hook` com 4 variantes (`Enter`, `Exit`, `ShortCircuit`, `Transform`)
   no prelude.
2. ✅ `enum Target` com 3 variantes (`Action`, `Function`, `Any`) no prelude.
3. ✅ Lexer tokeniza `directive` como keyword.
4. ✅ Parser aceita `directive nome{when: Hook::Enter, on: Target::Action}` e
   produz `Item::DirectiveDecl { name, args, body }`.
5. ✅ `DirectiveRegistry` é populado durante o resolution processando
   `Item::DirectiveDecl`. Extrai `when`/`on` dos `args` (como
   `extract_test_specs` extrai `desc`/`args`).
6. ✅ Resolution valida:
   - `ShortCircuit`/`Transform` com `Target::Function` ou `Target::Any` →
     erro.
   - `Target::Any` coexistindo com específico para o mesmo `(nome, when)` →
     erro.
   - `directive` e `action` com mesmo nome no mesmo escopo → erro.
7. ✅ Duas declarations com mesmo nome e mesmo `(when, on)` → erro de conflito.
8. ✅ Dois passes no resolution: primeiro coletar `Item::DirectiveDecl`, depois
   validar `@nome` em `Item::ActionDecl` e `Item::Sig` contra o
   `DirectiveRegistry`.

### Fase 2 — Desugaring (Inlining) ✅

9. `desugar_directives` processa `@nome` em `Item::ActionDecl` e `Item::Sig`,
   resolve `nome` no `DirectiveRegistry`, e inlinea o body conforme o Hook.
10. **Enter** inlinea o body da diretiva (com `let` bindings de reflexão)
    antes do body original.
11. **Exit** envolve todos os pontos de saída (return explícito, retorno
    implícito, braços de match) com `let __result := ...; <body inlined>;
    __result`.
12. **ShortCircuit** inlinea o body da diretiva como valor de `__decision`,
    envolve o body original em `match __decision { Some: r, None: <body> }`.
13. **Transform** envolve pontos de saída com `let __result := ...; <body
    inlined (último statement = valor transformado)>`.
14. Variáveis de reflexão estáticas (`_name`, `_arity`, `_types`,
    `_return_type`, `_is_action`) são sintetizadas como `let` bindings com
    literais na AST.
15. Variáveis de reflexão dinâmicas (`_args`, `_return`) são sintetizadas
    como `Expr::Tuple` dos params e `let _return := __result`.
16. Validação de Target na aplicação: `@log` com `on: Target::Action`
    aplicada em função pura → erro.

### Fase 3 — Stacking ✅

17. Múltiplas diretivas customizadas empilhadas no mesmo item são inlineadas
    em ordem (primeira = mais externa).
18. Diretiva intrínseca + customizada no mesmo item coexistem sem conflito.
19. Short-circuit propaga corretamente: Exit/Transform externo dispara com
    valor short-circuitado, corpo interno é pulado.

### Fase 4 — Overloading ✅

20. Múltiplas declarations com mesmo nome e `(when, on)` diferentes coexistem.
21. `@log` aplicado em action inlinea apenas as declarações com
    `on: Action` ou `on: Any`.
22. `@log` aplicado em função pura inlinea apenas as declarações com
    `on: Function` ou `on: Any`.

### Fase 5 — Importação ✅

23. `export log` exporta a diretiva. `import mod.trace` torna `@log`
    disponível no módulo importador.
24. Diretiva importada resolve corretamente no desugaring.
25. `merge_two` mescla `DirectiveRegistry` preservando overloads por
    `(when, on)`.

### Fase 6 — Integração ✅

26. `cargo test --workspace --no-fail-fast` passa sem regressão.
27. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

---

## 13. Riscos

| Risco | Mitigação |
|---|---|
| Cobertura de pontos de saída incompleta (match aninhado, loop com return) | Desugaring recursivo que caminha toda a AST. Testes E2E com casos complexos. |
| Inlining de body com múltiplos statements em contextos de expressão (ShortCircuit) | Envolver em `Expr::Block` — o último statement é o valor. Testar com bodies de 1 e N statements. |
| Desugaring precisa de `DirectiveRegistry` mas `desugar.rs` hoje só recebe `Spanned<Expr>` | `desugar_directives` opera em `Module` (nível de item), não em `Expr`. É uma passada separada no driver, não parte de `desugar(expr)`. |
| Resolução de `@nome` ambígua quando múltiplos módulos exportam diretivas com mesmo nome | Mesma regra de resolução de actions: sombreamento lexical. O `@nome` mais próximo no escopo vence. |
| Interação com `@log` intrínseca no mesmo item | Desugar customizadas primeiro (pré-typeck), `@log` age depois (codegen). Cada um em sua fase — sem conflito. |
| `merge_two` quebra overloading de diretivas | `DirectiveRegistry` tem lógica de merge separada de `actions` — preserva overloads por `(when, on)`. |

---

## 14. Exemplos

### 14.1. Tracing de entrada e saída

```kata
directive trace_enter{when: Hook::Enter, on: Target::Any}
    log!(LogLevel::Info, "enter: " + _name)

directive trace_exit{when: Hook::Exit, on: Target::Any}
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))

@trace_enter
@trace_exit
action processar(x :: Int) => Int
    x + 1
```

### 14.2. Auth gate com short-circuit

```kata
directive auth_gate{when: Hook::ShortCircuit, on: Target::Action}
    match check_token(_args.0)
        Boolean::True: Optional::None
        Boolean::False: Optional::Some(deny())

@auth_gate
action handler(req :: Request) => Response
    process(req)
```

### 14.3. Sanitização com transform

```kata
directive redact{when: Hook::Transform, on: Target::Action}
    sanitize_response(_return)

@redact
action handler(req :: Request) => Response
    process(req)
```

### 14.4. Stacking completo

```kata
@trace_exit                          # Exit — externo
@redact                              # Transform — meio
@auth_gate                           # ShortCircuit — interno
action handler(req :: Request) => Response
    process(req)
```

Expande para (conceptual):

```kata
action handler(req :: Request) => Response
    # ── inlined: auth_gate (ShortCircuit) ──
    let _name := "handler"
    let _args := (req,)
    let __decision :=
        match check_token(_args.0)
            Boolean::True: Optional::None
            Boolean::False: Optional::Some(deny())
    match __decision
        Optional::Some(r):
            # ── inlined: trace_exit (Exit) com _return = r ──
            let _return := r
            log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
            r
        Optional::None:
            # ── body original ──
            let __result := process(req)
            # ── inlined: redact (Transform) ──
            let _return := __result
            let __transformed := sanitize_response(_return)
            # ── inlined: trace_exit (Exit) com _return = __transformed ──
            let _return := __transformed
            log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
            __transformed
```

### 14.5. Diretiva importada

```kata
# mod tracing.kata
export trace_enter
export trace_exit

directive trace_enter{when: Hook::Enter, on: Target::Any}
    log!(LogLevel::Info, "enter: " + _name)

directive trace_exit{when: Hook::Exit, on: Target::Any}
    log!(LogLevel::Info, "exit: " + _name)
```

```kata
# main.kata
import tracing.(trace_enter, trace_exit)

@trace_enter
@trace_exit
action processar(x :: Int) => Int
    x + 1
```

---

## 15. Próximos passos

1. **Implementar Fase 1** (keyword `directive` + parser + `DirectiveRegistry`
   + validação) — validar que `directive nome{...}` é parseada, resolvida e
   validada corretamente.
2. **Implementar Fase 2** (desugaring com inlining) — validar Enter, Exit,
   ShortCircuit, Transform com testes E2E.
3. **Implementar Fase 3-5** (stacking, overloading, importação).
4. **Integrar com Fase 6** (testes + clippy).