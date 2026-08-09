# TODO: Sistema de mensagens de erro do Kata5

Avaliação do estado atual e plano de correção. Baseado em análise
de `kata-diagnostics`, `kata-resolution`, `kata-comptime`, `kata-codegen`,
`kata-driver/src/pipeline.rs`, `kata-lsp`, e testes no CLI.

## Estado atual (resumo)

| Aspecto | Estado |
|---|---|
| Arquitetura de tipos (Frontend/Middle/Backend) | Correta |
| Códigos namespaced (`type.mismatch`, `parse.unexpected_token`) | Bom |
| miette no frontend/middleend | Funciona — com source context |
| miette em ComptimeError | Não implementado |
| miette em CodegenError | Não implementado |
| miette em LoadError | Tipos estruturados preservados |
| `BackendError` em kata-diagnostics | Dead code — não usado |
| Acumulação de erros (resolve) | Vec, mas join em string |
| Múltiplos erros no pipeline | Aborta no primeiro `?` |
| Spans preservados end-to-end | LoadError preserva span |
| `#[help]` em erros que precisam | 4 de ~35 (UnboundName, NonExhaustive, ArityMismatch, LambdaInference) |
| Prefixos string no pipeline | Mascaram estrutura |
| Source context (linha + indicador) | Implementado via `NamedSource` |
| `FrontendBatch` (wrapper LSP) | Renomeado, documentado |

---

## 1. Source context: exibir linha de erro + indicador ✅

### Problema

Os erros que têm `Span` (FrontendError, MiddleError, ResolveError) já
implementam `miette::Diagnostic` com `#[label]` → `MietteSpan` →
`SourceSpan { offset, len }`. O miette sabe **onde** o erro está.

Mas o miette não mostra a linha de código-fonte porque nenhum
`NamedSource` / `SourceCode` é anexado ao `Report`. Sem o source code,
o miette só pode mostrar:

```
Error: parse.unexpected_token

  × token inesperado: esperado `IntLit`, encontrado `Ident`
```

Com `NamedSource` anexado, o miette renderiza:

```
Error: parse.unexpected_token

  × token inesperado: esperado `IntLit`, encontrado `Ident`
   ╭─[ arquivo.kata:1:1 ]
 1 │ x = 
   ·     ─
   ╰────
```

### Solução

O fluxo atual é:

```
Pipeline::new(source) → lex/parse/resolve/infer (erros com Span)
  → .map_err(IntoReport::into_report)
  → miette::Report::new_boxed(Box::new(error))  // sem source code
```

`IntoReport` (em `kata-driver/src/main.rs:94`) cria o `Report` sem
anexar source code. O `source` está disponível no `Pipeline` (`self.source`)
e o `file_path` é conhecido no caller — mas nenhum dos dois chega ao
`Report`.

**Correção:** `IntoReport::into_report` precisa receber o source code e
file_path para anexar `NamedSource`. Dois caminhos:

**Opção A — `IntoReport` extendido:**

Adicionar um método `into_report_with_source(self, source: &str, file:
Option<&str>)` que cria `NamedSource::new(file.unwrap_or("<eval>"),
source.to_string())` e envolve o erro com
`Report::new(error).with_source_code(named_source)`.

O pipeline já tem `self.source` e `file_path` em cada método. Passá-los
ao converter o erro é trivial.

**Opção B — Pipeline carrega source no erro:**

Fazer o `Pipeline` armazenar `source: Arc<str>` e `file_path:
Option<PathBuf>`, e o `into_report` pegar esses campos. Menos
explícito, mais mágico.

**Recomendado: Opção A.** Explícito, não acopla, e o LSP pode ignorar
(o LSP já extrai span separadamente via `extract_span`).

### Mudanças necessárias

1. `kata-driver/src/main.rs`: estender `IntoReport` com
   `into_report_with_source(self, source, file)`.
