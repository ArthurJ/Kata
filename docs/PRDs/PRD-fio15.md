# PRD — Fio 15: AOT, REPL

**Status:** Rascunho
**Data:** 2026-07-19
**Depende de:** Fio 1–14 (todas as features precisam funcionar em AOT e REPL)

## 1. Objetivo

Entregar os dois backends de execução que faltam para a Kata-Lang 1.0:

- **`kata build`** — compila um programa Kata para um **executável nativo** via
  `cranelift-object` (emissão de `.o`) + linker. O executável é self-contained:
  roda sem o compilador, sem JIT, sem runtime instalado (no modo padrão).
- **`kata repl`** — REPL interativo que mantém `TypeEnv` e bindings entre
  expressões, permitindo exploração incremental.

O backend JIT existente (`kata eval`, `kata run`) permanece inalterado e é a
base sobre a qual o REPL opera.

## 2. Contexto do codebase

### 2.1. Acoplamento JITModule

O lowering inteiro (`LowerCtx`, `lower_module`, `declare_kata_function`,
`define_kata_action`, `test_runner`, etc.) referencia `cranelift_jit::JITModule`
como tipo concreto — não o trait `Module` do `cranelift_module`. Isso impede
reusar o lowering para `ObjectModule` (AOT) sem generalização.

`LowerCtx` em `lowering/mod.rs`:
```rust
pub(crate) struct LowerCtx<'a, 'b> {
    pub module: &'a mut cranelift_jit::JITModule,
    // ...
}
```

Todos os submódulos (`action_def.rs`, `function_def.rs`, `test_runner.rs`,
`module.rs`) recebem `&mut cranelift_jit::JITModule` como parâmetro.

### 2.2. FFI symbols

No JIT, `register_ffi_symbols` injeta ponteiros das funções C do `kata-rt`
no `JITBuilder`. No AOT, os símbolos FFI são `Linkage::Import` resolvidos em
link-time contra `libkata_rt.a` (ou `.so` no modo dinâmico). O codegen não
registra ponteiros — declara imports que o linker resolve.

### 2.3. cranelift-object

`cranelift-object = "0.133"` já é dependência do workspace e do `kata-codegen`,
mas não é usado em nenhum lugar. `ObjectModule` é o backend AOT do Cranelift:
emite `.o` (ELF/Mach-O/COFF) com relocations pendentes, que o linker resolve.

### 2.4. kata-rt linking

`kata-rt` não tem `crate-type = ["staticlib"]` no `Cargo.toml` — só produz
`rlib` (para consumo interno dos crates do compilador). Para AOT, precisamos
linkar o runtime como biblioteca estatica no executável final.

### 2.5. JITModule não suporta extensão

`JITModule::finalize_definitions()` é terminal — não é possível declarar novas
funções após finalizar. O REPL não pode usar um único `JITModule` para múltiplas
expressões. Cada expressão (ou batch) precisa de um `JITModule` fresco, mas
precisa preservar o `TypeEnv` e os bindings de expressões anteriores.

### 2.6. Tree shaking

O ROADMAP menciona tree shaking de `@test` em produção (linha 699) e tree
shaking incondicional em `kata build` (linha 716). O crate `kata-tree-shaking`
não existe no workspace. Nenhum passe de dead code elimination foi implementado.

### 2.7. Estrutura do TypedModule

```rust
pub struct TypedModule {
    pub pre_entry: Vec<Spanned<TypedExpr>>,
    pub entry: Spanned<TypedExpr>,
    pub dispatch_table: DispatchTable,
    pub type_env: TypeEnv,
    pub functions: Vec<TypedFunction>,
    pub actions: Vec<TypedAction>,
}
```

O `type_env` contém todos os bindings de tipo ao final do typeck. O REPL
precisa preservar e estender este estado entre expressões.

### 2.8. Pipeline atual (driver)

```
lex → parse → resolve(prelude + user) → infer_module → monomorphize → optimize → jit_eval
```

`jit_eval` cria `JITBuilder`, registra FFI symbols, declara FFI imports,
faz `lower_module`, `finalize_definitions`, transmuta e executa.

