#!/bin/bash

# Nockchain Mining Statistics Monitor
# Script untuk menampilkan statistik mining real-time

echo "🚀 NOCKCHAIN MINING STATISTICS MONITOR 🚀"
echo "=========================================="

# Function to get mining process info
get_mining_info() {
    local nockchain_pids=$(pgrep -f "nockchain.*mine" 2>/dev/null)
    if [ -z "$nockchain_pids" ]; then
        echo "❌ No mining processes found"
        return 1
    fi

    echo "✅ Mining processes found: $nockchain_pids"
    return 0
}

# Function to get system stats
get_system_stats() {
    echo ""
    echo "💻 SYSTEM RESOURCES"
    echo "==================="

    # CPU usage
    local cpu_usage=$(top -l 1 -n 0 | grep "CPU usage" | awk '{print $3}' | sed 's/%//')
    echo "🔥 CPU Usage: ${cpu_usage}%"

    # Memory usage
    local memory_info=$(vm_stat | grep -E "(free|active|inactive|wired)" | awk '{print $3}' | sed 's/\.//')
    local page_size=4096
    local free_pages=$(echo "$memory_info" | sed -n '1p')
    local active_pages=$(echo "$memory_info" | sed -n '2p')
    local inactive_pages=$(echo "$memory_info" | sed -n '3p')
    local wired_pages=$(echo "$memory_info" | sed -n '4p')

    local total_memory=$((($free_pages + $active_pages + $inactive_pages + $wired_pages) * $page_size / 1024 / 1024))
    local used_memory=$((($active_pages + $inactive_pages + $wired_pages) * $page_size / 1024 / 1024))
    local memory_percent=$((used_memory * 100 / total_memory))

    echo "🧠 Memory Usage: ${used_memory}MB / ${total_memory}MB (${memory_percent}%)"

    # Core count
    local core_count=$(sysctl -n hw.ncpu)
    echo "⚙️  CPU Cores: $core_count"
}

# Function to monitor mining logs
monitor_mining_logs() {
    echo ""
    echo "📊 MINING ACTIVITY (Last 10 entries)"
    echo "===================================="

    # Look for mining-related log entries
    if [ -f "nockchain.log" ]; then
        echo "📄 From nockchain.log:"
        tail -10 nockchain.log | grep -E "(mining|worker|block|attempt)" | tail -5
    fi

    # Check for recent mining activity in system logs
    echo ""
    echo "🔍 Recent Mining Activity:"
    ps aux | grep -E "(nockchain|mining)" | grep -v grep | head -5
}

# Function to get network stats
get_network_stats() {
    echo ""
    echo "🌐 NETWORK STATUS"
    echo "================="

    # Check for libp2p connections
    local connections=$(lsof -i -P | grep -E "(nockchain|libp2p)" | wc -l | tr -d ' ')
    echo "🔗 Active Connections: $connections"

    # Check for specific Nockchain ports
    local nockchain_ports=$(lsof -i -P | grep nockchain | awk '{print $9}' | sort | uniq)
    if [ ! -z "$nockchain_ports" ]; then
        echo "🚪 Listening Ports: $nockchain_ports"
    fi
}

# Function to estimate mining performance
estimate_performance() {
    echo ""
    echo "⚡ PERFORMANCE ESTIMATION"
    echo "========================"

    # Get process start time and calculate uptime
    local nockchain_pid=$(pgrep -f "nockchain.*mine" | head -1)
    if [ ! -z "$nockchain_pid" ]; then
        local start_time=$(ps -o lstart= -p $nockchain_pid 2>/dev/null)
        if [ ! -z "$start_time" ]; then
            echo "🕐 Mining Started: $start_time"
        fi

        # CPU and memory usage for mining process
        local process_stats=$(ps -o pid,pcpu,pmem,time -p $nockchain_pid 2>/dev/null | tail -1)
        if [ ! -z "$process_stats" ]; then
            echo "📈 Process Stats: $process_stats"
        fi
    fi
}

# Function to show mining tips
show_mining_tips() {
    echo ""
    echo "💡 MINING OPTIMIZATION TIPS"
    echo "==========================="
    echo "• Ensure all CPU cores are utilized"
    echo "• Monitor memory usage to avoid swapping"
    echo "• Check network connectivity for peer synchronization"
    echo "• Keep system temperature under control"
    echo "• Use SSD storage for better I/O performance"
}

# Main monitoring loop
main() {
    while true; do
        clear
        echo "🚀 NOCKCHAIN MINING STATISTICS MONITOR 🚀"
        echo "=========================================="
        echo "⏰ $(date)"
        echo ""

        # Check if mining is running
        if get_mining_info; then
            get_system_stats
            get_network_stats
            monitor_mining_logs
            estimate_performance
        else
            echo ""
            echo "🔧 TROUBLESHOOTING"
            echo "=================="
            echo "1. Make sure nockchain is running with --mine flag"
            echo "2. Check if mining key is properly configured"
            echo "3. Verify network connectivity"
            echo ""
            echo "To start mining:"
            echo "cd nockchain && cargo run --release -- --mine --mining-key YOUR_KEY"
        fi

        show_mining_tips

        echo ""
        echo "🔄 Refreshing in 10 seconds... (Press Ctrl+C to exit)"
        sleep 10
    done
}

# Handle script arguments
case "${1:-}" in
    "once")
        get_mining_info
        get_system_stats
        get_network_stats
        monitor_mining_logs
        estimate_performance
        ;;
    "help"|"-h"|"--help")
        echo "Usage: $0 [once|help]"
        echo ""
        echo "Options:"
        echo "  once    Run monitoring once and exit"
        echo "  help    Show this help message"
        echo ""
        echo "Default: Run continuous monitoring"
        ;;
    *)
        main
        ;;
esac