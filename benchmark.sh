#!/bin/bash

set -e

echo "========================================="
echo "🚀 RMX vs RM Performance Benchmark"
echo "========================================="
echo ""

TEST_DIR="/test/benchmark"
FILES_COUNT=10000
FILE_SIZE_KB=100

generate_test_files() {
    local dir=$1
    local count=$2
    local size=$3

    echo "📦 Generating $count files ($size KB each) in $dir..."
    mkdir -p "$dir"

    for i in $(seq 1 $count); do
        dd if=/dev/urandom of="$dir/file_$i.dat" bs=1024 count=$size 2>/dev/null
    done

    local total_size=$(du -sh "$dir" | cut -f1)
    local file_count=$(find "$dir" -type f | wc -l)
    echo "✓ Generated $file_count files, Total size: $total_size"
    echo ""
}

benchmark_rmx() {
    local dir=$1
    echo "⚡ Testing RMX..."
    echo "Command: rmx $dir"
    echo ""

    /usr/bin/time -f "Real time: %E\nCPU usage: %P\nMax memory: %M KB" \
        rmx "$dir" 2>&1 | tee /tmp/rmx_output.txt

    echo ""
}

benchmark_rm() {
    local dir=$1
    echo "🗑️  Testing standard rm..."
    echo "Command: rm -rf $dir"
    echo ""

    /usr/bin/time -f "Real time: %E\nCPU usage: %P\nMax memory: %M KB" \
        sh -c "rm -rf '$dir'" 2>&1

    echo ""
}

echo "========================================="
echo "Test 1: Small files (10,000 files × 100KB)"
echo "========================================="
echo ""

# Test RMX
generate_test_files "${TEST_DIR}_rmx" $FILES_COUNT $FILE_SIZE_KB
benchmark_rmx "${TEST_DIR}_rmx"

# Test RM
generate_test_files "${TEST_DIR}_rm" $FILES_COUNT $FILE_SIZE_KB
benchmark_rm "${TEST_DIR}_rm"

echo "========================================="
echo "Test 2: Large files (1,000 files × 10MB)"
echo "========================================="
echo ""

# Test RMX
generate_test_files "${TEST_DIR}_rmx_large" 1000 10240
benchmark_rmx "${TEST_DIR}_rmx_large"

# Test RM
generate_test_files "${TEST_DIR}_rm_large" 1000 10240
benchmark_rm "${TEST_DIR}_rm_large"

echo "========================================="
echo "Test 3: Many small files (50,000 files × 10KB)"
echo "========================================="
echo ""

# Test RMX
generate_test_files "${TEST_DIR}_rmx_many" 50000 10
benchmark_rmx "${TEST_DIR}_rmx_many"

# Test RM
generate_test_files "${TEST_DIR}_rm_many" 50000 10
benchmark_rm "${TEST_DIR}_rm_many"

echo "========================================="
echo "✅ Benchmark completed!"
echo "========================================="