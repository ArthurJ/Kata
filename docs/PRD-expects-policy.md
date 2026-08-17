# PRD — `expects` com `policy`: Verificação de Erro em Testes

**Status:** Concluído
**Data:** 2026-08-17
**Depende de:** `@test` runner ✅, `show` sintetizado ✅, `Result::(T, E)` ✅
**Não depende de:** `enum extends`, `KataError` no prelude, `ExpectSpec`, type bounds

## 1. Objetivo

Hoje `@test{expects: "..."}` aceita uma string mas **nunca a verifica** — o campo é
armazenado em `TestSpec.expects: Option<String>` e ignorado pelo driver. O teste
passa independentemente do erro retornado.

Este PRD faz `expects` funcional: o wrapper gerado aplica `show` no payload de
`Result::Err` e compara contra a string usando uma política de match.

## 2. Sintaxe

```kata
@test{desc: "timeout", expects: "Timeout", policy: prefix, args: ("http://slow.example")}
@test{desc: "panic", expects: "connection refused", policy: contains, args: ("http://crash.example")}
@test{desc: "exact", expects: "Panic(crash)", policy: exact, args: ("http://crash.example")}
```

### 2.1. Campos

| Campo | Tipo | Obrigatório | Descrição |
|-------|------|-------------|-----------|
| `desc` | `Text` | Não | Identificação do teste no relatório |
| `args` | `Expr` | Não | Argumentos para chamar a action |
| `timeout` | `Int` | Não | Timeout em ms |
| `expects` | `Text` | Não | String esperada do `show` do erro |
| `policy` | `Ident` | Não | Política de match: `exact`, `prefix`, `contains` |

### 2.2. Default de `policy`

Se `policy` é omitido e `expects` está presente, default é `exact`.

### 2.3. Políticas

| Política | Semântica | Caso de uso |
|----------|-----------|-------------|
| `exact` | `show(err) == expects` | Match completo da representação |
| `prefix` | `show(err).starts_with(expects)` | Casar só nome da variante: `"Timeout"` casa `"Timeout"` e `"Timeout(x)"` |
| `contains` | `show(err).contains(expects)` | Substring em qualquer posição |

### 2.4. Quando `expects` está ausente

Sem `expects`, o teste passa se a action completa sem timeout/deadlock —
comportamento atual, inalterado.

## 3. Semântica

### 3.1. O que é verificado

O wrapper gerado chama a action. Se o resultado é `Result::Err(payload)`, o
wrapper aplica `show` no payload e compara contra `expects` com a `policy`.

Se o resultado é `Result::Ok(_)`, o teste **falha** — `expects` declarou que
esperava erro, mas a action retornou sucesso.

### 3.2. `show` já é sintetizado

O inference já sintetiza `show :: Tipo => Text` para todo enum:
- Variante unitária → `"VariantName"`
- Variante com payload → `"VariantName(show(payload))"`

Para `Result::Err(MeuErro.Timeout)`, `show` produz `"Timeout"`.
Para `Result::Err(KataError.Panic("crash"))`, `show` produz `"Panic(crash)"`.

Nenhuma mudança na síntese de `show` é necessária.

### 3.3. Tipos não-Result

Se a action não retorna `Result`, `expects` é **erro compile-time** — o
usuário declara "espero erro" numa action que estruturalmente não pode
retornar erro. Compile error é mais honesto que warning: não dá falsa
sensação de segurança.

### 3.4. Interação com timeout/deadlock

Se a action dá timeout ou deadlock, o teste falha com `TIMEOUT`/`DEADLOCK`
antes de verificar `expects`. A verificação de `expects` só ocorre quando a
action completa normalmente.

## 4. Implementação

### 4.1. `TestSpec` — adicionar `policy`

Em `crates/kata-resolution/src/types.rs`:

```rust
pub struct TestSpec {
    pub desc: Option<String>,
    pub args: Option<Spanned<Expr>>,
    pub timeout: Option<i64>,
    pub expects: Option<String>,
    pub policy: Option<MatchPolicy>,  // novo
}

pub enum MatchPolicy {
    Exact,
    Prefix,
    Contains,
}
```

### 4.2. Directive parsing — aceitar `policy`

Em `crates/kata-resolution/src/directives.rs`:

- `expects: "..."` — já aceita `TextLit`, inalterado.
- `policy: exact|prefix|contains` — novo. Aceita `Ident` (não `TextLit`).
  Mapeia `exact` → `Exact`, `prefix` → `Prefix`, `contains` → `Contains`.
  Se omitido e `expects` presente → default `Exact`.

### 4.3. `TypedTestSpec` — propagar `policy`

Em `crates/kata-inference/src/typed_module.rs`:

```rust
pub struct TypedTestSpec {
    pub desc: Option<String>,
    pub args: Option<Spanned<TypedExpr>>,
    pub timeout: Option<i64>,
    pub expects: Option<String>,
    pub policy: Option<MatchPolicy>,  // novo
}
```

### 4.4. Codegen — wrapper verifica `expects`

