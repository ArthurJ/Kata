# PRD — LSP para Kata Language

**Status:** 🔨 Implementação (Fases 1-3 ✅, Fase 4 pendente)
**Data:** 2026-07-28
**Depende de:** Fio 1 ✅ (lexer, parser, inference), Fio 10 ✅ (módulos, imports)

## 1. Problema

O compilador Kata tem um pipeline front-end completo (lex → parse → resolve → infer)
que produz diagnósticos com spans precisos e tipagem rica (TAST). Mas não há editor
integration — o usuário escreve `.kata` sem feedback de tipos, sem hover, sem go-to-def.

A infraestrutura já existe:
- `Span { offset, line, col, len }` em todo nó da AST/TAST
- `FrontendError` e `MiddleError` com códigos namespaced (`type.mismatch`,
  `parse.unexpected_token`, etc.) e spans via `MietteSpan`
- `TypedModule` / `TypedExpr` — TAST com tipos resolvidos em cada nó
- `ResolvedModule` — tabela de símbolos com spans (para go-to-def futuro)
- `TypeEnv` — árvore de escopos (para completion futuro)
- `imports::load_module_imports` — resolução de imports multi-arquivo

O que falta: um servidor LSP que conecte essa infraestrutura ao editor.

## 2. Objetivo

Construir `kata-lsp`, um crate novo que expõe o front-end do compilador como servidor
LSP via `tower-lsp`. MVP entrega:

1. **Diagnósticos** em tempo real (lex + parse + resolve + infer) — `textDocument/publishDiagnostics`
2. **Hover de tipos** — `textDocument/hover` mostra o tipo inferido da expressão sob o cursor

O LSP roda o pipeline **front-end apenas** (sem codegen, sem JIT, sem monomorph).
É rápido e puro — nenhum side-effect, nenhum runtime.

## 3. Arquitetura

### 3.1. Novo crate: `kata-lsp`

```
crates/
  kata-lsp/          ← NOVO
    Cargo.toml
    src/
      lib.rs         — entry point, LSP server setup
      server.rs      — tower-lsp LanguageServer impl
      state.rs       — DocumentStore (estado por arquivo)
      analysis.rs    — front-end pipeline reusável (lex → parse → resolve → infer)
      hover.rs       — busca tipo no TAST por posição
      diagnostics.rs — converte FrontendError/MiddleError → LSP Diagnostic
      unicode.rs     — conversão byte offset ↔ LSP Position (UTF-16)
```

### 3.2. Dependências

```toml
[dependencies]
tower-lsp = "0.20"
tokio = { version = "1", features = ["full"] }
# Internal
kata-ast = { path = "../kata-ast" }
kata-lexer = { path = "../kata-lexer" }
kata-parser = { path = "../kata-parser" }
kata-resolution = { path = "../kata-resolution" }
kata-inference = { path = "../kata-inference" }
kata-diagnostics = { path = "../kata-diagnostics" }
kata-core = { path = "../kata-core" }
```

Não depende de: `kata-codegen`, `kata-optimizer`, `kata-monomorph`, `kata-tree-shaking`,
`kata-rt`, `kata-driver`.

### 3.3. Pipeline reusável

O driver atual tem `run_pipeline_with_file` que mistura front-end + codegen. O LSP
precisa só do front-end. Fatorar a parte frontal em uma função compartilhada:

