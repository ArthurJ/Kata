# PRD — `@cache` estratégias FIFO/MRU/LFU + capacity configurável

**Status:** Não implementado
**Data:** 2026-08-28
**Depende de:** Fio 12 ✅ (`@cache` LRU funcional, `fn_id` canônico, type descriptors)
**Não depende de:** `@timer`, `@log`, diretivas customizadas

## 1. Objetivo

Generalizar `@cache` para suportar múltiplas estratégias de eviction (LRU,
FIFO, MRU, LFU) e tornar a capacidade configurável em vez de hardcoded em 256.

### Princípio: enum, não string

A estratégia deixa de ser uma `String` opaca no `CacheSpec` e vira um enum
`CacheStrategy { LRU, FIFO, MRU, LFU }`. O parser ainda aceita strings no dict
(`"LRU"`, `"FIFO"`, `"MRU"`, `"LFU"`), mas a conversão para enum acontece no
resolution — string inválida é erro de compilação, não fallback silencioso para
LRU.

### Princípio: `@cache` sem args

Hoje `@cache` sem `{strategy: "LRU"}` não ativa cache (o resolution deixa
`cache_strategy = None`). Com este PRD, `@cache` sozinho ativa LRU com capacity
256. `@cache{}` (dict vazio) idem.

### Princípio: capacity é entradas, não bytes

`capacity: N` significa N entradas na tabela, não N bytes. O usuário controla
quantos resultados distintos podem estar cacheados simultaneamente. O consumo
de memória por entrada depende do tipo do valor (escalares = 8 bytes, tipos
complexos = ponteiros para arena).

## 2. Sintaxe

```kata
@cache                                    -- LRU, 256 (defaults)
@cache{}                                  -- LRU, 256 (idem)
@cache{strategy: "LRU"}                   -- LRU, 256
@cache{strategy: "FIFO"}                  -- FIFO, 256
@cache{strategy: "MRU"}                   -- MRU, 256
@cache{strategy: "LFU"}                   -- LFU, 256
@cache{capacity: 512}                     -- LRU, 512
@cache{strategy: "FIFO", capacity: 512}  -- FIFO, 512
@cache{capacity: 8}                      -- LRU, 8 (testes de eviction)
```

## 3. Argumentos

| Argumento | Tipo | Default | Descrição |
|-----------|------|---------|-----------|
| `strategy` | `Text` | `"LRU"` | Política de eviction: `"LRU"`, `"FIFO"`, `"MRU"`, `"LFU"` |
| `capacity` | `Int` | `256` | Número máximo de entradas antes de eviction |

### 3.1. Validação

- `strategy` deve ser exatamente `"LRU"`, `"FIFO"`, `"MRU"`, ou `"LFU"`. Qualquer
  outro valor é erro de compilação: `type.unknown_cache_strategy`.
- `capacity` deve ser `Int > 0`. `capacity: 0` é erro: `type.cache_capacity_zero`.
  `capacity` negativo é erro: `type.cache_capacity_negative`.
- Ambos os campos são opcionais e independentes — qualquer subconjunto é válido.

## 4. Estratégias

### 4.1. LRU (Least Recently Used) — existente

Evicta a entrada com menor timestamp de acesso (lookup ou insert). Cada
lookup atualiza o timestamp. Comportamento já implementado.

**Caso de uso:** funções com localidade temporal — argumentos repetidos tendem
a ser reusados. Padrão recomendado para a maioria dos casos.

### 4.2. FIFO (First In First Out) — novo

Evicta a entrada inserida há mais tempo (ordem de inserção). O lookup **não**
atualiza a prioridade — só o insert conta. A entrada mais antiga é evicta
independentemente de acessos recentes.

**Caso de uso:** quando o valor de cada argumento é computado uma vez e não há
razão para privilegiar acessos recentes. Mais simples e previsível que LRU.
Overhead menor (sem update no lookup).

### 4.3. MRU (Most Recently Used) — novo

Evicta a entrada com maior timestamp de acesso (a mais recentemente tocada).
Cada lookup atualiza o timestamp. Inverso de LRU.

