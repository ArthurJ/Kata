# PRD — Dict Dispatch Fix: funções posicionais + default args em actions

**Status:** Concluído
**Data:** 2026-08-13
**Substitui:** Seções 3.2-3.4 e Fases 1-2, 5 do `PRD-arity-uniformization.md` (dict dispatch para funções puras)
**Depende de:** Pipeline lex→parse→resolve→infer existente, DispatchTable, parser arity-aware

## 1. Objetivo

Duas mudanças complementares:

1. **Tornar funções puras exclusivamente posicionais.** Eliminar a ambiguidade
   entre "dict como argumentos nomeados" e "dict como valor" em funções puras.
   Dict dispatch (args nomeados) passa a ser exclusivo de actions.

2. **Default args em actions via dict-template.** Actions com defaults declaram
   params como um dict-template onde `_` marca obrigatórios e literais marcam
   defaults. O prólogo faz merge do dict da chamada sobre o template.

### Problema

`f{"a": 1 "b": 2}` tem duas interpretações possíveis em funções puras:

1. **Args nomeados** — as chaves são nomes de parâmetros, reordenados para tupla posicional
2. **Dict como valor** — as chaves são chaves de um dicionário, passadas como argumento

A inferência decide qual via `has_dict_overload` (apply.rs:218-228): se a função
tem um overload que aceita `Ty::Dict`, o DictLit é valor; caso contrário, é args
nomeados. Esse guard é frágil:

- Se a função tem overload que aceita Dict **e** params nomeados, a chamada nomeada
  fica inacessível — o guard sempre escolhe "dict como valor".
- O `any()` basta um overload que aceita Dict para bloquear chamada nomeada para
  todos os overloads.
- Em actions, **não há guard** — DictLit é incondicionalmente tratado como args
  nomeados (action_call.rs:378-383), tornando impossível passar um Dict como valor
  para uma action que recebe Dict.
- A decisão é por tipo, não por sintaxe — o usuário não sabe olhando para
  `f{"k": v}` qual semântica aplica.

### Causa raiz

A sintaxe `(x::Int)` em assinaturas de função foi adicionada no commit `dbf3f7c`
(6 Ago 2026) especificamente para habilitar dict dispatch em funções puras. Antes
disso, só actions tinham `param_names` (commit `30acbf1`, 19 Jul 2026). A extensão
introduziu a bifurcação — e o guard foi o patch sobre a bifurcação.

Nenhum código de usuário real usa dict dispatch em funções puras. Todos os
exemplos e stdlib usam apenas chamada posicional em funções: `Int Int => Int`,
`[A] [B] => [(A, B)]`, etc. A sintaxe `(x::Int)` em assinaturas de função existe
apenas em testes E2E do próprio compilador.

### Solução

- **Funções puras**: exclusivamente posicionais. Açúcar posicional (`f 1 2`) ou
  tupla explícita (`f(1, 2)`). A ABI é direta: `i64, i64, ...`. Sem `param_names`,
  sem dict dispatch, sem `has_dict_overload`.
- **Actions**: mantém tupla posicional (`f!(1, 2)`) e dict nomeado (`f!{"x": 1}`).
  O prólogo da action desempacota o dict para os bindings. Default args, quando
  implementados, surgem naturalmente aqui — se uma key não está no dict, usa o
  default. O caller não precisa saber quais params têm default.
- **Dict como valor**: `f({"k": v})` (função) ou `f!({"k": v})` (action) — o dict
  está dentro de uma tupla de 1 elemento. Sem ambiguidade.

## 2. Modelo de argumentos após esta mudança

| Aspecto | Função pura | Action |
|---|---|---|
| Sintaxe posicional (açúcar) | `f 1 2` | `f!(1, 2)` |
| Sintaxe nomeada (dict) | ❌ não existe | `f!{"x": 1 "y": 2}` |
| Tupla como valor posicional | `f (1, 2)` | `f!((1, 2))` |
| Dict como valor posicional | `f ({"k": v})` | `f!({"k": v})` |
| `param_names` na assinatura | ❌ removido | `(x::Int, y::Int)` |
| Default args | ❌ não suportado | futuro — via prólogo da action |
| Marcador de side-effect | sem `!` | com `!` |

A diferença entre função e action permanece sendo o `!` — que marca impureza.
A passagem de argumentos é diferente por design: funções são posicionais (simples,
diretas, ABI enxuta), actions suportam nomeação (flexíveis, têm prólogo).

## 3. Mudanças por crate

### 3.1. `kata-parser` — `sig.rs`

**Remover `try_parse_named_type_param` do parser de assinaturas de função.**

Hoje `parse_sig` (sig.rs:110-118) chama `try_parse_named_type_param` em cada
posição de param. Se casar (`Ident snake_case :: Type` entre parênteses), registra
`param_names.push(Some(pname))`. Após esta mudança, `parse_sig` só parseia tipos
posicionais:

```rust
// ANTES (sig.rs:110-118):
while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
    if let Some((pname, ty)) = self.try_parse_named_type_param() {
        params.push(ty);
        param_names.push(Some(pname));
    } else {
        params.push(self.parse_type_expr()?);
        param_names.push(None);
    }
}

// DEPOIS:
while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
    params.push(self.parse_type_expr()?);
}
// param_names removido do Item::Sig ou sempre vec![None; params.len()]
```

**Atenção:** `try_parse_named_type_param` também é usado em `interface_decl.rs:344`
e `interface_decl.rs:477` para métodos de `implements`. Interfaces são assinaturas de
funções dentro de implementações de tipo. Se a decisão é que funções são
exclusivamente posicionais, métodos de interface também são — não há motivo para
`implements` suportar params nomeados se as funções que os implementam não podem
receber dict dispatch.

**Decisão necessária:** remover `try_parse_named_type_param` de `interface_decl.rs`
também? Ou manter para documentação? Ver seção 6 — Decisões de design.

### 3.2. `kata-parser` — `expr_apply.rs`

**Remover o caminho de dict dispatch para funções (linha 216-232).**

Hoje, em `parse_apply_impl`, quando `as_arg == false` e o callee é `Ident(name)`
com aridade conhecida, se o próximo token é `Token::LBrace`, o parser consome
um `DictLit` como único arg:

```rust
// ANTES (expr_apply.rs:222-232):
if !as_arg && matches!(parser.peek(), Token::LBrace) {
    let dict = parser.parse_brace_lit()?;
    let span = callee.span.cover(dict.span);
    return Ok(Spanned::new(
        Expr::Apply {
            callee: Box::new(callee),
            args: vec![dict],
        },
        span,
    ));
}
```

Após a mudança, `Ident {` sem `!` deixa de ser dict dispatch. O `{` após `Ident`
em posição de função é tratado como... o quê? Duas opções:

- **Opção A:** `{` após `Ident` sem `!` é erro de sintaxe — "funções não aceitam
  chamada nomeada, use `f(args)` posicional ou `action!{...}` para chamada nomeada".
- **Opção B:** `{` após `Ident` sem `!` é o início de um `DictLit` como valor
  posicional — `f {"k": v}` passa o dict como 1º argumento posicional.

A Opção B é mais alinhada com o modelo "tupla e dict são valores": `f {"k": v}`
passa um dict como argumento, assim como `f (1, 2)` passa uma tupla. Mas exige
que o parser saiba que `{` após `Ident` sem `!` inicia um átomo (DictLit) e não
uma construção de bloco. Como `{` já é tratado por `parse_brace_lit` que
disambigua Array/Dict/Set, isso é factível.

**Recomendado: Opção B.** `f {"k": v}` = `Apply(f, [DictLit])` — dict como valor
posicional. Consistente com `f (1, 2)` = tupla como valor posicional. O typeck
faz dispatch normal: se a função aceita `Dict`, ok; senão, erro de tipo.

### 3.3. `kata-ast` — `item.rs`

**Remover `param_names` de `Item::Sig`, ou manter como `vec![None; N]`.**

`Item::Sig` tem campo `param_names: Vec<Option<String>>`. Se funções são
exclusivamente posicionais, esse campo é sempre `vec![None; params.len()]`.

Opções:
- Remover o campo e atualizar todos os construtores (produz `missing field` em
  ~40 arquivos de teste, como já aconteceu com `directive_registry`).
- Manter o campo populado com `vec![None; params.len()]` — menos invasivo.

**Recomendado:** remover o campo. É mais limpo e força a consistência. O custo de
atualizar testes é mecânico (já feito antes com `directive_registry`).

### 3.4. `kata-resolution` — `types.rs` e `lib.rs`

**Remover `param_names` de `Signature`.**

`Signature` (types.rs:45-70) tem `param_names: Vec<Option<String>>`. Se removido:

- `lib.rs:189` — `param_names` não é mais extraído do `Item::Sig`.
- `lib.rs:314` — `Signature` não tem `param_names`.
- `pass0.rs:457,565` — propagação de `param_names` de métodos de interface para
  `Signature` também é removida.

`ActionDef` (types.rs:170) mantém `param_names` — actions continuam com params
nomeados.

### 3.5. `kata-inference` — `infer/helpers.rs`

**Remover `reorder_dict_args_to_tuple` do caminho de funções.**

`reorder_dict_args_to_tuple` (helpers.rs:145-244) é usado em dois lugares:
- `apply.rs:269` — dict dispatch para funções puras → **remover**
- `action_call.rs:380` — dict dispatch para actions → **manter**

A função em si pode permanecer em `helpers.rs`, mas só é chamada por
`action_call.rs`.

