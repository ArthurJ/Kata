# PRD — `constant`: Constantes de Módulo + Remoção de `@comptime` + Comentários Multilinha

**Status:** Pendente
**Data:** 2026-08-10
**Depende de:** Fio 12 ✅ (comptime pass — JIT-and-execute, HeapSnapshot, serialização), Module system ✅ (import/export, ModuleLoader)
**Substitui:** `@comptime` como diretiva de usuário (Fio 12 §2.1–2.3)
**Não depende de:** `@cache` (Fio 12 §2.4 — permanece, é ortogonal)
**Inclui:** Comentários multilinha `#{}#` (feature ortogonal, mesmo PRD)

## 1. Objetivo

Introduzir `constant` como keyword reservada para declarar constantes de módulo
— valores imutáveis computados em compile-time, acessíveis de qualquer escopo
(lambdas, funções nomeadas, actions) e exportáveis via `export`.

Remover `@comptime` da superfície da linguagem. A maquinaria interna do comptime
pass (JIT-and-execute, HeapSnapshot, serialização, fold automático) permanece
intacta — é usada internamente por `constant` e pelo fold de chamadas puras.

### Princípio: declaração vs expressão

`constant` é uma **declaração de módulo** — como `data`, `enum`, assinaturas de
função. Vive no top level, define um nome no escopo do módulo, é exportável.
Não é uma expressão; não aparece dentro de actions ou funções.

`let` é um **binding local de runtime** — vive dentro de actions, funções,
lambdas. Não é uma declaração de módulo; não aparece no top level.

A diretiva `@comptime` misturava os dois: era uma anotação de expressão que
podia aparecer em qualquer contexto, criando ambiguidade semântica sobre escopo
e momento de avaliação. `constant` separa claramente: declaração de módulo =
compile-time, binding local = runtime.

### Princípio: fold automático existe, mas é internals

O comptime pass já faz fold automático de chamadas puras com args literais
(`fib 20` → JIT-and-execute → HeapSnapshot) sem o usuário pedir. Isso
continua. O usuário não precisa anotar — o compilador otimiza o que pode.

`constant` é o único caso onde o usuário **força** avaliação em compile-time.
Se o RHS não é comptime-available (depende de I/O, parâmetros, runtime), erro
de compilação. Não há "talvez avalia."

## 2. Sintaxe

### 2.1. Declaração `constant`

```
constant nome := expressão
```

- `nome` é um identificador em **minúsculo** (snake_case, como todo binding
  em Kata). `Constant` com maiúscula é erro — maiúsculas são reservadas para
  tipos (Struct, Enum, Interface). `nome` pode ser `_` para descartar o
  binding — útil quando o objetivo é apenas o side effect da avaliação,
  como pré-aquecer cache).
- `expressão` é avaliada em compile-time via JIT-and-execute.
- Resultados escalares (Int, Float, Text, Boolean, Unit) → literal na TAST.
- Resultados complexos (List, Tuple, Struct, Sum) → `HeapSnapshot` na TAST.
- Top-level only. Erro compile-time se aparece dentro de action/função/lambda.

### 2.2. Exemplos

```kata
constant pi := 3.14159265
constant tabela := range 0 1000
constant config := parse_config "default.json"
constant _ := fib 1000          # side effect: pré-aquece cache de fib
```

### 2.3. `let` no top level — proibido

```kata
let x := 42          # ERRO: let não é permitido no top level
                      # use `constant x := 42` para constante de módulo
                      # ou mova o código para uma action
```

`let` é binding local de runtime. No top level, não há frame de runtime para
abrigá-lo — o top level é declarações e a última expressão (entry point).

### 2.4. `@comptime` — removido

```kata
@comptime let x := [1 2 3]       # ERRO: @comptime removido
@comptime fatorial 10            # ERRO: @comptime removido
let y := @comptime (fib 30)      # ERRO: @comptime removido
```

Toda funcionalidade de `@comptime` é coberta por:
- `constant` para declarações de módulo comptime
- Fold automático para otimização sem anotação
- A última expressão top-level continua sendo o entry point (runtime)

### 2.5. REPL — fora do escopo

O REPL não é "top level de módulo". `let` no prompt do REPL continua funcionando
como hoje (frozen_bindings / snapshot_bindings). A restrição de `let` no top
level aplica-se apenas a arquivos `.kata` compilados como módulos.

