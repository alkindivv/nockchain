#!/bin/bash

echo "🚀 NOCKCHAIN OPTIMIZED MINING LAUNCHER 🚀"
echo "=========================================="

# Check if mining key is provided
if [ -z "$1" ]; then
    echo "❌ Mining key diperlukan!"
    echo ""
    echo "Usage: $0 <mining-pubkey> [options]"
    echo ""
    echo "Example:"
    echo "  $0 your-mining-pubkey-here"
    echo "  $0 your-mining-pubkey-here --verbose"
    echo ""
    exit 1
fi

MINING_KEY="$1"
VERBOSE_MODE=""

# Check for verbose flag
if [ "$2" = "--verbose" ] || [ "$2" = "-v" ]; then
    VERBOSE_MODE="debug"
    echo "🔍 Verbose mode enabled"
fi

# Set optimal logging configuration
if [ "$VERBOSE_MODE" = "debug" ]; then
    export RUST_LOG="debug,nockchain=debug,zkvm_jetpack=info"
    echo "📋 Log Level: DEBUG (semua detail)"
else
    export RUST_LOG="info,nockchain=info"
    echo "📋 Log Level: INFO (statistics dan events penting)"
fi

# Set performance environment variables
export RUST_BACKTRACE=0
export RAYON_NUM_THREADS=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "4")

echo "⚙️  CPU Cores: $RAYON_NUM_THREADS"
echo "🔑 Mining Key: ${MINING_KEY:0:20}..."
echo ""

# Build if needed
if [ ! -f "target/release/nockchain" ]; then
    echo "🔨 Building optimized client..."
    cargo build --release
    if [ $? -ne 0 ]; then
        echo "❌ Build failed!"
        exit 1
    fi
    echo "✅ Build completed"
fi

# Create log directory
mkdir -p logs
LOG_FILE="logs/mining_$(date +%Y%m%d_%H%M%S).log"

echo "📝 Logs akan disimpan di: $LOG_FILE"
echo ""

# Function to show real-time stats
show_stats_help() {
    echo "💡 CARA MELIHAT STATISTICS REAL-TIME:"
    echo "======================================"
    echo "1. Statistics otomatis muncul setiap 30 detik di output"
    echo "2. Worker details muncul setiap 2 menit"
    echo "3. Buka terminal baru dan jalankan:"
    echo "   tail -f $LOG_FILE | grep 'NOCKCHAIN MINING STATS'"
    echo "4. Atau gunakan script monitoring:"
    echo "   ./show_mining_stats.sh"
    echo ""
}

show_stats_help

# Start mining with logging
echo "🚀 Starting optimized mining client..."
echo "Press Ctrl+C to stop mining"
echo ""

# Run mining with both console output and file logging
cargo run --release --bin nockchain -- --mine --mining-pubkey "$MINING_KEY" 2>&1 | tee "$LOG_FILE"

echo ""
echo "⛏️  Mining stopped"
echo "📝 Logs tersimpan di: $LOG_FILE"