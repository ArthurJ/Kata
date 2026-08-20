# PRD — Sistema de Módulos Rust-style com `super.` e `stdlib.`

**Data:** 2026-08-20
**Status:** Implementado (Fases 1-5 completas)
**Autor:** Arthur + Hermes
**Depende de:** `kata-parser` (imports), `kata-resolution` (module_loader), `kata-driver` (imports)
**Não depende de:** AOT, LSP, codegen, `kata-rt`

## Status de implementação

| Fase | Descrição | Status | Commit |
|---|---|---|---|
| 1 | Parser: Token::Super + parse_import_decl | ✅ | `98aa7ea` |
| 2 | resolve_path: 3 modos + mod.kata | ✅ | `9d4e239` |
| 3 | Recursão de imports + merge_imports em kata-resolution | ✅ | `5a83c88` |
| 4 | Testes E2E (10 testes) | ✅ | `d536621` |
| 5 | Documentação (sintaxe-mapa, manual, TODO) | ✅ | — |

**Débito técnico:** `filter_exports` não preserva dependências transitivas
de funções (fn1 exportada referencia fn2 não-exportada → unbound_name
no importador). Testes E2E contornam com funções autocontidas.

## Contexto

O sistema de módulos de Kata5 tem search paths fixos: `entry_dir` (diretório do
arquivo importador) + `stdlib`. Não há como importar de diretórios pai, nem
configurar search paths adicionais. O TODO registrava "import de módulo inteiro
falha" — era diagnóstico errado; import de módulo inteiro funciona quando o
módulo está no search path correto.

A solução original do handoff era Python-style: `..` no path + `-I`/`KATA_PATH`.
Após discussão, decidimos seguir o modelo Rust: árvore de módulos filesystem-
derived com `super.` para navegação intra-projeto e `stdlib.` como namespace
explícito para a stdlib. Sem search paths configuráveis — Kata5 ainda não suporta
bibliotecas externas.

### Decisões fechadas

1. **`mod.kata` obrigatório para `import math`** (diretório como unidade). Sem
   ele, `import math` erro. `import math.algebra` funciona sem `mod.kata`
   (submódulo direto pelo filesystem).
2. **`mod.kata` não declara filhos** — children são auto-descobertos pelo
   filesystem. `export` em cada arquivo controla visibilidade individual.
3. **Árvore é filesystem-derived** — `super.` navega pela posição no disco,
   não por declarações.
4. **`super.` sobe um nível na árvore de módulos.** Siblings resolvem sem
   `super.` — `import calculus` de `math/algebra.kata` carrega
   `math/calculus.kata` (mesmo diretório). `super` é só para acessar o que
   está **acima** do módulo atual.
5. **`import X` sem qualificador** procura local (diretório do arquivo)
   primeiro, stdlib como fallback. Local sombra stdlib.
6. **`import stdlib.X`** é um path explícito que sempre resolve para a stdlib
   built-in, de qualquer profundidade. Permite acessar a stdlib mesmo quando
   um módulo local faz shadow do nome.
7. **Core é injetado implicitamente** em todo módulo (já funciona hoje via
   `load_prelude`). Não precisa de `import core`.
8. **Sem search paths configuráveis** — sem `-I`, sem `KATA_PATH`. Kata5 ainda
   não suporta bibliotecas externas. Foco em navegação intra-projeto.
9. **Imports sem qualificador continuam relativos ao diretório do arquivo**
   (compatibilidade). `super.` é escape para subir.

### Decisões em aberto (a fechar durante implementação)

- **`super` é keyword** (`Token::Super`) — é navegação relativa, mesma
  natureza de `match`/`import`/`return`. Não pode ser nome de variável,
  função, ou módulo. `let super := 5` é parse error.
