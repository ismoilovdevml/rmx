# RMX Benchmark Guide

## Running Benchmarks in Docker

### Prerequisites
- Docker installed
- Docker Compose installed

### Quick Start

1. **Build and run benchmark:**
```bash
docker-compose up --build
```

2. **Or run manually:**
```bash
# Build the image
docker build -f Dockerfile.benchmark -t rmx-benchmark .

# Run benchmarks
docker run --rm rmx-benchmark
```

### Benchmark Tests

The benchmark suite includes 5 comprehensive tests:

1. **Small Files Test**
   - 1000 files × 1KB each
   - Tests performance on many small files

2. **Medium Files Test**
   - 500 files × 100KB each
   - Tests balanced workload

3. **Large Files Test**
   - 100 files × 1MB each
   - Tests performance on larger files

4. **Many Tiny Files Test**
   - 5000 files × 512B each
   - Stress test for file count

5. **Nested Directories Test**
   - 10 directories × 50 files each
   - Tests recursive deletion performance

### Expected Results

RMX should show:
- **3-5x faster** on large directories with many files
- **2-3x faster** on nested directory structures
- **Similar or better** performance on all test scenarios
- **Lower memory usage** compared to standard `rm`

### Performance Metrics

Each test measures:
- **Execution time** (seconds)
- **Speedup factor** (rmx vs rm)
- **Files/Directories deleted**
- **Total size processed**

### Example Output

```
================================
  RMX vs RM Benchmark Tests
================================

[1/5] Test: 1000 files × 1KB
Creating 1000 files of size 1024 in /tmp/test_1k_small...
Created 1000 files

--- 1000 small files (1KB each) ---
Testing rm...
Testing rmx...

Results:
  rm time:   1.25s
  rmx time:  0.35s
  Speedup:   3.57x
```

## Manual Testing

You can also run manual tests:

### Test 1: Basic file removal
```bash
# Create test files
mkdir test_dir
for i in {1..100}; do
    echo "test" > test_dir/file_$i
done

# Compare performance
time rm -rf test_dir_copy1
time rmx -rf test_dir_copy2
```

### Test 2: Interactive mode
```bash
rmx -i test_file.txt  # Prompts before deletion
```

### Test 3: Verbose mode
```bash
rmx -v -r test_dir/  # Shows each file being deleted
```

### Test 4: Multiple paths
```bash
rmx file1.txt file2.txt dir1/ dir2/
```

## Performance Optimization

RMX achieves better performance through:

1. **Parallel Processing**
   - Uses Rayon for multi-threaded deletion
   - Processes multiple files simultaneously

2. **Optimized I/O**
   - Minimizes system calls
   - Batch operations where possible

3. **Smart Memory Management**
   - Uses iterators instead of collecting
   - filter_map for efficient processing

4. **Compiler Optimizations**
   - Link-Time Optimization (LTO)
   - Maximum optimization level
   - Strip symbols for smaller binary

## Troubleshooting

If benchmarks fail:

1. **Permission issues:**
```bash
docker run --rm --privileged rmx-benchmark
```

2. **Not enough space:**
```bash
# Clean up Docker
docker system prune -a
```

3. **Build errors:**
```bash
# Clean rebuild
docker-compose down
docker-compose build --no-cache
docker-compose up
```

## Contributing

To add new benchmark tests:

1. Edit `benchmark.sh`
2. Add new test case following the pattern
3. Rebuild Docker image
4. Run tests

## License

MIT License - see LICENSE file
