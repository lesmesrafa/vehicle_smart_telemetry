#!/usr/bin/env python3
"""Checks the answers of the model against the answers the signals themselves give.

For every window, the right answer of each question is worked out by a rule from the same
numbers the model was shown (last, minimum, maximum, mean and change of each signal), with the
limits that the text of the question states. So this measures whether the model read the window
well, and nothing else.

    python3 scripts/score_answers.py --prompts prompts.ndjson --answers answers.ndjson
    python3 scripts/score_answers.py --prompts prompts.ndjson            # only what the right answers are

Balanced accuracy is the mean, over the answers that occur, of the share of windows with that
answer that the model got right. A model that always says the same scores 1 / (answers that occur),
or nothing if what it says never occurs.
"""
import argparse
import json
import re
import sys


def yes_no(condition):
    return "yes" if condition else "no"


def turning(s):
    left, right = s["SAS11.SAS_Angle"]["max"], -s["SAS11.SAS_Angle"]["min"]
    if max(left, right) <= 30:
        return "straight"
    return "left" if left >= right else "right"


def speed_band(s):
    speed = s["CLU11.CF_Clu_Vanz"]["last"]
    return "stopped" if speed < 1 else "slow" if speed < 30 else "medium" if speed < 80 else "fast"


def speed_trend(s):
    change = s["CLU11.CF_Clu_Vanz"]["change"]
    return "faster" if change >= 5 else "slower" if change <= -5 else "steady"


# The limits are the ones each question states in questions.tsv.
RULES = {
    "hard_braking": lambda s: yes_no(s["ESP12.LONG_ACCEL"]["min"] < -2.0),
    "turning": turning,
    "speed_band": speed_band,
    "speed_trend": speed_trend,
    "idling": lambda s: yes_no(s["CLU11.CF_Clu_Vanz"]["max"] < 1 and s["EMS11.N"]["min"] > 300),
    "coasting": lambda s: yes_no(s["CLU11.CF_Clu_Vanz"]["change"] < -3 and s["ESP12.CYL_PRES"]["max"] <= 3),
}

# What the text of each question has to say for its rule to be the one that applies. A question
# with the same name and another text was asked something else, and its score here means little.
STATED = {
    "hard_braking": "goes below -2 m/s^2",
    "turning": "goes beyond 30 deg to either side",
    "speed_band": "Below 1 km/h it is stopped, below 30 km/h slow, below 80 km/h medium, and from 80 km/h fast",
    "speed_trend": "changes by less than 5 km/h",
    "idling": "below 1 km/h, and the engine runs above 300 rpm",
    "coasting": "drops by more than 3 km/h, and the brake is in use when the pressure goes above 3 bar",
}

SIGNAL_NAME = re.compile(r"^(\d+)\. (?:.*\()?([A-Za-z0-9_]+\.[A-Za-z0-9_]+)")
# A line of a window: a signal, by its number in the list or by what is known of it, and either
# its five values or "no data". After the five values there may come how the signal moved.
SIGNAL_LINE = re.compile(r"^(?:\d+\.|-) ")
VALUES = re.compile(r"last (\S+), min (\S+), max (\S+), mean (\S+), change ([-+0-9.eE]+)")

# What each question is about, as one number of the window that grows with its answers, and the
# order of those answers. A model that reads the window gives answers that follow that number,
# wherever it puts its own limits: this is what can be asked of it when it is not told the limits.
FOLLOWS = {
    "hard_braking": ("how hard it slows down", lambda s: -s["ESP12.LONG_ACCEL"]["min"], ["no", "yes"]),
    "turning": ("the steering wheel angle, left being positive",
                lambda s: max(s["SAS11.SAS_Angle"]["max"], s["SAS11.SAS_Angle"]["min"], key=abs), ["right", "straight", "left"]),
    "speed_band": ("the speed at the end", lambda s: s["CLU11.CF_Clu_Vanz"]["last"], ["stopped", "slow", "medium", "fast"]),
    "speed_trend": ("the change of speed", lambda s: s["CLU11.CF_Clu_Vanz"]["change"], ["slower", "steady", "faster"]),
    "idling": ("how slow its fastest moment is", lambda s: -s["CLU11.CF_Clu_Vanz"]["max"], ["no", "yes"]),
    "coasting": ("how much speed it loses", lambda s: -s["CLU11.CF_Clu_Vanz"]["change"], ["no", "yes"]),
}