```rust
/// Resultado do front-end — tudo que o LSP precisa.
pub(crate) struct FrontendResult {
    pub module: Module,           // AST
    pub resolved: ResolvedModule, // símbolos resolvidos
    pub typed: TypedModule,       // TAST com tipos
}

/// Roda lex → parse → resolve → infer (sem codegen).
/// Reusada por LSP e (futuramente) por driver.
fn run_frontend(source: &str, file_path: Option<&str>) -> Result<FrontendResult, Vec<Diagnostic>> {
    // 1. Lex
    let tokens = lex(source).map_err(|e| vec![to_diagnostic(e)])?;

    // 2. Parse
    let module = parse(tokens).map_err(|e| vec![to_diagnostic(e)])?;

    // 2a. Imports (se file_path)
    let imports = if let Some(file) = file_path {
        imports::load_module_imports(file, &module)?
    } else { Vec::new() };

    // 3. Resolve
    let prelude = load_prelude()?;
    let user = resolve(&module)?;
    let mut resolved = merge_resolved(prelude, user);
    imports::merge_imports(&mut resolved, &imports);

    // 4. Infer
    let typed = infer_module(&module, &resolved)?;

    Ok(FrontendResult { module, resolved, typed })
}
```

**Opção de localização:** `run_frontend` pode viver em:
- **A) `kata-lsp`** — o LSP é o único consumidor hoje; o driver tem `run_pipeline_with_file`
  que não fatora o front-end. Duplicação mínima (~15 linhas).
- **B) `kata-driver` como módulo pub** — o LSP depende de `kata-driver` para reusar.
  Mas `kata-driver` puxa `kata-codegen` + `kata-rt` como deps transitivas, defeating
  o propósito de não depender de codegen.
- **C) Novo crate `kata-frontend`** — extrair front-end puro do driver. Mais limpo
  arquiteturalmente, mas adiciona um crate para 1 função.

**Decisão: Opção A.** `run_frontend` vive em `kata-lsp/src/analysis.rs`. Se um segundo
consumidor surgir (REPL querendo reusar front-end sem recompilar codegen), refatorar
para Opção C. Evitar premature abstraction.

### 3.4. DocumentStore — estado por arquivo

```rust
/// Estado de um documento aberto no editor.
struct Document {
    uri: Url,
    version: i32,
    text: String,
    // Resultado do último analysis pass (None se houve erro)
    analysis: Option<FrontendResult>,
    // Diagnósticos do último pass
    diagnostics: Vec<Diagnostic>,
}

/// Estado global do servidor — map de URI → Document.
struct DocumentStore {
    docs: HashMap<Url, Document>,
}
```

### 3.5. Fluxo de eventos

```
Editor                          kata-lsp
  │                                │
  ├── didOpen(uri, text) ────────►│  analysis(text) → FrontendResult
  │                                │  publishDiagnostics(uri, diags)
  │◄── publishDiagnostics ────────┤
  │                                │
  ├── didChange(uri, changes) ───►│  apply changes → new text
  │                                │  analysis(new_text) → FrontendResult
  │                                │  publishDiagnostics(uri, diags)
  │◄── publishDiagnostics ────────┤
  │                                │
  ├── hover(uri, position) ─────►│  find_expr_at(typed, position)
  │                                │  return type string
  │◄── hover result ──────────────┤
  │                                │
  └── didClose(uri) ─────────────►│  remove from store
```

### 3.6. Debounce

O editor pode enviar múltiplos `didChange` em rápida sucessão (typing). O LSP
precisa de debounce para não re-rodar o pipeline a cada keystroke.

Estratégia: canal Tokio com `tokio::time::sleep`. Cada `didChange` envia a URI
para um canal; o worker aguarda 100ms de silêncio antes de rodar analysis. Se
uma nova change chega durante a espera, reset o timer.

```
didChange → tx.send(uri)
worker loop:
  uri = rx.recv()
  sleep(100ms)
  while rx.try_recv().is_ok() { sleep(100ms) }  // drain pending, re-wait
  analysis(docs[uri])
  publish_diagnostics(uri)
```

## 4. Features do MVP

### 4.1. Diagnósticos (`textDocument/publishDiagnostics`)

Converte `FrontendError` e `MiddleError` para `lsp_types::Diagnostic`:

```rust
fn to_diagnostic(error: impl miette::Diagnostic) -> Diagnostic {
    Diagnostic {
        range: span_to_range(error.span()),  // Span → LSP Range
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(error.code().map(|c| c.to_string()).unwrap_or_default()),
        source: Some("kata".into()),
        message: error.to_string(),
        ..Default::default()
    }
}
```

**Span → LSP Range:** LSP usa 0-indexed line/character. Span usa 1-indexed line/col
em bytes. Conversão:

```rust
fn span_to_range(span: Span, text: &str) -> Range {
    let start_line = span.line.saturating_sub(1);
    let start_col = byte_col_to_char_col(text, span.line, span.col);
    // Para spans de comprimento 0 (erros sintéticos), usar end = start
    let end_line = start_line; // assumindo single-line span (comum)
    let end_col = start_col + span.len;
    Range {
        start: Position { line: start_line, character: start_col },
        end: Position { line: end_line, character: end_col },
    }
}
```

**Atenção — Unicode:** Span col é em bytes (`col += ch.len_utf8()` no lexer). O LSP
spec usa **UTF-16 code units** para `Position.character` (não codepoints, não
bytes). A conversão é não-trivial:

- ASCII: 1 byte = 1 codepoint = 1 UTF-16 code unit — idêntico
- BMP (áéí, ç, ñ): 2 bytes = 1 codepoint = 1 UTF-16 code unit — diverge byte vs LSP
- Supplementary (emoji, CJK raros): 4 bytes = 1 codepoint = **2 UTF-16 code units**
  (surrogate pair) — diverge em ambos os eixos

A conversão precisa do texto fonte completo. Duas funções são necessárias desde
a Fase 1:

```rust
/// Byte offset (do início do arquivo) → LSP Position (0-indexed line, UTF-16 char).
/// Usada para converter Span de diagnósticos → LSP Range.
fn byte_offset_to_lsp_position(text: &str, byte_offset: usize) -> Position {
    let offset = byte_offset.min(text.len());
    let prefix = &text[..offset];
    let line = prefix.matches('\n').count();             // 0-indexed
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_prefix = &text[line_start..offset];
    let character = line_prefix.encode_utf16().count();   // UTF-16 code units
    Position { line: line as u32, character: character as u32 }
}

/// LSP Position (0-indexed line, UTF-16 char) → byte offset.
/// Usada para converter cursor do editor → offset no TAST para hover.
fn lsp_position_to_byte_offset(text: &str, pos: Position) -> usize {
    let line = pos.line as usize;
    let target_utf16 = pos.character as usize;
    let mut line_start = 0;
    for _ in 0..line {
        match text[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..].find('\n')
        .map(|i| line_start + i).unwrap_or(text.len());
    let mut utf16_acc = 0;
    let mut byte_pos = line_start;
    for c in text[line_start..line_end].chars() {
        if utf16_acc >= target_utf16 { break; }
        utf16_acc += c.len_utf16();
        byte_pos += c.len_utf8();
    }
    byte_pos
}
```

Estas funções são O(n) no pior caso (n = tamanho da linha), mas amortizam
bem: cada chamada percorre só a linha relevante, não o arquivo inteiro.
Para arquivos Kata típicos (< 500 linhas, linhas < 100 chars), o custo é
negligenciável.

**Span → Range final:**

```rust
fn span_to_range(text: &str, span: Span) -> Range {
    let start = byte_offset_to_lsp_position(text, span.offset);
    let end = byte_offset_to_lsp_position(text, span.offset + span.len);
    Range { start, end }
}
```

**Nota sobre `positionEncoding`:** LSP 3.17 permite o servidor declarar
`positionEncoding: "utf-8"` e receber positions em byte offsets, eliminando
a conversão. Mas nem todos os clientes suportam (VSCode sim, Neovim/lspconfig
depende da versão). Para compatibilidade máxima, implementamos UTF-16
(default do spec). Negociar UTF-8 é otimização futura.