### 3.6. `kata-inference` — `infer/apply.rs`

**Remover o bloco de dict dispatch para funções (linhas 203-380).**

O bloco inteiro que detecta `DictLit` como único arg e faz `reorder_dict_args_to_tuple`
é removido. Após a remoção, `infer_apply` faz dispatch puramente posicional:

1. Expande spread (`$`)
2. Infere tipos dos args
3. Tenta dispatch por interface (Caminho 0)
4. Tenta dispatch por DispatchTable (Caminho 1)
5. Tenta dispatch por TypeEnv (Caminho 2 — call_indirect)

Se o único arg é um `DictLit`, ele é inferido como `Ty::Dict(_, _)` e despachado
normalmente — a função precisa ter um overload que aceita `Dict`.

Também remover:
- `has_dict_overload` (linhas 218-227)
- `has_named_in_table` (linhas 230-236)
- `has_named_in_env` (linha 239)
- O caminho B inteiro (linhas 290-378 — TypeEnv com `param_names` para let-bound
  lambdas)

### 3.7. `kata-inference` — `infer/action_call.rs`

**Confirmar que a disambiguação é sintática (`!{` vs `!(`), sem guard de tipo.**

Hoje action_call.rs:378-383 incondicionalmente trata `DictLit` como args nomeados.
Isso impede passar um Dict como valor para uma action que recebe Dict.

A solução **não** é adicionar um guard de tipo (`has_dict_overload`) — isso
recriaria o mesmo defeito que estamos eliminando de `apply.rs`: disambiguação
por tipo em vez de por sintaxe. A solução é confirmar que o parser já distingue
as duas formas no momento do parse:

- `config!({"k": v})` — `!(` abre tupla posicional; `DictLit` dentro dela é
  **valor posicional**. O typeck infere `Ty::Dict` e despacha normalmente.
- `config!{"k": v}` — `!{` abre dict nomeado; o typeck reordena chaves para
  params via `reorder_dict_args_to_tuple`.

A disambiguação é puramente sintática, simétrica a `f ({"k": v})` vs `f {"k": v}`
no lado das funções (§3.2). Sem guard, sem `has_dict_overload`, sem disambiguação
por tipo. O bug latente (action que recebe Dict não pode ser chamada com Dict
como valor) é resolvido pela sintaxe `!({...})`, não por introspecção de tipo.

**Ação concreta:** verificar que `action_call.rs` trata `DictLit` como args
nomeados **apenas** quando o parser produziu `!{` (dict dispatch), não quando
o `DictLit` vem como elemento de tupla posicional dentro de `!(`. Se o parser
já separa os dois caminhos, `action_call.rs` não precisa de mudança — o
`DictLit` dentro de tupla posicional chega como argumento normal, não como
dict dispatch.

### 3.8. `kata-inference` — TypeEnv `lookup_param_names`

**Remover ou tornar dead code.**

`env.lookup_param_names` (usado em apply.rs:239,295) só serve para o caminho B
(dict dispatch em let-bound lambdas com params nomeados). Se o caminho B é
removido, `lookup_param_names` fica sem chamadores.

Se `param_names` é removido de `Signature`, o TypeEnv também não precisa carregar
essa informação. Verificar se `TypeBinding` tem campo `param_names` e remover.

## 4. Testes

### 4.1. Testes que viram testes de action

Os testes E2E de dict dispatch em funções puras são convertidos para actions:

| Teste atual (função pura) | Teste novo (action) |
|---|---|
| `dict_dispatch_simples` | `action soma (a::Int, b::Int) => Int` + `soma!{"a": 3 "b": 4}` |
| `dict_dispatch_ordem_invertida` | mesma action, `soma!{"b": 4 "a": 3}` |
| `dict_dispatch_tres_params` | `action sub (x::Int, y::Int, z::Int) => Int` + `sub!{"z": 1 "x": 10 "y": 3}` |
| `kata_run_dict_dispatch_sem_bang` | vira teste de action com `!` |
| `kata_run_dict_dispatch_ordem_invertida` | idem |
| `kata_run_dict_dispatch_whitespace_nao_distingue` | idem |
| `kata_run_dict_dispatch_tres_params_embaralhados` | idem |

### 4.2. Teste que vira teste negativo

`dict_dispatch_funcao_pura_sem_bang` (dict_dispatch_e2e.rs:125) vira teste
negativo: `dobro :: (x::Int) => Int` + `dobro{"x": 21}` deve dar erro de
compilação — "funções puras não aceitam chamada nomeada, use action".

`let_lambda_dict_dispatch_via_typeenv` (two_passes_e2e.rs:337) também vira teste
negativo pelo mesmo motivo.

`kata_eval_dict_dispatch_um_param` (two_passes_e2e.rs:155) — idem.

### 4.3. Novo teste: dict como valor posicional em função

