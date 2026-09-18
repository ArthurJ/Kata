# PRD: erros granulares para extensão de família polimórfica

Estado-alvo: `T implements IFACE` que não define todos os métodos de
IFACE produz **erro de implementação incompleta**, controlável via
`#!allow type.incomplete_interface`. `Fam::T` é registrada mesmo com
implementação incompleta, mas **usar** `Fam::T` sem a sobrecarga que o
predicado requer produz **erro de uso** claro, não controlável —
indicando qual sobrecarga está faltando e onde.

## Contexto

`data (NUM, != _ (zero _), = _ _) as NonZero` em `core.kata:519`.
`NUM` exige `zero :: Self => Self` (core.kata:81).

`private_type.kata` declara:

```kata
data Internal (val::Int)
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)
```

`Internal implements NUM` define `+` mas não `zero`, `-`, `*`, `div`,
`/`, `mod`, `//`, `abs`. Hoje isso **não provoca erro** — o resolver
registra a impl e segue.

`extend_families_for_implementors` (lib.rs:856) estende `NonZero` com
`NonZero::Internal`. O inference sintetiza o construtor falível, que
inclui o predicado `!= _ (zero _)`. O predicado chama `zero Internal`
— não existe overload. Erro atual:

```
Error: type.family_extension_invalid

  × `Internal implements NUM` estendeu a família `NonZero`, mas o
    predicado da família não é válido para `Internal`
```

### Problemas do comportamento atual

1. **Erro na criação, não no uso.** `NonZero::Internal` é rejeitado na
   síntese eager do construtor, mesmo que o usuário nunca construa um
   `NonZero::Internal`. Bastaria `Internal implements NUM` existir para
   o erro disparar.

2. **Mensagem não diz o que está faltando.** "Predicado não é válido"
   não informa que `zero` está faltando. O usuário não sabe o que fazer.

3. **Implementação incompleta de NUM não é detectada.** `Internal`
   define só `+` de 8 métodos de NUM. O compilador aceita silenciosamente.
   O erro só aparece como efeito colateral da extensão de NonZero — se
   não houvesse NonZero, `Internal implements NUM` incompleto passaria
   impune.

4. **Sem via de escape.** Even se o usuário sabe que `Internal` é
   incompleto e quer proibir `NonZero::Internal`, não há como suprimir
   o erro para compilar o resto.

## Design

### Erro 1: implementação incompleta de interface

**Quando:** ao processar `T implements IFACE`, após validar que IFACE
existe e que T é válido (regras atuais), verificar que todos os métodos
de IFACE têm sobrecarga definida para T.

**Como:** para cada método `m :: params => ret` na interface IFACE,
procurar overload de `m` com `Self = T` no DispatchTable. Se algum
método não tem overload:

```
Error: type.incomplete_interface

  × `Internal implements NUM` não define todos os métodos de NUM
    ╭─[private_type.kata:7:1]
  7 │ Internal implements NUM
    · ──┬─
    ·   ╰── 7 métodos faltando:
    │
    │   zero :: Internal => Internal
    │   -    :: Internal Internal => Internal
    │   *    :: Internal Internal => Internal
    │   div  :: Internal Internal => Result::(Internal, Text)
    │   /    :: Internal NonZero => Internal
    │   //   :: Internal NonZero => Int
    │   abs  :: Internal => Internal
    │
    │ Para silenciar: #!allow type.incomplete_interface
    │ Para warning:  #!warn  type.incomplete_interface
    ╰────
```

**Requisito de mensagem:** o código do diagnóstico deve aparecer
pelo menos 1x na saída do erro (cabeçalho já satisfaz). A instrução
de controle (`#!allow ...`) na ajuda só aparece em diagnósticos
controláveis. Diagnósticos não controláveis não oferecem controle.

**Metadados de diagnóstico:** cada diagnóstico no enum de erros
(`kata-diagnostics`) declara seu default e controlabilidade via
atributo:

```rust
#[diagnostic(code = "type.incomplete_interface", severity = "deny", controllable = true)]
IncompleteInterface { ... }

#[diagnostic(code = "type.missing_overload", severity = "deny", controllable = false)]
MissingOverload { ... }
```

- `severity` — `deny` (erro, padrão) ou `warn` (warning). Define o
  comportamento quando o usuário não escreve `#!`.
