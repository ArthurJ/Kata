# PRD — Sistema de Módulos: import/export no pipeline de compilação

**Status:** 📄 Rascunho
**Data:** 2026-07-24
**Depende de:** Scaffolding existente (parser, ModuleLoader, AST)
**Desbloqueia:** Migração de `test_imports.kata` + `mock_math.kata` para `examples/`

## 1. Problema

O manual (§3) descreve um sistema de módulos completo: `import`, `export`,
path resolution, cache, cycle detection. A implementação atual é
**scaffolding desconectado**:

### 1.1. O que existe

- **Parser** (`kata-parser/src/imports.rs`): parseia `import mod.submod`,
  `import mod as alias`, `import MOD.(items)`, `export item1 item2`,
  `export MOD.(items)`. Funciona — produz `Item::ImportDecl` e
  `Item::ExportDecl` corretamente.
- **AST** (`kata-ast/src/item.rs`): `ImportDecl { path, alias, items }`
  e `ExportDecl { items: Vec<ExportItem> }` estão definidos.
- **ModuleLoader** (`kata-resolution/src/module_loader.rs`): estrutura
  completa com cache, cycle detection, search paths, lex→parse→resolve.
  Tem testes próprios que passam. Marcado `#![allow(dead_code)]`.
- **Manual** (§3.1–3.5): descreve visibilidade, exportação, importação,
  prelude, path resolution, prevenção de ciclos.

### 1.2. O que falta

1. **`resolve()` ignora `ImportDecl` e `ExportDecl`.** O `kata-resolution`
   não tem nenhum código que processa esses itens. O comentário em
   `infer/mod.rs:271` diz "Já processado no resolution" — mas é falso.
   Nenhum crate processa imports/exports.

2. **`ModuleLoader` não é chamado pelo driver.** `run_pipeline` em
   `kata-driver/src/main.rs` faz: lex → parse → `resolve(&module)` →
   `merge_resolved(prelude, user)` → infer. Nenhum passo carrega módulos
   importados. O `ModuleLoader` existe mas é dead code.

3. **`merge_resolved` só mergeia prelude + user.** Não há merge de
   módulos importados. O `ResolvedModule` do módulo importado não é
   trazido para o escopo do importador.

4. **Acesso qualificado (`mod.fn`) não é module access.** O parser
   parseia `mock_math.dobrar` como `Expr::DotAccess { expr: Ident("mock_math"),
   index: Field("dobrar") }`. O inference trata `DotAccess` como field
   access em struct ou index access em tupla — não como acesso a função
   de módulo importado. Não há path 5 em `infer_expr` para
   `Ident("mod")` → módulo → `Field("fn")` → função exportada.

5. **`export` não filtra visibilidade.** Hoje tudo definido num módulo
   é visível dentro do módulo. O `export` deveria restringir o que é
   visível para importadores, mas como nada é importado, o conceito
   não existe na prática.

6. **Sintaxe legada nos exemplos.** `test_imports.kata` usa `$()` (não
   existe mais) e `format` (não está no prelude). Precisa ser
   modernizado durante a migração.

### 1.3. Quadro do gap

| Componente | Manual diz | Implementação faz | Gap |
|---|---|---|---|
| Parser import | `import mod.sub`, `as alias`, `MOD.(items)` | ✅ Parseia tudo | Nenhum |
| Parser export | `export item1 item2`, `MOD.(items)` | ✅ Parseia tudo | Nenhum |
| ModuleLoader | Cache + cycle detection + path resolution | ✅ Implementado + testado | Dead code — não chamado |
| resolve() | "resolve imports" (§2.2 linha 340) | ❌ Ignora ImportDecl/ExportDecl | Total |
| merge_resolved | N/A (não está no manual) | Só prelude + user | Não mergeia módulos importados |
| Driver pipeline | "module load" entre parse e resolution (§2.2) | ❌ Não chama ModuleLoader | Total |
| Access qualificado | `mod.fn` implícito no import de módulo inteiro | ❌ DotAccess só struct/tupla | Total |
| export filtering | Só exported é importável | ❌ Nada filtra | Total |
| Prelude em sub-módulos | "Módulos secundários não importam core magicamente" (§3.3) | resolve() não injeta prelude em sub-módulos | Parcial — ModuleLoader chama resolve() que não injeta prelude |

