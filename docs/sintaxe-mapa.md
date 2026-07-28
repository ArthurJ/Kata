# Mapa de Sintaxe — Kata-Lang

Relações entre operadores, tokens e construções sintáticas. Cada entrada lista
contexto(s) de uso, semântica e relações com outras peças.

---

## Operadores de Binding

### `let`, `var` e `:=`

```kata
let nome := expressão
var nome := expressão
var nome := nova_expressão   # reatribuição (apenas var)
```

- **`let`**: Declara binding imutável. **`var`**: Binding mutável (exclusivo de Actions).
- **`:=`**: Operador de binding — atribui o valor ao nome. **Exclusivo para binding** (`let` e `var`); não aparece em outro contexto.
- **Relações**:
  - Interage com `|` para fallback: `let a := expr | fallback`.
  - Interage com `?` em Actions: `let texto := ler_arquivo!(id)?`.

---

## Operador Hole (`_`)

Token que representa um "buraco" a ser preenchido. Dois contextos:

### Currying Explícito
```kata
let soma_dez := + 10 _       # gera closure de aridade 1
```

- Posição: call-site, no lugar de um argumento.
- Semântica: Congela a aplicação, gerando closure que aguarda o argumento faltante.
- **Relações**: Toda closure na TAST tem campo `holes` contando quantos `_` restam.

### Placeholder em Predicados
```kata
data (Int, > _ 0) as PositiveInt
enum IMC
    Magreza(< _ 18.5)
```

- Posição: dentro de declaração de tipo (`data`, `enum`), no lugar do valor testado.
- Semântica: Representa o valor sendo testado pelo predicado no construtor inteligente.
- **Relações**: Construtor inteligente sintetiza lambda com Guard usando `_` como parâmetro implícito.

### Distinção do Separador Visual
O símbolo `_` em literais numéricos (`1_000`) é puramente léxico — o lexer descarta, não vira token. Não tem relação semântica com o Hole.

---

## Função `$` (Spread / Aplicação Explícita)

```kata
f $ (a, b, c)         # spread: f recebe a, b, c como argumentos separados
$ (+ 1 2)             # standalone: $ recebe o tuplo (+ 1 2)
let idade := $(PositiveInt 25)
```

- **Lexer**: `$` é `Ident("$")` — não é operador sintático. Qualquer símbolo
  não-reservado vira identificador.
- **Parser**: `$` participa da aplicação greedy normal. Não há tratamento
  especial no parser.
- **Semântica (typeck, não middle-end)**: `$` é um identificador interceptado
  pelo typeck em dois contextos:

  **Contexto 1 — `$` como prefixo de argumento** (`f $ tuplo`):
  O typeck reescreve `f $ (a, b, c)` para que `f` receba `a`, `b`, `c` como
  argumentos posicionais separados. Na TAST, isto vira
  `TypedExprKind::Spread(Box<TypedExpr>)` com tipo `Unit` — `Spread` é um
  marcador, não um valor. Os handlers de aplicação (closures, lambdas, TRMA)
  expandem `Spread(Tuple([a, b]))` em `[a, b]`.

  **Contexto 2 — `$` como callee standalone** (`$ (tuplo)`):
  O typeck cria `Spread(Tuple([...]))`. O argumento deve ser uma tupla
  (`SpreadExpectedTuple` se não for).

  Em ambos os casos, `$` **nunca chega ao codegen como call**. É totalmente
  resolvido no typeck — `Spread` é expandido pelos handlers antes do lowering.

- **Por que não é um builtin sintetizado**: `$` não tem assinatura de função
  e não produz um valor de retorno. É um marcador na TAST com tipo `Unit`,
  expandido pelos handlers de aplicação. Se fosse um builtin, teria que ter
  tipo `Tuple([T]) → T` — mas o `Spread` tem tipo `Unit`, não `T`. A
  interceptação no typeck é o mecanismo correto, não síntese de builtin.

- **Relações**:
  - Útil quando o argumento é um tuplo que precisa ser desempacotado em
    argumentos posicionais. Para construtores refinados, a notação prefixa
    `PositiveInt 25` funciona diretamente — `$` só é necessário quando o
    argumento já está em forma de tuplo.
  - Interage com `|` em `$(PositiveInt 25) | 0` e com `?` em Actions.
  - O TRMA helpers expandem spreads antes de analisar padrões recursivos.

---

## Operador `::` (Etiqueta de Tipo)

Em todos os contextos, `::` realiza a mesma operação: **anexa uma etiqueta de
tipo ao nome à esquerda**. `X :: Y` diz "X tem tipo/forma/pertinência Y".

A decisão de unificar todos estes usos sob um único operador foi deliberada —
não é acidente. As alternativas seriam operadores distintos para cada contexto
(mais sintaxe, mais casos no parser) ou reutilização de `:` (que já separa
guards e patterns). O `::` unificado com distinção por lookahead de 1 token
(`::(` = type params, `::Ident` = variante) provou ser a opção mais simples.
| Contexto | Exemplo | O que está sendo etiquetado | A etiqueta |
|---|---|---|---|
| Assinatura de função | `+ :: Int Int => Int` | A função `+` | Sua assinatura completa |
| Campo de struct | `data Pessoa (nome::Text idade::Int)` | O campo `nome` | Seu tipo `Text` |
| Parâmetro de tipo (genérico) | `Result::(T, E)`, `Iterable::A` | O tipo genérico `Result` | Seus parâmetros `(T, E)` |
| Qualificação de variante | `Transacao::Aprovada`, `Result::Ok` | O enum `Transacao` / `Result` | A variante `Aprovada` / `Ok` |
| Ascription de expressão | `5::PositiveInt`, `x::Int` | A expressão `5` / `x` | O tipo afirmado `PositiveInt` / `Int` |
| Downcast estrutural | `a::Int` onde `a :: PositiveInt` | O valor `a` (refined/alias) | Seu tipo base `Int` |

