# PRD — Conversões Numéricas, Módulo Math, e Separação NUM/ORD

**Status:** ✅ Concluído
**Data:** 2026-08-20
**Depende de:** FFI framework ✅, InterfaceRegistry ✅, DispatchTable ✅, `constant` ✅
**Handoff:** `/tmp/kata5-math-handoff.md`

## 1. Objetivo

Três frentes relacionadas que consolidam o sistema numérico de Kata:

1. **Conversões entre tipos numéricos** — sobrecargas de `int`, `float`, `rational`, `complex` que recebem outros tipos numéricos e os convertem. Unificação de nomes (`to_float`/`from_int`/`from_float` removidos em favor de `float`/`rational`).
2. **Módulo `math.kata`** — constantes (pi, euler, phi), trigonométricas, hiperbólicas, raiz/log, floor/ceil, aritmética de Int (gcd, lcm, pow, signum), e `min`/`max` genéricos no `core.kata`.
3. **Separação `NUM` de `ORD`** — `NUM implements EQ` (não mais `ORD`). Complexos não têm ordem total; a herança `NUM extends ORD` é matematicamente incorreta. Int/Float/Rational implementam `ORD` explicitamente.

## 2. Estado atual

### 2.1. Conversões (CONCLUÍDO)

**Convenção:** funções de conversão levam o nome do tipo de destino em lowercase.

| Função | Tipos | Implementação |
|---|---|---|
| `int` | `Text => Result::(Int, Text)` | `@ffi("kata_rt_try_int")` — action, pode falhar |
| `int` | `Float => Int` | `@ffi("kata_rt_float_to_int")` — trunca em direção a zero |
| `int` | `Rational => Int` | `@ffi("kata_rt_rational_to_int")` — trunca via divisão inteira |
| `int` | `Byte => Int` | `@ffi("kata_rt_byte_to_int")` — já existia |
| `int` | `Complex => Int` | Kata puro em `complex.kata`: `lambda z: int z.re` |
| `float` | `Int => Float` | `@ffi("kata_rt_int_to_float")` |
| `float` | `Rational => Float` | `@ffi("kata_rt_rat_to_float")` |
| `float` | `Byte => Float` | Kata puro: `lambda b: float (int b)` |
| `rational` | `Int => Rational` | `@ffi("kata_rt_int_to_rational")` |
| `rational` | `Float => Rational` | `@ffi("kata_rt_rat_from_float")` |
| `complex` | `Int => Complex` | Kata puro em `complex.kata`: `lambda n: Complex (float n) 0.0` |
| `complex` | `Float => Complex` | Kata puro em `complex.kata`: `lambda f: Complex f 0.0` |

**Nomes removidos:** `to_float`, `from_int`, `from_float`. Todos os call sites em `core.kata`, `complex.kata`, testes, e documentação foram atualizados.

**Runtime (Rust):**
- `kata_rt_float_to_int` em `crates/kata-rt/src/float.rs` — `f64 → i64 tagged`, trunca, NaN/Inf → 0, suporta BigInt
- `kata_rt_rational_to_int` em `crates/kata-rt/src/rational.rs` — `*const BigRational → i64 tagged`, trunca via `numer/denom`
- `alloc_bigint` tornado `pub(crate)` em `crates/kata-rt/src/bigint.rs`

**Compilador:**
- `FfiSymbol::FloatToInt` e `FfiSymbol::RatToInt` em `crates/kata-core/src/ffi.rs`
- Registrados em `crates/kata-codegen/src/ffi_registry.rs` e `ffi_sigs/arithmetic.rs`

**Verificação:** `cargo build` ✅, `cargo test` ✅, testes E2E ✅.

### 2.2. Separação NUM/ORD (CONCLUÍDO)

**Problema:** A hierarquia atual é `NUM implements ORD implements EQ`. `Complex implements NUM` — mas complexos não têm ordem total. Módulo (`|z|`) é um preorder, não ordem: `|1+0i| = |0+1i| = 1` mas `1+0i ≠ 0+1i`, violando o contrato de `<=` (que deve ser derivado de `<` e `=`).

