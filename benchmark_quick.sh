#!/bin/bash

set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  RMX vs RM - QUICK BENCHMARK${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

declare -a RESULTS

create_test_files() {
    local dir=$1
    local count=$2
    local size=$3

    echo -e "${YELLOW}Creating $count files of size $size...${NC}"
    mkdir -p "$dir"

    for i in $(seq 1 $count); do
        dd if=/dev/zero of="$dir/file_$i" bs=$size count=1 2>/dev/null
    done
    echo -e "${GREEN}✓ Created $count files${NC}"
}

run_benchmark() {
    local test_name=$1
    local dir=$2

    echo ""
    echo -e "${CYAN}━━━ $test_name ━━━${NC}"

    local file_count=$(find "$dir" -type f | wc -l | tr -d ' ')
    local dir_size=$(du -sh "$dir" 2>/dev/null | cut -f1)
    echo -e "Files: $file_count | Size: $dir_size"

    cp -r "$dir" "${dir}_rm"
    cp -r "$dir" "${dir}_rmx"
    sync

    echo -e "${YELLOW}Testing rm...${NC}"
    START_RM=$(date +%s.%N)
    rm -rf "${dir}_rm" 2>/dev/null || true
    END_RM=$(date +%s.%N)
    RM_TIME=$(echo "$END_RM - $START_RM" | bc)

    sync
    sleep 0.5

    echo -e "${YELLOW}Testing rmx...${NC}"
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

# Test 1: 10K small files
echo -e "${BLUE}[1/5] 10K small files (1KB)${NC}"
create_test_files "/tmp/bench_10k" 10000 1024
run_benchmark "10K × 1KB" "/tmp/bench_10k"
rm -rf /tmp/bench_10k

# Test 2: 500 medium files
echo -e "${BLUE}[2/5] 500 medium files (100KB)${NC}"
create_test_files "/tmp/bench_500m" 500 102400
run_benchmark "500 × 100KB" "/tmp/bench_500m"
rm -rf /tmp/bench_500m

# Test 3: 100 large files
echo -e "${BLUE}[3/5] 100 large files (10MB)${NC}"
create_test_files "/tmp/bench_100l" 100 10485760
run_benchmark "100 × 10MB" "/tmp/bench_100l"
rm -rf /tmp/bench_100l

# Test 4: 20K tiny files
echo -e "${BLUE}[4/5] 20K tiny files (100B)${NC}"
create_test_files "/tmp/bench_20k_tiny" 20000 100
run_benchmark "20K × 100B" "/tmp/bench_20k_tiny"
rm -rf /tmp/bench_20k_tiny

# Test 5: Nested structure
echo -e "${BLUE}[5/5] Nested directories${NC}"
mkdir -p /tmp/bench_nested
for i in {1..20}; do
    mkdir -p /tmp/bench_nested/dir_$i
    for j in {1..100}; do
        dd if=/dev/zero of=/tmp/bench_nested/dir_$i/file_$j bs=5120 count=1 2>/dev/null
    done
done
run_benchmark "Nested (20 dirs × 100 files)" "/tmp/bench_nested"
rm -rf /tmp/bench_nested

# Summary
echo ""
echo -e "${BLUE}========== SUMMARY ==========${NC}"
printf "%-30s | %-10s | %-10s | %-10s\n" "Test" "rm(s)" "rmx(s)" "Speedup"
echo "--------------------------------------------------------------------"

total_speedup=0
count=0

for result in "${RESULTS[@]}"; do
    IFS='|' read -r name rm_time rmx_time speedup <<< "$result"
    printf "%-30s | %-10s | %-10s | ${GREEN}%-9s${NC}x\n" "$name" "$rm_time" "$rmx_time" "$speedup"

    if [[ $speedup != "∞" ]]; then
        total_speedup=$(echo "$total_speedup + $speedup" | bc)
        count=$((count + 1))
    fi
done

if [ $count -gt 0 ]; then
    avg=$(echo "scale=2; $total_speedup / $count" | bc)
    echo "--------------------------------------------------------------------"
    echo -e "AVERAGE SPEEDUP: ${GREEN}${avg}x${NC}"
fi

echo ""
rm -rf /tmp/bench_* 2>/dev/null || true
echo -e "${GREEN}Benchmark complete!${NC}"
