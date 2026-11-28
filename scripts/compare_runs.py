#!/usr/bin/env python3
"""Two runs over the same windows, side by side, and how sure their difference is.

For each question and for their mean it gives the two figures of score_answers.py for each run,
balanced accuracy and how much the answers follow the number the question is about, and the
difference between the runs with a 95 % interval.

    python3 scripts/compare_runs.py --prompts results/balanced.graph.open.prompts.ndjson \\
        --answers results/balanced.graph.open.LFM2.5-1.2B-Instruct-Q4_K_M.answers.ndjson \\
        --against results/balanced.bare.open.LFM2.5-1.2B-Instruct-Q4_K_M.answers.ndjson

The two runs must have answered the same windows: with the graph and without it, or with the
series and without it. The right answers and the numbers of the windows are read from --prompts,
and they are the same whichever of the two files of prompts is given.

The interval comes from drawing the windows again, with replacement, as many times as --resamples
says: each draw takes whole windows, with all their questions and with the answers of both runs,
so what the two runs share stays shared. It is the range that holds 95 % of the differences of
those draws. An interval that leaves zero outside says the difference is not one of chance in
this drive; it says nothing of another drive or another car.
"""
import argparse
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score_answers import FOLLOWS, RULES, read_prompts, together  # noqa: E402


def answers_of(path):
    given = {}
    for line in open(path):
        entry = json.loads(line)
        given[(entry["question"], entry["t"])] = entry["answer"]
    return given


def figures(rows, questions):
    """For each question, (balanced accuracy, follows) of each of the two runs over these rows.
    A row is (question, right answer, number, answer of the first run, answer of the second)."""
    out = []
    for question in questions:
        mine = [row for row in rows if row[0] == question]
        order = FOLLOWS[question][2]
        pair = []
        for run in (3, 4):
            classes = sorted({row[1] for row in mine})
            recalls = [sum(row[run] == c for row in mine if row[1] == c) / sum(row[1] == c for row in mine) for c in classes]
            follows = together([row[2] for row in mine], [order.index(row[run]) if row[run] in order else -1 for row in mine])
            pair.append((100 * sum(recalls) / len(recalls), follows or 0.0))
        out.append(pair)
    return out


def with_mean(per_question):
    """The same list with one more entry: the mean over the questions."""
    mean = [tuple(sum(pair[run][k] for pair in per_question) / len(per_question) for k in (0, 1)) for run in (0, 1)]
    return per_question + [mean]


def main():
    parser = argparse.ArgumentParser(description="Two runs over the same windows, and the interval of their difference.")
    parser.add_argument("--prompts", required=True, help="the prompts of either run: they give the right answers")
    parser.add_argument("--answers", required=True, help="the answers of the first run")
    parser.add_argument("--against", required=True, help="the answers of the second run")
    parser.add_argument("--resamples", type=int, default=1000, help="how many times the windows are drawn again (1000)")
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()

    read = read_prompts(args.prompts)
    truth, stats = read["truth"], read["stats"]
    first, second = answers_of(args.answers), answers_of(args.against)
    keys = sorted(key for key in truth if key in first and key in second)
    if not keys:
        sys.exit("error: the two runs have no window in common with a right answer")
    only = sum(key in truth for key in first) + sum(key in truth for key in second) - 2 * len(keys)
    questions = [q for q in RULES if any(key[0] == q for key in keys)]
    by_time = {}
    for key in keys:
        question, t = key
        by_time.setdefault(t, []).append((question, truth[key], FOLLOWS[question][1](stats[key]), first[key], second[key]))
    times = sorted(by_time)

    here = with_mean(figures([row for t in times for row in by_time[t]], questions))
    rng, differences = random.Random(args.seed), [[[], []] for _ in here]
    for _ in range(args.resamples):
        drawn = [row for t in rng.choices(times, k=len(times)) for row in by_time[t]]
        if {row[0] for row in drawn} != set(questions):
            continue        # a draw that left a question with no window: it says nothing of the mean
        for i, pair in enumerate(with_mean(figures(drawn, questions))):
            for k in (0, 1):
                differences[i][k].append(pair[0][k] - pair[1][k])

    def interval(values):
        values = sorted(values)
        return values[int(0.025 * (len(values) - 1))], values[int(round(0.975 * (len(values) - 1)))]

    print(f"first:  {args.answers}\nsecond: {args.against}")
    print(f"{len(keys)} answers of {len(questions)} questions, over {len(times)} windows both runs answered"
          + (f"; {only} answers of only one of them left out" if only else ""))
    print(f"\n{'':14s}{'balanced accuracy, %':^38s}    {'follows the number of the question':^42s}")
    print(f"{'question':14s}{'first':>6s} {'second':>6s}   {'first - second':24s}{'first':>6s} {'second':>6s}   first - second")
    for name, pair, (accuracy, follows) in zip(questions + ["mean"], here, differences):
        low, high = interval(accuracy)
        f_low, f_high = interval(follows)
        print(f"{name:14s}{pair[0][0]:6.1f} {pair[1][0]:6.1f}   {pair[0][0] - pair[1][0]:+5.1f} [{low:+5.1f}, {high:+5.1f}]      "
              f"{pair[0][1]:+6.2f} {pair[1][1]:+6.2f}   {pair[0][1] - pair[1][1]:+.2f} [{f_low:+.2f}, {f_high:+.2f}]")
    print(f"\n95 % intervals from {len(differences[-1][0])} draws of the {len(times)} windows, each with all its questions and both runs")


if __name__ == "__main__":
    main()