# PRD — Refinement Propagation (Path Conditions no Typeck)

**Status:** ✅ Nível 1 implementado
**Data:** 2026-08-23
**Implementado em:** sessão 2026-08-23 — `path_conditions.rs`, `_match.rs`, `apply_lambda.rs`, `ascription.rs`, `lowering/expr.rs`
**Depende de:** `const_eval_predicate` (const_eval.rs) ✅, `Z3Translator` (guard_completeness.rs) ✅, `StructRegistry` com predicados ✅, smart constructors falíveis (constructors_refined.rs) ✅
**Não depende de:** Nenhum PRD pendente

## 1. Objetivo

Propagar facts (conhecimento acumulado de guards) como path conditions no
visitor de inferência, permitindo ao Z3 provar que ascriptions refinadas
são válidas mesmo quando o valor não é literal.

Hoje, `const_eval_predicate` só valida ascriptions refinadas sobre
**literais** (`5::PositiveInt`). Para não-literais (`n::PositiveInt` onde
`n` é `Ident`), retorna `None` — a ascription passa sem verificação
compile-time, adiando para o smart constructor em runtime.

Com path conditions, o fato `n > 0` (extraído do guard do braço) alimenta
o Z3, que prova `n > 0 ⟹ n > 0` — a ascription é válida em compile-time
sem precisar de literal.

**Escopo:** Nível 1 (guards locais). Níveis 2 e 3 são futuro.

## 2. Motivação

### 2.1. Ascription-refined só funciona para literais

```kata
data (Int, > _ 0) as PositiveInt

lambda n: Int:
    match (> n 0):
        Boolean::True:  n::PositiveInt
        Boolean::False: Result::Err "não positivo"
```

No braço `True`, **sabemos** que `n > 0` veio do guard do match. Mas
`const_eval_predicate` recebe `n` (Ident, não literal) → retorna `None`
→ fallback sem verificação. O fato `n > 0` está visível no contexto mas
o typeck não enxerga.

### 2.2. O conhecimento já está disponível

O visitor de inferência (`_match.rs:217-230`) processa guards antes de
inferir o body de cada braço. A condição do guard é um `TypedExpr`
booleano que já passa pelo `Z3Translator` em `guard_completeness.rs`.
O `StructRegistry` já carrega predicados de tipos refinados. O smart
constructor falível já é o fallback conservador em runtime.

O único componente ausente é a **coleta e propagação** desses facts
até o ponto onde ascriptions são validadas.

### 2.3. Benefício: validação compile-time onde antes só runtime

Sem path conditions, `n::PositiveInt` no braço `True` compila sem
verificação — o smart constructor executa em runtime e pode retornar
`Err`. Com path conditions, o typeck prova que o predicado é satisfeito
no contexto do braço e a ascription é válida estaticamente.

## 3. Componentes existentes

### 3.1. `const_eval_predicate` (const_eval.rs)

Avalia predicados sobre literais. Substitui `Hole` pelo valor, reduz
comparações (`=`, `<`, `>`, `<=`, `>=`) sobre `IntLit`/`FloatLit`.
Retorna `Some(bool)` para literais, `None` para não-literais.

### 3.2. `Z3Translator` (guard_completeness.rs:270-459)

Traduz `TypedExpr` → expressões Z3 (`Int`, `Bool`). Suporta:
- Literais Int → `Int::from_i64`
- `Ident` → `Int::new_const` ou `Bool::new_const` (com cache)
- `>`, `<`, `>=`, `<=`, `=`, `!=` → operações Z3
- `+`, `-`, `*` → aritmética Z3
- `and`, `or`, `not` → lógica proposicional Z3
- Qualquer outra construção → variável booleana opaca (fallback conservador)

### 3.3. `StructRegistry` com predicados

Cada tipo refinado (`PositiveInt`) registra seus predicados como
`Vec<Spanned<Expr>>` no `StructRegistry`. O ascription inference
(`ascription.rs:264-293`) itera sobre predicados e chama
`const_eval_predicate` para cada um.

### 3.4. Smart constructor falível (constructors_refined.rs)

`PositiveInt :: Int => Result::(PositiveInt, Text)` — valida em runtime.
Retorna `Ok` se predicados satisfeitos, `Err` com mensagem se falha.
É o fallback conservador quando compile-time não consegue provar.

### 3.5. Visitor de match (_match.rs:158-254)

