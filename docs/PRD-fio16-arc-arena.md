# PRD — Fio 16: ARC Arena — Deallocation Individual na Root Arena

**Status:** 📄 Rascunho
**Data:** 2026-07-24
**Depende de:** Fio 11 ✅ (CSP, scheduler, fibers, arenas), Fio 13 ✅ (Closure Unification, CaptureBox)
**Não depende de:** `@parallel` (congelado)

## 1. Problema

Valores que circulam por canais (`!>`) precisam sobreviver ao fiber sender. O
manual (§5.2) descreve que esses valores são alocados na heap global com
`Arc<T>` nativo (reference counting thread-safe). A implementação atual **não
cumpre isso**:

- O escape analysis já marca `EscapeTarget::Heap` para variáveis enviadas via
  `Send` (`!>`) — funciona no typeck.
- O codegen **ignora** essa marcação: `lower_channel_send` passa o `i64` cru
  (ponteiro bruto da arena do sender) para `kata_rt_channel_send`.
- `kata_rt_channel_send` armazena `Option<i64>` — o ponteiro cru no slot do
  canal.
- Se o sender termina antes do receiver consumir, o receiver segura um
  dangling pointer para uma arena já destruída — **use-after-free silencioso**.
- `kata_rt_decref` decrementa o refcount mas não libera memória — o comentário
  no código diz explicitamente: "Por ora, a arena cuida do lifetime."

Primitivos (Int, Float, Boolean) não sofrem — são SMI-tagged, o `i64` é o
próprio valor. Textos, Listas, Structs, Arrays sofrem.

## 2. Objetivo

Implementar deallocation individual na root arena para valores ARC-managed,
seguindo o princípio "último a sair apaga a luz":

1. Valores ARC-managed são alocados na **root arena** (não na fiber arena).
2. `incref` quando alguém pega referência (envio por canal, captura de
   closure).
3. `decref` quando alguém solta a referência (epílogo de fiber, consumo de
   canal, destruição de closure).
4. `decref → 0` libera o bloco individualmente da root arena.
5. Teardown da root arena libera o que sobrou (valores com refcount > 0 no
   fim do programa — fallback de segurança).

Fiber arenas continuam com bumpalo: alloc O(1), reset O(1), sem dealloc
individual. O fast path (tuplas locais, bindings, temporários) não muda.

## 3. Design

### 3.1. Duas estratégias de allocation na mesma interface

```
Fiber arenas: bumpalo (inalterado)
    ├── alloc: bump.alloc_layout(layout) → ptr   [O(1)]
    ├── dealloc: NÃO EXISTE
    └── destroy: bump.reset()                      [O(1), libera tudo]

Root arena: std::alloc + tracking
    ├── alloc: std::alloc::alloc(layout) + blocks.push((ptr, layout))  [O(1)+push]
    ├── dealloc: std::alloc::dealloc(ptr, layout) + blocks.remove(ptr) [O(n) ou O(1) com swap-remove]
    └── destroy: percorre blocks, std::alloc::dealloc para cada        [O(n)]
```

### 3.2. Estrutura

```rust
/// Arena da root — std::alloc + tracking para dealloc individual.
pub(crate) struct TrackedArena {
    /// Blocos alocados e ainda vivos. Usado para teardown.
    /// Vec por trás — swap_remove para dealloc O(1) se não precisar de ordem.
    blocks: Vec<(*mut u8, std::alloc::Layout)>,
}

impl TrackedArena {
    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { std::alloc::alloc(layout) };
        if !ptr.is_null() {
            self.blocks.push((ptr, layout));
        }
        ptr
    }

    fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        // swap_remove: O(1), não preserva ordem (ordem não importa aqui)
        if let Some(idx) = self.blocks.iter().position(|(p, _)| *p == ptr) {
            self.blocks.swap_remove(idx);
            unsafe { std::alloc::dealloc(ptr, layout) };
        }
    }

    fn destroy(&mut self) {
        for (ptr, layout) in self.blocks.drain(..) {
            unsafe { std::alloc::dealloc(ptr, layout) };
        }
    }
}
```

### 3.3. Pool de arenas: enum dispatch

O pool de arenas (`ARENAS: RefCell<Vec<ArenaKind>>`) passa a suportar dois
tipos:

```rust
enum ArenaKind {
    Bump(Bump),                // fiber arenas — bumpalo
    Tracked(TrackedArena),      // root arena — std::alloc + tracking
}
```

`kata_rt_arena_alloc(handle, size)` dispatcha:
- `ArenaKind::Bump` → `bump.alloc_layout(layout)` (comportamento atual)
- `ArenaKind::Tracked` → `tracked.alloc(layout)`

### 3.4. Root arena no scheduler

O scheduler cria a root arena como `Tracked` em `Scheduler::new()`:

```rust
// scheduler.rs — new()
let root_arena = create_tracked_arena();  // em vez de kata_rt_arena_create()
```

Fiber arenas continuam sendo criadas via `kata_rt_arena_create` (bumpalo).

### 3.5. `decref` com dealloc real

O `decref` precisa de acesso à root arena. Solução: TLS `ROOT_ARENA_HANDLE`
setada no `kata_rt_scheduler_init` (consistente com padrão existente de TLS
no scheduler).

```rust
thread_local! {
    static ROOT_ARENA_HANDLE: Cell<i64> = const { Cell::new(0) };
}

pub extern "C" fn kata_rt_decref(box_ptr: i64) -> i64 {
    if box_ptr == 0 { return 0; }
    unsafe {
        let refcount_ptr = (box_ptr as *mut u8).add(8) as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        if count > 0 {
            let new_count = count - 1;
            std::ptr::write_unaligned(refcount_ptr, new_count);
            if new_count == 0 {
                // ARC chegou a 0 — liberar o bloco da root arena.
                // Layout: 16 bytes header + n_captures * 8.
                // O tamanho precisa ser conhecido. Opções:
                //   A) Ler n_captures do header (offset 8 já tem refcount,
                //      precisamos de n_captures em outro offset ou inferir).
                //   B) Guardar layout no header (aumentar header para 24 bytes).
                //   C) Guardar (ptr, layout) na TrackedArena e procurar por ptr.
                // Ver §3.7 — Decisão de Layout.
                root_arena_dealloc(box_ptr);
            }
        }
    }
    0
}
```

### 3.6. Codegen: alocar ARC values na root arena

**Closures com captura** (`alloc_capture_box` em `closure.rs:304`):

Hoje usa `fiber_arena.or(caller_arena)`. Muda para `root_arena`:

```rust
// Antes:
let capture_arena = ctx.fiber_arena.or(ctx.caller_arena)
    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

// Depois:
let capture_arena = ctx.root_arena
    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
```

`ctx.root_arena` é populado pelo `LowerCtx` a partir da TLS
`ROOT_ARENA_HANDLE` (ou passado como param implícito no trampoline — ver
§3.8).

**Valores de canal** (`lower_channel_send` em `csp.rs:152`):

Hoje envia o `i64` cru. Precisa alocar o valor na root arena antes de enviar:

```rust
pub(crate) fn lower_channel_send(channel, value, ctx) -> ... {
    let handle = lower_expr(&channel.node, ctx)?;

    // Se o valor tem EscapeTarget::Heap (marcado pelo escape analysis),
    // alocar na root arena antes de enviar.
    let val = if value.node.escape == EscapeTarget::Heap {
        alloc_arc_value(&value.node, ctx)?  // aloca na root arena, retorna ptr
    } else {
        lower_expr(&value.node, ctx)?  // primitivo — SMI, envia direto
    };

    ctx.builder.ins().call(fref, &[handle, val]);
    Ok(ctx.builder.ins().iconst(I64, 0))
}
```

Alternativa: o escape analysis já marcou o `EscapeTarget` no TAST. Se o
valor foi alocado com `EscapeTarget::Heap` desde o início (no site de
criação, não no site de send), o codegen já o colocou na root arena. Nesse
caso, `lower_channel_send` não precisa de lógica extra — o ponteiro já é
válido. **Esta é a abordagem preferida** — aloca na root arena desde o
início, evita cópia no send.

### 3.7. Decisão de Layout do CaptureBox

O `decref → 0` precisa saber o tamanho do bloco para passar ao
`std::alloc::dealloc`. O layout atual do CaptureBox:

```
offset 0:   fn_ptr       (8 bytes)
offset 8:   refcount     (8 bytes)
offset 16:  captures[0]  (8 bytes)
...
offset 16 + (n-1)*8: captures[n-1]
```

`std::alloc::dealloc` requer o `Layout` exato (size + align). Opções:

**A) Inferir tamanho do header (16 + n_captures * 8):**
Precisamos de `n_captures` no header. Hoje não está armazenado — o codegen
sabe `n_captures` no momento da alocação, mas o `decref` não tem acesso.
Adicionar `n_captures` ao header:

```
offset 0:   fn_ptr       (8 bytes)
offset 8:   refcount      (8 bytes)
offset 16:  n_captures   (8 bytes)   ← NOVO
offset 24:  captures[0]  (8 bytes)
...
```

Header cresce de 16 para 24 bytes. `decref` lê `n_captures` no offset 16,
calcula `size = 24 + n_captures * 8`, chama `dealloc(ptr, Layout::from_size_align(size, 8))`.

