# PRD — Diretiva `trace` com Args no Site de Aplicação

**Status:** 📝 Proposto
**Data:** 2026-08-12
**Pré-requisito 1:** PRD-diretivas.md (sistema de diretivas customizadas — ✅ implementado)
**Pré-requisito 2:** PRD-stdio-alignment.md (stdin/stdout/stderr como `File` em módulo `stdio`, `log!()` com `File`, remoção de tópicos mágicos — 📋 proposto, NÃO implementado)
**Substitui:** `@log` intrínseco (PRD-fio14-log.md) — Fase 3 deste PRD

## 0. Resumo

Estender o sistema de diretivas customizadas para suportar **args no site de
aplicação** (`@trace{msg: "..."}`) e **variáveis de reflexão em funções puras**
(`_args` operacional). Usando essa infraestrutura, definir a diretiva `trace`
em Kata no stdlib — uma versão flexível do `@log` intrínseco que usa `log!()`
como path único de publicação e `format` + variáveis de reflexão para
composição de mensagem.

Após validação, a diretiva `trace` é renomeada para `log` e a diretiva `@log`
intrínseca é removida do compilador.

## 1. Motivação

O `@log` intrínseco tem três problemas:

1. **Pipeline paralelo.** `LogSpec` → `log_synthesis.rs` → `TypedLogSpec` →
   `inject_log` duplica o que o sistema de diretivas já faz (inlining, reflexão,
   hooks de Enter/Exit). São ~233 linhas de template engine (`parse_template`,
   `parse_placeholder`) que reimplantam `format`.

2. **Reflexão frágil em funções.** O `@log` extrai nomes de params dos patterns
   da **primeira cláusula** (`function_infer.rs:130-143`). Se a primeira
   cláusula é `lambda []:` (pattern `Nil`), nenhum nome está disponível e a
   interpolação `{x}` falha. O sistema de diretivas já tem `_args` como tupla
   posicional, mas `for_function` zera `has_args` — o gap é de infraestrutura,
   não de design.

3. **Acoplamento.** `@log` é a única diretiva intrínseca de instrumentação.
   Toda evolução (novos campos, policies, levels) exige mudança no compilador.
   Diretivas customizadas já provaram que hooks de instrumentação podem ser
   definidos em Kata.

## 2. Design

### 2.1. Args no site de aplicação

Hoje: `@trace` (sem args). O desugar inlinea o body fixo da declaration.

Proposto: `@trace{msg: "entering {}", when: "enter"}`. Os args do site de
aplicação são injetados como `let` bindings no body inlined, antes dos
statements da diretiva.

#### Sintaxe de aplicação

```kata
@trace{msg: "entering {} with {}", when: "enter"}
quicksort :: [Int] => [Int]
lambda []: []
lambda [pivo:resto]:
    + (quicksort menores) [pivo : (quicksort maiores)]
    with
        menores := filter (< _ pivo) resto
        maiores := filter (>= _ pivo) resto
```

#### Args como bindings

Os args nomeados do site de aplicação (exceto `when`, que é consumido como
seletor de overload) viram bindings `let _<key> := <value>` no body inlined:

```
# @trace{msg: "entering {}", when: "enter"} aplicado a quicksort
# Desugaring conceptual:
let _msg := "entering {}"
let _name := "quicksort"
let _arity := 1
let _types := ["List::Int"]
let _return_type := "List::Int"
let _is_action := False
let _args := (__arg_0,)
log!(LogLevel::Info, format _msg (_name, _args))
```

### 2.2. Overloading por args

Cada combinação de args no site de aplicação é uma declaration distinta.
O `when` no site de aplicação seleciona a overload (Enter vs Exit) — mecânica
que já existe por `(when, on)`.

O despacho por args estende `DirectiveKey` com a lista de chaves dos args
do site de aplicação (exclusindo `when`, que já está na chave):

```rust
pub struct DirectiveKey {
    pub name: String,
    pub when: Hook,
    pub on: Target,
    pub arg_keys: Vec<String>,  // NOVO: ["msg"] vs ["msg", "topic"] etc.
}
```