## 3. `kata build` — AOT

### 3.1. CLI

```
kata build <arquivo.kata>                    # produz ./<nome> (sem extensão)
kata build <arquivo.kata> -o <output>        # path de saída custom
kata build <arquivo.kata> --dynamic          # link dinâmico (libkata_rt.so)
kata build <arquivo.kata> --target <triple>  # cross-compilação (post-1.0)
```

Defaults:
- Saída: nome do arquivo sem extensão, no diretório atual
- Linking: estático (`libkata_rt.a` embutida)
- Otimização: mesma do JIT (`optimize` já roda no pipeline)

### 3.2. Pipeline AOT

```
lex → parse → resolve → infer_module → monomorphize → optimize
    → tree_shake
    → aot_emit (ObjectModule → .o)
    → link (.o + libkata_rt.a → executável)
```

Diferenças vs JIT:
- **`tree_shake`** antes do codegen (remove `@test`, funções não alcançadas)
- **`aot_emit`** em vez de `jit_eval` — usa `ObjectModule` em vez de `JITModule`
- **`link`** — invoca linker do sistema (`cc`) para linkar `.o` com runtime

### 3.3. Generalização do lowering — `ModuleBackend` trait

Para evitar duplicar o lowering inteiro, introduzimos um trait que abstrai
as operações que `JITModule` e `ObjectModule` compartilham:

```rust
/// Backend de codegen — abstrai JITModule vs ObjectModule.
///
/// As operações são as que o lowering já faz via `Module` trait do
/// cranelift-module, mas com a diferença de FFI:
/// - JIT: registra ponteiros no builder, declara imports (resolvidos em runtime)
/// - AOT: declara imports (resolvidos em link-time pelo linker)
pub(crate) trait ModuleBackend {
    /// Declara uma função no module.
    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        sig: &Signature,
    ) -> Result<FuncId, CodegenError>;

    /// Define uma função no module.
    fn define_function(
        &mut self,
        func_id: FuncId,
        ctx: &mut cranelift_codegen::Context,
    ) -> Result<(), CodegenError>;

    /// Declara um data symbol (string literal).
    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> Result<DataId, CodegenError>;

    /// Define um data symbol.
    fn define_data(
        &mut self,
        data_id: DataId,
        data_desc: &cranelift_module::DataDescription,
    ) -> Result<(), CodegenError>;

    /// Declara FFI no function sendo compilado (FuncRef).
    fn declare_func_in_func(
        &mut self,
        func_id: FuncId,
        func: &mut cranelift_codegen::Function,
    ) -> cranelift_codegen::ir::FuncRef;

    /// Declara data no function sendo compilado (GlobalValue).
    fn declare_data_in_func(
        &mut self,
        data_id: DataId,
        func: &mut cranelift_codegen::Function,
    ) -> cranelift_codegen::ir::GlobalValue;

    /// Cria um Context do Cranelift.
    fn make_context(&mut self) -> cranelift_codegen::Context;

    /// Limpa um Context.
    fn clear_context(&mut self, ctx: &mut cranelift_codegen::Context);

    /// Finaliza todas as definições (resolve relocations, compila).
    /// No JIT, compila machine code em memória.
    /// No AOT, escreve o object file.
    fn finalize_definitions(&mut self) -> Result<(), CodegenError>;

    /// Retorna o nome do backend (para mensagens de erro).
    fn backend_name(&self) -> &'static str;
}
```

`LowerCtx` passa a usar `&dyn ModuleBackend`:

```rust
pub(crate) struct LowerCtx<'a, 'b> {
    pub backend: &'a mut dyn ModuleBackend,
    // ... resto inalterado
}
```

**Implementações:**
- `JitBackend` — wrap `JITModule`, implementa `ModuleBackend`. O registro
  de FFI symbols já acontece no `JITBuilder` antes de criar o module.
- `AotBackend` — wrap `ObjectModule`, implementa `ModuleBackend`. Não
  registra ponteiros (FFI são `Linkage::Import` resolvidos pelo linker).

