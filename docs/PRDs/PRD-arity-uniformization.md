# PRD — Uniformização de aplicação: arity-aware parsing e dict dispatch para funções e actions

**Status:** ✅ Concluído (parsing arity-aware implementado; dict dispatch para funções removido pelo PRD-dict-dispatch-fix.md)
**Data:** 2026-08-06
**Depende de:** Pipeline lex→parse→resolve→infer existente, DispatchTable, InterfaceRegistry

## 1. Objetivo

Uniformizar a passagem de argumentos entre funções puras e actions, e introduzir
parsing arity-aware que elimina a ambiguidade de `+ 5 * 2 2` e o descarte silencioso
de expressões em `5 + 2 2`.

Hoje:
- **Funções**: aplicação prefixa posicional com átomos greedy — `f a b c`.
- **Actions**: tupla posicional ou dict nomeado com `!` — `f!(a, b)` ou `f!{x: 1}`.

Proposta:
- **Ambas** recebem argumentos posicionais (açúcar) ou nomeados (dict).
- **Açúcar posicional**: `+ 1 2` — o parser coleta exatamente a **aridade
  padrão** da função, cada argumento parseado como sub-expressão completa.
- **Dict**: `+{a: 1, b: 2}` — dispatch por `(nome, nomes_params, tipos_params)`.
  Só funciona para overloads que declaram nomes de params.
- **Aridade não-padrão**: sobrecargas com aridade diferente da padrão **só** são
  acessíveis via dict. Chamada posicional sempre coleta a aridade padrão.
- `+ 1 2 3` com aridade padrão 2 → **erro** (excesso posicional sem separador).

## 2. Contexto do codebase

### 2.1. Parser atual — `parse_apply` (expr_apply.rs)

```rust
pub(crate) fn parse_apply(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    let callee = parser.parse_expr_post_ascription()?;
    // Literais não são callee — retornam imediatamente
    if matches!(&callee.node, Expr::IntLit { .. } | ...) {
        return Ok(callee);
    }
    let mut args = Vec::new();
    while parser.can_start_expr() {
        args.push(parser.parse_expr_atom_or_ascription()?);  // átomos greedy
    }
    // ...
}
```

O parser coleta **átomos** greedy sem limite de aridade. `+ 5 * 2 2` vira
`Apply(+, [5, *, 2, 2])` — 4 argumentos, onde `*` é `Ident("*")` solto.

`5 + 2 2` vira dois items: `EntryExpr(5)` e `EntryExpr(Apply(+, [2, 2]))`.
O REPL descarta o primeiro e imprime o resultado do segundo — comportamento
silenciosamente incorreto.

### 2.2. Action call — sintaxe `!` (expressions.rs:277-296)

```rust
// Ident ! (tuple) ou Ident ! {dict}
if matches!(self.peek(), Token::Bang) {
    self.advance(); // consume !
    let args = if matches!(self.peek(), Token::LBrace) {
        self.parse_brace_lit()?      // DictLit
    } else {
        self.parse_paren_expr()?     // Tuple ou Grouping
    };
    return Ok(Spanned::new(
        Expr::ActionCall { callee: name, args: Box::new(args) },
        span,
    ));
}
```

Actions já suportam tupla e dict. O dict é reordenado para tupla pelo typeck
via `reorder_dict_args_to_tuple` (helpers.rs:141).

### 2.3. DispatchTable — `OverloadInfo` (dispatch.rs:20)

```rust
pub struct OverloadInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub ffi_symbol: Option<String>,
    pub is_action: bool,
    // ...
    pub param_names: Vec<Option<String>>,  // Some(nome) para actions, None/empty para funções
}
```

`param_names` já existe mas só é populado para actions. Funções puras e FFI
têm `param_names: vec![]`.

### 2.4. Signature (types.rs:45)

```rust
pub struct Signature {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub ffi_symbol: Option<String>,
    pub is_action: bool,
    pub is_commutative: bool,
    pub type_params: Vec<String>,
    // NOTA: não há param_names — só ActionDef tem.
}
```