**B) Guardar (ptr, Layout) na TrackedArena:**
`decref` procura `ptr` na `blocks: Vec<(*mut u8, Layout)>`, pega o `Layout`,
chama `dealloc`. `blocks.iter().position(|(p, _)| *p == ptr)` é O(n) no pior
caso. Com `swap_remove` após encontrar, o remove é O(1) mas a busca é O(n).

**C) HashMap ptr → Layout na TrackedArena:**
`HashMap<*mut u8, Layout>` para lookup O(1). Overhead de hash por alloc/dealloc.

**Decisão: Opção A.** Adicionar `n_captures` ao header. É a mais simples, não
exige estruturas auxiliares, e o header já é opaque para o usuário da
linguagem. O custo é 8 bytes por CaptureBox — irrelevante.

### 3.8. Passagem da root arena para o codegen

Duas opções:

**A) TLS `ROOT_ARENA_HANDLE`:**
Setada em `kata_rt_scheduler_init`. `LowerCtx` lê no construtor. O codegen
passa o handle como argumento para `kata_rt_alloc_arc`.

**B) Param implícito no trampoline:**
Hoje o trampoline passa `(fiber_arena, caller_arena, args_ptr)`. Adicionar
`root_arena` como 4º param. O codegen lê do param.

**Decisão: Opção A (TLS).** O scheduler já mantém TLS para `CURRENT_SUSPEND`,
`LOG_CONFIG`, etc. Uma `ROOT_ARENA_HANDLE` é consistente. Evita mudar a
assinatura do trampoline (que é FFI — mudar assinatura tem blast radius
maior).

### 3.9. `incref` no envio por canal

Quando um valor é enviado por canal, o channel segura uma referência. O
codegen emite `incref(ptr)` antes de `channel_send(handle, ptr)`:

```rust
// lower_channel_send (após alocar/obter o ptr na root arena)
ctx.builder.ins().call(incref_ref, &[val]);     // channel segura ref
ctx.builder.ins().call(send_ref, &[handle, val]);
```

O receiver, ao consumir, não precisa `incref` — ele herda a ref do channel.
Quando o receiver termina (epílogo), `decref` é emitido.

### 3.10. Epílogo de fiber: decref de todas as refs locais

O ARC pass do optimizer já insere `incref`/`decref` em closures. Precisa ser
estendido para cobrir valores de canal:

1. Toda variável local que segura um valor ARC-managed recebe `decref` no
   epílogo da action (antes de `arena_destroy(fiber_arena)`).
2. O `arena_destroy(fiber_arena)` libera os dados locais (não-ARC). Os
   dados ARC estão na root arena — não são tocados pelo `arena_destroy`.
3. Se o fiber não consumiu o valor de canal (enviou adiante ou descartou),
   o `decref` no epílogo libera a ref do fiber.
4. Se o valor está no channel slot, o channel segura a ref. Quando o
   channel é destruído, `decref` é chamado para o valor no slot.

### 3.11. Destruição de canais

Quando um canal é destruído (arena do canal resetada), valores ainda no slot
precisam `decref`. Hoje o channel slot é `Option<i64>` — o reset do bumpalo
libera a memória do `ChannelInner`, mas não dá `decref` no valor.

O `ChannelInner` precisa de um `Drop` que `decref` o valor no slot. Mas
`ChannelInner` está numa arena (bumpalo) — arenas não chamam `Drop`. Solução:
o scheduler, ao destruir a arena onde o canal vive, percorre os canais
registrados e `decref` os valores pendentes. Ou: canais são alocados na root
arena (tracked) em vez da fiber arena, permitindo cleanup individual.

**Decisão: Canais na root arena.** Canais são objetos de coordination entre
fibers — natural que vivam na root arena. `kata_rt_channel_create` recebe o
handle da root arena em vez da fiber arena. Quando um canal é destruído
(dealloc individual na root arena), `decref` o valor no slot.

## 4. Fases de Implementação

### Fase 1: TrackedArena + root arena no scheduler
- Implementar `TrackedArena` com `alloc`, `dealloc`, `destroy`
- `ArenaKind` enum no pool
- Scheduler cria root arena como `Tracked`
- TLS `ROOT_ARENA_HANDLE` setada no init
- `kata_rt_arena_alloc` dispatcha por tipo
- `kata_rt_arena_destroy` dispatcha por tipo (Tracked = percorre blocks)
- Testes: root arena alloc/dealloc/destroy funcionando isoladamente

### Fase 2: Header com n_captures + decref real
- Adicionar `n_captures` ao header do CaptureBox (offset 16)
- Captures empurram para offset 24
- `kata_rt_decref` lê `n_captures`, calcula size, chama `root_arena.dealloc`
- Atualizar `alloc_capture_box` no codegen para escrever `n_captures`
- Testes: alloc arc, incref, decref → 0, verifica que dealloc foi chamado