**Pré-registro de FFI:**
- JIT: `register_ffi_symbols(&mut builder)` antes de `JITModule::new(builder)`
- AOT: FFI symbols são apenas declarados como `Linkage::Import` (já é o caso
  em `declare_ffi_symbols`). O linker resolve contra `libkata_rt.a`.

### 3.4. `aot_emit` — emissão de object file

```rust
pub fn aot_emit(typed: &TypedModule) -> Result<Vec<u8>, CodegenError> {
    // Configura flags (preserve_frame_pointers = true, mesmo do JIT)
    let flags_builder = ...;
    let isa = ...; // target isa (host native por ora)

    // ObjectModule em vez de JITModule
    let builder = cranelift_object::ObjectBuilder::new(isa, ...);
    let mut module = cranelift_object::ObjectModule::new(builder);

    // FFI: declare_ffi_symbols (Linkage::Import — sem registro de ponteiros)
    let ffi_ids = declare_ffi_symbols(&mut module)?;

    // lower_module (agora via &dyn ModuleBackend)
    let (_metadata, _string_table, _test_wrappers) =
        lower_module(typed, &mut module as &mut dyn ModuleBackend, &ffi_ids)?;

    // finalize_definitions escreve o object file
    module.finalize_definitions()?;
    // Ou: module.emit() retorna o ObjectBytes
    let object_bytes = module.emit()?;
    Ok(object_bytes.to_vec())
}
```

### 3.5. `link` — invocação do linker do sistema

O driver invoca `cc` (ou `clang`, `gcc` — o primeiro disponível) para linkar:

```
cc -o <output> <object_file> -L<runtime_lib_path> -lkata_rt -lm -lpthread
```

**Estático (padrão):**
- `libkata_rt.a` produzida por `cargo build -p kata-rt` (precisa
  `crate-type = ["staticlib", "rlib"]` no Cargo.toml de kata-rt)
- O driver descobre o path: `<workspace_root>/target/<profile>/libkata_rt.a`
- Link estático: o executável embute o runtime

**Dinâmico (`--dynamic`):**
- `libkata_rt.so` produzida por `cargo build -p kata-rt` (adiciona `cdylib`
  ao `crate-type`)
- O driver linka com `-lkata_rt` dinâmico
- O executável precisa de `libkata_rt.so` no `LD_LIBRARY_PATH` ou instalada

**Descoberta do workspace root:**
- O driver busca `Cargo.toml` do workspace a partir do diretório do binário
  `kata` (env!("CARGO_MANIFEST_DIR")) ou de uma variável de ambiente
  `KATA_BUILD_ROOT` configurada em build.rs.

### 3.6. Tree shaking

**Escopo mínimo para 1.0:** remover `@test` wrappers e funções/actions não
alcançadas a partir do entry point.

**Algoritmo:**
1. Construir call graph a partir do `entry` e `pre_entry` do `TypedModule`
2. Marcar como reached: entry, pre_entry, e todas as funções/actions chamadas
   (transitivamente)
3. Remover do `TypedModule`: funções/actions não reached, `TypedTestSpec` de
   todas as actions (wrappers `@test` não são gerados em AOT)
4. O `TypedModule` resultante é menor — o codegen só lowera o que sobra

**Crate:** `kata-tree-shaking` (novo crate no workspace)

**Não inclui:**
- Dead code dentro de funções (só removal de funções inteiras)
- Análise de escape para remoção de captures não usadas
- Inlining ou otimizações além do que `optimize` já faz

**Nota:** o `optimize` (TRMA + passes existentes) já roda antes do tree shaking.
Tree shaking remove funções; optimize transforma o que resta. A ordem
(optimize → tree_shake) evita otimizar código que será removido.

### 3.7. `__kata_entry` como main

No JIT, `__kata_entry` é chamado pelo driver via transmute. No AOT,
`__kata_entry` precisa ser chamado pelo `main` do C runtime.

**Abordagem:** o driver gera um `main.rs` shim que chama `__kata_entry`:

```rust
// kata_build_main.rs (gerado em /tmp ou no output dir)
extern "C" {
    fn __kata_entry() -> i64;
}
fn main() {
    let result = unsafe { __kata_entry() };
    // print_result equivalente ao driver
    std::process::exit(result as i32);
}
```

