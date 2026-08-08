# Diretivas Kata — Visão de Design

**Status:** 🧭 Exploração
**Data:** 2026-08-04
**Pré-requisito:** PRD de Reflexão de Funções (`docs/PRD-fn-reflection.md`) — não implementado
**Não é um PRD:** Este documento captura a ideia atual e as arestas em aberto.
Muitos pontos não podem ser definidos até que a reflexão de funções exista
no compilador e possamos validar as hipóteses contra código real.

---

## 1. Premissa central

Qualquer action pode ser diretiva. Não há palavra reservada `directive`;
não há constructo sintático novo. O usuário escreve uma action comum e a
anota com `@directive` (a meta-diretiva que a promove a hook). Depois aplica
`@nome_da_action` em outras actions/funções para injetar a chamada.

```kata
@directive{when: Hook::Enter, on: Target::Action}
action trace() => Unit
    log!(LogLevel::Info, "enter: " + _name)
```

Uso:

```kata
@trace
action processar(x :: Int) => Int
    x + 1
```

Desugaring (no nível do AST, antes do typeck):

```kata
action processar(x :: Int) => Int
    trace!()
    x + 1
```

No corpo de `trace`, a variável de reflexão `_name` resolve para
`TextLit("processar")` em compile-time — o caso estático do PRD de reflexão.
O compilador sintetiza as variáveis de reflexão (`_name`, `_arity`, etc.)
no escopo da diretiva no momento da injeção. Zero overhead de runtime para
bindings estáticos. A sidecar table e o lookup dinâmico do PRD de reflexão
não se aplicam aqui porque o compilador sempre sabe qual função está
decorada.

---

## 2. A meta-diretiva `@directive`

`@directive` é a única diretiva intrínseca nova. Ela promove uma action ao
status de hook e configura como o compilador a injeta. As diretivas
intrínsecas existentes (`@ffi`, `@builtin`, `@commutative`, `@associative`,
`@cache`, `@test`) continuam chumbadas no compilador — não há plano de
migrá-las.

### 2.1. Enums de configuração

```kata
enum Hook
    Enter       # injeta no prólogo (antes do corpo)
    Exit        # injeta no epílogo (após o corpo, antes do retorno)
    Intercept   # intercepta: short-circuit ou transforma (só Target::Action)

enum Target
    Action      # só decora actions
    Function    # só decora funções puras
    Any         # decora ambos

# Intercept exige Target::Action. O compilador rejeita
# Hook::Intercept com Target::Function ou Target::Any.
```

### 2.2. Campos de `@directive`

| Campo | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `when` | `Hook` | **sim** | Ponto de injeção |
| `on` | `Target` | **sim** | Que tipo de item pode decorar |

`@directive` não tem campo de configuração de argumentos. Em vez disso, o
compilador disponibiliza **variáveis de reflexão** (`_name`, `_arity`,
`_return`, etc.) no escopo do corpo da action anotada com `@directive`.
Essas variáveis são sintetizadas no momento da injeção e referenciam a
entidade decorada (ver seção 3).

### 2.3. Múltiplas actions por diretiva (overloading por Hook e Target)

