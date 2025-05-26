#!/bin/bash

echo "🚀 NOCKCHAIN MINING STATISTICS VIEWER 🚀"
echo "========================================="
echo "Menampilkan statistics dari mining client yang sedang berjalan..."
echo ""

# Function to show real-time mining stats from logs
show_mining_stats() {
    echo "📊 MINING STATISTICS REAL-TIME"
    echo "==============================="

    # Check if mining process is running
    local mining_pid=$(pgrep -f "nockchain.*mine")
    if [ -z "$mining_pid" ]; then
        echo "❌ Mining process tidak ditemukan!"
        echo "Pastikan mining client sedang berjalan dengan:"
        echo "  RUST_LOG=info cargo run --release --bin nockchain -- --mine --mining-pubkey YOUR_KEY"
        return 1
    fi

    echo "✅ Mining process ditemukan (PID: $mining_pid)"
    echo ""

    # Monitor logs for statistics
    echo "🔍 Monitoring mining statistics dari logs..."
    echo "Press Ctrl+C untuk keluar"
    echo ""

    # Follow logs and filter for mining statistics
    if [ -d "logs" ] && [ "$(ls -A logs/mining_*.log 2>/dev/null)" ]; then
        # Use latest log file if available
        local latest_log=$(ls -t logs/mining_*.log 2>/dev/null | head -1)
        echo "📄 Reading from log file: $latest_log"
        tail -f "$latest_log" | grep -E "(NOCKCHAIN MINING STATS|Worker.*found a block|Mining worker.*started|🚀|⏱️|🔨|✅|❌|📊|⚡|👷)" --line-buffered --color=always
    elif command -v journalctl >/dev/null 2>&1; then
        # Use journalctl if available (systemd systems)
        echo "📄 Reading from system journal..."
        journalctl -f --since "1 minute ago" | grep -E "(NOCKCHAIN MINING STATS|Worker.*found a block|Mining worker.*started)" --line-buffered --color=always
    else
        # Fallback to monitoring process output
        echo "📄 Monitoring process activity..."

        # Monitor for mining activity
        while true; do
            # Check for recent mining activity
            local recent_activity=$(ps -p $mining_pid -o etime,pcpu,pmem --no-headers 2>/dev/null)
            if [ ! -z "$recent_activity" ]; then
                echo "$(date '+%H:%M:%S') - Mining Process: $recent_activity"
            fi

            # Check for any nockchain processes
            local all_nockchain=$(pgrep -f nockchain | wc -l)
            if [ "$all_nockchain" -gt 0 ]; then
                echo "$(date '+%H:%M:%S') - Active Nockchain processes: $all_nockchain"
            fi

            sleep 10
        done
    fi
}

# Function to show current system stats
show_system_stats() {
    echo ""
    echo "💻 SYSTEM RESOURCES"
    echo "==================="

    # CPU cores
    local cores=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "unknown")
    echo "⚙️  CPU Cores: $cores"

    # Memory usage
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        local memory_pressure=$(memory_pressure 2>/dev/null | grep "System-wide memory free percentage" | awk '{print $5}' | sed 's/%//')
        if [ ! -z "$memory_pressure" ]; then
            echo "🧠 Memory Free: ${memory_pressure}%"
        fi
    else
        # Linux
        local mem_info=$(free -m 2>/dev/null | grep "^Mem:")
        if [ ! -z "$mem_info" ]; then
            local total=$(echo $mem_info | awk '{print $2}')
            local used=$(echo $mem_info | awk '{print $3}')
            local percent=$((used * 100 / total))
            echo "🧠 Memory Usage: ${used}MB / ${total}MB (${percent}%)"
        fi
    fi

    # Mining process stats
    local mining_pid=$(pgrep -f "nockchain.*mine")
    if [ ! -z "$mining_pid" ]; then
        local process_stats=$(ps -p $mining_pid -o pcpu,pmem,etime --no-headers 2>/dev/null)
        if [ ! -z "$process_stats" ]; then
            echo "⛏️  Mining Process: CPU:$(echo $process_stats | awk '{print $1}')% MEM:$(echo $process_stats | awk '{print $2}')% Uptime:$(echo $process_stats | awk '{print $3}')"
        fi
    fi
}

# Function to show mining tips
show_tips() {
    echo ""
    echo "💡 CARA MENGAKSES STATISTICS"
    echo "============================"
    echo "1. Statistics otomatis ditampilkan setiap 30 detik di log"
    echo "2. Worker stats ditampilkan setiap 2 menit"
    echo "3. Jalankan mining dengan RUST_LOG=info untuk melihat semua output"
    echo "4. Gunakan script ini untuk monitoring real-time"
    echo ""
    echo "📋 CONTOH MENJALANKAN MINING:"
    echo "  cd nockchain"
    echo "  RUST_LOG=info cargo run --release --bin nockchain -- --mine --mining-pubkey YOUR_KEY"
    echo ""
    echo "🔍 MONITORING COMMANDS:"
    echo "  ./show_mining_stats.sh        # Script ini"
    echo "  htop                          # CPU usage"
    echo "  journalctl -f | grep mining   # System logs"
}

# Main function
main() {
    case "${1:-}" in
        "system")
            show_system_stats
            ;;
        "tips")
            show_tips
            ;;
        "help"|"-h"|"--help")
            echo "Usage: $0 [system|tips|help]"
            echo ""
            echo "Options:"
            echo "  system  Show system resource usage"
            echo "  tips    Show tips for accessing mining statistics"
            echo "  help    Show this help message"
            echo ""
            echo "Default: Show real-time mining statistics"
            ;;
        *)
            show_system_stats
            show_tips
            echo ""
            show_mining_stats
            ;;
    esac
}

main "$@"