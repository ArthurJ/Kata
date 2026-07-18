# PRD — Fio 14: `@test` (Test Runner)

**Status:** Planejamento
**Data:** 2026-07-18
**Depende de:** Fio 4 ✅ (Result), Fio 11 ✅ (Actions, scheduler, JIT)
**Não depende de:** `@parallel` (congelado), `@log` (segunda parte deste fio, adiada para sub-PRD)

## 1. Objetivo

Permitir que programas Kata contenham testes escritos em Kata, descobertos e
executados por um subcomando `kata test`. Os testes são **anotações em actions
existentes** — não são itens de módulo separados.

## 2. Sintaxe

`@test` é uma diretiva que anota uma `action` existente. A action mantém sua
semântica normal (pode ser chamada por `foo!(args)` em código de produção). O
`@test` marca para o runner descobri-la e executá-la com os argumentos
fornecidos na diretiva.

### 2.1. Sintaxe de diretivas — regra geral

Diretivas Kata seguem o modelo:

- `@nome` — sem argumentos
- `@nome(...)` — **tupla**: elementos são `Text`, `Int`, ou literais compostos (tupla, variant)
- `@nome{...}` — **dict**: pares `chave: valor`, valores seguem o mesmo conjunto do `()` — `Text`, `Int`, tupla, variant, ou apply posicional de construtor

O parser entrega `Vec<DirectiveArg>` (tupla) ou `Vec<DirectiveArg::Named>` (dict).
**Cada diretiva trata seus argumentos individualmente** — o parser não impõe
semântica, só entrega a estrutura. `@ffi` exige 1 string no primeiro elemento;
`@test` exige `desc` e aceita `args`/`expects`/`timeout` como chaves do dict.

Não há mistura: `()` recebe só elementos posicionais, `{}` recebe só pares
nomeados. Para `@test`, usa-se `()` quando só a descrição basta, `{}` quando
args/expects/timeout são necessários.

**Literais compostos em valores:** o parser de diretivas aceita os mesmos
literais que o resto da linguagem — tupla `(a, b, ...)`, variant
`Result::Ok 42`, e apply posicional de construtor `Pessoa "João" 30`. Structs
NÃO usam sintaxe `{campo: valor}` — Kata constrói structs via apply posicional
(o mesmo caminho que qualquer expressão Kata). Reusa `parse_atom` onde possível
para evitar duplicar lógica entre diretivas e código normal.

### 2.2. Formas de `@test`

```kata
@test("descrição")
action foo
    ...

@test{desc: "descrição", args: (1, 2)}
action soma a b
    return + a b

@test{desc: "espera pânico", args: (0, 0), expects: "Panic: divisão por zero"}
action div a b
    return / a b

@test{desc: "espera erro de compilação", expects: "CompileError: type mismatch"}
action bug
    return + 1 "texto"
```

### 2.3. Argumentos da diretiva

| Argumento | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `desc` (chave em `{}`) ou elemento 0 em `()` | `Text` | sim | Identifica o teste no relatório |
| `args` (chave em `{}`) | tupla | não (default `()`) | Argumentos para a action |
| `expects` (chave em `{}`) | `Text` | não | Mensagem esperada de erro |
| `timeout` (chave em `{}`) | `Int` (ms) | não (default 5000) | Timeout da execução |

Quando `@test("desc")` (forma curta em `()`), `desc` é o único elemento da
tupla. As outras chaves usam defaults.

### 2.4. Semântica de sucesso

| Forma | Sucesso quando |
|---|---|
| Sem `expects` | Action executa sem pânico e sem deadlock dentro do timeout |
| `expects: "CompileError: msg"` | Compilação falha com mensagem idêntica (ou substring, ver §5.2) |
| `expects: "Panic: msg"` | Action pânica em runtime com mensagem idêntica |
| Timeout excedido | Falha — reporta "timeout" |

## 3. Descoberta

`kata test <arquivo.kata>` (ou diretório) carrega o módulo com
`kata-module-loader`, faz lex → parse → resolve → infer → monomorph → optimize.
Após o pipeline, o runner percorre `TypedModule` procurando por items
`ActionDecl` cujo `directives` contenham uma `Directive { name: "test", .. }`.

Não há tree-shaking envolvido — `@test` é preservado no `TypedModule`. O runner
itera as actions anotadas e as executa como entry points separados.