### Fase 3: Codegen — closures na root arena
- `alloc_capture_box` usa `root_arena` em vez de `fiber_arena`
- `LowerCtx` carrega `root_arena` da TLS
- Testes: closure com captura sobrevive à destruição da fiber arena

### Fase 4: Codegen — valores de canal na root arena
- `lower_channel_send`: se `EscapeTarget::Heap`, valor já está na root arena
  (alocado desde o início) — enviar ptr direto
- Escape analysis garante que valores marcados `Heap` são alocados na root
  arena no site de criação, não no site de send
- `incref` antes de `channel_send`
- Testes: valor enviado por canal sobrevive à destruição da fiber do sender

### Fase 5: Epílogo — decref de refs locais
- ARC pass estendido: insere `decref` para toda variável local ARC-managed no
  epílogo da action (antes de `arena_destroy(fiber_arena)`)
- Testes: memory leak não ocorre — refs liberadas no epílogo

### Fase 6: Canais na root arena + cleanup
- `kata_rt_channel_create` usa root arena
- Destruição de canal faz `decref` do valor no slot
- Testes: canal destruído libera valor pendente

### Fase 7: Testes E2E + integração
- Testes end-to-end com canais, forks, closures
- Verificar: sender termina, receiver ainda consome valor válido
- Verificar: sem memory leaks (contar allocs/deallocs)
- Verificar: primitivos continuam funcionando (SMI, sem ARC)

### Fase 8: Revisar mensagens de erro do compilador
- Hoje as mensagens de erro do Kata trazem informações de span (linha/coluna),
  mas não mostram o trecho de código com problema.
- Usar o span para extrair e exibir o trecho de código fonte relevante,
  com indicador visual (caret `^` ou highlight) apontando para a posição exata.
- Aplicar em todos os pontos de emissão de erros (parser, inference, resolution).
- Testes: snapshot de mensagens de erro com trecho de código embutido

## 5. Decisões de Design

| Decisão | Escolha | Razão |
|---|---|---|
| Allocator root arena | `std::alloc` + `Vec<(*mut u8, Layout)>` | Zero deps, ~30 linhas, controle total |
| Deallocation individual | `std::alloc::dealloc` | O SO já otimiza para sizes variáveis |
| Tracking de blocos | `Vec` com `swap_remove` | O(1) remove, ordem não importa |
| Root arena handle | TLS `ROOT_ARENA_HANDLE` | Consistente com padrão existente |
| n_captures no header | Offset 16, 8 bytes | `decref` precisa saber o size para dealloc |
| Canais na root arena | `kata_rt_channel_create(root_arena)` | Canais sobrevivem a fibers, natural que vivam na root |
| Alocação de escaping values | No site de criação, não no site de send | Evita cópia; escape analysis já marcou no TAST |

## 6. O Que Não Muda

- **Fiber arenas**: bumpalo, fast path, zero mudança
- **`arena_alloc` em fiber arenas**: comportamento idêntico
- **`arena_destroy` em fiber arenas**: `bump.reset()`, idêntico
- **Primitivos (Int, Float, Boolean)**: SMI-tagged, não passam por ARC
- **Scheduler, fibers, yield/block**: sem mudança estrutural
- **ARC pass do optimizer**: continua inserindo incref/decref, agora com
  semântica real (decref → 0 libera memória)

## 7. Riscos e Mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| Fragmentação na root arena (long-running) | Acúmulo de memória morta | Fase 7 mede; evolução futura = size-class pool |
| `decref` esquecido em algum path | Memory leak | ARC pass sistemático + testes de leak counting |
| `decref` duplo em algum path | Use-after-free / panic | Refcount já protege (count > 0 check) |
| Esquecer `incref` antes de `channel_send` | Receiver segura ptr mas refcount não subiu | Escape analysis marca, codegen emite incref |
| Mudança no header do CaptureBox | Breaking change em todos os call sites | Fase 2 atualiza todos os call sites do codegen |

## 8. Evolução Futura (Não Escopo)

- **Size-class pool**: Substituir `std::alloc` por pools de size classes fixas
  para reduzir fragmentação em long-running. Interface não muda (`alloc`/`dealloc`
  recebem `Layout`).
- **Compaction**: Mover valores vivos para blocos contíguos durante idle do
  scheduler.
- **ARC pass no AOT**: Hoje o ARC pass roda no JIT path. Garantir que o AOT
  path também emita incref/decref corretamente.

## 9. Critérios de Aceitação

1. `cargo test` passa sem regressões
2. Teste E2E: sender fiber termina, receiver fiber ainda consome valor válido
   (List, Text, Struct — não só primitivos)
3. Teste de leak: contador de allocs vs deallocs fecha no fim do programa
4. Primitivos continuam funcionando sem overhead (SMI, sem ARC)
5. `decref → 0` chama `std::alloc::dealloc` — verificável via test contador
6. Root arena teardown libera todos os blocos restantes