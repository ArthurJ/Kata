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
@directive{when: Hook::Enter, on: Target::Action, pass: [f.name]}
action trace(name :: Text) => Unit
    log!(LogLevel::Info, "enter: " + name)
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
    trace!("processar")
    x + 1
```

`f.name` vira `TextLit("processar")` em compile-time — o caso estático do
PRD de reflexão. Zero overhead de runtime. A sidecar table e o lookup
dinâmico não se aplicam aqui porque o compilador sempre sabe qual função
está decorada.

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
    Around      # envolve o corpo com possibilidade de short-circuit

enum Target
    Action      # só decora actions
    Function    # só decora funções puras
    Any         # decora ambos
```

### 2.2. Campos de `@directive`

| Campo | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `when` | `Hook` | **sim** | Ponto de injeção |
| `on` | `Target` | **sim** | Que tipo de item pode decorar |
| `pass` | tupla de expressões | não | O que passar como args para a diretiva |

`pass` referencia bindings especiais que o compilador sintetiza no momento
da injeção (ver seção 3).

### 2.3. Resolução

Quando o compilador encontra `@nome` num item:

1. É intrínseca conhecida (`@ffi`, `@cache`, etc.)? → trata como anotação
   hardcoded (comportamento existente).
2. Senão, resolve `nome` no escopo de módulos → é action? → tem
   `@directive`? → lê os campos e injeta a chamada conforme declarado.
3. Senão → erro: `nome não é uma diretiva válida`.

---

## 3. Bindings disponíveis em `pass`

O `pass` é uma lista de expressões avaliadas no contexto da função
decorada. O compilador traduz cada item:

| Expressão | Resolução | Origem | Quando |
|---|---|---|---|
| `f.name` | `TextLit` constante | PRD de reflexão (caso estático) | sempre |
| `f.arity` | `IntLit` constante | idem | sempre |
| `f.param_types` | `List` literal de `TextLit` | idem | sempre |
| `f.return_type` | `TextLit` constante | idem | sempre |
| `f.is_action` | `Boolean::True`/`False` | idem | sempre |
| `args` | tupla runtime dos argumentos | novo (sintetizado dos params) | `Enter`, `Exit`, `Around` |
| `result` | valor de retorno runtime | novo (capturado do corpo) | `Exit`, `Around` apenas |

**Estáticos** (`f.*`) são resolvidos em compile-time — o compilador conhece
a função decorada e extrai as constantes da assinatura. Zero overhead.

**Dinâmicos** (`args`, `result`) são valores de runtime — o compilador
sintetiza `let args := (x,)` a partir dos parâmetros e `let result := <corpo>`
a partir do valor de retorno.

---

## 4. Os três modos de injeção

### 4.1. Enter

Injeta a chamada da diretiva **antes** do corpo da função decorada.

```kata
@directive{when: Hook::Enter, on: Target::Action, pass: [f.name]}
action trace(name :: Text) => Unit
    log!(LogLevel::Info, "enter: " + name)
```

Desugaring de `@trace` em `processar`:

```kata
action processar(x :: Int) => Int
    trace!("processar")
    x + 1
```

### 4.2. Exit

Injeta a chamada **após** o corpo, capturando o resultado. Precisa cobrir
todos os pontos de saída: `return` explícito, retorno implícito (última
expr), e braços de `match`.

```kata
@directive{when: Hook::Exit, on: Target::Any, pass: [f.name, result]}
action trace_exit(name :: Text, result) => Unit
    log!(LogLevel::Info, "exit: " + name)
```

Desugaring de `@trace_exit` em `processar`:

```kata
action processar(x :: Int) => Int
    let __result := x + 1
    trace_exit!("processar", __result)
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
            trace_exit!("buscar", __result)
            return __result
        Optional::None:
            let __result := 0
            trace_exit!("buscar", __result)
            return __result
```

### 4.3. Around

Around é o modo mais complexo. Diferente de Enter/Exit (que são
observação pura), around pode **short-circuit**: a diretiva decide se
chama a função original ou retorna um valor cacheado.

**Restrição de pureza estrutural:**

1. Pode short-circuit (ex: cache pula a execução).
2. O resultado não pode diferir do que a função produziria.
3. Erro de runtime da diretiva não impede o retorno da função.

Isso faz as diretivas around serem **semanticamente transparentes** —
adicionar ou remover a diretiva não muda o comportamento observável da
função.

Implicações:

- `Around` só faz sentido em **funções puras**. Se a função tem efeito
  colateral, short-circuit pula o efeito e muda o comportamento. A
  diretiva deve declarar `on: Target::Function`.
- `@cache` (intrínseca existente) é o caso canônico: o valor cacheado é
  idêntico ao que a função produziria porque a função é pura e
  determinística.
- A regra de "erro não impede retorno" levanta a questão de como o codegen
  protege a chamada da diretiva. `panic!` em Kata5 aborta com `exit(1)` —
  capturar aborto exige isolamento de fiber, que tem overhead. Possível
  interpretação: a regra cobre erros "suaves" (blocking, IO timeout) e
  `panic!` na diretiva é bug do autor e aborta normalmente. Isso precisa
  ser decidido quando a reflexão existir.