```kata
# Função que aceita Dict como tipo
mostra :: Dict::Text Int => Unit
lambda d: show d

mostra ({"chave": 42})   # dict como valor posicional — ok
```

### 4.4. Novo teste: dict como valor vs args nomeados em action

```kata
# Action que aceita Dict como tipo
action config (opts::Dict::Text Int) => Unit
    show opts

config!({"timeout": 30})    # dict como valor posicional — ok (DictLit dentro de tupla)
config!{"timeout": 30}     # args nomeados — ERRO: config não tem param "timeout"
```

A disambiguação é sintática: `!(` = posicional, `!{` = nomeado. Sem guard de tipo.

## 5. Fases de implementação

### Fase 1: Remover params nomeados de assinaturas de função (parser)

- Remover `try_parse_named_type_param` de `parse_sig` (sig.rs)
- Remover `param_names` de `Item::Sig` (item.rs) ou tornar sempre `vec![None; N]`
- Remover uso de `try_parse_named_type_param` em `interface_decl.rs` (se decidido)
- Atualizar construtores de `Item::Sig` em testes (~40 arquivos)
- `cargo check --workspace`

**DoD:** `cargo check --workspace` compila. `parse_sig` não aceita `(x::Int)`.

### Fase 2: Remover dict dispatch de funções (inference)

- Remover bloco de dict dispatch em `apply.rs` (linhas 203-380)
- Remover `has_dict_overload`, `has_named_in_table`, `has_named_in_env`
- Remover caminho B (TypeEnv com `param_names`)
- Remover `param_names` de `Signature` (types.rs) e propagação em lib.rs/pass0.rs
- Remover `lookup_param_names` do TypeEnv (se aplicável)
- `reorder_dict_args_to_tuple` só é chamado por `action_call.rs`
- `cargo check --workspace`

**DoD:** `cargo check --workspace` compila. `infer_apply` não tem caminho de dict
dispatch.

### Fase 3: Dict como valor posicional (parser + inference)

- Em `expr_apply.rs`, mudar o caminho de `Ident {` sem `!`: em vez de produzir
  `Apply(callee, [DictLit])` como dict dispatch, produzir `Apply(callee, [DictLit])`
  como valor posicional — ou seja, **remover o branch especial** e deixar o
  loop greedy/arity-aware coletar o `DictLit` como argumento normal.
  - Se arity-aware: o DictLit é coletado como 1 arg (via `parse_arg`).
  - Se greedy: o DictLit é coletado pelo loop `can_start_expr`.
  - **Atenção:** `DictLit` já está na lista de não-callees (expr_apply.rs:182),
    mas precisa estar na lista de tokens que `can_start_expr` aceita. Verificar.
- `cargo test --workspace --no-fail-fast`

**DoD:** `f {"k": v}` passa o dict como argumento posicional. Se a função aceita
`Dict`, despacha normalmente. Se não, erro de tipo.

### Fase 4: Confirmar disambiguação sintática em actions

- Verificar que o parser distingue `!{` (dict nomeado) de `!(` (tupla posicional)
  — o `DictLit` dentro de `!(` deve chegar a `action_call.rs` como argumento
  posicional, não como dict dispatch
- Confirmar que `action_call.rs` só chama `reorder_dict_args_to_tuple` quando o
  parser produziu `!{` (dict nomeado), não quando o `DictLit` vem como elemento
  de tupla posicional dentro de `!(`
- **Não adicionar `has_dict_overload`** — a disambiguação é sintática, não por tipo
- `cargo test --workspace --no-fail-fast`

**DoD:** `config!({"k": v})` passa dict como valor posicional para action que
aceita Dict. `config!{"k": v}` continua como args nomeados. A disambiguação é
puramente sintática (`!(` vs `!{`), sem guard de tipo.

### Fase 5: Converter testes

- Converter testes E2E de dict dispatch em funções para testes de action
- Converter `dict_dispatch_funcao_pura_sem_bang` e `let_lambda_dict_dispatch_via_typeenv`
  em testes negativos (erro de compilação)
- Adicionar testes novos: dict como valor posicional em função e action
- `cargo test --workspace --no-fail-fast`

**DoD:** Todos os testes passam. Testes negativos verificam que `f{"k": v}` em
função pura dá erro claro.

### Fase 6: Parser de dict-template em actions

- `action_decl.rs`: reconhecer `{` após o nome da action como início de
  dict-template (em vez de `(`). Parsear entradas `nome::Tipo: valor` onde
  valor é `_` (obrigatório) ou uma expressão (default).
- Desugaração: `(x::Int, y::Int)` → `{x::Int: _, y::Int: _}` internamente.
  Ambas produzem a mesma estrutura no AST — `ParamSpec { name, type, required, default }`.
- `try_parse_named_type_param` permanece em `action_decl.rs` (actions mantêm
  params nomeados).