`Signature` não tem `param_names`. Para suportar dict dispatch em funções
puras, é preciso propagar nomes de params do AST (`Sig` com params nomeados)
até o `OverloadInfo`.

### 2.5. Prelude — aridades

O prelude tem 58 nomes de função. `+` tem 8 overloads, todas aridade 2.
`*` tem 3 overloads, todas aridade 2. A verificação completa de aridade
única está pendente, mas a evidência parcial sugere que o prelude não tem
overloads com aridades diferentes para o mesmo nome.

### 2.6. Pipeline atual (driver)

```
lex → parse → resolve(prelude + user) → infer_module → monomorphize → optimize → jit_eval
```

O parser não tem acesso a informações de aridade — só vê tokens. A aridade
só está disponível após `resolve`, que produz `ResolvedModule.signatures`.

## 3. Design

### 3.1. Regra de aridade padrão

Cada nome de função tem uma **aridade padrão**: a aridade da **primeira
overload declarada** para aquele nome. O parser usa essa aridade para decidir
quantos argumentos posicionais coletar.

```kata
# Aridade padrão = 2 (primeira overload)
+ :: Int Int => Int
+ :: Float Float => Float        # mesma aridade — ok

# Aridade diferente da padrão
+ :: (x::Int) => Int              # aridade 1 ≠ padrão 2
# Chamada posicional: + 1 → erro (esperava 2, recebeu 1)
# Chamada por dict: +{"x": 1} → ok (dispatch por nomes_params + tipos)
```

### 3.2. Açúcar posicional

`+ 1 2` — o parser coleta exatamente N argumentos (aridade padrão), cada um
parseado como sub-expressão completa via `parse_apply`. O parser:

1. Parseia o callee (`+` → `Ident("+")`)
2. Consulta `arities["+"]` → `2` (aridade padrão)
3. Coleta exatamente 2 argumentos, cada um via `parse_apply` (permite
   sub-aplicações: `+ 5 * 2 2` → arg1 = `5`, arg2 = `Apply(*, [2, 2])`)
4. Constrói `Apply { callee: "+", args: [5, Apply(*, [2, 2])] }`

Se após coletar N args o próximo token `can_start_expr()` e **não** é
`StmtSep` nem `Eof` → erro: "excesso de argumentos posicionais para `+`
(aridade padrão 2). Use `+{...}` para aridade diferente."

**Regra de aplicação vs valor:**

- `Ident` seguido de tokens que `can_start_expr()` → **aplicação**
  (arity-aware, coleta N args)
- `Ident` sem tokens following → **valor** (referência à função)
- `(Ident)` ou `(expr)` → **grouping**, sempre valor — nunca aplicação
  no escopo externo

O grouping é o mecanismo explícito para passar uma função como valor em
posição de argumento. Sem grouping, um `Ident` com aridade conhecida
seguido de tokens é sempre interpretado como aplicação.

```kata
# + como aplicação (callee seguido de args):
+ 1 2              # Apply(+, [1, 2]) → 3

# + como valor (grouping sem args following):
map (+) [2 3]      # Grouping(Ident("+")) → valor Int->Int->Int
# (se map espera Int->Int, precisa de holes: map (+ _ _) [2 3])

# Sub-aplicação como argumento (arity-aware):
* 5 + 2 2          # Apply(*, [5, Apply(+, [2, 2])]) → 5 * 4 → 20
```

**Restrição:** `{...}` após um callee é **sempre** dict dispatch (args
nomeados), independente de whitespace. Para passar um dict como **valor**
posicional, é obrigatório nomeá-lo: `foo{x: {a: 1}}`.

Tuplas não têm essa restrição: `+ (1, 2)` é `+` aplicado a 1 argumento
posicional (a tupla `(1, 2)` como valor). O parser já lida com isso —
`parse_paren_expr` produz `Expr::Tuple` quando há vírgula e
`Expr::Grouping` quando não há, ambos átomos válidos. Não há
desaçúcaração nem ambiguidade, porque tuplas não têm chaves para casar com
nomes de params.

