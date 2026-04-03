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
    
    if [[ "$filename" == test2fail* ]]; then
        cargo run -q -- test "$file" > "$log_file" 2>&1
    else
        cargo run -q -- build "$file" > "$log_file" 2>&1
    fi
    
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
echo -e "Resumo Final: ${GREEN}${#SUCCESS_LIST[@]} Sucessos${NC} / ${RED}${#FAILED_LIST[@]} Falhas${NC}"

# --- Meta-Log Status History ---
LOG_FILE="meta_status_log.txt"
TIMESTAMP=$(date +"%Y-%m-%d %H:%M:%S")
CURRENT_SUCCESS=${#SUCCESS_LIST[@]}
CURRENT_FAILS=${#FAILED_LIST[@]}

# Read previous success count if log exists
PREV_SUCCESS=$CURRENT_SUCCESS
if [ -f "$LOG_FILE" ]; then
    # Extracts the last "Sucessos: N " pattern
    LAST_SUC=$(grep "Sucessos: [0-9]" "$LOG_FILE" | tail -n 1 | awk '{print $2}')
    if [ ! -z "$LAST_SUC" ]; then
        PREV_SUCCESS=$LAST_SUC
    fi
fi

DIFF=$((CURRENT_SUCCESS - PREV_SUCCESS))
DIFF_STR=""
if [ $DIFF -gt 0 ]; then
    DIFF_STR="(+${DIFF})"
elif [ $DIFF -lt 0 ]; then
    DIFF_STR="(${DIFF})"
else
    DIFF_STR="(-)"
fi

echo "[$TIMESTAMP] Sucessos: $CURRENT_SUCCESS $DIFF_STR | Falhas: $CURRENT_FAILS" >> "$LOG_FILE"
echo "Success_List: ${SUCCESS_LIST[*]}" >> "$LOG_FILE"
echo "Failed_List: ${FAILED_LIST[*]}" >> "$LOG_FILE"
echo "" >> "$LOG_FILE"

echo "-> Historico detalhado salvo em $LOG_FILE"
echo ""
