# **Especificação Técnica: Kata-Lang**

```
  ┌────────────────────────────────────────────────────────────────────────┐
  │                         KATA-LANG ARCHITECTURE                         │
  ├────────────────────────────────────────────────────────────────────────┤
  │  FRONTEND (Parsing)         MIDDLE-END (IR)           BACKEND (Code)   │
  │  ─────────────────          ───────────────           ───────────────  │
  │  ┌───────────┐              ┌───────────┐             ┌─────────────┐  │
  │  │  Lexer    │ → Tokens →   │   AST     │    → IR →   │  Cranelift  │  │
  │  │ (chumsky) │              │  (typed)  │             │   (AOT)     │  │
  │  └───────────┘              └───────────┘             └─────────────┘  │
  │       ↓                          ↓                           ↓         │
  │  ┌───────────┐              ┌───────────┐             ┌─────────────┐  │
  │  │  Parser   │              │   IR      │             │  Object     │  │
  │  │ (chumsky) │              │  (DAG)    │             │   File      │  │
  │  └───────────┘              └───────────┘             └─────────────┘  │
  │                                   ↓                           ↓        │
  │                             ┌───────────┐             ┌─────────────┐  │
  │                             │TreeShaker │             │  Linker     │  │
  │                             │  (Shake)  │             │   (cc)      │  │
  │                             └───────────┘             └─────────────┘  │
  │                                   ↓                           ↓        │
  │                             ┌───────────┐             ┌─────────────┐  │
  │                             │ TCO Pass  │             │  Runtime    │  │
  │                             │(Tail Call)│             │  (kata-rt)  │  │
  │                             └───────────┘             └─────────────┘  │
  └────────────────────────────────────────────────────────────────────────┘
```

## **Kata-Lang Standard Library (StdLib)**

Especificação das primitivas embutidas (*built-ins*) e funções da biblioteca padrão. Todas as funções listadas aqui respeitam a pureza referencial e avaliam de forma estrita, enquanto as *actions* (sufixadas com \!) interagem com o escalonador e recursos do sistema operacional.

### **1\. Functions (Domínio Puro)**

As funções operam sobre dados e retornam dados. Não possuem efeitos colaterais. O compilador aplica fusão de fluxo (*Stream Fusion*) sempre que funções iteráveis são encadeadas.

#### **1.1. Operadores Matemáticos e Lógicos (Interface NUM, ORD, EQ)**

Resolvidos por *Multiple Dispatch* em tempo de compilação.

* \+ :: NUM NUM \=\> NUM (Soma)  
* \- :: NUM NUM \=\> NUM (Subtração)  
* \* :: NUM NUM \=\> NUM (Multiplicação, suporta produto de tensores)  
* / :: NUM NonZero \=\> Float (Divisão exata. Requer que o divisor seja estritamente não-zero via Tipo Refinado, eliminando a necessidade de Result).  
* // :: NUM NonZero \=\> Int (Divisão inteira. Requer Tipo Refinado).  
* div :: NUM NUM \=\> Result::(NUM, Err) (Divisão segura em tempo de execução. Valida o divisor dinamicamente e retorna Ok NUM ou Err "Divisão por zero").  
* \*\* :: NUM NUM \=\> NUM (Exponenciação)  
* mod :: Int NonZero \=\> Int (Módulo. Requer Tipo Refinado no divisor).  
* abs :: NUM \=\> NUM (Valor absoluto)  
* sin, cos, tan :: Float \=\> Float (Trigonometria)  
* sqrt :: Float \=\> Float (Raiz quadrada)  
* \= :: EQ EQ \=\> Bool (Comparações de igualdade estrutural profunda)  
* \>, \<, \>=, \<= :: ORD ORD \=\> Bool (Comparações de ordenação)  
* and, or :: Bool Bool \=\> Bool (Operadores lógicos booleanos. Possuem garantia estrita de avaliação em curto-circuito, omitindo o processamento do ramo desnecessário).

#### **1.2. Álgebra Linear (Interface Tensor)**

* dot :: Tensor Tensor \=\> Tensor (Produto Escalar. Requer DOT\_BEHAVIOR).  
* shape :: Tensor \=\> Tuple (Retorna a tupla de dimensões de um tensor).  
* scalar :: Tensor::T::() \=\> T (Recebe um Tensor 0-D e retorna um escalar NUM).

#### **1.3. Manipulação de Coleções e Fluxos (ITERABLE)**

* map :: (A \-\> B) Iterable::A \=\> Iterable::B (Aplica a função a cada elemento).  
* filter :: (A \-\> Bool) Iterable::A \=\> Iterable::A (Filtra elementos via predicado).  
* fold :: (B A \-\> B) B Iterable::A \=\> B (Acumulação de valores com seed inicial)  
* reduce :: (A A \-\> A) Iterable::A \=\> Result::(A, Err) (Acumulação de valores).  
* zip :: Iterable::A Iterable::B \=\> Iterable::(A B) (Agrupa iteráveis em tuplos).  
* head :: List::T \=\> Optional::T (Retorna o primeiro elemento de uma lista persistente).  
* tail :: List::T \=\> Optional::List::T (Retorna a lista sem o primeiro elemento).  
* at :: (Coordenadas) Colecao \=\> Result::(T, Err) (Extração posicional segura com validação de limites *Out-of-Bounds* que retorna Result).

#### **1.4. Manipulação de Dicionários e Conjuntos**

* get :: Dict::(K, V) K \=\> Optional::V (Extração segura de valor por chave).  
* insert :: Dict::(K, V) K V \=\> Dict::(K, V) (Retorna um novo dicionário contendo a nova chave-valor).  
* has :: Set::T T \=\> Bool (Verifica a existência de um elemento no conjunto. Também aplicável a chaves de Dicionários).  
* union :: Set::T Set::T \=\> Set::T (Álgebra de conjuntos: união de todos os elementos únicos).  
* intersect :: Set::T Set::T \=\> Set::T (Álgebra de conjuntos: interseção dos elementos presentes em ambos).  
* diff :: Set::T Set::T \=\> Set::T (Álgebra de conjuntos: diferença, elementos do primeiro conjunto que não estão no segundo).

#### **1.5. Utilitários para Tipos de Soma (Optional e Result)**

* unwrap\_or :: Result::(T, E) T \=\> T (Desempacota o valor Ok ou retorna o valor padrão fornecido caso encontre Err. Função pura, sem efeitos secundários).  
* unwrap\_or :: Optional::T T \=\> T (Desempacota o valor Some ou retorna o valor padrão fornecido caso encontre None).

#### **1.6. Introspecção e Formatação**

* str :: SHOW \=\> Text (Stringificação de qualquer tipo que implemente a interface SHOW).  
* format :: Text Args... \=\> Text (Template Funcional para interpolação explícita de strings).  
* type :: T \=\> Text (Retorna a representação estrita do tipo do objeto em runtime).  
* fields :: T \=\> List::Text (Retorna os campos/chaves de uma estrutura de dados em runtime).  
* int, float :: NUM \=\> NUM (Cast de precisão nativa).

### **2\. Actions (Domínio Impuro)**

As *Actions* interagem com o *Runtime* Kata, o Sistema Operacional e orquestram a simultaneidade. Não podem ser chamadas a partir de Funções.

#### **2.1. Concorrência e Escalonamento (Modelo CSP)**

* fork\! :: Action \=\> () (Submete uma Action ao escalonador para execução assíncrona. Se anotada com @parallel, inicia processo de SO).  
* channel\! :: () \=\> (Sender::T Receiver::T) (Cria um canal síncrono *Rendezvous* bloqueante).  
* queue\! :: Int \=\> (Sender::T Receiver::T) (Cria um canal assíncrono com buffer de tamanho fixo).  
* broadcast\! :: () \=\> (Sender::T Subscribe::Function) (Cria canal 1-para-N. O recetor invoca a função *Subscribe* para obter a sua fila).  
* \!\> :: Sender::T T \=\> () (Operador direcional de envio. Transfere a propriedade do dado para o canal).  
* \<\! :: Receiver::T \=\> T (Operador direcional de receção. Bloqueia a *Green Thread* até os dados estarem disponíveis).  
* \<\!? :: Receiver::T \=\> Optional::T (Tentativa não-bloqueante de leitura de canal).  
* select :: (Estrutura de bloco multiplexadora para aguardar em múltiplos canais simultaneamente).  
* timeout :: Int \=\> () (Ramo especial do select para desbloqueio por inatividade em milissegundos).

#### **2.2. Entrada e Saída (I/O)**

* echo\! :: Text \=\> () (Escrita padrão na consola).  
* input\! :: Text \=\> Text (Imprime a mensagem fornecida e aguarda entrada do utilizador).  
* log\! :: Text Nivel \=\> () (Registo no sistema de logs estruturado da linguagem).  
* read\! :: Text \=\> Array::Byte (Lê ficheiros em memória ou como fluxo iterável).  
* read\_text\! :: Text \=\> Text (Lê ficheiros em memória ou como fluxo iterável).  
* write\! :: Text A \=\> () (Grava dados em recursos do sistema).

#### **2.3. Controle de Sistema e Interrupção**

* panic\! :: Text \=\> () (Aborta a execução atual imediatamente gerando log e rastreamento. Usado para estados de violação fatal ou exceções não recuperáveis).  
* unwrap\_or\_panic\! :: Result::(T, E) Text \=\> T (Açúcar sintático imperativo para extração em falhas críticas. Desempacota o valor Ok / Some. Se encontrar Err / None, aborta a execução automaticamente com panic\! invocando a mensagem fornecida).  
* assert\! :: Bool Text \=\> () (Avalia uma expressão booleana. Se for False, invoca um panic\! com a mensagem fornecida. Completamente extirpado em compilações de produção via *Tree-Shaking*).  
* sleep\! :: Int \=\> () (Pausa a execução da *Action* atual pelos milissegundos informados, libertando o escalonador).  
* exit\! :: Int \=\> Void (Encerra o processo do programa integralmente retornando um código de status para o SO).  
* now\! :: () \=\> Int (Retorna o carimbo de data/hora atual do sistema \- *Timestamp*).

