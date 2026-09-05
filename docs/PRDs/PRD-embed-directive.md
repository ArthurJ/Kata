# PRD: Diretiva @embed_text / @embed_bytes — Embedding de Recursos em Compile-Time

## Status

✅ Concluído
**Data:** 2026-09-04
**Depende de:** nenhum PRD pendente
**Relacionado:** PRD-recursion-limit (remove Fase 7 — comptime JIT com I/O)

## 1. Objetivo

Permitir que um módulo Kata embuta o conteúdo de arquivos externos como literais
no código compilado, resolvido após o parse e antes da resolution de tipos — não
no comptime JIT.

```kata
constant config := @embed_text{path: "config.txt"}
constant banner := @embed_text{path: "banner.txt"}
constant lookup := @embed_bytes{path: "lookup_table.bin"}
```

Após `resolve_embeds`, `@embed_text{path: "config.txt"}` é substituído por
`Expr::TextLit("conteúdo do arquivo")`. O typeck, comptime, optimizer e codegen
veem apenas um literal. O comptime JIT nunca toca I/O.

## 2. Motivação

### 2.1. constant não deve fazer I/O

`constant` é determinístico: dado o mesmo código-fonte, produz o mesmo TAST.
I/O quebra essa propriedade — `read_file("config.txt")` faz o resultado da
compilação depender do estado do filesystem.

O PRD-recursion-limit Fase 7 propunha levantar a infraestrutura completa do
Runtime (scheduler, fibers, I/O) no comptime JIT para permitir `constant x :=
read_file(...)`. Isso é desproporcional: a FFI de depth (`set_recursion_limit`)
só precisa de um Runtime vazio com um `Cell<u32>`. A Fase 7 foi motivada por
I/O em `constant`, não por depth tracking.

`@embed_text` resolve o embedding de recursos sem tocar no comptime JIT.
A leitura de filesystem vive em `resolve_embeds`, chamada entre parse e
resolution — o ponto do pipeline que já lida com filesystem (module_loader lê
módulos, stdlib).

### 2.2. Analogia com linguagens existentes

- **Rust:** `include_str!("config.txt")` — macro do compilador, resolvida antes
  da const-eval. Retorna `&'static str`. Aparece em qualquer posição de expressão.
- **Zig:** `@embedFile("config.txt")` — builtin do compilador, resolvido em
  análise semântica. Retorna `*const [N:0]u8`.
- **C:** `#include "config.txt"` — preprocessador, inclusão bruta de texto.

Em todos os casos, o mecanismo de embedding é **distinto** do mecanismo de
computação compile-time. Rust não permite `const X: &str = read_file(...)`
— `include_str!` é uma macro dedicada. Kata segue o mesmo princípio:
`@embed_text` é uma diretiva dedicada, não `constant` com I/O.

### 2.3. Composição com constant

`@embed_text` adquire o recurso (`resolve_embeds`). `constant` computa sobre ele
(comptime). Os mecanismos compõem naturalmente:

```kata
constant raw := @embed_text{path: "config.json"}
constant config := parse_json(raw)
constant timeout := config_get(config, "timeout")
```

`@embed_text` retorna um `Text` literal. `constant` avalia `parse_json`
sobre o literal. O comptime JIT executa `parse_json` — uma função Kata pura,
sem I/O.

## 3. Design

### 3.1. Sintaxe

```
@embed_text{path: <Text>}    # → Text literal
@embed_bytes{path: <Text>}   # → Bytes literal
```

- `path` é argumento nomeado (consistente com `@log{msg: "...", when: "..."}`).
- O valor de `path` deve ser um `Expr::TextLit` — o parser não faz const-eval,
  exige o literal diretamente. Path dinâmico não faz sentido: o arquivo é lido
  em compile-time, o path deve ser conhecido estaticamente.
- Path é relativo ao diretório do módulo onde `@embed_text` aparece.
  Path absoluto também é aceito, mas produz **warning de compilação** na v1
  (não futuro) por quebrar reprodutibilidade.
- Se o arquivo não existe: erro gracioso com path e span.

### 3.2. Dois diretivos separados

**Escolha:** `@embed_text` e `@embed_bytes` como diretivos distintos.

**Alternativa rejeitada:** `@embedded{path: "...", type: Text}` com `type`
recebendo um Enum (`EmbedType::Text` / `EmbedType::Bytes`). Motivo: `type`
mistura tipo em posição de valor. Embora `Text`/`Bytes` como variantes de Enum
sejam valores (não referências de tipo), o resolver de `@embedded` precisaria
interpretar `EmbedType::Text` antes do typeck — seria um reconhecimento por
nome, não por tipo. Dois diretivos separados tornam o tipo do resultado
estático pelo nome do diretivo, sem ambiguidade.

