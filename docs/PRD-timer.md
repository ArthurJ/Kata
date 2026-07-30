# PRD — `@timer` diretiva

**Status:** Não implementado
**Data:** 2026-07-29
**Depende de:** Fio 14 ✅ (`@log` infra — publish/subscribe, tópicos), Fio 12 ✅ (`fn_id` canônico)
**Não depende de:** `@parallel`, Fio 13, Fio 15

## 1. Objetivo

Medir o tempo de execução de funções e actions em runtime, publicando o
resultado via infraestrutura existente de `@log`. Funciona com TCO
(recursão de cauda mede a cadeia inteira, não cada chamada).

### Princípio: reusa `@log`, não duplica

`@timer` é uma especialização de `@log`. O runtime já tem
`kata_rt_log_publish(topic, msg)` e `log_recv!("topic")`. O timer só
precisa de `kata_rt_timer_now()` (nova FFI) e reusa o publish/subscribe
existente. Zero infra nova de canal para output.

### Princípio: TCO preservado

`@timer` sozinho **não** desativa TCO. A medição usa um canal interno
buffer-1 com policy drop para preservar o timestamp da chamada mais
externa através da destruição de frames do `return_call`. `@cache` já
desativa TCO por si — quando combinado com `@timer`, a estratégia muda
para stack slot.

## 2. Sintaxe

```kata
@timer
fatorial :: Int => Int

@timer{topic: "perfil"}
fatorial :: Int => Int

@timer{stats: true}
fatorial :: Int => Int

@timer{stats: true, repeat: 100}
fatorial :: Int => Int

@timer{topic: "perfil", stats: true, repeat: 50, msg: "{name}: min={min}ms"}
fatorial :: Int => Int
```

## 3. Argumentos

| Argumento | Tipo | Default | Descrição |
|-----------|------|---------|-----------|
| `topic` | `Text` | nome da função | Canal de output via `kata_rt_log_publish` |
| `stats` | `Bool` | `false` | Se `true`, agrega min/max/mean sobre `repeat` amostras |
| `repeat` | `Int` | `1` (sem stats) ou `10` (com stats) | Janela de amostras antes de publicar |
| `msg` | `Text` | ver abaixo | Template interpolada da mensagem publicada |

### 3.1. Defaults de `msg`

- **Sem stats** (`stats: false`): `"{name}: {delta}ms"`
- **Com stats** (`stats: true`): `"{name}: min:{min}ms, mean:{mean}ms, max:{max}ms"`

### 3.2. Variáveis interpoladas

| Variável | Disponível | Descrição |
|----------|------------|-----------|
| `{name}` | sempre | Nome da função/action |
| `{delta}` | sem stats | Delta de tempo da chamada (milissegundos) |
| `{min}` | com stats | Mínimo da janela de amostras (milissegundos) |
| `{mean}` | com stats | Média da janela de amostras (milissegundos) |
| `{max}` | com stats | Máximo da janela de amostras (milissegundos) |

A interpolação usa o mesmo mecanismo do `@log` — o codegen resolve
`{name}`, `{delta}`, `{min}`, `{mean}`, `{max}` no `var_map`.

## 4. Mecanismo

### 4.1. Dois canais

**Canal interno** (buffer-1, drop policy) — preserva `start_time` através
da destruição de frames do TCO. Key = `fn_id` (hash FNV-1a canônico, já
existente do `@cache`). É detalhe de implementação, invisível para o
usuário.

**Canal de output** (`topic`) — onde o delta computado é publicado via
`kata_rt_log_publish`. Key = `topic`, default = nome da função. É o que
o usuário consome com `log_recv!("topic")`.

### 4.2. Seleção de estratégia via `tail_pos`

A TAST já marca `tail_pos: true` em chamadas elegíveis para TCO. Antes
de baixar a função, um walk na TAST responde: "este corpo tem `Closure
{ tail_pos: true, ffi_symbol: None }`?"

Árvore de decisão:

```
@timer presente?
├── @cache também presente?
│   → no_tail_calls = true (já é assim)
│   → sem return_call, frame sobrevive
│   → stack slot + hit jumpa para epilogue
│
├── @timer sozinho, body tem tail_pos call?
│   → TCO ativo, return_call destrói frame
│   → canal interno buffer-1 drop (first-write-wins)
│   → prólogo: timer_now() !> canal_interno
│   → epílogo: start = <! canal_interno; delta = now - start; publish
│
└── @timer sozinho, body NÃO tem tail_pos call?
    → sem return_call, frame sobrevive
    → stack slot (start no stack_slot do frame)
    → prólogo: start = timer_now()
    → epílogo: delta = timer_now() - start; publish
```

### 4.3. Caso TCO — canal first-write-wins

```
prólogo:
  start = kata_rt_timer_now()
  timer_chan_{fn_id} !> start        // send com drop policy

epílogo (só base case — return_call não chega aqui):
  start = <! timer_chan_{fn_id}      // recebe o timestamp mais externo
  delta = kata_rt_timer_now() - start
  publish(topic, format_msg(name, delta))
```

Chamada 1 envia start₁ → canal tem start₁. Chamada 2 (`return_call`)
envia start₂ → drop, canal mantém start₁. Caso base recebe start₁.
Delta = cadeia inteira. Frame reusado não importa — o canal vive na
heap, não no stack.

### 4.4. Caso não-TCO — stack slot

```
prólogo:
  start = kata_rt_timer_now()         // stack_slot do frame

epílogo:
  delta = kata_rt_timer_now() - start
  publish(topic, format_msg(name, delta))
