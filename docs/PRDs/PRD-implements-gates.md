# PRD: gates estruturais de `implements` (regra do órfão + extensão de família inválida)

Estado-alvo: o compilador rejeita `T implements IFACE` com erros nomeados e
diagnósticos orientados em dois casos que hoje falham downstream com erros
genéricos de dispatch (`type.no_overload`):

1. **Regra do órfão** — `T` e `IFACE` ambos externos ao módulo do `implements`.
2. **Extensão de família inválida** — `T implements IFACE` estende uma família
   cujos predicados não são satisfeitos para `T`.

## Motivação

### Regra do órfão

`Complex implements NUM` em módulo de usuário (ambos definidos na stdlib)
deveria falhar como violação da regra do órfão. Hoje o `implements` é aceito
no `pass0` e o erro aparece downstream como `type.no_overload` em default
methods — mensagem genérica apontando código do prelude, não a causa raiz.

O `register_impl` (`interface_registry.rs:147`) apenas detecta duplicação.
`validate_impls_after_merge` (`interface_registry.rs:365`) apenas verifica se
a interface existe. Nenhum dos dois valida se o tipo ou a interface é local
ao módulo do implements.

### Extensão de família inválida

`extend_families_for_implementors` (`lib.rs:860`) estende famílias
polimórficas quando um novo implementor aparece. A função registra a
instância cegamente — não valida se os predicados da família são
sintetizáveis para o novo tipo.

Se `data (NUM, > _ 0) as Positive` e `MyNum implements NUM` mas `MyNum` não
implementa `ORD`, a instância `Positive::MyNum` é registrada. O erro aparece
quando o inference sintetiza o predicado `> _ 0` para `MyNum` e falha com
`type.no_overload` — sem indicar que a causa é a extensão da família.

O PRD-check-family-completeness já especifica o erro nomeado
`type.family_extension_invalid` no §"Quando rejeitar", mas observa que não
está implementado. Este PRD fecha essa lacuna.

## Design

### Gate 1: regra do órfão

#### Onde

Em `validate_impls_after_merge` (`interface_registry.rs:365`), chamada após
o merge do prelude em `lib.rs:652`. Neste ponto, todos os registries estão
populados: interfaces do prelude + usuário, tipos de todos os módulos.

A função já percorre todos os `ImplEntry`s para validar se a interface
existe. O gate do órfão é uma verificação adicional no mesmo percurso.

#### Como

Para cada `ImplEntry` com origin `O`:

1. Consultar `struct_registry.origins_of(type_name)` e
   `enum_registry.origins_of(type_name)` — origins que definem o tipo.
2. Consultar `interface_registry.origins_of(interface_name)` — origins que
   definem a interface.
3. Se `O` não aparece em nenhuma das listas → `type.orphan_impl`.

Tipos primitivos (`Int`, `Float`, `Text`, `Rational`) não têm entrada em
nenhum registry. Um `implements` sobre tipo primitivo em módulo de usuário
é sempre órfão (a menos que a interface seja local) — alinhado com a
semântica: primitivos são "externos" por definição.

#### Assinatura

`validate_impls_after_merge` hoje recebe apenas `&self` e retorna
`Vec<String>` (warnings). Para o gate do órfão, precisa consultar
`struct_registry` e `enum_registry`. Duas opções:

- **(a)** Passar `&StructRegistry` e `&EnumRegistry` como parâmetros
  adicionais. Mudança de assinatura mas sem nova estrutura.
- **(b)** Mover a validação para uma função separada
  `validate_orphan_rule(&interface_registry, &struct_registry,
  &enum_registry) -> Vec<ResolveError>`, chamada no mesmo ponto.

Recomendar (b): separa responsabilidades, retorna `ResolveError` (não
`String`), e `validate_impls_after_merge` mantém sua assinatura.

#### Erro

```
Error: type.orphan_impl

  × `Complex implements NUM` viola a regra do órfão
  ╭─[main.kata:5:1]
  5 │ Complex implements NUM
    · ────────┬────────
    ·           ╰── implements em módulo de usuário
    │
    │ `Complex` é definido em stdlib/complex.kata
    │ `NUM` é definido em stdlib/core.kata
    │ Para implementar interface externa em tipo externo, use `alias`:
    │   alias Complex as MyComplex
    │   MyComplex implements NUM
  ╰────
```

