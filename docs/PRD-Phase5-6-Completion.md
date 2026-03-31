# PRD: Completude das Fases 5 e 6

**Status:** Draft
**Data:** 2026-03-31
**Autor:** Análise do código atual
**Prioridade:** Alta

---

## 1. Visão Geral

Este documento descreve os requisitos necessários para completar as Fases 5 (Runtime) e 6 (Backend) do compilador Kata-lang. Atualmente, estas fases estão parcialmente implementadas: o código existe mas não está integrado ao pipeline de compilação final.

### 1.1 Situação Atual

| Componente | Implementado | Integrado | Status |
|------------|-------------|-----------|--------|
| Runtime Tokio | ✅ | ❌ | Código existe em `kata_rt/` mas não é linkado |
| Canais CSP | ✅ | ❌ | Implementados em Rust, sem codegen |
| Arenas/Bump Allocator | ✅ | ❌ | Existe, sem integração |
| FFI Primitives | ⚠️ | ❌ | Shim C duplicado no linker |
| Cranelift Codegen | ⚠️ | ✅ | MVP básico funcional |
| Actions como Futures | ❌ | ❌ | Não implementado |
| CSP no Backend | ❌ | ❌ | Sem tradução de `!>`, `<!` |

### 1.2 Objetivo

Transformar o compilador MVP atual em um sistema AOT completo capaz de gerar binários standalone com:
- Runtime integrado (linkagem dinâmica/estática)
- Concorrência CSP real via canais
- Actions compiladas como máquinas de estado assíncronas

---

## 2. Fase 5: Runtime Integration

### 2.1 Problema Central

O linker atual (`src/codegen/linker.rs`) gera um arquivo C inline com funções de runtime duplicadas, ao invés de linkar com o módulo `kata_rt` compilado em Rust.

**Código atual problemático:**
```rust
// linker.rs:5-55
let main_c_content = r#"
#include <stdio.h>
// ... funções duplicadas ...
"#;
std::fs::write(main_c_path, main_c_content)?;
```

### 2.2 Requisitos

#### REQ-5.1: Linkagem com kata_rt compilado

**Prioridade:** P0 (Crítico)

**Descrição:** O linker deve compilar `kata_rt` como uma biblioteca estática (`.a`) e linká-la ao binário final, eliminando o shim C duplicado.

**Critérios de Aceitação:**
- [ ] `kata_rt` compilado como `libkata_rt.a`
- [ ] Linker invoca `cc` com `-L<path> -lkata_rt`
- [ ] Shim C removido ou reduzido a apenas `main()`
- [ ] Funções FFI em `types.kata` apontam para símbolos do `kata_rt`

**Implementação sugerida:**

```rust
// linker.rs (novo)
pub fn link_executable(object_file: &str, output_bin: &str) -> Result<(), String> {
    // 1. Localizar libkata_rt.a
    let kata_rt_lib = find_kata_rt_library()?;

    // 2. Gerar entrypoint C mínimo
    let entrypoint = generate_minimal_entrypoint();

    // 3. Linkar
    let status = Command::new("cc")
        .arg(entrypoint)
        .arg(object_file)
        .arg("-L").arg(kata_rt_lib.parent())
        .arg("-lkata_rt")
        .arg("-o").arg(output_bin)
        .status()?;

    Ok(())
}
```

---

#### REQ-5.2: Bootstrap do Runtime

**Prioridade:** P0 (Crítico)

**Descrição:** O runtime deve inicializar o Tokio antes de executar a `main!` action.

**Critérios de Aceitação:**
- [ ] `kata_rt_boot` inicializa `tokio::runtime::Runtime`
- [ ] `kata_main` é executada dentro de `rt.block_on()`
- [ ] Suporte a múltiplas threads (work-stealing scheduler)

**Implementação atual:**
```rust
// kata_rt/mod.rs:11-27
pub extern "C" fn kata_rt_boot(main_action: extern "C" fn()) {
    let rt = Runtime::new().expect("...");
    rt.block_on(async {
        tokio::task::spawn_blocking(move || {
            main_action();  // PROBLEMA: síncrono, não é Future
        }).await.unwrap();
    });
}
```