> **Nota de design (2026-08-07).** Em vez de uma action única com `@directive`
> que cobre todos os modos, uma diretiva pode ser formada por **múltiplas
> actions com o mesmo nome**, cada uma anotada com `when` (Hook) e/ou `on`
> (Target) diferentes. O nome agrupa; `when` e `on` distinguem. Ao aplicar
> `@trace` num item, o compilador injeta **todas** as definições aplicáveis
> ao Hook e ao Target daquele item — simultaneamente.
>
> ```kata
> @directive{when: Hook::Enter, on: Target::Action}
> action trace() => Unit
>     log!(LogLevel::Info, "enter action: " + _name)
>
> @directive{when: Hook::Enter, on: Target::Function}
> action trace() => Unit
>     log!(LogLevel::Info, "enter function: " + _name)
>
> @directive{when: Hook::Exit, on: Target::Any}
> action trace() => Unit
>     log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
> ```
>
> Uso — uma anotação dispara todos os hooks cujo `when` e `on` casam:
>
> ```kata
> @trace
> action processar(x :: Int) => Int
>     x + 1
> ```
>
> Desugaring (Target::Action casa com Enter::Action e Exit::Any):
>
> ```kata
> action processar(x :: Int) => Int
>     trace!()                           # Enter on Action — _name = "processar"
>     let __result := x + 1
>     trace!()                           # Exit on Any — _name = "processar", _return = __result
>     __result
> ```
>
> Se `processar` fosse função pura, casaria com Enter::Function e
> Exit::Any — action e function disparam hooks diferentes para o mesmo
> nome de diretiva.
>
> O **nome da diretiva é o nome da action** — não há campo de agrupamento
> nem constructo sintático de bloco (`directive trace { ... }` foi
> descartado: a premissa central do documento é não introduzir constructo
> sintático novo). Múltiplas actions com o mesmo nome coexistem quando
> diferem por `when` e/ou `on` em `@directive` — overloading por Hook e
> Target.
>
> Vantagens:
>
> - **Type-checking limpo:** cada action tem sua própria assinatura, que
>   corresponde exatamente ao seu call site. O Exit que usa `_return` tem
>   contexto diferente do Enter que usa só `_name` — e cada um type-checka
>   independentemente. Resolve o problema de dual-call no Intercept
>   enter+exit (seção 4.3).
> - **Separação de responsabilidades:** cada combinação de Hook e Target
>   é uma action distinta, mais simples e auditável.
> - **Composicionalidade natural:** o usuário declara só os Hooks que
>   precisa. Uma diretiva que só faz tracing de entrada declara só a
>   action de Enter. Não há Exit injetado se nenhuma action de Exit existe
>   para aquele nome.
> - **Especialização por Target:** actions e funções podem ter
>   comportamentos diferentes para a mesma diretiva — útil quando a
>   semântica de instrumentação difere entre os dois.
>
> Regras de validação:
>
> - Duas actions com o mesmo nome e o **mesmo** par `(when, on)` é
>   conflito — erro.
> - O compilador precisa permitir múltiplas definições de action com o
>   mesmo nome no mesmo escopo quando diferem pelo par `(when, on)` de
>   `@directive`. Isso é overloading por Hook e Target, análogo a
>   overloading por tipo em outras linguagens, mas discriminado pelos
>   campos `when`/`on` em compile-time.
> - `on` pode diferir entre as actions da mesma diretiva — cada
>   Hook/Target tem necessidades diferentes (Enter não tem `_return`, Exit
>   tem; Intercept exige `Target::Action`).
> - `Target::Any` só pode coexistir com outras definições da mesma
>   diretiva se for a única definição para aquele `when` — ou seja, para
>   um dado `(nome, when)`, ou você tem `on: Any` ou `on: Action`/`on:
>   Function`, mas não mistura `Any` com específico. O compilador rejeita
>   a mistura na declaração, eliminando ambiguidade de resolução. Any
>   existe para o caso comum onde o comportamento é idêntico para
>   actions e funções (tracing, logging) e duplicar a definição seria
>   pura burocracia.

### 2.4. Resolução

Quando o compilador encontra `@nome` num item:

1. É intrínseca conhecida (`@ffi`, `@cache`, etc.)? → trata como anotação
   hardcoded (comportamento existente).
2. Senão, resolve `nome` no escopo de módulos → é action? → tem
   `@directive`? → lê os campos e injeta a chamada conforme declarado.
3. Senão → erro: `nome não é uma diretiva válida`.

---

## 3. Variáveis de reflexão em diretivas

Actions anotadas com `@directive` têm acesso a **variáveis de reflexão** no
corpo — bindings prefixados com `_` que o compilador sintetiza no momento
da injeção, referenciando a entidade decorada. Identificadores começando
com `_` são reservados para o compilador (ver A4b), de modo que essas
variáveis nunca colidem com código de usuário.

| Variável | Tipo | Origem | Disponível em |
|---|---|---|---|
| `_name` | `Text` | estático (compile-time) | sempre |
| `_arity` | `Int` | estático | sempre |
| `_types` | `List[Text]` | estático | sempre |
| `_return_type` | `Text` | estático | sempre |
| `_is_action` | `Bool` | estático | sempre |
| `_args` | tupla runtime | sintetizado dos params | Enter, Exit, Intercept |
| `_return` | valor de retorno runtime | capturado do corpo | Exit, Intercept |

