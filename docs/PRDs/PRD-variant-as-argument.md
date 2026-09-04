# PRD: Variante de Enum como Argumento — Unificação de Ty::Var em match_score

## Motivo

Variantes de enum genérico como argumento de função falham em `type.no_overload`
mesmo quando o tipo é perfeitamente compatível. O problema afeta **todos** os
enums genéricos, não apenas `Optional::None`:

```kata
foo :: Optional::(Int) => Int
lambda x: 42
echo!(foo None)              # type.no_overload

bar :: Result::(Int, Text) => Int
lambda r: 42
echo!(bar (Err "msg"))       # type.no_overload
```

Como **retorno** (onde há hint), ambos funcionam — a inferência bidirecional
(`PRD-inferencia-bidirecional-variants.md`) preenche type params não-inferidos
pelo payload usando o tipo esperado da assinatura. Como **argumento**, o typeck
infere o tipo do arg **antes** do dispatch, sem hint do parâmetro esperado. O
resultando é `Generic("Optional", [Var("T")])` ou `Generic("Result", [Var("T"), Text])`.

## Diagnóstico

`match_score` (`dispatch/mod.rs:511-586`) compara arg vs param com `arg == param`
para exact match. `Generic("Optional", [Var("T")]) != Generic("Optional", [Int])`
→ cai no else final → `Score::incompatible()`.

`fits_return` (`expr.rs:155-188`) **já trata isso corretamente**:
- `Ty::Var(_) => true` (linha 157) — Var aceita qualquer tipo
- `Generic` com mesmo nome e aridade recursa para os type args (linha 172-174)

`match_score` não usa `fits_return` e não tem branch para `Ty::Var`.

## Design

### Princípio

`match_score` deve tratar `Ty::Var` dentro de `Ty::Generic` como compatível,
alinhando-se com `fits_return`. Um arg `Generic("Optional", [Var("T")])` casa
com param `Generic("Optional", [Int])` — não é exact match (Var não é Int), mas
é compatível (score iface, não incompatible).

### Site de mudança

`crates/kata-core/src/dispatch/mod.rs` — função `match_score`.

Após o bloco de `OverloadSet` (linha 558-574) e antes do else final (linha 575),
adicionar branch para `Ty::Generic` com mesmo nome e aridade, recursando para
type args via uma nova função auxiliar `ty_compatible` (ou reusar `fits_return`
se acessível — mas `fits_return` está em `kata-inference`, e `match_score` está
em `kata-core`, então a lógica precisa ser duplicada ou extraída para `kata-core`).

### Lógica

```rust
} else if let (Ty::Generic(a_name, a_args), Ty::Generic(p_name, p_args))
    = (arg, param)
    && a_name == p_name
    && a_args.len() == p_args.len()
    && a_args.iter().zip(p_args).all(|(a, p)| ty_var_compatible(a, p))
{
    // Generic com Var não-resolvido em type args — compatível mas não exact.
    iface += 1;
}
```

Onde `ty_var_compatible` é:

```rust
fn ty_var_compatible(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Var(_), _) | (_, Ty::Var(_)) => true,
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2))
            if n1 == n2 && a1.len() == a2.len() =>
        {
            a1.iter().zip(a2).all(|(x, y)| ty_var_compatible(x, y))
        }
        _ => a == b,
    }
}
```

### Por que iface e não exact

`Var("T")` não é `Int` — é um placeholder não-resolvido. Dar exact match
mascararia overloads mais específicas. Iface score é compatível (dispatch
seleciona) mas perde para exact match se houver uma overload com tipo concreto
exato. Isto é consistente com o tratamento de interface dispatch já existente.

### Não-alteração de match_score para Ty::Var top-level

`Ty::Var` como arg top-level (não dentro de Generic) já é tratado: o branch
`arg == param` falha, e cai para incompatible. Isto é **correto** — `Ty::Var`
top-level como argumento é um type param não-inferido sem contexto, e não deve
casar com parâmetros concretos. O fix é apenas para `Var` **dentro de Generic**.

## Ortogonalidade

- **Inferência bidirecional** (PRD existente): preenche type params via hint de
  retorno. Continua funcionando inalterada.
- **Default type params** (`Err(E|Text)`): preenche params com default quando
  nem o contexto fornece. Continua funcionando. Não é necessário `T|Unit` em
  `Optional` para resolver A7 — o `Var("T")` não-resolvido agora casa com
  qualquer tipo concreto no dispatch.
- **Ret-directed dispatch** (`fits_return` na filtragem de overloads por hint
  de retorno): já trata `Var` corretamente. Inalterado.

## Testes

### Teste 1: None como argumento

```kata
foo :: Optional::(Int) => Int
lambda x: 42

action main
    echo!(foo None)
main!()
```
Saída: `42`

### Teste 2: Err como argumento

```kata
bar :: Result::(Int, Text) => Int
lambda r: 42

action main
    echo!(bar (Err "msg"))
main!()
```
Saída: `42`

### Teste 3: Some como argumento (com payload)

```kata
baz :: Optional::(Int) => Int
lambda x: 42

action main
    echo!(baz (Some 10))
main!()
```
Saída: `42` (já funciona hoje — `Some 10` infere `T=Int` do payload)

### Teste 4: Qualificado sem payload

```kata
foo :: Optional::(Int) => Int
lambda x: 42

action main
    echo!(foo Optional::None)
main!()
```
Saída: `42`

### Teste 5: Default E|Text coopera

```kata
bar :: Result::(Int, Text) => Int
lambda r: 42

action main
    echo!(bar (Result::Err "msg"))
main!()
```
Saída: `42`

### Teste 6: Ambiguidade não introduzida

```kata
foo :: Optional::(Int) => Int
lambda x: 42
foo :: Optional::(Text) => Int
lambda x: 42

action main
    echo!(foo None)
main!()
```
Deve falhar com ambiguidade — `None` produz `Optional::(Var("T"))` que casa
com ambas as overloads com mesmo score iface.

## Definições de done

- [x] `match_score` trata `Generic` com `Var` em type args como compatível (iface)
- [x] `ty_var_compatible` extraída em `kata-core/src/dispatch/mod.rs`
- [x] `resolve_with_swap` caminho Ok aplica `unify` + `apply_subs` em overloads genéricas
- [x] Teste 1 (None como arg) passa
- [x] Teste 2 (Err como arg) passa
- [x] Teste 3 (Some como arg) continua passando
- [x] Teste 4 (Optional::None qualificado) passa
- [x] Teste 5 (Result::Err qualificado) passa
- [x] Teste 6 (ambiguidade) falha corretamente
- [x] `cargo test --workspace` passa (0 failures)
- [x] TODO.md atualizado (A7 removido)