- **Relações**:
  - `Result::(T, E)` vs `Result::Ok`: Não são operações distintas — é sempre `Nome :: Algo`. Parâmetros de tipo naturalmente formam tupla `(T, E)`; variantes naturalmente são identificadores `Ok`. O `::` é o mesmo.
  - A qualificação `Enum::Variante` é **sempre válida**. `Result::Ok`, `Optional::Some`, `Boolean::True` existem. Só não são obrigatórios quando a variante já está disponível no escopo (ex: as do prelude vêm importadas automaticamente).
  - `Optional::T` é análogo a `Result::(T, E)`: `enum Optional { Some(T), None }`.
  - `Boolean` é `enum Boolean { True, False }` — variantes unitárias, sem payload.
  - `::` em assinatura é declaração; `->` é descrição de tipo de função como valor.
  - **Ascription de expressão** é pós-fixada: `expr::Type` etiqueta a expressão à esquerda com o tipo à direita. Para tipos refined com literal (`5::PositiveInt`), o compilador valida predicados em compile-time e entrega o tipo refined direto (sem `Result`). Para tipos base (`x::Int`), verifica e rebaixa ao tipo anotado. É o mesmo `::` dos outros contextos, agora em posição de expressão.
  - **Açúcar `[T]`**: `quicksort :: [Int] => [Int]` é equivalente a `quicksort :: List::Int => List::Int`. O parser desugara `[T]` para `TypeExpr::ParamApp { name: "List", params: [T] }` — o mesmo nó AST que `List::T` produz. Funciona em qualquer posição de tipo: parâmetros, retorno, anotações. Aninhamento: `[[Int]]` → `List::(List::Int)`. Múltiplos: `[A] [B]` → duas `List::` independentes. O desugaring é puramente sintático — typeck, codegen e runtime veem apenas `List::T`.
  - **Downcast estrutural** (post-refines): `a::Int` onde `a` é refined ou alias sobre `Int` rebaixa ao tipo base. O typeck verifica que `target_ty` é o base (via `alias_of`). No-op em runtime (mesmos bits) — o codegen emite `bitcast` apenas quando o Cranelift type difere (ex: `I64→F64`). Não valida predicado; é a válvula explícita para combinar refineds distintos ou interagir com a base, complementar ao `refines` (que é automático via fallback). Ver manual §4.2.7 modo 4 e §4.4.

---

## Operadores `=>` e `->` (Assinaturas e Tipos de Função)

| Operador | Contexto | Exemplo |
|---|---|---|
| `=>` | Declaração de assinatura (nível léxico) | `soma :: Int Int => Int` |
| `->` | Tipo de função como valor | `(T2 T3 -> T4)` |

### `=>` — Declaração de Assinatura

Separa os tipos dos argumentos (esquerda) do tipo de retorno (direita). Os argumentos não usam parênteses — são listados com espaços:

```kata
nome :: TipoArg1 TipoArg2 => TipoSaida
```