`infer_match` processa cada braço:
1. Cria escopo filho (`env.push_scope()`)
2. Tipa pattern (bindings entram no escopo)
3. Tipa guard como `TypedExpr` booleano (linhas 217-230)
4. Tipa body do braço com hint (linhas 235-242)

O guard tipado é o `TypedExpr` que carrega o fact. Hoje ele é usado
apenas para verificação de tipo (`ty == Boolean`) e descartado.

### 3.6. Visitor de lambda (lambda.rs + apply_lambda.rs)

`infer_lambda_body` processa cláusulas com guards de forma similar:
tipa guard, verifica tipo, infere body. Mesmo padrão de escopo filho.

## 4. Design

### 4.1. PathConditionCtx

Nova estrutura que carrega facts acumulados:

```rust
/// Facts (path conditions) acumulados no contexto de inferência.
///
/// Cada fact é um TypedExpr booleano conhecido como verdadeiro no
/// ponto atual do programa (ex: guard de match, condição de braço).
/// Quando uma ascription-refined é encontrada, os facts são
/// conjuntamente asserting no Z3 junto com o predicado, e o Z3
/// prova se o predicado é implicado.
#[derive(Clone, Default)]
pub(crate) struct PathConditionCtx {
    /// Facts acumulados: cada um é verdadeiro no escopo atual.
    facts: Vec<TypedExpr>,
}
```

### 4.2. Propagação no InferCtx

Adicionar `path_conditions: PathConditionCtx` ao `InferCtx`:

```rust
pub(crate) struct InferCtx<'a> {
    pub table: &'a DispatchTable,
    // ... campos existentes ...
    pub path_conditions: PathConditionCtx,
}
```

Como `InferCtx` é passado por referência imutável (`&InferCtx`), a
mutação dos facts não pode ser direta. Duas opções:

**Opção A — `RefCell<PathConditionCtx>`:**
`InferCtx` já usa `RefCell` para `deferred_lambdas`. Mesmo padrão.
O visitor de match/lambda faz `borrow_mut()` para adicionar facts
antes de inferir o body, e `borrow_mut()` para remover ao sair.

**Opção B — Snapshot/restore:**
O visitor cria um snapshot (`Clone`) antes de adicionar facts,
restaura ao sair do braço. Mais simples, sem `RefCell`, mas
copia o contexto a cada braço.

**Recomendação:** Opção B (snapshot/restore). `PathConditionCtx` é
leve (`Vec<TypedExpr>`), `Clone` é barato, e o número de braços é
pequeno. Evita `RefCell` e seus riscos de borrow panic. O visitor
já cria escopo filho (`push_scope`) — o snapshot segue o mesmo
padrão lexical.

### 4.3. Coleta no visitor de match

Em `infer_match`, após tipar o guard (linha 217-230) e antes de inferir
o body (linha 235):

```rust
// Após tipar guard:
let typed_guard = if let Some(guard_expr) = &arm.guard {
    let guard_typed = infer_expr(...)?;
    // ... validação de tipo Boolean ...

    // NOVO: coletar fact do guard.
    // O guard é verdadeiro neste braço — adicionar como path condition.
    let mut arm_ctx = ctx.clone_with_path_conditions();
    arm_ctx.path_conditions.add_fact(guard_typed.clone());

    // Inferir body com path conditions do braço.
    let typed_body = infer_expr_hinted(
        &arm.body.node, &arm.body.span,
        &mut arm_env, &arm_ctx, tail_pos, hint,
    )?;
    // ... resto do processamento ...
} else {
    // Sem guard — path conditions do escopo externo (herdadas).
    let typed_body = infer_expr_hinted(...)?;
};
```

**Atenção:** `InferCtx` hoje é `&InferCtx` (imutável). Para snapshot/
restore, o visitor precisa criar uma cópia local com facts adicionados.
Como `InferCtx` carrega referências (`&'a DispatchTable`, etc.), o
`Clone` copia apenas as referências + `PathConditionCtx` — é barato.

Alternativa sem clonar `InferCtx`: passar `PathConditionCtx` como
parâmetro adicional em `infer_expr_hinted` e `infer_type_ascription`.
Mais invasivo (muda assinaturas) mas evita clone.

**Decisão de design:** Ver §4.6.

### 4.4. Coleta no visitor de lambda

