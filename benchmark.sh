#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}================================${NC}"
echo -e "${BLUE}  RMX vs RM Benchmark Tests${NC}"
echo -e "${BLUE}================================${NC}"
echo ""

# Function to create test files
create_test_files() {
    local dir=$1
    local count=$2
    local size=$3

    echo -e "${YELLOW}Creating $count files of size $size in $dir...${NC}"
    mkdir -p "$dir"

    for i in $(seq 1 $count); do
        dd if=/dev/urandom of="$dir/file_$i" bs=$size count=1 2>/dev/null
    done

    echo -e "${GREEN}Created $count files${NC}"
}

# Function to run benchmark
run_benchmark() {
    local test_name=$1
    local dir=$2
    local command=$3

    echo ""
    echo -e "${BLUE}--- $test_name ---${NC}"

    # Copy directory for both tests
    cp -r "$dir" "${dir}_rm"
    cp -r "$dir" "${dir}_rmx"

    # Benchmark rm
    echo -e "${YELLOW}Testing rm...${NC}"
    START_RM=$(date +%s.%N)
    eval "$command ${dir}_rm" 2>/dev/null || true
    END_RM=$(date +%s.%N)
    RM_TIME=$(echo "$END_RM - $START_RM" | bc)

    # Benchmark rmx
    echo -e "${YELLOW}Testing rmx...${NC}"
    START_RMX=$(date +%s.%N)
    eval "rmx $4 ${dir}_rmx" 2>/dev/null || true
    END_RMX=$(date +%s.%N)
    RMX_TIME=$(echo "$END_RMX - $START_RMX" | bc)

    # Calculate speedup
    SPEEDUP=$(echo "scale=2; $RM_TIME / $RMX_TIME" | bc)

    # Print results
    echo ""
    echo -e "${GREEN}Results:${NC}"
    echo -e "  rm time:   ${RM_TIME}s"
    echo -e "  rmx time:  ${RMX_TIME}s"
    echo -e "  ${GREEN}Speedup:   ${SPEEDUP}x${NC}"

    # Cleanup
    rm -rf "${dir}_rm" "${dir}_rmx" 2>/dev/null || true
}

echo -e "${YELLOW}Running benchmarks...${NC}"
echo ""

# Test 1: Small files (1000 files × 1KB)
echo -e "${BLUE}[1/5] Test: 1000 files × 1KB${NC}"
create_test_files "/tmp/test_1k_small" 1000 1024
run_benchmark "1000 small files (1KB each)" "/tmp/test_1k_small" "rm -rf" "-rf"

# Test 2: Medium files (500 files × 100KB)
echo -e "${BLUE}[2/5] Test: 500 files × 100KB${NC}"
create_test_files "/tmp/test_500_medium" 500 102400
run_benchmark "500 medium files (100KB each)" "/tmp/test_500_medium" "rm -rf" "-rf"

# Test 3: Large files (100 files × 1MB)
echo -e "${BLUE}[3/5] Test: 100 files × 1MB${NC}"
create_test_files "/tmp/test_100_large" 100 1048576
run_benchmark "100 large files (1MB each)" "/tmp/test_100_large" "rm -rf" "-rf"

# Test 4: Many small files (5000 files × 512B)
echo -e "${BLUE}[4/5] Test: 5000 files × 512B${NC}"
create_test_files "/tmp/test_5k_tiny" 5000 512
run_benchmark "5000 tiny files (512B each)" "/tmp/test_5k_tiny" "rm -rf" "-rf"

# Test 5: Nested directories
echo -e "${BLUE}[5/5] Test: Nested directories${NC}"
mkdir -p /tmp/test_nested
for i in {1..10}; do
    mkdir -p /tmp/test_nested/dir_$i
    for j in {1..50}; do
        dd if=/dev/urandom of=/tmp/test_nested/dir_$i/file_$j bs=10240 count=1 2>/dev/null
    done
done
run_benchmark "Nested directories (10 dirs × 50 files)" "/tmp/test_nested" "rm -rf" "-rf"

# Cleanup
echo ""
echo -e "${GREEN}Cleaning up...${NC}"
rm -rf /tmp/test_* 2>/dev/null || true

echo ""
echo -e "${BLUE}================================${NC}"
echo -e "${BLUE}  Benchmark Complete!${NC}"
echo -e "${BLUE}================================${NC}"