### 3.3. @ em posição de expressão

Hoje todos os `@` são atributos em **declarações**: `@ffi(...)`, `@cache`,
`@builtin(...)`, `@log{...}` precedem uma assinatura ou declaração.
`@embed_text{...}` é o primeiro `@` em **posição de expressão**:

```kata
constant x := @embed_text{path: "config.txt"}
echo!(@embed_text{path: "banner.txt"})
constant c := parse_json(@embed_text{path: "config.json"})
```

Isso é um padrão novo no parser, mas justificável: `@` sinaliza "diretiva do
compilador" e `embed_text` sinaliza "inclusão de arquivo como texto".
O parser de expressões (`parse_expr_atom`) ganha um caso: se vê `@` seguido
de `embed_text` ou `embed_bytes`, parseia o dict `{path: "..."}` e
produz `Expr::EmbedText { path: String }` ou `Expr::EmbedBytes { path: String }`.

`@` em posição de declaração (antes de assinatura) continua sendo tratado pelo
parser de declarações, não por `parse_expr_atom`. O parser de expressões só vê
`@` quando está esperando uma expressão (não uma declaração).

### 3.4. Resolução: `resolve_embeds` entre parse e resolution

O fluxo do pipeline:

1. **Parser:** `@embed_text{path: "config.txt"}` →
   `Expr::EmbedText { path: "config.txt" }`
2. **resolve_embeds:** walker recursivo sobre `Module` encontra
   `Expr::EmbedText`, lê o arquivo do filesystem (relativo ao diretório do
   módulo), substitui por `Expr::TextLit("conteúdo")`. Se `Expr::EmbedBytes`,
   substitui por `Expr::BytesLit(...)`. Retorna `(Module, Vec<PathBuf>)` —
   o módulo modificado e a lista de dependências.
3. **Resolution (pass0 + pass1):** vê apenas `TextLit` ou `BytesLit` —
   `@embed_text` já não existe.
4. **Typeck/inference:** `constant x := "conteúdo"` — bind trivial, sem I/O.
5. **Codegen:** literal baked no binário.

O comptime JIT nunca toca I/O. A infraestrutura de leitura vive em
`resolve_embeds`, na crate `kata-resolution`.

### 3.5. Por que `resolve_embeds` é uma função separada, não dentro de `resolve_with_origin`

`resolve_with_origin` e `resolve_with_prelude` recebem `&Module` imutável e são
chamadas em 15+ testes com strings puras (sem filesystem). Mudar a assinatura
para `&mut Module` ou adicionar I/O dentro delas quebraria o contrato de testes
existentes — testes que passam strings hardcoded esperam resolution pura.

`resolve_embeds` é uma função separada, chamada explicitamente entre parse e
resolve. Recebe `Module` (owned) e `module_dir: &Path`, devolve `(Module,
Vec<PathBuf>)`. O caller (Pipeline, ModuleLoader, REPL, etc.) chama
`resolve_embeds` antes de passar o `Module` para `resolve_with_origin`.

Como rede de segurança contra callers que esquecem o passo, `infer_module`
tem um `debug_assert` que varre o `Module` e panica em debug se encontrar
`Expr::EmbedText`/`Expr::EmbedBytes` residual. Custo O(AST) só em debug,
zero em release.

### 3.6. Call sites

O grafo de frontends que compilam Kata converge para `resolve_embeds` antes
de chamar as funções de resolution. Cada caller já tem acesso ao path do
arquivo (ou cwd):

| Frontend | Arquivo | Path disponível |
|---|---|---|
| Pipeline (run/build/test) | `kata-driver/src/pipeline.rs` | `file_path` (após `.parse()`) |
| ModuleLoader | `kata-resolution/src/module_loader/mod.rs` (`load_path`) | `path` (do arquivo sendo carregado) |
| REPL JIT | `kata-driver/src/repl/mod.rs` | `.` (cwd) |
| REPL interp | `kata-driver/src/repl/interp_session.rs` | `.` (cwd) |
| Doctest | `kata-driver/src/doctest.rs` | `.` (cwd) |
| LSP | `kata-lsp/src/analysis.rs` | path do documento |

### 3.7. Rastreamento de dependências

