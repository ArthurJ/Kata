# Kata-Lang Development Roadmap & Architecture

Este documento define a arquitetura definitiva e o plano de implementação passo a passo para o compilador e ambiente de execução da **Kata-lang**. 
A linguagem busca unir a elegância do Python, o rigor matemático de Haskell e a performance de Rust/Julia, gerando binários estáticos x64 (AOT) e provendo um REPL interativo (JIT).

Este documento serve como a fonte de verdade para futuras sessões de desenvolvimento, preservando todas as decisões arquiteturais tomadas.

---

## 🏗️ Arquitetura do Pipeline (Visão Geral)

O fluxo de compilação é estritamente unidirecional e divide claramente as responsabilidades de estruturação sintática e validação semântica:

`Código Fonte -> Lexer -> Parser -> [AST Plana] -> Analisador Semântico (Type Checker / TAST) -> Otimizador (MIR) -> Backend (Cranelift) -> Binário / JIT`

 src/
 ├── lexer/
 │   ├── mod.rs
 │   ├── token.rs
 │   ├── error.rs
 │   └── lexer.rs          # Nova adição: Enum `LexMode { File, Repl }`. No modo Repl, 
 │                         # regras de indentação são relaxadas.
 ├── parser/               # Totalmente reescrito
 │   ├── mod.rs            # Expõe `parse_module`, `parse_expr`, `parse_statement`
 │   ├── error.rs
 │   ├── core/             # Utilitários Chumsky isolados
 │   │   ├── combinators.rs
 │   │   └── tokens.rs     
 │   ├── grammar/          # A gramática dividida de forma hierárquica clara
 │   │   ├── module.rs     # imports, exports, top-level decls
 │   │   ├── stmt.rs       # let, var, loop, match
 │   │   ├── expr.rs       # literais, identificadores, chamadas prefixas
 │   │   └── types.rs      # assinaturas e tipos refinados
 │   └── tests/
 ├── type_checker/
 │   ├── mod.rs
 │   ├── checker.rs        # Adição da API: `check_isolated_expr(&mut self, expr: Expr)`
 │   ├── environment.rs    # Deve suportar persistência de estado entre interações do REPL
 │   └── ...
 └── repl/                 # Totalmente reescrito
     ├── mod.rs            # Loop principal (Rustyline)
     ├── state.rs          # Gerencia o `SessionContext` (Environment vivo, JIT vivo)
     ├── evaluator.rs      # Pipeline puro: Lex -> ParseExpr -> CheckExpr -> RunExpr
     └── commands.rs       # Comandos dot (.ast, .env, .type)

**A Grande Decisão de Design:** Para suportar a notação prefixa sem "adivinhar" aridades e sem quebrar o REPL, o Parser **NÃO** monta a árvore de chamadas (Call Tree). O Parser apenas agrupa itens em `Sequences` (AST Plana). A árvore de chamadas real (TAST) só é montada na Fase 3 (Analisador Semântico), onde a Tabela de Símbolos informa a aridade exata de cada função.

---

## 📍 Fase 1: Fundação e CLI (Tooling)
Estabelecer a interface do usuário e as ferramentas de inspeção do compilador. Como a AST só vira árvore na Fase 3, inspecionar o pipeline visualmente é crucial.

- [ ] Estruturação do projeto em Rust (módulos: `lexer`, `parser`, `type_checker`, `codegen`, `kata_rt`, `repl`).
- [ ] Implementar a CLI principal (`kata build`, `kata run`, `kata test`, `kata repl`).
- [ ] Adicionar flags de depuração visual do pipeline:
  - `--dump-tokens`: Imprime a saída linear do Lexer.
  - `--dump-ast`: Imprime a Árvore Sintática Bruta (Plana / Sequences).
  - `--dump-tast`: Imprime a Árvore Tipada Resolvida (com os nós de `Call` reais).

---

## 📍 Fase 2: Frontend (Lexer & Parser)
O Frontend é focado em regras de texto e estrutura bruta. Ele não sabe o que as funções significam nem quantos argumentos recebem.

### Lexer
- [ ] **Whitespace Significativo:** Emissão automática de tokens `INDENT` e `DEDENT` baseada na indentação.
- [ ] **Capitalização Estrita (Tipagem Léxica):**
  - `InterfaceID`: Apenas UPPERCASE (ex: `NUM`, `SHOW`).
  - `TypeID`: Apenas PascalCase (ex: `Int`, `List`).
  - `Ident`: snake_case ou símbolos matemáticos (ex: `soma`, `+`, `echo!`).
  - `TypeVar` (Generics): Uma única letra maiúscula (ex: `A`, `T`).

### Parser (AST Plana)
- [ ] **Domínios Isolados:** Estruturação em blocos de `data`, `enum`, `interface`, `lambda` (domínio puro) e `action` (domínio impuro).
- [ ] **Declarações vs Expressões:**
  - *Functions (Lambdas):* Tudo é Expression (retorna valor). `let` é uma expressão de amarração; `match` imperativo é proibido.
  - *Actions:* Contêm Statements (`Let`, `Var`, `Loop`, `Match`, `Return`).
- [ ] **Sequências (Agrupamento Guloso):** Linhas de código sem delimitadores viram um nó `Expr::Sequence(Vec<Expr>)`. Exemplo: `+ 1 * 2 3` se torna uma lista plana.
- [ ] **Tuplas e Construtores:** Agrupamentos explícitos com `()` geram nós estruturais `Tuple`.

---

## 📍 Fase 3: Middle-end (Análise Semântica e TAST)
O coração do compilador. É aqui que as sequências planas ganham significado lógico e matemático e a verdadeira árvore de execução é formada.

