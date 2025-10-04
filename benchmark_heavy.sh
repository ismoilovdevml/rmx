#!/bin/bash

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  RMX vs RM - HEAVY LOAD BENCHMARK${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Results array
declare -a RESULTS

# Function to create test files
create_test_files() {
    local dir=$1
    local count=$2
    local size=$3
    local desc=$4

    echo -e "${YELLOW}Creating $count files of $size in $dir... ($desc)${NC}"
    mkdir -p "$dir"

    # Use parallel creation for speed
    if command -v parallel &> /dev/null; then
        seq 1 $count | parallel -j $(nproc) "dd if=/dev/zero of='$dir/file_{}' bs=$size count=1 2>/dev/null"
    else
        for i in $(seq 1 $count); do
            dd if=/dev/zero of="$dir/file_$i" bs=$size count=1 2>/dev/null
        done
    fi

    echo -e "${GREEN}✓ Created $count files${NC}"
}

# Function to create nested directory structure
create_nested_structure() {
    local base_dir=$1
    local depth=$2
    local dirs_per_level=$3
    local files_per_dir=$4
    local file_size=$5

    echo -e "${YELLOW}Creating nested structure (depth=$depth, dirs=$dirs_per_level, files=$files_per_dir, size=$file_size)...${NC}"
    mkdir -p "$base_dir"

    create_nested_recursive "$base_dir" $depth $dirs_per_level $files_per_dir $file_size
    echo -e "${GREEN}✓ Created nested structure${NC}"
}

create_nested_recursive() {
    local current_dir=$1
    local remaining_depth=$2
    local dirs_per_level=$3
    local files_per_dir=$4
    local file_size=$5

    # Create files in current directory
    for i in $(seq 1 $files_per_dir); do
        dd if=/dev/zero of="$current_dir/file_$i.dat" bs=$file_size count=1 2>/dev/null
    done

    # Create subdirectories if depth remains
    if [ $remaining_depth -gt 0 ]; then
        for i in $(seq 1 $dirs_per_level); do
            local subdir="$current_dir/subdir_$i"
            mkdir -p "$subdir"
            create_nested_recursive "$subdir" $((remaining_depth - 1)) $dirs_per_level $files_per_dir $file_size
        done
    fi
}

# Function to run benchmark
run_benchmark() {
    local test_name=$1
    local dir=$2
    local rm_flags=$3
    local rmx_flags=$4

    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  $test_name${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    # Get directory info
    local file_count=$(find "$dir" -type f | wc -l | tr -d ' ')
    local dir_size=$(du -sh "$dir" 2>/dev/null | cut -f1)

    echo -e "Files: $file_count | Total size: $dir_size"
    echo ""

    # Copy directory for both tests
    echo -e "${YELLOW}Preparing test directories...${NC}"
    cp -r "$dir" "${dir}_rm"
    cp -r "$dir" "${dir}_rmx"
    sync # Ensure disk cache is flushed

    # Benchmark rm
    echo -e "${YELLOW}Testing standard rm...${NC}"
    START_RM=$(date +%s.%N)
    rm $rm_flags "${dir}_rm" 2>/dev/null || true
    END_RM=$(date +%s.%N)
    RM_TIME=$(echo "$END_RM - $START_RM" | bc)

    # Clear cache
    sync
    sleep 1

    # Benchmark rmx
    echo -e "${YELLOW}Testing rmx...${NC}"
    START_RMX=$(date +%s.%N)
    ./target/release/rmx $rmx_flags "${dir}_rmx" 2>/dev/null || true
    END_RMX=$(date +%s.%N)
    RMX_TIME=$(echo "$END_RMX - $START_RMX" | bc)

    # Calculate speedup
    if (( $(echo "$RMX_TIME > 0" | bc -l) )); then
        SPEEDUP=$(echo "scale=2; $RM_TIME / $RMX_TIME" | bc)
    else
        SPEEDUP="∞"
    fi

    # Print results
    echo ""
    echo -e "${GREEN}┌─────────────────────────────────────┐${NC}"
    echo -e "${GREEN}│           RESULTS                   │${NC}"
    echo -e "${GREEN}├─────────────────────────────────────┤${NC}"
    printf "${GREEN}│${NC} %-20s ${YELLOW}%13s${NC} ${GREEN}│${NC}\n" "rm time:" "${RM_TIME}s"
    printf "${GREEN}│${NC} %-20s ${CYAN}%13s${NC} ${GREEN}│${NC}\n" "rmx time:" "${RMX_TIME}s"
    printf "${GREEN}│${NC} %-20s ${RED}%13s${NC} ${GREEN}│${NC}\n" "Speedup:" "${SPEEDUP}x"
    echo -e "${GREEN}└─────────────────────────────────────┘${NC}"

    # Save results
    RESULTS+=("$test_name|$file_count|$dir_size|$RM_TIME|$RMX_TIME|$SPEEDUP")

    # Cleanup
    rm -rf "${dir}_rm" "${dir}_rmx" 2>/dev/null || true
}

# Start benchmarks
echo -e "${YELLOW}Starting heavy load benchmarks...${NC}"
echo ""

# Test 1: Very many small files (50,000 files × 1KB)
echo -e "${BLUE}[1/8] EXTREME TEST: 50,000 small files${NC}"
create_test_files "/tmp/rmx_bench_50k_small" 50000 1024 "50K × 1KB"
run_benchmark "50K Small Files (1KB)" "/tmp/rmx_bench_50k_small" "-rf" "-rf"
rm -rf /tmp/rmx_bench_50k_small

# Test 2: Large files (1,000 files × 10MB)
echo -e "${BLUE}[2/8] HEAVY TEST: 1,000 large files${NC}"
create_test_files "/tmp/rmx_bench_1k_large" 1000 10485760 "1K × 10MB"
run_benchmark "1K Large Files (10MB)" "/tmp/rmx_bench_1k_large" "-rf" "-rf"
rm -rf /tmp/rmx_bench_1k_large

# Test 3: Very large files (100 files × 100MB)
echo -e "${BLUE}[3/8] EXTREME TEST: 100 very large files${NC}"
create_test_files "/tmp/rmx_bench_100_xlarge" 100 104857600 "100 × 100MB"
run_benchmark "100 Very Large Files (100MB)" "/tmp/rmx_bench_100_xlarge" "-rf" "-rf"
rm -rf /tmp/rmx_bench_100_xlarge

# Test 4: Many tiny files (100,000 files × 100B)
echo -e "${BLUE}[4/8] STRESS TEST: 100,000 tiny files${NC}"
create_test_files "/tmp/rmx_bench_100k_tiny" 100000 100 "100K × 100B"
run_benchmark "100K Tiny Files (100B)" "/tmp/rmx_bench_100k_tiny" "-rf" "-rf"
rm -rf /tmp/rmx_bench_100k_tiny

# Test 5: Deep nested structure (depth=5, 3 dirs/level, 20 files/dir)
echo -e "${BLUE}[5/8] DEEP NESTING TEST: 5-level deep structure${NC}"
create_nested_structure "/tmp/rmx_bench_deep" 5 3 20 10240
run_benchmark "Deep Nested (5 levels × 3 dirs × 20 files)" "/tmp/rmx_bench_deep" "-rf" "-rf"
rm -rf /tmp/rmx_bench_deep

# Test 6: Wide flat structure (10,000 files in one directory)
echo -e "${BLUE}[6/8] WIDE FLAT TEST: 10,000 files in single directory${NC}"
create_test_files "/tmp/rmx_bench_wide_flat" 10000 5120 "10K × 5KB"
run_benchmark "Wide Flat (10K files in 1 dir)" "/tmp/rmx_bench_wide_flat" "-rf" "-rf"
rm -rf /tmp/rmx_bench_wide_flat

# Test 7: Mixed sizes (1000 files with varying sizes)
echo -e "${BLUE}[7/8] MIXED TEST: Various file sizes${NC}"
mkdir -p /tmp/rmx_bench_mixed
for i in {1..300}; do
    dd if=/dev/zero of="/tmp/rmx_bench_mixed/small_$i" bs=1024 count=1 2>/dev/null
done
for i in {1..300}; do
    dd if=/dev/zero of="/tmp/rmx_bench_mixed/medium_$i" bs=102400 count=1 2>/dev/null
done
for i in {1..300}; do
    dd if=/dev/zero of="/tmp/rmx_bench_mixed/large_$i" bs=1048576 count=1 2>/dev/null
done
for i in {1..100}; do
    dd if=/dev/zero of="/tmp/rmx_bench_mixed/xlarge_$i" bs=10485760 count=1 2>/dev/null
done
run_benchmark "Mixed Sizes (1K files: 300×1KB + 300×100KB + 300×1MB + 100×10MB)" "/tmp/rmx_bench_mixed" "-rf" "-rf"
rm -rf /tmp/rmx_bench_mixed

# Test 8: Real-world simulation (node_modules style)
echo -e "${BLUE}[8/8] REAL-WORLD TEST: node_modules simulation${NC}"
mkdir -p /tmp/rmx_bench_nodemodules
for pkg in {1..50}; do
    mkdir -p "/tmp/rmx_bench_nodemodules/package_$pkg/src"
    mkdir -p "/tmp/rmx_bench_nodemodules/package_$pkg/dist"
    mkdir -p "/tmp/rmx_bench_nodemodules/package_$pkg/node_modules"

    for f in {1..20}; do
        dd if=/dev/zero of="/tmp/rmx_bench_nodemodules/package_$pkg/src/file_$f.js" bs=5120 count=1 2>/dev/null
    done
    for f in {1..10}; do
        dd if=/dev/zero of="/tmp/rmx_bench_nodemodules/package_$pkg/dist/bundle_$f.js" bs=51200 count=1 2>/dev/null
    done
done
run_benchmark "Real-world (node_modules: 50 packages)" "/tmp/rmx_bench_nodemodules" "-rf" "-rf"
rm -rf /tmp/rmx_bench_nodemodules

# Print summary
echo ""
echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                          BENCHMARK SUMMARY                                 ║${NC}"
echo -e "${BLUE}╠════════════════════════════════════════════════════════════════════════════╣${NC}"
printf "${BLUE}║${NC} %-35s ${BLUE}│${NC} %-8s ${BLUE}│${NC} %-8s ${BLUE}│${NC} %-8s ${BLUE}║${NC}\n" "Test Name" "Files" "rm(s)" "rmx(s)" "Speedup"
echo -e "${BLUE}╠════════════════════════════════════════════════════════════════════════════╣${NC}"

total_speedup=0
test_count=0

for result in "${RESULTS[@]}"; do
    IFS='|' read -r name files size rm_time rmx_time speedup <<< "$result"

    # Color code speedup
    if (( $(echo "$speedup > 3" | bc -l 2>/dev/null || echo 0) )); then
        speedup_color=$GREEN
    elif (( $(echo "$speedup > 2" | bc -l 2>/dev/null || echo 0) )); then
        speedup_color=$CYAN
    elif (( $(echo "$speedup > 1" | bc -l 2>/dev/null || echo 0) )); then
        speedup_color=$YELLOW
    else
        speedup_color=$RED
    fi

    printf "${BLUE}║${NC} %-35s ${BLUE}│${NC} %-8s ${BLUE}│${NC} %-8s ${BLUE}│${NC} ${speedup_color}%-7s${NC}x ${BLUE}║${NC}\n" \
        "${name:0:35}" "$files" "$rm_time" "$rmx_time" "$speedup"

    if [[ $speedup != "∞" ]]; then
        total_speedup=$(echo "$total_speedup + $speedup" | bc)
        test_count=$((test_count + 1))
    fi
done

echo -e "${BLUE}╠════════════════════════════════════════════════════════════════════════════╣${NC}"

if [ $test_count -gt 0 ]; then
    avg_speedup=$(echo "scale=2; $total_speedup / $test_count" | bc)
    printf "${BLUE}║${NC} %-62s ${GREEN}%-10s${NC} ${BLUE}║${NC}\n" "AVERAGE SPEEDUP:" "${avg_speedup}x"
fi

echo -e "${BLUE}╚════════════════════════════════════════════════════════════════════════════╝${NC}"

# System info
echo ""
echo -e "${CYAN}System Information:${NC}"
echo -e "  OS: $(uname -s)"
echo -e "  Kernel: $(uname -r)"
echo -e "  CPU Cores: $(nproc 2>/dev/null || sysctl -n hw.ncpu)"
echo -e "  rmx version: $(rmx version 2>/dev/null || echo 'unknown')"

# Cleanup
echo ""
echo -e "${GREEN}Cleaning up temporary files...${NC}"
rm -rf /tmp/rmx_bench_* 2>/dev/null || true

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    BENCHMARK COMPLETE!                                     ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════════════════════╝${NC}"
