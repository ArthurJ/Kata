# Aprendizados da Iteração 5 — Kata5

Kata5 é a quinta iteração da linguagem Kata. Este documento consolida a
arquitetura da linguagem (propriedades e consequências), os princípios de
design, e as lições que emergiram do desenvolvimento.

## Arquitetura — Propriedades e Consequências

Cada propriedade é uma característica estrutural da linguagem. As
consequências derivam dela — tanto ganhos quanto custos.

### Sintaxe

#### Propriedade: Aplicação prefix-only greedy

`f a b` é `f(a, b)`, não `(f(a))(b)`. Não há Pratt, não há precedência de
operadores.

Consequências:
- Parser é recursive-descent puro — uma regra de apply serve para tudo
- `+ 1 2` é `Apply(Ident("+"), [1, 2])` — `+` é função, não operador
- Currying é explícito via Hole (`+ 10 _`), não automático via aplicação parcial
- Sem ambiguidade de precedência: `+ 1 * 2 3` é erro, precisa `+ 1 (* 2 3)`
- Custo: sintaxe menos familiar para usuários de linguagens infixas

#### Propriedade: Indent-sensitive

Blocos são delimitados por indentação, não chaves. O lexer emite
INDENT/DEDENT sintéticos.

Consequências:
- Sem chaves ruído visual — código é mais limpo
- Estrutura visual refleta estrutura lógica
- Custo: tabs vs spaces é decisão obrigatória desde o lexer

#### Propriedade: Parênteses com vírgula = tupla; sem vírgula = agrupamento

`(1, 2, 3)` é tupla. `(+ 1 2)` é agrupamento. A presença de vírgula
disambigua.

Consequências:
- Tupla e agrupamento não são ambíguos — uma regra de parsing cada
- `(42)` é agrupamento redundante, `(42,)` é tupla de um elemento
- Custo: tupla de um elemento exige vírgula trailing

### Sistema de tipos

#### Propriedade: Sem primitivos

`Int`, `Float`, `Text` são `data` opacos com `@ffi` no prelude. `Boolean`
é `enum` no prelude. `PrimTy` é mapeamento de representação FFI (`i64`,
`f64`, `kata_rt_string`), não tipo da linguagem.

Consequências:
- O compilador não tem tipos primitivos — tudo é `data` ou `enum`
- `Complex` funciona sem `@ffi` e sem tratamento especial — é `data` como
  qualquer outro
- O compilador conhece apenas `FfiSymbol`, mapeamento de representação, e
  `@builtin`
- Custo: o prelude precisa declarar todo tipo base explicitamente

#### Propriedade: Monomorphização nos call sites

`List::Int` e `List::Text` são código diferente — cada instância de tipo
genérico é especializada. `mono_instance: u64` na TAST rastreia a instância.

Consequências:
- Não há boxing de genéricos, não há vtable lookup
- Otimizações (stream fusion, TCO, inlining) operam sobre código concreto
- Custo: tempo de compilação e tamanho de código crescem com o número de
  instâncias

#### Propriedade: Dispatch por dominância — Score 2D

O `DispatchTable` resolve sobrecargas por pontuação, não por primeira
correspondência. Cada par (arg, param) é classificado em duas categorias:
`exact` > `iface`, com tiebreak concreto > genérico. Alias→base e
refined→base **não** são dimensões do Score — são resolvidos por fallback
em `apply_dispatch.rs` quando o dispatch normal falha.

Consequências:
- Múltiplas sobrecargas coexistem sem ambiguidade na maioria dos casos
- O scoring nasce em Fio 1 mesmo com 1 overload — não retrofit em Fio 7
- Empate final é `AmbiguousDispatch` — erro, não chute
- Custo: o algoritmo de seleção é mais complexo que lookup simples

#### Propriedade: `::` é um operador com cinco contextos

O parser reconhece `::` desde Fio 1 (tabela postfix). Os contextos
(assinatura, campo, type param, variante, ascription de expressão) são
interpretados progressivamente pelo typeck.

Consequências:
- O parser não cresce entre Fio 1 e Fio 6 — só o typeck ganha interpretações
- Adição de contextos é incremental no typeck, não no parser
- Custo: o typeck precisa distinguir contexto — mais lógica condicional

