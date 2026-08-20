# Adendo — Rationale de Design

Kata é a iteração 5 de um processo de desenvolvimento — mas é a primeira versão pública, então não há comparações com versões anteriores. As decisões de design abaixo não são a spec da linguagem; são registro de *por que* certas escolhas foram feitas. A spec descreve *o que* a linguagem faz; este adendo explica *o porquê*.

## Por que não existe `if`?

`match`, cláusulas `lambda` e guards cobrem os casos em que `if-else` seria usado. Não temos as palavras `if`/`else`, mas a semântica correspondente está presente na linguagem.

A exaustividade do `match` é um segundo motivo. Se `if` existisse, o `else` teria que ser obrigatório para preservar exaustividade — um incômodo desnecessário dado que `match` já lida com isso de forma mais estruturada.

Por último, um dos objetivos da linguagem é um código menos aninhado. `if-else` incentiva aninhamento. Não sabemos ainda se alcançamos um resultado satisfatório nesse frente — só o uso dirá.

## Por que funções não podem executar actions?

Uma forte inspiração inicial foram linguagens funcionais, mas em geral não encontrei exemplos satisfatórios de separação entre os mundos puro e impuro. Gosto muito de Haskell, por exemplo, mas não consegui entender monadas a ponto de explicar para outra pessoa.

A separação explícita entre funções puras e actions tem vantagens concretas: manutenção e testagem da lógica do programa ficam mais fáceis, e as possibilidades de otimização das funções puras são uma vantagem real.

## Por que `Result` é um enum normal e `|` é definido sobre qualquer enum?

Um dos meus objetivos é separar operadores da linguagem de funções. Não tive 100% de sucesso nesse ponto, mas procurei implementar a linguagem nela própria sempre que possível (*eat your own dog food*). O sistema de enums me pareceu bom o bastante para suportar `Result`, `Optional` e `Boolean` sem tratamento especial.

Oferecer o `|` (fallback) para todos os enums me pareceu uma oportunidade excelente de tornar enums mais úteis e a linguagem menos verbosa e mais prática. Special-casing `Result` daria a ele um privilégio que não há motivo para existir.

## Por que `var` só existe em actions?

A imutabilidade da linguagem é relevante por diversos motivos, mas entendo que às vezes é necessário ou útil permitir mutação. As actions são o espaço seguro em que isso faz sentido — elas servem para modelar comportamento, e isso demanda mutação.

No top-level, `var` seria um convite a bugs. Em funções puras, seria um risco à pureza — uma determinada entrada deve sempre produzir a mesma saída, sem efeitos colaterais.

Além disso, embora a linguagem não force, espero que as cláusulas `lambda` sejam em geral one-liners, ou usem guards, ou matches. Em todos esses casos, não cabe `var`.

## Por que canais em vez de async/await?

A inspiração inicial foi o sistema `async` de Python — um modelo que considero simples e elegante. Canais em Kata não são todos síncronos: rendezvous bloqueia até o receptor sincronizar, mas queues bufferizadas com backpressure e broadcasts fire-and-forget são assíncronos. O que é síncrono é o *scheduler*: single-threaded, cooperativo, com fibers (wasmtime-fiber, 1MB stack cada). `send`/`recv` que não podem completar suspendem o fiber; o scheduler faz um wake pass verificando disponibilidade sem consumir, e acorda o fiber quando pode prosseguir. Deadlock é detectado quando todos os fibers estão blocked sem progresso possível.

O `select` (multiplexação) combina channels, file handles e sockets numa única suspensão atômica, com timeout opcional. Concorrência interprocess é suportada via fork + pipes Unix (canais IPC), e o `select` já está preparado para threads no futuro. A escolha por canais em vez de async/await evita a complexidade de colorir funções como async ou sync, e mantém o raciocínio sobre concorrência no nível das operações de canal, não no nível de cada chamada.

## Por que refined types não são novos tipos runtime?

Refined types são aliases com predicados validados em compile-time. Em runtime, um `PositiveInt` é literalmente um `Int` — mesmos bits, mesmo Cranelift type, sem wrapping, sem tag, sem overhead. A validação dos predicados acontece no smart constructor (que retorna `Result`) ou em ascriptions de literais (validadas em compile-time, sem custo runtime). A decisão de não criar novos tipos runtime evita o custo de boxing/unboxing e mantém o codegen simples — o compilador só precisa resolver a cadeia de aliases até o primitivo base.

O trade-off é que não há type safety em runtime: se você escapar do sistema de tipos (e.g., via FFI), nada impede que um `Int` negativo seja tratado como `PositiveInt`. Isso é aceitável porque a validação está nos construtores, que são a única forma de construir valores refined dentro da linguagem.

## Por que shadowing é proibido?

Menor superfície de erro no código Kata, e maior facilidade na implementação do compilador.

## Por que NUM e ORD são typeclasses separadas?

Nem todo número é ordenável — `Complex` é o exemplo. Não faz sentido semântico forçar ambos juntos.

## Por que notação prefixa?

