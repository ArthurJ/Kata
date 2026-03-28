# Product Requirements Document (PRD) - Kata-Lang Compiler

## Fase 3: Middle-end (Análise Semântica e TAST)

### Visão Geral
A Fase 3 é o coração lógico do compilador Kata-Lang. É nesta etapa que as sequências planas (AST Bruta) ganham significado matemático e comportamental. O *Middle-end* é responsável por duas missões vitais: **resolver a aridade** (transformando listas de tokens estruturados na verdadeira Árvore de Execução / Typed Abstract Syntax Tree - TAST) e **validar a correção semântica**, garantindo a pureza funcional, a tipagem estrita e os contratos de interfaces.

### Objetivos

#### 1. Construção do Environment (Tabela de Símbolos)
O `TypeEnv` é a memória central do compilador durante a validação.
*   **Coleta Top-Level:** Varredura inicial no módulo para registrar todas as declarações `data`, `enum`, `interface` e assinaturas (`::`).
*   **Registro de Aridade:** Mapear a quantidade exata de argumentos que cada função pura e cada construtor de tipo exige (ex: `Vec2` tem aridade 2).
*   **Registro de Contratos:** Armazenar as implementações (`implements`) e validar se a "Regra de Coerência" (Orphan Rule) é respeitada.

#### 2. Resolução de Aridade (Construção da Árvore Real)
Uma vez que o Parser da Fase 2 agrupou a notação prefixada em `Sequence`s lineares, a Fase 3 deve dar formato de árvore (Call Graph).
*   **Varredura Gulosa (Greedy):** Percorrer as `Sequence`s da esquerda para a direita. Ao encontrar um identificador registado como função/construtor, o compilador consulta a sua aridade no `TypeEnv` e "engole" (consome) a exata quantidade de expressões seguintes para montar o nó de `Call`.
*   **Apoio do Operador `$`:** Resolução de blocos que utilizam a aplicação explícita (desambiguação forçada de tuplos).
*   **Currying Explícito:** Identificar o operador *Hole* (`_`) numa sequência de aplicação e transformar o nó num *Lambda* (closure) que aguarda a injeção do argumento em falta.
*   **Actions vs. Funções:** Enquanto funções puras têm aridade estritamente fixa, as Actions variam (ex: `echo!` é variádica e consome o resto da instrução, `queue! 16` consome apenas 1 argumento).

#### 3. Type Checking e Múltiplo Despacho
O sistema de tipos da Kata-Lang entra em ação.
*   **Multiple Dispatch (Despacho Múltiplo):** Quando ocorre uma chamada de função (ex: `+ a b`), o compilador deve decidir qual a implementação concreta a usar. A pontuação de especificidade segue a regra: Correspondência Exata > Subtipo/Interface > Generic `TypeVar`.
*   **Tipos Refinados & Smart Constructors:** Validar a degradação de Tipos-Refinados em operações matemáticas. Garantir que a invocação de um construtor de Tipo-Refinado devolve um `Result` (a menos que a prova estática aconteça numa fase posterior).
*   **Generics & Early Checking:** Provar matematicamente a validade de uma função genérica no momento da sua definição, baseando-se estritamente nas cláusulas `with T as INTERFACE`.

#### 4. Validações Semânticas de Domínio (Barreiras de Segurança)
A separação entre `data`, `lambdas` e `actions` deve ser garantida taticamente:
*   **Barreira de Pureza:** Emitir um Erro Fatal de compilação caso seja detetado uso de mutabilidade (`var`), laços imperativos (`loop`, `for`), ou chamadas a `Actions` (sufixo `!`) dentro de um bloco `lambda`.
*   **Exaustividade de Padrões:** Garantir que expressões de `pattern matching` (nos argumentos de lambdas) e blocos `match` (nas Actions) cubram todas as variantes de um Tipo Soma (Enum) ou contenham a cláusula `otherwise:`.
*   **Proibição de Recursão em Actions:** Detetar ciclos recursivos em Actions e gerar um erro fatal de *Stack Overflow* preventivo.

#### 5. Integração com a Standard Library (Prelude)
A biblioteca padrão da Kata-Lang (`@src/core/**`) atua como a fundação lógica do sistema.
*   **Auto-Importação do Prelude:** O `TypeEnv` deve ser inicializado processando automaticamente o arquivo `src/core/prelude.kata` (e suas dependências como `types.kata`, `io.kata`, `csp.kata`).
*   **Tratamento de Diretivas `@ffi` e Atributos Mágicos:** O Parser e a TAST devem ser ajustados para suportar metadados/atributos nas declarações top-level (ex: `@ffi("kata_rt_map")` e `@comutative`), essenciais para ligar assinaturas puras a código de máquina Rust ou alterar o comportamento do Multiple Dispatch.
*   **Abertura do Namespace:** Tudo o que o `prelude.kata` declarar via `export` deve ser injetado no namespace global de qualquer módulo de usuário que for compilado.

### Requisitos Não-Funcionais
*   **Nova Estrutura de AST:** Criar uma árvore estritamente tipificada (`TAST` - Typed AST) que contenha o tipo resolvido de cada expressão, dissociando a Fase 3 da AST bruta do Parser.
*   **Integração Visual de Erros:** Manter o uso do `ariadne` e `miette` já configurados na Fase 2 para exibir erros semânticos belos (ex: grifar de vermelho os tipos incompatíveis ou onde a pureza foi violada no código-fonte).

### Entregáveis da Fase 3
1. Arquivos `src/type_checker/tast.rs` contendo a definição da TAST.
2. Arquivos `src/type_checker/environment.rs` implementando a Tabela de Símbolos (`TypeEnv`).
3. Algoritmo de resolução de aridade (`src/type_checker/arity_resolver.rs` ou similar) capaz de transformar `Expr::Sequence` em chamadas `Expr::Call`.
4. Algoritmo de type checking e resolução de Multiple Dispatch (`src/type_checker/checker.rs`).
5. Conexão do analisador semântico no fluxo de compilação de `main.rs`, emitindo a `TAST` quando a *flag* `--dump-tast` é utilizada.
6. Suíte de testes validando: a construção correta de `Call`s, falhas de *Type Mismatch*, a verificação de exaustividade nos *Enums* e os erros da barreira de pureza.