- `controllable` — `true` permite `#!` sobrescrever o `severity`;
  `false` ignora `#!` (sempre segue o `severity` declarado).

Os 38 diagnósticos existentes são implicitamente `severity = "deny",
controllable = false` — comportamento inalterado. Apenas os novos
diagnósticos que precisam de controle declaram os atributos.

**Controle:** três níveis via pseudo-comentário no bloco `implements`:

```kata
# Silencia o diagnóstico (default do compador é deny = erro)
Internal implements NUM #!allow type.incomplete_interface
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Emitir como warning (compilação continua)
Internal implements NUM #!warn type.incomplete_interface
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Elevar a erro (explicita o default)
Internal implements NUM #!deny type.incomplete_interface
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)
```

`#!allow type.incomplete_interface` silencia o erro 1, mas **não**
afeta o erro 2 — usar `NonZero::Internal` ainda falha se `zero` falta.

### Erro 2: uso de instância de família sem sobrecarga

**Quando:** o codegen/monomorphizador encontra uma chamada que precisa
de uma sobrecarga que não existe para o tipo concreto. Ex: o predicado
`!= _ (zero _)` de `NonZero::Internal` precisa de `zero :: Internal =>
Internal`, que não está definida.

**Como:** hoje isso já falha, mas com mensagem genérica
(`family_extension_invalid`). Mudar para:

```
Error: type.missing_overload

  × `NonZero::Internal` requer a sobrecarga `zero :: Internal =>
    Internal`, mas `Internal` não define `zero`
    ╭─[private_type.kata:7:1]
  7 │ Internal implements NUM
    · ──┬─
    ·   ╰── `zero` não definido nesta implementação
    │
    │ O predicado `!= _ (zero _)` da família `NonZero` chama `zero`
    │ sobre o tipo base. Sem a sobrecarga, o predicado não pode ser
    │ avaliado em runtime.
    │
    │ Para resolver:
    │   1. Defina `zero :: Internal => Internal` no implements
    │   2. Ou evite usar `NonZero::Internal` (o tipo base Internal
    │      funciona sem o refinamento)
    ╰────
```

`type.missing_overload` **não é controlável** — não há
`#!allow type.missing_overload`. É um erro de uso, não de
declaração.

**Não controlável.** É um erro de uso — o programa tenta executar
algo que não tem implementação. `#!allow type.incomplete_interface`
controla a *declaração* incompleta, não o *uso* de algo que não
existe.

### Síntese do construtor: eager vs lazy

Hoje `extend_families_for_implementors` registra a instância e
`RefinedDeclInfo` com `extension_impl: Some(...)`, e o inference
sintetiza o construtor eager. Se o predicado falha, o erro dispara
na síntese — antes de qualquer uso.

**Ideal (lazy):** quando `extension_impl` é `Some(...)`, não
sintetizar o construtor eager. Registrar a instância no
`struct_registry` (para que `NonZero::Internal` seja um tipo
válido), mas marcar o `RefinedDeclInfo` como `lazy = true`. O
construtor só é sintetizado quando um call site `NonZero(...)` com
`Internal` é encontrado. Isso garante que `Internal implements NUM
#!allow type.incomplete_interface` compila sem erro se
`NonZero::Internal` nunca é usado.

**Porém:** a síntese eager pode ser necessária para corretude em
casos que não são óbvios — ex: expansão de signatures que usam
`Family("NonZero")`, dispatch de métodos default de NUM que
referenciam `NonZero` no tipo (`/ :: Self NonZero => Self`), ou
outras passadas que dependem do construtor existir no
`DispatchTable` antes do monomorphizador rodar.

### Fase de investigação (pré-implementação)

Antes de mudar eager → lazy, investigar:

1. **Quem consome o construtor sintetizado?** Mapear todos os
   pontos do pipeline que dependem de `__kata_show__NonZero::T`,
   `__pred_NonZero_T_*`, ou do `RefinedDeclInfo` da instância
   existir com predicados resolvidos. Se algo além do call site
   `NonZero(...)` depende disso, lazy quebra.

2. **Dispatch de métodos default de NUM.** `NUM` tem
   `/ :: Self NonZero => Self`. Quando `Internal implements NUM`
   sem definir `/`, o default method do prelude precisa de
   `NonZero::Internal` no DispatchTable? Ou o default é instanciado
   só quando `/` é chamado? Se eager, a instância precisa existir
   antes. Se lazy, só no call site.

