# PRD — Args Nomeados Obrigatórios para 2+ Params

**Status:** 🔴 Pendente
**Data:** 2026-09-18

## Objetivo

Tornar a chamada nomeada (`f!{a: x, b: y}`) a forma canônica para actions
com 2 ou mais argumentos, eliminando a chamada posicional `f!(x, y)` nessa
aridade. Açúcar sintático para aridades 0 e 1 (`f!()` e `f!(x)`) permanece.
O objetivo é eliminar a classe de bugs de normalização `Grouping → Tuple`
que causou um SIGSEGV por divergência silenciosa entre 6 sites de
normalização duplicados.

## Motivação

O parser produz `Expr::Grouping` para `f!(x)` (1 arg sem vírgula) e
`Expr::Tuple` para `f!(x, y)` (2+ args). O codegen de action calls exige
`TypedExprKind::Tuple` — `Grouping` lowera como a expressão interna (ex:
SMI), e o codegen passa o valor bruto como `args_ptr`, causando SIGSEGV.

Para resolver isso, 6 sites duplicam a normalização `Grouping → Tuple`:

1. `action_call.rs:168` — action call direta
2. `action_call.rs:267` — action call indireta (job/call_indirect)
3. `action_call.rs:413` — terceira cópia no mesmo arquivo
4. `action_infer.rs:141` — wrapper de `@test`
5. `csp_concurrency.rs:134` — `spawn!`
6. `log_builtins.rs:144` — builtins de log

O SIGSEGV histórico veio de um site que esqueceu a normalização. A
duplicação é a causa raiz — cada novo site de action call precisa lembrar
de normalizar, e o esquecimento é silencioso até crashar em runtime.

A chamada posicional para 2+ args também é menos legível: `log!(level,
msg, topic, policy)` não revela o que cada valor significa.

## Depende de

- **PRD-named-args** ✅ — forma nomeada `f!{k: v}` já implementada,
  `reorder_dict_args_to_tuple` funcional no `helpers.rs`.

## Design

### Sintaxe

| Sintaxe | Aridade | Semântica |
|---|---|---|
| `f!()` | 0 args | Açúcar — action sem params |
| `f!(x)` | 1 arg | Açúcar — resolve para o primeiro param da action |
| `f!{a: x, b: y}` | 2+ args | Forma canônica — chaves correspondem aos nomes dos params |
| `f!(x, y)` | 2+ args | **Erro de sintaxe** |

A forma `f!{}` (Dict vazio) continua válida para actions sem params.
A forma `f!{:}` também — é o Dict vazio explícito.

### Desugar na inference

O parser não conhece os nomes dos params — essa informação está no
`DispatchTable` (inference). O desugar acontece no início de
`infer_action_call`:

| Input do parser | Desugar para | Reorder produz |
|---|---|---|
| `Unit` (0 args) | `DictLit {}` | `Tuple []` |
| `Grouping(x)` (1 arg) | `DictLit {first_param: x}` | `Tuple [x]` |
| `DictLit {k: v}` (nomeado) | — (já é DictLit) | `Tuple [v_pos_k]` |

O desugar de `Grouping(x)` precisa do primeiro `param_name` do overload.
Se o overload tem `param_names: [Some("msg")]`, `echo!(x)` vira
`DictLit {"msg": x}`. Se `param_names` é vazio (action sem params
nomeados), é erro — a action não pode ser chamada com 1 arg.

### Seleção de overload

O `reorder_dict_args_to_tuple` já resolve overloads múltiplos
(`helpers.rs:198-211`): busca o overload cujos `param_names` contêm todas
as chaves do Dict. Com o desugar, `echo!("hi")` vira `DictLit {"msg":
"hi"}`, e o selection escolhe o overload de 1 param `echo (msg::SHOW)` em
vez do de 2 params `echo {msg::SHOW: _, end::Text: "\n"}`.

Para o açúcar de 1 arg, o desugar precisa escolher qual overload usar
antes de saber o nome do param. Estratégia: tentar todos overloads com
exatamente 1 param nomeado. Se apenas um existe, usar seu param name. Se
múltiplos existem com 1 param, erro de ambiguidade (o usuário deve usar
`f!{param: x}`).

### Codegen

**Sem mudança.** O codegen recebe `TypedExprKind::Tuple` produzido pelo
reorder e lê por offset. A posição pós-reorder é determinística —
corresponde à ordem da declaração dos params da action.

### Casos especiais

#### `spawn!` e `fork!`

`spawn!` aceita tupla `(action, args)` ou dict `{callee: action, raw:
args}`. O segundo elemento (args) passa pelo mesmo desugar: se é
`Grouping` (1 arg), desugara para `DictLit`; se é `Tuple` com 2+,
**erro** — o usuário deve passar `DictLit` nomeado.

