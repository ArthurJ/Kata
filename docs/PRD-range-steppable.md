# PRD — Range Float + STEPPABLE + Default Methods

**Data:** 2026-08-17
**Status:** Concluído
**Autor:** Arthur + Hermes

## Contexto

O range hoje é praticamente Int-only no codegen, apesar do typeck aceitar
`Range(Float)`. O `range_iter.rs` usa `icmp` signed (i64) para condição de
parada — compara bits de float como inteiros, produzindo comportamento
incorreto.

Ao corrigir o Float, surge a questão do step opcional: `[0..10]` como
shorthand para `[0..1..10]`. Step default exige um mecanismo para definir
"qual é o incremento natural" de um tipo — análogo ao `@associative(N)`
que define o elemento neutro de um operador.

A solução proposta é uma interface `STEPPABLE` com default methods: a
interface fornece um `step` default, e tipos concretos podem sobrescrever
(shadow) com sua própria implementação. O typeck resolve `step` em
compile-time — insere o valor como literal no `RangeLit`, e o `+` do tipo
avança a iteração no codegen. Sem ponteiros, sem runtime dispatch, sem
Optional.

Isso exige duas mudanças estruturais no Kata5:

1. **Self substitution no monomorphizador** — `Self` nas assinaturas de
   interface precisa ser substituído pelo tipo concreto durante a
   instanciação
2. **Default methods em interfaces** — interfaces podem fornecer
   implementação default que o impl pode sobrescrever

## Objetivos

1. Range de Float funciona corretamente no codegen
2. Step opcional: `[0..10]` usa step default do tipo
3. Interface `STEPPABLE` com `step :: Self => Self`
4. Default methods em interfaces com shadowing
5. Self substituído pelo tipo concreto no monomorphizador

## Não-objetivos

- Tensor (PRD separado)
- Range bare sem colchetes (avaliar depois)
- Generalização para tipos além de Int/Float/Rational (design permite, mas
  não é objetivo implementar)

---

## Design

### 1. Interface `STEPPABLE`

```kata
interface STEPPABLE
    step :: Self => Self
    lambda x:
        x
```

- `step` retorna `Self`: o incremento natural do tipo.
- Default method (com `lambda`) retorna `x` — identidade (sem step
  default significativo). Tipos que querem um step default fazem shadow:

```kata
Int implements STEPPABLE
    step :: Int => Int
    lambda x:
        1

Float implements STEPPABLE
    step :: Float => Float
    lambda x:
        1.0
```

O typeck chama `step` em **compile-time** e insere o valor resultante
como literal no `RangeLit`. O `+` do tipo (já existente no NUM/ORD)
avança a iteração no codegen — sem mudança no runtime.

Tipos que não implementam STEPPABLE não podem omitir step — erro de
tipo em compile-time.

### 2. Sintaxe de default methods em interfaces

Não há keyword `default`. A presença de `lambda` após a assinatura
distingue assinatura pura de default method — exatamente como já
funciona em `implements` hoje.

```kata
interface STEPPABLE
    step :: Self => Self
    lambda x:
        x
```

Assinatura sem `lambda` = método obrigatório (impl deve definir).
Assinatura com `lambda` = default method (impl pode sobrescrever).

Métodos (com ou sem corpo) são separados entre si por **linha em
branco** (`StmtSep` duplo). A linha em branco é o separador estrutural
de métodos dentro da interface — facilita o parser (cada método é uma
unidade coesa delimitada por linha em branco) e a leitura:

```kata
interface FOO
    bar :: Self => Self
    lambda x:
        x

    baz :: Self => Self
```

O parser agrupa assinatura + lambda num único método quando não há
linha em branco entre eles. A linha em branco marca a fronteira entre
um método e o próximo.

### 3. AST

`InterfaceSig` ganha corpo opcional — mesmo padrão de `ImplMethod`:

```rust
pub struct InterfaceSig {
    pub name: String,
    pub params: Vec<Spanned<TypeExpr>>,
    pub ret: Spanned<TypeExpr>,
    /// Default method body. None = assinatura obrigatória (sem default).
    /// Some = default method com corpo Kata (cláusulas lambda).
    pub default_body: Option<Vec<Spanned<LambdaClause>>>,
}
```

### 4. Parser

`parse_interface_decl` precisa, após parsear uma assinatura:

1. Consumir `StmtSep` (newline entre assinatura e possível corpo)
2. Se o próximo token é `Lambda` — parsear o corpo como
   `parse_sig_clauses()` (mesma função usada em `implements`)