## **Especificação do Modelo de Execução e Memória**

Este documento estabelece as regras de tempo de execução (runtime), o modelo de concorrência e a topologia de memória da Kata-Lang. O design é otimizado para concorrência massiva de I/O sem penalizar o processamento matemático puro.

### **1\. Paradigma de Avaliação (Eager Evaluation)**

A Kata-Lang utiliza avaliação **Estrita (Eager)** em tempo de execução.

Os argumentos de uma função são avaliados integralmente antes de a execução entrar no corpo da função. O conceito de "Grafo Acíclico Dirigido (DAG)" referido na especificação não existe como estrutura de dados em runtime (não há alocação de *Thunks*). O DAG é estritamente uma Representação Intermediária (IR) em tempo de compilação, utilizada para aplicar *Stream Fusion* (fundir loops adjacentes) e *Constant Folding*.

### **2\. Modelo de Concorrência (M:N via Tokio/Runtime)**

A linguagem suporta concorrência assíncrona de uso geral através de um escalonador cooperativo M:N (*Green Threads*), integrado no runtime embutido.

#### **2.1 Separação Arquitetural de Compilação**

A compilação do código divide-se em dois domínios gerados pelo backend (Cranelift/LLVM):

* **Functions (Processamento de CPU):** Sendo puras e desprovidas de I/O, as funções são geradas como código de máquina linear síncrono. Elas operam na pilha (stack) nativa da *thread* do Sistema Operativo. O escalonador não as interrompe no meio da sua execução lógica.  
* **Actions (I/O e Efeitos):** Sendo o domínio impuro, o corpo de uma Action é compilado como uma Máquina de Estado (uma estrutura que encapsula o estado das variáveis locais). Sempre que uma Action invoca um canal ou uma operação de sistema bloqueante, a máquina de estado cede o controlo (yield) ao escalonador, que aloca a CPU para outra *Green Thread*.

Esta separação garante que o custo de abstração do I/O assíncrono não contamina a performance de algoritmos matemáticos ou de transformação de dados.

### **3\. Topologia de Memória, Ciclo de Vida e Ownership**

A Kata-Lang não possui *Garbage Collector* tradicional de varredura (Tracing GC), nem exige um *Borrow Checker* restritivo. A gestão de posse (Ownership) e de tempo de vida é resolvida organicamente pelas regras de imutabilidade de Data e pelo escopo das Arenas.

#### **3.1 Ownership Local (Arenas Zero-Cost)**

* **Propriedade:** Cada Action instanciada é a proprietária exclusiva de uma *Arena* (um *bump allocator* de bloco de memória contígua).  
* **Ausência de Aliasing Mutável:** Como os dados são estritamente imutáveis, o compilador autoriza múltiplas referências (aliasing) para o mesmo endereço de memória dentro da Arena sem risco de concorrência. A palavra-chave var na Action apenas muta a referência na *Stack*, nunca o dado na Arena.  
* **Ciclo de Vida:** Todas as alocações locais são afixadas na Arena e inicializadas com um cabeçalho de metadados reservado para promoções. Não há limpeza individual de objetos. Quando a Action conclui, a Arena inteira é libertada em tempo O(1), garantindo que nenhuma referência sobrevive ao dono original.

#### **3.2 Ownership Partilhado e Análise de Fuga (Escape Analysis)**

A linguagem utiliza uma abordagem híbrida guiada pelo compilador (Escape Analysis) para evitar o custo de cópias profundas em tempo de execução (Zero-Copy) ao enviar dados por canais, sem penalizar a performance de dados estritamente locais.

* **Análise Estática (Escape Analysis):** Durante a compilação, o otimizador rastreia o ciclo de vida de cada dado. Se um dado (ou parte dele) for enviado por um canal (usando o operador `!>`), o compilador determina estaticamente que esse dado "foge" (escapes) do escopo local da *Action*.
* **Alocação Direta na Heap Global (ARC):** Estruturas que sofrem fuga são alocadas **diretamente na Heap Global** partilhada desde o seu nascimento, encapsuladas num bloco ARC (Atomic Reference Counting).
* **Envio O(1) (Zero-Copy):** Quando o dado é enviado pelo canal, o *runtime* não precisa realizar verificações de cabeçalho ou cópias. Ele apenas incrementa o contador atómico e transfere o ponteiro (8 bytes). Não há cópia de memória em runtime, independentemente do tamanho e profundidade da estrutura.
* **Isolamento Local (Arenas):** Se o dado nunca for enviado para um canal (o que representa a esmagadora maioria dos casos em cálculos puros), ele é alocado na Arena local, garantindo máxima performance de cache (*Cache Locality*) e libertação em O(1) sem nenhum *overhead* atómico.

Esta arquitetura delega a responsabilidade de roteamento de memória para o compilador, isolando o custo da sincronização atómica estritamente ao I/O de mensagens, evitando duplicações na memória partilhada e resolvendo o ownership sem intervenção manual do programador. **A clonagem explícita profunda (*Deep Copy*) é inexistente e desnecessária na linguagem**, uma vez que a pureza dos dados garante a segurança da partilha estrutural sob este modelo por ponteiros.

### **4\. O Runtime da Kata-Lang e Geração de Binário**

O binário final gerado é um executável estático autossuficiente (Standalone) que encapsula o código gerado pelo Cranelift e o pacote do Runtime.

#### **4.1 Eliminação de Código Morto (Tree-Shaking Perfeito)**

A Kata-Lang proíbe a reflexão (Reflection) e a invocação dinâmica baseada em strings em tempo de execução. Esta restrição permite que o compilador realize uma eliminação de código morto matemática e determinística.

Durante a Otimização da AST, o compilador constrói um Grafo de Chamadas (Call Graph) partindo exclusivamente do ponto de entrada do programa (a Action principal, usualmente main).

* Qualquer função, bloco de dados, contrato de interface ou módulo externo que não faça parte da árvore de dependências diretas ou indiretas deste grafo é extirpado.  
* O Cranelift nunca recebe instruções para gerar IR ou Assembly de código inalcançável.  
* Isto garante que as bibliotecas padrão pesadas não penalizam o tamanho do executável, compensando o espaço ocupado pelo escalonador embutido.

#### **4.2 Responsabilidades da Camada do Runtime**

O Runtime é uma camada compacta acoplada ao binário responsável por:

* Inicializar o *pool* de threads nativas do Sistema Operativo e arrancar o reator de I/O assíncrono (Work-Stealing Scheduler).  
* Gerir as estruturas atómicas dos Canais de Comunicação (CSP).  
* Administrar a Heap Global, gerir os contadores ARC e libertar os ponteiros órfãos.  
* Fornecer os adaptadores FFI (Foreign Function Interface) entre o Assembly gerado pelo Cranelift e as Syscalls do Sistema Operativo.

## **Especificação Léxica e Parsing**

O design prioriza a simplicidade de *tokenização*, eliminando ambiguidades sintáticas e minimizando o *lookahead* (leitura antecipada) do compilador.

### **1\. Convenções Estritas de Nomenclatura**

A Kata-Lang impõe padrões de nomenclatura estritos não por estilização, mas porque o *Lexer/Parser* depende da capitalização para desambiguação e construção correta da AST, dispensando anotações redundantes.

| Padrão | Categoria | Uso Principal |
| :---- | :---- | :---- |
| ALL\_CAPS | **Interfaces** | Contratos universais e super-traits (ex: NUM, ORD, EQ, SHOW, HASH). |
| CamelCase | **Tipos (Data)** | Estruturas de dados concretas, parâmetros genéricos, tipos nativos (ex: Int, List, Complexo, T). |
| snake\_case | **Comportamentos** | Funções (soma\_valores), Actions (main\!, echo\!), variáveis de escopo (meu\_valor). |

A violação desta regra de capitalização constitui um erro fatal e imediato de compilação durante a fase de análise léxica.

### **2\. A Palavra-chave Lambda (lambda ou λ)**

Para definições de funções anônimas (ou corpos de funções nomeadas), o compilador reconhece as palavras-chave lambda e o caractere unicode λ como sendo perfeitamente intercambiáveis e idempotentes.

### **3\. Aridade, Funções Variádicas e Aplicação Parcial**

A linguagem faz uma distinção estrita no *parsing* baseada no domínio de execução (Functions vs. Actions) e impõe regras rígidas de consumo de argumentos.

#### **3.1 Funções (Aridade Fixa Estrita)**

* **Regra:** Todas as Functions (puras) possuem aridade fixa e conhecida estaticamente. Funções variádicas são expressamente proibidas no domínio funcional.  
* **Mecânica do Parser:** Como a aridade é rígida, a notação prefixa pura (ex: \+ \* 2 3 4) é resolvida de forma determinística em tempo O(N). O compilador sabe exatamente quantos argumentos empilhar na AST para cada função, dispensando delimitadores.

#### **3.2 Currying Explícito (O Operador Hole \_)**

O operador *Hole* (\_) permite realizar aplicação parcial de uma função, transformando a chamada numa *closure* (Lambda) que aguarda a injeção dos argumentos faltantes. O uso do \_ é independente do operador de aplicação `$`.

* *Sintaxe Greedy:* let f \+ 1 \_  
* *Sintaxe Explícita:* let f $(+ 1 \_)
* *AST Tipada :* Lambda(Arity: 1, Closure: \[1\])

#### **3.3 Actions (Marcador Imperativo e Tuplas Obrigatórias)**

* **Regra:** Todas as chamadas a Actions devem ser explicitamente sufixadas com um ponto de exclamação \! (ex: echo\!, channel\!). Ao contrário das funções, as Actions **consomem obrigatoriamente uma única tupla** como argumento.
* **Mecânica do Parser:** Diferente do modelo *greedy* das funções, uma Action exige delimitadores explícitos (parênteses) que englobam todos os seus argumentos, formando uma tupla.
  * *Válido:* echo\!("Erro no processamento" id\_utilizador data)
  * *Válido:* echo\!($(fizzbuzz 3)) *(tupla com um único elemento que resulta de aplicação explícita)*
  * *Inválido:* echo\! "Erro no processamento" id\_utilizador data *(Erro de compilação: Tupla esperada)*
