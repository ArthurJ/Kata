PRD: Kata Language - REPL & Parser Reboot

  Objetivo: Refazer do zero a arquitetura do REPL e a sintaxe/parser de expressões. O objetivo é estabelecer pipelines desacoplados: um para a compilação de arquivos (.kata) e um específico, leve e tolerante a falhas para a avaliação interativa de expressões (REPL).

  2. Princípios Arquiteturais

   1. Separação de Preocupações (Entrypoints Distintos): O Parser deve expor múltiplos pontos de entrada. Parsear uma expressão (parse_expr) é fundamentalmente diferente de parsear um módulo (parse_module). O REPL consumirá apenas fragmentos.
   2. Avaliação de Expressões como Cidadãs de Primeira Classe: O Type Checker e o JIT Compiler devem ter APIs públicas para tipar e executar uma Expr solta dentro do Environment atual, sem precisar que ela pertença a uma Action ou Function.
   3. Gramática Prefixa Estrita: A notação prefixa (ex: + 1 2) deve ser formalizada. A aplicação de função deve ser o "nodo raiz" das expressões, mas com regras claras de delimitação (exigência de parênteses para aninhamentos ou uso estrito de aridade).
   4. Resiliência do Lexer no REPL: O sistema de indentação (INDENT/DEDENT) deve ser desativado ou operado em um "Modo REPL/Single-line", evitando que quebras de linha injetem blocos lógicos inesperados.

  ---

  3. Arquitetura de Módulos e Estrutura de Pastas

  Esta será a nova taxonomia interna para os módulos afetados. 

    1 src/
    2 ├── lexer/
    3 │   ├── mod.rs
    4 │   ├── token.rs
    5 │   ├── error.rs
    6 │   └── lexer.rs          # Nova adição: Enum `LexMode { File, Repl }`. No modo Repl, 
    7 │                         # regras de indentação são relaxadas.
    8 ├── parser/               # Totalmente reescrito
    9 │   ├── mod.rs            # Expõe `parse_module`, `parse_expr`, `parse_statement`
   10 │   ├── error.rs
   11 │   ├── core/             # Utilitários Chumsky isolados
   12 │   │   ├── combinators.rs
   13 │   │   └── tokens.rs     
   14 │   ├── grammar/          # A gramática dividida de forma hierárquica clara
   15 │   │   ├── module.rs     # imports, exports, top-level decls
   16 │   │   ├── stmt.rs       # let, var, loop, match
   17 │   │   ├── expr.rs       # literais, identificadores, chamadas prefixas
   18 │   │   └── types.rs      # assinaturas e tipos refinados
   19 │   └── tests/
   20 ├── type_checker/
   21 │   ├── mod.rs
   22 │   ├── checker.rs        # Adição da API: `check_isolated_expr(&mut self, expr: Expr)`
   23 │   ├── environment.rs    # Deve suportar persistência de estado entre interações do REPL
   24 │   └── ...
   25 └── repl/                 # Totalmente reescrito
   26     ├── mod.rs            # Loop principal (Rustyline)
   27     ├── state.rs          # Gerencia o `SessionContext` (Environment vivo, JIT vivo)
   28     ├── evaluator.rs      # Pipeline puro: Lex -> ParseExpr -> CheckExpr -> RunExpr
   29     └── commands.rs       # Comandos dot (.ast, .env, .type)

  ---

  4. Especificações Detalhadas dos Componentes

  4.1. O Novo Parser de Expressões (src/parser/grammar/expr.rs)
  O design anterior usava recursão profunda e escolha arbitrária, o que engasgava no REPL. 
  O novo design usará Pratt Parsing (ou uma hierarquia explícita no Chumsky) com as seguintes regras para a notação prefixa:

   * Átomos: Literais, Identificadores simples.
   * Agrupamento: Tudo entre ( e ) é avaliado como uma única expressão.
   * Aplicação Prefixa: A regra de aplicação será Identificador seguido por N expressões. Como o parser não sabe a aridade da função em tempo de parsing, a regra será agressiva: uma função prefixa consome todas as expressões subsequentes na mesma linha/bloco, a menos que agrupada por parênteses.
       * Válido: + 1 2 (Lido como Add(1, 2))
       * Ambíguo/Proibido: + 1 * 2 3 (Sem parênteses, o parser não sabe onde o + termina. Exigirá (+ 1 (* 2 3))).

  4.2. A Nova API do Type Checker (src/type_checker/checker.rs)
  Para parar de criar módulos falsos (__repl_expr_1), o Checker ganhará a seguinte capacidade:

   1 impl Checker {
   2     /// Pega uma expressão bruta do parser e a tipa no contexto atual.
   3     /// Retorna a Expressão Tipada (TAST) e o seu Tipo.
   4     pub fn type_check_expression(&mut self, expr: &Expr) -> Result<(TypedExpr, Type), TypeError> { ... }
   5     
   6     /// Tipa uma declaração solta (ex: `let x = 10` digitado no REPL) e altera o Environment.
   7     pub fn type_check_statement(&mut self, stmt: &Stmt) -> Result<TypedStmt, TypeError> { ... }
   8 }

  4.3. O Novo Pipeline do REPL (src/repl/evaluator.rs)
  O fluxo interativo será linear e sem "hacks" de AST:

   1. Classificação da Entrada: O REPL decide se a string é um Command (começa com .), um Statement (começa com let, var, action, etc) ou uma Expression (resto).
   2. Lexing: Executa o lexer em modo LexMode::Repl (ignora falta de newline final, não força DEDENTs estritos se não houver bloco).
   3. Parsing Específico: Chama parser::parse_expr(tokens) ou parser::parse_stmt(tokens).
   4. Type Checking Específico: Chama checker.type_check_expression(ast).
   5. Injeção de FFI (SHOW): Se for uma expressão (cujo tipo não seja Unit), o REPL em nível de TAST (não no nível de string ou AST bruto) engloba a TypedExpr resultante numa chamada para a interface global SHOW.str().
   6. Compilação e Execução: Envia a TypedExpr final para o compilador JIT compilar como uma função anônima efêmera, roda, e imprime o ponteiro de string retornado.

  ---

  5. Plano de Implementação (Fases Sugeridas)

  Para executarmos essa refatoração de forma cirúrgica e testável:

  Fase 1: O Núcleo do Parser (Core & Grammar)
   * Deletar as lógicas confusas de expr.rs e decl.rs atuais.
   * Criar parser/core/ e implementar apenas o parse_expr funcional e rigoroso com testes unitários locais (garantindo que (+ 1.5 2.5) funcione perfeitamente isolado).

  Fase 2: Adaptação do Middle-End (Type Checker)
   * Adicionar o método type_check_expression no Type Checker.
   * Garantir que ele consiga consultar o Environment global para resolver operadores nativos (como + que despacha para Float implements NUM).

  Fase 3: O Novo REPL
   * Jogar fora o evaluator.rs atual.
   * Escrever a nova máquina de estados (SessionContext) que consome os novos entrypoints do Parser e do Checker.
