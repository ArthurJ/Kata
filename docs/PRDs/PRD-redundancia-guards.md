# PRD — Redundância de Cláusulas com Guards

**Status:** Rascunho
**Data:** 2026-08-23
**Depende de:** `check_redundant_clauses` (redundancy.rs) ✅, `check_guard_completeness` (guard_completeness.rs) ✅, `Z3Translator` (guard_completeness.rs) ✅
**Não depende de:** Nenhum PRD pendente

## 1. Objetivo

Estender `check_redundant_clauses` para detectar redundância quando a
cláusula anterior (M) ou a cláusula posterior (N) tem guards. Hoje,
cláusulas com guards são totalmente ignoradas pela verificação de
redundância — puladas sem verificação.

## 2. Motivação

### 2.1. Cláusulas redundantes com guards não são detectadas

Hoje `check_redundant_clauses` pula cláusulas com guards:

```rust
// Cláusulas com guards não são redundantes por pattern alone —
// a condição do guard pode diferenciá-las.
if !clause.guards.is_empty() {
    continue;
}
```

Isso é conservador mas impreciso. Considere:

```kata
lambda x:
    x:                    # M — pattern Ident cobre tudo, sem guards
        > x 0: A
        <= x 0: B
    x: C                  # N — redundante: M cobre tudo e sempre dispara
```

M cobre todo input (Ident) e seus guards são exaustivos (`x > 0` ∨ `x <= 0`
é tautologia). N é inalcançável — M sempre dispara antes. Hoje não detectado.

### 2.2. Sobreposição parcial de guards

Caso mais sutil:

```kata
lambda x:
    > x 0: A              # M — guard x > 0 (não é tautologia)
    > x 5: B              # N — redundante: x > 5 implica x > 0
```

M não é tautológico (falha para x ≤ 0), mas para qualquer input que
chegue a N (x > 5), M já disparou (x > 0 inclui x > 5). N é inalcançável.

### 2.3. Caso onde NÃO é redundante

```kata
lambda x:
    > x 0: A              # M — guard x > 0
    > x 0: B              # N — mesmo guard, mesmo pattern
```

Aqui M e N têm o mesmo guard. M dispara primeiro para x > 0, mas N
**não é redundante** para x ≤ 0 — ambos falham, e a próxima cláusula
(ou fallback) trata x ≤ 0. N é alcançável quando M falha (x ≤ 0) **e**
N falha (x ≤ 0) — ou seja, N nunca dispara, mas não é porque M a cobre,
é porque a condição nunca se aplica. Espera — na verdade, se N tem o
mesmo guard de M, e M vem antes, N é inalcançável: qualquer input que
satisfaça o guard de N também satisfaz o guard de M (são idênticos),
e M dispara primeiro. N é redundante sim.

O caso onde NÃO é redundante é:

```kata
lambda x:
    > x 0: A              # M — guard x > 0
    <= x 5: B             # N — guard x <= 5
```

Para x = 3: M dispara (x > 0). N também satisfaria (x <= 5), mas M vem
antes. Para x = -1: M falha (x > 0 é false). N dispara (x <= 5). N é
alcançável. **N não é redundante.**

## 3. Design

### 3.1. Duas fases

A extensão tem duas fases com complexidade diferente:

**Fase 1 — Tautologia dos guards de M (sem Z3 novo):**

Se `patterns_cover(M, N)` e M tem guards, verificar se os guards de M
são tautologia (sempre disparam). Se sim, M sempre dispara sobre os
patterns que cobre, e N é inalcançável.

Usa `check_guard_completeness` que já existe — a função prova tautologia
da disjunção dos guards. Se retorna `Ok(())` (UNSAT para a negação), os
guards são tautologia.

**Fase 2 — Implicação entre guards (Z3 novo):**

Se M tem guards não-tautológicos, verificar se os guards de N implicam
os guards de M. Se todo input que satisfaz os guards de N também
satisfaz algum guard de M, e M vem antes, N é inalcançável.

Para provar `guards_N ⟹ guards_M`, verificar se
`guards_N ∧ ¬guards_M` é insatisfazível (UNSAT). Se UNSAT, a implicação
vale e N é redundante.

Isso exige uma nova função em `guard_completeness.rs`:
`check_guard_implication(guards_n, guards_m, span)` que constrói a
fórmula `guards_N ∧ ¬guards_M` e checa satisfatibilidade.

