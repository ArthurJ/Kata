# PRD: Desambiguação de Variantes por Hint Contextual

## Status

**Status:** ✅ Completo
**Data:** 2026-09-17
**Resolve:** TODO.md item 🟢 "Qualificação obrigatória em conflito de variantes"

## 1. Objetivo

Quando duas ou mais enums têm uma variante de mesmo nome (ex: `Err` em
`Result` e `ReadResult`), o compilador hoje exige qualificação manual
(`Result::Err`) em posição de expressão. Este PRD propõe usar o hint de tipo
contextual (tipo de retorno, ascription, argumento de função tipada) para
disambiguar automaticamente — o mesmo princípio que `scrutinee_ty` já aplica
em patterns de match.

## 2. Motivação

### 2.1. O problema

`resolve_unqual_variant` (`variant.rs:64`) busca no `EnumRegistry` por nome
de variante. Se 2+ enums têm a variante, emite erro de ambiguidade sem
consultar nenhum tipo contextual. O hint de tipo já está disponível nos
sites de chamada (`infer_expr_hinted` recebe `hint: Option<&Ty>`,
`infer_apply` recebe `hint` e já o propaga para `infer_variant_construct`),
mas é ignorado na disambiguação de enum.

### 2.2. Inconsistência com match

Patterns de match não têm esse problema: `scrutinee_ty` disambigua antes de
resolver o braço. Se o scrutinee é `Result::(Int, Text)`, o pattern `Err _`
sabe que vem de `Result`. Expressões sem o mesmo mecanismo criam uma
assimetria: o usuário qualifica em expressão mas não em pattern.

### 2.3. Quando o hint está ausente

Lambda anônimo sem assinatura, `let` sem anotação, ou contexto genuinamente
polimórfico — não há tipo esperado. Nesses casos, a ambiguidade é real e
exigir qualificação é o comportamento correto. O hint não substitui a
qualificação; ele a torna desnecessária quando o contexto já fornece a
resposta.

## 3. Design

### 3.1. Princípio

**O tipo do contexto disambigua; sem contexto, qualifica.**

Não há heurística de precedência (prelude, frequência, escopo). Se o hint
filtra para exatamente 1 enum, resolve. Se não filtra ou não há hint,
qualifica. O mecanismo é o mesmo que `scrutinee_ty` em match — estendido
para expressões.

### 3.2. Extração do enum name do hint

O hint é um `Option<&Ty>`. Para disambiguar **qual enum**, só precisamos do
nome do enum no hint — os type args são irrelevantes para esta etapa (eles
importam depois, para `infer_variant_construct` preencher params faltantes,
já implementado pelo PRD-inferencia-bidirecional-variants).

```rust
fn enum_name_from_hint(hint: Option<&Ty>) -> Option<&str> {
    match hint? {
        Ty::Generic(name, _) => Some(name.as_str()),
        Ty::Sum(name) => Some(name.as_str()),
        _ => None,
    }
}
```

`Ty::Var`, `Ty::Prim`, `Ty::Function`, etc. não carregam nome de enum —
retornam `None` e o hint é ignorado (comportamento atual preservado).

### 3.3. Protocolo de filtragem

`resolve_unqual_variant` recebe `hint: Option<&Ty>`. Após
`find_enums_with_variant` retornar `candidates`:

```
candidates = find_enums_with_variant(name)

if candidates.is_empty() → UnboundName (inalterado)

if candidates.len() == 1 → resolve (inalterado)

if candidates.len() > 1:
    match enum_name_from_hint(hint):
        None → erro de ambiguidade (inalterado: "qualifique")
        Some(expected_enum) →
            filtered = candidates.filter(|c| *c == expected_enum)
            match filtered.len():
                1 → resolve com este enum
                0 → erro de INCOMPATIBILIDADE (novo)
                _ → impossível (filtered ⊆ candidates, max 1 match)
```

O caso `filtered.len() > 1` é impossível: `candidates` é uma lista de nomes
de enums únicos (dedup em `find_enums_with_variant`), e o filtro é por
igualdade de string. No máximo 1 sobrevive.

### 3.4. Dois tipos de erro distintos

Hoje todo caso de 2+ candidatos produz o mesmo erro. Com a filtragem,
surge um segundo tipo:

| Cenário | Erro | Mensagem |
|---|---|---|
| 2+ candidatos, sem hint | Ambiguidade | "variante 'Err' é ambígua — existe em: Result, ReadResult. Qualifique" |
| 2+ candidatos, hint incompatível | Incompatibilidade | "o contexto espera Option, mas Err não é variante de Option. Variantes de Option: Some, None" |