**Solução:** Separar `NUM` de `ORD`:
- `NUM implements EQ` — números têm igualdade, não necessariamente ordem
- `ORD implements EQ` — ordem total, separada de NUM
- Int, Float, Rational implementam `NUM` + `ORD` + `EQ` explicitamente
- Complex implementa `NUM` + `EQ` (não implementa `ORD`)

**Mudanças necessárias:**

`stdlib/core.kata`:
```kata
# Antes:
interface NUM implements ORD EQ
    + :: NUM NUM => NUM
    ...

# Depois:
interface NUM implements EQ
    + :: NUM NUM => NUM
    - :: NUM NUM => NUM
    * :: NUM NUM => NUM
    div :: NUM NUM => Result::NUM
    abs :: NUM => NUM

interface ORD implements EQ
    < :: Self Self => Boolean
    > :: Self Self => Boolean
    <= :: Self Self => Boolean
    >= :: Self Self => Boolean
```

Int, Float, Rational já implementam `NUM` (com `+`, `-`, `*`, etc.) e já têm `<`, `>`, `<=`, `>=` definidos. A mudança é:
1. Mudar `interface NUM implements ORD EQ` → `interface NUM implements EQ`
2. Adicionar `Int implements ORD`, `Float implements ORD`, `Rational implements ORD` com os comparadores que já existem (mover de `implements NUM` para `implements ORD`)
3. `Complex implements NUM` não precisa mais de comparadores — mas precisa de `= :: Complex Complex => Boolean` e `!= :: Complex Complex => Boolean` (EQ)

**Atenção:** `NonZero refines NUM` (linha 169 do core.kata) — verificar se `refines` ainda funciona após a mudança. NonZero delega para Int, que implementa tanto NUM quanto ORD. O `refines NUM` não deve ser afetado.

**Verificação:** `cargo build` + `cargo test` + `cargo insta test --accept` (snapshots TAST vão mudar).

### 2.3. Módulo math.kata (CONCLUÍDO)

**Escopo aprovado pelo usuário:**

**Constantes:**
- `pi` — 3.14159265358979323846264338327950288
- `euler` — 2.71828182845904523536028747135266249
- `phi` — 1.61803398874989484820458683436563811
- **Não** incluir `tau`.

**Trigonométricas (Float → Float):**
- `sin`, `cos`, `tan`, `asin`, `acos`, `atan`
- `atan2 :: Float Float => Float`

**Hiperbólicas (Float → Float):**
- `sinh`, `cosh`, `tanh`

**Raiz e log (Float → Float):**
- `sqrt`, `cbrt`, `log` (natural), `log2`, `log10`, `exp`

**Floor e ceil (Float → Int):**
- `floor` — arredonda em direção a -∞
- `ceil` — arredonda em direção a +∞

**Aritmética Int (Int → Int):**
- `gcd :: Int Int => Int` — máximo divisor comum (Euclidiana)
- `lcm :: Int Int => Int` — mínimo múltiplo comum
- `pow :: Int Int => Int` — exponenciação (BigInt)
- `signum :: Int => Int` — -1, 0, ou 1
- `abs` — **já existe** na interface NUM em `core.kata`, não duplicar

**Min/max (em core.kata, não math.kata):**
- `min :: A A => A` — genérico via ORD
- `max :: A A => A` — genérico via ORD
- Restrição: `A` implementa `ORD`. Verificar se o sistema suporta funções standalone genéricas com constraint de interface. Se não, overloads por tipo concreto.

**FFI symbols novos (~24):**