**Nota sobre tree-shaking:** o ROADMAP menciona tree-shaking de `@test` em
produção (linha 688). O crate `kata-tree-shaking` **não existe hoje** no
workspace. Tree-shaking real fica para o Fio 15 (AOT), onde `kata build` é
definido. Neste PRD, `@test` em código de produção é simplesmente não chamado
pelo entry point — já é morto por não ser reached.

## 4. Execução — um entry point por `@test`

Para cada `@test` descoberto, o codegen gera um entry point separado chamado
`__kata_test_<action_name>_<idx>`, onde `idx` é o índice do `@test` na action
(uma action pode ter múltiplos `@test`).

O entry point é um wrapper que:
1. Carrega os argumentos literais da diretiva (passados como `SpawnArgs` ou
   tupla no stack, conforme a aridade da action)
2. Chama a action pelo `fn_ptr` (JIT já lowera o call)
3. Retorna o resultado (ou o motivo do pânico, capturado pelo runtime)

Isto reusa o lowering de `ActionCall` existente em `action_call.rs`. A diferença
é que os args são literais embutidos no wrapper, não vindos do caller.

### 4.1. JITModule compartilhado, TLS resetado entre testes

O `JITModule` é construído uma vez para o módulo inteiro (como `jit_eval` faz
hoje). Todos os wrappers `__kata_test_*` são declarados e finalizados no mesmo
module. O runner então executa cada wrapper em sequência.

**Entre cada `@test`**, o runner chama:
- `kata_rt::reset_all_arenas()` — limpa pool de arenas TLS
- Reset do scheduler TLS (novo `kata_rt_scheduler_reset()` a adicionar, ou
  re-init via `kata_rt_scheduler_init`)

Isto equivale ao `#[serial]` dos testes Rust atuais — garante que estado global
não vaza entre testes. Compartilhar o `JITModule` é seguro (FFI symbols são
estáticos); compartilhar TLS sem reset não é.

### 4.2. Timeout

O runner inicia um timer antes de chamar o wrapper. Se o wrapper não retorna
dentro de `timeout` ms, o runner:
- Para testes sem CSP: aborta a execução (mata o fiber se houver)
- Para testes com CSP (canais): sinaliza deadlock ao scheduler, que retorna
  `DEADLOCK_SENTINEL`

O runner reporta "timeout" como falha.

## 5. Relatório

### 5.1. Formato

```
Running tests from examples/test_assert.kata
  ✓ test_foo: "descrição"
  ✗ test_bar: "espera pânico" — timeout após 5000ms
  ✓ test_baz: "espera erro de compilação" — CompileError match

Result: 2 passed, 1 failed, 3 total
```

### 5.2. Match de mensagens

`expects: "msg"` compara a mensagem de erro do runtime/compilador com a string
fornecida. A comparação é **substring case-sensitive** — `expects: "Panic: div"`
casa com `Panic: divisão por zero`. Isto evita testes frágeis que dependem de
pontuação exata.

Exceção: `expects: "CompileError: msg"` casa com o tipo de erro
(`type.mismatch`, `parse.unexpected_token`, etc.) — substring da mensagem
completa do diagnóstico, que já é estruturada pelo `kata-diagnostics`.

## 6. CLI

Subcomando do `kata-driver`:

```
kata test <arquivo.kata>           # roda testes de um arquivo
kata test <diretório>              # descobre *.kata recursivamente
kata test <arquivo> --filter "foo" # roda só testes cuja descrição casa "foo"
```

Sem `--filter`, roda todos os `@test` do(s) módulo(s) carregado(s).

## 7. Escopo

### 7.1. Inclui

- Parser: `@test` já é parseável como `Directive` (nome `test`, args posicional
  + nomeados). Sem mudança no parser.
- Typeck: `@test` é uma anotação em `ActionDecl`. O typeck precisa aceitar a
  diretiva sem reclamar (hoje só `@ffi`, `@commutative`, `@builtin` são
  reconhecidos em actions — ver `kata-resolution/src/lib.rs`).
- Codegen: gerar wrappers `__kata_test_*` para cada `@test` no `TypedModule`.
- Driver: subcomando `kata test` que chama o runner.
- Runtime: `kata_rt_scheduler_reset` (ou equivalente) para resetar TLS entre
  testes.

### 7.2. Não inclui

- `@log` (segunda parte do Fio 14, sub-PRD separado)
- Tree-shaking de `@test` em produção (Fio 15)
- Testes em funções puras (apenas actions)
- `@test` com args dinâmicos (args são literais na diretiva)

