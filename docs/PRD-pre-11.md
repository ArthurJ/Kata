# PRD: Pré-11 — Infraestrutura de Memória Hierárquica

## Objetivo

Estabelecer o modelo de gerenciamento de memória que torna o Fio 11 (CSP,
canais, scheduler multithread) seguro. Hoje, todo objeto que escapa do fiber
— CaptureBox, Sum results, tuplas em função pura ou entry point — cai na
arena global (handle 0), que nunca é destruída. Isso é um vazamento
permanente.

O PRD pré-11 substitui a arena global por uma **árvore hierárquica de
arenas**, onde cada fiber tem sua arena e acesso às arenas dos ancestrais.
Objetos que precisam ser compartilhados entre fibers são alocados
diretamente na arena do ancestral comum mais próximo (LCA), via escape
analysis em compile-time. A árvore garante a segurança: um pai só é
destruído quando todos os filhos terminaram, então qualquer objeto
promovido para o pai está vivo enquanto algum filho o referencia.

Sem heap separado. Sem free individual. Sem refcount para segurança. A
árvore é o gerenciamento de lifetime.

## Depende de

Fio 1 (pipeline, Ty, FfiSymbol), Fio 2 (lambdas, tail_pos, TAST enriquecida),
Fio 3 (Actions, arena per-fiber, scheduler, caller_arena), Fio 9 (closures
com captura, CaptureBox, escape analysis inicial).

## Problema

### Vazamento da arena global

O codegen escolhe a arena de alocação com base em `tail_pos`:

- `tail_pos = true` → `caller_arena` (sobrevive à destruição da arena local)
- `tail_pos = false` → `fiber_arena` (liberada no epílogo do fiber)

Quando `caller_arena` ou `fiber_arena` é `None` (entry point, função pura),
o fallback é `iconst(0)` — a arena global (handle 0). Esta arena é criada
no prólogo do `__kata_entry` e **nunca destruída**.

| Objeto | Onde aloca hoje | Libera? |
|---|---|---|
| Tupla em tail_pos | `caller_arena` | Sim (destroy do caller) |
| Tupla local | `fiber_arena` | Sim (destroy do fiber) |
| Tupla em função pura | Arena global (0) | **Não** |
| Tupla no entry point | Arena global (0) | **Não** |
| Sum result | Arena global (0) | **Não** |
| CaptureBox | Arena global (0) | **Não** |

Em execução short-lived (compila + roda + morre), o SO recupera a memória.
Em execução long-lived (REPL, servidor), é vazamento permanente.

### Refcount ornamental

`kata_rt_incref` e `kata_rt_decref` existem, estão registrados no
`ffi_registry` e no `ffi_sigs`, mas o lowering **nunca os emite**. O
refcount do CaptureBox nasce em 1 e morre em 1. A arena global garante o
lifetime por acidente, não por design. O refcount é escrito mas nunca lido
para tomar decisão de liberação.

### `tail_pos` como proxy limitado para escape

A análise de escape atual é binária: "escapa para o caller" vs "não escapa"
(via `tail_pos: bool`). A Fase 13 (escape analysis com `EscapeKind`) foi
eliminada do PRD dos Fios 3+4+9 por decisão de design — wrapper-only: toda
closure com captures vai para a arena global sem análise. Não há noção de
"escapa para um ancestral mais distante" — o caso que surge quando fibers
comunicam-se via canais e o destinatário não é o pai direto.

## Modelo

### Árvore hierárquica de arenas

Cada fiber tem sua arena e recebe a arena do pai no `SpawnArgs`. A arena do
pai é o `caller_arena` — já existe hoje. A generalização é:

1. O scheduler rastreia a **árvore** de fibers, não uma fila plana. Cada
   `FiberEntry` conhece `parent_id`.
2. A destruição é **bottom-up**: um fiber só é destruído (e sua arena
   liberada) quando ele termina **e** todos os seus filhos terminaram.
3. A arena da raiz (substituindo a arena global) é destruída no fim do
   scheduler run.