`ResolveError::OrphanImpl` com campos `type_name`, `interface_name`,
`impl_origin`, `type_origin`, `iface_origin`, `span: MietteSpan` (span do
`implements`, do `ImplEntry.span`). `OrphanImpl` é o primeiro variante de
`ResolveError` com `#[label] span` — a infraestrutura de `MietteSpan` já
existe em `kata-diagnostics/frontend.rs` e `kata-resolution` já depende de
`kata-diagnostics`.

### Gate 2: extensão de família inválida

#### Onde

No inference, na síntese de predicados de `RefinedDeclInfo`.

#### Como

`extend_families_for_implementors` cria `RefinedDeclInfo` para cada
instância estendida. Hoje, esses `RefinedDeclInfo` são indistinguíveis dos
originais. O inference sintetiza predicados para todos e, se a síntese falha,
emite `type.no_overload`.

O fluxo:

1. `extend_families_for_implementors` marca cada `RefinedDeclInfo` criado
   por extensão com o par `(type_name, iface_name)` que disparou a extensão.
2. O inference, ao sintetizar predicados de um `RefinedDeclInfo` marcado,
   se a síntese falha com `NoOverload`, emite
   `MiddleError::FamilyExtensionInvalid` em vez de `NoOverload`.

#### Campo em RefinedDeclInfo

```rust
pub struct RefinedDeclInfo {
    pub name: String,
    pub base_ty: Ty,
    pub predicates: Vec<Spanned<Expr>>,
    pub lazy_type_param: Option<String>,
    /// Some((type_name, iface_name, span)) se esta instância foi criada por
    /// extensão automática de família via `extend_families_for_implementors`.
    /// None para instâncias originais (declaradas no pass0).
    pub extension_impl: Option<(String, String, Span)>,
}
```

Todos os pontos que criam `RefinedDeclInfo` no pass0 passam `None`.
Apenas `extend_families_for_implementors` passa `Some((type_name,
iface_name, span))`. O span é obtido do `ImplEntry.span` (campo adicionado
na Fase 1) via `interface_registry.impls_view()` — disponível no momento
da criação. O span viaja com o `RefinedDeclInfo` até o inference, onde
`FamilyExtensionInvalid` o usa diretamente, sem busca adicional.

#### Erro

```
Error: type.family_extension_invalid

  × `MyNum implements NUM` estendeu a família `Positive`, mas o predicado
    da família não é válido para `MyNum`
  ╭─[main.kata:5:1]
  5 │ MyNum implements NUM
    · ──┬─
    ·   ╰── estende Positive::MyNum
    │
    │ O predicado `> _ 0` requer ORD, que MyNum não implementa.
    │ Declare `MyNum implements ORD` antes, ou declare `Positive` com
    │ uma interface menos ampla (ex: `data (NUMORD, ...) as Positive` com
    │ `interface NUMORD implements NUM ORD`).
  ╰────
```

`MiddleError::FamilyExtensionInvalid` com campos `type_name`,
`iface_name`, `family_name`, `missing_ifaces: Vec<String>` (interfaces
que definem o método cujo dispatch falhou e que o tipo não implementa),
`span` (span do `implements`).

O `span` do implements: `extend_families_for_implementors` não tem acesso
ao span do `ImplementsDecl`. Para produzir o diagnóstico com span correto,
a função precisa receber os spans dos impls. Como os `ImplEntry` já têm
`origin` mas não `span`, adicionar `span: Span` ao `ImplEntry` (ou um mapa
paralelo `(type_name, iface_name) → Span` construído no pass0).

#### Ponto de interceptação em `synthesize_refined`

Hoje `synthesize_refined` (constructors_refined.rs:161) faz:
```rust
let typed_body = infer_expr(&desugared.node, &desugared.span, &mut pred_env, &ctx, true)?;
```

O `?` propaga `MiddleError::NoOverload` diretamente. Para mapear para
`FamilyExtensionInvalid`, a interceptação ocorre dentro do loop
`for (i, pred_name) in pred_names.iter().enumerate()` (linha 149), onde
`decl` (o `RefinedDeclInfo` atual) está disponível:

```rust
let typed_body = match infer_expr(&desugared.node, &desugared.span, &mut pred_env, &ctx, true) {
    Ok(t) => t,
    Err(MiddleError::NoOverload { name, .. }) if decl.extension_impl.is_some() => {
        let (type_name, iface_name, span) = decl.extension_impl.unwrap();
        let missing_ifaces = method_to_ifaces[&name]
            .iter()
            .filter(|iface| !interface_registry.implements(type_name, iface))
            .cloned()
            .collect();
        return Err(MiddleError::FamilyExtensionInvalid {
            type_name, iface_name, family_name: decl.name.clone(),
            missing_ifaces, span: span.into(),
        });
    }
    Err(e) => return Err(e),
};
```

Se qualquer predicado falhar com `NoOverload` e `decl.extension_impl` for
`Some(...)`, mapeia para `FamilyExtensionInvalid`. Predicados de instâncias
originais (`extension_impl = None`) propagam `NoOverload` normalmente.

A síntese do predicado falha com `NoOverload` para um operador (ex: `>`).
Para produzir a mensagem orientada ("o predicado `> _ 0` requer ORD"), o
erro precisa saber qual operador falhou e qual interface ele exige.

O `NoOverload` já carrega `name` (nome do operador, ex: `>`). A relação
operador → interface é derivável: o `interface_registry` lista quais
interfaces definem aquele método. Se `>` é método de `ORD`, a mensagem
aponta ORD.