**Caso de uso:** padrões de acesso onde o mais recente é o menos provável de
reaparecer — ex: buffers circulares, janelas deslizantes, workloads onde cada
novo argumento é distinto e o reuso é de argumentos antigos.

### 4.4. LFU (Least Frequently Used) — novo

Evicta a entrada com menor contagem de acessos (lookup + insert). Cada lookup
incrementa `access_count`. O insert inicializa `access_count = 1`.

**Caso de uso:** workloads onde frequência é estável — poucos argumentos
distintos, muitos hits em cada. Ex: função de lookup em tabela estática, hash
de valores de um domínio pequeno.

**New-key penalty:** toda chave recém-inserida tem `access_count = 1`. Se o
cache está cheio e entra uma chave nova, ela é imediatamente a candidata a
eviction na próxima inserção. Em workloads com muitos argumentos distintos
(o caso comum de memoização), LFU pura evicta as entradas que acabaram de
chegar — justamente as que mais custam para recomputar.

Exemplo: `fib 100` com capacity 50. fib(0..49) são acessadas muitas vezes,
fib(50..99) uma vez cada. LFU mantém fib(0..49) (frequentes) e evicta
fib(50..99) (count=1). Mas fib(50..99) são as que custam caro para recomputar.

A solução clássica é aging (decair contagens periodicamente), mas isso
introduz parâmetros (taxa de decay, intervalo) e complexidade similar a TTL.
Este PRD implementa **LFU pura (sem aging)**. A ressalva é documentada — quem
escolher LFU sabe que é ideal para frequência estável e pior para workloads
com alta cardinalidade de argumentos.

## 5. Mecanismo

### 5.1. Cadeia de mudanças

```
Parser     → já aceita dict com pares key:value. Sem mudança estrutural.
Resolution → extrai strategy (String) + capacity (Int) do dict. Valida.
             Se @cache presente (mesmo sem args), produz defaults.
Inference  → CacheSpec carrega enum CacheStrategy + capacity: i64.
Codegen    → passa capacity dinâmico ao invés de iconst(256).
             Passa strategy_tag (i64) ao runtime via FFI.
Runtime    → CacheTable guarda CacheStrategy. Eviction dispatch por estratégia.
```

### 5.2. Mudanças por camada

#### Parser (`kata-parser/src/directives.rs`)

Sem mudança. O parser já aceita `DirectiveArg::Named { key, value }` para
qualquer diretiva. A extração de campos específicos acontece no resolution.

#### Resolution (`kata-resolution/src/lib.rs`)

Hoje (linhas 272-281):

```rust
"cache" => {
    for arg in &d.args {
        if let DirectiveArg::Named { key, value } = arg
            && key == "strategy"
            && let Expr::TextLit { text } = &value.node
        {
            cache_strategy = Some(text.clone());
        }
    }
}
```

Novo:

```rust
"cache" => {
    // @cache presente — ativa com defaults mesmo sem args.
    cache_strategy = Some("LRU".to_string());
    cache_capacity = Some(256);
    for arg in &d.args {
        if let DirectiveArg::Named { key, value } = arg {
            match key.as_str() {
                "strategy" => {
                    if let Expr::TextLit { text } = &value.node {
                        match text.as_str() {
                            "LRU" | "FIFO" | "MRU" => {
                                cache_strategy = Some(text.clone());
                            }
                            _ => errors.push(ResolveError::UnknownCacheStrategy {
                                strategy: text.clone(),
                                item_name: name.clone(),
                            }),
                        }
                    }
                }
                "capacity" => {
                    if let Expr::IntLit { text } = &value.node
                        && let Ok(n) = text.parse::<i64>()
                    {
                        if n <= 0 {
                            errors.push(ResolveError::CacheCapacityInvalid {
                                value: n,
                                item_name: name.clone(),
                            });
                        } else {
                            cache_capacity = Some(n);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
```

`FunctionDef` (types.rs) ganha campo:

```rust
pub cache_capacity: Option<i64>,
```

#### Inference (`kata-inference/src/typed_module.rs`)

`CacheSpec` muda de:

```rust
pub struct CacheSpec {
    pub strategy: String,
}
```

para:

```rust
pub enum CacheStrategy {
    LRU,
    FIFO,
    MRU,
    LFU,
}

pub struct CacheSpec {
    pub strategy: CacheStrategy,
    pub capacity: i64,
}
```

`function_infer.rs` mapeia:

```rust
cache_spec: func_def.cache_strategy.as_ref().map(|s| CacheSpec {
    strategy: match s.as_str() {
        "LRU" => CacheStrategy::LRU,
        "FIFO" => CacheStrategy::FIFO,
        "MRU" => CacheStrategy::MRU,
        "LFU" => CacheStrategy::LFU,
        _ => CacheStrategy::LRU, // inalcançável — resolution já validou
    },
    capacity: func_def.cache_capacity.unwrap_or(256),
}),
```

#### Codegen (`kata-codegen/src/lowering/function_def.rs`)

Linha 291 — capacity deixa de ser hardcoded:

```rust
// Antes:
let cap_val = builder.ins().iconst(I64, 256);

// Depois:
let cap_val = builder.ins().iconst(I64, cache_spec.as_ref().map_or(256, |s| s.capacity));
```

Linha 300 — `kata_rt_cache_get_or_create` ganha um parâmetro `strategy_tag`:

```rust
// Antes:
let handle = builder.ins().call(*get_fn, &[arena_handle, fn_id_val, cap_val]);

// Depois:
let strategy_tag = builder.ins().iconst(I64, cache_strategy_to_tag(cache_spec));
let handle = builder.ins().call(*get_fn, &[arena_handle, fn_id_val, cap_val, strategy_tag]);
```

Onde `cache_strategy_to_tag` retorna: LRU=0, FIFO=1, MRU=2, LFU=3.

#### Runtime (`kata-rt/src/cache.rs`)

`CacheTable` muda para guardar a estratégia:

```rust
struct CacheTable {
    entries: HashMap<Vec<u8>, i64>,
    capacity: usize,
    strategy: CacheStrategy,
    /// Contador global de acessos (lookup + insert).
    access_counter: i64,
    /// Último acesso de cada key (para LRU/MRU).
    last_access: HashMap<Vec<u8>, i64>,
    /// Ordem de inserção de cada key (para FIFO).
    insert_order: HashMap<Vec<u8>, i64>,
    /// Contador de inserção (para FIFO).
    insert_counter: i64,
    /// Contagem de acessos por key (para LFU).
    access_count: HashMap<Vec<u8>, i64>,
}

enum CacheStrategy {
    LRU = 0,
    FIFO = 1,
    MRU = 2,
    LFU = 3,
}
```

`kata_rt_cache_get_or_create` ganha `strategy_tag`:

```rust
pub extern "C" fn kata_rt_cache_get_or_create(
    _arena_handle: i64,
    fn_id: i64,
    capacity: i64,
    strategy_tag: i64,
) -> i64 {
    let strategy = match strategy_tag {
        1 => CacheStrategy::FIFO,
        2 => CacheStrategy::MRU,
        3 => CacheStrategy::LFU,
        _ => CacheStrategy::LRU,
    };
    // cria CacheTable com strategy
    ...
}
```

`kata_rt_cache_lookup` — sem mudança estrutural. Atualiza `last_access` para
LRU/MRU e `access_count` para LFU. Para FIFO, não atualiza nada (só insert
conta).

`kata_rt_cache_insert` — eviction dispatch:

```rust
fn evict(table: &mut CacheTable) -> Option<Vec<u8>> {
    match table.strategy {
        CacheStrategy::LRU => {
            // Evicta menor last_access.
            table.last_access.iter()
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::FIFO => {
            // Evicta menor insert_order.
            table.insert_order.iter()
                .min_by_key(|(_, order)| *order)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::MRU => {
            // Evicta maior last_access.
            table.last_access.iter()
                .max_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::LFU => {
            // Evicta menor access_count.
            table.access_count.iter()
                .min_by_key(|(_, count)| *count)
                .map(|(k, _)| k.clone())
        }
    }
}
```

No insert:

```rust
// LRU e MRU: atualizam last_access no insert.
// FIFO: atualiza insert_order no insert, não atualiza last_access.
// LFU: inicializa access_count = 1 no insert.
table.access_counter += 1;
table.entries.insert(key.clone(), value);
match table.strategy {
    CacheStrategy::LRU | CacheStrategy::MRU => {
        table.last_access.insert(key, table.access_counter);
    }
    CacheStrategy::FIFO => {
        table.insert_counter += 1;
        table.insert_order.insert(key, table.insert_counter);
    }
    CacheStrategy::LFU => {
        table.access_count.insert(key, 1);
    }
}
```

No lookup:

```rust
// LRU e MRU: atualizam last_access no lookup.
// LFU: incrementa access_count no lookup.
// FIFO: não atualiza — só insert determina ordem de eviction.
match table.strategy {
    CacheStrategy::LRU | CacheStrategy::MRU => {
        table.access_counter += 1;
        table.last_access.insert(key, table.access_counter);
    }
    CacheStrategy::LFU => {
        *table.access_count.get_mut(&key).unwrap() += 1;
    }
    CacheStrategy::FIFO => {}
}
```

#### FFI Registry (`kata-codegen/src/ffi_registry.rs`)

A assinatura de `kata_rt_cache_get_or_create` muda de 3 para 4 parâmetros. O
registro no `ffi_registry.rs` e as assinaturas em `ffi_sigs/comptime.rs`
precisam ser atualizadas.

#### Monomorph (`kata-monomorph/src/instantiate.rs`)

Linha 45 — `cache_spec: orig.cache_spec.clone()` já propaga `CacheSpec` inteiro.
Sem mudança — `Clone` cobre os novos campos.

## 6. FFI

### 6.1. Assinaturas

| FFI | Antes | Depois |
|-----|-------|--------|
| `kata_rt_cache_get_or_create` | `(arena, fn_id, capacity) → handle` | `(arena, fn_id, capacity, strategy_tag) → handle` |
| `kata_rt_cache_lookup` | `(handle, key_ptr, key_len) → i64` | sem mudança |
| `kata_rt_cache_insert` | `(handle, key_ptr, key_len, value) → ()` | sem mudança |
| `kata_rt_serialize_key` | `(value, desc_ptr, desc_len, out_ptr, out_cap) → i64` | sem mudança |

`strategy_tag`: `0=LRU`, `1=FIFO`, `2=MRU`, `3=LFU`.

### 6.2. Quebra de ABI

A mudança no número de parâmetros de `kata_rt_cache_get_or_create` quebra a
ABI do código existente. Como o compilador e o runtime são compilados juntos
(cargo workspace) e o JIT é intra-processo, não há binários distribuídos — a
quebra é transparente.

## 7. Testes E2E

### 7.1. Casos existentes (regressão)

Os 11 testes atuais de `cache_e2e.rs` devem continuar passando sem mudança.
Eles usam `@cache{strategy: "LRU"}` com capacity 256 (default) — o
comportamento de LRU não muda.

### 7.2. Novos casos — FIFO

**FIFO básico** — `@cache{strategy: "FIFO"}` com mesma assinatura dos testes
existentes. Verifica que FIFO funciona para o caso simples (igual a LRU quando
não há eviction).

**FIFO eviction** — capacity=3, insere 4 keys distintas. A primeira inserida
(k1) deve ser evicta (não a menos recentemente acessada). Teste:

```kata
@cache{strategy: "FIFO", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)   -- miss, insert k1
    echo!(f 2)   -- miss, insert k2
    echo!(f 3)   -- miss, insert k3
    echo!(f 1)   -- HIT (k1 ainda está — só 3 entradas)
    echo!(f 4)   -- miss, insert k4, evict k1 (FIFO: primeira inserida)
    echo!(f 1)  2 -- miss (k1 foi evicta)
main!()
```

**FIFO não promove por acesso** — capacity=2, insere k1, k2. Acessa k1. Insere
k3. FIFO evicta k1 (primeira inserida), não k2 (menos recentemente acessada).
Prova que lookup não afeta eviction em FIFO.