`@trace{msg: "...", when: "enter"}` despacha para a declaration cuja chave é
`(name="trace", when=Enter, on=Any, arg_keys=["msg"])`.

`@trace{msg: "...", when: "enter", topic: "audit"}` despacha para
`(name="trace", when=Enter, on=Any, arg_keys=["msg", "topic"])`.

### 2.3. Diretiva `trace` no stdlib

Definida em `stdlib/core.kata` com overloads separados por target (ver §6
para rationale de pureza):

**Actions** (8 overloads — `policy: "block"` disponível):

```kata
directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args))

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return))

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), _topic)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), _topic)

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), "default", _policy)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), "default", _policy)

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), _topic, _policy)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), _topic, _policy)
```

**Funções** (4 overloads — sem `policy`, `topic` default "log"):

```kata
directive trace{when: Hook::Enter, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _args))

directive trace{when: Hook::Exit, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _return))

directive trace{when: Hook::Enter, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _args), _topic)

directive trace{when: Hook::Exit, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _return), _topic)
```

**Notas:**
- `topic` e `file` podem coexistir — o log publica no canal CSP E escreve no
  arquivo. Overloads com ambos seriam adicionados conforme necessário.
- Default do tópico é `"log"` (não `"default"` — ver PRD-stdio-alignment).
- `file` como arg do site de aplicação exige o PRD-stdio-alignment implementado
  (stdout/stderr como `File`, `log!()` aceitando `File`).

**Decisão D1:** `when` é obrigatório no site de aplicação. O `when` seleciona
Enter vs Exit. Sem `when`, o compilador não sabe qual overload despachar.
Diferente do `@log` intrínseco onde `when` era campo da declaration, aqui `when`
é arg do site de aplicação e seletor de overload simultaneamente.

**Decisão D2:** `msg` é obrigatório no site de aplicação. Sem `msg`, não há
mensagem para logar. Validação em compile-time: se `@trace{when: "enter"}`
sem `msg`, erro.

**Decisão D3:** `level`, `topic`, `policy` são opcionais. A combinação de args
presentes no site seleciona a overload. `level` não consta nos overloads acima
— se desejado, adicionar mais 8 overloads. Para a Fase 1, `level` defaulta
para `LogLevel::Info` (hardcoded no body da diretiva). Fase 2 pode adicionar
overloads com `level`.

### 2.4. Reflexão em funções puras

#### `_args` operacional

Hoje `for_function` zera `param_names` e marca `has_args: false`. Para `_args`
funcionar:

1. `for_function` recebe `param_names` (hoje `Vec<Option<String>>` já existe
   em `FunctionDef` mas é descartado). Marca `has_args: true`.

2. `arg_idents` usa `__arg_{i}` quando o param não tem nome (funções puras
   não nomeiam params na assinatura por design).

3. O desugar injeta `let _args := (__arg_0, __arg_1, ...)` no body da cláusula.

4. A inference precisa de `__arg_{i}` no `TypeEnv` com o tipo posicional
   correto. Hoje `function_infer.rs:147` já define `__param_{i}` — renomear
   para `__arg_{i}` ou usar ambos.

5. O codegen precisa registrar `__arg_{i}` no `var_map` apontando para
   `clause_params[i]` em **cada** bloco de cláusula, não só na primeira.
   Hoje `bind_patterns_to_params` só binda patterns `Ident` da primeira
   cláusula (`function_def.rs:234`). Adicionar registro de `__arg_{i}`
   independente de pattern.

#### `_return` em funções

Já funciona via `synthesize_return_binding` — `let _return := __result`.
O codegen de funções cria `__result` no epílogo quando há hooks de Exit
(`function_def.rs:263-265`). Sem mudanças.

### 2.5. `format` como template engine

O `@log` intrínseco tem `parse_template` que extrai `{expr}` do template e
constrói `Expr::Ident(name)` para cada placeholder. Isso reimplanta `format`.