## 2. Objetivo

Integrar o `ModuleLoader` no pipeline de compilação para que `import`/`export`
funcionem de ponta a ponta, permitindo que `test_imports.kata` + `mock_math.kata`
sejam migrados para `examples/` como testes da funcionalidade.

## 3. Design

### 3.1. Visão geral do fluxo

```
Driver::run_pipeline(source)
    │
    ▼
  lex → parse → Module { items: [ImportDecl, ..., ExportDecl, ...] }
    │
    ▼
  inject_implicit_core_import(&mut module)
    │  Insere ImportDecl { path: ["core"] } virtual no topo
    │  (entrypoints e sub-módulos — mesmo mecanismo)
    │
    ▼
  ModuleLoader::load_imports(&module, search_paths)
    │  Para cada ImportDecl (incluindo o implícito de core):
    │    1. resolve_path → filesystem path
    │    2. load_path → lex → parse → inject_implicit_core →
    │       load_imports (recursivo) → resolve() → ResolvedModule
    │    3. Filtra por export: só itens em ExportDecl são visíveis
    │    4. Retorna Vec<ImportedModule>
    │
    ▼
  resolve(&module) → ResolvedModule (user)
    │
    ▼
  merge_resolved(user, &imports)
    │  merge: imports (incluindo prelude) + user
    │  Prelude (core) importado como Selective com todos os exported
    │  → itens no escopo direto (echo!, Int, +, etc.)
    │  Import seletivo explícito: só itens nomeados em `MOD.(items)`
    │  Import de módulo inteiro: itens exportados sob prefixo `mod.`
    │
    ▼
  infer_module → TypedModule
```

### 3.2. Fase 1: ModuleLoader no pipeline do driver

**Mudança mínima:** o driver chama `ModuleLoader` entre parse e resolve.

```rust
// kata-driver/src/main.rs — run_pipeline

// 2. Parse
let mut module = parse(tokens)?;

// 2a. Import implícito de core (prelude)
inject_implicit_core_import(&mut module);

// 2b. Carregar módulos importados (incluindo core)
let entry_dir = Path::new(file).parent().unwrap_or(Path::new("."));
let search_paths = vec![
    entry_dir.to_path_buf(),           // diretório do arquivo atual
    stdlib_dir.to_path_buf(),          // stdlib/ (onde está core.kata)
];
let mut loader = ModuleLoader::new(search_paths);
let imports = loader.load_imports(&module)?;
// imports: Vec<ImportedModule> — primeiro é core (prelude)

// 3. Resolve (user + imports, sem prelude especial)
let user = resolve(&module)?;
let resolved = merge_resolved(user, &imports);
```

**Search paths:**
1. Diretório do arquivo sendo compilado (entrypoint).
2. Diretório `stdlib/` relativo ao Cargo manifest (para `import core`
   explícito, embora o prelude já seja injetado automaticamente).

**Decisão: `ModuleLoader::load_imports` é um método novo.** Ele itera
sobre `module.items`, encontra `Item::ImportDecl`, chama `self.load()`
para cada um, e retorna a lista de módulos carregados. O cache do
ModuleLoader evita reload de módulos importados múltiplas vezes.

### 3.3. Estrutura `ImportedModule`

```rust
/// Um módulo importado + como foi importado.
pub struct ImportedModule {
    /// O ResolvedModule do módulo importado (já filtrado por export).
    pub resolved: Arc<ResolvedModule>,
    /// Como foi importado.
    pub import_kind: ImportKind,
}

pub enum ImportKind {
    /// `import mod` — módulo inteiro, acesso via `mod.fn`.
    /// Todos os itens exported ficam acessíveis sob o prefixo `mod.`
    /// (último componente do path).
    WholeModule { prefix: String },
    /// `import mod as alias` — módulo inteiro, acesso via `alias.fn`.
    WholeModuleAliased { alias: String },
    /// `import MOD.(item1 item2)` — seletivo, itens no escopo direto.
    Selective { items: Vec<String> },
}
```

### 3.4. Fase 2: Export filtering

