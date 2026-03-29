# Product Requirements Document (PRD) - Kata-Lang Compiler

## Fase 4: Otimizador (MIR - Mid-level Intermediate Representation)

### Visão Geral
A Fase 4 introduz a camada de otimização na Typed Abstract Syntax Tree (TAST), operando estritamente antes da geração de código de máquina pelo backend. Uma vez que o compilador utiliza o **Cranelift** (um gerador de código rápido, porém de muito baixo nível que desconhece conceitos de alto nível como polimorfismo ou recursão de cauda), esta fase é inteiramente responsável por garantir o princípio de **Zero-Cost Abstractions**. O Otimizador transforma código funcional abstrato, genérico e seguro numa estrutura imperativa interna, estática e despida de custos dinâmicos (V-Tables, *boxing*, verificações redundantes em *runtime* ou alocações intermediárias).

### Estrutura de Diretórios (Arquitetura)
O otimizador deve ser construído como um *pipeline* de passagens (Passes), onde cada módulo recebe a TAST e devolve uma versão otimizada da mesma.

```text
src/
└── optimizer/
    ├── mod.rs               # Orquestrador: recebe a TAST e executa os passes em ordem
    ├── passes/              # Diretório contendo cada otimização isolada
    │   ├── mod.rs
    │   ├── tree_shaker.rs   # Reachability / Dead Code Elimination (Passos 1 e 7)
    │   ├── const_folder.rs  # Dobragem de constantes matemáticas/lógicas (Passo 2)
    │   ├── comptime.rs      # Avaliação de Predicados e @comptime (Passo 3)
    │   ├── monomorph.rs     # Especialização de Generics (Passo 4)
    │   ├── tco.rs           # Tail Call Optimization (Passo 5)
    │   └── stream_fusion.rs # Fusão de loops funcionais (Passo 6)
    └── error.rs             # Erros gerados pelo otimizador (ex: Recursão não-TCO)
```

### Objetivos (Pipeline de Passagens)
A ordem de execução é fundamental para que otimizações primárias abram oportunidades para as subsequentes.

#### 1. Early Tree-Shaking (Eliminação Inicial de Código Morto)
*   **Ação:** Construir um Grafo de Chamadas (*Call Graph*) partindo estritamente do ponto de entrada (a Action `main!` ou equivalente).
*   **Responsabilidade:** Remover da TAST todas as declarações *top-level* inatingíveis: funções importadas mas nunca chamadas, interfaces não utilizadas, variantes de enums ignoradas.
*   **Benefício:** Reduz substancialmente o volume de código que as fases pesadas (como Monomorfização) terão de processar, acelerando drasticamente o tempo de compilação.

#### 2. Constant Folding (Dobragem de Constantes)
*   **Ação:** Travessia Bottom-Up (de baixo para cima) da árvore de expressões.
*   **Responsabilidade:** Identificar chamadas a funções matemáticas/lógicas puras da *StdLib* (ex: `+`, `-`, `>`, `and`) onde **todos** os argumentos já sejam `TLiteral`. O compilador (em Rust) resolve a equação e substitui o nó `Call` inteiro pelo `TLiteral` resultante.
*   **Benefício:** Remove o custo de computação estática do binário final. Ramos de `Guard` que avaliem estaticamente para `False` são completamente podados.

#### 3. Otimização de Tipos Refinados e `@comptime`
*   **Ação:** Intercetar a invocação dinâmica de *Smart Constructors* associados a Tipos Refinados (ex: `PositiveInt(10)`).
*   **Responsabilidade:** Se o argumento recebido for um literal conhecido, rodar a árvore de predicados lógicos em tempo de compilação.
    *   Se for válido: Remover o nó do tipo `Result` e embutir o literal puro.
    *   Se for inválido: Emitir erro de compilação imediato (prevenindo que o programa sequer chegue a *runtime* com um estado matematicamente impossível).
*   **Benefício:** Segurança matemática sem o *overhead* tradicional de validação em tempo de execução.

#### 4. Monomorfização (Zero-Cost Generics)
*   **Ação:** Identificar chamadas a funções que fazem uso de *Generics* (resolvidos pela Fase 3 no *Type Checker*).
*   **Responsabilidade:** Para cada combinação única de tipos concretos com os quais a função genérica é invocada (ex: invocação com `Int` e `Float`), clonar a função original, decorando a sua assinatura (ex: `soma_Int_Int`). Modificar a TAST para que os pontos de chamada apontem fisicamente para a variante especializada.
*   **Benefício:** Elimina a necessidade de despacho dinâmico (V-Tables ou *Boxing*). O Cranelift pode gerar código C-ABI nativo, direto e hiperotimizado.

#### 5. Tail Call Optimization (TCO) & Enforcement
*   **Ação:** Analisar as *LambdaDefs* para detetar a presença de recursividade.
*   **Responsabilidade:**
    *   Validar estritamente se a chamada recursiva encontra-se em posição de cauda (*Tail Position*).
    *   Se sim: Transformar o nó de recursão num nó semântico imperativo interno (ex: `TStmt::Loop` / salto JMP).
    *   Se não (ou se a recursão ocorrer num domínio impuro de `Actions`): Lançar um erro fatal de compilação.
*   **Benefício:** Prevenção algorítmica de *Stack Overflows* a nível de arquitetura do Sistema Operativo.

#### 6. Stream-Fusion (Fusão de Fluxos)
*   **Ação:** Detetar chamadas adjacentes/encadeadas a iteradores funcionais (ex: `map` seguido de `filter`).
*   **Responsabilidade:** Mesclar internamente os blocos de processamento lógico num único laço (Loop).
*   **Benefício:** Iteração orientada a Cache (Cache-Friendly), prevenindo a alocação custosa na *Heap* de listas/coleções intermediárias.

#### 7. Late Tree-Shaking (Limpeza Final)
*   **Ação:** Reexecutar o algoritmo do Passo 1 na TAST resultante.
*   **Responsabilidade:** Descartar as *templates* de funções genéricas originais (que já foram monomorfizadas) e limpar blocos mortos (ex: ramos `otherwise` que nunca serão tocados graças ao *Constant Folding*).
*   **Benefício:** TAST Definitiva, enxuta, otimizada e pronta para o Cranelift.

### Requisitos Não-Funcionais
*   **Controle Condicional:** As passagens mais destrutivas/pesadas devem ser ativadas condicionalmente através da *flag* global `--release` definida na CLI (Fase 1). Modos de desenvolvimento iterativo (`kata run` ou `REPL`) podem saltar certas otimizações (como Fusão de Fluxo) para priorizar a velocidade de compilação.
*   **Isolamento:** Nenhuma passagem deve exigir que o *Backend* (Cranelift) esteja ciente da abstração; ou seja, a TAST resultante deve ser representável usando as mesmas estruturas originais da TAST, apenas modificadas semanticamente.

### Entregáveis da Fase 4
1. Diretório raiz `src/optimizer/` construído seguindo a arquitetura em *pipeline*.
2. Implementação das passagens fundamentais iniciais: `tree_shaker.rs`, `const_folder.rs` e a interface de `error.rs`.
3. Integração do orquestrador (`optimizer::optimize(tast)`) no fluxo principal em `main.rs`, antes do repasse para a simulação do `codegen`.
4. Suite de testes unitários dedicada garantindo que literais simples (`+ 2 2`) geram o literal (`4`), e garantindo as falhas do enforcement TCO em lambdas mal formatados.