def read_prompts(prompts_path):
    """What a file of prompts holds, for the questions this script has a rule for:
    truth: (question, t) -> the answer the rule gives, when all the signals of the window had values;
    stats: (question, t) -> the numbers of that window, by signal;
    skipped: how many windows were left out; no_rule: the questions asked that have no rule here;
    other_text: the ones whose text does not state the limits of their rule, though it should;
    hidden: the ones whose limits were left out on purpose."""
    names, truth, stats_of, skipped, no_rule, other_text, hidden = {}, {}, {}, 0, [], [], []
    for line in open(prompts_path):
        entry = json.loads(line)
        question = entry["question"]
        if "prefix" in entry:
            if question not in RULES:
                no_rule.append(question)
            elif entry.get("limits") == "not shown":
                hidden.append(question)
            elif STATED[question] not in entry["prefix"]:
                other_text.append(question)
            # The signals of the question, in the order the windows list them: either the file
            # says them, or they are the list the prefix opens with. That list ends where the
            # question starts: a prefix may go on with worked examples.
            if "signals" in entry:
                names[question] = entry["signals"]
            else:
                found = [SIGNAL_NAME.match(text) for text in entry["prefix"].split("\nQuestion:")[0].split("\n")]
                names[question] = [f.group(2) for f in sorted((f for f in found if f), key=lambda f: int(f.group(1)))]
            continue
        if question not in RULES:
            continue
        stats, place = {}, 0
        for text in entry["window"].split("\n"):
            if not SIGNAL_LINE.match(text):
                continue
            found = VALUES.search(text)
            if found:
                last, low, high, mean, change = (float(x) for x in found.groups())
                stats[names[question][place]] = dict(last=last, min=low, max=high, mean=mean, change=change)
            place += 1
        try:
            truth[(question, entry["t"])] = RULES[question](stats)
            stats_of[(question, entry["t"])] = stats
        except KeyError:
            skipped += 1        # a signal with no data in this window: there is no answer to check
    return dict(truth=truth, stats=stats_of, skipped=skipped, no_rule=no_rule, other_text=other_text, hidden=hidden)


def right_answers(prompts_path):
    """(question, t) -> the right answer; windows left out; questions with no rule; questions with another text."""
    read = read_prompts(prompts_path)
    return read["truth"], read["skipped"], read["no_rule"], read["other_text"]


def ranks(values):
    """The place of each value among the others, from the lowest; equal values share the mean of their places."""
    order = sorted(range(len(values)), key=lambda i: values[i])
    placed, i = [0.0] * len(values), 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        for k in range(i, j + 1):
            placed[order[k]] = (i + j) / 2
        i = j + 1
    return placed


def together(xs, ys):
    """Rank correlation: +1 when one grows whenever the other does, 0 when they have nothing to do. None if either never changes."""
    rx, ry = ranks(xs), ranks(ys)
    mx, my = sum(rx) / len(rx), sum(ry) / len(ry)
    sx, sy = sum((x - mx) ** 2 for x in rx), sum((y - my) ** 2 for y in ry)
    if sx == 0 or sy == 0:
        return None
    return sum((x - mx) * (y - my) for x, y in zip(rx, ry)) / (sx * sy) ** 0.5


def following(question, keys, stats, truth, given):
    """Lines that say whether the answers of the model follow the number the question is about, and how much they do."""
    what, number, order = FOLLOWS[question]
    numbers = [number(stats[key]) for key in keys]
    model = together(numbers, [order.index(given[key]) if given[key] in order else -1 for key in keys])
    right = together(numbers, [order.index(truth[key]) for key in keys])
    lines = [f"    follows {what}: " + ("not at all, it is one answer" if model is None else f"{model:+.2f}")
             + (f" (the right answers: {right:+.2f}; 0 is not at all)" if right is not None else "")]
    # The windows in three equal parts by that number, and what the model said in each.
    by_number = sorted(keys, key=lambda key: number(stats[key]))
    third = len(by_number) // 3
    parts = [by_number[:third], by_number[third:len(by_number) - third], by_number[len(by_number) - third:]]
    if third:
        said = ["; ".join(f"{a} {100 * sum(given[k] == a for k in part) / len(part):.0f} %" for a in order if any(given[k] == a for k in part)) for part in parts]
        lines.append(f"        where it is lowest: {said[0]} | in the middle: {said[1]} | where it is highest: {said[2]}")
    return lines, model or 0.0