- `cargo check --workspace`

**DoD:** `action f{x::Int: _, y::Int: 5}` parseia. `action f(x::Int, y::Int)`
continua parseando (açúcar).

### Fase 7: Prólogo de merge com defaults

- `action_call.rs`: estender `reorder_dict_args_to_tuple` para preencher
  faltantes com defaults do template, não apenas reordenar chaves.
- Chamada posicional mapeia por índice para o template, preenche defaults
  dos faltantes.
- Chamada nomeada faz merge: `template ∪ call_dict`.
- Validação: chave obrigatória (`_`) ausente → erro de compilação.
- `cargo test --workspace --no-fail-fast`

**DoD:** `act!{"msg": "hi"}` com `action act{msg::Text: _, dft::Int: 5}`
funciona — `dft` usa default 5. `act!("hi")` posicional também. `act!{"dft": 3}`
sem `msg` dá erro.

### Fase 8: Testes de default args

- Teste E2E: action com defaults, chamada nomeada omitindo args com default
- Teste E2E: action com defaults, chamada posicional omitindo args com default
- Teste E2E: action com defaults, chamada nomeada sobrescrevendo default
- Teste E2E: action sem defaults (sintaxe `(x::Int)`) continua funcionando
- Teste negativo: action com defaults, chamada omitindo arg obrigatório (`_`)
- `cargo test --workspace --no-fail-fast`

**DoD:** Todos os testes passam.

### Fase 9: Atualizar documentação

- `docs/PRD-arity-uniformization.md` — marcar Fases 1-2 e 5 como substituídas
- `docs/sintaxe-mapa.md` — atualizar § sobre chamada de funções (remover dict
  dispatch para funções, esclarecer que `{...}` após `Ident` sem `!` é dict como
  valor posicional). Adicionar § sobre dict-template em actions.
- `docs/Kata-lang-manual.md` — **NÃO atualizar sem permissão explícita**
- `docs/ROADMAP.md` — adicionar entrada

**DoD:** Documentação reflete a nova semântica.

## 6. Decisões de design

### D1: Funções são exclusivamente posicionais

**Justificativa:** A ABI de função pura é `i64, i64, ...` — direta, sem prólogo
de desempacotamento. Dict dispatch exigiria reordenação no caller (como hoje) ou
um prólogo no callee (que funções não têm). A simplicidade da função pura é
preservada: o que você vê na assinatura é o que a ABI passa.

**Alternativa descartada:** Funções com prólogo de desempacotamento. Quebra a
separação função/action — funções deixam de ser "diretas" e ganham overhead.

### D2: `f {"k": v}` é dict como valor posicional, não erro

**Justificativa:** Consistente com `f (1, 2)` = tupla como valor. O dict é um
átomo válido (DictLit) e pode ser passado como argumento. O typeck faz dispatch
normal — se a função aceita `Dict`, ok; senão, erro de tipo. Isso é mais
flexível que um erro de sintaxe e não introduz ambiguidade (não há params
nomeados em funções, então não há duas interpretações).

**Alternativa descartada:** `f {"k": v}` é erro de sintaxe. Mais restritivo sem
benefício — o usuário precisa envolver em tupla: `f ({"k": v})`. Mas isso é
atrito desnecessário se o dict já é um átomo válido.

### D3: Métodos de interface também perdem params nomeados

**Justificativa:** Métodos de `implements` são assinaturas de funções. Se funções
não têm dict dispatch, métodos também não. Manter `try_parse_named_type_param` em
`interface_decl.rs` criaria uma inconsistência: o método tem `param_names` mas
não pode receber dict dispatch. Os nomes seriam documentação morta.

**Alternativa descartada:** Manter `param_names` em métodos como documentação.
Sem valor prático — o leitor já sabe a posição pelos tipos. E mantém código
morto no parser e resolution.

### D4: Disambiguação sintática em actions, sem guard de tipo

**Justificativa:** A ABI de action recebe `args_ptr` (ponteiro para tupla na
arena do caller). A sintaxe já distingue as duas formas de chamada: `!(` abre
tupla posicional, `!{` abre dict nomeado. Um `DictLit` dentro de `!(` é valor
posicional; um `DictLit` após `!{` é args nomeados. Adicionar `has_dict_overload`
recriaria o mesmo defeito eliminado de `apply.rs` — disambiguação por tipo em
vez de por sintaxe. Se uma action tem params nomeados **e** aceita Dict, o guard
quebraria a chamada nomeada. A regra sintática é consistente, simétrica com
funções (§3.2), e não tem corner cases.

**Alternativa descartada:** `has_dict_overload` guard (simétrico ao que existia
em `apply.rs`). Reintroduz o problema que o PRD elimina: disambiguação por tipo.
Se a action aceita Dict e tem params nomeados, `config!{"timeout": 30}` fica
ambíguo — o guard diria "tem Dict overload → é valor", quebrando a chamada
nomeada.

