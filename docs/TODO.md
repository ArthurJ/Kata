# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-10.

---

## Resolvido

#### Invariant: interfaces não chegam ao `match_score` como argumento

**Resolvido 2026-09-10.** Mapeamento completo dos caminhos onde
`Ty::Interface` pode aparecer como `.ty` de valor:

1. **`let x :: NUM := expr`** — ÚNICO caminho illegítimo. `let` é imutável,
   não há widening: `.ty = Ty::Interface` (concreto perdido). Agora é **erro
   de compilação** rejeitado no binder (`expr.rs`), antes de `.ty` ser
   atribuído.
2. **`var z :: NUM := expr`** — Widening: concreto como `.ty`, interface como
   `.declared_ty`. Interface nunca chega ao `match_score`. ✓
3. **`expr :: NUM`** (ascription) — Intencional, interceptada pelo Caminho 0
   (`iface_dispatch`) antes do `match_score`. Se o Caminho 0 não intercepta
   (func não é método da interface), `incompatible` é a resposta correta. ✓

O `match_score` (`kata-core/src/dispatch/mod.rs`) ganhou comentário
documentando o invariant: o fallback `incompatible` para `arg =
Ty::Interface` é comportamento correto, não bug. `debug_assert!` foi
considerado mas removido — `Ty::Interface` pode chegar legitimamente via
ascription intencional quando o Caminho 0 não intercepta.

**Commit:** `fix(typeck): proibir let com interface + documentar invariant do match_score`

---

## Pendentes

### 🟡 Médio

#### Trampoline do scheduler engole erros (interp)

`interp_trampoline` (`csp.rs:212-218`) captura qualquer `InterpError`
(exceto `Return`), imprime no stderr, e retorna `0`. O scheduler vê `0`
como sucesso — o exit code do processo não reflete o erro. Toda
validação de erro gracioso do interp tem sua mensagem impressa mas não
propagada como exit code não-zero.

**Impacto:** tests E2E não podem confiar em exit code para detectar
falhas do interp (precisam inspecionar stderr).

**Caminho:** o trampoline retorna `i64` (não `Result`), alinhado à FFI
do scheduler. Mudar para propagar erro requer reformular a interface
trampoline/scheduler ou usar um canal lateral (e.g. célula
`Mutex<Option<InterpError>>` no `InterpCtx`).

### 🟢 Baixo

#### `spawn!` no Windows é stub

`ipc.rs:155-161` — Implementar `spawn` no Windows. Ver
`docs/PRDs/PRD-portability-windows.md`.

#### Tree-shaking por instância de família polimórfica

O tree-shaking remove funções por **nome** — se uma função com overloads
polimórficas é alcançada, **todas** as instâncias expandidas sobrevivem,
mesmo as nunca chamadas com aquele tipo concreto. Ex: `mod :: Int
Instance("NonZero","Float") => Int` sobrevive mesmo se `mod` só é
chamado com `Instance("NonZero","Int")`.

**Impacto:** baixo. As overloads extras são inócuas em runtime (nunca
executadas), mas ocupam espaço no binário e tempo de compilação
(Cranelift compila cada uma). Só seria significativo com famílias
grandes (centenas de instâncias) e corpos pesados.

**Caminho:** propagar o tipo do argumento até o `collect_refs` do
tree-shaking para distinguir qual instância específica uma chamada
refere-se a, permitindo remover overloads não-usadas antes do codegen.

---

## Futuro

- **Tensor/SIMD** — design a definir.
- **Sistema de supressão de diagnóstico** — braço de match redundante
  agora é erro (decisão 8 do PRD-exaustividade-aninhada); `otherwise`
  inútil é isento, mas patterns não-otherwise redundantes com intenção
  documentada precisam de via de escape (`@allow redundant`?). Projetar
  sintaxe e escopo de supressão.
- **`select_arms_different_types`** — test placeholder em
  `kata-inference/tests/csp_typeck.rs:215`, depende de T0 unification.
  Corpo vazio, sem assertions.