**Necessário:** Actions devem ser compiladas como Futures (ver REQ-6.2).

---

#### REQ-5.3: FFI Completa

**Prioridade:** P1 (Alta)

**Descrição:** Todas as funções FFI referenciadas em `types.kata` devem estar implementadas em `kata_rt/ffi/`.

**Funções existentes vs. necessárias:**

| Função | Arquivo | Status |
|--------|---------|--------|
| `kata_rt_add_int` | `ffi/math.rs` | ✅ |
| `kata_rt_sub_int` | `ffi/math.rs` | ✅ |
| `kata_rt_mul_int` | `ffi/math.rs` | ✅ |
| `kata_rt_div_int` | `ffi/math.rs` | ✅ |
| `kata_rt_eq_int` | `ffi/math.rs` | ✅ |
| `kata_rt_gt_int` | `ffi/math.rs` | ✅ |
| `kata_rt_lt_int` | `ffi/math.rs` | ✅ |
| `kata_rt_int_to_str` | `ffi/system.rs` | ✅ |
| `kata_rt_flt_to_str` | `ffi/system.rs` | ✅ |
| `kata_rt_concat_text` | `ffi/system.rs` | ❌ Faltando |
| `kata_rt_bool_to_str` | `ffi/system.rs` | ❌ Faltando |
| `kata_rt_eq_generic` | `ffi/system.rs` | ⚠️ Stub |

**Critérios de Aceitação:**
- [ ] Todas as funções FFI em `types.kata` implementadas
- [ ] Funções exportadas com `#[no_mangle] pub extern "C"`
- [ ] Testes unitários para cada função FFI

---

#### REQ-5.4: Canais CSP no Runtime

**Prioridade:** P1 (Alta)

**Descrição:** O runtime deve expor APIs C-ABI para criação e operação de canais.

**APIs necessárias:**

```c
// kata_rt/ffi/channel.rs
#[no_mangle]
pub extern "C" fn kata_rt_channel_create() -> ChannelHandle;

#[no_mangle]
pub extern "C" fn kata_rt_queue_create(size: usize) -> ChannelHandle;

#[no_mangle]
pub extern "C" fn kata_rt_broadcast_create(size: usize) -> BroadcastHandle;

#[no_mangle]
pub extern "C" fn kata_rt_channel_send(handle: ChannelHandle, data: *mut u8) -> bool;

#[no_mangle]
pub extern "C" fn kata_rt_channel_recv(handle: ChannelHandle) -> *mut u8;

#[no_mangle]
pub extern "C" fn kata_rt_channel_recv_try(handle: ChannelHandle) -> *mut u8;

#[no_mangle]
pub extern "C" fn kata_rt_channel_free(handle: ChannelHandle);
```

**Critérios de Aceitação:**
- [ ] Handles opacos para canais
- [ ] Envio bloqueante para rendezvous
- [ ] Receção bloqueante e não-bloqueante
- [ ] Thread-safety garantida

---

#### REQ-5.5: Gerenciamento de Memória (Arenas + ARC)

**Prioridade:** P1 (Alta)

**Descrição:** Implementar alocação em arenas locais e ARC para dados compartilhados.

**APIs necessárias:**

```c
// kata_rt/ffi/alloc.rs
#[no_mangle]
pub extern "C" fn kata_rt_arena_alloc(size: usize, align: usize) -> *mut u8;

#[no_mangle]
pub extern "C" fn kata_rt_arena_clear();

#[no_mangle]
pub extern "C" fn kata_rt_arc_alloc(size: usize) -> *mut u8;

#[no_mangle]
pub extern "C" fn kata_rt_arc_incr(ptr: *mut u8);

#[no_mangle]
pub extern "C" fn kata_rt_arc_decr(ptr: *mut u8) -> bool; // retorna true se deva liberar
```

**Critérios de Aceitação:**
- [ ] Arena por Action (thread-local)
- [ ] ARC para dados enviados por canais
- [ ] Liberação automática ao fim da Action
- [ ] Escape Analysis integrado (REQ-4.x já implementado)