* **Variadismo Fortemente Tipado:** Actions podem ser variádicas definindo seus tipos como sequências utilizando reticências `...`. O variadismo é expresso puramente no nível do *Type Checker* através da tupla recebida.
  * *Exemplo de Assinatura:* action echo(SHOW...) *(A Action aceita uma tupla com zero ou múltiplos argumentos, onde cada um deve implementar SHOW)*.
  * *Exemplo de Tipagem Mista:* action log(LogLevel Text...) *(Aceita uma tupla onde o primeiro elemento é um LogLevel e os demais são Text).*

### **4\. Strings e Templates (Dados Cegos)**

O Lexer da Kata-Lang não realiza interpretação de código embutido dentro de blocos de texto.

* **Regra:** *Strings* (textos delimitados por aspas duplas "...") são "dados cegos" absolutos.  
* **Mecânica do Lexer:** Não existe modo de interpolação léxica (como "$var" ou "${expr}"). Ao encontrar uma aspa, o Lexer consome todos os caracteres como texto plano até à aspa de fecho.  
* **Resolução:** A injeção de variáveis em textos é delegada para o domínio de execução funcional através de funções de formatação e *Templates* explícitos (ex: format "Olá {}" (nome)).

### **5\. A Teoria Unificada das Tuplas e Parênteses**

A Kata-Lang adota uma distinção sintática clara entre tuplas e aplicação de funções. O token (...) cria invariavelmente um nó Tuple na AST. A aplicação de função ocorre de duas formas: **greedy** (sem parênteses) ou **explícita** com o operador `$`.

#### **5.1 Delimitadores Internos**

Dentro de parênteses, os itens podem ser separados por espaços ou vírgulas. Para o parser, são tokens equivalentes. (1 2 3\) produz a mesma AST que (1, 2, 3\).

#### **5.2 Regras de Avaliação**

1. **Aplicação Greedy (sem parênteses):** Uma função seguida de argumentos sem parênteses é aplicada automaticamente.
2. **Tuplas (com parênteses, sem $):** Parênteses sempre criam tuplas, nunca invocam funções. Tuplas de um único elemento preservam a sua topologia: `(42)` é uma Tupla(42), não o inteiro 42.
3. **Aplicação Explícita com $:** O operador `$` força a avaliação do conteúdo entre parênteses como aplicação de função, resolvendo ambiguidades onde um tuplo literal seria esperado.
4. **Currying Explícito com Hole:** O operador `_` pode ser usado em qualquer um dos modos acima para criar aplicações parciais.

#### **5.3 Tuplas Implícitas em Assinaturas e Tipos de Funções**

Na definição de assinaturas, os delimitadores de tipo dispensam o uso de parênteses para agrupar os argumentos. O *Parser* estabelece uma distinção léxica rígida para desambiguar a **declaração de uma assinatura** do **tipo literal de uma função**:

* **O operador \=\> (Declaração):** É utilizado exclusivamente no escopo léxico para declarar a assinatura estrita de uma função no momento da sua definição. O *Lexer* entende que a sequência à esquerda são os argumentos e à direita é o retorno.  
  * *Sintaxe:* soma :: Int Int \=\> Int  
* **O operador \-\> (Tipo de Função):** É utilizado quando o programador precisa descrever a assinatura de um *Lambda* como um tipo de dados de primeira classe (por exemplo, ao definir o tipo de um parâmetro que recebe uma função, ou no retorno). Exige encapsulamento para evitar ambiguidades no tipo.  
  * *Sintaxe:* map :: (A \-\> B) Iterable::A \=\> Iterable::B *(A função map recebe como primeiro argumento um tipo invocável (A \-\> B)).*

### **6\. Escopo e Terminadores (Significant Whitespace)**

#### **6.1 Terminador de Instrução**

O caractere de quebra de linha \\n é o terminador oficial. O Parser encerra o ramo atual da AST ao encontrar uma quebra de linha (exceto se a expressão estiver aberta dentro de um nó de fechamento pendente (...) ou \[...\]).

#### **6.2 Escopo por Indentação**

* **Regra:** Blocos de escopo são definidos pelo nível de indentação em relação à linha de declaração.  
* **Uniformidade:** O uso de espaços ou *tabs* (\\t) é válido, contanto que seja estritamente uniforme dentro daquele bloco específico. A mistura resulta num erro léxico abortivo.  
* **Mecânica do Lexer:** O Lexer rastreia a indentação no início de cada linha, emitindo tokens sintéticos INDENT (abertura de escopo) e DEDENT (fecho de escopo).

## **Sistema de Tipos e Interfaces**

Regras do sistema de tipos da Kata-Lang, focando na separação estrita entre dados e código executável, o mecanismo de Early Checking para funções genéricas, a manipulação puramente funcional de Tipos-Refinados e a hierarquia de interfaces.

### **1\. Separação Estrita: Tipos, Dados e Funções**

No motor do compilador (TypeEnv), toda entidade manipulável possui um **Tipo** (ex: Int, List::T ou \[T\], (A \-\> B)). Funções são cidadãs de primeira classe e podem ser passadas como argumentos.

No entanto, a categoria **Dado (Data)** é um subconjunto estritamente restrito:

* **Dados (Data):** São inertes, imutáveis, livres de comportamento e **nativamente serializáveis**. Englobam primitivos, coleções persistentes, arrays, tensores e uniões (ADTs).  
* **Funções (Lambdas):** *Não* são Dados. Lambdas formam *closures* (capturam ambiente imutável e possuem ponteiros de execução de máquina). Por estarem atreladas ao *layout* de memória do binário compilado, não possuem portabilidade universal:  
  * **IPC Local (@parallel):** O compilador *permite* o envio de Lambdas entre processos locais do mesmo binário. Isto é resolvido internamente de forma opaca (via tabelas de despacho estáticas), sem necessidade de serialização real.  
  * **I/O de Rede e Persistência:** O compilador *rejeitará* a tentativa de tratar um Lambda como um "Dado" para ser exportado (ex: JSON sobre TCP). O envio de código pela rede (processamento distribuído) ou execução dinâmica (REPL) exige a serialização da Representação Intermediária (IR) e a utilização de um compilador JIT no destino, mecanismos que transcendem o contrato de um Dado inerte.

### **2\. Interfaces e Despacho Múltiplo (Multiple Dispatch)**

A linguagem não possui métodos atrelados a classes. O polimorfismo *ad-hoc* ocorre através de Interfaces e implementações de Despacho Múltiplo.

#### **2.1 Definição de Interfaces (Contratos)**

A declaração de uma interface estabelece um contrato rigoroso de aridade e tipagem. O motor de *Early Checking* exige estas definições para validar funções genéricas antes do tempo de execução.

Utiliza-se a palavra-chave interface seguida do nome (em ALL\_CAPS) e um bloco indentado contendo as declarações de assinatura obrigatórias. O próprio nome da interface atua como o tipo alvo no contrato.

\# A interface define que, para a implementar, o tipo  
\# deve fornecer todas estas funções com a tipagem estrita abaixo:  
interface NUM implements ORD EQ  
    \+ :: NUM NUM \=\> NUM  
    \- :: NUM NUM \=\> NUM  
    \* :: NUM NUM \=\> NUM  
    abs :: NUM \=\> NUM  
    div :: NUM NUM \=\> Result::(NUM, Err)

#### **2.2 Declaração de Contratos Planos (Implementação)**

Tipos de dados estruturais não sabem quais comportamentos possuem no momento da sua definição estrutural. A implementação do contrato é firmada no **nível superior do módulo (top-level)**, de forma isolada.

Isto significa que as rotas de despacho ficam registadas no TypeEnv do ficheiro atual. Quando um tipo é exportado e importado por outro módulo, as implementações de interface atreladas a ele viajam em conjunto, garantindo a interoperabilidade sem poluir o escopo global.

\# Definição estrutural pura    
data Vec2 (x y)

\# Implementação do contrato no top-level do módulo    
Vec2 implements ADD\_BEHAVIOR    
    \# Assinaturas explícitas definem as rotas do despacho    
    \+ :: Vec2 Float \=\> Vec2    
    lambda v f: ...

    \+ :: Vec2 Vec2 \=\> Vec2    
    lambda v1 v2: ...

#### **2.3 Herança de Interfaces (Super-Traits)**

As Interfaces em Kata-Lang suportam herança e restrições compostas na sua definição, separadas por espaços (como visto na definição de NUM acima). O uso de Super-Traits minimiza código redundante e permite que o Type Checker deduza propriedades em cascata. O caso principal é a família de interfaces numéricas.

* **Declaração de Super-Trait:** Quando se declara interface NUM implements ORD EQ, a linguagem estabelece que, para que um tipo consiga implementar NUM, ele é **obrigado** a implementar todos os métodos de ORD e EQ primeiro.  
* **Isolamento de Domínio (A Regra Cruzada):** A herança propaga características, mas não oblitera domínios de verificação. O facto de Int e Text implementarem (direta ou indiretamente) ORD não implica interoperabilidade livre entre si (ex: testar \>, \< entre uma String e um Número). As implementações de ORD são resolvidas de acordo com a compatibilidade tipificada explicitamente, preservando a segurança.

### **3\. Generics e Early Checking**

A Kata-Lang adota o modelo de **Early Checking** para funções genéricas. O compilador prova matematicamente a validade de uma função genérica no momento de sua definição, sem depender de "hints" do momento em que a função for chamada (*Late Checking*).

Se o compilador não conseguir unificar os tipos internamente, ele exigirá que o programador assine um contrato explícito no bloco with.

\# Função genérica com Early Checking    
soma\_generica :: A B \=\> C    
lambda x y: + x y    
with     
    \# A restrição obriga a existência prévia desta assinatura no TypeEnv    
    \+ :: A B \=\> C

#### **3.1 Monomorfização (Zero-Cost Abstractions)**

O compilador utiliza o processo de **Monomorfização** para instanciar funções genéricas.