## 3. Semântica

### 3.1. Avaliação em compile-time

O RHS de `constant` é avaliado pelo comptime pass existente:
1. Verifica constness (todos os valores referenciados são comptime-available)
2. JIT-and-execute via `jit_execute_expr`
3. Converte resultado em literal (escalar) ou `HeapSnapshot` (complexo)
4. Registra o binding no `type_env` como binding de módulo

Se o RHS não é comptime-available → erro de compilação:
"`constant config := input!(\"...\")` — `input!` não é disponível em compile-time."

### 3.2. Escopo de módulo

`constant` registra o binding no `type_env` do `TypedModule` como binding de
módulo (não no `pre_entry`). Isso torna o nome visível de:
- **Lambdas** — `lambda n: + n SCALE` resolve `SCALE` no type_env
- **Funções nomeadas** — `dobro x := * x SCALE` resolve `SCALE`
- **Actions** — `action main => ... echo!(SCALE) ...` resolve `SCALE`
- **Outras constants** — `constant dobro_pi := * pi 2` resolve `pi`
- **Entry point** — a última expressão top-level resolve constants

### 3.3. Codegen

Para escalares (literal na TAST): o codegen já lowera literais. Sem mudança.

Para complexos (`HeapSnapshot` na TAST): o codegen já lowera
`kata_rt_get_snapshot(snapshot_id)`. O snapshot é carregado no prólogo de
`__kata_entry` na root_arena. Como a root_arena é compartilhada entre todos
os fibers, o ponteiro é válido de qualquer contexto (actions, funções, lambdas).

O ponto de toque novo é: quando o codegen encontra um `Ident("SCALE")` dentro
de uma action/função/lambda e `SCALE` é um binding de módulo (não local), ele
precisa emitir o acesso ao snapshot/literal. Hoje, `Ident` dentro de actions
só resolve bindings locais (parâmetros, `let`/`var` do corpo) e funções do
DispatchTable. A resolução de bindings de módulo é nova para o codegen de
actions/funções — hoje só o codegen do entry point (via `pre_entry`) acessa
esses valores.

### 3.4. Exportação

```kata
constant escala := 2
export escala
```

O módulo compilado embute o snapshot (ou literal) de `escala`. O módulo
importador faz `import mod.escala` e o valor é carregado na root_arena do
importador. Mecanismo similar ao comptime pass atual, mas cross-módulo.

Dependência transitiva: se uma função exportada referencia `constant escala`,
o módulo importador precisa de `escala` transitivamente. O ModuleLoader já
resolve dependências transitivas para funções — `constant` segue o mesmo
caminho.

#### Importação indireta (função que referencia constant)

Quando uma função exportada referencia uma `constant` no seu corpo, o comptime
pass já substitui a referência antes do codegen:

- **Escalar:** o valor é inlined como `iconst` no IR da função. A função
  compilada é auto-contida — zero dependência externa da constant no
  importador.
- **Complexo (HeapSnapshot):** a função compilada emite
  `kata_rt_get_snapshot(id)`. O snapshot precisa estar carregado no runtime
  do importador. O ModuleLoader puxa o snapshot da constant transitivamente
  ao importar a função — mesmo mecanismo de dependência transitiva de
  símbolos que já existe para funções que chamam outras funções exportadas.

O importador não precisa declarar `import mod.escala` explicitamente se só
usa a função — a dependência transitiva é resolvida pelo ModuleLoader.

### 3.5. Arena e memória

Constants vivem na root_arena via bump allocation (sem ARC). O mecanismo de
snapshot já usa `kata_rt_arena_alloc` — bump na root_arena, sem header ARC,
sem refcount, sem destructor. Justificativa:

- **Imutáveis:** ninguém modifica o valor após a criação
- **Alocados uma vez:** no prólogo de `__kata_entry`, carregados via
  `kata_rt_load_snapshot`
- **Nunca desalocados:** a root_arena vive pelo tempo de vida do Runtime
- **Compartilhados:** todos os fibers acessam o mesmo ponteiro na root_arena

ARC é para valores que podem precisar de deallocation quando a última
referência cai. Constants nunca caem — o tempo de vida é o do programa.

### 3.6. Side effects da avaliação