2. `kata-driver/src/pipeline.rs`: cada `.map_err(IntoReport::into_report)`
   torna-se `.map_err(|e| e.into_report_with_source(&self.source,
   file_path))`.
3. `kata-driver/src/main.rs`: `cmd_lex`, `cmd_parse`, `cmd_run` já têm
   `source` e `file` — usar a nova versão.
4. Para `cmd_eval` (REPL/eval sem arquivo), passar `None` como file —
   miette mostra `<eval>` como nome.

### Verificação

Após a mudança, `cargo run --bin kata -- run /tmp/test_err.kata` deve
mostrar a linha de código com o indicador `─` apontando para o token
problemático.

---

## 2. Preservar spans end-to-end ✅

### Problema

`LoadError` (em `kata-resolution/src/module_loader.rs:19`) armazena
erros como `String`:

```rust
pub enum LoadError {
    NotFound { path: String },
    LexError(String),      // ← Span perdido
    ParseError(String),   // ← Span perdido
    ResolveError(String), // ← Span perdido
    CircularImport { path: String },
    IoError(String),
}
```

Quando `load_path` encontra um erro de lex, faz:

```rust
let tokens = lex(&source).map_err(|e| {
    LoadError::LexError(format!("{e}"))  // FrontendError → String
})?;
```

O `FrontendError` estruturado (com `MietteSpan`, com código miette) é
convertido em string plana. O span é perdido. Quem recebe `LoadError`
no pipeline não tem como extrair a localização.

### Solução

`LoadError` deve carregar os erros estruturados, não strings:

```rust
pub enum LoadError {
    NotFound { path: String },
    Lex(kata_diagnostics::FrontendError),
    Parse(kata_diagnostics::FrontendError),
    Resolve(Vec<crate::ResolveError>),
    CircularImport { path: String },
    Io(String),
}
```

Isto requer que `kata-resolution` dependa de `kata-diagnostics`. Hoje
a dependência é o oposto: `kata-diagnostics` depende de `kata-ast` para
`Span`, mas `kata-resolution` não depende de `kata-diagnostics`.

**Verificar direção da dep:** `kata-diagnostics` não depende de
`kata-resolution`. Adicionar `kata-resolution → kata-diagnostics` não
cria ciclo. É seguro.

**Alternativa se houver ciclo:** Criar `LoadError` com `Span` (de
`kata-ast`, que já é dependência) em vez do tipo estruturado completo.
Mas isso perde o código miette. Melhor carregar o tipo completo.

### Mudanças necessárias

1. `kata-resolution/Cargo.toml`: adicionar dep `kata-diagnostics`.
2. `kata-resolution/src/module_loader.rs`: trocar `LexError(String)` →
   `Lex(kata_diagnostics::FrontendError)`, etc.
3. `module_loader.rs::load_path`: remover `format!("{e}")`, passar o
   erro diretamente.
4. Quem consome `LoadError` no pipeline (`imports.rs`) precisa adaptar:
   em vez de `format!("{e}")`, usar `into_report_with_source`.

### Bônus

`LoadError` deve implementar `miette::Diagnostic` com códigos
namespaced:

```rust
#[diagnostic(code = "load.not_found")]
NotFound { path: String },

#[diagnostic(code = "load.circular_import")]
CircularImport { path: String },
```

Os variantes `Lex`/`Parse`/`Resolve` delegam ao erro interno (miette
faz isso automaticamente com `#[transparent]`).

---

## 3. Unificar os tipos de erro ✅

### Problema

Existem dois `FrontendError`:
- `kata_diagnostics::FrontendError` — lex + parse estruturado
- `kata_lsp::analysis::FrontendError` — wrapper que envelopa o primeiro
  + `Resolve(Vec<ResolveError>)` + `Infer(MiddleError)`

O wrapper LSP existe só para carregar `ResolveError` junto. Mas
`kata_diagnostics` poderia ter um enum unificado.