`resolve_embeds` retorna `Vec<PathBuf>` com cada arquivo embutido. O caller
popula `ResolvedModule.embed_dependencies` com essa lista. Para futura
incremental compilation e cache invalidation: se `config.txt` muda, o módulo
que faz `@embed_text{path: "config.txt"}` é recompilado.

**Estrutura:** `ResolvedModule` ganha campo `embed_dependencies: Vec<PathBuf>`.
`merge_two` e `merge_imports` concatenam o campo — sem isso, dependências de
módulos importados se perdem no merge e incremental compilation não invalida
quando um arquivo embutido por um import muda.

### 3.8. Path relativo vs absoluto

- **Relativo:** interpretado a partir do diretório do arquivo-fonte que contém
  o `@embed_text`. Consistente com `import` (que resolve relativo ao módulo).
- **Absoluto:** aceito, mas produz **warning de compilação na v1** (5 linhas,
  protege reprodutibilidade desde o dia 1). Binários compilados do mesmo fonte
  diferem por máquina com path absoluto.
- Não há busca em paths configurados (`-I` flags) na versão inicial. Futuro.

### 3.9. Stdlib embedded

Módulos stdlib passam por `load_path` com path sintético (`$stdlib/core.kata`).
Se a stdlib usar `@embed_text` no futuro, `resolve_path` rejeita com erro
explícito ("embed não suportado em módulo embedded") — não silêncio. Hoje não
é problema (stdlib não tem embeds), mas o erro explícito previne bug latente.

### 3.10. Tamanho do arquivo

Sem limite na versão inicial. O conteúdo vira literal no AST e é threaded
pelo pipeline. Arquivos muito grandes (>1MB) aumentam tempo de compilação
e consumo de memória do compilador.

**Nota de custo:** `ResolvedModule` é `Clone` e é clonado em pelo menos 3
pontos do pipeline (mutação de `type_env` em `pipeline.rs`, `merge_imports`,
`filter_exports` dos imports). Um `TextLit` de 10MB é clonado em cada ponto —
custo O(tamanho × clones). Não é bloqueante para a v1, mas se embeds grandes
se tornarem comuns, `Arc<str>`/`Arc<[u8]>` nos literais ou interning resolve
o custo de clone. Futuro.

## 4. Decisões de design

### 4.1. resolve_embeds entre parse e resolution, não no comptime

**Escolha:** `resolve_embeds` roda após parse, antes de `resolve_with_origin`.

**Alternativa rejeitada:** resolver no comptime JIT (como `constant`).
Motivo: exige infraestrutura de I/O no comptime JIT (scheduler, fibers, I/O
runtime — a Fase 7 do PRD-recursion-limit). Desproporcional para o caso de
uso. O resolver já lida com filesystem; reusar essa infraestrutura é trivial.

### 4.2. Path deve ser literal

**Escolha:** `path` deve ser `Expr::TextLit` — string literal no fonte.

**Alternativa rejeitada:** permitir path dinâmico (`path: some_variable`).
Motivo: o arquivo é lido em compile-time; o path deve ser conhecido
estaticamente. Path dinâmico exigiria const-eval antes da resolution, o que
inverte a ordem do pipeline (typeck/comptime antes de resolution).

### 4.3. Dois diretivos, não um com parâmetro type

**Escolha:** `@embed_text` e `@embed_bytes` separados.

**Alternativa rejeitada:** `@embedded{path: "...", type: EmbedType::Text}`.
Motivo: o tipo do resultado é determinado pelo nome do diretivo, sem
ambiguidade. `type` como argumento seria reconhecimento por nome no resolver,
não por tipo — decoração sem função semântica.

### 4.4. @ em posição de expressão

**Escolha:** permitir `@embed_text{...}` em qualquer posição de expressão.

**Alternativa rejeitada:** apenas como argumento de `constant`.
Motivo: restringe desnecessariamente. `echo!(@embed_text{path: "banner.txt"})`
é válido e útil. A resolução em `resolve_embeds` substitui a diretiva por
literal antes da inference, então a posição não importa.

### 4.5. Função separada, não dentro de resolve_with_origin

**Escolha:** `resolve_embeds` é uma função pública separada em `kata-resolution`.

**Alternativa rejeitada:** alojar dentro de `resolve_with_origin`/`resolve_with_prelude`.
Motivo: essas funções recebem `&Module` imutável e são chamadas em 15+ testes
com strings puras (sem filesystem). Mudar assinatura ou adicionar I/O quebraria
o contrato de testes existentes. A função separada + `debug_assert` em
`infer_module` como rede de segurança é menos invasiva e igualmente robusta.

### 4.6. Walker com match exaustivo, sem wildcard

