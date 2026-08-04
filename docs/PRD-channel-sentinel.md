# PRD: Channel Sentinel — Eliminar Colisão WOULD_BLOCK com Valor de Usuário

## Status

**Status:** ⏸️ Pendente (não iniciado)
**Data:** 2026-08-04
**Depende de:** PRD-csp-channels (canais rendezvous, queue, broadcast), PRD-socket-io (teste `socket_connected_listen_fails` que expôs o bug)
**Habilita:** Enviar qualquer `Int` (incluindo `-1`) por canal sem deadlock

## 1. Objetivo

Eliminar a colisão entre o sentinel `WOULD_BLOCK = -1` e valores de usuário
enviados por canais. Hoje, enviar `-1` por um canal rendezvous ou queue
deadlocka: `try_recv` retorna `-1` (o valor real), mas `kata_rt_channel_recv`
interpreta como "canal vazio" e suspende o fiber indefinidamente.

## 2. Motivação

### 2.1. O bug

O runtime usa `WOULD_BLOCK = -1` como sentinel para "operação não pode
completar, suspender". O SMI tagging é `encode_smi(val) = (val << 1) | 1`, e
`encode_smi(-1) = -1`. O valor Kata5 `-1` é indistinguível do sentinel no nível
do runtime.

**Fluxo do deadlock:**

1. Servidor envia `-1` via `tx !> -1` → `try_send` coloca `-1` no slot → OK
2. `wake_pass` chama `can_recv(handle)` → `slot.is_some()` → `true` → acorda main
3. Main chama `try_recv(handle)` → pega `-1` do slot → retorna `-1`
4. `kata_rt_channel_recv` verifica `if result != WOULD_BLOCK` → `-1 != -1` → `false`
5. Main suspende novamente (loop volta ao início)
6. `try_recv` agora retorna `WOULD_BLOCK` (slot vazio) → suspende
7. Scheduler: sem deadline, sem IPC, sem FDs → deadlock

**Confirmação empírica** (2026-08-04, commit `5fbe32f`):

```
[DBG recv] handle=0x... try_recv=-1     ← main pega -1 do slot
[DBG wake_pass] fiber 0 ... can_recv=false  ← wake_pass vê slot vazio
[DBG wake_pass] fiber 0 ... can_recv=true   ← servidor enviou, slot tem -1
[DBG recv] handle=0x... try_recv=-1     ← main pega -1 de novo
[DBG wake_pass] fiber 0 ... can_recv=false  ← slot vazio de novo
kata_rt_run: deadlock: 1 fibers bloqueados sem progresso
```

### 2.2. Impacto

Qualquer programa Kata5 que envie `-1` (Int negativo comum — código de erro,
sentinela de "não encontrado", resultado de `listen!(conn)`) por `channel!()`
ou `queue!()` deadlocka. O bug é do runtime, não do código do usuário.

### 2.3. Por que só canal é afetado

`FILE_WOULD_BLOCK` e `SOCKET_WOULD_BLOCK` também são `-1`, mas `try_select_files`
e `try_select_sockets` retornam **índices** (sempre ≥ 0), não valores de usuário.
O sentinel `-1` ("nenhum handle pronto") nunca colide com um índice válido.

`SELECT_TIMEOUT = -2` é par (LSB = 0), não é SMI-tagged válido. Não colide.

O bug é específico de `kata_rt_channel_recv` / `try_recv`: a única FFI que
retorna um valor de usuário E usa o mesmo sentinel para "vazio".

## 3. Design

### 3.1. Abordagem: out-parameter em `try_recv`

Mudar `try_recv` de "retorna valor OU sentinel" para "retorna bool, valor em
out-parameter":

**Antes:**
```rust
fn try_recv(handle: i64) -> i64 {
    // ...
    match tag {
        TAG_CHANNEL => {
            let mut slot = inner.slot.lock()...;
            if let Some(v) = slot.take() {
                v           // valor de usuário
            } else {
                WOULD_BLOCK  // sentinel — colide se v == -1
            }
        }
        // ...
    }
}

// Caller:
loop {
    let result = try_recv(handle);
    if result != WOULD_BLOCK {   // ← falha se result == -1 (valor real)
        return result;
    }
    suspend();
}
```

**Depois:**
```rust
/// Tenta receber sem bloquear. Retorna `true` se há valor (escrito em
/// `out`), `false` se o canal está vazio.
///
/// # Safety
/// `handle` deve ser um handle válido. `out` deve apontar para i64 válido.
fn try_recv(handle: i64, out: *mut i64) -> bool {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        match tag {
            TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                let mut slot = inner.slot.lock()...;
                if let Some(v) = slot.take() {
                    *out = v;
                    true
                } else {
                    false
                }
            }
            TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                let mut buffer = inner.buffer.lock()...;
                if let Some(v) = buffer.pop_front() {
                    *out = v;
                    true
                } else {
                    false
                }
            }
            TAG_BROADCAST_RX => {
                let rx = &*(ptr as *const BroadcastReceiver);
                let inner = &*rx.inner;
                let ver = inner.version.lock()...;
                if *ver > rx.last_seen_version {
                    let val = inner.value.lock()...;
                    *out = val.unwrap_or(0);
                    true
                } else {
                    false
                }
            }
            TAG_IPC_CHANNEL => {
                // IPC: desserializar valor. Se vazio, false.
                // ... ver seção 3.3 ...
            }
            _ => false,
        }
    }
}
```

