# PRD: mecanismo `#!` — pseudo-comentários extensíveis

Estado-alvo: `#!<token> <payload>` é um pseudo-comentário que o parser
preserva no AST. O primeiro token identifica o consumidor; o payload é
livre. O compilador processa tokens que conhece (`allow`, `warn`, `deny`,
`test`, `deprecated`, `must_use`) e preserva os demais como
`UnknownPragma`. Ferramentas externas podem definir novos pragmas sem
alterar o compilador, usando prefixo obrigatório.

O PRD também define a **distinção `#!` vs `@`** e a **migração de
`@test`** (sem `expects`) para `#!test`.

## Motivação

Kata tem diretivas `@` para extensões de funcionalidade — `@embed`,
`@ffi`, `@log`, `@test`. Mas nem toda anotação é extensão de
funcionalidade. Controle de diagnóstico (`#!allow
type.incomplete_interface`) é metadado externo à lógica do código: o
programa faz a mesma coisa com ou sem o pragma.

Precisamos de um mecanismo para anotações que **não mudam o
comportamento do programa** — só guiam o compilador, o tooling, ou
ferramentas externas. E esse mecanismo precisa ser **extensível**:
ferramentas externas (benchmark, linter, fuzzer) podem definir novos
pragmas sem registrar no compilador.

## Design

### Sintaxe

`#!<token> <payload>` é um pseudo-comentário. Sintaticamente é um
comentário (`#`), mas o `!` sinaliza ao parser para **preservar** o
conteúdo no AST em vez de descartar.

```kata
#!allow type.incomplete_interface
#!test("soma correta")
#!bench-config iterations: 1000, warmup: 100
#!mylint-max_line 80
```

Pode aparecer antes de uma declaração ou inline no fim de uma linha:

```kata
#!test("soma")
action soma(a::Int, b::Int) :: Int => + a b

match x
    Some v: ...
    Some v: ...  #!warn type.redundant_clause  # segundo Some é intencional
    None: ...
```

### Escopo posicional

O escopo do `#!` é determinado pela sua posição:

- **Início de linha** (em linha própria, antes de uma declaração) →
  aplica à **declaração inteira**.
- **Inline** (no fim de uma linha) → aplica à **unidade sintática que
  começa nessa linha** (o método, o braço do match, a cláusula).
- **Em linha própria dentro de um bloco**, entre dois elementos →
  anexa ao **próximo elemento sintático**.

```kata
# Escopo: declaração inteira (todo o implements)
#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Escopo: só o método +
Internal implements NUM
    + :: Internal Internal => Internal  #!allow type.incomplete_interface
    lambda a b: Internal (+ a.val b.val)

# Escopo: só o próximo método (-)
Internal implements NUM
    + :: Internal Internal => Internal

    #!allow type.something
    - :: Internal Internal => Internal
```

Isso habilita controle seletivo por método dentro de um bloco
`implements` — o pragma inline anexa só ao método marcado.

### Dispatch por token

O `<token>` (primeiro token após `#!`) identifica o consumidor. O
`<payload>` (resto da linha) é livre — cada consumidor define seu
próprio formato.

**Conjunto fechado do compilador** (sem prefixo):

| Token | Dono | Payload | Parser faz |
|---|---|---|---|
| `allow` | compilador / diagnóstico | `<diagnostic_code>` | parseia estruturado |
| `warn` | compilador / diagnóstico | `<diagnostic_code>` | parseia estruturado |
| `deny` | compilador / diagnóstico | `<diagnostic_code>` | parseia estruturado |
| `test` | compilador / test runner | `("desc")` ou `{desc, args, timeout}` | parseia estruturado |
| `deprecated` | compilador / diagnóstico | `<message>` (futuro) | parseia estruturado (futuro) |
| `must_use` | compilador / diagnóstico | `<message>` (futuro) | parseia estruturado (futuro) |

**Pragmas externos** (prefixo obrigatório `#!<prefixo>-<resto>`):

| Token | Dono | Payload | Parser faz |
|---|---|---|---|
| `<prefixo>-<resto>` | ferramenta externa | livre | preserva como `UnknownPragma` |

O parser valida: token sem `-` que não está no conjunto fechado →
erro: "unknown pragma `benchmark` — external pragmas must be prefixed,
e.g. `#!mytool-benchmark`".