* **Mecânica:** Para cada combinação única de tipos concretos que invoca uma função genérica no código, o compilador gera uma cópia física especializada (código de máquina nativo) dessa função (ex: uma soma\_generica gera uma versão soma\_Int\_Int e uma versão soma\_Float\_Float no assembly).  
* **Vantagem:** O acesso é estático, eliminando completamente o *overhead* de despacho dinâmico (v-tables, *boxing/unboxing*) em tempo de execução. O uso de Generics em Kata é tão rápido quanto escrever as tipagens de forma manual e redundante.

### **4\. Tipos-Refinados e Smart Constructors**

Tipos-Refinados aplicam restrições matemáticas e lógicas sobre um tipo base usando tuplas e o operador Hole (\_).

Estados inválidos são irrepresentáveis.

#### **4.1 Sintaxe e Predicados**

\# Restrição numérica padrão  
data PositiveInt as (Int, \> \_ 0\)

\# Restrição por Exclusão (Except) operando como Predicado nativo  
data Real as (NUM, except Complex)

\# Encadeamento de múltiplos predicados e exclusões  
data RealPositivo as (NUM, except Complex, except Fraction, \> \_ 0\)

**O Predicado except:** O except não altera a mecânica estrutural do motor de tipos (não introduz "tipagem negativa" no unificador Hindley-Milner). Ele atua estritamente como uma função de ordem superior (*Higher-Order Function*) avaliada em tempo de execução ou de compilação. Quando recebe um tipo genérico ou interface (como Complex), o except devolve uma *Closure* de validação lógica (T \-\> Bool). Assim, a sintaxe (NUM, except Complex) é processada nativamente como uma lista de funções de validação dentro da tupla do Tipo Refinado, dispensando o uso do operador Hole (\_). Se o tipo passado coincidir com o tipo excluído, o predicado falha. O compilador gera automaticamente um *Smart Constructor* para o tipo-refinado.

* **O tipo de erro em Result:** Tipos-refinados e operações falíveis retornam `Result::(T, E)`. A variante de falha `Err(E)` costuma carregar um `Text` com a descrição do erro, mas pode carregar qualquer tipo `E` definido na assinatura.

#### **4.2 Preservação e Composição de Predicados (Predicate Inheritance)**

Na Kata-Lang, as operações matemáticas entre Tipos Refinados (ou entre um Tipo Refinado e sua Base) não descartam as restrições associadas. O sistema de tipos aplica o princípio da **Herança Automática de Predicados**.

*   **Interseção Lógica de Predicados:** Sempre que uma função numérica é invocada e não existe uma sobrecarga manual específica para a combinação exata de tipos, o compilador utiliza a implementação da Base (ex: `Int`), mas o tipo de retorno herda a **composição lógica (AND)** de todos os predicados presentes nos argumentos.
    *   *Exemplo:* `PositiveInt` (> _ 0) + `NonZeroInt` (!= _ 0) resulta em um tipo que deve satisfazer ambos os predicados: `Result::((Int, > _ 0, != _ 0), Text)`.
*   **Prova Dinâmica via Result:** Como o resultado de uma operação pode violar as restrições herdadas (ex: `1 - 10`), o compilador envolve o retorno automaticamente em `Result::(T, Text)`. O Runtime revalida a árvore de predicados acumulada sobre o novo valor antes de entregar o `Ok`.
*   **Sobrecarga Pura (Supressão de Result):** Este comportamento automático é suprimido se o desenvolvedor fornecer uma **implementação explícita** para a combinação de tipos (ex: provar matematicamente que a soma de dois `PositiveInt` é sempre um `PositiveInt`). Nestes casos, o compilador respeita a assinatura manual e retorna o tipo puro.
*   **Otimização @comptime:** Se os argumentos forem literais constantes conhecidos em tempo de compilação, o compilador executa a interseção de predicados estaticamente. Se o valor satisfizer todas as restrições, o `Result` é eliminado e o valor puro é embutido no binário.

#### **Exemplo: Composição Automática e Sobrecarga Pura**

```kata
# 1. Composição Automática (Interseção de Predicados)
# x será Result::((Int, > _ 0, != _ 0), Text)
let x (+ 10_PositiveInt 5_NonZeroInt) 

# 2. Sobrecarga Pura (Manual)
PositiveInt implements ADD_BEHAVIOR
    + :: PositiveInt PositiveInt => PositiveInt
    lambda a b:
        # O desenvolvedor garante o retorno do tipo puro
        PositiveInt $(+ a b) |> $(
            lambda Ok valor: valor
            lambda Err _: 1 # Fallback seguro
        )
```

#### **4.3 Fallbacks Literais Estáticos**


Literais numéricos constantes inseridos no código-fonte são avaliados em tempo de compilação. Se o literal passar no predicado (ex: o literal 1 num contexto que exige PositiveInt), o compilador aceita-o como um tipo nativo PositiveInt, dispensando o construtor dinâmico e o retorno de Result.

### **5\. Tratamento de Erros e Padrões Funcionais Puros**

Para manter a pureza da Mônada Funcional, a construção imperativa match é **proibida dentro de lambdas**. Para desestruturar e reagir a um Result gerado por um tipo-refinado, a linguagem exige a composição com lambdas de Pattern-Matching.

#### **Exemplo: Sobrecarga Pura com Resolução de Result**

Vec2 implements ADD\_BEHAVIOR
    \# Prometemos que a saída será sempre PositiveInt (sem Result)
    \+ :: PositiveInt Int \=\> PositiveInt
    lambda p i:
        \# 1\. A soma $(- p i) degrada para Int.
        \# 2\. O construtor 'PositiveInt' reavalia e retorna Result.
        \# 3\. O Pipe despacha para os lambdas de resolução de padrão.
        PositiveInt $(+ p i) |\> $(
            lambda Ok valor\_puro: valor\_puro     \# Ramo de Sucesso
            lambda Err erro: 1                   \# Ramo de Falha
        )
    \# O fallback deve respeitar o tipo de retorno.
    \# '1' é aceito estaticamente pelo compilador como PositiveInt.  

Se o programador não quiser fornecer um fallback (como no caso acima), a linguagem obriga a alterar a assinatura principal da função para propagar o Result, delegando o tratamento imperativo da falha (como emissão de panic\!) para as Actions.

## **Controle de Fluxo: Funcional e Imperativo**

A linguagem Kata elimina estruturas de controlo ambíguas, dividindo as ferramentas de fluxo estritamente pelos domínios em que operam: Funcional (puro, declarativo) e Imperativo (impuro, baseado em estado).

### **1\. Controle de Fluxo Funcional (Domínio Puro)**

No domínio das **Functions**, não existem laços de repetição imperativos (for, while) nem a palavra-chave if. O fluxo é direcionado unicamente por casamento de padrões, guardas lógicas e recursão otimizada.

#### **1.1. Pattern Matching (Despacho de Lambda)**

O controlo de fluxo primário é feito pela assinatura do lambda. Uma função pode ser composta por múltiplas definições de lambda. O compilador avaliará os argumentos de cima para baixo e executará o primeiro corpo cujo padrão estrutural (*Pattern*) corresponda à entrada.

O `otherwise:` pode ser usado como uma definição única de catch-all para o corpo da função quando não há necessidade de desvios condicionais internos (Guards).

otherwise\_true :: A \=\> Bool
otherwise: True

fibonacci :: Int \=\> Int    
lambda 0: 0             \# Match exato literal    
lambda 1: 1    
lambda n: + fibonacci $(- n 1) fibonacci $(- n 2)

#### **1.2. Desestruturação e Omissão de Valores em Lambdas**

O *Pattern Matching* também atua como mecanismo primário de desestruturação de tuplos e estruturas nos argumentos. Não existem "Holes indexados" (como \_:2) na linguagem; para extrair ou ignorar valores específicos de coleções aninhadas durante um pipeline, utiliza-se a desestruturação posicional no cabeçalho do lambda, omitindo a variável indesejada no corpo:

\# Somando apenas o 1º, 2º e 4º elementos de tuplos de tamanho 4
let resultados $(lista\_tuplos |\> map $(lambda (a b c d): sum a b d))

*(O compilador identifica que c não é utilizado e o elimina estaticamente como código morto na otimização da Representação Intermediária).*

#### **1.3. Guards (Condicionais Puras) e otherwise**

Para desvios lógicos que não podem ser resolvidos por *Pattern Matching* estrutural, utilizam-se os *Guards*. Eles substituem as cadeias de if/else, operando como testes booleanos sucessivos (avaliados de cima para baixo). O sufixo `:` separa o identificador do resultado.

A linguagem requer o uso explícito da cláusula **otherwise** como *fallback* mandatário caso as condições acima não sejam satisfeitas, garantindo que o ramo não fique pendente.

**Sintaxe com Bloco `with`:** Os Guards em Kata-Lang utilizam um bloco `with` para definir as condições de forma nomeada. Esta abordagem permite avaliação lazy das condições e melhor legibilidade. Cada condição é definida com a sintaxe `identificador: resultado`, e o bloco `with` declara as expressões condicionais.

**Curto-Circuito Garantido:** O compilador assegura que a avaliação dos *Guards* obedece a um comportamento de curto-circuito (*Short-Circuit*). Isto significa que o processamento do bloco falso é omitido no *runtime*, protegendo a execução de potenciais avaliações que causariam *panic\!*.

max :: Int Int \=\> Int
lambda x y:
    maior: x
    menor: y
    otherwise: y
    with
        maior as > x y
        menor as < x y

#### **1.4. O Escopo Local: let vs with**

A linguagem provê dois mecanismos estritos para amarração de nomes (*bindings*) no escopo funcional, operando em direções opostas de avaliação:

* **let (Avaliação Top-Down):** Define uma variável imutável no fluxo léxico antes de seu uso. Utilizado para cálculos sequenciais no corpo do lambda. A Kata-Lang também suporta a desestruturação posicional direta utilizando a palavra reservada **as**.  
  1. Exemplo de Desestruturação com Aliasing: let (a, b) as tuple (extrai os elementos de tuple para os identificadores independentes a e b imediatamente).  