Isso elimina a ambiguidade entre "dict como valor" e "dict como dispatch
nomeado". A única forma de dispatch nomeado é `{...}` após callee. A única
forma de passar dict como valor é via param nomeado.

### 3.3. Forma nomeada (dict dispatch)

`+{a: 1, b: 2}` é sempre dispatch nomeado. O type checker:

1. Busca overloads de `+` que tenham `param_names` não-vazios
2. Casa chaves do dict contra nomes dos params
3. Verifica tipos dos valores
4. Despacha a overload que casa exatamente

Se nenhuma overload tem `param_names` → erro: "`+` não declara params nomeados
— use chamada posicional."

Se múltiplas overloads casam → `AmbiguousDispatch`.

**Tupla/dict como valor:** `+ (1, 2)` passa a tupla `(1, 2)` como 1
argumento posicional — aplicação normal, sem desaçúcaração. `foo{x: {a: 1}}`
passa um dict como valor do param `x` (dict como valor exige nome, porque
`foo {a: 1}` é interpretado como dict dispatch, não como valor posicional).

**Sem ambiguidade:** `f{...}` e `f {...}` são ambos dict dispatch — whitespace
não distingue. Para passar um dict como valor, use `f{x: {a: 1}}`. Para passar
uma tupla como valor, use `f (1, 2)` normalmente.

### 3.4. Uniformização funções × actions

| Aspecto | Função | Action |
|---|---|---|
| Sintaxe posicional (açúcar) | `+ 1 2` | `f!(1, 2)` |
| Sintaxe nomeada (dict) | `+{a: 1, b: 2}` | `f!{a: 1, b: 2}` |
| Tupla como valor posicional | `+ (1, 2)` | `f!((1, 2))` |
| Dict como valor (exige nome) | `foo{x: {a: 1}}` | `foo!{x: {a: 1}}` |
| Marcador de side-effect | sem `!` | com `!` |
| Mecanismo de dispatch | DispatchTable por tipos (posicional) ou nomes+tipos (dict) | idem |

A diferença entre função e action permanece sendo o `!` — que marca
side-effect. A passagem de argumentos é a mesma: posicional (açúcar) ou
nomeado (dict).

### 3.5. Ciclo de dois passes no pipeline

O parser precisa da aridade padrão antes de parsear. Como a aridade só está
disponível após `resolve`, introduzimos um ciclo. O Pass 1 parseia **apenas
declarações** (Sigs, implements, data, enum, action defs) — não entry exprs.
As aridades vêm exclusivamente de signatures, que são definidas em declarações,
não em expressões. Entry exprs consomem funções, não as definem.

```
Pass 1:  lex → parse_decls_only → resolve_decls → extrair aridades
Pass 2:  lex → parse (arity-aware, completo) → resolve → infer → ...
```

`extrair_aridades` produz `HashMap<String, usize>` — um valor por nome (a
aridade da primeira overload declarada).

`parse_decls_only` reconhece declarações pelos tokens iniciais:
- `Ident ::` → Sig
- `action` / `implements` / `data` / `enum` → keywords
- Tudo else → entry expr: skipar tokens até próximo `StmtSep` ou EOF

O custo é um parse parcial (declarações apenas) + parse completo arity-aware.
Tipicamente ~1.1x, não 2x. E não produz AST inválido, porque entry exprs não
são parseadas.

### 3.6. REPL

No REPL, cada input passa pelo ciclo de dois passes. Os items acumulados da
sessão fornecem aridades das funções do usuário definidas anteriormente.

```
Para cada input:
  Pass 1: parse (greedy) → resolve(prelude + items acumulados + input) → aridades
  Pass 2: parse (arity-aware) → resolve → infer → eval
```

## 4. Mudanças por crate

### 4.1. `kata-parser`

- `Parser` ganha campo `arities: Option<&HashMap<String, usize>>`.
- `parse(tokens)` mantém assinatura atual — `arities = None` (greedy atoms).
- Nova função `parse_with_arity(tokens, &arities)` — `arities = Some(...)`.
- `parse_apply` ganha branch: se callee é `Ident(name)` e `arities[name]`
  existe, coletar exatamente N args via `parse_apply` (não `parse_atom`).
  Após coletar N args, verificar se sobra token que `can_start_expr()` sem
  `StmtSep` → erro.
