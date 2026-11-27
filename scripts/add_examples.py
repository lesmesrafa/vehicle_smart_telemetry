#!/usr/bin/env python3
"""Adds worked examples to the fixed part of each question in a file of prompts.

A small model that is only told the limits of a question ("braking is hard below -2 m/s^2") may
not apply them. Here it is also shown a few windows with their right answer. The examples go in
the prefix, which the model reads once and keeps, so an answer costs what it cost without them.

The examples are taken from the first windows of the file, with the answers the rules of
score_answers.py give, the same number for every answer of the question. Those first windows are
left out of what is written, so no window is both an example and a question.

    python3 scripts/add_examples.py --prompts results/balanced.graph.prompts.ndjson \\
        --output results/balanced.graph.shots2.prompts.ndjson
    python3 scripts/add_examples.py --prompts ... --output ... --per-answer 3 --pool 150

With the same --pool, --per-answer and --seed, two files made from the same recording (with the
graph and without it) get the same windows as examples.
"""
import argparse
import json
import os
import random
import sys

# The rules live in score_answers.py, beside this file, wherever this is run from.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score_answers import right_answers


def main():
    parser = argparse.ArgumentParser(description="Worked examples in the fixed part of each question.")
    parser.add_argument("--prompts", required=True, help="what the receiver wrote for the model")
    parser.add_argument("--output", required=True, help="the same, with examples and without the windows they come from")
    parser.add_argument("--per-answer", type=int, default=2, help="examples of each answer of a question (default 2)")
    parser.add_argument("--pool", type=int, default=100, help="how many of the first windows the examples come from (default 100)")
    parser.add_argument("--seed", type=int, default=1, help="the same seed picks the same windows (default 1)")
    args = parser.parse_args()

    truth, _, no_rule, other_text = right_answers(args.prompts)
    if other_text:
        sys.exit(f"error: the text of {', '.join(other_text)} does not state the limits of its rule, so its examples would be wrong")
    prefixes, windows = {}, {}
    for line in open(args.prompts):
        entry = json.loads(line)
        if "prefix" in entry:
            prefixes[entry["question"]] = entry["prefix"]
        else:
            windows.setdefault(entry["question"], []).append(entry)

    kept = []
    for question, prefix in prefixes.items():
        asked = sorted(windows.get(question, []), key=lambda entry: entry["t"])
        pool, rest = asked[:args.pool], asked[args.pool:]
        if question in no_rule:
            # No rule, no right answers: the question stays as it was, over the same windows as the others.
            kept.append(({"question": question, "prefix": prefix}, rest))
            print(f"{question}: no rule for it, left without examples")
            continue
        # The answers in the order the question lists them.
        options = [o.strip() for o in next(l for l in prefix.split("\n") if l.startswith("Options:"))[len("Options:"):].split(",")]
        rng = random.Random(f"{args.seed} {question}")
        chosen = {}
        for answer in options:
            having = [entry for entry in pool if truth.get((question, entry["t"])) == answer]
            chosen[answer] = rng.sample(having, min(args.per_answer, len(having)))
        # One of each answer in turn, so that no answer comes several times in a row.
        examples = []
        for i in range(args.per_answer):
            examples += [(entry, answer) for answer, entries in chosen.items() for entry in entries[i:i + 1]]
        text = prefix + "Examples, each with its right answer:\n" + "\n".join(f"{entry['window']} {answer}" for entry, answer in examples) + "\n"
        kept.append(({"question": question, "prefix": text}, rest))
        counts = ", ".join(f"{answer} {len(entries)}" for answer, entries in chosen.items())
        short = [answer for answer, entries in chosen.items() if len(entries) < args.per_answer]
        print(f"{question}: examples {counts}; prefix from {len(prefix.encode())} to {len(text.encode())} bytes; {len(rest)} windows to answer"
              + (f"; NOT ENOUGH windows of {', '.join(short)} among the first {args.pool}: try a larger --pool" if short else ""))

    with open(args.output, "w") as out:
        for prefix, _ in kept:
            out.write(json.dumps(prefix) + "\n")
        # The windows in the order of the recording, one line per question, as the receiver writes them.
        for entries in zip(*(rest for _, rest in kept)):
            for entry in entries:
                out.write(json.dumps(entry) + "\n")
    print(f"written to {args.output}")


if __name__ == "__main__":
    main()