### D5: `param_names` removido de `Signature`, mantido em `ActionDef`

**Justificativa:** `Signature` é a estrutura para funções puras. `ActionDef` é
para actions. A separação reflete a distinção: actions têm params nomeados e dict
dispatch; funções não.

## 7. Compatibilidade

### 7.1. Código de usuário

**Zero quebra.** Nenhum exemplo, stdlib, ou código de usuário real usa:
- Sintaxe `(x::Int)` em assinaturas de função
- `f{"k": v}` como chamada nomeada de função pura

Verificado em `examples/`, `stdlib/`, e todos os arquivos `.kata` do repositório.

### 7.2. Testes

Os testes E2E de dict dispatch em funções (7 testes) são convertidos para
actions ou testes negativos. Nenhum teste de função pura real é perdido —
são testes da própria feature que está sendo removida.

### 7.3. Arity-aware parsing

O parser arity-aware (Fase 3 do PRD-arity-uniformization) **não é afetado**.
A aridade continua sendo extraída de assinaturas. O `scan_lambdas` continua
funcionando. O ciclo de dois passes no driver não muda.

## 8. Riscos — Análise verificado no código

### R1: `can_start_expr` e `Token::LBrace` — ✅ SEM PROBLEMA

`Token::LBrace` já está na lista de `can_start_expr` (expressions.rs:20) e
`parse_expr_atom` já lida com `Token::LBrace` chamando `parse_brace_lit`
(expressions.rs:188). O dict como valor posicional funciona no parser sem
mudanças adicionais — basta remover o branch especial de dict dispatch em
`expr_apply.rs:222-232` e deixar o loop normal coletar o `DictLit` como átomo.

### R2: `interface_decl.rs` quebra métodos existentes — ✅ SEM PROBLEMA

Verificado: nenhum `implements` no prelude (`stdlib/core.kata`) ou em
`examples/` usa a sintaxe `(x::Int)` em assinaturas de método. Todos os
métodos são posicionais:

- Prelude: `+ :: Int Int => Int`, `show :: Int => Text`, `abs :: Int => Int`, etc.
- Exemplos: `+ :: Internal Internal => Internal`, `iter :: CustomIter => CustomIter`, etc.

A remoção de `try_parse_named_type_param` em `interface_decl.rs` não quebra
nenhum código real. Os dois pontos de uso (linhas 344 e 477) podem ser
substituídos por `parse_type_expr()` direto.

### R3: Remoção de `param_names` em cascata — ⚠️ CUSTO MECÂNICO ALTO

Mapeamento completo do impacto:

**Arquivos de produção (não-teste) que referenciam `param_names`: 134 refs**

| Arquivo | Uso | Ação |
|---|---|---|
| `kata-parser/src/action_decl.rs` | Coleta e propaga `param_names` de actions | **MANTER** — actions continuam com params nomeados |
| `kata-parser/src/interface_decl.rs` | Coleta `param_names` de métodos | **REMOVER** — métodos viram posicionais |
| `kata-parser/src/sig.rs` | Coleta `param_names` de Sigs | **REMOVER** — funções viram posicionais |
| `kata-resolution/src/types.rs:69` | `Signature.param_names` | **REMOVER** |
| `kata-resolution/src/types.rs:170` | `ActionDef.param_names` | **MANTER** |
| `kata-resolution/src/lib.rs:189` | Extrai `param_names` de `Item::Sig` | **REMOVER** |
| `kata-resolution/src/lib.rs:314,320` | Propaga para `Signature` | **REMOVER** |
| `kata-resolution/src/lib.rs:412,420` | Propaga para `OverloadInfo` via `Signature` | **REMOVER** |
| `kata-resolution/src/pass0.rs:457,565` | Propaga `param_names` de métodos | **REMOVER** |
| `kata-inference/src/infer/helpers.rs:41` | `populate_dispatch_table`: `param_names: sig.param_names.clone()` | **REMOVER** — vira `vec![None; sig.param_types.len()]` ou some |
| `kata-inference/src/infer/helpers.rs:132-221` | `reorder_dict_args_to_tuple` | **MANTER** — só chamado por action_call.rs |
| `kata-inference/src/infer/apply.rs:203-380` | Dict dispatch para funções | **REMOVER** bloco inteiro |
| `kata-inference/src/infer/function_infer.rs:163` | `log_synthesis` usa `param_names` | **VERIFICAR** — ver abaixo |
| `kata-inference/src/infer/action_infer.rs:160` | `log_synthesis` usa `param_names` | **MANTER** — action |
| `kata-inference/src/infer/recursion.rs:108` | `zip(target.param_names.iter())` | **VERIFICAR** — ver abaixo |
| `kata-codegen/src/lowering/action_def.rs:172` | Codegen de actions usa `param_names` | **MANTER** |
| `kata-core/src/dispatch.rs:39` | `OverloadInfo.param_names` | **MANTER** — actions precisam |
| `kata-core/src/type_env.rs:31-193` | `TypeBinding.param_names`, `lookup_param_names` | **REMOVER** ou tornar dead code |
| `kata-comptime/src/constant_fold.rs:143` | Referencia `param_names` | **VERIFICAR** |