### 3.1 Construção do Environment (Tabela de Símbolos)
- [ ] Coleta de todas as declarações *top-level* (`data`, `enum`, assinaturas `::`).
- [ ] Registro de Interfaces customizadas pelo usuário e validação de contratos lógicos.
- [ ] Mapeamento da **Aridade** de cada função pura e Construtor (ex: `Vec2` tem aridade 2).

### 3.2 Resolução de Aridade (Construção da Árvore Real)
- [ ] Varredura das `Sequence`s da esquerda para a direita.
- [ ] Transformação de identificadores em nós de `Call`, consumindo a quantidade exata de argumentos subsequentes baseada na Aridade da Tabela de Símbolos.
- [ ] **Actions Variádicas vs Fixas:** Resolução de funções impuras (ex: `echo!` consome até o fim da linha; `queue! 16` consome apenas 1 argumento, deixando o resto da linha intacto).

### 3.3 Type Checking e Múltiplo Despacho
- [ ] **Algoritmo de Especificidade (Multiple Dispatch):** Resolução de sobrecargas pontuando: Correspondência Exata > Subtipo/Interface > Generic `TypeVar`.
- [ ] **Validação de Tipos Refinados:** Retornar construtores dinâmicos (`Result`) quando associados a variáveis em runtime.
- [ ] **Checagem de Exaustividade:** Garantir que blocos `match` (em Actions) tratam todas as variantes de um `enum` (ex: `Ok`/`Err`, `Some`/`None`).
- [ ] **Barreira de Pureza:** Emitir Erro Fatal se houver mutabilidade (`var`) ou I/O (operador `!`) dentro de Lambdas.

---

## 📍 Fase 4: Otimizador (MIR)
Garantir o princípio de *Zero-Cost Abstractions* transformando a TAST antes da geração de código de máquina.

- [ ] **Monomorfização:** Duplicar e especializar funções genéricas (Generics `with T as NUM`) para os tipos concretos inferidos, eliminando V-Tables dinâmicas.
- [ ] **Avaliação @comptime (Fallbacks Literais Estáticos):** Se um Tipo Refinado (ex: `PositiveInt`) receber um literal que passa no predicado (ex: `10`), provar estaticamente e remover a emissão do `Result`.
- [ ] **TCO (Tail Call Optimization):** Transformar recursões de cauda puras em saltos de memória (JMP / Loops), prevenindo Stack Overflow. (Erro fatal se recursão não for TCO ou ocorrer em Actions).
- [ ] **Tree-Shaking:** Montar um Grafo de Chamadas a partir da `main!` e extirpar nós, interfaces e funções inacessíveis para o binário final.
- [ ] **Stream-Fusion:** Funder iterações adjacentes (`map`, `filter`) num único loop sem alocar coleções intermediárias na Heap.
- [ ] **Constant Folding:** Resolver expressões puras com literais estáticos no Type Checker (ex: `+ 2 2` vira `4`).

---

## 📍 Fase 5: O Runtime (`kata-rt` com Tokio)
A biblioteca base em Rust embutida no binário gerado, focada no modelo CSP (Communicating Sequential Processes). O fluxo real do programa é controlado pelo Tokio.

- [ ] **Integração com Tokio:** Inicializar o runtime multi-thread do Tokio sob os panos e submeter a `Action` principal via `tokio::spawn`.
- [ ] **Implementação de Canais CSP (Wrappers Tokio):**
  - `fork!` -> `tokio::spawn(action_future)`.
  - `@parallel fork!` -> `tokio::task::spawn_blocking` (OS Thread isolada).
  - `channel!` -> Fila *Rendezvous* via `mpsc::channel(1)` bloqueante.
  - `queue!(N)` -> Canal com buffer e *Backpressure* via `mpsc::channel(N)`.
  - `broadcast!` -> Topologia Pub/Sub 1-para-N via `tokio::sync::broadcast`.
  - `select` -> Mapeado para a macro `tokio::select!`.
- [ ] **Gerenciamento de Memória:**
  - *Arenas Locais:* Estado e variáveis da Action pertencem inteiramente à Task do Tokio (Lock-free, limpo em O(1)).
  - *ARC Global:* Promoção de referências para `Arc<T>` (Atomic Reference Counting) quando dados imutáveis são enviados através de canais CSP.

---

## 📍 Fase 6: Backend (Codegen via Cranelift)
Geração de código nativo (AOT) usando o Cranelift IR Emitter.

- [ ] **Functions (Lambdas):** Compilação para código linear rápido utilizando C-ABI nativa (Stack do Sistema Operacional).
- [ ] **Actions (State Machines):** Compilação para **Máquinas de Estado (Futures do Rust)**. Quando encontram `sleep!` ou bloqueios de canal (`<!`), salvam o contexto na Arena e cedem controle (`yield` / `Poll::Pending`) para o Tokio não travar a CPU.
- [ ] **Linker:** Geração do arquivo Objeto (`.o`) e linkagem final com o `kata-rt` para gerar o executável nativo standalone.

---

## 📍 Fase 7: REPL Interativo (JIT)
Ambiente de desenvolvimento iterativo de alta performance. Reutiliza as fases 1 a 4, mas desvia do Backend AOT.

- [ ] **LexMode::Repl:** Flexibilização do Lexer para tolerar formatações imperfeitas e ausência de *newlines* finais.
- [ ] **SessionContext:** Type Checker mantém um ambiente vivo na memória, persistindo imports e variáveis léxicas entre os comandos.
- [ ] **Cranelift JIT:** A TAST resultante é compilada diretamente na RAM (sem gerar arquivo `.o`).
- [ ] **Injeção de FFI (SHOW):** O REPL envelopa o resultado das expressões numa chamada invisível para a interface `SHOW.str()`, executa o ponteiro de memória e imprime a string na tela.
