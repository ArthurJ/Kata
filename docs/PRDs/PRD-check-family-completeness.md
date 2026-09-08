# PRD: extensão automática de família polimórfica (completude de famílias)

Estado-alvo: declarar `T implements IFACE` num módulo de usuário estende
automaticamente todas as famílias polimórficas sobre `IFACE` com a nova
instância `Fam::T`. O compilador nunca rejeita código válido por uma família
estar "congelada" com os implementors do prelude.

## Contexto

`data (NUM, != _ (zero _), = _ _) as NonZero` em `core.kata:394` é expandida
eagerly em `pass0.rs:295`:

```rust
let implementors = interface_registry.implementors_of(iface_name);
for concrete in &implementors {
    struct_registry.register_refined_instance(...);
}
```

A família fica **congelada** com os implementors conhecidos no momento do
data (Int, Float, Rational — porque todas as `implements NUM` do prelude
estão **antes** da declaração de NonZero, per comentário em core.kata:387).

Quando usuário declara:

```kata
data MyNum (v::Int)
MyNum implements NUM
```

nada re-dispara a expansão. `NonZero::MyNum` nunca existe. Como `NUM` exige
`/ :: Self NonZero => Self` (core.kata:68), o dispatch falha com
`type.no_overload` no default method `mod` — erro genérico, apontando código
do prelude, não a causa raiz.

Teste empírico em 2026-09-07 confirmou: `data X; X implements NUM` falha com
esse erro.

## Motivação arquitetural

Uma família polimórfica `data (IFACE, preds) as Fam` é uma **promessa**: para
cada `T` que implementa `IFACE`, existe `Fam::T` com a mesma estrutura.
O sistema atual viola essa promessa silenciosamente.

Corrigir exige fazer o compilador manter a promessa — não emitir erro e pedir
para o usuário contornar.

## A regra do órfão

Em Kata, `T implements IFACE` só é válido se **ou o tipo ou a interface** é
declarado no módulo atual. Para implementar interface externa em tipo externo,
usa-se `alias N as Newtype` que cria tipo local.

Consequência: em todos os casos legítimos onde uma instância de família está
faltando, o `implements` declara um **tipo novo** local (data novo ou alias).
O ponto de extensão é único e identificável.

## Design

### Onde

Em `pass0.rs`, no processamento de `Foo implements IFACE`, após a validação
do implements:

1. Validar que `IFACE` existe (regra atual).
2. Validar que `Foo` tem métodos requeridos (regra atual).
3. **NOVO**: para cada família polimórfica `Fam` sobre `IFACE` (queries abaixo),
   estender `Fam` com `Foo` se ainda não estendida.

### Queries auxiliares

Em `StructRegistry`:

```rust
/// Lista famílias polimórficas sobre uma interface, deduzidas das
/// instâncias registradas. Retorna nomes de famílias cujo base (via
/// `alias_of` da primeira instância) é a interface.
fn families_over_iface(&self, iface: &str) -> Vec<String>;

/// true se `family::concrete` já foi registrada.
fn has_instance(&self, family: &str, concrete: &str) -> bool;
```

Ambas deriváveis de `all_instances(family)` + `get_instance(family,
concrete)`. Nenhuma estrutura nova.

