# Handoff — Estado do Projeto Kata5

**Última atualização:** 2026-07-31
**Branch:** `main` (29 commits à frente de `origin/main`)
**Working tree:** limpa
**Testes:** 1344 passed, 0 failed, 5 ignored

## O que foi feito nesta sessão

### IPC cross-process via Unix pipe para `spawn!`

Implementado canal IPC cross-process para comunicação entre parent e child
após `spawn!` (fork). O child executa a Action em processo OS separado,
herdando a arena via copy-on-write. A comunicação é exclusivamente por
canais IPC passados como args da Action.

**Commits (em ordem cronológica):**
1. `8ae8e4b` — feat(rt,codegen,inference): canais cross-process via Unix pipe para spawn!
2. `c43eefe` — fix(ipc,docs): cross_process rastreia IndexAccess + testes E2E + documentação
3. `ab0397b` — feat(inference): unificar Var(T0) de canais no spawn! + type vars unicos
4. `c151d9d` — fix(cross_process,ipc): resolver ChannelCreate com tipo concreto + type table recursiva

### Arquitetura

```
channel!() → Var("Tn") (nome único por canal via contador TLS)
    ↓
spawn!(worker, (rx1, tx2))
    ↓ infer_spawn_builtin: extract_var_subs + env.apply_substitutions
    ↓ resolve Var("Tn") no TypeEnv (igual fork!)
    ↓
cross_process.rs (pass pós-inferência):
    1. Rastreia let bindings de ChannelCreate → Ident nos args do Spawn
    2. Marca ChannelCreate como cross_process: true
    3. resolve_channel_create: coleta tipos concretos de ChannelSend/ChannelRecv
       e muta ChannelCreate (elem_ty + expr.ty) pelo span
    ↓
codegen: lookup_type_id encontra type_id correto (não 0/Prim)
    ↓ emitir kata_rt_ipc_channel_create(arena, type_id)
    ↓
runtime: try_ipc_send → to_bytes(value, type_id, arena) → write(pipe)
         try_ipc_recv → read(pipe) → from_bytes(blob, arena)
```

### Componentes modificados

- **`kata-inference/src/infer/csp_builtins.rs`** — `channel!()` cria `Var` com nome
  único (T0, T1, T2, ...) via contador TLS; `spawn!` faz `extract_var_subs` +
  `env.apply_substitutions` (mesmo mecanismo do `fork!`)
- **`kata-inference/src/infer/cross_process.rs`** — pass pós-inferência:
  rastreia `IndexAccess` (não `FieldAccess`) para `ch.0`/`ch.1`; marca
  `cross_process: true`; `resolve_channel_create` muta `ChannelCreate` com
  tipo concreto extraído de `ChannelSend`/`ChannelRecv`
- **`kata-codegen/src/type_table.rs`** — `collect_module_types` agora coleta
  sub-tipos recursivamente (Sender/Receiver inner, Tuple elements, List elem)
- **`kata-codegen/src/lowering/csp.rs`** — `lookup_type_id` extrai elem_ty do
  `Sender(inner)` no `expr.ty` do `ChannelCreate`
- **`kata-rt/src/channel/ipc.rs`** — `IpcChannelInner { write_fd, read_fd, type_id }`,
  `try_ipc_send`/`try_ipc_recv` via pipe Unix, `block_until_readable` (poll blocking)
- **`kata-rt/src/channel/ops.rs`** — `block_ipc_until_readable` para child sem scheduler
- **`kata-rt/src/scheduler.rs`** — poll blocking no FD quando todos fibers blocked em IPC
- **`kata-core/src/ty.rs`** — `apply_subs_to_ty` promovida a `pub`
- **`kata-codegen/tests/spawn_ipc_e2e.rs`** — 5 testes E2E: fire-and-forget,
  round-trip Int, tupla, struct, lista

### Testes E2E de IPC (`spawn_ipc_e2e.rs`)

| Teste | Status | Descrição |
|---|---|---|
| `spawn_ipc_send_fire_and_forget` | ✅ pass | spawn! sem comunicação |
| `spawn_ipc_round_trip` | ✅ pass | Int round-trip via pipe |
| `spawn_ipc_tupla_round_trip` | ✅ pass | Tupla (Int, Int) via pipe |
| `spawn_ipc_struct_round_trip` | ✅ pass | Struct Ponto(x, y) via pipe |
| `spawn_ipc_lista_round_trip` | ✅ pass | List Int via pipe |

**Nenhum teste de IPC está ignorado.** Os 5 `#[ignore]` do workspace são:
- `named_functions_e2e.rs` (1) — não relacionado a IPC
- `panic_assert_e2e.rs` (2) — não relacionado a IPC
- `test_runner_e2e.rs` (2) — não relacionado a IPC

### Decisões-chave

1. **`IndexAccess` em vez de `FieldAccess`** — o typeck lowered `ch.0`/`ch.1`
   para `IndexAccess` compile-time em tuplas, não `FieldAccess`
2. **Type vars únicos por canal** — cada `channel!()` cria `Var("T0")`, `Var("T1")`,
   etc. via contador TLS. Antes todos usavam `Var("T0")`, causando colisão quando
   múltiplos canais existiam no mesmo escopo
3. **`resolve_channel_create`** — o `spawn!` resolve `Var` no `TypeEnv` durante
   a inferência, mas o `ChannelCreate` na TAST já foi construído antes com `Var`.
   O pass pós-inferência muta o `ChannelCreate` diretamente pelo span com o tipo
   concreto extraído de `ChannelSend`/`ChannelRecv`
4. **`collect_module_types` recursiva** — coleta sub-tipos de `Sender`/`Receiver`/
   `Tuple`/`List` etc. para que `Tuple([Int, Int])` esteja na type table mesmo
   aparecendo apenas dentro de `Receiver(Tuple([Int, Int]))`
5. **`spawn!` é fire-and-forget** — retorna `Unit`, não há pipe de resultado.
   Comunicação exclusivamente por canais IPC passados como args
6. **Sintaxe de struct usa espaços** — `data Ponto (x::Int y::Int)`, não vírgulas
7. **Sintaxe de tupla em tipo** — `Receiver::((Int, Int))` (parênteses extras)
8. **Pattern matching de tupla em Action** — `match t (a, b): expr` funciona;
   alternativa é `t.0`/`t.1` via IndexAccess

## Próximos passos possíveis

- **Push** — branch está 29 commits à frente de `origin/main`
- **Documentação** — PRD-fio11, ROADMAP, aprendizados.md precisam ser atualizados
  para refletir que os 3 testes de tipos complexos agora passam (remover notas
  de limitação)
- **Buffered/Broadcast IPC** — atualmente só Rendezvous IPC funciona; buffered
  e broadcast fazem fallback para in-process
- **Non-blocking write** — `try_ipc_send` faz `write` blocking; se o pipe buffer
  enche, bloqueia. Futuro: non-blocking write + yield