O `csp_concurrency.rs` já tem `normalize_grouping` em `:410` — essa
função é removida. O desugar centralizado em `infer_action_call` (ou
helper compartilhado) trata todos os casos.

#### `@test{args: ...}`

`action_infer.rs` processa args de `@test`. Hoje normaliza
`Grouping → Tuple` em `:141`. Com o desugar centralizado, essa
normalização é removida — args passam pelo mesmo path.

#### `log_builtins.rs`

`extract_tuple_elements` (`:150`) trabalha na AST não-tipada (antes da
inferência). Recebe `Grouping`, `Tuple`, ou `Unit` e extrai elementos.
Com a mudança, o parser rejeita `Tuple` com 2+ para action calls, mas
builtins de log são interceptados antes do `infer_action_call` geral.
`extract_tuple_elements` muda para aceitar apenas `Grouping` (1 arg) e
`Unit` (0 args), ou é removida se os builtins passarem a usar o path
comum.

#### `format!`

`format!` é interceptado em `action_call.rs:99` antes do dispatch geral.
Hoje aceita `format!("template {} {}", (a, b))` (tupla posicional de
args de template) e `format!{"_msg": _msg, "_name": _name}` (dict
nomeado). A tupla posicional de `format!` é um caso especial: os
elementos são os valores a substituir nos placeholders `{}`, não params
nomeados da action. Este caso precisa de tratamento separado — `format!`
não se encaixa no modelo de params nomeados porque os "params" são
posicionais por natureza (ordem dos `{}` no template).

**Decisão:** `format!` mantém sua forma posicional interna
(`format!("tmpl", (a, b))`) como exceção — o parser reconhece `format!`
especificamente e permite `Tuple` após `!`. Alternativa: `format!` passa
a aceitar apenas dict onde as chaves são os placeholders textuais
(`format!{"{}": a, "{}": b}` ou `format!{"0": a, "1": b}`), mas isso é
mais verboso e menos natural. A exceção é justificada: `format!` não é
uma action genérica, é um builtin de interpolação.

## Decisões de design

### DD1: Açúcar para 1 arg permanece

**Escolha:** `f!(x)` continua válido.

**Alternativa rejeitada:** exigir `f!{param: x}` para toda aridade > 0.
Rejeitada porque 254 calls de 1 arg migrariam para uma forma
significativamente mais verbosa (`echo!{msg: "hello"}` vs
`echo!("hello")`) sem ganho de segurança — com 1 arg, não há ambiguidade
posicional.

### DD2: Desugar na inference, não no parser

**Escolha:** o parser emite `Grouping`/`Unit`/`DictLit` e a inference
desugara para `DictLit` usando `param_names` do `DispatchTable`.

**Alternativa rejeitada:** desugar no parser emitindo `DictLit` com
placeholder `_` para o primeiro param. Rejeitada porque o parser não tem
acesso ao `DispatchTable` — introduziria uma semântica ad hoc no
`reorder_dict_args_to_tuple` para tratar `_` como "primeiro param vazio",
misturando responsabilidades.

### DD3: `format!` é exceção

**Escolha:** `format!` mantém tupla posicional interna.

**Alternativa rejeitada:** forçar `format!` a usar dict com chaves
numéricas. Rejeitada porque os placeholders de template são posicionais
por natureza — numerá-los no dict é verbosidade sem clareza.

### DD4: Parser rejeita `!(x, y)` com vírgula

**Escolha:** o parser produce um erro de sintaxe quando vê `!` seguido
de `(` com 2+ elementos.

**Alternativa rejeitada:** aceitar `!(x, y)` no parser e rejeitar na
inference. Rejeitada porque o erro de sintaxe é mais cedo e mais claro —
o usuário vê o problema no parser sem precisar do contexto de tipos.

## Fases

### Fase 1 — Parser rejeita `!(x, y)` com 2+ args

**Escopo:** modificar `expressions.rs:287-294` para, após consumir `!`,
verificar se `parse_paren_expr` produziria `Tuple` com 2+ elementos. Se
sim, erro: "chamada de action com 2+ args exige forma nomeada:
f!{param1: x, param2: y}".

Na prática, o parser não pode chamar `parse_paren_expr` e depois
verificar — precisa de uma produção dedicada `parse_action_args` que
aceita apenas 0 ou 1 elementos entre parênteses, emitindo `Unit` ou
`Grouping` diretamente, e rejeitando vírgula.

**Exceção:** `format!` é reconhecido pelo nome antes de chamar
`parse_action_args` e passa a usar `parse_paren_expr` normalmente.

**DoD:**
- `echo!("hi")` parseia (Grouping)
- `echo!()` parseia (Unit)
- `echo!{msg: "hi"}` parseia (DictLit)
- `echo!("hi", "bye")` é erro de sintaxe
- `format!("{} {}", (a, b))` parseia normalmente

