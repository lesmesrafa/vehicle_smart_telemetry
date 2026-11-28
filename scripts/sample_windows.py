#!/usr/bin/env python3
"""Keeps a sample of the windows of a file of prompts: for each question, as many windows of each
right answer, chosen at random with a seed.

Most windows of a drive have the common answer: the car does not brake hard, it goes straight.
How well a model tells the answers apart is measured on each answer by itself, so the windows of
the rare answer set how sure the figure is, and the common ones beyond that number cost time and
add little. With 40 windows of each answer, the balanced drive goes from 3594 questions to 639.

    python3 scripts/sample_windows.py --prompts results/balanced.graph.prompts.ndjson \\
        --output results/balanced.graph.s40.prompts.ndjson --per-answer 40

The right answers are the ones the rules of score_answers.py give. The choice depends only on
the seed, the question, the right answer and the time of the window, so the run with the graph
and the run without it, which have the same windows, keep the same ones. The lines kept are
copied as they are.
"""
import argparse
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score_answers import read_prompts  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description="For each question, as many windows of each right answer.")
    parser.add_argument("--prompts", required=True, help="what the receiver wrote for the model")
    parser.add_argument("--output", required=True, help="the same file with only the windows of the sample")
    parser.add_argument("--per-answer", type=int, default=40, help="windows of each right answer of each question (40)")
    parser.add_argument("--seed", type=int, default=1, help="which sample (1)")
    args = parser.parse_args()

    truth = read_prompts(args.prompts)["truth"]
    if not truth:
        sys.exit(f"error: {args.prompts} has no window with a right answer")
    times = {}
    for (question, t), answer in truth.items():
        times.setdefault((question, answer), []).append(t)
    kept = set()
    for (question, answer), found in sorted(times.items()):
        found.sort()
        picked = found if len(found) <= args.per_answer else random.Random(f"{args.seed}:{question}:{answer}").sample(found, args.per_answer)
        kept.update((question, t) for t in picked)

    # Written under another name and given its name at the end: the file is never seen half written.
    written, part = 0, f"{args.output}.{os.getpid()}.part"
    with open(part, "w") as out:
        for line in open(args.prompts):
            entry = json.loads(line)
            if "prefix" in entry or (entry["question"], entry["t"]) in kept:
                out.write(line)
                written += "window" in entry
    os.replace(part, args.output)
    questions = sorted({question for question, _ in times})
    for question in questions:
        counts = ", ".join(f"{answer} {min(len(found), args.per_answer)} of {len(found)}"
                           for (q, answer), found in sorted(times.items()) if q == question)
        print(f"{question}: {counts}")
    print(f"{written} of {len(truth)} windows written to {args.output}")


if __name__ == "__main__":
    main()