O shim é compilado por `cc` (ou `rustc`) para `.o` e linkado junto com o
`.o` do Cranelift e `libkata_rt.a`.

**Alternativa considerada (descartada):** fazer `__kata_entry` ser o `main`
direto. Problema: `__kata_entry` usa `CallConv::SystemV` e não segue a ABI
de `main` do C (que depende da plataforma). O shim isola essa diferença.

**Nota:** o shim precisa replicar `print_result` do driver para formatar
a saída (SMI untag, BigInt show, Float bits→f64, Text CStr, etc). O código
de display vive em `kata-driver` — pode ser extraído para um módulo
compartilhado ou duplicado no shim.

### 3.8. `kata-rt` como staticlib

**Mudança no Cargo.toml de kata-rt:**

```toml
[lib]
crate-type = ["staticlib", "rlib", "cdylib"]
```

- `rlib` — para consumo interno dos crates do compilador (JIT)
- `staticlib` — para AOT estático (`libkata_rt.a`)
- `cdylib` — para AOT dinâmico (`libkata_rt.so`)

O `cargo build -p kata-rt` produz todos os três. O driver escolhe qual
linkar baseado em `--dynamic`.

### 3.9. Entry point display — extração de `print_result`

`print_result` em `kata-driver/src/main.rs` faz SMI untag, BigInt show,
Float bits→f64, Text CStr, Boolean, Unit, e fallback. Este código precisa
estar disponível no shim de AOT.

**Opção:** extrair `print_result` para um módulo `kata-driver/src/display.rs`
e incluí-lo no shim gerado. O shim é código Rust que referencia `kata_rt`
para as funções de display (BigInt show, etc).

## 4. `kata repl` — REPL interativo

### 4.1. CLI

```
kata repl                    # inicia REPL interativo
```

Comandos dentro do REPL:

```
:help          — mostra comandos disponíveis
:type <expr>   — mostra o tipo de <expr> sem executar
:env           — mostra bindings e tipos no TypeEnv atual
:quit          — sai do REPL
:reset         — limpa bindings, recarrega prelude
:load <file>   — carrega arquivo .kata (let bindings e defs entram no env)
```

### 4.2. Arquitetura — TypeEnv persistente + JIT fresco

```
┌─────────────────────────────────────────────────┐
│ REPL Session                                     │
│                                                  │
│  TypeEnv (persistente entre expressões)          │
│  ├── bindings: x: Int, y: Float, fat: ...       │
│  ├── dispatch_table (prelude + user sigs)        │
│  ├── enum_registry, struct_registry, ...        │
│                                                  │
│  History (rustyline)                             │
│                                                  │
│  Loop:                                           │
│    1. Ler input (rustyline)                      │
│    2. Se comando (:), executar comando           │
│    3. Se expressão:                               │
│       a. lex → parse                             │
│       b. merge com TypeEnv atual                 │
│       c. infer_module (typeck)                   │
│       d. monomorphize → optimize                │
│       e. JITModule fresco → lower_module → exec  │
│       f. Resultado → display                     │
│       g. Se let binding: atualizar TypeEnv       │
└─────────────────────────────────────────────────┘
```

**JITModule fresco por expressão:** não há como adicionar funções a um
`JITModule` após `finalize_definitions`. Cada expressão (ou batch de
expressões) usa um `JITModule` novo, mas o `TypeEnv` persiste.

**Custo aceitável:** o compile-time de uma expressão via Cranelift JIT é
milissegundos — não há percepção de latência para uso interativo.

### 4.3. TypeEnv estendido

`TypeEnv` é uma árvore de escopos. O REPL mantém um `TypeEnv` raiz que
carrega o prelude. Cada `let` ou `lambda` nomeado adiciona um binding.

**Desafio:** `infer_module` recebe um `&Module` (AST) e `&ResolvedModule`,
produz um `TypedModule`. O REPL precisa:
1. Parsear a expressão como um `Module` (com 1 item)
2. Resolver contra o `TypeEnv` acumulado
3. Inferir
4. Executar
5. Se a expressão é um `let` (ou `lambda` nomeado, `action`, `data`, etc),
   adicionar o binding ao `TypeEnv` para a próxima iteração