3. **Expansão de signatures com `Family("NonZero")`.**
   `expand_family_signatures` (lib.rs:949) itera sobre instâncias
   registradas. Se `NonZero::Internal` está registrada no
   `struct_registry` mas sem predicados resolvidos, a expansão
   gera signatures com tipos inválidos? Ou só importa o
   `StructKey::Instance`, não os predicados?

4. **Impacto nos testes existentes.** Os 11 testes em
   `family_completeness_e2e.rs` assumem eager. Quantos quebram
   com lazy? Se a maioria depende de eager, lazy é caro demais.

5. **Alternativa: eager com erro deferido.** Manter eager mas,
   quando o predicado falha, registrar a instância como
   "inválida" (sem abortar) e emitir o erro só no uso. Isso
   preserva o timing do eager sem o custo de reestruturar para
   lazy. Viável?

**Resultado da investigação determina o caminho:** lazy puro,
eager com erro deferido, ou híbrido. A tabela de interação entre
os dois erros e a fase de testes devem ser ajustadas conforme o
resultado.

### Interação entre os dois erros

| Cenário | Erro 1 (incomplete) | Erro 2 (missing_overload) |
|---|---|---|
| `Internal implements NUM` (completo) | não | não |
| `Internal implements NUM` (incompleto, sem uso) | sim (deny) | não |
| `Internal implements NUM #!allow type.incomplete_interface` (sem uso) | silenciado | não |
| `Internal implements NUM #!warn type.incomplete_interface` (sem uso) | warning | não |
| `Internal implements NUM #!allow type.incomplete_interface` (com uso) | silenciado | sim |
| `Internal implements NUM` (incompleto, com uso) | sim (deny) | sim (ou só 1?) |

Última linha: se o usuário não suprimiu, o erro 1 já diz que a impl
está incompleta. O erro 2 é redundante neste caso — o usuário precisa
resolver o erro 1 primeiro. **Decisão:** se o erro 1 não é suprimido,
emitir só o erro 1. Se o erro 1 é suprimido, emitir o erro 2 no uso.

### Sistema de controle de diagnóstico (`#!`)

Sintaxe: `#!<level> <diagnostic_code>` como pseudo-comentário em
qualquer declaração. `<level>` é `allow`, `warn`, ou `deny`.

```kata
# Silencia implementação incompleta
Internal implements NUM #!allow type.incomplete_interface
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

# Braço de match redundante como warning (futuro)
match x
    Some v: ...
    Some v: ...  #!warn type.redundant_clause  # segundo Some é intencional
    None: ...
```

**Por que pseudo-comentário, não diretiva (`@`)?**

Diretivas (`@`) são semântica de linguagem — afetam o tipo, o
dispatch, o código gerado. `@allow` sugere que a supressão é parte
do programa, com semântica formal. Mas controle de diagnóstico é
metainformação para o compilador, não semântica de linguagem. O
programa não muda de comportamento com ou sem `#!allow` — só os
diagnósticos mudam.

Pseudo-comentário (`#!`) comunica isso: é um comentário que o
compilador lê, mas não é código. Sintaticamente é um comentário,
semanticamente é um pragmatismo do compilador. O `!` distingue de
comentário comum (`#`).

**Códigos de diagnóstico:** usam o mesmo namespace dos 38
diagnósticos existentes (`type.unbound_name`, `parse.unexpected_token`,
etc). O usuário vê o código na mensagem de erro e copia para o
pragma. Sem sistema paralelo de aliases.

**Três níveis:**

| Nível | Comportamento |
|---|---|
| `#!allow` | silencia — não emite nada |
| `#!warn` | emite warning, compilação continua |
| `#!deny` | eleva a erro, compilação falha |

Cada diagnóstico tem um **default** definido no atributo
`severity` do enum de diagnóstico. Hoje:
- `type.incomplete_interface` → `severity = "deny"`, `controllable = true`
- `type.missing_overload` → `severity = "deny"`, `controllable = false`
- `type.redundant_clause` (futuro) → `severity = "deny"`, `controllable = true`

O usuário só escreve o pragma para **sobrescrever** o default.
`#!deny` existe para explicitar o default em código que precisa
ser claro (ex: biblioteca pública).