A avaliação de `constant` em compile-time pode ter side effects úteis:
- Pré-aquecer cache de função (`@cache`) — `constant _ := fib 1000` executa
  `fib`, popula o cache, e o cache persiste se o código JIT permanece mapeado
- Inicializar estruturas de dados estáticas

O binding `_` indica que o valor não é usado — só o side effect importa.

### 3.7. Tipos suportados

Mesmos tipos do comptime pass atual (Fio 12 §3.2):
- **Escalar → literal:** Int (SMI e BigInt), Float, Text, Boolean, Unit
- **Complexo → HeapSnapshot:** List, Tuple, Struct, Sum, Rational
- **Não suportado:** Function (closures não são serializáveis em compile-time
  de módulo — o endereço JIT é válido mas não determinístico entre compilações)

## 4. Mudanças por camada

### 4.1. Lexer / Parser

**Nova keyword `constant`:**
- `Token::Constant` (ou interceptar `Ident("constant")` antes do handler
  genérico, como feito com outras keywords)
- `is_keyword` atualizado
- `Display` atualizado

**Novo `Item::ConstantDecl`:**
```rust
Item::ConstantDecl {
    name: String,           // ou "_" para descarte
    value: Spanned<Expr>,
}
```

**Parser — `parse_module`:**
- `Token::Constant` → parse `constant nome := expr` → `Item::ConstantDecl`
- `Token::Let` no top level → **erro de parse**: "`let` não é permitido no top
  level. Use `constant` para constantes de módulo ou mova o código para uma
  action."

**Remoção de `@comptime`:**
- O parser não envolve mais `let`/expressões em `Expr::Comptime` quando
  encontra a diretiva `@comptime`
- `Expr::Comptime` pode ser removido do AST (ou mantido como internals para
  o fold automático — ver §4.3)

### 4.2. Resolution / Inference

**`Item::ConstantDecl` → binding de módulo:**
- O resolvedor registra `nome` no `type_env` como `TypeBinding` com `origin`
  do módulo local
- O inference avalia o RHS via comptime pass e produz o `TypedExpr` (literal
  ou `HeapSnapshot`)
- O `TypedModule` armazena constants em uma nova coleção
  `constants: Vec<(String, TypedExpr)>` (ou reusa `pre_entry` com um flag de
  "módulo" vs "entry")

**Acesso de actions/funções/lambdas:**
- Quando o typeck encontra `Ident("SCALE")` dentro de uma action e `SCALE`
  não é um binding local, procura no `type_env` (bindings de módulo)
- Se encontra, resolve o tipo. O `TypedExprKind::Ident` na TAST carrega a
  informação de que é um binding de módulo (não função do DispatchTable)
- O codegen usa essa informação para emitir o acesso correto

### 4.3. Comptime pass

**Avaliação de `constant`:**
- O comptime pass já avalia expressões via `jit_execute_expr`
- Para `constant`, o fluxo é: inference produz o TAST do RHS → comptime pass
  JIT-executa → substitui por literal/HeapSnapshot
- Se o RHS não é comptime-available, o comptime pass retorna erro

**Fold automático — mantido:**
- O fold de chamadas puras com args literais continua funcionando
  internamente, sem anotação do usuário
- `Expr::Comptime` pode permanecer no AST como marca interna do fold, mas não
  é exposto ao usuário

**Remoção da diretiva `@comptime`:**
- O parser não produz mais `Expr::Comptime` a partir de `@comptime`
- O comptime pass não busca mais nós `Comptime` na TAST para avaliação
  explícita — a avaliação é acionada por `constant` (declaração) e pelo fold
  (automático)

### 4.4. Codegen

**Acesso a bindings de módulo em actions/funções:**
- Hoje: `Ident` dentro de action resolve para parâmetro, `let`/`var` local,
  ou função do DispatchTable
- Novo: se `Ident` não é local nem função, procura em `constants` do
  `TypedModule`. Se encontra, emite:
  - **Literal escalar:** `iconst` direto (Int, Float) ou string global (Text)
  - **HeapSnapshot:** `kata_rt_get_snapshot(snapshot_id)` — mesmo código que
    o entry point já usa

**Prólogo de `__kata_entry`:**
- Snapshots de constants já são carregados no prólogo (mecanismo existente)
- Sem mudança — o snapshot_id é global ao módulo

### 4.5. Module system (export/import)