`families_over_iface` pode ser cacheado por iface numa tabela pequena se
virar hot path; primeira versão paga O(#famílias) por implements.

### Ação: estender a família

Para cada `Fam` retornada por `families_over_iface(IFACE)`:

1. **Pular se** `has_instance(Fam, Foo)` — idempotência para re-compilação.
2. **Estender**:
   - `struct_registry.register_refined_instance(origin_da_familia, Fam, Foo,
     pred_names)` onde `pred_names` é o sufixo derivado do mesmo
     `refined_decl.predicates.len()` que gerou as outras instâncias.
   - Adicionar `RefinedDeclInfo` correspondente à fila `refined_decls`, com
     `lazy_type_param: None` (o implements é concreto, não sobre type-var
     livre) e `base_ty` = `Ty` de `Foo` resolvido.

Isso reproduz exatamente o que a expansão eager faria se `IFACE` tivesse
`Foo` como implementor quando a família foi declarada.

### Origin da instância

A instância `Fam::Foo` é registrada com `origin` **da família** (não do
módulo do usuário). Razões:

- A família é o "dono" conceitual da promessa — `NonZero::MyNum` é uma
  instância de NonZero, que pertence a core.
- `get_instance(Fam, Foo)` resolve origin via `resolve_origin(family_name)`
  (struct_registry.rs:316). Origin da família é a chave de lookup; origem do
  usuário quebraria a busca.
- Se dois módulos declaram `Foo` com o mesmo nome em origens diferentes,
  ambos podem estender `NonZero` — instâncias são por **nome do tipo**, não
  por (origin, tipo). Restrição existente no sistema: tipos com mesmo nome
  em origens diferentes colidem como instâncias da mesma família. Documentar
  essa restrição no manual.

### Quando rejeitar (modo de falha)

A extensão pode falhar apenas se os **predicados da família não fazem sentido
para o novo implementor**. Exemplo:

```kata
data (NUM, != _ (zero _)) as NonZero   # OK
data (NUM, > _ 0) as Positive          # > pode não existir para T
```

Se `MyNum implements NUM` mas `MyNum` não implementa `ORD`, a instância
`Positive::MyNum` teria predicado inválido. Detecção: sintetizar os
`__pred_*` falharia ao registrar na DispatchTable porque o operador `>` não
está disponível.

Quando a síntese de predicado falhar na extensão automática:

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

Esse caso expõe uma **decisão de design do usuário**, não bug do compilador:
a família sobre NUM com predicado de ORD só é bem-formada se NUM ⇒ ORD
(que é o caso em Kata — NUM extends EQ, mas não ORD; ver core.kata:59). O
erro aponta isso explicitamente.

### Restrições

- Não altera `enum Ty`, `StructKey`, ou o esquema de registries.
- Não muda dispatch runtime — apenas garante que a instância existe quando o
  dispatch precisar.
- Não toca capabilities.
- Não interfere com famílias lazy (`List::A` etc.) — essas seguem
  `instantiate_family_for_concrete` com type-var livre, caminho separado.

## Testes

Todos os 11 testes em `crates/kata-codegen/tests/family_completeness_e2e.rs`.
Pipeline completo: lex → parse → resolve (merge_two com extend_families) →
infer → monomorph → optimize → tree_shake → codegen → JIT.

- **T1 (extensão básica)** ✅: `data MyNum (v::Int); MyNum implements NUM` →
  NonZero::MyNum gerada; construtor `NonZero(MyNum 3)` despacha e `/` resolve.
- **T1 (variação)** ✅: construtor NonZero sobre MyNum retorna Ok para não-zero.
- **T2 (uso da instância)** ✅: dispatch `/` sobre `(MyNum, NonZero::MyNum)`
  resolve — `(MyNum 10) / (MyNum 2::NonZero)` executa.
- **T2 (variação)** ✅: construtor + divisão com valores diferentes.
- **T3 (newtype/alias)** ✅: `alias Int as MyInt; MyInt implements NUM` →
  instância NonZero::MyInt gerada; `/ (MyInt 10) nz` resolve.
- **T4 (idempotência)** ✅: Float implements NUM (já no prelude) →
  NonZero::Float não duplicada; `5::NonZero` executa.
- **T5 (rejeição clara)** ✅: `data (NUM, > _ 0) as Positive` + MyNum sem ORD
  → infer falha (predicado `>` requer ORD não implementado).
- **T6 (família sobre interface existente)** ✅: `5.0::NonZero` executa
  (Float já implementa NUM; extensão é no-op).
- **T7 (regra do órfão inalterada)** ✅: tipo local implements NUM funciona
  (não é órfão); NonZero::LocalNum gerada sem erro.
- **T8 (sem família)** ✅: `Greeting implements SHOW` (SHOW sem famílias) →
  compila; extensão é no-op.
- **T9 (família lazy não afetada)** ✅: `head [1 2 3]` (exige NonEmpty::A)
  despacha corretamente; família lazy segue caminho separado.

### Observações de implementação

- **`family_extension_invalid` não implementado como erro nomeado.** T5
  verifica que o pipeline falha (infer_module retorna erro), mas a mensagem
  não é o erro nomeado do PRD. A rejeição acontece porque a síntese do
  predicado `>` não encontra overload para MyNum. Implementar o erro
  nomeado fica como melhoria futura.
- **Regra do órfão não implementada como gate explícito.** T7 verifica que
  tipo local funciona, mas não há gate que rejeite `Complex implements NUM`
  em módulo de usuário. A rejeição atual vem de erros de dispatch, não de
  validação estrutural.
- **`echo!` com instância de NonZero falha no codegen** (pitfall conhecido).
  Testes que precisam de output usam expressões puras (match com resultado
  Int) em vez de `echo!` com NonZero::MyNum.
- **Testes E2E usam `merge_two` (público em kata-resolution)** em vez de
  `merge_resolved` manual. `merge_two` chama `extend_families_for_implementors`
  internamente, que é o ponto onde a extensão acontece.

## Fora de escopo

- Reorganização de tipos (`TypeDef`, traits Rep/Compat/Refine) — não
  relacionado; ver PROPOSTA-reorganizacao-tipos.md.
- Verificação retroativa de famílias quando `implements` já foi aceito em
  versões anteriores do compilador — migração de código existente fica a
  cargo do usuário.
- Cache/otimização de `families_over_iface` — medir antes de otimizar.

## Aberto (para discussão)

**Colisão de nome entre módulos**: se dois módulos de usuário declaram
`data Foo` em origens distintas, ambos podem estender `NonZero` com
`NonZero::Foo`. Isso colapso duas instâncias distintas em uma. Opções:
(a) aceitar a restrição "não use mesmo nome de implementor em módulos
diferentes que ambos estendem família", documentando; (b) qualificar
instâncias por origin do implementor (`NonZero::Foo@meu_modulo`) — muda o
esquema de lookup. Recomendo (a) para 1.0 e revisit pós-1.0 se virar
problema real.