**Escolha:** o walker usa braços explícitos para todas as variantes de `Expr`.
Sem `_ =>`.

**Motivo:** `Expr` tem 48 variantes com esconderijos profundos (`Pattern::Literal`
dentro de match arms, `GuardClause.condition`, `WithBinding.value`,
`DotIndex::Range`, `ReadMode::Chunk` dentro de `SelectArm::IoRead`, etc.).
Um walker com `_ => {}` compila silenciosamente e pula `EmbedText` dentro de
um pattern literal ou guard. Sem wildcard, o compilador Rust emite E0004
(non-exhaustive match) quando uma nova variante de `Expr` for adicionada no
futuro, transformando "missing variant silencioso" em erro de compilação.

`desugar.rs` (`kata-inference/src/desugar.rs`) é o gabarito estrutural — é o
único walker genérico de `Expr` existente, escrito à mão com ~83 referências
a `Expr::`, e já cobre `GuardClause`, `WithBinding`, `SelectArm`, `ReadMode`,
`DotIndex`.

## 5. AST — novos nós

### `Expr` (kata-ast/src/expr.rs)

```rust
pub enum Expr {
    // ... existentes ...

    /// @embed_text{path: "..."} — lê arquivo como Text.
    /// Resolvido por resolve_embeds (antes da resolution de tipos).
    EmbedText { path: String },

    /// @embed_bytes{path: "..."} — lê arquivo como Bytes.
    /// Resolvido por resolve_embeds (antes da resolution de tipos).
    EmbedBytes { path: String },
}
```

Estes nós são **efêmeros**: existem apenas entre parser e `resolve_embeds`.
Após `resolve_embeds`, não há `Expr::EmbedText` no AST — foram substituídos
por literais. O typeck, inference, codegen e interp nunca os veem.

### `TypedExprKind`

Não há novos `TypedExprKind` — após `resolve_embeds`, o nó já é `TextLit` ou
`BytesLit`, tipados normalmente.

### `ResolvedModule`

```rust
pub struct ResolvedModule {
    // ... campos existentes ...
    /// Arquivos embutidos via @embed_text/@embed_bytes.
    /// Para rastreamento de dependências (incremental compilation).
    /// Concatenado em merge_two e merge_imports.
    pub embed_dependencies: Vec<PathBuf>,
}
```

## 6. Parser

### `parse_expr_atom` — novo caso para `@`

Em `crates/kata-parser/src/expressions.rs`, `parse_expr_atom` ganha um branch:

- Se o token atual é `@`:
  1. Consumir `@`.
  2. Ler identificador: `embed_text` ou `embed_bytes`.
  3. Esperar `{` e parsear dict com `parse_directive_args` (reusa o parser
     existente que trata `{key: value}`).
  4. `parse_directive_args` retorna `Vec<DirectiveArg>` onde cada
     `DirectiveArg::Named { key, value }` tem `value: Box<Spanned<Expr>>`.
     Extrair o `Expr::TextLit` de dentro do `DirectiveArg`.
  5. Validar: exatamente uma key `path`, valor é `Expr::TextLit`.
  6. Produzir `Expr::EmbedText { path }` ou `Expr::EmbedBytes { path }`.
  7. Se o identificador não é `embed_text`/`embed_bytes`: erro de parser
     ("unknown directive in expression position").

**Nota:** `@` em posição de declaração (antes de assinatura) continua sendo
tratado pelo parser de declarações (`parse_directives`), não por
`parse_expr_atom`. O parser de expressões só vê `@` quando está esperando
uma expressão (não uma declaração).

### Validação do argumento path

O parser valida:
- O dict tem exatamente uma key: `path`.
- O valor de `path` é `Expr::TextLit`.
- Keys desconhecidas → erro de parser.

## 7. resolve_embeds

### Assinatura

```rust
/// Lê arquivos referenciados por @embed_text/@embed_bytes e substitui
/// os nós por literais (TextLit/BytesLit). Chamada entre parse e
/// resolve_with_origin/resolve_with_prelude.
///
/// `module_dir` é o diretório do arquivo-fonte (para resolver paths
/// relativos). Path absoluto é aceito mas produz warning.
///
/// Retorna o Module modificado e a lista de arquivos embutidos
/// (para rastreamento de dependências).
pub fn resolve_embeds(
    module: Module,
    module_dir: &Path,
) -> Result<(Module, Vec<PathBuf>), Vec<EmbedError>>
```

Vive em `kata-resolution` (a crate do "mundo externo" — I/O de módulos,
types de erro estruturados, dependência comum de todos os frontends).