`infer_lambda_body` (apply_lambda.rs) processa guards de cláusulas
lambda. Mesmo padrão: após tipar guard, adicionar como fact antes de
inferir o body da cláusula.

### 4.5. Consulta no ascription inference

Em `infer_type_ascription` (ascription.rs:264-293), quando
`const_eval_predicate` retorna `None` (não-literal):

```rust
None => {
    // Predicado complexo — não avaliável localmente pelo const_eval.

    // NOVO: tentar Z3 com path conditions antes de adiar para comptime.
    if let Some(proven) = try_prove_with_path_conditions(
        pred,
        expr,
        &ctx.path_conditions,
    ) {
        if proven {
            // Predicado provado pelas path conditions — OK.
            continue;
        } else {
            // Predicado refutado pelas path conditions — erro compile-time.
            return Err(MiddleError::TypeMismatch {
                expected: format!("predicado {i} de {} satisfeito", key.name()),
                found: "predicado refutado pelas path conditions".into(),
                span: expr.span.into(),
            });
        }
    }

    // Fallback: adiar para comptime (comportamento atual).
    let substituted = super::const_eval::substitute_hole(pred, expr);
    let typed_pred = infer_expr_hinted(...)?;
    pending.push(Spanned::new(typed_pred, substituted.span));
}
```

### 4.6. `try_prove_with_path_conditions`

Nova função em `const_eval.rs` ou `guard_completeness.rs`:

```rust
/// Tenta provar que o predicado é satisfeito dado as path conditions.
///
/// Constrói no Z3: `(fact1 ∧ fact2 ∧ ... ∧ factN) ⟹ predicado`
/// e verifica se é tautologia (i.e., `facts ∧ ¬predicado` é UNSAT).
///
/// Retorna:
/// - `Some(true)` — predicado provado satisfeito pelas path conditions.
/// - `Some(false)` — predicado refutado (path conditions implicam ¬predicado).
/// - `None` — Z3 não decidiu (Unknown). Fallback conservador.
fn try_prove_with_path_conditions(
    pred: &Spanned<Expr>,
    value: &Spanned<Expr>,
    path_conditions: &PathConditionCtx,
) -> Option<bool> {
    if path_conditions.is_empty() {
        return None; // Sem path conditions — nada a fazer.
    }

    // Substitui Hole por value no predicado.
    let substituted = substitute_hole(pred, value);

    // Necessário tipar o predicado e os facts para Z3Translator.
    // Os facts já são TypedExpr; o predicado substituído precisa ser tipado.
    // ... (ver §4.7 sobre tipagem do predicado) ...

    let cfg = z3_config();
    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();

        // Traduz facts como asserts (conjunção).
        let z3_facts: Vec<Bool> = path_conditions.facts()
            .iter()
            .map(|f| translator.translate_bool(f))
            .collect();
        let facts_conjunction = Bool::and(&z3_facts);

        // Traduz predicado.
        let z3_pred = translator.translate_bool(&typed_pred);

        // Asserção: facts ∧ ¬predicado.
        // Se UNSAT, predicado é implicado pelas path conditions.
        solver.assert(facts_conjunction);
        solver.assert(z3_pred.not());

        match solver.check() {
            SatResult::Unsat => Some(true),   // provado
            SatResult::Sat => Some(false),    // refutado
            SatResult::Unknown => None,       // indecidível
        }
    })
}
```

### 4.7. Tipagem do predicado substituído

O `Z3Translator` opera sobre `TypedExpr`, mas o predicado substituído
(`substitute_hole(pred, expr)`) é `Spanned<Expr>` (não tipado). Hoje,
o fallback `None` em `ascription.rs` já tipa o predicado substituído
via `infer_expr_hinted` (linhas 281-289). Essa tipagem pode ser
reutilizada: tipar o predicado uma vez, usar para Z3 e/ou para pending.

### 4.8. Extraindo facts de match sobre Boolean

Caso especial importante: `match (> n 0)` com braços `Boolean::True` /
`Boolean::False`. O scrutinee é uma expressão booleana, e cada braço
sabe que o scrutinee é `True` ou `False`.

Hoje, o pattern `Boolean::True` é tratado como variant — coleta
`"True"` para exaustividade. Mas não extrai o fact `scrutinee = True`.

Para Nível 1, o fact extraído deve ser:
- Braço `Boolean::True`: a condição do scrutinee é verdadeira
  (`(> n 0)` é true, i.e., `n > 0`)
