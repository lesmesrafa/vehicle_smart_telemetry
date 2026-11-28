#!/usr/bin/env bash
# Prepares one Raspberry Pi for jedge-stream. Nobody runs it by hand: `make setup` on your
# computer sends it to each Pi through ssh, and it runs there.
#
#     setup_pi.sh ROLE REPO DIR ADDRESS IFACE
#
#     ROLE      gateway or vehicle
#     REPO      git address of the project
#     DIR       folder of the project on the Pi, inside the home folder
#     ADDRESS   address of this Pi on the cable between the two, for example 10.77.0.1/24
#     IFACE     network port of that cable, for example eth0
#
# Every step looks first and acts only if something is missing, so it can be run again.
set -euo pipefail

say() {
    printf '\n== %s\n' "$*"
}

# git, curl, make and a C compiler, which cargo needs to link the programs.
packages() {
    local missing=""
    for tool in git curl make cc; do
        command -v "$tool" > /dev/null || missing="$missing $tool"
    done
    if [ -z "$missing" ]; then
        echo "packages: nothing to install"
        return
    fi
    echo "packages: installing, because these are missing:$missing"
    sudo -n apt-get update -q < /dev/null
    sudo -n apt-get install -y -q git curl make gcc libc6-dev < /dev/null
}

# Rust 1.77 or later. If the Pi has none, or an older one, rustup installs the current one.
rust() {
    export PATH="$HOME/.cargo/bin:$PATH"
    local minor=0
    if command -v cargo > /dev/null; then
        minor=$(cargo --version | cut -d. -f2)
    fi
    if [ "$minor" -lt 77 ]; then
        echo "rust: installing with rustup"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    fi
    echo "rust: $(cargo --version)"
}

# The first time it clones the project. After that it brings what is new.
project() {
    local repo=$1 dir=$2
    export GIT_TERMINAL_PROMPT=0
    if [ -d "$dir/.git" ]; then
        git -C "$dir" pull --ff-only
    elif [ -e "$dir" ]; then
        echo "project: $dir exists and is not a git clone. Move it away, or choose another DIR" >&2
        exit 1
    else
        git clone "$repo" "$dir"
    fi
    echo "project: $(git -C "$dir" log -1 --format='commit %h, %s')"
}

# The fixed address of this Pi on the cable between the two. NetworkManager keeps it after a
# reboot.
link() {
    local address=$1 iface=$2
    local ip=${address%/*}
    if ip -o -4 addr show | grep " $ip/" > /dev/null; then
        echo "link: $ip is already set"
        return
    fi
    # The port this ssh session comes in through is never touched: that would cut the session.
    local session_ip
    session_ip=$(echo "${SSH_CONNECTION:-}" | cut -d' ' -f3)
    if [ -n "$session_ip" ] && ip -o addr show dev "$iface" | grep " $session_ip/" > /dev/null; then
        echo "link: not changed, because this session comes in through $iface"
        return
    fi
    echo "link: setting $address on $iface"
    if nmcli -t -f NAME connection show | grep -x bench > /dev/null; then
        sudo -n nmcli connection modify bench ipv4.addresses "$address"
    else
        sudo -n nmcli connection add type ethernet ifname "$iface" con-name bench \
            ipv4.method manual ipv4.addresses "$address" ipv4.never-default yes \
            ipv6.method link-local connection.autoconnect yes connection.autoconnect-priority 10
    fi
    sudo -n nmcli connection up bench
}

# Compiles the two programs. The vehicle also gets the DBC; the gateway does not need it.
build() {
    local role=$1 dir=$2
    make -C "$dir" --no-print-directory build
    if [ "$role" = vehicle ]; then
        make -C "$dir" --no-print-directory dbc
    fi
}

main() {
    if [ $# -ne 5 ]; then
        echo "usage: setup_pi.sh ROLE REPO DIR ADDRESS IFACE" >&2
        exit 2
    fi
    local role=$1 repo=$2 dir=$3 address=$4 iface=$5
    say "$role: $(hostname)"
    packages
    rust
    project "$repo" "$dir"
    link "$address" "$iface"
    build "$role" "$dir"
    say "$role: ready"
}

# The call is the last line on purpose. bash has read the whole file before anything runs, so a
# program started here cannot swallow part of it when the file arrives through ssh.
main "$@"