**Severidades:**
- Erros léxicos (`lex.*`) → ERROR
- Erros sintáticos (`parse.*`) → ERROR
- Erros de tipo (`type.*`) → ERROR
- Recursão em action (`action.recursive`) → ERROR
- Não há warnings no front-end atual — todos são hard errors

### 4.2. Hover de tipos (`textDocument/hover`)

Dada uma posição no texto, encontrar o nó `TypedExpr` no TAST cujo span contém
a posição, e retornar o tipo inferido.

```rust
fn hover_at(typed: &TypedModule, pos: Position, text: &str) -> Option<Hover> {
    let offset = lsp_position_to_byte_offset(text, pos);  // UTF-16 → byte
    // Busca em profundidade no TypedModule — encontra o TypedExpr
    // cujo span contém offset, com maior especificidade (menor span)
    let expr = find_typed_expr_at(typed, offset)?;
    let ty_str = format_type(&expr.ty);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```kata\n{}\n```", ty_str),
        }),
        range: Some(span_to_range(expr.span, text)),
    })
}
```

**Busca no TAST:** O `TypedModule` contém items, cada item pode conter expressões
tipadas. A busca é uma traversal que:
1. Itera items do módulo
2. Para cada item com expressão (action body, let binding, etc.), faz traversal
   recursiva na `TypedExpr`
3. Encontra o nó cujo span contém o offset, com maior profundidade (mais específico)

**Formato do tipo:** Usar o `Display` impl de `Ty` (já existe em `kata-core`).
Para tipos paramétricos (`List(Int)`, `Result::(T, E)`), o Display já formata
corretamente.

## 5. Protocolo LSP — Implementação

### 5.1. Capacidades declaradas (`initialize`)

```json
{
  "capabilities": {
    "textDocumentSync": {
      "openClose": true,
      "change": 1,  // Full document sync (mais simples, suficiente para MVP)
      "save": false
    },
    "hoverProvider": true,
    "diagnosticProvider": {
      "interFileDependencies": false,
      "workspaceDiagnostics": false
    }
  }
}
```

**Full sync (change: 1):** O editor envia o texto completo a cada `didChange`.
Simplifica a implementação — não precisamos aplicar incremental edits. O custo
de re-analisar o arquivo inteiro é baixo para a maioria dos arquivos Kata.

### 5.2. CLI

O binário `kata-lsp` é um servidor stdio:

```rust
#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(KataLsp::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
```

Configuração no editor (ex: Neovim, VSCode):

```json
{
  "languageserver": {
    "kata": {
      "command": "kata-lsp",
      "filetypes": ["kata"],
      "rootPatterns": [".git", "Cargo.toml"]
    }
  }
}
```

### 5.3. Multi-arquivo: resolução de imports

Kata tem `import`/`export` (Fio 10). Quando o usuário edita `foo.kata` que faz
`import bar`, o LSP precisa resolver `bar.kata` para dar diagnósticos corretos.

Reuso direto: `imports::load_module_imports(file_path, &module)` do driver.
Esta função descobre e carrega módulos importados a partir do filesystem.

**Limitação do MVP:** Se `bar.kata` também está aberto no editor com mudanças
não salvas, o LSP usa a versão do disco (não a versão em memória do editor).
Isso é uma limitação aceitável para MVP — a versão do disco é consistente e
determinística. Suporte a "dirty buffers" (usar texto em memória dos arquivos
importados abertos no editor) é uma extensão futura.

## 6. Fases de Implementação

### Fase 1: Esqueleto + DocumentStore + diagnósticos + conversão Unicode ✅
- Criar crate `kata-lsp` com `Cargo.toml` + workspace
- Implementar `server.rs` com tower-lsp: `initialize`, `initialized`, `didOpen`,
  `didChange`, `didClose`