A distinction importa porque a ação do usuário muda: na ambiguidade, ele
qualifica (`Result::Err`). Na incompatibilidade, a variante está errada —
qualificar não resolve, ele precisa trocar a variante ou mudar o tipo
esperado.

A mensagem de incompatibilidade lista as variantes do enum esperado para
orientar o usuário.

## 4. Sites de mudança

### 4.1. `resolve_unqual_variant` — `variant.rs`

Assinatura atual:
```rust
pub(crate) fn resolve_unqual_variant(
    name: &str,
    span: &Span,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)>
```

Nova assinatura:
```rust
pub(crate) fn resolve_unqual_variant(
    name: &str,
    span: &Span,
    ctx: &InferCtx,
    hint: Option<&Ty>,  // NOVO
) -> InferResult<(Ty, TypedExprKind)>
```

Lógica adicionada no branch `candidates.len() > 1` (linha 77):
```rust
if candidates.len() > 1 {
    if let Some(expected) = enum_name_from_hint(hint) {
        if let Some(&matched) = candidates.iter().find(|c| **c == expected) {
            // Hint filtrou para 1 — resolve com este enum.
            return resolve_variant(matched, name, span, ctx);
        } else {
            // Hint aponta para enum que não tem esta variante.
            return Err(MiddleError::UnboundName {
                suggestion: None,
                name: format!(
                    "o contexto espera {expected}, mas '{name}' não é variante de {expected}. \
                     Variantes de {expected}: {}",
                    ctx.enum_registry.variant_names(expected).join(", ")
                ),
                span: (*span).into(),
            });
        }
    }
    // Sem hint ou hint não carrega enum — ambiguidade original.
    return Err(MiddleError::UnboundName { ... });  // inalterado
}
```

O corpo após o branch de ambiguidade (linhas 88-149) é extraído para
`resolve_variant(enum_name, name, span, ctx)` para reuso tanto no caminho
de 1 candidato quanto no caminho de hint-filtrado.

### 4.2. `infer_expr_hinted` — `expr.rs:243`

```rust
// Antes:
match resolve_unqual_variant(name, span, ctx) {

// Depois:
match resolve_unqual_variant(name, span, ctx, hint) {
```

O `hint` já é parâmetro de `infer_expr_hinted` — nenhuma mudança na
assinatura do caller.

### 4.3. `infer_apply` — `apply.rs:467-501`

O caminho de variante desqualificada com payload (ex: `Ok 42`) faz
disambiguação inline:

```rust
// Antes (linha 467):
let candidates = ctx.enum_registry.find_enums_with_variant(&func_name);
if candidates.len() == 1 {
    let enum_name = candidates[0];
    // ... infer_variant_construct
}
if candidates.len() > 1 {
    return Err(/* ambígua */);
}
```

Depois:
```rust
let candidates = ctx.enum_registry.find_enums_with_variant(&func_name);
if candidates.len() == 1 {
    let enum_name = candidates[0];
    // ... infer_variant_construct (inalterado)
}
if candidates.len() > 1 {
    if let Some(expected) = enum_name_from_hint(hint) {
        if let Some(&enum_name) = candidates.iter().find(|c| **c == expected) {
            // ... infer_variant_construct com enum_name (inalterado)
        } else {
            return Err(/* incompatibilidade */);
        }
    } else {
        return Err(/* ambígua — inalterado */);
    }
}
```

O `hint` já é parâmetro de `infer_apply` e já é passado para
`infer_variant_construct` (linha 485). A mudança é apenas no branch de
disambiguação.

### 4.4. `enum_name_from_hint` — localização

Função livre, `pub(crate)`, em `variant.rs` — reusada por `expr.rs` e
`apply.rs`.

### 4.5. `EnumRegistry::variant_names` — método novo

Para a mensagem de incompatibilidade, listar as variantes do enum esperado:

```rust
/// Lista os nomes das variantes de um enum.
pub fn variant_names(&self, enum_name: &str) -> Vec<&str> {
    let origin = self.resolve_origin(enum_name)?;
    let key = (origin.to_string(), enum_name.to_string());
    self.variants.get(&key).map(|vs| vs.iter().map(|v| v.name.as_str()).collect()).unwrap_or_default()
}
```

## 5. Fontes de hint — cobertura

| Fonte | Já propaga hint? | Cobertura |
|---|---|---|
| Retorno de função nomeada | ✅ | `ret_ty` vira hint no body |
| Ascription `(Err "x")::Result` | ✅ | ascription propaga hint ao inner |
| Argumento de função tipada | ✅ | `infer_apply` passa hint para args |
| `?` (sugar) | ✅ | desugara para match; scrutinee_ty resolve |
| `let` com anotação de tipo | Verificar | Se não propaga, wirear |
| Lambda anônimo sem assinatura | N/A | Sem hint — qualifica (correto) |