**Princípio do manual (§3.1):** "Apenas o que é exportado pode ser
importado por outros módulos."

O `ModuleLoader::load_path` já faz lex→parse→resolve. Após `resolve()`,
precisa filtrar o `ResolvedModule` para só conter itens exportados.

#### 3.4.1. Transitividade do export de tipos

Exportar um tipo não é só exportar o nome do tipo — é exportar todo o
**ecossistema semântico** que torna o tipo usável no módulo importador.
Quando `export Complex` aparece num módulo, o importador precisa de:

1. **O tipo no `TypeEnv`** — binding `Complex → Ty::Struct("Complex")`
   (ou `Ty::Enum`), para que o importador possa declarar parâmetros e
   variáveis desse tipo.

2. **A definição no `StructRegistry`/`EnumRegistry`** — campos e
   offsets (struct) ou variantes e payloads (enum), para que o
   importador possa construir valores, fazer pattern matching, e
   acessar fields.

3. **Todas as implementações de interface** no `InterfaceRegistry`
   onde `type_name == "Complex"` — cada `ImplEntry` carrega
   `interface_name` (ex: `NUM`, `SHOW`, `EQ`) e os métodos da impl.
   Sem isso, o importador não pode usar `+`, `show`, `=` etc. sobre
   valores do tipo importado.

4. **As signatures dos métodos dessas implementações** — cada método
   de `implements` vira uma `Signature` flat no `ResolvedModule`
   (ex: `+ :: Complex Complex => Complex`). O DispatchTable do
   importador precisa dessas signatures para resolver chamadas.

5. **As `FunctionDef`s dos métodos com corpo Kata** — métodos de
   interface implementados com `lambda` (não-FFI) precisam do corpo
   para o inference produzir `TypedFunction`. Sem isso, o codegen não
   tem o que compilar.

6. **As interfaces implementadas** (se definidas no módulo exportador)
   — se o módulo define `interface VETOR` e `Complex implements VETOR`,
   exportar `Complex` exige exportar `VETOR` também, senão o
   `ImplEntry` referencia uma interface inexistente no importador.

```rust
fn filter_exports(resolved: ResolvedModule, module: &Module) -> ResolvedModule {
    // Coletar nomes exportados: percorrer module.items por ExportDecl.
    let exported: HashSet<String> = module.items.iter()
        .filter_map(|item| match &item.node {
            Item::ExportDecl { items } => {
                items.iter().filter(|ei| ei.reexport_from.is_none())
                    .map(|ei| ei.name.clone())
                    .collect()
            }
            _ => None
        })
        .flatten()
        .collect();

    // Se não há export decl, TUDO é exportado (compatibilidade — módulo
    // sem export é módulo aberto, como o prelude atual).
    if exported.is_empty() {
        return resolved; // sem filtro
    }

    // ── Fechamento transitivo do export ──────────────────────────
    // Para cada tipo exportado, coletar dependências transitivas:
    // - ImplEntry onde type_name == tipo exportado
    // - Interface names dessas impls (interfaces implementadas)
    // - Métodos dessas impls (signatures + functions)
    // - Interfaces definidas no módulo que essas impls referenciam

    let mut closure = exported.clone();

    // 1. Para cada tipo exportado, encontrar impls e adicionar nomes
    //    de interfaces + nomes de métodos ao closure.
    for impl_entry in &resolved.interface_registry.impls_view() {
        if closure.contains(&impl_entry.type_name) {
            // Adicionar interface implementada
            closure.insert(impl_entry.interface_name.clone());
            // Adicionar nomes de métodos
            for method in &impl_entry.methods {
                closure.insert(method.name.clone());
            }
        }
    }

    // 2. Interfaces definidas no módulo que estão no closure:
    //    adicionar suas supertraits (recursivo).
    let mut changed = true;
    while changed {
        changed = false;
        for iface_name in closure.iter().cloned().collect::<Vec<_>>() {
            if let Some(info) = resolved.interface_registry.get_interface(&iface_name) {
                for st in &info.supertraits {
                    if closure.insert(st.clone()) {
                        changed = true;
                    }
                }
            }
        }
    }

    // ── Filtrar ResolvedModule pelo closure ──────────────────────

    // Signatures: manter se nome está no closure (função exportada
    // diretamente OU método de impl de tipo exportado).
    let signatures: Vec<Signature> = resolved.signatures.into_iter()
        .filter(|s| closure.contains(&s.name))
        .collect();

    // Functions: mesmo critério (corpos Kata de métodos de interface).
    let functions: Vec<FunctionDef> = resolved.functions.into_iter()
        .filter(|f| closure.contains(&f.name))
        .collect();

    // Actions: só se explicitamente exportadas.
    let actions: Vec<ActionDef> = resolved.actions.into_iter()
        .filter(|a| closure.contains(&a.name))
        .collect();

    // TypeEnv: manter bindings de tipos no closure.
    // (TypeEnv.merge_bindings_from já copia tudo; o filtro acontece
    // no merge_resolved ao só copiar bindings de tipos exportados.)

    // EnumRegistry: manter enums cujo nome está no closure.
    // StructRegistry: manter structs cujo nome está no closure.
    // InterfaceRegistry: manter interfaces no closure + impls onde
    //   type_name está no closure.

    ResolvedModule {
        signatures,
        functions,
        actions,
        // type_env, enum_registry, etc. — filtrados por closure
        // (implementação específica em cada registry)
        ..resolved
    }
}
```