## 8. Decisões de design

### D1: `@test` é anotação, não item separado

A action existe como antes. O `@test` marca. Justificativa: a action carrega
seu teste — menos indireção que "existe um teste que chama `foo`". O runner
descobre via `directives` no `ActionDecl`.

### D2: Um entry point por `@test`, args literais

Cada `@test` vira um wrapper `__kata_test_<name>_<idx>` que chama a action com
args literais. Justificativa: reusa o lowering de `ActionCall` existente; sem
serialização de args (que seria o caminho (b) — mais flexível mas mais complexo,
associado ao `@parallel` congelado).

### D3: JITModule compartilhado entre testes positivos, negativos em arquivo isolado

Não criar `JITModule` novo por teste positivo (compilação cara). Todos os
wrappers `__kata_test_*` dos **positivos** são declarados e finalizados no
mesmo module. Testes **negativos** (`expects: "CompileError: ..."`) são
segregados em arquivos `.kata` próprios — o runner roda `infer_module` no
arquivo isolado; se `Ok`, falha ("esperava CompileError mas compilou"); se
`Err`, compara mensagem com `expects`. É o padrão de test runners maduros
(Rust `compile_fail`, ui tests) e evita inventar infra de fatiamento de
`TypedModule` por dependências transitivas (que não existe no projeto).

Resetar arenas e scheduler TLS entre testes para evitar vazamento de estado.
Equivalente ao `#[serial]` dos testes Rust atuais.

### D4: Match de mensagem é substring

Evita testes frágeis dependentes de pontuação exata. `expects: "Panic: div"`
casa com `Panic: divisão por zero`.

### D5: Tree-shaking de `@test` fica no Fio 15

O crate `kata-tree-shaking` não existe. Criá-lo só para `@test` aumenta o
escopo sem necessidade — `kata run` (JIT) só executa o entry point, então
`@test` em actions não chamadas já é morto por não ser reached. Tree-shaking
real pertence ao Fio 15 (AOT).

## 9. Fases de implementação

### Fase 1: Parser — permitir literais compostos como valor em diretiva

Hoje o parser de diretivas aceita:
- `@nome` — sem args
- `@nome("str" ou 1)` — `()` com posicionais (`TextLit`/`IntLit`)
- `@nome{chave: "valor" ou 1}` — `{}` com nomeados, valores só `TextLit`/`IntLit`

O parser entrega `Vec<DirectiveArg>` (tupla) ou `Vec<DirectiveArg::Named>`
(dict). **Cada diretiva trata seus argumentos individualmente** — o parser não
valida semântica, só entrega a estrutura.

**Mudança:** permitir que valores dentro do dict (e elementos posicionais do
tupla) sejam literais compostos — tupla e variant — para suportar
`@test{args: (1, 2)}` e `@test{args: (Result::Ok 42)}`. Hoje
`parse_directive_value` só aceita `TextLit`/`IntLit`.

**Sem `DirectiveValue::Struct`:** Kata constrói structs via apply posicional
(`Pessoa "João" 30`), não via `{campo: valor}`. Adicionar `Struct` ao
`DirectiveValue` criaria sintaxe que não existe em nenhum outro lugar da
linguagem — assimetria entre diretivas e código normal. Para passar uma struct
como arg de teste, usa-se apply posicional: `@test{args: (Pessoa "João" 30)}`,
parseado pelo `parse_atom` existente.

**Mudanças:**
- `kata-ast/src/item.rs`: adicionar `DirectiveValue::Tuple(Vec<DirectiveValue>)`
  e `DirectiveValue::Variant(String, Vec<DirectiveValue>)`. Sem
  `DirectiveValue::Struct`.
- `kata-parser/src/directives.rs`: em `parse_directive_value`, adicionar braços:
  - `Token::LParen` → parse recursivo (mesma lógica de `parse_directive_args`
    mas produzindo `Vec<DirectiveValue>`).
  - `Token::UpperIdent` → Variant: nome + opcional `LParen` de args recursivos.
  Reusar lógica de `parse_atom` onde possível.
- `kata-resolution/src/lib.rs` e `pass0.rs`: não quebra — todos os sites usam
  `if let Some(DirectiveArg::Str(s)) = d.args.first()` (não-exaustivo).
