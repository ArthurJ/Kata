# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-22.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### `filter_exports` não preserva dependências transitivas de funções

**Estado:** Se módulo A exporta `fn1` cujo corpo referencia `fn2` (não
exportada), e módulo B importa `A.(fn1)`, o `infer_module` falha com
`unbound_name` para `fn2`. O `filter_exports` remove `fn2` do
`ResolvedModule` filtrado, mas o corpo de `fn1` referencia `fn2`. O
`resolved_unfiltered` tem `fn2`, mas `merge_imports` usa o filtrado.

**Impacto:** Médio. Sub-módulos que importam de outros sub-módulos e
re-exportam funções que dependem de imports internos falham. Workaround:
exportar todas as funções referenciadas por funções exportadas, ou usar
funções autocontidas (sem dependências internas).

**Análise (2026-08-22):**

O fluxo do bug:
1. `load_imports` (module_loader.rs:250) chama `filter_exports` → produz
   `resolved_filtered` (sem `fn2`) e `resolved_unfiltered` (com `fn2`).
2. `merge_imports` (merge_imports.rs:20) mescla `resolved_filtered` no
   importador B. `resolved_unfiltered` só é usado por
   `evaluate_imported_constants` (imports.rs:140) — inference de constants,
   não de funções.
3. `infer_module` (infer/mod.rs:97) clona `resolved.functions` — agora
   inclui `fn1` (de A, filtrada) mas não `fn2`. No passo 3 (mod.rs:309),
   processa cada `FunctionDef` via `infer_named_function`. Ao inferir o
   corpo de `fn1`, a referência a `fn2` não está no `dispatch_table` nem
   no `type_env` → `UnboundName`.

Por que o inference re-processa funções importadas: `resolved.functions`
contém `FunctionDef` (AST com cláusulas lambda, não TAST). O `infer_module`
tipa todas as funções indiscriminadamente — por design, o codegen precisa
da TAST de todas as funções (incluindo importadas) para gerar código
Cranelift. O `resolved_unfiltered` existe para dar acesso a helpers
internos, mas só é usado no pipeline recursivo de constants, não no de
funções.

O que `filter_exports` já faz: fechamento transitivo para **tipos**
(mod_loader.rs:489-526) — tipo exportado → impls → interfaces → métodos
→ supertraits (fixpoint).

**Solução proposta:** estender o closure com dependências de corpos de
funções — análogo ao fixpoint de supertraits que já existe:
1. Coletar `module_names` — nomes de signatures/functions/actions
   definidas no módulo (via `Module` AST, que `filter_exports` recebe).
2. Para cada função no `closure`, percorrer as cláusulas lambda (corpo)
   coletando `Expr::Ident(name)` onde `name ∈ module_names`.
3. Adicionar esses nomes ao `closure`. Repetir até fixpoint (`fn2` pode
   referenciar `fn3`, etc.).
4. O filtro de `functions`/`signatures` por `closure.contains(&s.name)`
   (linhas 532-543) já faz o resto.

A peça que falta é um walker de AST que percorra `FunctionDef` (cláusulas
→ body `Expr`) coletando `Ident`. Não há visitor existente no codebase
para isso — a AST de `Expr` é recursiva, é trabalho mecânico.

Risco principal: o walker precisa cobrir todos os nós de `Expr` que podem
conter `Ident` (call, pipe desugared, let, match arms, lambda body, etc.).
Se perder um caso, o fixpoint não converge e `fn2` ainda é filtrada — mas
o sintoma é o mesmo erro atual, não regresso silencioso.

Questão arquitetural: a raiz mais profunda é que funções importadas são
re-inferidas pelo importador, não no módulo de origem. Seria "mais correto"
inferir no módulo de origem (com `resolved_unfiltered`) e importar a TAST
já tipada — eliminando a necessidade do fechamento transitivo. Mas isso
seria refatoração grande (o pipeline não tem noção de "TAST importada").
O fechamento transitivo é pragmático, resolve o bug, e é consistente com
o que já existe para tipos.

---

### Tree-shaking por instância de família polimórfica

**Estado:** O tree-shaking remove funções por **nome** — se uma função
com overloads polimórficas é alcançada, **todas** as instâncias expandidas
sobrevivem, mesmo as nunca chamadas com aquele tipo concreto. Ex:
`mod :: Int Instance("NonZero","Float") => Int` sobrevive mesmo se `mod`
só é chamado com `Instance("NonZero","Int")`.

**Impacto:** Baixo. A expansão gera N overloads concretas por família; o
tree-shaking por nome preserva todas as N quando o nome é alcançado. As
overloads extras são inócuas em runtime (nunca executadas), mas ocupam
espaço no binário e tempo de compilação (Cranelift compila cada uma). Para
famílias pequenas (3 instâncias) e poucas funções, o custo é negligenciável.
Só seria significativo com famílias grandes (centenas de instâncias) e
corpos pesados — cenário extremo e improvável na prática.

**Quando surgir caso de uso real:** propagar o tipo do argumento até o
`collect_refs` do tree-shaking para distinguir qual instância específica
uma chamada refere-se a, permitindo remover overloads não-usadas antes
do codegen.

---

### Import implícito de stdlib via `mod.kata` (substituir `load_prelude` + `merge_two`)

**Estado:** O prelude (`stdlib/core.kata`) é carregado via `include_str!` em
`prelude_sigs.rs` e injetado no módulo do usuário via `merge_two`. Isto
bypassa o sistema de módulos — `filter_exports` não é aplicado, então tudo
no core é visível para o usuário (incluindo funções internas como
`bi_div`, `f_div`, `rat_div` de `core_internals`).

O sistema de módulos já suporta `mod.kata` (gateway com `export` seletivo)
e `filter_exports` com fechamento transitivo (exportar `Int` traz
automaticamente `NUM`, `EQ`, `+`, `show`, etc.). O que falta:

1. Criar `stdlib/mod.kata` com `import core` + `export` dos símbolos públicos
   (tipos + funções standalone). `core_internals` não é re-exportado.
2. Substituir `load_prelude() + merge_two()` por `ModuleLoader::load_imports()`
   com import implícito de `stdlib`. Aplicar em 3 callers:
   - `pipeline.rs` (linha 299+318)
   - `repl/mod.rs` (REPL)
   - `kata-lsp/src/analysis.rs` (linha 62+64)
3. Remover `prelude_sigs.rs` (substituído pelo ModuleLoader).
4. Decidir: stdlib embedded no binário (`include_str!`) vs filesystem em
   runtime. Hoje o ModuleLoader lê do filesystem; o prelude é embedded.
5. O `module_loader.rs` linha 337 (`is_core` bypass) é removido — o core
   passa a ter imports reais (`import core_internals`).

**Impacto:** Médio-alto. Resolve a visibilidade de funções internas,
unifica prelude e módulos no mesmo caminho de código, e prepara para
múltiplos módulos stdlib (math, complex, stdio) com visibilidade controlada.

---

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD
- `@restart` (retry policy para Actions)
- Renomear `@trace` de volta para `@log` (diretiva de telemetria)