### 7.3. Novos casos — MRU

**MRU básico** — `@cache{strategy: "MRU"}` com cláusula única. Comportamento
igual a LRU quando não há eviction.

**MRU eviction** — capacity=3, insere k1, k2, k3. Acessa k1 (k1 vira MRU). Insere
k4. MRU evicta k1 (mais recentemente acessada), não k2. Teste:

```kata
@cache{strategy: "MRU", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)   -- miss, insert. last_access: k1=1
    echo!(f 2)   -- miss, insert. last_access: k2=2
    echo!(f 3)   -- miss, insert. last_access: k3=3
    echo!(f 1)   -- HIT. last_access: k1=4 (agora é MRU!)
    echo!(f 4)   -- miss, evict k1 (maior last_access), insert k4
    echo!(f 1)   -- miss (k1 foi evicta)
    echo!(f 2)   -- HIT (k2 sobreviveu)
main!()
```

### 7.4. Novos casos — capacity

**Capacity default** — `@cache{strategy: "LRU"}` sem `capacity` → 256. Teste
existente já cobre (fib 35 gera 36 entradas < 256).

**Capacity explícita** — `@cache{capacity: 2}` (strategy default LRU). fib 10
com capacity 2: eviction frequente, mas resultado correto (cache parcial).

**Capacity inválida** — `@cache{capacity: 0}` → erro de compilação
`type.cache_capacity_zero`. `@cache{capacity: -5}` → erro
`type.cache_capacity_negative`.

### 7.5. Novos casos — LFU

**LFU básico** — `@cache{strategy: "LFU"}` com cláusula única. Comportamento
igual aos outros quando não há eviction.

**LFU eviction** — capacity=3, insere k1, k2, k3. Acessa k1 duas vezes (count=3).
Acessa k2 uma vez (count=2). Insere k4. LFU evicta k3 (count=1, menor). Teste:

```kata
@cache{strategy: "LFU", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)   -- miss, insert. count: k1=1
    echo!(f 2)   -- miss, insert. count: k2=1
    echo!(f 3)   -- miss, insert. count: k3=1
    echo!(f 1)   -- HIT. count: k1=2
    echo!(f 1)   -- HIT. count: k1=3
    echo!(f 2)   -- HIT. count: k2=2
    echo!(f 4)   -- miss, evict k3 (count=1, menor), insert k4. count: k4=1
    echo!(f 3)   -- miss (k3 foi evicta)
    echo!(f 2)   -- HIT (k2 sobreviveu, count=2)
    echo!(f 1)   -- HIT (k1 sobreviveu, count=3)
main!()
```

**LFU new-key penalty** — capacity=2, insere k1, k2. Acessa k1 5 vezes (count=6).
Insere k3 (count=1). Insere k4 (count=1). LFU evicta k3 (count=1, menor — acabou
de chegar). Prova que LFU pura sem aging pune entradas novas.

### 7.6. Novos casos — @cache sem args

**@cache sozinho** — `@cache` (sem dict) ativa LRU 256. `dobro 5` → 10.

**@cache{} (dict vazio)** — `@cache{}` idem.

## 8. Interação com outras diretivas

### 8.1. `@cache` + `@timer`

Sem mudança. `@cache` já desativa TCO (`no_tail_calls = true`). A estratégia de
eviction é ortogonal à medição de tempo. O hit block jumpa para o epílogo onde
`cache_insert` e `timer_stop` executam, nesta ordem (PRD-timer §4.7).

### 8.2. `@cache` stacking com diretivas customizadas

Sem mudança. `@cache` é intrínseca (PRD-diretivas D9) — não migra para
diretivas customizadas. O stacking onion já funciona: diretiva customizada
envolve `@cache` no modelo cebola.

## 9. Decisões de design