- Implementar `analysis.rs`: `run_frontend` (lex → parse → resolve → infer)
- Implementar `diagnostics.rs`: conversão `FrontendError`/`MiddleError` → `Diagnostic`
- Implementar `unicode.rs`: `byte_offset_to_lsp_position` e
  `lsp_position_to_byte_offset` (UTF-16 code units, com testes para ASCII, BMP,
  e supplementary planes)
- Implementar `state.rs`: `DocumentStore` com debounce worker
- Adicionar `kata-lsp` ao workspace `Cargo.toml`
- **DoD:** Abrir um `.kata` no editor mostra erros de sintaxe e tipo em tempo real.
  Editar o arquivo re-publica diagnósticos após debounce.
  Arquivo com ` Olá` em string literal: diagnóstico aponta para a posição correta.

### Fase 2: Hover de tipos ✅
- Implementar `hover.rs`: busca no TAST por posição
- Implementar conversão `Position` → `offset` (char → byte mapping)
- Registrar `hoverProvider` nas capabilities
- **DoD:** Hover sobre `+ 1 2` mostra `Int`. Hover sobre `x` num `let x := 5` mostra
  `Int`. Hover sobre lambda mostra tipo da função `(Int -> Int)`.

### Fase 3: Multi-arquivo (imports) ✅
- Integrar `ModuleLoader` de `kata-resolution` no `run_frontend` (reimplementação
  de `merge_imports` em `kata-lsp/src/analysis.rs` — `kata-driver::imports` é `pub(crate)`)
- `ResolveError` em `kata-resolution` agora deriva `Error` + `miette::Diagnostic`
  (códigos `resolve.*`) para integração com diagnósticos do LSP
- **DoD:** Arquivo com `import foo` recebe diagnósticos que referenciam tipos
  de `foo.kata` corretamente. Erro se `foo.kata` não existe é reportado.

### Fase 4: Polimento
- Error recovery: se parse falha, ainda tentar dar diagnósticos parciais
  (lexer sempre produz tokens; parser pode produzir AST parcial)
- Performance: medir latência do analysis pass em arquivos de exemplo
- Adicionar `kata-lsp` ao `kata` CLI como subcomando `kata lsp` (opcional — pode
  ser binário separado)
- **DoD:** Latência < 100ms para arquivos < 500 linhas. Parser error não derruba
  o servidor.

## 7. Decisões de Design

| Decisão | Escolha | Razão |
|---|---|---|
| Framework | `tower-lsp` | Padrão de facto em Rust, async, bem documentado |
| Sync mode | Full document sync | Simples, suficiente para MVP; evita lógica de incremental edit |
| Debounce | 100ms via canal Tokio | Evita re-análise a cada keystroke; simples |
| Localização do front-end | `kata-lsp/src/analysis.rs` | Único consumidor hoje; evitar premature abstraction |
| Multi-arquivo (MVP) | Usa versão do disco | Determinístico; dirty buffers é extensão futura |
| Codegen | Excluído do LSP | LSP não precisa de execução; pipeline puro |
| Binário | Separado (`kata-lsp`) | LSP é processo de longa duração, diferente do CLI |
| Unicode em spans | Byte→UTF-16 code unit | LSP spec é UTF-16; Span é bytes; conversão O(n) por linha |
| `positionEncoding` | UTF-16 (default spec) | Compatibilidade máxima; negociar UTF-8 é futuro |

## 8. O Que Não Muda

- **Lexer** (`kata-lexer`): reusado as-is, nenhuma mudança
- **Parser** (`kata-parser`): reusado as-is
- **Resolution** (`kata-resolution`): `ResolveError` agora deriva `Error` + `miette::Diagnostic` (códigos `resolve.*`). Restante reusado as-is.
- **Inference** (`kata-inference`): reusado as-is
- **Diagnostics** (`kata-diagnostics`): reusado as-is (FrontendError, MiddleError)
- **AST** (`kata-ast`): reusado as-is (Span, Token, Expr, Module)
- **Driver** (`kata-driver`): sem mudança (LSP é crate separado)
- **Codegen/RT/Optimizer**: não referenciados pelo LSP

