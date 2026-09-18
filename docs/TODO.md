# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-18.
Itens resolvidos devem ser removidos — o histórico vive no git.

---

## Pendentes
### 🟡 Médio

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

- **Sintaxe de action call: posicional vs nomeado** — discutir remover a
  forma posicional `f!(x, y)` e manter apenas `f!{a: x, b: y}` (dict
  nomeado). Motivação: diferenciação sintática entre funções (`f x y`,
  curried) e actions (`f!{a: x, b: y}`, nomeado) — hoje ambas usam
  parênteses, diferindo só pelo ``. A mudança eliminaria a ambiguidade
  Grouping/Tuple no path de action calls (4 pontos de normalização
  duplicados: action_call.rs, action_infer.rs, csp_concurrency.rs,
  log_builtins.rs). Contras: migração de ~416 calls posicionais e
  verbosidade para 1 arg (`echo!{msg: x}` vs `echo!(x)`). Alternativa:
  açúcar sintático para aridade 0 (`f!()`) e 1 (`f!(x)` ≡ `f!{param: x}`
  usando o primeiro param da action), mantendo `!{` obrigatório para 2+.
  Decisão pendente — precisa avaliar ergonomia vs consistência.

- **Centralizar normalização Grouping→Tuple** — mesmo sem decisão sobre a
  sintaxe de action call, a normalização `Grouping → Tuple de 1` está
  duplicada em 4 pontos (action_call.rs:168, action_infer.rs:141,
  csp_concurrency.rs:134, log_builtins.rs:153). Extrair para função única
  e chamar de todos os sites, evitando divergência silenciosa (o bug do
  SIGSEGV foi causado por exatamente esse tipo de divergência).

- **`select_arms_different_types`** — test placeholder em
  `kata-inference/tests/csp_typeck.rs:215`, depende de T0 unification.
  Corpo vazio, sem assertions.