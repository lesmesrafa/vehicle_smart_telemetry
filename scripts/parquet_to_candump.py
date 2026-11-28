#!/usr/bin/env python3
"""Turns a recording of raw CAN messages kept as a Parquet file into the text log that
signal_sender reads, one line per message:

    (1700000000.000700) can0 2B0#0000000000

It was written for the raw recordings of the X-CANIDS dataset (raw/dump*.parquet), without
having seen them: the names of their columns are guessed, and --show prints what the file
really has so that they can be given by hand.

    pip3 install pyarrow
    python3 scripts/parquet_to_candump.py dump1.parquet --show
    python3 scripts/parquet_to_candump.py dump1.parquet --output data/dump1.log --seconds 300
"""
import argparse
import sys

import pyarrow.parquet as pq

# Names a column may have, in lower case, for each of the three things a line needs.
TIME_NAMES = ("timestamp", "time", "t", "ts", "time_stamp")
ID_NAMES = ("arbitration_id", "arb_id", "can_id", "id", "identifier", "aid")
DATA_NAMES = ("data", "payload", "data_field", "bytes", "raw")


def pick(asked, names, columns, what):
    """The column asked for by name, or the first one whose name is a usual one."""
    if asked:
        if asked not in columns:
            sys.exit(f"error: there is no column '{asked}'. The file has: {', '.join(columns)}")
        return asked
    for column in columns:
        if column.lower() in names:
            return column
    sys.exit(f"error: no column looks like the {what}. The file has: {', '.join(columns)}\n"
             f"Name it with --col-{what}.")


def seconds(value, unit):
    """A time as seconds. Numbers are in `unit`; dates and times of day are converted."""
    if hasattr(value, "timestamp"):
        return value.timestamp()
    if hasattr(value, "total_seconds"):
        return value.total_seconds()
    return float(value) * unit


def identifier(value):
    """The identifier of a message as a number, whether the file has a number or its hexadecimal text."""
    if isinstance(value, str):
        return int(value, 16)
    return int(value)


def payload(value):
    """The bytes of a message as hexadecimal text, from bytes, a list of numbers or text."""
    if value is None:
        return ""
    if isinstance(value, (bytes, bytearray)):
        return bytes(value).hex().upper()
    if isinstance(value, str):
        text = value.replace(" ", "").replace("0x", "")
        bytes.fromhex(text)   # fails here if it is not hexadecimal
        return text.upper()
    return bytes(int(b) for b in value).hex().upper()


def main():
    parser = argparse.ArgumentParser(description="Parquet recording of raw CAN messages to a candump log.")
    parser.add_argument("parquet", help="the recording, for example raw/dump1.parquet of X-CANIDS")
    parser.add_argument("--show", action="store_true", help="print the columns and the first rows, and stop")
    parser.add_argument("--output", metavar="FILE", help="the log to write")
    parser.add_argument("--seconds", type=float, metavar="S", help="stop after S seconds of recording")
    parser.add_argument("--max-frames", type=int, metavar="N", help="stop after N messages")
    parser.add_argument("--col-time", metavar="NAME", help="column with the time of each message")
    parser.add_argument("--col-id", metavar="NAME", help="column with the identifier of each message")
    parser.add_argument("--col-data", metavar="NAME", help="column with the bytes of each message")
    parser.add_argument("--time-unit", type=float, default=1.0, metavar="S",
                        help="seconds in one unit of the time column, if it is a number (default 1; 1e-6 for microseconds)")
    parser.add_argument("--bus", default="can0", help="name of the bus written in each line (default can0)")
    args = parser.parse_args()

    file = pq.ParquetFile(args.parquet)
    columns = file.schema_arrow.names
    if args.show:
        print(f"{args.parquet}: {file.metadata.num_rows} rows")
        for field in file.schema_arrow:
            print(f"  {field.name}: {field.type}")
        first = next(file.iter_batches(batch_size=5)).to_pylist()
        for row in first:
            print("  ", row)
        return
    if not args.output:
        sys.exit("error: --output FILE says where to write the log (or --show to look at the file)")

    time_col = pick(args.col_time, TIME_NAMES, columns, "time")
    id_col = pick(args.col_id, ID_NAMES, columns, "id")
    data_col = pick(args.col_data, DATA_NAMES, columns, "data")

    written, skipped, start, last, seen = 0, 0, None, None, set()
    with open(args.output, "w") as out:
        for batch in file.iter_batches(batch_size=65536, columns=[time_col, id_col, data_col]):
            times, ids, datas = (batch.column(c).to_pylist() for c in (time_col, id_col, data_col))
            for t, i, d in zip(times, ids, datas):
                try:
                    t, i, d = seconds(t, args.time_unit), identifier(i), payload(d)
                except (TypeError, ValueError):
                    skipped += 1
                    continue
                if len(d) > 16:      # more than 8 bytes: not a classic CAN message
                    skipped += 1
                    continue
                start = t if start is None else start
                if args.seconds is not None and t - start > args.seconds:
                    break
                out.write(f"({t:.6f}) {args.bus} {i:03X}#{d}\n")
                written, last = written + 1, t
                seen.add(i)
                if args.max_frames and written >= args.max_frames:
                    break
            else:
                continue
            break

    if not written:
        sys.exit(f"error: nothing was written; {skipped} rows could not be read. Look at the file with --show.")
    print(f"{args.output}: {written} messages, {len(seen)} identifiers, {last - start:.1f} s of recording"
          + (f"; {skipped} rows skipped" if skipped else ""))
    print(f"columns used: time '{time_col}', identifier '{id_col}', data '{data_col}'")


if __name__ == "__main__":
    main()