Com a diretiva `trace`, o body usa `format` diretamente:

```kata
log!(LogLevel::Info, format _msg (_name, _args))
```

O `format` builtin já existe e faz `text_replace_first` em cadeia. O template
`"entering {} with {}"` com args `(_name, _args)` produz a mensagem final.

**D4:** O template usa `{}` (posicional) em vez de `{expr}` (nomeado). Os args
são passados como tupla posicional para `format`. Isto é mais simples e
consistente com `format` existente. O usuário compõe a tupla de args no body
da diretiva — `{_name}` vira `{}` com `_name` na posição correspondente da
tupla.

## 3. Fases

### Fase 1 — Args no site de aplicação + reflexão em funções

**Objetivo:** Fazer `@trace{msg: "...", when: "enter"}` funcionar com
diretivas customizadas, com `_args` operacional em funções.

**Mudanças:**

1. **`DirectiveKey`** (`types.rs`): adicionar `arg_keys: Vec<String>`.
   Atualizar `insert`, `lookup_by_name`, `merge`, `validate_any_conflicts`.

2. **`DirectiveDef`** (`types.rs`): adicionar `param_keys: Vec<String>` —
   os nomes dos args que a declaration espera no site de aplicação
   (extraídos do dict da declaration, excluindo `when` e `on`).

3. **`extract_directive_spec`** (`directives.rs`): extrair `param_keys` dos
   args da declaration. Hoje só aceita `when` e `on` — aceitar chaves
   adicionais como `msg`, `topic`, `policy`, `level` e registrá-las.

4. **`custom_directives`** (`types.rs`): mudar de `Vec<String>` para
   `Vec<CustomDirectiveApp>` onde `CustomDirectiveApp` carrega nome + args
   do site de aplicação:
   ```rust
   pub struct CustomDirectiveApp {
       pub name: String,
       pub args: Vec<Spanned<Expr>>,  // args do site, em ordem
       pub arg_keys: Vec<String>,     // chaves dos args nomeados
   }
   ```

5. **Resolution** (`lib.rs`): ao processar `@trace{msg: "...", when: "enter"}`
   em Sig/ActionDecl, construir `CustomDirectiveApp` com os args. Validar
   que o `arg_keys` do site casa com alguma declaration no registry
   (despacho por args).

6. **Desugar** (`desugar_directives/mod.rs`): antes de inlinear o body,
   injetar `let _<key> := <value>` para cada arg do site de aplicação.

7. **`for_function`** (`reflection.rs`): receber `param_names` de
   `FunctionDef` (ou usar `__arg_{i}` como fallback). Marcar `has_args: true`.

8. **`FunctionDef`** (`types.rs`): o `param_names` já existe em `Item::Sig`
   mas `FunctionDef` não o armazena. Adicionar campo.

9. **Codegen de funções** (`function_def.rs`): registrar `__arg_{i}` no
   `var_map` apontando para `clause_params[i]` em cada bloco de cláusula,
   independente de pattern.

10. **Inference** (`function_infer.rs`): definir `__arg_{i}` no `TypeEnv`
    das cláusulas com o tipo posicional correto.

**DoD:**
- `@trace{msg: "test {}", when: "enter"}` aplicado a função pura com
  diretiva customizada definida pelo usuário funciona.
- `_args` acessível no body da diretiva em funções multi-cláusula.
- Teste E2E: quicksort com `@trace` imprime args na entrada.

### Fase 2 — Definir `trace` no stdlib

**Objetivo:** Tornar `trace` uma diretiva disponível por default.

**Mudanças:**

1. Adicionar as 8 declarations de `trace` (4 overloads × 2 hooks) em
   `stdlib/core.kata`.

2. Garantir que o prelude merge traga as declarations de `trace` no
   `DirectiveRegistry` do módulo usuário.

**DoD:**
- `@trace{msg: "entering {}", when: "enter"}` funciona sem declaration
  local de `trace`.
- `@trace{msg: "result: {}", when: "exit", topic: "audit", policy: "block"}`
  funciona com topic e policy.
