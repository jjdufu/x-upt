#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ "$(basename "$SCRIPT_DIR")" = "scripts" ]]; then
    PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
    PROJECT_ROOT="$SCRIPT_DIR"
fi

PID_FILE="${PROJECT_ROOT}/upupt.pid"
CONSOLE_LOG="${PROJECT_ROOT}/logs/upupt.console.log"

usage() {
    echo "Usage: $0 [start|status|stop|restart]"
    echo ""
    echo "Commands:"
    echo "  start    Start the service (default)"
    echo "  status   Check service status"
    echo "  stop     Stop the service"
    echo "  restart  Restart the service"
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

read_pid() {
    if [[ ! -f "$PID_FILE" ]]; then
        return 1
    fi
    local pid
    pid="$(tr -d '[:space:]' < "$PID_FILE" || true)"
    if [[ ! "$pid" =~ ^[1-9][0-9]*$ ]]; then
        return 1
    fi
    printf '%s' "$pid"
}

proc_state() {
    local pid="$1"
    local stat rest
    stat="$(cat "/proc/${pid}/stat" 2>/dev/null || true)"
    if [[ -z "$stat" ]]; then
        return 0
    fi
    rest="${stat##*)}"
    rest="${rest#"${rest%%[![:space:]]*}"}"
    printf '%s' "${rest:0:1}"
}

is_upupt() {
    local pid="$1"
    if ! kill -0 "$pid" 2>/dev/null; then
        return 1
    fi
    if [[ -r "/proc/${pid}/comm" ]]; then
        local comm
        comm="$(tr -d '\0' < "/proc/${pid}/comm" 2>/dev/null || true)"
        if [[ "$comm" != "upupt" ]]; then
            return 1
        fi
    fi
    local state
    state="$(proc_state "$pid")"
    if [[ "$state" == "Z" ]]; then
        return 1
    fi
    return 0
}

running_pid() {
    local pid
    pid="$(read_pid || true)"
    if [[ -n "${pid}" ]] && is_upupt "$pid"; then
        printf '%s' "$pid"
        return 0
    fi
    return 1
}

resolve_binary() {
    local candidate
    for candidate in "${PROJECT_ROOT}/upupt" "${PROJECT_ROOT}/target/release/upupt"; do
        if [[ -x "$candidate" ]]; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    return 1
}

binary_config() {
    local bin_dir
    bin_dir="$(cd "$(dirname "$1")" && pwd)"
    printf '%s' "${bin_dir}/config/app.toml"
}

require_start_files() {
    local binary config
    binary="$(resolve_binary || true)"
    if [[ -z "$binary" ]]; then
        echo "Error: Binary not found. Place 'upupt' in ${PROJECT_ROOT}/ or run 'cargo build --release' first."
        exit 1
    fi
    config="$(binary_config "$binary")"
    if [[ ! -f "$config" ]]; then
        echo "Error: ${config} not found."
        echo "Config is read next to the binary. Put app.toml there, or place the binary at ${PROJECT_ROOT}/upupt beside config/."
        exit 1
    fi
}

start_service() {
    local binary pid i
    if pid="$(running_pid)"; then
        echo "UPUPT is already running (PID ${pid})."
        exit 0
    fi
    rm -f "$PID_FILE"
    require_start_files
    binary="$(resolve_binary)"

    echo "==> Starting UPUPT..."
    mkdir -p "${PROJECT_ROOT}/logs"
    cd "$PROJECT_ROOT"
    nohup "$binary" >> "$CONSOLE_LOG" 2>&1 &
    echo $! > "$PID_FILE"

    pid="$(cat "$PID_FILE")"
    for i in 1 2 3; do
        sleep 1
        if ! is_upupt "$pid"; then
            echo "    Failed to start. Check ${CONSOLE_LOG} and the log file in config/app.toml ([log] section)."
            rm -f "$PID_FILE"
            exit 1
        fi
    done

    echo "    PID: ${pid}"
    echo "    Binary: ${binary}"
    echo "    Console: ${CONSOLE_LOG}"
    echo "    Log: configured in the binary's config/app.toml ([log] section)"
}

wait_gone() {
    local pid="$1"
    local seconds="$2"
    local i
    for ((i = 1; i <= seconds; i++)); do
        if ! kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
        if [[ "$(proc_state "$pid")" == "Z" ]]; then
            return 0
        fi
        sleep 1
    done
    return 1
}

stop_service() {
    local pid
    if pid="$(running_pid)"; then
        echo "==> Stopping UPUPT (PID ${pid})..."
        kill -TERM "$pid" 2>/dev/null || true
        if ! wait_gone "$pid" 12; then
            echo "    Still running after 12s, sending SIGKILL..."
            kill -KILL "$pid" 2>/dev/null || true
            if ! wait_gone "$pid" 2; then
                echo "    Failed to stop PID ${pid}."
                exit 1
            fi
        fi
        rm -f "$PID_FILE"
        echo "    Stopped."
        return 0
    fi

    if [[ -f "$PID_FILE" ]]; then
        echo "UPUPT is not running (stale PID file)."
        rm -f "$PID_FILE"
    else
        echo "UPUPT is not running."
    fi
}

status_service() {
    local pid
    if pid="$(running_pid)"; then
        echo "UPUPT is running. PID: ${pid}"
        exit 0
    fi
    if [[ -f "$PID_FILE" ]]; then
        echo "UPUPT is not running (stale PID file)."
    else
        echo "UPUPT is not running."
    fi
    exit 1
}

case "${1:-start}" in
    start)
        start_service
        ;;
    status)
        status_service
        ;;
    stop)
        stop_service
        ;;
    restart)
        stop_service
        start_service
        ;;
    *)
        echo "Unknown command: $1"
        usage
        exit 1
        ;;
esac
