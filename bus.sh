#!/usr/bin/env bash
# Creates the virtual CAN bus of this Raspberri Pi and, given the IP of another one, joins both with canneloni
#   ./bus.sh start 10.77.0.2    on "car"      (the IP is always that of the OTHER Raspberry)
#   ./bus.sh start 10.77.0.1    on "gateway"
#   ./bus.sh start              only the local bus, no tunnel
#   ./bus.sh status
#   ./bus.sh stop
#
# Behaviour can be changed with environment variables:
#   BUS=vcan0  PORT=20000  WAIT_US=1000  TCP=""  (TCP=s or TCP=c to use TCP instead of UDP)
set -euo pipefail

BUS="${BUS:-vcan0}"
PORT="${PORT:-20000}"
WAIT_US="${WAIT_US:-1000}"          # how long cannelloni holds a frame before sending it, in microseconds
TCP="${TCP:-}"
DIR="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$DIR/logs"
PIDF="$DIR/logs/cannelloni.pid"
LOGF="$DIR/logs/cannelloni.log"

tunnel_alive() { [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; }



case "${1:-}" in
  start)
    sudo modprobe vcan                                   # loads the virtual CAN bus driver
    if ! ip link show "$BUS" >/dev/null 2>&1; then
      sudo ip link add dev "$BUS" type vcan              # creates the bus
    fi
    sudo ip link set up "$BUS"                           # brings it up
    echo "bus $BUS ready"
    if [ -n "${2:-}" ]; then
      command -v cannelloni >/dev/null || { echo "cannelloni not found: install it first (GUIDE.md, step B6)"; exit 1; }
      if tunnel_alive; then
        echo "the tunnel was already running (PID $(cat "$PIDF"))"
        exit 0
      fi
      if [ -n "$TCP" ]; then
        nohup cannelloni -I "$BUS" -C "$TCP" -R "$2" -r "$PORT" -l "$PORT" >"$LOGF" 2>&1 &
      else
        nohup cannelloni -I "$BUS" -R "$2" -r "$PORT" -l "$PORT" -t "$WAIT_US" >"$LOGF" 2>&1 &
      fi
      echo $! >"$PIDF"
      sleep 1
      if tunnel_alive; then
        echo "tunnel to $2 running (PID $(cat "$PIDF")); its log is in logs/cannelloni.log"
      else
        echo "cannelloni did not start. This is what it said:"
        cat "$LOGF"
        rm -f "$PIDF"
        exit 1
      fi
    fi
    ;;

    status)
        if ip link show "$BUS" >/dev/null 2>&1; then
            ip -statistics link show "$BUS"
        else
            echo "bus $BUS does not exist"
        fi
        if tunnel_alive; then
            echo "tunnel: running (PID $(cat "$PIDF"))"
        else
            echo "tunnel: stopped"
        fi
        ;;

    stop)
        if tunnel_alive; then
            kill "$(cat "$PIDF")"
            echo "tunnel stopped"
        fi
            rm -f "$PIDF"
        if ip link show "$BUS" >/dev/null 2>&1; then
            sudo ip link delete "$BUS"
            echo "bus $BUS removed"
        fi
        ;;
    *)
        sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
        exit 1
        ;;
    esac