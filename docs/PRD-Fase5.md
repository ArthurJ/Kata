# PRD - Fase 5: Kata Runtime (kata-rt) e Concorrência CSP

## 1. Objetivo
Implementar o pacote `kata-rt`, uma biblioteca base em Rust (embedded no binário) responsável por fornecer o ambiente de execução da Kata-Lang. Ele atua como uma fina camada de sistema operacional: inicializa o escalonador cooperativo M:N (`tokio`), expõe primitivas de I/O embutidas e orquestra a topologia híbrida de memória (Arenas locais vs. ARC global) orientada por *Escape Analysis*.

## 2. Escopo Arquitetural e Interação
O `kata-rt` não interpreta código. Ele fornece uma API ABI (Application Binary Interface) em C (`#[no_mangle]`) que será invocada pelas instruções Assembly/IR geradas pelo Cranelift na Fase 6.

*   **Ponto de Entrada:** O executável compilado invocará a função de inicialização do runtime, que levantará o `tokio::Runtime` e colocará a Action `main!` para rodar.
*   **Domínio Impuro (Actions):** Compiladas como Máquinas de Estado Assíncronas (Futures). Quando uma Action executa I/O ou bloqueia num canal, ela cede controle (`yield` / `Poll::Pending`) para o Tokio.
*   **Domínio Puro (Functions):** Executam código de máquina síncrono e linear diretamente na *Call Stack* da CPU. Não sofrem interrupção do escalonador cooperativo e alocam estruturas efêmeras em Arenas.

## 3. Modelo de Memória Híbrido (Low-Cost & Zero-Copy)
Com a implementação do *Escape Analysis* no otimizador (Fase 4), a Kata-Lang elimina o custo de cópias profundas e a necessidade de um *Tracing Garbage Collector*.

*   **Alocação Local (Arenas):** Dados que não sofrem fuga (não são enviados por canais) são alocados no `BumpAllocator` anexado à Task do Tokio atual. Alocação e limpeza custam `O(1)`. Ideal para processamento matemático puro e garantindo *Cache Locality*.
*   **Alocação Global (ARC):** Dados marcados com fuga (`AllocMode::Shared`) são alocados diretamente na Heap Global partilhada dentro de um bloco ARC (Atomic Reference Counting).
*   **Zero-Copy em Canais:** Ao transferir um dado via canal CSP, apenas o ponteiro do bloco ARC é enviado. Nenhuma clonagem estrutural de memória ocorre durante o I/O concorrente.

## 4. O Sistema de Concorrência CSP
O `kata-rt` envolverá as primitivas do ecossistema `tokio` para criar as estruturas nativas da Kata-Lang (`src/core/csp.kata`):

*   `fork!(action)` -> `tokio::spawn`.
*   `@parallel fork!(action)` -> `tokio::task::spawn_blocking` (Thread OS nativa isolada).
*   `channel!()` -> Fila *Rendezvous* via `tokio::sync::mpsc::channel(1)` com garantia síncrona.
*   `queue!(N)` -> Fila assíncrona com *Backpressure* via `tokio::sync::mpsc::channel(N)`.
*   `broadcast!()` -> Topologia Pub/Sub via `tokio::sync::broadcast`.
*   `select` -> Mapeamento em *runtime* para o comportamento da macro `tokio::select!`.

## 5. Estrutura de Pastas e Arquivos

O módulo `kata-rt` será construído dentro do compilador atual (`src/kata_rt/`) para posterior extração ou ligação estática.

```text
src/kata_rt/
├── mod.rs           # Inicialização do Tokio e Ponto de Entrada (Bootstrapper)
├── memory.rs        # Implementação das Arenas (Bumpalo) e Global Promoters (ARC)
├── csp.rs           # Wrappers dos Canais (Sender/Receiver) e Task Spawning
├── ffi/             # Funções exportadas via C-ABI
│   ├── mod.rs       # Registro global
│   ├── alloc.rs     # kata_rt_alloc_local, kata_rt_alloc_shared
│   ├── channel.rs   # kata_rt_chan_create, kata_rt_chan_send, kata_rt_chan_recv
│   ├── math.rs      # Funções de fallback puro (kata_rt_add_int, kata_rt_safe_div)
│   └── system.rs    # kata_rt_print_str, kata_rt_panic, kata_rt_now
└── task.rs          # Definição do contexto da Action (Task State Machine e Local Arena)
```

## 6. Entregáveis da Fase 5

1.  **Motor de Inicialização:** Função `kata_rt_boot` que configura o `tokio::Runtime` (work-stealing, multi-thread) e inicia o loop de eventos.
2.  **Infraestrutura de Memória:**
    *   Estrutura de contexto vinculada a cada *Green Thread* contendo um `BumpAllocator` (`bumpalo`).
    *   Interface unificada de alocação que aceita ponteiros opacos e devolve endereços.
3.  **Motor CSP (Canais e Tarefas):**
    *   Implementação atômica das filas de comunicação (Rendezvous, Queue, Broadcast) com abstração de tipos.
    *   Sistema de injeção e suspensão de *Futures* compatível com a compilação gerada na Fase 6.
4.  **Biblioteca FFI Padrão (StdLib Bindings):** Implementação em Rust de todas as funções matemáticas e utilitárias declaradas em `src/core/*.kata` marcadas com `@ffi`.
5.  **Testes de Integração (Runtime Bruto):** Suíte de testes validando se a alocação em Arena é limpa com segurança, e se as tarefas concorrentes (CSP) não vazam memória ao finalizar.