### `Action(Params) => Ret` — Tipo de Action como valor first-class

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)
```

- **Sintaxe**: `Action(T1, T2, ...) => Ret` — espelha a assinatura de actions
  (`action nome (p::T, ...) => Ret`), sem os nomes dos params.
- **Posição**: qualquer posição onde um tipo aparece (assinaturas de Action,
  tipos de param, annotations).
- **Semântica**: `Ty::Action(Vec<Ty>, Box<Ty>)` — separada de `Ty::Function`
  porque as ABIs são semanticamente diferentes (Actions usam scheduler cooperativo,
  funções puras não).
- **Referência sem `!()`**: `worker_a` (sem `!()`) é uma referência que carrega
  o tipo `Action(Int) => Unit`. O valor em runtime é o `fn_ptr` (i64) da Action.
- **Relações**:
  - `worker_a` é referência (valor first-class); `worker_a!(42)` é invocação.
  - Pode ser passada como parâmetro de outra Action: `dispatcher!(worker_a, 42)`.
  - Pode ser armazenada em `let`: `let f := worker_a`.
  - Invocação indireta: `f!(42)` onde `f` é variável com `ty: Action`.
  - **Restrições**: Actions não entram em `data` (são comportamento, não
    informação), não entram em canais, e não são aceitas como parâmetro de
    função pura. Sem interface `CALLABLE` — Functions e Actions são reinos
    separados com ABIs diferentes.

### `->` — Tipo de Função

Descreve a assinatura de uma função como tipo transitável. Exige parênteses para desambiguar:

```kata
nome2 :: T1 (T2 T3 -> T4) => T5
```

A função `nome2` recebe:
1. Um argumento do tipo `T1`
2. Uma função que recebe `T2` e `T3` e retorna `T4`

E retorna `T5`.

- **Relações**: Internamente, toda função é `Closure` na TAST. Uma closure com tipo `(A -> B)` pode ser passada como argumento para `map`, armazenada em binding, ou retornada por outra função.

---

## Operadores de Tratamento de Erro

### `?` (Delegação / Fail-Fast)
```kata
let texto := ler_arquivo!(id)?
```

- Posição: sufixo de expressão.
- Semântica: Se `Ok(v)` ou `Some(v)`, desempacota `v`. Se `Err(e)` ou `None`, aborta a rotina atual retornando `Err(e)` ou `None`.
- **Domínio**: Exclusivo de Actions. A Action precisa ter `Result` (ou `Optional`) como tipo de retorno — `?` injeta `return Err(e)`/`return None` na TAST.
- **Relações**: Interage com `!` (ações) e com `|` (que é a alternativa local, sem aborto de fluxo).

### `|` (Contenção / Fallback Local)
```kata
... / p $(* a a) | 1                    # lambda pura
let a := $(Altura typed_input!(...)) | exit!(...) # action
```

- Posição: infixo entre duas expressões.
- Semântica: Desempacota o payload de qualquer variante não-cauda. Se a
  esquerda é a cauda (última variante, unitária), avalia e retorna a direita.
- **Aplica-se a**: Qualquer enum cujas variantes (exceto a última) carreguem
  payload. A generalização para qualquer enum (não apenas `Optional`)
  foi uma decisão deliberada — `IMC` (4 variantes: Magreza/Normal/Sobrepeso/Obesidade)
  funciona com `|`: desempacota o payload se for qualquer variante não-cauda,
  avalia o lado direito se for a última. Enums com variantes unitárias não-cauda
  (ex: `Boolean` com `True` antes de `False`) não são compatíveis com `|` — type
  error, porque não há payload para desempacotar. `Result` NÃO é compatível
  com `|` (Err tem payload, não é cauda unitária) — use `?` para fail-fast.
- **Domínio**: Funções puras e Actions.
- **Relações**: Preserva pureza — ao contrário de `?`, não aborta fluxo. Com
  construtores refinados (notação prefixa `PositiveInt 25 | 0`), se o fallback é
  literal do tipo base, o compilador valida os predicados do fallback em
  compile-time (coerção contextual). A declaração de `enum` **não** usa `|`
  — variantes são listadas por indentação.

---

## Operador `in` (Membership / Contenção)

```kata
3 in {1 2 3}              # true — Array contains
5 in [0..2..10]           # true — Range O(1) aritmético
x in [1 2 3]             # List contains (percurso linear)
```

- **Posição**: infixo entre item (esquerda) e coleção (direita).
- **Semântica**: Testa se o item pertence à coleção. Produz `Boolean`.
- **Por tipo concreto** (inlined pelo codegen, sem dispatch em runtime):
  - **List**: percurso linear via `kata_rt_list_contains`.
  - **Array**: percurso linear via `kata_rt_array_contains`.
  - **Range**: O(1) aritmético — `start <= item AND item < end` (não verifica
    step). Dois `icmp` combinados com `band`, resultado `uextend` para I64.
- **Domínio**: Funções puras e Actions.
- **Relações**: Despacha via interface `CONTAINS(A)`. No codegen, o tipo concreto
  é conhecido em compile-time pela TAST, então o dispatch é inlined por tipo —
  sem FFI dispatch. Adicionado no Fio 8 (Fase 6).

---

## Operador `!` (Actions e Builtins)

### Sufixo de Chamada
```kata
echo!("mensagem")
fork!(minha_action)
conectar_servidor!()
```

- Posição: sufixo do nome da função na chamada.
- Semântica: **Sinaliza impureza** — toda chamada a Action usa `!`. Na declaração, Actions não usam `!` (`action conectar_servidor` vs chamada `conectar_servidor!()`). Actions podem ser passadas como argumento sem `!` (referência, sem ativação).
- **Argumentos**: Toda Action recebe exatamente uma tupla como argumento (parênteses obrigatórios na chamada). Pode ser tupla vazia (`!()`) para Actions sem parâmetros, ou tupla de N elementos para variadismo.
- **Relações**:
  - Algumas Actions são builtins do compilador (`fork!`, `panic!`, `assert!`), outras são stdlib (`echo!`), mas todas seguem a mesma sintaxe `!`.
  - Interage com `?` e `|` no tratamento de erro.
  - **First-class**: `worker_a` sem `!()` é uma referência (valor do tipo `Action(Int) => Unit`). `worker_a!(42)` é invocação. Ver secção `Action(Params) => Ret` acima.

### `panic!` e `assert!` (Builtins de Abort)

```kata
panic!("mensagem")              # aborta: stderr + exit(1), retorna Unit
assert!(cond)                   # 1 arg: panic!("assertion failed") se False
assert!(cond, "msg custom")     # 2 args: panic!(msg) se False
```

- **`panic!`**: Aborta imediatamente. Destrói a arena local. Retorna `Unit` no
  tipo, mas o fluxo nunca chega ao retorno. Lowerado direto para FFI
  (`kata_rt_panic`).
- **`assert!`**: Interceptado no typeck antes do DispatchTable. Desugado para
  `match cond { True: Unit, False: panic!(msg) }`. `cond` deve avaliar para
  `Boolean`. 1 arg = mensagem default; 2 args = mensagem customizada.
- **Domínio**: Ambos são Actions (exigem `!`). Não existem em funções puras.
- **Relações**: `assert!` sobrevive a tree shaking (não é `@test`). Testes de
  abort (`panic!`, `assert!(False)`) não podem usar `eval_src` — `exit(1)` mata
  o runner. Usar `#[ignore]` e validar via `cargo run --bin kata -- run`.

### Canais CSP
| Operador | Direção | Exemplo |
|---|---|---|
| `!>` | Envio | `canal !> valor` |
| `<!` | Recebimento | `canal <! variavel` |