* **with (Avaliação Bottom-Up / Metadados):** Uma cláusula declarativa fixada no final do lambda. Possui duas finalidades exclusivas:  
  1. Resolver computações requeridas pelos *Guards* antes que as condições sejam testadas.  
  2. Anexar restrições de Tipos Genéricos (Contratos de Interface).

processar :: T \=\> Int    
lambda entrada:    
    \# 'let' com desestruturação para computação sequencial    
    let (identificador, carga) as (extrair\_tupla entrada)  
        
    \# Guards consumindo as variáveis declaradas no 'with'    
    \> variante 10: \+ base variante    
    otherwise: base    
        
    \# 'with' definindo as dependências lógicas e o Tipo Genérico    
    with     
        base as calcular\_base entrada  
        variante as extrair\_variante carga    
        T as (T implements ORD)

### **2\. Controle de Fluxo Imperativo (Actions)**

O domínio das **Actions** é projetado para interação com o Sistema Operacional e concorrência. Aqui, a recursividade é banida e a desestruturação segura é imposta.

#### **2.1. A Proibição de Recursão (Hard Error)**

As *Actions* operam sobre um escalonador de *Green Threads* em tempo de execução (compilado para máquinas de estado sob o *Cranelift/Tokio*). **É expressamente proibido realizar chamadas recursivas dentro de uma Action.** Como o escalonador não pode garantir *Tail Call Optimization* (TCO) para esse contexto impuro, qualquer detecção de ciclo recursivo pelo *Type Checker* resultará em um **Erro Fatal de Compilação**, prevenindo *Stack Overflows*.

#### **2.2. Laços de Repetição (loop e for)**

Para substituir a recursão em operações de I/O, as *Actions* utilizam primitivas iterativas clássicas com as chaves de controlo break (interrompe o laço) e continue (avança a iteração).

* **loop**: Um laço infinito fundamental.
* **for elemento in colecao**: Um iterador seguro que consome estruturas que implementam a interface nativa ITERABLE (Listas, Arrays, Ranges). A sintaxe utiliza a palavra-chave `in` para separar a variável de iteração da coleção, mantendo clareza semântica.

Abaixo, um exemplo da utilização do loop coordenando estado mutável (var), saltos de iteração (continue) e condição de saída (break):

action conectar\_servidor    
    var tentativas 0    
        
    loop    
        var tentativas (+ tentativas 1\)    
            
        \# Limite de segurança para evitar loops infinitos acidentais    
        match (\> tentativas 5\)    
            True:    
                echo\!("Limite de tentativas excedido. Abortando.")    
                break    
            False:    
                () \# Continua a execução normalmente    
            
        let pronto ping\!()    
            
        match pronto    
            False:    
                echo\!("Servidor indisponível. Aguardando...")    
                sleep\!(1000)    
                continue  \# Pula o restante do bloco e inicia nova iteração    
            True:    
                echo\!("Conexão estabelecida com sucesso\!")    
                break     \# Sai do loop imediatamente

#### **2.3. Desestruturação Exaustiva (match) A palavra-chave if não existe na linguagem Kata. O desvio condicional dentro do mundo imperativo é centralizado no bloco match. O match é obrigatório para lidar com Tipos de Soma (como Result::(T, E) e Optional::T), e deve ser estritamente exaustivo. O compilador recusará qualquer programa que não cubra todas as variantes possíveis da estrutura, a menos que a cláusula de fallback otherwise: seja fornecida.**

action ler\_banco (id\_utilizador)
    let resposta db\_query\! id\_utilizador

    match resposta
        \# Extração de valores em caso de sucesso
        Ok dados: echo\!(format "Nome: {}" dados.nome)

        \# O compilador força o tratamento do 'Err', prevenindo crashes silenciosos
        Err falha:
            log\!(format "Falha no banco: {}" falha)
            panic\!("Abortando ação.")

#### **2.4. Propagação de Erros (Operador ?)**

A ausência de um retorno antecipado converte operações falíveis encadeadas numa pirâmide de aninhamento inaceitável. O operador **pós-fixado** `?` atua como açúcar sintático para desestruturação monádica e retorno de escopo em *Actions*.

* **Mecânica:** Ao encontrar o operador `?` pós-fixado a uma expressão, o compilador injeta invisivelmente um bloco match. Em caso de Ok(v) ou Some(v), desempacota o valor. Em caso de Err(e) ou None, aborta a *Action* atual e retorna o erro imediatamente para o chamador.
* **Sintaxe:** O `?` é posicionado **após** a expressão ou aplicação, não antes.
* **Restrição Arquitetural:** O Type Checker exige que a assinatura de retorno da *Action* envolvente seja compatível com a falha propagada (deve retornar Result ou Optional).

\# O '?' pós-fixado intercepta o Result mantendo o fluxo estritamente linear
action processar\_ficheiro (caminho)
    let fd (abrir\_ficheiro\! caminho)?
    let dados (ler\_dados\! fd)?
    let json (parse\_json dados)?

    Ok json

## **Sistema de Tipos: Primitivos e Topologia de Coleções**

A linguagem Kata separa estritamente os tipos de dados básicos das estruturas de coleção. O layout de memória das coleções é ditado diretamente pelos delimitadores sintáticos (( ), \[ \], { }), permitindo que o compilador e o programador saibam exatamente o custo de alocação em tempo de execução.

### **1\. Tipos Primitivos (Atómicos)**

Os blocos de construção atómicos da linguagem são geridos de forma nativa e, na sua maioria, são tipos copiáveis de baixo custo:

* **Int / Float:** Numéricos com precisão padrão da arquitetura (ex: 64-bit). Suportam o uso de `_` como separador visual (ex: `10_000`). Inteiros podem ser representados nas bases binária (`0b`), octal (`0o`) e hexadecimal (`0x`).  
* **Byte:** Inteiro sem sinal de 8-bit (0-255), essencial para manipulação de I/O.  
* **Text:** Cadeias de caracteres UTF-8. Tratado como dado "cego" e puro pelo analisador léxico, sem suporte a interpolação embutida (mágica) para preservar a pureza do fluxo.  
* **Unit:** Representado também pelo literal tuplo vazio (), indica a ausência matemática de valor, substituindo conceitos como void.

### **2\. Coleções e Layout de Memória**

A linguagem divide as coleções fundamentadas em três eixos: tipagem (homogénea vs. heterogénea), mutabilidade de tamanho (dinâmico vs. fixo) e topologia de alocação de memória.

#### **2.1. Tuplos (Heterogêneo, Tamanho Fixo)**

* **Sintaxe:** Parênteses ( ).  
* **Comportamento:** Estruturas heterogéneas utilizadas para agrupamento posicional estrito.  
* **Mecânica do Compilador:** Como definido pela "Teoria Unificada", qualquer agrupamento isolado por parênteses que não seja resolvido como uma chamada de função torna-se um tuplo literal estrutural.

#### **2.2. Listas Persistentes (Homogêneo, Tamanho Dinâmico)**

* **Sintaxe:** Parênteses retos \[ \].  
* **Assinatura:** List::T.  
* **Comportamento:** Otimizadas para algoritmos funcionais puramente recursivos. Operam sob uma topologia encadeada, garantindo imutabilidade de custo zero através da partilha estrutural de caudas (Cons de cabeças e caudas).

#### **2.3. Arrays Contíguos (Homogêneo, Tamanho Dinâmico)**

* **Sintaxe:** Chaves sem terminadores ; (ex: {1 2 3}).  
* **Assinatura:** Array::T.  
* **Comportamento:** Estruturas flexíveis de I/O em bloco. Ocupam blocos de memória contígua garantindo o máximo aproveitamento da cache da CPU (Cache-Friendly), permitindo iterações rápidas.  
* **Restrição Matemática:** Por terem dimensões desconhecidas no momento da compilação, o *Type Checker* **não** desbloqueia operadores matemáticos avançados de álgebra linear para Arrays.

#### **2.4. Tensores Estáticos (Homogêneo numérico, Tamanho Fixo N-Dimensional)**

* **Sintaxe:** Chaves com a presença do promotor ; para forçar quebra de dimensões (ex: vetor linha {1 2 3 ;} ou matriz {1 2 ; 3 4}).  
* **Assinatura:** Tensor::T::(Int…).  
* **Comportamento:** A família de elite para processamento matemático acelerado. O seu tamanho e dimensionalidade são conhecidos em tempo de compilação (*Const Generics*). O *Type Checker* desbloqueia regras matemáticas rigorosas (ADD\_BEHAVIOR, MUL\_BEHAVIOR, DOT\_BEHAVIOR), traduzindo estas operações diretamente para instruções SIMD no motor Cranelift sem qualquer *overhead*.

#### **2.5. Dicionários Persistentes (Dict)**

* **Assinatura:** Dict::(K, V) (onde o tipo K deve obrigatoriamente implementar as interfaces HASH e EQ).  
* **Sintaxe de Construção:** A invocação do próprio tipo como construtor, recebendo uma List de tuplos chave-valor. Duplicatas subescrevem o valor anterior silenciosamente.
  let capitais Dict \[("Brasil" "Brasília") ("Japão" "Tóquio")\]  
* **Comportamento:** Estruturas de mapeamento não-ordenadas e imutáveis baseadas em árvores de partilha estrutural (HAMT). O método insert devolve um novo dicionário fundido, e o método get obriga o retorno sob um Optional para segurança espacial.

#### **2.6. Conjuntos (Set)**

* **Assinatura:** Set::T (onde o tipo T implementa HASH e EQ).  
* **Sintaxe de Construção:** A invocação do tipo como construtor recebendo uma List.
  let permitidos Set \[80 443 8080\]  
* **Comportamento:** Coleção imutável que garante a singularidade matemática dos seus elementos. Otimizada para operações de álgebra relacional fornecidas nativamente pela *StdLib* (union, intersect, diff).

#### **2.7. Intervalos Geradores (Ranges)**