#### Propriedade: Refined types via fallback no dispatch

`refines` não cria overloads no DispatchTable nem registra no
InterfaceRegistry. O typeck faz fallback: substitui refined por base,
retenta dispatch, envolve retorno em construtor falível se o retorno
implementa a interface.

Consequências:
- DispatchTable não é poluído com overloads sintetizados
- PositiveInt não é formalmente NUM no InterfaceRegistry — sem mentira no
  type system
- Atrito nominal entre refineds distintos é preservado
- Custo: o fallback é semântica substancial embutida no typeck

#### Propriedade: T? é açúcar puro sem subtyping

`T?` desaçuca para `Result::(T, Err)`. Não cria subtipo, não cria Ok
implícito, não muda o operador `?` de runtime.

Consequências:
- `Int` não satisfaz `=> Int?` sem wrap explícito
- Sem polimorfismo via interface para refined types
- Custo: o usuário desempacota explicitamente

### Memória

#### Propriedade: Arena per-fiber com bumpalo

Cada fiber tem sua arena (bumpalo: alloc O(1), reset O(1), sem dealloc
individual). Destruição bottom-up: arena do pai sobrevive até filhos
terminarem.

Consequências:
- Fast path é extremamente rápido — bump allocation é um pointer increment
- Liberação O(1) ao término da Action — não há GC pause
- Custo: sem dealloc individual na fiber arena — objetos que precisam
  sobreviver ao fiber precisam ser promovidos (ARC na root arena)

#### Propriedade: Root arena com `std::alloc` + tracking para ARC

Valores que cruzam fronteiras entre arenas (canais, closures escapadas) são
alocados na root arena com ARC. Header de 24 bytes (fn_ptr + refcount +
n_captures). `decref → 0` chama `std::alloc::dealloc`.

Consequências:
- Deallocation individual é possível na root arena — sem vazamento
- Tracking com `Vec<(*mut u8, Layout)>` e `swap_remove` para dealloc O(1)
- Custo: fragmentação em long-running (evolução futura: size-class pool)

#### Propriedade: Structured concurrency — árvore de fibers

`fork!` cria fibers filhos. Um fiber só é destruído quando `completed &&
children.is_empty()`. Destruição é bottom-up.

Consequências:
- Qualquer objeto promovido para a arena do pai está vivo enquanto algum
  filho o referencia
- Deadlock detection é trivial no run loop
- Canais sempre descem (via `fork!` args), nunca sobem (proibido em retorno)
- Custo: fibers long-lived (servers, event loops) acumulam memória —
  reclamation granular é necessária (fora do escopo 1.0)

#### Propriedade: Canais descem, nunca sobem

`Ty::Sender`, `Ty::Receiver`, `Ty::ReceiverFactory` são proibidos no tipo
de retorno de Actions. A invariante é enforced em compile-time.

Consequências:
- Canais vivem na fiber_arena do criador — `bump.reset()` no epílogo libera
- Não há vazamento de canais na root arena
- A invariante é consequência direta da estrutura de árvore, não regra
  arbitrária
- Custo: Actions não podem retornar canais — pattern de factory em Action
  precisa usar args, não retorno

### Concorrência

#### Propriedade: CSP com fibers cooperativas

Concorrência via fibers (wasmtime-fiber), não threads OS. Canais
(rendezvous, buffered, broadcast) sincronizam. Yield cooperativo.

Consequências:
- Single-threaded determinístico — sem data races
- Composable: `fork!` + canais é estruturado
- Custo: head-of-line blocking sem yield points — resolvido com back-edge
  checks no codegen

#### Propriedade: Yield points no codegen

O codegen injeta checks em back-edges de `Loop`/`ForIn` a cada N iterações.
O check pergunta ao scheduler se há fibers prontos.

Consequências:
- Head-of-line blocking mitigado — fiber em loop infinito cede
- Timeout cooperativo via thread OS + AtomicBool (Fio 14)
- Custo: overhead de um check por iteração de loop
- Custo: não preemptivo — recursão pura e I/O sem loop não cobertos

#### Propriedade: Single-threaded com exceção do timeout timer