### Regra de preservação

O parser **sempre preserva** `#!` no AST — nunca descarta. Para tokens
conhecidos, parseia o payload em um nó estruturado
(`DiagnosticControl`, `TestSpec`). Para tokens externos com prefixo,
preserva como `UnknownPragma { token: String, prefix: String, raw: String, span: Span }`.

O resolution, inference, e codegen ignoram `UnknownPragma` — é
comentário para o compilador, mas sobrevive no AST para ferramentas
que consomem AST (LSP, doc generator, formatter).

### Extensibilidade com prefixo obrigatório

Ferramentas externas definem pragmas com prefixo obrigatório
`#!<prefixo>-<resto>`. O prefixo identifica a ferramenta; o parser
preserva o texto; a ferramenta lê do AST e parseia o payload segundo
sua própria convenção.

`#!bench-config iterations: 1000` é comentário para o compilador e
dado para a ferramenta de benchmark (prefixo `bench`).

**Privacidade por prefixo:** a API de consumo
(`pragmas_with_prefix("bench")`) só retorna pragmas com o prefixo
solicitado. Uma ferramenta não enxerga pragmas de outra — cada uma
consome apenas os seus.

**A linha divisória:** o compilador lê o token? → sem prefixo (conjunto
fechado). Só ferramenta externa lê? → prefixo obrigatório.

### Pragma `allow` / `warn` / `deny` — controle de diagnóstico

```kata
# Silencia (ajusta para allow)
#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Emite como warning (compilação continua)
#!warn type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Eleva a erro (explicita o default)
#!deny type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)
```

**Três níveis:**

| Nível | Comportamento |
|---|---|
| `#!allow` | silencia — não emite nada |
| `#!warn` | emite warning, compilação continua |
| `#!deny` | eleva a erro, compilação falha |

**Códigos de diagnóstico:** usam o mesmo namespace dos 38 diagnósticos
existentes (`type.unbound_name`, `parse.unexpected_token`, etc). O
usuário vê o código na mensagem de erro e copia para o pragma. Sem
sistema paralelo de aliases.

**Default e ajustabilidade:** cada diagnóstico no enum de erros
declara seu default via atributo:

```rust
#[diagnostic(code = "type.incomplete_interface", severity = "deny", severity_adjustable = true)]
IncompleteInterface { ... }

#[diagnostic(code = "type.missing_overload", severity = "deny", severity_adjustable = false)]
MissingOverload { ... }
```

- `severity` — `deny` (erro), `warn` (warning), ou `allow` (silenciado
  por padrão). Define o comportamento quando o usuário não escreve
  `#!`.
- `severity_adjustable` — `true` permite `#!` sobrescrever o
  `severity` em qualquer direção; `false` rejeita `#!` com erro.

Os 38 diagnósticos existentes são implicitamente `severity = "deny",
severity_adjustable = false` — comportamento inalterado. Apenas novos
diagnósticos que precisam de controle declaram os atributos.

O usuário só escreve o pragma para **sobrescrever** o default.
`#!deny` existe para explicitar o default em código que precisa ser
claro (ex: biblioteca pública).

**Sem `forbid`:** Kata não tem aninhamento hierárquico de módulos
como Rust (`mod foo { mod bar { ... } }`). Imports são entre
arquivos, não aninhamento. Sem árvore de escopo, não há
"downstream" para proteger contra override. `allow`/`warn`/`deny`
cobre tudo.

### Erros de pragma

**Ajustar diagnóstico não-ajustável** — erro:

```
error: `parse.unexpected_token` is not severity-adjustable
  this diagnostic cannot be adjusted with `#!allow` or `#!warn`
help: adjustable diagnostics in this context:
      type.incomplete_interface, type.missing_overload,
      type.redundant_clause
```

**Código de diagnóstico inexistente** (typo) — erro:

```
error: unknown diagnostic code `type.incomplte_interface`
  did you mean `type.incomplete_interface`?
```

**Pragma redundante** (mesmo nível do default) — warning:

```
warning: `#!deny` matches the default severity of `type.incomplete_interface`
  this pragma has no effect and can be removed
```

### Pragma `test` — marker para o test runner

```kata
#!test("soma correta")
action soma(a::Int, b::Int) :: Int => + a b