* **Sintaxe:** \[início..fim\] ou \[início..passo..fim\] encapsulados na sintaxe de listas genéricas.  
* **Assinatura:** Range::T (onde T implementa NUM).  
* **Comportamento:** Um Range é um gerador preguiçoso (*Lazy Iterator*). O motor do compilador armazena apenas os limites e a passada, sem alocar memória em bloco na *Heap*.  
  * São consumidos nativamente por laços for imperativos nas *Actions*.  
  * Podem ser acoplados a processos do *Stream Fusion* no domínio funcional (ex: \[1..100\] |\> map f).  
  * Se houver a necessidade de os materializar numa coleção estrita em memória, o programador deve fornecer o Range ao construtor desejado (ex: List \[1..10\]).

### **3\. Tipos Algébricos de Dados (ADTs)**

A Kata-Lang não possui o conceito de classes ou herança orientada a objetos. A modelagem de domínio é feita estritamente através de Tipos Algébricos de Dados, divididos em Tipos Produto (Estruturas/Registos) e Tipos Soma (Variantes/Enums).

A linguagem utiliza duas palavras-chave distintas para a declaração, garantindo desambiguação léxica imediata: data para conjunções e enum para disjunções.

#### **3.1. Tipos Produto (Estruturas Lógicas)**

Representam a conjunção lógica (AND). Uma instância de um Tipo Produto contém simultaneamente todos os campos declarados. A topologia de memória é a de um bloco contíguo alocado (Struct).

* **Sintaxe:** Declarados com a palavra-chave data. Os campos ficam entre parênteses, separados por espaços. A anotação de tipo é feita com :: (opcional caso o compilador consiga inferir, mas recomendada para documentação de domínio).  
* **Acesso:** Os campos são acedidos via notação de ponto (.).

\# Produto: Um Vetor2D possui um 'x' E um 'y'    
data Vetor2D (x::Float y::Float)    
    
\# Produto genérico    
data Caixa::T (conteudo::T peso::Int)    
    
action processar\_vetor
    let v Vetor2D 10.5 20.0
    echo\!(format "Eixo X: {}" v.x)

*(Nota: Tuplos (A B) são, na sua essência, Tipos Produto anónimos sem chaves nomeadas).*

#### **3.2. Tipos Soma (Variantes)**

Representam a disjunção lógica (OR). Uma instância de um Tipo Soma ocupa o tamanho em memória da sua maior variante, mais uma *tag* de identificação discriminatória (Discriminant Tag). O compilador garante a segurança de acesso obrigando o uso de *Pattern Matching* em Functions ou match em Actions.

* **Sintaxe:** Declarados com a palavra-chave enum. As variantes são separadas pelo operador |. Podem ser unitárias (sem carga de dados) ou carregar tipos associados entre parênteses. O uso de múltiplas linhas é encorajado para clareza visual.

\# Soma: Uma Transação é Aprovada, OU Recusada (com um motivo), OU Pendente
enum Transacao
    | Aprovada
    | Recusada(Text)
    | Pendente

action verificar\_pagamento (t::Transacao)
    match t
        Aprovada: echo\!("Sucesso")
        Recusada motivo: echo\!(format "Falha: {}" motivo)
        Pendente: echo\!("Aguardando processamento")

#### **3.2.1. Variantes com Predicados e Valores Fixos**

As variantes de um enum podem carregar valores com restrições adicionais, estendendo o conceito de Tipos-Refinados para o domínio dos Tipos Soma.

##### **Formas de Variante**

| Variante | Sintaxe | Significado |
|----------|---------|-------------|
| Unitária | `Nome` | Sem dados |
| Com tipo | `Nome(Tipo)` | Carrega qualquer valor do tipo |
| Com valor fixo | `Nome(valor)` | Carrega valor literal (tipo inferido do valor) |
| Com predicado | `Nome(pred)` | Carrega valor que satisfaz predicado (tipo inferido) |
| Default | `Nome` ou `Nome(Tipo)` | Última variante, sem predicado |

##### **Variantes com Valores Fixos**

Uma variante pode carregar um valor literal específico, útil para constantes nomeadas:

\# Códigos HTTP como variantes
enum StatusHTTP
    | OK(200)
    | Created(201)
    | BadRequest(400)
    | NotFound(404)
    | InternalError(500)

\# Direções cardeais
enum Direcao
    | Norte("N")
    | Sul("S")
    | Leste("L")
    | Oeste("O")

##### **Variantes com Predicados**

Uma variante pode restringir os valores que aceita através de um predicado, unificando Tipos Soma com Tipos-Refinados. O predicado segue a mesma sintaxe dos Tipos-Refinados.

\# Classificação de IMC com predicados
enum IMC
    | Magreza(< _ 18.5)
    | Normal(<= _ 25.0)
    | Sobrepeso(<= _ 30.0)
    | Obesidade

\# Com valor preservado no default
enum IMCDetalhado
    | Magreza(< _ 18.5)
    | Normal(<= _ 25.0)
    | Sobrepeso(<= _ 30.0)
    | Obesidade(Float)     # default preserva o valor

##### **Regras para Variantes Predicadas**

Quando um enum possui variantes predicadas, as seguintes regras obrigatórias aplicam-se:

1. **Obrigatoriedade de Predicados:** Se houver variantes predicadas, TODAS as variantes (exceto a última) DEVEM ter predicados. Não é permitido misturar variantes com e sem predicado no meio da declaração.

2. **Ordem de Resolução:** Os predicados são avaliados de cima para baixo. A primeira variante cujo predicado for satisfeito pelo valor é escolhida.

3. **Variante Default:** A última variante é obrigatoriamente sem predicado e atua como default, aceitando qualquer valor restante. Pode ser unitária (descarta o valor) ou carregar um tipo (preserva o valor).

\# ✅ VÁLIDO: todas as variantes predicadas (exceto a última)
enum IMC
    | Magreza(< _ 18.5)
    | Normal(<= _ 25.0)
    | Sobrepeso(<= _ 30.0)
    | Obesidade           # default: unitário

\# ❌ INVÁLIDO: variante sem predicado no meio
enum IMC_Invalido
    | Magreza(< _ 18.5)
    | Normal              # ERRO: predicado esperado
    | Sobrepeso(<= _ 30.0)
    | Obesidade

##### **Construtor Inteligente**

Enums com variantes predicadas possuem um construtor inteligente que avalia os predicados automaticamente:

\# O construtor avalia os predicados em ordem
let resultado IMC(17.0)    # → Magreza(17.0)
let resultado IMC(22.0)    # → Normal(22.0)
let resultado IMC(28.0)    # → Sobrepeso(28.0)
let resultado IMC(35.0)    # → Obesidade

\# Resolução passo a passo para IMC(35.0):
\# 35.0 < 18.5?    NÃO → próxima variante
\# 35.0 <= 25.0?   NÃO → próxima variante
\# 35.0 <= 30.0?   NÃO → próxima variante
\# Obesidade       → default (sem predicado)

##### **Pattern Matching com Variantes Predicadas**

O pattern matching extrai o valor das variantes de forma usual:

\# Com default unitário (valor descartado)
match resultado
    Magreza(v): echo\!(format "Abaixo do peso (IMC: {})" v)
    Normal(v): echo\!(format "Peso normal (IMC: {})" v)
    Sobrepeso(v): echo\!(format "Sobrepeso (IMC: {})" v)
    Obesidade: echo\!("Obesidade (IMC >= 30.0)")

\# Com default preservando valor
match resultado_detalhado
    Magreza(v): echo\!(format "Abaixo do peso (IMC: {})" v)
    Normal(v): echo\!(format "Peso normal (IMC: {})" v)
    Sobrepeso(v): echo\!(format "Sobrepeso (IMC: {})" v)
    Obesidade(v): echo\!(format "Obesidade (IMC: {})" v)

### **3.3. Tipos Soma Fundamentais (Standard Library)**

Para eliminar comportamentos especiais (mágica de compilador), três conceitos fundamentais de controlo de fluxo são definidos nativamente como Tipos Soma na biblioteca padrão utilizando a palavra-chave enum.

#### **3.3.1. Bool (Lógica Booleana) O tipo primitivo Bool é uma variante simples. Isto significa que guardas e lógicas de desvio não requerem código de máquina especial no compilador; utilizam a mesma infraestrutura de análise de exaustividade de qualquer outro Tipo Soma.**

enum Bool (True | False)

#### **3.3.2. Optional::T (Ausência Segura de Valor) A linguagem não possui o conceito de null ou nil (o "erro de mil milhões de dólares"). A ausência de valor é semanticamente explícita no sistema de tipos através do Optional. Funções que podem não encontrar um resultado (como a busca numa lista) devem retornar este tipo.**

enum Optional::T
    | Some(T)
    | None

#### **3.3.3. Result::(T, E) (Tratamento de Falhas) Não existe mecanismo de lançamento de exceções (try/catch) na linguagem (exceções invisíveis quebram a pureza funcional). Operações passíveis de falha (I/O, conversões dinâmicas de tipos, divisão por zero em tempo de execução) retornam o tipo Result. O programador é forçado a lidar estaticamente com o cenário de sucesso (Ok) e o de falha (Err).**

enum Result::(T, E)
    | Ok(T)
    | Err(E)

### **Fronteira Dinâmica para Estática (Cast Seguro)**

A conversão de dados flexíveis (Array::T) adquiridos do I/O para dados matemáticos rígidos (Tensor::T::(Int…)) não é implícita. Deve ser feita usando a chamada ao tipo como um construtor de coerção (ex: Tensor::Int::(3 3\) dados\_dinamicos) em *runtime*, o qual sempre retorna um Result, forçando o programador a lidar com uma eventual falha na incompatibilidade de dimensões de forma pura antes da execução do cálculo matemático.

## **Sistema de Módulos, Visibilidade e Coerência**

O sistema de pacotes da Kata-Lang é concebido para evitar ambiguidade de resolução de símbolos e prevenir o "Inferno de Dependências" (onde o comportamento de uma aplicação muda drasticamente dependendo da ordem de importação de bibliotecas). O compilador preserva a pureza de escopo bloqueando a execução de efeitos colaterais originários de importação.