**Estáticos** (`_name`, `_arity`, `_types`, `_return_type`, `_is_action`) são
resolvidos em compile-time — o compilador conhece a função decorada e extrai
as constantes da assinatura. Zero overhead.

**Dinâmicos** (`_args`, `_return`) são valores de runtime — o compilador
sintetiza `_args` a partir dos parâmetros e captura `_return` do valor de
retorno.

`_return` representa o **valor de retorno observável** da função, não
necessariamente o valor que o corpo produziu. Se uma diretiva `Intercept`
interna short-circuita, Exit externo recebe o valor short-circuitado —
não o valor do corpo (que não executou). Ver seção 5 para a semântica
de propagação em stacking.

---

## 4. Os três modos de injeção

### 4.1. Enter

Injeta a chamada da diretiva **antes** do corpo da função decorada.

```kata
@directive{when: Hook::Enter, on: Target::Action}
action trace() => Unit
    log!(LogLevel::Info, "enter: " + _name)
```

Desugaring de `@trace` em `processar`:

```kata
action processar(x :: Int) => Int
    trace!()
    x + 1
```

### 4.2. Exit

Injeta a chamada **após** o corpo, capturando o resultado. Precisa cobrir
todos os pontos de saída: `return` explícito, retorno implícito (última
expr), e braços de `match`.

```kata
@directive{when: Hook::Exit, on: Target::Any}
action trace_exit() => Unit
    log!(LogLevel::Info, "exit: " + _name + " => " + format("{}", _return))
```

Desugaring de `@trace_exit` em `processar`:

```kata
action processar(x :: Int) => Int
    let __result := x + 1
    trace_exit!()
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
            trace_exit!()
            return __result
        Optional::None:
            let __result := 0
            trace_exit!()
            return __result
```

### 4.3. Intercept

Intercept é o modo de **interceptação** — short-circuit (pular o corpo) ou
transformação (modificar o resultado). Não há modo observacional de
intercept: observar antes e depois sem interceptar é exatamente Enter +
Exit, sem ganho adicional. Intercept existe para quando a diretiva precisa
decidir se o corpo executa ou modificar o que ele retorna.

**Restrição estrutural: `Intercept` só decora actions (`Target::Action`).**

Esta restrição não é pragmática — é estrutural. Funções puras têm a
garantia de que nenhuma diretiva customizada pode interceptar sua
execução. A garantia vale por impossibilidade (o compilador rejeita
`Hook::Intercept` com `Target::Function` ou `Target::Any`), não por
convenção. O leitor de código Kata5 nunca precisa auditar uma diretiva
de interceptação para saber se uma função pura é segura — a
impossibilidade é verificada uma vez, na declaração da diretiva.

Em actions, intercept é explicitamente **não-transparente**: a diretiva
pode mudar o comportamento observável (short-circuit, substituição de
resultado, negação de acesso). O autor da diretiva assume essa
responsabilidade. `panic!` na diretiva aborta o processo, como em
qualquer action — não há isolamento de fiber nem regra de fallback.

**Por que `@cache` fica intrínseca:** `@cache` é intercept transparente
em função pura — short-circuit onde o valor cacheado é idêntico ao que
a função produziria. É exatamente a categoria que customizadas não
suportam, por design. Se intercept customizada fosse permitida em
funções, ou exigiria um campo `transparent` que o leitor precisa checar
(garantia por convenção, sujeita a degradação), ou quebraria a garantia
de pureza das funções globalmente. A restrição a `Target::Action`
preserva a garantia por impossibilidade estrutural.

**Escape hatch para funções:** se for necessário interceptar uma função
pura, o caminho é embrulhar a função numa action que apenas a chama e
aplicar a diretiva na action. O custo é visível no ponto de uso (uma
action extra), não espalhado no type system. Isto cobre casos raros
como profiling — que tipicamente é instrumentação de plataforma, não
diretiva de linguagem.

**Protocolo de short-circuit:**

```kata
@directive{when: Hook::Intercept, on: Target::Action}
action auth_intercept() => Optional::Response
    # _args disponível, tipado pelos parâmetros da função decorada
    ...
```

Desugaring de `@auth_intercept` em `handler`:

```kata
action handler(req :: Request) => Response
    let __decision := auth_intercept!()    # Intercept — _args = req
    match __decision
        Optional::Some(r): r          # short-circuit — retorna r
        Optional::None:               # prossegue
            let __result := process(req)
            __result
```