3. Se não é `Lambda` — assinatura pura, sem default

Isso é idêntico ao padrão em `parse_implements_decl` (linhas 353-360),
que já faz: consome StmtSep, checa `Lambda`, se sim parseia cláusulas.

A linha em branco (`StmtSep` duplo) entre métodos é o separador
estrutural — o parser a usa para delimitar onde termina um método
(assinatura + opcionalmente lambda) e começa o próximo.

### 5. Resolution

`InterfaceInfo.signatures` passa a carregar `default_body` quando existir.

`ImplEntry.methods` já tem `body: Option<Vec<LambdaClause>>` — não muda.

A novidade no dispatch: quando o typeck procura um método de interface
para um tipo concreto:

1. Se o `ImplementsDecl` define o método → usa o impl concreto (shadow)
2. Se não define → verifica se a interface tem `default_body` → usa o
   default
3. Se nenhum dos dois → erro de conformidade (como hoje)

### 6. Self substitution no monomorphizador

Hoje `Self` é `Ty::Var("Self")` — placeholder que o unifier aceita como
casando com qualquer coisa (`(Ty::Var(_), _) => Ok(())`). Mas nunca é
substituído pelo tipo concreto.

Para `STEPPABLE` funcionar, o monomorphizador precisa substituir `Self`
pelo tipo concreto ao instanciar um impl:

- `Int implements STEPPABLE` com `step :: Int => Int` —
  `Self` já está resolvido para `Int` no impl (assinatura concreta).
- Default method da interface: `step :: Self => Self` com
  corpo `x` — quando despachado para `Int`, `Self` vira `Int` no tipo
  de retorno.

Isso exige que o monomorphizador, ao instanciar um default method de
interface para um tipo concreto, aplique a substituição `Self → Tipo`
na assinatura e no corpo.

### 7. Step opcional no parser de range

`parse_list_or_range` hoje exige `start..step..end` (3 componentes).

Para step opcional, o parser precisa distinguir:

- `[0..10]` — 2 componentes, step default
- `[0..2..10]` — 3 componentes, step explícito
- `[0..=10]` — 2 componentes, step default, inclusive

Após parsear `start` e ver `..`:
1. Parseia próximo elemento
2. Se vê `..` ou `..=` — é step explícito (comportamento atual)
3. Se vê `]` — é end (step default). Se era `..=` → inclusive.

Para `..=` com 2 componentes: `[0..=10]` — start=0, step=default, end=10,
inclusive=true. O primeiro `..` é sempre exclusive (separa start do resto);
se for `..=`, é inclusive direto (step default, end inclusive).

### 8. Typeck do range com step default

`infer_range_lit` hoje exige 3 componentes (start, step, end) do mesmo
tipo.

Com step opcional:

1. Se step está presente — comportamento atual
2. Se step está ausente — o typeck precisa determinar o step default:
   a. Inferir o tipo de `start` e `end`
   b. Verificar se o tipo implementa `STEPPABLE`
   c. Se implementa, chamar `step` em **compile-time** para obter o valor
   d. Inserir o valor como literal no `RangeLit` (açúcar sintático —
      o `+` do tipo avança a iteração no codegen, sem mudança de runtime)
   e. Se não implementa STEPPABLE — erro: "tipo não suporta step default"
3. Se os tipos de start e end diferem — erro (como hoje)

A chamada `step` é resolvida em compile-time (const evaluation ou
evaluação do lambda no typeck). O resultado vira um literal no TAST —
o range struct continua armazenando 4 words (start, step, end,
inclusive), sem ponteiros ou dispatch em runtime.

### 9. Codegen de Float no range

`range_iter.rs` hoje é hardcoded para i64 signed. Precisa de:

1. **Despacho por tipo no TAST:** o typeck já conhece o tipo — insere a
   informação no TAST (`TypedExprKind::RangeLit { ..., elem_ty }`) e o
   codegen despacha por tipo. Sem mudança de layout do runtime.

2. **Condição de parada para Float:** usar `float_cmp` (Cranelift) em vez
   de `icmp`. As 4 condições (step pos/neg × excl/incl) permanecem
   iguais, só muda a instrução de comparação.

3. **Layout do runtime para Float:** hoje `kata_rt_range_alloc` aloca
   32 bytes (4×i64). Para Float, start/step/end seriam f64 (8 bytes cada)
   — mesmo tamanho, só a interpretação muda. O codegen despacha por
   `elem_ty` do TAST para saber qual instrução de comparação usar.

