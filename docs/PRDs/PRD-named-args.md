# PRD — Args Nomeados em Actions via Dict

**Status:** ✅ Concluído
**Data:** 2026-08-08
**Implementação:** Parser aceita `g!{"k": v}` via `parse_brace_lit`, inference mapeia chaves→params e reordena para tupla posicional em `action_call.rs`. Funciona em `fork!` e `@test{args: ...}`.

## Visão

Permitir que chamadas de action usem Dict literal como alternativa à tupla
posicional. As chaves do Dict correspondem aos nomes dos params da action;
o typeck mapeia por nome e reordena para posicional em compile-time. A ABI
não muda — a action continua recebendo tupla posicional internamente.

```kata
# Definition — não muda
action g(a::Int, b::Int) => Int
    + a b

# Call site — duas opções válidas
g!(1, 2)              # tupla posicional (atual)
g!{"b": 2 "a": 1}     # Dict nomeado (novo — ordem não importa)
```

## Depende de

- **Fio 13** ✅ (Dict/Set HAMT, DictLit no parser, inferência de DictLit)
- **Fio 2** ✅ (actions, tupla posicional como args)

## Motivação

Actions com muitos params tornam a chamada posicional opaca — `g!(1, 0, 42, 3, 2)`
não revela o que cada valor significa. Args nomeados via Dict melhoram legibilidade
sem mudar o modelo de execução. É opcional: o desenvolvedor escolhe no call site.

## Sintaxe

### Call site

| Sintaxe | Semântica |
|---|---|
| `g!(1, 2)` | Tupla posicional — mapeia params por ordem de declaração |
| `g!{"b": 2 "a": 1}` | Dict nomeado — mapeia params por chave |
| `g!()` | Tupla vazia — action sem params |
| `g!{}` | Erro — Array vazio, não Dict. Usar `g!{:}` para Dict vazio (action sem params) |

O parser já produz `Expr::DictLit` para `{"k": v}`. A novidade é aceitar
DictLit como `args` em `ActionCall`, onde hoje só `Expr::Tuple`, `Expr::Grouping`,
e `Expr::Unit` são aceitos.

### `@test{args: ...}`

| Sintaxe | Semântica |
|---|---|
| `@test{desc: "...", args: (3, 4)}` | Tupla posicional (atual) |
| `@test{desc: "...", args: {"b": 4 "a": 3}}` | Dict nomeado (novo) |

`args` continua sendo a chave de metadata. Seu tipo (tupla vs Dict) distingue
posicional de nomeado. `desc`, `timeout`, `expects` coexistem no mesmo `{}`.

### `fork!`

| Sintaxe | Semântica |
|---|---|
| `fork!(worker, (1, 2))` | Tupla posicional (atual) |
| `fork!(worker, {"b": 2 "a": 1})` | Dict nomeado (novo) |

O segundo elemento da tupla de `fork!` pode ser Dict em vez de tupla.

## Design

### D1: Mapeamento chave→param

O typeck extrai os nomes dos params da declaração da action. Quando `args`
é `TypedExprKind::DictLit`, o typeck:

1. Verifica que cada chave do Dict é um `TextLit` cujo valor corresponde a
   um nome de param da action.
2. Verifica que todo param tem correspondência (nenhum faltante).
3. Verifica que nenhum extra (chave sem param).
4. Verifica que nenhum duplicado (Dict já rejeita por semântica).
5. Reordena os valores na ordem posicional dos params.
6. Produz `TypedExprKind::Tuple` com os valores reordenados — o codegen
   não muda.

**Resultado:** o TAST produced é idêntico ao da chamada posicional. O Dict
literal é "desugarado" para tupla posicional em compile-time. Zero custo
em runtime.

### D2: Tipos de chave

Chaves do Dict literal são sempre `TextLit` no parser (sintaxe `{"k": v}`).
Os nomes dos params são `String`. A comparação é string == string.

Se uma chave não é `TextLit` (ex: `{"a" + "b": 1}` — expressão como chave),
erro: "args nomeados exigem chaves literais de Text".

### D3: Type inference

O typeck precisa inferir o DictLit antes de mapear. Hoje `infer_dict_lit`
produz `Ty::Dict(K, V)` e verifica `HASHABLE` em K. Para args nomeados,
K é sempre `Text` — não precisa de `HASHABLE` check porque o typeck não
vai produzir um `Ty::Dict` no TAST, vai produzir `Ty::Tuple`.

**Fluxo:**
1. Parser produz `Expr::ActionCall { args: Expr::DictLit }`
2. Typeck infere `args` como `TypedExprKind::DictLit` (caminho atual)
3. Typeck detecta que é `ActionCall` com `DictLit` args
4. Typeck extrai nomes dos params da action
5. Typeck mapeia chaves→params, reordena, produz `TypedExprKind::Tuple`
6. Typeck prossegue com dispatch posicional normal

### D4: Params sem nome

Se a action usa syntax posicional sem nomes (ex: `action f(Int, Int) => Int`),
não há nomes para mapear. Erro: "action `f` não tem params nomeados — use
chamada posicional `f!(1, 2)`".

Na prática, actions já declaram params com nome (`action f(a::Int, b::Int)`),
mas a sintaxe sem nome é aceita pelo parser. Verificar se a action tem
nomes antes de aceitar Dict args.

### D5: Overloads

Se a action tem múltiplas overloads com params de nomes diferentes, o typeck
tenta mapear contra cada overload. Se apenas uma matcha (todas as chaves
correspondem a params e tipos casam), despacha. Se múltiplas matcham,
erro de ambiguidade. Se nenhuma matcha, `NoOverload`.