- Teste E2E: quicksort com `@trace` stdlib imprime corretamente.

### Fase 3 — Renomear `trace` para `log` e remover `@log` intrínseco

**Objetivo:** Unificar o path de logging.

**Mudanças:**

1. Renomear `trace` → `log` no stdlib.

2. Remover do compilador:
   - `LogSpec` (`types.rs`)
   - `TypedLogSpec` (`typed_module.rs`)
   - `log_synthesis.rs` (arquivo inteiro)
   - `log.rs` no codegen (arquivo inteiro)
   - `extract_log_spec` (`directives.rs`)
   - Campos `log` em `FunctionDef`, `ActionDef`, `TypedFunction`, `TypedAction`
   - Injeção de `@log` em `function_def.rs` e `action_def.rs` (codegen)
   - Síntese de log em `function_infer.rs` e `action_infer.rs`

3. Migrar 14 testes E2E de `log_e2e.rs` para usar `@log{msg: "...", when: "..."}`
   com a nova diretiva Kata.

4. Migrar exemplos (`quicksort.kata`, `log_telemetry.kata`, etc).

5. Atualizar docs: `sintaxe-mapa.md`, `Kata-lang-manual.md`, `PRD-fio14-log.md`
   (marcar como substituído).

**DoD:**
- `@log{msg: "entering {}", when: "enter"}` funciona via diretiva Kata.
- `@log{msg: "result: {}", when: "exit", topic: "audit", policy: "block"}`
  funciona.
- 14 testes E2E migrados e passando.
- Zero referências a `LogSpec`/`TypedLogSpec`/`log_synthesis` no código.
- `cargo test --workspace` passa.

## 4. Decisões

| ID | Decisão | Rationale |
|----|---------|-----------|
| D1 | `when` obrigatório no site de aplicação | Seleciona Enter vs Exit. Sem `when`, despacho ambíguo. |
| D2 | `msg` obrigatório no site de aplicação | Sem `msg`, não há mensagem. Erro compile-time. |
| D3 | `level`/`topic`/`policy` opcionais, despachados por overload | Combinação de args presentes seleciona a declaration. `level` default `Info` na Fase 1. |
| D4 | Template usa `{}` posicional com `format` | Consistente com `format` existente. Remove `parse_template`/`parse_placeholder`. |
| D5 | `trace` é definida no stdlib, não no compilador | Validar que diretivas customizadas são infraestrutura suficiente para logging. |
| D6 | `_args` em funções usa `__arg_{i}` posicional | Funções puras não nomeiam params na assinatura por design. `_args` é tupla posicional dos valores brutos. |
| D7 | `policy: "block"` só em `Target::Action`; funções usam `policy: "drop"` only | Preserva garantia de pureza estrutural. `policy: "block"` pode deadlockar — não é observacional. Impossibilidade estrutural via overloads separados por target. |

## 5. Riscos

1. **Despacho por args pode ser ambíguo.** Se duas declarations têm
   `arg_keys` sobrepostos (uma com `["msg"]`, outra com `["msg", "topic"]`),
   o site `@trace{msg: "...", topic: "..."}` só casa com a segunda. O site
   `@trace{msg: "..."}` só casa com a primeira. Não há ambiguidade — o match
   é exato por conjunto de chaves. Se nenhuma casar, erro
   `NoMatchingDirective`.

2. **`_args` em funções multi-cláusula.** Cada cláusula recebe
   `clause_params[i]` como block param no codegen. O registro de `__arg_{i}`
   precisa acontecer em cada bloco de cláusula. Se `lower_clause_chain`
   cria blocos separados, o `var_map` precisa ser populado em cada um.

3. **Performance de `format` vs template direto.** O `@log` intrínseco
   constrói a cadeia de `text_replace_first` em compile-time via
   `infer_format`. A diretiva `trace` chama `format` que faz o mesmo em
   runtime. Diferença: `format` é uma chamada de função vs inlining do
   template. Aceitável — `log!()` já é uma chamada de FFI.