**Decisão: módulo sem `export` é aberto.** Se um módulo não tem
`ExportDecl`, tudo é exportado. Isso mantém compatibilidade com o
prelude (que não tem `export` explícito) e com módulos simples.
`mock_math.kata` tem `export dobrar triplicar` — só essas duas funções
serão visíveis.

**Decisão: export de tipo é transitivo.** Exportar `Complex` leva
automaticamente: `ImplEntry`s de `Complex`, interfaces implementadas,
métodos dessas impls (signatures + functions), e supertraits dessas
interfaces. O exportador não precisa listar cada método individualmente
— o fechamento transitivo cuida disso.

### 3.5. Fase 3: `merge_resolved` com imports

A função `merge_resolved` ganha uma nova assinatura: `(user, imports)`
onde `imports` inclui o prelude como primeiro elemento (import
implícito da Fase 7). O parâmetro especial `prelude` desaparece.

```rust
pub(crate) fn merge_resolved(
    user: ResolvedModule,
    imports: &[ImportedModule],
) -> ResolvedModule {
    // Começa com um ResolvedModule vazio (apenas Unit no TypeEnv).
    let mut merged = ResolvedModule::empty();

    // Merge de cada módulo importado (incluindo prelude como primeiro)
    for imported in imports {
        match &imported.import_kind {
            ImportKind::Selective { items } => {
                // Import seletivo: trazer itens nomeados para o escopo
                // direto (sem prefixo). `triplicar` fica acessível como
                // `triplicar`, não `mock_math.triplicar`.
                for item_name in items {
                    // Buscar no resolved do módulo importado
                    if let Some(sig) = imported.resolved.signatures
                        .iter().find(|s| &s.name == item_name)
                    {
                        merged.signatures.push(sig.clone());
                    }
                    if let Some(func) = imported.resolved.functions
                        .iter().find(|f| &f.name == item_name)
                    {
                        merged.functions.push(func.clone());
                    }
                    if let Some(action) = imported.resolved.actions
                        .iter().find(|a| &a.name == item_name)
                    {
                        merged.actions.push(action.clone());
                    }
                }
            }
            ImportKind::WholeModule { prefix } => {
                // Módulo inteiro: itens exportados ficam acessíveis via
                // prefixo `mod.`. Registrar no TypeEnv como namespace.
                // Ver §3.6 — ModuleAccess no inference.
                register_module_namespace(&mut merged, prefix,
                    &imported.resolved);
            }
            ImportKind::WholeModuleAliased { alias } => {
                register_module_namespace(&mut merged, alias,
                    &imported.resolved);
            }
        }
    }

    // Merge do módulo do usuário por último (sobrescreve imports)
    merge_in(&mut merged, user);

    merged
}
```

