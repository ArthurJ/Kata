# PRD - Arrumação de Casa (Feature Complete Fases 1-6 & Corrotinas)

## 1. Visão Geral e Objetivo
Este PRD tem como objetivo consolidar o compilador Kata-lang, eliminando dívidas técnicas, *stubs* (simulações) e atalhos de implementação (a "mentalidade MVP") presentes no código atual. A principal mudança arquitetural desta fase é a substituição das *threads* bloqueantes do S.O. por um modelo de **Corrotinas Nativas no C-ABI** para a execução de `Actions`, garantindo concorrência massiva de I/O em cooperação com o *runtime* Tokio, conforme idealizado na filosofia original da linguagem.

Além disso, este PRD abrange as lacunas de arquitetura identificadas que impediam a linguagem de ser "Feature Complete", como vazamentos de memória por falta de ARC, ausência da infraestrutura de testes, e falhas no sistema de imports e multiplexação de canais.

O objetivo final é declarar as Fases 1 a 6 como estritamente completas e prontas para uso em produção.

---

## 2. Arquitetura de Execução: Corrotinas Nativas (O Fim do `spawn_blocking`)

**Problema Atual:** O `kata-rt` está executando o ponto de entrada (`kata_main`) e chamadas de canal usando `tokio::task::spawn_blocking`. Isso sequestra *threads* pesadas do Sistema Operacional, quebrando a promessa de *Green Threads* (M:N) ultraleves e destruindo a performance de concorrência.

**Solução:**
*   **Corrotinas Stackful (Context Switching):** O Cranelift/Codegen deixará de gerar código C-ABI linear bloqueante para *Actions*. Em vez disso, o `kata-rt` implementará ou integrará uma abstração de corrotinas em C/Rust (como manipulação de contexto em Assembly nativo ou geração de Máquinas de Estado explícitas no Cranelift).
*   **Yielding Cooperativo:** Sempre que uma *Action* chamar um FFI bloqueante (`<!` ou `sleep!`), a corrotina nativa fará o salvamento dos seus registradores e fará o *yield* (`Poll::Pending`) de volta para a *Task* do Tokio. Quando a mensagem chegar no canal, o Tokio acorda a *Task*, que restaura o contexto da corrotina e continua a execução nativa.

---

## 3. Melhorias no Frontend (Lexer & Parser)

- [x] **3.1. Correção de Sintaxe Faltante (Coerção e Coleções):**
    *   *Problema:* O parser falha em coerções explícitas e *namespaces* compostos (ex: `tensor_result::Tensor`), e em instâncias de expressões como `[pivo : resto]`.
    *   *Solução:* Ajustar a gramática de expressões (`expr_parser`) e de tipos para suportar `DoubleColon` na composição de caminhos de acesso. A sintaxe de *cons* de listas (`[pivo : resto]`) já foi suportada em `expr.rs`.
- [x] **3.2. Nova Sintaxe para Tipos Refinados:**
    *   *Problema:* A sintaxe de tipos refinados usa a estrutura `data PositiveInt as (Int, > _ 0)`. Semanticamente, a leitura declarativa não é ideal.
    *   *Solução:* Alterar a gramática para suportar a notação `data (Int, > _ 0) as PositiveInt`, que expressa melhor a ideia de que a restrição gera um novo tipo nomeado.
- [ ] **3.3. Tokens e Sintaxe para `select` e `timeout`:**
    *   *Problema:* A linguagem promete multiplexação de canais não-determinística, mas os tokens e as estruturas na AST não existem no Lexer e no Parser.
    *   *Solução:* Adicionar os tokens pertinentes e estender a gramática de `Stmt` para suportar o bloco imperativo `select` com ramos `case` e `timeout`.
- [ ] **3.4. Nova Assinatura para Tensores:**
    *   *Problema:* O formato atual de declaração de tensores não é rigoroso o suficiente quanto aos tipos numéricos e variabilidade de dimensões esperadas.
    *   *Solução:* Atualizar a gramática e a resolução de tipos para suportar e validar a assinatura no padrão `Tensor::(NUM, (Int...))`, especificando diretamente o tipo dos dados e as dimensões no tipo parametrizado.