**`let` bindings como pre_entry:** no modelo atual, `let` top-level vai para
`pre_entry` do `TypedModule`. No REPL, cada `let` adiciona ao `pre_entry`
acumulado da sessão. A próxima expressão é compilada com todos os
`pre_entry` anteriores + a nova expressão como entry.

**Recompilação incremental:** naive — recompila tudo a cada expressão.
O `JITModule` fresco compila todas as funções (prelude + user + nova).
Optimização (cache de compilação do prelude) é post-1.0.

### 4.4. `:type <expr>`

Executa o pipeline até `infer_module` e imprime o tipo do entry point
sem fazer codegen. Reusa `infer_module` e lê `typed.entry.node.ty`.

### 4.5. `:load <file>`

Carrega um arquivo `.kata` e processa cada item como se fosse digitado
no REPL. `let` bindings, `data`, `enum`, `lambda` nomeados entram no
`TypeEnv`. Actions e funções FFI são registradas no `DispatchTable`.

### 4.6. `:reset`

Recarrega o prelude, limpa bindings do usuário. Equivalente a sair e
entrar de novo no REPL.

### 4.7. rustyline

Dependência nova no `kata-driver`:

```toml
[dependencies]
rustyline = "14"
```

Features: history persistente (`~/.kata_repl_history`), completion
(básica de comandos `:`), multiline (para lambdas com múltiplas cláusulas).

### 4.8. Display de resultados

Reusa `print_result` do driver (mesma lógica de SMI untag, Float, Text, etc).
Se extraído para módulo compartilhado (§3.9), ambos usam o mesmo código.

### 4.9. Erros no REPL

Erros de typeck ou runtime não abortam a sessão. O erro é impresso e o
usuário pode corrigir e reintentar. O `TypeEnv` não é modificado em caso
de erro (rollback para o estado anterior à expressão que falhou).

## 5. Decisões de design

### D1: Trait object (`&dyn ModuleBackend`) em vez de genéricos

Justificativa: o lowering tem ~15 funções que recebem `&mut JITModule`.
Generalizar com `M: Module` propagaria parâmetros de tipo por toda a
codebase, inflando assinaturas. Trait object isola a mudança no `LowerCtx`
e nas funções de declaração/definição. O custo de dynamic dispatch é
irrelevante (chamadas de codegen, não hot path).

### D2: Staticlib por padrão, `--dynamic` opcional

Justificativa: self-containment é mais valioso para adoção que
upgradeabilidade do runtime. O binário padrão roda em qualquer máquina
sem dependências. `--dynamic` fica disponível para quem quer binários
menores e aceita a dependência de `libkata_rt.so`.

O custo é duplo crate-type no Cargo.toml de kata-rt e lógica adicional
no driver para escolher a lib correta no link.

### D3: Tree shaking minimal — funções inteiras, não DCE intra-função

Justificativa: tree shaking de funções inteiras (remover `@test`,
remover funções não alcançadas) é suficiente para `kata build`. DCE
dentro de funções é trabalho do `optimize` (que já existe). Não criar
passo de DCE que duplica o que Cranelift pode fazer na lowering.

### D4: Shim `main.rs` gerado pelo driver

Justificativa: `__kata_entry` usa `CallConv::SystemV` e não segue a ABI
de `main` do C. O shim isola essa diferença e permite que o display de
resultados seja consistente entre JIT e AOT.

Alternativa descartada: fazer `__kata_entry` ser o `main` diretamente.
Problema: ABI incompatível, e o display de resultados (SMI untag, etc)
precisaria ser embutido no entry point, misturando responsabilidades.

### D5: REPL recompila tudo a cada expressão (naive)

Justificativa: o custo de recompilar prelude + bindings + nova expressão
via Cranelift JIT é milissegundos. Otimização (cache de compilação do
prelude, reuso de `JITModule`) é complexa e post-1.0. O naive é simples
e funcional.