---

## 3. Fase 6: Backend Cranelift

### 3.1 Problema Central

O backend atual compila Actions como funções síncronas simples, não como Futures/máquinas de estado. Isso impede I/O assíncrono real e a integração com o scheduler Tokio.

### 3.2 Estratégia de Implementação: Wrapper Rust + Cranelift

Para aproveitar o ecossistema async do Rust sem reimplementar tudo no Cranelift, adotamos uma **arquitetura em camadas**:

```
┌─────────────────────────────────────────────────────────────┐
│                     Tokio Runtime                            │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              ActionFuture (Rust)                      │   │
│  │  impl Future {                                        │   │
│  │      poll() -> Poll {                                 │   │
│  │          call FFI -> action_step(state, waker)        │   │
│  │      }                                                │   │
│  │  }                                                    │   │
│  └─────────────────────────────────────────────────────┘   │
│                          │ FFI                               │
│                          ▼                                   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │            Código Cranelift Gerado                    │   │
│  │  action_step(state_ptr, waker) -> StepResult          │   │
│  │                                                       │   │
│  │  - Executa até próximo ponto de suspensão            │   │
│  │  - Retorna { Done, Pending }                          │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Vantagens desta abordagem:**
- Usa `Future` trait e wakers nativos do Tokio
- Cranelift só gera código de negócio, não infraestrutura async
- Menor complexidade, debugging mais fácil
- Evolução gradual para implementação mais otimizada

### 3.3 Requisitos

#### REQ-6.1: Suporte a Coleções no Backend

**Prioridade:** P1 (Alta)

**Descrição:** O codegen deve suportar alocação de Listas, Arrays, Tuplas e Dicionários.

**Estado atual:**
```rust
// codegen/expr.rs:225-229
TExpr::Tuple(exprs, _, _) => {
    if exprs.is_empty() {
        Ok(self.builder.ins().iconst(types::I32, 0))
    } else {
        Err(format!("Tuplas com elementos não suportadas no MVP"))
    }
}
```

**Critérios de Aceitação:**
- [ ] Tuplas alocadas na Arena
- [ ] Listas (linked lists) com structural sharing
- [ ] Arrays contíguos alocados via `kata_rt_arena_alloc`
- [ ] Dicionários (HAMT) - pode ser fase 6.5

**Implementação sugerida:**

```rust
// codegen/collections.rs (novo)
pub fn translate_tuple(&mut self, exprs: &[Spanned<TExpr>]) -> Result<Value, String> {
    if exprs.is_empty() {
        return Ok(self.builder.ins().iconst(types::I32, 0));
    }

    // Calcular tamanho total
    let total_size: usize = exprs.iter().map(|e| self.size_of_expr(e)).sum();

    // Alocar na Arena
    let ptr = self.call_ffi("kata_rt_arena_alloc", &[
        self.builder.ins().iconst(types::I64, total_size as i64),
        self.builder.ins().iconst(types::I64, 8), // align
    ]);

    // Armazenar cada elemento
    let mut offset = 0;
    for expr in exprs {
        let val = self.translate_expr(expr)?;
        self.builder.ins().store(ptr, val, offset);
        offset += self.size_of_expr(expr);
    }

    Ok(ptr)
}
```

---

#### REQ-6.2: Actions como Futures (Wrapper Rust)

**Prioridade:** P0 (Crítico)

**Descrição:** Actions são compiladas como funções `step` que executam até o próximo ponto de suspensão, envoltas por um `Future` Rust que integra com Tokio.

##### 6.2.1 Tipos Compartilhados (Rust)

```rust
// kata_rt/future.rs

/// Resultado de um passo de execução
#[repr(C)]
pub enum StepResult {
    /// Execução completa, retorna valor
    Done { value: u64 },
    /// Aguardando I/O, waker já registrado
    Pending,
}

/// Handle para uma Action compilada
#[repr(C)]
pub struct ActionHandle {
    /// Função step gerada pelo Cranelift
    pub step_fn: extern "C" fn(*mut u8, *const WakerRaw) -> StepResult,
    /// Tamanho do estado em bytes
    pub state_size: usize,
}