- **`stdlib` é ident com handling especial em resolution** — não é keyword.
  Pode ser nome de variável em expressões (`let stdlib := "path"` é válido).
  Mas **é proibido nomear um módulo como `stdlib`**: `stdlib.kata` ou
  `stdlib/mod.kata` gera erro de compilação em resolution ("não é possível
  nomear um módulo como `stdlib` — nome reservado para a stdlib built-in").
  Isto elimina ambiguidade: `import stdlib.X` sempre resolve para a stdlib
  built-in, nunca para um módulo local.
- **`super.super.` (múltiplos níveis)**: aceitar `import super.super.utils`
  para subir dois níveis. Sem objeções até agora — parece natural.

## Objetivos

1. `import super.mod` para acessar módulos em diretórios pai
2. `import stdlib.mod` para acessar a stdlib explicitamente (shadow-safe)
3. `mod.kata` como ponto de entrada de diretórios importados como unidade
4. Diretórios sem `mod.kata` ainda permitem `import dir.submodulo` direto
5. `import super.mod.(items)` e `import super.mod as alias` funcionam
6. Retrocompatibilidade: imports existentes continuam funcionando

## Não-objetivos

- Package manager (estilo Cargo) — fora de escopo
- Bibliotecas externas (`-I`, `KATA_PATH`) — fora de escopo por agora
- `#[path = "..."]` ou atributos de path customizado — escape hatch desnecessário
- Renomeação de módulos via filesystem layout
- Módulos aninhados sem `mod.kata` como unidade (só submódulos diretos)
- Reexportação transitiva automática (`mod.kata` faz reexportação explícita via
  `import` + `export`)

---

## Design

### 1. Árvore de módulos

A árvore de módulos espelha a hierarquia de diretórios. Cada arquivo `.kata` é
um módulo. Cada diretório pode ser um módulo (via `mod.kata`) ou apenas um
namespace (acessando filhos diretamente).

```
projeto/
  main.kata              ← módulo raiz do projeto
  utils.kata             ← módulo "utils" (sibling de main)
  math/
    mod.kata             ← módulo "math" (diretório como unidade)
    algebra.kata         ← submódulo "algebra" de math
    calculus.kata        ← submódulo "calculus" de math
    vectors/
      mod.kata           ← submódulo "vectors" de math
      vec2.kata          ← submódulo "vec2" de vectors
```

### 2. Resolução de imports

Três modos de resolução, mutuamente exclusivos pelo prefixo do path:

#### 2.1. `import X` — local, fallback stdlib

Procura `X.kata` ou `X/mod.kata` no diretório do arquivo importador
(`entry_dir`). Se não encontrar, procura na stdlib.

```kata
# De main.kata (raiz do projeto):
import utils       → utils.kata (local)
import math        → math/mod.kata (local, diretório com mod.kata)
import complex     → stdlib/complex.kata (fallback stdlib, não há local)
```

```kata
# De math/algebra.kata:
import calculus    → math/calculus.kata (local, mesmo diretório)
import complex     → stdlib/complex.kata (fallback stdlib)
```

#### 2.2. `import super.X` — subir um nível

Sobe um nível na árvore de diretórios a partir do `entry_dir` do importador,
depois procura `X` a partir dali.

```kata
# De math/algebra.kata:
import super.utils        → utils.kata (raiz do projeto, um nível acima de math/)
import super.math         → math/mod.kata (sobe para raiz, desce para math)
import super.super.X      → sobe dois níveis (acima da raiz do projeto)
```

```kata
# De math/vectors/vec2.kata:
import super.calculus     → math/calculus.kata (subiu de vectors/ para math/)
import super.super.utils  → utils.kata (subiu de vectors/ → math/ → raiz)
```

`super.` só resolve relativo ao `entry_dir` do arquivo importador. Nunca
resolve relativo a stdlib — `import super.stdlib` não faz sentido.

#### 2.3. `import stdlib.X` — stdlib explícita

Sempre resolve para a stdlib built-in, independente da profundidade ou de
shadows locais. Útil quando um módulo local faz shadow de um nome da stdlib.

```kata
# De qualquer arquivo, em qualquer profundidade:
import stdlib.math       → stdlib/math.kata (sempre)
import stdlib.complex    → stdlib/complex.kata
import stdlib.stdio      → stdlib/stdio.kata

# Shadow: se há math/mod.kata local, `import math` pega o local,
# mas `import stdlib.math` pega o da stdlib.
```

### 3. `mod.kata` — diretório como unidade

`import math` carrega `math/mod.kata`. O `mod.kata` é um módulo normal que
pode importar filhos e re-exportar:

```kata
# math/mod.kata
import algebra       # carrega math/algebra.kata (filho, relativo ao dir)
import calculus      # carrega math/calculus.kata (filho)
import vectors       # carrega math/vectors/mod.kata (filho-diretório)

export dobrar fatorial norm
```

`import math` de `main.kata` → carrega `math/mod.kata` → itens exportados
ficam acessíveis como `math.dobrar`, `math.fatorial`, `math.norm`.

**Sem `mod.kata`:** `import math` erro: "módulo `math` não encontrado". Mas
`import math.algebra` funciona — carrega `math/algebra.kata` diretamente.

### 4. Sintaxe

#### 4.1. `super` e `stdlib` em import paths

`super` e `stdlib` são prefixos especiais do path. `super` pode aparecer
múltiplas vezes no início (`super.super.X`). `stdlib` aparece uma vez no
início. Após o prefixo especial, o restante do path é navegação normal.

```kata
import super.calculus                    # subir 1 nível
import super.super.utils                 # subir 2 níveis
import super.vectors.vec2                # subir 1, descer 2
import super.calculus.(dobrar fatorial)  # seletivo
import super.calculus as calc            # alias

import stdlib.math                       # stdlib explícita
import stdlib.math.(sqrt)                # seletivo da stdlib
import stdlib.complex as cm              # alias da stdlib
```

**Regras de validação:**
- `super` só pode aparecer como prefixo (antes de qualquer componente normal)
- `stdlib` só pode aparecer como primeiro componente, uma única vez
- `super` e `stdlib` não coexistem no mesmo path (`import super.stdlib.X` é
  erro)
- `import super` sozinho é erro (não carrega nada)
- `import stdlib` sozinho é erro (não carrega nada)

**Representação no AST:** `path: Vec<String>` atual usa strings. `super` é
representado como string especial `"super"` (vindo de `Token::Super` no parser)
e `stdlib` como `"stdlib"` (vindo de `Token::Ident("stdlib")`). A resolution
detecta esses prefixos pelas strings. Alternativa: migrar para
`path: Vec<PathComponent>` onde `PathComponent` é `Super | Stdlib |
Normal(String)` — mais type-safe mas exige refactor em todos os consumidores
de `path`. Decidir na implementação; começar com strings se o refactor for
grande.

#### 4.2. Compatibilidade com sintaxe existente

Toda a sintaxe de import atual continua válida:
- `import modulo.submodulo` — sem prefixo especial
- `import modulo.submodulo as alias`
- `import modulo.(item1 item2)`
- `import modulo.item as alias` (açúcar para seletivo)

A única mudança é que `super` e `stdlib` são reconhecidos como prefixos
especiais quando aparecem como primeiro componente do path.

### 5. Resolução concreta — mudanças no `resolve_path`

Atual:

```rust
fn resolve_path(&self, module_path: &[String]) -> Result<PathBuf, LoadError> {
    // monta relative path: a/b/c.kata
    // procura em cada search_path (entry_dir + stdlib)
}
```

Novo:

```rust
fn resolve_path(
    &self,
    module_path: &[PathComponent],
    entry_dir: &Path,
) -> Result<PathBuf, LoadError> {
    // 1. Extrair prefixo: Super* | Stdlib | None
    //
    // 2. Se prefixo é Super*:
    //    - Consumir N Super, subindo N níveis de entry_dir → resolved_base
    //    - Montar relative path dos componentes Normal restantes
    //    - Procurar SÓ em resolved_base (não fallback stdlib)
    //    - Detecção de mod.kata em cada diretório intermediário
    //
    // 3. Se prefixo é Stdlib:
    //    - Montar relative path dos componentes Normal
    //    - Procurar SÓ no stdlib_dir (hardcoded)
    //
    // 4. Se sem prefixo:
    //    - Montar relative path dos componentes Normal
    //    - Procurar em entry_dir primeiro
    //    - Se não encontrar, procurar em stdlib_dir (fallback)
    //
    // 5. Para cada componente que é diretório D:
    //    - Se D é o último componente: carregar D/mod.kata
    //    - Se D não é o último: D é só namespace, continuar navegando
}
```

**Path traversal:** `super` só sobe a partir de `entry_dir`. Não há search
paths externos onde `super` pudesse escapar. O limite natural é o diretório
raiz do filesystem — subir além resulta em `NotFound`.

### 6. REPL

`load_repl_imports` atual usa `[".", stdlib]`. Com a mudança:
- `entry_dir` do REPL é `.` (cwd)
- `import X` procura em cwd primeiro, stdlib como fallback
- `super.X` sobe a partir de cwd
- `stdlib.X` sempre resolve stdlib
- Sem mudança estrutural — só adaptar para o novo `resolve_path`

### 7. Core implícito

O prelude (`core.kata`) já é injetado implicitamente em todo módulo via
`load_prelude()` no `ModuleLoader`. Isto não muda. `import core` funciona mas
é redundante — o core já está no escopo.

---

## Fases

### Fase 1 — Parser: aceitar `super` e `stdlib` em import paths

**Objetivo:** `parse_import_decl` aceita `super` e `stdlib` como prefixos
especiais de path.

**Mudanças:**
1. `Token::Super` é nova keyword (lexer + parser). `stdlib` continua como
   `Token::Ident("stdlib")` com handling especial em resolution.
2. Em `parse_import_decl`, aceitar `Token::Super` como primeiro componente
   do path. `super` pode repetir (`super.super.X`). `Token::Ident("stdlib")`
   como primeiro componente é aceito e marcado para resolution tratar como
   prefixo stdlib.
3. Após o prefixo especial, o loop existente de `Dot Ident` continua normal.
4. Validação:
   - `super` só como prefixo (antes de componentes normais)
   - `stdlib` só como primeiro componente, uma vez
   - `super` e `stdlib` não coexistem
   - `import super` e `import stdlib` sozinhos são erro

**Verificação:** `cargo test -p kata-parser` + novos testes unitários:
- `import super.calculus` → path = `["super", "calculus"]`
- `import super.super.utils` → path = `["super", "super", "utils"]`
- `import super.vectors.vec2` → path = `["super", "vectors", "vec2"]`
- `import super.calculus.(dobrar)` → seletivo com super
- `import super.calculus as calc` → alias com super
- `import stdlib.math` → path = `["stdlib", "math"]`
- `import stdlib.math.(sqrt)` → seletivo da stdlib
- `import stdlib.complex as cm` → alias da stdlib
- `import math.super` → erro (super não é componente normal)
- `import super` → erro (super sozinho)
- `import stdlib` → erro (stdlib sozinho)
- `import super.stdlib.X` → erro (super e stdlib não coexistem)
- Imports existentes continuam parseando sem mudança

### Fase 2 — resolve_path: resolver `super`, `stdlib`, e `mod.kata`

**Objetivo:** `ModuleLoader::resolve_path` resolve os três modos e detecta
`mod.kata`.

**Mudanças:**
1. `resolve_path` recebe `entry_dir` como parâmetro adicional.
2. Detectar prefixo do path (`Super*` | `Stdlib` | `None`).
3. Se `Super*`: subir N níveis de `entry_dir`, procurar só no resolved_base.
4. Se `Stdlib`: procurar só no `stdlib_dir` (hardcoded via
   `CARGO_MANIFEST_DIR`).
5. Se `None`: procurar em `entry_dir` primeiro, `stdlib_dir` como fallback.
6. Detecção de `mod.kata`: se componente é diretório e é o último componente,
   carregar `dir/mod.kata`. Se não tem `mod.kata`, `NotFound`. Se componente é
   diretório mas não é o último, é namespace — continuar navegando.

**Verificação:** `cargo test -p kata-resolution` + novos testes unitários:
- Estrutura com `mod.kata` → `import math` carrega `mod.kata`
- Diretório sem `mod.kata` → `import math` erro, `import math.algebra` OK
- `super.calculus` resolve sibling no diretório pai
- `super.super.utils` resolve dois níveis acima
- `stdlib.math` resolve stdlib built-in
- `import math` com math local → carrega local (não stdlib)
- `import stdlib.math` com math local → carrega stdlib (não local)
- `import complex` sem complex local → fallback stdlib

### Fase 3 — Driver: propagar entry_dir para resolve_path

**Objetivo:** `load_module_imports` e `load_repl_imports` passam `entry_dir`
corretamente para que `super.` funcione.

**Mudanças:**
1. `load_module_imports` já tem `entry_dir` (linha 216-219). Passar para
   `ModuleLoader` ou tornar explícito na chamada de `load_imports`.
2. `ModuleLoader::load_imports` precisa saber o `entry_dir` do módulo
   importador para resolver `super.` nos imports deste módulo.
3. Quando `ModuleLoader` carrega um sub-módulo (ex: `math/mod.kata`), o
   `entry_dir` para os imports *desse* sub-módulo é `math/` (diretório do
   arquivo carregado), não o `entry_dir` original. Cada módulo na cadeia de
   imports usa seu próprio diretório como base para `super.`.

**Verificação:** `cargo test -p kata-driver` + testes E2E:
- `main.kata` importa `math` (com mod.kata), `mod.kata` importa `super.utils`
  → `utils.kata` na raiz é carregado
- `math/algebra.kata` importa `super.calculus` → carrega `math/calculus.kata`
  (espera — `super` sobe de `math/` para raiz, procura `calculus` na raiz.
  Para sibling, usar `import calculus` sem `super`)
- `math/algebra.kata` importa `import calculus` → carrega `math/calculus.kata`
  (sibling, sem super)
- `math/vectors/vec2.kata` importa `import super.calculus` → carrega
  `math/calculus.kata` (subiu de vectors/ para math/)
- `math/vectors/vec2.kata` importa `import super.super.utils` → carrega
  `utils.kata` (subiu para raiz)

### Fase 4 — Testes E2E

**Objetivo:** Cobertura completa de cenários reais.

**Testes em `kata-driver/tests/`:**

1. **Estrutura básica com mod.kata:**
   ```
   tmp/main.kata       → import math
   tmp/math/mod.kata   → import algebra; export dobrar
   tmp/math/algebra.kata → dobrar :: Int => Int lambda x: + x 2
   ```
   Verificar: `math.dobrar 3` retorna 5

2. **Sibling via import sem super:**
   ```
   tmp/math/algebra.kata → import calculus.(integrar)
   tmp/math/calculus.kata → integrar :: Int => Int lambda x: x
   ```
   Verificar: import resolve e função é chamável

3. **super. para um nível acima:**
   ```
   tmp/main.kata → import math
   tmp/math/mod.kata → import super.utils.(helper)
   tmp/utils.kata → helper :: Int => Int lambda x: x
   ```
   Verificar: super.utils carrega utils.kata na raiz

4. **super.super para dois níveis:**
   ```
   tmp/math/vectors/vec2.kata → import super.super.utils.(helper)
   tmp/utils.kata → helper :: Int => Int lambda x: x
   ```

5. **Diretório sem mod.kata:**
   ```
   tmp/math/ (sem mod.kata, tem algebra.kata)
   import math        → erro "módulo não encontrado"
   import math.algebra → OK
   ```

6. **stdlib. explícito com shadow:**
   ```
   tmp/main.kata → import math; import stdlib.math.(sqrt)
   tmp/math/mod.kata → export dobrar
   ```
   Verificar: `math.dobrar` é local, `sqrt` é da stdlib

7. **Fallback stdlib sem shadow:**
   ```
   tmp/main.kata → import complex
   ```
   Verificar: carrega stdlib/complex.kata (não há complex local)

8. **REPL com super.:**
   ```
   cd tmp/math
   kata repl
   > import super.utils
   ```

9. **Retrocompatibilidade:**
   ```
   Todos os examples/ e stdlib imports existentes continuam funcionando
   ```

### Fase 5 — Documentação

1. `docs/sintaxe-mapa.md` — adicionar `super.`, `stdlib.`, `mod.kata` na
   sintaxe de import
2. `docs/Kata-lang-manual.md` — seção de módulos reescrita com modelo
   Rust-style: árvore de módulos, `super.`, `stdlib.`, `mod.kata`, core
   implícito
3. `docs/TODO.md` — marcar feature como implementada, remover diagnóstico
   antigo
4. `docs/kata-book/` — cap de módulos (se existir) atualizado
5. Skill `kata-compiler` — atualizar § Module system e § Parser em pitfalls

**Princípio:** documentação de referência sintática reflete o que o
parser/resolution/typechecker realmente aceitam. Verificar cada afirmação
contra o código antes de documentar.

---

## Atualização da documentação

Ao concluir:
- `docs/TODO.md` — remover item de search paths, marcar como implementado
- `docs/sintaxe-mapa.md` — adicionar sintaxe `super.`, `stdlib.`, `mod.kata`
  (referência do usuário — solicitar permissão antes de editar)
- `docs/Kata-lang-manual.md` — seção de módulos reescrita (referência do
  usuário — solicitar permissão)
- `docs/kata-book/` — cap de módulos se houver (referência do usuário)
- Skill `kata-compiler` — atualizar § Module system e § Parser em pitfalls
  (interno)

## Regras críticas

- Ler `docs/sintaxe-mapa.md` E `docs/Kata-lang-manual.md` antes de investigar
- `patch` >15 linhas Rust com aspas = corrompe. `write_file` .rs dispara
  rustfmt — reler antes de patch
- `print_pipeline_errors` retorna `miette::Report` — propagar via `return
  Err(print_pipeline_errors(errors))`, não envolver em `Report::msg`
- PRD é fonte de verdade para status. Se `cargo test` passa mas PRD diz
  "pendente", PRD está desatualizado
- Testes >120s = bug
- Retrocompatibilidade é mandatória: imports existentes (stdlib, examples,
  testes) continuam funcionando sem alteração
- Cada módulo na cadeia de imports usa seu próprio diretório como `entry_dir`
  para seus imports — `super.` é relativo ao arquivo que faz o import, não
  ao arquivo original
- `super` só sobe — siblings resolvem sem `super` via import sem qualificador
- `stdlib.` é o único modo de acessar a stdlib quando há shadow local

## Estrutura final esperada

```
crates/kata-parser/src/imports.rs          — parse_import_decl aceita super/stdlib (MODIFICADO)
crates/kata-ast/src/item.rs                — ImportDecl path aceita super/stdlib  (PODE MODIFICAR)
crates/kata-resolution/src/module_loader.rs — resolve_path 3 modos + mod.kata    (MODIFICADO)
crates/kata-driver/src/imports.rs          — propagar entry_dir por módulo        (MODIFICADO)
crates/kata-driver/src/repl/mod.rs         — REPL adapta para novo resolve_path   (MODIFICADO)
crates/kata-driver/tests/modules_e2e.rs    — testes de super + stdlib + mod.kata  (NOVO OU EXTENDIDO)
docs/sintaxe-mapa.md                       — sintaxe super + stdlib + mod.kata   (ATUALIZADO)
docs/Kata-lang-manual.md                   — seção módulos reescrita              (ATUALIZADO)
docs/TODO.md                               — item corrigido                      (ATUALIZADO)
docs/PRD-modulos-super.md                  — este PRD                            (NOVO)
```