**Nota:** O prelude (import implícito de `core`) é o primeiro elemento
em `imports`. Como é `WholeModule { prefix: "core" }`, seus itens
exportados ficam acessíveis via `core.echo!` etc. Mas o manual §3.3
diz que itens do prelude devem ser acessíveis sem prefixo. Solução:
o import implícito de `core` é tratado como `Selective` com todos os
itens exportados (efeito prático: tudo no escopo direto). Alternativa:
tratar `core` como caso especial onde `WholeModule` também registra
itens no escopo direto. **Decisão: import implícito de core como
Selective com todos os exported.** É o comportamento atual do
`merge_resolved(prelude, user)` — itens do prelude no escopo direto.

### 3.6. Fase 4: Module access no inference

**Problema:** `mock_math.dobrar 21` é parseado como
`DotAccess { expr: Ident("mock_math"), index: Field("dobrar") }`.
O inference trata `DotAccess` como field access em struct ou index em
tupla. Precisa de um novo caminho: quando o receptor é um nome de
módulo importado, resolver `Field("fn")` como função do módulo.

**Solução:** adicionar um `ModuleRegistry` no `InferCtx` (ou
`ResolvedModule`) que mapeia `module_name → ResolvedModule`. No
`infer_dot_access`, antes de tentar struct/tupla, verificar se o
receptor é `Ident("mod_name")` onde `mod_name` está no ModuleRegistry.

```rust
// infer_dot_access — novo caminho antes do match em (Ty, DotIndex):

if let Expr::Ident { name } = &expr.node {
    if let Some(module) = ctx.module_registry.get(name) {
        // É acesso a módulo: mod.fn
        if let DotIndex::Field(fn_name) = index {
            // Buscar função no módulo importado
            if let Some(sig) = module.signatures.iter()
                .find(|s| &s.name == fn_name)
            {
                // Retornar como TypedExprKind::Ident com nome qualificado
                // ou como nova variante ModuleAccess.
                return Ok(TypedExpr {
                    ty: Ty::Function(
                        sig.param_types.clone(),
                        Box::new(sig.return_type.clone()),
                    ),
                    kind: TypedExprKind::ModuleAccess {
                        module: name.clone(),
                        item: fn_name.clone(),
                    },
                    span: *span,
                });
            }
        }
    }
}
```

**Codegen:** `TypedExprKind::ModuleAccess { module, item }` é tratado
como `Ident` com nome qualificado `module.item` — o DispatchTable já
resolve por nome. Alternativamente, o name mangling do monomorphizador
pode usar `module.item` como nome único.

**Decisão: Nova variante `TypedExprKind::ModuleAccess`.** Mais
explícito que reusar `Ident`. O codegen e o monomorphizador precisam
tratá-la, mas é trivial — é semanticamente idêntico a `Ident` com nome
qualificado.

### 3.7. Fase 5: Prelude em sub-módulos

O manual (§3.3) diz: "Módulos secundários não importam o `core`
magicamente — cada módulo resolve seus tipos independentemente."

Hoje `ModuleLoader::load_path` chama `resolve()` que cria um `TypeEnv`
vazio (só com `Unit`). O sub-módulo não tem acesso a `Int`, `Float`,
`Text`, `Boolean`, etc. — vai falhar na inferência.

**Decisão: import implícito de `core` em sub-módulos.** O
`ModuleLoader::load_path` insere um `ImportDecl { path: ["core"],
alias: None, items: None }` virtual no topo do módulo antes de
processar imports. O `core.kata` é carregado do filesystem (search
path `stdlib/`) pelo mesmo código path que qualquer outro import.
O sub-módulo resolve tipos contra o prelude, como o entrypoint faz.

Isso é o mesmo mecanismo da Fase 7 (import implícito do prelude em
entrypoints) — aplicado a sub-módulos. Não há `load_prelude()` nem
`merge_resolved(prelude, ...)` — o prelude é o primeiro
`ImportedModule` na lista de imports do sub-módulo.

```rust
// ModuleLoader::load_path — após parse, antes de resolve
let mut module = parsed_module;
inject_implicit_core_import(&mut module);
let imports = self.load_imports(&module)?;
let user = resolve(&module)?;
let resolved = merge_resolved(user, &imports);
// Agora filter_exports sobre o resolved
let filtered = filter_exports(resolved, &module);
```