Hoje o wrapper (`define_test_wrapper` em `test_runner.rs`) faz:
1. `scheduler_init` → arena
2. Lowera args
3. `spawn(action, args)`
4. `kata_rt_run()` → `result: i64`
5. `return result`

Com `expects`, o wrapper adicionalmente:
6. Se `expects` é `Some` e `result` não é sentinel:
   a. Chama `show(result)` via FFI (`__kata_show__{Tipo}`)
   b. Compara a string retornada contra `expects` com a `policy`
   c. Retorna status code: `0` = pass, `1` = fail (match falhou), `2` = fail (Ok quando esperava Err)
7. Se `expects` é `None`: retorna `result` como hoje.

**Detalhe de codegen:** O wrapper conhece o tipo de retorno da action
(`action.ret_ty`). Se é `Result::(T, E)`, sabe que precisa inspecionar `Err`.
O `show` do `Result` já é sintetizado — chama `__kata_show__Result` que
produz `"Ok(value)"` ou `"Err(variant)"`. Para distinguir Ok de Err, o
wrapper pode:

- **Opção A:** Chamar `show` no resultado completo e comparar. `"Ok(42)"` não
  casa `"Timeout"` com `prefix` → fail. Simples, mas não distingue "action
  retornou Ok" de "action retornou Err errado".
- **Opção B:** O wrapper faz `match` no Result em Kata (não em Rust). Se
  `Result::Ok(_)` → retorna `2` (esperava erro, veio Ok). Se `Result::Err(e)`
  → `show(e)` e compara. Mais preciso.

**Decisão:** Opção B. O codegen já sabe gerar `match` em Sum. O wrapper
gera um match no resultado com dois braços: Ok → fail status 2, Err → show +
compara.

### 4.5. Driver — interpretar status codes

Em `crates/kata-driver/src/main.rs`:

```rust
enum TestOutcome {
    Pass,
    Timeout,
    Deadlock,
    Fail(String),  // novo
}
```

`run_test_wrapper` interpreta o `i64` retornado:
- `TIMEOUT_SENTINEL` → `Timeout`
- `DEADLOCK_SENTINEL` → `Deadlock`
- `0` → `Pass`
- `1` → `Fail("expects mismatch: show(err) não casa policy")`
- `2` → `Fail("expected Err, got Ok")`

O relatório imprime `[FAIL]` com a mensagem.

### 4.6. Status codes — evitar colisão com sentinels

`TIMEOUT_SENTINEL` e `DEADLOCK_SENTINEL` são valores especiais de `i64`.
Os status codes `0`, `1`, `2` precisam não colidir com eles. Verificar os
valores dos sentinels e escolher codes fora desse range.

## 5. Estrutura esperada

```
crates/kata-resolution/src/types.rs        + MatchPolicy, TestSpec.policy
crates/kata-resolution/src/directives.rs    policy aceita Ident (exact/prefix/contains)
crates/kata-inference/src/typed_module.rs   TypedTestSpec.policy
crates/kata-inference/src/infer/action_infer.rs  cascade
crates/kata-codegen/src/lowering/test_runner.rs  wrapper gera match + show + compara
crates/kata-driver/src/main.rs              TestOutcome::Fail, interpreta status codes
crates/kata-driver/tests/test_runner_e2e.rs  + testes expects com policy
```

## 6. Testes E2E

1. Action que retorna `Result::Err(MeuErro.Timeout)` com `expects: "Timeout", policy: prefix` → PASS
2. Action que retorna `Result::Err(MeuErro.Timeout)` com `expects: "Panic", policy: prefix` → FAIL
3. Action que retorna `Result::Ok(42)` com `expects: "Timeout", policy: prefix` → FAIL (expected Err, got Ok)
4. Action que retorna `Result::Err(KataError.Panic("crash"))` com `expects: "crash", policy: contains` → PASS
5. Action que retorna `Result::Err(KataError.Panic("crash"))` com `expects: "Panic(crash)", policy: exact` → PASS
6. `expects` sem `policy` → default `exact`
7. Sem `expects` → comportamento atual (pass se completa)
8. Action sem `Result` com `expects` → compile error (não warning)

## 7. Decisões

| ID | Decisão | Status |
|----|---------|--------|
| D1 | `show` no payload de Err + string match — sem inspecionar Sum em Rust | Aprovada |
| D2 | `policy` é Ident no directive, não string | Aprovada |
| D3 | Default de `policy` é `exact` | Aprovada |
| D4 | Wrapper gera match em Result (Opção B) — distingue Ok de Err | Aprovada |
| D5 | `expects` sem Result é compile error, não warning | Aprovada |
| D6 | `partial` removido — só exact/prefix/contains | Aprovada |

## 8. Não-objetivos

- **`enum extends`.** Removido. Não há necessidade.
- **`KataError` no prelude.** Não há enum canônico de erros. O usuário
  declara seus próprios enums.
- **Coverage de suite.** Não verificar que todas as variantes são testadas.
- **Type bounds (`E extends X`).** Sem subtyping, sem constraints.
- **`expects` tipado (Variant/Exhaustive).** Substituído por string + policy.