Scheduler, arenas, TLS são acessados por uma thread. A única exceção é a
thread OS do timeout que escreve num `AtomicBool` isolado.

Consequências:
- Modelo de memória simples — sem locks no scheduler
- A thread timer não toca scheduler, arenas, nem TLS
- Custo: `kata_rt_run` não é concorrente — testes devem ser serializados

### Codegen

#### Propriedade: Cranelift JIT com TCO delegado

A TAST carrega `tail_pos: bool`. O codegen emite `call` em posição de
cauda com `CallConv::Tail`. Cranelift decide se a otimização é viável.

Consequências:
- Recursão de cauda não estoura stack — Cranelift otimiza
- O compilador não implementa TCO — apenas marca
- Custo: dependência do Cranelift para a otimização funcionar

#### Propriedade: TRMA via reescrita no TAST

`@associative(0)` habilita reescrita com acumulador que converte recursão
bloqueada em recursão de cauda.

Consequências:
- Fatorial com `@associative(0)` vira recursão de cauda mesmo sem o
  usuário reescrever
- Custo: só funciona com operadores associativos e single-clause
  (multi-clause não otimiza — TECH-DEBT)

#### Propriedade: Stream fusion via `@builtin`

`map`/`filter`/`fold` com `@builtin` são interceptados no typeck para
gerar nós TAST estruturados. Cadeias `map.filter.fold` fundem num único
loop.

Consequências:
- Cadeias de HOF não alocam coleções intermediárias
- Custo: três nomes hardcoded no typeck (única exceção ao princípio
  "sem builtins" — aceita por pragmatismo)

#### Propriedade: AOT com `cranelift-object` + linker

`kata build` emite object file via `cranelift-object`, linka com `kata-rt`
estático ou dinâmico. Tree shaking incondicional (sem `--release`).

Consequências:
- Executável nativo sem o compilador
- Tree shaking remove `@test` e código morto em produção
- Custo: ABI de link é específica por plataforma

## Princípios

### 1. Três reinos: Data, Funções, Actions

Data, Funções e Actions são reinos separados com restrições distintas.

- **Data** é sempre imutável, serializável, não-transiente. Vive em arenas
  ou na heap via ARC. Não contém comportamento — Actions são proibidas em
  campos de `data`.
- **Funções** são puras — não geram efeitos colaterais, não acessam dados
  mutáveis, não invocam Actions. Recebem `@commutative`, `@associative`,
  `@builtin`. Despacham via DispatchTable.
- **Actions** podem gerar efeitos colaterais. Têm `var`, `loop`, `return`,
  `?`, `!`. São executadas em fibers com arena própria. Podem ser
  first-class (`Ty::Action`), invocadas indiretamente, passadas como
  parâmetro para outras Actions, mas não entram em `data` nem em canais.