### 3.8. Sintaxe legada nos exemplos

`test_imports.kata` usa:

1. `$(mock_math.dobrar 21)` — `$()` não existe. Era chamada de função
   em contexto de Action. Em Kata5, funções puras são chamadas
   diretamente: `mock_math.dobrar 21` (que agora será `ModuleAccess`).

2. `$(format "Dobro: {}" resultado)` — `format` não está no prelude.
   `echo!` chama `show` internamente. Para interpolação, o manual
   descreve `@log{msg: "template {expr}"}` que desugara para `format`.
   `format` como função standalone não existe no prelude atual.
   **Decisão:** simplificar o exemplo para não depender de `format`.
   Usar `echo!` com `show` e `string_concat` para construir a mensagem.

## 4. Fases de Implementação

### Fase 1: ModuleLoader::load_imports + driver integration
- `ModuleLoader::load_imports(&self, module: &Module) -> Result<Vec<ImportedModule>>`
- `ImportedModule` e `ImportKind` structs
- Driver chama `load_imports` entre parse e resolve
- Search paths: dir do entrypoint + stdlib dir
- Testes: ModuleLoader carrega módulo simples, retorna ResolvedModule

### Fase 2: Export filtering + fechamento transitivo de tipos
- `filter_exports(resolved, module) -> ResolvedModule`
- Módulo sem ExportDecl = aberto (tudo exportado)
- Módulo com ExportDecl = só exported é visível
- Fechamento transitivo: export de tipo leva impls, interfaces, métodos
- `impls_view()` no InterfaceRegistry para iterar impls (read-only)
- Testes: módulo com export filtra signatures/functions/actions;
  export de tipo leva impls + interfaces + métodos

### Fase 3: merge_resolved com imports
- `merge_resolved(user, imports)` — nova assinatura (sem prelude especial)
- Prelude é o primeiro `ImportedModule` (Selective com todos os exported)
- Import seletivo: itens no escopo direto
- Import de módulo inteiro: registrar no ModuleRegistry
- `merge_in(&mut merged, user)` — merge do user por último (sobrescreve)
- Testes: import seletivo traz item para escopo direto

### Fase 4: ModuleAccess no inference
- `ModuleRegistry` no `InferCtx` (HashMap<String, Arc<ResolvedModule>>)
- `infer_dot_access`: novo caminho para `Ident("mod") + Field("fn")`
- `TypedExprKind::ModuleAccess { module, item }`
- Codegen: ModuleAccess → DispatchTable lookup por nome qualificado
- Monomorph: ModuleAccess → name mangling com `module.item`
- Testes: `mod.fn arg` infere e executa corretamente

### Fase 5: Prelude em sub-módulos
- Sub-módulos importam `core` implicitamente (mesmo mecanismo da Fase 7)
- `ModuleLoader::load_path` insere `ImportDecl { path: ["core"] }` virtual
  antes de processar imports do sub-módulo
- O `core.kata` é carregado do filesystem (search path `stdlib/`)
- Testes: sub-módulo com `Int => Int` funciona (type env tem Int)

### Fase 6: Migrar exemplos
- Criar `examples/modules/mock_math.kata` (módulo exportado)
- Criar `examples/modules/imports.kata` (entrypoint que importa)
- Adaptar para sintaxe moderna (sem `$()`, sem `format`)
- Snapshot test: `imports.kata` executa e produz output esperado
- Atualizar `examples_snapshot.rs` para suportar subdiretório em
  `examples/` (hoje só lista top-level `.kata`)

### Fase 7: Importação implícita do prelude (substituir `load_prelude`)

**Objetivo:** eliminar o caso especial do prelude. Hoje o prelude é
injetado por `load_prelude()` + `merge_resolved(prelude, user)` em 4
call sites distintos no driver. Com o sistema de import/export
funcionando, o prelude passa a ser simplesmente o primeiro módulo
importado implicitamente por todo entrypoint.

**Mudança:**

1. O driver insere um `ImportDecl { path: ["core"], alias: None,
   items: None }` virtual no topo da lista de items do módulo
   (antes dos imports explícitos do usuário). Isso é feito no
   `run_pipeline` (e equivalentes em `cmd_test`, `cmd_build`, REPL)
   logo após o parse.