## Arquitetura

### Parser — `kata-parser/src/expressions.rs`

Hoje: `Ident ! ( args )` — `!` deve ser seguido de `(`. `args` é
`parse_paren_expr()` que produz `Tuple`, `Grouping`, ou `Unit`.

Mudança: `!` pode ser seguido de `(` (tupla) OU `{` (Dict). Se `{`:
- Parsear como DictLit (já existe `parse_brace_lit`)
- Wrapping em `Expr::ActionCall { args: DictLit }`

```
Ident ! ( ... )  →  ActionCall { args: Tuple/Grouping/Unit }    # atual
Ident ! { ... }  →  ActionCall { args: DictLit }                 # novo
```

### Typeck — `kata-inference/src/infer/action_call.rs`

Hoje: `infer_action_call` infere `args` como expr, normaliza Grouping→Tuple,
extrai tipos, despacha no DispatchTable.

Mudança: após inferir `args`, se for `TypedExprKind::DictLit`:
1. Extrair nomes dos params da action (do DispatchTable/overload)
2. Validar chaves contra nomes de params
3. Reordenar valores para posicional
4. Substituir `DictLit` por `Tuple` reordenada
5. Prosseguir com dispatch normal

### `fork!` — `kata-inference/src/infer/action_call.rs`

Mesma lógica: o segundo elemento da tupla de `fork!` pode ser DictLit.
Após inferir, se for DictLit, mapear e reordenar para Tuple.

### `@test{args: ...}` — `kata-codegen/src/lowering/test_runner.rs`

O `spec.args` é `Option<TypedExpr>`. Se for `TypedExprKind::DictLit`,
mesma lógica de mapeamento e reordenação. O wrapper recebe tupla
posicional reordenada.

### Codegen — sem mudança

O codegen recebe `TypedExprKind::Tuple` como args — não sabe se veio de
tupla posicional ou Dict reordenado. Zero mudança em codegen.

## Fases

### Fase 1: Parser — aceitar `g!{...}`

- Modificar `expressions.rs`: depois de `!`, aceitar `{` além de `(`
- Se `{`, parsear como DictLit via `parse_brace_lit`
- Produzir `Expr::ActionCall { args: DictLit }`
- Testes de parser: `g!{"b": 2 "a": 1}` parseia como ActionCall com DictLit

**DoD:** `cargo test -p kata-parser` passa. Novo teste confirma parse.

### Fase 2: Typeck — mapeamento chave→param

- Em `infer_action_call`, detectar `TypedExprKind::DictLit` em args
- Extrair nomes de params do overload (precisa de acesso aos param_names)
- Mapear, validar, reordenar → produzir `TypedExprKind::Tuple`
- Tratar erros: chave não corresponde, param faltante, action sem nomes
- Mesma lógica em `infer_fork_builtin`

**DoD:** `cargo test -p kata-inference` passa. Testes de inferência
confirmam mapeamento.

### Fase 3: `@test{args: ...}` com Dict

- Em `test_runner.rs` (ou onde `spec.args` é lowered), detectar DictLit
- Mesma lógica de mapeamento e reordenação

**DoD:** `cargo test -p kata-codegen --test test_wrapper_codegen` passa.
Novo teste: `@test{desc: "...", args: {"b": 4 "a": 3}}` gera wrapper.

### Fase 4: E2E

- Teste E2E: `g!{"b": 2 "a": 1}` executa e retorna mesmo resultado que `g!(1, 2)`
- Teste E2E: `fork!(worker, {"x": 42})` executa
- Teste E2E: `@test{args: {"b": 4 "a": 3}}` compila e gera wrapper

**DoD:** `cargo test --workspace --no-fail-fast` passa. Sem regressões.

### Fase 5: Documentação

- `sintaxe-mapa.md`: documentar `g!{"k": v}` como alternativa a `g!(args)`
- `Kata-lang-manual.md`: atualizar seção de actions com args nomeados
- Solicitar permissão antes de alterar o manual

**DoD:** Docs atualizadas e consistentes com a implementação.

## Não no escopo

- **Args nomeados em funções puras (lambdas):** funções puras usam aplicação
  prefix `f a b`, não `f!`. O mecanismo de dispatch é diferente. Se futuro
  quiser args nomeados em funções, é outro PRD.
- **Valores default:** `action g(a::Int, b::Int = 0)` — não implementar agora.
- **Params opcionais:** `action g(a::Int, b::Optional::Int)` — não implementar.
- **Ordem mista:** `g!(1, "b": 2)` — não permitir. Ou tudo posicional ou tudo nomeado.

## Decisões

| # | Decisão | Status |
|---|---|---|
| D1 | Mapeamento chave→param em compile-time, reordena para tupla posicional | ✅ Confirmado |
| D2 | Chaves devem ser TextLit — expressões como chave são rejeitadas | ✅ Confirmado |
| D3 | DictLit é desugarado para Tuple no typeck — zero custo runtime | ✅ Confirmado |
| D4 | Action sem nomes de params rejeita Dict args | ✅ Confirmado |
| D5 | Overloads: match por nomes+tipos, ambiguidade = erro | ✅ Confirmado |
| D6 | `@test{args: {...}}` — Dict direto como valor de `args` (opção A) | ✅ Confirmado por Arthur |
| D7 | `fork!` segue mesmo padrão | ✅ Confirmado por Arthur |
| D8 | Tudo posicional OU tudo nomeado — sem mistura | ✅ Confirmado |