O item `let` com anotação precisa verificação durante implementação. Se o
`let` não propaga hint para o RHS, é um wire adicional pontual — não muda o
design.

## 6. Ortogonalidade

- **PRD-inferencia-bidirecional-variants:** resolve type args faltantes no
  payload da variante usando o expected_ty. É ortogonal — opera depois que
  o enum já foi identificado. Este PRD resolve **qual enum**; aquele resolve
  **quais type args**.
- **Default type params (`Err(E=Text)`):** preenche params quando nem o
  contexto nem o payload fornecem. Também ortogonal — funciona depois da
  disambiguação de enum.
- **Exaustividade de match:** não afeta nem é afetado. Patterns já usam
  `scrutinee_ty`.

## 7. Testes

### Teste 1: Variante ambígua resolvida por hint de retorno

```kata
div :: Int Int => Result::(Int, Text)
lambda a b:
    match (NonZero b)
        Result::Ok nz: Result::Ok (/ a nz)
        Result::Err _: Result::Err("divisão por zero")
```

`Result::Err` é qualificado — não exercita o PRD. Versão que exercita:

```kata
fail :: Text => Result::(Int, Text)
lambda msg: Err msg
```

`Err` existe em `Result` e `ReadResult`. O hint de retorno
`Result::(Int, Text)` filtra para `Result`. Deve compilar sem qualificação.

### Teste 2: Variante ambígua sem hint — erro de ambiguidade

```kata
lambda msg: Err msg
```

Sem assinatura, sem hint. `Err` em 2+ enums. Erro: "ambígua — qualifique".

### Teste 3: Variante incompatível com hint — erro de incompatibilidade

```kata
bad :: Option::(Int) => Option::(Int)
lambda x: Err x
```

Hint é `Option`. `Err` não existe em `Option`. Erro: "o contexto espera
Option, mas 'Err' não é variante de Option. Variantes de Option: Some,
None".

### Teste 4: Variante unitária ambígua resolvida por hint

```kata
default :: Result::(Int, Text)
lambda: None
```

Se `None` existe em `Option` e outro enum, e o hint é `Result`, deve erro
de incompatibilidade (None não é de Result). Se `None` só existe em
`Option`, resolve sem hint (1 candidato — comportamento atual).

Cenário que testa filtragem com 2+ candidatos:

```kata
// Dado: True existe em Boolean e Flag
explicit_bool :: Boolean
lambda: True
```

Hint `Boolean` filtra `True` para `Boolean`. Resolve.

### Teste 5: Apply com payload — ambígua resolvida por hint

```kata
wrap :: Int => ReadResult::(Bytes)
lambda x: Data (bytes [x])
```

Se `Data` existe em `ReadResult` e outro enum, o hint `ReadResult::(Bytes)`
filtra. Resolve sem qualificar.

### Teste 6: Aplicação de função tipada passa hint

```kata
consume :: Result::(Int, Text) => Int
lambda r:
    match r
        Ok v: v
        Err _: 0

action main => Int
    consume (Err "fail")
```

`(Err "fail")` é argumento de `consume`, cujo parâmetro é
`Result::(Int, Text)`. O hint propaga pelo `infer_apply` para o arg.
Filtra `Err` para `Result`.

## 8. Definições de done

- [x] `resolve_unqual_variant` aceita `hint: Option<&Ty>`
- [x] `enum_name_from_hint` extrai nome de enum de `Ty::Generic` e `Ty::Sum`
- [x] Filtragem: 2+ candidatos + hint compatível → resolve 1 enum
- [x] Erro de incompatibilidade quando hint aponta para enum sem a variante
- [x] `expr.rs:243` passa `hint` para `resolve_unqual_variant`
- [x] `apply.rs:467` filtra candidatos por hint antes de erro de ambiguidade
- [x] `EnumRegistry::variant_names` implementado (reusado `variants_of` existente)
- [x] Testes 1-6 passam
- [x] `cargo test --workspace` passa (0 failures — 2253 testes)
- [x] TODO.md item atualizado/removido

## 9. Não-fazer

- Heurística de precedência por escopo/prelude/frequência
- Disambiguação por type args (não precisa — só o nome do enum importa)
- Mudanças em patterns de match (já coberto por `scrutinee_ty`)
- Disambiguação em nested apply (`f (Err x)`) além do que o hint de
  argumento já propaga