Para evitar uma busca O(#interfaces) por operador, um mapa
`method_name → Vec<iface_name>` pode ser pré-computado no
`interface_registry` uma vez (todas as assinaturas de todas as interfaces).
O mapa é pequeno (dezenas de entradas) e reutilizável.

## Decisões de design

### D1: gate do órfão em validate pós-merge, não no pass0

**Escolhido:** validar após merge do prelude.
**Alternativa rejeitada:** validar no pass0 (`pass0.rs:231`), ao processar o
`ImplementsDecl`. Rejeitado porque no pass0 as interfaces do prelude ainda
não estão visíveis — `origins_of(interface_name)` retornaria vazio para
interfaces do prelude, produzindo falsos positivos.

### D2: orphan_impl como ResolveError, não MiddleError

**Escolhido:** `ResolveError::OrphanImpl`.
**Razão:** a violação é estrutural (origin do tipo vs origin da interface),
detectada no resolution, não no inference. `ResolveError` é o enum correto
para erros de resolution.

### D3: family_extension_invalid como MiddleError, não ResolveError

**Escolhido:** `MiddleError::FamilyExtensionInvalid`.
**Razão:** a falha é detectada na síntese de predicados (inference), não no
resolution. O `RefinedDeclInfo` é criado no resolution mas a validação dos
predicados acontece no inference. O erro carrega o span do implements para
apontar a causa, mas o ponto de detecção é o inference.

### D4: marcar RefinedDeclInfo em vez de pré-validar predicados

**Escolhido:** marcar `extension_impl` no `RefinedDeclInfo` e mapear o erro
no inference.
**Alternativa rejeitada:** pré-validar predicados em
`extend_families_for_implementors` antes de registrar a instância. Rejeitado
porque os predicados são `Spanned<Expr>` com holes — pré-validar exigiria
duplicar a lógica de síntese de dispatch sem o contexto completo do
inference. Deixar o inference tentar e capturar o erro é mais robusto e
não duplica lógica.

### D5: span do implements via ImplEntry

**Escolhido:** adicionar `span: Span` ao `ImplEntry`.
**Alternativa rejeitada:** mapa paralelo `(type_name, iface_name) → Span`.
Rejeitado porque `ImplEntry` é a estrutura natural para carregar metadados
do implements, e o span é um metadado. Um mapa paralelo seria uma estrutura
auxiliar frágil (chaves duplicadas em caso de overloading).

### D6: método → interface via mapa pré-computado

**Escolhido:** pré-computar `method_name → Vec<iface_name>` no
`interface_registry`.
**Razão:** a relação é estática (definida pelas assinaturas de cada
interface) e reutilizável. Evita O(#interfaces) por operador na mensagem
de erro.

## Fases

### Fase 1: gate do órfão

**Escopo:** rejeitar `T implements IFACE` quando ambos são externos.

- Adicionar `ResolveError::OrphanImpl` com campos `type_name`,
  `interface_name`, `impl_origin`, `type_origin`, `iface_origin`.
- Adicionar `span: Span` ao `ImplEntry` (preenchido no pass0 a partir do
  `ImplementsDecl`).
- Implementar `validate_orphan_rule(&interface_registry,
  &struct_registry, &enum_registry) -> Vec<ResolveError>`.
- Chamar após `validate_impls_after_merge` em `lib.rs:652`.

**DoD:**

- `Complex implements NUM` em módulo de usuário → `type.orphan_impl`.
- `data Local; Local implements NUM` → compila (não é órfão).
- `alias Int as MyInt; MyInt implements NUM` → compila.
- `Int implements NUM` no prelude → compila (origin da stdlib).

### Fase 2: family_extension_invalid

**Escopo:** diagnóstico orientado quando extensão de família falha.

- Adicionar `extension_impl: Option<(String, String)>` ao
  `RefinedDeclInfo`. Todos os pontos de criação no pass0 passam `None`.
- `extend_families_for_implementors` passa `Some((type_name,
  iface_name))` nas instâncias que cria.
- Pré-computar `method_name → Vec<iface_name>` no `interface_registry`.
- Adicionar `MiddleError::FamilyExtensionInvalid` com campos `type_name`,
  `iface_name`, `family_name`, `missing_ifaces: Vec<String>`, `span`.
- No inference, ao sintetizar predicados de `RefinedDeclInfo` com
  `extension_impl = Some(...)`, se a síntese falha com `NoOverload`,
  mapear para `FamilyExtensionInvalid` usando o mapa
  `method_name → iface_name` para produzir a mensagem.

**DoD:**

- `data (NUM, > _ 0) as Positive` + `MyNum implements NUM` sem ORD →
  `type.family_extension_invalid` com mensagem apontando ORD.
- `MyNum implements NUM` com ORD → compila (predicado `>` funciona).
- `data MyNum; MyNum implements NUM` (sem família com predicado ORD) →
  compila (NonZero só exige `!= _ (zero _)`, que usa `=` de EQ).

## Estruturas afetadas

| Camada | Arquivo | Mudança |
|--------|---------|---------|
| resolution | `pass0.rs` | Preencher `span` em `ImplEntry` |
| resolution | `lib.rs` | Chamar `validate_orphan_rule` pós-merge |
| core | `interface_registry.rs` | `ImplEntry` ganha `span`; mapa `method → ifaces` |
| core | `types.rs` (kata-resolution) | `RefinedDeclInfo` ganha `extension_impl: Option<(String,String,Span)>` |
| resolution | `lib.rs` | `extend_families_for_implementors` marca `extension_impl` |
| diagnostics | `types.rs` (kata-resolution) | `ResolveError::OrphanImpl` |
| diagnostics | `middleend.rs` | `MiddleError::FamilyExtensionInvalid` |
| inference | síntese de predicados | Mapear `NoOverload` → `FamilyExtensionInvalid` |

## Testes

### Gate 1 — regra do órfão

- **O1:** `Complex implements NUM` em módulo de usuário →
  `type.orphan_impl`.
- **O2:** `data Local (v::Int); Local implements NUM` → compila.
- **O3:** `alias Int as MyInt; MyInt implements NUM` → compila.
- **O4:** `interface LOCAL; data Local; Local implements LOCAL` →
  compila (ambos locais).
- **O5:** `data Local; Local implements SHOW` (SHOW externa, Local local)
  → compila (tipo é local).

### Gate 2 — family_extension_invalid

- **F1:** `data (NUM, > _ 0) as Positive` + `MyNum implements NUM` sem
  ORD → `type.family_extension_invalid`.
- **F2:** Mesma família + `MyNum implements NUM ORD` → compila.
- **F3:** `data (NUM, != _ (zero _)) as NonZero` + `MyNum implements NUM`
  sem EQ → `type.family_extension_invalid` (NonZero exige `!=` que
  precisa de EQ).
- **F4:** `data (NUM, != _ (zero _)) as NonZero` + `MyNum implements NUM`
  com EQ → compila.

## Fora do escopo

- Pré-validação de predicados em `extend_families_for_implementors` sem
  depender do inference — decidido contra (D4).
- Generalização do gate do órfão para `refines` — `refines` já tem
  validação em `InvalidRefines`; não relacionado.
- Colisão de nome entre módulos na extensão de família — já discutido no
  PRD-check-family-completeness §"Aberto".