A barreira é sintática — o typeck bifurca por domínio desde o início. O
enum `Effect` tentou classificar expressões por efeito e foi removido
(TECH-DEBT #1): a distinção real é estrutural (`ret_ty: Some` vs `None`),
não por enum de efeito.

### 2. Canais são controle, não dados

Canais (`Sender`, `Receiver`, `ReceiverFactory`) são estruturas de
controle de CSP, não valores de dados. Sempre descem na árvore de fibers
(passados como argumento via `fork!`), nunca sobem (proibidos em tipo de
retorno de Actions). A invariante é enforced em compile-time
(`contains_channel_type` no inference).

A justificativa é estrutural: os três caminhos de invocação de Actions
tornam o retorno de canal ou impossível (`fork!` retorna `Unit`) ou sem
sentido (entry point retorna `i64` para o host). A proibição formaliza o
que a arquitetura já impõe.

### 3. Memória por arena hierárquica

Cada fiber tem sua arena (bumpalo: alloc O(1), reset O(1), sem dealloc
individual). Quando um fiber termina e não há fibers filhas, a arena é
destruída com toda a memória gravada nela. Destruição é bottom-up: a
arena do pai sobrevive até todos os filhos terminarem
(`completed && children.is_empty()`).

O scheduler rastreia a árvore de fibers (parent_id, children, completed).
Structured concurrency garante que qualquer objeto promovido para a arena
do pai está vivo enquanto algum filho o referencia.

### 4. ARC na heap para valores que cruzam fronteiras

Estruturas de dados que cruzam fronteiras entre arenas (canais, closures
escapadas, captures) são alocadas na root arena com ARC (reference
counting). O epílogo da fiber que decrementa o ARC para zero é
responsável por remover o objeto da memória.

Fiber arenas continuam com bumpalo — o fast path (tuplas locais,
bindings, temporários) não muda. Root arena usa `std::alloc` + tracking
com `Vec<(*mut u8, Layout)>` e `swap_remove` para dealloc O(1). Header
ARC de 24 bytes (fn_ptr + refcount + n_captures) permite que `decref → 0`
saiba o tamanho do bloco para passar ao `dealloc`.

### 5. Dispatch por dominância — Score 2D

O `DispatchTable` resolve sobrecargas por pontuação, não por primeira
correspondência. Cada par (argumento, parâmetro) é classificado em uma de
**duas** categorias mutuamente exclusivas:

```
Score = (exact, iface, is_generic_origin)
```

- **exact**: tipo do argumento é idêntico ao tipo do parâmetro (`Int` vs `Int`)
- **iface**: parâmetro é interface e argumento a implementa (`Int` implementa `NUM`)

Ordenação lexicográfica decrescente: mais `exact` vence; empate desempata
por `iface`; empate total desempata por `is_generic_origin` (concreto vence
genérico). Empate final é `AmbiguousDispatch` — erro, não chute.

Alias→base e refined→base **não** são dimensões do Score. O scoring
original era 4D (`exact, alias, refined, iface`), mas as dimensões
`alias` e `refined` eram sempre 0 — o mecanismo de fallback em
`apply_dispatch.rs` já resolve refined→base e alias→base sem scoring.
Foram removidas em 2026-08-20 (commits `ed4ea22` e `d2686fa`). O Score
passou de 4D → 3D → 2D. Alias puro (sem `refines`) é nominalmente
distinto do base e não interoperaciona sem downcast explícito — por design.

### 6. Tuplas fora do grupo de collections

Tuplas são heterogêneas — `(Text, Int, Float)` tem elementos de tipos
diferentes, tamanho estático conhecido em compile-time. Por isso não
implementam `ITERABLE` (que é homogênea) e ficam fora do grupo de
collections (List, Array, Range, Dict, Set). O acesso é por índice
compile-time com bounds check (`.0`, `.1`, `.(-1)`).

`len` de tupla é síntese compile-time — não despacha `COUNTABLE`, o
typeck sabe o tamanho. `.N` em tupla é IndexAccess compile-time com
bounds check, não desugar para `at` via `INDEXABLE`.

### 7. Currying via Hole é o mecanismo unificador de abstração

O `_` em posição de argumento (`+ 10 _`) vira uma lambda com captures no
typeck — a TAST nunca contém `Hole`. O mesmo mecanismo serve para currying
de aplicação, predicados de refined types, predicados de enum, e
posicionamento de `|>`. Não é um recurso de funções — é um mecanismo de
tipo que opera em qualquer posição de argumento.

### 8. Ascription é um operador, não anotação pós-inferência

O parser reconhece `::` desde Fio 1 (tabela postfix). Os contextos
(assinatura, campo, type param, variante, ascription de expressão) são
interpretados progressivamente pelo typeck, não pelo parser. Três modos
semânticos:

1. **Rebaixamento de literal** — texto bruto reinterpretado no tipo alvo
   (`42::Float`, `3.14::Rational`). Sem conversão em runtime.
2. **Confirmação de tipo** — verifica que a expressão já tem o tipo alvo
   (`42::Int`). No-op em runtime.
3. **Ascription-construção** — promove tupla anônima a tipo nominal
   (`("João" 30)::Pessoa`). Valida shape e anexa `type_id`.

Para refined types, ascription de literal é validada em compile-time
(avaliação constante local ao typeck — não JIT-and-execute).
Ret-directed dispatch: a ascription propaga o tipo anotado como hint de
retorno para selecionar sobrecarga (`(/ 1 3)::Int` seleciona `idiv`).

### 9. `::` é uma operação, não declaração de tipo

`::` aparece em cinco contextos diferentes (assinatura, campo de struct,
type param de enum, qualificação de variante, ascription de expressão) e
o parser produz a mesma AST em todos. É o typeck que interpreta por
contexto. O parser não cresce entre Fio 1 e Fio 6 — só o typeck ganha
interpretações novas.

### 10. Operadores vs Funções — o compilador conhece operadores, não funções

**Operadores** são sintaxe hardcoded no parser com semântica fixa no
typeck. Não despacham, não têm overloads, não são redefiníveis:

- `::` — ascription/qualificação (cinco contextos, mesmo token)
- `!` — sufixo de chamada de Action
- `<!`, `!>` — send/receive de canal
- `|` — fallback/coalescência
- `?` — fail-fast (desugar para `return Err(e)`)
- `|>` — pipeline (desugar no typeck)
- `_` — hole (desugar para lambda no typeck)

**Funções** despacham via DispatchTable, têm overloads, são definíveis
pelo usuário, recebem diretivas. `+`, `-`, `*`, `/`, `=`, `<`, `>` — tudo
funções. O parser produz `Apply { callee: Ident("+"), args }` — não há
nó `Plus` na AST. São definidas no prelude via `@ffi`.

O compilador conhece a semântica de operadores (são parte da sintaxe).
O compilador não conhece a semântica de funções (tudo via DispatchTable).

### 11. Sem builtins implícitos — propriedades declaradas, não deduzidas

O compilador age apenas sobre propriedades explicitamente declaradas pelo
usuário via diretivas ou keywords. Não deduz que `+` é comutativo — o
usuário diz via `@commutative`. Não deduz que PositiveInt delega NUM — o
usuário diz via `refines`. As exceções aceitas são todas declarativas e
opt-in:

- `@commutative` — dispatch tenta args invertidos
- `@associative` — TRMA reescreve recursão com acumulador
- `@builtin("map"/"filter"/"fold")` — typeck intercepta para stream fusion
  (única exceção que conhece nomes específicos, aceita por pragmatismo)
- `refines` — typeck faz fallback no dispatch (substitui refined por base,
  retenta, envolve retorno em construtor falível se retorno implementa a
  interface)

`Int`, `Float`, `Text` são `data` opacos com `@ffi` no prelude. `Boolean`
é `enum` no prelude. `PrimTy` é mapeamento de representação FFI (`i64`,
`f64`, `kata_rt_string`) — não é tipo da linguagem. `Complex` é
implementado inteiramente em Kata sem `@ffi` e funciona com `+`, `show`,
comparação — o compilador não sabe que ele existe.

### 12. TCO delegado ao Cranelift, não implementado no compilador

A TAST carrega `tail_pos: bool` marcado pelo typeck. O codegen emite
`call` em posição de cauda com `CallConv::Tail` e
`preserve_frame_pointers`. Cranelift decide se a otimização é viável. TRMA
(`@associative(0)`) é a exceção — reescrita no TAST que converte recursão
bloqueada em recursão de cauda quando o operador é associativo.

### 13. Structured concurrency: fibers formam uma árvore, não um grafo

`fork!` cria fibers filhos. Um fiber só é destruído quando `completed &&
children.is_empty()`. Destruição é bottom-up. Deadlock detection é
trivial no run loop — se `run_queue` está vazia e `blocked` não está,
deadlock. A consequência é que canais sempre descem (via `fork!` args),
nunca sobem (proibido em retorno). A invariante "canais descem, nunca
sobem" não é uma regra arbitrária — é consequência direta da estrutura
de árvore.

### 14. Tagging SMI é transparente

O compilador vê tipo canônico, o runtime decide representação. Um `Int`
em Kata é `data Int ()` com `@ffi("i64")`. O `i64` no IR é ou um small
integer tagged inline ou um ponteiro para BigInt na heap — o compilador
não sabe, não precisa saber. O runtime faz tag/untag nas fronteiras FFI.
Isso separa o sistema de tipos da estratégia de representação.

### 15. `tail_pos` e `escape` são ortogonais

`tail_pos: bool` responde "esta chamada está em posição de cauda?" —
governa TCO. `escape: EscapeTarget` responde "para onde este valor
escapa?" — governa seleção de arena. Um valor pode estar em tail position
e escapar para heap (closure retornada em tail position). Um valor pode
não estar em tail position e não escapar (temporário local). O enum
`Effect` tentou confluenciar os dois e foi removido — a distinção real é
estrutural, e as decisões de TCO e escape são independentes.

### 16. Monomorphização nos call sites, não type erasure

`List::Int` e `List::Text` são código diferente — cada instância de tipo
genérico é especializada no ponto de chamada. `mono_instance: u64` na TAST
rastreia qual instância cada call resolve. Não há boxing de genéricos,
não há vtable lookup. O código é especializado como se o usuário tivesse
escrito à mão. O custo é tempo de compilação e tamanho de código; o ganho
é que otimizações (stream fusion, TCO, inlining) operam sobre código
concreto.

### 17. Tudo é aplicação de função

Construção de struct é apply posicional: `Pessoa "João" 30` é
`Apply { callee: Ident("Pessoa"), args }`. Não há sintaxe `{campo: valor}`.
Smart constructors são funções que despacham. `+ 1 2` é
`Apply { callee: Ident("+"), args }`. A única coisa que não é aplicação
de função são os operadores. Isso simplifica o parser (uma regra de apply
serve para tudo) e o typeck (uma regra de dispatch serve para tudo).

### 18. HAMT para estruturas de dados imutáveis

Dict e Set são Hash Array Mapped Tries — árvores prefixadas com
bitmap-indexed nodes. O(log₃₂ n) para lookup/insert, sharing estrutural
— inserção copia só o caminho da raiz à folha modificada, o resto é
compartilhado. Para n = 1000, são ~2 níveis.

A escolha conecta com o princípio de Data imutável: HAMT maximiza
sharing, duas versões de um Dict que diferem em uma chave compartilham
todos os outros nodes. Menos cópia, menos pressão no ARC, menos
fragmentação. Dict mantém ordem de inserção via overlay de Cons-list
(escolha de DX — o usuário espera ordem como Python 3.7+ e JS).

### 19. Tracer bullets acumulam débito horizontal — zeladoria é contínua

Cada fio é vertical (frontend → backend → runtime), mas corta camadas
horizontalmente. Arquivos crescem rápido. A iteração 5 abandonou o modelo
de zeladorias planejadas em marcos em favor de zeladoria contínua via
skill `zeladoria-kata5`. O débito horizontal do modelo vertical precisa
ser pago constantemente, não adiando para uma janela dedicada que cresce
o backlog.

## Lições

### L1. Fio 16 → 16b: não resolver problemas que a arquitetura já resolve

A Fase 6 do Fio 16 moveu canais da `fiber_arena` para a `root_arena` para
"garantir sobrevivência após fiber criador." Mas structured concurrency
já garante que o criador é always-last — a árvore de fibers não deixa o
pai morrer antes dos filhos. O problema não existia. Canais na root_arena
nunca eram dealocados individualmente (não são ARC-managed), então cada
canal vazava 48-72 bytes permanentemente.

A correção (16b) reverte e formaliza a invariante com proibição
compile-time de retorno de canais. Lição: antes de adicionar maquinaria
para um lifetime concern, verificar se o modelo existente já cobre aquele
caso. Structured concurrency cobre canais que descem; ARC cobre valores
que cruzam fibers. Canais não são valores que cruzam fibers — são
controle que descende.

### L2. Enum `Effect` — não confluenciar conceitos ortogonais

O enum `Effect` com 4 variants (`Puro`, `IO`, `Spawn`, `ChannelOp`) e o
campo `effect` de `TypedExpr` foram completamente removidos. Nenhum
variant era consumido — zero `==`, `!=`, ou `matches!` sobre o campo em
qualquer crate. Cerca de 80 referências em ~30 arquivos removidas. A
distinção Action vs função pura é estrutural (`ret_ty: Some` vs `None`),
não por efeito. `tail_pos` e `escape` são ortogonais e governam decisões
independentes. Lição: não criar enum classificatório antes de ter
consumidores reais para cada variant.

### L3. `EscapeTarget::Ancestor(n)` — não adicionar código forward-looking sem implementação

Era código morto projetado para LCA (lowest common ancestor) entre fibers
que compartilham canais, mas a implementação real do LCA nunca existiu.
Removido. Lição: não adicionar variantes/fields para features futuras
antes da feature existir — código morto é débito sem benefício.

### L4. Manual aspiracional vs implementação real

O manual descreve features que podem não existir. O ModuleLoader é
testado mas é dead code — não é chamado pelo driver. O `resolve()` ignora
`ImportDecl`/`ExportDecl`. A tensão é produtiva (o manual guia o
desenvolvimento) mas precisa ser reconhecida: o manual é aspiracional, a
implementação é a verdade. PRDs devem documentar o gap explicitamente
(como o PRD de módulos faz).

### L5. `@builtin` é a única exceção ao princípio "sem builtins"

`@builtin("map"/"filter"/"fold")` conhece três nomes específicos no
typeck para viabilizar stream fusion. Aceita como pragmatismo: sem a
interceptação, seriam chamadas de função opacas que o optimizer não
consegue fundir. A generalização futura seria um `@fusion_eligible` que
o typeck respeita para qualquer função — pós-1.0.

### L6. `channel!()` cria `Var("T0")` — resolvido via pass `cross_process.rs`

`channel!()` cria `Ty::Var("T0")` como tipo de elemento. O `type_compatible`
em `csp.rs` aceita `Var` como coringa (retorna `true` para qualquer tipo),
mas nunca unifica `T0` com o tipo concreto. Para `fork!`, isso é
mascarado pela Action ter parâmetros tipados (`tx::Sender::Int`) — dentro
da Action, o tipo é concreto. Para `spawn!` com canais IPC, o `type_id`
do canal ficava 0 (Prim) porque o tipo era `Var("T0")`, e a serialização de
tipos complexos não funcionava. Tipos primitivos (Int, Unit) funcionavam
porque SMI é inline (8 bytes, sem serialização recursiva).

**Solução implementada:** o pass `cross_process.rs` (pós-inferência) resolve
`ChannelCreate` na TAST substituindo `Var("T0")` pelo tipo concreto do
primeiro `<!`/`!>` via `resolve_channel_create`. O `spawn!` também unifica
`Var("T0")` no `TypeEnv` (mesmo mecanismo do `fork!`). `collect_module_types`
recursiva em `type_table.rs` garante que o codegen encontre o `type_id`
correto para serialização. Resultado: Int, tupla, struct e lista funcionam
com IPC.

A lição permanece válida: `type_compatible` com `Var` como coringa é um
atalho que funciona para dispatch mas não para codegen que precisa do
tipo concreto. A solução não é mudar `type_compatible`, mas adicionar um
pass separado que resolve `Var` para tipo concreto na TAST antes do codegen.

### L7. `kata-inference` faz três jobs numa só camada — separar na próxima iteração

`kata-inference` (16.4k LOC, 57 arquivos) intercala três concerns distintos
num único `infer_module`: **desugar** (pipe, hole, directives — 1.7k LOC),
**síntese** (show, smart constructors, log, timer — 2.8k LOC), e **type
checking** (inferência, dispatch, pattern checking, CSP, const — 12k LOC).
A ordem de descoberta acopla a ordem de processamento: a síntese de `show`
roda dentro de `infer_module` e muta o `InterfaceRegistry` que o typeck
consome; o desugar é chamado em 8 pontos diferentes, sempre imediatamente
antes de `infer_expr`, em vez de um pass global.

A separação em três camadas (`resolve → synthesize → infer`, cada uma com
input/output bem definido) é conceitualmente limpa e foi analisada no item
A4 do TODO. Não vale a pena executar na iteração 5 — o ROI é baixo: não
elimina código, apenas move; não reduz match arms no codegen; `?` e `|`
são context-dependent (exigem `InferCtx`) e não podem ir para o desugarer
puro. Mas a separação deve ser nativa na arquitetura da próxima iteração:
desugarer como pass global antes do typeck, synthesizer como camada que
produz `DispatchTable` + `InterfaceRegistry` + `Vec<TypedFunction>` antes
do type checker consumi-los, type checker que apenas consome sem sintetizar.