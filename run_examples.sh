#!/bin/bash

# Cores
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

mkdir -p out_logs

echo "====================================="
echo "  Rodando Exemplos Kata-lang (Build) "
echo "====================================="
echo "Os logs completos de cada teste estarao em out_logs/"
echo ""

SUCCESS_LIST=()
FAILED_LIST=()

for file in examples/*.kata; do
    echo -n "Testando $file... "
    filename=$(basename -- "$file")
    bin_name="${filename%.*}"
    log_file="out_logs/${bin_name}.log"
    
    cargo run -q -- build "$file" > "$log_file" 2>&1
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}[OK]${NC}"
        SUCCESS_LIST+=("$file")
    else
        echo -e "${RED}[ERRO]${NC}"
        # Continua exibindo um pequeno preview do erro no terminal
        grep -E "Error:|Erro" "$log_file" | head -n 3 | sed 's/^/    /'
        FAILED_LIST+=("$file")
    fi

    rm -f "$bin_name" "$bin_name.o" "kata_entry.c"
done

echo ""
echo "====================================="
echo -e "${GREEN}### SUCESSOS (${#SUCCESS_LIST[@]}) ###${NC}"
for s in "${SUCCESS_LIST[@]}"; do
    echo "  - $s"
done

echo ""
echo -e "${RED}### FALHAS (${#FAILED_LIST[@]}) ###${NC}"
for f in "${FAILED_LIST[@]}"; do
    echo "  - $f"
done
echo "====================================="
echo ""
echo "Resumo Final: ${GREEN}${#SUCCESS_LIST[@]} Sucessos${NC} / ${RED}${#FAILED_LIST[@]} Falhas${NC}"
