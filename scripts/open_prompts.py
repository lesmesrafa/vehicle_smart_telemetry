#!/usr/bin/env python3
"""Rewrites a file of prompts so that the model has to judge by itself.

Two things change, and nothing else: the windows and their numbers are the same.

1. The question loses the sentences that say where its answers part ("braking is hard when the
   longitudinal acceleration goes below -2 m/s^2"). Only the question stays, so what "hard"
   means is for the model to know.
2. Each value of a window comes beside what is known of its signal, in place of a number that
   points at a list given before:

       - longitudinal acceleration, negative when braking, in m/s^2: last -0.42, min -3.12, ...

   With the graph, what is known of a signal is its quantity, its sign and its unit. Without
   it, its name in the DBC:

       - ESP12.LONG_ACCEL: last -0.42, min -3.12, ...

    python3 scripts/open_prompts.py --prompts results/balanced.graph.prompts.ndjson \\
        --output results/balanced.graph.open.prompts.ndjson

With --series, a third thing is added: after the summary of each signal comes how it moved
through the window, which is its mean in each bucket of the window, in order of time:

       - vehicle speed, in km/h: last 21.5, min 21.5, max 37.0, mean 27.8, change -15.5; second by second: 35.4, 31.8, 27.6, 23.9, 21.6

The receiver writes those means beside each window since version 0.10.0, under "series".

score_answers.py still checks the answers against the limits, which it holds, and says besides
whether the answers follow the number each question is about. --keep-limits leaves the question
whole, to tell apart what each of the changes does.
"""
import argparse
import json
import os
import re
import sys

# A line of the list of signals a prefix opens with: its number, what is known of the signal
# and, with the graph, the name in the DBC between brackets.
LISTED = re.compile(r"^(\d+)\. (.*?)(?: \(([A-Za-z0-9_]+\.[A-Za-z0-9_]+)[^()]*\))?$")
VALUE_LINE = re.compile(r"^(\d+)\. (last .*|no data)$")
SECONDS = re.compile(r"^Window of (\S+) s ")
READS = "Each window lists signals of the vehicle. For each one it gives: last value, minimum, maximum, mean and change (last minus first)."
MOVED = " After that comes how the signal moved through the window: its mean {every}, from the start of the window to its end."


def every(entry, path):
    """How far apart the means of a series are, in words: the length of the window over how many there are."""
    if not entry.get("series"):
        sys.exit(f"{path} has no series beside its windows: it was written by a receiver older than 0.10.0.\n"
                 "Bring the latest code to the Pis (make update) and write the windows again (make windows LOG=data/balanced.log)")
    step = float(SECONDS.match(entry["window"]).group(1)) / len(entry["series"][0])
    return "second by second" if step == 1 else f"every {step:g} s"


def main():
    parser = argparse.ArgumentParser(description="The question without its limits, and each value beside what is known of its signal.")
    parser.add_argument("--prompts", required=True, help="what the receiver wrote for the model")
    parser.add_argument("--output", required=True, help="the same windows, written the new way")
    parser.add_argument("--keep-limits", action="store_true", help="leave the text of the question whole")
    parser.add_argument("--series", action="store_true", help="after the summary of each signal, how it moved through the window")
    args = parser.parse_args()

    entries = [json.loads(line) for line in open(args.prompts)]
    # With --series, how far apart the means are: the first window of each question says it.
    steps = {}
    if args.series:
        for entry in entries:
            if "window" in entry and entry["question"] not in steps:
                steps[entry["question"]] = every(entry, args.prompts)

    # Written under another name and given its name at the end: the file is never seen half written.
    known, written, part = {}, 0, f"{args.output}.{os.getpid()}.part"
    with open(part, "w") as out:
        for entry in entries:
            question = entry["question"]
            if "prefix" in entry:
                lines = entry["prefix"].split("\n")
                if any(l.startswith("Examples,") for l in lines):
                    os.remove(part)
                    sys.exit(f"{args.prompts} carries worked examples, written the other way: run this on the prompts before add_examples.py")
                # The list of signals ends where the question starts.
                listed = [LISTED.match(l) for l in lines[:next(i for i, l in enumerate(lines) if l.startswith("Question:"))]]
                listed = sorted((m for m in listed if m), key=lambda m: int(m.group(1)))
                known[question] = [m.group(2) for m in listed]
                names = [m.group(3) or m.group(2) for m in listed]
                text = next(l for l in lines if l.startswith("Question:"))[len("Question:"):].strip()
                options = next(l for l in lines if l.startswith("Options:"))
                if not args.keep_limits and "?" in text:
                    text = text[:text.index("?") + 1]      # the question itself; what follows states its limits
                reads = READS + (MOVED.format(every=steps[question]) if question in steps else "")
                out.write(json.dumps({"question": question, "prefix": f"Question: {text}\n{options}\n{reads}\n", "signals": names,
                                      "limits": "shown" if args.keep_limits else "not shown"}) + "\n")
                print(f"{question}: {text}")
                for label in known[question]:
                    print(f"    - {label}: ..." + (f"; {steps[question]}: ..." if question in steps else ""))
                continue
            if args.series and "series" not in entry:
                os.remove(part)
                sys.exit(f"{args.prompts}: the window of {question} that ends at t={entry['t']} has no series")
            lines = []
            for l in entry["window"].split("\n"):
                found = VALUE_LINE.match(l)
                if not found:
                    lines.append(l)
                    continue
                place = int(found.group(1)) - 1
                moved = ""
                if args.series and found.group(2) != "no data":
                    moved = f"; {steps[question]}: " + ", ".join("no data" if mean is None else mean for mean in entry["series"][place])
                lines.append(f"- {known[question][place]}: {found.group(2)}{moved}")
            out.write(json.dumps({"question": question, "t": entry["t"], "window": "\n".join(lines)}) + "\n")
            written += 1
    os.replace(part, args.output)
    print(f"{written} windows written to {args.output}")


if __name__ == "__main__":
    main()