#!/bin/bash

# Optimized Nockchain Build Script
# This script builds Nockchain with maximum performance optimizations

set -e

echo "🚀 Starting Optimized Nockchain Build"
echo "======================================"

# Check CPU capabilities
echo "🔍 Detecting CPU capabilities..."
if grep -q avx2 /proc/cpuinfo; then
    echo "✅ AVX2 support detected"
    AVX2_SUPPORT=true
else
    echo "❌ AVX2 not supported"
    AVX2_SUPPORT=false
fi

if grep -q bmi2 /proc/cpuinfo; then
    echo "✅ BMI2 support detected"
    BMI2_SUPPORT=true
else
    echo "❌ BMI2 not supported"
    BMI2_SUPPORT=false
fi

# Set environment variables for maximum performance
export CARGO_PROFILE_RELEASE_LTO=fat
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_PANIC=abort
export CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=false
export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false

# CPU-specific optimizations
if [ "$AVX2_SUPPORT" = true ] && [ "$BMI2_SUPPORT" = true ]; then
    echo "🎯 Enabling maximum CPU optimizations (AVX2 + BMI2)"
    export RUSTFLAGS="-C target-cpu=native -C target-feature=+avx2,+bmi2,+adx,+aes -C link-arg=-fuse-ld=lld"
elif [ "$AVX2_SUPPORT" = true ]; then
    echo "🎯 Enabling AVX2 optimizations"
    export RUSTFLAGS="-C target-cpu=native -C target-feature=+avx2,+aes -C link-arg=-fuse-ld=lld"
else
    echo "🎯 Using basic optimizations"
    export RUSTFLAGS="-C target-cpu=native -C link-arg=-fuse-ld=lld"
fi

# Memory optimization
echo "🧠 Configuring memory optimizations..."
export MALLOC_CONF="background_thread:true,metadata_thp:auto,dirty_decay_ms:30000,muzzy_decay_ms:30000"

# Parallel build configuration
NPROC=$(nproc)
PARALLEL_JOBS=$((NPROC > 8 ? 8 : NPROC))
echo "🔧 Using $PARALLEL_JOBS parallel jobs"

# Clean previous builds
echo "🧹 Cleaning previous builds..."
cargo clean

# Copy optimized Cargo.toml
# if [ -f "Cargo_optimized.toml" ]; then
#     echo "📋 Using optimized Cargo.toml"
#     cp Cargo.toml Cargo.toml.backup
#     cp Cargo_optimized.toml Cargo.toml
# fi

# Build hoonc first
echo "🔨 Building hoonc compiler..."
make install-hoonc

# Build optimized version
echo "🚀 Building optimized Nockchain..."
cargo build --release --jobs $PARALLEL_JOBS

# Build with mining profile if available
if grep -q "\[profile.mining\]" Cargo.toml; then
    echo "⛏️  Building with mining profile..."
    cargo build --profile mining --jobs $PARALLEL_JOBS
fi

# Install binaries
echo "📦 Installing optimized binaries..."
make install-nockchain
make install-nockchain-wallet

# Restore original Cargo.toml if we backed it up
# if [ -f "Cargo.toml.backup" ]; then
#     mv Cargo.toml.backup Cargo.toml
# fi

# Performance validation
echo "🧪 Running performance validation..."
if [ -f "target/release/nockchain" ]; then
    echo "✅ Nockchain binary built successfully"
    ls -lh target/release/nockchain

    # Check if binary has optimizations
    if command -v objdump >/dev/null 2>&1; then
        echo "🔍 Checking for SIMD instructions..."
        if objdump -d target/release/nockchain | grep -q "vpaddd\|vpaddq\|vmul"; then
            echo "✅ SIMD instructions found in binary"
        else
            echo "⚠️  No SIMD instructions detected"
        fi
    fi
fi

# Create optimized run script
cat > run_optimized_miner.sh << 'EOF'
#!/bin/bash

# Optimized Nockchain Miner Runner
# This script runs the miner with optimal settings

# CPU affinity and priority
export OMP_NUM_THREADS=$(nproc)
export RAYON_NUM_THREADS=$(nproc)

# Memory settings
export MALLOC_CONF="background_thread:true,metadata_thp:auto,dirty_decay_ms:30000,muzzy_decay_ms:30000"

# Rust runtime optimizations
export RUST_BACKTRACE=0
export RUST_LOG=info,nockchain=info

# Check if we have the mining profile binary
if [ -f "target/mining/nockchain" ]; then
    BINARY="target/mining/nockchain"
    echo "🚀 Using mining profile binary"
elif [ -f "target/release/nockchain" ]; then
    BINARY="target/release/nockchain"
    echo "🚀 Using release profile binary"
else
    echo "❌ No optimized binary found!"
    exit 1
fi

# Set CPU governor to performance if available
if [ -f /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]; then
    echo "⚡ Setting CPU governor to performance..."
    echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor > /dev/null 2>&1 || true
fi

# Disable CPU frequency scaling
echo "🔧 Disabling CPU frequency scaling..."
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo > /dev/null 2>&1 || true

# Run with high priority and CPU affinity
echo "⛏️  Starting optimized miner..."
exec nice -n -20 taskset -c 0-$(($(nproc)-1)) $BINARY "$@"
EOF

chmod +x run_optimized_miner.sh

echo ""
echo "🎉 Optimized build completed successfully!"
echo "======================================"
echo ""
echo "📋 Build Summary:"
echo "  • Binary location: target/release/nockchain"
echo "  • Optimizations: LTO=fat, target-cpu=native"
echo "  • SIMD support: $AVX2_SUPPORT"
echo "  • Parallel jobs: $PARALLEL_JOBS"
echo ""
echo "🚀 To run the optimized miner:"
echo "  ./run_optimized_miner.sh --mine --mining-pubkey YOUR_PUBKEY"
echo ""
echo "💡 Performance Tips:"
echo "  • Use dedicated mining machine"
echo "  • Ensure adequate cooling"
echo "  • Monitor CPU temperature"
echo "  • Use SSD for faster I/O"
echo "  • Close unnecessary applications"
echo ""