**Sem `forbid`:** Kata não tem aninhamento hierárquico de módulos
como Rust (`mod foo { mod bar { ... } }`). Imports são entre
arquivos, não aninhamento. Sem árvore de escopo, não há
"downstream" para proteger contra override. `allow`/`warn`/`deny`
cobre tudo.

**Escopo:** `#!` aplica-se à declaração inteira (todo o bloco
`implements` ou o braço do match). Não há controle seletivo por
método.

**Diagnósticos controláveis:**
- `type.incomplete_interface` — implementação incompleta de interface
- `type.redundant_clause` — braço de match redundante (futuro)

**Diagnósticos não controláveis:**
- `type.missing_overload` — uso de sobrecarga inexistente (erro de uso)
- Erros de sintaxe, tipo, etc.

### Onde implementar

**Erro 1 (incomplete):** em `kata-resolution`, após
`extend_families_for_implementors` ou no processamento de `implements`
em `pass0.rs`. Percorrer métodos da interface, checar overloads no
DispatchTable. Emitir `type.incomplete_interface` se faltar algum.

**Erro 2 (missing_overload):** substituir o `family_extension_invalid`
atual. Onde o predicado falha ao sintetizar (hoje eager, futuramente
lazy), emitir `type.missing_overload` com a sobrecarga específica.

**Lazy:** em `extend_families_for_implementors`, marcar
`RefinedDeclInfo` com `lazy = true` quando `extension_impl` é `Some`.
No inference, pular a síntese eager de construtores lazy. Sintetizar
on-demand quando um call site é encontrado.

**`#!`:** no parser, aceitar `#!<level> <diagnostic_code>` como
pseudo-comentário em declarações. `<level>` é `allow`, `warn`, ou
`deny`. No resolver, armazenar controles no `InterfaceRegistry`
(ou `ImplEntry`). No inference/codegen, consultar controles antes
de emitir diagnósticos controláveis.

## Testes

- **T1 (incompleto sem uso)**: `Internal implements NUM` sem `zero`,
  sem usar `NonZero::Internal` → erro `type.incomplete_interface`
  listando os 7 métodos faltando com signatures.
- **T2 (incompleto com allow, sem uso)**:
  `#!allow type.incomplete_interface` → compila sem erro.
- **T3 (incompleto com warn, sem uso)**:
  `#!warn type.incomplete_interface` → warning, compila.
- **T4 (incompleto com allow, com uso)**:
  `#!allow type.incomplete_interface` + `NonZero (Internal 3)` →
  erro `type.missing_overload` apontando `zero`.
- **T5 (completo)**: `Internal` define todos os métodos de NUM →
  compila, `NonZero::Internal` funciona.
- **T6 (completo, uso)**: `NonZero (Internal 3)` → `Ok` (não-zero).
- **T7 (mensagem clara — incomplete)**: erro `type.incomplete_interface`
  lista cada método faltando com sua signature completa
  (`zero :: Internal => Internal`, não só `zero`).
- **T8 (mensagem clara — missing_overload)**: erro
  `type.missing_overload` diz qual sobrecarga falta, qual predicado
  a exige, e onde (`zero` não definido no implements, usado pelo
  predicado `!= _ (zero _)` de NonZero).
- **T9 (allow não afeta missing_overload)**:
  `#!allow type.incomplete_interface` não controla
  `type.missing_overload`.
- **T10 (sem família)**: `Greeting implements SHOW` incompleto, sem
  família sobre SHOW → erro `type.incomplete_interface` (não
  `type.family_extension_invalid`).
- **T11 (sem síntese sem uso)**: `Internal implements NUM #!allow
  type.incomplete_interface` sem uso → não sintetiza construtor
  (eager ou lazy, conforme fase de investigação).
- **T12 (deny explícito)**: `#!deny type.incomplete_interface` →
  erro, mesmo comportamento que o default.
- **T13 (código na mensagem)**: a instrução de controle na ajuda
  só aparece em diagnósticos controláveis. `type.incomplete_interface`
  oferece `#!allow`; `type.missing_overload` não oferece.

## Fora de escopo

- Reorganização de tipos (`TypeDef`, traits Rep/Compat/Refine).
- Verificação retroativa de famílias em código existente.
- Controle seletivo por método (apenas por bloco `implements`).
- `#!` em expressões (apenas em declarações).
- `forbid` — sem aninhamento hierárquico em Kata, não há downstream.