- Braço `Boolean::False`: a condição do scrutinee é falsa
  (`(> n 0)` é false, i.e., `¬(> n 0)`, i.e., `n <= 0`)

Isso exige que o visitor de match reconheça:
1. Scrutinee é `Boolean` (tipo)
2. Pattern é `Boolean::True` ou `Boolean::False` (variant)
3. Extrair o scrutinee tipado como fact (positivo para True, negado para False)

**Implementação:** Em `infer_match`, após tipar o scrutinee e antes de
processar os braços, se `scrutinee_ty == Ty::boolean()`, guardar
referência ao `typed_scrutinee`. Ao processar cada braço, se o pattern
é `Boolean::True`, adicionar `typed_scrutinee` como fact. Se
`Boolean::False`, adicionar `not(typed_scrutinee)`.

### 4.9. Guard como fact direto

Para guards explícitos (`> x 0:`), o guard tipado é o fact direto —
já é um `TypedExpr` booleano. Não há extração especial, basta adicionar
ao `PathConditionCtx`.

## 5. Exemplos

### 5.1. Guard direto → ascription

```kata
data (Int, > _ 0) as PositiveInt

classify :: Int => Result::(PositiveInt, Text)
    lambda n:
        n:
            > n 0: n::PositiveInt
            otherwise: Result::Err "não positivo"
```

No braço `> n 0:`, o guard tipado `(> n 0)` entra como path condition.
Quando `n::PositiveInt` é inferido, `const_eval_predicate` retorna
`None` (n não é literal). `try_prove_with_path_conditions` é chamado:
- Facts: `[> n 0]`
- Predicado: `> _ 0` → substituído: `> n 0`
- Z3: `(n > 0) ∧ ¬(n > 0)` → UNSAT → `Some(true)`
- Ascription válida em compile-time. Sem smart constructor em runtime.

### 5.2. Match sobre Boolean → ascription

```kata
data (Int, > _ 0) as PositiveInt

classify :: Int => Result::(PositiveInt, Text)
    lambda n:
        match (> n 0):
            Boolean::True:  n::PositiveInt
            Boolean::False: Result::Err "não positivo"
```

No braço `Boolean::True`, o fact extraído é `(> n 0)` (scrutinee).
No braço `Boolean::False`, o fact é `¬(> n 0)`.

`n::PositiveInt` no braço True:
- Facts: `[> n 0]`
- Predicado: `> n 0`
- Z3 prova `(n > 0) ⟹ (n > 0)` → UNSAT → `Some(true)`

### 5.3. Predicado refutado pelas path conditions

```kata
data (Int, > _ 0) as PositiveInt

classify :: Int => Result::(PositiveInt, Text)
    lambda n:
        n:
            <= n 0: n::PositiveInt   # ERRO: n <= 0 contradiz > n 0
            otherwise: Result::Err "não positivo"
```

No braço `<= n 0:`, o fact é `(<= n 0)`.
`n::PositiveInt`:
- Facts: `[<= n 0]`
- Predicado: `> n 0`
- Z3: `(n <= 0) ∧ ¬(n > 0)` → SAT → `Some(false)`
- Erro compile-time: "predicado refutado pelas path conditions"

### 5.4. Z3 indecidível — fallback conservador

Se o predicado envolve construção que o `Z3Translator` não suporta
(qualquer coisa além de comparações/aritmética/booleanos), o tradutor
produz variáveis opacas. Z3 retorna Unknown → `None` → fallback para
comptime/runtime (comportamento atual). Sem regressão.

## 6. Estrutura de dados

### 6.1. `PathConditionCtx`

```rust
/// Facts acumulados no contexto de infereração.
/// Cada fact é um TypedExpr booleano verdadeiro no escopo atual.
#[derive(Clone, Default)]
pub(crate) struct PathConditionCtx {
    facts: Vec<TypedExpr>,
}

impl PathConditionCtx {
    /// Adiciona um fact (TypedExpr booleano verdadeiro no escopo).
    fn add_fact(&mut self, fact: TypedExpr) {
        self.facts.push(fact);
    }

    /// Facts acumulados.
    fn facts(&self) -> &[TypedExpr] {
        &self.facts
    }

    /// True se não há path conditions.
    fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }
}
```

### 6.2. Modificação do `InferCtx`