```

### 4.5. Caso `@cache` + `@timer`

`@cache` já desativa TCO (`no_tail_calls = true`). O hit block passa a
jumpar para o epilogue em vez de `return_` direto. O epilogue recebe
`is_hit: i64` (0 ou 1):

```
epilogue(result, is_hit):
  if is_hit:
    delta = ~0                        // cache hit — tempo negligível
  else:
    delta = timer_now() - start       // medição normal
    cache_insert(...)
  publish(topic, format_msg(name, delta))
  return_(result)
```

Isso também conserta a limitação existente do `@log Exit` em cache hit
— o epilogue sempre dispara, hit ou miss.

### 4.6. `stats: true` — agregação

Quando `stats: true`, o runtime acumula `repeat` amostras de delta num
buffer antes de publicar. O buffer vive no cache handle (reusando a
mesma TLS HashMap do `@cache`) ou num storage per-fiber.

```
epilogue (a cada chamada):
  delta = timer_now() - start
  stats_buffer.push(delta)
  if stats_buffer.len() == repeat:
    min = stats_buffer.min()
    max = stats_buffer.max()
    mean = stats_buffer.mean()
    publish(topic, format_msg(name, min, mean, max))
    stats_buffer.clear()
```

Sem `stats`, cada chamada publica imediatamente.

### 4.7. Interação com outras diretivas

Ordem no codegen de Sig:

```
1. bind_patterns_to_params
2. @timer start                    ← antes de tudo
3. @log Enter (se presente)
4. @cache lookup (se presente)
5. [hit → jump epilogue com is_hit=1]
6. [miss → body]
7. epilogue_block:
   a. @log Exit (se presente)
   b. @cache insert (se miss)
   c. @timer stop + publish
   d. return_(result)
```

## 5. Runtime

### 5.1. Nova FFI

```c
// Clock monotônico em nanossegundos.
i64 kata_rt_timer_now(void);
```

### 5.2. Canal interno

O canal interno buffer-1 com drop policy pode ser implementado como:
- Um slot no fiber struct do scheduler (per-fiber, evita race entre
  fibers que chamam a mesma função)
- Ou reusando a TLS HashMap do `@cache` indexada por `fn_id`

Requisito: `start_time` sobrevive à destruição de frame do TCO e não é
compartilhado entre fibers concorrentes.

### 5.3. Buffer de stats

Quando `stats: true`, um buffer de `repeat` amostras é mantido por
função. Pode reusar a mesma TLS HashMap do `@cache`, indexada por
`fn_id`, ou viver no fiber struct.

## 6. Codegen

### 6.1. Parser

`@timer` é reconhecido como diretiva antes de assinatura de função ou
action. Argumentos: `topic`, `stats`, `repeat`, `msg`.

### 6.2. Resolver

Registra `@timer` como diretiva válida em Sigs. `TimerSpec` na TAST:

```rust
struct TimerSpec {
    topic: String,           // default = nome da função
    stats: bool,             // default = false
    repeat: u32,             // default = 1 ou 10
    msg: String,             // template
}
```

### 6.3. Lowering

Em `define_function_body`:

1. Walk na TAST do body para detectar `tail_pos` calls (seleção de
   estratégia)
2. Prólogo: `kata_rt_timer_now()` → stack slot ou canal interno
3. Epílogo: computar delta, formatar msg, `kata_rt_log_publish`
4. Se `@cache` presente: hit jumpa para epilogue com `is_hit = 1`

**`no_tail_calls` não muda.** `@timer` não desativa TCO — a estratégia
canal preserva o timestamp através da destruição de frames. A flag
continua:

```rust
no_tail_calls: cache_spec.is_some(),
```

O timer se adapta à presença ou ausência de TCO, não o força.

### 6.4. Detecção de tail_pos

Walk na TAST antes do lowering:

```rust
fn has_tail_pos_call(clauses: &[TypedClause]) -> bool {
    clauses.iter().any(|c| {
        walk_typed_expr(&c.body, &|e| {
            matches!(&e.kind, TypedExprKind::Closure {
                tail_pos: true, ffi_symbol: None, ..
            })
        })
    })
}
```

Se `has_tail_pos_call && !cache_spec` → estratégia canal.
Senão → estratégia stack slot.

## 7. Limitações 1.0

- `@timer` em funções com múltiplas cláusulas: só a primeira cláusula
  tem prólogo (timer start). Cláusulas subsequentes podem não passar
  pelo epilogue. Mesma limitação do `@log Exit` com multi-clause.
- `@timer` + `@cache` hit: delta é ~0 (tempo de lookup). O usuário vê
  que o cache funcionou.
- Canal interno per-fiber: se o scheduler não tiver slot para
  `start_time`, fallback para TLS (aceitando race entre fibers que
  chamam a mesma função — função pura não faz yield, então o race é
  improvável na prática).

## 8. DoD

- `@timer` em `fatorial` recursivo de cauda mede a cadeia inteira
  (delta = N chamadas), não cada chamada individual. TCO preservado.
- `@timer{topic: "perfil"}` publica no tópico "perfil". Consumidor
  faz `log_recv!("perfil")` e recebe eventos de timing.
- `@timer{stats: true, repeat: 100}` acumula 100 amostras, publica
  `"{name}: min:{min}ms, mean:{mean}ms, max:{max}ms"`, zera, repete.
- `@timer` + `@cache`: hit reporta delta ~0, miss reporta delta real.
- `@timer` em função sem TCO: mede cada chamada individualmente via
  stack slot.
- `msg` custom: `@timer{msg: "{name}: {delta}ms"}` produz saída
  formatada conforme template.