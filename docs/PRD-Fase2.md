# Product Requirements Document (PRD) - Kata-Lang Compiler

## Fase 2: Frontend (Lexer & Parser)

### Visão Geral
A Fase 2 foca em transformar o código fonte em texto bruto na nossa Árvore Sintática Abstrata Plana (AST). O processo é dividido em duas etapas que se comunicam através de um fluxo de Tokens: o **Lexer** e o **Parser**. A arquitetura da Kata-lang exige regras léxicas estritas para indentação (Significant Whitespace) e capitalização, que servem para desambiguar e simplificar a gramática do Parser.

### Objetivos

#### 1. Lexer (Análise Léxica)
O Lexer processa a string de entrada e emite uma lista de `Token`s.
*   **Controle de Indentação:** Emissão de tokens sintéticos `INDENT` e `DEDENT` ao detectar mudanças no nível de indentação do código.
*   **Regras de Capitalização (Tipagem Léxica):**
    *   `InterfaceID`: Apenas MAIÚSCULAS (ex: `NUM`, `ORD`).
    *   `TypeID`: PascalCase (ex: `Int`, `List`).
    *   `Ident`: snake_case ou símbolos matemáticos (ex: `soma`, `+`, `echo!`).
    *   `TypeVar`: Letra maiúscula única (ex: `A`, `T`).
*   **Tokens Estruturais:** Parênteses `()`, colchetes `[]`, chaves `{}`, delimitadores de escopo (`:` , `=>`, `->`), etc.
*   **Tratamento de Strings e Comentários:** Strings literais são dados "cegos" e comentários (iniciados por `#`) são ignorados.
*   **LexMode:** Suporte para `LexMode::File` e `LexMode::Repl` (onde as regras de indentação e terminadores de quebra de linha `\n` são mais tolerantes).

#### 2. Parser (AST Plana)
O Parser consome os `Token`s e monta a AST "Plana".
*   **Sem Árvore de Chamadas (Ainda):** O Parser não agrupa argumentos para funções. Ele apenas coleta `Sequence`s. (Ex: `+ 1 * 2 3` vira uma lista linear). A resolução de aridade acontecerá na Fase 3.
*   **Declarações de Top-Level:** `data`, `enum`, `interface`, assinaturas (`::`), `lambda`, `action` e `export`.
*   **Domínios Isolados:**
    *   **Functions (Lambdas):** Compostas inteiramente por Expressões. Proibido o uso de `var` e ações com efeito colateral (`!`).
    *   **Actions:** Compostas por Statements (Instruções: `let`, `var`, `loop`, `match`) que controlam a máquina de estado.
*   **Tuplas e Coleções:** Reconhecimento explícito de `()`, `[]` e `{}`.
*   **Tratamento de Erros:** Integração contínua com `miette` e `ariadne` para relatar falhas sintáticas com o snippet do código fonte.

### Entregáveis da Fase 2
1. Módulo `src/lexer` completo com definição de `Token`, `LexMode` e a função de lexing.
2. Definições da AST em `src/parser/ast.rs` (Expressões, Instruções e Top-Level).
3. Módulo `src/parser` implementando a gramática utilizando combinadores (com `chumsky`).
4. Conexão do Lexer e Parser no fluxo do comando `kata build` (substituindo os *stubs*).
5. Casos de teste básicos garantindo a formação correta das `Sequence`s e falhas esperadas por erro de capitalização ou indentação.