### Solução

Criar `kata_diagnostics::KataError` como enum guarda-chuva:

```rust
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum KataError {
    #[error(transparent)]
    Frontend(#[from] FrontendError),

    #[error(transparent)]
    Middle(#[from] MiddleError),

    #[error(transparent)]
    Resolve(#[from] ResolveError),

    // Quando há múltiplos erros de resolução
    ResolveBatch(Vec<ResolveError>),
}
```

Ou, sem criar um enum guarda-chuva, apenas mover o wrapper do LSP para
`kata-diagnostics` como `FrontendBatch`:

```rust
pub enum FrontendBatch {
    Lex(FrontendError),
    Parse(FrontendError),
    Resolve(Vec<ResolveError>),
    Infer(MiddleError),
}
```

O LSP importa de `kata-diagnostics` e perde o wrapper local.

### Recomendação

**Não criar `KataError` guarda-chuva.** Erros guarda-chuva tendem a
ficar um dump de variantes e dificultam o match exaustivo. Manter os
três enums separados (`FrontendError`, `MiddleError`, `ResolveError`)
e mover `FrontendBatch` para `kata-diagnostics` é mais limpo.

O pipeline pode continuar retornando `miette::Result<T>` (erro
type-erased via `Report`). O importante é que os tipos estruturados
existam em um lugar só.

### Mudanças necessárias

1. Mover `FrontendBatch` de `kata-lsp/src/analysis.rs` para
   `kata-diagnostics/src/lib.rs` (ou novo módulo `batch.rs`).
2. `kata-lsp` importa `FrontendBatch` de `kata-diagnostics`.
3. `kata-lsp/src/analysis.rs` perde ~15 linhas do enum wrapper.
4. `kata-resolution` passa a depender de `kata-diagnostics` (já
   necessário para o item 2).

---

## 4. Separar erro do compilador de erro do desenvolvedor Kata

### Problema

A distinção existe na arquitetura mas não é visível para o usuário.
`BackendError` (em `kata-diagnostics/src/backend.rs`) tem o comentário
correto:

> Não carregam Span — não há código do usuário para apontar (I6).

Mas `CodegenError` (em `kata-codegen/src/lowering/module.rs:27`) — o
que é realmente usado — não faz essa distinção:

```rust
pub enum CodegenError {
    FfiSymbolNotFound(String),  // bug do compilador? ou FFI não linkado?
    Cranelift(String),          // bug do compilador
    UnsupportedNode(String),    // pode ser bug do compilador OU TAST inválido
}
```

`UnsupportedNode` é ambíguo: se o type checker produziu um TAST
válido mas o codegen não o suporta, é uma **limitação do compilador**,
não um erro do usuário. Se o TAST é inválido (bug no typeck), é um
**bug interno**. O usuário não deveria ver "nó TAST não suportado" em
nenhum dos casos — deveria ver "esta construção ainda não é suportada"
(limitação) ou "erro interno do compilador" (bug).

### Solução: dois níveis de erro

**Erros do usuário (User-facing):** o desenvolvedor Kata escreveu código
que viola uma regra da linguagem. Carregam `Span`, têm código
namespaced, têm `#[help]` quando aplicável, são renderizados com source
context.

Tipos: `FrontendError`, `MiddleError`, `ResolveError`, e
`LoadError` (depois do item 2).

**Erros internos (Compiler bugs / limitações):** o compilador falhou
por um motivo que não é culpa do usuário. Não carregam `Span` (ou
carregam `Span::synthetic()`). Devem ser claramente identificados como
"erro interno" para que o usuário saiba que não é culpa dele.

Tipos: `BackendError` (já existe, não é usado), `CodegenError`
(reformulado), `ComptimeError` (reformulado).

### Reforma do CodegenError

