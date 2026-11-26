#!/usr/bin/env python3
"""Turns recordings of decoded signals into the text log of CAN messages that signal_sender reads.

It is for the signal files of the X-CANIDS dataset (sig/dump*.parquet): one column per signal,
named <identifier in hexadecimal>_<signal>, and one row every few milliseconds. Each message is
built again from its signals with the DBC, the inverse of what the vehicle does when it decodes.

What comes out is not the traffic of the car: the values are the real ones, but every message
leaves at the same fixed rate, and only the messages asked for are written. For the bytes as
the car sent them, use the raw files of the dataset and parquet_to_candump.py.

Several recordings are put one after another. Each starts on a multiple of --window seconds,
after one empty window, so that no window of the gateway mixes two recordings.

    pip3 install pyarrow
    python3 scripts/signals_to_candump.py sig/dump1.parquet sig/dump2.parquet --output data/xcanids.log
"""
import argparse
import math
import re
import sys

import pyarrow.parquet as pq

START = 1700000000.0
# The messages of the signals the questions of this project ask about.
MESSAGES = "CLU11,ESP12,SAS11,WHL_SPD11,EMS11,EMS12,TCU12"


def read_dbc(path):
    """The messages of a DBC: name -> (identifier, [(signal, start bit, bits, signed, factor, offset)]),
    and the names some signals give to their values: (identifier, signal) -> {name: number}.
    Only signals written the Intel way (least significant bit first) are known here."""
    messages, names, current = {}, {}, None
    for line in open(path, encoding="latin-1"):
        table = re.match(r"VAL_ (\d+) (\w+) (.*);", line)
        if table:
            pairs = re.findall(r'(-?\d+) "([^"]*)"', table.group(3))
            names[(int(table.group(1)), table.group(2))] = {text: float(number) for number, text in pairs}
            continue
        head = re.match(r"BO_ (\d+) (\w+):", line)
        if head:
            current = messages.setdefault(head.group(2), (int(head.group(1)), []))
            continue
        signal = re.match(r"\s+SG_ (\w+) (?:\w+ )?: (\d+)\|(\d+)@([01])([+-]) \(([^,]+),([^)]+)\)", line)
        if signal and current is not None:
            name, start, bits, order, sign, factor, offset = signal.groups()
            if order != "1":
                sys.exit(f"error: {name} is not written the Intel way, which is all this script knows")
            current[1].append((name, int(start), int(bits), sign == "-", float(factor), float(offset)))
    return messages, names


def number(value, table):
    """A value of the recording as a number. Some signals come as the name of their value: the
    DBC says which number that is, and a name it does not know leaves the signal at its raw zero.
    Returns (number or None, whether the value is missing)."""
    if value is None or value == "nan" or (isinstance(value, float) and math.isnan(value)):
        return None, True
    if isinstance(value, str):
        return (table or {}).get(value), False
    return float(value), False


def encode(signals, values):
    """The bytes of a message from the physical value of each of its signals. A signal with no
    column in the recording stays at its raw zero."""
    word = 0
    for name, start, bits, signed, factor, offset in signals:
        value = values.get(name)
        if value is None:
            continue
        raw = round((value - offset) / factor)
        low, high = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
        raw = max(low, min(high, raw)) & ((1 << bits) - 1)
        word |= raw << start
    return word.to_bytes(8, "little")


def main():
    parser = argparse.ArgumentParser(description="Recordings of decoded signals to a candump log.")
    parser.add_argument("parquet", nargs="+", help="the recordings, in the order they go in the log")
    parser.add_argument("--output", required=True, metavar="FILE", help="the log to write")
    parser.add_argument("--dbc", default="hyundai_2015_ccan.dbc", help="the DBC of the car")
    parser.add_argument("--messages", default=MESSAGES, help=f"messages to write, by name (default {MESSAGES})")
    parser.add_argument("--rate", type=float, default=20.0, metavar="HZ", help="how often each message leaves (default 20)")
    parser.add_argument("--window", type=float, default=5.0, metavar="S", help="window of the gateway, in seconds (default 5)")
    parser.add_argument("--seconds", type=float, metavar="S", help="take only the first S seconds of each recording")
    parser.add_argument("--bus", default="can0", help="name of the bus written in each line (default can0)")
    args = parser.parse_args()

    dbc, names = read_dbc(args.dbc)
    wanted = [name.strip() for name in args.messages.split(",") if name.strip()]
    for name in wanted:
        if name not in dbc:
            sys.exit(f"error: the DBC has no message {name}")

    period, origin, written = 1.0 / args.rate, 0.0, 0
    with open(args.output, "w") as out:
        for path in args.parquet:
            file = pq.ParquetFile(path)
            columns = set(file.schema_arrow.names)
            # For each message, the column of each of its signals, where the recording has it.
            plan = []
            for name in wanted:
                identifier, signals = dbc[name]
                found = {s[0]: f"{identifier:03X}_{s[0]}" for s in signals if f"{identifier:03X}_{s[0]}" in columns}
                if found:
                    plan.append((identifier, signals, found))
                else:
                    print(f"{path}: no column of {name} ({identifier:03X}); the message is left out")
            needed = sorted({column for _, _, found in plan for column in found.values()})
            table = file.read(columns=needed + ["timestamp"])
            stamps = table.column("timestamp").to_pylist()
            times = [s.total_seconds() if hasattr(s, "total_seconds") else float(s) for s in stamps]
            data = {column: table.column(column).to_pylist() for column in needed}

            first, due, frames = times[0], times[0], 0
            for row, t in enumerate(times):
                if args.seconds is not None and t - first > args.seconds:
                    break
                if t + 1e-9 < due:
                    continue
                due += period
                for k, (identifier, signals, found) in enumerate(plan):
                    read = {name: number(data[column][row], names.get((identifier, name))) for name, column in found.items()}
                    # Until every signal of the message has arrived once, the message is not written.
                    if any(missing for _, missing in read.values()):
                        continue
                    payload = encode(signals, {name: value for name, (value, _) in read.items()}).hex().upper()
                    # The messages of one instant leave 1 ms apart, like frames sharing one bus.
                    out.write(f"({START + origin + (t - first) + 0.001 * k:.6f}) {args.bus} {identifier:03X}#{payload}\n")
                    frames += 1
            length = (t if args.seconds is None else min(t, first + args.seconds)) - first
            print(f"{path}: {frames} messages over {length:.0f} s, from t = {origin:.0f} s of the log")
            written += frames
            # The next recording starts on a multiple of the window, with one empty window before it.
            origin = (math.floor((origin + length) / args.window) + 2) * args.window
    print(f"{args.output}: {written} messages, {len(wanted)} kinds, every {1000 * period:.0f} ms")


if __name__ == "__main__":
    main()
