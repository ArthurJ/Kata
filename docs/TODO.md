# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-18 (refined_collections.kata resolvido: colisão de nomes na monomorfização de show genérico; exemplos de módulos removidos).

---

## Pendentes
### 🟡 Médio

#### `@test{expects}` com args Int causa SIGSEGV

`@test{desc: "...", expects: "...", args: (0)}` numa action que retorna
`Result::(Int, MeuErro)` crasha com misaligned pointer dereference (exit
139). O mesmo padrão com `args: ("texto")` (Text) e
`Result::(Text, MeuErro)` funciona normalmente.

**Reprodução:**
```kata
enum MeuErro
    ValidacaoFail

@test{desc: "valida", expects: "ValidacaoFail", policy: prefix, args: (0)}
action valida (x::Int) => Result::(Int, MeuErro)
    Result::Err MeuErro::ValidacaoFail
valida!(0)
```
→ SIGSEGV no `kata test`.

**Impacto:** testes `@test{expects}` com actions Int são impossíveis.
Workaround: usar args Text e retorno `Result::(Text, _)`.

**Caminho:** o crash acontece no codegen do wrapper de `expects` — provável
problema de SMI double-tagging ou layout de CaptureBox no path de
marshalling de Int args. Investigar `jit_tests` / `TestWrapper` no
codegen, comparar com o path de Text args (que funciona). Verificar se
o wrapper lê o payload de `Err` com o type_shape correto para Int.

#### `private_type.kata` — family_extension_invalid

`Internal implements NUM` estende a família `NonZero`, mas o predicado
da família não é válido para `Internal`. Erro de type system: o
predicado `NonZero` rejeita o tipo `Internal` na extensão.

**Impacto:** exemplo não roda. Pode ser um bug no exemplo (predicado
mal escrito) ou uma limitação real do family refinement.

**Caminho:** revisar o predicado `NonZero` e o tipo `Internal` no
exemplo. Se o exemplo está correto, investigar por que o family
checker rejeita a extensão.

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

#### Remover diretivas `@trace_*`

Diretivas `@trace_enter`, `@trace_args`, `@trace_exit`, `@trace_meta`,
`@trace_fn`, `@trace_act`, `@log_enter`, `@log_exit` são hooks de
tracing de depuração que inserem código no codegen. Avaliar se ainda
são usadas ou se foram substituídas por mecanismos melhores
(`@log{when: "enter"}`, etc.). Se obsoletas, remover do parser, AST,
codegen, e testes.

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

- **`select_arms_different_types`** — test placeholder em
  `kata-inference/tests/csp_typeck.rs:215`, depende de T0 unification.
  Corpo vazio, sem assertions.