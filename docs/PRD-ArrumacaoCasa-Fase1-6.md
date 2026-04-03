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
- [x] **3.3. Tokens e Sintaxe para `select` e `timeout`:**
    *   *Problema:* A linguagem promete multiplexação de canais não-determinística, mas os tokens e as estruturas na AST não existem no Lexer e no Parser.
    *   *Solução:* Adicionadas as estruturas sintáticas de `select`, `case` e `timeout` ponta-a-ponta na AST, TAST e Passes de Otimização.

- [x] **3.4. Nova Assinatura para Tensores:**
    *   *Problema:* O formato atual de declaração de tensores não é rigoroso o suficiente quanto aos tipos numéricos e variabilidade de dimensões esperadas.
    *   *Solução:* Atualizada a gramática e a resolução de tipos para suportar `Tensor::(NUM, (Int...))` (migração completa de Arrays abstratos transferida para a nova **Fase 6.5** no PRD-Fase6.5-Iterable.md).

---

## 4. Melhorias no Backend & Codegen (Cranelift)

Remover os atalhos e *hacks* do gerador de código de máquina:

- [x] **4.1. Remoção de Tipagem Hardcoded (Payloads de Enum):**
    *   *Problema:* O `Match` de enums assume prematuramente que quase todos os *payloads* são castáveis para `I64` no MVP.
    *   *Solução:* Implementar a resolução dinâmica do *layout* de memória no `Match`, consultando o `TypeEnv` para gerar a instrução Cranelift correta (`F64`, Pointers complexos, Tuplas, etc.) ao extrair dados na posição `+8` do ponteiro.
- [x] **4.2. Respeito ao Escape Analysis e Emissão de ARC (Compiler-Driven Drop):**
    *   *Problema:* A instanciação de estruturas ignora a flag `alloc_mode`. A `LocalArena` não tem como fazer limpeza dinâmica de ponteiros fugados, o que causaria Memory Leaks severos em laços imperativos longos (ex: Daemons).
    *   *Solução:* Implementar *Compiler-Driven Drop*. O otimizador de *Escape Analysis* deve não apenas marcar o `AllocMode::Shared` para variáveis que fogem do escopo (via canais), mas também rastrear o fim da vida útil dessas variáveis em seus respectivos blocos (fim de um `for`, `loop`, `match arm` ou `action`). O Codegen do Cranelift, ao sair desses blocos, injetará chamadas explícitas nativas para `kata_rt_decref`, garantindo que contadores ARC sejam reduzidos no exato momento em que perdem referência, liberando a memória da Heap Global em O(1) sem vazar.
- [x] **4.3. Implementação Completa da TAST (Foco em Laços):**
    *   *Problema:* Nós cruciais falham com `panic!("... não suportada no TODO")` (ex: `Guard`, `Hole`, `ChannelSend`, `ChannelRecv`, e laços).
    *   *Solução:* Mapear e implementar a tradução Cranelift de todos os nós restantes. Especial atenção à implementação estrita de laços imperativos (`Loop`, `For`, `Break`, `Continue`), pois a recursão é proibida em `Actions`, tornando os laços a única forma de iteração no domínio impuro.

---

## 5. Otimizador e Análise Semântica (Middle-end)

Ajustar as análises para não deixarem "pontas soltas" que corrompam lógicas avançadas:

- [x] **5.1. Captura de Escopo Avançada (Closure Free Vars):**
    *   *(Resolvido)* O rastreio de variáveis capturadas por *closures* (`free_vars.rs`) foi expandido para suportar *patterns* de *Match Arms*.
- [x] **5.2. Type Checker & Prelude Exports:**
    *   *(Resolvido)* Corrigida a necessidade de exportar explicitamente operadores matemáticos nativos (`+`, `<`, etc) no `prelude.kata`.
