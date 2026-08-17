# PRD — `enum extends`: Herança Composicional de Enums + `expects` Tipado

**Status:** Rascunho
**Data:** 2026-08-17
**Depende de:** EnumRegistry ✅, `Result::(T, E)` ✅, `@test` runner ✅, `refines` ✅ (modelo de referência)
**Não depende de:** Subtyping nominal, widening implícito, enums abertos

## 1. Objetivo

Permitir que um enum herde variantes de outro enum (base), produzindo um enum
fechado novo com o conjunto unionizado de variantes. O enum resultante é
exaustivo — `match` verifica cobertura total, como qualquer enum fechado.

Motivação direta: `@test{expects: Tipo.Variante}` tipado. Hoje `expects` aceita
string (`"Panic: msg"`), sem verificação de tipo. Com `extends`, o prelude
define um enum base de erros de teste, e o usuário estende localmente com
variantes de domínio — o test runner verifica que a variante existe e que a
suite de testes cobre todas as variantes do enum esperado.

## 2. Sintaxe

### 2.1. Declaração `enum extends`

```
enum MeuErro extends ErroBase
    Timeout
    ValidacaoFail
```

- `MeuErro` é o enum novo (declarado com `enum ... extends ...`).
- `ErroBase` é um enum já declarado, visível no escopo, e **não-`final`**.
- Variantes listadas no corpo são adicionadas às herdadas.
- O enum resultante tem `variantes_base ++ variantes_próprias`.

### 2.2. `final` — bloqueio de extensão

Enums são **abertos por padrão**. Para bloquear extensão, declarar com `final`:

```
final enum ConnectionState
    Connected
    Disconnected
```

Tentar estender um enum `final` é erro compile-time:
*enum base é final — não pode ser estendido*.

`final` é opt-in: o autor declara que o enum é fechado e não deve ser
estendido. Sem `final`, qualquer enum visível pode ser base de `extends`.

**Enums fundacionais do prelude são `final`:** `Boolean`, `Result`, `Optional`.
São tipos estruturais do type system — estendê-los quebraria a semântica
contravariante de `match` e o contract de `Result::(T, E)`.

### 2.3. Enum base no prelude

```
enum KataError
    Panic
    AssertionFail
```

`KataError` é o enum canônico de erros de teste da linguagem. Não precisa ser
importado — vive no prelude como `Boolean`, `Result`, `Optional`. É aberto
por padrão (sem `final`), permitindo que módulos de teste o estendam.

### 2.4. Exemplo de uso

```
enum MeuErro extends KataError
    Timeout
    ValidacaoFail

action buscar (url::Text) => Result::(Text, MeuErro)
    match (http_get url)
        Result::Ok(resp): Result::Ok resp
        Result::Err(e): match (timeout_expirou e)
            Boolean::True: Result::Err MeuErro.Timeout
            Boolean::False: Result::Err MeuErro.Panic

@test{desc: "timeout", expects: MeuErro.Timeout, args: ("http://slow.example")}
@test{desc: "panic", expects: MeuErro.Panic, args: ("http://crash.example")}
buscar!("http://test.example")
```

## 3. Semântica

### 3.1. Flattening (resolução estática)

`extends` é resolvido em compile-time. Quando o resolution encontra
`enum MeuErro extends KataError`, ele:

1. Busca `KataError` no `EnumRegistry` (mesma origin ou prelude).
2. Verifica que `KataError` não é `final`. Se é → erro compile-time:
   *enum base é final — não pode ser estendido*.
3. Copia as variantes de `KataError` para `MeuErro`.
4. Adiciona as variantes declaradas no corpo de `MeuErro`.
5. Registra `MeuErro` no `EnumRegistry` com a lista unionizada.

O enum resultante é indistinguível de um enum declarado manualmente com todas
as variantes. `extends` é açúcar sintático de resolution — não existe em
runtime, não gera código diferente.

### 3.2. Sharing, não shadowing

Variantes herdadas são as mesmas — mesma identidade, mesmo payload, mesmo
construtor. `MeuErro.Panic` e `KataError.Panic` são a mesma variante lógica.

Consequências:
- Pattern matching em `MeuErro` pode referenciar `Panic` sem qualificar
  (resolução desqualificada funciona como hoje).
- Uma extensão **não pode** redefinir uma variante herdada com payload
  diferente. `Panic` em `KataError` é unitário; `MeuErro extends KataError`
  não pode declarar `Panic(Text)`. Erro compile-time: *variant herdada não
  pode ser redefinida*.
- Duas extensões do mesmo base não colidem entre si. `MeuErro extends KataError`
  e `OutroErro extends KataError` são enums independentes. `MeuErro.Panic` e
  `OutroErro.Panic` são a mesma variante lógica (a de `KataError`), mas os
  enums são tipos distintos — não são intercambiáveis.

