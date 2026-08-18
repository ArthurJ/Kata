# Capítulo 13 — Módulos

Kata organiza código em módulos. Cada arquivo `.kata` é um módulo. Itens exportados com `export` ficam visíveis para quem importar; itens não exportados são privados.

## Exportando

Declare funções, actions, e tipos normalmente. No final, `export` lista o que é público:

```kata
# mod_math.kata
dobrar :: Int => Int
lambda x: + x x

triplicar :: Int => Int
lambda x: * x 3

quadruplar :: Int => Int
lambda x: * x 4

export dobrar triplicar
```

`quadruplar` não está na lista — é privada do módulo.

## Importando módulo inteiro

`import mod` traz o módulo para o escopo. Acesso via prefixo `mod.fn`:

```kata
import mock_math

action main
    let dobro := mock_math.dobrar 21
    let triplo := mock_math.triplicar 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Import seletivo

`import mod.(item)` traz itens específicos para o escopo direto, sem prefixo:

```kata
import mock_math.(triplicar)

action main
    let triplo := triplicar 21
    echo!(triplo)
main!()
```

```
63
```

## Import com alias

Renomeie itens ao importar para evitar colisões:

```kata
import mock_math.(dobrar as d, triplicar as t)

action main
    let dobro := d 21
    let triplo := t 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Resolução de paths

O loader procura módulos no mesmo diretório do arquivo que faz o import. Para `import mock_math`, o loader procura `mock_math.kata` no diretório do importador. Imports aninhados (`import subdir.mod`) seguem o path relativo.

## Próximo capítulo

Módulos organizam código. O próximo capítulo mostra as otimizações que o compilador aplica automaticamente — TCO, TRMA, stream fusion, e `@cache`.