2. O `ModuleLoader` carrega `stdlib/core.kata` como qualquer outro
   módulo — `resolve_path(["core"])` encontra `core.kata` no search
   path `stdlib/`. O `filter_exports` aplica-se normalmente (core.kata
   não tem `export` → módulo aberto → tudo exportado).

3. `merge_resolved` perde o parâmetro especial `prelude`. Passa a ser
   `merge_resolved(user, imports)` onde `imports` inclui o prelude
   como primeiro elemento (import implícito).

4. `load_prelude()` é removido. O `prelude_sigs.rs` é deletado. O
   `include_str!("../../../stdlib/core.kata")` deixa de ser necessário
   — o ModuleLoader lê o arquivo do filesystem.

5. Sub-módulos (Fase 5) também importam `core` implicitamente — o
   ModuleLoader insere o mesmo `ImportDecl` virtual ao carregar
   qualquer módulo.

**Call sites afetados (4):**
- `main.rs:314` — `run_pipeline` (comando `run`)
- `main.rs:158` — `cmd_test` (comando `test`)
- `aot.rs:46` — `cmd_build` (comando `build`)
- `repl.rs:45,57` — REPL (init + reset)

**Antes:**
```rust
let prelude = load_prelude()?;
let user = resolve(&module)?;
let resolved = merge_resolved(prelude, user);
```

**Depois:**
```rust
// Insere import implícito de core no topo do módulo
inject_implicit_core_import(&mut module);
let imports = loader.load_imports(&module)?;
let user = resolve(&module)?;
let resolved = merge_resolved(user, &imports);
```

**Decisão: import implícito, não injecção especial.** O prelude é
tratado pelo mesmo código path que qualquer outro import. Sem casos
especiais no `merge_resolved`, sem `load_prelude()`, sem
`prelude_sigs.rs`. O manual §3.3 diz: "O prelude é carregado
automaticamente em todos os ficheiros executados como entrypoint" —
import implícito cumpre isso sem mecanismo dedicado.

**Nota sobre `include_str!`:** Hoje `prelude_sigs.rs` embute o prelude
no binário via `include_str!("../../../stdlib/core.kata")`. Com a
mudança, o ModuleLoader lê do filesystem. Em builds de release/AOT
onde o filesystem pode não estar disponível, o `core.kata` pode ser
procurado em search paths que incluem um diretório de assets embutido.
Isso é um detalhe de empacotamento, não de design da linguagem.

## 5. Decisões de Design

| Decisão | Escolha | Razão |
|---|---|---|
| Onde carregar imports | Driver, entre parse e resolve | Consistente com manual §2.2 (module load antes de resolution) |
| Estrutura de import | `ImportedModule` + `ImportKind` enum | Distingue import seletivo vs módulo inteiro vs alias |
| Módulo sem export | Aberto (tudo exportado) | Compatibilidade com prelude; módulo simples não precisa de export |
| Access qualificado | Novo caminho em `infer_dot_access` | Reusa parser existente (DotAccess); não inventa nova sintaxe |
| TypedExprKind | Nova variante `ModuleAccess` | Mais explícito que reusar `Ident`; codegen e monomorph tratam diferentemente |
| Prelude em sub-módulos | Injetado pelo ModuleLoader | Manual §3.3: "carregamento do prelude é responsabilidade do kata-module-loader" |
| Export de tipo é transitivo | ImplEntry + interfaces + métodos + supertraits | Tipo sem impls é inútil no importador; fechamento automático |
| Sintaxe `$()` | Removida (legado) | Não existe no Kata5; funções puras chamadas diretamente |
| Prelude como import implícito | `inject_implicit_core_import` + ModuleLoader | Elimina caso especial; prelude é módulo como qualquer outro |
| `format` | Não adicionado ao prelude | Não está no prelude atual; exemplos usam `show` + `string_concat` |
| Estrutura de exemplos | `examples/modules/` subdiretório | mock_math não é entrypoint; precisa de diretório separado |

## 6. O Que Não Muda