### 3.3. Exaustividade preservada

`MeuErro` é fechado com N variantes (herdadas + próprias). `match` sobre
`MeuErro` exige cobertura de todas as N variantes ou catch-all. O typeck
verifica exaustividade como faz hoje para qualquer enum — nenhuma mudança no
algoritmo de exaustividade.

### 3.4. Genéricos

Enum base genérico: `Result` é o caso canônico. Se o base tem type params,
a extensão os herda implicitamente.

```
enum Result
    Ok(T)
    Err(E|Text)

enum MeuResult extends Result
    # Ok e Err herdadas com mesmos type params T, E
    # Não pode adicionar variantes — Result é genérico
```

**Restrição inicial (D1):** extensão de enum genérico herda apenas as
variantes — não pode adicionar novas. O caso de uso principal (erros de teste)
é com enums não-genéricos. Generalizar extensão com variantes genéricas novas
é post-1.0.

### 3.5. Transitividade

```
enum A
    X

enum B extends A
    Y

enum C extends B
    Z
```

`C` tem variantes `[X, Y, Z]`. Transitividade é natural — o resolution
faz flattening recursivo. Se houver ciclo (`A extends B`, `B extends A`),
erro compile-time: *herança cíclica de enum*.

### 3.6. O que `extends` **não** é

- **Não é subtyping.** `MeuErro` não é subtipo de `KataError`. Uma função
  que aceita `KataError` não aceita `MeuErro`. São tipos nominais distintos.
- **Não é widening.** Não há conversão implícita entre o enum filho e o base.
- **Não é enum aberto.** O base não "sabe" que foi estendido. `KataError`
  continua com 2 variantes após alguém declarar `MeuErro extends KataError`.

## 4. `expects` Tipado

### 4.1. Sintaxe

```
@test{desc: "...", expects: Tipo.Variante, args: (...)}
@test{desc: "...", expects: Tipo, args: (...)}
```

Duas formas:
- `expects: Tipo.Variante` — o teste espera que a action retorne
  `Result::Err(Variante)`. Verifica variante específica.
- `expects: Tipo` — o conjunto de testes deve cobrir todas as variantes de
  `Tipo`. Verificação de exaustividade da suite.

### 4.2. Tipo esperado vs tipo retornado

A action testada deve retornar `Result::(T, E)` onde `E` é o enum referenciado
por `expects` (ou uma extensão dele — não, sem subtyping; `E` deve ser exatamente
o enum referenciado). Se a action não retorna `Result`, `expects` é erro
compile-time: *expects requer action que retorna Result*.

### 4.3. Mudança no campo `expects`

Hoje `expects: Option<String>` (string bruta). Muda para:
`expects: Option<ExpectSpec>` onde `ExpectSpec` é:

```rust
pub enum ExpectSpec {
    /// expects: Tipo.Variante — verifica variante específica
    Variant { enum_name: String, variant: String },
    /// expects: Tipo — verifica exaustividade da suite
    Exhaustive { enum_name: String },
}
```

### 4.4. Coverage de suite (exaustividade)

Quando a action tem múltiplos `@test{expects: Tipo}`, o driver verifica que
todas as variantes de `Tipo` aparecem em pelo menos um teste. Se faltar
cobertura, warning compile-time:

```
warning: MeuErro.ValidacaoFail não coberto por nenhum @test
```

Não é erro — tests podem não cobrir todos os caminhos de erro. Mas é
informação útil para o desenvolvedor.

### 4.5. `expects: "Panic: msg"` — legacy

A forma string (`expects: "Panic: msg"`) permanece como fallback para casos
onde o erro é panic em runtime (não `Result::Err`). O parser distingue:
se o valor é `TextLit`, é string legacy; se é `EnumType.Variant` ou
`EnumType`, é forma tipada.

## 5. Interações

### 5.1. `refines` vs `extends`

Analogia estrutural:
- `refines`: tipo refined delega interface ao tipo base. O filho conhece o
  pai, o pai não conhece o filho. Relação **horizontal** (delegação).
- `extends`: enum filho herda variantes do enum base. O filho conhece o pai,
  o pai não conhece o filho. Relação **vertical** (composição).

Ambos produzem um tipo novo fechado a partir de um base. Ambos são resolvidos
estaticamente (refines no typeck dispatch, extends no resolution flattening).

### 5.2. `Result::(T, E extends KataError)`

A assinatura `Result::(T, E)` já é genérica em `E`. Com `extends`, o usuário
pode escrever:

```
action f (x::Int) => Result::(Int, MeuErro)
```

E o test runner sabe que `MeuErro` tem variantes `Panic, AssertionFail,
Timeout, ValidacaoFail`. O typeck não precisa saber que `MeuErro extends
KataError` — só precisa saber que `MeuErro` é um enum com N variantes.