### D6: `:load <file>` processa item por item

Justificativa: processar item por item permite que erros em um item não
invalidem todo o arquivo. Items válidos anteriores permanecem no `TypeEnv`.
Mesmo modelo do REPL — `:load` é um batch de inputs.

## 6. Fases de implementação

### Fase 1: `ModuleBackend` trait + `JitBackend`

- Criar trait `ModuleBackend` em `kata-codegen/src/lowering/backend.rs`
- Implementar `JitBackend` wrap `JITModule`
- Migrar `LowerCtx` para `&dyn ModuleBackend`
- Migrar `lower_module`, `declare_kata_function`, `define_kata_function`,
  `declare_kata_action`, `define_kata_action`, `generate_test_wrappers`,
  `declare_ffi_symbols` para usar `&dyn ModuleBackend` (ou funções do trait)
- `jit_eval` e `jit_compile_tests` usam `JitBackend`
- `cargo test --workspace` não regrediu

**DoD Fase 1:** todos os testes existentes passam com `JitBackend`. Nenhuma
mudança comportamental. O trait está pronto para `AotBackend`.

### Fase 2: `AotBackend` + `aot_emit`

- Implementar `AotBackend` wrap `cranelift_object::ObjectModule`
- `aot_emit(typed) -> Result<Vec<u8>, CodegenError>` — emite `.o` bytes
- FFI: `declare_ffi_symbols` funciona sem registro de ponteiros (Linkage::Import)
- Testes unitários: `aot_emit` em um módulo simples (`+ 1 2`) produz `.o`
  válido (verificar magic bytes ELF/Mach-O/COFF)

**DoD Fase 2:** `aot_emit` produz object file válido para um módulo simples.
`cargo test -p kata-codegen` cobre o novo backend.

### Fase 3: Tree shaking

- Novo crate `kata-tree-shaking` no workspace
- `tree_shake(typed: TypedModule) -> TypedModule` — remove funções/actions
  não alcançadas e `TypedTestSpec`
- Algoritmo: worklist a partir de entry + pre_entry, marcar reached,
  remover não reached
- Testes: módulo com `@test` → tree shaking remove test specs. Módulo com
  função não chamada → tree shaking remove. Módulo com função chamada
  transitivamente → tree shaking mantém.

**DoD Fase 3:** `tree_shake` remove `@test` e código morto. Testes unitários
cobrem os 3 casos. `cargo test -p kata-tree-shaking` passa.

### Fase 4: `kata-rt` staticlib + linker

- Adicionar `crate-type = ["staticlib", "rlib", "cdylib"]` no Cargo.toml
  de kata-rt
- `cargo build -p kata-rt` produz `libkata_rt.a` e `libkata_rt.so`
- Driver: função `link(object_bytes, output_path, dynamic: bool) -> Result<(), ...>`
  que invoca `cc` com flags apropriadas
- Descoberta do path `libkata_rt.a`/`.so` via `CARGO_MANIFEST_DIR` ou
  variável de ambiente configurada em build.rs

**DoD Fase 4:** `link` produz executável que roda sem o compilador. O
executável imprime o mesmo resultado que `kata run` para o mesmo input.

### Fase 5: Shim `main.rs` + `kata build`

- Gerar shim `main.rs` que chama `__kata_entry` e faz display
- Compilar shim via `rustc` ou incluir `print_result` extraído
- Subcomando `Command::Build { file, output, dynamic }` no driver
- Pipeline: lex → parse → resolve → infer → monomorph → optimize →
  tree_shake → aot_emit → link
- Testes E2E: `kata build examples/fatorial.kata` produz `./fatorial`
  que executa e imprime `120`

**DoD Fase 5:** `kata build examples/fatorial.kata -o /tmp/fat` produz
executável que imprime `120`. `kata build examples/hello_action.kata -o /tmp/hello`
produz executável que imprime `hello\nworld`.

### Fase 6: REPL — TypeEnv persistente + `:type`