### Fase 2 — Desugar centralizado na inference

**Escopo:** no início de `infer_action_call` (antes do dispatch),
desugarar `Unit` → `DictLit {}` e `Grouping(x)` → `DictLit
{first_param: x}`. Todo args passa por `reorder_dict_args_to_tuple`.

Extrair o desugar para um helper compartilhado (ex:
`desugar_action_args`) que todos os sites chamam:
- `action_call.rs` (3 sites)
- `action_infer.rs` (1 site)
- `csp_concurrency.rs` (2 sites — `spawn!` posicional e dict)
- `log_builtins.rs` (1 site)

Remover as 6 cópias de normalização `Grouping → Tuple`.

**DoD:**
- `echo!("hi")` desugara para `DictLit {"msg": "hi"}` → reorder →
  `Tuple [TextLit "hi"]`
- `echo!{msg: "hi", end: "!"}` passa pelo reorder → `Tuple [TextLit
  "hi", TextLit "!"]`
- `spawn!(worker, (42))` desugara o segundo elemento para DictLit
- Nenhuma cópia de normalização Grouping → Tuple permanece

### Fase 3 — Migração das 54 calls posicionais

**Escopo:** migrar todas as 54 calls `f!(x, y, ...)` com 2+ args nos
arquivos `.kata` para `f!{a: x, b: y}`. Automatizável com script que lê
a assinatura da action e gera `!{param: arg}`.

**DoD:**
- `cargo test` passa
- `grep -rn '!(' --include='*.kata'` não retorna nenhum resultado com
  vírgula dentro dos parênteses (exceto `format!`)

## Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| Parser | `expressions.rs:287-294` | Produção dedicada `parse_action_args`; rejeitar `!(x, y)` |
| Inference | `action_call.rs:168,267,413` | Remover 3 normalizações; desugar centralizado |
| Inference | `action_infer.rs:141-165` | Remover normalização; usar desugar compartilhado |
| Inference | `csp_concurrency.rs:134,263,321,410` | Remover `normalize_grouping`; usar desugar |
| Inference | `log_builtins.rs:144-161` | Adaptar `extract_tuple_elements` |
| Inference | `helpers.rs:170-301` | Sem mudança (reorder já funciona) |
| Codegen | `lowering/action_call.rs` | Sem mudança |
| Stdlib/examples | 54 calls em `.kata` | Migrar para `!{param: arg}` |

## Fora do escopo

- **Sintaxe de definição de action** — `action f(a::Int, b::Int)` e
  `action f{a::Int: _, b::Int: 5}` já funcionam. Sem mudança.
- **Funções puras** — `f x y` (aplicação curried) não é afetada. A
  mudança é exclusiva de action calls (`!`).
- **`format!` posicional** — mantém tupla interna como exceção (DD3).
- **Codegen/ABI** — sem mudança. O reorder produz `Tuple` na ordem
  canonical dos params; o codegen lê por offset.
- **Migração de calls de 0 e 1 arg** — não migram. `f!()` e `f!(x)`
  permanecem como açúcar.

## Riscos

### R1: `format!` como exceção cria bifurcação no parser

O parser precisa reconhecer `format!` pelo nome antes de decidir qual
produção usar. Isso acopla o parser a um nome específico de builtin.
Mitigação: a lista de builtins com exceção é fixa e pequena (`format!`
é o único caso onde args posicionais são semanticamente necessários).

### R2: Ambiguidade no desugar de 1 arg com overloads múltiplos

Se uma action tem dois overloads de 1 param com nomes diferentes (ex:
`action f(a::Int) => Int` e `action f(b::Text) => Text`), o desugar de
`f!(x)` não sabe qual param name usar. Mitigação: se múltiplos overloads
de 1 param existem, o desugar tenta todos — se apenas um casar por tipo,
despacha; se múltiplos casarem, erro de ambiguidade (o usuário deve usar
`f!{a: x}` ou `f!{b: x}`). O `reorder_dict_args_to_tuple` já tem
selection de overload por chaves; o desugar pode gerar `DictLit` com
todas as chaves candidatas e deixar o reorder selecionar.

### R3: Migração automatizada pode gerar chaves erradas

O script de migração precisa ler as assinaturas das actions para saber
os nomes dos params. Se uma action é importada e a assinatura não está
disponível no momento do script, a migração manual é necessária.
Mitigação: as 54 calls estão concentradas na stdlib e exemplos — todas
as assinaturas estão no repositório.

## Documentação

- `docs/base/kata-book/` — atualizar seções sobre action calls
- `docs/base/sintaxe-mapa.md` — atualizar tabela de sintaxe de actions
- `docs/base/Kata-lang-manual.md` — atualizar seção de action calls
- `docs/TODO.md` — remover item "Sintaxe de action call: posicional vs
  nomeado" e "Centralizar normalização Grouping→Tuple"