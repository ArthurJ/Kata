# Product Requirements Document (PRD) - Kata-Lang Compiler

## Fase 6.5: Abstração de Coleções e Iteradores (A Interface ITERABLE)

### 1. Visão Geral e Objetivo
Atualmente, as funções fundamentais de transformação de dados (`map`, `filter`, `fold`, `zip`) na Kata-lang operam estritamente sobre Listas Encadeadas (`[T]`). Com a introdução de novos layouts de memória (Arrays Contíguos `{T}`, Tensores N-Dimensionais e Ranges `[a..b]`), o Múltiplo Despacho falha ao tentar aplicar lógicas funcionais sobre essas coleções.

O objetivo da Fase 6.5 é introduzir o polimorfismo paramétrico de coleções através da super-interface `ITERABLE`, unificando o comportamento de todas as coleções do sistema sem sacrificar a performance (Zero-Cost Abstractions) e sem violar a tipagem forte.

---

### 2. Especificação Arquitetural

#### 2.1. A Interface `ITERABLE` no Prelude
A biblioteca padrão (`src/core/types.kata` ou `prelude.kata`) deverá declarar a interface base `ITERABLE`.
```kata
interface ITERABLE::A
    # Opcionalmente um método interno para instanciar um Cursor/Iterator puro
```

As funções fundamentais deverão ser reescritas para aceitarem a interface:
```kata
map :: (A -> B) T => T_OUT
with 
    T as ITERABLE::A
    T_OUT as ITERABLE::B
```

#### 2.2. Implementação Implícita no TypeChecker
Uma vez que `Array`, `List`, `Tensor` e `Range` são primitivas nativas cujos layouts de memória são controlados pelo compilador, o `TypeChecker` (`src/type_checker/arity_resolver.rs`) será atualizado para reconhecer implicitamente que esses tipos satisfazem a restrição `with T as ITERABLE`. O usuário não precisará escrever `Array implements ITERABLE`.

---

### 3. Impacto no Otimizador (MIR)

A Fase de **Stream Fusion** (`src/optimizer/passes/stream_fusion.rs`) precisará de uma grande refatoração.
*   **Problema Atual:** A função de fusão sintetizada assume que a estrutura base é um `Cons(head, tail)` de Lista Encadeada.
*   **Nova Solução:** O passe de Fusão de Fluxo precisará consultar o `TypeEnv` para identificar a topologia da coleção original:
    *   Se for `List::T`: Funde usando recursão de cauda sobre `Cons` (comportamento atual).
    *   Se for `Array::T`: Sintetiza um loop imperativo interno baseado em índices/offsets estáticos.
    *   Se for `Range::T`: Sintetiza um gerador matemático baseado nos limites de início/fim/passo sem alocar a coleção em memória (Lazy Evaluation plena).

---

### 4. Impacto no Backend (Codegen) e Runtime

*   **Loops Imperativos (`for x in iteravel`):** O codegen `translator.rs` e `expr.rs` precisa suportar o desempacotamento de arrays contíguos durante os loops `For` nas Actions.
*   **Alocação Dinâmica AOT:** O resultado de um `map` sobre um `Array` devolve um novo `Array`. O Cranelift deve gerar chamadas para o `kata_rt_alloc_local` dimensionando a arena com `tamanho_do_array * tamanho_do_tipo` em runtime.

---

### 5. Critérios de Aceite (Definition of Done)

- [ ] O script `examples/test_array.kata` (restaurado para usar Arrays literais `{10 20 30}`) compila perfeitamente sem `Type Mismatch`.
- [ ] O encadeamento `{1.0 2.0 3.0} |> map $(* _ 2.0) |> filter (> _ 3.0)` executa e produz um novo Array na memória contígua usando apenas 1 alocação (Stream Fusion ativo).
- [ ] O Múltiplo Despacho é capaz de resolver funções genéricas que impõem limites de `ITERABLE`.
- [ ] Laços imperativos `for x in array_de_floats` em Actions são traduzidos nativamente para avanços de ponteiro (Pointer Chasing) de 8 bytes no Cranelift.