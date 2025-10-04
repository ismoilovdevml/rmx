#!/bin/bash

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  RMX vs RM - MEDIUM LOAD BENCHMARK${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

declare -a RESULTS

create_test_files() {
    local dir=$1
    local count=$2
    local size=$3

    echo -e "${YELLOW}Creating $count files...${NC}"
    mkdir -p "$dir"
    for i in $(seq 1 $count); do
        dd if=/dev/zero of="$dir/file_$i" bs=$size count=1 2>/dev/null
    done
    echo -e "${GREEN}✓ Done${NC}"
}

run_benchmark() {
    local test_name=$1
    local dir=$2

    echo ""
    echo -e "${CYAN}━━━ $test_name ━━━${NC}"

    local file_count=$(find "$dir" -type f | wc -l | tr -d ' ')
    echo -e "Files: $file_count"

    cp -r "$dir" "${dir}_rm"
    cp -r "$dir" "${dir}_rmx"
    sync

    START_RM=$(date +%s.%N)
    rm -rf "${dir}_rm" 2>/dev/null || true
    END_RM=$(date +%s.%N)
    RM_TIME=$(echo "$END_RM - $START_RM" | bc)

    sync
    sleep 0.3

    START_RMX=$(date +%s.%N)
    ./target/release/rmx -rf "${dir}_rmx" 2>/dev/null || true
    END_RMX=$(date +%s.%N)
    RMX_TIME=$(echo "$END_RMX - $START_RMX" | bc)

    if (( $(echo "$RMX_TIME > 0" | bc -l) )); then
        SPEEDUP=$(echo "scale=2; $RM_TIME / $RMX_TIME" | bc)
    else
        SPEEDUP="∞"
    fi

    echo -e "${GREEN}rm: ${RM_TIME}s | rmx: ${RMX_TIME}s | Speedup: ${SPEEDUP}x${NC}"
    RESULTS+=("$test_name|$RM_TIME|$RMX_TIME|$SPEEDUP")

    rm -rf "${dir}_rm" "${dir}_rmx" 2>/dev/null || true
}

# Test 1: 30K tiny files
echo -e "${BLUE}[1/6] 30K tiny files (100B)${NC}"
create_test_files "/tmp/bench_30k" 30000 100
run_benchmark "30K × 100B" "/tmp/bench_30k"
rm -rf /tmp/bench_30k

# Test 2: 5K small files
echo -e "${BLUE}[2/6] 5K small files (10KB)${NC}"
create_test_files "/tmp/bench_5k" 5000 10240
run_benchmark "5K × 10KB" "/tmp/bench_5k"
rm -rf /tmp/bench_5k

# Test 3: 1K medium files
echo -e "${BLUE}[3/6] 1K medium files (1MB)${NC}"
create_test_files "/tmp/bench_1k" 1000 1048576
run_benchmark "1K × 1MB" "/tmp/bench_1k"
rm -rf /tmp/bench_1k

# Test 4: 200 large files
echo -e "${BLUE}[4/6] 200 large files (10MB)${NC}"
create_test_files "/tmp/bench_200" 200 10485760
run_benchmark "200 × 10MB" "/tmp/bench_200"
rm -rf /tmp/bench_200

# Test 5: Deep nested
echo -e "${BLUE}[5/6] Deep nested (50 dirs × 200 files)${NC}"
mkdir -p /tmp/bench_nested
for i in {1..50}; do
    mkdir -p /tmp/bench_nested/dir_$i
    for j in {1..200}; do
        dd if=/dev/zero of=/tmp/bench_nested/dir_$i/file_$j bs=2048 count=1 2>/dev/null
    done
done
run_benchmark "Nested (50×200)" "/tmp/bench_nested"
rm -rf /tmp/bench_nested

# Test 6: 50K ultra-tiny files (stress test)
echo -e "${BLUE}[6/6] 50K ultra-tiny files (10B)${NC}"
mkdir -p /tmp/bench_50k
for i in $(seq 1 50000); do
    echo "test" > /tmp/bench_50k/file_$i
done
run_benchmark "50K × 10B" "/tmp/bench_50k"
rm -rf /tmp/bench_50k

# Summary
echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║              BENCHMARK SUMMARY                         ║${NC}"
echo -e "${BLUE}╠════════════════════════════════════════════════════════╣${NC}"
printf "${BLUE}║${NC} %-25s | %-10s | %-10s | %-8s ${BLUE}║${NC}\n" "Test" "rm(s)" "rmx(s)" "Speedup"
echo -e "${BLUE}╠════════════════════════════════════════════════════════╣${NC}"

total_speedup=0
count=0

for result in "${RESULTS[@]}"; do
    IFS='|' read -r name rm_time rmx_time speedup <<< "$result"
    printf "${BLUE}║${NC} %-25s | %-10s | %-10s | ${GREEN}%-7s${NC}x ${BLUE}║${NC}\n" "$name" "$rm_time" "$rmx_time" "$speedup"

    if [[ $speedup != "∞" ]]; then
        total_speedup=$(echo "$total_speedup + $speedup" | bc)
        count=$((count + 1))
    fi
done

echo -e "${BLUE}╠════════════════════════════════════════════════════════╣${NC}"

if [ $count -gt 0 ]; then
    avg=$(echo "scale=2; $total_speedup / $count" | bc)
    printf "${BLUE}║${NC} %-42s ${GREEN}%-10s${NC} ${BLUE}║${NC}\n" "AVERAGE SPEEDUP:" "${avg}x"
fi

echo -e "${BLUE}╚════════════════════════════════════════════════════════╝${NC}"

echo ""
rm -rf /tmp/bench_* 2>/dev/null || true
echo -e "${GREEN}Benchmark complete!${NC}"