```rust
pub(crate) struct InferCtx<'a> {
    // ... campos existentes ...
    pub path_conditions: PathConditionCtx,
}
```

**Atenção:** `InferCtx` é construído em vários pontos (mod.rs, lambda.rs,
constructors_refined.rs, etc.). Todos precisam inicializar
`path_conditions: PathConditionCtx::default()`.

### 6.3. Alternativa: parâmetro explícito

Em vez de modificar `InferCtx`, passar `&PathConditionCtx` como
parâmetro adicional em `infer_expr_hinted` e `infer_type_ascription`.

**Vantagem:** Não modifica `InferCtx` (menos call sites afetados).
**Desvantagem:** Modifica assinaturas de funções chamadas em muitos
pontos. O visitor de match/lambda cria um `PathConditionCtx` local,
adiciona facts, e passa para `infer_expr_hinted`.

**Decisão:** Ver §7.1.

## 7. Decisões de design

### 7.1. Onde path conditions vivem — `InferCtx` vs parâmetro

**`InferCtx`:** natural porque path conditions são contexto de inferência.
Mas `InferCtx` é `&InferCtx` (imutável) em todos os call sites —
mutação exige `RefCell` ou snapshot/restore com clone.

**Parâmetro explícito:** mais локально, não toca `InferCtx`. Mas
`infer_expr_hinted` é chamada em ~15 lugares — todos precisam do
parâmetro. E a propagação precisa ser manual em cada ponto de chamada.

**Recomendação:** Começar com **parâmetro explícito** em
`infer_expr_hinted` e `infer_type_ascription` apenas. Isso isola a
mudança: o visitor de match/lambda cria `PathConditionCtx`, adiciona
facts, passa adiante. Se a propagação se mostrar natural, migrar para
`InferCtx` depois. Assim, Nível 1 não toca na estrutura de `InferCtx`.

### 7.2. Tipagem do predicado para Z3

O `Z3Translator` opera sobre `TypedExpr`. O predicado substituído é
`Spanned<Expr>`. Duas opções:

**A — Tipar sempre:** Antes de chamar Z3, tipar o predicado via
`infer_expr_hinted`. Reutiliza a tipagem que o fallback já faz.

**B — Estender Z3Translator para `Spanned<Expr>`:** O tradutor operaria
sobre AST não-tipada. Menos preciso (sem tipos para distinguir Int de
Bool em Idents), mas evita a tipagem.

**Recomendação:** Opção A. A tipagem já é necessária para o fallback
(pending), então reutilizá-la para Z3 é natural. Se `infer_expr_hinted`
falhar (predicado não tipa), isso é erro de tipo independente de path
conditions — já deveria falhar.

### 7.3. Extração de facts de match sobre Boolean

O caso `match (> n 0): Boolean::True / Boolean::False` é o padrão mais
comum para refinement propagation. A extração do fact do scrutinee é
essencial para Nível 1.

**Implementação:** Em `infer_match`, se `scrutinee_ty` é `Boolean`:
- Antes de processar os braços, guardar `typed_scrutinee`
- Ao processar braço com pattern `Boolean::True`: adicionar
  `typed_scrutinee` como fact
- Ao processar braço com pattern `Boolean::False`: adicionar
  `not(typed_scrutinee)` como fact (construir `TypedExprKind::Closure
  { callee: Ident("not"), args: [typed_scrutinee] }`)

### 7.4. Composição de facts

Múltiplos níveis de match/lambda aninham facts. Ex:

```kata
lambda n:
    match (> n 0):
        Boolean::True:
            match (> n 10):
                Boolean::True: n::BigPositive
                Boolean::False: n::PositiveInt
```

No braço interno `True`, facts = `[> n 0, > n 10]`. A conjunção é
asserted no Z3. O snapshot/restore natural do escopo lexical cuida
disso: cada braço cria seu `PathConditionCtx` a partir do externo
(Clone + add_fact), e o contexto externo é restaurado ao sair.

### 7.5. Fallback conservador

Se Z3 retorna `Unknown`, `try_prove_with_path_conditions` retorna `None`
— mesmo fallback de hoje (pending para comptime, smart constructor em
runtime). **Nenhuma regressão**: código que compila hoje continua
compilando. A única mudança é que alguns casos que hoje passam sem
verificação agora são provados (ou refutados) em compile-time.

## 8. Plano de implementação

### Fase 1 — `PathConditionCtx` e prova Z3