Desugaring conceitual de `@cache` (intrínseca, não migrada — só para
ilustrar o modelo around):

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

Modelo cebola (onion), como Python decorators:

```kata
@trace
@cache
action processar(x :: Int) => Int
    ...
```

Expande para:

```
trace.before
    cache.before
        <body → __result>
    cache.after
trace.after
```

Primeira diretiva = camada mais externa. `Enter` executa de cima para
baixo, `Exit` de baixo para cima. Com `Around`, a diretiva mais externa
envolve a mais interna.

---

## 6. Diretivas existentes (não migradas)

As diretivas intrínsecas continuam chumbadas no compilador:

| Diretiva | Modelo | Por que continua chumbada |
|---|---|---|
| `@ffi` | anotação | informa linker de símbolo externo — semântica de compilação, não hook |
| `@builtin` | anotação | marca função para síntese de nó TAST — codegen, não hook |
| `@commutative` | anotação | habilita TRMA — transformação algébrica, não hook |
| `@associative` | anotação | idem |
| `@cache` | around | short-circuit em funções puras — intrínseca, não migrada |
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
diretivas dependem de `f.name`, `f.arity`, etc. em compile-time.

O que o PRD de reflexão fornece:

- `f.name` → `TextLit` constante (caso estático)
- `f.arity` → `IntLit` constante
- `f.param_types` → `List` literal de `TextLit`
- `f.return_type` → `TextLit` constante
- `f.is_action` → `Boolean::True`/`False`

Tudo resolvido em compile-time quando `f` é `Ident` direto — que é
exatamente o caso das diretivas (o compilador sempre sabe qual função está
decorada). A sidecar table e o lookup dinâmico do PRD de reflexão **não se
aplicam** aqui.

Sem a reflexão implementada, não há como o compilador sintetizar
`TextLit("processar")` a partir de `f.name` — a infraestrutura não existe.

---

## 8. Arestas em aberto

Estas são as questões que não podemos definir agora, seja porque dependem
da implementação da reflexão, seja porque precisamos validar contra o
compilador real.

### A1. `args` e polimorfismo

Se `pass: [f.name, args]` e a diretiva declara `args :: (Int, Text)`, ela
só decora funções com essa assinatura exata. Alternativas:

- **Monomórfica:** cada diretiva serve uma assinatura fixa. Simples, mas
  pouco reutilizável.
- **Polimórfica:** a diretiva é genérica em `args`. Precisa de polimorfismo
  de action no Kata5 — precisa verificar se isso existe.
- **Textual:** `args` é serializado como `Text` (via `ty_to_text` do PRD
  de reflexão). Perde o valor real, ganha universalidade. Bom para
  `@log`, inútil para `@cache`.

### A2. `result` e o tipo de retorno

Mesma questão. `pass: [result]` com `result :: Int` só decora funções que
retornam `Int`. Para `@log` isso não é problema porque `{result}` vira
`format` que é polimórfico. Para diretivas customizadas com around, a
solução natural seria receber o valor tipado — mas aí a diretiva é
monomórfica por tipo de retorno.

### A3. `Around` e mecanismo de fallback

A regra "erro de runtime da diretiva não impede retorno" exige que o
codegen envolva a chamada da diretiva around num bloco protegido. Em
Kata5, `panic!` aborta o processo. Capturar aborto exige isolamento de
fiber, que tem overhead. Precisa decidir:

- A regra cobre todo erro de runtime, ou só erros "suaves"?
- `panic!` na diretiva é bug do autor e aborta, ou precisa ser capturado?
- O overhead de isolamento é aceitável para around?

### A4. Desugaring no AST vs. fase de resolução

O desugaring precisa acontecer **depois** da resolução de módulos (para
saber qual diretiva `@trace` se refere) mas **antes** do typeck (para
validar o código expandido). Isso significa:

- Uma nova passada entre resolution e typeck, ou
- Um hook no início do typeck.

Precisa verificar a arquitetura atual do pipeline para decidir onde
encaixar.

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

---

## 9. Próximos passos

1. **Implementar o PRD de reflexão** (`docs/PRD-fn-reflection.md`) — sem
   ele, não há `f.name` em compile-time e as diretivas não têm metadata.
2. **Verificar polimorfismo de action** (A1) — determinar se Kata5
   suporta actions genéricas e se isso resolve o problema de `args`.
3. **Verificar pipeline de compilação** (A4) — onde encaixar o desugaring
   de diretivas entre resolution e typeck.
4. **Verificar mecanismo de early return** (A5) — como o codegen lida
   com `return` hoje e como a injeção de `Exit` cobre todos os caminhos.
5. **Decidir sobre around e fallback** (A3) — se a regra de pureza
   estrutural cobre `panic!` ou só erros suaves.
6. **Escrever o PRD** — depois de validar as hipóteses acima, converter
   este documento num PRD com fases, DoD, e comandos de verificação.