### Walker recursivo

O walker percorre `Module` → `Item` → `Expr`, substituindo nós
`EmbedText`/`EmbedBytes` por `TextLit`/`BytesLit`.

**Estrutura do walker:**

- `walk_module(module: Module, ctx: &mut EmbedCtx) -> Module` — percorre
  `module.items`, chama `walk_item` em cada um.
- `walk_item(item: Item, ctx: &mut EmbedCtx) -> Item` — match em todas as
  variantes de `Item`, recursa nos campos que contêm `Expr`.
- `walk_expr(expr: Spanned<Expr>, ctx: &mut EmbedCtx) -> Spanned<Expr>` —
  match em todas as 48 variantes de `Expr`, sem wildcard. Troca apenas
  `.node`, preserva `.span` (âncora de diagnóstico).

**Esconderijos profundos** que o walker cobre explicitamente:

Em nível de `Expr`:
- `Apply { callee, args }` — recursa em callee e cada arg
- `TypeAscription { expr, ty }` — recursa em expr (ty é TypeExpr, não Expr)
- `Grouping { inner }` — recursa
- `Tuple { elements }` — recursa em cada elemento
- `Let { value }`, `LetDestruct { value }` — recursa em value
- `Lambda { patterns, body, guards, with_bindings }` — recursa em body,
  guards (condition + body), with_bindings (value); patterns podem conter
  `Pattern::Literal(Spanned<Expr>)` — recursa nessas
- `Match { scrutinee, arms }` — recursa em scrutinee; cada `MatchArm` tem
  `guard: Option<Spanned<Expr>>` e `body: Spanned<Expr>` — recursa em ambos;
  `pattern: Option<Spanned<Pattern>>` pode conter `Pattern::Literal` — recursa
- `Pipe { lhs, rhs }`, `PipeLimit { lhs, rhs, limit }` — recursa em todos
- `PipeFallback { lhs, rhs }` — recursa
- `ActionCall { callee, args }` — recursa em args
- `TypeOf { expr }` — recursa
- `Return(expr)`, `Question(expr)` — recursa
- `Loop { body }` — recursa em cada statement
- `Var { value }`, `Reassign { value }` — recursa em value
- `DotAccess { expr, index }` — recursa em expr; se `index` é
  `DotIndex::Range { start, end }`, recursa em start e end
- `ListLit`, `ArrayLit`, `SetLit` — recursa em cada elemento
- `DictLit { entries }` — recursa em cada key e value
- `RangeLit { start, step, end }` — recursa em todos
- `ForIn { iterable, body }` — recursa em iterable e cada statement do body
- `In { item, collection }` — recursa em ambos
- `ChannelSend { channel, value }` — recursa em ambos
- `ChannelRecv { channel }` — recursa em channel
- `Select { arms, timeout_ms, timeout_body }` — recursa em cada arm
  (channel, body, handle_expr, read_mode); `ReadMode::Chunk(Spanned<Expr>)`
  — recursa; recursa em timeout_ms e timeout_body se Some
- `Block { stmts }` — recursa em cada statement
- `IntLit`, `FloatLit`, `TextLit`, `BytesLit`, `Unit`, `Ident`, `Hole`,
  `Break`, `Continue`, `VariantQual` — retornam self (sem sub-expressões)

Em nível de `Item`:
- `Sig { body }` — se `Some(clauses)`, recursa em cada `LambdaClause`
  (body, guards, with_bindings, synthetic_pre, synthetic_post)
- `ActionDecl { param_defaults, body }` — recursa em cada default se `Some`;
  recursa em cada `ActionStmt.expr` do body
- `ConstantDecl { value }` — recursa em value
- `EntryExpr(expr)` — recursa
- `DataDecl { refined }` — se `Some(RefinedDecl)`, recursa em cada predicado
- `EnumDecl { variants }` — cada `VariantDecl` tem `predicate` e
  `fixed_value` (ambos `Option<Spanned<Expr>>`) — recursa se Some
- `InterfaceDecl { signatures }` — cada `InterfaceSig` tem
  `default_body: Option<Vec<LambdaClause>>` — recursa se Some
- `ImplementsDecl { methods }`, `RefinesDecl { methods }` — cada `ImplMethod`
  tem `body: Option<Vec<LambdaClause>>` — recursa se Some
- `DirectiveDecl { args, body }` — `args: Vec<DirectiveArg>` onde
  `DirectiveArg::Expr(Box<Spanned<Expr>>)` e `DirectiveArg::Named { value }`
  contêm `Expr` — recursa; `body: Vec<ActionStmt>` — recursa em cada stmt