- [x] **5.3. Validação de Auto-Expansão de Exports (Clean Exports):**
    *   *Problema:* O arquivo `src/core/types.kata` possui uma lista gigante de exportações explícitas desnecessárias.
    *   *Solução:* A auto-expansão copia métodos nativos das interfaces/tipos ao reexportar.
- [x] **5.8. Conversão Implícita Genérica de `SHOW`:**
    *   *Problema:* O `echo!` quebra se receber literais matemáticos puros, forçando o usuário a escrever verbosamente `echo!(str 10)`. Um check hardcoded do nome "echo" feria a arquitetura da linguagem.
    *   *Solução:* A assinatura de `echo` em `io.kata` foi atualizada para exigir `SHOW...`. O TypeChecker (`ArityResolver`) agora intercepta *qualquer* parâmetro exigido como `SHOW` (em qualquer função) e injeta sinteticamente o *call* invisível ao `str` caso o argumento fornecido não seja primariamente um `Text`.
- [x] **5.9. Síntese de Construtores Inteligentes (Enum Predicativo):**
    *   *Problema:* Enums que usavam predicados lógicos (ex: `< _ 18.5`) criavam a assinatura vazia para a variante, causando link error em tempo de máquina. O compilador não criava a função de checagem de IFs para o Múltiplo Despacho.
    *   *Solução:* O Type Checker agora forja uma árvore sintática `TAST` de `LambdaDef` com um encadeamento de `Guards` e a injeta silenciosamente para avaliação dinâmica em Run-Time.
- [x] **5.4. Igualdade Estrutural e Tipagem Nominal-Estrutural de Tipos Refinados:**
    *   *Problema:* O Type Checker checava compatibilidade de forma rasa, ignorando os predicados, e extraía falhamente o payload de Enums em blocos `match`.
    *   *Solução:* Implementada igualdade profunda da AST (`exprs_equal`). Ajustada a regra de `types_compatible` para o "Meio-Termo" (Nominal nas fronteiras de funções para garantir intenção semântica, mas estrutural/interoperável com o tipo base para matemática). Corrigida a inferência do Payload em Pattern Matching (`MatchArm`).
- [x] **5.5. Limpeza Final no Tree-Shaker:**
    *   *Problema:* Tipos de dados, Enums e Interfaces não utilizados não estão sendo extirpados.
    *   *Solução:* O `tree_shaker.rs` foi estendido para rastrear o grafo de dependência profundo de `TypeRef` e `Expr`, removendo `Data` e `Enum` inativos da TAST.
- [x] **5.6. Refinamento de Generics (Scoring):**
    *   *Problema:* O compilador assume que "qualquer tipo com uma letra maiúscula" é Genérico.
    *   *Solução:* Introduzido o nó explícito `TypeRef::TypeVar` na AST desde o Parser. O algoritmo de Múltiplo Despacho agora pontua genéricos com base real nas restrições de Interface (`TypeEnv::constraints`), eliminando a adivinhação de strings.
- [x] **5.10. Re-resolução de Chamadas no Monomorfizador (Fix Cranelift Verifier Error):**
    *   *Problema:* O Monomorfizador (Fase 4) realiza apenas a substituição nominal de tipos (ex: troca a variável `A` por `Float` no corpo de funções genéricas e construtores sintéticos de Enums), mas **não re-avalia** as chamadas de funções internas (`TExpr::Call`). Isso faz com que operações polimórficas (como `< __val 18.5`) continuem apontando para a instrução de máquina do fallback deduzido na Fase 3 (ex: `lt_Int_Int`). Quando o Cranelift (Fase 6) recebe a variável substituída (`f64`) para executar numa instrução `i64`, ocorre um *Verifier Error*.
    *   *Solução Planejada:* Para manter a arquitetura limpa (DRY), o `ArityResolver` terá sua lógica de despacho extraída para que possa ser compartilhada. O `Monomorphizer` deverá usar essa lógica para re-avaliar/re-linkar estritamente os nós de `Call` dentro da árvore monomorfizada usando os novos tipos concretos (ex: substituindo a chamada de fallback `lt_Int_Int` pela `lt_Float_Float` recém resolvida). Isso corrige o Verifier Error no backend sem penalizar o tempo de execução.