### 3.2. Mudança na FFI `kata_rt_channel_recv`

A assinatura **não muda** — continua `(handle: i64) -> i64`. A mudança é
interna:

```rust
pub extern "C" fn kata_rt_channel_recv(handle: i64) -> i64 {
    loop {
        let mut out: i64 = 0;
        let has_value = try_recv(handle, &mut out);
        if has_value {
            return out;   // valor de usuário — pode ser qualquer i64
        }
        // Não há dado. Se há fiber em execução, suspende.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(YieldReason::WaitingOnChannel(handle));
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — pode ser child após spawn!
            // (processo OS sem scheduler).
            if tag_of(handle) == TAG_IPC_CHANNEL {
                unsafe { super::ipc::block_until_readable(handle); }
                continue;
            }
            return WOULD_BLOCK;  // sentinel para "fora de fiber, canal vazio"
        }
        // Fiber resumido — scheduler acredita que há dado. Tentar novamente.
    }
}
```

O `WOULD_BLOCK` **permanece** no retorno de `kata_rt_channel_recv` para o path
"fora de fiber, canal vazio". Esse path é atingido apenas em testes unitários
ou child processes após `spawn!` — nunca pelo codegen (que sempre executa dentro
de fiber). O codegen não compara o resultado de `kata_rt_channel_recv` com
`WOULD_BLOCK`; ele usa o valor retornado diretamente.

### 3.3. `try_send` — sem mudança

`try_send` retorna `OK` (0) ou `WOULD_BLOCK` (-1) — nunca retorna valor de
usuário. Sem colisão. Permanece inalterado.

### 3.4. `can_recv` / `can_send` — sem mudança

`can_recv` e `can_send` verificam prontidão **sem consumir**. Não retornam
valores de usuário. Permanecem inalterados.

### 3.5. `select.rs` — sem mudança

`try_select` (canais no select) usa `can_recv` (sem consumo). Não chama
`try_recv`. Permanece inalterado.

### 3.6. `FILE_WOULD_BLOCK` / `SOCKET_WOULD_BLOCK` — sem mudança

Retornam índices (≥ 0), não valores de usuário. Sem colisão. Permanecem
inalterados.

### 3.7. IPC — `try_recv` em `ipc.rs`

O path IPC (`TAG_IPC_CHANNEL`) é o mais complexo. Hoje, `try_ipc_recv`
desserializa o valor do pipe. Retorna `WOULD_BLOCK` se não há dado.

Com a Opção B, `try_recv` com `TAG_IPC_CHANNEL` precisa:
1. Verificar se há dado no pipe (poll POLLIN non-blocking)
2. Se sim, desserializar e escrever em `out`, retornar `true`
3. Se não, retornar `false`

A lógica de desserialização existe em `ipc.rs` — precisa ser adaptada para
escrever em `out` em vez de retornar o valor. Mudança mecânica.

## 4. Escopo

### 4.1. O que muda

| Arquivo | Mudança |
|---|---|
| `crates/kata-rt/src/channel/ops.rs` | `try_recv` muda de `-> i64` para `(handle, *mut i64) -> bool`. `kata_rt_channel_recv` adaptado. |
| `crates/kata-rt/src/channel/ipc.rs` | `try_ipc_recv` (ou equivalente) adaptado para out-parameter. |
| `crates/kata-rt/src/channel/select.rs` | Sem mudança — usa `can_recv`, não `try_recv`. |

### 4.2. O que não muda

| Componente | Razão |
|---|---|
| `kata_rt_channel_send` / `try_send` | Retorna OK/WOULD_BLOCK, não valor de usuário |
| `can_recv` / `can_send` | Verifica prontidão sem consumo |
| `FILE_WOULD_BLOCK` / `SOCKET_WOULD_BLOCK` | Retornam índices, não valores |
| `SELECT_TIMEOUT` | Já é -2 (par, não colide) |
| Codegen (`lower_channel_recv`, `lower_select`) | Assinatura FFI não muda |
| `ffi_sigs.rs`, `ffi_registry.rs` | Assinatura FFI não muda |
| Tag system (TAG_CHANNEL, TAG_QUEUE, etc.) | Permanece — dispatch por tag |
| `WOULD_BLOCK` constante | Permanece `-1` — usada por `try_send`, file, socket, e path "fora de fiber" de `recv` |

## 5. Decisões fechadas

### 5.1. Out-parameter em vez de mudar o valor de WOULD_BLOCK

**Decisão:** out-parameter (`*mut i64`) em `try_recv`.
**Alternativa considerada:** mudar `WOULD_BLOCK` para `i64::MIN` (par, fora do
range SMI). Rejeitada porque o out-parameter elimina a classe inteira de
colisão — nenhum valor de usuário pode colidir porque o success/failure é
sinalizado por `bool`, não por inspeção do valor.

