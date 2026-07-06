#!/bin/sh
# QEMU runner for the embassy qemu-tests. Networking runs over the board's emulated
# LAN9118 NIC with QEMU's built-in user-mode stack (guest 10.0.2.15 via DHCP, host
# services reachable at 10.0.2.2) — no privileges or host daemons needed. When the
# harness pty created by `just up` exists, guest UART1 is wired to the Modbus RTU
# bridge; otherwise it is stubbed so plain `cargo run` still works.
#
# Debugging the NIC: append
#   -object filter-dump,id=dump0,netdev=net0,file=/tmp/lan9118.pcap
# to capture the guest's traffic.
set -eu

RUN_DIR="${FERREDGE_QEMU_RUN_DIR:-/tmp/ferredge-qemu-harness}"

if [ -e "$RUN_DIR/modbus-guest" ]; then
    # QEMU's -serial shorthand only accepts device paths, so resolve the pty symlink.
    set -- -serial null -serial "$(readlink -f "$RUN_DIR/modbus-guest")" -kernel "$@"
else
    set -- -serial null -serial null -kernel "$@"
fi

exec qemu-system-arm -cpu cortex-m3 -machine mps2-an385 -nographic -monitor none \
    -nic user,model=lan9118,id=net0 \
    -semihosting-config enable=on,target=native "$@"