```rust
// kata-codegen/src/lowering/module.rs

// Erro interno do compilador — bug ou limitação. Nunca é culpa do usuário.
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum CodegenError {
    #[error("construção não suportada no codegen: {node}\nisto é uma limitação do compilador, não um erro no seu código")]
    #[diagnostic(code = "codegen.unsupported", help = "abra uma issue descrevendo o que você estava tentando fazer")]
    UnsupportedNode { node: String },

    #[error("erro interno do Cranelift: {reason}\nisto é um bug do compilador, não um erro no seu código")]
    #[diagnostic(code = "codegen.cranelift", help = "abra uma issue com o código que causou este erro")]
    Cranelift { reason: String },

    #[error("símbolo FFI não encontrado: {symbol}")]
    #[diagnostic(code = "codegen.ffi_not_found", help = "verifique se o runtime foi linkado corretamente")]
    FfiSymbolNotFound { symbol: String },
}
```

### Reforma do ComptimeError

```rust
// kata-comptime/src/error.rs

#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum ComptimeError {
    #[error("expressão não é avaliável em compile-time: {reason}")]
    #[diagnostic(code = "comptime.not_available")]
    NotConsttime { reason: String },

    #[error("expressão é impura: {reason}")]
    #[diagnostic(code = "comptime.impure")]
    Impure { reason: String },

    #[error("erro interno durante JIT em compile-time: {reason}")]
    #[diagnostic(code = "comptime.jit_failure")]
    JitError { reason: String },

    #[error("tipo não suportado em compile-time: {ty}")]
    #[diagnostic(code = "comptime.unsupported_type")]
    UnsupportedType { ty: Ty },
}
```

### Critério de separação

Uma pergunta guia: **"o usuário fez algo errado?"**

- Sim → erro do usuário (FrontendError, MiddleError, ResolveError).
  Renderiza com source context, sugere correção.
- Não → erro interno (CodegenError, ComptimeError, BackendError).
  Renderiza com "isto é um bug do compilador" ou "limitação do
  compilador", pede para abrir issue.

### Mudanças necessárias

1. `CodegenError`: adicionar `thiserror::Error` + `miette::Diagnostic`
   derive, códigos namespaced, `#[help]`.
2. `ComptimeError`: adicionar `thiserror::Error` + `miette::Diagnostic`
   derive, códigos namespaced.
3. `pipeline.rs`: parar de mascarar com `format!("erro de codegen:
   {e}")` — passar o erro diretamente via `into_report_with_source`.
4. Ativar o `BackendError` dead code ou removê-lo (ver item 6).

---

## 5. Melhorar qualidade das mensagens ✅

### Problema

Apenas `LambdaInferenceFail` tem `#[help]`. Erros que beneficiariam
muito de ajuda contextual:

- `UnboundName` — não sugere "você quis dizer X?"
- `NoOverload` — não lista as sobrecargas disponíveis
- `NonExhaustiveMatch` — carrega `missing: Vec<String>` mas não sugere
  a sintaxe do `otherwise`
- `ArityMismatch` — não mostra a assinatura esperada
- `TypeMismatch` — não mostra os tipos de forma legível

### Solução

Adicionar `#[help]` seletivamente nos erros onde o contexto ajuda. O
miette renderiza `#[help]` como texto separado abaixo do erro:

```
Error: type.unbound_name

  × nome `foo` não está no escopo
  help: você quis dizer `for`? (nomes similares no escopo atual: for, foo_bar)
```

### Priorização

**Alta** (maior impacto no DX, fácil de implementar):

1. **`UnboundName`** — adicionar campo `suggestions: Vec<String>` e
   `#[help]` com "você quis dizer X?". A geração de sugestões pode ser
   simples: para cada nome no escopo, calcular distância de Levenshtein
   (ou simples prefix match) e retornar os 3 mais próximos.

2. **`NonExhaustiveMatch`** — já tem `missing: Vec<String>`. Adicionar
   `#[help]` com "variantes faltantes: X, Y, Z. Adicione um caso para
   cada uma ou use `otherwise:` como fallback".

