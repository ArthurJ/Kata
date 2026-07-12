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

- **Relações**:
  - `Result::(T, E)` vs `Result::Ok`: Não são operações distintas — é sempre `Nome :: Algo`. Parâmetros de tipo naturalmente formam tupla `(T, E)`; variantes naturalmente são identificadores `Ok`. O `::` é o mesmo.
  - A qualificação `Enum::Variante` é **sempre válida**. `Result::Ok`, `Optional::Some`, `Boolean::True` existem. Só não são obrigatórios quando a variante já está disponível no escopo (ex: as do prelude vêm importadas automaticamente).
  - `Optional::T` é análogo a `Result::(T, E)`: `enum Optional { Some(T), None }`.
  - `Boolean` é `enum Boolean { True, False }` — variantes unitárias, sem payload.
  - `::` em assinatura é declaração; `->` é descrição de tipo de função como valor.
  - **Ascription de expressão** é pós-fixada: `expr::Type` etiqueta a expressão à esquerda com o tipo à direita. Para tipos refined com literal (`5::PositiveInt`), o compilador valida predicados em compile-time e entrega o tipo refined direto (sem `Result`). Para tipos base (`x::Int`), verifica e rebaixa ao tipo anotado. É o mesmo `::` dos outros contextos, agora em posição de expressão.

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
  compile-time (coerção contextual). A declaração de `enum` **não** usa `|` —
  variantes são listadas por indentação.

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
- **Relações**: `fork!` submete Action a corrotina no scheduler M:N. `select` multiplexa canais. Escape Analysis rastreia dados enviados por `!>` para alocação heap/`Arc<T>`.

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

---

## Delimitadores de Coleções

| Sintaxe | Tipo | Layout |
|---|---|---|
| `[1 2 3]` | Lista persistente | Encadeada (Cons). Pattern: `[h : t]` (cabeça : cauda). `[]` para lista vazia. |
| `{1 2 3}` | Array contíguo | Bloco contíguo (imutável por padrão). |
| `{1; 2; 3}` | Tensor N-D | Dimensões separadas por `;` |
| `(1, 2, 3)` | Tupla | Agrupamento heterogêneo. `(42,)` é tupla de 1 elemento (vírgula obrigatória). `(42)` é agrupamento, não tupla. `()` é `Unit`. |

- **Ranges (Lazy)**: `[0..10]` (0 a 9), `[0..=9]` (0 a 9 incluso), `[0..2..10]` (step 2: 0, 2, 4, 6, 8). Geram `Range` lazy que implementa `ITERABLE`. O step é definido por um segundo `..`: `start..step..end`.

---

## Palavras-Chave de Estrutura

| Palavra | Uso |
|---|---|
| `lambda` / `λ` | Declara função anônima. Múltiplas cláusulas após assinatura: `lambda <padrões>: <corpo>` — a primeira que encaixa vence |
| `action` | Declara Action |
| `data` | Declara tipo produto |
| `enum` | Declara tipo soma |
| `alias` | Cria Newtype |
| `interface` | Declara contrato de tipo |
| `implements` | Implementa interface (ex: `T implements IFACE`) |
| `import` | Importa módulo |
| `export` | Exporta itens |
| `as` | Alias de import (`import x as y`) ou de tipo (`data (...) as Nome`, `alias T as Nome`) |
| `with` | Bloco bottom-up ao final de lambda: computações prévias nomeadas para Guards, e restrições de genéricos |
| `match` | Pattern matching disponível em ambos os domínios (funções puras e Actions). `otherwise` é obrigatório quando o compilador não consegue provar estaticamente que os braços cobrem todas as variantes possíveis do tipo inspecionado. Em funções puras, cada braço deve retornar um valor (expressão); em Actions, braços podem ser statements. |
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

Sem `()` externo — itens são separados por espaço. `()` só é usado para reexportar itens de um submódulo (`export MOD.(itens)`).

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