- Testes: adicionar casos para:
  - `@test("desc")` — tupla de 1 string (forma curta)
  - `@test{desc: "...", args: (1, 2)}` — dict com tupla como valor
  - `@test{desc: "...", args: (Result::Ok 42)}` — dict com variant como valor
  - `@test{desc: "...", args: (Pessoa "João" 30)}` — dict com apply posicional
    (construtor de struct) — via `parse_atom`, não `DirectiveValue::Struct`
  - `@test{desc: "...", expects: "Panic: msg", timeout: 5000}` — dict sem args
  - Diretivas existentes (`@ffi`, `@builtin`, `@commutative`) continuam parseando.

**DoD Fase 1:** `kata parse` de módulo com as formas acima produz AST sem
erro. `cargo test --workspace` não regrediu. Diretivas existentes continuam
parseando.

### Fase 2: Typeck — aceitar `@test` em `ActionDecl` + rejeitar desconhecidas

- Resolution/inference: aceitar `@test` em `ActionDecl` sem reclamar. Hoje só
  `@ffi` era reconhecido em actions. Adicionar `test` à lista de diretivas
  aceitas.
- **Rejeitar diretivas desconhecidas em TODOS os contextos** (endurece o
  contrato). Antes, o catch-all `_ => {}` silenciosamente engolia qualquer
  nome. Typos como `@tset` ou `@fffi` viravam diretivas fantasma. Agora cada
  contexto valida contra sua lista:
  - **Sig**: `ffi`, `builtin`, `commutative`, `associative`
  - **Action**: `ffi`, `test`
  - **Implements method**: `ffi`, `builtin`, `commutative`, `associative`
  - **Data**: `ffi`
  Diretivas fora da lista → `ResolveError::UnknownDirective { name, context, item_name }`.
- Validar: `desc` é `Str`, `args` é `Tuple` (se presente), `expects` é `Str`,
  `timeout` é `Int`.

**DoD Fase 2:** Typeck aceita `@test` em actions e rejeita diretivas desconhecidas
em todos os contextos. `kata parse` de módulo com `@test` em action não produz
warning. `@tset` (typo) produz erro de resolution.

### Fase 3: Codegen — gerar wrappers `__kata_test_*`

- `kata-codegen/src/lowering/`: novo módulo `test_runner.rs` (ou extensão de
  `module.rs`).
- Para cada `ActionDecl` com `@test`, gerar wrapper `__kata_test_<name>_<idx>`
  que carrega args literais e chama a action.
- O wrapper é uma função JIT separada no mesmo `JITModule`.

**DoD Fase 3:** `cargo test` passa. `JITModule` contém os wrappers. Inspeção
via `eprintln!(ctx.func.display())` mostra wrappers com args literais.

### Fase 4: Runtime — checar deadline no scheduler + reset de TLS

O scheduler já recupera controle periodicamente via yield cooperativo
(`YIELD_INTERVAL=1000`, `kata_rt_yield_check` no header de Loop/ForIn). A
mudança é fazer o scheduler checar o deadline do teste antes de cada
`resume()` no loop principal — sem adicionar `YieldReason::Timeout` novo,
sem `scheduler_signal_timeout`, sem thread OS.

**Mudanças:**
- `kata-rt/src/scheduler.rs`: adicionar campo `test_deadline: Option<Instant>`
  ao `Scheduler`. No loop principal (antes de `resume_fiber`), checar:
  se `test_deadline` está expirado, retornar `TIMEOUT_SENTINEL` (i64::MIN + 2,
  distinto de `DEADLOCK_SENTINEL` = i64::MIN + 1).
- `kata-rt/src/scheduler.rs`: expor `set_test_deadline(Option<Instant>)` para
  o runner configurar antes de cada teste.
- Adicionar `kata_rt_scheduler_reset` (ou equivalente) a `kata-rt` — reseta
  TLS do scheduler entre testes. Pitfall #31: registrar em TODOS os sites —
  `FfiSymbol` enum (`kata-core/src/ffi.rs`), `symbol_name()`, `return_type()`,
  `from_name()`, `ffi_signature()` (`kata-codegen/src/ffi_sigs.rs`),
  `all_ffi_symbols()` e `declare_ffi_symbols` (`kata-codegen/src/ffi_registry.rs`),
  `register_ffi_symbols` (builder.symbol).
- Teste Rust: chama `reset_all_arenas()` + `scheduler_reset()` e verifica
  arenas/scheduler limpos. Teste de timeout: configura deadline curto (ex: 10ms)
  e roda action com loop infinito cooperativo — deve retornar `TIMEOUT_SENTINEL`.