1. Criar `PathConditionCtx` em `const_eval.rs` ou novo arquivo
   `path_conditions.rs`
2. Implementar `try_prove_with_path_conditions` reutilizando
   `Z3Translator` de `guard_completeness.rs`
3. Integrar na chamada de `const_eval_predicate` em `ascription.rs`:
   quando `None`, tentar Z3 com path conditions antes de pending
4. Path conditions vazias → `None` (sem mudança de comportamento)

**Critério:** `cargo test -p kata-inference` passa. Nenhum teste novo
ainda — apenas garantir que path conditions vazias não mudam nada.

### Fase 2 — Coleta no visitor de match

1. Adicionar parâmetro `path_conditions: &PathConditionCtx` em
   `infer_expr_hinted` (default: `&PathConditionCtx::default()`)
2. Em `infer_match`: criar `PathConditionCtx` local, adicionar guard
   como fact, propagar para body do braço
3. Em `infer_match`: extrair fact de `Boolean::True`/`False` quando
   scrutinee é `Boolean`
4. Propagar `path_conditions` para chamadas recursivas de
   `infer_expr_hinted` dentro do body

**Critério:** Testes E2E com guard → ascription (exemplo §5.1) provam
refinement em compile-time. Teste com predicado refutado (§5.3) gera
erro compile-time.

### Fase 3 — Coleta no visitor de lambda

1. Em `infer_lambda_body` (apply_lambda.rs): mesmo padrão de match —
   adicionar guard como fact antes de inferir body da cláusula
2. Propagar path conditions para o body

**Critério:** Testes E2E com lambda + guards + ascription.

### Fase 4 — Testes e edge cases

1. Match aninhado (§7.4) — facts compostos
2. Predicado complexo que Z3 não suporta → Unknown → fallback
3. Match sobre Boolean sem ascription — path conditions não interferem
4. Lambda sem guards — path conditions vazias, sem overhead
5. Ascription sobre literal — `const_eval_predicate` resolve antes de
   Z3 (sem mudança)

**Critério:** Suíte de testes cobre todos os exemplos deste PRD.
`cargo test --workspace` passa.

## 9. Nível 2 — Post-condições de funções (pattern matches)

**Status:** Design especificado, não implementado.

Nível 1 coleta facts **diretos** do código fonte (guard `> n 0` → fact
`> n 0`). Nível 2 extrai **post-condições inter-procedurais**: quando
uma função tem guards que decidem entre variants de um enum (ex:
`Result::Ok` vs `Result::Err`, `Some` vs `None`), o caller que faz
`match (f a b): Ok n: ...` aprende a condição que fez aquele variant
ser produzido.

### 9.1. Caso paradigmático

```kata
div :: Int Int => Result::(Int, Text)
lambda a b:
    = b 0: Result::Err "divisão por zero"
    otherwise: Result::Ok (bi_div a b)
```

No call site `match (div 10 b): Result::Ok n: ...`, o braço `Ok` deveria
saber que `b ≠ 0` — porque o guard `= b 0` produz `Err`, e `Ok` só é
produzido no `otherwise` (negação do guard).

### 9.2. Pass de extração (module-level)

Um pass rodando **antes** da inferência dos call sites analisa cada
`FunctionDef` que retorna `Result` e tem guards. Extrai qual condição
faz a função produzir `Ok` vs `Err`.

**Input:** `&[FunctionDef]` (disponível em `ResolvedModule.functions`)

**Output:** `PostCondTable` — mapa `func_name → Vec<PostCondition>`

```rust
struct PostCondition {
    /// Qual variante do enum esta post-condição descreve.
    /// Ex: "Ok", "Err", "Some", "None", ou variant de enum customizado.
    variant: String,
    /// Enum ao qual o variant pertence (ex: "Result", "Optional").
    enum_name: String,
    /// Condição (sobre os params da função) que produz esta variante.
    /// Negação da disjunção dos guards que produzem outros variants
    /// do mesmo enum.
    condition: TypedExpr,
    /// Nomes dos parâmetros da função, na ordem posicional.
    /// Extraídos dos patterns da primeira cláusula lambda.
    param_names: Vec<String>,
}
```

**Algoritmo por função:**

1. Filtra funções cujo `return_type` é um enum (`Ty::Sum` ou
   `Ty::Generic` com base de enum). `Result`, `Optional`, e enums
   customizados do usuário todos se qualificam.
