# Product Requirements Document (PRD) - Kata-Lang Compiler

## Fase 1: Fundação e Tooling (CLI)

### Visão Geral
A Fase 1 estabelece o alicerce do projeto do compilador Kata-Lang. O foco exclusivo é criar a estrutura de diretórios e arquivos (scaffolding), definir a Interface de Linha de Comando (CLI) robusta utilizando o padrão da indústria (crate `clap`) e preparar a fundação de tratamento de erros visual (`miette`/`ariadne`) e logging (`log`/`env_logger`). Nenhuma lógica real de compilação ou parsing ocorrerá nesta fase, apenas a infraestrutura de despacho de comandos e visualização de *debug*.

### Objetivos
1. **Scaffolding Modular:** Refletir a arquitetura do "Roadmap" criando módulos Rust distintos para `lexer`, `parser`, `type_checker`, `codegen`, `kata_rt`, `repl` e `cli`.
2. **CLI Tipada e Robusta:** Implementar subcomandos e *flags* (opções) para cobrir todas as fases de uso e debug do compilador.
3. **Reporte de Erros Amigável:** Configurar a base para emitir mensagens de erro com contexto de código-fonte de alta qualidade.
4. **Despacho de Comandos:** Ligar a entrada da CLI a funções "stub" (vazias/simuladas) em seus respectivos módulos, validando a passagem de parâmetros.

### Requisitos Funcionais (CLI e Subcomandos)

A CLI principal responderá pelo binário `kata`.

#### 1. Subcomandos Principais
*   `kata build <ENTRYPOINT>`
    *   **Propósito:** Compila o módulo principal (e suas dependências) gerando um binário executável nativo (AOT).
    *   **Argumentos:** `<ENTRYPOINT>`: Caminho para o arquivo `.kata` inicial.
*   `kata run <ENTRYPOINT>`
    *   **Propósito:** Compila e executa o código imediatamente (usando JIT ou compilação e execução temporária).
    *   **Argumentos:** `<ENTRYPOINT>`: Caminho para o arquivo `.kata` inicial.
*   `kata test [PATH]`
    *   **Propósito:** Vasculha o diretório ou arquivo específico executando blocos marcados com `@test`.
    *   **Argumentos:** `[PATH]`: Caminho opcional (padrão é o diretório atual `.`).
*   `kata repl`
    *   **Propósito:** Inicia o ambiente de execução interativa (Read-Eval-Print Loop) acoplado ao compilador JIT.

#### 2. Flags Globais de Depuração Visual
Estas *flags* são vitais para o desenvolvimento iterativo das próximas fases, permitindo inspecionar as estruturas de dados internas antes da compilação de máquina. Devem estar disponíveis em comandos como `build` e `run`.

*   `--dump-tokens`: Imprime a saída linear produzida pelo Lexer.
*   `--dump-ast`: Imprime a Árvore Sintática Bruta (Plana / Sequences) produzida pelo Parser.
*   `--dump-tast`: Imprime a Árvore Tipada Resolvida (com aridade resolvida) produzida pelo Type Checker.

### Requisitos Não-Funcionais
*   **Linguagem/Ferramentas:** Rust 2021, Cargo.
*   **Dependências Chave Adicionadas nesta Fase:** `clap` (com feature `derive`).
*   **Tratamento de Erros:** O programa deve retornar códigos de saída (exit codes) padronizados em caso de erro no parseamento da CLI.

### Entregáveis da Fase 1
1. Arquivo `Cargo.toml` atualizado com a dependência `clap`.
2. Estrutura de diretórios expandida (`src/lexer`, `src/parser`, etc.) com seus respectivos `mod.rs` iniciais.
3. Arquivo `src/cli/mod.rs` ou similar contendo as *structs* e *enums* do `clap`.
4. Arquivo `src/main.rs` atualizado para instanciar a CLI, inicializar logs e despachar a execução para *stubs* nos submódulos que imprimem mensagens confirmando as *flags* recebidas.