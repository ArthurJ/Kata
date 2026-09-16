# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-10.

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

- **Tensor size parameters** — tensor base implementado (PRD-tensor.md ✅).
  Size parameters no type system + Z3 para shapes parcialmente conhecidos
  + refined pair (DotCompatible) na fronteira. Não exige novo runtime —
  trabalho de compilador (size params + Ty::Generic("Tensor") + parser
  + codegen) + FFI em kata-rt.
- **Sistema de supressão de diagnóstico** — braço de match redundante
  agora é erro (decisão 8 do PRD-exaustividade-aninhada); `otherwise`
  inútil é isento, mas patterns não-otherwise redundantes com intenção
  documentada precisam de via de escape (`@allow redundant`?). Projetar
  sintaxe e escopo de supressão.
- **`select_arms_different_types`** — test placeholder em
  `kata-inference/tests/csp_typeck.rs:215`, depende de T0 unification.
  Corpo vazio, sem assertions.