- `channel!`, `queue!(N)`, `broadcast!` são actions que criam canais.
- **Relações**: `fork!` submete Action a corrotina no scheduler cooperativo single-threaded. `select` multiplexa canais. Escape Analysis rastreia dados enviados por `!>` para alocação heap/`Arc<T>`.

---

## Operador `|>` (Pipeline / Composição)

```kata
f x |> g              # equivale a g(f(x))
f x |> g _ y          # Hole define posição: g(f(x), y)
f x |> g |> h         # encadeamento left-assoc: h(g(f(x)))
5 |> + 10 _           # 15 — Hole substituído por 5: + 10 5
5 |> + _ 10           # 15 — Hole substituído por 5: + 5 10
5 |> + 1 _ |> * 2 _   # 12 — left-assoc: ((5 + 1) * 2)
```

- Posição: infixo entre uma expressão (esquerda) e uma função (direita).
- Semântica: O resultado da esquerda é inserido na função à direita. A resolução depende da presença de Hole:
  - **Com Hole**: `f x |> g _ y` → o typeck substitui o `_` pela AST de `f x` e infere o resultado. Equivalente a `g (f x) y`. O Hole pode estar em qualquer posição.
  - **Sem Hole**: `f x |> g y z` → o resultado da esquerda é injetado como primeiro argumento: `g (f x) y z`. Se o lado direito é uma função nua (`f x |> g`), injeta como único argumento: `g (f x)`.
  - **Múltiplos Holes**: `f x |> g _ _` — todos os Holes são substituídos pelo mesmo valor da esquerda. Erro de tipo se a função não aceita argumentos repetidos do tipo esperado.
- **Precedência**: Mais baixa que aplicação de função. Associatividade à esquerda: `a |> b |> c` = `(a |> b) |> c`.
- **Implementação**: O desugar é total no typeck — a TAST nunca contém `Pipe`. O codegen não precisa de mudanças.
- **Relações**: Diferente de `|`, que é coalescência de erro. `|>` é pipeline de transformação pura. Mencionado na seção 5.1 como alternativa funcional para compor tratamento de sucesso/falha sem `match`.

---

## Operador `return` (Early Return em Actions)

```kata
action buscar
    let x := ler!()?
    match x
        Optional::Some(v): return v       # early return explícito
        Optional::None: return 0

action calcular
    let x := 5
    let y := + x 1
    y                                      # retorno implícito (última expr sem ;)
```

- Posição: prefixo de expressão dentro de Action.
- Semântica: Aborta a execução da Action e retorna o valor. O compilador aloca o
  valor na **caller's arena** (arena de quem chamou a Action), que persiste até o
  caller terminar.
- **Domínio**: Exclusivo de Actions. Não existe em funções puras — guards e
  pattern matching são o mecanismo de fluxo no domínio puro, e `return` seria
  redundante com guards (que já são early return).
- **Retorno implícito**: A última expressão de uma Action sem `;` é tratada como
  retorno implícito. Mesma semântica de caller's arena.
- **Contexto de design**: `return` keyword + caller's arena resolvem
  estruturalmente o problema de Actions que retornam coleções. Sem `return`
  explícito, o codegen vazava ponteiros crus quando a expressão era uma coleção
  (a arena da action é destruída no epílogo). A solução paliativa mapeava
  coleções → `Unit` no retorno. O `return` explícito + caller's arena resolve
  estruturalmente.
- **Relações**: Interage com `?` (que também é early return, mas para erro).
  Interage com `;` (que suprime retorno implícito → Unit).

---

## Terminador `;` (Statement em Actions)

```kata
action processar
    let x := 5; echo!(x)       # dois statements na mesma linha
    let y := + x 1
    y                           # retorno implícito (última expr sem ;)

action greet
    echo!("hello")
    echo!("world");            # ; → statement, action retorna Unit
```

- Posição: sufixo de expressão em Actions.
- Semântica: Termina um statement. Quando a última expressão de uma Action tem
  `;`, a Action retorna `Unit`. Sem `;`, a última expressão é o retorno
  implícito.
- **Domínio**: Exclusivo de Actions. Não existe em funções puras — o domínio
  puro não tem statements, só expressões. A última expressão é sempre o retorno.
- **Uso primário**: Permitir múltiplos statements na mesma linha e explicitar
  que uma expressão não é valor de retorno.
- **Relações**: O `;` distingue "computação local" de "valor que escapa" — esta
  distinção é o que habilita o caller's arena. Sem `;`, o compilador sabe que a
  expressão é um retorno e aloca na arena do caller. Com `;`, a expressão é
  computação local na arena da Action, liberada no epílogo.
- **Sem conflito com `;` em coleções**: `;` em `{1; 2; 3}` (tensor) é separador
  de dimensões dentro de delimitadores de coleção. `;` em statement level vive
  fora de delimitadores. O parser distingue por estado: dentro de `{}` = dimensão,
  fora = terminador. Não há caso onde um `;` seja ambíguo entre os dois papéis —
  o `}` sempre fecha o contexto de coleção antes de um `;` terminador aparecer.

---

## Operador `.` (Acesso de Campo e Indexação)

```kata
pessoa.nome              # field access em struct
t.0                      # indexação em tupla (compile-time safe, retorna T_N direto)
t.(-1)                   # último elemento da tupla (índice negativo conta do fim)
arr.0                    # indexação em array (desugar → at, retorna Result)
arr.(-1) ?               # último elemento do array (unwrap do Result)
lst.2 ?                  # indexação em lista (O(n) traversal, retorna Result)
```