**Arquivos de teste que referenciam `param_names`: 15 refs**

| Arquivo | Uso | Ação |
|---|---|---|
| `kata-codegen/tests/coercion_grouped_e2e.rs:251,263` | `param_names: vec![]` em `OverloadInfo` literal | **MANTER** (vec![] é o default para funções) |
| `kata-codegen/tests/ret_directed_dispatch_e2e.rs:245,257` | idem | **MANTER** |
| `kata-core/tests/dispatch_test.rs:16` | idem | **MANTER** |
| `kata-core/tests/dispatch_iface.rs:22` | idem | **MANTER** |
| `kata-parser/tests/parser_test/actions.rs:158,199,208-209` | Testa `param_names` de actions | **MANTER** |
| `kata-parser/tests/parser_test/action_type_syntax.rs:67,73-74` | Testa `param_names` de actions | **MANTER** |
| `kata-driver/tests/two_passes_e2e.rs:335` | Comentário sobre dict dispatch em função | **REMOVER/REESCREVER** |

**Verificações necessárias — RESOLVIDAS:**

1. **`function_infer.rs:158-163`** — `@log` em funções puras referencia nomes
   de params no template. O código define `__param_{i}` por posição e `name`
   por nome no `log_env`. Se `param_names` for sempre `None` em funções, o
   segundo loop (linha 158-162) não define nomes — só `__param_{i}` funciona.
   **Impacto:** `@log{msg: "{x}"}` em função pura deixa de interpolar `x`
   por nome. Mas como funções não terão params nomeados, isso é coerente —
   o usuário usa `{__param_0}` ou não nomeia. **Ação:** passar
   `vec![None; param_types.len()]` para `synthesize_log_specs`. Não quebra
   `@log` em actions (que mantém `param_names`).

2. **`recursion.rs:108`** — `zip(target.param_names.iter())` onde `target`
   é `TypedAction` (itera sobre `actions`, não `functions`). `TypedAction`
   tem `param_names` vindo de `ActionDef` — **não é afetado**. O `collect_indirect_edges`
   só roda sobre actions.

3. **`kata-comptime/constant_fold.rs:143`** — `action.param_names` onde
   `action` é `TypedAction`. Também só actions — **não é afetado**.

**Conclusão:** As três verificações são não-problemas. `param_names` em
`Signature`/`FunctionSpec` pode ser removido sem afetar `@log`, recursão, ou
comptime — todos operam sobre `TypedAction`/`ActionDef` que mantém o campo.

**Estimativa de arquivos a tocar:**

- ~8 arquivos de produção (remoção/cambio)
- ~3 arquivos de teste (converter/remover)
- ~0-40 arquivos de teste (se `param_names` removido de `Signature` →
  construtores literais de `Signature` em testes quebram — estimativa
  baseada no padrão `directive_registry`)

**Mitigação:** Se o custo de remover `param_names` de `Signature` for proibitivo,
alternativa é manter o campo como `vec![None; N]` sempre. Menos limpo, mas zero
quebra mecânica. A semântica é a mesma — funções nunca têm nomes.

## 9. Default args em actions via dict-template

### 9.1. Motivação

Com dict dispatch exclusivo de actions, o prólogo da action já desempacota
o dict da chamada. Default args surgem naturalmente nesse modelo: se uma chave
do dict não está presente na chamada, usa o valor default definido na
declaração. O caller não sabe nem precisa saber quais params têm default —
só passa o que tem.

### 9.2. Sintaxe de declaração — dict-template

Actions com defaults declaram params como um **dict-template**: as chaves são
os params, `_` marca os obrigatórios, literais marcam os defaults.

```kata
# Action sem defaults (sintaxe atual, mantida como açúcar):
action echo(msg::Text) => Unit
    echo!(_msg)

# Action com defaults (dict-template):
action act{msg::Text: _, dft::Int: 5} => Unit
    echo!(_msg)
    echo!(_dft)
```

**Desugaração:** `(x::Int, y::Int)` (sintaxe atual sem defaults) é açúcar para
`{x::Int: _, y::Int: _}` — todos obrigatórios, sem defaults. O compilador
desugara uma forma na outra, mantendo o código existente funcionando.

### 9.3. Semântica do prólogo — merge

O prólogo da action faz um **merge** do dict da chamada sobre o dict-template:

```
result = declaration_defaults ∪ call_dict
```