2. Percorre os guards das cláusulas lambda.
3. Para cada guard, classifica o body: qual variant do enum é produzido.
   - `TypedExprKind::VariantConstruct { enum_name, variant, .. }` →
     registra `(enum_name, variant)`.
   - `TypedExprKind::VariantQual { enum_name, variant, .. }` →
     equivalente para variants sem payload.
   - Outro → não classificável, skip da função.
4. Agrupa guards por variant produzido. Para cada variant V:
   - Post-condição de V = negação da disjunção dos guards que produzem
     **outros** variants do mesmo enum.
   - Para `div`: guard `= b 0` → `Err`. Post-cond de `Ok` = `not(= b 0)`.
     Post-cond de `Err` = `= b 0`.
   - Para `find` com guard `x in lst` → `Some`: Post-cond de `Some` =
     `x in lst`. Post-cond de `None` = `not(x in lst)`.
5. `otherwise` (guard sem condition) produz o body daquele braço —
   classifica o variant pelo body. A negação dos guards anteriores é a
   condição implícita do `otherwise`.
6. Registra na tabela com `param_names` extraídos dos patterns da
   primeira cláusula (`lambda a b:` → `["a", "b"]`).

### 9.3. Threading no InferCtx

`PostCondTable` entra no `InferCtx` como referência imutável:

```rust
pub(crate) struct InferCtx<'a> {
    // ... campos existentes ...
    pub post_conds: &'a PostCondTable,
}
```

Construído em `infer_module` logo após `populate_dispatch_table`, antes
de qualquer `InferCtx` ser instanciado. O pass de extração tipa os
guards reusando o `InferCtx` inicial (que já tem `table`,
`enum_registry`, etc. populados).

### 9.4. Consumo no visitor de match

Em `infer_match` (`_match.rs`), ao processar um braço cujo pattern é
`TypedPattern::Variant { enum_name, variant, .. }`:

1. **Verifica se o scrutinee é `TypedExprKind::Closure { callee: Ident(name), args, .. }`.
2. Consulta `ctx.post_conds.get(name)` → encontra post-condições da função.
3. Identifica o variant do pattern (`enum_name`, `variant`) e busca a
   post-condição correspondente.
4. **Substituição parâmetro→argumento:** mapeia cada `Ident(param_name)`
   na condition pelo arg correspondente do `Closure`.
   - `div`'s condition `not(= b 0)` com `param_names: ["a", "b"]`.
   - Call site `div 10 b` → args = `[10, b]`.
   - `Ident("a")` → `args[0]` (IntLit 10), `Ident("b")` → `args[1]` (Ident "b").
   - Resultado: `not(= b 0)` onde `b` agora referencia o `b` do caller.
5. Adiciona o fact substituído às `arm_path_conditions`.

Funciona para qualquer enum, não só `Result`. Se o pattern é
`Some v` e a função tem guard que decide `Some` vs `None`, a
post-condição de `Some` é adicionada.

### 9.5. Substituição parâmetro→argumento

A substituição é estrutural (alpha-renaming + splice), não unificação.
A condition é uma `TypedExpr` com `Ident`s referenciando nomes dos
params. Para cada `Ident(name)` na condition, se `name` corresponde a
`param_names[i]`, substitui por `args[i].node.clone()`. Se o arg é
`Ident("b")` (mesmo nome léxico, escopo diferente — coincidência), o
fact vira `!= b 0` referenciando o `b` do caller. Se o arg é `(+ m n)`,
vira `!= (+ m n) 0` — o Z3 trata aritmética de Ints, então funciona.
Se o arg é uma chamada de função opaca, o Z3 produz variável opaca
(fallback conservador).

### 9.6. Generalização

Não só `div` — qualquer função com o padrão:

```kata
f :: A B => Result::(C, Text)
lambda x y:
    <guard1>: Err "msg1"
    <guard2>: Err "msg2"
    otherwise: Ok (computation)
```

Post-cond de `Ok` = `not(<guard1> OR <guard2>)`.
Post-cond de `Err` = `<guard1> OR <guard2>`.

Ex: `safe_get :: List::A Int => Result::(A, Text)` com guard
`>= idx (len lst): Err` → braço `Ok` aprende `< idx (len lst)`.

### 9.7. Limitações honestas