## 9. Riscos e Mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| Latência em arquivos grandes | Editor lento | Medir com Fase 4; se > 200ms, considerar incremental lex/parse |
| Parse error recovery | AST parcial = sem hover | Parser já recupera parcialmente em alguns casos; melhorar recovery é trabalho de fio, não do LSP |
| Import circular | Lock no analysis | `load_module_imports` já tem cycle detection (Fio 10) |
| Unicode em spans | Hover/diagnostics em posição errada | Pré-calcular byte→char map; testar com acentos |
| tower-lsp versão | Breaking changes | Pin major version; testar antes de upgrade |
| Threading model | Deadlock entre debounce worker e request handler | tower-lsp é async; usar `tokio::spawn` para debounce, `Mutex` para DocumentStore |

## 10. Evolução Futura (Não Escopo)

- **Go-to-def** (`textDocument/definition`): usar `ResolvedModule` para saltar
  à declaração do símbolo sob o cursor. A tabela de símbolos já carrega spans.
- **Completion** (`textDocument/completion`): combinar `TypeEnv` (escopos) com
  `ResolvedModule` (símbolos globais) para autocompletar nomes. Type-directed
  completion (sugerir apenas funções que aceitam o tipo esperado) é mais avançado.
- **Document symbols** (`textDocument/documentSymbol`): iterar `Module.items`
  para listar actions, data, enum, interfaces do arquivo.
- **Semantic tokens** (`textDocument/semanticTokens/full`): colorir tipos,
  keywords, funções, variáveis baseado na AST/TAST.
- **Inlay hints** (`textDocument/inlayHint`): mostrar tipos inferidos inline
  em bindings sem anotação (`let x := 5` → `let x: Int := 5`).
- **Cross-file hover**: hover sobre símbolo importado mostra tipo do módulo
  importado. Precisa de cache de TAST por módulo.
- **Format on save** (`textDocument/formatting`): chamar `cargo fmt`-equivalent
  (indent-sensitive pretty printer).
- **Dirty buffer support**: usar texto em memória do editor para arquivos
  importados que estão abertos, em vez da versão do disco.
- **Incremental parsing**: re-parse apenas a região modificada (tree-sitter ou
  incremental parser custom). Reduz latência em arquivos grandes.
- **Workspace symbols** (`workspace/symbol`): buscar símbolos em todo o workspace,
  não só no arquivo atual. Precisa indexar todos os `.kata` do projeto.

## 11. Critérios de Aceitação

1. ✅ `cargo build --workspace` passa com o novo crate incluído
2. ✅ `cargo test --workspace` passa sem regressões (1192 passed, 0 failed, 5 ignored)
3. ⬜ Conectando o LSP no editor (Neovim/VSCode), abrir um `.kata` com erro de
   sintaxe mostra o erro sublinhado com a mensagem correta
4. ⬜ Editar o arquivo e corrigir o erro remove o diagnóstico (após debounce)
5. ⬜ Hover sobre uma expressão aritmética (`+ 1 2`) mostra `Int`
6. ⬜ Hover sobre um `let` binding mostra o tipo inferido
7. ⬜ Hover sobre um lambda mostra o tipo da função
8. ⬜ Arquivo com `import` funciona — diagnósticos consideram tipos do módulo
   importado
9. ⬜ Arquivo com acentos em strings literais (` Olá`, `café`): diagnósticos e
   hover funcionam na posição correta
10. ⬜ Arquivo com emoji em string literal (` 🚀`): hover funciona na posição
    correta (supplementary plane — surrogate pair em UTF-16)

> **Itens 1-2 ✅** (verificação automatizada). **Itens 3-10 ⬜** (testes E2E
> com editor — pendentes para a próxima sessão, ver handoff
> `/tmp/kata5-lsp-handoff.md`).