| # | Decisão | Racional |
|---|---------|---------|
| D1 | `CacheStrategy` é enum, não String | String opaca permite `"FIFO"` silenciosamente virar LRU. Enum torna estratégia desconhecida impossível após o resolution. O parser aceita string, mas o resolution valida e converte — erro de compilação, não fallback. |
| D2 | `capacity` é entradas, não bytes | Bytes exigiria o usuário estimar o tamanho serializado de cada valor — conhecimento de layout interno. Entradas é o que o usuário controla: "quero no máximo 512 resultados distintos cacheados". |
| D3 | `@cache` sem args ativa cache com defaults | Hoje `@cache` sem `{strategy: "LRU"}` é no-op silencioso. Surpreendente. Defaults explícitos tornam `@cache` sozinho equivalente a `@cache{strategy: "LRU", capacity: 256}`. |
| D4 | `strategy_tag` como i64 na FFI | Mantém ABI C. Enum Rust não atravessa FFI. Tag numérica é estável e trivial de debugar. LRU=0 preserva compatibilidade: se o tag for omitido (caso antigo), 0=LRU. |
| D5 | FIFO não atualiza `last_access` no lookup | FIFO é definido por ordem de inserção. Se lookup promovesse, viraria LRU. A distinção é semântica, não implementação. |
| D6 | `insert_order` como HashMap separado | Não reusa `access_counter` porque FIFO precisa distinguir insert de lookup. `VecDeque` seria mais eficiente (O(1) pop front) mas requer sincronização com `entries`. HashMap é consistente com `last_access` e simples. Eviction é O(n) scan, mas cache é para workloads onde o custo da função cachedada domina — O(n) eviction é negligenciável. |
| D7 | Validação de capacity no resolution, não no runtime | Erro de compilação é melhor que comportamento indefinido em runtime. `capacity: 0` no runtime significaria cache sempre vazio (sempre miss) — útil para debug, mas confuso se não for intencional. |
| D8 | Default capacity permanece 256 | 256 cobre fib 35 (36 entradas) com folga. É um bom default para a maioria dos casos sem desperdiçar memória. |
| D9 | LFU pura sem aging | LFU pura tem new-key penalty (entradas novas têm count=1 e são evictas primeiro). Aging resolveria mas introduz parâmetros (taxa de decay, intervalo) com complexidade similar a TTL. LFU pura é útil para frequência estável; LRU permanece default para memoização geral. |
| D10 | Eviction é O(n) scan | Todas as estratégias fazem scan linear no HashMap para encontrar a vítima. Para capacity típico (≤1000), isto é irrelevante frente ao custo da função cachedada. Otimização (min-heap, LRU doubly-linked list) é PRD separado se perf for mensurado como problema. |

## 10. Fora do escopo

- **LFU com aging** — LFU pura (sem decay de contagens) tem new-key penalty.
  Aging (decair contagens periodicamente) resolve isso mas introduz parâmetros
  (taxa de decay, intervalo). Adiar até haver caso de uso real onde LFU pura
  não é suficiente.
- **TTL (Time To Live)** — não se aplica a `@cache`. `@cache` memoiza funções
  puras — o resultado é sempre o mesmo para os mesmos argumentos, nunca
  stale. TTL resolve staleness, que não existe aqui. Como mecanismo de redução
  de footprint, capacity já é o bound correto e determinístico. TTL seria uma
  segunda política de eviction por cima de capacity, adicionando timer,
  sweep (lazy vs. active) e granularidade (global vs. por entrada) — tudo para
  resolver um problema que capacity já resolve. Removido permanentemente.
- **Tamanho em bytes** — limitar memória consumida em vez de número de
  entradas. Exige estimativa de tamanho por valor (type descriptor já tem a
  informação, mas o overhead de tracking é significativo). Adiar.
- **Cache persistente entre runs** — hoje o cache vive em TLS e morre com a
  thread. Persistência exige serialização de entries para disco. PRD separado.
- **Migração arena-allocated** — o `_arena_handle` é ignorado hoje. Migrar
  o cache para a arena do fiber (em vez de Rust heap TLS) é uma decisão
  arquitetural que afeta lifecycle do cache. PRD separado.
- **Otimização de eviction** — eviction é O(n) scan no HashMap. Estruturas
  dedicadas (min-heap para LFU, doubly-linked list para LRU) reduziriam para
  O(log n) ou O(1). Irrelevante para capacity típico (≤1000). Adiar até perf
  ser mensurada como problema.