- Nova sintaxe: `Ident { dict }` (sem `!`) → `Expr::ApplyDict` ou
  `Expr::Apply { callee, args: [DictLit] }`.

### 4.2. `kata-resolution`

- `Signature` ganha campo `param_names: Vec<Option<String>>`.
- `pass0.rs` propaga nomes de params do AST (`Sig` com params nomeados
  `(x::Int)`) até `Signature.param_names`.
- `populate_dispatch_table` propaga `param_names` para `OverloadInfo`.

### 4.3. `kata-inference`

- `infer_apply` ganha caminho para dict dispatch em funções puras (não apenas
  actions). Reusa `reorder_dict_args_to_tuple` existente.
- Validação: dict dispatch só funciona se a overload tem `param_names`
  não-vazios.

### 4.4. `kata-driver`

- `run` e `repl` passam pelo ciclo de dois passes:
  1. Pass 1: `parse_decls_only` → resolve_decls → extract_arities
  2. Pass 2: parse arity-aware (completo) → resolve → infer → codegen
- `extract_arities(signatures: &[Signature]) -> HashMap<String, usize>`.
- `parse_decls_only`: novo entry point no parser que skipa entry exprs,
  parseando apenas Sigs, implements, data, enum, action defs.

## 5. Decisões de design

### D1: Aridade padrão = primeira overload declarada

Justificativa: determinística, não requer heurística. A primeira overload
definida no código (prelude ou usuário) estabelece a aridade do açúcar
posicional.

Risco: reordenar overloads muda a aridade padrão. Mitigação: o resolver
pode warningar se um nome tem overloads com aridades diferentes, indicando
que a ordem importa.

Alternativa descartada: aridade mais comum (moda). Requer contagem e pode
mudar ao adicionar overloads — não-determinístico com respeito ao código
existente.

### D2: `{...}` após callee é sempre dict dispatch; tupla como valor é aplicação normal

Justificativa: `f{...}` e `f {...}` são ambos dict dispatch — whitespace não
distingue. Para passar um dict como **valor** posicional, é obrigatório
nomeá-lo: `f{x: {a: 1}}`. Isso elimina a ambiguidade entre "dict como valor"
e "dict como dispatch nomeado".

Tuplas não têm essa ambiguidade: `+ (1, 2)` é `+` aplicado a 1 argumento
posicional (a tupla como valor). O parser já lida com isso — `(1, 2)` é um
átomo válido. Não há desaçúcaração nem restrição, porque tuplas não têm chaves
para casar com nomes de params.

Alternativa descartada: `+(1, 2)` como desempacotamento posicional (2 args).
Problema: ambíguo com `+` recebendo 1 arg do tipo tupla. Sem desempacotamento
implícito, a semântica é determinística.

### D3: Dict dispatch exige nomes de params declarados

Justificativa: o dict casam por nome. Sem nomes, não há como dispatchar.
Overloads com aridade não-padrão precisam declarar nomes — é o custo de
ter aridade diferente.

Overloads com aridade padrão não precisam de nomes — o açúcar posicional
funciona sem eles. Dict é opt-in para aridade padrão.

### D4: Ciclo de dois passes em vez de prelude-only

Justificativa: o prelude é fixo, mas funções do usuário também precisam de
arity-aware parsing. O ciclo permite que o Pass 2 use aridades do prelude
**e** do usuário. O custo é um parse extra, que é trivial.

Alternativa descartada: pré-calcular aridades do prelude em build time.
Limitado a prelude — não cobre funções do usuário.

### D5: `+ 1 2 3` é erro, não dois items

Justificativa: hoje `+ 1 2 3` vira `Apply(+, [1, 2, 3])` (3 args) ou dois
items separados (`+ 1 2` e `3`). Ambos são comportamentos surpreendentes.
Com arity-aware parsing, `+` coleta 2 args e `3` é excesso sem separador →
erro claro. O usuário usa `StmtSep` para separar items ou dict para aridade
diferente.