- Chave presente no call dict → usa o valor do call
- Chave ausente no call dict com default → usa o default do template
- Chave ausente no call dict com `_` (obrigatório) → erro de compilação

```kata
action act{msg::Text: _, dft::Int: 5} => Unit
    echo!(_msg)
    echo!(_dft)

act!{"msg": "hello", "dft": 10}   # dft = 10 (sobrescrito)
act!{"msg": "hello"}              # dft = 5  (default do template)
act!{"dft": 10, "msg": "hi"}     # ordem não importa — dict
```

Não há caso especial: todos os params têm um valor no template, seja `_`
(obrigatório) ou literal (default). O merge é a operação única.

### 9.4. Chamada posicional

Actions com dict-template também suportam chamada posicional via tupla.
A ordem das chaves na declaração define a ordem posicional:

```kata
action act{msg::Text: _, dft::Int: 5} => Unit
    ...

act!("hello")        # posicional → msg="hello", dft=5 (default)
act!("hello", 10)    # posicional → msg="hello", dft=10
act!{"msg": "hi"}    # nomeado → msg="hi", dft=5
act!{"dft": 3}       # ERRO: msg é obrigatório (_), não tem default
```

Na chamada posicional, o prólogo mapeia por índice: elemento `i` da tupla →
chave `i` do template. Se a tupla tem menos elementos que o template, os
restantes usam defaults (ou erro se `_`).

### 9.5. Coexistência das duas sintaxes de declaração

| Sintaxe | Significado | Uso |
|---|---|---|
| `(x::Int, y::Int)` | Açúcar para `{x::Int: _, y::Int: _}` | Actions sem defaults (forma comum) |
| `{x::Int: _, y::Int: 5}` | Dict-template com defaults | Actions com defaults |

O compilador desugara `(x::Int, y::Int)` para `{x::Int: _, y::Int: _}`
internamente. Ambas produzem a mesma estrutura no AST — um dict-template
onde cada entrada tem `name`, `type`, `required: bool`, `default: Option<Expr>`.

Actions sem defaults (a grande maioria) continuam usando a sintaxe `(x::Int)`
sem mudança. Actions com defaults usam a sintaxe `{x::Int: _, y::Int: 5}`.

### 9.6. Vantagens sobre `y::Int = 5`

1. **Simetria declaração-chamada.** Declaração e chamada usam a mesma
   estrutura (dict). O "default" é literalmente "o valor que está no template
   quando a chave não vem do caller".

2. **`_` reutiliza conceito existente.** Kata5 já tem `_` como hole/placeholder.
   `_` na declaração = "este espaço é obrigatório, o caller tem que preencher".
   Mesmo conceito semântico — um espaço a preencher.

3. **Merge é a semântica, não desempacotamento.** Com `y::Int = 5`, o prólogo
   precisa saber qual param tem default e qual não tem — é um caso especial.
   Com dict-template, o prólogo faz um merge: `template ∪ call`. Não há caso
   especial — todos os params têm um valor no template.

4. **Sem novo operador.** `=` é comparação em Kata5, `:=` é binding. Usar
   `y::Int = 5` para default introduziria um terceiro significado para `=`.
   Dict-template usa `:` (já existe em DictLit) e `_` (já existe como hole).

### 9.7. Impacto no parser

- `action_decl.rs`: reconhecer `{` após o nome da action como início de
  dict-template (em vez de `(`). Parsear entradas `nome::Tipo: valor` onde
  valor é `_` (obrigatório) ou uma expressão (default).
- `try_parse_named_type_param` permanece em `action_decl.rs` (actions mantêm
  params nomeados).
- A sintaxe `(x::Int, y::Int)` continua parseada como hoje e desugarada
  para `{x::Int: _, y::Int: _}` internamente.

### 9.8. Impacto na inferência

- `action_call.rs`: o prólogo da action já desempacota o dict. A mudança é
  que o desempacotamento agora faz merge com defaults do template, não apenas
  mapeia chaves para posições.
- `reorder_dict_args_to_tuple` é estendido: em vez de apenas reordenar,
  preenche faltantes com defaults do template.
- A chamada posicional `act!("hello")` mapeia por índice para o template,
  preenchendo defaults dos faltantes.

### 9.9. Impacto na ABI

A ABI da action não muda — continua `(rt, fiber_arena, caller_arena, args_ptr)`.
O `args_ptr` aponta para uma tupla na arena do caller. O prólogo da action
recebe a tupla, converte para dict interno (se chamada posicional) ou usa
o dict diretamente (se chamada nomeada), faz merge com defaults, e produz
os bindings `__param_N` para o body.

### 9.10. Fases de implementação de default args

Default args é implementado nas Fases 6-8 deste PRD, **após** as Fases 1-5
(remoção de dict dispatch de funções, guard em actions, dict como valor,
testes, documentação preliminar). Ver seção 5.