- **Parser:** já funciona para import/export. Sem mudanças.
- **AST:** `ImportDecl` e `ExportDecl` já definidos. Sem mudanças.
- **ModuleLoader (estrutura):** cache, cycle detection, path resolution
  já implementados. Só falta chamar `load_imports` e injetar prelude.
- **`stdlib/core.kata`:** conteúdo inalterado. Só muda como é carregado
  (filesystem via ModuleLoader em vez de `include_str!`).
- **Pipeline pós-resolution:** inference, monomorph, codegen, etc.
  não mudam estruturalmente — só ganham `ModuleAccess` como nova
  variante de TypedExprKind (tratada como Ident qualificado).

## 7. Riscos e Mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| `merge_resolved` muda assinatura | Compile error em aot.rs, repl.rs | Fase 7 atualiza todos os 4 call sites; `load_prelude` removido |
| ModuleAccess não tratado pelo codegen | ICE em `lower_expr` | Fase 4 inclui codegen; testar com exemplo simples |
| ModuleAccess não tratado pelo monomorphizador | ICE em monomorph | Fase 4 inclui monomorph; name mangling trivial |
| `DotAccess` ambíguo: struct field vs module access | Type error se ordem errada | ModuleRegistry check ANTES do match em (Ty, DotIndex) |
| Sub-módulo importando outro sub-módulo (ciclo) | Hang | ModuleLoader já tem cycle detection |
| Sub-módulo com tipos próprios (enum, struct) | Tipos não visíveis no importador | Fechamento transitivo (§3.4.1) leva tipo + impls + interfaces |
| `examples/modules/` não roda no snapshot test | Test falha | Atualizar `example_files()` para recursão em subdirs |

## 8. Evolução Futura (Não Escopo)

- **Tipos qualificados (`mod.MyType`):** Hoje o `DotAccess` em
  contexto de tipo (ex: `mod.MyType` em assinatura) não é tratado.
  O `resolve_type_expr` não procura tipos em módulos importados.
  Os tipos importados entram no `TypeEnv` sem prefixo (merge direto).
  Tipos qualificados exigiriam mudanças no `resolve_type_expr` e no
  parser de tipos.
- **Reexportação:** `export MOD.(items)` — parser já suporta, mas
  semântica não implementada. Requer que o módulo importe `MOD` e
  reexporte itens específicos.
- **Path resolution com `mod.kata`:** O manual menciona
  `utilidades/matematica/mod.kata` como fallback. O `resolve_path`
  atual só procura `file.kata`. Adicionar o fallback é trivial.
- **Import de prelude explícito:** `import core` em sub-módulos —
  a Fase 7 faz o import implícito. O import explícito funciona
  naturalmente após a Fase 7 (o ModuleLoader já sabe carregar `core`).
  Pode ser usado para evitar o import implícito em módulos que não
  precisam do prelude (otimização de tree shaking).
- **Refined types importados:** Refined types trazem predicados e
  smart constructors. O fechamento transitivo precisa incluir
  `refined_decls` e `enum_pred_decls` quando um tipo refined é exportado.

## 9. Critérios de Aceitação

1. `cargo test --workspace` passa sem regressões
2. `kata run examples/modules/imports.kata` executa e produz output
3. `mock_math.dobrar 21` retorna 42 (acesso qualificado a módulo)
4. `triplicar 21` retorna 63 (import seletivo no escopo direto)
5. Itens não exportados por `mock_math` não são acessíveis de `imports.kata`
6. Snapshot test cobre `examples/modules/imports.kata`
7. Sub-módulo (`mock_math.kata`) resolve tipos do prelude (`Int`) sem erro
8. Export de tipo leva transitivamente: impls, interfaces implementadas,
   métodos (signatures + functions), e supertraits dessas interfaces
9. Importador pode usar operadores (`+`, `show`, `=`) sobre valores de
   tipo importado (dispatch funciona pois ImplEntry + signatures foram
   mergeados)
10. `load_prelude()` e `prelude_sigs.rs` removidos — prelude é import
    implícito via ModuleLoader (Fase 7)
11. `echo!`, `Int`, `+` etc. continuam acessíveis sem prefixo em
    entrypoints (import implícito de core como Selective)