O tradutor `Z3Translator` já existe e é reutilizado. A diferença é que
em vez de negar a disjunção (tautologia), negamos a disjunção de M e
conjugamos com a disjunção de N (implicação).

### 3.2. Quando N tem guards

O raciocínio muda dependendo de quem tem guards:

**N sem guards, M sem guards:** caso atual. `patterns_cover(M, N)` → redundante.

**N sem guards, M com guards:** M pode falhar para algum input que N
cobre. N **não é redundante** — a menos que os guards de M sejam
tautologia (Fase 1). Se os guards de M são tautologia, M sempre dispara
sobre os patterns que cobre, e N (sem guards) é inalcançável.

**N com guards, M sem guards:** M sempre dispara sobre os patterns que
cobre (sem guards = sempre). Se `patterns_cover(M, N)`, M captura todo
input que N casaria antes de N. N é redundante — independente dos
guards de N.

**N com guards, M com guards:** M dispara se `patterns_cover(M, N)` e
algum guard de M satisfaz. N é inalcançável se, para todo input que
casa os patterns de N e satisfaz algum guard de N, M também casa e
satisfaz algum guard de M. Isso é a implicação da Fase 2:
`guards_N ⟹ guards_M` (assumindo `patterns_cover(M, N)`).

### 3.3. Estrutura da verificação

```
para cada cláusula N (N > 0):
    para cada cláusula M < N:
        se não patterns_cover(M, → próxima M

        match (M tem guards, N tem guards):
            (false, false) → N redundante (caso atual)
            (false, true)  → N redundante (M sempre dispara, guards de N irrelevantes)
            (true,  false) → Fase 1: check_guard_completeness(guards de M)
                             se Ok (tautologia) → N redundante
                             senão → não redundante (M pode falhar)
            (true,  true)  → Fase 2: check_guard_implication(guards de N, guards de M)
                             se Ok (implicação provada) → N redundante
                             senão → não redundante
```

### 3.4. `check_guard_implication` — nova função

Nova função em `guard_completeness.rs`:

```rust
/// Verifica se os guards de N implicam os guards de M.
///
/// Prova: guards_N ⟹ guards_M
/// Ou seja: guards_N ∧ ¬guards_M é insatisfazível.
///
/// Se UNSAT → implicação provada (N é redundante).
/// Se SAT → contra-exemplo existe (N é alcançável).
/// Se UNKNOWN → limite atingido, não concluir redundância (conservador).
pub(crate) fn check_guard_implication(
    guards_n: &[TypedGuardClause],
    guards_m: &[TypedGuardClause],
    span: &Span,
) -> GuardResult
```

Implementação:
1. Traduzir guards de N para disjunção Z3: `disj_N = guard_n1 ∨ ... ∨ guard_nK`
2. Traduzir guards de M para disjunção Z3: `disj_M = guard_m1 ∨ ... ∨ guard_mJ`
3. Asserção: `disj_N ∧ ¬disj_M`
4. `solver.check()`:
   - `Unsat` → implicação provada, `Ok(())`
   - `Sat` → contra-exemplo, `Err(NonExhaustiveMatch)` — não, espera.
     Aqui não é NonExhaustiveMatch. Se há contra-exemplo, N **não é
     redundante**. Retornar `Err` significaria erro, mas não é — é
     "não podemos provar redundância". Precisamos de um tipo de retorno
     que distinga "prova de redundância" de "não provada".
   - `Unknown` → limite, não provar (conservador).

### 3.5. Tipo de retorno

`check_guard_completeness` retorna `Result<(), MiddleError>` onde
`Err` é um erro de compilação (NonExhaustiveMatch ou MissingOtherwise).
Para `check_guard_implication`, o resultado é diferente:

- **Implicação provada (UNSAT)** → N é redundante → reportar `RedundantClause`
- **Implicação refutada (SAT)** → N **não é redundante** → não reportar nada
- **Unknown** → não decidido → não reportar nada (conservador)

SAT e Unknown significam "não provar redundância" — não são erros. A
função deve retornar um `bool` (ou `Option<CounterExample>`), não um
`Result`:

```rust
/// Retorna `true` se a implicação foi provada (N é redundante).
/// Retorna `false` se refutada ou não decidida.
pub(crate) fn check_guard_implication(
    guards_n: &[TypedGuardClause],
    guards_m: &[TypedGuardClause],
    span: &Span,
) -> bool
```