A notação prefixa elimina a necessidade de regras de precedência entre operadores. Em notação infixa, o compilador precisa saber que `*` vem antes de `+`, que parênteses agrupam, e assim por diante — uma tabela de precedência que cresce com cada novo operador. Em notação prefixa, a aplicação de função é sempre "callee seguido de argumentos", seja o callee `+` ou `soma_valores`. O parser não trata operadores aritméticos de maneira especial: `+`, `-`, `*`, `<`, `>`, `=` são todos identificadores comuns, lexados e despachados como qualquer outro nome de função.

Isso também simplifica a implementação do compilador. Sem precedência, o parser é mais direto — não há ambiguidade a resolver. E visualmente, quando o código é quebrado em pedaços pequenos o suficiente, a notação prefixa é menos densa porque dispensa organizadores (parênteses, vírgulas) que a notação infixa exige para desambiguar.

## Por que arenas em vez de garbage collector?

Kata não tem tracing GC nem borrow checker. A gestão de memória é por arenas: cada fiber tem sua própria arena (bump allocator), onde dados locais são alocados em O(1) e liberados em O(1) quando o fiber termina — não há deallocação individual. Dados que precisam sobreviver ao fiber (valores retornados ao caller, valores enviados por canais) são alocados na arena do caller ou na root arena, esta última com deallocação individual via reference counting para closures com captura.

O compilador faz escape analysis para determinar onde alocar cada valor: local ao fiber, escapando para o caller, ou escapando para outro fiber via canal. Essa seleção é estática, decidida em compile-time. O resultado é que dados puramente locais — a esmagadora maioria dos casos em funções puras — têm máxima localidade de cache e zero overhead atômico, sem que o programador precise pensar em ownership.

O modelo funciona porque o scheduler é structured concurrency: um fiber só é destruído quando completa *e* todos os seus filhos completaram. Isso garante que a caller_arena do pai está viva quando um filho retorna um valor ou envia por canal, e que irmãos que compartilham a arena do pai trocam valores válidos. Sem essa invariante, o modelo quebraria — valores enviados por canal poderiam ser use-after-free se o fiber sender morresse antes do receiver consumir.

## Por que não existe try/catch?

Exceções invisíveis quebram a pureza funcional. Uma função que pode disparar uma exceção a qualquer momento, sem declarar, não é realmente pura — o caller não sabe que está sujeito a um desvio de fluxo não-expresso na assinatura. A ausência de try/catch força o tratamento explícito de falhas no sistema de tipos: operações falíveis retornam `Result`, e o compilador exige que o programador lide com ambos os ramos. O operador `?` em actions e o operador `|` em funções puras são os mecanismos de propagação — mas a falha é sempre visível na assinatura.

## Por que sem reflexão dinâmica?

A proibição de reflexão dinâmica — invocação por string, `eval`, dispatch baseado em nome de função em runtime — permite que o compilador construa um grafo de chamadas completo e determinístico a partir do entry point. Qualquer função, tipo, interface ou implementação que não está no grafo de dependências é código morto, extirpado pelo tree-shaker antes do codegen. O binário final não carrega stdlib não usada.

Kata tem dois mecanismos de introspecção, ambos compile-time. `type!()` consulta o tipo de uma expressão, retornando o nome nominal como `Text` — resolvido no monomorphizador a partir do tipo estático, sem aresta no call graph. Variáveis de reflexão (`_name`, `_arity`, `_types`, `_return_type`, `_is_action`) são disponibilizadas no body de diretivas e expõem metadados da função decorada; as estáticas são substituídas por literais no desugaring, e as dinâmicas (`_args`, `_return`) são sintetizadas a partir dos parâmetros e do retorno da função. Nenhum dos dois mecanismos consulta runtime, invoca por string, ou interfere no tree-shaking.

## Por que convenções de nomenclatura obrigatórias?

O lexer e o parser usam a capitalização de identificadores para desambiguação. A convenção não é estilística — é estrutural. O parser precisa distinguir um nome de tipo de um nome de função em posições ambíguas, e a capitalização resolve isso sem anotações redundantes. A violação constitui erro fatal de compilação, não aviso.

## Por que o compilador não tem builtins?

O princípio "sem builtins" significa que aritmética, comparação, strings, coleções e I/O são todos definidos na stdlib em código Kata via `@ffi` — não há tratamento especial para `+`, `-`, `<`, `=` no parser, typeck, ou codegen. São identificadores comuns apontando para funções em `kata-rt`. O compilador conhece apenas o catálogo de símbolos FFI e as strings de mapeamento de representação (`"i64"`, `"f64"`, `"kata_rt_string"`).

Não tivemos 100% de sucesso nesse ponto. `map`, `filter` e `fold` são interceptadas pelo typeck antes do dispatch normal — o compilador as reconhece por nome, extrai o tipo do elemento da coleção, infere o callback com hint, e produz nós TAST dedicados. Isso é necessário para stream fusion e para desugarar operadores standalone como callbacks (`map + [1 2 3]` precisa transformar `+` em lambda sintético). É o ponto onde o compilador ainda conhece nomes específicos da linguagem.