/// Estado de uma Action em execução
pub struct ActionState {
    /// Buffer de estado alocado
    data: Box<[u8]>,
    /// Handle da Action
    handle: ActionHandle,
}

/// Wrapper que implementa Future para Tokio
pub struct ActionFuture {
    state: Option<ActionState>,
}

impl Future for ActionFuture {
    type Output = u64;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state = self.state.as_mut().expect("polled after completion");

        // Waker do Tokio convertido para formato FFI
        let waker = WakerRaw::from_tokio(cx.waker());

        // Chama código Cranelift
        let result = (state.handle.step_fn)(
            state.data.as_mut_ptr(),
            &waker as *const _ as *const WakerRaw
        );

        match result {
            StepResult::Done { value } => {
                self.state = None;
                Poll::Ready(value)
            }
            StepResult::Pending => {
                // Waker já foi registrado pelo código Cranelift
                Poll::Pending
            }
        }
    }
}
```

##### 6.2.2 Layout de Memória do Estado

```
ActionState (state_ptr):
┌────────────────────┬───────────────────────────────────┐
│ stage: u8          │ offset 0                          │
├────────────────────┼───────────────────────────────────┤
│ padding: [7 bytes] │ offset 1-7                        │
├────────────────────┼───────────────────────────────────┤
│ args/params        │ offset 8+ (depende da assinatura) │
├────────────────────┼───────────────────────────────────┤
│ variáveis locais   │ (alocado conforme necessário)     │
└────────────────────┴───────────────────────────────────┘
```

##### 6.2.3 Código Cranelift Gerado

Para uma Action como:

```kata
action exemplo rx tx
    let a (<! rx)       # Ponto de suspensão
    let b (+ a 10)
    !> tx b              # Ponto de suspensão
```

Geramos:

```llvm
; Função step: executa até await ou fim
define i32 @exemplo_step(i8* %state_ptr, i8* %waker) {
entry:
    %stage_ptr = getelementptr i8, i8* %state_ptr, 0
    %stage = load i8, i8* %stage_ptr

    switch i8 %stage, label %invalid [
        i8 0, label %stage_0
        i8 1, label %stage_1
    ]

stage_0:                                    ; Entrada inicial
    ; Carrega argumentos
    %rx_ptr = getelementptr i8, i8* %state_ptr, 8
    %rx = load i64, i64* %rx_ptr

    ; Tenta receber (chamada FFI async)
    %result = call i64 @kata_rt_channel_recv_async(i64 %rx, i8* %waker)

    ; -1 = Pending (waker registrado no canal)
    %is_pending = icmp eq i64 %result, -1
    br i1 %is_pending, label %return_pending, label %stage_0_continue

stage_0_continue:
    ; Recebeu valor, salva e avança
    %a_ptr = getelementptr i8, i8* %state_ptr, 16
    store i64 %result, i64* %a_ptr

    ; Computa b
    %b = add i64 %result, 10
    %b_ptr = getelementptr i8, i8* %state_ptr, 24
    store i64 %b, i64* %b_ptr

    ; Avança para stage 1
    store i8 1, i8* %stage_ptr
    br label %stage_1

stage_1:
    %tx_ptr = getelementptr i8, i8* %state_ptr, 32
    %tx = load i64, i64* %tx_ptr
    %b_val = load i64, i64* %b_ptr

    ; Tenta enviar
    %sent = call i1 @kata_rt_channel_send_async(i64 %tx, i64 %b_val, i8* %waker)
    br i1 %sent, label %stage_1_done, label %return_pending

stage_1_done:
    store i8 255, i8* %stage_ptr      ; Marca como completo
    ret i32 0                          ; StepResult::Done

return_pending:
    ret i32 1                          ; StepResult::Pending

invalid:
    unreachable
}
```

##### 6.2.4 API FFI Async para Canais

```rust
// kata_rt/ffi/channel.rs