#!test{desc: "com args", args: (2, 3), timeout: 5000}
action testa_soma :: Unit => println (soma 2 3)
```

`#!test` marca uma action para o test runner descobrir e executar. A
action faz a mesma coisa com ou sem o marker — é metadado, não
extensão de funcionalidade. Tree shaking remove `#!test` em produção.

`#!test` não suporta `expects`. Verificação ativa de erro (extrair
payload de Result, chamar show, comparar string) gera código no
wrapper — é extensão de funcionalidade, permanece `@test{expects}`.

## Decisões de design

### `#!` como pseudo-comentário, não diretiva `@`

Diretivas (`@`) são extensões de funcionalidade — afetam o tipo, o
dispatch, o código gerado. `#!allow` é metadado externo à lógica do
código: o programa não muda de comportamento com ou sem `#!allow` —
só os diagnósticos mudam.

Pseudo-comentário comunica isso: é um comentário que o compilador lê,
mas não é código. O `!` distingue de comentário comum (`#`).

### Distinção `#!` vs `@`

| Categoria | Mecanismo | Critério | Exemplos |
|---|---|---|---|
| Metadado externo à lógica | `#!` | Programa faz a mesma coisa com ou sem | `#!allow`, `#!test`, `#!deprecated` |
| Extensão de funcionalidade | `@` | Programa faz algo que não faria sem | `@embed`, `@ffi`, `@log`, `@test{expects}` |

**Teste:** remover o pragma, o programa faz a mesma coisa?
- Sim → `#!`. O pragma anota, observa, guia.
- Não → `@`. O pragma adiciona uma capacidade.

### Dispatch por primeiro token, payload livre

O primeiro token após `#!` identifica o consumidor. O compilador
processa os tokens do conjunto fechado (`allow`, `warn`, `deny`,
`test`, `deprecated`, `must_use`) e preserva os demais como
`UnknownPragma`. Cada consumidor define seu próprio formato de
payload — o compilador não valida formato de pragmas que não são seus.

**Prefixo obrigatório para pragmas externos** garante que ferramentas
não colidam: cada ferramenta usa seu prefixo (`#!bench-*`,
`#!mylint-*`). A API de consumo filtra por prefixo, oferecendo
privacidade — uma ferramenta só vê os pragmas com seu prefixo.

Alternativa rejeitada: namespace gerenciado pelo compilador com
registry de pragmas. Exigiria coordenação entre ferramentas externas
e compilador, acoplando extensibilidade ao release cycle do
compilador.

Alternativa rejeitada: tokens sem prefixo para ferramentas externas
(`#!benchmark` livre). Colisão silenciosa entre ferramentas que
querem o mesmo token, sem feedback ao usuário.

### Preservação de pragmas desconhecidos no AST

O parser sempre preserva `#!` no AST — nunca descarta. Para tokens
conhecidos, parseia estruturado. Para externos com prefixo, preserva
como `UnknownPragma { token, prefix, raw, span }`. O custo é mínimo
(um struct + um match arm no parser). O ganho é que ferramentas que
consomem AST (LSP, doc generator) podem acessar pragmas externos sem
re-parsing do fonte.

Alternativa rejeitada: descartar pragmas externos como comentário
comum. Ferramentas externas teriam que re-parsing do fonte. Funciona,
mas perde a infraestrutura de AST já construída pelo LSP.

### Escopo posicional

O escopo do `#!` é determinado pela posição: início de linha →
declaração inteira; inline → unidade sintática da linha; em linha
própria dentro de bloco → próximo elemento sintático. Isso
habilita controle seletivo por método sem sintaxe adicional — a
posição já comunica o escopo.

Alternativa rejeitada: escopo sempre na declaração inteira,
independente de posição. Forçaria o usuário a criar declarações
separadas para controlar diagnósticos em métodos individuais.

### `@test` bifurca: `#!test` para marker, `@test` para assertion

`@test("desc")` sem `expects` é um marker — a action faz a mesma
coisa com ou sem a diretiva. É metadado externo à lógica, pertence
a `#!`.

`@test{desc, expects: "Panic: msg"}` com `expects` gera código —
o wrapper extrai payload de Result, chama show, compara strings,
decide pass/fail. É extensão de funcionalidade, permanece `@`.

