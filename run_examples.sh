#!/bin/bash

# Cores
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo "====================================="
echo "  Rodando Exemplos Kata-lang (Build) "
echo "====================================="
echo ""

SUCCESS=0
FAILED=0

for file in examples/*.kata; do
    echo -n "Testando $file... "
    # Obtém o nome do binário que será gerado (removendo o diretório e a extensão)
    filename=$(basename -- "$file")
    bin_name="${filename%.*}"
    
    # Run the compiler on the file, redirecting stdout/stderr to a variable to keep the screen clean
    # Using 'build' command since 'run' might not be fully functional for JIT yet according to PRD
    OUTPUT=$(cargo run -q -- build "$file" 2>&1)
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}[OK]${NC}"
        SUCCESS=$((SUCCESS + 1))
    else
        echo -e "${RED}[ERRO]${NC}"
        # Print a short snippet of the error for context
        echo "$OUTPUT" | grep -E "Error:|Erro" | head -n 3 | sed 's/^/    /'
        FAILED=$((FAILED + 1))
    fi

    # Limpeza de binários e arquivos objetos gerados
    rm -f "$bin_name" "$bin_name.o" "kata_entry.c"
done

echo ""
echo "====================================="
echo "Resumo:"
echo -e "  Sucesso: ${GREEN}${SUCCESS}${NC}"
echo -e "  Falhas:  ${RED}${FAILED}${NC}"
echo "====================================="
