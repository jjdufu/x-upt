#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ "$(basename "$SCRIPT_DIR")" = "scripts" ]]; then
    PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
    PROJECT_ROOT="$SCRIPT_DIR"
fi

PID_FILE="${PROJECT_ROOT}/x-upt.pid"
DATA_DIR="${X_UPT_DATA_DIR:-${PROJECT_ROOT}/data}"
CONSOLE_LOG="${DATA_DIR}/logs/x-upt.console.log"
ADMIN_LISTEN="${X_UPT_ADMIN_LISTEN:-0.0.0.0:18768}"

usage() {
    echo "Usage: $0 [start|status|stop|restart]"
    echo ""
    echo "Commands:"
    echo "  start    Start x-upt (default)"
    echo "  status   Check service status"
    echo "  stop     Stop x-upt"
    echo "  restart  Restart x-upt"
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

is_x_upt() {
    local pid="$1"
    if ! kill -0 "$pid" 2>/dev/null; then
        return 1
    fi
    if [[ -r "/proc/${pid}/comm" ]]; then
        local comm
        comm="$(tr -d '\0' < "/proc/${pid}/comm" 2>/dev/null || true)"
        if [[ "$comm" != "x-upt" ]]; then
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
    if [[ -n "${pid}" ]] && is_x_upt "$pid"; then
        printf '%s' "$pid"
        return 0
    fi
    return 1
}

resolve_binary() {
    local candidate
    for candidate in "${PROJECT_ROOT}/x-upt" "${PROJECT_ROOT}/target/release/x-upt" "${PROJECT_ROOT}/target/debug/x-upt"; do
        if [[ -x "$candidate" ]]; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    return 1
}

start_service() {
    local binary pid
    if pid="$(running_pid)"; then
        echo "x-upt is already running (PID ${pid})."
        exit 0
    fi
    rm -f "$PID_FILE"
    binary="$(resolve_binary || true)"
    if [[ -z "$binary" ]]; then
        echo "Error: Binary not found. Build 'x-upt' first."
        exit 1
    fi

    echo "==> Starting x-upt..."
    mkdir -p "${DATA_DIR}/logs"
    cd "$PROJECT_ROOT"
    nohup "$binary" serve --admin-listen "$ADMIN_LISTEN" --data-dir "$DATA_DIR" >> "$CONSOLE_LOG" 2>&1 &
    echo $! > "$PID_FILE"

    pid="$(cat "$PID_FILE")"
    for _ in 1 2 3; do
        sleep 1
        if ! is_x_upt "$pid"; then
            echo "    Failed to start. Check ${CONSOLE_LOG}."
            rm -f "$PID_FILE"
            exit 1
        fi
    done

    echo "    PID: ${pid}"
    echo "    Binary: ${binary}"
    echo "    Admin: ${ADMIN_LISTEN}"
    echo "    Data: ${DATA_DIR}"
    echo "    Console: ${CONSOLE_LOG}"
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
        echo "==> Stopping x-upt (PID ${pid})..."
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
        echo "x-upt is not running (stale PID file)."
        rm -f "$PID_FILE"
    else
        echo "x-upt is not running."
    fi
}

status_service() {
    local pid
    if pid="$(running_pid)"; then
        echo "x-upt is running. PID: ${pid}"
        exit 0
    fi
    if [[ -f "$PID_FILE" ]]; then
        echo "x-upt is not running (stale PID file)."
    else
        echo "x-upt is not running."
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