A diretiva retorna `Optional::Some(value)` para short-circuit ou
`Optional::None` para prosseguir. Protocolo simples, sem continuation,
sem lambda — cabe no sistema de tipos atual de Kata5.

**Transformação de resultado (enter+exit):** se o corpo da diretiva
Intercept referencia `_return`, o compilador gera também um ponto de
transformação após o corpo. O desugaring passa a ter dois pontos de
interceptação:

```kata
action handler(req :: Request) => Response
    let __decision := auth_intercept!()    # Intercept enter — _args = req
    match __decision
        Optional::Some(r): r
        Optional::None:
            let __body := process(req)
            auth_intercept!()              # Intercept exit — _return = __body
            __body                         # (ou valor transformado pela diretiva)
```

Se o corpo referencia `_return`, a diretiva é enter+exit (short-circuit +
transform). Se não, é enter-only (short-circuit puro). A distinção é
inferida do uso no corpo — não há campo declarativo.

Desugaring conceitual de `@cache` (intrínseca, não migrada — só para
ilustrar o modelo intercept transparente que customizadas não suportam):

```kata
# diretiva cache (intrínseca) em função pura
@cache
fib :: Int => Int
    lambda n: ...
```

Expansão conceitual:

```kata
fib :: Int => Int
    lambda n:
        let __cached := cache_lookup(n)
        match __cached
            Optional::Some(v): v          # hit — mesmo valor
            Optional::None:               # miss
                let __result := <corpo original>
                cache_store(n, __result)
                __result
```

---

## 5. Stacking de diretivas

Modelo cebola (onion), como Python decorators. Diretivas intrínsecas e
customizadas coexistem no mesmo stack — cada uma é aplicada na sua fase
(intrínsecas na codegen/lowering, customizadas no desugaring pré-typeck),
mas o modelo de composição é o mesmo:

```kata
@trace
@cache
fib :: Int => Int
    lambda n: ...
```

Expande para:

```
trace.before           # customizada — injeta chamada no prólogo
    cache.before       # intrínseca — cache lookup
        <body → __result>
    cache.after        # intrínseca — cache store
trace.after            # customizada — injeta chamada no epílogo
```

Primeira diretiva = camada mais externa. `Enter` executa de cima para
baixo, `Exit` de baixo para cima. Com `Intercept`, a diretiva mais
externa envolve a mais interna.

A interação entre intrínsecas e customizadas é bem-definida: o
desugaring de customizadas acontece antes do typeck, produzindo o
código que as intrínsecas então transformam. Cada camada é
independente — não há ordem de aplicação ambígua.

### 5.1. Propagação de short-circuit

Quando uma diretiva `Intercept` short-circuita (retorna um valor sem
executar o corpo), a propagação segue o modelo middleware:

- **Tudo interno ao Intercept é pulado** — o corpo não executa, nem
  diretivas Enter/Exit internas ao Intercept.
- **Exit externo ao Intercept dispara** com o valor short-circuitado.
- **Enter externo já disparou** (Enter é top-down, anterior ao Intercept).

Exemplo:

```kata
@trace_exit                          # Exit — camada externa
@auth_intercept                      # Intercept — camada interna
action handler(req :: Request) => Response
    process(req)
```

Se `auth_intercept` short-circuita (retorna `Optional::Some(deny)`):

1. `trace_exit` (externo) **dispara** com `_return = deny`.
2. `auth_intercept` executou e decidiu short-circuit.
3. O corpo (`process(req)`) **não executa**.

Se `auth_intercept` prossegue (retorna `Optional::None`):

1. O corpo executa normalmente.
2. `trace_exit` (externo) dispara com `_return = <valor do corpo>`.

Isto significa que `_return` em Exit é o **valor de retorno observável**
— pode vir do corpo ou de uma diretiva Intercept interna que
short-circuitou. O autor da diretiva Exit não precisa distinguir os
dois casos.

---

## 6. Diretivas existentes (não migradas)

As diretivas intrínsecas continuam chumbadas no compilador:

| Diretiva | Modelo | Por que continua chumbada |
|---|---|---|
| `@ffi` | anotação | informa linker de símbolo externo — semântica de compilação, não hook |
| `@builtin` | anotação | marca função para síntese de nó TAST — codegen, não hook |
| `@commutative` | anotação | habilita TRMA — transformação algébrica, não hook |
| `@associative` | anotação | idem |
| `@cache` | intercept | intercept transparente em função pura — customizadas não suportam intercept em funções por design (ver 4.3) |
| `@test` | anotação | tree shaking em produção — compilação condicional |
| `@log` | enter/exit | **candidata a migração** — mas já implementada com poder compile-time (template interpolation, policies de canal) |

`@log` é a candidata natural a ser reexpressa como action com `@directive`
no futuro. Hoje ela tem poder que o sistema de diretivas customizadas
ainda não cobre: interpolação de template (`{expr}` em `msg`), policies de
canal (`"drop"`/`"block"`), herança de config via `log_config!()`. A
migração só faz sentido quando o sistema de diretivas customizadas
atingir esse nível.

---

## 7. O que a reflexão de funções habilita

O PRD de reflexão (`docs/PRD-fn-reflection.md`) é pré-requisito porque as
diretivas dependem de `_name`, `_arity`, etc. em compile-time.

O que o PRD de reflexão fornece:

- `_name` → `TextLit` constante (caso estático)
- `_arity` → `IntLit` constante
- `_types` → `List` literal de `TextLit`
- `_return_type` → `TextLit` constante
- `_is_action` → `Boolean::True`/`False`

Tudo resolvido em compile-time — o compilador sempre sabe qual função está
decorada e sintetiza os bindings `_` no escopo da diretiva. A sidecar table
e o lookup dinâmico do PRD de reflexão **não se aplicam** aqui.

Sem a reflexão implementada, não há como o compilador sintetizar
`TextLit("processar")` a partir de `_name` — a infraestrutura não existe.

---

## 8. Arestas em aberto

Estas são as questões que não podemos definir agora, seja porque dependem
da implementação da reflexão, seja porque precisamos validar contra o
compilador real.

### A1. `_args`/`_return` e polimorfismo

Sem `pass`, o tipo de `_args` e `_return` não é declarado na assinatura da
diretiva — é inferido do uso no corpo. Se o corpo faz `format("{}", _return)`,
`_return` é polimórfico (funciona com qualquer tipo que tenha `Format`). Se
faz `_return + 1`, é monomórfico em `Int`. A inferência bidirecional do Kata5
faz esse trabalho.

O limite é do corpo, não da declaração: a diretiva é tão polimórfica quanto
o uso de `_args`/`_return` permitir. Se o corpo é monomórfico, a diretiva só
decora funções com o tipo compatível. Falta verificar se Kata5 suporta
actions genéricas para casos onde o polimorfismo do corpo não é suficiente.

### A2. `_return` e o tipo de retorno

Mesma questão de A1, aplicada ao valor de retorno. Sem `pass`, `_return`
não tem tipo declarado — é inferido do uso no corpo. Para `@log` isso não
é problema porque `format("{}", _return)` é polimórfico. Para diretivas
customizadas com intercept que precisam do valor tipado, a diretiva é
monomórfica por tipo de retorno — limitada pelo uso no corpo.

### A3. Intercept e mecanismo de fallback

**Resolvido pelo design atual.** Intercept é `Target::Action` only e
explicitamente não-transparente. `panic!` na diretiva aborta o processo,
como em qualquer action — não há isolamento de fiber nem regra de
fallback. A regra de "erro não impede retorno" foi removida: intercept
pode mudar o comportamento observável por design, e o autor da diretiva
assume a responsabilidade.

### A4. Desugaring no AST vs. fase de resolução

O desugaring precisa acontecer **depois** da resolução de módulos (para
saber qual diretiva `@trace` se refere) mas **antes** do typeck (para
validar o código expandido). Isso significa:

- Uma nova passada entre resolution e typeck, ou
- Um hook no início do typeck.

Precisa verificar a arquitetura atual do pipeline para decidir onde
encaixar.

### A4b. Nomes gerados e colisão com identificadores de usuário

O desugaring gera variáveis internas (`__result`, `__decision`,
`__cached`) e as variáveis de reflexão (`_name`, `_arity`, `_return`,
etc.). Se o usuário puder declarar variáveis com esses nomes, há colisão.