- `ImportDecl`, `ExportDecl`, `AliasDecl` — sem Expr, retornam self

### `EmbedError`

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum EmbedError {
    #[error("cannot embed file \"{path}\": {message}")]
    EmbedFailed {
        path: String,
        message: String,
        span: Span,
    },
    #[error("embed not supported in embedded module")]
    EmbeddedModule { span: Span },
}
```

Com `#[diagnostic(code = "resolve.embed_failed")]` e label no span, seguindo
o padrão de `ResolveError`. Conversões:
- `IntoReport` para o Pipeline (`Vec<miette::Report>`)
- `LoadError::Embed(Vec<EmbedError>)` para o ModuleLoader

### `resolve_path`

```rust
fn resolve_path(path: &str, module_dir: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() { p.to_path_buf() }
    else { module_dir.join(p) }
}
```

### Erro gracioso

Se o arquivo não existe ou não pode ser lido:
```
error: cannot embed file "config.txt": No such file or directory
  --> src/main.kata:3:20
   |
 3 | constant config := @embed_text{path: "config.txt"}
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

### Safety net: debug_assert em infer_module

No início de `infer_module` (`kata-inference/src/infer/mod.rs`), em builds
de debug:

```rust
#[cfg(debug_assertions)]
{
    for item in &module.items {
        assert_no_embed_residual(item); // panica se encontrar EmbedText/Bytes
    }
}
```

Custo O(AST) só em debug, zero em release. Garante que nenhum frontend
esqueça o passo — o programa trava em teste em vez de produzir ICE na
inference (match exaustivo sem braço para `EmbedText`).

## 8. Interação com TwoPass

O Pipeline em modo `TwoPass` parseia o módulo duas vezes:

1. `parse_decls_only` → `quick_resolve` (extrai assinaturas e aridades)
2. `parse_with_arity_recovery` → resolve completo

`parse_decls_only` não percorre expressões de corpo — só extrai assinaturas
(Sig, ActionDecl, data, enum, interface, implements). Mas pass0
(`pass0.rs:581-586`) lê `fixed_value` e `predicate` de `VariantDecl`, que
são `Option<Spanned<Expr>>`. Se alguém escrever
`enum E: OK(@embed_bytes{path: "x.bin"})`, o Pass 1 veria `Expr::EmbedBytes`
cru em `fixed_value`.

**Mitigação:** `resolve_embeds` roda sobre o módulo do Pass 2 (o módulo
completo, não o `decls_only`). O `decls_module` do Pass 1 é temporário —
descartado após `extract_arities`. Mas `quick_resolve` chama
`resolve_with_prelude` que roda pass0, e pass0 lê `fixed_value`/`predicate`.

**Solução:** rodar `resolve_embeds` sobre o `decls_module` do Pass 1 também.
O custo é uma walk extra sobre o módulo de declarações, mas garante que
pass0 nunca veja `Expr::EmbedText`/`EmbedBytes`. Como `parse_decls_only`
não produz corpos de funções, a walk é mais rápida (menos nós).

Alternativa: confirmar empiricamente que `parse_decls_only` nunca materializa
`fixed_value`/`predicate` com `Expr::EmbedText` (o parser de decls_only
skipa EntryExprs mas ainda parseia EnumDecl com variantes). Se confirmado
que `parse_decls_only` preserva `fixed_value`, rodar `resolve_embeds` no
Pass 1 é obrigatório. Teste de regressão com `enum` cujo `fixed_value` usa
`@embed` valida isso.

## 9. Fases

### Fase 1 — AST + Parser

**Escopo:** `crates/kata-ast/src/expr.rs`, `crates/kata-parser/src/expressions.rs`

- Adicionar `Expr::EmbedText { path: String }` e `Expr::EmbedBytes { path: String }`.
- `parse_expr_atom` trata `@embed_text{...}` e `@embed_bytes{...}`.
- Reusar `parse_directive_args` para o dict `{path: "..."}`.
- Extrair `Expr::TextLit` de `DirectiveArg::Named { key, value }`.
- Validar: exatamente uma key `path`, valor é `TextLit`.

**DoD:** `kata parse "constant x := @embed_text{path: \"f.txt\"}"` produz
`Expr::ConstantDecl` com `Expr::EmbedText { path: "f.txt" }`.

### Fase 2 — resolve_embeds + call sites

**Escopo:** `crates/kata-resolution/src/embed.rs` (novo), `crates/kata-resolution/src/lib.rs`,
`crates/kata-resolution/src/types.rs`, `crates/kata-resolution/src/module_loader/mod.rs`,
`crates/kata-driver/src/pipeline.rs`, `crates/kata-driver/src/repl/mod.rs`,
`crates/kata-driver/src/repl/interp_session.rs`, `crates/kata-driver/src/doctest.rs`,
`crates/kata-lsp/src/analysis.rs`, `crates/kata-inference/src/infer/mod.rs`

- Walker recursivo com match exaustivo (sem wildcard) em `embed.rs`.
- `EmbedError` com thiserror + conversões (`IntoReport`, `LoadError::Embed`).
- `resolve_embeds` chamada em: Pipeline (Pass 1 e Pass 2), ModuleLoader,
  REPL JIT, REPL interp, doctest, LSP.
- `debug_assert` em `infer_module` (safety net).
- `ResolvedModule.embed_dependencies` + concatenação em `merge_two`/`merge_imports`.
- Path absoluto → warning.
- Stdlib embedded → erro explícito.
- Auditoria do TwoPass Pass 1.

**DoD:** `@embed_text{path: "test.txt"}` (onde `test.txt` contém "hello")
é substituído por `Expr::TextLit("hello")` após `resolve_embeds`. Arquivo
inexistente produz erro de resolution com path e span. `@embed_text` em
módulo importado aparece nas dependências agregadas do importador.

### Fase 3 — Testes E2E

**Escopo:** `crates/kata-codegen/tests/embed_e2e.rs`

| Caso | Esperado |
|---|---|
| `constant x := @embed_text{path: "fixtures/hello.txt"}` + `echo!(x)` | imprime "hello" |
| `constant x := @embed_bytes{path: "fixtures/data.bin"}` + `len x` | tamanho do arquivo |
| `@embed_text{path: "nonexistent.txt"}` | erro de resolution gracioso |
| `@embed_text{path: "fixtures/config.json"}` + `parse_json` em comptime | valor parsed disponível em runtime |
| `echo!(@embed_text{path: "fixtures/banner.txt"})` (sem constant) | imprime conteúdo do banner |
| `@embed_text{path: 123}` (path não-TextLit) | erro de parser |
| `@embed_text` em guard de lambda (`> _ x: @embed_text{...}`) | substituído corretamente |
| `@embed_text` em pattern literal de match arm | substituído corretamente |
| `@embed_text` em with_binding | substituído corretamente |
| `@embed_text` em param_default de action | substituído corretamente |
| `@embed_text` em DotAccess slice range | substituído corretamente |
| `@embed_text` em select timeout_ms | substituído corretamente |
| `@embed_text{path: "/abs/path.txt"}` (path absoluto) | warning de compilação + funciona |
| módulo A importa B, B tem `@embed_text{path: "b.txt"}` | `b.txt` nas dependências agregadas de A |

**DoD:** todos os casos passam em ambos backends (JIT e interp).

## 10. Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| ast | `crates/kata-ast/src/expr.rs` | `Expr::EmbedText`, `Expr::EmbedBytes` |
| parser | `crates/kata-parser/src/expressions.rs` | `parse_expr_atom` trata `@` em posição de expressão |
| resolution | `crates/kata-resolution/src/embed.rs` (novo) | `resolve_embeds` + walker recursivo + `EmbedError` |
| resolution | `crates/kata-resolution/src/types.rs` | `ResolvedModule.embed_dependencies` |
| resolution | `crates/kata-resolution/src/lib.rs` | `pub mod embed;` + export `resolve_embeds`, `EmbedError` |
| resolution | `crates/kata-resolution/src/module_loader/mod.rs` | `load_path` chama `resolve_embeds`; `LoadError::Embed` |
| resolution | `crates/kata-resolution/src/merge_imports.rs` | `merge_imports` concatena `embed_dependencies` |
| driver | `crates/kata-driver/src/pipeline.rs` | Pipeline chama `resolve_embeds` (Pass 1 e Pass 2) |
| driver | `crates/kata-driver/src/repl/mod.rs` | REPL JIT chama `resolve_embeds` |
| driver | `crates/kata-driver/src/repl/interp_session.rs` | REPL interp chama `resolve_embeds` |
| driver | `crates/kata-driver/src/doctest.rs` | Doctest chama `resolve_embeds` |
| lsp | `crates/kata-lsp/src/analysis.rs` | LSP chama `resolve_embeds` |
| inference | `crates/kata-inference/src/infer/mod.rs` | `debug_assert` de safety net |
| codegen | (nenhuma) | Vê apenas `TextLit`/`BytesLit` após `resolve_embeds` |
| interp | (nenhuma) | Idem |
| comptime | (nenhuma) | Vê apenas literais — sem I/O no comptime JIT |

## 11. Fora do escopo

- **`@env{var: "PATH"}`** — leitura de variável de ambiente em compile-time.
  Mesmo padrão (resolvido em `resolve_embeds`, vira literal), mas menos
  determinístico. PRD próprio quando necessário.
- **Path search (`-I` flags)** — busca em diretórios configurados. Futuro.
- **Limite de tamanho** — warning para arquivos grandes. Futuro.
- **Compressão** — `@embed_bytes{path: "...", compress: true}`. Futuro.
- **Hash/checksum** — verificar integridade do arquivo embutido. Futuro.
- **`Arc<str>`/`Arc<[u8]>` nos literais** — mitigar custo de clone de embeds
  grandes. Futuro, se embeds grandes se tornarem comuns.

## 12. Impacto no PRD-recursion-limit

Este PRD torna a **Fase 7** do PRD-recursion-limit desnecessária:

- Fase 7 motivava levantar infraestrutura de I/O no comptime JIT para permitir
  `constant x := read_file(...)`.
- `@embed_text` resolve o embedding de arquivos sem tocar no comptime JIT.
- O comptime JIT continua enxuto (Runtime vazio + FFI de depth).
- A seção "Pureza" do PRD-recursion-limit é simplificada: `check_purity` é
  removido porque `set_recursion_limit` é FFI de configuração (side effect
  legítimo), não porque `constant` deve aceitar I/O arbitrário.

**Ação ao concluir este PRD:** atualizar PRD-recursion-limit — remover Fase 7,
revisar justificativa de `check_purity`, remover débito técnico stale.

## 13. Riscos

### 13.1. @ em posição de expressão — novo padrão no parser

`@` em posição de expressão é inédito em Kata. O parser precisa distinguir
`@` como atributo de declaração (antes de assinatura) de `@` como expressão
(dentro de `constant :=`, `echo!(...)`, etc.). A distinção é posicional: o
parser de expressões só vê `@` quando está esperando uma expressão. Se o
parser de declarações já consumiu o `@`, o parser de expressões não o vê.

O único caso ambíguo seria `@` no início de um statement em action body,
onde poderia ser tanto declaração quanto expressão. Mas Kata não tem
statements de expressão livres no top-level (só `constant`, `let`,
declarações). Em action bodies, `@` no início de linha seria tratado como
atributo de declaração pelo parser de declarações.

Mitigação: testes de parser cobrem ambos os contextos.

### 13.2. Walker com missing variant

O walker recursivo sobre `Expr` (48 variantes) e `Item` pode omitir
esconderijos profundos se escrito descuidadamente. Um braço `_ => {}`
compila silenciosamente e pula `EmbedText` dentro de um pattern literal
ou guard.

Mitigação: **zero wildcard** no walker. Braços explícitos para todas as
variantes. O compilador Rust emite E0004 (non-exhaustive match) quando uma
nova variante for adicionada. `desugar.rs` é o gabarito estrutural. Testes
E2E cobrem cada esconderijo (guard, pattern literal, with_binding,
param_default, range, select).

### 13.3. Dois pontos de mutação no TwoPass

O Pipeline em modo TwoPass parseia o módulo duas vezes. `resolve_embeds`
precisa rodar sobre ambos (Pass 1 e Pass 2) para que `quick_resolve` não
veja `Expr::EmbedText` cru em `fixed_value`/`predicate` de enum.

Mitigação: `resolve_embeds` roda sobre `decls_module` (Pass 1) e sobre o
módulo completo (Pass 2). Teste de regressão com `enum` cujo `fixed_value`
usa `@embed`.

### 13.4. Determinismo e builds reprodutíveis

Path absoluto produz binários não-reprodutíveis (depende do layout do
filesystem da máquina). Path relativo é reprodutível.

Mitigação: warning de compilação na v1 para path absoluto. Documentar
que path relativo é o uso recomendado.

### 13.5. Custo de clone de embeds grandes

`ResolvedModule` é `Clone` e é clonado 3+ vezes no pipeline. Um `TextLit`
de 10MB é clonado em cada ponto — custo O(tamanho × clones).

Mitigação: não bloqueante para v1. Se embeds grandes se tornarem comuns,
`Arc<str>`/`Arc<[u8]>` nos literais ou interning resolve o custo. Futuro.