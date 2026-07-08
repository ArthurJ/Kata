# Inspirações:

- Haskell: sintaxe fora de série, sistema de tipos infernal  
- Python: sintaxe linda, praticidade excepcional  
- Shellscript: A capacidade de encadear processos é excelente  
- Rust, Elixir…  
- Apache Spark: Forma de pensar  
- julia: A minha ideia de polimorfismo inicial é muito próxima do multiple dispatch dessa linguagem

# TL;DR:

\- Linguagem que separe o código em 3 "domínios":  
\- dados (data), que são os os dados processados  
\- cálculos, que são as funções (puras)  
\- ações (actions), que são código com efeito colateral (impuro)  
\- Valores imutáveis (let) por padrão, e valores variáveis (var) nas Actions opcionalmente  
\- As actions chamadas no toplevel do módulo são os pontos de entrada do código.  
\- Notação pré-fixada  
\- Sistema de tipos forte, com inferência, tipo-dependência, interfaces e Generics (parametric polymorphism).

# Blocos x Pattern Matching x Guards:

	Blocos de código, por serem maiores, são mais propensos a erros que definições declarativas. Por isso, o esquema de Pattern Match de Haskell é muito interessante, o código tenta encaixar o input em padrões e dessa forma escolher o “corpo” da função a ser usado.  
	Me incomodou um pouco no início a semelhança com if-else encadeado, ou com switch-case, mas nesse contexto faz bastante sentido e parece muito útil para segregar comportamentos do código de acordo com o input e, dessa forma diminuir a possibilidade de erros no código.  
	Ainda mais parecido com switch-case são os Guards, que não realizam Pattern-Match, mas condicionais, para escolher a linha a ser executada.  
	Guards e Pattern-Matching podem soar redundantes, mas Guards realizam computação (servem para isso aliás), enquanto Pattern-Matching não fazem o mesmo, são mais “simples”.

# Espaços e Notação prefixa “bizarra”:

	A notação prefixa é chocante à primeira vista, afinal toda educação em matemática usa a notação infixa, e toda linguagem de programação também. Certo?  
	Mais ou menos… estamos acostumados a notação infixa, mas quando aprendemos funções fora da aritmética, usamos notação prefixa.  
	Na definição “*f(x) \= x+1*”, ao usar a função *f* para o valor y, escreve-se “*f(y)*”, e na eventualidade de existir uma função *g*, para qual queremos aplicar no valor *f(y)*, escrevemos “*g(f(y))*”, isso já é notação prefixa.  
	E nós ainda misturamos infixa e prefixa, por exemplo, “*f(g(x)) \+ x \+ g(f(x))*”. Isso é mais confuso do que pensamos. Só não percebemos por que nos habituamos cedo e praticamos à exaustão.  
	Soma, divisão e multiplicação são funções . Não são particularmente diferentes de quaisquer outras. Por que essa diferença de tratamento? Ou, dado que já estamos acostumados com esse modo de fazer, talvez a pergunta devesse ser “por que não?”.  
	Bem, o primeiro motivo é a implementação de precedência. Notação infixa gera a necessidade de se estabelecer regras de precedência para definir qual função deve ser aplicada primeiro em relação às demais, e também torna essencial o uso de organizadores, como parênteses, chaves, colchetes, vírgulas e etc, tanto para humanos lendo, como para à máquina executando o código.   
A notação prefixa não tem esse problema, os organizadores ainda são necessários para o nosso entendimento e conforto (dependendo da complexidade da expressão escrita), porém para a máquina executando o código, eles são irrelevantes. E isso também torna a implementação da linguagem mais simples, já que não é necessário tratar os operadores aritméticos de maneira especial.   
Além de tornar a escrita visualmente menos densa quando quebramos o código em pedaços simples o suficiente, dado que deixa de ser obrigatório o uso dos organizadores. Isto também pode ser verdade para a notação infixa, mas comparando o melhor caso em cada notação, a prefixa é mais legível.  
A notação prefixa também cria a necessidade de um uso correto de espaços entre as funções em argumentos: “1+1” é perfeitamente válido e compreensível, tanto quanto “1 \+ 1”. Na notação prefixa “+ 1 1” é válida, porém “+1 1” não seria, e o motivo se torna evidente quando se substitui o nome da função de soma (“+”) por “somar”: “`+ 1 1"` se torna “`somar 1 1`" e eliminar o espaço entre o operador e o primeiro argumento (`somar1 1`)  causa a referência a uma função de nome diferente, “somar1” em vez de “somar”, que pode nem mesmo ter sido definida.   
Isso deixa a sintaxe de números com os símbolos de \+ e \- disponível,  sem causar confusão na leitura ou complexidade na implementação; “-10”, sem espaço, é “10 negativo”, enquanto “- 10” é uma subtração à espera do segundo argumento.  
Em conjunto com a estratégia de pattern-matching, eu *espero* que essa notação acabe deixando o código mais limpo, e mais fácil de se implementar e manter.