**Decisão:** identificadores começando com `_` são reservados para o
compilador. O usuário não pode declarar `let __result` nem `let _temp`
nem `let _name`. O `_` simples continua válido como hole (`+ 10 _`),
wildcard em pattern matching (`Result::Err(_)`), e predicados em tipos
refinados (`> _ 0`) — esses são símbolos sintáticos, não identificadores.

As variáveis de reflexão (`_name`, `_arity`, `_types`, `_return_type`,
`_is_action`, `_args`, `_return`) são disponibilizadas pelo compilador
no corpo de actions anotadas com `@directive` (ver seção 3). Fora desse
contexto, referenciar essas variáveis é erro — elas não existem no
escopo global nem em actions comuns.

Esta regra precisa ser implementada no lexer ou no typeck (ver A4
para onde o desugaring encaixa no pipeline).

### A5. `return` explícito e injeção de Exit

Kata5 tem `return` explícito em actions. O desugaring de `Exit` precisa
cobrir todos os pontos de saída. Precisa verificar:

- Todos os caminhos de `return` são explícitos no AST (não há retorno
  implícito escondido em algum constructo)?
- `match` com braços que fazem `return` — o desugaring precisa envolver
  cada braço?
- Como o codegen lida com early return hoje — há um mecanismo de
  "wrapped block" que a injeção pode reusar?

### A6. Validação de contrato

A diretiva declara `on: Target::Action`. Aplicar `@trace` numa função
pura é erro. Mas quem valida — o compilador no momento da aplicação de
`@trace`, ou o typeck no momento da injeção? Provavelmente os dois:
a aplicação de `@nome` verifica que `nome` é uma diretiva válida e que
o alvo é compatível com `on`.

### A7. Importação de diretivas

Diretivas são actions. Actions são importadas normalmente (`import
mod.trace`). A diretiva `@trace` resolve pelo nome da action no escopo.
Isso deveria funcionar sem mecanismo extra, mas precisa ser validado.

### A8. Interação com `@log` existente

`@log` já injeta `kata_rt_log_publish` no prólogo/epílogo com template
interpolation e policies de canal. Se o sistema de diretivas
customizadas não cobre esse poder, `@log` continua intrínseca. Precisa
decidir: `@log` migra quando as diretivas customizadas atingirem paridade,
ou `@log` é sempre intrínseca porque tem poder compile-time que
diretivas customizadas não terão?

### A9. Intercept enter+exit: qualificação por uso de `_return`

No design sem `pass`, a distinção entre Intercept enter-only (short-circuit
puro) e enter+exit (short-circuit + transform) é inferida do corpo: se o
corpo referencia `_return`, o compilador gera o ponto de transformação
após o corpo. Precisa validar:

- O compilador consegue detectar confiavelmente se `_return` é referenciado
  no corpo antes do typeck (no desugaring)?
- O enter call retorna `Optional` (a decisão); o exit call retorna o valor
  final (possivelmente transformado). A mesma action tem dois call sites
  com tipos de retorno diferentes — o typeck aceita isso?
- Alternativa: usar overloading — Intercept para short-circuit, Exit para
  transform. Mas Exit hoje é observational (retorna `Unit`, não muda o
  resultado). Permitir Exit transformador exige mudar a semântica de Exit.

---

## 9. Próximos passos

1. **Implementar o PRD de reflexão** (`docs/PRD-fn-reflection.md`) — sem
   ele, não há `_name` em compile-time e as diretivas não têm metadata.
2. **Verificar polimorfismo de action** (A1) — determinar se Kata5
   suporta actions genéricas e se isso resolve o problema de `_args`.
3. **Verificar pipeline de compilação** (A4) — onde encaixar o desugaring
   de diretivas entre resolution e typeck.
4. **Verificar mecanismo de early return** (A5) — como o codegen lida
   com `return` hoje e como a injeção de Exit cobre todos os caminhos.
5. **Intercept e fallback** (A3) — resolvido: intercept é
   `Target::Action` only, não-transparente, `panic!` aborta.
6. **Intercept enter+exit** (A9) — validar se a detecção de `_return`
   no corpo é viável no desugaring, ou se é necessário overloading
   Intercept + Exit.
7. **Escrever o PRD** — depois de validar as hipóteses acima, converter
   este documento num PRD com fases, DoD, e comandos de verificação.