/// Recebe de canal de forma assíncrona
/// Retorna:
///   - valor >= 0: valor recebido
///   - -1: Pending (waker registrado)
///   - -2: Canal fechado
#[no_mangle]
pub extern "C" fn kata_rt_channel_recv_async(
    handle: u64,
    waker: *const WakerRaw,
) -> i64 {
    let rx = unsafe { &*(handle as *const mpsc::Receiver<Box<u8>>) };

    match rx.try_recv() {
        Ok(value) => value.as_ref() as *const _ as i64,
        Err(TryRecvError::Empty) => {
            // Registra waker para notificação
            let waker_safe = unsafe { (*waker).to_tokio() };
            rx.set_waker(waker_safe);
            -1  // Pending
        }
        Err(TryRecvError::Closed) => -2,
    }
}

/// Envia para canal de forma assíncrona
/// Retorna:
///   - true: enviado com sucesso
///   - false: Pending (waker registrado)
#[no_mangle]
pub extern "C" fn kata_rt_channel_send_async(
    handle: u64,
    value: u64,
    waker: *const WakerRaw,
) -> bool {
    let tx = unsafe { &*(handle as *const mpsc::Sender<Box<u8>>) };

    match tx.try_send(unsafe { Box::from_raw(value as *mut u8) }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            let waker_safe = unsafe { (*waker).to_tokio() };
            tx.set_waker(waker_safe);
            false
        }
        Err(TrySendError::Closed(_)) => false,
    }
}
```

##### 6.2.5 Critérios de Aceitação

- [ ] `ActionHandle` e `ActionFuture` implementados em Rust
- [ ] Função `step` gerada pelo Cranelift com switch de estados
- [ ] Layout de memória do estado documentado
- [ ] Pontos de suspensão em: `<!`, `!>`, `sleep!`
- [ ] Wakers registrados corretamente para retomar execução
- [ ] Integração com `tokio::spawn`
- [ ] Teste: Action com I/O não bloqueia outras Actions

---

#### REQ-6.3: Tradução de Operações CSP

**Prioridade:** P0 (Crítico)

**Descrição:** O backend deve traduzir `!>`, `<!`, e `<!?` para chamadas FFI do runtime.

**Estado atual:**
```rust
// codegen/expr.rs - TExpr::ChannelSend/Recv não tratados
_ => Err(format!("Expressão não suportada no MVP: {:?}", e))
```

**Traduções necessárias:**

| Expressão Kata | Tradução Cranelift |
|---------------|-------------------|
| `<! channel` | `call kata_rt_channel_recv_async(channel, waker)` |
| `!> channel value` | `call kata_rt_channel_send_async(channel, value, waker)` |
| `<!? channel` | `call kata_rt_channel_recv_try(channel)` (não bloqueante) |
| `fork!(action)` | `call kata_rt_spawn(action_handle)` |
| `@parallel fork!` | `call kata_rt_spawn_blocking(action_handle)` |

**Implementação em codegen/csp.rs:**

```rust
pub fn translate_channel_recv(
    &mut self,
    channel: &Spanned<TExpr>,
    is_blocking: bool,
) -> Result<(Value, bool), String> {
    let chan_val = self.translate_expr(channel)?;
    let waker = self.get_current_waker();

    if is_blocking {
        // Chamada async - pode retornar Pending
        let result = self.builder.ins().call(
            self.get_ffi("kata_rt_channel_recv_async"),
            &[chan_val, waker]
        );

        let result_val = self.builder.inst_results(result)[0];

        // Verificar se é Pending (-1)
        let is_pending = self.builder.ins().icmp(
            IntCC::Equal,
            result_val,
            self.builder.ins().iconst(types::I64, -1)
        );

        // Se pending, salvar estado e retornar
        let pending_block = self.builder.create_block();
        let continue_block = self.builder.create_block();

        self.builder.ins().brif(is_pending, pending_block, &[], continue_block, &[]);

        // Bloco pending: salvar stage e retornar
        self.builder.switch_to_block(pending_block);
        self.save_current_stage();
        self.return_step_pending();

        // Bloco continue: valor recebido
        self.builder.switch_to_block(continue_block);

        Ok((result_val, false)) // não é pending
    } else {
        // try_recv - não bloqueia
        let result = self.builder.ins().call(
            self.get_ffi("kata_rt_channel_recv_try"),
            &[chan_val]
        );
        Ok((self.builder.inst_results(result)[0], false))
    }
}
```

**Critérios de Aceitação:**
- [ ] `TExpr::ChannelSend` traduzido com waker
- [ ] `TExpr::ChannelRecv` traduzido com waker
- [ ] `TExpr::ChannelRecvNonBlock` traduzido (síncrono)
- [ ] Estado salvo corretamente antes de retornar Pending
- [ ] Variáveis restauradas ao retomar execução

---

#### REQ-6.4: Select Non-Determinístico

**Prioridade:** P2 (Média)

**Descrição:** Suportar `select` para multiplexagem de canais.

**Sintaxe Kata:**
```kata
select
    case (<! rx_a) -> valor_a:
        ...
    case (<! rx_b) -> valor_b:
        ...
    timeout 1000:
        ...
