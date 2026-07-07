# shellcheck shell=bash
# Test harness for the bare-metal QEMU tests (qemu-tests/). `just up` starts the host
# services the guest talks to:
#
#   emulated LAN9118 NIC -- QEMU user-mode net (host 10.0.2.2)  -> mosquitto :41883,
#                                                                  http :48080,
#                                                                  diagslave -m tcp :41502
#   guest UART1 --pty-- diagslave -m rtu -a 1
#
# QEMU's built-in user-mode network stack needs no privileges or host daemons.
# Requires: socat, mosquitto, python3, qemu-system-arm, and diagslave (fetched by
# `just install-tools`).

run_dir := "/tmp/ferredge-qemu-harness"

# Show available recipes.
default:
    @just --list

# Download diagslave/modpoll and install them into /usr/local/bin.
install-tools:
    ./scripts/install-modbusdriver.sh

# Start the host-side services (idempotent: tears down any previous instance first).
up: down install-tools
    #!/usr/bin/env bash
    set -euo pipefail
    run="{{run_dir}}"
    mkdir -p "$run"

    # Modbus leg: pty pair bridging guest UART1 to diagslave.
    socat -d0 pty,link="$run/modbus-host",raw,echo=0 pty,link="$run/modbus-guest",raw,echo=0 \
        > "$run/socat-modbus.log" 2>&1 &
    echo $! > "$run/socat-modbus.pid"

    for link in modbus-host modbus-guest; do
        for _ in $(seq 50); do [ -e "$run/$link" ] && break; sleep 0.1; done
        [ -e "$run/$link" ] || { echo "socat pty $link did not appear" >&2; exit 1; }
    done

    diagslave -m rtu -a 1 "$run/modbus-host" > "$run/diagslave.log" 2>&1 &
    echo $! > "$run/diagslave.pid"

    diagslave -m tcp -p 41502 > "$run/diagslave-tcp.log" 2>&1 &
    echo $! > "$run/diagslave-tcp.pid"

    mosquitto -c qemu-tests/harness/mosquitto.conf > "$run/mosquitto.log" 2>&1 &
    echo $! > "$run/mosquitto.pid"

    python3 -m http.server 48080 --bind 0.0.0.0 \
        --directory qemu-tests/harness/www > "$run/http.log" 2>&1 &
    echo $! > "$run/http.pid"

    echo "harness up in $run"

# Stop all harness services and remove the run directory.
down:
    #!/usr/bin/env bash
    set -u
    run="{{run_dir}}"
    [ -d "$run" ] || exit 0
    for pidfile in "$run"/*.pid; do
        [ -e "$pidfile" ] || continue
        kill "$(cat "$pidfile")" 2>/dev/null || true
    done
    sleep 0.3
    rm -rf "$run"
    echo "harness down"

# Run the QEMU tests in self-contained (in-memory I/O) mode; no harness needed.
run:
    cd qemu-tests && cargo run --release

# Run the QEMU tests against the live harness services (requires `just up`).
test:
    cd qemu-tests && cargo run --release --features harness

# Tail the harness service logs.
logs:
    tail -n +1 "{{run_dir}}"/*.log