### 5.2. Assinatura da FFI `kata_rt_channel_recv` não muda

**Decisão:** `kata_rt_channel_recv(handle: i64) -> i64` permanece.
A mudança é interna ao runtime. O codegen não é tocado.

### 5.3. `WOULD_BLOCK` permanece `-1`

**Decisão:** `WOULD_BLOCK = -1` permanece para `try_send`, `FILE_WOULD_BLOCK`,
`SOCKET_WOULD_BLOCK`, e o path "fora de fiber" de `kata_rt_channel_recv`.
Nenhum desses retorna valores de usuário. Sem colisão.

### 5.4. Tag system permanece

**Decisão:** não migrar para enum. O tag system resolve o problema da FFI
boundary (handle `i64` cru) e permite dispatch sem dereferenciar. A Opção B é
ortogonal ao mecanismo de dispatch.

### 5.5. Canal auxiliar de controle rejeitado

**Decisão:** não adicionar canal auxiliar. Não resolve o bug por si só
(`try_recv` ainda precisaria de sentinel ou out-parameter). Dobraria a
quantidade de canais sem benefício.

## 6. Fases

### Fase 1 — Migrar `try_recv` para out-parameter

1. Mudar assinatura: `fn try_recv(handle: i64, out: *mut i64) -> bool`
2. Adaptar todos os branches (TAG_CHANNEL, TAG_QUEUE, TAG_BROADCAST_RX,
   TAG_IPC_CHANNEL) para escrever em `out` e retornar `bool`
3. Adaptar `kata_rt_channel_recv` para usar nova assinatura
4. `cargo check -p kata-rt` — deve compilar limpo

### Fase 2 — Testes

1. Criar teste E2E: enviar `-1` via `channel!()`, receber, verificar resultado
2. Criar teste E2E: enviar `-1` via `queue!(1)`, receber, verificar resultado
3. Reativar o teste `socket_connected_listen_fails` com `channel!()` (versão
   original que usava `tx !> -1`) — deve passar agora
4. Rodar `cargo test --workspace --no-fail-fast` — 1372 passed, 0 failed,
   5 ignored (pelo menos)

### Fase 3 — Verificação de regressão

1. Rodar todos os testes E2E de canal (`csp_channels_e2e.rs`)
2. Rodar todos os testes E2E de spawn/IPC (`spawn_ipc_e2e.rs`)
3. Rodar todos os testes E2E de select (`select_io_e2e.rs`)
4. Rodar todos os testes E2E de socket (`socket_io_e2e.rs`)
5. Confirmar: nenhum teste novo falha, contagem de testes não diminui

### Fase 4 — Documentação

1. Atualizar `docs/PRD-csp-channels.md` (ou criar seção neste PRD) com a
   mudança de design
2. Atualizar `docs/PRD-socket-io.md` — item 7 (`socket_connected_listen_fails`)
   pode voltar a usar `channel!()` se desejado, ou manter a versão sem canal
3. Atualizar `docs/sintaxe-mapa.md` se houver documentação de sentinel de canal

## 7. Critérios de aceite

- [ ] `try_recv(handle, &mut out)` retorna `bool` (true = valor em `out`,
      false = vazio)
- [ ] `kata_rt_channel_recv` continua `(handle: i64) -> i64` — assinatura FFI
      inalterada
- [ ] Enviar `-1` via `channel!()` e receber funciona (não deadlocka)
- [ ] Enviar `-1` via `queue!(1)` e receber funciona
- [ ] Teste `socket_connected_listen_fails` com `channel!()` passa
- [ ] `cargo test --workspace --no-fail-fast` — ≥ 1372 passed, 0 failed
- [ ] `WOULD_BLOCK` permanece `-1` para `try_send`, file, socket
- [ ] Codegen não é modificado

## 8. Riscos

### 8.1. IPC `try_recv` pode ter lógica de desserialização complexa

O path `TAG_IPC_CHANNEL` em `try_recv` desserializa valores do pipe Unix.
A mudança para out-parameter é mecânica (escrever em `out` em vez de
retornar), mas a desserialização envolve type_id, leitura de bytes do pipe,
e decodificação. Erro aqui pode corromper valores IPC.

**Mitigação:** testes IPC existentes (`spawn_ipc_e2e.rs`) cobrem o path.

### 8.2. Broadcast Rx pode ter valor `None`

`TAG_BROADCAST_RX` em `try_recv` hoje faz `val.unwrap_or(WOULD_BLOCK)`. Se o
broadcast foi setado mas o valor é `None` (não deveria acontecer), retorna
`WOULD_BLOCK`. Com out-parameter, `unwrap_or(0)` e retornar `true`. Mudança
de semântica sutil — mas `None` não deveria acontecer em broadcast válido.

### 8.3. Quebra de testes que comparam resultado de recv com -1

Se algum teste Rust compara `kata_rt_channel_recv` com `-1` esperando
`WOULD_BLOCK` (fora de fiber), a mudança pode quebrar. Verificar com
`grep -rn 'WOULD_BLOCK\|== -1\|!= -1' crates/kata-rt/tests/` antes de
prosseguir.