---

## Fases

### Fase 1: Self substitution no monomorphizador ✅

- Monomorphizador substitui `Ty::Var("Self")` pelo tipo concreto ao
  instanciar impls de interface (commit `e28f172`)
- Testes: impl que usa `Self` no retorno produz tipo concreto correto
  na TAST monomorfizada

### Fase 2: Default methods em interfaces ✅

- AST: `InterfaceSig.default_body: Option<Vec<Spanned<LambdaClause>>>`
- Parser: após assinatura, se próximo token (após StmtSep) é `Lambda`,
  parseia corpo como default method — mesmo padrão de `implements`
- Resolution: `InterfaceInfo` carrega default_body
- Dispatch: fallback para default method quando impl não define
- Testes: 3 testes em `default_methods.rs` (commit `5fce36e`)

### Fase 3: Interface STEPPABLE + step opcional ✅

- Prelude: `STEPPABLE` com default `step` (identidade) (commit `9314dab`)
- Prelude: `Int implements STEPPABLE` com `step` retornando `1`
- Prelude: `Float implements STEPPABLE` com `step` retornando `1.0`
- Parser: step opcional em range (`[0..10]` = step default via Hole)
  (commit `1cd53b5`)
- Typeck: `infer_range_lit` despacha `step` via STEPPABLE quando step é
  Hole — insere literal no TAST (Int→1, Float→1.0) (commit `115dc61`)
- Testes: 5 parser, 5 inferência

### Fase 4: Codegen de Float no range ✅

- `range_iter.rs`: `range_done` despacha por `elem_ty` (Int=icmp,
  Float=fcmp com FloatCC) (commit `7afaa7f`)
- `range_iter.rs`: `range_advance` despacha por `elem_ty` (Int=iadd+SMI,
  Float=fadd+bitcast)
- 5 callers atualizados (map, filter, fused_stream, collections_hof,
  for_in)
- Testes E2E: 4 testes AOT em `build_e2e.rs` — Int/Float × step
  default/explicit, inclusive

---

## Decisões de design

### Por que `Self` direto (sem Optional) no step?

O typeck resolve `step` em compile-time e insere o valor como literal no
`RangeLit`. Se o tipo não implementa STEPPABLE, é erro de tipo antes do
codegen. Optional seria necessário apenas se o step fosse decidido em
runtime — mas como é compile-time, `Self` direto é suficiente e mais
simples. Tipos sem step default significativo simplesmente não implementam
STEPPABLE.

### Por que `lambda` em vez de keyword `default`?

A presença de `lambda` após a assinatura já é o discriminador natural —
é exatamente como `ImplMethod` funciona hoje (`body: None` = assinatura,
`body: Some` = implementação). Introduzir `default` seria redundante e
inventar sintaxe nova onde já existe um padrão estabelecido.

### Por que linha em branco obrigatória?

A linha em branco é o separador estrutural de métodos dentro da
interface. O parser a usa para delimitar cada método (assinatura +
opcionalmente lambda) como uma unidade coesa. Sem ela, o parser teria
que adivinhar se um `lambda` pertence à assinatura anterior ou é o
início de algo novo. Com ela, cada método é uma unidade delimitada —
facilita o parser e a leitura.

### Por que não `@default_unit(1)` (diretiva em tipo)?

A diretiva seria mais simples mas menos expressiva: não permite lógica
(condicional, cálculo). A interface `STEPPABLE` com default method
permite que o step default seja computado, não apenas um literal. E é
ortogonal a NUM — um tipo pode ser STEPPABLE sem ser NUM.

### Por que step default em compile-time?

O range struct no runtime armazena 4 words (start, step, end,
inclusive) — todas i64 ou f64, sem ponteiros. Se o step fosse decidido
em runtime, seria preciso armazenar um ponteiro para função ou tag de
dispatch no struct, complicando o layout. Em compile-time, o typeck
insere o valor literal — o range struct não muda, o codegen não muda, e
a iteração usa o `+` do tipo que já existe no codegen.

### Self: por que substituir no monomorphizador e não no typeck?

O typeck já aceita `Self` como casando com qualquer coisa. A substituição
real precisa acontecer quando o tipo concreto é conhecido — que é no
monomorphizador, ao instanciar o impl para o tipo específico. Substituir
no typeck exigiria conhecer o tipo concreto durante a inferência, o que
não é o caso para interfaces genéricas.