**Export de constant:**
- `export escala` marca o binding como exportado no `ResolvedModule`
- O módulo compilado embute o snapshot/literal

**Import de constant:**
- `import mod.escala` carrega o snapshot do módulo importado
- O ModuleLoader resolve a dependência transitiva
- O snapshot é carregado na root_arena do importador

## 5. Fases

### Fase 1: Keyword `constant` + parse

- Adicionar `Token::Constant` (ou keyword `constant`)
- Adicionar `Item::ConstantDecl { name, value }`
- Parser: `Token::Constant` → `Item::ConstantDecl`
- Parser: `Token::Let` no top level → erro
- **DoD:** `constant x := 42` parseia; `let x := 42` no top level dá erro

### Fase 2: Inference + comptime pass

- `Item::ConstantDecl` → resolvedor registra no `type_env`
- Inference avalia RHS via comptime pass → literal/HeapSnapshot
- `TypedModule` armazena constants
- **DoD:** `constant x := 42` produz `IntLit { "42" }`; `constant xs := [1 2 3]`
  produz `HeapSnapshot`

### Fase 3: Acesso de actions/funções/lambdas ✅ (commit `a6609e4`)

- Typeck: `Ident` dentro de action/função/lambda resolve binding de módulo
- Comptime pass: `constant_fold.rs` substitui `Ident(name)` por literal/snapshot
  nos corpos de functions e actions após o fixpoint
- `walk_mut`: adicionados braços para `ActionCall` e `ConstantBinding`
- `is_already_evaluated`: insere no `comptime_bindings` antes de pular
- **DoD:** ✅ `constant scale := 2` + `dobro :: Int => Int` + `lambda x: * x scale`
  + `echo!(dobro 21)` imprime `42`
- **Testes E2E:** 9 passed, 1 ignored (HeapSnapshot em function body — Fase 3b)
- **Débito:** `constant base := [1 2 3]` + function que referencia `base` falha
  com `comptime.jit_failure` (fold_literal_calls tenta JIT antes do fold)

### Fase 4: Export/import

- `export escala` marca constant como exportado
- `import mod.escala` carrega snapshot do módulo importado
- ModuleLoader resolve dependência transitiva
- **DoD:** módulo A exporta `constant escala := 2`; módulo B importa e usa
  `escala` dentro de uma action

### Fase 5: Remoção de `@comptime`

- Remover `Expr::Comptime` do AST (ou manter como internals)
- Remover tratamento de `@comptime` no parser
- Remover `TypedExprKind::Comptime` (ou manter como internals do fold)
- Atualizar testes que usam `@comptime`
- Atualizar manual (`docs/Kata-lang-manual.md`) — remover seção de `@comptime`,
  adicionar seção de `constant`
- Atualizar sintaxe-mapa (`docs/sintaxe-mapa.md`) — adicionar `constant` na
  seção de bindings, adicionar `#{}#` na seção de comentários, remover
  `@comptime` da seção de diretivas
- **DoD:** `@comptime` não é aceito pelo parser; todos os testes passam;
  manual e sintaxe-mapa refletem `constant` e `#{}#`

### Fase 6: REPL

- Verificar que `let` no prompt do REPL continua funcionando
- `constant` no REPL deve funcionar (registrar binding persistente)
- **DoD:** REPL aceita `constant x := 42` e `let x := 42` (ambos no prompt)

### Fase 7: Comentários multilinha `#{}#`

- Lexer: `#{` inicia comentário multilinha, `}#` termina
- Lexer: `#` sem `{` continua sendo comentário de linha
- Erro léxico se `#{` não tem `}#` até EOF
- Testes de lexer para casos edge (§9.3)
- Atualizar sintaxe-mapa (`docs/sintaxe-mapa.md`) — adicionar `#{}#` na
  seção de comentários
- **DoD:** `#{ multilinha }#` é ignorado; `# linha` continua funcionando;
  `#{` sem fechamento dá erro léxico

## 6. Breaking changes

1. **`let` no top level de arquivos `.kata`** — passa a ser erro de parse.
   Migração: `let x := 42` → `constant x := 42` (se for constante) ou mover
   para `action main`.

2. **`@comptime` removido** — qualquer uso de `@comptime` no código fonte
   passa a ser erro. Migração: `@comptime let x := expr` → `constant x := expr`;
   `@comptime expr` (entry point) → `constant _ := expr` (se quer o side
   effect) ou remover (se era só display).