```
root (arena 0)
├── fiber A (arena 1)
│   ├── fiber C (arena 3)
│   └── fiber D (arena 4)
└── fiber B (arena 2)
```

- C e D são irmãos → compartilham via arena 1 (pai comum)
- A e B são irmãos → compartilham via arena 0 (raiz)
- C e B → LCA é arena 0 (raiz)

### Escape analysis para LCA

Quando o typeck sabe que um valor vai cruzar a fronteira do fiber (via
canal, retorno para ancestral não-direto, ou shared state), ele marca o
destino como a arena do LCA. O codegen aloca diretamente na arena do LCA —
sem cópia posterior.

O `tail_pos` atual é o caso especial onde o LCA é o pai direto
(`caller_arena`). A generalização estende isso para ancestrais arbitrários.

### Destruição bottom-up

Hoje o scheduler destrói a arena do fiber imediatamente após `resume()`.
No modelo hierárquico, a arena só é destruída quando:

1. O fiber terminou, **e**
2. Todos os filhos do fiber terminaram (e suas arenas já foram destruídas)

Isso garante que objetos promovidos para a arena de um pai estão vivos
enquanto qualquer filho os referencia.

### Paralelismo sem compartilhamento

Fibers que não se comunicam não precisam de promoção. Cada fiber aloca na
sua arena, e tudo é liberado quando o fiber termina. Sem cópia, sem
 overhead.

## Escopo

### Scheduler: árvore de fibers

#### `FiberEntry` estendido

Hoje:

```rust
struct FiberEntry {
    fiber: KataFiber,
    spawn_args: SpawnArgs,
}
```

Pré-11:

```rust
struct FiberEntry {
    fiber: KataFiber,
    spawn_args: SpawnArgs,
    parent_id: Option<FiberId>,
    children: Vec<FiberId>,
    completed: bool,  // fiber terminou execução
}
```

Um fiber é "completado" quando `resume()` retorna. A arena só é destruída
quando `completed == true` **e** `children` está vazia (todos os filhos
foram destruídos).

#### `spawn` estendido

Hoje `spawn(fn_ptr, caller_arena, args_ptr)` não registra parentesco.
Pré-11: `spawn` registra o fiber atual como pai do novo fiber. Se não há
fiber atual (entry point), o pai é a raiz (fiber 0 virtual / arena global).

#### `run` estendido

Hoje: `run()` executa um fiber, destrói a arena, remove do map.

Pré-11: `run()` executa um fiber, marca `completed = true`. Se `children`
está vazia, destrói a arena e remove do map. Se `children` não está vazia,
mantém o fiber no map (arena ainda viva para os filhos). Quando um filho é
destruído, remove-se do `children` do pai e verifica se o pai pode ser
destruído (propagação bottom-up).

#### Arena raiz

A arena global (handle 0) deixa de ser criada no prólogo do `__kata_entry`.
Em vez disso, o scheduler cria a arena raiz no `scheduler_init` e a
destrói no fim do `run` (quando todos os fibers terminaram). O entry point
recebe a arena raiz como `caller_arena`.

### Codegen: alocação no LCA

#### `SpawnArgs` estendido

Hoje `SpawnArgs` carrega `fiber_arena` e `caller_arena`. Pré-11: o
`caller_arena` já é a arena do pai direto — a mudança é garantir que o
`caller_arena` passado seja de fato a arena do pai na árvore, não um
fallback hardcoded para 0.

#### Lowering de tupla/sum/capturebox

A escolha de arena hoje é:

```rust
let handle = if expr.tail_pos {
    ctx.caller_arena.unwrap_or_else(|| iconst(0))
} else {
    ctx.fiber_arena.unwrap_or_else(|| iconst(0))
};
```

Pré-11: o `tail_pos` é substituído por `EscapeTarget`, que indica o **nível
do LCA** — qual ancestral na árvore é o destino. O codegen traduz isso
para a arena apropriada.

Casos:

| Escape | Arena de alocação |
|---|---|
| Não escapa (local) | `fiber_arena` |
| Escapa para caller | `caller_arena` (pai direto) |
| Escapa para LCA distante | Arena do LCA (passada via SpawnArgs ou registrada) |
| Função pura / entry point | Arena raiz |

#### CaptureBox e Sum results

Hoje ambos alocam na arena global (handle 0) hardcoded. Pré-11: alocam na
arena determinada pelo escape analysis — que para closures e sums em
funções puras é a arena raiz, mas para closures/sums em Actions pode ser a
arena do fiber ou do caller, dependendo do escape.

### Escape analysis

O `tail_pos: bool` atual é uma análise de escape de 1 bit: "escapa para o
caller ou não". Pré-11 substitui por `EscapeTarget`, que sabe **para onde**
o valor escapa. A Fase 13 (escape analysis) foi eliminada do PRD dos Fios
3+4+9 por decisão de design (wrapper-only). O Pré-11 reintroduz a escape
analysis com semântica nova — não é a mesma análise que foi eliminada.

#### `EscapeTarget` na TAST

```rust
enum EscapeTarget {
    /// Valor local ao fiber — aloca em fiber_arena.
    Local,
    /// Valor escapa para o caller direto — aloca em caller_arena.
    Caller,
    /// Valor escapa para ancestral distante — aloca na arena do LCA.
    /// O índice é a profundidade do LCA relativa ao fiber atual.
    Ancestor(u32),
}
```

O typeck determina o `EscapeTarget` analisando:

1. **Retorno direto** (tail call, return): `Caller` se o valor é retornado
   para o caller direto.
2. **Canal/envio cross-fiber**: `Ancestor(n)` onde `n` é a profundidade do
   LCA entre o fiber que envia e o que recebe. (No Fio 11 — por ora, o
   typeck não tem canais, então este caso é preparado mas não exercitado.)
3. **Closure com captura**: se a closure escapa para o caller, as capturas
   também escapam — promovidas para a mesma arena.
4. **Função pura**: sempre `Ancestor(0)` (raiz), porque não há fiber_arena
   nem caller_arena.

#### Passo a passo

A Fase 13 (escape analysis) do PRD dos Fios 3+4+9 foi eliminada por decisão
de design — wrapper-only: toda closure com captures vai para a arena
global sem análise. O Pré-11 reintroduz escape analysis do zero, com
semântica diferente: `EscapeTarget` classifica o destino, não apenas
se escapa.

1. **Coleta de escape points**: para cada valor alocado, identifica os
   pontos onde ele escapa (return, envio por canal, armazenamento em
   estrutura compartilhada).
2. **Cálculo do LCA**: para cada escape point, determina o ancestral
   comum mais próximo entre o fiber que cria o valor e o fiber que o
   consome.
3. **Anotação na TAST**: cada nó de alocação recebe `EscapeTarget`.

Na ausência de canais (pré-Fio 11), os únicos escape points são:
- Retorno de Action → `Caller`
- Closure com captura que escapa → `Caller`
- Função pura → `Ancestor(0)` (raiz)

Isso cobre o caso atual sem introduzir dependência em canais.

### Runtime: arena raiz com destruction point

#### `kata_rt_scheduler_init` estendido

Hoje: cria `Scheduler::new()` vazio. A arena global é criada pelo codegen
no prólogo do `__kata_entry`.

Pré-11: `scheduler_init` cria a arena raiz e guarda o handle. O entry
point recebe a arena raiz como `caller_arena` em vez de criá-la.

#### `kata_rt_run` estendido

Hoje: executa um fiber e destrói a arena.

Pré-11: executa fibers até que todos terminem. Quando o último fiber é
destruído, destrói a arena raiz. Se o scheduler run completa, toda a
memória do programa foi liberada.

### `arena_destroy` semântica

Hoje `arena_destroy(handle)` faz `reset()` — substitui o `Bump` por `new()`.
O handle permanece válido no pool.