- Posição: infixo entre expressão e nome/índice.
- Semântica tripla, distinguida pelo tipo do receptor (typeck resolve):
  - **Struct**: `expr.nome` acessa campo da struct pelo nome. Typeck verifica
    que o campo existe (`FieldNotFound` se não). Retorno direto.
  - **Tupla**: `expr.N` acessa elemento por índice literal. Typeck faz bounds
    check em compile-time (`IndexOutOfBounds` se fora). Índice negativo conta
    do fim (`t.(-1)` = `t.(len-1)`, resolvido estaticamente). Retorno direto —
    `T_N`, sem `Result`. Lowering é `Load` por offset.
  - **Coleções (INDEXABLE)**: `expr.N` é syntactic sugar para `at expr N`.
    Retorna `Result::(A, Err)` — o programador usa `?` (em Actions) ou `match`
    explícito para desempacotar.
    Índice negativo conta do fim (runtime resolve). List é O(n), Array é O(1).
- **Distinção**: o parser aceita `Ident` (field access) ou `Int` (indexação)
  após `.`. O typeck resolve pelo tipo: struct → field, tupla → IndexAccess
  compile-time, INDEXABLE → desugar para `at`, outro → `NotIndexable`.
- **Atrito sadio**: Tupla retorna direto (compile-time proof), coleções retornam
  `Result` (runtime risk). Mesma distinção de `/` (exato) vs `div` (dinâmico).
- **Invariante de codegen**: Tuple é sempre heap type (ponteiro). Acesso por
  índice é `Load` por offset. Mesmo padrão que Sum com payload.
- **Relações**: Interage com `?` em coleções: `arr.0 ?` (unwrap de Result).
  `arr.0 | 0` é type error (Result não é compatível com `|` — use `?` em Actions
  ou `match` explícito em funções puras).
  Em tuplas, `t.0 ?` é type error (`?` exige Result, tupla retorna direto)
  — o type system enforces a distinção.

---

## Função `len` (Tamanho)

```kata
len (10, 20, 30)         # 3 — tupla (síntese compile-time, IntLiteral)
len {1 2 3}              # 3 — array (COUNTABLE dispatch, kata_rt_array_len)
len [1 2 3]              # 3 — lista (COUNTABLE dispatch, traversal stdlib)
len "hello"              # 5 — text (COUNTABLE dispatch, kata_rt_string_len)
```

- Posição: prefixo de expressão.
- Semântica: Retorna o número de elementos. Uniforme na sintaxe, distinta no
  mecanismo:
  - **Tupla**: Special case do typeck — conta `element_types.len()`, emite
    `IntLiteral`. Zero-cost, nunca chega ao codegen. Tuple não implementa
    interfaces (é tipo estrutural, não nominal).
  - **Coleções**: Dispatch via interface `COUNTABLE` — `len :: Self => Int`.
    Cada tipo implementa com o mecanismo apropriado (FFI para Array/Text,
    traversal stdlib para List).
- **Relações**: `COUNTABLE` é interface nominal; `len` em Tuple é special case
  do typeck. Mesma split de `.N` (Tuple = compile-time, coleções = interface).

---

## Diretivas (`@`)

```kata
@nome
@nome("arg")
@nome{chave: valor}
```

- Posição: precedem item de topo (action, lambda, data). Podem ser empilhadas no mesmo item; a ordem importa (avaliação sequencial).
- Catálogo: `@parallel`, `@comptime`, `@cache_strategy`, `@test`, `@log`, `@associative`, `@commutative`, `@ffi`, `@builtin`.
- **Relações**:
  - `@test` → tree shaking remove em produção.
  - `@associative` + `@commutative` → habilitam TRMA.
  - `@ffi` → informa linker de símbolo externo.
  - `@builtin` → marca função para síntese de nó TAST especializado (map/filter/fold).
  - `@parallel` → spawn de processo OS separado (multiprocess).
  - `@log` → veja seção dedicada abaixo.

---

## Diretiva `@log` (Telemetria via CSP)

Anotação em actions e funções nomeadas que injeta `kata_rt_log_publish` no wrapping (prólogo ou epílogo da definição). Permite emitir telemetria estruturada sem contaminar a assinatura matemática — a pureza nominal da função não muda. Independente da action nativa `log!()` (que dispara na execução da linha).

```kata
@log{msg: "processando {x}", level: LogLevel::Info, topic: "audit", policy: "block", when: "exit"}
action processar (x::Int) => Int
  let result := * x 2
  result
```

### Campos da diretiva

| Campo | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `msg` | `Text` | **sim** | Template compile-time. `{expr}` interpola expressão do escopo (Ident ou `Ident.field`). `{{` escapa `{` literal; `}}` escapa `}`. `{` sem `}` = erro. Desugara para `format "template" (expr1, ...)` via `infer_format`. |
| `when` | `Text` | **sim** (Decisão D1) | `"enter"` = loga no prólogo. `"exit"` = loga no epílogo. Ausente = erro compile-time (`when é obrigatório em @log`). Outro valor = erro. |
| `level` | `LogLevel` | não | Variante do enum `LogLevel` do prelude (`Debug`/`Info`/`Warn`/`Error`). Default: `Info`. |
| `topic` | `Text` | não | Nome do canal onde publicar. Default: herdado do fiber ancestral (ou `"default"` se nenhuma config). |
| `policy` | `Text` | não | `"drop"` (fire-and-forget via Broadcast) ou `"block"` (Queue bounded cap=1 com backpressure, bloqueia se cheio). Default: herdado (ou `"drop"`). |

### Restrições de `when`