4. **`log!()` em funções puras:** verificado empiricamente — `log!()`
   já funciona em funções puras. Desugara para FFI direta
   (`kata_rt_log_publish`), não passa pelo scheduler. Não é blocker.

## 6. Pureza estrutural e policy por target

### Contexto das discussões

O PRD-diretivas estabelece a garantia: "O leitor de código Kata5 nunca precisa
auditar uma diretiva para saber se uma função pura é segura." ShortCircuit e
Transform são `Target::Action` only — a garantia de pureza é por impossibilidade
estrutural, não por convenção.

Enter e Exit são `Target::Any` — são observacionais por design. Mas
observacional significa "não muda o comportamento observável da função
decorada". `log!()` com `policy: "drop"` é observacional (fire-and-forget,
não bloqueia). `log!()` com `policy: "block"` **não é observacional** —
deadlock se não há consumidor, a função não retorna.

### Decisão D7

`trace` (e futura `log`) usa overloads separados por target:

- **`Target::Function`**: body usa `log!()` com `policy: "drop"` apenas.
  Fire-and-forget. Não bloqueia. Preserva transparência semântica.
- **`Target::Action`**: body usa `log!()` com `policy: "block"` quando
  solicitado. Pode bloquear — actions já têm semântica de bloqueio
  (canais, scheduler).

A garantia estrutural é preservada: funções puras nunca recebem código que
pode bloquear. O overloading por `(when, on)` já suporta bodies diferentes
por target — é a mecânica existente.

### Declarations do stdlib (revisadas)

```kata
# ── trace para Actions: policy "block" disponível ──

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args))

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return))

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), _topic)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), _topic)

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), "default", _policy)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), "default", _policy)

directive trace{when: Hook::Enter, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _args), _topic, _policy)

directive trace{when: Hook::Exit, on: Target::Action}
    log!(LogLevel::Info, format _msg (_name, _return), _topic, _policy)

# ── trace para Funções: sem policy "block" ──

directive trace{when: Hook::Enter, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _args))

directive trace{when: Hook::Exit, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _return))

directive trace{when: Hook::Enter, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _args), _topic)

directive trace{when: Hook::Exit, on: Target::Function}
    log!(LogLevel::Info, format _msg (_name, _return), _topic)
```

Funções puras têm 4 overloads (sem policy). Actions têm 8 overloads (com
policy). O despacho por `(when, on, arg_keys)` seleciona a declaration
correta automaticamente — `@trace{msg: "...", when: "enter", policy: "block"}`
em função pura não encontra declaration com `Target::Function` que aceite
`policy` → erro `NoMatchingDirective`. A impossibilidade é estrutural.

### `log!()` em função pura

**Verificado empiricamente:** `log!()` dentro de função pura já compila e
executa sem erro. Desugura para `TypedExprKind::Closure { ffi_symbol:
"kata_rt_log_publish" }` — FFI direta, não passa pelo scheduler. O codegen
faz call direto independente de `scheduler_mode`.

O `log!()` é semanticamente uma primitiva pura quando `policy: "drop"`:
publica num canal Broadcast fire-and-forget sem alterar o resultado da
função. Com `policy: "block"` deixaria de ser pura (pode deadlockar), mas
as declarations com `Target::Function` não oferecem policy — a
impossibilidade estrutural garante que funções puras nunca recebem
`policy: "block"`.

**Dependência do PRD-stdio-alignment:** o `file` como arg da diretiva
`trace` (escrever em stdout/stderr/arquivo) exige que o `log!()` action
já aceite `File` como argumento, o que é implementado pelo
PRD-stdio-alignment. Sem esse pré-requisito, a diretiva `trace` só
suporta `topic` (CSP).

## 7. Não-objetivos

- Migrar `@timer` para diretiva Kata (futuro).
- Migrar `@cache` para diretiva Kata (futuro).
- Sintaxe de defaults em declarations de diretiva (`{msg: Text = "..."}`).
- Args posicionais no site de aplicação (`@trace("...")`).