---

## 4. Melhorias no Backend & Codegen (Cranelift)

Remover os atalhos e *hacks* do gerador de código de máquina:

- [ ] **4.1. Remoção de Tipagem Hardcoded (Payloads de Enum):**
    *   *Problema:* O `Match` de enums assume prematuramente que quase todos os *payloads* são castáveis para `I64` no MVP.
    *   *Solução:* Implementar a resolução dinâmica do *layout* de memória no `Match`, consultando o `TypeEnv` para gerar a instrução Cranelift correta (`F64`, Pointers complexos, Tuplas, etc.) ao extrair dados na posição `+8` do ponteiro.
- [ ] **4.2. Respeito ao Escape Analysis e Emissão de ARC:**
    *   *Problema:* A instanciação de variantes de Enum e closures ignora a flag `alloc_mode`. Pior ainda, o Codegen não emite instruções para libertar a memória compartilhada gerando Memory Leaks.
    *   *Solução:* Ler a propriedade `alloc_mode`. Se for `Shared`, invocar a alocação ARC. Mais importante: o compilador deve identificar os pontos em que o escopo de uma variável compartilhada termina e emitir chamadas de *RefCounting* (Incremento/Decremento) e limpeza (`Decref`).
- [ ] **4.3. Implementação Completa da TAST (Foco em Laços):**
    *   *Problema:* Nós cruciais falham com `panic!("... não suportada no TODO")` (ex: `Guard`, `Hole`, `ChannelSend`, `ChannelRecv`, e laços).
    *   *Solução:* Mapear e implementar a tradução Cranelift de todos os nós restantes. Especial atenção à implementação estrita de laços imperativos (`Loop`, `For`, `Break`, `Continue`), pois a recursão é proibida em `Actions`, tornando os laços a única forma de iteração no domínio impuro.

---

## 5. Otimizador e Análise Semântica (Middle-end)

Ajustar as análises para não deixarem "pontas soltas" que corrompam lógicas avançadas:

- [x] **5.1. Captura de Escopo Avançada (Closure Free Vars):**
    *   *(Resolvido)* O rastreio de variáveis capturadas por *closures* (`free_vars.rs`) foi expandido para suportar *patterns* de *Match Arms*.
- [x] **5.2. Type Checker & Prelude Exports:**
    *   *(Resolvido)* Corrigida a necessidade de exportar explicitamente operadores matemáticos nativos (`+`, `<`, etc) no `prelude.kata`.
- [ ] **5.4. Igualdade Estrutural de Tipos Refinados:**
    *   *Problema:* `types_equal_ignore_span` checa apenas se dois tipos refinados têm a mesma base e a mesma quantidade de predicados (verificação rasa).
    *   *Solução:* Implementar uma função de igualdade profunda de AST (`Expr`) para comparar matematicamente se os predicados de um `PositiveInt` são logicamente os mesmos de outro tipo antes de permitir a compatibilidade.
- [ ] **5.5. Limpeza Final no Tree-Shaker:**
    *   *Problema:* Tipos de dados, Enums e Interfaces não utilizados não estão sendo extirpados.
    *   *Solução:* Estender o `tree_shaker.rs` para rastrear o grafo de dependência de `Data` e `Enum`.
- [ ] **5.6. Refinamento de Generics (Scoring):**
    *   *Problema:* O compilador assume que "qualquer tipo com uma letra maiúscula" é Genérico.
    *   *Solução:* Usar o `TypeEnv` real para verificar *TypeVars*, e refinar o algoritmo de *scoring* do `ArityResolver` para diferenciar e desempatar assinaturas genéricas puras das resoluções via super-traits.
