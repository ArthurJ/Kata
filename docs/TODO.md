# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-08.

---

## Ativo

### Patterns aninhados (Maranget + SMT)

`docs/PRDs/PRD-exaustividade-aninhada.md` (5 fases, revisado
2026-08-30, **Emenda 1** pousada). O buraco NÃO é o parser — são os 3
CHECKERS (`check_exhaustiveness`, `check_clause_exhaustiveness`,
`pattern_covers`) que ignoram payload, mais um panic de aridade em
`lambda Some True:` desqualificado e um falso-positivo de redundância.
Bugs reproduzidos em `f64eff8`. **Emenda 1:** F1 encolha para Fundação
(oráculos + bound-check + parser + fall-through de codegen como no-op
provado); a cobertura recursiva ad-hoc foi REMOVIDA — F2 é motor antes
dos consumidores (`maranget.rs` puro com trait de ambiente, depois 3
consumidores, um por commit). Fase 3 Z3 na folha, Fase 4 refined
Int/Float na folha, Fase 5 Rational (const-eval de `rational <lit>` +
par (num, den) no Z3). Oráculos adversariais K medidos em `b5e2d9e`
(3 níveis, grade multi-param, arity-tuple).

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

#### Ascription em binding de `var`

`var l::Int := 0` rejeitado pelo parser (`parse.unexpected_token`, espera
`:=` após nome). Hoje só é possível travar o tipo via ascription no valor:
`var l := (0 :: Int)`. Adicionar `::Tipo` entre nome e `:=` no `var`
elimina o grouping extra e abre caminho para widening de interface
(`var l::NUM := 0`). Decisão: sintaxe de binding, não de valor — `::` ali
é anotação do binding, não operação sobre RHS.

#### `family_extension_invalid` + regra do órfão — gates de `implements`

`docs/PRDs/PRD-implements-gates.md` (2 fases). Dois gates estruturais
para `T implements IFACE` que hoje falham downstream com
`type.no_overload`:

1. **Regra do órfão** (`type.orphan_impl`): `T` e `IFACE` ambos externos
   ao módulo. Validação em `validate_orphan_rule` pós-merge, consultando
   `origins_of` em struct/enum/interface registries.
2. **Extensão de família inválida** (`type.family_extension_invalid`):
   `T implements IFACE` estende família cujos predicados não são
   satisfeitos para `T`. `RefinedDeclInfo` ganha `extension_impl` para
   marcar instâncias criadas por `extend_families_for_implementors`;
   inference mapeia `NoOverload` → `FamilyExtensionInvalid`.

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