Isso mantém o som conservador: `false` = "não provamos redundância, assume
não-redundante". `true` = "prova sólida de redundância".

### 3.6. Ordem de verificação

A verificação de redundância roda **antes** da verificação de
exaustividade em `function_infer.rs`:

```rust
// linha 134
crate::redundancy::check_redundant_clauses(&typed_clauses)?;
// linha 141
check_clause_exhaustiveness(&typed_clauses, param_types, ctx, ...)?;
```

Isso é correto: uma cláusula redundante deve ser reportada antes de
verificar exaustividade (que pode ser afetada pela remoção mental da
cláusula redundante). A ordem não muda.

### 3.7. Cláusulas com `with` bindings

`with_bindings` são açúcar injetado nas cláusulas que referenciam o nome.
Não afeta a análise de redundância — os bindings são resolvimentos de
nomes, não condições. Se M tem `with` bindings e N não tem, a
verificação de redundância continua válida: o que importa é patterns
e guards.

### 3.8. Interação com `otherwise`

Se M tem `otherwise` (guard com `condition: None`), seus guards são
trivialmente tautologia — `otherwise` sempre dispara. A Fase 1
(`check_guard_completeness`) retorna `Ok(())` imediatamente, sem
chamar Z3. Correto: M com `otherwise` sempre dispara sobre os patterns
que cobre.

Se N tem `otherwise`, seus guards são trivialmente tautologia. Na Fase 2
(`check_guard_implication`), `disj_N` inclui `True` (do `otherwise`),
então `disj_N` é `True`. A fórmula fica `True ∧ ¬disj_M` = `¬disj_M`.
Se `disj_M` é tautologia, `¬disj_M` é UNSAT → implicação provada → N
redundante. Se `disj_M` não é tautologia, `¬disj_M` é SAT → N não
redundante. Correto: se M não cobre tudo, o `otherwise` de N é
alcançável.

### 3.9. Quando Z3 não decide (Unknown)

Z3 retorna `unknown` para fórmulas complexas ou não-lineares. Em ambos
os casos (Fase 1 e Fase 2), Unknown é tratado conservadoramente:
- Fase 1: `check_guard_completeness` retorna `Err(MissingOtherwise)`.
  Mas isso é um erro de compilação, não "não provar redundância".
  **Problema**: usar `check_guard_completeness` diretamente na Fase 1
  reportaria `MissingOtherwise` quando Z3 não decide — mas isso não é
  um problema de exaustividade, é um problema de redundância. Precisamos
  tratar Unknown diferentemente.

**Solução para Fase 1:** Em vez de chamar `check_guard_completeness`
diretamente, a Fase 1 precisa de um retorno trivalorado:
`Tautology/CounterExample/Unknown`. Se `Tautology` → redundante.
Se `CounterExample` ou `Unknown` → não provar redundância (conservador).

Isso sugere que `check_guard_completeness` deve ser refatorada para
expor o resultado trivalorado internamente, ou a Fase 1 replica a
lógica de chamada Z3 com tratamento diferente de Unknown.

**Alternativa:** Extrair uma função `prove_tautology(guards, span)
-> Ternary` em `guard_completeness.rs` que retorna o resultado cru do
Z3 (`Unsat/Sat/Unknown`), e tanto `check_guard_completeness` (para
exaustividade) quanto a Fase 1 (para redundância) usam essa função
com tratamento diferente de Unknown.

```rust
enum Ternary {
    Proven,       // UNSAT — tautologia provada
    Refuted,      // SAT — contra-exemplo existe
    Unknown,      // limite
}

fn prove_tautology(guards: &[TypedGuardClause]) -> Ternary
```

- `check_guard_completeness` (exaustividade): `Proven` → Ok, `Refuted`
  → Err(NonExhaustiveMatch), `Unknown` → Err(MissingOtherwise).
- Fase 1 (redundância): `Proven` → redundante, `Refuted`/`Unknown` →
  não provar.
- Fase 2 (implicação): usa `prove_implication` que segue o mesmo padrão.

### 3.10. Refatoração de `guard_completeness.rs`

Extrair a lógica Z3 em duas funções internas:

```rust
fn prove_tautology(guards: &[TypedGuardClause]) -> Ternary
fn prove_implication(guards_n: &[TypedGuardClause], guards_m: &[TypedGuardClause]) -> Ternary
```

E expor wrappers para os dois consumidores:

```rust
/// Exaustividade (uso atual): tautologia da disjunção.
pub(crate) fn check_guard_completeness(guards, span) -> Result<(), MiddleError>

/// Redundância (novo uso): implicação guards_N ⟹ guards_M.
pub(crate) fn check_guard_implication(guards_n, guards_m, span) -> bool
```

`check_guard_implication` retorna `bool`:
`Proven → true`, `Refuted/Unknown → false`.

## 4. Estruturas afetadas

### 4.1. `check_redundant_clauses` (redundancy.rs)

Hoje pula cláusulas com guards. Nova versão:

```rust
pub(crate) fn check_redundant_clauses(clauses: &[TypedLambdaClause]) -> InferResult<()> {
    for (i, clause_n) in clauses.iter().enumerate().skip(1) {
        for (j, clause_m) in clauses[..i].iter().enumerate() {
            if !patterns_cover(&m_patterns, &n_patterns) {
                continue;
            }

            let m_has_guards = !clause_m.guards.is_empty();
            let n_has_guards = !clause_n.guards.is_empty();

            match (m_has_guards, n_has_guards) {
                (false, false) => {
                    // Caso atual: M sem guards sempre dispara.
                    return Err(RedundantClause { span: clause_n.body.span });
                }
                (false, true) => {
                    // M sem guards sempre dispara sobre os patterns.
                    // Guards de N não importam — M captura antes.
                    return Err(RedundantClause { span: clause_n.body.span });
                }
                (true, false) => {
                    // Fase 1: guards de M são tautologia?
                    if check_guard_completeness(&clause_m.guards, &span)? {
                        // Hmm, check_guard_completeness retorna Result.
                        // Precisa do wrapper trivalorado.
                    }
                    // Se não provou tautologia, M pode falhar → N não redundante.
                }
                (true, true) => {
                    // Fase 2: guards_N ⟹ guards_M?
                    if check_guard_implication(&clause_n.guards, &clause_m.guards, &span) {
                        return Err(RedundantClause { span: clause_n.body.span });
                    }
                }
            }
        }
    }
    Ok(())
}
```

### 4.2. `guard_completeness.rs`

Adicionar:
- `enum Ternary { Proven, Refuted, Unknown }`
- `fn prove_tautology(guards) -> Ternary` — extraída de `check_guard_completeness`
- `fn prove_implication(guards_n, guards_m) -> Ternary` — nova
- `pub(crate) fn check_guard_implication(guards_n, guards_m, span) -> bool`

Refatorar `check_guard_completeness` para usar `prove_tautology`.

### 4.3. `MiddleError` (kata-diagnostics)

Sem mudanças. `RedundantClause` já existe com `span` — é reutilizado.
O `hint` não existe em `RedundantClause` hoje. Considerar adicionar:

```rust
RedundantClause {
    #[label("cláusula redundante")]
    span: MietteSpan,
    #[help]
    hint: Option<String>,  // novo
}
```

Com hint: `"cláusula sombreada por cláusula anterior. Os guards da \
cláusula anterior sempre disparam para os mesmos patterns"` (Fase 1)
ou `"cláusula sombreada: os guards desta cláusula implicam os guards \
da cláusula anterior"` (Fase 2).

**Decisão:** adicionar `hint` a `RedundantClause` é uma mudança em
`kata-diagnostics` que afeta todos os callers de `RedundantClause`. Como
só há um caller (`check_redundant_clauses`), o impacto é mínimo.

### 4.4. `function_infer.rs`

Sem mudanças. `check_redundant_clauses` mantém a mesma assinatura.

## 5. Mensagens de erro

### 5.1. RedundantClause (caso atual, sem guards)

```
erro: "cláusula redundante: sombreada por cláusula anterior"
span: linha da cláusula N
hint: "a cláusula anterior já cobre todos os patterns desta cláusula"
```

### 5.2. RedundantClause (Fase 1 — guards de M são tautologia)

```
erro: "cláusula redundante: sombreada por cláusula anterior"
span: linha da cláusula N
hint: "a cláusula anterior cobre os mesmos patterns e seus guards \
       sempre disparam (são exaustivos)"
```

### 5.3. RedundantClause (Fase 2 — guards de N implicam guards de M)

```
erro: "cláusula redundante: sombreada por cláusula anterior"
span: linha da cláusula N
hint: "qualquer input que satisfaça os guards desta cláusula também \
       satisfaz os guards da cláusula anterior, que dispara primeiro"
```

