# PRD - Fase 6: Backend (Codegen via Cranelift)

## 1. Objetivo
Implementar o backend de compilação da Kata-Lang usando o framework Cranelift (`cranelift-codegen` e `cranelift-module`). O objetivo desta fase é consumir a Árvore Sintática Tipada (TAST) otimizada e gerar instruções de código de máquina nativo (AOT - Ahead of Time), produzindo um arquivo objeto (.o/.obj) autossuficiente e chamando o Linker do Sistema Operacional para conectá-lo à biblioteca padrão e ao runtime embutidos (`kata-rt`).

## 2. Escopo Arquitetural e Workaround (MVP)
Para permitir que o projeto evolua rapidamente sem a complexidade colossal de escrever um transformador de Máquinas de Estado (State Machine) em Cranelift do zero, a Fase 6 assumirá a seguinte postura arquitetural:

*   **Mapeamento 1-para-1:** Tanto `Functions` (Lambdas puros) quanto `Actions` (Impuras) serão compiladas da mesma maneira estrutural no Cranelift: como **código linear síncrono** usando a *Stack* de CPU normal (C-ABI).
*   **O Truque de Concorrência:** Apesar da Kata-Lang operar com um modelo CSP concorrente via *Green Threads* (M:N), o bloqueio em canais (`<!`) ou as chamadas de espera não suspenderão a *Call Stack*. A Action irá de fato bloquear a Thread atual no `kata-rt`. A mágica que impede o Tokio de travar ocorrerá inteiramente na biblioteca `kata-rt` (Fase 5), que usa `tokio::task::spawn_blocking` para orquestrar essas execuções lineares síncronas numa *Thread Pool* especializada do Sistema Operacional.
*   **Integração FFI:** Toda a StdLib Kata-Lang (`+`, `/`, `echo!`, `channel!`) já está codificada em Rust no `kata_rt/ffi`. O Cranelift registrará essas assinaturas `extern "C"` globais e irá gerar as instruções (Assembly) de *Call* invocando-as.

*(Nota: O suporte a Máquinas de Estado assíncronas reais ou Fibras de Stack Switching será delegado para a recém-criada Fase 8).*

## 3. Estrutura de Pastas e Arquivos

O módulo do Backend ficará centralizado na pasta `src/codegen/`.

```text
src/codegen/
├── mod.rs               # Ponto de entrada (substituindo o stub `run_stub`). Controla o pipeline de geração e linking.
├── context.rs           # `CodegenContext`: Wrapper para as ferramentas do Cranelift (Context, BuilderContext, Module). Mantém a tabela de mapeamento String -> Cranelift FuncId.
├── translator.rs        # `FunctionTranslator`: Classe focada em iterar os nós de `TTopLevel` (LambdaDef, ActionDef) e traduzi-los para `cranelift::builder::FunctionBuilder`.
├── expr.rs              # Lógica de conversão para `TExpr` e `TStmt`: Como traduzir variáveis, blocos Let, Loops, Match (geração de Basic Blocks nativos).
└── linker.rs            # Módulo imperativo para invocar a cadeia de compilação externa do SO (ex: `gcc -o output arquivo.o libkata_rt.a`).
```

## 4. O Que Será Desenvolvido

### 4.1. Configuração do Cranelift (Módulo e Builder)
*   Instanciar o *Target ISA* com a arquitetura nativa da máquina hospedeira.
*   Configurar a criação de objetos no formato `Object` (.o) para a pipeline AOT do `kata build`.
*   Criar funções para pré-declarar as assinaturas globais (Functions, Actions e as FFIs do `kata_rt`). Isso resolverá o problema de funções se invocando mutuamente ou em ordens arbitrárias.

### 4.2. Tradução de AST para IR do Cranelift
*   Mapear os tipos do Kata-Lang (`Int`, `Float`, `Bool`, etc) para tipos de IR nativos do Cranelift (`I64`, `F64`, `I8`/`B1`).
*   **Controle de Fluxo e Variáveis (Let/Var):** Uso extensivo do sistema Cranelift `Variable`. As variáveis da linguagem serão traduzidas para variáveis virtuais na SSA form (Static Single Assignment) do Cranelift.
*   **Tradução de Funções e Lambdas:** Mapear o escopo do bloco `lambda`, criar a assinatura do Cranelift, processar os argumentos da função e associá-los aos blocos iniciais (`Block 0`).
*   **If/Else, Guard, Match:** Geração de blocos básicos (Basic Blocks) adicionais. Criar `Block 1`, `Block 2`, adicionar instruções de `icmp` ou comparativos booleanos, e emitir instruções condicionais de pulo (`brif`).
*   **Loops Impuros (For, Loop):** Tradução dos `TStmt::Loop` emitindo nós de pulo (`jump`) diretos e incondicionais para o início de um Bloco Básico previamente criado (criando assim o loop de *machine code*). Tratamento especial do `Break` e `Continue`.

### 4.3. A Passagem Final (Compilação Nativa e Linking)
*   Ao terminar a tradução de todas as declarações `TopLevel`, invocar o encerramento do `Cranelift ObjectModule`, exportando bytes em memória para um arquivo real (ex: `output.o`).
*   Implementar no `linker.rs` uma chamada de `std::process::Command` que procurará um Linker compatível na máquina (`cc`, `clang` ou `gcc`) e fará a ponte do `output.o` com a biblioteca estática que devemos gerar do `kata_rt`.

## 5. Entregáveis

1.  **Tradução Primitiva e Matemática:** O comando `kata build` conseguirá receber scripts de testes executando cálculos de Inteiros, Floats, manipulação Booleana simples e Guard clauses, resultando em um arquivo objeto compilado.
2.  **O Módulo Linker Funcional:** O compilador vai emitir de fato o binário (ex: `meu_programa`) e não apenas checar erros. O binário rodará instanciando o `kata_rt_boot` no início de sua Main nativa gerada pelo Cranelift.
3.  **Tradução Completa Mínima (MVP):** Integração correta com a FFI. A instrução `echo!("Olá mundo")` traduzirá com sucesso para a invocação da função assembly referenciando `kata_rt_print_str`.
4.  **Testes de Regressão do Codegen:** Conjunto de testes unitários que validam se a saída JIT temporária do Cranelift (usando um `JITBuilder` apenas para teste local na memória) produz os resultados corretos num ambiente fechado (ex: o resultado de uma TAST simples `+ 2 2` retorna `4` na avaliação nativa em RAM).