```

**Tradução via wrapper Rust:**

O `select` pode ser implementado primeiramente via FFI para o Rust, que usa `tokio::select!` internamente:

```rust
// kata_rt/ffi/select.rs
#[no_mangle]
pub extern "C" fn kata_rt_select_start(
    num_cases: usize,
    cases: *const SelectCase,
    timeout_ms: u64,
    waker: *const WakerRaw,
) -> i32 {
    // Registra interesse em múltiplos canais
    // Retorna -1 se pending, ou índice do case que resolveu
}
```

**Critérios de Aceitação:**
- [ ] Parser para `select` em Actions (stmt.rs)
- [ ] FFI `kata_rt_select_start` implementado
- [ ] Tradução Cranelift para chamada select
- [ ] Suporte a `timeout`
- [ ] Desestruturação do resultado no branch correto

---

#### REQ-6.5: Funções de Alta Ordem

**Prioridade:** P2 (Média)

**Descrição:** Suportar funções como valores de primeira classe.

**Estado atual:**
```rust
// codegen/expr.rs:186
_ => return Err("Chamadas anonimas ou high-order nao suportadas no MVP.")
```

**Critérios de Aceitação:**
- [ ] Lambdas como closures (captura de ambiente)
- [ ] Passagem de funções como argumentos
- [ ] Retorno de funções
- [ ] Chamada de função armazenada em variável

**Nota:** Este requisito pode ser adiado para uma fase posterior, pois funções de alta ordem não são essenciais para o MVP de Actions assíncronas.

---

## 4. Dependências e Ordem de Implementação

### 4.1 Grafo de Dependências

```
Sprint 1: Fundação
─────────────────
REQ-5.1 (Linkagem libkata_rt.a)
    │
    ▼
REQ-5.3 (FFI Completa)

Sprint 2: Runtime Core
─────────────────────
REQ-5.4 (Canais Async FFI) ─────┐
    │                            │
    ▼                            ▼
REQ-5.5 (Arenas + ARC)     REQ-5.2 (Bootstrap Actions)

Sprint 3: Actions Assíncronas
─────────────────────────────
REQ-6.2 (ActionFuture Wrapper) ───► REQ-6.3 (CSP Codegen)
    │                                       │
    ▼                                       ▼
REQ-6.1 (Coleções)                    Testes de Integração