3. **`ArityMismatch`** — adicionar campo `signature: Option<String>`
   carregando a assinatura esperada. `#[help]` com "assinatura
   esperada: `{signature}`".

**Média** (útil, mais complexo):

4. **`NoOverload`** — adicionar campo `available: Vec<String>` com as
   sobrecargas existentes. `#[help]` com "sobrecargas disponíveis para
   `{name}`: ...".

5. **`TypeMismatch`** — o erro já mostra `expected` e `found`, mas
   poderia adicionar `#[help]` com explicação de por que os tipos são
   incompatíveis. Mais complexo pois exige raciocínio sobre a
   hierarquia de interfaces.

**Baixa** (nice-to-have):

6. **`UnexpectedToken`** — adicionar `#[help]` com exemplo de sintaxe
   válida para o contexto.

### Mudanças necessárias

1. Adicionar campos aos enums em `kata-diagnostics/src/frontend.rs` e
   `middleend.rs`.
2. Atualizar os call sites que produzem esses erros (em
   `kata-parser`, `kata-inference`, `kata-resolution`) para
   preencher os novos campos.
3. Adicionar `#[help]` nas variantes.

---

## 6. Eliminar prefixos string no pipeline

### Problema

`pipeline.rs` mascara erros estruturados com `format!`:

```rust
.map_err(|e| err(format!("erro de codegen: {e}")))
.map_err(|e| err(format!("erro de codegen AOT: {e}")))
.map_err(|e| err(format!("erro de comptime: {e}")))
.map_err(|e| err(format!("erro ao carregar prelude: {}")))
.map_err(|e| err(format!("erro de resolução: {}")))
```

`err()` = `miette::Report::msg(string)` — cria um diagnóstico sem
código, sem label, sem help. O erro original (que tinha código miette,
que tinha span) vira uma string plana.

### Solução

Após os itens 1-4 (CodegenError e ComptimeError implementam
`miette::Diagnostic`), o pipeline não precisa mais formatar strings:

```rust
// Antes:
.map_err(|e| err(format!("erro de codegen: {e}")))

// Depois:
.map_err(|e| e.into_report_with_source(&self.source, file_path))
```

Para erros que vêm em `Vec` (resolve, load), criar múltiplos `Report`
em vez de join com `; `:

```rust
// Antes:
.map_err(|e| err(format!("erro de resolução: {}", format_err_vec(&e))))

// Depois (múltiplos erros):
// Retornar o primeiro como Report principal e os demais como
// related/notes, OU coletar todos e imprimir separadamente.
```

O miette suporta `Report::new(error)` que pode carregar
`#[related]` para múltiplos erros. Alternativamente, o pipeline pode
retornar `Result<_, Vec<Report>>` e o driver imprime cada um.

### Mudanças necessárias

1. Após CodegenError/ComptimeError terem `miette::Diagnostic`, trocar
   cada `err(format!(...))` por `into_report_with_source`.
2. Para `Vec<ResolveError>`: parar de fazer `join("; ")` e tratar como
   múltiplos reports.
3. Remover `format_err_vec` e `format_error_vec` (mortos após a
   mudança).
4. Os erros de "prelude não carregou" e "erro de I/O" que são genuinamente
   sem estrutura podem continuar como `Report::msg` — mas adicionar
   código miette manual se fizer sentido:
   `miette::Report::msg(...).with_code("load.prelude")`.

---

## 7. Múltiplos erros por passada ✅ (curto prazo)

### Problema

O pipeline usava `?` em cada passo — aborta no primeiro erro. Se o
usuário tem 3 erros de parse e 2 erros de tipo, vê só o primeiro.

### Solução

**Curto prazo (baixo esforço) — ✅ Concluído:**