- [ ] **5.7. Memoização via `@cache_strategy`:**
    *   *Problema:* A diretiva de cache é validada sintaticamente mas ignorada no resto do pipeline.
    *   *Solução:* Criar uma passagem no otimizador que intercepte funções puras anotadas com `@cache_strategy` e injete verificações a uma *Hash Table* global gerenciada pelo `kata-rt`.

---

## 6. FFI, Standard Library e Kata-Runtime

- [ ] **6.1. Implementação dos Stubs C-ABI:**
    *   Substituir os retornos silenciados `NULL` no `linker.rs` por implementações reais na linguagem C (ou Rust injetada): `kata_rt_range_create`, `kata_rt_default_repr`, etc.
- [ ] **6.2. Canais Rendezvous Reais:**
    *   Em `csp.rs`, substituir o `mpsc::channel(1)` no `channel!()` por um canal de zero-buffer (sincronia perfeita).
- [ ] **6.3. Atomic Reference Counting (ARC) Real:**
    *   *Problema:* `SharedMemory::alloc` é apenas um `malloc` cego. Enviar dados por canais causará vazamento perpétuo.
    *   *Solução:* Implementar a estrutura atômica real de ARC no runtime, com contador de referências thread-safe e descarte seguro quando a contagem chegar a zero.
- [ ] **6.4. Multiplexação de Canais (`select` / `timeout`):**
    *   *Problema:* O modelo atual só aguarda um canal por vez (síncrono).
    *   *Solução:* Mapear e implementar os bindings no runtime (usando `tokio::select!`) para permitir espera concorrente em múltiplos canais simultaneamente.

---

## 7. Tooling e CLI (Finalização)

- [ ] **7.1. Comando `kata run` Completo:**
    *   Substituir os comentários no bloco `Commands::Run` de `main.rs` pela compilação temporária em `.tmp` seguida de execução imediata e descarte seguro (ou JIT).
- [ ] **7.2. Expurgo de Stubs:**
    *   Remover fisicamente `run_stub()`, `init_stub()` e `start_stub()` de todos os módulos.
- [ ] **7.3. Resolução Real de Imports no File System:**
    *   *Problema:* O compilador carrega a StdLib local estaticamente e não processa árvores de dependência reais para `import modulo.submodulo`.
    *   *Solução:* Implementar um mecanismo em `main.rs` (ou no `TypeChecker`) que vasculhe o disco (`src/**` e `mod.kata`) para resolver, parsear e anexar dependências ao `TypeEnv` dinamicamente.
- [ ] **7.4. Execução Efetiva do `kata test`:**
    *   *Problema:* O comando de testes atual apenas imprime na tela "Pronto para gerar Entrypoint", sem testar nada.
    *   *Solução:* Gerar dinamicamente um AST de entrypoint que invoque todas as funções anotadas com `@test`, passá-lo pelo pipeline (Codegen) e executar o binário reportando o resultado (Success/Fail) para o usuário.

---

## 8. Critérios de Aceite (Definition of Done)

- [ ] **Concorrência Real:** Um código testando a criação de 100.000 `fork!(...)` simultâneos que fazem `sleep!` deve rodar rapidamente num hardware normal, sem estourar o limite de *threads* do S.O.
- [ ] **Segurança de Memória (ARC Zero Leak):** Uma suíte de testes de estresse enviando milhares de `Enum` por canais não deve vazar RAM ao finalizar o programa (validado com Valgrind/Heaptrack).
- [x] **Captura Limpa:** *Closures* definidas dentro do ramo `Ok` de um bloco `match` devem capturar variáveis sem erro de runtime.
- [ ] **Multiplexação Ativa:** O comando `select` deverá escutar dois canais independentes e um timer (`timeout`) com resolução limpa no terminal.
- [ ] **Testes Funcionais:** O comando `kata test` invocado na raiz do projeto deve localizar arquivos, compilar, testar as lógicas puras (booleanas) e impuras (`assert!`), e sair com *Exit Code 0* ou *1* conforme os resultados.
- [ ] **CLI Operacional:** O comando `kata run test_concurrency.kata` deve funcionar perfeitamente de ponta a ponta.