## 11. DoDs (Definitions of Done)

### Fase 1 — Tipos e validação

1. `CacheStrategy` enum com 4 variantes (LRU, FIFO, MRU, LFU) em
   `kata-inference/src/typed_module.rs`.
2. `CacheSpec` carrega `strategy: CacheStrategy` + `capacity: i64`.
3. `FunctionDef` tem `cache_capacity: Option<i64>`.
4. Resolution extrai `strategy` e `capacity` do dict, valida, produz erros
   `UnknownCacheStrategy` e `CacheCapacityInvalid`.
5. `@cache` sem args e `@cache{}` produzem defaults (LRU, 256).
6. `function_infer.rs` mapeia string → enum e capacity → i64.

### Fase 2 — Codegen

7. `function_def.rs` usa `cache_spec.capacity` em vez de `iconst(256)`.
8. `kata_rt_cache_get_or_create` recebe `strategy_tag` como 4º parâmetro.
9. `ffi_registry.rs` e `ffi_sigs/comptime.rs` atualizados para 4 params.
10. Helper `cache_strategy_to_tag` (LRU=0, FIFO=1, MRU=2, LFU=3).

### Fase 3 — Runtime

11. `CacheTable` guarda `CacheStrategy` + `insert_order` + `insert_counter`
    + `access_count`.
12. `kata_rt_cache_get_or_create` faz match do tag e cria tabela com estratégia.
13. `kata_rt_cache_lookup` atualiza `last_access` (LRU/MRU) ou `access_count`
    (LFU). FIFO não atualiza.
14. `kata_rt_cache_insert` evicta por estratégia (LRU=min access, FIFO=min
    insert_order, MRU=max access, LFU=min count).
15. `kata_rt_cache_insert` atualiza `last_access` (LRU/MRU), `insert_order`
    (FIFO), ou `access_count=1` (LFU) conforme estratégia.

### Fase 4 — Testes E2E

16. 11 testes existentes passam sem mudança (regressão LRU).
17. Teste FIFO básico (cláusula única, sem eviction).
18. Teste FIFO eviction (capacity=3, 4 keys, primeira inserida evicta).
19. Teste FIFO não-promove-por-acesso (capacity=2, lookup não afeta eviction).
20. Teste MRU básico (cláusula única, sem eviction).
21. Teste MRU eviction (capacity=3, MRU evicta o mais recentemente acessado).
22. Teste LFU básico (cláusula única, sem eviction).
23. Teste LFU eviction (capacity=3, evicta menor access_count).
24. Teste LFU new-key penalty (capacity=2, entradas novas são evictas primeiro).
25. Teste capacity explícita (capacity=2 com LRU, resultado correto).
26. Teste `@cache` sem args (LRU 256 default).
27. Teste `@cache{}` (dict vazio, LRU 256 default).
28. Teste capacity inválida — `@cache{capacity: 0}` → erro de compilação.

### Fase 5 — Documentação

29. `Kata-lang-manual.md` §diretivas: atualizar `@cache` com sintaxe completa
    (`strategy`, `capacity`, defaults, validação).
30. `Kata-lang-manual.md` §runtime FFI: atualizar assinatura de
    `kata_rt_cache_get_or_create` (4 params, `strategy_tag`).
31. `mapa-funcionalidades.md`: corrigir `@cache_strategy` → `@cache`.
32. `examples/cache.kata`: adicionar exemplos de FIFO, MRU, LFU, capacity.
33. `sintaxe-mapa.md`: atualizar catálogo de diretivas com `@cache` (strategy +
    capacity).

## 12. Cronograma

| Fase | Escopo | Estimativa |
|------|--------|------------|
| 1 | Tipos + validação | ~30 min |
| 2 | Codegen | ~20 min |
| 3 | Runtime (LRU/FIFO/MRU/LFU) | ~50 min |
| 4 | Testes E2E (13 novos) | ~50 min |
| 5 | Documentação (manual, mapa, examples) | ~30 min |

Total: ~3h. Build + testes: ~10 min.