### D6: Funções sem aridade conhecida usam greedy atoms

Justificativa: funções definidas no Pass 1 que o Pass 2 ainda não conhece
(ex: forward references dentro do mesmo módulo) caem para greedy atoms.
Isso é o fallback seguro — o type checker reclama de arity mismatch se
houver excesso.

## 6. Fases de implementação

### Fase 1: Propagação de `param_names` em `Signature`

> **⚠️ Substituída** — Fases 1-2 do `PRD-dict-dispatch-fix.md` revertem esta fase:
> funções puras são exclusivamente posicionais. `param_names` removido de
> `Signature`, `Item::Sig`, e `ImplMethod`. `OverloadInfo.param_names` é `vec![]`
> para funções.

- Adicionar `param_names: Vec<Option<String>>` em `Signature` (types.rs)
- Propagar nomes de params do AST em `pass0.rs` (Sigs com params nomeados)
- Propagar em `populate_dispatch_table` (helpers.rs)
- `cargo test --workspace` não regrediu

**DoD Fase 1:** `OverloadInfo.param_names` populado para funções puras com
params nomeados. Testes existentes passam.

### Fase 2: Dict dispatch para funções puras

> **⚠️ Substituída** — `PRD-dict-dispatch-fix.md` Fase 2 remove o bloco de dict
> dispatch de `apply.rs` (~180 linhas). Funções não recebem args nomeados.

- Em `infer_apply`, detectar `Expr::Apply` com `DictLit` como único arg
- Reusar `reorder_dict_args_to_tuple` para mapear chaves → params
- Validar que a overload alvo tem `param_names` não-vazios
- Testes: `+{a: 1, b: 2}` despacha para `+ :: (a::Int) (b::Int) => Int`

**DoD Fase 2:** `f{a: 1, b: 2}` funciona para funções puras com params nomeados.

### Fase 3: Parser arity-aware

- Adicionar `arities: Option<&HashMap<String, usize>>` em `Parser`
- `parse_with_arity(tokens, &arities)` — novo entry point
- `parse_apply` branch: se callee é `Ident` com aridade conhecida, coletar N
  args via `parse_apply`; erro se excesso sem `StmtSep`
- Parser de `Ident { dict }` → `Expr::Apply` com `DictLit` (ou `Expr::ApplyDict`)
- Testes: `+ 5 * 2 2` → `Apply(+, [5, Apply(*, [2, 2])])` → `9`
- Testes: `+ 1 2 3` com aridade 2 → erro de parser

**DoD Fase 3:** `+ 5 * 2 2` parseia e avalia como `9`. `+ 1 2 3` dá erro de
parser. `parse(tokens)` (sem aridade) mantém comportamento atual.

### Fase 4: Ciclo de dois passes no driver

- `extract_arities(signatures) -> HashMap<String, usize>`
- `parse_decls_only`: novo entry point no parser que skipa entry exprs
- Modificar pipeline de `run` e `repl`:
  1. Pass 1: `parse_decls_only` → resolve_decls → extract_arities
  2. Pass 2: parse arity-aware (completo) → resolve → infer → codegen
- Testes E2E: `+ 5 * 2 2` → `9` via `kata run` e REPL
- Testes E2E: `+ 1 2 3` → erro claro via `kata run` e REPL

**DoD Fase 4:** `kata run` e `kata repl` executam o ciclo de dois passes.
`+ 5 * 2 2` imprime `9`. `+ 1 2 3` reporta erro de parser.

### Fase 5: Sintaxe `Ident { dict }` para funções (sem `!`)

> **⚠️ Substituída** — `PRD-dict-dispatch-fix.md` Fase 3 remove o branch especial
> de dict dispatch em `expr_apply.rs`. `f {"k": v}` agora é dict como valor
> posicional (não args nomeados). Funções são exclusivamente posicionais.

- Parser: `Ident {` sem `!` → `Expr::Apply` com `DictLit` (ou `Expr::ApplyDict`)
- Type checker: dispatch por nomes de params
- Testes: `+{a: 1, b: 2}` despacha para overload com params nomeados `a`, `b`
- Testes: `+{x: 1}` despacha para overload de aridade 1 com param `x`