**Pitfalls aplicáveis:** #38 (extern "C" é nounwind — nunca `panic!` na stack
de `Fiber::resume()` — retornar sentinela), #43 (wasmtime-fiber panica no Drop
se não completou — `ManuallyDrop`), #44 (FFI durante `resume()` não pode
re-borrow `SCHEDULER` RefCell — usar TLS `PENDING_SPAWNS`).

**DoD Fase 4:** `cargo test -p kata-rt` passa. Scheduler reseta corretamente.
Teste de timeout com loop cooperativo retorna `TIMEOUT_SENTINEL` dentro do
deadline.

### Fase 5: Driver — subcomando `kata test`

- `Command::Test { path: String, filter: Option<String> }` no `kata-driver`.
- `cmd_test`: carrega módulo, roda pipeline, descobre `@test`, executa
  wrappers, reporta.

**DoD Fase 5:** `kata test examples/test_assert.kata` roda testes e reporta.

### Fase 6: Testes E2E

- `crates/kata-codegen/tests/test_runner_e2e.rs` (ou `kata-driver/tests/`).
- Mínimo: 5 testes
  - `@test` sem args, sem expects — sucesso
  - `@test` com args, sem expects — sucesso
  - `@test` com `expects: "Panic: msg"` — sucesso
  - `@test` com `expects: "CompileError: msg"` — sucesso
  - `@test` com timeout — falha por timeout

**DoD Fase 6:** Todos os 5 testes passam. `cargo test --workspace` não regrediu.

### Fase 7: Documentação

- Atualizar `docs/ROADMAP.md` Fio 14 com status `@test` ✅.
- Atualizar `docs/Kata-lang-manual.md` se a sintaxe divergiu do manual (linha
  1338-1340 descreve `@test("descrição")` — confirmar compatibilidade).
- Atualizar `docs/sintaxe-mapa.md` se a diretiva `@test` não estiver listada.

**DoD Fase 7:** Documentação reflete a implementação.

## 10. DoD (Definition of Done)

1. `@test("descrição") action foo ...` compila sem erro.
2. `kata test arquivo.kata` descobre e executa todos os `@test`.
3. `@test` com `args: (...)` executa a action com os args.
4. `@test` com `expects: "Panic: msg"` valida o pânico.
5. `@test` com `expects: "CompileError: msg"` valida o erro de compilação.
6. `@test` com timeout reporta "timeout" se excedido.
7. TLS resetado entre testes — estado não vaza.
8. Relatório mostra pass/fail/error por teste.
9. Testes E2E cobrem os 5 casos acima.
10. `cargo test --workspace` passa sem regressão.
11. `@test` em `kata run` (sem `kata test`) não executa — só `kata test` roda.

## 11. Fora do escopo

- `@log` (segunda parte do Fio 14, sub-PRD separado)
- Testes em funções puras (só actions)
- `@test` com args dinâmicos
- Paralelização de testes (testes rodam em sequência)
- `kata build` e tree-shaking de `@test` (Fio 15)

## 12. Riscos

- **`expects: "CompileError: msg"` exige rodar pipeline de compilação por
  teste.** Hoje `jit_eval` assume que o módulo inteiro compila. Testes negativos
  precisam de um caminho que captura o erro de compilação sem abortar o runner.
  Solução: testes negativos são segregados em arquivos `.kata` próprios. O
  runner chama `infer_module` no arquivo isolado; se `Ok`, falha ("esperava
  CompileError mas compilou"); se `Err`, compara a mensagem com `expects`.
  É o padrão de test runners maduros (Rust `compile_fail`, ui tests) e evita
  inventar infra de fatiamento de `TypedModule` por dependências transitivas
  (que não existe no projeto).

- **`@test` em action com CSP pode bloquear o runner.** Se a action faz `<!`
  em canal sem sender, o scheduler deadlocka. Solução: timeout cooperativo
  via `YIELD_INTERVAL` existente — o scheduler já recupera controle
  periodicamente via `kata_rt_yield_check`. A mudança é fazer o scheduler
  checar `test_deadline` antes de cada `resume()` no loop principal e
  retornar `TIMEOUT_SENTINEL` se expirado. Sem thread OS (que quebraria o
  invariant single-threaded do runtime), sem `YieldReason::Timeout` novo.
  Mesmo loops infinitos cooperativos disparam o timeout — o scheduler checa
  o deadline a cada iteração do loop principal.