- [x] **5.11. Igualdade Semântica de Enums Predicativos (Domain-Driven Equality):**
    *   *Problema:* Atualmente, a linguagem não define um comportamento claro de igualdade para instâncias de Enums que carregam *payloads* diferentes mas que caem na mesma variante lógica (ex: `IMC(18)` e `IMC(15)` caindo em `Magreza`). Compará-los via igualdade estrutural tradicional retornaria `False`, o que fere a semântica de domínio onde a "identidade da variante" deve ter precedência sobre o valor bruto medido.
    *   *Solução Planejada (Abordagem A - TypeChecker):* Quando o compilador analisar a declaração de um Enum com predicados na Fase 3, o TypeChecker forjará uma implementação invisível da interface `EQ` (`=`) na TAST. Essa implementação sintética extrairá estritamente a "Tag" dos ponteiros de ambos os Enums (os 8 bytes que os identificam) e fará a comparação, ignorando o *payload*. Essa abordagem mantém a filosofia da linguagem intacta, pois obedece naturalmente ao *Multiple Dispatch*, deixa o backend (Cranelift) isento de regras de negócio de domínio e permite ao Otimizador (Fase 4) fazer dobragem de constantes sem obstáculos.
- [x] **5.7. Memoização via `@cache_strategy`:**
    *   *Problema:* A diretiva de cache é validada sintaticamente mas ignorada no resto do pipeline.
    *   *Solução:* Criar uma passagem no otimizador que intercepte funções puras anotadas com `@cache_strategy` e injete verificações a uma *Hash Table* global gerenciada pelo `kata-rt`.

---

## 6. FFI, Standard Library e Kata-Runtime

- [x] **6.1. Implementação Real dos Stubs C-ABI (Feature Complete):**
    *   *Problema:* O `linker.rs` e o `kata_rt/ffi` ainda possuem stubs, retornos `NULL` (no cache) e delegações para `malloc/free` em C (ARC simplificado).
    *   *Solução:* Implementar a gestão real de memória em Rust exportada via C-ABI. O ARC (Atomic Reference Counting) alocará um bloco `[AtomicUsize + Payload]` garantindo incremento/decremento thread-safe e desalocação limpa sem vazamentos. O cache (`@cache_strategy`) utilizará um mapa de alta performance (como `DashMap` ou `std::collections::HashMap` com locks adequados) no runtime em vez de um "miss" perpétuo. As lógicas injetadas via texto em `linker.rs` devem ser eliminadas em favor da lib estática.
- [x] **6.2. Canais Rendezvous Reais:**
    *   Em `csp.rs`, implementada a camada com `KataSender` e `KataReceiver`.
- [x] **6.3. Atomic Reference Counting (ARC) Real:**
    *   Substituída a base de canais e transferências de structs opacas via ponteiros na ABI (suporte à alocação via Arenas acoplado à `Handle::current().block_on()` para Zero Leak CSP real).
- [x] **6.4. Multiplexação de Canais (`select` / `timeout`):**
    *   A interface FFI C foi mapeada nativamente em `tokio::select!` na `kata-rt`.
    *   *(Pendência residual: ligar a instrução Cranelift `TStmt::Select` à função FFI, previsto para as limpezas finais do Codegen)*.

---

## 7. Tooling e CLI (Finalização)

- [ ] **7.1. Comando `kata run` Completo:**
    *   Substituir os comentários no bloco `Commands::Run` de `main.rs` pela compilação temporária em `.tmp` seguida de execução imediata e descarte seguro (ou JIT).