### **1\. Encapsulamento, Visibilidade e Importação**

* **A Unidade Base:** Cada ficheiro .kata representa um módulo isolado.  
* **O Agregador (mod.kata):** Para estruturar projetos maiores, um diretório é tratado como um pacote centralizado se contiver o arquivo base mod.kata. Ele atua como agregador e definidor das exportações públicas daquele diretório inteiro.  
* **Privacidade por Predefinição:** Qualquer tipo (data/enum), função (lambda) ou ação (action) declarada num ficheiro é estritamente invisível para o exterior.  
* **A Cláusula export:** Apenas identificadores explicitamente listados após a palavra-chave export são acessíveis por outros módulos. A cláusula export é uma lista de nomes, não um bloco de delimitação léxica.  
* **Regra Dourada de Execução:** Na Kata-Lang, *imports* são estritamente **declarativos**. Nenhuma instrução importada de um ficheiro executará código de inicialização. Apenas o ficheiro de entrada (o *Entrypoint* compilado ou executado explicitamente via terminal) tem a permissão de instanciar Actions no escopo global.

\# modulo\_matematico.kata    
let pi 3.14159 \# Privado    
    
soma :: Int Int \=\> Int    
lambda x y: + x y    
    
export soma

* **Importação e Namespaces:** A linguagem suporta três mecânicas estritas de importação:
  * **Importação por Namespace:** `import biblioteca`. Acesso via `biblioteca.funcao`.
  * **Importação Unitária:** `import biblioteca.Item`.
  * **Importação em Bloco:** `import biblioteca.(ItemA ItemB ItemC)`. Importa múltiplos itens selecionados para o escopo atual.

import sistema_ficheiros              # Importa o namespace completo
import modulo_matematico.soma         # Importação unitária

action principal
    let fd sistema_ficheiros.abrir\!("config.txt")
    let calc soma 10 20                         

### **2\. Declaração de Contratos (implements)**

A implementação de Interfaces (Polimorfismo Ad-Hoc e Despacho Múltiplo) em Kata **não é encapsulável**. As declarações implements são contratos globais ao nível do módulo. Elas devem ser declaradas no nível superior (*top-level*) do ficheiro, nunca aninhadas dentro de controlos de fluxo ou blocos de visibilidade.

Quando um módulo importa um Tipo de Dado, **importa irrevogavelmente todas as implementações de interface** atreladas a esse tipo que estejam visíveis no módulo de origem.

data Vec2 (x y)

\# A implementação é feita no topo do escopo. Não requer a palavra 'export'.    
\# Se 'Vec2' for exportado, este contrato viaja com ele automaticamente.    
Vec2 implements NUM
    \+ :: Vec2 Vec2 \=\> Vec2
    lambda (a b) Vec2 $(+ a.x b.x) $(+ a.y b.y)

    \- :: Vec2 Vec2 \=\> Vec2
    lambda (a b) Vec2 $(- a.x b.x) $(- a.y b.y)

### **3\. A Regra de Coerência (Orphan Rule)**

Para evitar que duas bibliotecas de terceiros implementem a mesma interface para o mesmo tipo de forma contraditória, o *Type Checker* impõe a **Regra de Coerência** no ato da compilação:

*Para implementar uma Interface para um Tipo, **pelo menos um dos dois** (a Interface ou o Tipo) tem de ter sido definido no módulo atual.*

* **Permitido:** Tipo Local \+ Interface Local.  
* **Permitido:** Tipo Local \+ Interface Externa.  
* **Permitido:** Tipo Externo \+ Interface Local.  
* **Proibido (Falha de Compilação):** Tipo Externo \+ Interface Externa (Instância Órfã).

**Resolução via Padrão *Newtype* (alias):** Caso seja estritamente necessário implementar uma Interface Externa num Tipo Externo, o programador deve encapsular o tipo externo numa nova estrutura definida localmente utilizando a palavra-chave alias. O compilador tratará isto como um novo tipo nominal estrito (satisfazendo a Regra de Coerência), mas eliminará o custo de alocação deste invólucro em tempo de execução (*Zero-Cost Abstraction*).

import biblioteca\_json           \# Importação por Namespace    
import biblioteca\_math.Matrix    \# Importação Unitária

\# 1\. Cria-se o Newtype Local (Alias Nominal Forte)    
alias MatrizLocal Matrix

\# 2\. Implementa-se a interface externa (sob o namespace) no tipo local    
MatrizLocal implements biblioteca\_json.JsonSerializable    
    to\_json :: MatrizLocal \=\> Text
    lambda mat: …

    ## **Concorrência e Isolamento (Modelo CSP)**
A Kata-Lang implementa concorrência exclusivamente no domínio das **Actions**. O modelo baseia-se em *Communicating Sequential Processes* (CSP). Não existe partilha de memória entre processos paralelos; o isolamento é absoluto e a sincronização ocorre estritamente através da passagem de mensagens por canais tipados.

### **1\. Criação de Processos (fork\!)**

A primitiva fork\! aceita a invocação de uma Action e submete-a ao escalonador do *runtime*. Por predefinição, a execução ocorre numa *Green Thread* cooperativa (M:N).

action worker (id)
    echo\!(format "Worker {} a iniciar" id)

action main    
    \# Inicia a execução concorrente e liberta a thread atual imediatamente    
    fork\! (worker 1\)

### **2\. Topologia de Canais**

Os canais são o único meio de comunicação inter-processos. São unidirecionais na sua utilização, mas a sua criação devolve sempre um tuplo contendo o lado emissor (tx) e o lado recetor (rx). A linguagem oferece três topologias com garantias de bloqueio distintas:

#### **2.1. Canal Rendezvous (channel\!)**

Síncrono e sem *buffer*. A transferência de dados exige que o emissor e o recetor estejam prontos simultaneamente.

* O operador de envio \!\> bloqueia até que uma *Action* execute uma receção \<\!.
* O operador de receção \<\! bloqueia até que uma *Action* execute um envio \!\>.

let (tx rx) channel\!()

#### **2.2. Fila Assíncrona (queue\!) Possui um *buffer* de tamanho fixo em memória. Funciona como mecanismo primário de *backpressure* (contrapressão) para ritmos desiguais de I/O.**

* O envio \!\> bloqueia a *Action* emissora caso o *buffer* atinja o seu limite, forçando-a a aguardar (travada) até que um recetor consuma pelo menos um item e liberte espaço na fila.
* A receção \<\! bloqueia a *Action* recetora apenas se o *buffer* estiver completamente vazio.

let (tx rx) queue\!(16) \# Fila com capacidade máxima para 16 elementos

#### **2.3. Difusão Múltipla (broadcast\!) Topologia *Publish-Subscribe* (1 para N).**

* A criação devolve um emissor (tx) e uma **fábrica de recetores** (subscribe).  
* **Sem retroatividade (*No Replay*):** Quando um novo recetor é inscrito através de subscribe, ele não tem acesso ao histórico de mensagens passado. Receberá estritamente as mensagens publicadas após o momento exato da sua inscrição.  
* O envio \!\> **nunca** bloqueia o emissor. Se um recetor específico estiver demasiado atrasado a ponto de encher o seu *buffer* local associado à subscrição, as mensagens mais antigas desse recetor são descartadas silenciosamente (*Drop-Oldest*), garantindo que o publicador nunca é penalizado por subscritores lentos.

let (tx subscribe) broadcast\!()    
let rx\_cliente\_1 subscribe(4) \# Subscreve com um buffer local de 4    
let rx\_cliente\_2 subscribe(8) \# Receberá apenas os envios feitos a partir desta linha

### **3\. Operações de Comunicação**

A interação com os canais utiliza a notação prefixada e os operadores direcionais de I/O.

* \!\> tx valor: Transfere a propriedade do valor para o canal.
* \<\! rx: Extrai o próximo valor disponível do canal de forma bloqueante.
* \<\!? rx: Tentativa de extração não-bloqueante. Se não houver dados disponíveis imediatamente, não bloqueia a *Action* e retorna None. Caso contrário, retorna Some valor. (Equivalente a um try\_recv).

action produtor (tx)
    valor \!\> tx

action consumidor (rx)
    let dados (\<\! rx)
    echo\!(dados)

action consumidor\_nao\_bloqueante (rx)
    match (\<\!? rx)
        Some dados: echo\!(format "Recebido agora: {}" dados)
        None: echo\!("Nenhum dado disponível no canal. Continuando...")

### **4\. Multiplexagem Não-Determinística (select)**

O select é a estrutura de controlo imperativa para aguardar múltiplos eventos assíncronos. Avalia todas as operações de canal declaradas nos seus ramos (case) e bloqueia a *Action* até que **um** dos eventos esteja pronto. Se múltiplos canais estiverem prontos em simultâneo, o escalonador escolhe um ramo de forma pseudoaleatória para garantir justiça (*fairness*) e evitar a inanição (*starvation*) de canais secundários.

action multiplexador (rx\_a rx\_b tx\_c)
    loop
        select
            \# Ramo de receção
            case (\<\! rx\_a) -> valor\_a:
                echo\! format "Recebido de A: {}" valor\_a

            \# Ramo de receção alternativo
            case (\<\! rx\_b) -> valor\_b:
                echo\! format "Recebido de B: {}" valor\_b

            \# Ramo de emissão (só executa se tx\_c tiver espaço no buffer)
            case ("Ping" \!\> tx\_c):
                echo\!("Sinal enviado para C")

            \# Desbloqueio temporal
            timeout 1000:
                echo\!("Inatividade detetada. 1s passado.")

## **Diretivas de Compilação e Runtime**

As diretivas são anotações sintáticas prefixadas pelo símbolo @. Atuam como instruções explícitas para o compilador e para o *runtime*, alterando características não-funcionais do código (como políticas de resiliência, alocação de memória ou ligação a bibliotecas externas) ou instruindo o otimizador sem modificar a semântica matemática do programa.

### **1\. Otimização e Despacho (@commutative e @associative)**

Utilizadas primariamente para otimizar funções de biblioteca padrão e algoritmos recursivos.