### Sem `forbid`

Kata não tem aninhamento hierárquico de módulos. Sem árvore de
escopo, não há "downstream" para proteger contra override.
`allow`/`warn`/`deny` cobre tudo.

## Estruturas afetadas

| Camada | Mudança |
|---|---|
| `kata-parser` | Aceitar `#!<token> <payload>` em declarações (antes e inline). Validar prefixo obrigatório para tokens externos. Preservar `UnknownPragma` no AST. |
| `kata-ast` | Adicionar `UnknownPragma { token, prefix, raw, span }`. Adicionar `DiagnosticControl { level, code, span }`. |
| `kata-resolution` | Extrair `DiagnosticControl` e `TestSpec` de pragmas conhecidos. Armazenar controles no `InterfaceRegistry`/`ImplEntry`. |
| `kata-diagnostics` | Macro procedural `#[diagnostic(code, severity, severity_adjustable)]` para declarar metadados. Erro ao ajustar não-ajustável. Erro em código inexistente. Warning em pragma redundante. |
| `kata-tree-shaking` | Remover `#!test` specs em produção (como hoje com `@test`). |
| `kata-lsp` | Expor `UnknownPragma` em semantic tokens (conhecido, desconhecido, comentário comum) e document symbols. Hover em `#!allow` mostra diagnóstico silenciado. API `pragmas_with_prefix` filtra por prefixo. |
| `kata-driver` | `kata test` descobre `#!test` além de `@test{expects}`. |

## Fases

### Fase 1: Mecanismo `#!` no parser

- Parser aceita `#!<token> <payload>` em declarações (antes e inline)
- Valida prefixo obrigatório para tokens externos
- Tokens conhecidos (`allow`, `warn`, `deny`) → `DiagnosticControl`
- Token `test` → `TestSpec` (reusa parsing atual de `@test`)
- Tokens externos com prefixo → `UnknownPragma { token, prefix, raw, span }`
- `UnknownPragma` anexado à declaração/unidade sintática que o contém
- **DoD:** `#!allow type.incomplete_interface` parseia e produz
  `DiagnosticControl`. `#!bench-config iterations: 1000` parseia e
  produz `UnknownPragma`. `#!benchmark iterations: 1000` (sem
  prefixo) produz erro. Arquivo compila sem erro nos dois primeiros
  casos.

### Fase 2: Metadados de diagnóstico

- Macro procedural `#[diagnostic(code, severity, severity_adjustable)]`
- `severity` pode ser `deny`, `warn`, ou `allow`
- Os 38 diagnósticos existentes permanecem implicitamente
  `deny` + não ajustáveis
- Erro ao ajustar diagnóstico não-ajustável (com sugestão de ajustáveis)
- Erro em código de diagnóstico inexistente (com sugestão de typo)
- Warning em pragma redundante (mesmo nível do default)
- **DoD:** diagnóstico com `severity_adjustable = true` responde a
  `#!allow` (silencia), `#!warn` (warning), `#!deny` (erro).
  Diagnóstico com `severity_adjustable = false` + `#!allow` → erro
  com sugestão. `#!allow type.incomplte_interface` (typo) → erro
  com sugestão. `#!deny` em diagnóstico deny-default → warning de
  redundância.

### Fase 3: Migração `@test` → `#!test`

- `#!test("desc")` e `#!test{desc, args, timeout}` sem `expects` →
  marker puro, processado como `TestSpec`
- `@test{desc, expects, policy}` → permanece diretiva `@`, gera
  wrapper com verificação ativa
- `kata test` descobre ambos: `#!test` (marker) e `@test{expects}`
  (assertion)
- Tree shaking remove `#!test` em produção (como hoje)
- Atualizar exemplos, manual, TM bundle, testes existentes
- **DoD:** `#!test("soma")` roda no `kata test` igual ao
  `@test("soma")` atual. `@test{expects: "Panic"}` permanece
  funcionando como diretiva. Todos os testes E2E existentes passam
  com a nova sintaxe.

### Fase 4: LSP

- Semantic tokens distinguem `#!` conhecido, `#!` externo, e
  comentário comum
- Document symbols incluem pragmas anexados à declaração
- Hover em `#!allow <code>` mostra "silencia diagnóstico `<code>`" +
  link para documentação