- `when: "enter"` → placeholders do `msg` **só podem referenciar params** da função. Referenciar variável do corpo é erro compile-time (a variável não existe no prólogo).
- `when: "exit"` → placeholders podem referenciar params e variáveis do corpo. O codegen injeta a publicação antes de cada ponto de saída (`return` explícito, retorno implícito, braços de `match`).

### Canais e policies

Tópicos são canais nomeados, resolvidos sob demanda num registry `HashMap<String, i64>` (nome → handle). Primeira referência a `"audit"` cria o canal; subsequentes reusam.

- **`"drop"`** → canal Broadcast (fire-and-forget). Não bloqueia o publisher. Cada receiver mantém seu próprio `last_seen_version`; receivers lentos perdem mensagens intermediárias (o `BroadcastInner` guarda só a última versão). Reusa `kata_rt_broadcast_create` + `kata_rt_channel_send`.
- **`"block"`** → Queue bounded (cap=1) com backpressure. Bloqueia o publisher via `WaitingOnChannelSend` até o consumidor liberar o slot com `channel_recv`. Reusa `kata_rt_queue_create` + `kata_rt_channel_send`.

### Enum `LogLevel` no prelude

```kata
enum LogLevel
  Debug
  Info
  Warn
  Error
```

Fixo no `stdlib/core.kata`. Extensibilidade (interfaces, herança de enum) é dívida técnica — não no escopo do Fio 14.

### `TypedLogSpec` na TAST

O typeck consome o `LogSpec` do resolution e produz `TypedLogSpec`, que é **enum** (não struct):

```rust
enum TypedLogSpec {
    Enter { msg_expr: Spanned<TypedExpr>, topic: Option<String>, policy: Option<String>, level: i64 },
    Exit  { msg_expr: Spanned<TypedExpr>, topic: Option<String>, policy: Option<String>, level: i64 },
}
```

O codegen despacha no variant: `Enter` → injeta `kata_rt_log_publish` no prólogo; `Exit` → injeta antes de cada saída. `level` é tag numérica (`Debug=0, Info=1, Warn=2, Error=3`).

### Configuração herdada via `log_config!()`

Defaults de `topic`/`policy`/`level` são armazenados em TLS `LOG_CONFIG: RefCell<Option<LogConfig>>` no runtime. No `kata_rt_spawn`, o scheduler copia o `LOG_CONFIG` do fiber pai para o filho (snapshot). Mudanças no pai após o spawn não propagam para filhos já spawnados. Configura-se em runtime via `log_config!()` (abaixo).

- **Relações**:
  - Independente de `log!()` — ambos podem coexistir na mesma action.
  - `@log` dispara no wrapping (chamada); `log!()` dispara na linha.
  - `when` é obrigatório — diverge do PRD aspiracional (`§2.2`), que descreve automação; decisão fechada com Arthur (2026-07-19) tornou `when` obrigatório. PRD mantido como referência histórica.
  - `policy: "block"` pode deadlockar se nenhum consumidor existe — mitigação via `DEADLOCK_SENTINEL` existente do scheduler.
  - Não se aplica a métodos de `implements` — só actions e funções nomeadas.

---

## Actions nativas de log: `log!()`, `log_recv!()`, `log_config!()`

Três actions interceptadas no typeck (como `format`, `map`, `filter`, `len`) — não passam pelo DispatchTable. Desugaram para FFIs do runtime (`kata_rt_log_publish`, `kata_rt_log_recv`, `kata_rt_log_config`).

### `log!()` — publicação explícita

```kata
log!(LogLevel::Info, "mensagem dinâmica", "audit", "drop")
```

A mensagem (pos 1) é **dinâmica** — construída em runtime, sem interpolação
de template. `{valor}` na string é texto literal, não é substituído. Para
interpolação compile-time com `{expr}`, use a diretiva `@log`.

Sintaxe posicional (action call existente: `Ident ! (tuple)`):

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `LogLevel` | Level da mensagem. |
| 1 | `Text` | Mensagem. Pode ser dinâmica (construída em runtime). |
| 2 | `Text` | Tópico. Opcional — default herdado ou `"default"`. |
| 3 | `Text` | Policy. Opcional — default herdado ou `"drop"`. |

Typeck aceita 2, 3 ou 4 args. Dispara no ponto da chamada (linha), diferente de `@log` que dispara no wrapping.

### `log_recv!()` — consumo de telemetria

```kata
let msg := log_recv!("audit")
```

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `Text` | Tópico a consumir. |

Bloqueia via `YieldReason::BlockedOnRecv` até chegar mensagem. Retorna `Text` (payload) ou `Unit` se o canal fechou. Precisa estar em fiber context (`fork!()` ou action) — `kata_rt_channel_recv` só bloqueia dentro de um fiber; entry point não é fiber.

Para Broadcast, o receiver é criado eagerly no `get_or_create_topic` (antes de qualquer publish) e cached em `RECEIVER_REGISTRY` (thread_local) — garante que mensagens publicadas antes de qualquer `log_recv` sejam visíveis.

### `log_config!()` — configura defaults do fiber

```kata
log_config!("audit", "block", LogLevel::Info)
```

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `Text` | Tópico default. |
| 1 | `Text` | Policy default. |
| 2 | `LogLevel` | Level default. |

Setta `LOG_CONFIG` TLS no fiber atual. Filhos spawnados herdam via snapshot no `kata_rt_spawn`.

- **Relações**: `log_config!()` é action nativa (não diretiva) porque configura em runtime, dinamicamente. Diretiva seria compile-time.
- **Void FFI**: `kata_rt_log_config` é void (sem retorno). O `lower_closure` verifica `inst_results.is_empty()` antes de indexar e retorna `iconst(I64, 0)` (Unit) — não panica no codegen.