def main():
    parser = argparse.ArgumentParser(description="Answers of the model against the answers the signals give.")
    parser.add_argument("--prompts", default="prompts.ndjson", help="what the receiver wrote for the model")
    parser.add_argument("--answers", help="what model_client wrote; without it, only the right answers are counted")
    parser.add_argument("--save", metavar="FILE", help="write question, t, right answer and answer of the model, as TSV")
    args = parser.parse_args()

    read = read_prompts(args.prompts)
    truth, stats, skipped, no_rule, other_text, hidden = (read[k] for k in ("truth", "stats", "skipped", "no_rule", "other_text", "hidden"))
    if not truth:
        sys.exit(f"error: {args.prompts} has no window of a question this script has a rule for: {', '.join(RULES)}")
    given = {}
    if args.answers:
        for line in open(args.answers):
            entry = json.loads(line)
            given[(entry["question"], entry["t"])] = entry["answer"]

    questions = [q for q in RULES if any(key[0] == q for key in truth)]
    print(f"{len(truth)} windows with a right answer, {skipped} left out because a signal had no data")
    warnings = []
    if no_rule:
        warnings.append(f"not scored, this script has no rule for them: {', '.join(no_rule)}")
    if other_text:
        warnings.append(f"WARNING: the text of {', '.join(other_text)} does not state the limits applied here, so the model was asked "
                        "something else.\n         Is the questions.tsv of the gateway the one that came with this script?")
    if hidden:
        warnings.append(f"the limits of {', '.join(hidden)} were not shown to the model: its answers are checked against them all the same,\n"
                        "and against the number each question is about")
    missing = [q for q in RULES if q not in questions]
    if missing and (no_rule or other_text):
        warnings.append(f"not asked: {', '.join(missing)}")
    print("\n".join(warnings + [""]))
    balanced_all, follows_all, rows = [], [], []
    for question in questions:
        keys = sorted(key for key in truth if key[0] == question)
        classes = sorted({truth[key] for key in keys})
        counts = ", ".join(f"{c} {sum(truth[k] == c for k in keys)}" for c in classes)
        print(f"{question}: {len(keys)} windows; right answers: {counts}")
        if not args.answers:
            continue
        answered = [key for key in keys if key in given]
        if not answered:
            print("    the model answered none of them")
            continue
        if len(answered) < len(keys):
            among = ", ".join(f"{c} {sum(truth[k] == c for k in answered)}" for c in classes if any(truth[k] == c for k in answered))
            print(f"    only {len(answered)} of them were answered; among those, right answers: {among}")
        said = sorted({given[key] for key in answered})
        print("    the model said: " + ", ".join(f"{c} {sum(given[k] == c for k in answered)}" for c in said))
        recalls = []
        for c in classes:
            of_class = [key for key in answered if truth[key] == c]
            if of_class:
                recalls.append(sum(given[key] == c for key in of_class) / len(of_class))
        accuracy = sum(given[key] == truth[key] for key in answered) / len(answered)
        balanced, constant = sum(recalls) / len(recalls), 1 / len(recalls)
        balanced_all.append(balanced)
        print(f"    accuracy {100 * accuracy:.1f} %; balanced accuracy {100 * balanced:.1f} %, "
              f"against {100 * constant:.1f} % for always the same answer ({len(answered)} answers)")
        lines, follows = following(question, answered, stats, truth, given)
        follows_all.append(follows)
        print("\n".join(lines))
        rows += [(question, key[1], truth[key], given[key]) for key in answered]
    if balanced_all:
        print(f"\nmean balanced accuracy over {len(balanced_all)} questions: {100 * sum(balanced_all) / len(balanced_all):.1f} %")
        print(f"the answers follow the number of their question by {sum(follows_all) / len(follows_all):+.2f} on average "
              "(+1 always, 0 not at all or with one answer, below 0 the other way round)")
    if other_text and balanced_all:
        print("\n".join(w for w in warnings if w.startswith("WARNING")))
    if args.save:
        with open(args.save, "w") as out:
            out.write("question\tt\tright_answer\tmodel_answer\n")
            for key in sorted(truth):
                out.write(f"{key[0]}\t{key[1]}\t{truth[key]}\t{given.get(key, '')}\n")
        print(f"written to {args.save}")


if __name__ == "__main__":
    main()