Sprint 4: Features Avançadas
────────────────────────────
REQ-6.4 (Select)
REQ-6.5 (High-Order Functions)
```

### 4.2 Fases de Implementação

#### Sprint 1: Fundação (Semana 1-2)

**Objetivo:** Binário linkado com runtime Rust funcional.

| Tarefa | Requisito | Entregável |
|--------|-----------|------------|
| Compilar kata_rt como `.a` | REQ-5.1 | `libkata_rt.a` gerado |
| Atualizar linker | REQ-5.1 | Shim C mínimo |
| Implementar FFIs faltantes | REQ-5.3 | Todas as funções em `types.kata` |
| Testar linkagem | REQ-5.1 | Binário simples funciona |

#### Sprint 2: Runtime Core (Semana 3-4)

**Objetivo:** Canais e memória funcionais via FFI.

| Tarefa | Requisito | Entregável |
|--------|-----------|------------|
| FFI async para canais | REQ-5.4 | `kata_rt_channel_recv_async` |
| Waker management | REQ-5.4 | `WakerRaw` conversão |
| Arenas thread-local | REQ-5.5 | `kata_rt_arena_*` |
| ARC para canais | REQ-5.5 | `kata_rt_arc_*` |
| Bootstrap melhorado | REQ-5.2 | Actions rodam no Tokio |

#### Sprint 3: Actions Assíncronas (Semana 5-7)

**Objetivo:** Actions compiladas como Futures reais.

| Tarefa | Requisito | Entregável |
|--------|-----------|------------|
| `ActionFuture` wrapper | REQ-6.2 | Rust struct + impl Future |
| `ActionHandle` + `ActionState` | REQ-6.2 | Tipos compartilhados |
| Gerar função `step` | REQ-6.2 | Cranelift gera switch |
| Traduzir `<!` no codegen | REQ-6.3 | TExpr::ChannelRecv |
| Traduzir `!>` no codegen | REQ-6.3 | TExpr::ChannelSend |
| Teste: 2 Actions comunicando | - | E2E funcionando |

#### Sprint 4: Features Avançadas (Semana 8-10)

**Objetivo:** Features secundárias e otimizações.

| Tarefa | Requisito | Entregável |
|--------|-----------|------------|
| Tuplas no backend | REQ-6.1 | `translate_tuple` |
| Listas no backend | REQ-6.1 | Linked list com sharing |
| Select FFI | REQ-6.4 | `kata_rt_select_*` |
| Select codegen | REQ-6.4 | Tradução de stmt |
| Closures (opcional) | REQ-6.5 | Captura de ambiente |

### 4.3 Marcos de Validação

**Marco 1 (Fim Sprint 1):**
```bash
$ kata build examples/hello.kata
$ ./hello
Hello, World!  # Usando FFI kata_rt_print_str
```

**Marco 2 (Fim Sprint 2):**
```bash
$ kata build examples/channels_basic.kata
$ ./channels_basic
# Programa com canais funciona (ainda síncrono)
```

**Marco 3 (Fim Sprint 3):**
```bash
$ kata build examples/async_pingpong.kata
$ ./async_pingpong
# Duas Actions comunicando assincronamente
# sleep! não bloqueia outra Action
```

**Marco 4 (Fim Sprint 4):**
```bash
$ kata build examples/select_example.kata
$ ./select_example
# Select funcionando com timeout
```

---

## 5. Critérios de Aceitação Gerais

### 5.1 Testes de Integração

- [ ] Compilar e executar `action main` com I/O básico
- [ ] Comunicação entre duas Actions via canal
- [ ] Action com `sleep!` não bloqueia outras Actions
- [ ] Binário standalone sem dependências externas (exceto libc)
- [ ] Múltiplas Actions concorrentes executam em paralelo real

### 5.2 Benchmarks

- [ ] Overhead de spawn de Action < 10μs (incluindo wrapper Rust)
- [ ] Latência de canal rendezvous < 500ns
- [ ] Throughput de canal > 500K msgs/s
- [ ] Overhead de FFI call < 50ns

### 5.3 Documentação

- [ ] Atualizar ROADMAP.md com progresso
- [ ] Documentar API FFI do kata_rt
- [ ] Documentar layout de memória de ActionState
- [ ] Exemplos de código funcionais em `examples/`

---

## 6. Riscos e Mitigações

| Risco | Probabilidade | Impacto | Mitigação |
|-------|--------------|---------|-----------|
| FFI overhead | Baixa | Médio | Wrapper Rust é mínimo, benchmark antes de otimizar |
| Bugs de concorrência no runtime | Média | Crítico | Testes com ThreadSanitizer, Miri |
| Layout de memória inconsistente | Média | Alto | Testes de alinhamento, assertions em debug |
| Performance de canais | Baixa | Médio | Usar implementações testadas do Tokio |
| Waker não chamado | Média | Alto | Timeout de segurança, logs de debug |
| Compatibilidade FFI C-ABI | Média | Alto | Testar em Linux/macOS/Windows |

---

## 7. Evolução Futura: Otimização com Future ABI Nativo

Após completar a implementação com Wrapper Rust (Opção C), podemos otimizar removendo a camada de FFI:

### 7.1 Opção B: Future ABI Direto no Cranelift (Futuro)

Quando o wrapper Rust estiver estável, podemos gerar código Cranelift que implementa diretamente a ABI de `Future::poll`:

```rust
// Geração direta de vtable Future
struct FutureVTable {
    poll: extern "C" fn(*mut u8, *mut Context) -> Poll<u64>,
    drop: extern "C" fn(*mut u8),
    size: usize,
    align: usize,
}