3. **Testes E2E** — testes que usam `@comptime` precisam migrar para
   `constant`. Testes que usam `let` no top level precisam migrar para
   `constant` ou mover para actions.

## 7. Decisões settled

- **D1:** `constant` é top-level only. Dentro de actions/funções/lambdas → erro.
- **D2:** `let` é proibido no top level de arquivos `.kata`. REPL não é top
  level de módulo — `let` no REPL continua funcionando.
- **D3:** `constant` é acessível de actions, funções nomeadas, lambdas, e
  outras constants. Não quebra pureza — constante é imutável e conhecida em
  compile-time, semanticamente idêntica a inlinar o literal.
- **D4:** `@comptime` é removido da superfície da linguagem. A maquinaria
  interna (comptime pass, fold automático, HeapSnapshot) permanece.
- **D5:** `constant _ := expr` é válido — avalia para side effect, descarta
  o binding.
- **D6:** `constant` é exportável via `export`. Import carrega o
  snapshot/literal no módulo importador.
- **D7:** O RHS de `constant` é implicitamente comptime-avaliado. Não há
  `@comptime constant` — é redundante. Se o RHS não é comptime-available,
  erro de compilação.
- **D8:** Fold automático de chamadas puras continua existindo internamente,
  sem anotação do usuário. É ortogonal a `constant`.
- **D9:** Nomes de `constant` são em minúsculo (snake_case), como todo
  binding em Kata. Maiúsculas são reservadas para tipos.
- **D10:** Comentários multilinha `#{}#` são ortogonais a `constant` —
  feature de lexer, sem dependência de outras fases.

## 9. Comentários multilinha `#{}#`

### 9.1. Sintaxe

```
# Este é um comentário de linha (existente)

#{ Este é um
   comentário multilinha }#
```

- `#{` inicia comentário multilinha. `}#` termina.
- Tudo entre `#{` e `}#` é ignorado pelo lexer — não produz tokens.
- Pode conter qualquer texto, incluindo `#`, `{`, `}`, newlines.
- Não há aninhamento — o primeiro `}#` fecha o comentário.
- `#{` sem `}#` até EOF → erro léxico "comentário multilinha não fechado".

### 9.2. Interação com comentário de linha

`#` sozinho continua sendo comentário de linha (skipa até `\n`). O lexer
distingue:

- `#` seguido de `{` → comentário multilinha (procura `}#`)
- `#` seguido de qualquer outra coisa → comentário de linha (skipa até `\n`)

Isso é um lookahead de 1 caractere no ponto onde o lexer encontra `#`.

### 9.3. Casos edge

- `#{}#` vazio (sem conteúdo) → válido, comentário vazio
- `#{ # }#` → válido, `#` dentro do bloco é texto
- `#{ { } }#` → válido, chaves dentro são texto
- `#{{}}#` → válido, o primeiro `}#` fecha (não há aninhamento)
- `#{` sem `}#` → erro léxico
- `#{ # }` sem `#` final → erro léxico (o `}` sozinho não fecha)

### 9.4. Implementação

**Lexer (`kata-lexer/src/lib.rs`):**

No ponto onde o lexer encontra `#` (linha 178):
1. Peek próximo caractere
2. Se for `{` → entrar em modo comentário multilinha: consumir até encontrar `}#`
3. Senão → comentário de linha (comportamento atual: skipa até `\n`)

O modo comentário multilinha precisa de um loop que consome caracteres
um a um, procurando a sequência `}#` (dois caracteres). Se chegar a EOF
sem encontrar `}#`, emitir erro léxico.

## 10. Não-goals

- **Bindings globais mutáveis** — `var` no top level. Rejeitado. Estado
  mutável global compartilhado entre fibers é pesadelo de concorrência.
- **`@comptime` como expression annotation** — removido. `constant` cobre o
  caso de declaração; fold automático cobre otimização sem anotação.
- **Elevação implícita de `let` para `constant`** — rejeitado. O compilador
  não adivinha intenção. `input!("...")` mostra que nem todo `let` top-level
  é constante.
- **`constant` dentro de actions** — rejeitado. `constant` é declaração de
  módulo, não binding local. Dentro de action, use `let`.