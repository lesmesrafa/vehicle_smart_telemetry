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
VALUES = re.compile(r"^(\d+)\. last (\S+), min (\S+), max (\S+), mean (\S+), change (\S+)$")


def right_answers(prompts_path):
    """(question, t) -> the answer the rule gives, for every window whose signals all had values;
    how many windows were left out; the questions asked that have no rule here; and the ones
    whose text does not state the limits of their rule."""
    signals, truth, skipped, no_rule, other_text = {}, {}, 0, [], []
    for line in open(prompts_path):
        entry = json.loads(line)
        question = entry["question"]
        if "prefix" in entry:
            # The signals of the question, in the order the windows list them.
            signals[question] = {}
            if question not in RULES:
                no_rule.append(question)
            elif STATED[question] not in entry["prefix"]:
                other_text.append(question)
            for text in entry["prefix"].split("\n"):
                found = SIGNAL_NAME.match(text)
                if found:
                    signals[question][int(found.group(1))] = found.group(2)
            continue
        if question not in RULES:
            continue
        stats = {}
        for text in entry["window"].split("\n"):
            found = VALUES.match(text)
            if found:
                last, low, high, mean, change = (float(x) for x in found.groups()[1:])
                stats[signals[question][int(found.group(1))]] = dict(last=last, min=low, max=high, mean=mean, change=change)
        try:
            truth[(question, entry["t"])] = RULES[question](stats)
        except KeyError:
            skipped += 1        # a signal with no data in this window: there is no answer to check
    return truth, skipped, no_rule, other_text


def main():
    parser = argparse.ArgumentParser(description="Answers of the model against the answers the signals give.")
    parser.add_argument("--prompts", default="prompts.ndjson", help="what the receiver wrote for the model")
    parser.add_argument("--answers", help="what model_client wrote; without it, only the right answers are counted")
    parser.add_argument("--save", metavar="FILE", help="write question, t, right answer and answer of the model, as TSV")
    args = parser.parse_args()

    truth, skipped, no_rule, other_text = right_answers(args.prompts)
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
    missing = [q for q in RULES if q not in questions]
    if missing and (no_rule or other_text):
        warnings.append(f"not asked: {', '.join(missing)}")
    print("\n".join(warnings + [""]))
    balanced_all, rows = [], []
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
        rows += [(question, key[1], truth[key], given[key]) for key in answered]
    if balanced_all:
        print(f"\nmean balanced accuracy over {len(balanced_all)} questions: {100 * sum(balanced_all) / len(balanced_all):.1f} %")
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