- Subcomando `Command::Repl` no driver
- `rustyline` como linha de leitura
- `ReplSession` struct: `TypeEnv`, `DispatchTable`, `ResolvedModule` (prelude)
- Loop: ler → parse → infer → monomorph → optimize → jit_eval → display
- `let` bindings atualizam `TypeEnv` da sessão
- `:type <expr>` executa até infer e mostra tipo
- `:env` mostra bindings
- `:quit` sai
- `:reset` recarrega prelude

**DoD Fase 6:** `kata repl` inicia, `+ 1 2` imprime `3`, `let x := 10`
silenciosamente binds, `+ x 5` imprime `15`, `:type + 1 2` imprime `Int`,
`:env` mostra bindings, `:quit` sai.

### Fase 7: `:load` + multiline + testes E2E

- `:load <file>` processa arquivo item por item
- Multiline: rustyline configurado para continuar linha quando o input
  está incompleto (lambda com cláusulas, action com body)
- Testes E2E do REPL via subprocess: pipe de comandos para stdin, verifica
  stdout

**DoD Fase 7:** `:load examples/fatorial.kata` carrega a função `fat` no
REPL. `fat 5 1` imprime `120`. Multiline para `lambda n: ...` funciona.
Testes E2E cobrem os 5 cenários básicos.

### Fase 8: Documentação

- Atualizar `docs/ROADMAP.md` Fio 15 com status ✅
- Atualizar `docs/Kata-lang-manual.md` com seções `kata build` e `kata repl`
- Atualizar `docs/sintaxe-mapa.md` se houver mudanças de sintaxe (não deve haver)

**DoD Fase 8:** Documentação reflete a implementação.

## 7. DoD (Definition of Done)

1. `kata build examples/fatorial.kata -o /tmp/fat` produz executável nativo
   que imprime `120` ao executar `/tmp/fat`.
2. `kata build examples/hello_action.kata -o /tmp/hello` produz executável
   que imprime `hello\nworld`.
3. `kata build --dynamic` produz executável que linka com `libkata_rt.so`.
4. Tree shaking remove `@test` e funções não alcançadas do binário AOT.
5. `kata repl` inicia sessão interativa com rustyline.
6. `+ 1 2` no REPL imprime `3`.
7. `let x := 10` no REPL permite `+ x 5` → `15` na próxima linha.
8. `:type + 1 2` imprime `Int` sem executar.
9. `:env` mostra bindings e tipos da sessão.
10. `:load examples/fatorial.kata` carrega `fat` no REPL. `fat 5 1` → `120`.
11. `:reset` limpa bindings, recarrega prelude.
12. `:quit` sai do REPL.
13. Multiline funciona para lambdas e actions.
14. Erros de typeck/runtime não abortam a sessão REPL.
15. `cargo test --workspace` passa sem regressão.
16. Testes E2E cobrem `kata build` (3 casos) e `kata repl` (5 cenários).

## 8. Fora do escopo

- Cross-compilação (`--target <triple>`) — post-1.0
- Otimização de REPL (cache de compilação do prelude, reuso de JITModule)
- DCE intra-função (tree shaking só remove funções inteiras)
- Inlining ou otimizações além do que `optimize` já faz
- REPL com syntax highlighting (rustyline básico)
- REPL com autocompletar de nomes de binding (post-1.0)
- Debug info no executável AOT (DWARF/symbols para debugger) — post-1.0
- `kata build` para múltiplos arquivos/módulos (single-file AOT por ora)
- Tree shaking de imports não usados (só funções/actions)

## 9. Riscos

### R1: Generalização do lowering pode quebrar testes existentes

O lowering inteiro referencia `JITModule` como tipo concreto. Migrar para
`&dyn ModuleBackend` toca ~15 arquivos. Testes de codegen existentes podem
quebrar se o dynamic dispatch introduzir diferenças sutis (não deveria —
o trait é uma fachada sobre as mesmas operações).

**Mitigação:** Fase 1 é isolada — só introduz o trait e `JitBackend`, sem
mudar comportamento. `cargo test --workspace` deve passar sem regressão
antes de qualquer mudança AOT.

### R2: `ObjectModule` pode ter API diferente de `JITModule`