// Cranelift gera a função poll diretamente,
// sem passar pelo wrapper Rust
```

**Benefícios:**
- Elimina ~10-20ns de overhead por poll
- Menos código Rust embutido no binário

**Quando implementar:**
- Após wrapper estar estável por 2+ sprints
- Se profiling mostrar FFI como bottleneck

---

## 8. Apêndice: Estrutura de Arquivos

### 8.1 Novos Arquivos Necessários

```
src/
├── codegen/
│   ├── action_step.rs              # REQ-6.2 - Gera função step
│   ├── action_state_layout.rs      # REQ-6.2 - Calcula layout de estado
│   ├── collections.rs               # REQ-6.1 - Tuplas, Listas, Arrays
│   └── csp.rs                       # REQ-6.3 - Tradução de canais
│
├── kata_rt/
│   ├── future.rs                    # REQ-6.2 - ActionFuture, ActionHandle
│   ├── waker.rs                     # REQ-6.2 - WakerRaw conversão
│   ├── ffi/
│   │   ├── channel.rs              # REQ-5.4 - Canais async FFI
│   │   ├── alloc.rs                # REQ-5.5 - Arenas + ARC
│   │   └── select.rs                # REQ-6.4 - Select FFI
│   └── tests/
│       └── integration_async.rs    # Testes de Actions assíncronas
│
├── build.rs                         # Compilar kata_rt como .a
│
└── examples/
    ├── hello.kata                   # Exemplo básico
    ├── channels_basic.kata          # Canais síncronos
    ├── async_pingpong.kata          # Actions assíncronas
    └── select_example.kata          # Select multiplexado
```

### 8.2 Arquivos a Modificar

```
src/
├── codegen/
│   ├── linker.rs          # REQ-5.1 - Linkar libkata_rt.a
│   ├── expr.rs            # REQ-6.1, REQ-6.3 - Coleções e canais
│   └── translator.rs      # REQ-6.2 - Chamar action_step compiler
│
├── kata_rt/
│   ├── mod.rs             # REQ-5.2 - Bootstrap melhorado
│   └── ffi/
│       ├── math.rs        # REQ-5.3 - Completar FFI
│       └── system.rs      # REQ-5.3 - Completar FFI
│
└── main.rs                # Atualizar para usar novo pipeline
```

### 8.3 Dependências Cargo a Adicionar

```toml
# Cargo.toml
[dependencies]
# ... existing ...
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }
futures = "0.3"
parking_lot = "0.12"  # Para locks performáticos

[build-dependencies]
cc = "1.0"  # Para compilar entrypoint C mínimo

# kata_rt como biblioteca separada
[lib]
name = "kata_rt"
path = "src/kata_rt/mod.rs"
crate-type = ["staticlib", "rlib"]
```

---

## 9. Referências

- [Cranelift Documentation](https://cranelift.readthedocs.io/)
- [Tokio Runtime Internals](https://tokio.rs/blog/2019-10-scheduler)
- [Futures API - Rust](https://doc.rust-lang.org/std/future/trait.Future.html)
- [Waker and Context - Rust](https://doc.rust-lang.org/std/task/struct.Waker.html)
- [CSP Papers - Hoare](https://www.cs.cmu.edu/~crary/819-f09/Hoare78.pdf)
- [Async Book - Rust](https://rust-lang.github.io/async-book/)
- ROADMAP.md - Definição original das fases