- **Parse recovery:** `parse_with_recovery` / `parse_with_arity_recovery`
  retornam `(Module, Vec<FrontendError>)`. O pipeline usa esses, retorna
  `Err(Vec<Report>)` se há erros. O driver imprime todos via
  `print_pipeline_errors` (1 erro → formato direto; múltiplos → cada
  um + resumo).
- **Lex recovery:** `lex_with_recovery` retorna
  `(Vec<TokenWithSpan>, Vec<FrontendError>)`. Recovery skipa até `\n`
  ou `;` (pontos de sincronização). `process_indent` refactorizado para
  pre-check (não muta pilha antes de validar). `lex()` é wrapper sobre
  `lex_with_recovery` — callers existentes (LSP, REPL, testes) sem
  mudança.
- **Pipeline:** `PipelineResult<T> = Result<T, Vec<Report>>`. Cada
  fase é all-or-nothing: se lex ou parse tem erros, aborta com todos
  os erros da fase. Não continua parcialmente entre fases.

**Médio prazo (não implementado):**

Inferência poderia acumular erros de tipo (em vez de abortar no
primeiro). `infer_module` retorna `Result<TypedModule, MiddleError>`
— mudar para `Result<TypedModule, Vec<MiddleError>>` permite reportar
múltiplos erros de tipo.

Mas inferência com erros acumulados é difícil: um erro de tipo no
início pode causar erros em cascata. Estratégias:
- **Abortar após N erros** (ex: 10) para evitar cascata.
- **Error recovery**: após um erro de tipo, usar `Ty::Unknown` e
  continuar. Erros que decorrem do `Unknown` são suprimidos.

Isto é trabalho de fio, não zeladoria — fora do escopo deste TODO.

---

## Ordem de execução recomendada

| Ordem | Item | Esforço | Risco | Impacto no DX | Status |
|---|---|---|---|---|---|
| 1 | #1 Source context | Médio | Baixo | Alto | ✅ Concluído |
| 2 | #4 Separar compilador/usuário | Médio | Baixo | Médio | Pendente |
| 3 | #6 Eliminar prefixos string | Baixo | Baixo | Médio | Pendente |
| 4 | #5 `#[help]` nos erros prioritários | Médio | Baixo | Alto | ✅ Concluído |
| 5 | #2 Preservar spans em LoadError | Médio | Médio | Médio | ✅ Concluído |
| 6 | #3 Unificar FrontendBatch | Baixo | Baixo | Baixo | ✅ Concluído |
| 7 | #7 Múltiplos erros | Alto | Alto | Alto | ✅ Concluído (curto prazo) |

Itens 1-3 podem ser feitos em paralelo (toucham arquivos diferentes).
Item 6 depende de 2 (precisa da dep `kata-resolution →
kata-diagnostics`).

## Dependências entre itens

```
#1 (source context)     ← independente
#2 (LoadError span)    ← independente, mas #6 depende dele
#3 (unificar batch)    ← depende de #2 (dep kata-resolution → kata-diagnostics)
#4 (separar comp/user) ← independente, mas #6 depende dele (CodegenError miette)
#5 (#[help])           ← independente
#6 (prefixos string)   ← depende de #4 (CodegenError/ComptimeError com miette)
#7 (múltiplos erros)   ← independente, mas beneficia de #1
```

## Notas

- `miette` está com feature `fancy` no workspace (`Cargo.toml:29`),
  o que habilita o handler gráfico. Sem `set_hook`, o miette usa o
  handler default que já é o graphical com `fancy`. Mas sem
  `NamedSource` anexado, não há source code para renderizar.
- `Span` (`kata-ast/src/span.rs`) já carrega `offset`, `line`, `col`,
  `len` — toda a informação necessária existe. O gap está só no
  anexo de source code ao `Report`.
- `BackendError` em `kata-diagnostics/src/backend.rs` é dead code com
  `#![allow(dead_code)]`. Após reformular `CodegenError` (item 4),
  decidir: fundir com `CodegenError` ou remover.