| FfiSymbol | Símbolo C | Assinatura ABI |
|---|---|---|
| `Sin` | `kata_rt_sin` | `f64 → f64` |
| `Cos` | `kata_rt_cos` | `f64 → f64` |
| `Tan` | `kata_rt_tan` | `f64 → f64` |
| `Asin` | `kata_rt_asin` | `f64 → f64` |
| `Acos` | `kata_rt_acos` | `f64 → f64` |
| `Atan` | `kata_rt_atan` | `f64 → f64` |
| `Atan2` | `kata_rt_atan2` | `f64, f64 → f64` |
| `Sinh` | `kata_rt_sinh` | `f64 → f64` |
| `Cosh` | `kata_rt_cosh` | `f64 → f64` |
| `Tanh` | `kata_rt_tanh` | `f64 → f64` |
| `Sqrt` | `kata_rt_sqrt` | `f64 → f64` |
| `Cbrt` | `kata_rt_cbrt` | `f64 → f64` |
| `Log` | `kata_rt_log` | `f64 → f64` |
| `Log2` | `kata_rt_log2` | `f64 → f64` |
| `Log10` | `kata_rt_log10` | `f64 → f64` |
| `Exp` | `kata_rt_exp` | `f64 → f64` |
| `Floor` | `kata_rt_floor` | `f64 → i64 tagged` |
| `Ceil` | `kata_rt_ceil` | `f64 → i64 tagged` |
| `Gcd` | `kata_rt_gcd` | `i64, i64 → i64 tagged` |
| `Lcm` | `kata_rt_lcm` | `i64, i64 → i64 tagged` |
| `Pow` | `kata_rt_pow` | `i64, i64 → i64 tagged` |
| `Signum` | `kata_rt_signum` | `i64 → i64 tagged` |

## 3. Fases de implementação

### Fase 1: Separação NUM/ORD (core.kata)

**Prioridade:** alta — deve vir antes de math.kata porque `min`/`max` dependem de ORD, e a separação afeta o dispatch de Complex.

1. Mudar `interface NUM implements ORD EQ` → `interface NUM implements EQ`
2. Mover comparadores (`<`, `>`, `<=`, `>=`) de `Int implements NUM` para `Int implements ORD`
3. Mesmo para `Float implements NUM` → `Float implements ORD`
4. Mesmo para `Rational implements NUM` → `Rational implements ORD`
5. Adicionar `Complex implements EQ` em `complex.kata` com `=` e `!=` (comparar re e im)
6. Verificar `NonZero refines NUM` — ainda funciona
7. `cargo build` + `cargo test` + `cargo insta test --accept`

### Fase 2: min/max em core.kata

1. Adicionar `min :: A A => A` e `max :: A A => A` após implementações de ORD
2. Verificar se funções standalone genéricas com constraint funcionam
3. Se não funcionar, overloads por tipo concreto (Int, Float, Rational)
4. `cargo build` + `cargo insta test --accept`

### Fase 3: Runtime (Rust)

1. Criar `crates/kata-rt/src/math.rs` (ou adicionar em `float.rs`)
2. Implementar 22 wrappers C-ABI (`kata_rt_sin` ... `kata_rt_signum`)
3. `floor`/`ceil` seguem o padrão de `kata_rt_float_to_int` (f64 → tagged Int)
4. `gcd`/`lcm`/`pow`/`signum` operam em SMI-tagged Ints
5. Re-exports em `lib.rs`
6. `cargo build -p kata-rt`

### Fase 4: Compilador (FfiSymbol)

1. Adicionar 22 variants ao enum `FfiSymbol` em `crates/kata-core/src/ffi.rs`
2. `symbol_name()` + `return_type()` para cada
3. Registrar em `ffi_registry.rs` (símbolos + `all_ffi_symbols`)
4. Assinaturas ABI em `ffi_sigs/arithmetic.rs`
5. `cargo build`

### Fase 5: Stdlib (math.kata)

1. Criar `stdlib/math.kata` com `import core`
2. Constantes (`constant pi := ...`)
3. 22 funções FFI + exports
4. `cargo build`

### Fase 6: Testes E2E

1. Criar `crates/kata-driver/tests/math_e2e.rs`
2. Testar cada função com valores conhecidos
3. `cargo test`

### Fase 7: Documentação