`cranelift-object` é um backend distinto. Embora ambos implementem `Module`
do `cranelift-module`, pode haver diferenças em métodos específicos ou
constraints (ex: `ObjectModule` pode não suportar certos `Linkage` ou
pode exigir finalização diferente).

**Mitigação:** Fase 2 começa com um módulo trivial (`+ 1 2`) para validar
que `ObjectModule` funciona com o lowering existente. Se houver fricção,
o trait `ModuleBackend` absorve a diferença.

### R3: Linker do sistema pode não estar disponível

`cc` é presumido, mas pode não estar instalado em todos os ambientes. O
driver precisa detectar e reportar claramente: "linker não encontrado:
instale cc ou gcc".

**Mitigação:** Detectar `cc`, `gcc`, `clang` em ordem. Se nenhum, erro
claro com instrução de instalação.

### R4: Display de resultados no shim AOT

O shim `main.rs` precisa replicar `print_result` do driver, que faz SMI
untag, BigInt show, Float bits→f64, etc. Se o display divergir entre
JIT e AOT, os resultados parecerão diferentes.

**Mitigação:** extrair `print_result` para um módulo compartilhado
(`kata-driver/src/display.rs`) que ambos (driver JIT e shim AOT) usam.
O shim é gerado com o código de display embutido.

### R5: REPL — `infer_module` não foi designado para extensão incremental

`infer_module` recebe um `Module` completo (AST) e produz um `TypedModule`.
O REPL precisa alimentar expressões individuais e acumular bindings no
`TypeEnv`. Isso pode exigir adaptações em `infer_module` ou um caminho
paralelo que reusa o typeck sem reprocessar tudo.

**Mitigação:** o naive (recompilar tudo) funciona sem mudar `infer_module`.
O REPL constrói um `Module` sintético com todos os `pre_entry` acumulados
+ nova expressão como entry. `infer_module` processa isso normalmente.
O custo é recompilar o prelude a cada expressão — aceitável para 1.0.

### R6: `crate-type = ["staticlib", "cdylib"]` pode aumentar tempo de build

kata-rt passa a produzir 3 artefatos (rlib, staticlib, cdylib). O build
fica mais lento. Se for problemático, `cdylib` pode ser feature-gated.

**Mitigação:** medir. Se o aumento for >2s, feature-gate `cdylib` atrás
de `--features dynamic` no cargo.

### R7: Tree shaking pode remover código que Actions chamam dinamicamente

Se uma Action chama outra por nome (string) ou via `fork!`, tree shaking
pode não detectar a dependência. O call graph do `fork!` usa `action_name`
(string), não uma referência direta.

**Mitigação:** tree shaking mantém todas as Actions que aparecem em
`fork!` (string match). Se uma Action não é chamada diretamente nem via
`fork!`, é candidata a remoção. Validar com testes.

## 10. Dependências novas

### Crates do workspace

- `kata-tree-shaking` (novo crate, Fase 3)

### Crates externos

- `rustyline = "14"` (em `kata-driver`, Fase 6)

### Mudanças em Cargo.toml

- `kata-rt`: adicionar `crate-type = ["staticlib", "rlib", "cdylib"]`
- `kata-driver`: adicionar `rustyline = "14"`, `kata-tree-shaking`
- `kata-codegen`: já tem `cranelift-object` como dep (usado em Fase 2)

## 11. Ordem de implementação recomendada

```
Fase 1 (ModuleBackend trait + JitBackend)
  ↓
Fase 2 (AotBackend + aot_emit)
  ↓
Fase 3 (Tree shaking)  ← pode rodar em paralelo com Fase 4
  ↓
Fase 4 (kata-rt staticlib + linker)
  ↓
Fase 5 (Shim main.rs + kata build)
  ↓
Fase 6 (REPL — TypeEnv persistente + :type)  ← independente de Fases 1-5
  ↓
Fase 7 (:load + multiline + E2E)
  ↓
Fase 8 (Documentação)
```

Fases 3 e 4 são independentes entre si e podem ser paralelizadas. Fase 6
(REPL) não depende de Fases 1-5 e pode começar em paralelo.