* **@commutative:** Informa o *Type Checker* que a ordem dos argumentos na função não altera o resultado (ex: `a + b == b + a`). Isso permite ao compilador tentar resolver ambiguidades de *Multiple Dispatch* trocando os argumentos de lugar caso a assinatura direta não seja encontrada.
* **@associative([identidade]):** Informa o compilador que a função obedece à propriedade associativa (Semigrupo) e, opcionalmente, fornece o seu **elemento neutro** (Monoide). Esta diretiva é vital para o **TRMA (Tail Recursion Modulo Associativity)**. Quando o otimizador deteta uma recursão bloqueada por uma função associativa, se o elemento neutro tiver sido fornecido, ele utilizará esse valor para injetar automaticamente um acumulador, convertendo o algoritmo para uma recursão de cauda perfeita (TCO) sem custo na *Call Stack*. Caso a função seja apenas associativa mas não possua elemento neutro (ex: `max`), a diretiva pode ser declarada sem argumentos (apenas `@associative`), impedindo o TRMA mas abrindo portas a outras otimizações estáticas da AST.

```kata
@commutative
@associative(0)
+ :: Int Int => Int
lambda a b: ...
```

### **2\. Políticas de Reinicialização e Resiliência (@restart)**

Delegar a resiliência e o controle de exceções transientes ao *Work-Stealing Scheduler* reduz a presença de laços imperativos. A diretiva @restart acopla metadados de ciclo de vida direto à raiz de uma *Action*. Se a execução atingir um estado de pânico (panic\!) ou for terminada de modo não premeditado, o escalonador tenta reinstanciá-la conforme a política definida.

* @restart{'always'}: Reinício incondicional. Muito utilizado para processos supervisores ou daemons longos.  
* @restart{tries: N, delay: Xms}: Configuração de retentativas limitadas com intervalo em milissegundos. Esgotado o limite, o *panic* é propagado e não há novas instâncias.

### **2\. Isolamento de Processo (@parallel)**

Por predefinição, a invocação de fork\! submete uma *Action* ao escalonador de *Green Threads* em modo cooperativo dentro do mesmo processo. A diretiva @parallel força o *runtime* a instanciar um **Processo Nativo do Sistema Operativo** (Thread Pesada) isolado.

* **Indicação:** Deve ser usada estritamente para tarefas com uso intensivo de CPU (CPU-bound) que bloqueariam o escalonador cooperativo, ou para obter isolamento de memória absoluto.  
* **Restrição de Transporte (IPC):** A comunicação com uma *Action* anotada com @parallel é feita via canais, o que obriga à serialização dos dados (IPC). É **estritamente proibido** enviar "Recursos Vinculados" (System Handles, como descritores de ficheiros ou sockets) através de canais ligados a processos @parallel, uma vez que estes não são serializáveis e dependem do contexto de memória do processo original.

@parallel    
action processar\_video (rx\_frames tx\_resultado)    
    \# Executa num processo nativo isolado pelo S.O.    
    let frame recv\! rx\_frames    
    \# ... processamento intensivo ...

### **3\. Memoização Automática (@cache\_strategy)**

Aplica-se **exclusivamente a Funções (puras)**. Como as funções em Kata são deterministicamente puras, o compilador pode interceptar a invocação e devolver um resultado previamente calculado armazenado numa tabela de dispersão (*Hash Table* global partilhada), evitando recomputações dispendiosas.

Por predefinição, quando anotada sem argumentos, a estratégia de cache ativada é um lru (*Least Recently Used*) com uma heurística de retenção gerida pelo próprio compilador. O programador pode, contudo, definir configurações exatas explicitamente ou até desabilitar a inferência padrão do compilador.

A diretiva recebe os seus argumentos através de um bloco de chaves { }:

* strategy: Estratégia de substituição explícita ('default', 'lru', 'lfu', ou 'disabled' para forçar a desativação da memoização nalgum contexto em que os custos de inserção em tabela sejam indesejáveis).  
* size: Limite absoluto de entradas no cache.  
* ttl: Tempo de vida das entradas em milissegundos (opcional).

@cache\_strategy{strategy: 'lru', size: 1000}    
fibonacci :: Int \=\> Int    
lambda 0: 0    
lambda 1: 1    
lambda n: + fibonacci $(- n 1) fibonacci $(- n 2)

### **4\. Foreign Function Interface (@ffi)**

Utilizada para ligar assinaturas de funções Kata a símbolos compilados externamente (em C ou Rust). É a válvula de escape do sistema de tipos, permitindo invocar primitivas de sistema. O compilador confia na pureza declarada na assinatura. Se a função chamada através do FFI introduzir efeitos colaterais e for anotada como uma Função pura (em vez de uma *Action*), a responsabilidade por falhas de corrupção de memória é inteiramente do programador. (Uso reservado à *Standard Library* em implementações iniciais).

\# O compilador não gera o corpo desta Action, apenas cria a ligação no Linker    
@ffi('kata\_rt\_print')    
echo\! :: (Text) \=\> Action::Unit

### **5\. Execução em Tempo de Compilação (@comptime)**

A diretiva @comptime instrui o compilador a avaliar a função ou bloco de código estritamente durante a fase de compilação (*Compile-Time Function Execution*).

* **Substituição de Macros:** Por basear-se em funções estritamente puras, o @comptime elimina a necessidade de um sistema de macros separado. O código é executado pela própria infraestrutura do compilador (ou interpretador interno), e o seu resultado final é embutido como um literal estático (constante) no binário gerado.  
* **Restrições:** O código @comptime só pode operar sobre dados que sejam inteiramente conhecidos em tempo de compilação (como literais estruturais, ranges estáticos, ou os resultados de outras funções @comptime). É impossível utilizar blocos de *Action* (I/O) ou canais concorrentes durante a avaliação desta diretiva.

@comptime    
gerar\_tabela\_senos :: Int \=\> Array::Float    
lambda (precisao) ...    
\# O resultado desta computação será chumbado no binário    
\# como um literal Array::Float sem qualquer custo de runtime.

### **6\. Registro de Logs Transparentes (@log)**

Para manter a pureza das funções matemáticas (Functions) intacta sem prejudicar a observabilidade em ambientes de produção, a linguagem fornece a diretiva `@log`.

* **Mecânica:** Quando uma função ou ação é anotada com `@log`, o compilador injeta invisivelmente código de I/O em volta da sua execução (antes e depois do bloco). Isso permite registrar no log do sistema os argumentos de entrada e o resultado retornado.
* **Isolamento de Pureza:** Como o registro no log é tratado estritamente pela infraestrutura da máquina virtual (*Runtime* via Cranelift) e injetado por fora da função, a pureza interna da equação não é violada perante o Type Checker, pois o programador não precisou escrever um `echo!` imperativo no meio da conta.

@log
soma_complexa :: Int Int => Int
lambda (x y)
    + (* x 2) (* y 3)

## **7\. Infraestrutura de Testes e Asserções**

A Kata-Lang fornece um ambiente de testes integrado (invocado via kata test) desenhado sob a premissa de **Zero-Cost Abstraction** no ambiente de produção. O isolamento entre o código de teste e a lógica de domínio garante que o binário gerado via kata build permanece estritamente desprovido de abstrações mortas.

### **1\. A Diretiva @test e Eliminação Estática**

Blocos de código concebidos para validação devem ser antecedidos pela diretiva @test("Descrição").

* **Comportamento em Produção (kata build):** A fase de *Tree-Shaking* no compilador extirpa agressivamente todos os nós da AST (Abstract Syntax Tree) assinalados com @test, ignorando também as dependências que sejam exclusivas desses blocos. Nenhum código de máquina é gerado para testes na compilação final.  
* **Comportamento de Validação (kata test):** O compilador gera um *entrypoint* sintético que agrega as funções assinaladas, instanciando um binário descartável que regista e imprime os resultados baseados nas regras de sucesso de cada domínio.

### **2\. Testes no Domínio Funcional (Funções Puras)**

No domínio puro, funções de teste não podem executar o assert\! (visto que é imperativo) nem estourar *panics*. A validação ocorre por consequência matemática. O compilador impõe que qualquer lambda anotado com @test retorne estritamente um booleano (Bool).

* True \= O teste passou.  
* False \= O teste falhou.

@test("Soma de matrizes reflete a propriedade comutativa")  
test\_matriz\_comutativa :: () \=\> Bool  
lambda ()  
    let m1 (Tensor::Int::(2 2\) {1 2 ; 3 4})  
    let m2 (Tensor::Int::(2 2\) {5 6 ; 7 8})  
      
    \# Apenas a igualdade pura é avaliada e retornada  
    \= (+ m1 m2) (+ m2 m1) 

### **3\. Testes no Domínio Imperativo (Actions)**

As Actions executam I/O e não têm um retorno booleano de validação natural. Uma action anotada com @test é considerada um **Sucesso** se atingir o final da sua máquina de estado sem incidentes. É considerada uma **Falha** se a execução desviar para um panic\! ou unwrap\_or\_panic\!.

Para simplificar a instrumentação de testes imperativos, o ecossistema fornece o assert\!.

#### **O Operador assert\!**

assert\! :: Bool Text \=\> ()

O assert\! é uma *Action* nativa desenhada para o programador validar invariantes de I/O em tempo de teste ou depuração. Ele verifica o booleano; se este for False, aborta a thread inteira com o panic\! utilizando a mensagem informada.

Tal como o bloco @test, as invocações de assert\! que "vazem" para *Actions* de produção são desativadas ou cortadas pelo optimizador em compilações *release*, preservando os ciclos de CPU.

@test("Abertura de socket local com credenciais válidas")  
action test\_socket\_operacional ()  
    let operacao abrir\_socket\! 8080  
      
    \# Usando o assert\! para encurtar testes lógicos num ambiente impuro
    assert\! $(!= operacao None) "O construtor do socket não deveria retornar None"

    let socket unwrap\_or\_panic\! operacao "Falha na desestruturação"
    assert\! socket.is\_open "O socket instanciado não está aberto"

berto"

