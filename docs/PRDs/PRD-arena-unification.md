# PRD: Unificação de Arenas — TrackedArena → Bump

## Status

**Status:** 🔴 Pendente
**Data:** 2026-09-17
**Substitui:** PRD-fio16-arc-arena (ARC nunca foi implementado — `incref`/`decref` não emitidos)
**Resolve:** Heap corruption intermitente em FFIs JIT (TrackedArena + `std::alloc::alloc`)

## 1. Objetivo

Eliminar a `TrackedArena` (root arena) e unificar toda alocação de runtime
em arenas Bump (bumpalo). A root arena passa a ser uma `Arena` (Bump) com
o mesmo mecanismo das fiber arenas. CaptureBoxes (closures com captura)
passam a respeitar `EscapeTarget` na seleção de arena, em vez de hardcoded
para root arena. O ARC vestigial (`incref`/`decref`/`arc_fn_ptr`) é
removido do codegen.

## 2. Motivação

### 2.1. Heap corruption intermitente

A `TrackedArena::alloc` chama `std::alloc::alloc` (malloc do glibc) por
alocação. Chamada de dentro de FFIs executadas pelo JIT, corrompe
intermitentemente a metadata do glibc malloc (~30-50% das corridas,
sintoma: "corrupted double-linked list" / SIGSEGV). A `Bump` arena
(bumpalo) gerencia sua própria memória — não chama `std::alloc` por
alocação. Por isso é segura em path JIT.

O contorno atual migrou apenas o path de `connect` TCP para fiber arena
(commit `098a47d6`). Todas as outras FFIs de I/O (`socket_read`,
`socket_write`, `file_read`, `file_readline`, `file_write`,
`socket_accept`) continuam alocando via `arena_alloc()` na root arena —
o bug permanece latente em toda a superfície de I/O.

### 2.2. ARC é vestigial

`kata_rt_incref` e `kata_rt_decref` estão registradas como símbolos JIT
(`ffi_registry.rs`) com assinaturas Cranelift definidas (`ffi_sigs/`),
mas **zero call sites** no lowering. O refcount nasce em 1
(`alloc_arc` escreve 1 no offset 8) e nunca muda. O epílogo da action
(`action_def.rs` L216) comenta "decref de ARC vars" mas só fecha I/O
handles. `kata_rt_arc_fn_ptr` também não é usada — o codegen lê `fn_ptr`
do CaptureBox com `load` direto do offset 0 (`closure.rs` L269).

### 2.3. `EscapeTarget::Heap` é dead code

A variante `Heap` do enum `EscapeTarget` (`escape.rs` L43) é tratada no
codegen (`escape_arena.rs` L33, `action_call.rs` L86`), mas a inferência
**nunca a atribui** — `search_files` por `EscapeTarget::Heap` em
`crates/kata-inference/src/` retorna zero resultados. A inferência só
atribui `Local` e `Caller`.

### 2.4. `alloc_capture_box` ignora escape analysis

`alloc_capture_box` (`closure.rs` L438-446`) chama
`kata_rt_get_root_arena_handle` hardcoded — sempre root arena,
independentemente do `EscapeTarget` do expr. FFIs de I/O seguem o mesmo
padrão: `arena_alloc()` em `socket/mod.rs` e `file.rs` pega a root arena
diretamente.

## 3. Design

### 3.1. Root arena vira Bump

A root arena é criada como `ArenaKind::Bump(Arena::new())` em vez de
`ArenaKind::Tracked(TrackedArena::new())` no `Runtime::new()`
(`runtime.rs` L58). A root arena nunca é resetada entre fibers (só
destruída no `Drop` do Runtime) — este comportamento não muda. A
diferença é que `Bump::alloc` não chama `std::alloc`; a memória vem de
chunks pré-alocados pelo bumpalo.

`ArenaKind::Tracked` e `TrackedArena` são removidos. O enum `ArenaKind`
fica com uma única variante: `Bump(Arena)`. `arena_create_tracked` é
removido da FFI.

### 3.2. CaptureBox respeita `EscapeTarget`

`alloc_capture_box` (`closure.rs`) usa
`arena_handle_for_escape(expr.escape, ctx)` em vez de
`kata_rt_get_root_arena_handle` hardcoded. O `escape` do expr é
propagado pela inferência:

- `Local` → `fiber_arena` (closure local ao fiber, morre com ele)
- `Caller` → `caller_arena` (closure que escapa para o caller, sobrevive
  na arena do LCA)
- `Heap` → removido do enum (dead code)

Para closures sem captures (`n_captures = 0`), o `box_ptr` é um wrapper
fino de `fn_ptr` — o custo de respeitar escape é irrelevante (a
alocação é de 24 bytes).

### 3.3. `escape_for_channel_send`: `Ty::Function` → `Caller`

`escape_for_channel_send` (`csp.rs` L798`) trata `Ty::Function` como
`Local` — assume que é `fn_ptr` inline. Mas uma closure com captures é
um `box_ptr` (ponteiro para CaptureBox alocado em arena). Se alocado na
fiber arena do sender (filho) e o filho termina antes do receiver (pai)
consumir, o `box_ptr` aponta para memória liberada — use-after-free.

Mudar `Ty::Function` para `Caller` (mesmo tratamento que compostos):
a `caller_arena` é a arena do LCA de sender e receiver. Em structured
concurrency, o pai só morre depois de todos os filhos, então a
`caller_arena` cobre o lifetime de ambos.

### 3.4. FFIs de I/O migradas para fiber arena

As FFIs de I/O que usam `arena_alloc()` (root arena) em `socket/mod.rs`
e `file.rs` são migradas para o pattern `fiber_alloc` já existente em
`socket/create.rs`: ler `CURRENT_FIBER_ARENA` TLS, alocar na fiber arena
(Bump), fallback para root arena (também Bump) se sem fiber.

Funções a migrar em `socket/mod.rs`:
- `arena_alloc` → `fiber_alloc` (mesma assinatura, mesmo fallback)
- `alloc_result_box`, `alloc_text`, `alloc_bytes`, `alloc_socket_inner`,
  `error_text` — usam `fiber_alloc` internamente

Funções a migrar em `file.rs`:
- `arena_alloc` → `fiber_alloc`
- `alloc_result_box`, `alloc_text`, `error_text` — usam `fiber_alloc`
- `arena_alloc_in` (usada por `file_open` com arena do codegen) —
  mantém, pois recebe arena_handle explicitamente

### 3.5. Remoção do ARC vestigial

Removidos do codegen:
- `FfiSymbol::IncRef`, `FfiSymbol::DecRef`, `FfiSymbol::ArcFnPtr` —
  enum + assinaturas Cranelift
- `builder.symbol("kata_rt_incref", ...)` — registro no `ffi_registry`
- `builder.symbol("kata_rt_decref", ...)`
- `builder.symbol("kata_rt_arc_fn_ptr", ...)`

Removidos do runtime:
- `kata_rt_incref` (`arc.rs`) — função inteira
- `kata_rt_decref` (`arc.rs`) — função inteira
- `kata_rt_arc_fn_ptr` (`arc.rs`) — função inteira
- `kata_rt_arena_dealloc` (`arena.rs`) — função inteira
- `kata_rt_arena_create_tracked` (`arena.rs`) — função inteira
- `kata_rt_arena_stats` (`arena.rs`) — função inteira
- `FfiSymbol::ArenaDealloc`, `FfiSymbol::ArenaCreateTracked`,
  `FfiSymbol::ArenaStats`, `FfiSymbol::GetRootArenaHandle` — enum

Mantidos:
- `kata_rt_alloc_arc` — aloca CaptureBox (sem refcount no offset 8,
  ou refcount fixo em 1 para compatibilidade de layout)
- `kata_rt_get_root_arena_handle` — ainda usado por `escape_arena.rs`
  e `action_call.rs` para obter o handle da root arena (agora Bump)

### 3.6. `EscapeTarget::Heap` removido

A variante `Heap` é removida do enum `EscapeTarget` (`escape.rs`). O
codegen (`escape_arena.rs`, `action_call.rs`) remove o braço `Heap`.
A inferência não precisa mudar — já não atribui `Heap`.

## 4. Decisões de design

### D1: Root arena como Bump, não slab próprio

**Escolhido:** root arena vira `Arena` (bumpalo).
**Alternativa rejeitada:** slab/pool próprio para CaptureBox com
free-list manual. O slab eliminaria `std::alloc` do path JIT, mas
introduziria complexidade de gerenciamento de memória manual. Como ARC
é vestigial (refcount nunca muda, dealloc nunca chamado), não há
necessidade de dealloc individual — Bump com cleanup no teardown é
suficiente.

### D2: CaptureBox respeita escape, não fiber-only

**Escolhido:** `alloc_capture_box` usa `arena_handle_for_escape`.
**Alternativa rejeitada:** sempre fiber arena. Closures enviadas via
canal de filho para pai causariam use-after-free: o filho termina, sua
fiber arena é resetada, o pai segura um `box_ptr` dangling. Respeitar
`EscapeTarget` (com `Ty::Function → Caller` em channel send) garante
que closures que escapam vão para `caller_arena` (arena do LCA).

### D3: FFIs de I/O migradas para fiber arena

**Escolhido:** migrar todas as FFIs de I/O para `fiber_alloc`.
**Alternativa rejeitada:** deixar FFIs de I/O na root arena (agora
Bump). Tecnicamente funciona (Bump não chama `std::alloc`), mas a
root arena acumula lixo (Result boxes, error_text, bytes) até o
teardown do Runtime. Fiber arena é resetada quando o fiber termina —
liberação mais frequente, menor footprint de memória.

### D4: Remover ARC em vez de implementar

**Escolhido:** remover `incref`/`decref`/`arc_fn_ptr` do codegen e
runtime.
**Alternativa rejeitada:** implementar ARC corretamente (emitir
`incref`/`decref` no codegen). A concorrência estruturada garante
always-last creator: canais fluem entre pai-filho/irmãos, o pai só
mora depois de todos os filhos. A `caller_arena` cobre o lifetime de
todos os interessados. ARC seria redundante — nenhum valor precisa
sobreviver além o LCA de sender e receiver.

## 5. Fases

### Fase 1 — `escape_for_channel_send`: `Ty::Function` → `Caller`

**Escopo:** Uma linha em `csp.rs` L798.

**Mudança:**
```rust
// De:
Ty::Function(..) => EscapeTarget::Local,
// Para:
Ty::Function(..) => EscapeTarget::Caller,
```

**DoD:** `cargo test --workspace --no-fail-fast` passa. Closures
enviadas via canal alocam na `caller_arena`.

**Oráculos:**
- Teste E2E existente de channel + closure continua passando (se houver).
- Novo teste: closure com capture enviada via canal de filho para pai —
  pai recebe e usa sem crash.

### Fase 2 — `alloc_capture_box` respeita `EscapeTarget`

**Escopo:** `closure.rs` — `alloc_capture_box`.

**Mudança:** Substituir a chamada hardcoded
`kata_rt_get_root_arena_handle` por
`arena_handle_for_escape(expr.escape, ctx)`. O `escape` vem do `expr`
que envolve a closure (propagado pela inferência). Se `escape` é
`Local`, usa `fiber_arena`; se `Caller`, usa `caller_arena`.

**DoD:** `cargo test --workspace --no-fail-fast` passa. CaptureBoxes
locais alocam na fiber arena; CaptureBoxes que escapam alocam na
caller_arena.

**Oráculos:**
- Testes E2E existentes de closures (map, filter, fold com lambdas)
  continuam passando.
- Novo teste: closure com capture alocada localmente — verificar que
  o `box_ptr` aponta para a fiber arena (não root arena).

### Fase 3 — Root arena vira Bump

**Escopo:** `runtime.rs` L58, `arena.rs`, `ArenaKind`.

**Mudança:**
1. `Runtime::new()`: `ArenaKind::Tracked(TrackedArena::new())` →
   `ArenaKind::Bump(Arena::new())`.
2. `ArenaKind`: remover variante `Tracked`, deixar só `Bump`.
3. `arena_alloc` em `runtime.rs`: remover braço `Tracked` do match.
4. `arena_dealloc`: no-op (Bump não suporta dealloc individual).
5. `arena_destroy`: `Bump::reset` (já implementado).
6. `arena_stats`: retornar 0 para Bump (já implementado).
7. Remover `TrackedArena` struct, `impl`, `Drop`.

**DoD:** `cargo test --workspace --no-fail-fast` passa. `cargo build`
sem warnings. Root arena usa Bump — `std::alloc` não é chamado em
path JIT.

**Oráculos:**
- Teste de stress: rodar `connect` TCP 20+ vezes — 0 crashes (antes:
  ~30% crash rate).
- Teste de stress: rodar `socket_read` 20+ vezes — 0 crashes.
- `arena_stats` retorna 0 (sem contabilidade de alloc/dealloc).
- Leak counting em testes que usavam `arena_stats`: ajustar ou remover.

### Fase 4 — FFIs de I/O migradas para `fiber_alloc`

**Escopo:** `socket/mod.rs`, `file.rs`.

**Mudança:**
1. Em `socket/mod.rs`: adicionar `fiber_alloc` (mesmo pattern de
   `socket/create.rs` L176-191). Trocar `arena_alloc` por `fiber_alloc`
   em `alloc_result_box`, `alloc_text`, `alloc_bytes`,
   `alloc_socket_inner`, `error_text`.
2. Em `file.rs`: adicionar `fiber_alloc`. Trocar `arena_alloc` por
   `fiber_alloc` em `alloc_result_box`, `alloc_text`, `error_text`.
   Manter `arena_alloc_in` (usada por `file_open` com arena do codegen).
3. Remover as funções `_fiber` duplicadas em `socket/create.rs` —
   unificar com as novas versões em `socket/mod.rs`.

**DoD:** `cargo test --workspace --no-fail-fast` passa. FFIs de I/O
alocam na fiber arena (Bump) — `std::alloc` não é chamado.

**Oráculos:**
- Testes E2E de file I/O (13 testes em `file_io_e2e.rs`) passam.
- Testes E2E de socket TCP passam.
- Teste de stress: `read!(file)` em arquivo grande 20+ vezes — 0 crashes.

### Fase 5 — Remoção do ARC vestigial

**Escopo:** `arc.rs`, `ffi_registry.rs`, `ffi_sigs/scheduler.rs`,
`ffi_sigs/arena.rs`, `kata-core/src/ffi.rs`.

**Mudança:**
1. Remover `kata_rt_incref`, `kata_rt_decref`, `kata_rt_arc_fn_ptr`
   de `arc.rs`.
2. Remover `kata_rt_arena_dealloc`, `kata_rt_arena_create_tracked`,
   `kata_rt_arena_stats` de `arena.rs`.
3. Remover `FfiSymbol::IncRef`, `DecRef`, `ArcFnPtr`, `ArenaDealloc`,
   `ArenaCreateTracked`, `ArenaStats`, `GetRootArenaHandle` do enum.
4. Remover `builder.symbol(...)` para as FFIs removidas.
5. Remover assinaturas Cranelift para as FFIs removidas.
6. `kata_rt_alloc_arc`: manter, mas sem refcount. Layout do CaptureBox
   muda: remove `refcount` (offset 8). Novo layout: `fn_ptr` (offset 0),
   `n_captures` (offset 8), `captures[0..n]` (offset 16+). Atualizar
   leitores do layout (`closure.rs` L269 lê `fn_ptr` do offset 0 —
   sem mudança; function_def L460 lê captures do offset 24 → offset 16).
7. `kata_rt_get_root_arena_handle`: manter (ainda usado por
   `escape_arena.rs` e `action_call.rs`).

**DoD:** `cargo test --workspace --no-fail-fast` passa. `cargo clippy
--workspace --all-targets -- -D warnings` limpo. Sem referências a
`incref`/`decref`/`arc_fn_ptr` no codegen.

**Oráculos:**
- `search_files` por `incref|decref|arc_fn_ptr|ArenaDealloc|
  ArenaCreateTracked` em `crates/` retorna 0 resultados (exceto
  históricos em comentários/docs).
- Todos os testes E2E passam.

### Fase 6 — Remoção de `EscapeTarget::Heap`

**Escopo:** `kata-core/src/escape.rs`, `escape_arena.rs`,
`action_call.rs`.

**Mudança:**
1. Remover variante `Heap` do enum `EscapeTarget`.
2. Remover braço `EscapeTarget::Heap` em `escape_arena.rs` e
   `action_call.rs`.
3. Atualizar comentário em `escape.rs` que descreve `Heap`.

**DoD:** `cargo test --workspace --no-fail-fast` passa. `cargo build`
sem warnings.

## 6. Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| Runtime | `kata-rt/src/arena.rs` | Remover `TrackedArena`, `ArenaKind::Tracked`, `arena_dealloc`, `arena_create_tracked`, `arena_stats` |
| Runtime | `kata-rt/src/runtime.rs` | Root arena vira `Bump`; remover braços `Tracked` dos matches |
| Runtime | `kata-rt/src/arc.rs` | Remover `incref`, `decref`, `arc_fn_ptr`; manter `alloc_arc` |
| Runtime | `kata-rt/src/socket/mod.rs` | `arena_alloc` → `fiber_alloc` |
| Runtime | `kata-rt/src/socket/create.rs` | Remover funções `_fiber` duplicadas (unificar com `mod.rs`) |
| Runtime | `kata-rt/src/file.rs` | `arena_alloc` → `fiber_alloc`; manter `arena_alloc_in` |
| Codegen | `kata-codegen/src/ffi_registry.rs` | Remover símbolos ARC/Tracked |
| Codegen | `kata-codegen/src/ffi_sigs/scheduler.rs` | Remover assinaturas ARC |
| Codegen | `kata-codegen/src/ffi_sigs/arena.rs` | Remover assinaturas Tracked |
| Codegen | `kata-codegen/src/lowering/closure.rs` | `alloc_capture_box` usa `arena_handle_for_escape` |
| Codegen | `kata-codegen/src/lowering/escape_arena.rs` | Remover braço `Heap` |
| Codegen | `kata-codegen/src/lowering/action_call.rs` | Remover braço `Heap` |
| Inferência | `kata-inference/src/infer/csp.rs` | `Ty::Function` → `Caller` em `escape_for_channel_send` |
| Core | `kata-core/src/escape.rs` | Remover variante `Heap` do enum |
| Core | `kata-core/src/ffi.rs` | Remover `FfiSymbol` variants ARC/Tracked |

## 7. Fora do escopo

- **`spawn!` (processo OS):** `kata_rt_spawn_process` faz `fork()` com
  COW da arena. Se a arena é Bump, o fork COW copia as páginas. O filho
  tem sua própria cópia. Não há mudança necessária — fora de escopo.
- **Leak counting em testes:** `arena_stats` retornava
  `(alloc_count, dealloc_count)`. Com Bump, retorna 0. Testes que
  dependem de contagem precisam ser ajustados — tarefa mecânica, fica
  na Fase 5.
- **`PRD-fio16-arc-arena`:** supersede. O ARC foi proposto mas nunca
  implementado. Este PRD substitui a abordagem.

## 8. Riscos

### R1: Closures em coleções que escapam

Se uma closure é elemento de uma lista enviada via canal, o
`box_ptr` da closure é armazenado na lista. A lista é alocada na
`caller_arena` (composto → `Caller`). Se a closure é alocada na
`fiber_arena` (Local), mas a lista escapa para `caller_arena`, o
`box_ptr` dentro da lista aponta para a fiber arena — dangling quando
o fiber morre.

**Mitigação:** `escape_for_channel_send` retorna `Caller` para
`Ty::Function` (Fase 1). Se a lista contém uma closure, o tipo do
elemento é `Ty::Function` — mas o tipo da lista é `List(Function)`,
que cai no wildcard `_ => Caller` (composto). A closure dentro da
lista tem seu próprio `escape` determinado pela inferência do
elemento, que deve propagar `Caller` se a lista escapa. Verificar na
implementação da Fase 2.

### R2: Testes que dependem de `arena_stats`

Testes Rust que chamam `kata_rt_arena_stats` para verificar leak
counting perdem a contabilidade. Identificar com `search_files` por
`arena_stats` em `crates/` antes da Fase 5.

### R3: Interp (interpretador) usa FFIs

O interpretador (`kata-interp`) chama `kata_rt_decref` via
`ffi_dispatch.rs` (L401). Remover `decref` quebra o interp. Adicionar
no-op stub ou remover a chamada no interp.

## 9. Documentação

Ao concluir:
- `docs/TODO.md` — remover item "TrackedArena + JIT FFI = heap
  corruption" se existir.
- `~/.hermes/profiles/kata5/skills/software-development/kata-compiler/SKILL.md`
  — atualizar PITFALL do TrackedArena: reescrever como limitação
  removida ou remover.
- `~/.hermes/profiles/kata5/skills/software-development/kata-compiler/
  references/io-model-analysis.md` — atualizar § connect-heap-corruption.
- `docs/PRDs/PRD-fio16-arc-arena.md` — marcar como supersede.