---

## Delimitadores de Coleções

| Sintaxe | Tipo | Layout |
|---|---|---|
| `[1 2 3]` | Lista persistente | Encadeada (Cons). Pattern: `[h : t]` (cabeça : cauda). `[]` para lista vazia. |
| `{1 2 3}` | Array contíguo | Bloco contíguo (imutável por padrão). |
| `{"k": v "k2": v2}` | Dict (HAMT) | Mapeamento persistente imutável. `:` após primeira entrada desambigua de Array. `{:}` para vazio. |
| `{|1 2 3|}` | Set (HAMT) | Conjunto persistente imutável. `|` após `{` ativa modo Set. `{||}` para vazio. |
| `{1; 2; 3}` | Tensor N-D | Dimensões separadas por `;` |
| `(1, 2, 3)` | Tupla | Agrupamento heterogêneo. `(42,)` é tupla de 1 elemento (vírgula obrigatória). `(42)` é agrupamento, não tupla. `()` é `Unit`. |

- **Ranges (Lazy)**: `[0..10]` (0 a 9), `[0..=9]` (0 a 9 incluso), `[0..2..10]` (step 2: 0, 2, 4, 6, 8). Geram `Range` lazy que implementa `ITERABLE`. O step é definido por um segundo `..`: `start..step..end`.

---

## Keyword `refines` (Delegação de Interface para Refined)

```kata
data (Int, > _ 0) as PositiveInt

PositiveInt refines NUM
# bloco opcional com overrides (caso misto)
```

- **Posição**: top-level, após declaração do tipo refined.
- **Semântica**: Registra que o tipo refined delega uma interface ao seu tipo
  base. **Não** cria overloads no `DispatchTable`, **não** registra no
  `InterfaceRegistry`. O mecanismo é um fallback no dispatch: quando `+ a b`
  falha porque os args são `PositiveInt`, o typeck substitui pelo base `Int`,
  retenta, e — se o retorno implementa a interface — passa pelo construtor
  falível do refined → `Result::(Refined, Err)`.
- **Sem bloco**: delega todos os métodos da interface ao base.
- **Com bloco**: métodos com corpo lambda = override (cria overload real,
  encontrado antes do fallback); métodos não-listados = delegação automática.
- **Restrições**:
  - Só se aplica a tipos refined (`alias_of` + `predicates`). Non-refined →
    erro compile-time.
  - A interface deve já estar implementada pelo base. Se não → erro.
  - Sem `type_params`/`iface_params` em 1.0.
- **Construtor condicional**: o construtor falível é chamado só quando o tipo
  de retorno implementa a interface. `+` retorna Int (implementa NUM) →
  `Result::(PositiveInt, Err)`. `<` retorna Boolean (não implementa NUM) →
  Boolean direto.
- **Interoperabilidade opt-in**: sem `refines`, `+ a 0` onde `a ::
  PositiveInt` falha. Com `refines NUM`, o fallback substitui e funciona.
- **Incompatibilidade nominal**: o fallback só dispara quando todos os args
  refined são o **mesmo** tipo. `+ a b` onde `a :: PositiveInt` e `b ::
  NonZeroInt` falha mesmo com ambos `refines NUM`.
- **Relações**:
  - Complementar ao downcast via `::` (§`::` modo downcast): `refines` é
    automático no dispatch; downcast é explícito.
  - `refines` vs `implements`: `implements` registra no InterfaceRegistry e
    cria overloads; `refines` não registra e usa fallback. Um tipo pode ter
    ambos.
  - Interage com `T?` (abaixo): o tipo de retorno do fallback é
    `Result::(Refined, Err)` ≡ `Refined?`.
  - SHOW é automático para todos os tipos, inclusive refined — não precisa de
    `refines SHOW`. Ver manual §4.2.9.

---

## Açúcar `T?` (Tipo Falível)

```kata
soma_positiva :: PositiveInt PositiveInt => PositiveInt?
# equivalente a: PositiveInt PositiveInt => Result::(PositiveInt, Err)
```

- **Posição**: sufixo de tipo, em qualquer posição onde um tipo aparece
  (assinaturas, retorno, campos, ascriptions).
- **Semântica**: `T?` desaçuca para `Result::(T, Err)` onde `Err = Text`.
  Açúcar puramente sintático — o typeck resolve antes de qualquer verificação.
- **Não é subtyping**: `Int` não é subtipo de `Int?`. São tipos distintos.
  Uma função que retorna `Int` não satisfaz `=> Int?` sem wrap explícito.
- **Não muda o operador `?` de runtime**: `?` em runtime continua sendo
  desempacotamento de `Result`. `T?` é açúcar de tipo; `?` em expressão é
  operador de runtime. Coisas diferentes, mesmo símbolo.
- **Relações**:
  - Com `refines`: o retorno do fallback no dispatch é `Result::(Refined, Err)`
    ≡ `Refined?`. O usuário pode escrever `PositiveInt?` em vez de
    `Result::(PositiveInt, Err)` no tipo de retorno de funções que recebem
    refined.
  - Com `?` operador: `?` desempacota `Result` em Actions. `T?` produz
    `Result`. São complementares — um é tipo, outro é operador.

---

## Palavras-Chave de Estrutura