**DoD Fase 5:** `+{a: 1, b: 2}` funciona para funções puras com params nomeados.

### Fase 6: Validação e mensagens de erro

- Resolver warninga quando nome tem overloads com aridades diferentes
- Erro claro para excesso posicional: "`+` tem aridade padrão 2, mas recebeu
  3 argumentos posicionais. Use `+{...}` para aridade diferente."
- Erro claro para dict sem nomes: "`+` não declara params nomeados — use
  chamada posicional `+ a b`."
- Testes de mensagens de erro

**DoD Fase 6:** Mensagens de erro são claras e acionáveis. `cargo test
--workspace` passa.

### Fase 7: Atualização de documentação

- `docs/Kata-lang-manual.md`:
  - Atualizar § sobre aplicação de funções (hoje descreve greedy atoms)
  - Adicionar § sobre açúcar posicional e aridade padrão
  - Adicionar § sobre dict dispatch para funções puras
  - Adicionar § sobre tupla/dict como valor posicional
  - Atualizar § sobre actions (`!` com tupla/dict) — referenciar uniformização
  - Atualizar § sobre sobrecargas — documentar regra de aridade padrão
- `docs/mapa-funcionalidades.md`:
  - Atualizar entrada de aplicação prefixa
  - Adicionar entrada de dict dispatch para funções
- `docs/ROADMAP.md`:
  - Adicionar entrada "Uniformização de aplicação ✅" no pós-Fio 15

**DoD Fase 7:** Manual, mapa e roadmap refletem a nova semântica. Seções
sobre aplicação de funções e actions são consistentes com o PRD.

## 7. Casos de exemplo

### 7.1. Açúcar posicional (aridade padrão)

```kata
+ 1 2              # → Apply(+, [1, 2]) → 3
+ 5 * 2 2          # → Apply(+, [5, Apply(*, [2, 2])]) → 9
* 5 + 2 2          # → Apply(*, [5, Apply(+, [2, 2])]) → 5 * 4 → 20
+ (+ 1 2) 3        # → Apply(+, [Grouping(Apply(+, [1, 2])), 3]) → 6
```

### 7.2. Excesso posicional (erro)

```kata
+ 1 2 3            # ERRO: + tem aridade padrão 2, recebeu 3 posicionais
+ 1 2
3                  # OK: dois items separados por quebra de linha
```

### 7.3. Dict dispatch (aridade não-padrão)

```kata
+ :: Int Int => Int                    # aridade padrão 2
+ :: (x::Int) => Int @ffi("unary_plus") # aridade 1, param nomeado "x"

+ 1 2              # → 3 (aridade padrão)
+ 1                # ERRO: + tem aridade padrão 2, recebeu 1
+{x: 1}            # → 1 (dict dispatch, overload de aridade 1)
+{a: 1, b: 2}      # ERRO: overload de aridade 2 não declara nomes
```

### 7.4. Tupla/dict como valor

```kata
# Tupla como valor posicional — aplicação normal, sem desaçúcaração
+ (1, 2)              # + aplicado a 1 arg (tupla) — se overload aceita (Int, Int), ok
foo (1, 2) (3, 4)     # foo aplicado a 2 args (duas tuplas)

# Dict como valor — exige param nomeado (f{...} é sempre dict dispatch)
bar{config: {a: 1, b: 2}}  # OK: dict como valor do param config
bar {a: 1, b: 2}           # dict dispatch — NÃO é dict como valor posicional
```

### 7.5. Passagem de função como valor

```kata
# Grouping é o mecanismo explícito para passar função como valor:
map (+) [2 3]            # (+) = Grouping(Ident("+")) → valor Int->Int->Int
                         # (se map espera Int->Int, mismatch de arity → erro de tipo)

map (+ _ _) [2 3]        # (+ _ _) = função parcial Int->Int → ok para map
map (+ 1 _) [2 3]        # (+ 1 _) = função parcial Int->Int → [3, 4]

# Sem grouping, + é aplicação (tem aridade 2, args following):
map + [2 3]              # + coleta [2 3] como 1º arg, falta 2º → erro de parser
map (lambda x: + x 1) [2 3]  # lambda sempre é valor → [3, 4]
```