1. **Só funções Kata com corpo visível** — FFI (`@ffi`) não tem guards.
   `div` é Kata com corpo (guard no lambda, chamada unchecked é FFI).
   `/` é FFI puro — não se aplica.
2. **Só guards cujo body é diretamente `VariantConstruct` ou
   `VariantQual`** — se o body é expressão complexa que eventualmente
   retorna um variant (ex: `let x := ...; Ok x`), o pass não rastreia
   através de `let`/`match` aninhados. Versão inicial: classificação
   direta apenas.
3. **Só funções não-recursivas** — post-condições de funções recursivas
   não são extraídas (a função chamada pode não ter sido inferida ainda).
4. **Substituição de args complexos** — se o arg é uma chamada de função
   opaca, o Z3 não prova (fallback conservador, sem regressão).

### 9.8. Decisões de design fechadas

1. **Nomes dos parâmetros:** extraídos dos patterns da primeira cláusula
   lambda. Se cláusulas diferentes usam nomes diferentes, a primeira
   vingar (padrão da linguagem — assinatura não nomeia, patterns sim).
2. **`otherwise` como guard negado:** o `otherwise` não tem condition
   explícita — é o fallback. A post-condição do body do `otherwise` é a
   negação da disjunção dos guards anteriores que produziram a variante
   oposta.
3. **Tipagem das condições:** o pass de extração tipa os guards no
   momento da extração, reusando `InferCtx` (que já tem `table`,
   `enum_registry`, etc. populados antes do passo 3 de `infer_module`).
4. **PostCondTable no InferCtx:** `&'a PostCondTable` (borrow), seguindo
   o padrão dos outros campos de `InferCtx` que são referências.

### 9.9. Nível 3 — Contratos de função (futuro)

Tipos refinados em assinaturas (`div :: Int NonZero => ...`) propagados
como path conditions no caller. Se `b::NonZero` está na assinatura, o
caller que passa `b` sabe que `b ≠ 0` no contexto. Refinement typing
completo — o typeck consulta predicados do `StructRegistry` em cada
ascription contra as constraints acumuladas.

Nível 2 e Nível 3 coexistem: Nível 2 para funções Kata com corpo (o
compiler deriva post-condições dos guards), Nível 3 para funções FFI
sem corpo (contratos declarados explicitamente na assinatura).

### Roadmap

Nível 1 cria a infraestrutura (`PathConditionCtx`, prova Z3, coleta no
visitor). Nível 2 adiciona o pass de extração de post-condições e o
consumo no visitor de match. Nível 3 adiciona contratos explícitos em
assinaturas. Cada nível é incremental sobre o anterior.

## 10. Riscos

### 10.1. Performance Z3

Cada ascription-refined sobre não-literal agora pode invocar Z3. Com
rlimit=10000 (configuração atual), cada prova é < 1ms. Path conditions
pouco numerosas (tipicamente 1-3 facts por escopo). Overhead
negligenciável.

### 10.2. Falsos negativos

Se o `Z3Translator` não suporta uma construção do predicado, produz
variável opaca → Z3 retorna Unknown → fallback. Sempre conservador:
nunca rejeita código válido, apenas falha em provar. O smart
constructor em runtime permanece como rede de segurança.

### 10.3. Falsos positivos (refutação errada)

Se o `Z3Translator` traduz incorretamente uma construção, Z3 pode
"provar" que o predicado é refutado quando na verdade é satisfeito.
Mitigação: o tradutor é conservador — construções não suportadas viram
variáveis opacas, não falsos. O risco existe apenas em construções
parcialmente suportadas (ex: aritmética sem overflow checking).

## 11. Dependências

- `z3` crate 0.20.2 (já no Cargo.toml de kata-inference) ✅
- `Z3Translator` (guard_completeness.rs) — reutilizado, não modificado ✅
- `const_eval_predicate` (const_eval.rs) — reutilizado, não modificado ✅
- `StructRegistry` com predicados ✅
- Smart constructors falíveis (constructors_refined.rs) — não modificados ✅

## 12. Verificação

```bash
# Fase 1 — path conditions vazias, sem regressão
cargo test -p kata-inference

# Fase 2 — coleta no match
cargo test -p kata-inference --test lambda_match_inference
cargo test -p kata-codegen --test t_refined_polimorfico_e2e
cargo test -p kata-codegen --test t_refines_e2e

# Fase 4 — suíte completa
cargo test --workspace --no-fail-fast
```