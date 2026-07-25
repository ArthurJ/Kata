# PRD — Fio 16b: Lifecycle de Canais — Retorno à Fiber Arena

**Status:** 📄 Rascunho
**Data:** 2026-07-25
**Depende de:** Fio 11 ✅ (CSP, scheduler, fibers, arenas), Fio 16 ✅ (ARC Arena, TrackedArena)
**Resolve:** Fio 16 §8 — "Lifecycle de canais" (deixado em aberto no commit `3b4ef39`)

---

## 1. Problema

A Fase 6 do Fio 16 (commit `54f44ce`) moveu a alocação de canais da
`fiber_arena` do criador para a `root_arena` (TrackedArena). A justificativa
foi: "Canais sobrevivem à destruição do fiber criador, garantindo que
receivers em fibers filhas possam consumir valores com segurança."

Esta justificativa é incorreta. O Fio 11 já estabelecia (linha 564-570 do
PRD-fio11) que canais vivem na fiber_arena do criador, e que structured
concurrency garante que o criador é always-last. A Fase 6 violou este
design sem necessidade.

**Consequência:** Canais alocados na TrackedArena nunca são dealocados
individualmente (não são ARC-managed, não têm header, não têm refcount).
Sobrevivem até o teardown da root_arena. Em programas longos com canais
efêmeros, cada canal vaza 48–72 bytes permanentemente.

## 2. Solução

### 2.1. Canais de volta na fiber_arena

Reverter a Fase 6: `lower_channel_create`, `lower_receiver_factory_call`,
e `lower_select` voltam a usar `ctx.fiber_arena` em vez de
`arena_handle_for_escape(EscapeTarget::Heap, ctx)`.

Isto restaura o design original do Fio 11:
- Canal é alocado na fiber_arena do criador via `kata_rt_arena_alloc`.
- `bump.reset()` no epílogo do fiber libera o canal — O(1), sem leak.
- Structured concurrency (`try_destroy` bottom-up: `completed &&
  children.is_empty()`) garante que o criador não morre antes dos filhos.

### 2.2. Proibir retorno de canais

A invariante "canais sempre descem, nunca sobem" é real em prática mas
não é enforced em compile-time. `fits_return` (expr.rs:76) aceita
`Sender`, `Receiver`, e `ReceiverFactory` em posições de retorno.

**Regra:** `Ty::Sender`, `Ty::Receiver`, `Ty::ReceiverFactory` são
proibidos no tipo de retorno de Actions — em qualquer profundidade
(dentro de Tuple, Struct, List, etc.).

**Justificativa — o que se perde:** Nada. Os três caminhos de invocação
de Actions tornam o retorno de canal ou impossível ou sem sentido:

1. **`fork!`** — retorna `Unit`. Não há caminho de retorno do filho
   para o pai. O canal só desce (como arg), nunca sobe.
2. **ActionCall direto** (dentro de Action) — mesmo fiber, mesma
   `fiber_arena`. O canal compartilha a arena de quem recebe. Não há
   crossing de fiber.
3. **Entry point (scheduler_mode)** — retorno vai para o host como
   `root_result: i64`. Um handle de canal é um ponteiro+tag interno do
   runtime; sem sentido para o host.

A proibição formaliza uma invariante que a arquitetura já impõe
estruturalmente.

## 3. Mudanças

### 3.1. Codegen — `crates/kata-codegen/src/lowering/csp.rs`

Reverter `lower_channel_create`, `lower_receiver_factory_call`, e
`lower_select` para usar `ctx.fiber_arena`:

```rust
// Antes (Fase 6):
let arena = crate::lowering::escape_arena::arena_handle_for_escape(
    kata_core::escape::EscapeTarget::Heap,
    ctx,
);

// Depois (revertido):
let arena = ctx
    .fiber_arena
    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
```

### 3.2. Inferência — `crates/kata-inference/src/infer/action_infer.rs`

Adicionar check em `infer_action` após inferir o body, antes de retornar
o `TypedAction`. Se `ret_ty` contém `Sender`/`Receiver`/`ReceiverFactory`
em qualquer profundidade, emitir `MiddleError::TypeMismatch`.

Função auxiliar `contains_channel_type(ty: &Ty) -> bool` recursiva em
Tuple, Struct, List, Array, Dict, Set, Sender, Receiver, ReceiverFactory.

### 3.3. PRD Fio 16 — `docs/PRD-fio16-arc-arena.md`

Mover "Lifecycle de canais" de §8 (Evolução Futura) para resolvido.

## 4. O Que Não Muda

- **Valores ARC-managed enviados por canal**: continuam na root_arena
  com incref/decref. O canal (struct) volta para fiber_arena; os
  valores que trafegam continuam na root_arena.
- **Fiber arenas**: bumpalo, fast path, zero mudança.
- **TrackedArena / root_arena**: continua para valores ARC-managed.
- **Scheduler, fibers, structured concurrency**: sem mudança.

## 5. Critérios de Aceitação

1. `cargo test --workspace` passa sem regressões.
2. Teste que retorna `Sender::T` de uma Action produz erro de tipo.
3. Teste E2E com canal criado, fork!, send/recv funciona corretamente.
4. `arena_stats` mostra canais não acumulam na root_arena.