### 7.6. Action com dict (já funciona, mantém)

```kata
action processar (x::Int) => Int
    + x 1

processar!(41)         # tupla posicional
processar!{x: 41}      # dict nomeado (já funciona hoje)
```

### 7.7. Função do usuário com aridade não-padrão

```kata
foo :: Int Int => Int                      # aridade padrão 2
foo :: (a::Int) (b::Int) (c::Int) => Int   # aridade 3, params nomeados

foo 1 2                # → foo(1, 2) → aridade padrão
foo 1 2 3              # ERRO: excesso posicional
foo{a: 1, b: 2, c: 3}  # → foo(1, 2, 3) → dict dispatch, aridade 3
```

## 8. Compatibilidade com código existente

### 8.1. Categoria A: Continua idêntico (a grande maioria)

Chamadas bem-formadas com aridade correta continuam produzindo o mesmo AST.
O parser arity-aware coleta o mesmo número de args que o greedy atoms coletaria
quando a expressão está bem-formada.

```kata
+ 1 2              # antes: Apply(+, [1, 2]) → depois: idêntico
show 42            # antes: Apply(show, [42]) → depois: idêntico
- 10 3             # idêntico
```

Chamadas com grouping explícito também continuam idênticas — o grouping é
um átomo válido em ambos os modos:

```kata
# quicksort.kata — grouping, continua igual
filter (< _ pivo) resto
+ (quicksort menores) [pivo : (quicksort maiores)]

# map_filter_fold.kata — grouping, continua igual
map (* _ 2) numeros
fold (+ _ _) 0 numeros
map (* _ 3) [1 2 3]
```

### 8.2. Categoria B: Antes dava erro, depois funciona (melhora)

Sub-aplicações sem grouping explícito que hoje falham passam a funcionar.
Isso é estritamente melhora — não quebra código existente.

```kata
+ 5 * 2 2          # antes: UnboundName("*") → depois: 9
* 5 + 2 2          # antes: UnboundName("+") → depois: 20
```

### 8.3. Categoria C: Antes dava erro ambíguo, depois dá erro claro

Chamadas com número incorreto de args posicionais hoje produzem erros de
type checking confusos (`NoOverload`, `UnboundName`). Depois produzem erros
de parser claros ("aridade padrão 2, recebeu 3"). O comportamento é o mesmo
(erro), mas a mensagem melhora.

```kata
+ 1                # antes: NoOverload (typeck) → depois: erro de parser (falta arg)
+ 1 2 3            # antes: NoOverload ou dois items → depois: erro de parser (excesso)
```

### 8.4. Categoria D: Quebra real — overloads com aridades diferentes

Se uma função tem overloads com aridades diferentes, a mudança é quebra real:

```kata
# foo :: Int Int => Int        (aridade padrão 2)
# foo :: Int Int Int => Int    (aridade 3, não-padrão)

foo 1 2 3
# antes: Apply(foo, [1, 2, 3]) → typeck despacha aridade 3 → funciona
# depois: parser coleta 2 args (aridade padrão), 3 é excesso → erro de parser
```

Esta é a única categoria de quebra real. Para que ela ocorra, é necessário
que existam funções com overloads de aridades diferentes — padrão que o PRD
desencoraja e que a Fase 6 warninga.

**Verificação necessária antes da Fase 3:**
1. Verificar todas as 58 funções do prelude — se todas têm aridade única,
   não há quebra no prelude.
2. Verificar todos os arquivos em `examples/` — se nenhum usa overloads
   com aridades diferentes, não há quebra nos exemplos.
3. Verificar testes E2E e de inferência — mesmo critério.

Se houver funções com aridades diferentes no prelude ou exemplos, refatorar
para aridade única antes de implementar o parser arity-aware.

### 8.5. Categoria E: `Ident` solto como argumento