Pré-11: a semântica não muda, mas o scheduler garante que um handle só é
destruído quando nenhum outro fiber o referencia (bottom-up). O handle
permanece no pool (pode ser reusado por um novo fiber), mas a memória da
arena foi liberada.

## Casos não cobertos

### Fiber long-lived

Um fiber que permanece vivo indefinidamente enquanto spawna filhos curtos
acumula objetos promovidos na sua arena sem bound. A arena só seria
liberada quando o fiber terminar — o que não acontece.

Este caso exige reclamation granular (GC local à arena ou allocator
separado com free individual). É **localizado** — só afeta fibers
long-lived, não o modelo geral. Fica fora do escopo deste PRD. Ver nota
no ROADMAP.md.

### Paralelismo verdadeiro (multithread)

O modelo hierárquico é seguro para single-thread com fibers cooperativos.
Para multithread real (Fio 11 com `@parallel`), cada thread precisa de sua
própria árvore de arenas, e o compartilhamento entre threads exige cópia
(não há como compartilhar uma arena entre threads sem sincronização no
allocator). Isto é tratado no Fio 11.

## Dependência com Fios 3+4+9

O PRD dos Fios 3+4+9 já entregou:
- `CaptureInfo` e `CaptureStorage` (Stack/Heap) na TAST
- `kata_rt_alloc_arc`, `kata_rt_incref`, `kata_rt_decref` no runtime
- `collect_captures` — coleta free variables do body do lambda
- Closures wrapper-only: toda closure com captures aloca CaptureBox na
  arena global sem análise de escape

O que foi **transferido** dos Fios 3+4+9 para o Pré-11:
- **Fase 13** (escape analysis) — eliminada do PRD atual, reintroduzida
  aqui como `EscapeTarget` (semântica nova, com destino)
- **Fase 15** (ARC pass) — o lowering passa a emitir `incref`/`decref`

O que **permanece** no PRD dos Fios 3+4+9:
- **Fase 16** (TRMA) — ortogonal, não depende de escape analysis nem ARC

O Pré-11 **consome** a infraestrutura já entregue e a estende:
- `EscapeTarget` substitui `tail_pos: bool` (e o `EscapeKind` que nunca
  chegou a ser implementado)
- O ARC pass passa a ser **emitido** pelo codegen (hoje não é)
- A arena de alocação do CaptureBox deixa de ser hardcoded para handle 0
- A arena raiz ganha destruction point no fim do scheduler run

## DoD

1. **Arena raiz é destruída no fim do scheduler run.** Após `kata_rt_run`
   completar, toda a memória do programa foi liberada. Zero vazamentos
   para execução single-fiber.

2. **CaptureBox, Sum results e tuplas de função pura não vazam.** Eles são
   alocados na arena determinada pelo escape analysis, que é destruída no
   fim do run ou quando o fiber pai termina.

3. **Destruição bottom-up.** Um fiber cujos filhos ainda estão vivos não
   tem sua arena destruída. Teste: fiber pai spawna filho, filho termina
   antes do pai, arena do pai sobrevive até o pai terminar.

4. **Escape analysis anota `EscapeTarget` na TAST.** O codegen usa
   `EscapeTarget` para escolher a arena, não `tail_pos` binário.

5. **ARC pass é emitido.** O lowering chama `kata_rt_incref`/`decref` nos
   pontos apropriados. O refcount ainda não libera individualmente
   (bumpalo não suporta), mas o decremento é registrado para uso futuro
   (GC para long-lived).

6. **Testes E2E.** Programa que cria tuplas, sums e closures em função
   pura, executa, e termina sem vazamento (verificado por métrica de
   arena — `arena_alloc` bytes ≠ 0 durante execução, `arena_destroy`
   zera tudo no fim).

## Não faz parte deste PRD

- Canais, `fork!`, `select`, `@parallel` (Fio 11)
- GC para fibers long-lived (ver nota no ROADMAP.md)
- Multithread real (Fio 11)
- Free individual de blocos na arena (bumpalo não suporta)
- Comptime / `@cache_strategy` (Fio 12)