| Palavra | Uso |
|---|---|
| `lambda` / `λ` | Declara função anônima. Múltiplas cláusulas após assinatura: `lambda <padrões>: <corpo>` — a primeira que encaixa vence |
| `action` | Declara Action com params nomeados: `action nome (p::T, ...) => Ret`. Forma posicional legada `(T1 T2) -> Ret` removida na migração total. Params sem nome não são mais aceitos; `::` etiqueta é obrigatória em cada argumento. `=>` separa args de retorno (igual a assinaturas de função). Sem params: `action greet` (retorna `Unit`) ou `action greet => Unit` (retorno explícito). |
| `data` | Declara tipo produto |
| `enum` | Declara tipo soma |
| `alias` | Cria Newtype |
| `interface` | Declara contrato de tipo |
| `implements` | Implementa interface (ex: `T implements IFACE`) |
| `refines` | Delega interface do tipo base ao refined (ex: `PositiveInt refines NUM`). Não registra no InterfaceRegistry; usa fallback no dispatch |
| `import` | Importa módulo |
| `export` | Exporta itens |
| `as` | Alias de import (`import x as y`) ou de tipo (`data (...) as Nome`, `alias T as Nome`) |
| `with` | Bloco bottom-up ao final de lambda: computações prévias nomeadas para Guards, e restrições de genéricos |
| `match` | Pattern matching disponível em ambos os domínios (funções puras e Actions). `otherwise` é obrigatório quando há guards na cláusula e o compilador não consegue provar estaticamente que os braços cobrem todas as variantes possíveis do tipo inspecionado. Sem guards, o body direto dispensa `otherwise`. Em funções puras, cada braço deve retornar um valor (expressão); em Actions, braços podem ser statements. |
| `return` | Early return em Actions. Não existe em funções puras. |
| `if` | **Não existe — invariante absoluta.** Lógica condicional é expressa via pattern matching (que garante exaustividade) e guards (que garantem fallback via `otherwise`). |

## Cláusulas Lambda Múltiplas

```kata
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail $(- n 1) $(* n acc)
```

- **Sintaxe:** Assinatura `nome :: T1 T2 => TRet` seguida de zero ou mais `lambda <padrões>: <corpo>`. Zero cláusulas = definição FFI (corpo suprido por `@ffi`).
- **Padrões:** Reusam `Pattern` integralmente — Ident, Wildcard, Literal, Variant, Tuple, Cons.
- **Dispatch:** Eager — argumentos avaliados pelo caller. Cláusulas testadas em ordem; a primeira que encaixa é executada. Nenhuma encaixa → runtime trap.
- **Exaustividade:** Reusada do `match`. Tipos soma (`Boolean`, `Result`) exigem cobertura de todas as variantes ou catch-all (wildcard/ident) na última cláusula. Tipos infinitos (`Int`, `Float`, `Text`, tipos opacos) exigem catch-all obrigatório.
- **Sobreposição:** Erro de compilação. Se a cláusula B é sombreada por uma cláusula A anterior (A aceita todo valor que B aceita), B é redundante → `RedundantClause`.
- **Relações:** Diferente de `match` (que avalia um scrutinee computado) — cláusulas despacham diretamente nos parâmetros. Diferente de Guards (que testam condições booleanas) — cláusulas casam padrões estruturais.

### `import`

```kata
import utilidades.matematica                 # import de módulo inteiro
import utilidades.matematica as mat           # com alias
import utilidades.(matematica TipoX IFACE)    # import seletivo de itens específicos (parênteses obrigatórios)
```

### `export`

```kata
export + - TipoX                              # itens separados por espaço, vírgulas opcionais
export tipos.(Int Float Boolean)              # reexportação: MOD.(itens) — parênteses só para o grupo
```

Sem `()` externo — itens são separados por espaço. `()` só é usado para reexportar itens de um sub-módulo (`export MOD.(itens)`).

---

## Convenções de Casing (Enforced)

O parser valida a capitalização de todos os nomes no momento do parse. A violação constitui erro fatal de compilação (`parse.invalid_casing`).

| Padrão | Categoria | Exemplos |
|---|---|---|
| **PascalCase** | Tipos (`data`), Enums, Enum variants, Refined types, Alias targets, Type params de struct/enum/interface | `Pessoa`, `Boolean`, `True`, `PositiveInt`, `Text`, `A` |
| **snake_case** | Funções (`sig`), Actions, Variáveis (`let`, lambda params), Campos de struct, Parâmetros de action, Nomes de método em interface/implements/refines | `soma_valores`, `main`, `nome`, `x`, `_print` |
| **ALL_CAPS** | Interfaces, Supertraits | `NUM`, `ORD`, `SHOW`, `HASH` |

### Regras

1. **Nomes simbólicos** (`+`, `-`, `*`, `<`, `>`, `=`) não são validados — são operadores, não identificadores alfabéticos.
2. **Prefixo `_`** é aceito em snake_case (convenção para internals/builtins: `_print`, `_println`).
3. **Prelude** não é isento — todo código Kata5 segue as mesmas regras.
4. **Type params de interface** seguem PascalCase (aceitam tanto single-letter `A`, `K` quanto nomes de tipos concretos `Text`).
5. **Mensagens de erro** em Português: `nome "X" deve be PascalCase, mas está em snake_case`.

---

## Palavras-Chave de Ação (somente Actions)

| Palavra | Uso |
|---|---|
| `loop` | Laço infinito |
| `for` | Iteração sobre coleção |
| `break` | Sai do laço |
| `continue` | Próxima iteração |
| `select` | Multiplexação de canais |
| `timeout` | Cláusula do `select` |
| `var` | Binding mutável |
| `return` | Early return em Actions. Não existe em funções puras. |