- [x] **7.2. Expurgo Definitivo de Stubs:**
    *   *Solução:* Remover fisicamente `run_stub()`, `init_stub()` e `start()` sem função de todos os módulos. Limpar o `linker.rs` de implementações C inline (`kata_rt_add_int` etc.) que mascaram a verdadeira biblioteca compilada em Rust.
- [x] **7.3. Resolução Real de Imports no File System (O Estilo mod.kata):**
    *   *Problema:* O compilador não tem visibilidade do File System e carrega módulos de forma hardcoded (`src/core/types.kata`).
    *   *Solução:* Criar o `ModuleLoader`. Ele interpretará `import modulo.submodulo` buscando por `modulo/submodulo.kata` ou `modulo/submodulo/mod.kata` (análogo ao `mod.rs` de Rust). O Loader utilizará um cache (`HashMap<String, TypeEnv>`) para manter módulos já parseados e evitar ciclos de importação.
- [x] **7.4. Execução Efetiva do `kata test`:**
    *   *Problema:* O comando de testes atual apenas imprime na tela "Pronto para gerar Entrypoint", sem testar nada.
    *   *Solução:* Gerar dinamicamente um AST de entrypoint que invoque todas as funções anotadas com `@test`, passá-lo pelo pipeline (Codegen) e executar o binário reportando o resultado (Success/Fail) para o usuário.

---

## 8. Critérios de Aceite (Definition of Done)

- [ ] **Concorrência Real:** Um código testando a criação de 100.000 `fork!(...)` simultâneos que fazem `sleep!` deve rodar rapidamente num hardware normal, sem estourar o limite de *threads* do S.O.
- [ ] **Segurança de Memória (ARC Zero Leak):** Uma suíte de testes de estresse enviando milhares de `Enum` por canais não deve vazar RAM ao finalizar o programa (validado com Valgrind/Heaptrack).
- [x] **Captura Limpa:** *Closures* definidas dentro do ramo `Ok` de um bloco `match` devem capturar variáveis sem erro de runtime.
- [ ] **Multiplexação Ativa:** O comando `select` deverá escutar dois canais independentes e um timer (`timeout`) com resolução limpa no terminal.
- [x] **Testes Funcionais:** O comando `kata test` invocado na raiz do projeto deve localizar arquivos, compilar, testar as lógicas puras (booleanas) e impuras (`assert!`), e sair com *Exit Code 0* ou *1* conforme os resultados.
- [ ] **CLI Operacional:** O comando `kata run test_concurrency.kata` deve funcionar perfeitamente de ponta a ponta.omando `kata test` invocado na raiz do projeto deve localizar arquivos, compilar, testar as lógicas puras (booleanas) e impuras (`assert!`), e sair com *Exit Code 0* ou *1* conforme os resultados.
- [ ] **CLI Operacional:** O comando `kata run test_concurrency.kata` deve funcionar perfeitamente de ponta a ponta.
---

## 9. Proximos Passos Evolutivos (Definidos em Sessao)

- [ ] **Sistema de Erros Estruturado (Error Codes):** O formato atual de erros limitou a precisao dos testes de *Expected Failures*. A solucao proposta e estabelecer um ecossistema nativo de codigos de erro.
    - **Definicao em Kata-Lang:** O `Enum` base de erros de compilacao e execucao devera ser definido em *puro Kata* (ex: dentro da StdLib `core/errors.kata`), permitindo que a propria linguagem descreva as variantes (`TypeError`, `PurityError`, `OrphanRuleError`, `TcoError`).
    - **Extensibilidade (User-Defined Errors):** A infraestrutura deve permitir que o desenvolvedor crie seus proprios erros estruturados derivados das interfaces do compilador, viabilizando assercoes precisas e *Domain-Driven Testing*.
    - **Integracao no Compilador:** O *TypeChecker* e o *Optimizer* serao refatorados para emitir estruturas Rust atreladas as variantes definidas no codigo-fonte Kata, em vez de Strings formatadas aleatorias.