## 6. Testes

### 6.1. Casos da Fase 1 (tautologia dos guards de M)

| Teste | Entrada | Esperado |
|-------|---------|----------|
| M com guards tautológicos, N sem guards | `lambda x:\n  x:\n    > x 0: A\n    <= x 0: B\n  x: C` | `RedundantClause` |
| M com guards não-tautológicos, N sem guards | `lambda x:\n  > x 0: A\n  x: B` | Ok (M pode falhar para x ≤ 0) |
| M com otherwise, N sem guards | `lambda x:\n  x:\n    > x 0: A\n    otherwise: B\n  x: C` | `RedundantClause` (otherwise = tautologia) |

### 6.2. Casos da Fase 2 (implicação entre guards)

| Teste | Entrada | Esperado |
|-------|---------|----------|
| Guards de N implicam guards de M | `lambda x:\n  > x 0: A\n  > x 5: B` | `RedundantClause` (x > 5 ⟹ x > 0) |
| Guards de N não implicam guards de M | `lambda x:\n  > x 0: A\n  <= x 5: B` | Ok (x ≤ 5 não implica x > 0) |
| Guards idênticos | `lambda x:\n  > x 0: A\n  > x 0: B` | `RedundantClause` (x > 0 ⟹ x > 0) |
| Guards disjuntos | `lambda x:\n  > x 10: A\n  < x 0: B` | Ok (x < 0 não implica x > 10) |

### 6.3. Casos onde N tem guards e M não

| Teste | Entrada | Esperado |
|-------|---------|----------|
| M sem guards, N com guards | `lambda x:\n  x: A\n  > x 0: B` | `RedundantClause` (M sempre dispara) |
| M sem guards (não cobre), N com guards | `lambda True: A\n  > x 0: B` | Ok (pattern diferente — patterns_cover falha antes) |

### 6.4. Não-regressão (caso atual)

| Teste | Entrada | Esperado |
|-------|---------|----------|
| Duas cláusulas sem guards, M cobre N | `lambda x: A\n  x: B` | `RedundantClause` (caso atual) |
| Duas cláusulas sem guards, M não cobre N | `lambda True: A\n  False: B` | Ok |
| Cláusulas com guards hoje não verificadas | (qualquer caso que passava antes) | Continua Ok (não-regressão) |

### 6.5. Snapshot

Snapshots insta em `tests/tast_snapshot.rs` para casos de redundância
com guards, seguindo o padrão existente.

## 7. Passos de implementação

1. **Refatorar `guard_completeness.rs`** — extrair `prove_tautology`
   que retorna `Ternary`. Refatorar `check_guard_completeness` para
   usar `prove_tautology`.

2. **Adicionar `prove_implication`** em `guard_completeness.rs` —
   constrói `disj_N ∧ ¬disj_M`, retorna `Ternary`.

3. **Adicionar `check_guard_implication`** em `guard_completeness.rs` —
   wrapper `pub(crate)` que retorna `bool` (`Proven → true`).

4. **Adicionar `hint` a `RedundantClause`** em `middleend.rs` (kata-diagnostics).

5. **Reescrever `check_redundant_clauses`** em `redundancy.rs` —
   implementar a matriz de casos da §3.3.

6. **Testes** — casos de §6.1 a §6.4 em
   `crates/kata-inference/tests/lambda_match_inference.rs` (ou arquivo
   novo). Snapshots em `tast_snapshot.rs`.

7. `cargo test --workspace` — zero regressão.

8. `graphify update .`

## 8. Fora do escopo

- **Refinement propagation (path conditions):** usar Z3 para provar
  implicações de tipos refinados sobre valores não-literais. Item
  separado no TODO.md sob "Futuro".

- **Patterns aninhados (Maranget + SMT):** verificar cobertura de
  patterns aninhados com SMT. Item separado no TODO.md.

- **Inlinamento de funções em guards:** a tradução para Z3 hoje trata
  chamadas de função como variáveis opacas. Inlinar funções puras
  conhecidas nos guards permitiria provas mais precisas, mas é item
  separado (documentado no PRD-exaustividade §9).

- **Contraposição de guards:** se `guards_N ⟹ guards_M` é provada, N é
  redundante. Mas se a implicação é refutada (SAT), o contra-exemplo
  encontrado pelo Z3 poderia ser reportado no hint. Não essencial —
  a resposta binária (redundante/não-redundante) é suficiente.