Hoje, `Ident` sem args em posição de argumento é tratado como valor
(referência à função). Com arity-aware parsing, se o `Ident` tem aridade
conhecida e há tokens following, é tratado como aplicação.

```kata
# foo :: Int Int => Int (aridade 2)
# bar :: Int => Int (aridade 1)

foo bar 1 2
# antes: Apply(foo, [bar, 1, 2]) → 3 args → NoOverload (erro)
# depois: Apply(foo, [Apply(bar, [1]), 2]) → 2 args → funciona
```

Isso é melhora (erro → acerto), não quebra. Mas muda o AST de chamadas
que hoje "funcionam" por acidente — onde o `Ident` solto tipa como valor
por uma overload que aceita função como argumento.

```kata
# map :: (Int -> Int) [Int] => [Int] (aridade 2)
# Se alguém escrevesse (hoje inválido em Kata5, mas hipotético):
map + [1 2 3]
# antes: Apply(map, [+, [1,2,3]]) → 2 args → se + tipa como Int->Int->Int,
#         mismatch com Int->Int → erro de tipo
# depois: parse_apply(+) → + tem aridade 2, coleta [1 2 3] como 1º arg,
#         falta 2º → erro de parser
```

O erro muda de tipo para parser, mas o comportamento (erro) é o mesmo.

## 9. Riscos e mitigacões

### R1: Reordenar overloads muda aridade padrão

Se o usuário reordena métodos num `implements`, a primeira overload muda,
e a aridade padrão pode mudar. O açúcar posicional que funcionava pode
parar de funcionar.

**Mitigação:** Fase 6 warninga quando há overloads com aridades diferentes.
O usuário sabe que a ordem importa.

### R2: Funções do prelude com aridades diferentes

Se o prelude tiver funções com overloads de aridades diferentes (não
verificado ainda), o açúcar posicional pode quebrar para essas funções.

**Mitigação:** Verificar todas as 58 funções do prelude antes da Fase 3.
Se houver, refatorar para aridade única ou documentar como exceção.

### R3: ~~Pass 1 produz AST inválido~~ (eliminado)

O Pass 1 agora parseia apenas declarações (`parse_decls_only`), não entry
exprs. Declarações são estruturalmente válidas independentemente de
arity-aware parsing — Sigs, implements, data, enum não dependem de aridade
para serem parseadas. Entry exprs, que poderiam produzir AST inválido em
modo greedy, não são parseadas no Pass 1. R3 deixou de existir.

### R4: Performance do ciclo de dois passes

O Pass 1 parseia apenas declarações — tipicamente uma fração pequena do
arquivo. O custo total é ~1.1x (parse parcial + parse completo arity-aware),
não 2x. Para arquivos grandes, o overhead é dominado pelo Pass 2, que é o
parse completo que aconteceria de qualquer forma.

### R5: `Ident { dict }` sem `!` — parsing de `{` após Ident

`f{a: 1}` e `f {a: 1}` são ambos dict dispatch. O parser precisa reconhecer
`{` após `Ident` (com ou sem espaço) como início de dict dispatch, não como
expressão solta. Hoje `{` não inicia expressão em Kata5 (não há block
literals como expressões soltas), então não há conflito real.

**Mitigação:** Verificar que `{` após `Ident` sem `!` é não-ambíguo no
parser. Como `{` não é um token que inicia expressão posicional, o parser
pode tratá-lo como início de dict dispatch. Se houver ambiguidade futura
(ex: block literals), revisitar.

## 10. Aspectos não cobertos

- **Açúcar para actions sem `!`**: não proposto. Actions mantêm `!` como
  marcador de side-effect. A uniformização é na estrutura de argumentos
  (tupla/dict), não na sintaxe de chamada.
- **Default arguments**: não proposto. `Optional` cobre o caso de args
  opcionais.
- **Varargs**: não proposto. List ou Dict cobre o caso de número variável
  de args.
- **Type-directed dispatch no parser**: não proposto. O parser usa apenas
  aridade (informação estrutural), não tipos. O type checker continua
  fazendo dispatch por tipos.