- API `pragmas_with_prefix(prefix)` filtra `UnknownPragma` por prefixo
- **DoD:** `#!allow` aparece como semantic token distinto de
  comentário comum. Hover mostra descrição do diagnóstico. LSP não
  expõe pragmas de prefixo que não são seus.

## Testes

- **T1 (pragma conhecido — allow)**: `#!allow type.incomplete_interface`
  antes de `implements` → `DiagnosticControl { level: Allow, code:
  "type.incomplete_interface" }` no AST.
- **T2 (pragma conhecido — warn)**: `#!warn type.redundant_clause`
  → `DiagnosticControl { level: Warn, ... }`.
- **T3 (pragma conhecido — deny)**: `#!deny type.incomplete_interface`
  → `DiagnosticControl { level: Deny, ... }`.
- **T4 (pragma externo preservado)**: `#!bench-config iterations:
  1000` antes de uma declaração → compila sem erro, `UnknownPragma`
  no AST com `token: "bench-config"`, `prefix: "bench"`,
  `raw: "iterations: 1000"`.
- **T5 (pragma externo inline)**: `#!mylint-max_line 80` no
  fim de uma linha de declaração → `UnknownPragma` anexado à
  unidade sintática, span correto.
- **T6 (pragma sem prefixo rejeitado)**: `#!benchmark iterations:
  1000` → erro: "unknown pragma `benchmark` — external pragmas must
  be prefixed".
- **T7 (severity_adjustable = true responde a #!)**: diagnóstico com
  `severity_adjustable = true` + `#!allow` → silencia. + `#!warn` →
  warning. + `#!deny` → erro.
- **T8 (severity_adjustable = false rejeita #!)**: diagnóstico com
  `severity_adjustable = false` + `#!allow` → erro "not
  severity-adjustable" com sugestão de diagnósticos ajustáveis.
- **T9 (código inexistente)**: `#!allow type.incomplte_interface`
  (typo) → erro "unknown diagnostic code" com sugestão
  `type.incomplete_interface`.
- **T10 (pragma redundante)**: `#!deny` em diagnóstico com
  `severity = "deny"` default → warning de pragma redundante.
- **T11 (#!test marker)**: `#!test("soma")` antes de action →
  `kata test` descobre e executa. Mesmo comportamento que
  `@test("soma")` atual.
- **T12 (@test expects permanece diretiva)**:
  `@test{desc: "div zero", expects: "Panic: div"}` → wrapper
  verifica `show(err)` contra `expects` com policy.
- **T13 (#!test sem expects não gera wrapper de verificação)**:
  `#!test("soma")` → wrapper retorna resultado bruto, não extrai
  tag nem compara string.
- **T14 (#!test com args e timeout)**: `#!test{desc: "com args",
  args: (2, 3), timeout: 5000}` → TestSpec com args e timeout
  populados, runner passa args para a action.
- **T15 (código na mensagem)**: instrução de controle na ajuda só
  aparece em diagnósticos ajustáveis. Diagnóstico não ajustável
  não mostra `#!allow`.
- **T16 (escopo posicional — declaração)**: `#!allow` em linha
  própria antes de `implements` → silencia diagnóstico em qualquer
  método do bloco.
- **T17 (escopo posicional — método)**: `#!allow` inline em um
  método → silencia só naquele método, não nos demais.
- **T18 (escopo posicional — próximo elemento)**: `#!allow` em
  linha própria dentro de bloco, antes de um método → anexa ao
  método seguinte, não ao bloco inteiro.
- **T19 (LSP semantic tokens)**: `#!allow` e `#!bench-*` aparecem
  como semantic tokens distintos de comentário comum.
- **T20 (LSP hover)**: hover em `#!allow type.incomplete_interface`
  mostra descrição do diagnóstico silenciado.
- **T21 (LSP filtra por prefixo)**: `pragmas_with_prefix("bench")`
  retorna só pragmas `#!bench-*`, não retorna `#!mylint-*`.

## Fora de escopo

- `#!` em expressões (apenas em declarações).
- `forbid` — sem aninhamento hierárquico, não há downstream.
- Convenção de formato para payloads de ferramentas externas —
  cada ferramenta define seu formato.