### 5.3. Pattern matching

```
match (resultado)
    MeuErro.Panic: ...
    MeuErro.Timeout: ...
    MeuErro.AssertionFail: ...
    MeuErro.ValidacaoFail: ...
```

O match é sobre `MeuErro` (4 variantes). O typeck verifica exaustividade
como faria com qualquer enum de 4 variantes. As variantes herdadas
(`Panic`, `AssertionFail`) são tratadas identicamente às próprias
(`Timeout`, `ValidacaoFail`).

## 6. Implementação

### Fase 1 — `extends` no resolution

**Escopo:** flattening de variantes no Pass 0.

1. **AST:** adicionar campo `extends: Option<String>` em `EnumDecl`.
   Adicionar campo `is_final: bool` em `EnumDecl`.
2. **Parser:** aceitar `enum Nome extends Base` + corpo indentado.
   Aceitar `final enum Nome` (prefixo `final` antes de `enum`).
3. **Resolution (Pass 0):** quando `extends` é `Some(base)`:
   - Buscar `base` no `EnumRegistry`.
   - Se não existe → erro compile-time: *enum base desconhecido*.
   - Se `base` é `final` → erro compile-time: *enum base é final*.
   - Copiar variantes do base para o enum sendo registrado.
   - Verificar que nenhuma variante própria colide com herdada (redefinição).
   - Registrar o enum com a lista unionizada.
4. **`final` no EnumRegistry:** adicionar flag `is_final` em `EnumRegistry`
   para que o Pass 0 possa verificar ao processar `extends`.
5. **Transitividade:** flattening recursivo (base que também extends).
6. **Detecção de ciclo:** BFS no grafo de extends. Ciclo → erro.

**DoD Fase 1:** `enum B extends A` com variantes de A + B registra corretamente.
Pattern matching sobre B é exaustivo com todas as variantes. `B.Panic` (se
herdada de A) resolve como construtor de B.

### Fase 2 — `KataError` no prelude + `expects` tipado

**Escopo:** enum base de erros + mudança no test runner.

1. **Prelude:** declarar `enum KataError` com `Panic`, `AssertionFail`.
   (Sem `final` — é aberto por padrão, permitindo extensão.)
   Marcar `Boolean`, `Result`, `Optional` como `final` no prelude.
2. **AST/Parser:** `expects` aceita `EnumType.Variant` ou `EnumType`
   (em vez de só `TextLit`).
3. **Resolution:** `ExpectSpec` em `TestSpec` (Variant ou Exhaustive).
4. **Driver:** `expects: Tipo.Variante` verifica que o `Result::Err`
   retornado tem a variante esperada. `expects: Tipo` coleta todas as
   variantes esperadas e warning de cobertura.
5. **Legacy:** `expects: "Panic: msg"` (TextLit) permanece aceito.

**DoD Fase 2:** `@test{expects: MeuErro.Timeout}` compila e o runner
verifica a variante no `Result::Err` retornado. Warning de cobertura
aparece quando variantes não são testadas.

### Fase 3 — Generalização

**Escopo:** validar que `extends` funciona para enums além de erros de teste.

1. Testar com enums de domínio (estados de máquina, tipos de evento).
2. Testar transitividade (A → B → C).
3. Documentar no sintaxe-mapa e manual.

**DoD Fase 3:** `extends` é feature geral, não acoplada a testes.

## 7. Decisões

| ID | Decisão | Status |
|----|---------|--------|
| D1 | Extensão de enum genérico herda variantes mas não pode adicionar | Aprovada |
| D2 | Sharing de variantes herdadas (mesma identidade, não shadowing) | Aprovada |
| D3 | `extends` não é subtyping — tipos filho e pai são nominais distintos | Aprovada |
| D4 | `expects: Tipo` gera warning de cobertura, não erro | Aprovada |
| D5 | Flattening é resolution-time, não sobrevive ao codegen | Aprovada |
| D6 | `expects: "Panic: msg"` (string) permanece como legacy | Aprovada |
| D7 | Enums são abertos por padrão; `final` é opt-in para bloquear extensão | Aprovada |

## 8. Não-objetivos

- **Subtyping entre enums.** `MeuErro` não é subtipo de `KataError`.
  Não há widening, não há dispatch polimórfico sobre a hierarquia.
- **Enums abertos.** Não há extensão em runtime. Não há "adicione variante
  a um enum existente de outro módulo". O enum base é imutável.
- **Redefinição de variantes.** Uma extensão não pode mudar o payload de
  uma variante herdada. Se precisa de payload diferente, declarar enum novo.
- **`extends` em structs.** Este PRD é sobre enums. Herança de structs é
  um tópico separado.