1. `docs/Kata-lang-manual.md` — adicionar seção sobre `import math` (solicitar permissão)
2. `docs/kata-book/` — possivelmente adicionar capítulo (solicitar permissão)
3. Atualizar snapshots TAST

## 4. Decisões de design

### 4.1. Truncamento vs floor/ceil

- `int :: Float => Int` — **trunca** em direção a zero (igual a Python `int()`, C cast)
- `floor :: Float => Int` — arredonda em direção a -∞
- `ceil :: Float => Int` — arredonda em direção a +∞

São três operações distintas. `int (-3.7)` = `-3` (trunca), `floor (-3.7)` = `-4` (floor), `ceil (-3.7)` = `-3` (ceil).

### 4.2. Constantes como `constant`

`constant` é avaliado em compile-time e embutido no binário. Os literais de `pi`, `euler`, `phi` são preservados como texto — não há passagem por `f64` se usados com `::Rational`. Para uso como `Float`, o literal é convertido para `f64` em compile-time.

### 4.3. Complex e ORD

Complex **não** implementa `ORD`. Tentar `z1 < z2` onde ambos são `Complex` deve ser erro de tipo. Se o usuário quer comparar módulos, usa `float z` (que extrai `re`) ou acessa `z.re` e `z.im` diretamente.

### 4.4. min/max genéricos

Se o sistema de tipos suporta funções standalone genéricas com constraint (`A` implementa `ORD`), `min`/`max` são uma única definição cada. Se não, overloads por tipo. A preferência é genérico.

## 5. Riscos

1. **Separação NUM/ORD pode quebrar dispatch** — overloads cross-type em `core.kata` (linhas 432-492) usam `+`, `-`, `*` que são de NUM. A separação não deve afetar esses overloads (eles não usam comparadores), mas o typechecker pode exigir que os tipos envolvidos implementem a interface completa.
2. **`NonZero refines NUM`** — NonZero delega NUM para Int. Se NUM não inclui mais ORD, NonZero perde acesso a comparadores via `refines NUM`. Solução: `NonZero refines ORD` adicional, ou confiar que o dispatch faz fallback para Int (que implementa ORD separadamente).
3. **Funções standalone genéricas** — se o sistema não suporta `min :: A A => A` com constraint `ORD`, o fallback é verboso (3 overloads por tipo).
4. **`pow` com BigInt** — exponenciação de BigInt pode produzir números enormes. Sem limite de memória. Considerar limite de expoente.

## 6. Arquivos modificados

### Concluído (conversões):
- `stdlib/core.kata`
- `stdlib/complex.kata`
- `crates/kata-rt/src/float.rs`, `rational.rs`, `bigint.rs`, `lib.rs`
- `crates/kata-core/src/ffi.rs`
- `crates/kata-codegen/src/ffi_registry.rs`, `ffi_sigs/arithmetic.rs`
- `crates/kata-codegen/tests/arity_aware_e2e.rs`
- `docs/Kata-lang-manual.md`, `docs/maquinaria-interna.md`
- `docs/kata-book/02-guessing-game.md`, `03-sintaxe-basica.md`, `10-enums-structs.md`

### Concluído (todas as frentes):
- `stdlib/core.kata` — separar NUM/ORD, adicionar min/max ✅
- `stdlib/complex.kata` — adicionar `Complex implements EQ` ✅
- `stdlib/math.kata` — **NOVO** ✅ (constantes, trig, hyper, raiz/log, floor/ceil, int arith, exports)
- `crates/kata-rt/src/math.rs` — **NOVO** ✅ (22 funções FFI)
- `crates/kata-core/src/ffi.rs` — 22 FfiSymbol novos ✅
- `crates/kata-codegen/src/ffi_registry.rs`, `ffi_sigs/arithmetic.rs` — registro ✅
- `crates/kata-driver/tests/math_e2e.rs` — **NOVO** ✅ (11 testes E2E)
- `docs/Kata-lang-manual.md` — seção math (pendente — solicitar permissão)